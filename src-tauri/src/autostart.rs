//! Launch-with-Windows, via the per-user Run key.
//!
//! Written directly rather than through `tauri-plugin-autostart`: that crate
//! ships a build script, and Smart App Control (enforcing on this machine)
//! blocks unsigned build-script executables outright. Turning SAC off is
//! irreversible without reinstalling Windows, which is a wildly
//! disproportionate price for one registry value.
//!
//! `HKCU\...\Run` needs no elevation and applies to this user only, which is
//! what a personal widget should do anyway.

use std::path::PathBuf;

use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// The registry value name. Also what shows in Task Manager's Startup tab, so
/// it should read as the product rather than the binary.
const VALUE_NAME: &str = "Sticky Widget";


fn run_key(access: u32) -> Result<RegKey, String> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, access)
        .map_err(|e| format!("Could not read Windows startup settings: {e}"))
}

fn command_line() -> Result<String, String> {
    let exe: PathBuf = std::env::current_exe()
        .map_err(|e| format!("Could not locate the application: {e}"))?;

    // Quoted because the path routinely contains spaces — "Sticky note" here,
    // "Program Files" once installed. Unquoted, Windows would run the wrong
    // thing or nothing.
    // No flags: whether to start in the tray is the "Start hidden" setting's
    // call alone. Entries written with the old --hidden flag no longer match
    // this, so refresh_path_if_registered rewrites them on the next launch.
    Ok(format!("\"{}\"", exe.display()))
}

pub fn is_enabled() -> bool {
    let Ok(key) = run_key(KEY_READ) else {
        return false;
    };
    key.get_value::<String, _>(VALUE_NAME).is_ok()
}

pub fn enable() -> Result<(), String> {
    let key = run_key(KEY_WRITE)?;
    let command = command_line()?;

    key.set_value(VALUE_NAME, &command)
        .map_err(|e| format!("Windows wouldn't save the startup setting: {e}"))?;

    log::info!("registered for launch at sign-in");
    Ok(())
}

/// A missing value counts as success — the goal is "not registered".
pub fn disable() -> Result<(), String> {
    let key = run_key(KEY_WRITE)?;

    match key.delete_value(VALUE_NAME) {
        Ok(()) => {
            log::info!("removed from launch at sign-in");
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("Windows wouldn't remove the startup setting: {err}")),
    }
}

/// Rewrites the stored path if the executable has moved.
///
/// A stale entry is worse than none: Windows silently fails at sign-in and the
/// user is left believing the setting works.
pub fn refresh_path_if_registered() {
    if !is_enabled() {
        return;
    }

    // Read *and* write: it compares the stored entry before replacing it.
    // Opened write-only, the read was refused, the failure was taken to mean
    // "nothing to fix", and a stale entry was never once repaired.
    let (Ok(key), Ok(current)) = (run_key(KEY_READ | KEY_WRITE), command_line()) else {
        return;
    };

    let stored: Result<String, _> = key.get_value(VALUE_NAME);
    // An unreadable entry is treated as stale, not as fine: rewriting a correct
    // value costs nothing, while leaving a broken one breaks sign-in silently.
    if stored.map(|s| s != current).unwrap_or(true) {
        log::info!("startup entry pointed at an old path; updating it");
        let _ = key.set_value(VALUE_NAME, &current);
    }
}

