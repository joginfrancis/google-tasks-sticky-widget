//! Window-level preferences that outlive a run.
//!
//! Position and size are handled by `tauri-plugin-window-state`; this module
//! covers the things that plugin does not persist. Kept separate from the task
//! cache on purpose — losing this file must never lose user data, so it is
//! plain JSON that can be deleted at any time.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Retained so settings files written before the three-state pin still
    /// resolve to the right layer. `window_layer` supersedes it.
    pub always_on_top: bool,
    /// "top" | "normal" | "bottom". `None` on files predating this field.
    #[serde(default)]
    pub window_layer: Option<String>,
    /// Hide to tray on launch rather than showing the widget.
    pub start_hidden: bool,
    /// Show a Windows taskbar button. Off by default — a widget that occupies a
    /// taskbar slot stops reading as a widget — but findable-by-Alt-Tab is a
    /// reasonable thing to want.
    #[serde(default)]
    pub show_in_taskbar: bool,
    /// Global show/hide accelerator. `None` means the shortcut is switched off;
    /// absent from the file means "never configured", which takes the default.
    #[serde(default)]
    pub global_hotkey: Option<String>,
    /// Distinguishes "off" from "not yet configured", which `global_hotkey`
    /// alone cannot express.
    #[serde(default = "default_true")]
    pub global_hotkey_enabled: bool,
    /// Per-list note colour, keyed by list id.
    ///
    /// Local-only: the Tasks API has no colour field (docs/ui-parity.md §1),
    /// so this never leaves the machine and is invisible in Google.
    #[serde(default)]
    pub list_colors: std::collections::HashMap<String, String>,
    /// Notes that were open when the app last exited, as label -> list id.
    ///
    /// Recreated on launch with the *same labels*, because the window-state
    /// plugin keys geometry by label. Reusing them is what restores each note
    /// to its own position and size rather than a default one.
    #[serde(default)]
    pub open_notes: std::collections::HashMap<String, String>,
    /// Which Google task list the main note shows. `None` until first sync picks
    /// one, so a fresh install does not guess at a list id that may not exist.
    #[serde(default)]
    pub selected_task_list_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            window_layer: None,
            start_hidden: false,
            show_in_taskbar: false,
            global_hotkey: None,
            global_hotkey_enabled: true,
            list_colors: std::collections::HashMap::new(),
            open_notes: std::collections::HashMap::new(),
            selected_task_list_id: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Where the window sits relative to everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowLayer {
    /// Above other windows — the default for a glanceable widget.
    Top,
    /// An ordinary window.
    Normal,
    /// Below other windows: visible on the desktop, never in the way.
    Bottom,
}

impl WindowLayer {
    pub fn as_str(self) -> &'static str {
        match self {
            WindowLayer::Top => "top",
            WindowLayer::Normal => "normal",
            WindowLayer::Bottom => "bottom",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "top" => Some(WindowLayer::Top),
            "normal" => Some(WindowLayer::Normal),
            "bottom" => Some(WindowLayer::Bottom),
            _ => None,
        }
    }
}

impl Settings {
    /// Falls back to the pre-three-state boolean when the field is absent, so an
    /// existing preference is honoured rather than silently reset to Top.
    pub fn layer(&self) -> WindowLayer {
        self.window_layer
            .as_deref()
            .and_then(WindowLayer::parse)
            .unwrap_or(if self.always_on_top {
                WindowLayer::Top
            } else {
                WindowLayer::Normal
            })
    }

    /// The accelerator to register, or `None` when the shortcut is off.
    pub fn effective_hotkey(&self) -> Option<String> {
        if !self.global_hotkey_enabled {
            return None;
        }
        Some(
            self.global_hotkey
                .clone()
                .unwrap_or_else(|| crate::hotkey::DEFAULT_ACCELERATOR.to_string()),
        )
    }

    pub fn set_layer(&mut self, layer: WindowLayer) {
        self.window_layer = Some(layer.as_str().to_string());
        // Kept in step so anything still reading the boolean stays correct.
        self.always_on_top = layer == WindowLayer::Top;
    }
}

pub struct AppState {
    pub settings: Mutex<Settings>,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
        }
    }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("window.json"))
}

/// Never fails: a missing, unreadable, or malformed file falls back to defaults.
/// A corrupted preferences file must not stop the widget from opening.
pub fn load(app: &AppHandle) -> Settings {
    let Some(path) = settings_path(app) else {
        return Settings::default();
    };

    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|err| {
            log::warn!("settings file unreadable, using defaults: {err}");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    }
}

pub fn save(app: &AppHandle, settings: &Settings) {
    let Some(path) = settings_path(app) else {
        log::warn!("no config directory available; settings not saved");
        return;
    };

    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            log::warn!("could not create config directory: {err}");
            return;
        }
    }

    match serde_json::to_string_pretty(settings) {
        Ok(json) => {
            if let Err(err) = fs::write(&path, json) {
                log::warn!("could not write settings: {err}");
            }
        }
        Err(err) => log::warn!("could not serialize settings: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_falls_back_to_the_old_boolean() {
        // Settings written before the three-state pin existed.
        let legacy_on = Settings {
            always_on_top: true,
            window_layer: None,
            ..Default::default()
        };
        assert_eq!(legacy_on.layer(), WindowLayer::Top);

        let legacy_off = Settings {
            always_on_top: false,
            window_layer: None,
            ..Default::default()
        };
        assert_eq!(
            legacy_off.layer(),
            WindowLayer::Normal,
            "an existing preference must not be silently reset"
        );
    }

    #[test]
    fn explicit_layer_wins_over_the_boolean() {
        let settings = Settings {
            always_on_top: true,
            window_layer: Some("bottom".into()),
            ..Default::default()
        };
        assert_eq!(settings.layer(), WindowLayer::Bottom);
    }

    #[test]
    fn an_unrecognised_layer_degrades_rather_than_panicking() {
        let settings = Settings {
            always_on_top: true,
            window_layer: Some("sideways".into()),
            ..Default::default()
        };
        assert_eq!(settings.layer(), WindowLayer::Top);
    }

    #[test]
    fn set_layer_keeps_the_legacy_boolean_in_step() {
        let mut settings = Settings::default();

        settings.set_layer(WindowLayer::Bottom);
        assert_eq!(settings.layer(), WindowLayer::Bottom);
        assert!(!settings.always_on_top);

        settings.set_layer(WindowLayer::Top);
        assert!(settings.always_on_top);
    }

    #[test]
    fn layer_names_round_trip() {
        for layer in [WindowLayer::Top, WindowLayer::Normal, WindowLayer::Bottom] {
            assert_eq!(WindowLayer::parse(layer.as_str()), Some(layer));
        }
        assert_eq!(WindowLayer::parse("nonsense"), None);
    }

    #[test]
    fn settings_files_missing_new_fields_still_parse() {
        // Exactly what is on disk right now for this project.
        let raw = r#"{"alwaysOnTop":true,"startHidden":false}"#;
        let parsed: Settings = serde_json::from_str(raw).expect("must parse");
        assert_eq!(parsed.layer(), WindowLayer::Top);
        assert!(!parsed.show_in_taskbar);
        assert_eq!(parsed.selected_task_list_id, None);
    }
}
