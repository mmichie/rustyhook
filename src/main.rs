mod config;
use clap::{Arg, ArgAction, Command};
use config::{load_config, EventType, HandlerConfig};
use log::{error, info};

#[tokio::main]
async fn main() {
    let matches = Command::new("RustyHook")
        .version("0.0.1")
        .author("Matt Michie")
        .about("Automates Git updates and Docker-compose restarts based on SQS messages")
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
                // Initialize and run the SQS handler
                // Use handler_config.options and handler_config.action
            }
            EventType::WebPolling => {
                // Initialize and run the Web Polling handler
            }
            EventType::Cron => {
                // Initialize and run the Cron job handler
            }
            EventType::Webhook => {
                // Initialize and run the Webhook listener
            }
            // Add other event types here
            _ => {
                // Handle other types or unknown types
                error!("Unknown handler type: {:?}", handler_config.event_type);
            }
        }
    }

    // Add any other necessary logic for your application
}
