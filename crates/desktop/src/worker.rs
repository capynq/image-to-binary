//! Owned background worker with one active job and one newest pending job.
//!
//! Jobs run on a dedicated thread, never on a UI thread. Submitting a job cancels the
//! active job and replaces the pending one. Suppression of superseded results is best
//! effort: a result can still be delivered after a newer submission has raced with it, so
//! the consumer is authoritative and must compare each delivered generation with the newest
//! generation it submitted, discarding older ones. The module has no Tauri types and is
//! testable without a window.

use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{
    channel, sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError,
};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle, ThreadId};
use std::time::Duration;

use image_to_binary_core::{Cancellation, ContractError, Generation};

/// Name given to the worker thread so diagnostics and smoke reports can identify it.
pub const WORKER_THREAD_NAME: &str = "image-to-binary-worker";
/// Capacity of the wake-up channel. Wake tokens coalesce, so a small bound is sufficient.
pub const COMMAND_CAPACITY: usize = 2;
/// Longest time shutdown waits for the worker thread before detaching it.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Failure to run or accept work on the worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerError {
    /// Shutdown was requested or the worker thread has exited.
    Stopped,
    /// The operating system refused to create the worker thread.
    Spawn,
    /// The generation counter cannot advance without overflow.
    GenerationExhausted,
    /// A job panicked; the worker remains usable for later jobs.
    JobPanicked,
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Stopped => "the processing worker has stopped",
            Self::Spawn => "the processing worker thread could not be started",
            Self::GenerationExhausted => "the processing generation counter is exhausted",
            Self::JobPanicked => "the processing job panicked",
        })
    }
}

impl std::error::Error for WorkerError {}

/// Outcome of [`Worker::shutdown`], so callers and tests can observe how the thread ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShutdownOutcome {
    /// The worker thread finished within the grace period and was joined.
    Joined,
    /// The grace period expired while a job was still running. The thread was detached and
    /// exits once that job returns or checkpoints.
    TimedOut,
    /// Shutdown was requested from the worker thread, which cannot join itself. The thread
    /// exits once the current job returns.
    OnWorkerThread,
    /// An earlier shutdown already took ownership of the thread, so this call did not wait.
    AlreadyShutDown,
}

/// Context passed to a running job so it can observe supersession and cancellation.
///
/// Long-running jobs must call [`JobContext::checkpoint`] between chunks; a job that never
/// checkpoints is detached when shutdown's grace period expires.
#[derive(Debug)]
pub struct JobContext {
    generation: Generation,
    cancellation: Cancellation,
    latest: Arc<AtomicU64>,
}

impl JobContext {
    /// Returns the generation this job was submitted under.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Returns an error when this job was superseded or cancelled and must stop.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::StaleGeneration`] when a newer job was submitted, or
    /// [`ContractError::Cancelled`] when this job was cancelled or the worker shut down.
    pub fn checkpoint(&self) -> Result<(), ContractError> {
        if self.is_superseded() {
            return Err(ContractError::StaleGeneration);
        }
        self.cancellation.check(self.generation)
    }

    fn is_superseded(&self) -> bool {
        self.latest.load(Ordering::Acquire) != self.generation.value()
    }

    fn is_current(&self) -> bool {
        !self.is_superseded() && !self.cancellation.is_cancelled()
    }
}

/// Handle to one submitted job, used to request its cancellation.
#[derive(Clone, Debug)]
pub struct JobHandle {
    generation: Generation,
    shared: Arc<Shared>,
}

impl JobHandle {
    /// Returns the generation assigned at submission.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Cancels the job if it is still pending or active; a finished job is unaffected.
    pub fn cancel(&self) {
        let mut slots = lock(&self.shared);
        if slots
            .pending
            .as_ref()
            .is_some_and(|p| p.generation == self.generation)
        {
            slots.pending = None;
        }
        if let Some(active) = slots
            .active
            .as_ref()
            .filter(|active| active.generation == self.generation)
        {
            active.cancellation.cancel();
        }
    }
}

type Job = Box<dyn FnOnce(&JobContext) + Send>;

struct Pending {
    generation: Generation,
    job: Job,
}

struct Active {
    generation: Generation,
    cancellation: Cancellation,
}

#[derive(Default)]
struct Slots {
    next_generation: u64,
    shutdown: bool,
    active: Option<Active>,
    pending: Option<Pending>,
}

struct Shared {
    slots: Mutex<Slots>,
    latest: Arc<AtomicU64>,
}

#[derive(Debug)]
struct WorkerThread {
    id: ThreadId,
    handle: JoinHandle<()>,
    /// Disconnects when the thread drops its sender on exit, so shutdown can wait with a timeout.
    finished: Receiver<()>,
}

/// Owns one background thread that runs the newest submitted job.
///
/// Invariants: at most one job is active and at most one is pending; a pending job is
/// replaced, not queued, by a newer submission. Delivery is best effort: a job's deliver
/// closure runs only if the job still looked current when it finished, so consumers must
/// compare generations. Dropping the worker runs [`Worker::shutdown`], which waits at most a
/// fixed grace period, so a job that never checkpoints cannot hang the process.
#[derive(Debug)]
pub struct Worker {
    shared: Arc<Shared>,
    wake: SyncSender<()>,
    thread: Mutex<Option<WorkerThread>>,
}

impl fmt::Debug for Shared {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Shared")
    }
}

impl Worker {
    /// Starts the worker thread.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::Spawn`] when the thread cannot be created.
    pub fn spawn() -> Result<Self, WorkerError> {
        let (wake, wakeups) = sync_channel(COMMAND_CAPACITY);
        let shared = Arc::new(Shared {
            slots: Mutex::new(Slots::default()),
            latest: Arc::new(AtomicU64::new(0)),
        });
        let thread_shared = Arc::clone(&shared);
        let (finished_tx, finished) = channel();
        let handle = thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_owned())
            .spawn(move || {
                run_loop(&thread_shared, &wakeups);
                drop(finished_tx);
            })
            .map_err(|_| WorkerError::Spawn)?;
        let id = handle.thread().id();
        Ok(Self {
            shared,
            wake,
            thread: Mutex::new(Some(WorkerThread {
                id,
                handle,
                finished,
            })),
        })
    }

    /// Submits a job, superseding the active job and replacing any pending job.
    ///
    /// `run` executes on the worker thread. `deliver` receives the job's own generation and
    /// its result, and is called only if the job still looked current after `run` finished.
    /// That check is best effort: a newer submission can land after it, so a superseded
    /// result can still be delivered, possibly after the consumer has moved on. The consumer
    /// is authoritative and MUST compare the delivered generation with the newest generation
    /// it submitted, discarding any older one.
    ///
    /// `deliver` runs on the worker thread, so it must not call main-thread APIs synchronously
    /// or block on them; doing so stalls the worker and every later job. A panic in `run` is
    /// delivered as `E::from(WorkerError::JobPanicked)` when the job still looks current.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::Stopped`] after shutdown and
    /// [`WorkerError::GenerationExhausted`] when the generation counter is exhausted.
    pub fn submit<T, E, Run, Deliver>(
        &self,
        run: Run,
        deliver: Deliver,
    ) -> Result<JobHandle, WorkerError>
    where
        T: Send + 'static,
        E: Send + From<WorkerError> + 'static,
        Run: FnOnce(&JobContext) -> Result<T, E> + Send + 'static,
        Deliver: FnOnce(Generation, Result<T, E>) + Send + 'static,
    {
        let job: Job = Box::new(move |context: &JobContext| {
            let outcome = catch_unwind(AssertUnwindSafe(|| run(context)))
                .unwrap_or_else(|_| Err(E::from(WorkerError::JobPanicked)));
            if context.is_current() {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    deliver(context.generation(), outcome);
                }));
            }
        });

        let generation = {
            let mut slots = lock(&self.shared);
            if slots.shutdown {
                return Err(WorkerError::Stopped);
            }
            let next = slots
                .next_generation
                .checked_add(1)
                .ok_or(WorkerError::GenerationExhausted)?;
            slots.next_generation = next;
            let generation = Generation::new(next);
            self.shared.latest.store(next, Ordering::Release);
            if let Some(active) = slots.active.as_ref() {
                active.cancellation.cancel();
            }
            slots.pending = Some(Pending { generation, job });
            generation
        };
        self.wake()?;
        Ok(JobHandle {
            generation,
            shared: Arc::clone(&self.shared),
        })
    }

    /// Cancels all work, drops the pending job, and waits at most a fixed grace period.
    ///
    /// A job that never checkpoints cannot be interrupted, so when the grace period expires
    /// the thread is detached rather than joined; it exits once that job returns. Idempotent:
    /// later calls return [`ShutdownOutcome::AlreadyShutDown`]. After shutdown, `submit` fails.
    #[must_use]
    pub fn shutdown(&self) -> ShutdownOutcome {
        self.shutdown_within(SHUTDOWN_GRACE)
    }

    fn shutdown_within(&self, grace: Duration) -> ShutdownOutcome {
        {
            let mut slots = lock(&self.shared);
            slots.shutdown = true;
            slots.pending = None;
            if let Some(active) = slots.active.as_ref() {
                active.cancellation.cancel();
            }
        }
        let _ = self.wake.try_send(());
        let Some(owned) = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return ShutdownOutcome::AlreadyShutDown;
        };
        if thread::current().id() == owned.id {
            return ShutdownOutcome::OnWorkerThread;
        }
        match owned.finished.recv_timeout(grace) {
            Err(RecvTimeoutError::Timeout) => ShutdownOutcome::TimedOut,
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                let _ = owned.handle.join();
                ShutdownOutcome::Joined
            }
        }
    }

    fn wake(&self) -> Result<(), WorkerError> {
        match self.wake.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Disconnected(())) => Err(WorkerError::Stopped),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn run_loop(shared: &Shared, wakeups: &Receiver<()>) {
    while wakeups.recv().is_ok() {
        loop {
            let Some((generation, job, cancellation)) = take_next(shared) else {
                break;
            };
            let context = JobContext {
                generation,
                cancellation,
                latest: Arc::clone(&shared.latest),
            };
            job(&context);
            lock(shared).active = None;
        }
        if lock(shared).shutdown {
            return;
        }
    }
}

fn take_next(shared: &Shared) -> Option<(Generation, Job, Cancellation)> {
    let mut slots = lock(shared);
    if slots.shutdown {
        return None;
    }
    let pending = slots.pending.take()?;
    let cancellation = Cancellation::new(pending.generation);
    slots.active = Some(Active {
        generation: pending.generation,
        cancellation: cancellation.clone(),
    });
    Some((pending.generation, pending.job, cancellation))
}

fn lock(shared: &Shared) -> MutexGuard<'_, Slots> {
    shared.slots.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::{channel, Sender};

    const WAIT: Duration = Duration::from_secs(10);

    fn gate() -> (Sender<()>, Receiver<()>) {
        channel()
    }

    #[test]
    fn jobs_run_on_the_named_worker_thread_not_the_caller() {
        let worker = Worker::spawn().unwrap();
        let (tx, rx) = channel();
        worker
            .submit(
                |_| Ok::<_, WorkerError>(thread::current().name().map(str::to_owned)),
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        let name = rx.recv_timeout(WAIT).unwrap().unwrap();
        assert_eq!(name.as_deref(), Some(WORKER_THREAD_NAME));
        assert_ne!(thread::current().name(), Some(WORKER_THREAD_NAME));
    }

    #[test]
    fn replacement_keeps_only_the_newest_pending_job() {
        let worker = Worker::spawn().unwrap();
        let (open_gate, wait_gate) = gate();
        let (started_tx, started_rx) = channel();
        let (tx, rx) = channel();
        let ran = Arc::new(AtomicUsize::new(0));

        let active_ran = Arc::clone(&ran);
        let active_tx = tx.clone();
        worker
            .submit(
                move |_| {
                    active_ran.fetch_add(1, Ordering::SeqCst);
                    let _ = started_tx.send(());
                    wait_gate.recv().unwrap();
                    Ok::<_, WorkerError>("A")
                },
                move |_, result| {
                    let _ = active_tx.send(result);
                },
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();

        let pending_ran = Arc::new(AtomicUsize::new(0));
        let replaced_ran = Arc::clone(&pending_ran);
        let replaced_tx = tx.clone();
        worker
            .submit(
                move |_| {
                    replaced_ran.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, WorkerError>("B")
                },
                move |_, result| {
                    let _ = replaced_tx.send(result);
                },
            )
            .unwrap();

        let newest_ran = Arc::new(AtomicUsize::new(0));
        let newest_counter = Arc::clone(&newest_ran);
        let newest = worker
            .submit(
                move |_| {
                    newest_counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, WorkerError>("C")
                },
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();

        open_gate.send(()).unwrap();
        assert_eq!(rx.recv_timeout(WAIT).unwrap().unwrap(), "C");
        assert_eq!(newest.generation().value(), 3);
        assert_eq!(ran.load(Ordering::SeqCst), 1);
        assert_eq!(pending_ran.load(Ordering::SeqCst), 0);
        assert_eq!(newest_ran.load(Ordering::SeqCst), 1);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn cancelling_a_pending_job_prevents_it_from_running() {
        let worker = Worker::spawn().unwrap();
        let (open_gate, wait_gate) = gate();
        let (started_tx, started_rx) = channel();
        let (tx, rx) = channel();
        worker
            .submit(
                move |_| {
                    let _ = started_tx.send(());
                    wait_gate.recv().unwrap();
                    Ok::<_, WorkerError>(())
                },
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();

        let pending_ran = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&pending_ran);
        let pending = worker
            .submit(
                move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, WorkerError>(())
                },
                |_, _| panic!("cancelled pending job must not deliver"),
            )
            .unwrap();
        pending.cancel();

        open_gate.send(()).unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        let _ = worker.shutdown();
        assert_eq!(pending_ran.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn cancelling_an_active_job_discards_its_result() {
        let worker = Worker::spawn().unwrap();
        let (open_gate, wait_gate) = gate();
        let (started_tx, started_rx) = channel();
        let (checkpoint_tx, checkpoint_rx) = channel();
        let (tx, rx) = channel();
        let handle = worker
            .submit(
                move |context| {
                    let _ = started_tx.send(());
                    wait_gate.recv().unwrap();
                    let _ = checkpoint_tx.send(context.checkpoint());
                    Ok::<_, WorkerError>("late")
                },
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        handle.cancel();
        open_gate.send(()).unwrap();

        assert_eq!(
            checkpoint_rx.recv_timeout(WAIT).unwrap(),
            Err(ContractError::Cancelled)
        );
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn superseded_job_observes_stale_generation_and_delivers_nothing() {
        let worker = Worker::spawn().unwrap();
        let (open_gate, wait_gate) = gate();
        let (started_tx, started_rx) = channel();
        let (checkpoint_tx, checkpoint_rx) = channel();
        let (tx, rx) = channel();
        worker
            .submit(
                move |context| {
                    let _ = started_tx.send(());
                    wait_gate.recv().unwrap();
                    let _ = checkpoint_tx.send(context.checkpoint());
                    Ok::<_, WorkerError>("old")
                },
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();

        let (new_tx, new_rx) = channel();
        worker
            .submit(
                |_| Ok::<_, WorkerError>("new"),
                move |_, result| {
                    let _ = new_tx.send(result);
                },
            )
            .unwrap();
        open_gate.send(()).unwrap();

        assert_eq!(
            checkpoint_rx.recv_timeout(WAIT).unwrap(),
            Err(ContractError::StaleGeneration)
        );
        assert_eq!(new_rx.recv_timeout(WAIT).unwrap().unwrap(), "new");
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn a_result_superseded_mid_delivery_carries_its_own_older_generation() {
        let worker = Worker::spawn().unwrap();
        let (in_deliver_tx, in_deliver_rx) = channel();
        let (release_tx, release_rx) = channel::<()>();
        let (delivered_tx, delivered_rx) = channel();
        let older = worker
            .submit(
                |_| Ok::<_, WorkerError>("A"),
                move |generation, result| {
                    let _ = in_deliver_tx.send(());
                    let _ = release_rx.recv();
                    let _ = delivered_tx.send((generation, result));
                },
            )
            .unwrap();
        in_deliver_rx.recv_timeout(WAIT).unwrap();

        let newer = worker
            .submit(|_| Ok::<_, WorkerError>("B"), |_, _| {})
            .unwrap();
        release_tx.send(()).unwrap();

        let (delivered, result) = delivered_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(result, Ok("A"));
        assert_eq!(delivered, older.generation());
        assert!(
            delivered < newer.generation(),
            "a consumer comparing against its newest submission must reject this result"
        );
    }

    #[test]
    fn shutdown_while_busy_cancels_joins_and_delivers_nothing() {
        let worker = Worker::spawn().unwrap();
        let (started_tx, started_rx) = channel();
        let (checkpoint_tx, checkpoint_rx) = channel();
        let (tx, rx) = channel();
        let pending_ran = Arc::new(AtomicUsize::new(0));
        worker
            .submit(
                move |context| {
                    let _ = started_tx.send(());
                    loop {
                        if let Err(error) = context.checkpoint() {
                            let _ = checkpoint_tx.send(error);
                            return Err(WorkerError::Stopped);
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                },
                move |_, result: Result<(), WorkerError>| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        let counter = Arc::clone(&pending_ran);
        worker
            .submit(
                move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, WorkerError>(())
                },
                |_, _| panic!("pending job must not deliver after shutdown"),
            )
            .unwrap();

        let (done_tx, done_rx) = channel();
        let joiner = thread::spawn(move || {
            let outcome = worker.shutdown();
            let _ = done_tx.send(outcome);
            worker
        });
        let outcome = done_rx.recv_timeout(WAIT).expect("shutdown must not hang");
        assert_eq!(outcome, ShutdownOutcome::Joined);
        let worker = joiner.join().unwrap();

        assert_eq!(
            checkpoint_rx.recv_timeout(WAIT).unwrap(),
            ContractError::StaleGeneration,
            "submitting the pending job superseded the active one before shutdown"
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "a superseded active job must not deliver"
        );
        assert_eq!(pending_ran.load(Ordering::SeqCst), 0);
        assert_eq!(
            worker
                .submit(|_| Ok::<_, WorkerError>(()), |_, _| {})
                .unwrap_err(),
            WorkerError::Stopped
        );
    }

    #[test]
    fn shutdown_cancels_an_active_job_without_a_pending_one() {
        let worker = Worker::spawn().unwrap();
        let (started_tx, started_rx) = channel();
        let (checkpoint_tx, checkpoint_rx) = channel();
        let (tx, rx) = channel();
        worker
            .submit(
                move |context| {
                    let _ = started_tx.send(());
                    loop {
                        if let Err(error) = context.checkpoint() {
                            let _ = checkpoint_tx.send(error);
                            return Err(WorkerError::Stopped);
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                },
                move |_, result: Result<(), WorkerError>| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(worker.shutdown(), ShutdownOutcome::Joined);
        assert_eq!(
            checkpoint_rx.recv_timeout(WAIT).unwrap(),
            ContractError::Cancelled
        );
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        assert_eq!(worker.shutdown(), ShutdownOutcome::AlreadyShutDown);
    }

    #[test]
    fn shutdown_reports_timeout_when_a_job_never_checkpoints() {
        let worker = Worker::spawn().unwrap();
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = channel::<()>();
        let (finished_tx, finished_rx) = channel();
        worker
            .submit(
                move |_| {
                    let _ = started_tx.send(());
                    let _ = release_rx.recv();
                    let _ = finished_tx.send(());
                    Ok::<_, WorkerError>(())
                },
                |_, _| {},
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();

        let (done_tx, done_rx) = channel();
        let shutdown = thread::spawn(move || {
            let outcome = worker.shutdown_within(Duration::from_millis(50));
            let _ = done_tx.send(outcome);
        });
        let outcome = done_rx
            .recv_timeout(WAIT)
            .expect("shutdown must return within the test bound");
        assert_eq!(outcome, ShutdownOutcome::TimedOut);
        shutdown.join().unwrap();

        release_tx.send(()).unwrap();
        finished_rx.recv_timeout(WAIT).unwrap();
    }

    #[test]
    fn shutdown_joins_a_checkpointing_job_and_an_idle_worker() {
        let worker = Worker::spawn().unwrap();
        let (started_tx, started_rx) = channel();
        worker
            .submit(
                move |context| {
                    let _ = started_tx.send(());
                    while context.checkpoint().is_ok() {
                        thread::yield_now();
                    }
                    Ok::<_, WorkerError>(())
                },
                |_, _| {},
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(worker.shutdown_within(WAIT), ShutdownOutcome::Joined);

        let idle = Worker::spawn().unwrap();
        assert_eq!(idle.shutdown_within(WAIT), ShutdownOutcome::Joined);
    }

    #[test]
    fn dropping_the_worker_joins_its_thread() {
        let worker = Worker::spawn().unwrap();
        let (tx, rx) = channel();
        worker
            .submit(
                |_| Ok::<_, WorkerError>(()),
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        rx.recv_timeout(WAIT).unwrap().unwrap();
        drop(worker);
    }

    #[test]
    fn only_the_newest_generation_is_delivered_under_burst_submission() {
        let worker = Worker::spawn().unwrap();
        let (open_gate, wait_gate) = gate();
        let (started_tx, started_rx) = channel();
        let (tx, rx) = channel();
        let first_tx = tx.clone();
        worker
            .submit(
                move |_| {
                    let _ = started_tx.send(());
                    wait_gate.recv().unwrap();
                    Ok::<_, WorkerError>(1_u64)
                },
                move |_, result| {
                    let _ = first_tx.send(result);
                },
            )
            .unwrap();
        started_rx.recv_timeout(WAIT).unwrap();

        let mut last = None;
        for value in 2..=50_u64 {
            let sender = tx.clone();
            last = Some(
                worker
                    .submit(
                        move |_| Ok::<_, WorkerError>(value),
                        move |_, result| {
                            let _ = sender.send(result);
                        },
                    )
                    .unwrap(),
            );
        }
        open_gate.send(()).unwrap();

        assert_eq!(rx.recv_timeout(WAIT).unwrap().unwrap(), 50);
        assert_eq!(last.unwrap().generation().value(), 50);
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn a_panicking_job_reports_an_error_and_the_worker_keeps_running() {
        let worker = Worker::spawn().unwrap();
        let (tx, rx) = channel();
        let first_tx = tx.clone();
        worker
            .submit(
                |_| -> Result<&'static str, WorkerError> { panic!("intentional test panic") },
                move |_, result| {
                    let _ = first_tx.send(result);
                },
            )
            .unwrap();
        assert_eq!(
            rx.recv_timeout(WAIT).unwrap(),
            Err(WorkerError::JobPanicked)
        );
        worker
            .submit(
                |_| Ok::<_, WorkerError>("after"),
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        assert_eq!(rx.recv_timeout(WAIT).unwrap().unwrap(), "after");
    }

    #[test]
    fn checkpoint_passes_for_the_current_uncancelled_job() {
        let worker = Worker::spawn().unwrap();
        let (tx, rx) = channel();
        worker
            .submit(
                |context| {
                    context.checkpoint().map_err(|_| WorkerError::Stopped)?;
                    Ok::<_, WorkerError>(context.generation().value())
                },
                move |_, result| {
                    let _ = tx.send(result);
                },
            )
            .unwrap();
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), Ok(1));
    }
}
