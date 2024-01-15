mod config;
mod handlers;
use clap::{Arg, Command};
use config::SpecificOptions;
use config::{load_config, EventType};
use handlers::{sqs_handler, webhook_handler};
use log::error;
use log::info;

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

    info!("Loading configuration from: {}", config_path);

    // Load the configuration file
    let config = match load_config(config_path) {
        Ok(cfg) => {
            info!("Configuration loaded successfully.");
            cfg
        }
        Err(e) => {
            error!("Failed to load configuration: {}", e);
            return;
        }
    };

    // Iterate over each handler in the configuration
    for handler_config in config.handlers {
        info!("Processing handler: {:?}", handler_config);
        match handler_config.event_type {
            EventType::SQS => {
                if let SpecificOptions::SQS {
                    queue_url,
                    poll_interval,
                } = handler_config.options.specific
                {
                    info!("Initializing SQS handler");
                    if let Err(e) = sqs_handler::sqs_poller(&queue_url, poll_interval).await {
                        error!("Error in SQS handler: {}", e);
                    }
                } else {
                    error!("Invalid options for SQS handler");
                }
            }
            EventType::WebPolling => {
                // Placeholder: Insert Web Polling handler logic here
                info!("Initializing Web Polling handler (Not implemented yet)");
            }
            EventType::Cron => {
                // Placeholder: Insert Cron handler logic here
                info!("Initializing Cron job handler (Not implemented yet)");
            }
            EventType::Webhook => {
                if let SpecificOptions::Webhook { port, path } = handler_config.options.specific {
                    info!(
                        "Initializing Webhook listener on port: {}, path: {}",
                        port, path
                    );
                    match webhook_handler::webhook_listener(port, path).await {
                        Ok(_) => info!("Webhook listener started successfully."),
                        Err(e) => error!("Failed to start webhook listener: {}", e),
                    }
                } else {
                    error!("Invalid options for Webhook handler");
                }
            }
            EventType::Filesystem => {
                // Placeholder: Insert Filesystem handler logic here
                info!("Initializing Filesystem handler (Not implemented yet)");
            }
            EventType::Database => {
                // Placeholder: Insert Database handler logic here
                info!("Initializing Database handler (Not implemented yet)");
            }
        }
    }
}
