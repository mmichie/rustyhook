use crate::command_executor::{execute_shell_command_with_context, CommandResult};
use crate::config::{RetryConfig, ShellConfig};
use crate::event::Event;
use crate::event_bus::EventBus;
use aws_sdk_sqs::types::Message;
use aws_sdk_sqs::Client;
use log::{debug, error, info, warn};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{sleep, Duration};

// Function to start the SQS message polling
#[allow(clippy::too_many_arguments)]
pub async fn sqs_poller(
    queue_url: String,
    poll_interval: u64,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    retry_config: RetryConfig,
    shell_config: ShellConfig,
    working_dir: Option<String>,
    mut shutdown_rx: broadcast::Receiver<()>,
    event_bus: Arc<EventBus>,
    mut event_rx: mpsc::UnboundedReceiver<Event>,
    forward_to: Vec<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Initializing SQS poller for queue: {}", queue_url);

    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let client = Client::new(&config);

    loop {
        tokio::select! {
            _ = async {
                info!("Polling SQS messages from {}", queue_url);
                match poll_sqs_messages(&client, &queue_url).await {
                    Some(messages) if !messages.is_empty() => {
                        info!("Received {} messages", messages.len());
                        for message in messages {
                            let result = process_message(&message, &shell_command, &handler_name, timeout, &retry_config, &shell_config, working_dir.as_deref()).await;

                            if result.is_success() {
                                delete_message(&client, &queue_url, &message).await;

                                // Forward event only on success
                                if !forward_to.is_empty() {
                                    let message_body = message.body().unwrap_or("");
                                    let message_id = message.message_id().unwrap_or("");
                                    let receipt_handle = message.receipt_handle().unwrap_or("");

                                    let event = Event::from_sqs(
                                        &handler_name,
                                        message_id,
                                        message_body,
                                        receipt_handle,
                                    );

                                    for target in &forward_to {
                                        if let Err(e) = event_bus.send(target, event.clone()) {
                                            warn!("Failed to forward event to '{}': {}", target, e);
                                        } else {
                                            debug!("Forwarded SQS event to '{}'", target);
                                        }
                                    }
                                }
                            } else {
                                let message_id = message.message_id().unwrap_or("unknown");
                                warn!(
                                    "Command failed for message {}, leaving in queue for retry",
                                    message_id
                                );
                            }
                        }
                    }
                    Some(_) => info!("No new messages received"),
                    None => info!("No messages received during this poll"),
                }

                info!("Waiting for next poll interval ({} seconds)", poll_interval);
                sleep(Duration::from_secs(poll_interval)).await;
            } => {}
            Some(forwarded_event) = event_rx.recv() => {
                // Handle forwarded events from other handlers
                info!(
                    "SQS handler '{}' received forwarded event from '{}'",
                    handler_name, forwarded_event.source_handler
                );
                let context = format!("Forwarded from: {}", forwarded_event.source_handler);
                execute_shell_command_with_context(
                    &shell_command,
                    &handler_name,
                    &context,
                    timeout,
                    &retry_config,
                    &shell_config,
                    working_dir.as_deref(),
                ).await;

                // Forward to next handlers if configured
                if !forward_to.is_empty() {
                    for target in &forward_to {
                        if let Err(e) = event_bus.send(target, forwarded_event.clone()) {
                            warn!("Failed to forward event to '{}': {}", target, e);
                        }
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("SQS handler '{}' received shutdown signal", handler_name);
                break;
            }
        }
    }

    info!("SQS handler '{}' shutting down", handler_name);
    Ok(())
}

async fn poll_sqs_messages(client: &Client, queue_url: &str) -> Option<Vec<Message>> {
    match client
        .receive_message()
        .queue_url(queue_url)
        .wait_time_seconds(20)
        .max_number_of_messages(10)
        .send()
        .await
    {
        Ok(response) => {
            info!("Successfully polled messages");
            response.messages
        }
        Err(e) => {
            error!("Error polling messages: {}", e);
            None
        }
    }
}

async fn process_message(
    message: &Message,
    shell_command: &str,
    handler_name: &str,
    timeout: u64,
    retry_config: &RetryConfig,
    shell_config: &ShellConfig,
    working_dir: Option<&str>,
) -> CommandResult {
    info!("Processing message: {:?}", message);

    let message_body = message.body().unwrap_or("(empty)");
    let message_id = message.message_id().unwrap_or("(no id)");
    let context = format!("Message ID: {}, Body: {}", message_id, message_body);

    execute_shell_command_with_context(
        shell_command,
        handler_name,
        &context,
        timeout,
        retry_config,
        shell_config,
        working_dir,
    )
    .await
}

async fn delete_message(client: &Client, queue_url: &str, message: &Message) {
    if let Some(receipt_handle) = message.receipt_handle() {
        match client
            .delete_message()
            .queue_url(queue_url)
            .receipt_handle(receipt_handle)
            .send()
            .await
        {
            Ok(_) => info!("Message deleted successfully."),
            Err(e) => error!("Error deleting message: {}", e),
        }
    } else {
        error!("No receipt handle found for message: {:?}", message);
    }
}
