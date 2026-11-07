//! Private table from opaque handles to filesystem paths. A path leaves this module only as a
//! value Rust code asks for; the page only ever receives a [`FileId`].

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, IpcError};

/// Opaque handle naming one selection. Ids are never reused within a process, so a stale
/// handle can only fail, never resolve to a newer selection.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct FileId(u64);

/// One stored path and the handle that names it. Its Debug output omits the path.
struct Entry {
    id: FileId,
    path: PathBuf,
}

impl fmt::Debug for Entry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Entry")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Source,
    Sink,
}

#[derive(Default)]
struct State {
    last_id: u64,
    source: Option<Entry>,
    sink: Option<Entry>,
}

/// Owns the one source and the one destination the page may refer to, and the gate that lets
/// only one native dialog open at a time.
///
/// Invariants: each kind holds at most one entry. Inserting a kind replaces its previous entry,
/// so the older handle of that kind fails from then on. Ids are never reused.
/// Failure contract: an id that is unknown, released, or replaced resolves to `invalid_argument`,
/// and a counter that cannot advance returns `internal` without changing any entry.
#[derive(Default)]
pub(crate) struct FileTable {
    state: Mutex<State>,
    dialog_open: Arc<AtomicBool>,
}

/// Proof that one dialog is open. Dropping it reopens the gate, so every path that ends a dialog,
/// including a dialog that never answers, releases it.
#[derive(Debug)]
pub(crate) struct DialogLease {
    open: Arc<AtomicBool>,
}

impl Drop for DialogLease {
    fn drop(&mut self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

impl FileTable {
    /// Stores a source path and returns its handle, replacing any earlier source.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::Internal`] when the handle counter is exhausted.
    pub(crate) fn insert_source(&self, path: PathBuf) -> Result<FileId, IpcError> {
        self.insert(Kind::Source, path)
    }

    /// Stores a destination path and returns its handle, replacing any earlier destination.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::Internal`] when the handle counter is exhausted.
    pub(crate) fn insert_sink(&self, path: PathBuf) -> Result<FileId, IpcError> {
        self.insert(Kind::Sink, path)
    }

    /// Returns a copy of the current source path for in-process reads. The copy must never cross
    /// IPC.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidArgument`] for an unknown, released, replaced, or non-source
    /// handle.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn source_path(&self, id: FileId) -> Result<PathBuf, IpcError> {
        let state = self.lock();
        matching(state.source.as_ref(), id)
    }

    /// Returns a copy of the current destination path for in-process writes. The copy must never
    /// cross IPC.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidArgument`] for an unknown, released, replaced, or non-destination
    /// handle.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn sink_path(&self, id: FileId) -> Result<PathBuf, IpcError> {
        let state = self.lock();
        matching(state.sink.as_ref(), id)
    }

    /// Removes the entry named by `id`, if it is still stored. Idempotent: releasing an unknown,
    /// released, or replaced id changes nothing.
    pub(crate) fn release(&self, id: FileId) {
        let mut state = self.lock();
        if state.source.as_ref().is_some_and(|entry| entry.id == id) {
            state.source = None;
        }
        if state.sink.as_ref().is_some_and(|entry| entry.id == id) {
            state.sink = None;
        }
    }

    /// Opens the dialog gate.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ResourceLimitExceeded`] while another dialog's lease is alive.
    pub(crate) fn begin_dialog(&self) -> Result<DialogLease, IpcError> {
        self.dialog_open
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| {
                IpcError::new(
                    ErrorCode::ResourceLimitExceeded,
                    "a file dialog is already open; answer or close it first",
                )
            })?;
        Ok(DialogLease {
            open: Arc::clone(&self.dialog_open),
        })
    }

    fn insert(&self, kind: Kind, path: PathBuf) -> Result<FileId, IpcError> {
        let mut state = self.lock();
        let next = state.last_id.checked_add(1).ok_or_else(|| {
            IpcError::new(ErrorCode::Internal, "the file handle counter is exhausted")
        })?;
        state.last_id = next;
        let id = FileId(next);
        let entry = Some(Entry { id, path });
        match kind {
            Kind::Source => state.source = entry,
            Kind::Sink => state.sink = entry,
        }
        Ok(id)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    fn with_last_id(last_id: u64) -> Self {
        Self {
            state: Mutex::new(State {
                last_id,
                ..State::default()
            }),
            dialog_open: Arc::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize) {
        let state = self.lock();
        (
            usize::from(state.source.is_some()),
            usize::from(state.sink.is_some()),
        )
    }
}

impl fmt::Debug for FileTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        formatter
            .debug_struct("FileTable")
            .field("sources", &usize::from(state.source.is_some()))
            .field("sinks", &usize::from(state.sink.is_some()))
            .field("dialog_open", &self.dialog_open.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

fn matching(entry: Option<&Entry>, id: FileId) -> Result<PathBuf, IpcError> {
    match entry {
        Some(entry) if entry.id == id => Ok(entry.path.clone()),
        _ => Err(unknown_handle()),
    }
}

fn unknown_handle() -> IpcError {
    IpcError::new(
        ErrorCode::InvalidArgument,
        "the file handle is unknown, released, or replaced",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    #[test]
    fn release_is_idempotent() {
        let table = FileTable::default();
        let id = table.insert_source(path("a.png")).unwrap();
        table.release(id);
        table.release(id);
        assert_eq!(table.counts(), (0, 0));
        assert!(table.source_path(id).is_err());
    }

    #[test]
    fn unknown_and_mismatched_handles_are_typed_errors() {
        let table = FileTable::default();
        let source = table.insert_source(path("in.png")).unwrap();
        let sink = table.insert_sink(path("out.png")).unwrap();

        let unknown = table.source_path(FileId(u64::MAX)).unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::InvalidArgument);
        assert_eq!(
            table.sink_path(source).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            table.source_path(sink).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
        assert_eq!(table.source_path(source).unwrap(), path("in.png"));
        assert_eq!(table.sink_path(sink).unwrap(), path("out.png"));
    }

    #[test]
    fn released_ids_are_not_reused() {
        let table = FileTable::default();
        let first = table.insert_source(path("a.png")).unwrap();
        table.release(first);
        let second = table.insert_source(path("b.png")).unwrap();
        assert_ne!(first, second);
        assert!(table.source_path(first).is_err());
        assert_eq!(table.source_path(second).unwrap(), path("b.png"));
    }

    #[test]
    fn a_new_source_replaces_the_previous_source_and_keeps_the_sink() {
        let table = FileTable::default();
        let sink = table.insert_sink(path("out.png")).unwrap();
        let old = table.insert_source(path("old.png")).unwrap();
        let new = table.insert_source(path("new.png")).unwrap();

        assert_eq!(table.counts(), (1, 1));
        assert_eq!(
            table.source_path(old).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
        assert_eq!(table.source_path(new).unwrap(), path("new.png"));
        assert_eq!(table.sink_path(sink).unwrap(), path("out.png"));
    }

    #[test]
    fn a_new_sink_replaces_the_previous_sink() {
        let table = FileTable::default();
        let old = table.insert_sink(path("old.png")).unwrap();
        let new = table.insert_sink(path("new.png")).unwrap();
        assert_eq!(table.counts(), (0, 1));
        assert!(table.sink_path(old).is_err());
        assert_eq!(table.sink_path(new).unwrap(), path("new.png"));
    }

    #[test]
    fn releasing_a_replaced_handle_leaves_its_successor_in_place() {
        let table = FileTable::default();
        let old = table.insert_source(path("old.png")).unwrap();
        let new = table.insert_source(path("new.png")).unwrap();
        table.release(old);
        assert_eq!(table.source_path(new).unwrap(), path("new.png"));
    }

    #[test]
    fn repeated_selections_never_grow_the_table() {
        let table = FileTable::default();
        for index in 0..1000 {
            table.insert_source(path(&format!("{index}.png"))).unwrap();
            table
                .insert_sink(path(&format!("{index}-out.png")))
                .unwrap();
        }
        assert_eq!(table.counts(), (1, 1));
    }

    #[test]
    fn id_counter_overflow_is_a_typed_error_that_changes_nothing() {
        let table = FileTable::with_last_id(u64::MAX - 1);
        let kept = table.insert_source(path("kept.png")).unwrap();
        assert_eq!(kept, FileId(u64::MAX));

        let error = table.insert_source(path("refused.png")).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
        assert_eq!(
            table
                .insert_sink(path("refused-out.png"))
                .unwrap_err()
                .code(),
            ErrorCode::Internal
        );
        assert_eq!(table.source_path(kept).unwrap(), path("kept.png"));
        assert_eq!(table.counts(), (1, 0));
    }

    #[test]
    fn dialog_gate_admits_one_lease_at_a_time() {
        let table = FileTable::default();
        let lease = table.begin_dialog().unwrap();
        let busy = table.begin_dialog().unwrap_err();
        assert_eq!(busy.code(), ErrorCode::ResourceLimitExceeded);
        drop(lease);
        assert!(table.begin_dialog().is_ok());
    }

    #[test]
    fn debug_output_counts_entries_and_never_shows_a_path() {
        let table = FileTable::default();
        table
            .insert_source(path("/private/SECRET-MARKER/photo.png"))
            .unwrap();
        table
            .insert_sink(path("/private/SECRET-MARKER/out.png"))
            .unwrap();
        let text = format!("{table:?}");
        assert!(!text.contains("SECRET-MARKER"), "{text}");
        assert!(!text.contains("/private"), "{text}");
        assert!(text.contains("sources: 1"), "{text}");
        assert!(text.contains("sinks: 1"), "{text}");

        let entry = Entry {
            id: FileId(7),
            path: path("/private/SECRET-MARKER/entry.png"),
        };
        let entry_text = format!("{entry:?}");
        assert!(!entry_text.contains("SECRET-MARKER"), "{entry_text}");
        assert!(entry_text.contains("FileId(7)"), "{entry_text}");
    }
}
