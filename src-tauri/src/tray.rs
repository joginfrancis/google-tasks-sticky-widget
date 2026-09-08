//! System tray icon and menu.
//!
//! The tray is the widget's only presence in the taskbar area (`skipTaskbar` is
//! on), so closing the window must never mean losing the app.

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, Manager, Wry};

use crate::commands;
use crate::settings::AppState;
use crate::window;

pub const MENU_TOGGLE: &str = "toggle";
pub const MENU_ALWAYS_ON_TOP: &str = "always_on_top";
pub const MENU_SYNC_NOW: &str = "sync_now";
pub const MENU_SETTINGS: &str = "settings";
pub const MENU_QUIT: &str = "quit";

/// Kept in managed state so always-on-top changes made from the UI can tick the
/// tray checkbox too — otherwise the two drift and the menu lies.
pub struct TrayHandles {
    pub always_on_top: CheckMenuItem<Wry>,
}

pub fn build(app: &App<Wry>) -> tauri::Result<()> {
    use crate::settings::WindowLayer;

    let always_on_top = app
        .state::<AppState>()
        .settings
        .lock()
        .map(|s| s.layer() == WindowLayer::Top)
        .unwrap_or(true);

    let toggle = MenuItem::with_id(app, MENU_TOGGLE, "Show / hide", true, None::<&str>)?;
    let aot = CheckMenuItem::with_id(
        app,
        MENU_ALWAYS_ON_TOP,
        "Always on top",
        true,
        always_on_top,
        None::<&str>,
    )?;
    let sync = MenuItem::with_id(app, MENU_SYNC_NOW, "Sync now", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, MENU_SETTINGS, "Settings…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &aot,
            &separator,
            &sync,
            &settings,
            &separator2,
            &quit,
        ],
    )?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().ok_or_else(|| {
            tauri::Error::AssetNotFound("default window icon missing".into())
        })?)
        .tooltip("Sticky Widget")
        .menu(&menu)
        // Left click toggles the widget; the menu is right-click only, which is
        // what Windows users expect from a tray icon.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_TOGGLE => {
                if let Some(win) = window::main_window(app) {
                    window::toggle_visibility(&win);
                }
            }
            MENU_ALWAYS_ON_TOP => {
                let _ = commands::toggle_top_from_tray(app);
            }
            MENU_SYNC_NOW => {
                app.state::<crate::sync::SyncManager>().wake();
            }
            MENU_SETTINGS => {
                // Settings lives in the WebView, so the window has to be up
                // before the event can land anywhere useful.
                if let Some(win) = window::main_window(app) {
                    window::show_and_focus(&win);
                }
                let _ = tauri::Emitter::emit(app, "ui:open-settings", ());
            }
            MENU_QUIT => app.exit(0),
            other => log::warn!("unhandled tray menu id: {other}"),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(win) = window::main_window(tray.app_handle()) {
                    window::toggle_visibility(&win);
                }
            }
        })
        .build(app)?;

    app.manage(TrayHandles {
        always_on_top: aot,
    });

    Ok(())
}
