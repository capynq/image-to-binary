//! Native menu bar, its accelerators and the page-facing action and enabled-state contracts.
//!
//! Menu selections reach the page only as [`MENU_EVENT`] payloads on the main window. The page
//! listens and never receives an emit permission, and it can change only the enabled state of
//! the items listed in [`MenuState`], through the validated `set_menu_state` command.

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuBuilder, MenuItem, MenuItemBuilder, MenuItemKind, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::error::{ErrorCode, IpcError};

/// Event name that carries a [`MenuPayload`] to the main window.
pub const MENU_EVENT: &str = "native-menu-action";
/// Window that receives menu actions; it is the only window the shell declares.
const MAIN_WINDOW: &str = "main";

/// A studio action a menu item requests. The page maps each one onto the same handler its button uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    Open,
    Export,
    Original,
    Artwork,
    Split,
    Render,
    Cancel,
}

impl MenuAction {
    /// Whether the item is enabled before the page reports any state. Only Open is available
    /// without an image, so every other item starts disabled and the first paint is correct.
    #[must_use]
    pub const fn starts_enabled(self) -> bool {
        matches!(self, Self::Open)
    }

    /// Stable wire name that the page's menu listener matches.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Export => "export",
            Self::Original => "original",
            Self::Artwork => "artwork",
            Self::Split => "split",
            Self::Render => "render",
            Self::Cancel => "cancel",
        }
    }
}

/// One item of the bar that routes to a studio action.
///
/// Invariant: `id` is unique across [`ENTRIES`], and the page addresses the item by it in
/// [`MenuState::items`]. `accelerator` uses the Tauri accelerator syntax, and `None` means the
/// item has no shortcut and the page owns the key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Entry {
    /// Stable menu item id, such as `file.export`.
    pub id: &'static str,
    /// Studio action the item requests when it is selected.
    pub action: MenuAction,
    /// Accelerator shown and bound by the menu, or `None` for no native shortcut.
    pub accelerator: Option<&'static str>,
}

/// Open image. `CmdOrCtrl+O` matches the native convention for opening a document.
pub const OPEN: Entry = Entry {
    id: "file.open",
    action: MenuAction::Open,
    accelerator: Some("CmdOrCtrl+O"),
};
/// Export. Uses `CmdOrCtrl+E` because no document is saved, so `S` would suggest a project save.
pub const EXPORT: Entry = Entry {
    id: "file.export",
    action: MenuAction::Export,
    accelerator: Some("CmdOrCtrl+E"),
};
/// Original view; `CmdOrCtrl+1` picks the first view without taking Enter or Escape.
pub const ORIGINAL: Entry = Entry {
    id: "view.original",
    action: MenuAction::Original,
    accelerator: Some("CmdOrCtrl+1"),
};
/// Artwork view; `CmdOrCtrl+2` pairs with the original view.
pub const ARTWORK: Entry = Entry {
    id: "view.artwork",
    action: MenuAction::Artwork,
    accelerator: Some("CmdOrCtrl+2"),
};
/// Split view; `CmdOrCtrl+3` continues the 1 and 2 views and shares their gate with Artwork.
pub const SPLIT: Entry = Entry {
    id: "view.split",
    action: MenuAction::Split,
    accelerator: Some("CmdOrCtrl+3"),
};
/// Render. Uses `CmdOrCtrl+R`, not `CmdOrCtrl+Enter`, so Enter keeps its meaning in fields and buttons.
pub const RENDER: Entry = Entry {
    id: "view.render",
    action: MenuAction::Render,
    accelerator: Some("CmdOrCtrl+R"),
};
/// Cancel has no accelerator so Escape stays with the page, where a field or dialog can claim it first.
pub const CANCEL: Entry = Entry {
    id: "view.cancel",
    action: MenuAction::Cancel,
    accelerator: None,
};

/// Every studio item, in menu order.
pub const ENTRIES: &[Entry] = &[OPEN, EXPORT, ORIGINAL, ARTWORK, SPLIT, RENDER, CANCEL];

/// Maps a menu item id to its studio action, or `None` for predefined or unknown ids.
#[must_use]
pub fn action_for_id(id: &str) -> Option<MenuAction> {
    ENTRIES
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| entry.action)
}

/// Payload of [`MENU_EVENT`]: the wire name of the requested action.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MenuPayload {
    action: &'static str,
}

/// Enabled state the page reports. Every field is required and no other field is accepted.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuState {
    pub export: bool,
    pub render: bool,
    pub original: bool,
    pub artwork: bool,
    pub split: bool,
    pub cancel: bool,
}

impl MenuState {
    /// Pairs each controllable item id with its requested enabled flag.
    #[must_use]
    pub const fn items(self) -> [(&'static str, bool); 6] {
        [
            ("file.export", self.export),
            ("view.render", self.render),
            ("view.original", self.original),
            ("view.artwork", self.artwork),
            ("view.split", self.split),
            ("view.cancel", self.cancel),
        ]
    }
}

/// Validates the raw `set_menu_state` argument.
///
/// Failure contract: `invalid_argument` for a non-object, a missing or non-boolean field, or any
/// unknown field, so a page that drifts from this shape is refused rather than partly applied.
pub fn parse_menu_state(value: serde_json::Value) -> Result<MenuState, IpcError> {
    serde_json::from_value(value).map_err(|error| {
        IpcError::new(
            ErrorCode::InvalidArgument,
            format!("the menu state is invalid: {error}"),
        )
    })
}

fn item<R: Runtime>(app: &AppHandle<R>, declared: &Entry) -> tauri::Result<MenuItem<R>> {
    let mut builder = MenuItemBuilder::with_id(declared.id, label(declared.action))
        .enabled(declared.action.starts_enabled());
    if let Some(accelerator) = declared.accelerator {
        builder = builder.accelerator(accelerator);
    }
    builder.build(app)
}

fn label(action: MenuAction) -> &'static str {
    match action {
        MenuAction::Open => "Open…",
        MenuAction::Export => "Export…",
        MenuAction::Original => "Original",
        MenuAction::Artwork => "Artwork",
        MenuAction::Split => "Split",
        MenuAction::Render => "Render",
        MenuAction::Cancel => "Cancel",
    }
}

/// Builds the menu bar: the platform application menu on macOS, then File, Edit, View and Window.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let bar = MenuBuilder::new(app);
    #[cfg(target_os = "macos")]
    let application = SubmenuBuilder::new(app, "Image to Binary")
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    #[cfg(target_os = "macos")]
    let bar = bar.item(&application);
    let open = item(app, &OPEN)?;
    let export = item(app, &EXPORT)?;
    let file = SubmenuBuilder::new(app, "File")
        .item(&open)
        .item(&export)
        .separator()
        .close_window();
    #[cfg(not(target_os = "macos"))]
    let file = file.separator().quit();
    let file = file.build()?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let original = item(app, &ORIGINAL)?;
    let artwork = item(app, &ARTWORK)?;
    let split = item(app, &SPLIT)?;
    let render = item(app, &RENDER)?;
    let cancel = item(app, &CANCEL)?;
    let view = SubmenuBuilder::new(app, "View")
        .item(&original)
        .item(&artwork)
        .item(&split)
        .separator()
        .item(&render)
        .item(&cancel)
        .build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .build()?;
    bar.items(&[&file, &edit, &view, &window]).build()
}

/// Sends the studio action of a menu id to the page. Returns `false` for ids that carry no action.
pub fn dispatch<R: Runtime>(app: &AppHandle<R>, id: &str) -> bool {
    let Some(action) = action_for_id(id) else {
        return false;
    };
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return false;
    };
    window
        .emit_to(
            window.label(),
            MENU_EVENT,
            MenuPayload {
                action: action.name(),
            },
        )
        .is_ok()
}

/// Applies a validated enabled state to the installed menu.
pub fn apply_state<R: Runtime>(app: &AppHandle<R>, state: MenuState) -> tauri::Result<()> {
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    for (id, enabled) in state.items() {
        if let Some(item) = find_item(menu.items()?, id) {
            item.set_enabled(enabled)?;
        }
    }
    Ok(())
}

/// Finds a plain menu item by id at any depth. `Menu::get` and `Submenu::get` search one level
/// only, and every studio item sits inside a submenu, so the search walks each submenu.
pub fn find_item<R: Runtime>(items: Vec<MenuItemKind<R>>, id: &str) -> Option<MenuItem<R>> {
    for kind in items {
        match kind {
            MenuItemKind::MenuItem(item) if item.id().as_ref() == id => return Some(item),
            MenuItemKind::Submenu(submenu) => {
                if let Ok(children) = submenu.items() {
                    if let Some(found) = find_item(children, id) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid() -> serde_json::Value {
        json!({
            "export": true,
            "render": false,
            "original": true,
            "artwork": false,
            "split": false,
            "cancel": true
        })
    }

    #[test]
    fn every_entry_has_a_unique_id_and_action() {
        for (index, entry) in ENTRIES.iter().enumerate() {
            assert!(ENTRIES[index + 1..]
                .iter()
                .all(|other| other.id != entry.id));
            assert!(ENTRIES[index + 1..]
                .iter()
                .all(|other| other.action != entry.action));
        }
    }

    #[test]
    fn ids_resolve_to_their_actions_and_unknown_ids_do_not() {
        assert_eq!(action_for_id("file.export"), Some(MenuAction::Export));
        assert_eq!(action_for_id("view.render"), Some(MenuAction::Render));
        assert_eq!(action_for_id("view.cancel"), Some(MenuAction::Cancel));
        assert_eq!(action_for_id("edit.paste"), None);
        assert_eq!(action_for_id(""), None);
    }

    #[test]
    fn accelerators_avoid_escape_and_enter_so_fields_keep_their_keys() {
        for entry in ENTRIES {
            if let Some(accelerator) = entry.accelerator {
                assert!(!accelerator.contains("Escape"), "{accelerator}");
                assert!(!accelerator.contains("Enter"), "{accelerator}");
            }
        }
        assert_eq!(CANCEL.accelerator, None);
    }

    #[test]
    fn export_and_render_use_the_documented_accelerators() {
        assert_eq!(EXPORT.accelerator, Some("CmdOrCtrl+E"));
        assert_eq!(RENDER.accelerator, Some("CmdOrCtrl+R"));
        assert_eq!(OPEN.accelerator, Some("CmdOrCtrl+O"));
    }

    #[test]
    fn split_is_a_view_item_with_the_third_view_accelerator() {
        assert_eq!(SPLIT.id, "view.split");
        assert_eq!(SPLIT.accelerator, Some("CmdOrCtrl+3"));
        assert_eq!(action_for_id("view.split"), Some(MenuAction::Split));
        assert_eq!(MenuAction::Split.name(), "split");
        assert!(ENTRIES
            .iter()
            .filter(|entry| entry.accelerator == Some("CmdOrCtrl+3"))
            .all(|entry| entry.action == MenuAction::Split));
    }

    #[test]
    fn split_shares_the_artwork_gate_and_starts_disabled() {
        assert!(!MenuAction::Split.starts_enabled());
        let state = parse_menu_state(json!({
            "export": true,
            "render": true,
            "original": true,
            "artwork": true,
            "split": true,
            "cancel": false
        }))
        .expect("valid state");
        let items = state.items();
        assert_eq!(items[3], ("view.artwork", true));
        assert_eq!(items[4], ("view.split", true));
    }

    #[test]
    fn a_state_without_split_is_refused() {
        let mut value = valid();
        value.as_object_mut().expect("object").remove("split");
        assert_eq!(
            parse_menu_state(value).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_complete_state_is_accepted_and_maps_each_item() {
        let state = parse_menu_state(valid()).expect("valid state");
        assert_eq!(
            state.items(),
            [
                ("file.export", true),
                ("view.render", false),
                ("view.original", true),
                ("view.artwork", false),
                ("view.split", false),
                ("view.cancel", true),
            ]
        );
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut value = valid();
        value["open"] = json!(true);
        assert_eq!(
            parse_menu_state(value).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn missing_or_non_boolean_fields_are_refused() {
        let mut missing = valid();
        missing.as_object_mut().expect("object").remove("cancel");
        assert_eq!(
            parse_menu_state(missing).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );

        let mut stringly = valid();
        stringly["export"] = json!("true");
        assert_eq!(
            parse_menu_state(stringly).unwrap_err().code(),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn non_object_arguments_are_refused() {
        for value in [json!(null), json!(true), json!([]), json!("state")] {
            assert_eq!(
                parse_menu_state(value).unwrap_err().code(),
                ErrorCode::InvalidArgument
            );
        }
    }

    #[test]
    fn only_open_is_enabled_before_the_page_reports_state() {
        for entry in ENTRIES {
            assert_eq!(
                entry.action.starts_enabled(),
                entry.action == MenuAction::Open
            );
        }
    }

    #[test]
    fn payload_carries_the_wire_name() {
        let payload = serde_json::to_value(MenuPayload {
            action: MenuAction::Render.name(),
        })
        .expect("payload serialises");
        assert_eq!(payload, json!({ "action": "render" }));
    }
}
