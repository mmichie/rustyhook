use crate::command_executor::execute_shell_command_with_context;
use crate::config::{RetryConfig, ShellConfig};
use crate::event::Event;
use crate::event_bus::EventBus;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use log::{debug, error, info, warn};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, Mutex};

/// Constant-time string comparison to prevent timing attacks.
/// Returns true if both strings are equal, comparing all bytes
/// regardless of where differences occur.
fn constant_time_compare(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    if a_bytes.len() != b_bytes.len() {
        return false;
    }

    let mut result: u8 = 0;
    for (x, y) in a_bytes.iter().zip(b_bytes.iter()) {
        result |= x ^ y;
    }
    result == 0
}

/// Token bucket rate limiter for controlling request throughput.
/// Allows bursting up to `capacity` requests, then refills at `rate` tokens per second.
pub struct RateLimiter {
    /// Maximum tokens (burst capacity)
    capacity: u64,
    /// Tokens added per second
    rate: u64,
    /// Current token count (scaled by 1000 for precision)
    tokens: AtomicU64,
    /// Last update timestamp in milliseconds
    last_update: Mutex<u64>,
}

impl RateLimiter {
    /// Create a new rate limiter.
    /// - `rate`: requests per second allowed
    /// - `capacity`: burst capacity (defaults to rate if None)
    pub fn new(rate: u64, capacity: Option<u64>) -> Self {
        let capacity = capacity.unwrap_or(rate);
        Self {
            capacity,
            rate,
            // Start with full bucket (scaled by 1000)
            tokens: AtomicU64::new(capacity * 1000),
            last_update: Mutex::new(Self::now_millis()),
        }
    }

    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Try to acquire a token. Returns true if allowed, false if rate limited.
    pub async fn try_acquire(&self) -> bool {
        let mut last_update = self.last_update.lock().await;
        let now = Self::now_millis();
        let elapsed = now.saturating_sub(*last_update);

        // Calculate tokens to add based on elapsed time
        // tokens_to_add = rate * (elapsed_ms / 1000), scaled by 1000 for precision
        let tokens_to_add = self.rate * elapsed;

        // Get current tokens and add refill
        let current = self.tokens.load(Ordering::Relaxed);
        let max_tokens = self.capacity * 1000;
        let new_tokens = (current + tokens_to_add).min(max_tokens);

        // Try to consume one token (1000 in scaled units)
        if new_tokens >= 1000 {
            self.tokens.store(new_tokens - 1000, Ordering::Relaxed);
            *last_update = now;
            true
        } else {
            // Update tokens even if we can't consume (for refill tracking)
            self.tokens.store(new_tokens, Ordering::Relaxed);
            *last_update = now;
            false
        }
    }
}

/// Shared state for webhook request handling
struct WebhookState {
    path: String,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    retry_config: Arc<RetryConfig>,
    shell_config: ShellConfig,
    working_dir: Option<String>,
    event_bus: Arc<EventBus>,
    forward_to: Vec<String>,
    /// Optional authentication token - if set, requests must include matching X-Auth-Token header
    auth_token: Option<String>,
    /// Optional rate limiter - if set, requests exceeding the rate will be rejected
    rate_limiter: Option<Arc<RateLimiter>>,
    /// Optional health check path - requests to this path return 200 OK without auth/rate limit
    health_path: Option<String>,
}

// Function to start the webhook listener
#[allow(clippy::too_many_arguments)]
pub async fn webhook_listener(
    port: u16,
    path: String,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    retry_config: RetryConfig,
    shell_config: ShellConfig,
    working_dir: Option<String>,
    mut shutdown_rx: broadcast::Receiver<()>,
    event_bus: Arc<EventBus>,
    mut event_rx: mpsc::UnboundedReceiver<Event>,
    forward_to: Vec<String>,
    auth_token: Option<String>,
    rate_limit: Option<u64>,
    health_path: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr: SocketAddr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener: TcpListener = match TcpListener::bind(&addr).await {
        Ok(listener) => {
            info!("Webhook server running on {}", addr);
            listener
        }
        Err(e) => {
            error!("Failed to bind webhook listener: {}", e);
            return Err(e.into());
        }
    };

    // Create rate limiter if configured
    let rate_limiter = rate_limit.map(|rate| {
        info!(
            "Rate limiting enabled for webhook '{}': {} requests/second",
            handler_name, rate
        );
        Arc::new(RateLimiter::new(rate, None))
    });

    // Log health check path if configured
    if let Some(ref hp) = health_path {
        info!(
            "Health check enabled for webhook '{}' at path: {}",
            handler_name, hp
        );
    }

    // Create shared state for request handlers
    let state = Arc::new(WebhookState {
        path,
        shell_command,
        handler_name: handler_name.clone(),
        timeout,
        retry_config: Arc::new(retry_config),
        shell_config,
        working_dir,
        event_bus: event_bus.clone(),
        forward_to: forward_to.clone(),
        auth_token,
        rate_limiter,
        health_path,
    });

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let io = TokioIo::new(stream);
                        let state_clone = Arc::clone(&state);
                        tokio::task::spawn(async move {
                            if let Err(err) = http1::Builder::new()
                                .serve_connection(
                                    io,
                                    service_fn(move |req| {
                                        let state_inner = Arc::clone(&state_clone);
                                        handle_webhook(req, state_inner)
                                    }),
                                )
                                .await
                            {
                                warn!("Error serving connection: {:?}", err);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Error accepting connection: {}", e);
                    }
                }
            }
            Some(forwarded_event) = event_rx.recv() => {
                // Handle forwarded events from other handlers
                info!(
                    "Webhook handler '{}' received forwarded event from '{}'",
                    handler_name, forwarded_event.source_handler
                );
                let context = format!("Forwarded from: {}", forwarded_event.source_handler);
                execute_shell_command_with_context(
                    &state.shell_command,
                    &handler_name,
                    &context,
                    state.timeout,
                    &state.retry_config,
                    &state.shell_config,
                    state.working_dir.as_deref(),
                ).await;

                // Forward to next handlers if configured
                if !forward_to.is_empty() {
                    for target in &forward_to {
                        if let Err(e) = event_bus.send(target, forwarded_event.clone()) {
                            warn!("Failed to forward event to '{}': {}", target, e);
                        }
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Webhook handler '{}' received shutdown signal", handler_name);
                break;
            }
        }
    }

    info!("Webhook handler '{}' shutting down", handler_name);
    Ok(())
}

/// Get the expected path for a handler (useful for testing)
#[cfg(test)]
pub fn get_expected_path(path: &str) -> String {
    path.to_string()
}

// Function to handle incoming webhooks
async fn handle_webhook(
    req: Request<hyper::body::Incoming>,
    state: Arc<WebhookState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    // Check for health check path first (no auth, no rate limit)
    if let Some(ref health_path) = state.health_path {
        if req.uri().path() == health_path {
            debug!("Health check request at {}", health_path);
            return Ok(Response::new(Full::new(Bytes::from("OK"))));
        }
    }

    if req.uri().path() == state.path {
        // Check rate limit if configured
        if let Some(ref limiter) = state.rate_limiter {
            if !limiter.try_acquire().await {
                warn!("Rate limit exceeded for webhook at {}", state.path);
                return Ok(Response::builder()
                    .status(StatusCode::TOO_MANY_REQUESTS)
                    .body(Full::new(Bytes::from("Too Many Requests")))
                    .unwrap());
            }
        }

        // Extract method and URI before consuming the request
        let method = req.method().to_string();
        let uri = req.uri().to_string();

        // Check authentication if configured (before consuming body)
        if let Some(expected_token) = &state.auth_token {
            let provided_token = req
                .headers()
                .get("X-Auth-Token")
                .and_then(|v| v.to_str().ok());

            match provided_token {
                Some(token) if constant_time_compare(token, expected_token) => {
                    debug!("Authentication successful for webhook at {}", state.path);
                }
                Some(_) => {
                    warn!("Invalid auth token provided for webhook at {}", state.path);
                    return Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Full::new(Bytes::from("Unauthorized")))
                        .unwrap());
                }
                None => {
                    warn!(
                        "Missing auth token for webhook at {} (X-Auth-Token header required)",
                        state.path
                    );
                    return Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Full::new(Bytes::from("Unauthorized")))
                        .unwrap());
                }
            }
        }

        // Read request body
        let body = match req.collect().await {
            Ok(collected) => String::from_utf8_lossy(&collected.to_bytes()).to_string(),
            Err(e) => {
                warn!("Failed to read request body: {}", e);
                String::new()
            }
        };

        // Process the webhook request
        info!("Webhook received at {}", state.path);

        let context = if body.is_empty() {
            format!("Method: {}, URI: {}", method, uri)
        } else {
            format!("Method: {}, URI: {}, Body: {}", method, uri, body)
        };

        // Execute the configured shell command
        execute_shell_command_with_context(
            &state.shell_command,
            &state.handler_name,
            &context,
            state.timeout,
            &state.retry_config,
            &state.shell_config,
            state.working_dir.as_deref(),
        )
        .await;

        // Forward event if configured
        if !state.forward_to.is_empty() {
            let event = Event::from_webhook(&state.handler_name, &method, &uri, &body);

            for target in &state.forward_to {
                if let Err(e) = state.event_bus.send(target, event.clone()) {
                    warn!("Failed to forward event to '{}': {}", target, e);
                } else {
                    debug!("Forwarded webhook event to '{}'", target);
                }
            }
        }

        Ok(Response::new(Full::new(Bytes::from("Webhook received"))))
    } else {
        // Respond with Not Found for requests on other paths
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RetryConfig, ShellConfig};
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::sleep;

    /// Find an available port for testing
    async fn find_available_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    /// Helper to create test dependencies
    fn create_test_deps() -> (Arc<EventBus>, mpsc::UnboundedReceiver<Event>) {
        let event_bus = Arc::new(EventBus::new());
        let (tx, rx) = mpsc::unbounded_channel();
        // We don't register with the bus since we're just testing the handler
        drop(tx);
        (event_bus, rx)
    }

    #[tokio::test]
    async fn test_webhook_valid_path_returns_200() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None, // No auth token
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Make HTTP request to valid path
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/webhook", port))
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "Webhook received");

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[tokio::test]
    async fn test_webhook_invalid_path_returns_404() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None, // No auth token
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Make HTTP request to invalid path
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/invalid", port))
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 404);
        assert_eq!(response.text().await.unwrap(), "Not Found");

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[tokio::test]
    async fn test_webhook_shutdown() {
        let port = find_available_port().await;
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            "/test".to_string(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None, // No auth token
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Send shutdown signal
        let _ = shutdown_tx.send(());

        // Handler should complete within timeout
        let result = tokio::time::timeout(Duration::from_secs(2), handler).await;
        assert!(result.is_ok(), "Handler should shut down within timeout");
    }

    #[tokio::test]
    async fn test_webhook_multiple_requests() {
        let port = find_available_port().await;
        let path = "/api/hook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None, // No auth token
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

        // Send multiple requests
        for i in 0..3 {
            let response = client
                .get(format!("http://127.0.0.1:{}/api/hook", port))
                .send()
                .await
                .expect(&format!("Failed to send request {}", i));

            assert_eq!(response.status(), 200);
        }

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[tokio::test]
    async fn test_webhook_post_request() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None, // No auth token
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Make POST request
        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://127.0.0.1:{}/webhook", port))
            .body("test payload")
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 200);

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[test]
    fn test_get_expected_path() {
        assert_eq!(get_expected_path("/webhook"), "/webhook");
        assert_eq!(get_expected_path("/api/v1/hook"), "/api/v1/hook");
    }

    #[tokio::test]
    async fn test_webhook_auth_token_valid() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            Some("secret-token".to_string()),
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Make request with valid auth token
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/webhook", port))
            .header("X-Auth-Token", "secret-token")
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "Webhook received");

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[tokio::test]
    async fn test_webhook_auth_token_missing() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            Some("secret-token".to_string()),
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Make request WITHOUT auth token - should be rejected
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/webhook", port))
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 401);
        assert_eq!(response.text().await.unwrap(), "Unauthorized");

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[tokio::test]
    async fn test_webhook_auth_token_invalid() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            Some("secret-token".to_string()),
            None, // No rate limit
            None, // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        // Make request with WRONG auth token - should be rejected
        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/webhook", port))
            .header("X-Auth-Token", "wrong-token")
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 401);
        assert_eq!(response.text().await.unwrap(), "Unauthorized");

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[test]
    fn test_constant_time_compare() {
        // Equal strings
        assert!(constant_time_compare("secret", "secret"));
        assert!(constant_time_compare("", ""));
        assert!(constant_time_compare("a", "a"));

        // Different strings of same length
        assert!(!constant_time_compare("secret", "secreT"));
        assert!(!constant_time_compare("aaaaaa", "aaaaab"));
        assert!(!constant_time_compare("a", "b"));

        // Different lengths
        assert!(!constant_time_compare("secret", "secrets"));
        assert!(!constant_time_compare("secrets", "secret"));
        assert!(!constant_time_compare("", "a"));
        assert!(!constant_time_compare("a", ""));
    }

    #[tokio::test]
    async fn test_rate_limiter_allows_requests_within_limit() {
        let limiter = RateLimiter::new(10, None); // 10 requests per second

        // Should allow up to 10 requests immediately (burst capacity)
        for _ in 0..10 {
            assert!(limiter.try_acquire().await);
        }
    }

    #[tokio::test]
    async fn test_rate_limiter_blocks_excess_requests() {
        let limiter = RateLimiter::new(2, Some(2)); // 2 requests per second, burst of 2

        // First 2 should succeed (burst capacity)
        assert!(limiter.try_acquire().await);
        assert!(limiter.try_acquire().await);

        // Third should fail (no tokens left)
        assert!(!limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_rate_limiter_refills_over_time() {
        let limiter = RateLimiter::new(10, Some(1)); // 10/sec rate, 1 burst

        // Use the one token
        assert!(limiter.try_acquire().await);
        assert!(!limiter.try_acquire().await);

        // Wait 150ms for refill (should get at least 1 token at 10/sec)
        sleep(Duration::from_millis(150)).await;

        // Should have refilled
        assert!(limiter.try_acquire().await);
    }

    #[tokio::test]
    async fn test_webhook_rate_limit_returns_429() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None,    // No auth token
            Some(2), // Rate limit: 2 requests per second
            None,    // No health path
        ));

        // Wait for server to start
        sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

        // First 2 requests should succeed (burst capacity)
        for _ in 0..2 {
            let response = client
                .get(format!("http://127.0.0.1:{}/webhook", port))
                .send()
                .await
                .expect("Failed to send request");
            assert_eq!(response.status(), 200);
        }

        // Third request should be rate limited
        let response = client
            .get(format!("http://127.0.0.1:{}/webhook", port))
            .send()
            .await
            .expect("Failed to send request");
        assert_eq!(response.status(), 429);
        assert_eq!(response.text().await.unwrap(), "Too Many Requests");

        // Shutdown
        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    // ============== Handler Forwarding Integration Tests ==============

    use tempfile::TempDir;

    async fn wait_for_marker(path: &std::path::Path, timeout_secs: u64) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        while tokio::time::Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    async fn test_webhook_forwards_event_to_handler() {
        let marker_dir = TempDir::new().expect("Failed to create marker dir");
        let target_marker = marker_dir.path().join("target_received.marker");

        let (shutdown_tx, _) = broadcast::channel(1);
        let event_bus = Arc::new(EventBus::new());

        // Register both handlers with EventBus
        let source_rx = event_bus.register("webhook-source").unwrap();
        let target_rx = event_bus.register("webhook-target").unwrap();

        // Start target webhook handler (listens for forwarded events)
        let target_port = find_available_port().await;
        let target_shell = format!("touch '{}'", target_marker.to_string_lossy());
        let target_handler = tokio::spawn(webhook_listener(
            target_port,
            "/target".to_string(),
            target_shell,
            "webhook-target".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            target_rx,
            Vec::new(), // No further forwarding
            None,
            None,
            None, // No health path
        ));

        // Start source webhook handler (forwards to target)
        let source_port = find_available_port().await;
        let source_handler = tokio::spawn(webhook_listener(
            source_port,
            "/source".to_string(),
            "echo 'source executed'".to_string(),
            "webhook-source".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            source_rx,
            vec!["webhook-target".to_string()], // Forward to target
            None,
            None,
            None, // No health path
        ));

        // Wait for servers to start
        sleep(Duration::from_millis(200)).await;

        // Trigger source webhook
        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://127.0.0.1:{}/source", source_port))
            .body("test payload")
            .send()
            .await
            .expect("Failed to send webhook request");

        assert_eq!(response.status(), 200);

        // Wait for forwarded event to execute target command
        let executed = wait_for_marker(&target_marker, 3).await;

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), source_handler).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), target_handler).await;

        assert!(
            executed,
            "Forwarded event should trigger target handler command"
        );
    }

    #[tokio::test]
    async fn test_chain_forwarding() {
        let marker_dir = TempDir::new().expect("Failed to create marker dir");
        let marker_a = marker_dir.path().join("a.marker");
        let marker_b = marker_dir.path().join("b.marker");
        let marker_c = marker_dir.path().join("c.marker");

        let (shutdown_tx, _) = broadcast::channel(1);
        let event_bus = Arc::new(EventBus::new());

        // Register all handlers
        let rx_a = event_bus.register("handler-a").unwrap();
        let rx_b = event_bus.register("handler-b").unwrap();
        let rx_c = event_bus.register("handler-c").unwrap();

        // Start handler C (end of chain, no forwarding)
        let port_c = find_available_port().await;
        let handler_c = tokio::spawn(webhook_listener(
            port_c,
            "/c".to_string(),
            format!("touch '{}'", marker_c.to_string_lossy()),
            "handler-c".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            rx_c,
            Vec::new(), // End of chain
            None,
            None,
            None, // No health path
        ));

        // Start handler B (forwards to C)
        let port_b = find_available_port().await;
        let handler_b = tokio::spawn(webhook_listener(
            port_b,
            "/b".to_string(),
            format!("touch '{}'", marker_b.to_string_lossy()),
            "handler-b".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            rx_b,
            vec!["handler-c".to_string()],
            None,
            None,
            None, // No health path
        ));

        // Start handler A (forwards to B)
        let port_a = find_available_port().await;
        let handler_a = tokio::spawn(webhook_listener(
            port_a,
            "/a".to_string(),
            format!("touch '{}'", marker_a.to_string_lossy()),
            "handler-a".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            rx_a,
            vec!["handler-b".to_string()],
            None,
            None,
            None, // No health path
        ));

        sleep(Duration::from_millis(200)).await;

        // Trigger handler A
        let client = reqwest::Client::new();
        client
            .post(format!("http://127.0.0.1:{}/a", port_a))
            .send()
            .await
            .expect("Failed to send request");

        // Wait for chain to complete
        sleep(Duration::from_millis(500)).await;

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler_a).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), handler_b).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), handler_c).await;

        // All three markers should exist
        assert!(marker_a.exists(), "Handler A should have executed");
        assert!(
            marker_b.exists(),
            "Handler B should have received forwarded event"
        );
        assert!(
            marker_c.exists(),
            "Handler C should have received forwarded event"
        );
    }

    #[tokio::test]
    async fn test_fanout_forwarding() {
        let marker_dir = TempDir::new().expect("Failed to create marker dir");
        let marker_b = marker_dir.path().join("b.marker");
        let marker_c = marker_dir.path().join("c.marker");

        let (shutdown_tx, _) = broadcast::channel(1);
        let event_bus = Arc::new(EventBus::new());

        let rx_a = event_bus.register("source").unwrap();
        let rx_b = event_bus.register("target-b").unwrap();
        let rx_c = event_bus.register("target-c").unwrap();

        // Start target B
        let port_b = find_available_port().await;
        let handler_b = tokio::spawn(webhook_listener(
            port_b,
            "/b".to_string(),
            format!("touch '{}'", marker_b.to_string_lossy()),
            "target-b".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            rx_b,
            Vec::new(),
            None,
            None,
            None, // No health path
        ));

        // Start target C
        let port_c = find_available_port().await;
        let handler_c = tokio::spawn(webhook_listener(
            port_c,
            "/c".to_string(),
            format!("touch '{}'", marker_c.to_string_lossy()),
            "target-c".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            rx_c,
            Vec::new(),
            None,
            None,
            None, // No health path
        ));

        // Start source (forwards to both B and C)
        let port_a = find_available_port().await;
        let handler_a = tokio::spawn(webhook_listener(
            port_a,
            "/a".to_string(),
            "echo 'source'".to_string(),
            "source".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_tx.subscribe(),
            event_bus.clone(),
            rx_a,
            vec!["target-b".to_string(), "target-c".to_string()],
            None,
            None,
            None, // No health path
        ));

        sleep(Duration::from_millis(200)).await;

        // Trigger source
        let client = reqwest::Client::new();
        client
            .post(format!("http://127.0.0.1:{}/a", port_a))
            .send()
            .await
            .expect("Failed to send request");

        sleep(Duration::from_millis(500)).await;

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler_a).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), handler_b).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), handler_c).await;

        // Both targets should receive the forwarded event
        assert!(
            marker_b.exists(),
            "Target B should have received forwarded event"
        );
        assert!(
            marker_c.exists(),
            "Target C should have received forwarded event"
        );
    }

    // ============== Health Check Tests ==============

    #[tokio::test]
    async fn test_health_check_returns_200() {
        let port = find_available_port().await;
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            "/webhook".to_string(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None,                             // No auth token
            None,                             // No rate limit
            Some("/health".to_string()),     // Health check path
        ));

        sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://127.0.0.1:{}/health", port))
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "OK");

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[tokio::test]
    async fn test_health_check_bypasses_auth() {
        let port = find_available_port().await;
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            "/webhook".to_string(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            Some("secret-token".to_string()), // Auth required for webhook
            None,
            Some("/health".to_string()),     // Health check path
        ));

        sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

        // Health check should work WITHOUT auth token
        let response = client
            .get(format!("http://127.0.0.1:{}/health", port))
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.unwrap(), "OK");

        // Webhook should still require auth
        let response = client
            .get(format!("http://127.0.0.1:{}/webhook", port))
            .send()
            .await
            .expect("Failed to send request");

        assert_eq!(response.status(), 401);

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }

    #[tokio::test]
    async fn test_health_check_bypasses_rate_limit() {
        let port = find_available_port().await;
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();
        let (event_bus, event_rx) = create_test_deps();

        let handler = tokio::spawn(webhook_listener(
            port,
            "/webhook".to_string(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            ShellConfig::Simple("sh".to_string()),
            None,
            shutdown_rx,
            event_bus,
            event_rx,
            Vec::new(),
            None,
            Some(1), // Very low rate limit: 1 request/sec
            Some("/health".to_string()),
        ));

        sleep(Duration::from_millis(100)).await;

        let client = reqwest::Client::new();

        // Use up the rate limit on the webhook path
        let _ = client
            .get(format!("http://127.0.0.1:{}/webhook", port))
            .send()
            .await;

        // Health check should STILL work even after rate limit exhausted
        for _ in 0..5 {
            let response = client
                .get(format!("http://127.0.0.1:{}/health", port))
                .send()
                .await
                .expect("Failed to send request");

            assert_eq!(response.status(), 200);
            assert_eq!(response.text().await.unwrap(), "OK");
        }

        let _ = shutdown_tx.send(());
        let _ = tokio::time::timeout(Duration::from_secs(2), handler).await;
    }
}
