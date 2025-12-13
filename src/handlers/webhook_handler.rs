use crate::command_executor::execute_shell_command_with_context;
use crate::config::RetryConfig;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use log::{error, info, warn};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

// Function to start the webhook listener
pub async fn webhook_listener(
    port: u16,
    path: String,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    retry_config: RetryConfig,
    mut shutdown_rx: broadcast::Receiver<()>,
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

    // Wrap retry_config in Arc for sharing across spawned tasks
    let retry_config = Arc::new(retry_config);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let io = TokioIo::new(stream);
                        let path_clone = path.clone();
                        let shell_command_clone = shell_command.clone();
                        let handler_name_clone = handler_name.clone();
                        let retry_config_clone = Arc::clone(&retry_config);
                        tokio::task::spawn(async move {
                            if let Err(err) = http1::Builder::new()
                                .serve_connection(
                                    io,
                                    service_fn(move |req| {
                                        let retry_config_inner = Arc::clone(&retry_config_clone);
                                        handle_webhook(
                                            req,
                                            path_clone.clone(),
                                            shell_command_clone.clone(),
                                            handler_name_clone.clone(),
                                            timeout,
                                            retry_config_inner,
                                        )
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
    path: String,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    retry_config: Arc<RetryConfig>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if req.uri().path() == path {
        // Process the webhook request
        info!("Webhook received at {}", path);

        let method = req.method().to_string();
        let uri = req.uri().to_string();
        let context = format!("Method: {}, URI: {}", method, uri);

        // Execute the configured shell command
        execute_shell_command_with_context(
            &shell_command,
            &handler_name,
            &context,
            timeout,
            &retry_config,
        )
        .await;

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
    use crate::config::RetryConfig;
    use std::time::Duration;
    use tokio::time::sleep;

    /// Find an available port for testing
    async fn find_available_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[tokio::test]
    async fn test_webhook_valid_path_returns_200() {
        let port = find_available_port().await;
        let path = "/webhook".to_string();
        let (shutdown_tx, _) = broadcast::channel(1);
        let shutdown_rx = shutdown_tx.subscribe();

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            shutdown_rx,
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

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            shutdown_rx,
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

        let handler = tokio::spawn(webhook_listener(
            port,
            "/test".to_string(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            shutdown_rx,
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

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            shutdown_rx,
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

        let handler = tokio::spawn(webhook_listener(
            port,
            path.clone(),
            "echo 'test'".to_string(),
            "test-handler".to_string(),
            30,
            RetryConfig::default(),
            shutdown_rx,
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
}
