//! Narrow typed command surface. Binary payloads travel as raw bytes, never as JSON or base64.

use std::sync::Arc;

use serde::Serialize;

use crate::error::IpcError;
use crate::files::dialog::{self, NativePicker};
use crate::files::{FileId, FileRef, FileTable, SinkRef};
use crate::limits::AppLimits;

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
    dialog::pick_source(&NativePicker::new(app), files.inner().clone()).await
}

/// Shows the save dialog with a suggested name that is used only as a display hint. Resolves to
/// the destination's handle and name, or `None` when cancelled. Never returns a path.
///
/// Failure contract: `invalid_argument` when the chosen location has no file name;
/// `resource_limit_exceeded` when the handle table is full.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) async fn save_image_dialog(
    app: tauri::AppHandle,
    files: tauri::State<'_, Arc<FileTable>>,
    suggested_name: Option<String>,
) -> Result<Option<SinkRef>, IpcError> {
    dialog::pick_destination(
        &NativePicker::new(app),
        files.inner().clone(),
        suggested_name.as_deref(),
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
