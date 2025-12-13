use serde::{Deserialize, Serialize};
use std::{error::Error, fs};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub handlers: Vec<HandlerConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HandlerConfig {
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub name: String,     // Renamed from 'handler' to 'name'
    pub options: Options, // Changed to a struct to match the YAML format
    pub shell: String,    // Directly included as a field
    #[serde(default = "default_timeout")]
    pub timeout: u64, // Command timeout in seconds (default: 300)
    #[serde(default)]
    pub retry: RetryConfig, // Retry configuration (optional)
}

fn default_timeout() -> u64 {
    300 // 5 minutes default timeout
}

/// Configuration for retry behavior with exponential backoff
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retry)
    #[serde(default)]
    pub max_retries: u32,
    /// Initial delay between retries in milliseconds (default: 1000ms)
    #[serde(default = "default_retry_delay_ms")]
    pub delay_ms: u64,
    /// Multiplier for exponential backoff (default: 2.0)
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
    /// Maximum delay between retries in milliseconds (default: 60000ms / 1 minute)
    #[serde(default = "default_max_delay_ms")]
    pub max_delay_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 0,
            delay_ms: default_retry_delay_ms(),
            backoff_multiplier: default_backoff_multiplier(),
            max_delay_ms: default_max_delay_ms(),
        }
    }
}

fn default_retry_delay_ms() -> u64 {
    1000 // 1 second
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

fn default_max_delay_ms() -> u64 {
    60000 // 1 minute
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum EventType {
    #[serde(rename = "SQS")]
    Sqs,
    WebPolling,
    Cron,
    Webhook,
    Filesystem,
    Database,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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
