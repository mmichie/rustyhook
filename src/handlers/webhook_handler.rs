// src/handlers/webhook_handler.rs

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use log::info;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::net::TcpListener;

// Function to start the webhook listener
pub async fn webhook_listener(
    port: u16,
    path: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(&addr).await?;

    info!("Webhook server running on {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(|req| handle_webhook(req)))
                .await
            {
                println!("Error serving connection: {:?}", err);
            }
        });
    }
}

// Function to handle incoming webhooks (as you originally had it)
async fn handle_webhook(
    _req: Request<hyper::body::Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    // Your original logic for handling webhook
    Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
}
