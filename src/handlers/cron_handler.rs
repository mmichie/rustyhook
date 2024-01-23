use crate::config::HandlerConfig;
use chrono::Utc;
use cron::Schedule;
use log::info;
use std::error::Error;
use std::str::FromStr;
use tokio::task::JoinHandle;
use tokio::time::sleep;

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
    let now = Utc::now();

    Ok(tokio::spawn(async move {
        for datetime in schedule.upcoming(chrono::Utc).take(10) {
            let duration_until = datetime.signed_duration_since(now);
            sleep(duration_until.to_std().unwrap_or_default()).await;
            info!("Executing scheduled task at {:?}", Utc::now());
            // Add task execution logic here
        }
    }))
}
