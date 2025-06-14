use crate::config::HandlerConfig;
use chrono::Utc;
use cron::Schedule;
use log::{error, info};
use std::error::Error;
use std::process::Command;
use std::str::FromStr;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

// Function to initialize the cron handler
pub fn initialize_cron_handler(
    handler_config: &HandlerConfig,
) -> Result<JoinHandle<()>, Box<dyn Error>> {
    let cron_expression = handler_config
        .options
        .cron_expression
        .as_ref()
        .ok_or("Cron expression missing in config")?;

    let schedule = Schedule::from_str(cron_expression)?;
    let shell_command = handler_config.shell.clone();
    let handler_name = handler_config.name.clone();

    info!("Initializing Cron handler '{}' with expression: {}", handler_name, cron_expression);

    Ok(tokio::spawn(async move {
        loop {
            let now = Utc::now();
            if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                let duration_until = next.signed_duration_since(now);
                if let Ok(std_duration) = duration_until.to_std() {
                    info!("Next cron execution for '{}' scheduled at: {}", handler_name, next);
                    sleep(std_duration).await;
                    
                    info!("Executing cron task '{}' at {:?}", handler_name, Utc::now());
                    match Command::new("sh").arg("-c").arg(&shell_command).output() {
                        Ok(output) => {
                            if output.status.success() {
                                info!("Cron task '{}' executed successfully", handler_name);
                            } else {
                                error!("Cron task '{}' failed with status: {}", handler_name, output.status);
                                if !output.stderr.is_empty() {
                                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to execute cron task '{}': {:?}", handler_name, e);
                        }
                    }
                } else {
                    error!("Invalid duration calculated for next cron execution");
                    sleep(Duration::from_secs(60)).await;
                }
            } else {
                error!("No upcoming cron executions found");
                break;
            }
        }
    }))
}
