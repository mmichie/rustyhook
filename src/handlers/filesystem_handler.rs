use log::{debug, error, info};
use notify::{event::EventKind, Config, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::channel;

pub async fn filesystem_watcher(path: String) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initializing filesystem watcher for path: {}", path);

    let (tx, rx) = channel::<notify::Result<notify::Event>>();
    let _config = Config::default();
    let mut watcher = notify::recommended_watcher(move |res| tx.send(res).unwrap())?;

    watcher.watch(Path::new(&path), RecursiveMode::Recursive)?;

    info!("Filesystem watcher started on path: {}", path);

    loop {
        match rx.recv() {
            Ok(Ok(event)) => {
                debug!("Received event: {:?}", event);
                match event.kind {
                    EventKind::Create(_) => {
                        info!("Create event detected in {:?}", event.paths);
                    }
                    EventKind::Modify(_) => {
                        info!("Modify event detected in {:?}", event.paths);
                    }
                    EventKind::Remove(_) => {
                        info!("Remove event detected in {:?}", event.paths);
                    }
                    // Add more cases as needed
                    _ => debug!("Other event detected: {:?}", event.kind),
                }
            }
            Ok(Err(e)) => error!("Watch error: {:?}", e),
            Err(e) => error!("Channel receive error: {:?}", e),
        }
    }
}
