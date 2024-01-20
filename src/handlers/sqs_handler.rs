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
    // Retrieve AWS credentials and region from environment variables
    let aws_region = env::var("AWS_REGION")
        .expect("AWS_REGION not set")
        .parse::<Region>()
        .expect("Invalid AWS region");

    let credentials_provider = EnvironmentProvider::default();
    let client = SqsClient::new_with(
        HttpClient::new().expect("Failed to create HTTP client"),
        credentials_provider,
        aws_region,
    );
    loop {
        // Poll for messages
        if let Some(messages) = poll_sqs_messages(&client, &queue_url).await {
            for message in messages {
                // Process each message
                process_message(&message).await;

                // Delete message from the queue to prevent reprocessing
                delete_message(&client, &queue_url, &message).await;
            }
        }

        // Wait for the next poll interval
        sleep(Duration::from_secs(poll_interval)).await;
    }
}

// Function to poll SQS messages
async fn poll_sqs_messages(client: &SqsClient, queue_url: &str) -> Option<Vec<Message>> {
    let request = ReceiveMessageRequest {
        queue_url: queue_url.to_string(),
        wait_time_seconds: Some(20), // Enable long polling for 20 seconds
        max_number_of_messages: Some(10),
        ..Default::default()
    };

    match client.receive_message(request).await {
        Ok(response) => response.messages,
        Err(e) => {
            error!("Error receiving messages: {}", e);
            None
        }
    }
}

// Function to process a single SQS message
async fn process_message(message: &Message) {
    // TODO: Add your message processing logic here
    info!("Processing message: {:?}", message);
}

// Function to delete a message from the SQS queue
async fn delete_message(client: &SqsClient, queue_url: &str, message: &Message) {
    if let Some(receipt_handle) = &message.receipt_handle {
        let delete_request = DeleteMessageRequest {
            queue_url: queue_url.to_string(),
            receipt_handle: receipt_handle.to_string(),
        };

        if let Err(e) = client.delete_message(delete_request).await {
            error!("Error deleting message: {}", e);
        }
    }
}
