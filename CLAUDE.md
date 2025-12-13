# CLAUDE.md

**Note**: This project uses bd (beads) for issue tracking. Use `bd` commands instead of markdown TODOs. See AGENTS.md for workflow details.

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# Build the project
cargo build

# Build for release
cargo build --release

# Check code compilation without building
cargo check

# Run the application with default config
cargo run

# Run with custom config file
cargo run -- -c /path/to/config.yml

# Format code
cargo fmt

# Run clippy linter
cargo clippy

# Run tests
cargo test
```

## Architecture Overview

Rustyhook is an event-driven automation framework that monitors various event sources and executes shell commands in response. The application uses an asynchronous architecture built on Tokio.

### Core Components

1. **Main Entry Point** (`src/main.rs`): CLI argument parsing, handler initialization, and graceful shutdown orchestration
2. **Configuration** (`src/config.rs`): YAML configuration parsing with `EventHandler` and `EventType` enums
3. **Command Executor** (`src/command_executor.rs`): Centralized shell command execution via `sh -c`
4. **Event Handlers** (`src/handlers/`):
   - `cron_handler.rs`: Executes commands on cron schedules using `chrono` and `cron` crate
   - `filesystem_handler.rs`: Monitors file system changes using `notify` crate with kqueue backend on macOS
   - `sqs_handler.rs`: Polls AWS SQS queues for messages using `rusoto_sqs`
   - `webhook_handler.rs`: HTTP server listening for webhook requests using `hyper`

### Concurrency and Shutdown Model

- All handlers run as independent `tokio::spawn` tasks and execute concurrently
- Uses `broadcast::channel` for graceful shutdown coordination
- On SIGINT/Ctrl+C, shutdown signal is broadcast to all handlers via `shutdown_tx`
- Each handler receives its own `shutdown_rx` and uses `tokio::select!` to race between normal operation and shutdown
- Main waits up to 30 seconds for clean handler termination before forcing exit
- Handler tasks are collected as `JoinHandle<()>` and awaited with `join_all`

### Event Flow

1. Application reads YAML configuration file specifying multiple handlers
2. Each handler is spawned into its own async task via `tokio::spawn`
3. Handlers wait for their specific events (file changes, webhooks, cron triggers, SQS messages)
4. When triggered, handlers call `execute_shell_command()` which runs commands via `Command::new("sh").arg("-c")`
5. All command output and errors are logged using the `log` crate

### Configuration Format

Handlers are defined in YAML with four required fields:
- `name`: Human-readable name for the handler
- `type`: Event type (Filesystem, Webhook, SQS, Cron)
- `options`: Type-specific configuration (includes nested `type` field and handler-specific options)
- `shell`: Command to execute when triggered

Example configuration available in `examples/` directory.