mod config;
use clap::{Arg, Command};
use futures::future::join_all;
use log::{error, info, warn};
use tokio;
use tokio::task::JoinHandle;

mod handlers {
    pub mod filesystem_handler;
    pub mod sqs_handler;
    pub mod webhook_handler;
}

use crate::config::{load_config, EventType};
use crate::handlers::{filesystem_handler, sqs_handler, webhook_handler};

#[tokio::main]
async fn main() {
    env_logger::init();
    let matches: clap::ArgMatches = Command::new("Arcnar")
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

    let config_path: &String = matches
        .get_one::<String>("config")
        .expect("Failed to get config path");

    info!("Loading configuration from: {}", config_path);

    let config: config::Config = load_config(config_path).expect("Failed to load configuration");

    let mut all_futures: Vec<JoinHandle<()>> = Vec::new();

    for handler_config in config.handlers {
        match handler_config.event_type {
            EventType::SQS => {
                if let (Some(queue_url), Some(poll_interval)) = (
                    handler_config.options.queue_url,
                    handler_config.options.poll_interval,
                ) {
                    let sqs_future: JoinHandle<()> = tokio::spawn(async move {
                        sqs_handler::sqs_poller(queue_url, poll_interval)
                            .await
                            .unwrap_or_else(|e| {
                                error!("SQS handler error: {:?}", e);
                            });
                    });
                    all_futures.push(sqs_future);
                }
            }
            EventType::Webhook => {
                if let (Some(port), Some(path)) =
                    (handler_config.options.port, handler_config.options.path)
                {
                    let webhook_future = tokio::spawn(async move {
                        webhook_handler::webhook_listener(port, path)
                            .await
                            .unwrap_or_else(|e| {
                                error!("Webhook handler error: {:?}", e);
                            });
                    });
                    all_futures.push(webhook_future);
                }
            }
            EventType::Filesystem => {
                if let Some(path) = handler_config.options.path {
                    let filesystem_future = tokio::spawn(async move {
                        filesystem_handler::filesystem_watcher(path)
                            .await
                            .unwrap_or_else(|e| {
                                error!("Filesystem handler error: {:?}", e);
                            });
                    });
                    all_futures.push(filesystem_future);
                }
            }
            EventType::WebPolling => {
                warn!("WebPolling is not yet implemented");
            }
            EventType::Cron => {
                warn!("Cron is not yet implemented");
            }
            EventType::Database => {
                warn!("Database is not yet implemented");
            }
        }
    }

    if !all_futures.is_empty() {
        let _ = join_all(all_futures).await;
    }

    info!("All handlers have completed their execution.");
}
