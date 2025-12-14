use crate::command_executor::execute_shell_command_with_context;
use crate::config::RetryConfig;
use crate::event::Event;
use crate::event_bus::EventBus;
use globset::{Glob, GlobSet, GlobSetBuilder};
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

/// Builds a GlobSet from a list of glob patterns
fn build_globset(patterns: &[String]) -> Result<GlobSet, globset::Error> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    builder.build()
}

/// Checks if a path should be processed based on include/exclude patterns
fn should_process_path(path: &Path, include_set: &GlobSet, exclude_set: &GlobSet) -> bool {
    // If the path matches any exclude pattern, skip it
    if exclude_set.is_match(path) {
        return false;
    }

    // If include patterns are specified, the path must match at least one
    // If no include patterns, process all paths (that aren't excluded)
    include_set.is_empty() || include_set.is_match(path)
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
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(
        "Initializing filesystem watcher for path: {} (debounce: {}ms)",
        path, debounce_ms
    );

    // Build glob sets for include/exclude patterns
    let include_set =
        build_globset(&include_patterns).map_err(|e| format!("Invalid include pattern: {}", e))?;
    let exclude_set =
        build_globset(&exclude_patterns).map_err(|e| format!("Invalid exclude pattern: {}", e))?;

    if !include_patterns.is_empty() {
        info!("Include patterns: {:?}", include_patterns);
    }
    if !exclude_patterns.is_empty() {
        info!("Exclude patterns: {:?}", exclude_patterns);
    }

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

                            // Add or update pending events for each path that passes filtering
                            let mut queued_count = 0;
                            for path in &event.paths {
                                if !should_process_path(path, &include_set, &exclude_set) {
                                    debug!("Skipping path (filtered): {:?}", path);
                                    continue;
                                }

                                pending_events.insert(
                                    path.clone(),
                                    PendingEvent {
                                        kind: event.kind,
                                        paths: vec![path.clone()],
                                    },
                                );
                                queued_count += 1;
                            }

                            if queued_count > 0 {
                                debug!("{} event queued for {} path(s)", event_type, queued_count);
                                // Reset debounce timer only if we actually queued something
                                debounce_timer = Some(tokio::time::Instant::now() + debounce_duration);
                            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_build_globset_empty() {
        let patterns: Vec<String> = vec![];
        let globset = build_globset(&patterns).expect("Should build empty globset");
        assert!(globset.is_empty());
    }

    #[test]
    fn test_build_globset_single_pattern() {
        let patterns = vec!["*.rs".to_string()];
        let globset = build_globset(&patterns).expect("Should build globset");
        assert!(!globset.is_empty());
        assert!(globset.is_match("main.rs"));
        assert!(globset.is_match("lib.rs"));
        assert!(!globset.is_match("main.txt"));
    }

    #[test]
    fn test_build_globset_multiple_patterns() {
        let patterns = vec!["*.rs".to_string(), "*.toml".to_string()];
        let globset = build_globset(&patterns).expect("Should build globset");
        assert!(globset.is_match("main.rs"));
        assert!(globset.is_match("Cargo.toml"));
        assert!(!globset.is_match("main.txt"));
    }

    #[test]
    fn test_build_globset_recursive_pattern() {
        let patterns = vec!["**/*.rs".to_string()];
        let globset = build_globset(&patterns).expect("Should build globset");
        assert!(globset.is_match("main.rs"));
        assert!(globset.is_match("src/lib.rs"));
        assert!(globset.is_match("src/handlers/mod.rs"));
        assert!(!globset.is_match("src/handlers/mod.txt"));
    }

    #[test]
    fn test_build_globset_invalid_pattern() {
        let patterns = vec!["[invalid".to_string()];
        let result = build_globset(&patterns);
        assert!(result.is_err());
    }

    #[test]
    fn test_should_process_path_no_filters() {
        let include = build_globset(&[]).unwrap();
        let exclude = build_globset(&[]).unwrap();

        // Without any filters, all paths should be processed
        assert!(should_process_path(
            Path::new("main.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(
            Path::new("file.txt"),
            &include,
            &exclude
        ));
        assert!(should_process_path(
            Path::new("target/debug/main"),
            &include,
            &exclude
        ));
    }

    #[test]
    fn test_should_process_path_include_only() {
        let include = build_globset(&["*.rs".to_string()]).unwrap();
        let exclude = build_globset(&[]).unwrap();

        // Only .rs files should be processed
        assert!(should_process_path(
            Path::new("main.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(Path::new("lib.rs"), &include, &exclude));
        assert!(!should_process_path(
            Path::new("file.txt"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("Cargo.toml"),
            &include,
            &exclude
        ));
    }

    #[test]
    fn test_should_process_path_exclude_only() {
        let include = build_globset(&[]).unwrap();
        let exclude = build_globset(&["*.tmp".to_string()]).unwrap();

        // All files except .tmp should be processed
        assert!(should_process_path(
            Path::new("main.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(
            Path::new("file.txt"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("backup.tmp"),
            &include,
            &exclude
        ));
    }

    #[test]
    fn test_should_process_path_include_and_exclude() {
        let include = build_globset(&["*.rs".to_string(), "*.toml".to_string()]).unwrap();
        let exclude = build_globset(&["*.bak".to_string()]).unwrap();

        // .rs and .toml files should be processed, but .bak should be excluded
        assert!(should_process_path(
            Path::new("main.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(
            Path::new("Cargo.toml"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("file.txt"),
            &include,
            &exclude
        )); // Not in include
    }

    #[test]
    fn test_should_process_path_exclude_takes_precedence() {
        let include = build_globset(&["*.rs".to_string()]).unwrap();
        let exclude = build_globset(&["test_*.rs".to_string()]).unwrap();

        // test_*.rs should be excluded even though it matches *.rs include
        assert!(should_process_path(
            Path::new("main.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(Path::new("lib.rs"), &include, &exclude));
        assert!(!should_process_path(
            Path::new("test_main.rs"),
            &include,
            &exclude
        ));
    }

    #[test]
    fn test_should_process_path_recursive_exclude() {
        let include = build_globset(&[]).unwrap();
        let exclude = build_globset(&["target/**".to_string()]).unwrap();

        // Files in target directory should be excluded
        assert!(should_process_path(
            Path::new("main.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(
            Path::new("src/lib.rs"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("target/debug/main"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("target/release/main"),
            &include,
            &exclude
        ));
    }

    #[test]
    fn test_should_process_path_multiple_excludes() {
        let include = build_globset(&[]).unwrap();
        let exclude = build_globset(&[
            "target/**".to_string(),
            "node_modules/**".to_string(),
            "*.tmp".to_string(),
        ])
        .unwrap();

        assert!(should_process_path(
            Path::new("main.rs"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("target/debug/main"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("node_modules/pkg/index.js"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("backup.tmp"),
            &include,
            &exclude
        ));
    }

    #[test]
    fn test_should_process_path_typical_rust_project() {
        // Typical patterns for watching a Rust project
        let include = build_globset(&["**/*.rs".to_string(), "Cargo.toml".to_string()]).unwrap();
        let exclude = build_globset(&["target/**".to_string()]).unwrap();

        assert!(should_process_path(
            Path::new("src/main.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(
            Path::new("src/handlers/mod.rs"),
            &include,
            &exclude
        ));
        assert!(should_process_path(
            Path::new("Cargo.toml"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("target/debug/rustyhook"),
            &include,
            &exclude
        ));
        assert!(!should_process_path(
            Path::new("README.md"),
            &include,
            &exclude
        ));
    }
}
