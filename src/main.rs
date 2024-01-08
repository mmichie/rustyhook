use clap::{Arg, Command};
use rusoto_core::{Region, HttpClient, credential::EnvironmentProvider};
use rusoto_sqs::{Sqs, SqsClient, ReceiveMessageRequest, DeleteMessageRequest};
use std::env;
use tokio;
use env_logger;
use log::{info, error};
use std::process::Command as ProcessCommand;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    env_logger::init();

    let matches = Command::new("RustyHook")
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
             .help("Enables Docker-compose restart"))
        .arg(Arg::new("poll-interval")
             .short('p')
             .long("poll-interval")
             .value_name("SECONDS")
             .help("Sets the poll interval in seconds"))
        .get_matches();

    let default_directory = env::current_dir().unwrap().to_str().unwrap().to_string();
    let directory = matches.get_one::<String>("directory").unwrap_or(&default_directory);
    let docker_restart = matches.contains_id("docker-restart");
    let poll_interval = matches.get_one::<u64>("poll-interval").copied().unwrap_or(30);

    if let Err(e) = validate_env_vars() {
        error!("Error: {}", e);
        return;
    }

    loop {
        poll_sqs_messages().await;
        perform_git_update(directory);
        if docker_restart {
            optional_docker_compose_restart(directory);
        }
        sleep(Duration::from_secs(poll_interval)).await;
    }
}

fn validate_env_vars() -> Result<(), String> {
    let required_vars = ["AWS_REGION", "SQS_QUEUE_URL"];
    for &var in required_vars.iter() {
        if env::var(var).is_err() {
            return Err(format!("Environment variable {} not set", var));
        }
    }
    // Add more validation logic here if necessary
    Ok(())
}


async fn poll_sqs_messages() {
    // Retrieve AWS credentials and region from environment variables
    let aws_region = env::var("AWS_REGION").expect("AWS_REGION not set").parse::<Region>().expect("Invalid AWS region");
    let queue_url = env::var("SQS_QUEUE_URL").expect("SQS_QUEUE_URL not set");

    // Create a custom credential provider
    let credentials_provider = EnvironmentProvider::default();

    // Create a custom client configuration
    let client = SqsClient::new_with(HttpClient::new().expect("Failed to create HTTP client"), credentials_provider, aws_region);

    let request = ReceiveMessageRequest {
        queue_url: queue_url.clone(),
        wait_time_seconds: Some(20), // Enable long polling for 20 seconds
        ..Default::default()
    };

    // Retrieve messages
    match client.receive_message(request).await {
        Ok(response) => {
            if let Some(messages) = response.messages {
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
            }
        }
        Err(error) => {
            error!("Error receiving messages: {}", error);
        }
    }
}

fn perform_git_update(directory: &str) {
    // Change to the specified directory
    std::env::set_current_dir(directory).expect("Failed to change directory");

    // Execute git pull using std::process::Command
    let output = std::process::Command::new("git")
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