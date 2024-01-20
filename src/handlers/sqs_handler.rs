// src/handlers/sqs_handler.rs

use log::{error, info};
use rusoto_core::credential::EnvironmentProvider;
use rusoto_core::{HttpClient, Region};
use rusoto_sqs::{DeleteMessageRequest, Message, ReceiveMessageRequest, Sqs, SqsClient};
use std::env;
use tokio::time::{sleep, Duration};

// Function to start the SQS message polling
pub async fn sqs_poller(
    queue_url: String,
    poll_interval: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Initializing SQS poller for queue: {}", queue_url);

    let aws_region = match env::var("AWS_REGION").map(|region| region.parse::<Region>()) {
        Ok(Ok(region)) => region,
        Ok(Err(_)) => return Err("Invalid AWS region".into()),
        Err(_) => return Err("AWS_REGION not set".into()),
    };

    let credentials_provider = EnvironmentProvider::default();
    let client = SqsClient::new_with(HttpClient::new()?, credentials_provider, aws_region);

    loop {
        info!("Polling SQS messages...");
        match poll_sqs_messages(&client, &queue_url).await {
            Some(messages) if !messages.is_empty() => {
                info!("Received {} messages", messages.len());
                for message in messages {
                    process_message(&message).await;
                    delete_message(&client, &queue_url, &message).await;
                }
            }
            Some(_) => info!("No new messages received"),
            None => info!("No messages received"),
        }

        sleep(Duration::from_secs(poll_interval)).await;
    }
}

async fn poll_sqs_messages(client: &SqsClient, queue_url: &str) -> Option<Vec<Message>> {
    let request = ReceiveMessageRequest {
        queue_url: queue_url.to_string(),
        wait_time_seconds: Some(20),
        max_number_of_messages: Some(10),
        ..Default::default()
    };

    match client.receive_message(request).await {
        Ok(response) => response.messages,
        Err(e) => {
            error!("Error polling messages: {}", e);
            None
        }
    }
}

async fn process_message(message: &Message) {
    info!("Processing message: {:?}", message);
    // Add message processing logic here
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
    }
}
