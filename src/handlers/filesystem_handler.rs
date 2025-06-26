use crate::command_executor::execute_shell_command_with_context;
use log::{debug, error, info};
use notify::{event::EventKind, RecursiveMode, Watcher};
use std::path::Path;
use tokio::sync::{broadcast, mpsc};

pub async fn filesystem_watcher(
    path: String,
    shell_command: String,
    handler_name: String,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("Initializing filesystem watcher for path: {}", path);

    let (tx, mut rx) = mpsc::unbounded_channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;

    watcher.watch(Path::new(&path), RecursiveMode::Recursive)?;

    info!("Filesystem watcher started on path: {}", path);

    loop {
        tokio::select! {
            Some(res) = rx.recv() => {
                match res {
                    Ok(event) => {
                        debug!("Received event: {:?}", event);
                        let should_execute = match event.kind {
                            EventKind::Create(_) => {
                                info!("Create event detected in {:?}", event.paths);
                                true
                            }
                            EventKind::Modify(_) => {
                                info!("Modify event detected in {:?}", event.paths);
                                true
                            }
                            EventKind::Remove(_) => {
                                info!("Remove event detected in {:?}", event.paths);
                                true
                            }
                            _ => {
                                debug!("Other event detected: {:?}", event.kind);
                                false
                            }
                        };

                        if should_execute {
                            let context = format!("Event: {:?}, Paths: {:?}", event.kind, event.paths);
                            execute_shell_command_with_context(&shell_command, &handler_name, &context);
                        }
                    }
                    Err(e) => error!("Watch error: {:?}", e),
                }
            }
            _ = shutdown_rx.recv() => {
                info!("Filesystem handler '{}' received shutdown signal", handler_name);
                break;
            }
        }
    }
    
    info!("Filesystem handler '{}' shutting down", handler_name);
    Ok(())
}
