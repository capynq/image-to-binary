//! Native processing owner. The desktop app holds one [`SessionSlot`] (one source, like the
//! browser) and runs every session call on the worker thread.
//!
//! Launch rule: a launch takes its token, submits to the worker and records itself as the active
//! job under one mutex. A preview or export launch must carry a generation greater than the last
//! one launched for the current source, and a result is accepted only while no later launch has
//! been made. Tokens and generations therefore advance in the same order.
//!
//! Argument rule: settings and generations arrive as raw JSON and are converted with the shared
//! argument contract, so a wrong type, fraction, negative or missing field yields the same
//! `{ code, message }` as the browser binding.

mod demo;
mod frame;
mod jobs;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use image_to_binary_adapters::session::args;
use image_to_binary_adapters::session::args::{BoolArg, StyleArgs, TextArg};
use image_to_binary_adapters::session::{PreviewSettings, Session};
use image_to_binary_core::Generation;
use serde::Deserialize;
use serde_json::Value;

pub(crate) use jobs::{
    ExportReport, ExportRequest, ImportedDemo, ImportedSource, PreviewMode, PreviewRequest,
    ProgressEvent, ProgressSink, SessionSlot,
};

use crate::error::{ErrorCode, IpcError};
use crate::files::{FileId, FileTable};
use crate::worker::{JobHandle, Worker};

/// The page's preview kind, as sent on the wire.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PreviewKind {
    Original,
    Artwork,
}

/// A JS-number view of a raw argument: a missing or non-number value is `NaN`, exactly as the
/// browser binding converts its arguments.
fn js_number(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(f64::NAN)
}

/// Converts the generation argument with the shared contract.
///
/// # Errors
///
/// Returns `invalid_argument` with the shared text for a missing, non-numeric, fractional,
/// negative or unsafe value.
pub(crate) fn generation_argument(value: Option<&Value>) -> Result<Generation, IpcError> {
    args::generation(js_number(value)).map_err(IpcError::from)
}

/// Converts the settings object with the shared contract, checking sampling first, then cell
/// size, font size and threshold, then mode, ramp and the render style.
///
/// # Errors
///
/// Returns the contract's `invalid_argument` or `invalid_settings` with its text.
pub(crate) fn settings_argument(value: Option<&Value>) -> Result<PreviewSettings, IpcError> {
    let field = |name: &str| value.and_then(|object| object.get(name));
    let sampling = field("sampling").and_then(Value::as_str);
    args::preview_settings_styled(
        js_number(field("cellSize")),
        js_number(field("fontSize")),
        js_number(field("threshold")),
        sampling,
        text_argument(field("mode")),
        text_argument(field("ramp")),
        StyleArgs {
            foreground: text_argument(field("foreground")),
            background: text_argument(field("background")),
            color_mode: text_argument(field("colorMode")),
            contrast: optional_number(field("contrast")),
            invert: bool_argument(field("invert")),
        },
    )
    .map_err(IpcError::from)
}

/// Maps a raw optional number field: a missing or `null` value is absent, any other value is a
/// JS-number view, so a non-number becomes `NaN` and is refused by the shared contract.
fn optional_number(value: Option<&Value>) -> Option<f64> {
    match value {
        None | Some(Value::Null) => None,
        Some(other) => Some(other.as_f64().unwrap_or(f64::NAN)),
    }
}

/// Maps a raw optional boolean field: a missing or `null` value is absent, a boolean is a value,
/// and any other JSON type is not a boolean.
fn bool_argument(value: Option<&Value>) -> BoolArg {
    match value {
        None | Some(Value::Null) => BoolArg::Absent,
        Some(Value::Bool(flag)) => BoolArg::Value(*flag),
        Some(_) => BoolArg::NotBool,
    }
}

/// Maps a raw optional text field onto the shared contract: a missing or `null` value is absent,
/// a string is text, and any other JSON type is not text.
fn text_argument(value: Option<&Value>) -> TextArg<'_> {
    match value {
        None | Some(Value::Null) => TextArg::Absent,
        Some(Value::String(text)) => TextArg::Text(text),
        Some(_) => TextArg::NotText,
    }
}

/// Accepts a worker result only while its token is still the newest launch.
///
/// # Errors
///
/// Returns `stale_generation` when a later launch exists.
pub(crate) fn accept_if_latest<T>(latest: u64, token: u64, outcome: T) -> Result<T, IpcError> {
    if latest == token {
        Ok(outcome)
    } else {
        Err(IpcError::new(
            ErrorCode::StaleGeneration,
            "the result was superseded by a newer request",
        ))
    }
}

/// How a launch relates to the page's generation sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Launch {
    /// A preview or export for this generation of the current source.
    Generation(u64),
    /// A source replacement. An import with a generation starts a new source epoch at that
    /// generation; a reset carries none and leaves the epoch unchanged.
    Source { generation: Option<u64> },
}

#[derive(Debug)]
struct ActiveJob {
    generation: Option<u64>,
    token: u64,
    handle: JobHandle,
}

/// Outcome channel of one import decode: the metadata and the session it would commit.
type ImportReceiver = tauri::async_runtime::Receiver<Result<(ImportedSource, Session), IpcError>>;

/// The import that most recently replaced the source and the source it displaced. It is kept
/// until the next launch, so a cancel that reaches the shell after the commit can restore the
/// displaced source.
struct Committed {
    generation: u64,
    displaced: Option<Session>,
}

#[derive(Default)]
struct Launches {
    token: u64,
    /// Highest accepted generation of the current source epoch. Later launches never lower it,
    /// so an older preview cannot be accepted after a newer one or a source change.
    high_water: Option<u64>,
    active: Option<ActiveJob>,
    /// Token of the active job that a cancel named, so its commit is refused.
    cancelled: Option<u64>,
    committed: Option<Committed>,
}

/// Managed state for the native pipeline. The launch mutex covers the token, the generation
/// order, the active job and the committed source, so they change together. The session slot is
/// only written while that mutex is held (launches before slot).
pub(crate) struct Processing {
    worker: Arc<Worker>,
    files: Arc<FileTable>,
    slot: Arc<SessionSlot>,
    launches: Mutex<Launches>,
}

impl Processing {
    pub(crate) fn new(worker: Arc<Worker>, files: Arc<FileTable>) -> Self {
        Self {
            worker,
            files,
            slot: Arc::new(SessionSlot::default()),
            launches: Mutex::new(Launches::default()),
        }
    }

    fn launches(&self) -> MutexGuard<'_, Launches> {
        self.launches.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Submits a job and records it as active, under the launch mutex. A generation not greater
    /// than the high-water mark is refused before anything is submitted. A job superseded before
    /// delivery drops its sender, so its receiver reports `cancelled` instead of hanging.
    fn launch<T, F>(
        &self,
        launch: Launch,
        body: F,
    ) -> Result<(u64, tauri::async_runtime::Receiver<Result<T, IpcError>>), IpcError>
    where
        T: Send + 'static,
        F: FnOnce(&crate::worker::JobContext) -> Result<T, IpcError> + Send + 'static,
    {
        let mut launches = self.launches();
        if let Launch::Generation(generation) = launch {
            if launches.high_water.is_some_and(|mark| generation <= mark) {
                return Err(IpcError::new(
                    ErrorCode::StaleGeneration,
                    "generation is not newer than the last accepted generation",
                ));
            }
        }
        let (sender, receiver) = tauri::async_runtime::channel::<Result<T, IpcError>>(1);
        let handle = self
            .worker
            .submit(body, move |_, outcome| {
                let _ = sender.try_send(outcome);
            })
            .map_err(IpcError::from)?;
        launches.token += 1;
        let token = launches.token;
        let generation = match launch {
            Launch::Generation(generation) => Some(generation),
            Launch::Source { generation } => generation,
        };
        if let Some(value) = generation {
            launches.high_water = Some(value);
        }
        launches.committed = None;
        launches.active = Some(ActiveJob {
            generation,
            token,
            handle,
        });
        Ok((token, receiver))
    }

    fn latest(&self) -> u64 {
        self.launches().token
    }

    /// Launches the decode of an import as the source epoch at `generation`. The held source is
    /// not replaced until [`Self::commit_import`] accepts the outcome.
    fn begin_import(
        &self,
        file: FileId,
        generation: u64,
    ) -> Result<(u64, ImportReceiver), IpcError> {
        let files = Arc::clone(&self.files);
        self.launch(
            Launch::Source {
                generation: Some(generation),
            },
            move |_| jobs::decode_import(&files, file),
        )
    }

    /// Replaces the held source with an accepted import. The check and the write happen under the
    /// launch mutex: a superseded or cancelled import is refused and never touches the slot.
    ///
    /// # Errors
    ///
    /// Returns `cancelled` when a later launch, or a cancel of this generation, precedes the commit.
    fn commit_import<T>(
        &self,
        token: u64,
        outcome: (T, Session),
        generation: u64,
    ) -> Result<T, IpcError> {
        let (metadata, session) = outcome;
        let mut launches = self.launches();
        if launches.token != token || launches.cancelled == Some(token) {
            return Err(IpcError::new(
                ErrorCode::Cancelled,
                "the import was superseded or cancelled before it committed",
            ));
        }
        let displaced = self.slot.replace(Some(session));
        launches.committed = Some(Committed {
            generation,
            displaced,
        });
        Ok(metadata)
    }

    /// Imports the file behind `file` as the source of the epoch at `generation`.
    ///
    /// # Errors
    ///
    /// Returns the import errors, or `cancelled` when this import is superseded or cancelled.
    pub(crate) async fn import_source(
        &self,
        file: FileId,
        generation: u64,
    ) -> Result<ImportedSource, IpcError> {
        let (token, receiver) = self.begin_import(file, generation)?;
        let outcome = await_outcome(receiver).await?;
        self.commit_import(token, outcome, generation)
    }

    /// Replaces the source with the built-in demo fixture. Generated on the worker, so no pixels
    /// cross IPC, and the demo is the same pixels the browser's demo import produces.
    ///
    /// # Errors
    ///
    /// Returns `cancelled` when this import is superseded or cancelled before it commits.
    pub(crate) async fn import_demo(&self, generation: u64) -> Result<ImportedDemo, IpcError> {
        let (token, receiver) = self.launch(
            Launch::Source {
                generation: Some(generation),
            },
            |_: &crate::worker::JobContext| jobs::build_demo(),
        )?;
        let outcome = await_outcome(receiver).await?;
        self.commit_import(token, outcome, generation)
    }

    /// Renders a preview. The frame is accepted only if no later launch arrived.
    ///
    /// # Errors
    ///
    /// Returns `stale_generation` for a generation not newer than the high-water mark or a
    /// superseded result, `cancelled` for a superseded job, and the render errors.
    pub(crate) async fn render_preview<P>(
        &self,
        request: PreviewRequest,
        progress: P,
    ) -> Result<Vec<u8>, IpcError>
    where
        P: ProgressSink + Send + 'static,
    {
        let slot = Arc::clone(&self.slot);
        let (token, receiver) = self
            .launch(Launch::Generation(request.generation), move |context| {
                jobs::render_preview(context, &slot, &progress, request)
            })?;
        let frame = await_outcome(receiver).await?;
        accept_if_latest(self.latest(), token, frame)
    }

    /// Exports a PNG into the chosen sink. A committed file is reported even if a later launch
    /// arrives during the commit, because the file exists; the consumer compares generations.
    ///
    /// # Errors
    ///
    /// Returns `stale_generation` for a generation not newer than the high-water mark, and the
    /// export errors. A job superseded before its commit reports `cancelled`.
    pub(crate) async fn export_png<P>(
        &self,
        request: ExportRequest,
        progress: P,
    ) -> Result<ExportReport, IpcError>
    where
        P: ProgressSink + Send + 'static,
    {
        let slot = Arc::clone(&self.slot);
        let files = Arc::clone(&self.files);
        let (_, receiver) = self
            .launch(Launch::Generation(request.generation), move |context| {
                jobs::export_png(context, &slot, &files, &progress, request)
            })?;
        await_outcome(receiver).await
    }

    /// Cancels the active job when it belongs to `generation`. An import with that generation is
    /// cancelled too, so its commit is refused. If the import already committed, the source it
    /// displaced is restored. Cancelling a finished or unknown generation is a no-op.
    pub(crate) fn cancel(&self, generation: u64) {
        let mut launches = self.launches();
        let cancelled = launches
            .active
            .as_ref()
            .filter(|active| active.generation == Some(generation))
            .map(|active| {
                active.handle.cancel();
                active.token
            });
        if let Some(token) = cancelled {
            launches.cancelled = Some(token);
        }
        if let Some(committed) = launches
            .committed
            .take_if(|committed| committed.generation == generation)
        {
            let _ = self.slot.replace(committed.displaced);
        }
    }

    /// Cancels all work and releases the held source.
    ///
    /// # Errors
    ///
    /// Returns `cancelled` when a later launch supersedes the reset before it runs.
    pub(crate) async fn reset(&self) -> Result<(), IpcError> {
        let slot = Arc::clone(&self.slot);
        let (_, receiver) = self.launch(Launch::Source { generation: None }, move |_| {
            let _ = slot.replace(None);
            Ok(())
        })?;
        await_outcome(receiver).await
    }
}

async fn await_outcome<T>(
    mut receiver: tauri::async_runtime::Receiver<Result<T, IpcError>>,
) -> Result<T, IpcError> {
    receiver.recv().await.unwrap_or_else(|| {
        Err(IpcError::new(
            ErrorCode::Cancelled,
            "the job was superseded or cancelled",
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::{Barrier, Mutex as StdMutex};
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::files::test_support::TestDir;
    use crate::processing::jobs::ProgressStage;

    struct Record(Arc<StdMutex<Vec<ProgressEvent>>>);

    impl ProgressSink for Record {
        fn report(&self, event: ProgressEvent) {
            self.0.lock().unwrap().push(event);
        }
    }
    use crate::worker::Worker;

    const WAIT: Duration = Duration::from_secs(20);

    fn processing() -> Arc<Processing> {
        Arc::new(Processing::new(
            Arc::new(Worker::spawn().unwrap()),
            Arc::new(FileTable::default()),
        ))
    }

    #[allow(clippy::needless_pass_by_value)]
    fn settings_json(cell: Value, font: Value, threshold: Value, sampling: Value) -> Value {
        json!({ "cellSize": cell, "fontSize": font, "threshold": threshold, "sampling": sampling })
    }

    #[test]
    fn settings_match_the_browser_contract_for_each_wrong_argument() {
        let valid = settings_argument(Some(&settings_json(
            json!(5),
            json!(10),
            json!(128),
            json!("average"),
        )));
        assert_eq!(
            valid.unwrap(),
            PreviewSettings::new(5, 10, 128, image_to_binary_core::Sampling::Average).unwrap()
        );
        let cases = [
            (
                settings_json(json!(5), json!(10), json!(-1), json!("classic")),
                "threshold must be a non-negative safe integer",
            ),
            (
                settings_json(json!(5.5), json!(10), json!(128), json!("classic")),
                "cellSize must be a non-negative safe integer",
            ),
            (
                settings_json(json!("5"), json!(10), json!(128), json!("classic")),
                "cellSize must be a non-negative safe integer",
            ),
            (
                json!({ "cellSize": 5, "threshold": 128, "sampling": "classic" }),
                "fontSize must be a non-negative safe integer",
            ),
            (
                settings_json(json!(4), json!(10), json!(128), json!("classic")),
                "cellSize must be between 5 and 30",
            ),
            (
                settings_json(json!(5), json!(10), json!(128), json!("median")),
                "sampling must be \"classic\" or \"average\"",
            ),
            (
                settings_json(json!(5), json!(10), json!(128), json!(7)),
                "sampling must be a string",
            ),
        ];
        for (value, message) in cases {
            let error = settings_argument(Some(&value)).unwrap_err();
            assert_eq!(error.message(), message);
            let contract = args::preview_settings(
                value["cellSize"].as_f64().unwrap_or(f64::NAN),
                value["fontSize"].as_f64().unwrap_or(f64::NAN),
                value["threshold"].as_f64().unwrap_or(f64::NAN),
                value["sampling"].as_str(),
                text_argument(value.get("mode")),
                text_argument(value.get("ramp")),
            )
            .unwrap_err();
            assert_eq!(
                (error.code(), error.message()),
                (contract.code(), contract.message().as_str())
            );
        }
        assert_eq!(
            settings_argument(None).unwrap_err().message(),
            "sampling must be a string"
        );
    }

    #[test]
    fn mode_and_ramp_reach_the_shared_contract_over_ipc() {
        let mut value = settings_json(json!(5), json!(10), json!(128), json!("average"));
        value["mode"] = json!("ascii");
        value["ramp"] = json!(" .#@");
        let settings = settings_argument(Some(&value)).unwrap();
        assert_eq!(
            settings,
            PreviewSettings::new(5, 10, 128, image_to_binary_core::Sampling::Average)
                .unwrap()
                .with_mode(image_to_binary_core::RenderMode::Ascii(
                    image_to_binary_core::Ramp::new(" .#@").unwrap()
                ))
                .unwrap()
        );
        value["ramp"] = json!("");
        assert_eq!(
            settings_argument(Some(&value)).unwrap_err().message(),
            "ramp must contain at least one character"
        );
        value["ramp"] = json!(7);
        assert_eq!(
            settings_argument(Some(&value)).unwrap_err().message(),
            "ramp must be a string"
        );
        value["ramp"] = Value::Null;
        value["mode"] = json!("block");
        assert_eq!(
            settings_argument(Some(&value)).unwrap().mode(),
            image_to_binary_core::RenderMode::Block
        );
        value["mode"] = json!("binary");
        value["sampling"] = json!("classic");
        assert_eq!(
            settings_argument(Some(&value)).unwrap().mode(),
            image_to_binary_core::RenderMode::Binary
        );
        value["mode"] = json!("block");
        assert_eq!(
            settings_argument(Some(&value)).unwrap_err().message(),
            "ascii and block modes require average sampling"
        );
    }

    #[test]
    fn style_fields_reach_the_shared_contract_over_ipc() {
        let mut value = settings_json(json!(5), json!(10), json!(128), json!("classic"));
        assert_eq!(
            settings_argument(Some(&value)).unwrap().style(),
            image_to_binary_processing::RenderStyle::default()
        );
        value["foreground"] = json!("#FF8000");
        value["background"] = json!("#102030");
        value["colorMode"] = json!("source");
        value["contrast"] = json!(-40);
        value["invert"] = json!(true);
        let style = settings_argument(Some(&value)).unwrap().style();
        assert_eq!(
            style,
            image_to_binary_processing::RenderStyle {
                foreground: image_to_binary_processing::Rgb {
                    red: 255,
                    green: 128,
                    blue: 0
                },
                background: image_to_binary_processing::Rgb {
                    red: 16,
                    green: 32,
                    blue: 48
                },
                color: image_to_binary_processing::ColorMode::Source,
                contrast: -40,
                invert: true,
            }
        );
        value["invert"] = json!(false);
        assert!(!settings_argument(Some(&value)).unwrap().style().invert);
        value["invert"] = json!("yes");
        assert_eq!(
            settings_argument(Some(&value)).unwrap_err().message(),
            "invert must be a boolean"
        );
        value["invert"] = json!(true);
        value["contrast"] = json!(101);
        assert_eq!(
            settings_argument(Some(&value)).unwrap_err().message(),
            "contrast must be between -100 and 100"
        );
        value["contrast"] = json!(0);
        value["foreground"] = json!("red");
        assert_eq!(
            settings_argument(Some(&value)).unwrap_err().message(),
            "foreground must be a #RRGGBB colour"
        );
        value["foreground"] = json!("#00ff00");
        value["colorMode"] = json!("Source");
        assert_eq!(
            settings_argument(Some(&value)).unwrap_err().message(),
            "colorMode must be \"monochrome\" or \"source\""
        );
    }

    #[test]
    fn generation_arguments_match_the_browser_contract() {
        assert_eq!(
            generation_argument(Some(&json!(0))).unwrap(),
            Generation::new(0)
        );
        for value in [json!("5"), json!(5.5), json!(-1), Value::Null] {
            let error = generation_argument(Some(&value)).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidArgument);
            assert_eq!(
                error.message(),
                "generation must be a non-negative safe integer"
            );
        }
        assert_eq!(
            generation_argument(None).unwrap_err().message(),
            "generation must be a non-negative safe integer"
        );
    }

    #[test]
    fn a_superseded_result_is_never_accepted() {
        assert_eq!(accept_if_latest(7, 7, "frame").unwrap(), "frame");
        assert_eq!(
            accept_if_latest(8, 7, "frame").unwrap_err().code(),
            ErrorCode::StaleGeneration
        );
    }

    #[test]
    fn a_launch_below_the_last_generation_is_refused_before_it_runs() {
        let processing = processing();
        let ran = Arc::new(StdMutex::new(Vec::new()));
        let record = |generation: u64| {
            let ran = Arc::clone(&ran);
            move |_: &crate::worker::JobContext| -> Result<u64, IpcError> {
                ran.lock().unwrap().push(generation);
                Ok(generation)
            }
        };
        processing.launch(Launch::Generation(5), record(5)).unwrap();
        assert_eq!(
            processing
                .launch(Launch::Generation(3), record(3))
                .unwrap_err()
                .code(),
            ErrorCode::StaleGeneration
        );
        assert_eq!(
            processing
                .launch(Launch::Generation(5), record(5))
                .unwrap_err()
                .code(),
            ErrorCode::StaleGeneration
        );
        processing
            .launch(
                Launch::Source { generation: None },
                |_: &crate::worker::JobContext| Ok::<(), IpcError>(()),
            )
            .unwrap();
        assert!(matches!(
            processing.launch(Launch::Generation(0), record(0)),
            Err(error) if error.code() == ErrorCode::StaleGeneration
        ));
        let (_, receiver) = processing.launch(Launch::Generation(6), record(6)).unwrap();
        assert_eq!(
            tauri::async_runtime::block_on(await_outcome(receiver)).unwrap(),
            6
        );
        let order = ran.lock().unwrap().clone();
        assert!(!order.contains(&3), "a refused launch ran: {order:?}");
        assert!(order.iter().filter(|generation| **generation == 5).count() <= 1);
    }

    #[test]
    fn a_cancelled_import_never_replaces_the_source() {
        let dir = TestDir::new("mod-cancelled-import");
        let processing = processing();
        import_bmp(&processing, &dir);
        let small = processing
            .files
            .insert_source(dir.file("small.bmp", &bmp_bytes(10, 10)))
            .unwrap();
        let (token, receiver) = processing.begin_import(small, 2).unwrap();
        processing.cancel(2);
        drop(receiver);
        let outcome = jobs::decode_import(&processing.files, small).unwrap();
        assert_eq!(
            processing
                .commit_import(token, outcome, 2)
                .unwrap_err()
                .code(),
            ErrorCode::Cancelled
        );
        assert_eq!(source_dimensions(&processing), (40, 30));
        tauri::async_runtime::block_on(processing.render_preview(
            artwork_request(3),
            Record(Arc::new(StdMutex::new(Vec::new()))),
        ))
        .expect("the previous source still renders");
        assert_eq!(source_dimensions(&processing), (40, 30));
    }

    #[test]
    fn cancelling_a_committed_import_restores_the_previous_source() {
        let dir = TestDir::new("mod-restore");
        let processing = processing();
        import_bmp(&processing, &dir);
        let small = processing
            .files
            .insert_source(dir.file("small.bmp", &bmp_bytes(10, 10)))
            .unwrap();
        tauri::async_runtime::block_on(processing.import_source(small, 2)).unwrap();
        assert_eq!(source_dimensions(&processing), (10, 10));
        processing.cancel(2);
        assert_eq!(source_dimensions(&processing), (40, 30));
    }

    #[test]
    fn an_import_epoch_keeps_refusing_generations_below_its_start() {
        let processing = processing();
        let accepted = |launch: Launch| {
            processing
                .launch(launch, |_: &crate::worker::JobContext| {
                    Ok::<(), IpcError>(())
                })
                .is_ok()
        };
        assert!(accepted(Launch::Generation(5)));
        assert!(accepted(Launch::Source {
            generation: Some(6)
        }));
        assert!(!accepted(Launch::Generation(3)));
        assert!(!accepted(Launch::Generation(6)));
        assert!(accepted(Launch::Generation(7)));
        assert!(accepted(Launch::Source { generation: None }));
        assert!(!accepted(Launch::Generation(7)));
        assert!(accepted(Launch::Generation(8)));
    }

    #[test]
    fn concurrent_launches_run_in_generation_order() {
        let processing = processing();
        let barrier = Arc::new(Barrier::new(2));
        let ran = Arc::new(StdMutex::new(Vec::new()));
        let handles: Vec<_> = [1_u64, 2]
            .into_iter()
            .map(|generation| {
                let processing = Arc::clone(&processing);
                let barrier = Arc::clone(&barrier);
                let ran = Arc::clone(&ran);
                std::thread::spawn(move || {
                    barrier.wait();
                    processing
                        .launch(Launch::Generation(generation), move |_| {
                            ran.lock().unwrap().push(generation);
                            Ok::<(), IpcError>(())
                        })
                        .map(|_| ())
                        .err()
                        .map(|error| error.code())
                })
            })
            .collect();
        let refused: Vec<Option<ErrorCode>> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(
            refused.iter().any(Option::is_none),
            "one launch must be accepted"
        );
        assert!(refused
            .iter()
            .flatten()
            .all(|code| *code == ErrorCode::StaleGeneration));
        let started = ran.lock().unwrap().clone();
        assert!(
            started.windows(2).all(|pair| pair[0] < pair[1]),
            "{started:?}"
        );
    }

    /// Blocks the first call only, and signals the test when it has been reached.
    struct Blocking {
        armed: std::sync::atomic::AtomicBool,
        reached: StdMutex<Sender<()>>,
        release: StdMutex<Receiver<()>>,
    }

    impl Blocking {
        fn new() -> (Self, Receiver<()>, Sender<()>) {
            let (reached_tx, reached_rx) = channel();
            let (release_tx, release_rx) = channel();
            (
                Self {
                    armed: std::sync::atomic::AtomicBool::new(true),
                    reached: StdMutex::new(reached_tx),
                    release: StdMutex::new(release_rx),
                },
                reached_rx,
                release_tx,
            )
        }

        fn pause(&self) {
            if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                let _ = self.reached.lock().unwrap().send(());
                let _ = self.release.lock().unwrap().recv_timeout(WAIT);
            }
        }
    }

    impl ProgressSink for Blocking {
        fn report(&self, _event: ProgressEvent) {
            self.pause();
        }
    }

    struct CommitGate(Blocking);

    impl ProgressSink for CommitGate {
        fn report(&self, _event: ProgressEvent) {}

        fn committed(&self) {
            self.0.pause();
        }
    }

    fn import_bmp(processing: &Processing, dir: &TestDir) {
        let id = processing.files.insert_source(bmp_fixture(dir)).unwrap();
        tauri::async_runtime::block_on(processing.import_source(id, 0)).unwrap();
    }

    fn bmp_fixture(dir: &TestDir) -> PathBuf {
        dir.file("source.bmp", &bmp_bytes(40, 30))
    }

    fn bmp_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"BM".to_vec();
        let row = (width * 3).div_ceil(4) * 4;
        let data = row * height;
        bytes.extend_from_slice(&(54 + data).to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0]);
        bytes.extend_from_slice(&width.to_le_bytes());
        bytes.extend_from_slice(&height.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 24, 0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&data.to_le_bytes());
        bytes.extend_from_slice(&[0; 16]);
        for y in 0..height {
            for x in 0..width {
                bytes.extend_from_slice(&[
                    u8::try_from((x * 9 + y) % 256).unwrap(),
                    u8::try_from((x * 3) % 256).unwrap(),
                    200,
                ]);
            }
            bytes.resize(bytes.len() + usize::try_from(row - width * 3).unwrap(), 0);
        }
        bytes
    }

    fn source_dimensions(processing: &Processing) -> (u32, u32) {
        let slot = processing.slot.lock();
        let dimensions = slot
            .as_ref()
            .expect("a source is held")
            .source_metadata()
            .unwrap()
            .dimensions();
        (dimensions.width(), dimensions.height())
    }

    fn artwork_request(generation: u64) -> PreviewRequest {
        PreviewRequest {
            generation,
            mode: PreviewMode::Artwork(
                PreviewSettings::new(5, 10, 128, image_to_binary_core::Sampling::Classic).unwrap(),
            ),
        }
    }

    fn export_request(generation: u64, sink: FileId) -> ExportRequest {
        ExportRequest {
            generation,
            settings: PreviewSettings::new(5, 10, 128, image_to_binary_core::Sampling::Classic)
                .unwrap(),
            sink,
        }
    }

    #[test]
    fn the_session_lock_is_free_while_a_progress_send_is_blocked() {
        let dir = TestDir::new("mod-lock-preview");
        let processing = processing();
        import_bmp(&processing, &dir);
        let (sink, reached, release) = Blocking::new();
        let worker_processing = Arc::clone(&processing);
        let rendering = std::thread::spawn(move || {
            tauri::async_runtime::block_on(
                worker_processing.render_preview(artwork_request(1), sink),
            )
        });
        reached.recv_timeout(WAIT).unwrap();
        assert!(
            processing.slot.try_lock().is_some(),
            "the session lock was held across a progress send"
        );
        release.send(()).unwrap();
        rendering.join().unwrap().unwrap();
    }

    #[test]
    fn the_session_lock_is_free_while_an_export_progress_send_is_blocked() {
        let dir = TestDir::new("mod-lock-export");
        let processing = processing();
        import_bmp(&processing, &dir);
        let sink_id = processing
            .files
            .insert_sink(dir.path().join("out.png"))
            .unwrap();
        let (sink, reached, release) = Blocking::new();
        let worker_processing = Arc::clone(&processing);
        let exporting = std::thread::spawn(move || {
            tauri::async_runtime::block_on(
                worker_processing.export_png(export_request(1, sink_id), sink),
            )
        });
        reached.recv_timeout(WAIT).unwrap();
        assert!(processing.slot.try_lock().is_some());
        release.send(()).unwrap();
        exporting.join().unwrap().unwrap();
    }

    #[test]
    fn a_committed_export_is_reported_even_when_a_later_launch_arrives_during_the_commit() {
        let dir = TestDir::new("mod-commit-point");
        let processing = processing();
        import_bmp(&processing, &dir);
        let destination = dir.path().join("out.png");
        let sink_id = processing.files.insert_sink(destination.clone()).unwrap();
        let (gate, reached, release) = Blocking::new();
        let worker_processing = Arc::clone(&processing);
        let exporting = std::thread::spawn(move || {
            tauri::async_runtime::block_on(
                worker_processing.export_png(export_request(1, sink_id), CommitGate(gate)),
            )
        });
        reached.recv_timeout(WAIT).unwrap();
        let before = processing.latest();
        let resetting_processing = Arc::clone(&processing);
        let resetting = std::thread::spawn(move || {
            tauri::async_runtime::block_on(resetting_processing.reset())
        });
        while processing.latest() == before {
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        let report = exporting.join().unwrap().unwrap();
        assert_eq!(report.generation, 1);
        assert_eq!(
            report.byte_length,
            std::fs::metadata(&destination).unwrap().len()
        );
        resetting.join().unwrap().unwrap();
    }

    #[test]
    fn a_progress_stage_reaches_the_sink_for_a_completed_preview() {
        let dir = TestDir::new("mod-stages");
        let processing = processing();
        import_bmp(&processing, &dir);
        let events = Arc::new(StdMutex::new(Vec::new()));
        let frame = tauri::async_runtime::block_on(
            processing.render_preview(artwork_request(4), Record(Arc::clone(&events))),
        )
        .unwrap();
        assert!(frame.len() > 24);
        let stages: Vec<ProgressStage> = events.lock().unwrap().iter().map(|e| e.stage).collect();
        assert_eq!(stages.last(), Some(&ProgressStage::Ready));
    }

    #[cfg(feature = "native-codecs")]
    #[test]
    fn a_heic_file_imports_through_import_source_when_the_codec_is_built_in() {
        let files = Arc::new(FileTable::default());
        let processing = Processing::new(Arc::new(Worker::spawn().unwrap()), Arc::clone(&files));
        let id = files.insert_source(heic_sample()).unwrap();
        let imported = tauri::async_runtime::block_on(processing.import_source(id, 1)).unwrap();
        assert_eq!(imported.name, "sample.heic");
        assert_eq!((imported.width, imported.height), (64, 48));
        assert_eq!(imported.input_bytes, 653);
        assert_eq!(imported.source, "file");
    }

    #[cfg(not(feature = "native-codecs"))]
    #[test]
    fn a_heic_file_reports_codec_unavailable_without_the_native_codecs() {
        let files = Arc::new(FileTable::default());
        let processing = Processing::new(Arc::new(Worker::spawn().unwrap()), Arc::clone(&files));
        let id = files.insert_source(heic_sample()).unwrap();
        let error = tauri::async_runtime::block_on(processing.import_source(id, 1)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::CodecUnavailable);
    }

    fn heic_sample() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../processing/tests/fixtures/heif/sample.heic"
        ))
    }
}
