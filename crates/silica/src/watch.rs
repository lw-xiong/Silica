//! Filesystem watching for the open vault.
//!
//! `notify` reports changes on its own thread; the paths are queued on a
//! channel that the workspace drains every few hundred milliseconds, so a
//! burst of events (a sync tool rewriting many files) becomes one reindex.

use notify::{RecursiveMode, Watcher as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

/// Keeps the watcher alive; dropping it stops the events.
pub struct Watcher {
    _inner: notify::RecommendedWatcher,
}

/// Starts watching `root` recursively. Returns the handle and the channel
/// changed paths arrive on.
pub fn start(root: &Path) -> Result<(Watcher, Receiver<PathBuf>), notify::Error> {
    let (tx, rx) = channel::<PathBuf>();
    let mut inner = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result {
            if matches!(event.kind, notify::EventKind::Access(_)) {
                return;
            }
            for path in event.paths {
                let _ = tx.send(path);
            }
        }
    })?;
    inner.watch(root, RecursiveMode::Recursive)?;
    Ok((Watcher { _inner: inner }, rx))
}
