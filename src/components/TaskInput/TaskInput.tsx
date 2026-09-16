import { useEffect, useRef, useState } from "react";
import { isOutlinePaste, parseOutline, type OutlineEntry } from "../../lib/outline";
import "./TaskInput.css";

interface Props {
  /** Resolves to an error message on failure, or null on success. */
  onSubmit: (title: string, notes?: string) => Promise<string | null>;
  /** Creates several tasks at once, from pasted text. */
  onSubmitOutline: (entries: OutlineEntry[]) => Promise<string | null>;
}

/**
 * Splits what was typed into a task.
 *
 * Enter commits and Shift+Enter adds a line, so anything past the first line is
 * description rather than title — a title is a label, and a two-line one reads
 * badly everywhere it is shown.
 */
function splitEntry(text: string): { title: string; notes: string } {
  const [first, ...rest] = text.split("\n");
  return { title: first.trim(), notes: rest.join("\n").trim() };
}

/**
 * Quick-add. Collapsed to a hint until clicked.
 *
 * Enter commits and leaves the field open and focused, because adding one task
 * almost always means adding another — typing out a list should never need a
 * reach for the mouse between entries.
 *
 * SPEC §3.3: a failed add must never silently discard what was typed. On
 * failure the text stays in the field rather than vanishing.
 */
export function TaskInput({ onSubmit, onSubmitOutline }: Props) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** How many tasks a paste in progress is creating; 0 when none is. */
  const [pasting, setPasting] = useState(0);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  // Grows with its content, so a Shift+Enter line is visible rather than
  // scrolled out of sight in a one-line box.
  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [value, open]);

  // Ctrl+A from anywhere in the widget focuses this field.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key.toLowerCase() === "a" && !open) {
        const target = event.target as HTMLElement;
        if (target.tagName !== "INPUT" && target.tagName !== "TEXTAREA") {
          event.preventDefault();
          setOpen(true);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  const commit = async () => {
    const { title, notes } = splitEntry(value);
    // An empty Enter keeps the field open and waiting. Pressing Enter again
    // after adding a task means "ready for the next one", not "done" — closing
    // here turned typing a list into type, Enter, click back in, repeat.
    // Escape is how the field closes.
    if (!title || busy) return;

    setBusy(true);
    // Clear optimistically so a fast success feels instant; the text comes
    // straight back, in full, if the write fails.
    const typed = value;
    setValue("");
    const failure = await onSubmit(title, notes || undefined);
    setBusy(false);

    if (failure) {
      setValue(typed);
      setError(failure);
    } else {
      setError(null);
    }
    // Stay open: adding one task usually means adding another.
    inputRef.current?.focus();
  };

  /**
   * Multi-line paste becomes multiple tasks, indentation becoming subtasks.
   *
   * Only when there is more than one line in it: a single-line paste is someone
   * pasting a task's title, and hijacking that would make it impossible to
   * paste text into the field at all.
   *
   * Typed newlines still mean a description (Shift+Enter), which is the
   * opposite reading of the same character — deliberate, because a pasted list
   * is a list and a typed line break is an elaboration on what is being typed.
   */
  const handlePaste = (event: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const text = event.clipboardData.getData("text/plain");
    if (!text || !isOutlinePaste(text) || busy) return;

    event.preventDefault();
    const entries = parseOutline(text);

    setBusy(true);
    setPasting(entries.length);
    setError(null);
    void onSubmitOutline(entries).then((failure) => {
      setBusy(false);
      setPasting(0);
      if (failure) setError(failure);
      else setValue("");
      inputRef.current?.focus();
    });
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Enter commits and moves on to the next task; Shift+Enter breaks the line
    // inside this one, and everything after the break becomes its description.
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void commit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      setValue("");
      setError(null);
      setOpen(false);
      inputRef.current?.blur();
    }
  };

  if (!open) {
    return (
      <button className="add-trigger" onClick={() => setOpen(true)}>
        <span className="add-plus">+</span> Add task…
      </button>
    );
  }

  return (
    <div className="add-wrap">
      {error && <p className="add-error">{error}</p>}
      {pasting > 0 && (
        <p className="add-progress" role="status">
          <span className="add-spinner" aria-hidden="true" />
          Adding {pasting} task{pasting === 1 ? "" : "s"}…
        </p>
      )}
      <div className="add-field">
        <span className="add-plus">+</span>
        <textarea
          ref={inputRef}
          className="add-input"
          rows={1}
          value={value}
          placeholder="What needs doing?"
          // Not disabled while saving, deliberately. Disabling a focused
          // field throws focus out of it, which fired onBlur with the field
          // just cleared — and closed the whole add box after every Enter.
          aria-busy={busy}
          onChange={(e) => {
            setValue(e.target.value);
            if (error) setError(null);
          }}
          onKeyDown={handleKeyDown}
          onPaste={handlePaste}
          onBlur={() => {
            if (!value.trim() && !error) setOpen(false);
          }}
        />
      </div>
    </div>
  );
}
