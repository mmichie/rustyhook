use crate::command_executor::execute_shell_command;
use crate::config::HandlerConfig;
use chrono::Utc;
use cron::Schedule;
use log::{error, info};
use std::error::Error;
use std::str::FromStr;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

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
    let mut shutdown_rx = shutdown_tx.subscribe();

    info!("Initializing Cron handler '{}' with expression: {}", handler_name, cron_expression);

    Ok(tokio::spawn(async move {
        loop {
            let now = Utc::now();
            if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                let duration_until = next.signed_duration_since(now);
                if let Ok(std_duration) = duration_until.to_std() {
                    info!("Next cron execution for '{}' scheduled at: {}", handler_name, next);

                    tokio::select! {
                        _ = sleep(std_duration) => {
                            info!("Executing cron task '{}' at {:?}", handler_name, Utc::now());
                            execute_shell_command(&shell_command, &handler_name, timeout).await;
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
