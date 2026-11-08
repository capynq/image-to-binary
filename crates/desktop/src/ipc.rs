//! Narrow typed command surface. Binary payloads travel as raw bytes, never as JSON or base64.

use std::sync::Arc;

use serde::Serialize;
use tauri::ipc::Channel;

use crate::error::{ErrorCode, IpcError};
use crate::files::dialog::{self, NativePicker};
use crate::files::{FileId, FileRef, FileTable, SinkRef};
use crate::limits::AppLimits;
use crate::processing::{
    self, ExportReport, ExportRequest, ImportedDemo, ImportedSource, PreviewKind, PreviewMode,
    PreviewRequest, Processing, ProgressEvent,
};

/// Codecs compiled into this binary; the native codec path is enabled by the `native-codecs` feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct CodecSupport {
    heif: bool,
    avif: bool,
}

/// Build identity returned by the `app_info` command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    name: &'static str,
    version: &'static str,
    target: &'static str,
    codecs: CodecSupport,
}

/// Returns the build identity without touching the window or the worker.
#[must_use]
pub fn app_info_value() -> AppInfo {
    let native_codecs = cfg!(feature = "native-codecs");
    AppInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        target: env!("IMAGE_TO_BINARY_TARGET"),
        codecs: CodecSupport {
            heif: native_codecs,
            avif: native_codecs,
        },
    }
}

#[tauri::command]
pub(crate) fn app_info() -> AppInfo {
    app_info_value()
}

#[tauri::command]
pub(crate) fn app_limits() -> AppLimits {
    crate::limits::app_limits()
}

/// Shows the image picker. Resolves to the chosen file's handle and name, or `None` when the
/// user cancels. Never returns a path.
///
/// Failure contract: `invalid_argument` for a non-local, directory, or non-regular selection;
/// `encoded_too_large` above the input limit; `resource_limit_exceeded` when the handle table is
/// full. The dialog is shown by the platform without blocking the UI thread.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn open_image_dialog(
    app: tauri::AppHandle,
    files: tauri::State<'_, Arc<FileTable>>,
) -> Result<Option<FileRef>, IpcError> {
    #[cfg(feature = "smoke")]
    if let Some(answer) = crate::smoke::take_open_answer(&app) {
        return answer;
    }
    dialog::pick_source(&NativePicker::new(app), files.inner().clone()).await
}

/// Shows the save dialog with a suggested name that is used only as a display hint. `outputKind`
/// (`png` by default, or `text`) picks the filter and the default extension. Resolves to the
/// destination's handle and name, or `None` when cancelled. Never returns a path.
///
/// Failure contract: `invalid_settings` or `invalid_argument` for an unknown or non-string kind;
/// `invalid_argument` when the chosen location has no file name; `resource_limit_exceeded` when
/// the handle table is full.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn save_image_dialog(
    app: tauri::AppHandle,
    files: tauri::State<'_, Arc<FileTable>>,
    suggested_name: Option<String>,
    output_kind: Option<serde_json::Value>,
) -> Result<Option<SinkRef>, IpcError> {
    let output = processing::output_kind_argument(output_kind.as_ref())?;
    #[cfg(feature = "smoke")]
    if let Some(answer) = crate::smoke::take_save_answer(&app) {
        return answer;
    }
    dialog::pick_destination(
        &NativePicker::new(app),
        files.inner().clone(),
        suggested_name.as_deref(),
        output,
    )
    .await
}

/// Releases a handle. Idempotent: an unknown or already released id succeeds, so the page can
/// release without tracking which handles remain.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn release_handle(files: tauri::State<'_, Arc<FileTable>>, id: FileId) {
    files.release(id);
}

/// Imports the file behind `file_id` as the one source and returns its metadata. The file is read
/// and decoded on the worker; the page never sends bytes. A rejected file keeps the previous source.
///
/// Failure contract: the read errors (`invalid_argument` for a non-regular file, `encoded_too_large`
/// above the input ceiling) and the decoder's codes (`unsupported_format`, `decode_failed`,
/// `codec_unavailable`), `cancelled` when superseded.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn import_source(
    processing: tauri::State<'_, Arc<Processing>>,
    file_id: FileId,
    generation: Option<serde_json::Value>,
) -> Result<ImportedSource, IpcError> {
    let generation = processing::generation_argument(generation.as_ref())?.value();
    processing.import_source(file_id, generation).await
}

/// Replaces the source with the built-in demo and returns its metadata. No pixels cross IPC.
///
/// Failure contract: `cancelled` when superseded, and the session's codes for the RGBA plan.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn import_demo(
    processing: tauri::State<'_, Arc<Processing>>,
    generation: Option<serde_json::Value>,
) -> Result<ImportedDemo, IpcError> {
    let generation = processing::generation_argument(generation.as_ref())?.value();
    processing.import_demo(generation).await
}

/// Renders one frame of a preview and returns the raw frame body described in
/// `processing::frame`. `frame` is the source frame index and defaults to `0`. `settings` is
/// required for `artwork` and ignored for `original`. The generation, frame and settings arrive as
/// raw JSON and are converted with the shared argument contract, so a wrong type or value gives
/// the browser's `{ code, message }`.
///
/// Failure contract: `invalid_argument` and `invalid_settings` from the contract (an index outside
/// the source's frames is `invalid_argument`), `no_job` without a source, `stale_generation`,
/// `cancelled` when superseded or when the page generation's frame run was cancelled, and the
/// session's codes.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn start_preview(
    processing: tauri::State<'_, Arc<Processing>>,
    kind: PreviewKind,
    settings: Option<serde_json::Value>,
    generation: Option<serde_json::Value>,
    frame: Option<serde_json::Value>,
    progress: Channel<ProgressEvent>,
) -> Result<tauri::ipc::Response, IpcError> {
    let generation = processing::generation_argument(generation.as_ref())?.value();
    let frame = processing::frame_argument(frame.as_ref())?;
    let mode = match kind {
        PreviewKind::Original => PreviewMode::Original,
        PreviewKind::Artwork => {
            PreviewMode::Artwork(processing::settings_argument(settings.as_ref())?)
        }
    };
    let frame = processing
        .render_preview(
            PreviewRequest {
                generation,
                mode,
                frame,
            },
            progress,
        )
        .await?;
    Ok(tauri::ipc::Response::new(frame))
}

/// Exports one frame of the artwork into the chosen sink as `outputKind` (`png` by default, or
/// `text`) and commits it atomically. `frame` is the source frame index and defaults to `0`.
/// Returns the committed byte length; the destination path never crosses IPC.
///
/// Failure contract: the shared argument codes for `settings`, `generation`, `frame` and
/// `outputKind`, `invalid_argument` for an unavailable destination or a frame outside the source,
/// `internal` for a write failure, `cancelled` or `stale_generation` when superseded before the
/// commit.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn export_file(
    processing: tauri::State<'_, Arc<Processing>>,
    settings: Option<serde_json::Value>,
    sink_id: FileId,
    generation: Option<serde_json::Value>,
    output_kind: Option<serde_json::Value>,
    frame: Option<serde_json::Value>,
    progress: Channel<ProgressEvent>,
) -> Result<ExportReport, IpcError> {
    let generation = processing::generation_argument(generation.as_ref())?.value();
    let settings = processing::settings_argument(settings.as_ref())?;
    let output = processing::output_kind_argument(output_kind.as_ref())?;
    let frame = processing::frame_argument(frame.as_ref())?;
    processing
        .export_file(
            ExportRequest {
                generation,
                settings,
                output,
                sink: sink_id,
                frame,
            },
            progress,
        )
        .await
}

/// Cancels the active preview or export when it belongs to `generation`. Idempotent.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn cancel(
    processing: tauri::State<'_, Arc<Processing>>,
    generation: Option<serde_json::Value>,
) -> Result<(), IpcError> {
    processing.cancel(processing::generation_argument(generation.as_ref())?.value());
    Ok(())
}

/// Cancels all work and releases the held source. Resolves once the release has run on the worker.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn reset(processing: tauri::State<'_, Arc<Processing>>) -> Result<(), IpcError> {
    processing.reset().await
}

#[cfg(feature = "smoke")]
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn echo_binary(
    request: tauri::ipc::Request<'_>,
    state: tauri::State<'_, crate::smoke::SmokeState>,
) -> Result<tauri::ipc::Response, crate::error::IpcError> {
    use crate::error::{ErrorCode, IpcError};
    use crate::limits::{validate_payload_length, SMOKE_ECHO_MAX_BYTES};

    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(IpcError::new(
            ErrorCode::InvalidArgument,
            "echo requires a raw binary request body",
        ));
    };
    validate_payload_length(bytes.len(), SMOKE_ECHO_MAX_BYTES)?;
    state.record_echo(bytes);
    Ok(tauri::ipc::Response::new(bytes.clone()))
}

/// Applies the page's enabled state to the menu bar. The argument is validated by
/// [`crate::menu::parse_menu_state`] before any item changes, so a malformed state changes nothing.
///
/// Failure contract: `invalid_argument` for any shape other than the five required booleans.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn set_menu_state(
    app: tauri::AppHandle,
    state: serde_json::Value,
) -> Result<(), IpcError> {
    let state = crate::menu::parse_menu_state(state)?;
    crate::menu::apply_state(&app, state).map_err(|error| {
        IpcError::new(
            ErrorCode::Internal,
            format!("the menu could not be updated: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn app_info_reports_identity_and_target() {
        let info = app_info_value();
        assert_eq!(info.name, "image-to-binary-desktop");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.target.is_empty());
        assert!(info.target.contains('-'));
    }

    #[test]
    fn app_info_serializes_camel_case_with_codec_flags() {
        let json = serde_json::to_value(app_info_value()).unwrap();
        assert_eq!(json["name"], json!("image-to-binary-desktop"));
        assert!(json["codecs"]["heif"].is_boolean());
        assert_eq!(json["codecs"]["heif"], json["codecs"]["avif"]);
        assert_eq!(
            json["codecs"]["heif"].as_bool().unwrap(),
            cfg!(feature = "native-codecs")
        );
    }

    #[test]
    fn app_limits_command_matches_the_limits_module() {
        let json = serde_json::to_value(app_limits()).unwrap();
        assert_eq!(
            json,
            serde_json::to_value(crate::limits::app_limits()).unwrap()
        );
    }
}
