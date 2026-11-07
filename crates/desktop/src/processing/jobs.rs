//! Job bodies that run on the worker thread. Every body takes the worker's [`JobContext`] and
//! checks it before each step, so a superseded or cancelled job stops at its next boundary and
//! releases its session work and any partial sink.
//!
//! Locking rule: the session lock is taken for one session call at a time and released before
//! any progress report, IPC send or file write, so a blocked sender never holds the session.

use std::io::{self, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};

#[cfg(test)]
use std::sync::TryLockError;

use image_to_binary_adapters::session::{
    EncodedPlan, PreviewSettings, Session, SessionError, Stage, DEFAULT_PIXEL_BUDGET,
};
use image_to_binary_core::Generation;
use serde::Serialize;
use tauri::ipc::Channel;

use super::frame::encode_frame;
use crate::error::{ErrorCode, IpcError};
use crate::files::read::read_regular_file;
use crate::files::sink::AtomicFileSink;
use crate::files::{display_name, FileId, FileTable};
use crate::limits::MAX_ENCODED_INPUT_BYTES;
use crate::worker::JobContext;

/// The one source the native shell holds. Only worker jobs lock it, one session call at a time.
#[derive(Debug, Default)]
pub(crate) struct SessionSlot {
    session: Mutex<Option<Session>>,
}

impl SessionSlot {
    /// Locks the held session. Callers keep the guard for one session call only.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn try_lock(&self) -> Option<MutexGuard<'_, Option<Session>>> {
        match self.session.try_lock() {
            Ok(guard) => Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

/// Stage named in a progress event. Only the stages a page can observe are represented.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ProgressStage {
    Sampling,
    Preview,
    Encoding,
    Ready,
}

/// Small JSON progress record sent over a channel while a job runs. `done` and `total` are in
/// the stage's own unit, exactly as the session reports them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct ProgressEvent {
    pub(crate) generation: u64,
    pub(crate) stage: ProgressStage,
    pub(crate) done: u64,
    pub(crate) total: u64,
}

/// Receives progress from a job. Calls are made without the session lock held, so an
/// implementation may block until it is released.
pub(crate) trait ProgressSink {
    fn report(&self, event: ProgressEvent);

    /// Called once an export has committed its file, before the job returns.
    fn committed(&self) {}
}

impl ProgressSink for Channel<ProgressEvent> {
    fn report(&self, event: ProgressEvent) {
        let _ = self.send(event);
    }
}

/// Destination of export chunks. `commit` publishes the bytes; dropping the sink without it
/// leaves no file behind.
pub(crate) trait ChunkSink {
    fn write_chunk(&mut self, chunk: &[u8]) -> io::Result<()>;
    fn commit(self) -> io::Result<()>;
}

impl ChunkSink for AtomicFileSink {
    fn write_chunk(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.write_all(chunk)
    }

    fn commit(self) -> io::Result<()> {
        AtomicFileSink::commit(self)
    }
}

fn progress_stage(stage: Stage) -> Option<ProgressStage> {
    match stage {
        Stage::Sampling => Some(ProgressStage::Sampling),
        Stage::Preview => Some(ProgressStage::Preview),
        Stage::Encoding => Some(ProgressStage::Encoding),
        Stage::Ready => Some(ProgressStage::Ready),
        Stage::Idle | Stage::Failed => None,
    }
}

fn no_source() -> IpcError {
    IpcError::new(
        ErrorCode::NoJob,
        "there is no source; import an image first",
    )
}

fn shared(error: SessionError) -> IpcError {
    IpcError::from(error)
}

/// Maps a file-system failure to a code and a fixed message. The path never appears.
fn file_failure(error: &io::Error) -> IpcError {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput => IpcError::new(
            ErrorCode::InvalidArgument,
            "the chosen destination is not available",
        ),
        kind => IpcError::new(
            ErrorCode::Internal,
            format!("the file could not be written: {kind:?}"),
        ),
    }
}

/// Runs one session call under the lock, which is released when the call returns.
fn with_session<R>(
    slot: &SessionSlot,
    step: impl FnOnce(&mut Session) -> Result<R, IpcError>,
) -> Result<R, IpcError> {
    let mut guard = slot.lock();
    let session = guard.as_mut().ok_or_else(no_source)?;
    step(session)
}

/// Metadata for a newly imported source. Field names match the browser's import result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportedSource {
    pub(crate) name: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) input_bytes: u64,
    pub(crate) source: &'static str,
}

/// Reads and decodes the chosen file, then replaces the held source. A rejected file leaves the
/// previous source in place.
///
/// # Errors
///
/// Returns the read errors of [`read_regular_file`] (`invalid_argument` for a non-regular file,
/// `encoded_too_large` above the input ceiling) and the decoder's typed codes.
pub(crate) fn import(
    files: &FileTable,
    slot: &SessionSlot,
    file: FileId,
) -> Result<ImportedSource, IpcError> {
    let path = files.source_path(file)?;
    let name = display_name(&path);
    let bytes = read_regular_file(&path, MAX_ENCODED_INPUT_BYTES)?;
    let input_bytes = u64::try_from(bytes.len())
        .map_err(|_| IpcError::new(ErrorCode::ArithmeticOverflow, "the input size overflows"))?;
    EncodedPlan::new(bytes.len()).map_err(shared)?;
    let session = EncodedPlan::decode_session(bytes).map_err(shared)?;
    let dimensions = session.source_metadata().map_err(shared)?.dimensions();
    *slot.lock() = Some(session);
    Ok(ImportedSource {
        name,
        width: dimensions.width(),
        height: dimensions.height(),
        input_bytes,
        source: "file",
    })
}

/// Which preview the page asked for: the original image, or the artwork rendered with settings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewMode {
    Original,
    Artwork(PreviewSettings),
}

/// One preview request. `generation` is the page's generation for the session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreviewRequest {
    pub(crate) generation: u64,
    pub(crate) mode: PreviewMode,
}

/// Renders a preview to completion and returns its raw frame body.
///
/// # Errors
///
/// Returns `no_job` without a source, the session's typed failure, `cancelled` or
/// `stale_generation` once the job is superseded, and the frame encoder's bound errors.
pub(crate) fn render_preview<P: ProgressSink>(
    context: &JobContext,
    slot: &SessionSlot,
    progress: &P,
    request: PreviewRequest,
) -> Result<Vec<u8>, IpcError> {
    let generation = Generation::new(request.generation);
    with_session(slot, |session| {
        match request.mode {
            PreviewMode::Original => session.start_original_preview(generation),
            PreviewMode::Artwork(settings) => session.start_preview(generation, settings),
        }
        .map(|_| ())
        .map_err(shared)
    })?;
    drive(context, slot, generation, progress, false, |_| Ok(()))?;
    let (width, height, pixels) = with_session(slot, |session| {
        let mut frame = session.finish(generation).map_err(shared)?;
        let (width, height) = (frame.width(), frame.height());
        let pixels = frame.take_pixels().map_err(shared)?;
        Ok((width, height, pixels))
    })?;
    checkpoint(context)?;
    encode_frame(request.generation, width, height, &pixels)
}

/// One PNG export. The sink is a temporary sibling of the destination until it commits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExportRequest {
    pub(crate) generation: u64,
    pub(crate) settings: PreviewSettings,
    pub(crate) sink: FileId,
}

/// Summary of a committed export. The page never receives the destination path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportReport {
    pub(crate) generation: u64,
    pub(crate) byte_length: u64,
}

/// Streams a PNG export through the shared session into an atomic sink and commits it once the
/// stream is complete.
///
/// # Errors
///
/// Returns `invalid_argument` for an unavailable destination, and the errors of [`export_into`].
pub(crate) fn export_png<P: ProgressSink>(
    context: &JobContext,
    slot: &SessionSlot,
    files: &FileTable,
    progress: &P,
    request: ExportRequest,
) -> Result<ExportReport, IpcError> {
    let path = files.sink_path(request.sink)?;
    let sink = AtomicFileSink::create(&path).map_err(|error| file_failure(&error))?;
    export_into(context, slot, progress, request, sink)
}

/// Writes a complete export into `sink` and commits it. Cancellation, supersession or any failure
/// before the commit drops the sink, which removes its temporary bytes. A committed file is
/// reported even if the job is superseded during the commit, because the file exists; the commit
/// point makes the worker deliver that result.
///
/// # Errors
///
/// Returns the session's typed failure, `internal` for a write or commit failure, and `cancelled`
/// or `stale_generation` when the job stops before the commit.
pub(crate) fn export_into<K: ChunkSink, P: ProgressSink>(
    context: &JobContext,
    slot: &SessionSlot,
    progress: &P,
    request: ExportRequest,
    mut sink: K,
) -> Result<ExportReport, IpcError> {
    let generation = Generation::new(request.generation);
    with_session(slot, |session| {
        session
            .start_export(generation, request.settings)
            .map(|_| ())
            .map_err(shared)
    })?;
    let mut byte_length = 0_u64;
    drive(context, slot, generation, progress, true, |chunks| {
        write_chunks(&mut sink, chunks, &mut byte_length)
    })?;
    let trailer = with_session(slot, |session| {
        session.finish_export(generation).map_err(shared)
    })?;
    write_chunks(&mut sink, trailer, &mut byte_length)?;
    checkpoint(context)?;
    sink.commit().map_err(|error| file_failure(&error))?;
    context.commit_point();
    progress.committed();
    Ok(ExportReport {
        generation: request.generation,
        byte_length,
    })
}

fn write_chunks<K: ChunkSink>(
    sink: &mut K,
    chunks: Vec<Vec<u8>>,
    byte_length: &mut u64,
) -> Result<(), IpcError> {
    for chunk in chunks {
        sink.write_chunk(&chunk)
            .map_err(|error| file_failure(&error))?;
        let length = u64::try_from(chunk.len()).map_err(|_| {
            IpcError::new(ErrorCode::ArithmeticOverflow, "the export size overflows")
        })?;
        *byte_length = byte_length.checked_add(length).ok_or_else(|| {
            IpcError::new(ErrorCode::ArithmeticOverflow, "the export size overflows")
        })?;
    }
    Ok(())
}

fn checkpoint(context: &JobContext) -> Result<(), IpcError> {
    context
        .checkpoint()
        .map_err(|error| shared(SessionError::from(error)))
}

/// Advances the sampled job one budget at a time until it is ready. Each step checks the job
/// context, takes the session lock for one call, releases it, then writes chunks and reports
/// progress. A failure cancels the session job so its partial work is released.
fn drive<P, W>(
    context: &JobContext,
    slot: &SessionSlot,
    generation: Generation,
    progress: &P,
    drain: bool,
    mut write: W,
) -> Result<(), IpcError>
where
    P: ProgressSink,
    W: FnMut(Vec<Vec<u8>>) -> Result<(), IpcError>,
{
    let outcome = drive_steps(context, slot, generation, progress, drain, &mut write);
    if outcome.is_err() {
        let _ = with_session(slot, |session| session.cancel(generation).map_err(shared));
    }
    outcome
}

fn drive_steps<P, W>(
    context: &JobContext,
    slot: &SessionSlot,
    generation: Generation,
    progress: &P,
    drain: bool,
    write: &mut W,
) -> Result<(), IpcError>
where
    P: ProgressSink,
    W: FnMut(Vec<Vec<u8>>) -> Result<(), IpcError>,
{
    loop {
        checkpoint(context)?;
        let (stage, chunks, done, total) = with_session(slot, |session| {
            let stage = session
                .advance(generation, DEFAULT_PIXEL_BUDGET)
                .map_err(shared)?;
            let chunks = if drain {
                session.drain_export(generation).map_err(shared)?
            } else {
                Vec::new()
            };
            let (done, total) = session.progress().map_err(shared)?;
            Ok((stage, chunks, done, total))
        })?;
        write(chunks)?;
        if let Some(reported) = progress_stage(stage) {
            progress.report(ProgressEvent {
                generation: generation.value(),
                stage: reported,
                done,
                total,
            });
        }
        if stage == Stage::Ready {
            return checkpoint(context);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::files::test_support::TestDir;
    use crate::worker::{JobHandle, Worker, WORKER_THREAD_NAME};

    const WAIT: Duration = Duration::from_secs(20);

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Vec<ProgressEvent>>,
        threads: Mutex<Vec<Option<String>>>,
    }

    impl ProgressSink for Recorder {
        fn report(&self, event: ProgressEvent) {
            self.events.lock().unwrap().push(event);
            self.threads
                .lock()
                .unwrap()
                .push(std::thread::current().name().map(str::to_owned));
        }
    }

    /// Blocks the job on its first progress report until the test releases it.
    struct Gate {
        reached: Mutex<Sender<()>>,
        release: Mutex<Receiver<()>>,
    }

    impl ProgressSink for Gate {
        fn report(&self, _event: ProgressEvent) {
            let _ = self.reached.lock().unwrap().send(());
            let _ = self.release.lock().unwrap().recv_timeout(WAIT);
        }
    }

    fn gate() -> (Gate, Receiver<()>, Sender<()>) {
        let (reached_tx, reached_rx) = channel();
        let (release_tx, release_rx) = channel();
        (
            Gate {
                reached: Mutex::new(reached_tx),
                release: Mutex::new(release_rx),
            },
            reached_rx,
            release_tx,
        )
    }

    fn settings() -> PreviewSettings {
        PreviewSettings::new(5, 10, 128, image_to_binary_core::Sampling::Classic).unwrap()
    }

    fn bmp(width: u32, height: u32) -> Vec<u8> {
        let row = (width * 3).div_ceil(4) * 4;
        let data = row * height;
        let mut bytes = b"BM".to_vec();
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
            let padding = usize::try_from(row - width * 3).unwrap();
            bytes.resize(bytes.len() + padding, 0);
        }
        bytes
    }

    fn wait_for_outcome<T: Send + 'static>(
        worker: &Worker,
        run: impl FnOnce(&JobContext) -> Result<T, IpcError> + Send + 'static,
    ) -> (JobHandle, Receiver<Result<T, IpcError>>) {
        let (tx, rx) = channel();
        let handle = worker
            .submit(run, move |_, outcome| {
                let _ = tx.send(outcome);
            })
            .unwrap();
        (handle, rx)
    }

    fn import_bmp(files: &FileTable, slot: &SessionSlot, dir: &TestDir, width: u32, height: u32) {
        let path = dir.file("source.bmp", &bmp(width, height));
        let id = files.insert_source(path).unwrap();
        import(files, slot, id).unwrap();
    }

    fn names_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn import_reports_browser_metadata_and_holds_the_source() {
        let dir = TestDir::new("jobs-import");
        let files = FileTable::default();
        let slot = SessionSlot::default();
        let path = dir.file("photo.bmp", &bmp(3, 2));
        let id = files.insert_source(path.clone()).unwrap();
        let imported = import(&files, &slot, id).unwrap();
        assert_eq!(imported.name, "photo.bmp");
        assert_eq!((imported.width, imported.height), (3, 2));
        assert_eq!(
            imported.input_bytes,
            std::fs::metadata(&path).unwrap().len()
        );
        assert_eq!(imported.source, "file");
        assert!(slot.lock().is_some());
    }

    #[test]
    fn oversized_and_special_sources_fail_with_typed_codes() {
        let dir = TestDir::new("jobs-special");
        let files = FileTable::default();
        let slot = SessionSlot::default();

        let directory = files.insert_source(dir.path().to_path_buf()).unwrap();
        assert_eq!(
            import(&files, &slot, directory).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );

        let big = dir.path().join("huge.png");
        let handle = std::fs::File::create(&big).unwrap();
        handle.set_len(MAX_ENCODED_INPUT_BYTES + 1).unwrap();
        drop(handle);
        let huge = files.insert_source(big).unwrap();
        assert_eq!(
            import(&files, &slot, huge).unwrap_err().code(),
            ErrorCode::EncodedTooLarge
        );

        let fifo = dir.path().join("pipe.png");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if made {
            let pipe = files.insert_source(fifo).unwrap();
            assert_eq!(
                import(&files, &slot, pipe).unwrap_err().code(),
                ErrorCode::InvalidArgument
            );
        } else {
            eprintln!("skipping the FIFO case: mkfifo is unavailable");
        }
        assert!(slot.lock().is_none());
    }

    #[test]
    fn a_rejected_import_keeps_the_previous_source() {
        let dir = TestDir::new("jobs-keep");
        let files = FileTable::default();
        let slot = SessionSlot::default();
        import_bmp(&files, &slot, &dir, 2, 2);
        let junk = dir.file("junk.png", b"not an image at all");
        let id = files.insert_source(junk).unwrap();
        assert_eq!(
            import(&files, &slot, id).unwrap_err().code(),
            ErrorCode::UnsupportedFormat
        );
        let kept = slot.lock().as_ref().unwrap().source_metadata().unwrap();
        assert_eq!(kept.dimensions().width(), 2);
    }

    #[test]
    fn a_cancelled_preview_delivers_nothing_and_releases_the_session_job() {
        let dir = TestDir::new("jobs-cancel-preview");
        let files = Arc::new(FileTable::default());
        let slot = Arc::new(SessionSlot::default());
        import_bmp(&files, &slot, &dir, 40, 30);
        let worker = Worker::spawn().unwrap();
        let (sink, reached, release) = gate();
        let job_slot = Arc::clone(&slot);
        let request = PreviewRequest {
            generation: 1,
            mode: PreviewMode::Artwork(settings()),
        };
        let (handle, rx) = wait_for_outcome(&worker, move |context| {
            render_preview(context, &job_slot, &sink, request)
        });
        reached.recv_timeout(WAIT).unwrap();
        handle.cancel();
        release.send(()).unwrap();
        assert!(
            rx.recv_timeout(WAIT).is_err(),
            "a cancelled preview was delivered"
        );
        let guard = slot.lock();
        assert_eq!(guard.as_ref().unwrap().stage().unwrap(), Stage::Failed);
    }

    #[test]
    fn a_superseded_preview_delivers_nothing_and_a_newer_one_completes() {
        let dir = TestDir::new("jobs-supersede-preview");
        let files = Arc::new(FileTable::default());
        let slot = Arc::new(SessionSlot::default());
        import_bmp(&files, &slot, &dir, 40, 30);
        let worker = Worker::spawn().unwrap();
        let (sink, reached, release) = gate();
        let old_slot = Arc::clone(&slot);
        let old = PreviewRequest {
            generation: 1,
            mode: PreviewMode::Artwork(settings()),
        };
        let (_handle, old_rx) = wait_for_outcome(&worker, move |context| {
            render_preview(context, &old_slot, &sink, old)
        });
        reached.recv_timeout(WAIT).unwrap();
        let new_slot = Arc::clone(&slot);
        let newer = PreviewRequest {
            generation: 2,
            mode: PreviewMode::Original,
        };
        let (_newer_handle, new_rx) = wait_for_outcome(&worker, move |context| {
            render_preview(context, &new_slot, &Recorder::default(), newer)
        });
        release.send(()).unwrap();
        assert!(
            old_rx.recv_timeout(WAIT).is_err(),
            "a superseded preview was delivered"
        );
        let frame = new_rx.recv_timeout(WAIT).unwrap().unwrap();
        assert_eq!(
            super::super::frame::decode_frame(&frame)
                .unwrap()
                .generation,
            2
        );
    }

    #[test]
    fn an_export_matches_the_session_bytes_and_reports_its_length() {
        let dir = TestDir::new("jobs-export-ok");
        let files = Arc::new(FileTable::default());
        let slot = Arc::new(SessionSlot::default());
        import_bmp(&files, &slot, &dir, 41, 29);
        let destination = dir.path().join("out.png");
        let sink = files.insert_sink(destination.clone()).unwrap();
        let worker = Worker::spawn().unwrap();
        let recorder = Arc::new(Recorder::default());
        let job_recorder = Arc::clone(&recorder);
        let job_slot = Arc::clone(&slot);
        let job_files = Arc::clone(&files);
        let request = ExportRequest {
            generation: 3,
            settings: settings(),
            sink,
        };
        let (_handle, rx) = wait_for_outcome(&worker, move |context| {
            export_png(context, &job_slot, &job_files, &*job_recorder, request)
        });
        let report = rx.recv_timeout(WAIT).unwrap().unwrap();
        let written = std::fs::read(&destination).unwrap();
        assert_eq!(report.byte_length, u64::try_from(written.len()).unwrap());
        assert_eq!(&written[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(&written[written.len() - 8..written.len() - 4], b"IEND");
        assert_eq!(
            names_in(dir.path()).len(),
            2,
            "only the source and the export remain"
        );
        let events = recorder.events.lock().unwrap();
        assert!(events.iter().all(|event| event.generation == 3));
        assert_eq!(
            events.last().map(|event| event.stage),
            Some(ProgressStage::Ready)
        );
        let threads = recorder.threads.lock().unwrap();
        assert!(threads
            .iter()
            .all(|name| name.as_deref() == Some(WORKER_THREAD_NAME)));
    }

    #[test]
    fn a_cancelled_export_leaves_no_partial_file_and_keeps_an_existing_destination() {
        let dir = TestDir::new("jobs-cancel-export");
        let files = Arc::new(FileTable::default());
        let slot = Arc::new(SessionSlot::default());
        import_bmp(&files, &slot, &dir, 40, 30);
        let destination = dir.file("out.png", b"previous");
        let sink = files.insert_sink(destination.clone()).unwrap();
        let worker = Worker::spawn().unwrap();
        let (progress, reached, release) = gate();
        let job_slot = Arc::clone(&slot);
        let job_files = Arc::clone(&files);
        let request = ExportRequest {
            generation: 1,
            settings: settings(),
            sink,
        };
        let (handle, rx) = wait_for_outcome(&worker, move |context| {
            export_png(context, &job_slot, &job_files, &progress, request)
        });
        reached.recv_timeout(WAIT).unwrap();
        handle.cancel();
        release.send(()).unwrap();
        assert!(
            rx.recv_timeout(WAIT).is_err(),
            "a cancelled export was delivered"
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
        assert_eq!(
            names_in(dir.path()).len(),
            2,
            "no temporary file may remain"
        );
    }

    #[test]
    fn a_superseded_export_leaves_no_partial_file() {
        let dir = TestDir::new("jobs-supersede-export");
        let files = Arc::new(FileTable::default());
        let slot = Arc::new(SessionSlot::default());
        import_bmp(&files, &slot, &dir, 40, 30);
        let destination = dir.path().join("fresh.png");
        let sink = files.insert_sink(destination.clone()).unwrap();
        let worker = Worker::spawn().unwrap();
        let (progress, reached, release) = gate();
        let job_slot = Arc::clone(&slot);
        let job_files = Arc::clone(&files);
        let request = ExportRequest {
            generation: 1,
            settings: settings(),
            sink,
        };
        let (_handle, rx) = wait_for_outcome(&worker, move |context| {
            export_png(context, &job_slot, &job_files, &progress, request)
        });
        reached.recv_timeout(WAIT).unwrap();
        let (_newer, newer_rx) = wait_for_outcome(&worker, |_| Ok::<_, IpcError>(()));
        release.send(()).unwrap();
        assert!(
            rx.recv_timeout(WAIT).is_err(),
            "a superseded export was delivered"
        );
        newer_rx.recv_timeout(WAIT).unwrap().unwrap();
        assert!(!destination.exists());
        assert_eq!(names_in(dir.path()).len(), 1, "only the source may remain");
    }

    #[test]
    fn export_without_a_source_is_no_job_and_leaves_no_file() {
        let dir = TestDir::new("jobs-no-source");
        let files = FileTable::default();
        let slot = SessionSlot::default();
        let sink = files.insert_sink(dir.path().join("x.png")).unwrap();
        let request = ExportRequest {
            generation: 0,
            settings: settings(),
            sink,
        };
        let worker = Worker::spawn().unwrap();
        let files = Arc::new(files);
        let slot = Arc::new(slot);
        let (_handle, rx) = wait_for_outcome(&worker, move |context| {
            export_png(context, &slot, &files, &Recorder::default(), request)
        });
        let error = rx.recv_timeout(WAIT).unwrap().unwrap_err();
        assert_eq!(error.code(), ErrorCode::NoJob);
        assert_eq!(
            names_in(dir.path()).len(),
            0,
            "no sink may be created without a source"
        );
    }

    /// Blocks on the PNG trailer chunk, which only `finish_export` produces, so a test can cancel
    /// the job during the trailer writes.
    struct TrailerGate {
        inner: AtomicFileSink,
        reached: Sender<()>,
        release: Receiver<()>,
    }

    impl ChunkSink for TrailerGate {
        fn write_chunk(&mut self, chunk: &[u8]) -> io::Result<()> {
            if chunk.windows(4).any(|window| window == b"IEND") {
                let _ = self.reached.send(());
                let _ = self.release.recv_timeout(WAIT);
            }
            ChunkSink::write_chunk(&mut self.inner, chunk)
        }

        fn commit(self) -> io::Result<()> {
            <AtomicFileSink as ChunkSink>::commit(self.inner)
        }
    }

    #[test]
    fn a_cancel_during_the_trailer_writes_reports_nothing_and_leaves_no_file() {
        let dir = TestDir::new("jobs-trailer");
        let files = Arc::new(FileTable::default());
        let slot = Arc::new(SessionSlot::default());
        import_bmp(&files, &slot, &dir, 40, 30);
        let destination = dir.path().join("out.png");
        let sink_id = files.insert_sink(destination.clone()).unwrap();
        let (reached, reached_rx) = channel();
        let (release, release_rx) = channel();
        let sink = TrailerGate {
            inner: AtomicFileSink::create(&destination).unwrap(),
            reached,
            release: release_rx,
        };
        let worker = Worker::spawn().unwrap();
        let job_slot = Arc::clone(&slot);
        let request = ExportRequest {
            generation: 1,
            settings: settings(),
            sink: sink_id,
        };
        let (handle, rx) = wait_for_outcome(&worker, move |context| {
            export_into(context, &job_slot, &Recorder::default(), request, sink)
        });
        reached_rx.recv_timeout(WAIT).unwrap();
        handle.cancel();
        release.send(()).unwrap();
        assert!(
            rx.recv_timeout(WAIT).is_err(),
            "a cancelled export reported an outcome after its trailer"
        );
        assert!(
            !destination.exists(),
            "a cancelled export committed its file"
        );
        assert_eq!(names_in(dir.path()).len(), 1, "only the source may remain");
    }
}
