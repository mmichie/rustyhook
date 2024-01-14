use hyper::{Request, Response, Body, StatusCode};
use hyper::service::{make_service_fn, service_fn};
use std::net::SocketAddr;
use std::convert::Infallible;
use tokio::net::TcpListener;
use log::{error, info};

// Function to start the webhook listener
pub async fn webhook_listener(port: u16, path: String) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    let listener = TcpListener::bind(&addr).await.expect("Failed to bind to address");
    info!("Webhook server running on {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(async move {
                    hyper::server::conn::Http::new()
                        .serve_connection(stream, service_fn(move |req| webhook_service(req, path.clone())))
                        .await
                        .unwrap();
                });
            }
            Err(e) => error!("Failed to accept connection: {}", e),
        }
    }
}

// Function to handle incoming webhook requests
async fn webhook_service(req: Request<Body>, path: String) -> Result<Response<Body>, Infallible> {
    if req.uri().path() == path {
        // Here, you would add your logic to handle the webhook
        info!("Webhook received at {}", path);
        Ok(Response::new(Body::from("Webhook received")))
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not Found"))
           
