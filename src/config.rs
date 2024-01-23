use serde::{Deserialize, Serialize};
use std::{error::Error, fs};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub handlers: Vec<HandlerConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandlerConfig {
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub name: String,     // Renamed from 'handler' to 'name'
    pub options: Options, // Changed to a struct to match the YAML format
    pub shell: String,    // Directly included as a field
}

#[derive(Debug, Serialize, Deserialize)]
pub enum EventType {
    SQS,
    WebPolling,
    Cron,
    Webhook,
    Filesystem,
    Database,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Options {
    #[serde(rename = "type")]
    pub options_type: String, // To handle the nested 'type' field in options
    pub queue_url: Option<String>,
    pub poll_interval: Option<u64>,
    pub port: Option<u16>,
    pub path: Option<String>,
    pub aws_region: Option<String>,
    pub cron_expression: Option<String>,
}

// Function to load and parse the YAML configuration file
pub fn load_config(file_path: &str) -> Result<Config, Box<dyn Error>> {
    let config_str = fs::read_to_string(file_path)?;
    let config: Config = serde_yaml::from_str(&config_str)?;
    Ok(config)
}
