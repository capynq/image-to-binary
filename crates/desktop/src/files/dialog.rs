//! Native open and save dialogs, driven only from Rust. The page is never granted a dialog
//! permission. The dialog sits behind a private trait so the selection logic is testable with a
//! fake picker.
//!
//! The dialog runs on the main thread, where the platform requires it, and the command awaits
//! the answer on the async runtime, so no caller blocks the UI thread while waiting. Only one
//! dialog may be open at a time.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::AppHandle;

use super::sink::suggested_file_name;
use super::table::DialogLease;
use super::{sink_ref, source_ref, FileRef, FileTable, SinkRef};
use crate::error::{ErrorCode, IpcError};

/// Extensions of the formats the browser accepts, so both shells offer the same choices.
pub(crate) const IMAGE_EXTENSIONS: &[&str] =
    &["bmp", "gif", "jpeg", "jpg", "png", "tif", "tiff", "webp"];
const PNG_EXTENSIONS: &[&str] = &["png"];

/// The user's choice: `Ok(None)` means the dialog was cancelled, which is not an error.
pub(crate) type Selection = Result<Option<PathBuf>, IpcError>;
type Done = Box<dyn FnOnce(Selection) + Send>;

/// Starts native dialogs. Implementations must return immediately and call `done` exactly once,
/// or drop it unanswered when the dialog cannot be shown.
pub(crate) trait Picker: Send + Sync + 'static {
    fn choose_image(&self, done: Done);
    fn choose_destination(&self, suggested_name: &str, done: Done);
}

/// Dialogs backed by `rfd`, reached only through Rust.
pub(crate) struct NativePicker {
    app: AppHandle,
}

impl NativePicker {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// Shows a dialog on the main thread and sends its answer to `done`. When the main thread
    /// cannot run the closure, `done` is dropped, and the waiting command reports `internal`.
    fn dispatch(&self, show: impl FnOnce() -> Option<PathBuf> + Send + 'static, done: Done) {
        let _ = self.app.run_on_main_thread(move || done(Ok(show())));
    }
}

impl Picker for NativePicker {
    fn choose_image(&self, done: Done) {
        self.dispatch(
            || {
                rfd::FileDialog::new()
                    .add_filter("Images", IMAGE_EXTENSIONS)
                    .pick_file()
            },
            done,
        );
    }

    fn choose_destination(&self, suggested_name: &str, done: Done) {
        let name = suggested_name.to_owned();
        self.dispatch(
            move || {
                rfd::FileDialog::new()
                    .add_filter("PNG image", PNG_EXTENSIONS)
                    .set_file_name(name)
                    .save_file()
            },
            done,
        );
    }
}

/// Waits for the single answer of a dialog. The channel holds one message, so it works as a
/// oneshot: a dropped `done` closes it and reports `internal`.
async fn await_selection(start: impl FnOnce(Done)) -> Selection {
    let (sender, mut receiver) = tauri::async_runtime::channel::<Selection>(1);
    start(Box::new(move |selection| {
        let _ = sender.try_send(selection);
    }));
    receiver.recv().await.unwrap_or_else(|| {
        Err(IpcError::new(
            ErrorCode::Internal,
            "the file dialog closed without an answer",
        ))
    })
}

/// Keeps the dialog gate closed until the answer is delivered, or until the unanswered callback
/// is dropped, so the gate reopens on every path.
fn hold(lease: DialogLease, done: Done) -> Done {
    Box::new(move |selection| {
        done(selection);
        drop(lease);
    })
}

/// Runs blocking file work on the blocking pool and maps a lost task to `internal`.
async fn run_blocking<T, F>(work: F) -> Result<T, IpcError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, IpcError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .unwrap_or_else(|_| {
            Err(IpcError::new(
                ErrorCode::Internal,
                "the file task did not finish",
            ))
        })
}

/// Shows the image picker and returns a handle for the chosen file, or `None` when cancelled.
///
/// # Errors
///
/// Returns `resource_limit_exceeded` while another dialog is open, and the errors of
/// [`source_ref`] for a directory, non-regular file, empty file, or oversized input.
pub(crate) async fn pick_source<P: Picker>(
    picker: &P,
    table: Arc<FileTable>,
) -> Result<Option<FileRef>, IpcError> {
    let lease = table.begin_dialog()?;
    match await_selection(move |done| picker.choose_image(hold(lease, done))).await? {
        None => Ok(None),
        Some(path) => run_blocking(move || source_ref(&table, path))
            .await
            .map(Some),
    }
}

/// Shows the save dialog with a sanitised suggested name and returns a handle for the chosen
/// destination, or `None` when cancelled. The hint is treated as a name only, never as a path.
///
/// # Errors
///
/// Returns `resource_limit_exceeded` while another dialog is open, and `invalid_argument` when
/// the chosen location has no file name.
pub(crate) async fn pick_destination<P: Picker>(
    picker: &P,
    table: Arc<FileTable>,
    hint: Option<&str>,
) -> Result<Option<SinkRef>, IpcError> {
    let lease = table.begin_dialog()?;
    let name = suggested_file_name(hint);
    match await_selection(move |done| picker.choose_destination(&name, hold(lease, done))).await? {
        None => Ok(None),
        Some(path) => sink_ref(&table, path).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::test_support::TestDir;
    use std::sync::mpsc::{channel, Sender};
    use std::sync::Mutex;
    use std::time::Duration;

    /// Answers each dialog with a fixed reply, or never answers when `silent` is set.
    struct FakePicker {
        reply: Option<Selection>,
        suggested: Mutex<Option<String>>,
    }

    impl FakePicker {
        fn replying(reply: Selection) -> Self {
            Self {
                reply: Some(reply),
                suggested: Mutex::new(None),
            }
        }

        fn silent() -> Self {
            Self {
                reply: None,
                suggested: Mutex::new(None),
            }
        }

        fn answer(&self, done: Done) {
            if let Some(reply) = self.reply.clone() {
                done(reply);
            }
        }
    }

    impl Picker for FakePicker {
        fn choose_image(&self, done: Done) {
            self.answer(done);
        }

        fn choose_destination(&self, suggested_name: &str, done: Done) {
            *self.suggested.lock().unwrap() = Some(suggested_name.to_owned());
            self.answer(done);
        }
    }

    /// Keeps each dialog open by holding its callback until the test answers it.
    struct HoldingPicker {
        held: Mutex<Sender<Done>>,
    }

    impl Picker for HoldingPicker {
        fn choose_image(&self, done: Done) {
            let _ = self.held.lock().unwrap().send(done);
        }

        fn choose_destination(&self, _suggested_name: &str, done: Done) {
            let _ = self.held.lock().unwrap().send(done);
        }
    }

    fn block<T>(future: impl std::future::Future<Output = T>) -> T {
        tauri::async_runtime::block_on(future)
    }

    #[test]
    fn cancelling_the_open_dialog_is_a_typed_none_without_a_handle() {
        let table = Arc::new(FileTable::default());
        let picker = FakePicker::replying(Ok(None));
        assert_eq!(
            block(pick_source(&picker, Arc::clone(&table))).unwrap(),
            None
        );
        assert_eq!(table.counts(), (0, 0));
        assert!(table.begin_dialog().is_ok(), "cancel must reopen the gate");
    }

    #[test]
    fn selecting_a_file_returns_its_name_and_length_but_no_path() {
        let dir = TestDir::new("dialog-select");
        let file = dir.file("photo.png", b"12345");
        let table = Arc::new(FileTable::default());
        let picker = FakePicker::replying(Ok(Some(file.clone())));
        let chosen = block(pick_source(&picker, Arc::clone(&table)))
            .unwrap()
            .unwrap();
        assert_eq!(chosen.name(), "photo.png");
        assert_eq!(chosen.byte_length(), 5);
        assert_eq!(table.source_path(chosen.id()).unwrap(), file);

        table.release(chosen.id());
        assert!(table.source_path(chosen.id()).is_err());
    }

    #[test]
    fn a_second_selection_replaces_the_first_handle() {
        let dir = TestDir::new("dialog-replace");
        let first = dir.file("first.png", b"1");
        let second = dir.file("second.png", b"22");
        let table = Arc::new(FileTable::default());
        let old = block(pick_source(
            &FakePicker::replying(Ok(Some(first))),
            Arc::clone(&table),
        ))
        .unwrap()
        .unwrap();
        let new = block(pick_source(
            &FakePicker::replying(Ok(Some(second.clone()))),
            Arc::clone(&table),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            table.source_path(old.id()).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
        assert_eq!(table.source_path(new.id()).unwrap(), second);
    }

    #[test]
    fn a_dialog_that_never_answers_is_an_internal_error_and_reopens_the_gate() {
        let table = Arc::new(FileTable::default());
        let picker = FakePicker::silent();
        let error = block(pick_source(&picker, Arc::clone(&table))).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Internal);
        assert!(table.begin_dialog().is_ok());
    }

    #[test]
    fn a_picker_error_reaches_the_caller() {
        let table = Arc::new(FileTable::default());
        let picker = FakePicker::replying(Err(IpcError::new(
            ErrorCode::InvalidArgument,
            "the chosen location is not a local file",
        )));
        let error = block(pick_source(&picker, table)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn a_second_dialog_while_one_is_open_fails_at_once_and_the_first_still_answers() {
        let table = Arc::new(FileTable::default());
        let (held_tx, held_rx) = channel();
        let picker = Arc::new(HoldingPicker {
            held: Mutex::new(held_tx),
        });
        let first_table = Arc::clone(&table);
        let first_picker = Arc::clone(&picker);
        let (first_tx, first_rx) = channel();
        std::thread::spawn(move || {
            let outcome = block(pick_source(&*first_picker, first_table));
            let _ = first_tx.send(outcome);
        });
        let open_dialog = held_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        let (busy_tx, busy_rx) = channel();
        let busy_table = Arc::clone(&table);
        let busy_picker = Arc::clone(&picker);
        std::thread::spawn(move || {
            let outcome = block(pick_source(&*busy_picker, busy_table)).map(|_| ());
            let _ = busy_tx.send(outcome);
        });
        let busy = busy_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a second open must be refused, not left waiting")
            .unwrap_err();
        assert_eq!(busy.code(), ErrorCode::ResourceLimitExceeded);

        let (save_tx, save_rx) = channel();
        let save_table = Arc::clone(&table);
        let save_picker = Arc::clone(&picker);
        std::thread::spawn(move || {
            let outcome = block(pick_destination(&*save_picker, save_table, None)).map(|_| ());
            let _ = save_tx.send(outcome);
        });
        let busy_save = save_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a second save must be refused, not left waiting")
            .unwrap_err();
        assert_eq!(busy_save.code(), ErrorCode::ResourceLimitExceeded);

        open_dialog(Ok(None));
        let first = first_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first.unwrap(), None);
        assert!(
            table.begin_dialog().is_ok(),
            "the gate must reopen after the answer"
        );
    }

    #[test]
    fn a_full_handle_cycle_never_grows_the_table() {
        let dir = TestDir::new("dialog-cycle");
        let file = dir.file("cycle.png", b"x");
        let table = Arc::new(FileTable::default());
        for _ in 0..200 {
            let picker = FakePicker::replying(Ok(Some(file.clone())));
            let chosen = block(pick_source(&picker, Arc::clone(&table)))
                .unwrap()
                .unwrap();
            table.release(chosen.id());
        }
        assert_eq!(table.counts(), (0, 0));
    }

    #[test]
    fn save_dialog_gets_only_the_final_name_and_returns_a_sink_handle() {
        let dir = TestDir::new("dialog-save");
        let destination = dir.path().join("export.png");
        let table = Arc::new(FileTable::default());
        let picker = FakePicker::replying(Ok(Some(destination.clone())));
        let chosen = block(pick_destination(
            &picker,
            Arc::clone(&table),
            Some("../../secret/x.png"),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            picker.suggested.lock().unwrap().as_deref(),
            Some("x.png"),
            "the hint must reach the dialog as a name only"
        );
        assert_eq!(chosen.name(), "export.png");
        assert_eq!(table.sink_path(chosen.id()).unwrap(), destination);
    }

    #[test]
    fn save_cancel_is_a_typed_none_and_adds_no_handle() {
        let table = Arc::new(FileTable::default());
        let picker = FakePicker::replying(Ok(None));
        assert_eq!(
            block(pick_destination(&picker, Arc::clone(&table), None)).unwrap(),
            None
        );
        assert_eq!(table.counts(), (0, 0));
        assert_eq!(
            picker.suggested.lock().unwrap().as_deref(),
            Some("image.png")
        );
    }

    #[test]
    fn save_destination_without_a_file_name_is_rejected() {
        let table = Arc::new(FileTable::default());
        let picker = FakePicker::replying(Ok(Some(PathBuf::from("/"))));
        let error = block(pick_destination(&picker, table, None)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidArgument);
    }
}
