//! IPC surface exposed to the WebView.
//!
//! ARCHITECTURE §3: every command here is a coarse-grained intent, not a
//! primitive. Nothing takes a URL or a path, and nothing returns credentials.
//! Keep this list short — each entry is attack surface.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::auth::{flow, tokens, AuthState};
use crate::settings::{self, AppState, WindowLayer};
use crate::tray::TrayHandles;
use crate::window;

/// Single place where the window layer changes: applies it to the window,
/// updates persisted settings, ticks the tray checkbox, and tells the UI. The
/// tray menu and the IPC command both route through here so the three can never
/// disagree.
pub fn apply_layer_to(
    app: &AppHandle,
    label: &str,
    layer: WindowLayer,
) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(label) {
        // Order matters: clearing the opposite flag first avoids a moment where
        // both are set, which Windows resolves unpredictably.
        match layer {
            WindowLayer::Top => {
                let _ = win.set_always_on_bottom(false);
                win.set_always_on_top(true)
                    .map_err(|e| format!("could not pin the window: {e}"))?;
            }
            WindowLayer::Normal => {
                let _ = win.set_always_on_top(false);
                let _ = win.set_always_on_bottom(false);
            }
            WindowLayer::Bottom => {
                let _ = win.set_always_on_top(false);
                win.set_always_on_bottom(true)
                    .map_err(|e| format!("could not push the window back: {e}"))?;
            }
        }
    }

    let state = app.state::<AppState>();
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;
        settings.set_layer_for(label, layer);
        settings::save(app, &settings);
    }

    // The tray checkbox speaks for the main note only; it has no way to show
    // three notes in different states.
    if label == crate::notes::MAIN_LABEL {
        if let Some(handles) = app.try_state::<TrayHandles>() {
            let _ = handles
                .always_on_top
                .set_checked(layer == WindowLayer::Top);
        }
    }

    // Addressed to the one window, not broadcast: every note listens for this,
    // and a broadcast would make them all redraw their pin to match whichever
    // one was clicked.
    let _ = app.emit_to(label, "window:layer", layer.as_str());
    Ok(())
}

#[tauri::command]
pub fn set_window_layer(
    app: AppHandle,
    window: tauri::Window,
    layer: String,
) -> Result<(), String> {
    let parsed = WindowLayer::parse(&layer)
        .ok_or_else(|| format!("unknown window layer: {layer}"))?;
    apply_layer_to(&app, window.label(), parsed)
}

#[tauri::command]
pub fn get_window_layer(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    state
        .settings
        .lock()
        .map(|s| s.layer_for(window.label()).as_str().to_string())
        .map_err(|_| "settings lock poisoned".to_string())
}

/// Toggling the tray checkbox means "top or ordinary" — it has no third state
/// to express, so Bottom is reachable only from the widget's own pin control.
pub fn toggle_top_from_tray(app: &AppHandle) -> Result<(), String> {
    let current = app
        .state::<AppState>()
        .settings
        .lock()
        .map(|s| s.layer_for(crate::notes::MAIN_LABEL))
        .unwrap_or(WindowLayer::Top);

    apply_layer_to(
        app,
        crate::notes::MAIN_LABEL,
        if current == WindowLayer::Top {
            WindowLayer::Normal
        } else {
            WindowLayer::Top
        },
    )
}

#[tauri::command]
pub fn set_show_in_taskbar(app: AppHandle, value: bool) -> Result<(), String> {
    if let Some(win) = window::main_window(&app) {
        win.set_skip_taskbar(!value)
            .map_err(|e| format!("Windows wouldn't change the taskbar button: {e}"))?;
    }

    let state = app.state::<AppState>();
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    settings.show_in_taskbar = value;
    settings::save(&app, &settings);
    Ok(())
}

/// Everything the Settings panel needs, in one round trip.
///
/// `start_with_windows` is read from the registry rather than our settings file,
/// because the registry entry is the truth: the user can remove it from Task
/// Manager's Startup tab, and a cached copy would then quietly lie.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSettings {
    pub window_layer: String,
    pub show_in_taskbar: bool,
    pub global_hotkey: String,
    pub global_hotkey_enabled: bool,
    /// Non-null when the stored shortcut could not be registered this run.
    pub global_hotkey_error: Option<String>,
    pub start_hidden: bool,
    pub start_with_windows: bool,
}

#[tauri::command]
pub fn get_window_settings(
    app: AppHandle,
    window: tauri::Window,
) -> Result<WindowSettings, String> {
    let state = app.state::<AppState>();
    let settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;

    Ok(WindowSettings {
        // This window's own pin state, not the global default. Every other
        // field here really is app-wide; pinning is the one that belongs to a
        // single note, and returning the default made every note's pin control
        // show — and appear to follow — whatever was last chosen in any of them.
        window_layer: settings.layer_for(window.label()).as_str().to_string(),
        show_in_taskbar: settings.show_in_taskbar,
        global_hotkey: settings
            .global_hotkey
            .clone()
            .unwrap_or_else(|| crate::hotkey::DEFAULT_ACCELERATOR.to_string()),
        global_hotkey_enabled: settings.global_hotkey_enabled,
        global_hotkey_error: app
            .try_state::<crate::hotkey::HotkeyStatus>()
            .and_then(|s| s.error.lock().ok().and_then(|e| e.clone())),
        start_hidden: settings.start_hidden,
        start_with_windows: crate::autostart::is_enabled(),
    })
}

#[tauri::command]
pub fn set_start_hidden(app: AppHandle, value: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    settings.start_hidden = value;
    settings::save(&app, &settings);
    Ok(())
}

/// Writes or removes the Windows startup registry entry.
///
/// Failures surface rather than being swallowed — a toggle that silently does
/// nothing is worse than one that says why it couldn't.
///
/// Returns what the registry actually holds afterwards, not what was asked for,
/// so the UI can never drift from the machine.
#[tauri::command]
pub fn set_start_with_windows(value: bool) -> Result<bool, String> {
    if value {
        crate::autostart::enable()?;
    } else {
        crate::autostart::disable()?;
    }
    Ok(crate::autostart::is_enabled())
}

/// Sets or clears the global shortcut.
///
/// On failure the previous setting is left untouched — a rejected accelerator
/// must not leave the user with no shortcut at all.
#[tauri::command]
pub fn set_global_hotkey(
    app: AppHandle,
    accelerator: Option<String>,
    enabled: bool,
) -> Result<(), String> {
    let candidate = if enabled {
        Some(
            accelerator
                .clone()
                .unwrap_or_else(|| crate::hotkey::DEFAULT_ACCELERATOR.to_string()),
        )
    } else {
        None
    };

    // Register first, persist only if Windows accepted it.
    crate::hotkey::apply(&app, candidate.as_deref())?;

    let state = app.state::<AppState>();
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    settings.global_hotkey_enabled = enabled;
    if enabled {
        settings.global_hotkey = candidate;
    }
    settings::save(&app, &settings);
    Ok(())
}

/* -- Note colour ---------------------------------------------------------- */

/// Colours are per *list*, not per window, so two notes on the same list match.
///
/// Stored locally only — the Tasks API has no colour field, so this is invisible
/// in Google's own clients and lost if the settings file is deleted. That is the
/// honest cost of a feature the API does not support.
#[tauri::command]
pub fn set_list_color(
    app: AppHandle,
    list_id: String,
    color: Option<String>,
) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;

        match &color {
            // `None` means "back to the default surface", which is an absence
            // rather than a colour — storing a literal default would freeze it
            // against future theme changes.
            None => {
                settings.list_colors.remove(&list_id);
            }
            Some(value) => {
                settings.list_colors.insert(list_id.clone(), value.clone());
            }
        }
        settings::save(&app, &settings);
    }

    // Every note showing this list repaints, not just the one that changed it.
    let _ = app.emit("list:color", (list_id, color));
    Ok(())
}

#[tauri::command]
pub fn get_list_colors(
    state: tauri::State<'_, AppState>,
) -> Result<std::collections::HashMap<String, String>, String> {
    state
        .settings
        .lock()
        .map(|s| s.list_colors.clone())
        .map_err(|_| "settings lock poisoned".to_string())
}

#[tauri::command]
pub fn get_selected_task_list(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    state
        .settings
        .lock()
        .map(|s| s.selected_task_list_id.clone())
        .map_err(|_| "settings lock poisoned".to_string())
}

#[tauri::command]
pub fn set_selected_task_list(app: AppHandle, task_list_id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    settings.selected_task_list_id = Some(task_list_id.clone());
    settings::save(&app, &settings);
    drop(settings);

    // Derive the polling set from the notes that are actually open, rather than
    // setting it to this one list.
    //
    // `set_list` *replaces* the set, so switching the main window's list used to
    // stop every other note syncing until it happened to re-register. The note
    // registry is the only thing that knows the full set, and refresh_lists
    // reads it.
    let sync = app.state::<crate::sync::SyncManager>();
    sync.refresh_lists(&app);
    sync.wake();
    Ok(())
}

#[tauri::command]
pub fn hide_to_tray(app: AppHandle) -> Result<(), String> {
    if let Some(win) = window::main_window(&app) {
        window::hide(&win);
    }
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Lets the WebView write into the same log file as the Rust side.
///
/// Without this, frontend behaviour is invisible in a release build — there is
/// no console attached — so a click that never reaches a command looks exactly
/// like a command that ran and did nothing.
///
/// Tagged with the window label, because with several notes open "which window
/// did this come from" is usually the question.
#[tauri::command]
pub fn ui_log(window: tauri::Window, level: String, message: String) {
    let label = window.label();
    match level.as_str() {
        "error" => log::error!("[ui:{label}] {message}"),
        "warn" => log::warn!("[ui:{label}] {message}"),
        _ => log::info!("[ui:{label}] {message}"),
    }
}

/* -- Google account ------------------------------------------------------- */

/// Everything the UI is allowed to know about the account.
///
/// Deliberately no email address: that would need the `userinfo.email` scope,
/// and SPEC §2.1 commits to `auth/tasks` alone. Connected-or-not is enough to
/// drive every screen we have.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub connected: bool,
    /// Set while a browser sign-in is open, so the UI can show progress.
    pub authorizing: bool,
}

#[tauri::command]
pub fn get_account_status(auth: tauri::State<'_, AuthState>) -> AccountStatus {
    AccountStatus {
        connected: tokens::has_refresh_token(),
        authorizing: auth
            .in_progress
            .lock()
            .map(|guard| *guard)
            .unwrap_or(false),
    }
}

/// Fire-and-forget: the flow blocks on a browser round-trip, so it runs on a
/// background thread and reports back through `auth:changed`.
#[tauri::command]
pub fn begin_google_auth(app: AppHandle) -> Result<(), String> {
    {
        let auth = app.state::<AuthState>();
        if !auth.begin() {
            return Err("A sign-in is already in progress. Check your browser.".into());
        }
    }

    std::thread::spawn(move || {
        let result = flow::authorize(|url| {
            tauri_plugin_opener::open_url(url, None::<&str>)
                .map_err(|e| format!("Could not open your browser: {e}"))
        });

        let auth = app.state::<AuthState>();
        auth.finish();

        match result {
            Ok(authenticated) => {
                auth.cache.store(authenticated.access);
                log::info!("google account connected");
                let _ = app.emit("auth:changed", true);
            }
            Err(message) => {
                log::warn!("sign-in failed: {message}");
                let _ = app.emit("auth:error", message);
                let _ = app.emit("auth:changed", tokens::has_refresh_token());
            }
        }
    });

    Ok(())
}

/// Revocation is best-effort; deletion is not. A network failure must never
/// leave credentials on the machine after the user asked to disconnect.
#[tauri::command]
pub fn disconnect_google(app: AppHandle) -> Result<(), String> {
    let stored = tokens::load_refresh_token().unwrap_or(None);

    if let Some(token) = stored {
        if let Err(err) = flow::revoke(&token) {
            log::warn!("token revocation failed, deleting locally anyway: {err}");
        }
    }

    let delete_result = tokens::delete_refresh_token();

    let auth = app.state::<AuthState>();
    auth.cache.clear();

    // Leaving one account's tasks on screen after disconnecting would be both
    // confusing and a small privacy leak.
    if let Err(err) = app.state::<crate::store::Store>().clear_tasks() {
        log::warn!("could not clear cached tasks on disconnect: {err}");
    }

    let _ = app.emit("auth:changed", false);
    delete_result
}
