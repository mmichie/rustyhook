use log::{error, info};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

/// Execute a shell command with timeout and log the results
pub async fn execute_shell_command(command: &str, handler_name: &str, timeout_secs: u64) {
    info!("Executing command for '{}' (timeout: {}s): {}", handler_name, timeout_secs, command);

    let timeout_duration = Duration::from_secs(timeout_secs);
    let command_future = Command::new("sh").arg("-c").arg(command).output();

    match timeout(timeout_duration, command_future).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                info!("Command for '{}' executed successfully", handler_name);
                if !output.stdout.is_empty() {
                    info!("Output: {}", String::from_utf8_lossy(&output.stdout));
                }
            } else {
                error!("Command for '{}' failed with status: {}", handler_name, output.status);
                if !output.stderr.is_empty() {
                    error!("Error output: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
        }
        Ok(Err(e)) => {
            error!("Failed to execute command for '{}': {:?}", handler_name, e);
        }
        Err(_) => {
            error!("Command for '{}' timed out after {} seconds", handler_name, timeout_secs);
        }
    }
}

/// Execute a shell command with additional context information
pub async fn execute_shell_command_with_context(command: &str, handler_name: &str, context: &str, timeout_secs: u64) {
    info!("Executing command for '{}' ({}): {}", handler_name, context, command);
    execute_shell_command(command, handler_name, timeout_secs).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execute_shell_command_success() {
        execute_shell_command("echo 'test'", "test_handler", 30).await;
    }

    #[tokio::test]
    async fn test_execute_shell_command_failure() {
        execute_shell_command("false", "test_handler", 30).await;
    }

    #[tokio::test]
    async fn test_execute_shell_command_timeout() {
        execute_shell_command("sleep 10", "test_handler", 1).await;
    }
}