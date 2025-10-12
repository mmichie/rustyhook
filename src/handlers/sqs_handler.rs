use crate::command_executor::execute_shell_command_with_context;
use log::{error, info};
use rusoto_core::credential::EnvironmentProvider;
use rusoto_core::{HttpClient, Region};
use rusoto_sqs::{DeleteMessageRequest, Message, ReceiveMessageRequest, Sqs, SqsClient};
use std::env;
use tokio::sync::broadcast;
use tokio::time::{sleep, Duration};

// Function to start the SQS message polling
pub async fn sqs_poller(
    queue_url: String,
    poll_interval: u64,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Initializing SQS poller for queue: {}", queue_url);

    let aws_region: Region = match env::var("AWS_REGION").map(|region| region.parse::<Region>()) {
        Ok(Ok(region)) => region,
        Ok(Err(_)) => {
            error!("Invalid AWS region format");
            return Err("Invalid AWS region".into());
        }
        Err(_) => {
            error!("AWS_REGION environment variable not set");
            return Err("AWS_REGION not set".into());
        }
    };

    let credentials_provider: EnvironmentProvider = EnvironmentProvider::default();
    let client: SqsClient =
        SqsClient::new_with(HttpClient::new()?, credentials_provider, aws_region);

    loop {
        tokio::select! {
            _ = async {
                info!("Polling SQS messages from {}", queue_url);
                match poll_sqs_messages(&client, &queue_url).await {
                    Some(messages) if !messages.is_empty() => {
                        info!("Received {} messages", messages.len());
                        for message in messages {
                            process_message(&message, &shell_command, &handler_name, timeout).await;
                            delete_message(&client, &queue_url, &message).await;
                        }
                    }
                    Some(_) => info!("No new messages received"),
                    None => info!("No messages received during this poll"),
                }

                info!("Waiting for next poll interval ({} seconds)", poll_interval);
                sleep(Duration::from_secs(poll_interval)).await;
            } => {}
            _ = shutdown_rx.recv() => {
                info!("SQS handler '{}' received shutdown signal", handler_name);
                break;
            }
        }
    }
    
    info!("SQS handler '{}' shutting down", handler_name);
    Ok(())
}

async fn poll_sqs_messages(client: &SqsClient, queue_url: &str) -> Option<Vec<Message>> {
    let request = ReceiveMessageRequest {
        queue_url: queue_url.to_string(),
        wait_time_seconds: Some(20),
        max_number_of_messages: Some(10),
        ..Default::default()
    };

    match client.receive_message(request).await {
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

async fn process_message(message: &Message, shell_command: &str, handler_name: &str, timeout: u64) {
    info!("Processing message: {:?}", message);

    let message_body = message.body.as_deref().unwrap_or("(empty)");
    let message_id = message.message_id.as_deref().unwrap_or("(no id)");
    let context = format!("Message ID: {}, Body: {}", message_id, message_body);

    execute_shell_command_with_context(shell_command, handler_name, &context, timeout).await;
}

async fn delete_message(client: &SqsClient, queue_url: &str, message: &Message) {
    if let Some(receipt_handle) = &message.receipt_handle {
        let delete_request = DeleteMessageRequest {
            queue_url: queue_url.to_string(),
            receipt_handle: receipt_handle.to_string(),
        };

        match client.delete_message(delete_request).await {
            Ok(_) => info!("Message deleted successfully."),
            Err(e) => error!("Error deleting message: {}", e),
        }
    } else {
        error!("No receipt handle found for message: {:?}", message);
    }
}
