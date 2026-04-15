//! Filesystem watcher for `dolphin-milk.toml` — detects changes and broadcasts reloaded
//! config via a `tokio::sync::watch` channel. Uses the `notify` crate for
//! sub-second detection (vs mtime polling).

use std::path::{Path, PathBuf};

use super::loader::load_config;
use super::schema::DmConfig;

pub struct ConfigWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl ConfigWatcher {
    /// Start watching `config_path`. Returns a receiver that yields the latest
    /// config whenever the file changes.
    ///
    /// Watches the **parent directory** (not the file itself) because editors
    /// often write to a temp file and rename, which doesn't trigger events on
    /// the file directly.
    pub fn start(
        config_path: PathBuf,
        initial_config: DmConfig,
    ) -> Result<(Self, tokio::sync::watch::Receiver<DmConfig>), notify::Error> {
        use notify::{EventKind, RecursiveMode, Watcher};

        let (tx, rx) = tokio::sync::watch::channel(initial_config);
        let watch_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let target_filename = config_path.file_name().unwrap_or_default().to_os_string();

        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<()>(8);

        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    if matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                        let touches_config = event
                            .paths
                            .iter()
                            .any(|p| p.file_name().map(|f| f == target_filename).unwrap_or(false));
                        if touches_config {
                            let _ = notify_tx.try_send(());
                        }
                    }
                }
            })?;

        watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

        let cfg_path = config_path.clone();
        tokio::spawn(async move {
            loop {
                if notify_rx.recv().await.is_none() {
                    break; // channel closed
                }
                // Debounce: drain further events within 500ms window
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                while notify_rx.try_recv().is_ok() {}

                match load_config(Some(&cfg_path)) {
                    Ok(new_config) => {
                        tracing::info!("dolphin-milk.toml changed, broadcasting reloaded config");
                        let _ = tx.send(new_config);
                    }
                    Err(e) => {
                        tracing::warn!("config reload failed (keeping previous): {e}");
                    }
                }
            }
        });

        tracing::info!("watching config file: {}", config_path.display());
        Ok((Self { _watcher: watcher }, rx))
    }
}
