mod config;
mod handlers;
use clap::{Arg, Command};
use config::{load_config, EventType};
use config::SpecificOptions;
use handlers::{sqs_handler, webhook_handler};
use log::error;

#[tokio::main]
async fn main() {
    let matches = Command::new("Arcnar")
        .version("0.0.1")
        .author("Matt Michie")
        .about("Event-driven automation tool")
        .arg(
            Arg::new("config")
                .short('c')
                .long("config")
                .value_name("CONFIG_FILE")
                .default_value("config.yml")
                .help("Sets the path to the configuration file"),
        )
        .get_matches();

    let config_path = matches
        .get_one::<String>("config")
        .expect("Config file path must be provided");

    // Load the configuration file
    let config = match load_config(config_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return;
        }
    };

    // Iterate over each handler in the configuration
    for handler_config in config.handlers {
        match handler_config.event_type {
            EventType::SQS => {
                if let SpecificOptions::SQS {
                    queue_url,
                    poll_interval,
                } = handler_config.options.specific
                {
                    if let Err(e) = sqs_handler::sqs_poller(&queue_url, poll_interval).await {
                        error!("Error in SQS handler: {}", e);
                    }
                } else {
                    error!("Invalid options for SQS handler");
                }
            }
            EventType::WebPolling => {
                // Initialize and run the Web Polling handler
                // Placeholder: Insert Web Polling handler logic here
            }
            EventType::Cron => {
                // Initialize and run the Cron job handler
                // Placeholder: Insert Cron handler logic here
            }
            EventType::Webhook => {
                // Extract options for the Webhook handler
                if let config::SpecificOptions::Webhook { port, path } =
                    handler_config.options.specific
                {
                    // Start the webhook listener with the specified port and path
                    webhook_handler::webhook_listener(port, path).await;
                } else {
                    error!("Invalid options for Webhook handler");
                }
            }
            EventType::Filesystem => {
                // Initialize and run the Filesystem handler
                // Placeholder: Insert Filesystem handler logic here
            }
            EventType::Database => {
                // Initialize and run the Database handler
                // Placeholder: Insert Database handler logic here
            }
            _ => {
                // Handle other types or unknown types
                error!("Unknown handler type: {:?}", handler_config.event_type);
            }
        }
    }
}
