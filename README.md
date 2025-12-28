# RustyHook

A lightweight, event-driven automation framework that monitors multiple event sources and executes shell commands in response. Written in Rust for performance and reliability.

## What is RustyHook?

RustyHook is an all-in-one automation tool that can:
- **Monitor filesystems** for changes (with include/exclude patterns)
- **Listen for webhooks** via HTTP endpoints (with auth and rate limiting)
- **Execute cron jobs** on schedules
- **Poll AWS SQS queues** for messages

When any of these events occur, RustyHook executes your configured shell commands.

## Features

- **Unified Configuration** - Single YAML file for all event handlers
- **Authentication** - Optional token-based auth for webhook endpoints
- **Rate Limiting** - Protect webhooks from abuse with configurable limits
- **Retry Logic** - Configurable retry with exponential backoff
- **Event Forwarding** - Chain handlers together for complex workflows
- **Glob Patterns** - Include/exclude files for filesystem monitoring
- **Working Directory** - Execute commands in specific directories
- **Graceful Shutdown** - Clean handler termination on SIGINT

## Installation

### From Releases

Download pre-built binaries from [GitHub Releases](https://github.com/mmichie/rustyhook/releases):

```bash
# Linux (x86_64)
curl -LO https://github.com/mmichie/rustyhook/releases/latest/download/rustyhook-x86_64-unknown-linux-gnu.tar.gz
tar xzf rustyhook-x86_64-unknown-linux-gnu.tar.gz

# macOS (Apple Silicon)
curl -LO https://github.com/mmichie/rustyhook/releases/latest/download/rustyhook-aarch64-apple-darwin.tar.gz
tar xzf rustyhook-aarch64-apple-darwin.tar.gz
```

### From Source

```bash
git clone https://github.com/mmichie/rustyhook.git
cd rustyhook
cargo build --release
./target/release/rustyhook -c your-config.yml
```

## Quick Start

Create a configuration file:

```yaml
# config.yml
handlers:
  - type: Webhook
    name: deploy-webhook
    options:
      type: http
      port: 8080
      path: "/deploy"
      auth_token: "secret-token"    # Optional: require X-Auth-Token header
      rate_limit: 10                 # Optional: max 10 requests/second
      health_path: "/health"         # Optional: health check endpoint
    shell: "cd /app && git pull && ./deploy.sh"
    timeout: 300                     # Command timeout in seconds
    working_dir: "/app"              # Working directory for command

  - type: Cron
    name: daily-backup
    options:
      type: cron
      cron_expression: "0 2 * * *"   # 2 AM daily
    shell: "pg_dump mydb > /backups/mydb-$(date +%Y%m%d).sql"
    retry:
      max_retries: 3
      initial_delay_ms: 1000
      max_delay_ms: 30000

  - type: Filesystem
    name: compile-assets
    options:
      type: filesystem
      path: "./src/styles"
      include: ["*.scss", "*.sass"]  # Only watch these patterns
      exclude: ["_*.scss"]           # Ignore partials
      debounce_ms: 500               # Wait for changes to settle
    shell: "sass src/styles/main.scss public/css/main.css"

  - type: SQS
    name: process-queue
    options:
      type: sqs
      queue_url: "https://sqs.us-east-1.amazonaws.com/123456789/my-queue"
      poll_interval: 10
    shell: "python process_message.py"
```

Run RustyHook:

```bash
rustyhook -c config.yml
```

## Configuration Reference

### Common Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `type` | string | required | Event type: `Filesystem`, `Webhook`, `Cron`, `SQS` |
| `name` | string | required | Handler name (must be unique) |
| `shell` | string | required | Command to execute |
| `timeout` | integer | 300 | Command timeout in seconds |
| `working_dir` | string | current | Working directory for command |
| `shell_program` | string | $SHELL or "sh" | Shell to use (sh, bash, zsh, etc.) |
| `forward_to` | list | [] | Handler names to forward events to |
| `retry.max_retries` | integer | 0 | Max retry attempts |
| `retry.initial_delay_ms` | integer | 1000 | Initial retry delay |
| `retry.max_delay_ms` | integer | 30000 | Maximum retry delay |

### Webhook Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `port` | integer | required | HTTP port to listen on |
| `path` | string | required | URL path to handle |
| `auth_token` | string | none | Required X-Auth-Token header value |
| `rate_limit` | integer | none | Max requests per second |
| `health_path` | string | none | Health check path (bypasses auth and rate limit) |

### Filesystem Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `path` | string | required | Directory to watch |
| `include` | list | [] | Glob patterns to include |
| `exclude` | list | [] | Glob patterns to exclude |
| `debounce_ms` | integer | 100 | Debounce delay in milliseconds |

### Cron Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `cron_expression` | string | required | Cron expression (5 or 6 fields) |

### SQS Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `queue_url` | string | required | Full SQS queue URL |
| `poll_interval` | integer | required | Seconds between polls |

## Why RustyHook?

### The Problem

Most automation tools focus on a single event type. You might use:
- `cron` for scheduled tasks
- `watchman` or `nodemon` for file watching
- A web server for webhooks
- Custom scripts for SQS polling

This leads to fragmented automation infrastructure with different tools, configurations, and monitoring approaches.

### The Solution

RustyHook unifies these event sources into a single, lightweight binary with:
- **Simple YAML configuration** - One config file for all your automation
- **Parallel execution** - All handlers run independently
- **Minimal dependencies** - Single binary deployment
- **Shell command focus** - Execute any command or script

## How It Compares

| Tool | Event Types | Deployment | Best For |
|------|------------|------------|----------|
| **RustyHook** | Files, Webhooks, Cron, SQS | Single binary | Unified local automation |
| Argo Events | 20+ sources | Kubernetes required | K8s environments |
| Ansible Event-Driven | Multiple | Enterprise platform | Large infrastructure |
| Watchman | Files only | Daemon + client | File watching |
| nodemon | Files only | Node.js required | Node development |
| cron | Time only | Built into OS | Scheduled tasks |

## Use Cases

- **Continuous Deployment**: Deploy when webhook received from GitHub
- **Asset Compilation**: Rebuild CSS/JS when source files change
- **Scheduled Backups**: Run database backups on a schedule
- **Queue Processing**: Process messages from AWS SQS
- **File Synchronization**: Sync files when changes detected
- **Log Rotation**: Archive logs on a schedule
- **Health Checks**: Periodic monitoring scripts

## Requirements

- Rust 1.70+ (for building from source)
- Linux or macOS
- AWS credentials (for SQS handler only, via environment variables)

## License

MIT
