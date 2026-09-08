import { invoke } from "@tauri-apps/api/core";

/**
 * Writes a line into the app's own log file, alongside the Rust output.
 *
 * A release build has no console, so `console.log` goes nowhere. Anything worth
 * diagnosing after the fact has to come through here.
 *
 * Never log task titles, notes, or anything from the user's data — the log file
 * is for behaviour, not content (ARCHITECTURE §7.3).
 */
export function uiLog(message: string, level: "info" | "warn" | "error" = "info") {
  invoke("ui_log", { level, message }).catch(() => {
    // Logging must never be the thing that breaks a feature.
  });
}
