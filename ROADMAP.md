# RustyHook Roadmap

This document outlines planned features and improvements for RustyHook.

## Phase 1: Core Stability (Current Focus)
- [x] Basic event handlers (Filesystem, Webhook, Cron, SQS)
- [x] Shell command execution
- [x] Cross-platform filesystem monitoring
- [x] Graceful shutdown support
- [ ] Command timeout handling
- [ ] Retry logic with exponential backoff
- [ ] Comprehensive test coverage
- [ ] Binary releases for major platforms

## Phase 2: Advanced Composition Features

### 2.1 Event Forwarding Between Handlers
Enable handlers to trigger other handlers directly without shell commands.

```yaml
handlers:
  - type: Filesystem
    name: Watch Logs
    id: log-watcher
    options:
      path: "/var/log/app"
    shell: "parse-log.sh"
    forward_to: ["log-processor", "alert-checker"]
    
  - type: Custom
    name: Process Logs
    id: log-processor
    shell: "aggregate-logs.sh"
```

**Implementation needs:**
- Handler ID system
- Internal event bus
- Event data passing between handlers

### 2.2 Conditional Execution
Execute different commands based on previous command results.

```yaml
handlers:
  - type: Webhook
    name: Deploy Pipeline
    options:
      port: 3000
      path: "/deploy"
    shell: "run-tests.sh"
    on_success:
      shell: "deploy.sh"
      forward_to: ["notify-success"]
    on_failure:
      shell: "rollback.sh"
      forward_to: ["notify-failure"]
      retry: 3
```

**Implementation needs:**
- Exit code tracking
- Conditional configuration parsing
- State management for retries

### 2.3 Handler Dependencies
Ensure handlers execute in specific order or wait for others.

```yaml
handlers:
  - type: Filesystem
    name: Compile Assets
    id: asset-compiler
    options:
      path: "./assets"
    shell: "compile-assets.sh"
    
  - type: Filesystem
    name: Build Application
    options:
      path: "./src"
    depends_on: ["asset-compiler"]
    shell: "build.sh"
```

**Implementation needs:**
- Dependency graph resolution
- Handler state tracking
- Deadlock prevention

### 2.4 Event Filtering and Routing
Route events to different commands based on patterns.

```yaml
handlers:
  - type: Filesystem
    name: File Processor
    options:
      path: "./uploads"
    routes:
      - pattern: "*.jpg|*.png"
        shell: "process-image.sh $FILE"
      - pattern: "*.csv"
        shell: "import-csv.sh $FILE"
        on_error: continue
      - pattern: "temp/*"
        action: ignore
      - pattern: "*"
        shell: "process-other.sh $FILE"
```

**Implementation needs:**
- Pattern matching engine
- Variable substitution ($FILE, $EVENT, etc.)
- Action types (execute, ignore, forward)

## Phase 3: Advanced Features

### 3.1 Event Context and Variables
Pass rich context between handlers and to shell commands.

```yaml
handlers:
  - type: SQS
    name: Queue Processor
    options:
      queue_url: "https://sqs..."
    shell: "process.sh"
    exports:
      - MESSAGE_ID
      - MESSAGE_BODY
    
  - type: Webhook
    name: Status Reporter
    options:
      port: 4000
      path: "/status"
    shell: 'echo "Last message: $MESSAGE_ID"'
```

### 3.2 Handler Plugins
Support custom handler types via plugins.

```yaml
handlers:
  - type: Plugin
    name: Kafka Consumer
    plugin: "rustyhook-kafka"
    options:
      brokers: "localhost:9092"
      topic: "events"
    shell: "process-kafka.sh"
```

### 3.3 Distributed Mode
Run handlers across multiple machines with coordination.

```yaml
cluster:
  mode: distributed
  coordinator: "redis://coordinator:6379"
  node_id: "worker-1"

handlers:
  - type: Cron
    name: Distributed Job
    cluster:
      mode: leader  # Only runs on leader node
    options:
      cron_expression: "0 * * * *"
    shell: "hourly-job.sh"
```

## Phase 4: Observability and Management

### 4.1 Web UI and API
- Real-time handler status dashboard
- Execution history and logs
- Dynamic handler management
- Metrics and monitoring

### 4.2 Advanced Logging and Tracing
- Structured logging with trace IDs
- OpenTelemetry support
- Log aggregation endpoints

### 4.3 Testing Framework
```yaml
handlers:
  - type: Webhook
    name: API Handler
    options:
      port: 3000
      path: "/api"
    shell: "process-api.sh"
    tests:
      - name: "Should process valid request"
        input:
          method: POST
          body: '{"test": true}'
        expect:
          exit_code: 0
          output_contains: "success"
```

## Phase 5: Enterprise Features

### 5.1 Security Enhancements
- Handler authentication and authorization
- Encrypted configuration
- Audit logging
- Secret management integration

### 5.2 High Availability
- Handler state replication
- Automatic failover
- Load balancing for webhooks

### 5.3 Integration Ecosystem
- Terraform provider
- Kubernetes operator
- GitHub Actions
- CI/CD integrations

## Contributing

We welcome contributions! Priority areas:
1. Test coverage for existing handlers
2. Documentation and examples
3. Platform-specific testing
4. Performance benchmarks

## Version Targets

- **v0.2.0**: Phase 1 completion (stability)
- **v0.3.0**: Event forwarding & conditional execution
- **v0.4.0**: Dependencies & routing
- **v0.5.0**: Context & variables
- **v1.0.0**: Production-ready with core composition features
- **v2.0.0**: Distributed mode & plugins

## Design Principles

1. **Simplicity First**: Core functionality should remain simple
2. **Backward Compatibility**: New features shouldn't break existing configs
3. **Performance**: Composition shouldn't add significant overhead
4. **Reliability**: Complex compositions should fail gracefully
5. **Debuggability**: Clear logs and error messages for composed handlers