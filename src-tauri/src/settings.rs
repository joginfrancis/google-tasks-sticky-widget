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

/// One note in a saved layout: which list it shows and where it sits.
///
/// Positions are physical pixels, as Windows reports them, with the scale
/// factor recorded so a layout saved on one display can be judged against
/// another rather than applied blindly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutNote {
    pub label: String,
    pub list_id: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_scale")]
    pub scale: f64,
    /// The display this note was on, by the name Windows gives it.
    ///
    /// What makes a layout survive a monitor being unplugged: with the display
    /// absent the saved coordinates are nowhere, and with it back they can be
    /// put right. Absent on layouts saved before this was recorded.
    #[serde(default)]
    pub monitor: Option<String>,

    /// Where that display was, and how big it was, when this was saved.
    ///
    /// Displays move: a laptop screen is at 0,0 alone and at 1920,0 once a
    /// monitor joins to its left. Without this the saved position is read in
    /// the new coordinate space and lands on whichever screen now occupies
    /// it — which is how notes came back on the wrong display.
    #[serde(default)]
    pub monitor_x: i32,
    #[serde(default)]
    pub monitor_y: i32,
    #[serde(default)]
    pub monitor_width: u32,
    #[serde(default)]
    pub monitor_height: u32,
}

fn default_scale() -> f64 {
    1.0
}

/// A note's place in an arrangement, as proportions rather than pixels.
///
/// The arrangement is described relative to its own bounding box: a note two
/// thirds across and a third of the way down stays there whatever the screen.
/// What makes a layout survive a change of display, where pixels cannot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelativeNote {
    pub label: String,
    pub list_id: String,
    pub fx: f64,
    pub fy: f64,
    pub fw: f64,
    pub fh: f64,
}

/// A named arrangement of notes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub name: String,
    pub notes: Vec<LayoutNote>,
}

/// How many arrangements can be kept.
///
/// A deliberate ceiling rather than an unbounded list: these are picked from a
/// dropdown on a 340px panel, and a dozen of them would be a filing problem
/// rather than a convenience.
pub const MAX_LAYOUTS: usize = 5;

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
    /// Pin state per window label, so one note can float while another sits
    /// behind. `window_layer` above is only the default for notes with no
    /// entry here.
    #[serde(default)]
    pub note_layers: std::collections::HashMap<String, String>,
    /// Named arrangements, chosen from Settings.
    #[serde(default)]
    pub layouts: Vec<Layout>,

    /// The layout most recently applied or saved.
    #[serde(default)]
    pub last_layout: Option<String>,

    /// The shape of the arrangement, independent of any screen.
    ///
    /// Recorded whenever the notes settle, and projected onto whatever display
    /// is available when the screens change. The per-display memory below is
    /// the exact-pixel version for a configuration that has been seen before;
    /// this is the answer for one that has not.
    #[serde(default)]
    pub relative_layout: Vec<RelativeNote>,

    /// The arrangement last seen with each set of displays, keyed by their
    /// names.
    ///
    /// Remembered without being asked, because "put it back as it was" depends
    /// on which screens were attached: the one-screen arrangement and the
    /// two-screen one are different answers to the same request, and a single
    /// saved layout can only give one of them.
    #[serde(default)]
    pub layout_by_screens: std::collections::HashMap<String, Vec<LayoutNote>>,

    /// The single arrangement kept by the first version of this feature.
    /// Migrated into `layouts` on load and then left empty.
    ///
    /// Separate from `open_notes`, which only records what is on screen right
    /// now: a template survives closing every note, and survives the drift that
    /// comes of dragging a window and forgetting.
    #[serde(default)]
    pub saved_layout: Vec<LayoutNote>,

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
            note_layers: std::collections::HashMap::new(),
            saved_layout: Vec::new(),
            layouts: Vec::new(),
            last_layout: None,
            layout_by_screens: std::collections::HashMap::new(),
            relative_layout: Vec::new(),
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

    /// This window's pin state, falling back to the global default for a note
    /// that has never been pinned or unpinned.
    pub fn layer_for(&self, label: &str) -> WindowLayer {
        self.note_layers
            .get(label)
            .map(String::as_str)
            .and_then(WindowLayer::parse)
            .unwrap_or_else(|| self.layer())
    }

    pub fn set_layer_for(&mut self, label: &str, layer: WindowLayer) {
        self.note_layers
            .insert(label.to_string(), layer.as_str().to_string());

        // The main note also moves the global default, so a newly opened note
        // inherits whatever the user last chose rather than a fixed value.
        if label == crate::notes::MAIN_LABEL {
            self.set_layer(layer);
        }
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

    let mut settings: Settings = match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|err| {
            log::warn!("settings file unreadable, using defaults: {err}");
            Settings::default()
        }),
        Err(_) => Settings::default(),
    };

    // Positions recorded while a note was hidden: Windows reports -32000 for
    // a parked window, and an older build wrote that down as the note's place.
    // Dropped here so a good arrangement can replace them.
    for notes in settings.layout_by_screens.values_mut() {
        notes.retain(|n| n.x > -30000 && n.y > -30000 && n.width > 120 && n.height > 120);
    }
    // An arrangement that lost entries to the cleaning above is no longer an
    // arrangement — restoring it would place one note and abandon the others.
    let full = settings.relative_layout.len();
    settings
        .layout_by_screens
        .retain(|_, notes| !notes.is_empty() && notes.len() >= full);
    for layout in &mut settings.layouts {
        layout
            .notes
            .retain(|n| n.x > -30000 && n.y > -30000 && n.width > 120 && n.height > 120);
    }

    // The single slot became a list; anything saved under the old shape keeps
    // working rather than quietly disappearing.
    if settings.layouts.is_empty() && !settings.saved_layout.is_empty() {
        settings.layouts.push(Layout {
            name: "Saved layout".to_string(),
            notes: std::mem::take(&mut settings.saved_layout),
        });
    }

    settings
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
