use crate::config::{RetryConfig, ShellConfig};
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
    shell_config: &ShellConfig,
    working_dir: Option<&str>,
) -> CommandResult {
    let shell_info = match shell_config.get_shell_command() {
        Some((prog, _)) => prog.to_string(),
        None => "direct".to_string(),
    };
    let dir_info = working_dir.unwrap_or("(current)");
    info!(
        "Executing command for '{}' (timeout: {}s, shell: {}, dir: {}): {}",
        handler_name, timeout_secs, shell_info, dir_info, command
    );

    let timeout_duration = Duration::from_secs(timeout_secs);

    let command_future = match shell_config.get_shell_command() {
        Some((shell, arg)) => {
            let mut cmd = Command::new(shell);
            cmd.arg(arg).arg(command);
            if let Some(dir) = working_dir {
                cmd.current_dir(dir);
            }
            cmd.output()
        }
        None => {
            // Direct execution without shell - split command into program and args
            let parts: Vec<&str> = command.split_whitespace().collect();
            if parts.is_empty() {
                return CommandResult::ExecutionError {
                    message: "Empty command".to_string(),
                };
            }
            let mut cmd = Command::new(parts[0]);
            if parts.len() > 1 {
                cmd.args(&parts[1..]);
            }
            if let Some(dir) = working_dir {
                cmd.current_dir(dir);
            }
            cmd.output()
        }
    };

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
    shell_config: &ShellConfig,
    working_dir: Option<&str>,
) -> CommandResult {
    let mut attempt = 0;
    let mut current_delay_ms = retry_config.delay_ms;

    loop {
        let result = execute_shell_command(
            command,
            handler_name,
            timeout_secs,
            shell_config,
            working_dir,
        )
        .await;

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
    shell_config: &ShellConfig,
    working_dir: Option<&str>,
) -> CommandResult {
    info!(
        "Executing command for '{}' ({}): {}",
        handler_name, context, command
    );
    execute_shell_command_with_retry(
        command,
        handler_name,
        timeout_secs,
        retry_config,
        shell_config,
        working_dir,
    )
    .await
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

    fn default_shell() -> ShellConfig {
        ShellConfig::Simple("sh".to_string())
    }

    #[tokio::test]
    async fn test_execute_shell_command_success() {
        let result =
            execute_shell_command("echo 'test'", "test_handler", 30, &default_shell(), None).await;
        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_shell_command_failure() {
        let result =
            execute_shell_command("false", "test_handler", 30, &default_shell(), None).await;
        assert!(matches!(result, CommandResult::Failed { .. }));
    }

    #[tokio::test]
    async fn test_execute_shell_command_timeout() {
        let result =
            execute_shell_command("sleep 10", "test_handler", 1, &default_shell(), None).await;
        assert_eq!(result, CommandResult::Timeout);
    }

    #[tokio::test]
    async fn test_execute_with_retry_success_no_retry_needed() {
        let result = execute_shell_command_with_retry(
            "echo 'test'",
            "test_handler",
            30,
            &retry_config(3, 100),
            &default_shell(),
            None,
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
            &default_shell(),
            None,
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
        let result = execute_shell_command_with_retry(
            "false",
            "test_handler",
            30,
            &no_retry(),
            &default_shell(),
            None,
        )
        .await;
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
            &default_shell(),
            None,
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
            &default_shell(),
            None,
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

    // ============== Shell configuration tests ==============

    #[test]
    fn test_shell_config_default() {
        let shell = ShellConfig::default();
        // Default should be either from $SHELL or "sh"
        match shell {
            ShellConfig::Simple(s) => assert!(!s.is_empty()),
            ShellConfig::None => panic!("Default should not be None"),
        }
    }

    #[test]
    fn test_shell_config_get_shell_command_sh() {
        let shell = ShellConfig::Simple("sh".to_string());
        let (prog, arg) = shell.get_shell_command().unwrap();
        assert_eq!(prog, "sh");
        assert_eq!(arg, "-c");
    }

    #[test]
    fn test_shell_config_get_shell_command_bash() {
        let shell = ShellConfig::Simple("bash".to_string());
        let (prog, arg) = shell.get_shell_command().unwrap();
        assert_eq!(prog, "bash");
        assert_eq!(arg, "-c");
    }

    #[test]
    fn test_shell_config_get_shell_command_zsh() {
        let shell = ShellConfig::Simple("zsh".to_string());
        let (prog, arg) = shell.get_shell_command().unwrap();
        assert_eq!(prog, "zsh");
        assert_eq!(arg, "-c");
    }

    #[test]
    fn test_shell_config_get_shell_command_powershell() {
        let shell = ShellConfig::Simple("powershell".to_string());
        let (prog, arg) = shell.get_shell_command().unwrap();
        assert_eq!(prog, "powershell");
        assert_eq!(arg, "-Command");
    }

    #[test]
    fn test_shell_config_get_shell_command_cmd() {
        let shell = ShellConfig::Simple("cmd".to_string());
        let (prog, arg) = shell.get_shell_command().unwrap();
        assert_eq!(prog, "cmd");
        assert_eq!(arg, "/c");
    }

    #[test]
    fn test_shell_config_none() {
        let shell = ShellConfig::None;
        assert!(shell.get_shell_command().is_none());
    }

    #[tokio::test]
    async fn test_execute_with_bash_shell() {
        let shell = ShellConfig::Simple("bash".to_string());
        let result =
            execute_shell_command("echo 'test with bash'", "test_handler", 30, &shell, None).await;
        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_with_custom_shell() {
        // Test with an explicitly specified shell
        let shell = ShellConfig::Simple("sh".to_string());
        let result = execute_shell_command("echo $0", "test_handler", 30, &shell, None).await;
        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_with_working_directory() {
        // Test command execution with a working directory
        let result =
            execute_shell_command("pwd", "test_handler", 30, &default_shell(), Some("/tmp")).await;
        assert_eq!(result, CommandResult::Success);
    }

    #[tokio::test]
    async fn test_execute_with_invalid_working_directory() {
        // Test command execution with a non-existent working directory
        let result = execute_shell_command(
            "pwd",
            "test_handler",
            30,
            &default_shell(),
            Some("/nonexistent/path/that/does/not/exist"),
        )
        .await;
        // Should fail due to invalid directory
        assert!(matches!(result, CommandResult::ExecutionError { .. }));
    }
}
