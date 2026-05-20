use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct FileWatcher {
    // Keep the watcher alive — dropping it stops watching
    _watcher: RecommendedWatcher,
    pub rx: mpsc::UnboundedReceiver<PathBuf>,
}

impl FileWatcher {
    pub fn new(root: &Path) -> Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();

        let watcher_tx = tx.clone();
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let Ok(event) = result else { return };

            // Only re-index on actual content changes or new files
            match event.kind {
                EventKind::Create(_) | EventKind::Modify(_) => {}
                EventKind::Remove(_) => {
                    // Propagate removes so the store can clean up
                    for path in event.paths {
                        let _ = watcher_tx.send(path);
                    }
                    return;
                }
                _ => return,
            }

            for path in event.paths {
                if path.is_file() {
                    let _ = watcher_tx.send(path);
                }
            }
        })?;

        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self { _watcher: watcher, rx })
    }
}

/// Read from the watcher channel with a 500ms debounce.
/// Returns the deduplicated set of changed paths after the quiet period.
pub async fn collect_debounced(
    rx: &mut mpsc::UnboundedReceiver<PathBuf>,
    debounce_ms: u64,
) -> Vec<PathBuf> {
    use std::collections::HashSet;
    use tokio::time::{sleep, Duration};

    let mut pending: HashSet<PathBuf> = HashSet::new();

    // Wait for first event
    let Some(first) = rx.recv().await else {
        return vec![];
    };
    pending.insert(first);

    // Drain any further events that arrive within the debounce window
    loop {
        match tokio::time::timeout(Duration::from_millis(debounce_ms), rx.recv()).await {
            Ok(Some(path)) => { pending.insert(path); }
            Ok(None) => break, // channel closed
            Err(_) => break,   // timeout — quiet window elapsed
        }
    }

    pending.into_iter().collect()
}
