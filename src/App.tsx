import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { Settings, ThemePreference, WindowLayer } from "./types";
import { useTasks } from "./hooks/useTasks";
import { uiLog } from "./lib/log";
import { TaskWidget } from "./components/TaskWidget/TaskWidget";
import { SettingsPanel } from "./components/Settings/SettingsPanel";
import { Onboarding } from "./components/Onboarding/Onboarding";

import "./styles/tokens.css";
import "./styles/base.css";

/** Pause between ticking a task and it moving to Completed, so the eye follows. */
const SETTLE_MS = 700;

const THEME_KEY = "theme-preference";

type View = "widget" | "settings";

/** Mirrors `commands::AccountStatus` in Rust. No email — see SettingsPanel. */
interface AccountStatus {
  connected: boolean;
  authorizing: boolean;
}

export default function App() {
  const [view, setView] = useState<View>("widget");
  const [settlingIds, setSettlingIds] = useState<Set<string>>(new Set());

  // `null` means "not asked yet", which is distinct from "not connected" —
  // showing the connect screen before Rust answers would flash it every launch.
  const [account, setAccount] = useState<AccountStatus | null>(null);
  const [authError, setAuthError] = useState<string | null>(null);

  const [windowLayer, setWindowLayer] = useState<WindowLayer>("top");
  const [showInTaskbar, setShowInTaskbar] = useState(false);
  const [startHidden, setStartHidden] = useState(false);
  const [startWithWindows, setStartWithWindows] = useState(false);
  const [globalHotkey, setGlobalHotkey] = useState("CmdOrCtrl+Shift+G");
  const [globalHotkeyEnabled, setGlobalHotkeyEnabled] = useState(true);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  /** Transient failure shown on the widget itself, e.g. a note that would not open. */
  const [widgetError, setWidgetError] = useState<string | null>(null);
  /** Note colour per list id. Local-only; the Tasks API has no colour field. */
  const [listColors, setListColors] = useState<Record<string, string>>({});
  const [theme, setTheme] = useState<ThemePreference>(() => {
    // Purely cosmetic and per-machine, so browser storage is the right home
    // for it — no round-trip to Rust before the first paint.
    try {
      const saved = localStorage.getItem(THEME_KEY);
      if (saved === "light" || saved === "dark" || saved === "system") return saved;
    } catch {
      /* private mode, cleared storage — fall through to the default */
    }
    return "system";
  });

  const tasks = useTasks(account?.connected ?? false);

  /* -- Theme --------------------------------------------------------------- */

  useEffect(() => {
    const root = document.documentElement;
    if (theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", theme);

    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      /* not worth surfacing; the theme still applies for this session */
    }
  }, [theme]);

  /* -- Account ------------------------------------------------------------- */

  useEffect(() => {
    invoke<AccountStatus>("get_account_status")
      .then(setAccount)
      .catch((err) => {
        console.error(err);
        setAccount({ connected: false, authorizing: false });
      });

    const changed = listen<boolean>("auth:changed", (event) => {
      setAccount({ connected: event.payload, authorizing: false });
      if (event.payload) setAuthError(null);
    });
    const failed = listen<string>("auth:error", (event) =>
      setAuthError(event.payload),
    );

    return () => {
      changed.then((fn) => fn());
      failed.then((fn) => fn());
    };
  }, []);

  const handleConnect = useCallback(() => {
    setAuthError(null);
    setAccount((a) => ({ connected: a?.connected ?? false, authorizing: true }));
    invoke("begin_google_auth").catch((err: unknown) => {
      setAuthError(String(err));
      setAccount((a) => ({ connected: a?.connected ?? false, authorizing: false }));
    });
  }, []);

  const handleDisconnect = useCallback(() => {
    invoke("disconnect_google")
      .then(() => setView("widget"))
      .catch((err: unknown) => setAuthError(String(err)));
  }, []);

  /* -- Native window ------------------------------------------------------- */

  useEffect(() => {
    invoke<{
      windowLayer: WindowLayer;
      showInTaskbar: boolean;
      startHidden: boolean;
      startWithWindows: boolean;
      globalHotkey: string;
      globalHotkeyEnabled: boolean;
      globalHotkeyError: string | null;
    }>("get_window_settings")
      .then((s) => {
        setWindowLayer(s.windowLayer);
        setShowInTaskbar(s.showInTaskbar);
        setStartHidden(s.startHidden);
        // Read from the registry, not a cached copy — the user can remove the
        // entry from Task Manager's Startup tab behind our back.
        setStartWithWindows(s.startWithWindows);
        setGlobalHotkey(s.globalHotkey);
        setGlobalHotkeyEnabled(s.globalHotkeyEnabled);
        // A shortcut another app already owns fails at startup, not on click —
        // so the only place to surface it is here.
        if (s.globalHotkeyError) setSettingsError(s.globalHotkeyError);
      })
      .catch(console.error);

    // The tray menu changes this too, so mirror it rather than assume.
    const unlisten = listen<WindowLayer>("window:layer", (event) =>
      setWindowLayer(event.payload),
    );
    const openSettings = listen("ui:open-settings", () => setView("settings"));

    invoke<Record<string, string>>("get_list_colors")
      .then(setListColors)
      .catch(console.error);

    // Every note showing a list repaints when its colour changes, not just the
    // window where the change was made.
    const colorChanged = listen<[string, string | null]>(
      "list:color",
      (event) => {
        const [listId, color] = event.payload;
        setListColors((prev) => {
          const next = { ...prev };
          if (color === null) delete next[listId];
          else next[listId] = color;
          return next;
        });
      },
    );

    return () => {
      unlisten.then((fn) => fn());
      openSettings.then((fn) => fn());
      colorChanged.then((fn) => fn());
    };
  }, []);

  const applyLayer = useCallback((layer: WindowLayer) => {
    setWindowLayer(layer);
    invoke("set_window_layer", { layer }).catch((err: unknown) =>
      setSettingsError(String(err)),
    );
  }, []);

  /* -- Completion settle --------------------------------------------------- */

  const handleToggle = useCallback(
    (id: string) => {
      // Hold the row in place briefly so it does not vanish under the cursor.
      setSettlingIds((prev) => new Set(prev).add(id));
      setTimeout(() => {
        setSettlingIds((prev) => {
          const next = new Set(prev);
          next.delete(id);
          return next;
        });
      }, SETTLE_MS);

      void tasks.toggleTask(id);
    },
    [tasks],
  );

  /* -- Activity reporting -------------------------------------------------- */

  useEffect(() => {
    // Lets the scheduler tell a widget being used from one left open all day.
    // Throttled hard: this is a hint, and one IPC call per click would be
    // wasteful for something with a two-minute resolution.
    let last = 0;
    const report = () => {
      const now = Date.now();
      if (now - last < 20_000) return;
      last = now;
      invoke("note_activity").catch(() => {
        /* a missed hint only costs a slightly slower poll */
      });
    };

    window.addEventListener("pointerdown", report);
    window.addEventListener("keydown", report);
    return () => {
      window.removeEventListener("pointerdown", report);
      window.removeEventListener("keydown", report);
    };
  }, []);

  /* -- Keyboard ------------------------------------------------------------ */

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key.toLowerCase() === "r") {
        event.preventDefault();
        void tasks.syncNow();
      } else if (event.ctrlKey && event.key === ",") {
        event.preventDefault();
        setView((v) => (v === "settings" ? "widget" : "settings"));
      } else if (event.key === "Escape" && view === "settings") {
        setView("widget");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tasks, view]);

  /* -- Render -------------------------------------------------------------- */

  // Rust decides what closing means: the main note hides to tray, extra notes
  // really close. Keeping that decision in one place stops the two windows
  // drifting apart.
  const hide = () => invoke("close_note_window").catch(console.error);

  const openInNewNote = (listId: string) => {
    uiLog("open note: click received, invoking command");
    return invoke<string>("open_note_window", { listId })
      .then((label) => uiLog(`open note: created window "${label}"`))
      .catch((err: unknown) => {
        // Surfaced on the widget itself, not just in Settings — the user is
        // looking at the list menu when this fails, not at a settings page.
        uiLog(`open note: FAILED — ${String(err)}`, "error");
        setWidgetError(String(err));
        setTimeout(() => setWidgetError(null), 6000);
      });
  };

  /** The `+` button: another note showing whatever this one is showing. */
  const duplicateNote = () => {
    if (!tasks.selectedListId) return;
    void openInNewNote(tasks.selectedListId);
  };

  const changeColor = (color: string | null) => {
    const listId = tasks.selectedListId;
    if (!listId) return;

    // Optimistic: repainting a panel is instant and a failed write only means
    // the colour is not remembered next launch.
    setListColors((prev) => {
      const next = { ...prev };
      if (color === null) delete next[listId];
      else next[listId] = color;
      return next;
    });

    invoke("set_list_color", { listId, color }).catch((err: unknown) =>
      uiLog(`set colour failed: ${String(err)}`, "error"),
    );
  };

  const quit = () => invoke("quit_app").catch(console.error);

  if (account === null) {
    return <div className="widget" />;
  }

  if (!account.connected) {
    return (
      <div className="widget">
        <Onboarding
          onConnect={handleConnect}
          authorizing={account.authorizing}
          error={authError}
        />
      </div>
    );
  }

  const settings: Settings = {
    windowLayer,
    showInTaskbar,
    startWithWindows,
    startHidden,
    globalHotkey,
    globalHotkeyEnabled,
    theme,
    selectedTaskListId: tasks.selectedListId,
  };

  if (view === "settings") {
    return (
      <SettingsPanel
        settings={settings}
        taskLists={tasks.taskLists}
        connected={account.connected}
        lastSyncedAt={tasks.status.lastSyncedAt}
        error={settingsError}
        onChange={(patch) => {
          setSettingsError(null);

          if (patch.windowLayer !== undefined) applyLayer(patch.windowLayer);
          if (patch.theme !== undefined) setTheme(patch.theme);
          if (patch.selectedTaskListId) tasks.selectList(patch.selectedTaskListId);

          if (patch.showInTaskbar !== undefined) {
            const next = patch.showInTaskbar;
            setShowInTaskbar(next);
            invoke("set_show_in_taskbar", { value: next }).catch(
              (err: unknown) => {
                setShowInTaskbar(!next);
                setSettingsError(String(err));
              },
            );
          }

          if (
            patch.globalHotkey !== undefined ||
            patch.globalHotkeyEnabled !== undefined
          ) {
            const accelerator = patch.globalHotkey ?? globalHotkey;
            const enabled = patch.globalHotkeyEnabled ?? globalHotkeyEnabled;
            const previous = { accelerator: globalHotkey, enabled: globalHotkeyEnabled };

            setGlobalHotkey(accelerator);
            setGlobalHotkeyEnabled(enabled);

            invoke("set_global_hotkey", { accelerator, enabled }).catch(
              (err: unknown) => {
                // Rejected by Windows — keep the shortcut that still works
                // rather than leaving the user with none.
                setGlobalHotkey(previous.accelerator);
                setGlobalHotkeyEnabled(previous.enabled);
                setSettingsError(String(err));
              },
            );
          }

          if (patch.startHidden !== undefined) {
            const next = patch.startHidden;
            setStartHidden(next);
            invoke("set_start_hidden", { value: next }).catch((err: unknown) => {
              setStartHidden(!next);
              setSettingsError(String(err));
            });
          }

          if (patch.startWithWindows !== undefined) {
            const next = patch.startWithWindows;
            setStartWithWindows(next);
            invoke<boolean>("set_start_with_windows", { value: next })
              // Trust the registry's answer over our optimistic guess.
              .then(setStartWithWindows)
              .catch((err: unknown) => {
                setStartWithWindows(!next);
                setSettingsError(String(err));
              });
          }
        }}
        onDisconnect={handleDisconnect}
        onSyncNow={() => void tasks.syncNow()}
        onClose={() => setView("widget")}
      />
    );
  }

  return (
    <TaskWidget
      tasks={tasks.tasks}
      taskLists={tasks.taskLists}
      settings={settings}
      status={tasks.status}
      settlingIds={settlingIds}
      errors={tasks.errors}
      onToggle={handleToggle}
      onDelete={(id) => void tasks.deleteTask(id)}
      onEdit={(id, patch) => void tasks.editTask(id, patch)}
      onSetDue={(id, due) => void tasks.editTask(id, { due })}
      onOpenInGoogle={tasks.openInGoogle}
      onMoveToList={(id, dest) => void tasks.moveTaskToList(id, dest)}
      onMoveTo={(id, target) => void tasks.moveTaskTo(id, target)}
      onAdopt={(id, fromListId, toIndex) =>
        void tasks.adoptTask(id, fromListId, toIndex)
      }
      onDepart={tasks.departTask}
      onCreateList={tasks.createList}
      onRenameList={tasks.renameList}
      onDeleteList={tasks.deleteList}
      onAdd={tasks.addTask}
      onAddOutline={tasks.addOutline}
      onSelectList={tasks.selectList}
      onDuplicateNote={duplicateNote}
      color={tasks.selectedListId ? (listColors[tasks.selectedListId] ?? null) : null}
      onChangeColor={changeColor}
      widgetError={widgetError}
      onChangeLayer={applyLayer}
      pendingDelete={tasks.pendingDelete}
      onUndoDelete={tasks.undoDelete}
      onSyncNow={() => void tasks.syncNow()}
      onOpenSettings={() => setView("settings")}
      onHide={hide}
      onQuit={quit}
    />
  );
}
