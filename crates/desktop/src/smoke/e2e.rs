//! End-to-end phases of the smoke run. The real studio UI, running in the webview, drives import,
//! preview, export, cancel and reset through its own buttons. The driver supplies only what a
//! person would supply through the OS: dialog answers, which the smoke build queues as fixture
//! paths, and window drag events. Exported bytes are compared with the browser reference files
//! that `scratchpad` preparation extracts from `.local/wasm/reference.json`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use super::Recorder;
use crate::files::dropped::{drop_outcome, DROP_EVENT, HOVER_EVENT};
use crate::files::FileTable;

const SCRIPT: &str = include_str!("e2e.js");
const PAGE_TIMEOUT: Duration = Duration::from_secs(180);
const DROP_TIMEOUT: Duration = Duration::from_secs(60);
/// Largest gap between animation frames tolerated during the 1200x1200 export.
const RAF_GAP_LIMIT_MS: u64 = 250;
/// Artwork preview size of the 23x17 fixture at cell 12: floor(width / cell) * cell per axis.
/// `png23x17.ui-preview.rgba` comes from `reference_fixtures` and matches the Chromium studio.
const PREVIEW_SIZE: (u64, u64) = (12, 12);
/// Artwork preview size of the 64x40 mode fixture at cell 6 with average sampling, from
/// `reference_fixtures`' `modes` entry `ascii_custom`.
const MODE_PREVIEW_SIZE: (u64, u64) = (66, 52);
/// Custom ramp typed into the studio for the ASCII phase; the reference export uses the same ramp.
const MODE_RAMP: &str = " .oO@";
/// RGBA8 bytes of the mode preview: 66 × 52 × 4.
const MODE_PREVIEW_BYTES: usize = 66 * 52 * 4;
/// Byte length of the ASCII PNG export reference from the `ascii_custom` entry.
const MODE_EXPORT_BYTES: usize = 5_282;
const REJECTED_DROP_MESSAGE: &str =
    "Choose one image at a time. 2 files were selected, so nothing was changed.";

/// Fixture and reference files, all prepared outside the repository.
struct Assets {
    png: PathBuf,
    png_export: PathBuf,
    ui_preview: PathBuf,
    large: PathBuf,
    large_export: PathBuf,
    mode: PathBuf,
    mode_preview: PathBuf,
    mode_export: PathBuf,
}

impl Assets {
    fn load(dir: &Path) -> Result<Self, String> {
        let assets = Self {
            png: dir.join("png23x17.png"),
            png_export: dir.join("png23x17.export.png"),
            ui_preview: dir.join("png23x17.ui-preview.rgba"),
            large: dir.join("large1200.png"),
            large_export: dir.join("large1200.export.png"),
            mode: dir.join("mode64x40.png"),
            mode_preview: dir.join("mode64x40.ascii-custom.preview.rgba"),
            mode_export: dir.join("mode64x40.ascii-custom.export.png"),
        };
        let empty: Vec<String> = [
            &assets.png,
            &assets.png_export,
            &assets.ui_preview,
            &assets.large,
            &assets.large_export,
            &assets.mode,
            &assets.mode_preview,
            &assets.mode_export,
        ]
        .into_iter()
        .filter(|path| !fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0))
        .map(|path| path.display().to_string())
        .collect();
        if !empty.is_empty() {
            return Err(format!("missing or empty: {}", empty.join(", ")));
        }
        let missing: Vec<String> = [
            &assets.png,
            &assets.png_export,
            &assets.ui_preview,
            &assets.large,
            &assets.large_export,
            &assets.mode,
            &assets.mode_preview,
            &assets.mode_export,
        ]
        .into_iter()
        .filter(|path| !path.is_file())
        .map(|path| path.display().to_string())
        .collect();
        if missing.is_empty() {
            Ok(assets)
        } else {
            Err(format!("missing: {}", missing.join(", ")))
        }
    }
}

/// Runs every end-to-end phase and records its checks. Returns the page's measurements, without
/// the raw preview pixels, for the report.
pub(super) fn run(
    recorder: &mut Recorder,
    app: &AppHandle,
    window: &WebviewWindow,
    reports: &Receiver<Value>,
    fixtures: Option<&Path>,
) -> Option<Value> {
    let Some(dir) = fixtures else {
        recorder.check(
            "e2e_fixtures_configured",
            false,
            "--smoke-e2e-dir was not given",
        );
        return None;
    };
    let assets = match Assets::load(dir) {
        Ok(assets) => assets,
        Err(missing) => {
            recorder.check("e2e_fixtures_present", false, missing);
            return None;
        }
    };
    let scratch = std::env::temp_dir().join(format!("i2b-e2e-{}", process::id()));
    let _ = fs::remove_dir_all(&scratch);
    if let Err(error) = fs::create_dir_all(&scratch) {
        recorder.check("e2e_scratch_created", false, error.to_string());
        return None;
    }
    let detail = main_phase(recorder, window, reports, &assets, &scratch);
    drop_phase(recorder, app, window, reports, &assets);
    let _ = fs::remove_dir_all(&scratch);
    detail
}

fn main_phase(
    recorder: &mut Recorder,
    window: &WebviewWindow,
    reports: &Receiver<Value>,
    assets: &Assets,
    scratch: &Path,
) -> Option<Value> {
    let config = json!({
        "mode": "main",
        "pngFixture": assets.png,
        "largeFixture": assets.large,
        "modeFixture": assets.mode,
        "modeRamp": MODE_RAMP,
        "outDir": scratch,
    });
    let main = match eval_and_wait(window, reports, &config, PAGE_TIMEOUT) {
        Ok(report) => report,
        Err(error) => {
            recorder.check("e2e_main_flow_completed", false, error);
            return None;
        }
    };
    let phase = text(&main, "phase");
    recorder.check(
        "e2e_main_flow_completed",
        phase == "main",
        format!("phase {phase}; error {}", text(&main, "error")),
    );
    check_main(recorder, &main, assets, scratch);
    let mut detail = main.clone();
    if let Some(preview) = detail.get_mut("preview").and_then(Value::as_object_mut) {
        preview.remove("rgba");
    }
    Some(detail)
}

fn check_main(recorder: &mut Recorder, main: &Value, assets: &Assets, scratch: &Path) {
    let png_metadata = text(&main["png"], "metadata");
    recorder.check(
        "e2e_png_import_through_the_ui",
        png_metadata.contains("png23x17.png")
            && png_metadata.contains("23 × 17 px")
            && png_metadata.contains("Local file"),
        format!("metadata {png_metadata:?}"),
    );

    let preview = &main["preview"];
    let expected_preview = read(&assets.ui_preview);
    let preview_size = (number(&preview["width"]), number(&preview["height"]));
    let preview_pixels = bytes_of(&preview["rgba"]);
    recorder.check(
        "e2e_preview_matches_browser_reference",
        preview_size == PREVIEW_SIZE && preview_pixels == expected_preview,
        format!(
            "size {preview_size:?}, {} bytes compared with {} reference bytes",
            preview_pixels.len(),
            expected_preview.len()
        ),
    );

    let png_status = text(&main["png"], "exportStatus");
    let png_out = read(&scratch.join("png.png"));
    recorder.check(
        "e2e_png_export_matches_browser_reference",
        png_status.starts_with("Saved png.png") && png_out == read(&assets.png_export),
        format!(
            "status {png_status:?}; {} bytes against {} reference bytes",
            png_out.len(),
            read(&assets.png_export).len()
        ),
    );

    check_mode(recorder, main, assets, scratch);

    let demo_metadata = text(&main["demo"], "metadata");
    let demo_preview = &main["demo"]["preview"];
    recorder.check(
        "e2e_demo_import_and_preview_through_the_ui",
        demo_metadata.contains("Built-in demo")
            && demo_metadata.contains("256 × 192 px")
            && number(&demo_preview["width"]) > 0
            && number(&demo_preview["height"]) > 0,
        format!("metadata {demo_metadata:?}, preview {demo_preview}"),
    );

    let large = &main["large"];
    let large_status = text(large, "status");
    recorder.check(
        "e2e_large_export_completed",
        large_status.starts_with("Saved large.png"),
        format!(
            "status {large_status:?}, {} ms",
            number(&large["elapsedMs"])
        ),
    );
    recorder.check(
        "e2e_large_export_matches_browser_reference",
        read(&scratch.join("large.png")) == read(&assets.large_export),
        format!("reference {} bytes", read(&assets.large_export).len()),
    );
    let gap = number(&large["maxRafGapMs"]);
    recorder.check(
        "e2e_large_export_keeps_the_ui_responsive",
        gap <= RAF_GAP_LIMIT_MS,
        format!(
            "largest rAF gap {gap} ms over {} frames (limit {RAF_GAP_LIMIT_MS} ms)",
            number(&large["frames"])
        ),
    );

    let loads: Vec<String> = main["loads"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    recorder.check(
        "e2e_no_csp_violations_during_studio_use",
        number(&main["cspViolations"]) == 0,
        format!("violations {}", main["cspViolations"]),
    );
    recorder.check(
        "e2e_native_app_loads_no_browser_worker_or_codec",
        loads.is_empty(),
        format!("browser worker or WASM resources {loads:?}"),
    );

    check_cancel_and_outputs(recorder, main, scratch);
}

/// The ASCII phase drives the mode and the custom ramp through the studio; its preview and PNG
/// must equal the independently rendered `ascii_custom` reference.
fn check_mode(recorder: &mut Recorder, main: &Value, assets: &Assets, scratch: &Path) {
    let mode = &main["mode"];
    let preview = &mode["preview"];
    let expected_preview = read(&assets.mode_preview);
    let preview_size = (number(&preview["width"]), number(&preview["height"]));
    let preview_pixels = bytes_of(&preview["rgba"]);
    recorder.check(
        "e2e_mode_ramp_is_applied_in_the_studio",
        text(mode, "ramp") == MODE_RAMP,
        format!("ramp field {:?}", text(mode, "ramp")),
    );
    recorder.check(
        "e2e_ascii_preview_matches_native_reference",
        preview_size == MODE_PREVIEW_SIZE
            && preview_pixels.len() == MODE_PREVIEW_BYTES
            && expected_preview.len() == MODE_PREVIEW_BYTES
            && preview_pixels == expected_preview,
        format!(
            "size {preview_size:?}, {} bytes compared with {} reference bytes",
            preview_pixels.len(),
            expected_preview.len()
        ),
    );
    let status = text(mode, "exportStatus");
    let exported = read(&scratch.join("mode-ascii.png"));
    recorder.check(
        "e2e_ascii_export_matches_native_reference",
        status.starts_with("Saved mode-ascii.png")
            && exported.len() == MODE_EXPORT_BYTES
            && exported == read(&assets.mode_export),
        format!(
            "status {status:?}; {} bytes against {} reference bytes",
            exported.len(),
            read(&assets.mode_export).len()
        ),
    );
}

fn check_cancel_and_outputs(recorder: &mut Recorder, main: &Value, scratch: &Path) {
    let cancel_status = text(&main["cancel"], "status");
    recorder.check(
        "e2e_cancel_during_export_leaves_no_file",
        cancel_status == "PNG export cancelled." && !scratch.join("cancelled.png").exists(),
        format!("status {cancel_status:?}"),
    );

    recorder.check(
        "e2e_reset_clears_the_source",
        main["reset"]["metadataCleared"] == json!(true)
            && main["reset"]["emptyCanvas"] == json!(true),
        format!("reset {}", main["reset"]),
    );

    recorder.check(
        "e2e_open_dialog_cancel_is_a_silent_no_op",
        number(&main["openCancel"]["alerts"]) == 0
            && main["openCancel"]["metadataAbsent"] == json!(true),
        format!("openCancel {}", main["openCancel"]),
    );

    let save_status = text(&main["saveCancel"], "status");
    recorder.check(
        "e2e_save_dialog_cancel_returns_to_idle_without_failure",
        save_status == "PNG export cancelled."
            && number(&main["saveCancel"]["alerts"]) == 0
            && main["saveCancel"]["errorPanel"] == json!(false),
        format!("saveCancel {}", main["saveCancel"]),
    );

    let names: BTreeSet<String> = fs::read_dir(scratch)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let expected: BTreeSet<String> = ["png.png", "large.png", "mode-ascii.png"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    recorder.check(
        "e2e_only_committed_outputs_exist",
        names == expected,
        format!("files {names:?}"),
    );
}

/// Native hover and drop events, delivered to the window the way the shell delivers them.
fn drop_phase(
    recorder: &mut Recorder,
    app: &AppHandle,
    window: &WebviewWindow,
    reports: &Receiver<Value>,
    assets: &Assets,
) {
    let config = json!({ "mode": "drop" });
    if let Err(error) = eval_script(window, &config) {
        recorder.check("e2e_native_drop_phase_started", false, error);
        return;
    }
    let _ = window.emit_to(window.label(), HOVER_EVENT, json!({ "active": true }));
    let hover = receive(reports, DROP_TIMEOUT);
    recorder.check(
        "e2e_native_hover_shows_a_drop_target",
        hover
            .as_ref()
            .is_ok_and(|value| text(value, "phase") == "drop-hover"),
        format!("hover report {hover:?}"),
    );
    let Some(table) = app.try_state::<Arc<FileTable>>() else {
        recorder.check("e2e_native_drop_imports_a_file", false, "no file table");
        return;
    };
    let accepted = drop_outcome(std::slice::from_ref(&assets.png), table.inner());
    let _ = window.emit_to(window.label(), DROP_EVENT, accepted);
    let imported = receive(reports, DROP_TIMEOUT);
    recorder.check(
        "e2e_native_drop_imports_a_file",
        imported.as_ref().is_ok_and(|value| {
            text(value, "phase") == "drop-accepted"
                && text(value, "metadata").contains("png23x17.png")
        }),
        format!("drop report {imported:?}"),
    );
    let rejected_paths = [assets.png.clone(), assets.large.clone()];
    let rejected = drop_outcome(&rejected_paths, table.inner());
    let _ = window.emit_to(window.label(), DROP_EVENT, rejected);
    let refused = receive(reports, DROP_TIMEOUT);
    recorder.check(
        "e2e_native_rejected_drop_text_matches_browser",
        refused.as_ref().is_ok_and(|value| {
            text(value, "phase") == "drop-rejected"
                && text(value, "message") == REJECTED_DROP_MESSAGE
        }),
        format!("rejected report {refused:?}"),
    );
}

fn eval_and_wait(
    window: &WebviewWindow,
    reports: &Receiver<Value>,
    config: &Value,
    timeout: Duration,
) -> Result<Value, String> {
    eval_script(window, config)?;
    receive(reports, timeout)
}

fn eval_script(window: &WebviewWindow, config: &Value) -> Result<(), String> {
    let script = format!("window.__i2bE2E = {config};\n{SCRIPT}");
    window.eval(script).map_err(|error| error.to_string())
}

fn receive(reports: &Receiver<Value>, timeout: Duration) -> Result<Value, String> {
    reports
        .recv_timeout(timeout)
        .map_err(|error| format!("no page report: {error}"))
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn number(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

fn bytes_of(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| u8::try_from(item.as_u64().unwrap_or(256)).unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_default()
}
