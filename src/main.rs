use clap::{Arg, Command, ArgAction};
use rusoto_core::{Region, HttpClient, credential::EnvironmentProvider};
use rusoto_sqs::{Sqs, SqsClient, ReceiveMessageRequest, DeleteMessageRequest, ListQueuesRequest, SendMessageRequest};
use std::env;
use tokio;
use dotenv::dotenv;
use env_logger;
use log::{info, error};
use std::process::Command as ProcessCommand;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    dotenv().ok();  // This will load the .env file if it exists
    env_logger::init();
    info!("Loaded .env file.");
    info!("SQS_QUEUE_URL from .env: {:?}", env::var("SQS_QUEUE_URL"));

    let matches: clap::ArgMatches = Command::new("RustyHook")
        .version("0.0.1")
        .author("Matt Michie")
        .about("Automates Git updates and Docker-compose restarts based on SQS messages")
        .arg(Arg::new("directory")
             .short('d')
             .long("directory")
             .value_name("DIRECTORY")
             .help("Sets the directory for Git operations"))
        .arg(Arg::new("docker-restart")
             .short('r')
             .long("docker-restart")
             .action(ArgAction::SetTrue)
             .help("Enables Docker-compose restart"))
        .arg(Arg::new("poll-interval")
             .short('p')
             .long("poll-interval")
             .value_name("SECONDS")
             .help("Sets the poll interval in seconds"))
        .arg(Arg::new("list-queues")
             .short('l')
             .long("list-queues")
             .action(ArgAction::SetTrue)
             .help("Lists all SQS queues"))
        .arg(Arg::new("send-test-message")
             .short('t')
             .long("send-test-message")
             .action(ArgAction::SetTrue)
             .help("Sends a test message to the SQS queue"))
        .get_matches();

    let default_directory: String = env::current_dir().unwrap().to_str().unwrap().to_string();
    let directory: &String = matches.get_one::<String>("directory").unwrap_or(&default_directory);
    let docker_restart: bool = matches.get_flag("docker-restart");
    let poll_interval: u64 = matches.get_one::<u64>("poll-interval").copied().unwrap_or(30);

    if matches.get_flag("list-queues") {
        list_all_queues().await;
        return; // Exit after listing queues
    }

    if matches.get_flag("send-test-message") {
        send_test_message().await;
        return; // Exit after sending test message
    }

    if let Err(e) = validate_env_vars() {
        error!("Error: {}", e);
        return;
    }

    loop {
        let messages_received: bool = poll_sqs_messages().await;
        if messages_received {
            perform_git_update(directory);
            if docker_restart {
                optional_docker_compose_restart(directory);
            }
        }
        sleep(Duration::from_secs(poll_interval)).await;
    }
}

async fn list_all_queues() {
    let region: Region = env::var("AWS_REGION").expect("AWS_REGION not set")
        .parse::<Region>().expect("Invalid AWS region");
    let client: SqsClient = SqsClient::new(region);

    let list_queues_request: ListQueuesRequest = ListQueuesRequest::default(); // Default request to list queues

    match client.list_queues(list_queues_request).await {
        Ok(response) => {
            match response.queue_urls {
                Some(queue_urls) => {
                    info!("List of SQS Queues:");
                    for url in queue_urls {
                        info!("{}", url);
                    }
                },
                None => info!("No SQS queues found."),
            }
        },
        Err(error) => {
            error!("Error listing queues: {}", error);
        },
    }
}

fn validate_env_vars() -> Result<(), String> {
    let required_vars: [&str; 2] = ["AWS_REGION", "SQS_QUEUE_URL"];
    for &var in required_vars.iter() {
        if env::var(var).is_err() {
            return Err(format!("Environment variable {} not set", var));
        }
    }
    // Add more validation logic here if necessary
    Ok(())
}


async fn poll_sqs_messages() -> bool {
    // Retrieve AWS credentials and region from environment variables
    let aws_region: Region = env::var("AWS_REGION").expect("AWS_REGION not set").parse::<Region>().expect("Invalid AWS region");
    let queue_url: String = env::var("SQS_QUEUE_URL").expect("SQS_QUEUE_URL not set");

    // Create a custom credential provider
    let credentials_provider: EnvironmentProvider = EnvironmentProvider::default();

    // Create a custom client configuration
    let client: SqsClient = SqsClient::new_with(HttpClient::new().expect("Failed to create HTTP client"), credentials_provider, aws_region);

    let request: ReceiveMessageRequest = ReceiveMessageRequest {
        queue_url: queue_url.clone(),
        wait_time_seconds: Some(20), // Enable long polling for 20 seconds
        max_number_of_messages: Some(10),
        ..Default::default()
    };

    // Retrieve messages
    match client.receive_message(request).await {
        Ok(response) => {
            if let Some(messages) = response.messages {
                if messages.is_empty() {
                    info!("No messages received in this poll.");
                    false
                } else {
                    info!("Received {} message(s).", messages.len());
                    for message in messages {
                        // Process each message
                        info!("Received message: {:?}", message);

                        // TODO: Add more processing logic here

                        // Delete the message from the queue to prevent reprocessing
                        if let Some(receipt_handle) = &message.receipt_handle {
                            let delete_request = DeleteMessageRequest {
                                queue_url: queue_url.clone(),
                                receipt_handle: receipt_handle.to_string(),
                            };

                            match client.delete_message(delete_request).await {
                                Ok(_) => info!("Message deleted successfully."),
                                Err(e) => error!("Error deleting message: {}", e),
                            }
                        }
                    }
                    true
                }
            } else {
                info!("No messages received in this poll.");
                false
            }
        }
        Err(error) => {
            error!("Error receiving messages: {}", error);
            false
        }
    }
}

async fn send_test_message() {
    let aws_region = env::var("AWS_REGION").expect("AWS_REGION not set")
        .parse::<Region>().expect("Invalid AWS region");
    let queue_url = env::var("SQS_QUEUE_URL").expect("SQS_QUEUE_URL not set");
    let client = SqsClient::new(aws_region);

    let send_message_request = SendMessageRequest {
        queue_url: queue_url.clone(),
        message_body: "Test message from RustyHook".to_string(),
        ..Default::default()
    };

    match client.send_message(send_message_request).await {
        Ok(_) => info!("Test message sent successfully."),
        Err(e) => error!("Error sending test message: {}", e),
    }
}

fn perform_git_update(directory: &str) {
    // Change to the specified directory
    std::env::set_current_dir(directory).expect("Failed to change directory");

    // Execute git pull using std::process::Command
    let output: std::process::Output = std::process::Command::new("git")
                         .args(["pull", "origin", "master"])
                         .output()
                         .expect("Failed to execute git pull");

    if output.status.success() {
        info!("Repository updated successfully.");
    } else {
        error!("Failed to update repository: {}", String::from_utf8_lossy(&output.stderr));
    }
}

fn optional_docker_compose_restart(directory: &str) {
    if let Err(e) = ProcessCommand::new("docker-compose")
        .args(&["-f", format!("{}/docker-compose.yml", directory).as_str(), "restart"])
        .status() {
            error!("Failed to restart Docker-compose: {}", e);
    } else {
        info!("Docker-compose restarted in directory: {}", directory);
    }
}