import { useEffect, useRef, useState } from "react";
import "./TaskInput.css";

interface Props {
  /** Resolves to an error message on failure, or null on success. */
  onSubmit: (title: string) => Promise<string | null>;
}

/**
 * Quick-add. Collapsed to a hint until clicked; Enter commits, Escape cancels.
 *
 * SPEC §3.3: a failed add must never silently discard what was typed. On
 * failure the text stays in the field rather than vanishing.
 */
export function TaskInput({ onSubmit }: Props) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  // Ctrl+A from anywhere in the widget focuses this field.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.key.toLowerCase() === "a" && !open) {
        const target = event.target as HTMLElement;
        if (target.tagName !== "INPUT") {
          event.preventDefault();
          setOpen(true);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  const commit = async () => {
    const title = value.trim();
    if (!title || busy) {
      if (!title) {
        setOpen(false);
        setValue("");
        setError(null);
      }
      return;
    }

    setBusy(true);
    // Clear optimistically so a fast success feels instant; the text comes
    // straight back if the write fails.
    setValue("");
    const failure = await onSubmit(title);
    setBusy(false);

    if (failure) {
      setValue(title);
      setError(failure);
    } else {
      setError(null);
    }
    // Stay open: adding one task usually means adding another.
    inputRef.current?.focus();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
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
      <div className="add-field">
        <span className="add-plus">+</span>
        <input
          ref={inputRef}
          value={value}
          placeholder="What needs doing?"
          disabled={busy}
          onChange={(e) => {
            setValue(e.target.value);
            if (error) setError(null);
          }}
          onKeyDown={handleKeyDown}
          onBlur={() => {
            if (!value.trim() && !error) setOpen(false);
          }}
        />
      </div>
    </div>
  );
}
