# CLAUDE.md

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
```

## Architecture Overview

Rustyhook is an event-driven automation framework that monitors various event sources and executes shell commands in response. The application uses an asynchronous architecture built on Tokio.

### Core Components

1. **Main Entry Point** (`src/main.rs`): CLI argument parsing and handler initialization
2. **Configuration** (`src/config.rs`): YAML configuration parsing with `EventHandler` and `EventType` enums
3. **Event Handlers** (`src/handlers/`):
   - `cron_handler.rs`: Executes commands on cron schedules
   - `filesystem_handler.rs`: Monitors file system changes using `notify`
   - `sqs_handler.rs`: Polls AWS SQS queues for messages
   - `webhook_handler.rs`: HTTP server listening for webhook requests

### Event Flow

1. Application reads YAML configuration file specifying multiple handlers
2. Each handler runs in its own async task via `tokio::spawn`
3. Handlers wait for their specific events (file changes, webhooks, cron triggers, SQS messages)
4. When triggered, handlers execute the configured shell command via `std::process::Command`

### Configuration Format

Handlers are defined in YAML with four required fields:
- `name`: Human-readable name for the handler
- `type`: Event type (Filesystem, Webhook, SQS, Cron)
- `options`: Type-specific configuration
- `shell`: Command to execute when triggered

Example configuration available in `examples/` directory.