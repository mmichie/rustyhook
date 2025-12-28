use crate::command_executor::execute_shell_command_with_retry;
use crate::config::{RetryConfig, ShellConfig};
use crate::event::Event;
use crate::event_bus::EventBus;
use chrono::Utc;
use cron::Schedule;
use log::{debug, error, info, warn};
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

/// Validate a cron expression without starting a handler
#[cfg(test)]
pub fn validate_cron_expression(expression: &str) -> Result<(), Box<dyn Error>> {
    Schedule::from_str(expression)?;
    Ok(())
}

// Function to create the cron handler
#[allow(clippy::too_many_arguments)]
pub fn create_cron_handler(
    cron_expression: String,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    retry_config: RetryConfig,
    shell_config: ShellConfig,
    working_dir: Option<String>,
    mut shutdown_rx: broadcast::Receiver<()>,
    event_bus: Arc<EventBus>,
    mut event_rx: mpsc::UnboundedReceiver<Event>,
    forward_to: Vec<String>,
) -> Result<JoinHandle<()>, Box<dyn Error + Send + Sync>> {
    let schedule = Schedule::from_str(&cron_expression)?;

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
                            execute_shell_command_with_retry(&shell_command, &handler_name, timeout, &retry_config, &shell_config, working_dir.as_deref()).await;

                            // Forward event if configured
                            if !forward_to.is_empty() {
                                let event = Event::from_cron(&handler_name, &cron_expression, next);

                                for target in &forward_to {
                                    if let Err(e) = event_bus.send(target, event.clone()) {
                                        warn!("Failed to forward event to '{}': {}", target, e);
                                    } else {
                                        debug!("Forwarded cron event to '{}'", target);
                                    }
                                }
                            }
                        }
                        Some(forwarded_event) = event_rx.recv() => {
                            // Handle forwarded events from other handlers
                            info!(
                                "Cron handler '{}' received forwarded event from '{}'",
                                handler_name, forwarded_event.source_handler
                            );
                            execute_shell_command_with_retry(&shell_command, &handler_name, timeout, &retry_config, &shell_config, working_dir.as_deref()).await;

                            // Forward to next handlers if configured
                            if !forward_to.is_empty() {
                                for target in &forward_to {
                                    if let Err(e) = event_bus.send(target, forwarded_event.clone()) {
                                        warn!("Failed to forward event to '{}': {}", target, e);
                                    }
                                }
                            }
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

    /// Helper to create test dependencies
    fn create_test_deps() -> (Arc<EventBus>, mpsc::UnboundedReceiver<Event>) {
        let event_bus = Arc::new(EventBus::new());
        let (tx, rx) = mpsc::unbounded_channel();
        // We don't register with the bus since we're just testing the handler
        drop(tx);
        (event_bus, rx)
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
    async fn test_create_cron_handler_invalid_expression() {
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let result = create_cron_handler(
            "invalid cron".to_string(),
            "echo 'test'".to_string(),
            "test-cron".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_cron_handler_valid_expression() {
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let result = create_cron_handler(
            "0 * * * * *".to_string(),
            "echo 'test'".to_string(),
            "test-cron".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
        );
        assert!(result.is_ok());

        // Clean up the spawned task
        result.unwrap().abort();
    }

    #[tokio::test]
    async fn test_cron_handler_shutdown() {
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handle = create_cron_handler(
            "0 * * * * *".to_string(),
            "echo 'test'".to_string(),
            "test-cron".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
        )
        .expect("Failed to create cron handler");

        // Send shutdown signal
        let _ = shutdown_tx.send(());

        // Wait for handler to finish (should complete quickly after shutdown)
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "Handler should shut down within timeout");
    }

    // ============== Integration Tests ==============

    use tempfile::TempDir;
    use tokio::fs;

    async fn wait_for_marker(path: &std::path::Path, timeout_secs: u64) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        while tokio::time::Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    #[tokio::test]
    async fn test_cron_executes_on_schedule() {
        let marker_dir = TempDir::new().expect("Failed to create marker dir");
        let marker_path = marker_dir.path().join("cron_executed.marker");

        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let shell_command = format!("touch '{}'", marker_path.to_string_lossy());

        // "* * * * * *" fires every second
        let handle = create_cron_handler(
            "* * * * * *".to_string(),
            shell_command,
            "test-cron-exec".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
        )
        .expect("Failed to create cron handler");

        // Wait up to 3 seconds for execution (should fire within ~1s)
        let executed = wait_for_marker(&marker_path, 3).await;

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(executed, "Cron should have executed within 3 seconds");
    }

    #[tokio::test]
    async fn test_cron_executes_multiple_times() {
        let marker_dir = TempDir::new().expect("Failed to create marker dir");
        let counter_path = marker_dir.path().join("cron_counter");

        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        // Append to counter file on each execution
        let shell_command = format!("echo 'tick' >> '{}'", counter_path.to_string_lossy());

        let handle = create_cron_handler(
            "* * * * * *".to_string(), // Every second
            shell_command,
            "test-cron-multi".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
        )
        .expect("Failed to create cron handler");

        // Wait ~3 seconds for multiple executions
        tokio::time::sleep(Duration::from_secs(3)).await;

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

        let count = if counter_path.exists() {
            fs::read_to_string(&counter_path)
                .await
                .map(|s| s.lines().count())
                .unwrap_or(0)
        } else {
            0
        };

        // Should have at least 2 executions in 3 seconds
        assert!(count >= 2, "Expected at least 2 executions, got {}", count);
    }

    #[tokio::test]
    async fn test_cron_handles_forwarded_events() {
        let marker_dir = TempDir::new().expect("Failed to create marker dir");
        let marker_path = marker_dir.path().join("forwarded.marker");

        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let event_bus = Arc::new(EventBus::new());
        let event_rx = event_bus.register("test-cron-forward").unwrap();

        let shell_command = format!("touch '{}'", marker_path.to_string_lossy());

        // Use a far-future cron schedule so only forwarded events trigger
        let handle = create_cron_handler(
            "0 0 0 1 1 * 2099".to_string(), // January 1, 2099 at midnight
            shell_command,
            "test-cron-forward".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus.clone(),
            event_rx,
            Vec::new(),
        )
        .expect("Failed to create cron handler");

        // Give handler time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send forwarded event
        let event = Event::from_filesystem("source-handler", "/tmp/test.txt", "create");
        event_bus
            .send("test-cron-forward", event)
            .expect("Failed to send event");

        // Wait for execution
        let executed = wait_for_marker(&marker_path, 2).await;

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert!(executed, "Forwarded event should trigger cron command");
    }
}
