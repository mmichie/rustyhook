use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use log::{info, warn};
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
        let path = path.clone();

        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(move |req| handle_webhook(req, path.clone())))
                .await
            {
                warn!("Error serving connection: {:?}", err);
            }
        });
    }
}

// Function to handle incoming webhooks
async fn handle_webhook(
    req: Request<hyper::body::Incoming>,
    path: String,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if req.uri().path() == path {
        // Process the webhook request
        info!("Webhook received at {}", path);
        Ok(Response::new(Full::new(Bytes::from("Webhook received"))))
    } else {
        // Respond with Not Found for requests on other paths
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap())
    }
}
