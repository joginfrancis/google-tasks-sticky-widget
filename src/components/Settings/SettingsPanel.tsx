import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
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
  onOpenHelp: () => void;
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
          {/* A real arrow, not a chevron glyph. "‹" is a typographic quote
              mark: it renders light, sits off the optical centre and changes
              shape with the font. This is the same 1.5px round-capped stroke
              as every other icon in the app. */}
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <path
              d="M12.5 8H4M7.5 4L3.5 8l4 4"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
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
          <h2>Layout</h2>
          <Layout />
        </section>

        <section>
          <h2>List</h2>
          <Toggle
            label="Show subtasks"
            checked={settings.showSubtasks}
            onChange={(v) => props.onChange({ showSubtasks: v })}
          />
          <p className="settings-hint">
            Off keeps the note to top-level tasks. A task you open still shows
            its own subtasks, so nothing becomes unreachable.
          </p>
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
          {/* Above Version, because it is the row someone actually came looking
              for: most of what this widget can do is a gesture with no
              affordance, and Help is the only place that says so. */}
          <button
            className="settings-row is-clickable"
            onClick={props.onOpenHelp}
          >
            <span className="settings-label">Help &amp; shortcuts</span>
            <span className="settings-value" aria-hidden="true">
              ›
            </span>
          </button>
          <div className="settings-row">
            <span className="settings-label">Version</span>
            <span className="settings-value">{useAppVersion()}</span>
          </div>
          {/* Whose app this is. A widget with no name on it reads as one
              nobody stands behind. */}
          <div className="settings-row">
            <span className="settings-label">Made by</span>
            <span className="settings-value">Jogin Francis</span>
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
 * Keeping arrangements, and rebuilding them.
 *
 * Notes are windows, and windows drift: a restart, a monitor unplugged, or an
 * afternoon of dragging leaves an arrangement that took effort to build
 * looking like it was dealt from a deck.
 *
 * Five buttons in two rows was the first attempt, and it read as a control
 * panel. This is the same capability as a choice and three marks: picking a
 * layout *is* applying it, arranging happens on the press, and what is left is
 * keeping the result — over the chosen name, or as a new one.
 */
function Layout() {
  const [names, setNames] = useState<string[]>([]);
  const [selected, setSelected] = useState("");
  const [naming, setNaming] = useState(false);
  const [draft, setDraft] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [screens, setScreens] = useState<
    { name: string; label: string; primary: boolean; hasNotes: boolean }[]
  >([]);
  /**
   * The screen last chosen here.
   *
   * The dropdown cannot be driven by where the notes are: the windows move on
   * a background thread, so asking straight afterwards still answers with the
   * old screen, the control snaps back to it, and the next choice is "the
   * value it already has" — which fires no change at all. Hence moving worked
   * exactly once.
   */
  const [screenChoice, setScreenChoice] = useState<string | null>(null);

  /** Arranged but not yet kept — the save marks pick this up. */
  const [dirty, setDirty] = useState(false);

  const refresh = (keep?: string) =>
    invoke<string[]>("list_layouts")
      .then((list) => {
        setNames(list);
        setSelected((current) =>
          keep ?? (list.includes(current) ? current : (list[0] ?? "")),
        );
      })
      .catch(() => {});

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    invoke<{ name: string; label: string; primary: boolean; hasNotes: boolean }[]>(
      "list_screens",
    )
      .then(setScreens)
      .catch(() => {});
  }, []);


  /** Says what is happening now, and later what happened. */
  const say = (message: string, settle = false) => {
    setNote(message);
    if (settle) window.setTimeout(() => setNote(null), 2600);
  };

  const apply = (name: string) => {
    if (!name) return;
    setSelected(name);
    say("Restoring " + name + "…");
    invoke<number>("apply_layout", { name })
      .then((n) => {
        setDirty(false);
        say(name + " — " + n + (n === 1 ? " note" : " notes"), true);
      })
      .catch((err: unknown) => say(String(err), true));
  };

  const arrange = () => {
    say("Arranging…");
    invoke<number>("arrange_notes")
      .then((n) => {
        setDirty(true);
        say("Arranged " + n + (n === 1 ? " note" : " notes") + " — keep it?", true);
      })
      .catch((err: unknown) => say(String(err), true));
  };

  const snap = () => {
    say("Aligning…");
    invoke<number>("snap_notes")
      .then((n) => {
        setDirty(true);
        say("Aligned " + n + (n === 1 ? " note" : " notes") + " — keep it?", true);
      })
      .catch((err: unknown) => say(String(err), true));
  };

  const save = (name: string) => {
    say("Saving " + name + "…");
    invoke<number>("save_layout", { name })
      .then((count) => {
        void refresh(name);
        setNaming(false);
        setDraft("");
        setDirty(false);
        say(name + " — " + count + (count === 1 ? " note" : " notes") + " saved", true);
      })
      .catch((err: unknown) => say(String(err), true));
  };

  return (
    <>
      <div className="settings-row">
        <span className="settings-label">Layout</span>
        <select
          className="settings-select"
          value={selected}
          // Choosing one is the whole gesture: a separate Use button asked the
          // user to say the same thing twice.
          onChange={(e) => apply(e.target.value)}
        >
          {names.length === 0 && <option value="">None saved</option>}
          {names.map((name) => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
      </div>

      {screens.length > 1 && (
        <div className="settings-row">
          <span className="settings-label">Screen</span>
          <select
            className="settings-select"
            // The one the notes are on, or the main one if they are spread.
            value={screenChoice ?? screens.find((s) => s.hasNotes)?.name ?? ""}
            onChange={(e) => {
              const name = e.target.value;
              const screen = screens.find((s) => s.name === name);
              setScreenChoice(name);
              say("Moving the notes…");
              invoke<number>("move_notes_to_screen", { name })
                .then((n) => {
                  setDirty(true);
                  // Asked again once the windows have actually moved: the
                  // placing happens on its own thread, so an immediate answer
                  // still describes where they were.
                  window.setTimeout(() => {
                    void invoke<typeof screens>("list_screens")
                      .then(setScreens)
                      .catch(() => {});
                  }, 900);
                  say(
                    "Moved " +
                      n +
                      (n === 1 ? " note" : " notes") +
                      " to " +
                      (screen?.label ?? "that screen"),
                    true,
                  );
                })
                .catch((err: unknown) => say(String(err), true));
            }}
          >
            {screens.map((screen) => (
              <option key={screen.name} value={screen.name}>
                {screen.label}
              </option>
            ))}
          </select>
        </div>
      )}

      {naming ? (
        <div className="settings-actions">
          <input
            className="settings-name"
            autoFocus
            value={draft}
            placeholder="Name this layout…"
            maxLength={40}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && draft.trim()) save(draft.trim());
              else if (e.key === "Escape") {
                e.stopPropagation();
                setNaming(false);
                setDraft("");
              }
            }}
          />
          <button
            className="settings-button"
            disabled={!draft.trim()}
            onClick={() => save(draft.trim())}
          >
            Keep
          </button>
        </div>
      ) : (
        <div className="layout-tools">
          <button className="layout-tool" onClick={arrange} title="Arrange the notes evenly">
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <rect x="2" y="2" width="5" height="12" rx="1.3" fill="none" stroke="currentColor" strokeWidth="1.3" />
              <rect x="9" y="2" width="5" height="12" rx="1.3" fill="none" stroke="currentColor" strokeWidth="1.3" />
            </svg>
            <span>Arrange</span>
          </button>

          <button
            className="layout-tool"
            onClick={snap}
            title="Snap the notes onto a grid, keeping where they are"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                d="M4.5 3v5a3.5 3.5 0 0 0 7 0V3"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.5"
              />
              <path d="M3 3h3M10 3h3" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" />
            </svg>
            <span>Align</span>
          </button>

          <button
            className={"layout-tool " + (dirty && selected ? "is-wanted" : "")}
            disabled={!selected}
            onClick={() => selected && save(selected)}
            title={selected ? "Save over " + selected : "No layout selected"}
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                d="M3 3.5h7.5L13 6v6.5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1z"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinejoin="round"
              />
              <path d="M5.5 3.5v3h5M5.5 13.5v-3h5v3" fill="none" stroke="currentColor" strokeWidth="1.3" />
            </svg>
            <span>Save</span>
          </button>

          <button
            className={"layout-tool " + (dirty && !selected ? "is-wanted" : "")}
            onClick={() => setNaming(true)}
            title="Keep this arrangement as a new layout"
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path d="M8 3.5v9M3.5 8h9" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
            </svg>
            <span>New</span>
          </button>

          <button
            className="layout-tool is-danger"
            disabled={!selected}
            onClick={() =>
              selected &&
              invoke("delete_layout", { name: selected })
                .then(() => {
                  void refresh();
                  say("Deleted " + selected, true);
                })
                .catch((err: unknown) => say(String(err), true))
            }
            title={selected ? "Delete " + selected : "No layout selected"}
          >
            <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
              <path
                d="M3.5 4.5h9M6.5 4.5V3h3v1.5M5 4.5l.6 8h4.8l.6-8"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </div>
      )}

      <p className="settings-hint">
        {note ??
          (names.length > 0
            ? "Pick one to put the notes back. Align tidies what you have; Arrange starts again in columns."
            : "Align snaps the notes where they are onto a grid; Arrange lays them out afresh in columns.")}
      </p>
    </>
  );
}

/** The version the app actually is, rather than one typed into this file. */
function useAppVersion(): string {
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => {});
  }, []);
  return version;
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
