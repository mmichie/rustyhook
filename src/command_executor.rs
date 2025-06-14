use log::{error, info};
use std::process::Command;

/// Execute a shell command and log the results
pub fn execute_shell_command(command: &str, handler_name: &str) {
    info!("Executing command for '{}': {}", handler_name, command);
    
    match Command::new("sh").arg("-c").arg(command).output() {
        Ok(output) => {
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
        Err(e) => {
            error!("Failed to execute command for '{}': {:?}", handler_name, e);
        }
    }
}

/// Execute a shell command with additional context information
pub fn execute_shell_command_with_context(command: &str, handler_name: &str, context: &str) {
    info!("Executing command for '{}' ({}): {}", handler_name, context, command);
    execute_shell_command(command, handler_name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_shell_command_success() {
        execute_shell_command("echo 'test'", "test_handler");
    }

    #[test]
    fn test_execute_shell_command_failure() {
        execute_shell_command("false", "test_handler");
    }
}