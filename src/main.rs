mod config;
use clap::{Arg, Command};
use log::{error, info, warn};
use tokio;

mod handlers {
    pub mod sqs_handler;
    pub mod webhook_handler;
}

use crate::config::{load_config, EventType};
use crate::handlers::{sqs_handler, webhook_handler};

#[tokio::main]
async fn main() {
    env_logger::init();
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
        .expect("Failed to get config path");

    info!("Loading configuration from: {}", config_path);

    let config = load_config(config_path).expect("Failed to load configuration");

    let mut handler_futures = Vec::new();

    for handler_config in config.handlers {
        match handler_config.event_type {
            EventType::SQS => {
                if let Some(queue_url) = handler_config.options.queue_url {
                    if let Some(poll_interval) = handler_config.options.poll_interval {
                        info!("Initializing SQS handler for queue: {}", queue_url);
                        let sqs_future = tokio::spawn(async move {
                            sqs_handler::sqs_poller(queue_url, poll_interval).await
                        });
                        handler_futures.push(sqs_future);
                    }
                }
            }
            EventType::Webhook => {
                if let Some(port) = handler_config.options.port {
                    if let Some(path) = handler_config.options.path {
                        info!("Initializing Webhook handler on port {} with path {}", port, path);
                        let webhook_future = tokio::spawn(async move {
                            webhook_handler::webhook_listener(port, path).await
                        });
                        handler_futures.push(webhook_future);
                    }
                }
            }
            EventType::WebPolling => {
                warn!("WebPolling is not yet implemented");
            }
            EventType::Cron => {
                warn!("Cron is not yet implemented");
            }
            EventType::Filesystem => {
                warn!("Filesystem is not yet implemented");
            }
            EventType::Database => {
                warn!("Database is not yet implemented");
            }
        }
    }

    for future in handler_futures {
        if let Err(e) = future.await {
            error!("An error occurred in a handler: {}", e);
        }
    }
}
