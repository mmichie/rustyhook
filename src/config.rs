// src/config.rs

use serde::{Deserialize, Serialize};

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
pub fn load_config(file_path: &str) -> Result<Config, serde_yaml::Error> {
    let config_str = std::fs::read_to_string(file_path).expect("Failed to read config file");
    serde_yaml::from_str(&config_str)
}
