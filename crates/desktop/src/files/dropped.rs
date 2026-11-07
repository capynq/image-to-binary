//! Native drag and drop. The window's typed drop event carries `PathBuf`s, which are mapped to
//! handles here. The page receives only a handle or an error, never a path, and no URI or path
//! string is ever parsed.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{DragDropEvent, Emitter, Manager, WebviewWindow, WindowEvent};

use super::{source_ref, FileRef, FileTable};
use crate::error::{ErrorCode, IpcError};

/// Event name that carries a [`DropOutcome`] to the page.
pub(crate) const DROP_EVENT: &str = "native-file-drop";

/// Result of one completed drop, emitted to the page as the [`DROP_EVENT`] payload.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum DropOutcome {
    Accepted { file: FileRef },
    Rejected { error: IpcError },
}

/// Maps the paths of one drop to the outcome the browser produces: exactly one file is imported,
/// while zero or several files are rejected and nothing is registered.
pub(crate) fn drop_outcome(paths: &[PathBuf], table: &FileTable) -> DropOutcome {
    match paths {
        [] => DropOutcome::Rejected {
            error: IpcError::new(
                ErrorCode::InvalidArgument,
                "Drop an image file. Links and text cannot be opened.",
            ),
        },
        [path] => match source_ref(table, path.clone()) {
            Ok(file) => DropOutcome::Accepted { file },
            Err(error) => DropOutcome::Rejected { error },
        },
        many => DropOutcome::Rejected {
            error: IpcError::new(
                ErrorCode::InvalidArgument,
                format!(
                    "Choose one image at a time. {} files were selected, so nothing was changed.",
                    many.len()
                ),
            ),
        },
    }
}

/// Handles a window event. Only a completed drop produces an outcome; hover and leave events are
/// not forwarded. The file checks run on the blocking pool, so the UI thread never waits on
/// metadata, and the outcome is emitted to this window alone once they finish.
pub(crate) fn on_window_event(window: &WebviewWindow, event: &WindowEvent) {
    let WindowEvent::DragDrop(DragDropEvent::Drop { paths, .. }) = event else {
        return;
    };
    let paths = paths.clone();
    let window = window.clone();
    let _detached = tauri::async_runtime::spawn_blocking(move || {
        let outcome = match window.try_state::<Arc<FileTable>>() {
            Some(table) => drop_outcome(&paths, table.inner()),
            None => DropOutcome::Rejected {
                error: IpcError::new(ErrorCode::Internal, "the file handle table is unavailable"),
            },
        };
        let _ = window.emit_to(window.label(), DROP_EVENT, outcome);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_support::TestDir;

    fn rejected_code(outcome: &DropOutcome) -> Option<ErrorCode> {
        match outcome {
            DropOutcome::Rejected { error } => Some(error.code()),
            DropOutcome::Accepted { .. } => None,
        }
    }

    fn rejected_message(outcome: &DropOutcome) -> String {
        match outcome {
            DropOutcome::Rejected { error } => error.message().to_owned(),
            DropOutcome::Accepted { .. } => String::new(),
        }
    }

    #[test]
    fn a_single_dropped_file_is_accepted_as_a_handle() {
        let dir = TestDir::new("drop-single");
        let file = dir.file("drop.webp", b"0123");
        let table = FileTable::default();
        let outcome = drop_outcome(&[file], &table);
        match outcome {
            DropOutcome::Accepted { file } => {
                assert_eq!(file.byte_length, 4);
                assert_eq!(file.name, "drop.webp");
            }
            DropOutcome::Rejected { error } => panic!("expected acceptance, got {error}"),
        }
        assert_eq!(table.counts(), (1, 0));
    }

    #[test]
    fn several_dropped_files_match_the_browser_rule_and_register_nothing() {
        let dir = TestDir::new("drop-many");
        let first = dir.file("a.png", b"a");
        let second = dir.file("b.png", b"b");
        let table = FileTable::default();
        let outcome = drop_outcome(&[first, second], &table);
        assert_eq!(rejected_code(&outcome), Some(ErrorCode::InvalidArgument));
        assert_eq!(
            rejected_message(&outcome),
            "Choose one image at a time. 2 files were selected, so nothing was changed."
        );
        assert_eq!(table.counts(), (0, 0));
    }

    #[test]
    fn an_empty_drop_matches_the_browser_message() {
        let table = FileTable::default();
        let outcome = drop_outcome(&[], &table);
        assert_eq!(
            rejected_message(&outcome),
            "Drop an image file. Links and text cannot be opened."
        );
    }

    #[test]
    fn a_dropped_directory_is_rejected_without_registering_anything() {
        let dir = TestDir::new("drop-directory");
        let table = FileTable::default();
        let outcome = drop_outcome(&[dir.path().to_path_buf()], &table);
        assert_eq!(rejected_code(&outcome), Some(ErrorCode::InvalidArgument));
        assert_eq!(table.counts(), (0, 0));
    }

    #[test]
    fn a_dropped_empty_file_is_rejected_with_the_browser_message() {
        let dir = TestDir::new("drop-empty");
        let file = dir.file("empty.png", b"");
        let table = FileTable::default();
        let outcome = drop_outcome(&[file], &table);
        assert_eq!(rejected_code(&outcome), Some(ErrorCode::InvalidArgument));
        assert_eq!(
            rejected_message(&outcome),
            "Encoded image bytes must not be empty."
        );
        assert_eq!(table.counts(), (0, 0));
    }

    #[test]
    fn the_outcome_payload_has_a_handle_and_no_path() {
        let dir = TestDir::new("drop-payload-MARKER");
        let file = dir.file("payload.png", b"xy");
        let table = FileTable::default();
        let outcome = drop_outcome(&[file], &table);
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["status"], "accepted");
        assert!(json["file"]["id"].is_u64());
        assert_eq!(json["file"]["name"], "payload.png");
        assert_eq!(json["file"]["byteLength"], 2);
        assert!(!serde_json::to_string(&outcome).unwrap().contains("MARKER"));
    }

    #[test]
    fn a_rejected_payload_serialises_its_code_and_message_only() {
        let table = FileTable::default();
        let outcome = drop_outcome(&[], &table);
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["status"], "rejected");
        assert_eq!(json["error"]["code"], "invalid_argument");
        assert!(json["error"]["message"].is_string());
    }
}
