use crate::config::RetryConfig;
use crate::event::{keys, Event, EventType};
use log::{error, info, warn};
use std::collections::HashMap;
use tokio::process::Command;
use tokio::time::{sleep, timeout, Duration};

/// Result of a command execution attempt
#[derive(Debug, Clone, PartialEq)]
pub enum CommandResult {
    Success,
    Failed { exit_code: Option<i32> },
    Timeout,
    ExecutionError { message: String },
}

impl CommandResult {
    pub fn is_success(&self) -> bool {
        matches!(self, CommandResult::Success)
    }

    #[allow(dead_code)]
    pub fn is_retriable(&self) -> bool {
        // Success is not retriable, everything else is
        !self.is_success()
    }
}

/// Execute a shell command with timeout and log the results
/// Returns the result of the execution
pub async fn execute_shell_command(
    command: &str,
    handler_name: &str,
    timeout_secs: u64,
) -> CommandResult {
    info!(
        "Executing command for '{}' (timeout: {}s): {}",
        handler_name, timeout_secs, command
    );

    let timeout_duration = Duration::from_secs(timeout_secs);
    let command_future = Command::new("sh").arg("-c").arg(command).output();

    match timeout(timeout_duration, command_future).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                info!("Command for '{}' executed successfully", handler_name);
                if !output.stdout.is_empty() {
                    info!("Output: {}", String::from_utf8_lossy(&output.stdout));
                }
                CommandResult::Success
            } else {
                error!(
                    "Command for '{}' failed with status: {}",
                    handler_name, output.status
                );
                if !output.stderr.is_empty() {
                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                }
                CommandResult::Failed {
                    exit_code: output.status.code(),
                }
            }
        }
        Ok(Err(e)) => {
            error!("Failed to execute command for '{}': {:?}", handler_name, e);
            CommandResult::ExecutionError {
                message: e.to_string(),
            }
        }
        Err(_) => {
            error!(
                "Command for '{}' timed out after {} seconds",
                handler_name, timeout_secs
            );
            CommandResult::Timeout
        }
    }
}

/// Execute a shell command with retry logic and exponential backoff
pub async fn execute_shell_command_with_retry(
    command: &str,
    handler_name: &str,
    timeout_secs: u64,
    retry_config: &RetryConfig,
) -> CommandResult {
    let mut attempt = 0;
    let mut current_delay_ms = retry_config.delay_ms;

    loop {
        let result = execute_shell_command(command, handler_name, timeout_secs).await;

        if result.is_success() {
            return result;
        }

        // Check if we should retry
        if attempt >= retry_config.max_retries {
            if retry_config.max_retries > 0 {
                error!(
                    "Command for '{}' failed after {} attempts (max retries: {})",
                    handler_name,
                    attempt + 1,
                    retry_config.max_retries
                );
            }
            return result;
        }

        attempt += 1;

        // Log retry attempt
        warn!(
            "Command for '{}' failed, retrying in {}ms (attempt {}/{})",
            handler_name, current_delay_ms, attempt, retry_config.max_retries
        );

        // Wait before retrying
        sleep(Duration::from_millis(current_delay_ms)).await;

        // Calculate next delay with exponential backoff, capped at max_delay_ms
        current_delay_ms = ((current_delay_ms as f64) * retry_config.backoff_multiplier) as u64;
        current_delay_ms = current_delay_ms.min(retry_config.max_delay_ms);
    }
}

/// Execute a shell command with additional context information (with retry support)
pub async fn execute_shell_command_with_context(
    command: &str,
    handler_name: &str,
    context: &str,
    timeout_secs: u64,
    retry_config: &RetryConfig,
) -> CommandResult {
    info!(
        "Executing command for '{}' ({}): {}",
        handler_name, context, command
    );
    execute_shell_command_with_retry(command, handler_name, timeout_secs, retry_config).await
}

/// Environment variable prefix for rustyhook event data
const ENV_PREFIX: &str = "RUSTYHOOK_";

/// Build environment variables from an Event
///
/// Returns a HashMap of environment variable names to values.
/// All variable names are prefixed with RUSTYHOOK_.
pub fn build_event_env_vars(event: &Event) -> HashMap<String, String> {
    let mut env_vars = HashMap::new();

    // Common event fields
    env_vars.insert(format!("{}EVENT_ID", ENV_PREFIX), event.id.clone());
    env_vars.insert(
        format!("{}EVENT_SOURCE", ENV_PREFIX),
        event.source_handler.clone(),
    );
    env_vars.insert(
        format!("{}EVENT_TYPE", ENV_PREFIX),
        event.event_type.to_string(),
    );
    env_vars.insert(
        format!("{}EVENT_TIMESTAMP", ENV_PREFIX),
        event.timestamp.to_rfc3339(),
    );

    // Full event as JSON
    if let Ok(json) = event.to_json() {
        env_vars.insert(format!("{}EVENT_JSON", ENV_PREFIX), json);
    }

    // Type-specific variables
    match event.event_type {
        EventType::Filesystem => {
            if let Some(path) = event.get(keys::FILE_PATH) {
                env_vars.insert(format!("{}FILE_PATH", ENV_PREFIX), path.clone());
            }
            if let Some(kind) = event.get(keys::FILE_EVENT_KIND) {
                env_vars.insert(format!("{}FILE_EVENT", ENV_PREFIX), kind.clone());
            }
        }
        EventType::Webhook => {
            if let Some(method) = event.get(keys::HTTP_METHOD) {
                env_vars.insert(format!("{}HTTP_METHOD", ENV_PREFIX), method.clone());
            }
            if let Some(uri) = event.get(keys::HTTP_URI) {
                env_vars.insert(format!("{}HTTP_URI", ENV_PREFIX), uri.clone());
            }
            if let Some(body) = event.get(keys::HTTP_BODY) {
                env_vars.insert(format!("{}HTTP_BODY", ENV_PREFIX), body.clone());
            }
            if let Some(headers) = event.get(keys::HTTP_HEADERS) {
                env_vars.insert(format!("{}HTTP_HEADERS", ENV_PREFIX), headers.clone());
            }
        }
        EventType::Sqs => {
            if let Some(body) = event.get(keys::SQS_MESSAGE_BODY) {
                env_vars.insert(format!("{}SQS_BODY", ENV_PREFIX), body.clone());
            }
            if let Some(id) = event.get(keys::SQS_MESSAGE_ID) {
                env_vars.insert(format!("{}SQS_MESSAGE_ID", ENV_PREFIX), id.clone());
            }
            if let Some(handle) = event.get(keys::SQS_RECEIPT_HANDLE) {
                env_vars.insert(format!("{}SQS_RECEIPT_HANDLE", ENV_PREFIX), handle.clone());
            }
        }
        EventType::Cron => {
            if let Some(expr) = event.get(keys::CRON_EXPRESSION) {
                env_vars.insert(format!("{}CRON_EXPRESSION", ENV_PREFIX), expr.clone());
            }
            if let Some(time) = event.get(keys::CRON_SCHEDULED_TIME) {
                env_vars.insert(format!("{}CRON_SCHEDULED_TIME", ENV_PREFIX), time.clone());
            }
        }
        EventType::Forwarded => {
            // For forwarded events, include all original data as-is
            // The type-specific vars from the original event are already in the data map
            for (key, value) in &event.data {
                // Convert key to uppercase and replace non-alphanumeric chars
                let env_key = key
                    .to_uppercase()
                    .replace(|c: char| !c.is_alphanumeric(), "_");
                env_vars.insert(format!("{}{}", ENV_PREFIX, env_key), value.clone());
            }
        }
    }

    env_vars
}

/// Execute a shell command with event context (sets RUSTYHOOK_* environment variables)
pub async fn execute_shell_command_with_event(
    command: &str,
    handler_name: &str,
    timeout_secs: u64,
    event: &Event,
) -> CommandResult {
    info!(
        "Executing command for '{}' with event {} (timeout: {}s): {}",
        handler_name, event.id, timeout_secs, command
    );

    let env_vars = build_event_env_vars(event);
    let timeout_duration = Duration::from_secs(timeout_secs);

    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);

    // Add event environment variables
    for (key, value) in &env_vars {
        cmd.env(key, value);
    }

    let command_future = cmd.output();

    match timeout(timeout_duration, command_future).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                info!("Command for '{}' executed successfully", handler_name);
                if !output.stdout.is_empty() {
                    info!("Output: {}", String::from_utf8_lossy(&output.stdout));
                }
                CommandResult::Success
            } else {
                error!(
                    "Command for '{}' failed with status: {}",
                    handler_name, output.status
                );
                if !output.stderr.is_empty() {
                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                }
                CommandResult::Failed {
                    exit_code: output.status.code(),
                }
            }
        }
        Ok(Err(e)) => {
            error!("Failed to execute command for '{}': {:?}", handler_name, e);
            CommandResult::ExecutionError {
                message: e.to_string(),
            }
        }
        Err(_) => {
            error!(
                "Command for '{}' timed out after {} seconds",
                handler_name, timeout_secs
            );
            CommandResult::Timeout
        }
    }
}

/// Execute a shell command with event context and retry logic
pub async fn execute_shell_command_with_event_and_retry(
    command: &str,
    handler_name: &str,
    timeout_secs: u64,
    event: &Event,
    retry_config: &RetryConfig,
) -> CommandResult {
    let mut attempt = 0;
    let mut current_delay_ms = retry_config.delay_ms;

    loop {
        let result =
            execute_shell_command_with_event(command, handler_name, timeout_secs, event).await;

        if result.is_success() {
            return result;
        }

        // Check if we should retry
        if attempt >= retry_config.max_retries {
            if retry_config.max_retries > 0 {
                error!(
                    "Command for '{}' failed after {} attempts (max retries: {})",
                    handler_name,
                    attempt + 1,
                    retry_config.max_retries
                );
            }
            return result;
        }

        attempt += 1;

        // Log retry attempt
        warn!(
            "Command for '{}' failed, retrying in {}ms (attempt {}/{})",
            handler_name, current_delay_ms, attempt, retry_config.max_retries
        );

        // Wait before retrying
        sleep(Duration::from_millis(current_delay_ms)).await;

        // Calculate next delay with exponential backoff, capped at max_delay_ms
        current_delay_ms = ((current_delay_ms as f64) * retry_config.backoff_multiplier) as u64;
        current_delay_ms = current_delay_ms.min(retry_config.max_delay_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_retry() -> RetryConfig {
        RetryConfig::default()
    }

    fn retry_config(max_retries: u32, delay_ms: u64) -> RetryConfig {
        RetryConfig {
            max_retries,
            delay_ms,
            backoff_multiplier: 2.0,
            max_delay_ms: 60000,
        }
    }

    #[tokio::test]
    async fn test_execute_shell_command_success() {
        let result = execute_shell_command("echo 'test'", "test_handler", 30).await;
        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_shell_command_failure() {
        let result = execute_shell_command("false", "test_handler", 30).await;
        assert!(matches!(result, CommandResult::Failed { .. }));
    }

    #[tokio::test]
    async fn test_execute_shell_command_timeout() {
        let result = execute_shell_command("sleep 10", "test_handler", 1).await;
        assert_eq!(result, CommandResult::Timeout);
    }

    #[tokio::test]
    async fn test_execute_with_retry_success_no_retry_needed() {
        let result = execute_shell_command_with_retry(
            "echo 'test'",
            "test_handler",
            30,
            &retry_config(3, 100),
        )
        .await;
        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_with_retry_failure_exhausts_retries() {
        let start = std::time::Instant::now();
        let result = execute_shell_command_with_retry(
            "false",
            "test_handler",
            30,
            &retry_config(2, 50), // 2 retries with 50ms initial delay
        )
        .await;
        let elapsed = start.elapsed();

        // Should have failed after retries
        assert!(matches!(result, CommandResult::Failed { .. }));

        // Should have waited for retries: 50ms + 100ms = 150ms minimum
        // Using 100ms as threshold to account for test execution time variance
        assert!(
            elapsed.as_millis() >= 100,
            "Expected at least 100ms delay for retries, got {}ms",
            elapsed.as_millis()
        );
    }

    #[tokio::test]
    async fn test_execute_with_retry_no_retry_configured() {
        let result =
            execute_shell_command_with_retry("false", "test_handler", 30, &no_retry()).await;
        assert!(matches!(result, CommandResult::Failed { .. }));
    }

    #[tokio::test]
    async fn test_execute_with_retry_eventual_success() {
        use tokio::fs;

        // Create a unique temporary file to track attempts using PID and timestamp
        let tmp_file = format!(
            "/tmp/rustyhook_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let attempt_file = tmp_file.clone();

        // Command that fails twice then succeeds
        // Uses a file to track state across shell invocations
        let command = format!(
            r#"
            if [ -f "{0}.2" ]; then
                echo "success on attempt 3"
                exit 0
            elif [ -f "{0}.1" ]; then
                touch "{0}.2"
                exit 1
            else
                touch "{0}.1"
                exit 1
            fi
            "#,
            tmp_file
        );

        let result = execute_shell_command_with_retry(
            &command,
            "test_handler",
            30,
            &retry_config(3, 10), // 3 retries with 10ms delay
        )
        .await;

        // Cleanup temp files
        let _ = fs::remove_file(format!("{}.1", attempt_file)).await;
        let _ = fs::remove_file(format!("{}.2", attempt_file)).await;

        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_with_context() {
        let result = execute_shell_command_with_context(
            "echo 'test'",
            "test_handler",
            "test context",
            30,
            &no_retry(),
        )
        .await;
        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_command_result_is_success() {
        assert!(CommandResult::Success.is_success());
        assert!(!CommandResult::Failed { exit_code: Some(1) }.is_success());
        assert!(!CommandResult::Timeout.is_success());
        assert!(!CommandResult::ExecutionError {
            message: "test".to_string()
        }
        .is_success());
    }

    #[tokio::test]
    async fn test_command_result_is_retriable() {
        assert!(!CommandResult::Success.is_retriable());
        assert!(CommandResult::Failed { exit_code: Some(1) }.is_retriable());
        assert!(CommandResult::Timeout.is_retriable());
        assert!(CommandResult::ExecutionError {
            message: "test".to_string()
        }
        .is_retriable());
    }

    // ============== Event-based execution tests ==============

    #[test]
    fn test_build_event_env_vars_common_fields() {
        let event = Event::new("test-source", EventType::Filesystem);
        let env_vars = build_event_env_vars(&event);

        assert_eq!(env_vars.get("RUSTYHOOK_EVENT_ID"), Some(&event.id));
        assert_eq!(
            env_vars.get("RUSTYHOOK_EVENT_SOURCE"),
            Some(&"test-source".to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_EVENT_TYPE"),
            Some(&"filesystem".to_string())
        );
        assert!(env_vars.contains_key("RUSTYHOOK_EVENT_TIMESTAMP"));
        assert!(env_vars.contains_key("RUSTYHOOK_EVENT_JSON"));
    }

    #[test]
    fn test_build_event_env_vars_filesystem() {
        let event = Event::from_filesystem("fs-handler", "/tmp/test.txt", "create");
        let env_vars = build_event_env_vars(&event);

        assert_eq!(
            env_vars.get("RUSTYHOOK_FILE_PATH"),
            Some(&"/tmp/test.txt".to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_FILE_EVENT"),
            Some(&"create".to_string())
        );
    }

    #[test]
    fn test_build_event_env_vars_webhook() {
        let event = Event::from_webhook_with_headers(
            "webhook-handler",
            "POST",
            "/api/hook",
            r#"{"data": "test"}"#,
            r#"{"Content-Type": "application/json"}"#,
        );
        let env_vars = build_event_env_vars(&event);

        assert_eq!(
            env_vars.get("RUSTYHOOK_HTTP_METHOD"),
            Some(&"POST".to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_HTTP_URI"),
            Some(&"/api/hook".to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_HTTP_BODY"),
            Some(&r#"{"data": "test"}"#.to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_HTTP_HEADERS"),
            Some(&r#"{"Content-Type": "application/json"}"#.to_string())
        );
    }

    #[test]
    fn test_build_event_env_vars_sqs() {
        let event = Event::from_sqs("sqs-handler", "message body", "msg-123", "receipt-456");
        let env_vars = build_event_env_vars(&event);

        assert_eq!(
            env_vars.get("RUSTYHOOK_SQS_BODY"),
            Some(&"message body".to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_SQS_MESSAGE_ID"),
            Some(&"msg-123".to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_SQS_RECEIPT_HANDLE"),
            Some(&"receipt-456".to_string())
        );
    }

    #[test]
    fn test_build_event_env_vars_cron() {
        use chrono::Utc;
        let now = Utc::now();
        let event = Event::from_cron("cron-handler", "0 * * * * *", now);
        let env_vars = build_event_env_vars(&event);

        assert_eq!(
            env_vars.get("RUSTYHOOK_CRON_EXPRESSION"),
            Some(&"0 * * * * *".to_string())
        );
        assert_eq!(
            env_vars.get("RUSTYHOOK_CRON_SCHEDULED_TIME"),
            Some(&now.to_rfc3339())
        );
    }

    #[test]
    fn test_build_event_env_vars_forwarded() {
        let original = Event::from_filesystem("original-handler", "/path/file.txt", "modify");
        let forwarded = Event::forwarded(&original);
        let env_vars = build_event_env_vars(&forwarded);

        // Should have common fields
        assert!(env_vars.contains_key("RUSTYHOOK_EVENT_ID"));
        assert_eq!(
            env_vars.get("RUSTYHOOK_EVENT_TYPE"),
            Some(&"forwarded".to_string())
        );

        // Should have original data with normalized keys
        assert_eq!(
            env_vars.get("RUSTYHOOK_FILE_PATH"),
            Some(&"/path/file.txt".to_string())
        );
    }

    #[tokio::test]
    async fn test_execute_with_event_env_vars_accessible() {
        let event = Event::from_filesystem("test-handler", "/tmp/test.txt", "create");

        // Run a command that echoes the environment variables
        let result = execute_shell_command_with_event(
            r#"echo "PATH=$RUSTYHOOK_FILE_PATH EVENT=$RUSTYHOOK_FILE_EVENT""#,
            "test_handler",
            30,
            &event,
        )
        .await;

        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_with_event_captures_env_vars() {
        let event = Event::from_filesystem("fs-handler", "/tmp/capture.txt", "modify");

        // Use a command that writes env vars to a temp file so we can verify
        let tmp_file = format!(
            "/tmp/rustyhook_env_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );

        let command = format!(
            r#"echo "$RUSTYHOOK_FILE_PATH|$RUSTYHOOK_FILE_EVENT|$RUSTYHOOK_EVENT_TYPE" > {}"#,
            tmp_file
        );

        let result = execute_shell_command_with_event(&command, "test_handler", 30, &event).await;
        assert_eq!(result, CommandResult::Success);

        // Read and verify the output
        let contents = tokio::fs::read_to_string(&tmp_file).await.unwrap();
        let _ = tokio::fs::remove_file(&tmp_file).await;

        assert_eq!(contents.trim(), "/tmp/capture.txt|modify|filesystem");
    }

    #[tokio::test]
    async fn test_execute_with_event_and_retry_success() {
        let event = Event::from_filesystem("handler", "/tmp/test", "create");

        let result = execute_shell_command_with_event_and_retry(
            "echo 'test'",
            "test_handler",
            30,
            &event,
            &retry_config(3, 10),
        )
        .await;

        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_with_event_and_retry_failure() {
        let event = Event::from_filesystem("handler", "/tmp/test", "create");

        let result = execute_shell_command_with_event_and_retry(
            "false",
            "test_handler",
            30,
            &event,
            &retry_config(1, 10),
        )
        .await;

        assert!(matches!(result, CommandResult::Failed { .. }));
    }

    #[tokio::test]
    async fn test_execute_with_event_timeout() {
        let event = Event::from_webhook("handler", "GET", "/test", "");

        let result = execute_shell_command_with_event("sleep 10", "test_handler", 1, &event).await;

        assert_eq!(result, CommandResult::Timeout);
    }
}
