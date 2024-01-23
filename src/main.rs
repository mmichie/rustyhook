mod config;
use clap::{Arg, Command};
use futures::future::join_all;
use log::{error, info, warn};
use tokio;
use tokio::task::JoinHandle;

mod handlers {
    pub mod cron_handler;
    pub mod filesystem_handler;
    pub mod sqs_handler;
    pub mod webhook_handler;
}

use crate::config::{load_config, EventType};
use crate::handlers::{cron_handler, filesystem_handler, sqs_handler, webhook_handler};

#[tokio::main]
async fn main() {
    env_logger::init();

    let config_path = get_config_path();
    let config = load_app_config(&config_path);
    let all_futures = initialize_handlers(&config);

    if all_futures.is_empty() {
        info!("No handlers were initialized.");
    } else {
        info!("All handlers initialized, starting execution.");
        let _ = join_all(all_futures).await;
        info!("All handlers have completed their execution.");
    }
}

fn get_config_path() -> String {
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

    matches
        .get_one::<String>("config")
        .expect("Failed to get config path")
        .clone()
}

fn load_app_config(config_path: &str) -> config::Config {
    info!("Loading configuration from: {}", config_path);
    load_config(config_path).unwrap_or_else(|e| {
        error!("Failed to load configuration: {:?}", e);
        std::process::exit(1);
    })
}

fn initialize_handlers(config: &config::Config) -> Vec<JoinHandle<()>> {
    let mut all_futures = Vec::new();
    for handler_config in &config.handlers {
        match handler_config.event_type {
            EventType::SQS => initialize_sqs_handler(&handler_config, &mut all_futures),
            EventType::Webhook => initialize_webhook_handler(&handler_config, &mut all_futures),
            EventType::Filesystem => {
                initialize_filesystem_handler(&handler_config, &mut all_futures)
            }
            EventType::WebPolling => warn!("WebPolling is not yet implemented"),
            EventType::Cron => {
                if let Ok(cron_future) = cron_handler::initialize_cron_handler(handler_config) {
                    all_futures.push(cron_future);
                } else {
                    error!(
                        "Failed to initialize Cron handler for: {}",
                        handler_config.name
                    );
                }
            }
            EventType::Database => warn!("Database is not yet implemented"),
        }
    }
    all_futures
}

fn initialize_sqs_handler(
    handler_config: &config::HandlerConfig,
    all_futures: &mut Vec<JoinHandle<()>>,
) {
    if let (Some(queue_url), Some(poll_interval)) = (
        handler_config.options.queue_url.clone(),
        handler_config.options.poll_interval,
    ) {
        info!("Initializing SQS handler for queue: {}", queue_url);
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

fn initialize_webhook_handler(
    handler_config: &config::HandlerConfig,
    all_futures: &mut Vec<JoinHandle<()>>,
) {
    if let (Some(port), Some(path)) = (
        handler_config.options.port,
        handler_config.options.path.clone(),
    ) {
        info!("Initializing Webhook handler on port: {}", port);
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

fn initialize_filesystem_handler(
    handler_config: &config::HandlerConfig,
    all_futures: &mut Vec<JoinHandle<()>>,
) {
    if let Some(path) = handler_config.options.path.clone() {
        info!("Initializing Filesystem handler for path: {}", path);
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
