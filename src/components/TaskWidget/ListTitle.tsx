import { useEffect, useRef, useState } from "react";
import type { TaskList } from "../../types";

interface Props {
  lists: TaskList[];
  selectedId: string;
  onSelect: (id: string) => void;
  onCreate: (title: string) => Promise<string | null>;
  onRename: (id: string, title: string) => Promise<string | null>;
}

/**
 * The note's heading: current list name, a chevron for switching, and
 * double-click to rename in place.
 *
 * Renaming lives here rather than in a menu because the title *is* the list
 * name — editing the thing you can see is more discoverable than finding a
 * "Rename" item three levels into a menu.
 */
export function ListTitle({ lists, selectedId, onSelect, onCreate, onRename }: Props) {
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState(false);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const current = lists.find((l) => l.id === selectedId);
  const title = current?.title ?? "My Tasks";

  useEffect(() => {
    if (editing || creating) inputRef.current?.select();
  }, [editing, creating]);

  const beginEdit = () => {
    setDraft(title);
    setError(null);
    setEditing(true);
    setOpen(false);
  };

  const beginCreate = () => {
    setDraft("");
    setError(null);
    setCreating(true);
    setOpen(false);
  };

  const commit = async () => {
    const value = draft.trim();
    if (!value || (editing && value === title)) {
      setEditing(false);
      setCreating(false);
      return;
    }

    const failure = creating
      ? await onCreate(value)
      : await onRename(selectedId, value);

    if (failure) {
      setError(failure);
    } else {
      setEditing(false);
      setCreating(false);
    }
  };

  if (editing || creating) {
    return (
      <div className="list-title-edit">
        <input
          ref={inputRef}
          value={draft}
          placeholder={creating ? "New list name" : "List name"}
          maxLength={1024}
          onChange={(e) => {
            setDraft(e.target.value);
            if (error) setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void commit();
            } else if (e.key === "Escape") {
              e.preventDefault();
              setEditing(false);
              setCreating(false);
            }
          }}
          // Committing on blur would fight the Escape key; require an explicit
          // Enter, and treat clicking away as "leave it alone".
          onBlur={() => {
            if (!error) {
              setEditing(false);
              setCreating(false);
            }
          }}
        />
        {error && <span className="list-title-error">{error}</span>}
      </div>
    );
  }

  return (
    <div className="list-title">
      <button
        className="list-title-button"
        onClick={() => setOpen((v) => !v)}
        onDoubleClick={beginEdit}
        title="Switch list — double-click to rename"
        aria-expanded={open}
      >
        <span className="list-title-text">{title}</span>
        <span className={`list-title-chevron ${open ? "is-open" : ""}`}>▾</span>
      </button>

      {open && (
        <>
          <div className="menu-scrim" onClick={() => setOpen(false)} />
          <div className="list-dropdown" role="menu">
            {lists.map((list) => (
              <button
                key={list.id}
                className="list-dropdown-item"
                role="menuitemradio"
                aria-checked={list.id === selectedId}
                onClick={() => {
                  setOpen(false);
                  onSelect(list.id);
                }}
              >
                <span className="check-slot">
                  {list.id === selectedId ? "✓" : ""}
                </span>
                <span className="list-dropdown-name">{list.title}</span>
              </button>
            ))}

            <div className="dropdown-sep" />

            <button className="list-dropdown-item" onClick={beginCreate}>
              <span className="check-slot">+</span>
              <span className="list-dropdown-name">New list…</span>
            </button>
          </div>
        </>
      )}
    </div>
  );
}
