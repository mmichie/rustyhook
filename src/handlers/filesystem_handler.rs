use crate::command_executor::execute_shell_command_with_context;
use crate::config::RetryConfig;
use crate::event::Event;
use crate::event_bus::EventBus;
use log::{debug, error, info, warn};
use notify::{event::EventKind, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// Represents a pending filesystem event that will be processed after debouncing
#[derive(Debug, Clone)]
struct PendingEvent {
    kind: EventKind,
    paths: Vec<PathBuf>,
}

#[allow(clippy::too_many_arguments)]
pub async fn filesystem_watcher(
    path: String,
    shell_command: String,
    handler_name: String,
    timeout: u64,
    retry_config: RetryConfig,
    mut shutdown_rx: broadcast::Receiver<()>,
    event_bus: Arc<EventBus>,
    mut event_rx: mpsc::UnboundedReceiver<Event>,
    forward_to: Vec<String>,
    debounce_ms: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "Initializing filesystem watcher for path: {} (debounce: {}ms)",
        path, debounce_ms
    );

    let (tx, mut rx) = mpsc::unbounded_channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })?;

    watcher.watch(Path::new(&path), RecursiveMode::Recursive)?;

    info!("Filesystem watcher started on path: {}", path);

    let debounce_duration = Duration::from_millis(debounce_ms);
    let mut pending_events: HashMap<PathBuf, PendingEvent> = HashMap::new();
    let mut debounce_timer: Option<tokio::time::Instant> = None;

    loop {
        // Calculate the sleep duration based on debounce timer
        let sleep_duration = debounce_timer
            .map(|deadline| {
                let now = tokio::time::Instant::now();
                if deadline > now {
                    deadline - now
                } else {
                    Duration::ZERO
                }
            })
            .unwrap_or(Duration::from_secs(3600)); // Long sleep if no pending events

        tokio::select! {
            Some(res) = rx.recv() => {
                match res {
                    Ok(event) => {
                        debug!("Received event: {:?}", event);
                        let should_queue = matches!(
                            event.kind,
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                        );

                        if should_queue {
                            let event_type = match event.kind {
                                EventKind::Create(_) => "Create",
                                EventKind::Modify(_) => "Modify",
                                EventKind::Remove(_) => "Remove",
                                _ => "Other",
                            };
                            debug!("{} event queued for {:?}", event_type, event.paths);

                            // Add or update pending events for each path
                            for path in &event.paths {
                                pending_events.insert(
                                    path.clone(),
                                    PendingEvent {
                                        kind: event.kind.clone(),
                                        paths: vec![path.clone()],
                                    },
                                );
                            }

                            // Reset debounce timer
                            debounce_timer = Some(tokio::time::Instant::now() + debounce_duration);
                        } else {
                            debug!("Other event detected: {:?}", event.kind);
                        }
                    }
                    Err(e) => error!("Watch error: {:?}", e),
                }
            }
            Some(forwarded_event) = event_rx.recv() => {
                // Handle forwarded events immediately (no debouncing)
                info!(
                    "Filesystem handler '{}' received forwarded event from '{}'",
                    handler_name, forwarded_event.source_handler
                );
                let context = format!("Forwarded from: {}", forwarded_event.source_handler);
                execute_shell_command_with_context(&shell_command, &handler_name, &context, timeout, &retry_config).await;

                // Forward to next handlers if configured
                if !forward_to.is_empty() {
                    for target in &forward_to {
                        if let Err(e) = event_bus.send(target, forwarded_event.clone()) {
                            warn!("Failed to forward event to '{}': {}", target, e);
                        }
                    }
                }
            }
            _ = tokio::time::sleep(sleep_duration) => {
                // Debounce timer fired - process pending events
                if !pending_events.is_empty() {
                    let events: Vec<_> = pending_events.drain().collect();
                    let event_count = events.len();

                    info!(
                        "Debounce timer fired for handler '{}': processing {} event(s)",
                        handler_name, event_count
                    );

                    // Build context with all affected paths
                    let paths: Vec<String> = events
                        .iter()
                        .flat_map(|(_, e)| e.paths.iter().map(|p| p.to_string_lossy().to_string()))
                        .collect();

                    let event_kinds: Vec<&str> = events
                        .iter()
                        .map(|(_, e)| match e.kind {
                            EventKind::Create(_) => "create",
                            EventKind::Modify(_) => "modify",
                            EventKind::Remove(_) => "remove",
                            _ => "other",
                        })
                        .collect();

                    let context = format!(
                        "Debounced events ({}): Paths: {:?}, Types: {:?}",
                        event_count, paths, event_kinds
                    );

                    execute_shell_command_with_context(&shell_command, &handler_name, &context, timeout, &retry_config).await;

                    // Forward consolidated event if configured
                    if !forward_to.is_empty() {
                        // Use the first path for the forwarded event
                        let path_str = paths.first().cloned().unwrap_or_default();
                        let event_kind = event_kinds.first().copied().unwrap_or("other");

                        let rustyhook_event = Event::from_filesystem(
                            &handler_name,
                            &path_str,
                            event_kind,
                        );

                        for target in &forward_to {
                            if let Err(e) = event_bus.send(target, rustyhook_event.clone()) {
                                warn!("Failed to forward event to '{}': {}", target, e);
                            } else {
                                debug!("Forwarded event to '{}'", target);
                            }
                        }
                    }

                    debounce_timer = None;
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
