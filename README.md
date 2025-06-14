# RustyHook

A lightweight, event-driven automation framework that monitors multiple event sources and executes shell commands in response. Written in Rust for performance and reliability.

## What is RustyHook?

RustyHook is an all-in-one automation tool that can:
- **Monitor filesystems** for changes
- **Listen for webhooks** via HTTP endpoints  
- **Execute cron jobs** on schedules
- **Poll AWS SQS queues** for messages

When any of these events occur, RustyHook executes your configured shell commands.

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

## Quick Start

1. Create a configuration file:

```yaml
# config.yml
handlers:
  - type: Webhook
    name: Deploy on Push
    options:
      type: Webhook
      port: 3000
      path: "/deploy"
    shell: "cd /app && git pull && npm install && npm run build"

  - type: Cron
    name: Backup Database
    options:
      type: Cron
      cron_expression: "0 2 * * *"  # 2 AM daily
    shell: "pg_dump mydb > /backups/mydb-$(date +%Y%m%d).sql"

  - type: Filesystem
    name: Compile Sass
    options:
      type: Filesystem
      path: "./src/styles"
      event_type: "write"
    shell: "sass src/styles/main.scss public/css/main.css"
```

2. Run RustyHook:

```bash
cargo run -- -c config.yml
```

## Installation

### From Source

```bash
git clone https://github.com/mmichie/rustyhook.git
cd rustyhook
cargo build --release
./target/release/rustyhook -c your-config.yml
```

## Configuration

Each handler requires:
- `type`: Event type (`Filesystem`, `Webhook`, `Cron`, or `SQS`)
- `name`: Human-readable name for logging
- `options`: Type-specific configuration
- `shell`: Command to execute when triggered

See the [examples](./examples) directory for complete configuration examples.

## Use Cases

- **Continuous Deployment**: Deploy when webhook received from GitHub
- **Asset Compilation**: Rebuild CSS/JS when source files change
- **Scheduled Backups**: Run database backups on a schedule
- **Queue Processing**: Process messages from AWS SQS
- **File Synchronization**: Sync files when changes detected
- **Log Rotation**: Archive logs on a schedule
- **Health Checks**: Periodic monitoring scripts

## Requirements

- Rust 1.70+ (for building)
- Linux, macOS, or Windows
- AWS credentials (for SQS handler only)

## License

MIT