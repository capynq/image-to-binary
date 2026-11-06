//! Real-app smoke driver, compiled only with the `smoke` feature.
//!
//! `--smoke` boots the production window and webview, injects a probe that calls the real
//! IPC commands from the page, resizes the real window, and writes a JSON report. The run
//! exits non-zero on any failed check, and a watchdog exits the process if it stalls.

use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{App, AppHandle, LogicalSize, Manager, State, WebviewWindow};

use crate::error::{ErrorCode, IpcError};
use crate::ipc::app_info_value;
use crate::limits::app_limits;
use crate::window::{MINIMUM_HEIGHT_LOGICAL, MINIMUM_WIDTH_LOGICAL};
use crate::worker::{Worker, WorkerError, WORKER_THREAD_NAME};

/// Length of the known byte pattern sent through the binary echo. It is deliberately not a
/// power of two, so truncation or padding is visible.
pub const SMOKE_ECHO_LENGTH: usize = 48 * 1024 + 123;
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const HARD_TIMEOUT: Duration = Duration::from_secs(120);
const SETTLE: Duration = Duration::from_millis(700);
const WORKER_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_LOGICAL: (f64, f64) = (1280.0, 800.0);
const SMALL_REQUEST: (f64, f64) = (640.0, 480.0);
const LARGE_REQUEST: (f64, f64) = (1400.0, 900.0);
const SIZE_TOLERANCE: f64 = 1.0;
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Command-line options that enable the smoke run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Options {
    report_path: Option<PathBuf>,
}

impl Options {
    /// Parses `--smoke` and an optional `--smoke-report <path>` or `--smoke-report=<path>`.
    ///
    /// Returns `None` when `--smoke` is absent, so the normal app runs unchanged.
    pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Option<Self> {
        let mut enabled = false;
        let mut report_path = None;
        let mut iterator = args.into_iter();
        while let Some(argument) = iterator.next() {
            if argument == "--smoke" {
                enabled = true;
            } else if let Some(path) = argument.strip_prefix("--smoke-report=") {
                report_path = Some(PathBuf::from(path));
            } else if argument == "--smoke-report" {
                report_path = iterator.next().map(PathBuf::from);
            }
        }
        enabled.then_some(Self { report_path })
    }
}

/// Page-side measurements posted by the injected probe through the `smoke_page` command.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageProbe {
    phase: String,
    token: String,
    href: String,
    inner_width: f64,
    inner_height: f64,
    device_pixel_ratio: f64,
    csp_violations: u32,
    root_children: u32,
    app_info: Option<Value>,
    app_limits: Option<Value>,
    echo: Option<EchoPageReport>,
    worker: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EchoPageReport {
    bytes: usize,
    request_fnv1a64: String,
    response_fnv1a64: String,
    request_matches_pattern: bool,
    response_matches_pattern: bool,
}

/// Record of the bytes the native echo command received, kept for cross-checking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EchoReceipt {
    bytes: usize,
    fnv1a64: u64,
}

/// Managed smoke state shared by the probe commands and the echo command.
pub struct SmokeState {
    probes: Mutex<Sender<PageProbe>>,
    echo: Mutex<Option<EchoReceipt>>,
}

impl SmokeState {
    /// Records the length and hash of a request body the native side received.
    pub(crate) fn record_echo(&self, bytes: &[u8]) {
        let receipt = EchoReceipt {
            bytes: bytes.len(),
            fnv1a64: fnv1a64(bytes),
        };
        *self.echo.lock().unwrap_or_else(PoisonError::into_inner) = Some(receipt);
    }

    fn last_echo(&self) -> Option<EchoReceipt> {
        *self.echo.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Creates the smoke state and returns the receiver the driver waits on.
pub fn install(app: &App) -> Receiver<PageProbe> {
    let (sender, receiver) = mpsc::channel();
    app.manage(SmokeState {
        probes: Mutex::new(sender),
        echo: Mutex::new(None),
    });
    receiver
}

/// Returns the probe script with this build's constants substituted.
#[must_use]
pub fn init_script() -> String {
    include_str!("smoke/probe.js").replace("__ECHO_LENGTH__", &SMOKE_ECHO_LENGTH.to_string())
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn smoke_page(state: State<'_, SmokeState>, probe: PageProbe) -> Result<(), IpcError> {
    let sender = state.probes.lock().unwrap_or_else(PoisonError::into_inner);
    sender.send(probe).map_err(|_| {
        IpcError::new(
            ErrorCode::Internal,
            "the smoke driver is no longer listening",
        )
    })
}

/// Result of a job the worker ran on behalf of the page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerProbe {
    thread_name: String,
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn smoke_worker_probe(
    worker: State<'_, Arc<Worker>>,
) -> Result<WorkerProbe, IpcError> {
    let (sender, receiver) = mpsc::channel();
    let handle = worker
        .submit(
            |_| Ok::<_, WorkerError>(thread::current().name().unwrap_or_default().to_owned()),
            move |generation, result| {
                let _ = sender.send((generation, result));
            },
        )
        .map_err(|error| IpcError::new(ErrorCode::Internal, error.to_string()))?;
    let outcome =
        tauri::async_runtime::spawn_blocking(move || receiver.recv_timeout(WORKER_TIMEOUT))
            .await
            .map_err(|error| IpcError::new(ErrorCode::Internal, error.to_string()))?;
    match outcome {
        Ok((generation, Ok(thread_name))) if generation == handle.generation() => {
            Ok(WorkerProbe { thread_name })
        }
        Ok((_, Ok(_))) => Err(IpcError::new(
            ErrorCode::Internal,
            "the worker delivered a superseded smoke probe",
        )),
        Ok((_, Err(error))) => Err(IpcError::new(ErrorCode::Internal, error.to_string())),
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Err(IpcError::new(
            ErrorCode::Internal,
            "the worker did not answer the smoke probe",
        )),
    }
}

/// Starts the watchdog and the driver thread. The driver exits the app with its verdict.
pub fn start(app: AppHandle, options: Options, probes: Receiver<PageProbe>) {
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog_finished = Arc::clone(&finished);
    let watchdog_options = options.clone();
    let watchdog = thread::Builder::new().name("smoke-watchdog".to_owned());
    let _ = watchdog.spawn(move || {
        thread::sleep(HARD_TIMEOUT);
        if !watchdog_finished.swap(true, Ordering::SeqCst) {
            write_report(&watchdog_options, &timeout_report());
            process::exit(2);
        }
    });

    let driver = thread::Builder::new().name("smoke-driver".to_owned());
    let _ = driver.spawn(move || {
        let report = drive(&app, &probes);
        let passed = report.passed;
        if !finished.swap(true, Ordering::SeqCst) {
            write_report(&options, &report);
            app.exit(i32::from(!passed));
        }
    });
}

#[derive(Debug, Serialize)]
struct Size {
    width: f64,
    height: f64,
}

#[derive(Debug, Serialize)]
struct Check {
    name: &'static str,
    passed: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct Report {
    schema: &'static str,
    platform: &'static str,
    arch: &'static str,
    passed: bool,
    scale_factor: Option<f64>,
    window: WindowReport,
    page: PageReport,
    ipc: IpcReport,
    checks: Vec<Check>,
    failures: Vec<&'static str>,
}

/// Window sizes in logical CSS pixels, as measured from the native window.
#[derive(Debug, Default, Serialize)]
struct WindowReport {
    units: &'static str,
    default: Option<Size>,
    minimum: Option<Size>,
    startup: Option<Size>,
    title_bar_offset: Option<f64>,
    small_request: Option<Size>,
    small_actual: Option<Size>,
    large_request: Option<Size>,
    large_actual: Option<Size>,
}

#[derive(Debug, Default, Serialize)]
struct PageReport {
    token_stable: bool,
    device_pixel_ratio: Option<f64>,
    csp_violations: Option<u32>,
    root_children: Option<u32>,
    startup_inner: Option<Size>,
    resize_small_inner: Option<Size>,
    resize_large_inner: Option<Size>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct IpcReport {
    app_info: Option<Value>,
    app_limits_matches_native: bool,
    echo: Option<Value>,
    echo_expected_fnv1a64: String,
    echo_received_by_native_fnv1a64: Option<String>,
    worker_thread: Option<String>,
}

#[derive(Default)]
struct Recorder {
    checks: Vec<Check>,
}

impl Recorder {
    fn check(&mut self, name: &'static str, passed: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            name,
            passed,
            detail: detail.into(),
        });
    }
}

/// Facts captured at startup that later resize checks compare against.
struct Baseline {
    token: String,
    title_bar_offset: Option<f64>,
}

#[derive(Clone, Copy)]
enum Resize {
    Small,
    Large,
}

impl Resize {
    const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Large => "large",
        }
    }

    const fn request(self) -> (f64, f64) {
        match self {
            Self::Small => SMALL_REQUEST,
            Self::Large => LARGE_REQUEST,
        }
    }
}

/// Mutable state for one smoke run, consumed into a [`Report`] by [`Run::finish`].
#[derive(Default)]
struct Run {
    recorder: Recorder,
    window: WindowReport,
    page: PageReport,
    ipc: IpcReport,
    scale_factor: Option<f64>,
}

impl Run {
    fn drive(&mut self, app: &AppHandle, probes: &Receiver<PageProbe>) {
        self.window.units = "logical";
        self.window.default = Some(size(DEFAULT_LOGICAL));
        self.window.minimum = Some(size((MINIMUM_WIDTH_LOGICAL, MINIMUM_HEIGHT_LOGICAL)));
        self.window.small_request = Some(size(SMALL_REQUEST));
        self.window.large_request = Some(size(LARGE_REQUEST));
        self.ipc.echo_expected_fnv1a64 = hex(fnv1a64(&pattern(SMOKE_ECHO_LENGTH)));

        let Some(window) = app.get_webview_window("main") else {
            self.recorder
                .check("main_window_present", false, "no window labelled main");
            return;
        };
        self.recorder
            .check("main_window_present", true, "window labelled main exists");

        let Some(baseline) = self.startup(app, &window, probes) else {
            return;
        };
        let small_stable = self.resize(&window, probes, Resize::Small, &baseline);
        let large_stable = self.resize(&window, probes, Resize::Large, &baseline);
        self.page.token_stable = small_stable && large_stable;
    }

    fn startup(
        &mut self,
        app: &AppHandle,
        window: &WebviewWindow,
        probes: &Receiver<PageProbe>,
    ) -> Option<Baseline> {
        let startup = next_probe(probes, &mut self.recorder, "startup")?;
        self.page.root_children = Some(startup.root_children);
        self.page.csp_violations = Some(startup.csp_violations);
        self.page.device_pixel_ratio = Some(startup.device_pixel_ratio);
        self.page.startup_inner = Some(size((startup.inner_width, startup.inner_height)));
        self.recorder.check(
            "startup_probe_has_no_error",
            startup.error.is_none(),
            startup.error.clone().unwrap_or_else(|| "none".to_owned()),
        );
        self.recorder.check(
            "page_loaded_real_app",
            startup.href.starts_with("tauri:") || startup.href.starts_with("http"),
            format!("href {}", startup.href),
        );
        check_startup_ipc(&startup, app, &mut self.recorder, &mut self.ipc);

        self.scale_factor = window.scale_factor().ok();
        if let Some(scale) = self.scale_factor {
            self.recorder.check(
                "scale_factor_readable",
                scale > 0.0,
                format!("scale factor {scale}"),
            );
        }
        let native = logical_size(window, self.scale_factor);
        self.window.startup = native.map(size);
        let title_bar_offset = native.map(|(_, height)| height - startup.inner_height);
        self.window.title_bar_offset = title_bar_offset;
        Some(Baseline {
            token: startup.token,
            title_bar_offset,
        })
    }

    /// Resizes the real window, probes the page, and returns whether the page was not reloaded.
    fn resize(
        &mut self,
        window: &WebviewWindow,
        probes: &Receiver<PageProbe>,
        kind: Resize,
        baseline: &Baseline,
    ) -> bool {
        let Some(probe) = resize_and_probe(
            window,
            probes,
            kind.request(),
            &mut self.recorder,
            kind.label(),
        ) else {
            return false;
        };
        let actual = logical_size(window, self.scale_factor);
        match kind {
            Resize::Small => {
                self.window.small_actual = actual.map(size);
                self.recorder.check(
                    "resize_below_minimum_is_clamped",
                    actual.is_some_and(|(w, h)| {
                        w + SIZE_TOLERANCE >= MINIMUM_WIDTH_LOGICAL
                            && h + SIZE_TOLERANCE >= MINIMUM_HEIGHT_LOGICAL
                    }),
                    format!(
                        "requested {SMALL_REQUEST:?}, actual {actual:?}, minimum {:?}",
                        (MINIMUM_WIDTH_LOGICAL, MINIMUM_HEIGHT_LOGICAL)
                    ),
                );
            }
            Resize::Large => {
                self.window.large_actual = actual.map(size);
                self.recorder.check(
                    "resize_above_default_reaches_request",
                    actual.is_some_and(|(w, h)| {
                        (w - LARGE_REQUEST.0).abs() <= SIZE_TOLERANCE
                            && (h - LARGE_REQUEST.1).abs() <= SIZE_TOLERANCE
                    }),
                    format!("requested {LARGE_REQUEST:?}, actual {actual:?}"),
                );
            }
        }
        check_resize_probe(
            &probe,
            &baseline.token,
            (actual, baseline.title_bar_offset),
            kind,
            &mut self.recorder,
            &mut self.page,
        )
    }

    fn finish(self) -> Report {
        let failures: Vec<&'static str> = self
            .recorder
            .checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.name)
            .collect();
        let passed = failures.is_empty() && !self.recorder.checks.is_empty();
        Report {
            schema: "image-to-binary.desktop-smoke/1",
            platform: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            passed,
            scale_factor: self.scale_factor,
            window: self.window,
            page: self.page,
            ipc: self.ipc,
            checks: self.recorder.checks,
            failures,
        }
    }
}

fn drive(app: &AppHandle, probes: &Receiver<PageProbe>) -> Report {
    let mut run = Run::default();
    run.drive(app, probes);
    run.finish()
}

fn check_startup_ipc(
    startup: &PageProbe,
    app: &AppHandle,
    recorder: &mut Recorder,
    ipc: &mut IpcReport,
) {
    let native_info = serde_json::to_value(app_info_value()).ok();
    let page_info = startup.app_info.clone();
    recorder.check(
        "app_info_matches_native",
        page_info.is_some() && page_info == native_info,
        format!("page {page_info:?} native {native_info:?}"),
    );
    ipc.app_info = page_info;

    let native_limits = serde_json::to_value(app_limits()).ok();
    let limits_match = startup.app_limits.is_some() && startup.app_limits == native_limits;
    recorder.check(
        "app_limits_match_native",
        limits_match,
        "page app_limits compared with native app_limits",
    );
    ipc.app_limits_matches_native = limits_match;

    let expected = hex(fnv1a64(&pattern(SMOKE_ECHO_LENGTH)));
    let page_echo = startup.echo.as_ref();
    let page_echo_ok = page_echo.is_some_and(|e| {
        e.bytes == SMOKE_ECHO_LENGTH
            && e.request_matches_pattern
            && e.response_matches_pattern
            && e.request_fnv1a64 == expected
            && e.response_fnv1a64 == expected
    });
    recorder.check(
        "echo_round_trip_is_byte_exact_in_page",
        page_echo_ok,
        format!("expected {expected} over {SMOKE_ECHO_LENGTH} bytes"),
    );
    ipc.echo = page_echo.map(echo_report_value);

    let native_receipt = app.state::<SmokeState>().last_echo();
    let native_ok = native_receipt.is_some_and(|receipt| {
        receipt.bytes == SMOKE_ECHO_LENGTH && hex(receipt.fnv1a64) == expected
    });
    recorder.check(
        "echo_request_is_byte_exact_in_native",
        native_ok,
        format!("native receipt {native_receipt:?}"),
    );
    ipc.echo_received_by_native_fnv1a64 = native_receipt.map(|receipt| hex(receipt.fnv1a64));

    let worker_thread = startup
        .worker
        .as_ref()
        .and_then(|value| value.get("threadName"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    recorder.check(
        "worker_runs_off_ui_thread",
        worker_thread.as_deref() == Some(WORKER_THREAD_NAME),
        format!("worker thread {worker_thread:?}"),
    );
    ipc.worker_thread = worker_thread;

    recorder.check(
        "frontend_rendered_into_root",
        startup.root_children > 0,
        format!("root children {}", startup.root_children),
    );
    recorder.check(
        "no_csp_violations_at_startup",
        startup.csp_violations == 0,
        format!("violations {}", startup.csp_violations),
    );
}

fn echo_report_value(echo: &EchoPageReport) -> Value {
    serde_json::json!({
        "bytes": echo.bytes,
        "requestFnv1a64": echo.request_fnv1a64,
        "responseFnv1a64": echo.response_fnv1a64,
        "requestMatchesPattern": echo.request_matches_pattern,
        "responseMatchesPattern": echo.response_matches_pattern,
    })
}

fn resize_and_probe(
    window: &WebviewWindow,
    probes: &Receiver<PageProbe>,
    request: (f64, f64),
    recorder: &mut Recorder,
    label: &'static str,
) -> Option<PageProbe> {
    if let Err(error) = window.set_size(LogicalSize::new(request.0, request.1)) {
        recorder.check("resize_request_accepted", false, error.to_string());
        return None;
    }
    thread::sleep(SETTLE);
    let trigger =
        "window.__imageToBinarySmokeAfterResize && window.__imageToBinarySmokeAfterResize()";
    if let Err(error) = window.eval(trigger) {
        recorder.check("resize_probe_triggered", false, error.to_string());
        return None;
    }
    next_probe(probes, recorder, label)
}

/// Records page-side checks for one resize and returns whether the page token was unchanged.
///
/// The webview sits below the native title bar, so its height equals the native content
/// height minus a constant offset measured at startup.
fn check_resize_probe(
    probe: &PageProbe,
    startup_token: &str,
    (actual, title_bar_offset): (Option<(f64, f64)>, Option<f64>),
    kind: Resize,
    recorder: &mut Recorder,
    page: &mut PageReport,
) -> bool {
    let stable = probe.token == startup_token;
    let inner = (probe.inner_width, probe.inner_height);
    let inner_matches = actual
        .zip(title_bar_offset)
        .is_some_and(|((w, h), offset)| {
            (inner.0 - w).abs() <= SIZE_TOLERANCE
                && ((h - inner.1) - offset).abs() <= SIZE_TOLERANCE
        });
    let names = ResizeCheckNames::for_kind(kind);
    recorder.check(
        names.stable,
        stable,
        format!("startup token {startup_token}, probe token {}", probe.token),
    );
    recorder.check(
        names.inner,
        inner_matches,
        format!(
            "page inner {inner:?}, native actual {actual:?}, title bar offset {title_bar_offset:?}"
        ),
    );
    recorder.check(
        names.rendered,
        probe.root_children > 0,
        format!("root children {}", probe.root_children),
    );
    recorder.check(
        names.csp,
        probe.csp_violations == 0,
        format!("violations {}", probe.csp_violations),
    );
    match kind {
        Resize::Small => page.resize_small_inner = Some(size(inner)),
        Resize::Large => page.resize_large_inner = Some(size(inner)),
    }
    page.csp_violations = Some(probe.csp_violations);
    page.root_children = Some(probe.root_children);
    stable
}

/// Static check names for one resize, so reports carry stable identifiers.
struct ResizeCheckNames {
    stable: &'static str,
    inner: &'static str,
    rendered: &'static str,
    csp: &'static str,
}

impl ResizeCheckNames {
    const fn for_kind(kind: Resize) -> Self {
        match kind {
            Resize::Small => Self {
                stable: "page_not_reloaded_after_small_resize",
                inner: "page_inner_matches_native_small",
                rendered: "frontend_rendered_after_small_resize",
                csp: "no_csp_violations_after_small_resize",
            },
            Resize::Large => Self {
                stable: "page_not_reloaded_after_large_resize",
                inner: "page_inner_matches_native_large",
                rendered: "frontend_rendered_after_large_resize",
                csp: "no_csp_violations_after_large_resize",
            },
        }
    }
}

fn next_probe(
    probes: &Receiver<PageProbe>,
    recorder: &mut Recorder,
    label: &'static str,
) -> Option<PageProbe> {
    let name: &'static str = match label {
        "startup" => "startup_probe_received",
        "small" => "small_resize_probe_received",
        _ => "large_resize_probe_received",
    };
    match probes.recv_timeout(PROBE_TIMEOUT) {
        Ok(probe) if probe.phase == label || (label != "startup" && probe.phase == "resize") => {
            recorder.check(name, true, format!("phase {}", probe.phase));
            Some(probe)
        }
        Ok(probe) => {
            recorder.check(name, false, format!("unexpected phase {}", probe.phase));
            None
        }
        Err(error) => {
            recorder.check(name, false, format!("no probe within timeout: {error}"));
            None
        }
    }
}

fn logical_size(window: &WebviewWindow, scale_factor: Option<f64>) -> Option<(f64, f64)> {
    let scale = scale_factor.filter(|scale| *scale > 0.0)?;
    let physical = window.inner_size().ok()?;
    Some((
        f64::from(physical.width) / scale,
        f64::from(physical.height) / scale,
    ))
}

fn size(value: (f64, f64)) -> Size {
    Size {
        width: value.0,
        height: value.1,
    }
}

fn timeout_report() -> Report {
    let mut run = Run::default();
    run.recorder.check(
        "completed_within_hard_timeout",
        false,
        format!("smoke run exceeded {} seconds", HARD_TIMEOUT.as_secs()),
    );
    run.finish()
}

fn write_report(options: &Options, report: &Report) {
    let text = serde_json::to_string_pretty(report)
        .unwrap_or_else(|error| format!("{{\"serializationError\":\"{error}\"}}"));
    match &options.report_path {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, format!("{text}\n"));
        }
        None => println!("{text}"),
    }
}

fn pattern(length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| {
            let value = (u64::try_from(index).unwrap_or(0) * 37 + 11) % 256;
            u8::try_from(value).unwrap_or(0)
        })
        .collect()
}

/// FNV-1a over bytes. Wrapping multiplication is the hash definition, not an overflow defect.
fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

fn hex(value: u64) -> String {
    format!("{value:016x}")
}
