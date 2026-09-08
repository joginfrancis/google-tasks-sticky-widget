//! Window placement and visibility helpers.

use tauri::{Manager, WebviewWindow};

/// How much of the widget must remain on a monitor for its position to be
/// considered usable. Enough that the header — the only drag handle — is
/// reachable with the mouse.
const MIN_VISIBLE_PX: i32 = 80;

/// Restored geometry can point at a monitor that no longer exists: an unplugged
/// dock, a laptop that woke up without its second screen, a resolution change.
/// The window-state plugin restores coordinates faithfully, which means it will
/// happily place the widget somewhere unreachable. Pull it back if so.
pub fn ensure_on_screen(window: &WebviewWindow) {
    let Ok(position) = window.outer_position() else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let Ok(monitors) = window.available_monitors() else {
        return;
    };

    if monitors.is_empty() {
        return;
    }

    let win_left = position.x;
    let win_top = position.y;
    let win_right = win_left + size.width as i32;
    let win_bottom = win_top + size.height as i32;

    let visible_somewhere = monitors.iter().any(|monitor| {
        let m_pos = monitor.position();
        let m_size = monitor.size();
        let m_left = m_pos.x;
        let m_top = m_pos.y;
        let m_right = m_left + m_size.width as i32;
        let m_bottom = m_top + m_size.height as i32;

        let overlap_x = (win_right.min(m_right) - win_left.max(m_left)).max(0);
        let overlap_y = (win_bottom.min(m_bottom) - win_top.max(m_top)).max(0);

        overlap_x >= MIN_VISIBLE_PX && overlap_y >= MIN_VISIBLE_PX
    });

    if !visible_somewhere {
        log::info!("restored window position is off-screen; recentering");
        let _ = window.center();
    }
}

/// Visibility drives the polling cadence, so every show/hide route has to tell
/// the sync manager. Centralised here so a new entry point cannot forget.
fn note_visibility(window: &WebviewWindow, visible: bool) {
    use tauri::Manager;
    if let Some(manager) = window.try_state::<crate::sync::SyncManager>() {
        manager.set_visible(visible);
    }
}

pub fn show_and_focus(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    note_visibility(window, true);
}

pub fn hide(window: &WebviewWindow) {
    let _ = window.hide();
    note_visibility(window, false);
}

/// Tray click and the tray menu both toggle, so they share this.
pub fn toggle_visibility(window: &WebviewWindow) {
    match window.is_visible() {
        Ok(true) => hide(window),
        _ => show_and_focus(window),
    }
}

pub fn main_window(app: &tauri::AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}
