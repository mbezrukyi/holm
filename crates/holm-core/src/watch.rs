//! The source of file changes. It is the only seam in the core.
//!
//! In the app, `notify` sends the changes. In tests, a [`Feed`] takes them by hand.
//! Both give the core a [`Batch`] of paths, which it checks against the disk.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use notify::event::{AccessKind, AccessMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::error::{Error, Result};

/// Paths relative to the notes folder where something may have changed.
/// The empty path stands for the whole notes folder.
pub(crate) type Batch = BTreeSet<PathBuf>;

/// Gives a batch to the core. It returns `None` when the core is gone.
pub(crate) type Apply = Arc<dyn Fn(Batch) -> Option<Result<()>> + Send + Sync>;

/// A batch waits until no event came for this long...
const QUIET: Duration = Duration::from_millis(50);
/// ...but never longer than this.
const MAX_WAIT: Duration = Duration::from_millis(500);

/// Watches the notes folder with `notify` while it lives.
#[derive(Debug)]
pub(crate) struct Watcher {
    _watcher: RecommendedWatcher,
}

/// The events of a [`Watcher`]. They wait in a queue until [`Events::run`].
pub(crate) struct Events {
    root: PathBuf,
    queue: Receiver<notify::Result<notify::Event>>,
}

/// Starts to watch `root` and all its subfolders.
pub(crate) fn start(root: &Path) -> notify::Result<(Watcher, Events)> {
    let (send, queue) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        // The receiver is gone only when the core is gone.
        let _ = send.send(event);
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    let events = Events {
        root: root.to_owned(),
        queue,
    };
    Ok((Watcher { _watcher: watcher }, events))
}

impl Events {
    /// Gives the events to `apply` in batches, on a new thread.
    /// The thread ends when the [`Watcher`] or the core is gone.
    pub(crate) fn run(self, apply: Apply) {
        let spawned = thread::Builder::new()
            .name("holm-watch".to_owned())
            .spawn(move || self.serve(&apply));
        if let Err(error) = spawned {
            tracing::warn!(%error, "cannot watch the notes folder");
        }
    }

    fn serve(&self, apply: &Apply) {
        while let Ok(first) = self.queue.recv() {
            let mut batch = Batch::new();
            self.add(&mut batch, first);
            let start = Instant::now();
            while start.elapsed() < MAX_WAIT {
                match self.queue.recv_timeout(QUIET) {
                    Ok(event) => self.add(&mut batch, event),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            match apply(batch) {
                None => return,
                Some(Err(error)) => tracing::warn!(%error, "cannot take in outside changes"),
                Some(Ok(())) => {}
            }
        }
    }

    fn add(&self, batch: &mut Batch, event: notify::Result<notify::Event>) {
        let event = match event {
            // The core opens files too, so these events would start a new batch after each batch.
            Ok(notify::Event {
                kind: EventKind::Access(AccessKind::Open(_) | AccessKind::Close(AccessMode::Read)),
                ..
            }) => return,
            Ok(event) if !event.need_rescan() => event,
            Ok(_) => {
                batch.insert(PathBuf::new());
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "lost file events, so Holm checks the whole notes folder");
                batch.insert(PathBuf::new());
                return;
            }
        };
        for path in event.paths {
            if let Ok(rel) = path.strip_prefix(&self.root) {
                batch.insert(rel.to_owned());
            }
        }
    }
}

/// Tells a core that has no file watching about changes on disk.
///
/// [`Core::open_with_feed`](crate::Core::open_with_feed) gives it. Tests use it in place of
/// real file events, so they decide when the core sees a change.
pub struct Feed {
    pub(crate) apply: Apply,
}

impl Feed {
    /// Tells the core that something changed at each of `paths`: a note, a folder, or any other file.
    ///
    /// The paths are relative to the notes folder. The empty path stands for the whole notes folder.
    /// The paths are one batch, like the events of one moment. So a file that moved
    /// comes as one [`Change::Moved`](crate::model::Change::Moved) only when both of its paths
    /// are in the same batch.
    ///
    /// The core checks the disk at each path and below it, updates the index,
    /// and sends the changes to each [`ChangeLists`](crate::ChangeLists) before this returns.
    /// Hidden paths are skipped.
    ///
    /// # Errors
    ///
    /// - [`Error::BadPath`] when a path is absolute or holds `..`. Then no path is checked.
    /// - [`Error::Read`] when the notes folder itself can't be read.
    /// - [`Error::Index`] as in [`Core::open`](crate::Core::open).
    pub fn changed<P: AsRef<Path>>(&self, paths: impl IntoIterator<Item = P>) -> Result<()> {
        let mut batch = Batch::new();
        for path in paths {
            let path = path.as_ref();
            if !path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            {
                return Err(Error::BadPath {
                    path: path.to_owned(),
                });
            }
            batch.insert(path.to_owned());
        }
        (self.apply)(batch).unwrap_or(Ok(()))
    }
}
