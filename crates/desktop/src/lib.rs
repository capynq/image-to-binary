//! Native Tauri shell for image-to-binary.
//!
//! The shell owns the window and the IPC surface. Image decoding and rendering are
//! scheduled on the background [`worker::Worker`], never on the UI thread.

pub mod error;
mod files;
mod ipc;
pub mod limits;
#[cfg(feature = "smoke")]
mod smoke;
#[cfg(any(feature = "smoke", test))]
mod window;
pub mod worker;

use std::error::Error;
use std::process::ExitCode;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use tauri::{App, RunEvent, WebviewWindowBuilder};

use worker::Worker;

/// Starts the desktop application and returns the process exit code.
///
/// Returns [`ExitCode::FAILURE`] when the worker or the Tauri runtime cannot start.
/// Exit cancels the worker and waits at most its shutdown grace period, so a job that never
/// checkpoints cannot hang the process.
#[must_use]
pub fn run() -> ExitCode {
    let Ok(worker) = Worker::spawn() else {
        return ExitCode::FAILURE;
    };
    let worker = Arc::new(worker);
    let exit_worker = Arc::clone(&worker);
    let builder = tauri::Builder::default()
        .manage(worker)
        .manage(Arc::new(files::FileTable::default()))
        .setup(setup);
    #[cfg(feature = "smoke")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        ipc::app_info,
        ipc::app_limits,
        ipc::open_image_dialog,
        ipc::save_image_dialog,
        ipc::release_handle,
        ipc::echo_binary,
        smoke::smoke_page,
        smoke::smoke_worker_probe,
    ]);
    #[cfg(not(feature = "smoke"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        ipc::app_info,
        ipc::app_limits,
        ipc::open_image_dialog,
        ipc::save_image_dialog,
        ipc::release_handle,
    ]);
    let Ok(app) = builder.build(tauri::generate_context!()) else {
        return ExitCode::FAILURE;
    };
    let requested = Arc::new(AtomicI32::new(0));
    let recorded = Arc::clone(&requested);
    let runtime_code = app.run_return(move |_, event| match event {
        RunEvent::ExitRequested {
            code: Some(code), ..
        } => recorded.store(code, Ordering::SeqCst),
        RunEvent::Exit => {
            let _ = exit_worker.shutdown();
        }
        _ => {}
    });
    let code = match requested.load(Ordering::SeqCst) {
        0 => runtime_code,
        requested_code => requested_code,
    };
    u8::try_from(code).map_or(ExitCode::FAILURE, ExitCode::from)
}

fn setup(app: &mut App) -> Result<(), Box<dyn Error>> {
    let config = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .ok_or("the configuration declares no main window")?;
    #[cfg(feature = "smoke")]
    let smoke_run =
        smoke::Options::from_args(std::env::args()).map(|options| (options, smoke::install(app)));
    let builder = WebviewWindowBuilder::from_config(app.handle(), &config)?;
    #[cfg(feature = "smoke")]
    let builder = if smoke_run.is_some() {
        builder.initialization_script(smoke::init_script())
    } else {
        builder
    };
    let main_window = builder.build()?;
    let drop_window = main_window.clone();
    main_window.on_window_event(move |event| files::dropped::on_window_event(&drop_window, event));
    #[cfg(feature = "smoke")]
    {
        let guard = main_window.clone();
        main_window.on_window_event(move |event| window::enforce_minimum(&guard, event));
    }
    #[cfg(feature = "smoke")]
    if let Some((options, probes)) = smoke_run {
        smoke::start(app.handle().clone(), options, probes);
    }
    Ok(())
}
