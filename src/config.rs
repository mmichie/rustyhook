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

// Function to parse configuration from a string (useful for testing)
#[cfg(test)]
pub fn parse_config(config_str: &str) -> Result<Config, Box<dyn Error>> {
    let config: Config = serde_yaml::from_str(config_str)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook_handler() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test-webhook
    options:
      type: http
      port: 8080
      path: /webhook
    shell: echo "webhook received"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        assert_eq!(config.handlers.len(), 1);
        let handler = &config.handlers[0];
        assert!(matches!(handler.event_type, EventType::Webhook));
        assert_eq!(handler.name, "test-webhook");
        assert_eq!(handler.options.port, Some(8080));
        assert_eq!(handler.options.path, Some("/webhook".to_string()));
        assert_eq!(handler.shell, "echo \"webhook received\"");
    }

    #[test]
    fn test_parse_sqs_handler() {
        let yaml = r#"
handlers:
  - type: SQS
    name: test-sqs
    options:
      type: sqs
      queue_url: https://sqs.us-east-1.amazonaws.com/123456789/my-queue
      poll_interval: 30
    shell: echo "sqs message"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        assert_eq!(config.handlers.len(), 1);
        let handler = &config.handlers[0];
        assert!(matches!(handler.event_type, EventType::Sqs));
        assert_eq!(
            handler.options.queue_url,
            Some("https://sqs.us-east-1.amazonaws.com/123456789/my-queue".to_string())
        );
        assert_eq!(handler.options.poll_interval, Some(30));
    }

    #[test]
    fn test_parse_cron_handler() {
        let yaml = r#"
handlers:
  - type: Cron
    name: test-cron
    options:
      type: cron
      cron_expression: "0 0 * * * *"
    shell: echo "cron triggered"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        assert_eq!(config.handlers.len(), 1);
        let handler = &config.handlers[0];
        assert!(matches!(handler.event_type, EventType::Cron));
        assert_eq!(
            handler.options.cron_expression,
            Some("0 0 * * * *".to_string())
        );
    }

    #[test]
    fn test_parse_filesystem_handler() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        assert_eq!(config.handlers.len(), 1);
        let handler = &config.handlers[0];
        assert!(matches!(handler.event_type, EventType::Filesystem));
        assert_eq!(handler.options.path, Some("/tmp/watch".to_string()));
    }

    #[test]
    fn test_default_timeout() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test
    options:
      type: http
      port: 8080
      path: /test
    shell: echo "test"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(handler.timeout, 300); // Default 5 minutes
    }

    #[test]
    fn test_custom_timeout() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test
    options:
      type: http
      port: 8080
      path: /test
    shell: echo "test"
    timeout: 60
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(handler.timeout, 60);
    }

    #[test]
    fn test_default_retry_config() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test
    options:
      type: http
      port: 8080
      path: /test
    shell: echo "test"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let retry = &config.handlers[0].retry;
        assert_eq!(retry.max_retries, 0);
        assert_eq!(retry.delay_ms, 1000);
        assert_eq!(retry.backoff_multiplier, 2.0);
        assert_eq!(retry.max_delay_ms, 60000);
    }

    #[test]
    fn test_custom_retry_config() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test
    options:
      type: http
      port: 8080
      path: /test
    shell: echo "test"
    retry:
      max_retries: 5
      delay_ms: 500
      backoff_multiplier: 1.5
      max_delay_ms: 30000
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let retry = &config.handlers[0].retry;
        assert_eq!(retry.max_retries, 5);
        assert_eq!(retry.delay_ms, 500);
        assert_eq!(retry.backoff_multiplier, 1.5);
        assert_eq!(retry.max_delay_ms, 30000);
    }

    #[test]
    fn test_partial_retry_config() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test
    options:
      type: http
      port: 8080
      path: /test
    shell: echo "test"
    retry:
      max_retries: 3
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let retry = &config.handlers[0].retry;
        assert_eq!(retry.max_retries, 3);
        // Other values should be defaults
        assert_eq!(retry.delay_ms, 1000);
        assert_eq!(retry.backoff_multiplier, 2.0);
        assert_eq!(retry.max_delay_ms, 60000);
    }

    #[test]
    fn test_multiple_handlers() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: webhook-handler
    options:
      type: http
      port: 8080
      path: /webhook
    shell: echo "webhook"
  - type: Cron
    name: cron-handler
    options:
      type: cron
      cron_expression: "0 * * * * *"
    shell: echo "cron"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        assert_eq!(config.handlers.len(), 2);
        assert!(matches!(config.handlers[0].event_type, EventType::Webhook));
        assert!(matches!(config.handlers[1].event_type, EventType::Cron));
    }

    #[test]
    fn test_invalid_yaml() {
        let yaml = "invalid: [yaml: content";
        let result = parse_config(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_required_field() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test
    options:
      type: http
    # Missing shell field
"#;
        let result = parse_config(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_handlers() {
        let yaml = r#"
handlers: []
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        assert_eq!(config.handlers.len(), 0);
    }

    #[test]
    fn test_load_config_file_not_found() {
        let result = load_config("/nonexistent/path/config.yml");
        assert!(result.is_err());
    }

    #[test]
    fn test_retry_config_default_impl() {
        let default = RetryConfig::default();
        assert_eq!(default.max_retries, 0);
        assert_eq!(default.delay_ms, 1000);
        assert_eq!(default.backoff_multiplier, 2.0);
        assert_eq!(default.max_delay_ms, 60000);
    }

    #[test]
    fn test_all_event_types() {
        // Test that all event types parse correctly
        let event_types = vec![
            ("SQS", "queue_url: http://test\n      poll_interval: 10"),
            ("WebPolling", ""),
            ("Cron", "cron_expression: \"* * * * * *\""),
            ("Webhook", "port: 8080\n      path: /test"),
            ("Filesystem", "path: /tmp"),
            ("Database", ""),
        ];

        for (event_type, extra_options) in event_types {
            let yaml = format!(
                r#"
handlers:
  - type: {}
    name: test
    options:
      type: test
      {}
    shell: echo "test"
"#,
                event_type, extra_options
            );
            let result = parse_config(&yaml);
            assert!(
                result.is_ok(),
                "Failed to parse event type {}: {:?}",
                event_type,
                result.err()
            );
        }
    }
}
