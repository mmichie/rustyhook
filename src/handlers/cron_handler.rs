use crate::command_executor::execute_shell_command_with_retry;
use crate::config::HandlerConfig;
use chrono::Utc;
use cron::Schedule;
use log::{error, info};
use std::error::Error;
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

/// Validate a cron expression without starting a handler
#[cfg(test)]
pub fn validate_cron_expression(expression: &str) -> Result<(), Box<dyn Error>> {
    Schedule::from_str(expression)?;
    Ok(())
}

// Function to initialize the cron handler
pub fn initialize_cron_handler(
    handler_config: &HandlerConfig,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<JoinHandle<()>, Box<dyn Error>> {
    let cron_expression = handler_config
        .options
        .cron_expression
        .as_ref()
        .ok_or("Cron expression missing in config")?;

    let schedule = Schedule::from_str(cron_expression)?;
    let shell_command = handler_config.shell.clone();
    let handler_name = handler_config.name.clone();
    let timeout = handler_config.timeout;
    let retry_config = handler_config.retry.clone();
    let mut shutdown_rx = shutdown_tx.subscribe();

    info!(
        "Initializing Cron handler '{}' with expression: {}",
        handler_name, cron_expression
    );

    Ok(tokio::spawn(async move {
        loop {
            let now = Utc::now();
            if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                let duration_until = next.signed_duration_since(now);
                if let Ok(std_duration) = duration_until.to_std() {
                    info!(
                        "Next cron execution for '{}' scheduled at: {}",
                        handler_name, next
                    );

                    tokio::select! {
                        _ = sleep(std_duration) => {
                            info!("Executing cron task '{}' at {:?}", handler_name, Utc::now());
                            execute_shell_command_with_retry(&shell_command, &handler_name, timeout, &retry_config).await;
                        }
                        _ = shutdown_rx.recv() => {
                            info!("Cron handler '{}' received shutdown signal", handler_name);
                            break;
                        }
                    }
                } else {
                    error!("Invalid duration calculated for next cron execution");

                    tokio::select! {
                        _ = sleep(Duration::from_secs(60)) => {}
                        _ = shutdown_rx.recv() => {
                            info!("Cron handler '{}' received shutdown signal", handler_name);
                            break;
                        }
                    }
                }
            } else {
                error!("No upcoming cron executions found");
                break;
            }
        }

        info!("Cron handler '{}' shutting down", handler_name);
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EventType, Options, RetryConfig};

    fn create_handler_config(cron_expression: Option<String>) -> HandlerConfig {
        HandlerConfig {
            event_type: EventType::Cron,
            name: "test-cron".to_string(),
            options: Options {
                options_type: "cron".to_string(),
                queue_url: None,
                poll_interval: None,
                port: None,
                path: None,
                aws_region: None,
                cron_expression,
            },
            shell: "echo 'test'".to_string(),
            timeout: 30,
            retry: RetryConfig::default(),
        }
    }

    #[test]
    fn test_validate_cron_expression_valid() {
        // Standard 6-field cron (seconds minutes hours day month weekday)
        assert!(validate_cron_expression("0 0 * * * *").is_ok());
        assert!(validate_cron_expression("*/5 * * * * *").is_ok());
        assert!(validate_cron_expression("0 30 9 * * Mon-Fri").is_ok());
    }

    #[test]
    fn test_validate_cron_expression_invalid() {
        // Invalid expressions
        assert!(validate_cron_expression("invalid").is_err());
        assert!(validate_cron_expression("60 * * * * *").is_err()); // 60 seconds invalid
        assert!(validate_cron_expression("").is_err());
    }

    #[tokio::test]
    async fn test_initialize_cron_handler_missing_expression() {
        let config = create_handler_config(None);
        let (shutdown_tx, _) = broadcast::channel(1);

        let result = initialize_cron_handler(&config, shutdown_tx);
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Cron expression missing"));
    }

    #[tokio::test]
    async fn test_initialize_cron_handler_invalid_expression() {
        let config = create_handler_config(Some("invalid cron".to_string()));
        let (shutdown_tx, _) = broadcast::channel(1);

        let result = initialize_cron_handler(&config, shutdown_tx);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_initialize_cron_handler_valid_expression() {
        let config = create_handler_config(Some("0 * * * * *".to_string()));
        let (shutdown_tx, _) = broadcast::channel(1);

        let result = initialize_cron_handler(&config, shutdown_tx);
        assert!(result.is_ok());

        // Clean up the spawned task
        result.unwrap().abort();
    }

    #[tokio::test]
    async fn test_cron_handler_shutdown() {
        let config = create_handler_config(Some("0 * * * * *".to_string()));
        let (shutdown_tx, _) = broadcast::channel(1);

        let handle = initialize_cron_handler(&config, shutdown_tx.clone())
            .expect("Failed to initialize cron handler");

        // Send shutdown signal
        let _ = shutdown_tx.send(());

        // Wait for handler to finish (should complete quickly after shutdown)
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "Handler should shut down within timeout");
    }
}
