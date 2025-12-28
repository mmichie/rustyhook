mod command_executor;
mod config;
mod event;
mod event_bus;
use clap::{Arg, Command};
use futures::future::join_all;
use log::{error, info, warn};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

mod handlers {
    pub mod cron_handler;
    pub mod filesystem_handler;
    pub mod sqs_handler;
    pub mod webhook_handler;
}

use crate::config::{load_config, validate_config, EventType};
use crate::event::Event;
use crate::event_bus::EventBus;
use crate::handlers::{cron_handler, filesystem_handler, sqs_handler, webhook_handler};

#[tokio::main]
async fn main() {
    env_logger::init();

    let cli_args = parse_cli_args();
    let config = load_app_config(&cli_args.config_path);

    // Validate configuration (checks for unique names, valid forward_to targets, no cycles,
    // handler-specific required options, and option value validity)
    if let Err(errors) = validate_config(&config) {
        for err in &errors {
            error!("Configuration error: {}", err);
        }
        if cli_args.validate_only {
            eprintln!(
                "Configuration validation failed with {} error(s)",
                errors.len()
            );
        }
        std::process::exit(1);
    }

    // In validate-only mode, print success and exit
    if cli_args.validate_only {
        println!(
            "Configuration is valid: {} handler(s) defined",
            config.handlers.len()
        );
        for handler in &config.handlers {
            println!("  - {} ({})", handler.name, handler.event_type);
        }
        std::process::exit(0);
    }

    info!("Configuration validated successfully");

    // Create shutdown channel
    let (shutdown_tx, _) = broadcast::channel(1);

    // Create EventBus for inter-handler communication
    let event_bus = Arc::new(EventBus::new());

    // Register all handlers with the EventBus
    let mut event_receivers: std::collections::HashMap<String, mpsc::UnboundedReceiver<Event>> =
        std::collections::HashMap::new();
    for handler_config in &config.handlers {
        match event_bus.register(&handler_config.name) {
            Ok(rx) => {
                event_receivers.insert(handler_config.name.clone(), rx);
            }
            Err(e) => {
                error!(
                    "Failed to register handler '{}' with EventBus: {}",
                    handler_config.name, e
                );
                std::process::exit(1);
            }
        }
    }
    info!(
        "Registered {} handlers with EventBus",
        event_bus.handler_count()
    );

    let all_futures = initialize_handlers(
        &config,
        shutdown_tx.clone(),
        event_bus.clone(),
        event_receivers,
    );

    if all_futures.is_empty() {
        info!("No handlers were initialized.");
    } else {
        info!("All handlers initialized, starting execution.");

        // Spawn handlers
        let handler_handle = tokio::spawn(async move { join_all(all_futures).await });

        // Wait for shutdown signal
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("Received shutdown signal, stopping handlers...");
                // Send shutdown signal to all handlers
                let _ = shutdown_tx.send(());

                // Wait for handlers to complete with timeout
                tokio::select! {
                    _ = handler_handle => {
                        info!("All handlers stopped gracefully.");
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(30)) => {
                        warn!("Timeout waiting for handlers to stop, forcing exit.");
                    }
                }
            }
            Err(err) => {
                error!("Error setting up signal handler: {}", err);
            }
        }
    }
}

struct CliArgs {
    config_path: String,
    validate_only: bool,
}

fn parse_cli_args() -> CliArgs {
    let matches: clap::ArgMatches = Command::new("rustyhook")
        .version(env!("CARGO_PKG_VERSION"))
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
        .arg(
            Arg::new("validate")
                .long("validate")
                .action(clap::ArgAction::SetTrue)
                .help("Validate configuration file and exit without starting handlers"),
        )
        .get_matches();

    CliArgs {
        config_path: matches
            .get_one::<String>("config")
            .expect("Failed to get config path")
            .clone(),
        validate_only: matches.get_flag("validate"),
    }
}

fn load_app_config(config_path: &str) -> config::Config {
    info!("Loading configuration from: {}", config_path);
    load_config(config_path).unwrap_or_else(|e| {
        error!("Failed to load configuration: {:?}", e);
        std::process::exit(1);
    })
}

fn initialize_handlers(
    config: &config::Config,
    shutdown_tx: broadcast::Sender<()>,
    event_bus: Arc<EventBus>,
    mut event_receivers: std::collections::HashMap<String, mpsc::UnboundedReceiver<Event>>,
) -> Vec<JoinHandle<()>> {
    let mut all_futures = Vec::new();
    for handler_config in &config.handlers {
        // Get the event receiver for this handler
        let event_rx = event_receivers
            .remove(&handler_config.name)
            .expect("Event receiver should exist for registered handler");

        match handler_config.event_type {
            EventType::Sqs => initialize_sqs_handler(
                handler_config,
                &mut all_futures,
                shutdown_tx.clone(),
                event_bus.clone(),
                event_rx,
            ),
            EventType::Webhook => initialize_webhook_handler(
                handler_config,
                &mut all_futures,
                shutdown_tx.clone(),
                event_bus.clone(),
                event_rx,
            ),
            EventType::Filesystem => initialize_filesystem_handler(
                handler_config,
                &mut all_futures,
                shutdown_tx.clone(),
                event_bus.clone(),
                event_rx,
            ),
            EventType::WebPolling => warn!("WebPolling is not yet implemented"),
            EventType::Cron => initialize_cron_handler(
                handler_config,
                &mut all_futures,
                shutdown_tx.clone(),
                event_bus.clone(),
                event_rx,
            ),
            EventType::Database => warn!("Database is not yet implemented"),
        }
    }
    all_futures
}

fn initialize_sqs_handler(
    handler_config: &config::HandlerConfig,
    all_futures: &mut Vec<JoinHandle<()>>,
    shutdown_tx: broadcast::Sender<()>,
    event_bus: Arc<EventBus>,
    event_rx: mpsc::UnboundedReceiver<Event>,
) {
    if let (Some(queue_url), Some(poll_interval)) = (
        handler_config.options.queue_url.clone(),
        handler_config.options.poll_interval,
    ) {
        info!("Initializing SQS handler for queue: {}", queue_url);
        let shell_command = handler_config.shell.clone();
        let handler_name = handler_config.name.clone();
        let timeout = handler_config.timeout;
        let retry_config = handler_config.retry.clone();
        let shell_config = handler_config.shell_program.clone();
        let working_dir = handler_config.working_dir.clone();
        let forward_to = handler_config.forward_to.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        let sqs_future: JoinHandle<()> = tokio::spawn(async move {
            sqs_handler::sqs_poller(
                queue_url,
                poll_interval,
                shell_command,
                handler_name,
                timeout,
                retry_config,
                shell_config,
                working_dir,
                shutdown_rx,
                event_bus,
                event_rx,
                forward_to,
            )
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
    shutdown_tx: broadcast::Sender<()>,
    event_bus: Arc<EventBus>,
    event_rx: mpsc::UnboundedReceiver<Event>,
) {
    if let (Some(port), Some(path)) = (
        handler_config.options.port,
        handler_config.options.path.clone(),
    ) {
        info!("Initializing Webhook handler on port: {}", port);
        let shell_command = handler_config.shell.clone();
        let handler_name = handler_config.name.clone();
        let timeout = handler_config.timeout;
        let retry_config = handler_config.retry.clone();
        let shell_config = handler_config.shell_program.clone();
        let working_dir = handler_config.working_dir.clone();
        let forward_to = handler_config.forward_to.clone();
        let auth_token = handler_config.options.auth_token.clone();
        let rate_limit = handler_config.options.rate_limit;
        let health_path = handler_config.options.health_path.clone();
        let hmac_secret = handler_config.options.hmac_secret.clone();
        let hmac_header = handler_config.options.hmac_header.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        let webhook_future = tokio::spawn(async move {
            webhook_handler::webhook_listener(
                port,
                path,
                shell_command,
                handler_name,
                timeout,
                retry_config,
                shell_config,
                working_dir,
                shutdown_rx,
                event_bus,
                event_rx,
                forward_to,
                auth_token,
                rate_limit,
                health_path,
                hmac_secret,
                hmac_header,
            )
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
    shutdown_tx: broadcast::Sender<()>,
    event_bus: Arc<EventBus>,
    event_rx: mpsc::UnboundedReceiver<Event>,
) {
    if let Some(path) = handler_config.options.path.clone() {
        let debounce_ms = handler_config.options.debounce_ms;
        let include_patterns = handler_config.options.include.clone();
        let exclude_patterns = handler_config.options.exclude.clone();
        info!(
            "Initializing Filesystem handler for path: {} (debounce: {}ms)",
            path, debounce_ms
        );
        let shell_command = handler_config.shell.clone();
        let handler_name = handler_config.name.clone();
        let timeout = handler_config.timeout;
        let retry_config = handler_config.retry.clone();
        let shell_config = handler_config.shell_program.clone();
        let working_dir = handler_config.working_dir.clone();
        let forward_to = handler_config.forward_to.clone();
        let shutdown_rx = shutdown_tx.subscribe();
        let filesystem_future = tokio::spawn(async move {
            filesystem_handler::filesystem_watcher(
                path,
                shell_command,
                handler_name,
                timeout,
                retry_config,
                shell_config,
                working_dir,
                shutdown_rx,
                event_bus,
                event_rx,
                forward_to,
                debounce_ms,
                include_patterns,
                exclude_patterns,
            )
            .await
            .unwrap_or_else(|e| {
                error!("Filesystem handler error: {:?}", e);
            });
        });
        all_futures.push(filesystem_future);
    }
}

fn initialize_cron_handler(
    handler_config: &config::HandlerConfig,
    all_futures: &mut Vec<JoinHandle<()>>,
    shutdown_tx: broadcast::Sender<()>,
    event_bus: Arc<EventBus>,
    event_rx: mpsc::UnboundedReceiver<Event>,
) {
    if let Some(cron_expression) = handler_config.options.cron_expression.clone() {
        info!(
            "Initializing Cron handler '{}' with expression: {}",
            handler_config.name, cron_expression
        );
        let shell_command = handler_config.shell.clone();
        let handler_name = handler_config.name.clone();
        let timeout = handler_config.timeout;
        let retry_config = handler_config.retry.clone();
        let shell_config = handler_config.shell_program.clone();
        let working_dir = handler_config.working_dir.clone();
        let forward_to = handler_config.forward_to.clone();
        let shutdown_rx = shutdown_tx.subscribe();

        match cron_handler::create_cron_handler(
            cron_expression,
            shell_command,
            handler_name.clone(),
            timeout,
            retry_config,
            shell_config,
            working_dir,
            shutdown_rx,
            event_bus,
            event_rx,
            forward_to,
        ) {
            Ok(cron_future) => {
                all_futures.push(cron_future);
            }
            Err(e) => {
                error!(
                    "Failed to initialize Cron handler '{}': {}",
                    handler_name, e
                );
            }
        }
    } else {
        error!(
            "Cron handler '{}' is missing cron_expression",
            handler_config.name
        );
    }
}
