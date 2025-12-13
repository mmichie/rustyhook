use crate::config::RetryConfig;
use log::{error, info, warn};
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
}
