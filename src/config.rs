use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::{env, error::Error, fmt, fs};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub handlers: Vec<HandlerConfig>,
}

/// Configuration for which shell to use when executing commands.
///
/// Supports common shells like sh, bash, zsh (Unix) and powershell, cmd (Windows).
/// Each shell has different syntax for executing commands.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ShellConfig {
    /// Simple shell name (e.g., "bash", "sh", "zsh", "powershell", "cmd")
    Simple(String),
    /// Explicit "none" - execute command directly without a shell
    #[serde(rename = "none")]
    None,
}

impl Default for ShellConfig {
    fn default() -> Self {
        // Default to $SHELL environment variable, or "sh" if not set
        if let Ok(shell) = env::var("SHELL") {
            // Extract just the shell name from the path (e.g., "/bin/bash" -> "bash")
            if let Some(name) = std::path::Path::new(&shell).file_name() {
                if let Some(name_str) = name.to_str() {
                    return ShellConfig::Simple(name_str.to_string());
                }
            }
            ShellConfig::Simple(shell)
        } else {
            ShellConfig::Simple("sh".to_string())
        }
    }
}

impl ShellConfig {
    /// Get the shell program and argument for command execution.
    /// Returns (program, arg) tuple, e.g., ("bash", "-c") or ("cmd", "/c").
    /// Returns None if shell is configured as "none" (direct execution).
    pub fn get_shell_command(&self) -> Option<(&str, &str)> {
        match self {
            ShellConfig::None => None,
            ShellConfig::Simple(shell) => {
                let shell_lower = shell.to_lowercase();
                match shell_lower.as_str() {
                    "sh" | "bash" | "zsh" | "fish" | "dash" | "ash" | "ksh" => {
                        Some((shell.as_str(), "-c"))
                    }
                    "powershell" | "pwsh" => Some((shell.as_str(), "-Command")),
                    "cmd" => Some((shell.as_str(), "/c")),
                    // Default to Unix-style -c for unknown shells
                    _ => Some((shell.as_str(), "-c")),
                }
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HandlerConfig {
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub name: String,     // Renamed from 'handler' to 'name'
    pub options: Options, // Changed to a struct to match the YAML format
    pub shell: String, // The command to execute (renamed from 'shell' but kept for config compat)
    #[serde(default = "default_timeout")]
    pub timeout: u64, // Command timeout in seconds (default: 300)
    #[serde(default)]
    pub retry: RetryConfig, // Retry configuration (optional)
    #[serde(default)]
    pub forward_to: Vec<String>, // Handler names to forward events to
    /// Shell to use for command execution (e.g., "bash", "zsh", "sh", "powershell")
    /// Defaults to $SHELL environment variable or "sh" if not set
    #[serde(default, rename = "shell_program")]
    pub shell_program: ShellConfig,
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

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventType::Sqs => write!(f, "SQS"),
            EventType::WebPolling => write!(f, "WebPolling"),
            EventType::Cron => write!(f, "Cron"),
            EventType::Webhook => write!(f, "Webhook"),
            EventType::Filesystem => write!(f, "Filesystem"),
            EventType::Database => write!(f, "Database"),
        }
    }
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
    /// Debounce duration in milliseconds for filesystem events (default: 100ms)
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    /// Glob patterns to include (e.g., ["*.rs", "src/**/*.toml"])
    /// If specified, only files matching at least one pattern are processed
    #[serde(default)]
    pub include: Vec<String>,
    /// Glob patterns to exclude (e.g., ["*.tmp", "target/**"])
    /// Files matching any exclude pattern are ignored, even if they match include patterns
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_debounce_ms() -> u64 {
    100 // 100ms default debounce
}

// Function to load and parse the YAML configuration file
pub fn load_config(file_path: &str) -> Result<Config, Box<dyn Error>> {
    let config_str = fs::read_to_string(file_path)?;
    let config: Config = serde_yaml::from_str(&config_str)?;
    Ok(config)
}

/// Errors that can occur during configuration validation
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// Two or more handlers have the same name
    DuplicateHandlerName { name: String },
    /// A handler's forward_to references a non-existent handler
    InvalidForwardTarget { handler: String, target: String },
    /// A handler attempts to forward to itself
    SelfForwarding { handler: String },
    /// Circular dependency detected in forward_to chain
    CircularDependency { cycle: Vec<String> },
    /// Handler is missing required options for its type
    MissingOption {
        handler: String,
        handler_type: String,
        option: String,
    },
    /// Invalid option value
    InvalidOption {
        handler: String,
        option: String,
        reason: String,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationError::DuplicateHandlerName { name } => {
                write!(f, "Duplicate handler name: '{}'", name)
            }
            ValidationError::InvalidForwardTarget { handler, target } => {
                write!(
                    f,
                    "Handler '{}' forwards to unknown handler '{}'",
                    handler, target
                )
            }
            ValidationError::SelfForwarding { handler } => {
                write!(f, "Handler '{}' cannot forward to itself", handler)
            }
            ValidationError::CircularDependency { cycle } => {
                write!(f, "Circular dependency detected: {}", cycle.join(" -> "))
            }
            ValidationError::MissingOption {
                handler,
                handler_type,
                option,
            } => {
                write!(
                    f,
                    "Handler '{}' (type: {}) is missing required option: {}",
                    handler, handler_type, option
                )
            }
            ValidationError::InvalidOption {
                handler,
                option,
                reason,
            } => {
                write!(
                    f,
                    "Handler '{}' has invalid option '{}': {}",
                    handler, option, reason
                )
            }
        }
    }
}

impl Error for ValidationError {}

/// Validate the configuration for handler forwarding rules.
///
/// Checks:
/// - All handler names are unique
/// - All forward_to targets reference existing handlers
/// - No handler forwards to itself
/// - No circular dependencies in the forwarding chain
/// - Handler-specific required options are present
/// - Handler option values are valid (e.g., cron expressions)
pub fn validate_config(config: &Config) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();

    // Build a map of handler names for quick lookup
    let handler_names: HashSet<&str> = config.handlers.iter().map(|h| h.name.as_str()).collect();

    // Check for duplicate handler names
    let mut seen_names: HashSet<&str> = HashSet::new();
    for handler in &config.handlers {
        if !seen_names.insert(&handler.name) {
            errors.push(ValidationError::DuplicateHandlerName {
                name: handler.name.clone(),
            });
        }
    }

    // Build forward_to map for cycle detection
    let mut forward_map: HashMap<&str, Vec<&str>> = HashMap::new();

    for handler in &config.handlers {
        // Check for self-forwarding
        if handler.forward_to.contains(&handler.name) {
            errors.push(ValidationError::SelfForwarding {
                handler: handler.name.clone(),
            });
        }

        // Check for invalid forward targets
        for target in &handler.forward_to {
            if !handler_names.contains(target.as_str()) {
                errors.push(ValidationError::InvalidForwardTarget {
                    handler: handler.name.clone(),
                    target: target.clone(),
                });
            }
        }

        // Add to forward map for cycle detection
        forward_map.insert(
            &handler.name,
            handler.forward_to.iter().map(|s| s.as_str()).collect(),
        );

        // Validate handler-specific options
        validate_handler_options(handler, &mut errors);
    }

    // Detect circular dependencies using DFS
    if let Some(cycle) = detect_cycle(&forward_map) {
        errors.push(ValidationError::CircularDependency { cycle });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate that a handler has the required options for its type
fn validate_handler_options(handler: &HandlerConfig, errors: &mut Vec<ValidationError>) {
    match handler.event_type {
        EventType::Sqs => {
            if handler.options.queue_url.is_none() {
                errors.push(ValidationError::MissingOption {
                    handler: handler.name.clone(),
                    handler_type: handler.event_type.to_string(),
                    option: "queue_url".to_string(),
                });
            }
            if handler.options.poll_interval.is_none() {
                errors.push(ValidationError::MissingOption {
                    handler: handler.name.clone(),
                    handler_type: handler.event_type.to_string(),
                    option: "poll_interval".to_string(),
                });
            }
        }
        EventType::Webhook => {
            if handler.options.port.is_none() {
                errors.push(ValidationError::MissingOption {
                    handler: handler.name.clone(),
                    handler_type: handler.event_type.to_string(),
                    option: "port".to_string(),
                });
            }
            if handler.options.path.is_none() {
                errors.push(ValidationError::MissingOption {
                    handler: handler.name.clone(),
                    handler_type: handler.event_type.to_string(),
                    option: "path".to_string(),
                });
            }
        }
        EventType::Filesystem => {
            if handler.options.path.is_none() {
                errors.push(ValidationError::MissingOption {
                    handler: handler.name.clone(),
                    handler_type: handler.event_type.to_string(),
                    option: "path".to_string(),
                });
            }
        }
        EventType::Cron => {
            match &handler.options.cron_expression {
                None => {
                    errors.push(ValidationError::MissingOption {
                        handler: handler.name.clone(),
                        handler_type: handler.event_type.to_string(),
                        option: "cron_expression".to_string(),
                    });
                }
                Some(expr) => {
                    // Validate cron expression syntax
                    if let Err(e) = validate_cron_expression(expr) {
                        errors.push(ValidationError::InvalidOption {
                            handler: handler.name.clone(),
                            option: "cron_expression".to_string(),
                            reason: e,
                        });
                    }
                }
            }
        }
        EventType::WebPolling | EventType::Database => {
            // Not yet implemented, no validation
        }
    }
}

/// Validate a cron expression syntax
fn validate_cron_expression(expression: &str) -> Result<(), String> {
    use cron::Schedule;
    use std::str::FromStr;

    Schedule::from_str(expression).map_err(|e| e.to_string())?;
    Ok(())
}

/// Detect cycles in the forward_to graph using DFS.
/// Returns the first cycle found, or None if no cycles exist.
fn detect_cycle(graph: &HashMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut rec_stack: HashSet<&str> = HashSet::new();
    let mut path: Vec<&str> = Vec::new();

    for &node in graph.keys() {
        if !visited.contains(node) {
            if let Some(cycle) = dfs_cycle(node, graph, &mut visited, &mut rec_stack, &mut path) {
                return Some(cycle);
            }
        }
    }

    None
}

/// DFS helper for cycle detection
fn dfs_cycle<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    rec_stack: &mut HashSet<&'a str>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<String>> {
    visited.insert(node);
    rec_stack.insert(node);
    path.push(node);

    if let Some(neighbors) = graph.get(node) {
        for &neighbor in neighbors {
            if !visited.contains(neighbor) {
                if let Some(cycle) = dfs_cycle(neighbor, graph, visited, rec_stack, path) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(neighbor) {
                // Found a cycle - extract it from the path
                let cycle_start = path.iter().position(|&n| n == neighbor).unwrap();
                let mut cycle: Vec<String> =
                    path[cycle_start..].iter().map(|&s| s.to_string()).collect();
                cycle.push(neighbor.to_string()); // Complete the cycle
                return Some(cycle);
            }
        }
    }

    path.pop();
    rec_stack.remove(node);
    None
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

    // ============== forward_to and validation tests ==============

    #[test]
    fn test_parse_forward_to() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: file-watcher
    options:
      type: filesystem
      path: /tmp
    shell: echo "file changed"
    forward_to:
      - processor
      - notifier
  - type: Webhook
    name: processor
    options:
      type: http
      port: 8080
      path: /process
    shell: echo "processing"
  - type: Webhook
    name: notifier
    options:
      type: http
      port: 8081
      path: /notify
    shell: echo "notifying"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        assert_eq!(config.handlers[0].forward_to, vec!["processor", "notifier"]);
        assert!(config.handlers[1].forward_to.is_empty());
        assert!(config.handlers[2].forward_to.is_empty());
    }

    #[test]
    fn test_validate_config_valid() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: handler-a
    options:
      type: filesystem
      path: /tmp
    shell: echo "a"
    forward_to:
      - handler-b
  - type: Webhook
    name: handler-b
    options:
      type: http
      port: 8080
      path: /b
    shell: echo "b"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_ok(), "Expected valid config: {:?}", result);
    }

    #[test]
    fn test_validate_config_duplicate_names() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: duplicate
    options:
      type: http
      port: 8080
      path: /a
    shell: echo "a"
  - type: Webhook
    name: duplicate
    options:
      type: http
      port: 8081
      path: /b
    shell: echo "b"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::DuplicateHandlerName { name } if name == "duplicate"
        )));
    }

    #[test]
    fn test_validate_config_invalid_target() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: handler-a
    options:
      type: http
      port: 8080
      path: /a
    shell: echo "a"
    forward_to:
      - nonexistent
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::InvalidForwardTarget { handler, target }
                if handler == "handler-a" && target == "nonexistent"
        )));
    }

    #[test]
    fn test_validate_config_self_forwarding() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: self-forward
    options:
      type: http
      port: 8080
      path: /a
    shell: echo "a"
    forward_to:
      - self-forward
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::SelfForwarding { handler } if handler == "self-forward"
        )));
    }

    #[test]
    fn test_validate_config_circular_dependency_direct() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: handler-a
    options:
      type: http
      port: 8080
      path: /a
    shell: echo "a"
    forward_to:
      - handler-b
  - type: Webhook
    name: handler-b
    options:
      type: http
      port: 8081
      path: /b
    shell: echo "b"
    forward_to:
      - handler-a
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::CircularDependency { .. })),
            "Expected CircularDependency error, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_validate_config_circular_dependency_chain() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: handler-a
    options:
      type: http
      port: 8080
      path: /a
    shell: echo "a"
    forward_to:
      - handler-b
  - type: Webhook
    name: handler-b
    options:
      type: http
      port: 8081
      path: /b
    shell: echo "b"
    forward_to:
      - handler-c
  - type: Webhook
    name: handler-c
    options:
      type: http
      port: 8082
      path: /c
    shell: echo "c"
    forward_to:
      - handler-a
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::CircularDependency { .. })),
            "Expected CircularDependency error in chain, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_validate_config_no_forward_to() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: standalone
    options:
      type: http
      port: 8080
      path: /a
    shell: echo "a"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_config_multiple_errors() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: handler-a
    options:
      type: http
      port: 8080
      path: /a
    shell: echo "a"
    forward_to:
      - nonexistent
      - handler-a
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        // Should have both InvalidForwardTarget and SelfForwarding errors
        assert!(
            errors.len() >= 2,
            "Expected at least 2 errors, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_validation_error_display() {
        assert_eq!(
            ValidationError::DuplicateHandlerName {
                name: "foo".to_string()
            }
            .to_string(),
            "Duplicate handler name: 'foo'"
        );

        assert_eq!(
            ValidationError::InvalidForwardTarget {
                handler: "a".to_string(),
                target: "b".to_string()
            }
            .to_string(),
            "Handler 'a' forwards to unknown handler 'b'"
        );

        assert_eq!(
            ValidationError::SelfForwarding {
                handler: "x".to_string()
            }
            .to_string(),
            "Handler 'x' cannot forward to itself"
        );

        assert_eq!(
            ValidationError::CircularDependency {
                cycle: vec!["a".to_string(), "b".to_string(), "a".to_string()]
            }
            .to_string(),
            "Circular dependency detected: a -> b -> a"
        );
    }

    #[test]
    fn test_validate_config_fan_out() {
        // One handler forwarding to multiple handlers (valid)
        let yaml = r#"
handlers:
  - type: Filesystem
    name: source
    options:
      type: filesystem
      path: /tmp
    shell: echo "source"
    forward_to:
      - target-1
      - target-2
      - target-3
  - type: Webhook
    name: target-1
    options:
      type: http
      port: 8081
      path: /1
    shell: echo "1"
  - type: Webhook
    name: target-2
    options:
      type: http
      port: 8082
      path: /2
    shell: echo "2"
  - type: Webhook
    name: target-3
    options:
      type: http
      port: 8083
      path: /3
    shell: echo "3"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_ok(), "Fan-out should be valid: {:?}", result);
    }

    #[test]
    fn test_validate_config_linear_chain() {
        // Linear chain without cycles (valid)
        let yaml = r#"
handlers:
  - type: Filesystem
    name: step-1
    options:
      type: filesystem
      path: /tmp
    shell: echo "1"
    forward_to:
      - step-2
  - type: Webhook
    name: step-2
    options:
      type: http
      port: 8081
      path: /2
    shell: echo "2"
    forward_to:
      - step-3
  - type: Webhook
    name: step-3
    options:
      type: http
      port: 8082
      path: /3
    shell: echo "3"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_ok(), "Linear chain should be valid: {:?}", result);
    }

    // ============== debounce_ms tests ==============

    #[test]
    fn test_default_debounce_ms() {
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
        let handler = &config.handlers[0];
        assert_eq!(handler.options.debounce_ms, 100); // Default 100ms
    }

    #[test]
    fn test_custom_debounce_ms() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
      debounce_ms: 500
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(handler.options.debounce_ms, 500);
    }

    #[test]
    fn test_zero_debounce_ms() {
        // Zero means no debouncing (immediate execution)
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
      debounce_ms: 0
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(handler.options.debounce_ms, 0);
    }

    #[test]
    fn test_large_debounce_ms() {
        // Support longer debounce periods for slow-changing scenarios
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
      debounce_ms: 5000
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(handler.options.debounce_ms, 5000);
    }

    #[test]
    fn test_debounce_ms_ignored_for_non_filesystem() {
        // debounce_ms can be set but is only used for filesystem handlers
        let yaml = r#"
handlers:
  - type: Webhook
    name: test-webhook
    options:
      type: http
      port: 8080
      path: /webhook
      debounce_ms: 200
    shell: echo "webhook"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        // Value is stored but only meaningful for Filesystem handlers
        assert_eq!(handler.options.debounce_ms, 200);
    }

    // ============== include/exclude pattern tests ==============

    #[test]
    fn test_default_include_exclude_empty() {
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
        let handler = &config.handlers[0];
        assert!(handler.options.include.is_empty());
        assert!(handler.options.exclude.is_empty());
    }

    #[test]
    fn test_include_patterns() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
      include:
        - "*.rs"
        - "*.toml"
        - "src/**/*.ts"
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(
            handler.options.include,
            vec!["*.rs", "*.toml", "src/**/*.ts"]
        );
        assert!(handler.options.exclude.is_empty());
    }

    #[test]
    fn test_exclude_patterns() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
      exclude:
        - "*.tmp"
        - "target/**"
        - "node_modules/**"
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert!(handler.options.include.is_empty());
        assert_eq!(
            handler.options.exclude,
            vec!["*.tmp", "target/**", "node_modules/**"]
        );
    }

    #[test]
    fn test_include_and_exclude_patterns() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
      include:
        - "*.rs"
        - "*.toml"
      exclude:
        - "*.bak"
        - "target/**"
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(handler.options.include, vec!["*.rs", "*.toml"]);
        assert_eq!(handler.options.exclude, vec!["*.bak", "target/**"]);
    }

    #[test]
    fn test_single_include_pattern() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
      path: /tmp/watch
      include:
        - "**/*.rs"
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let handler = &config.handlers[0];
        assert_eq!(handler.options.include, vec!["**/*.rs"]);
    }

    // ============== semantic validation tests ==============

    #[test]
    fn test_validate_sqs_missing_queue_url() {
        let yaml = r#"
handlers:
  - type: SQS
    name: test-sqs
    options:
      type: sqs
      poll_interval: 30
    shell: echo "sqs message"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingOption { handler, handler_type, option }
                if handler == "test-sqs" && handler_type == "SQS" && option == "queue_url"
        )));
    }

    #[test]
    fn test_validate_sqs_missing_poll_interval() {
        let yaml = r#"
handlers:
  - type: SQS
    name: test-sqs
    options:
      type: sqs
      queue_url: https://sqs.us-east-1.amazonaws.com/123456789/my-queue
    shell: echo "sqs message"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingOption { handler, handler_type, option }
                if handler == "test-sqs" && handler_type == "SQS" && option == "poll_interval"
        )));
    }

    #[test]
    fn test_validate_webhook_missing_port() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test-webhook
    options:
      type: http
      path: /webhook
    shell: echo "webhook"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingOption { handler, handler_type, option }
                if handler == "test-webhook" && handler_type == "Webhook" && option == "port"
        )));
    }

    #[test]
    fn test_validate_webhook_missing_path() {
        let yaml = r#"
handlers:
  - type: Webhook
    name: test-webhook
    options:
      type: http
      port: 8080
    shell: echo "webhook"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingOption { handler, handler_type, option }
                if handler == "test-webhook" && handler_type == "Webhook" && option == "path"
        )));
    }

    #[test]
    fn test_validate_filesystem_missing_path() {
        let yaml = r#"
handlers:
  - type: Filesystem
    name: test-fs
    options:
      type: filesystem
    shell: echo "file changed"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingOption { handler, handler_type, option }
                if handler == "test-fs" && handler_type == "Filesystem" && option == "path"
        )));
    }

    #[test]
    fn test_validate_cron_missing_expression() {
        let yaml = r#"
handlers:
  - type: Cron
    name: test-cron
    options:
      type: cron
    shell: echo "cron triggered"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            ValidationError::MissingOption { handler, handler_type, option }
                if handler == "test-cron" && handler_type == "Cron" && option == "cron_expression"
        )));
    }

    #[test]
    fn test_validate_cron_invalid_expression() {
        let yaml = r#"
handlers:
  - type: Cron
    name: test-cron
    options:
      type: cron
      cron_expression: "not a valid cron"
    shell: echo "cron triggered"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors.iter().any(
                |e| matches!(e, ValidationError::InvalidOption { handler, option, .. }
                if handler == "test-cron" && option == "cron_expression")
            ),
            "Expected InvalidOption error for cron_expression, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_validate_cron_valid_expression() {
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
        let result = validate_config(&config);
        assert!(result.is_ok(), "Expected valid config: {:?}", result);
    }

    #[test]
    fn test_validate_multiple_missing_options() {
        // SQS handler missing both queue_url and poll_interval
        let yaml = r#"
handlers:
  - type: SQS
    name: test-sqs
    options:
      type: sqs
    shell: echo "sqs message"
"#;
        let config = parse_config(yaml).expect("Failed to parse config");
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(
            errors.len() >= 2,
            "Expected at least 2 errors, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_validation_error_missing_option_display() {
        let err = ValidationError::MissingOption {
            handler: "my-handler".to_string(),
            handler_type: "Webhook".to_string(),
            option: "port".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Handler 'my-handler' (type: Webhook) is missing required option: port"
        );
    }

    #[test]
    fn test_validation_error_invalid_option_display() {
        let err = ValidationError::InvalidOption {
            handler: "my-cron".to_string(),
            option: "cron_expression".to_string(),
            reason: "invalid syntax".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Handler 'my-cron' has invalid option 'cron_expression': invalid syntax"
        );
    }
}
