import { useState } from "react";
import { insertLink, toggleBullet, toggleMarker } from "../../lib/richText";

/**
 * The formatting controls above a description being edited.
 *
 * The markers this writes are the text itself — Google stores plain text, so
 * `**bold**` is what is really saved and what the phone shows. The bar exists
 * because that convention is invisible until someone tells you: everything
 * here is something you could type by hand, and the bar is how you find out
 * that you can.
 *
 * Every button uses `onMouseDown` with `preventDefault`, so pressing one never
 * takes focus from the textarea. Losing focus would collapse the selection the
 * button is about to act on, and blur would save and close the editor.
 */
interface Props {
  /** The textarea being formatted. */
  field: HTMLTextAreaElement | null;
  /** Hands the new text back, with the selection to restore afterwards. */
  onChange: (text: string, selection: { start: number; end: number }) => void;
}

export function FormatBar({ field, onChange }: Props) {
  /** Open with "link" or "image" while a URL is being typed. */
  const [asking, setAsking] = useState<null | "link" | "image">(null);
  const [url, setUrl] = useState("");

  const marker = (mark: string) => {
    if (!field) return;
    const next = toggleMarker(field.value, field.selectionStart, field.selectionEnd, mark);
    onChange(next.text, { start: next.start, end: next.end });
  };

  const bullets = () => {
    if (!field) return;
    const next = toggleBullet(field.value, field.selectionStart, field.selectionEnd);
    onChange(next.text, { start: next.start, end: next.end });
  };

  const submitUrl = () => {
    const address = url.trim();
    if (!field || !address) {
      setAsking(null);
      setUrl("");
      return;
    }
    // A bare "example.com" is a link the user meant; without a scheme nothing
    // downstream will treat it as one.
    const full = /^https?:\/\//i.test(address) ? address : `https://${address}`;
    const next = insertLink(
      field.value,
      field.selectionStart,
      field.selectionEnd,
      full,
      asking === "image",
    );
    onChange(next.text, { start: next.caret, end: next.caret });
    setAsking(null);
    setUrl("");
  };

  return (
    <div className="format-bar" onMouseDown={(e) => e.preventDefault()}>
      <Button label="Bold" hint="Bold — Ctrl+B" onClick={() => marker("**")}>
        <span className="fb-bold">B</span>
      </Button>
      <Button label="Italic" hint="Italic — Ctrl+I" onClick={() => marker("*")}>
        <span className="fb-italic">I</span>
      </Button>
      <Button label="Strikethrough" hint="Strikethrough" onClick={() => marker("~~")}>
        <span className="fb-strike">ab</span>
      </Button>
      <Button label="Bulleted list" hint="Bulleted list" onClick={bullets}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <circle cx="3" cy="4" r="1.2" fill="currentColor" />
          <circle cx="3" cy="8" r="1.2" fill="currentColor" />
          <circle cx="3" cy="12" r="1.2" fill="currentColor" />
          <path
            d="M6.5 4h7M6.5 8h7M6.5 12h7"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      </Button>
      <Button
        label="Add link"
        hint="Link — or paste one over selected words"
        onClick={() => setAsking("link")}
      >
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path
            d="M6.5 9.5a2.8 2.8 0 0 0 4 0l2-2a2.8 2.8 0 1 0-4-4l-.8.8M9.5 6.5a2.8 2.8 0 0 0-4 0l-2 2a2.8 2.8 0 1 0 4 4l.8-.8"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      </Button>
      <Button label="Add image" hint="Picture, by its web address" onClick={() => setAsking("image")}>
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <rect
            x="2"
            y="3"
            width="12"
            height="10"
            rx="1.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
          />
          <circle cx="5.8" cy="6.5" r="1.1" fill="currentColor" />
          <path
            d="M3 11.5l3.2-3 2.4 2.2 2-1.7L13 11.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinejoin="round"
          />
        </svg>
      </Button>

      {asking && (
        <div className="fb-url">
          <input
            className="fb-url-input"
            autoFocus
            value={url}
            placeholder={asking === "image" ? "Image address…" : "Link address…"}
            onMouseDown={(e) => e.stopPropagation()}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                submitUrl();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                setAsking(null);
                setUrl("");
              }
            }}
          />
          <button className="fb-url-ok" onClick={submitUrl}>
            Add
          </button>
        </div>
      )}
    </div>
  );
}

function Button({
  label,
  hint,
  onClick,
  children,
}: {
  label: string;
  hint: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      className="format-button"
      aria-label={label}
      title={hint}
      onClick={onClick}
    >
      {children}
    </button>
  );
}
