import { useState } from "react";
import type { TaskList } from "../../types";
import "./HeaderMenu.css";

interface Props {
  taskLists: TaskList[];
  selectedListId: string;
  onOpenSettings: () => void;
  onOpenHelp: () => void;
  onQuit: () => void;
  onDeleteList: (id: string) => Promise<string | null>;
}

/**
 * What is left of the overflow menu, and deliberately very little.
 *
 * Removed because each duplicated a control already on screen:
 *   - "Sync now"      — the sync icon is that button
 *   - "Note colour"   — the colour wheel sits at the bottom of every note
 *   - "Hide to tray"  — exactly what ✕ does on the main note
 *
 * "Quit" stays even though ✕ exists: ✕ hides the main note or closes an extra
 * one, neither of which exits the app, and the tray icon is in Windows' hidden
 * overflow by default.
 */
export function HeaderMenu(props: Props) {
  const [open, setOpen] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const currentList = props.taskLists.find((l) => l.id === props.selectedListId);

  const close = () => {
    setOpen(false);
    setConfirmingDelete(false);
    setError(null);
  };

  const run = (fn: () => void) => () => {
    close();
    fn();
  };

  const confirmDelete = async () => {
    if (busy) return;
    setBusy(true);
    const failure = await props.onDeleteList(props.selectedListId);
    setBusy(false);
    if (failure) setError(failure);
    else close();
  };

  return (
    <div className="header-menu">
      <button
        className="icon-button"
        aria-label="Menu"
        title="Menu"
        onClick={() => setOpen((v) => !v)}
      >
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <circle cx="3" cy="8" r="1.4" fill="currentColor" />
          <circle cx="8" cy="8" r="1.4" fill="currentColor" />
          <circle cx="13" cy="8" r="1.4" fill="currentColor" />
        </svg>
      </button>

      {open && (
        <>
          <div className="menu-scrim" onClick={close} />
          <div className="dropdown" role="menu">
            <button className="dropdown-item" onClick={run(props.onOpenSettings)}>
              <span className="check-slot" />
              Settings
            </button>
            {/* Here as well as inside Settings. Help exists to make hidden
                gestures findable, so it cannot itself sit at the bottom of a
                scrolling page nobody opens to look for it. */}
            <button className="dropdown-item" onClick={run(props.onOpenHelp)}>
              <span className="check-slot" />
              Help &amp; shortcuts
            </button>

            <div className="dropdown-sep" />

            <button
              className="dropdown-item is-danger"
              onClick={() => {
                setConfirmingDelete(true);
                setOpen(false);
              }}
              // Google keeps a default list around; so do we, and the backend
              // enforces it too rather than trusting the UI.
              disabled={props.taskLists.length <= 1}
            >
              <span className="check-slot" />
              Delete this list…
            </button>

            <div className="dropdown-sep" />

            <button className="dropdown-item" onClick={run(props.onQuit)}>
              <span className="check-slot" />
              Quit
            </button>
          </div>
        </>
      )}

      {confirmingDelete && (
        <>
          <div className="menu-scrim" onClick={close} />
          <div className="list-prompt" role="dialog">
            <p className="list-prompt-title">Delete “{currentList?.title}”?</p>
            <p className="list-prompt-warning">
              Every task in it is deleted too, in Google as well as here. This
              can't be undone.
            </p>

            {error && <p className="list-prompt-error">{error}</p>}

            <div className="list-prompt-actions">
              <button className="list-prompt-button" onClick={close}>
                Cancel
              </button>
              <button
                className="list-prompt-button is-primary is-danger"
                disabled={busy}
                onClick={() => void confirmDelete()}
              >
                Delete
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
