import { useState } from "react";
import type {
  Settings,
  TaskList,
  ThemePreference,
  WindowLayer,
} from "../../types";
import { formatLastSynced } from "../../lib/date";
import "./SettingsPanel.css";

interface Props {
  settings: Settings;
  taskLists: TaskList[];
  /**
   * Connected-or-not, with no email address. Showing the account name would
   * need the `userinfo.email` scope, and SPEC §2.1 commits to `auth/tasks`
   * alone — a second scope on the consent screen is a real cost for a label.
   */
  connected: boolean;
  lastSyncedAt: string | null;
  /** Failure from a setting that couldn't be applied, e.g. a blocked registry write. */
  error: string | null;
  onChange: (patch: Partial<Settings>) => void;
  onDisconnect: () => void;
  onSyncNow: () => void;
  onClose: () => void;
}

const THEMES: { value: ThemePreference; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
];

// Two states, matching the pin control in the header. "Behind other windows"
// was dropped: on a note you summon with a hotkey and dismiss to the tray, a
// window that hides under everything has no job to do.
const LAYERS: { value: WindowLayer; label: string; hint: string }[] = [
  { value: "top", label: "On top", hint: "Above other windows" },
  { value: "normal", label: "Normal", hint: "An ordinary window" },
];

/**
 * Rendered as an in-widget overlay rather than the separate window SPEC §3.4
 * describes. On a 340px panel a second window costs more than it gives — see
 * BUILD_PLAN for the open decision.
 */
export function SettingsPanel(props: Props) {
  const { settings } = props;

  return (
    <div className="settings">
      <header className="settings-head" data-tauri-drag-region>
        <button
          className="icon-button"
          onClick={props.onClose}
          aria-label="Back"
          title="Back"
        >
          ‹
        </button>
        <span className="settings-title">Settings</span>
      </header>

      <div className="settings-body scroll-area">
        {props.error && <p className="settings-error">{props.error}</p>}

        <section>
          <h2>Window</h2>
          <div className="settings-row">
            <span className="settings-label">Position</span>
          </div>
          <div className="segmented">
            {LAYERS.map((layer) => (
              <button
                key={layer.value}
                className={`segment ${settings.windowLayer === layer.value ? "is-active" : ""}`}
                onClick={() => props.onChange({ windowLayer: layer.value })}
                title={layer.hint}
              >
                {layer.label}
              </button>
            ))}
          </div>

          <Toggle
            label="Show in taskbar"
            checked={settings.showInTaskbar}
            onChange={(v) => props.onChange({ showInTaskbar: v })}
          />
        </section>

        <section>
          <h2>Shortcut</h2>
          <Toggle
            label="Global show/hide shortcut"
            checked={settings.globalHotkeyEnabled}
            onChange={(v) => props.onChange({ globalHotkeyEnabled: v })}
          />
          {settings.globalHotkeyEnabled && (
            <>
              <HotkeyRecorder
                value={settings.globalHotkey}
                onChange={(accelerator) =>
                  props.onChange({ globalHotkey: accelerator })
                }
              />
              {accelerocatorWarning(settings.globalHotkey) ? (
                <p className="settings-warning">
                  {accelerocatorWarning(settings.globalHotkey)}
                </p>
              ) : (
                <p className="settings-hint">
                  Short and safe: a bare function key like F9, or Ctrl+`.
                </p>
              )}
            </>
          )}
        </section>

        <section>
          <h2>General</h2>
          <Toggle
            label="Start with Windows"
            checked={settings.startWithWindows}
            onChange={(v) => props.onChange({ startWithWindows: v })}
          />
          <Toggle
            label="Start hidden"
            checked={settings.startHidden}
            onChange={(v) => props.onChange({ startHidden: v })}
          />
          <p className="settings-hint">
            Opens straight to the tray instead of showing the widget.
          </p>
        </section>

        <section>
          <h2>Google</h2>
          <div className="settings-row">
            <span className="settings-label">Account</span>
            <span className="settings-value">
              {props.connected ? "Connected" : "Not connected"}
            </span>
          </div>

          <div className="settings-row">
            <span className="settings-label">Task list</span>
            <select
              className="settings-select"
              value={settings.selectedTaskListId ?? ""}
              onChange={(e) => props.onChange({ selectedTaskListId: e.target.value })}
            >
              {props.taskLists.map((list) => (
                <option key={list.id} value={list.id}>
                  {list.title}
                </option>
              ))}
            </select>
          </div>

          <div className="settings-row">
            <span className="settings-label">Last sync</span>
            <span className="settings-value">
              {formatLastSynced(props.lastSyncedAt)}
            </span>
          </div>

          <div className="settings-actions">
            <button className="settings-button" onClick={props.onSyncNow}>
              Sync now
            </button>
            <button
              className="settings-button is-danger"
              onClick={props.onDisconnect}
            >
              Disconnect
            </button>
          </div>
        </section>

        <section>
          <h2>Appearance</h2>
          <div className="segmented">
            {THEMES.map((theme) => (
              <button
                key={theme.value}
                className={`segment ${settings.theme === theme.value ? "is-active" : ""}`}
                onClick={() => props.onChange({ theme: theme.value })}
              >
                {theme.label}
              </button>
            ))}
          </div>
        </section>

        <section>
          <h2>About</h2>
          <div className="settings-row">
            <span className="settings-label">Version</span>
            <span className="settings-value">0.1.0</span>
          </div>
          <p className="settings-note">
            Your tasks travel only between this computer and Google. Nothing is
            sent anywhere else, and no analytics are collected.
          </p>
        </section>
      </div>
    </div>
  );
}

/**
 * Records a key combination by listening for one, rather than asking the user
 * to type an accelerator string. Nobody knows that `CmdOrCtrl+Shift+G` is the
 * spelling Tauri wants.
 */
function HotkeyRecorder({
  value,
  onChange,
}: {
  value: string;
  onChange: (accelerator: string) => void;
}) {
  const [recording, setRecording] = useState(false);

  const handleKeyDown = (event: React.KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();

    if (event.key === "Escape") {
      setRecording(false);
      return;
    }

    // A modifier on its own is not a shortcut — keep waiting for the real key.
    const modifierOnly = ["Control", "Shift", "Alt", "Meta"].includes(event.key);
    if (modifierOnly) return;

    const parts: string[] = [];
    if (event.ctrlKey || event.metaKey) parts.push("CmdOrCtrl");
    if (event.shiftKey) parts.push("Shift");
    if (event.altKey) parts.push("Alt");

    const key = event.key.length === 1 ? event.key.toUpperCase() : event.key;

    // Function keys type no character, so a bare one is a legitimate — and much
    // shorter — global shortcut. A bare letter is not: it would swallow that
    // letter in every application on the machine.
    const isFunctionKey = /^F([1-9]|1\d|2[0-4])$/.test(key);
    if (parts.length === 0 && !isFunctionKey) return;

    parts.push(key);

    setRecording(false);
    onChange(parts.join("+"));
  };

  return (
    <button
      className={`hotkey-recorder ${recording ? "is-recording" : ""}`}
      onClick={() => setRecording(true)}
      onKeyDown={recording ? handleKeyDown : undefined}
      onBlur={() => setRecording(false)}
    >
      {recording ? "Press a combination…" : prettyAccelerator(value)}
    </button>
  );
}

/**
 * Combinations Windows will happily give us, that you will regret taking.
 *
 * A global shortcut is exclusive: whatever we register stops reaching every
 * other application. These are the ones where that cost is large and invisible
 * until something breaks weeks later. Warn, don't block — it is the user's
 * machine.
 */
function accelerocatorWarning(accelerator: string): string | null {
  const normalized = accelerator.replace("CmdOrCtrl", "Ctrl");

  const known: Record<string, string> = {
    "Shift+Space": "types a space in every app — this would break typing",
    "Ctrl+Space": "switches input language on Windows, and is autocomplete in most editors",
    "Alt+Space": "opens the window menu in every Windows application",
    "Ctrl+C": "copy",
    "Ctrl+V": "paste",
    "Ctrl+X": "cut",
    "Ctrl+Z": "undo",
    "Ctrl+A": "select all",
    "Ctrl+S": "save",
    "Ctrl+F": "find",
    "Ctrl+Tab": "switches tabs everywhere",
    "Alt+Tab": "the Windows app switcher",
  };

  const reason = known[normalized];
  return reason ? `${normalized} is ${reason}. It would stop working elsewhere.` : null;
}

/** `CmdOrCtrl+Shift+G` reads as `Ctrl + Shift + G` on Windows. */
function prettyAccelerator(accelerator: string): string {
  return accelerator
    .split("+")
    .map((part) => (part === "CmdOrCtrl" ? "Ctrl" : part))
    .join(" + ");
}

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <button
      className="settings-row is-clickable"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
    >
      <span className="settings-label">{label}</span>
      <span className={`switch ${checked ? "is-on" : ""}`}>
        <span className="switch-knob" />
      </span>
    </button>
  );
}
