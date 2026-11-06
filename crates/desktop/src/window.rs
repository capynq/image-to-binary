//! Window geometry contract shared by `tauri.conf.json` and the smoke driver's resize checks.

/// Provisional minimum logical content width; the content test that confirms it is pending.
pub const MINIMUM_WIDTH_LOGICAL: f64 = 800.0;
/// Provisional minimum logical content height; the content test that confirms it is pending.
pub const MINIMUM_HEIGHT_LOGICAL: f64 = 600.0;

/// Lifts any resized content dimension below the minimum back to the minimum.
///
/// Installed only in smoke builds, where the driver resizes below the minimum and checks that
/// the result is clamped. Production sets no size programmatically. The guard only acts when a
/// dimension is below the minimum, so a clamped result does not trigger another correction.
#[cfg(feature = "smoke")]
pub fn enforce_minimum(window: &tauri::WebviewWindow, event: &tauri::WindowEvent) {
    use tauri::{LogicalSize, WindowEvent};

    let WindowEvent::Resized(physical) = event else {
        return;
    };
    let Ok(scale) = window.scale_factor() else {
        return;
    };
    let logical = physical.to_logical::<f64>(scale);
    if logical.width >= MINIMUM_WIDTH_LOGICAL && logical.height >= MINIMUM_HEIGHT_LOGICAL {
        return;
    }
    let width = logical.width.max(MINIMUM_WIDTH_LOGICAL);
    let height = logical.height.max(MINIMUM_HEIGHT_LOGICAL);
    let _ = window.set_size(LogicalSize::new(width, height));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn window_config() -> Value {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json must be valid JSON");
        config["app"]["windows"][0].clone()
    }

    #[test]
    fn configured_window_matches_the_geometry_contract() {
        let window = window_config();
        assert_eq!(window["width"].as_f64(), Some(1280.0));
        assert_eq!(window["height"].as_f64(), Some(800.0));
        assert_eq!(window["minWidth"].as_f64(), Some(MINIMUM_WIDTH_LOGICAL));
        assert_eq!(window["minHeight"].as_f64(), Some(MINIMUM_HEIGHT_LOGICAL));
        assert_eq!(window["resizable"].as_bool(), Some(true));
        assert_eq!(window["decorations"].as_bool(), Some(true));
        assert_eq!(window["center"].as_bool(), Some(true));
    }

    #[test]
    fn default_size_is_above_the_minimum() {
        let window = window_config();
        assert!(window["width"].as_f64() > window["minWidth"].as_f64());
        assert!(window["height"].as_f64() > window["minHeight"].as_f64());
    }
}
