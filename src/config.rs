use serde::{Deserialize, Serialize};
use std::fs;
use log::{info, error};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub handlers: Vec<HandlerConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandlerConfig {
    pub handler: String,
    pub event_type: EventType,
    pub options: HandlerOptions,
    pub action: ActionConfig,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventType {
    SQS,
    WebPolling,
    Cron,
    Webhook,
    Filesystem,
    Database,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandlerOptions {
    #[serde(flatten)]
    pub specific: SpecificOptions,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SpecificOptions {
    SQS {
        queue_url: String,
        poll_interval: u64,
    },
    WebPolling {
        url: String,
        poll_interval: u64,
    },
    Cron {
        schedule: String,
    },
    Webhook {
        port: u16,
        path: String,
    },
    Filesystem {
        path: String,
        event: String,
    },
    Database {
        db_connection: String,
        query: String,
        interval: u64,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActionConfig {
    pub shell: String,
}

// Function to load and parse the YAML configuration file
pub fn load_config(file_path: &str) -> Result<Config, Box<dyn std::error::Error>> {
    match fs::read_to_string(file_path) {
        Ok(config_str) => {
            info!("Config file read successfully.");
            match serde_yaml::from_str(&config_str) {
                Ok(config) => Ok(config),
                Err(e) => {
                    error!("Failed to parse config file: {}", e);
                    Err(Box::new(e))
                }
            }
        },
        Err(e) => {
            error!("Failed to read config file: {}", e);
            Err(Box::new(e))
        }
    }
}