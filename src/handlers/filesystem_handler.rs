use log::{error, info};
use notify::event::{CreateKind, EventKind, ModifyKind};
use notify::{Config, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::channel;

pub async fn filesystem_watcher(
    path: String,
    event_type: String,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initializing filesystem watcher for path: {}", path);

    let (tx, rx) = channel::<notify::Result<notify::Event>>();
    let _config = Config::default();
    let mut watcher = notify::recommended_watcher(move |res| tx.send(res).unwrap())?;

    watcher.watch(Path::new(&path), RecursiveMode::Recursive)?;

    info!("Filesystem watcher started on path: {}", path);

    loop {
        match rx.recv() {
            Ok(Ok(event)) => match event.kind {
                EventKind::Create(CreateKind::Any) if event_type == "create" => {
                    info!("Create event detected in {:?}", event.paths);
                }
                EventKind::Modify(ModifyKind::Any) if event_type == "write" => {
                    info!("Modify event detected in {:?}", event.paths);
                }
                _ => {}
            },
            Ok(Err(e)) => error!("Watch error: {:?}", e),
            Err(e) => error!("Channel receive error: {:?}", e),
        }
    }
}
