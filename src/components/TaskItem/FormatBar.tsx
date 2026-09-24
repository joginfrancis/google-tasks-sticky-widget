import { useRef, useState } from "react";

/**
 * The formatting controls above a description being edited.
 *
 * They act on the live editor through the browser's own editing commands, so
 * what happens is what the user sees: bold text goes bold, a link becomes a
 * link. The Markdown that ends up in Google is written when the edit is saved,
 * not typed by hand here.
 *
 * Every button uses `onMouseDown` with `preventDefault`, so pressing one never
 * takes focus from the editor. Losing focus would collapse the selection the
 * button is about to act on, and blur would save and close the editor.
 */
interface Props {
  /** The contenteditable being formatted. */
  editor: HTMLDivElement | null;
  /** Something changed, so the note may need to grow. */
  onChanged: () => void;
}

export function FormatBar({ editor, onChanged }: Props) {
  /** Open with "link" or "image" while an address is being typed. */
  const [asking, setAsking] = useState<null | "link" | "image">(null);
  const [url, setUrl] = useState("");
  /**
   * Where the caret was before the address field took focus.
   *
   * Typing into that field moves the selection out of the editor, and the
   * browser's editing commands act on the selection — so without putting it
   * back, a link would be applied to nothing at all.
   */
  const savedRange = useRef<Range | null>(null);
  /** Where the field floats: beside the selection it is about to act on. */
  const [anchor, setAnchor] = useState<{ top: number; left: number } | null>(null);

  const remember = () => {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0) return;
    const range = selection.getRangeAt(0);
    if (editor?.contains(range.commonAncestorContainer)) {
      savedRange.current = range.cloneRange();
    }
  };

  const restore = () => {
    const range = savedRange.current;
    editor?.focus();
    if (!range) return;
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  };

  const ask = (what: "link" | "image") => {
    remember();

    // Beside the words being linked, not up in the toolbar: the address and
    // the text it belongs to should be in the same glance, as they are in
    // Docs or Notion.
    const rect = savedRange.current?.getBoundingClientRect();
    if (rect && (rect.width || rect.height)) {
      // Clamped to the note: a selection near the right edge would otherwise
      // push the field off the window it is supposed to be helping with.
      const width = 232;
      const edge = 8;
      setAnchor({
        top: Math.min(rect.bottom + 6, window.innerHeight - 44),
        left: Math.max(edge, Math.min(rect.left, window.innerWidth - width - edge)),
      });
    } else {
      setAnchor(null);
    }
    setAsking(what);
  };

  const run = (command: string, value?: string) => {
    editor?.focus();
    document.execCommand(command, false, value);
    onChanged();
  };

  const submitUrl = () => {
    const address = url.trim();
    setUrl("");
    const what = asking;
    setAsking(null);
    setAnchor(null);
    if (!address || !editor || !what) return;

    // A bare "example.com" is a link the user meant; without a scheme nothing
    // downstream will treat it as one.
    const full = /^https?:\/\//i.test(address) ? address : `https://${address}`;

    // Built by hand rather than with execCommand("createLink"), which quietly
    // does nothing when the selection has been restored rather than made by
    // the user — which is exactly this case, every time.
    const range = savedRange.current;
    editor.focus();
    if (!range) return;

    const node =
      what === "image" ? document.createElement("img") : document.createElement("a");

    if (what === "image") {
      (node as HTMLImageElement).src = full;
    } else {
      (node as HTMLAnchorElement).href = full;
      if (range.collapsed) {
        // Nothing selected: the address becomes its own link, rather than an
        // empty one nobody can see or click.
        node.textContent = full;
      } else {
        node.appendChild(range.extractContents());
      }
    }

    range.insertNode(node);

    // Caret after whatever was just inserted, ready to keep typing.
    const after = document.createRange();
    after.setStartAfter(node);
    after.collapse(true);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(after);
    savedRange.current = after.cloneRange();

    onChanged();
  };

  return (
    <div className="format-bar" onMouseDown={(e) => e.preventDefault()}>
      <Button label="Bold" hint="Bold — Ctrl+B" onClick={() => run("bold")}>
        <span className="fb-bold">B</span>
      </Button>
      <Button label="Italic" hint="Italic — Ctrl+I" onClick={() => run("italic")}>
        <span className="fb-italic">I</span>
      </Button>
      <Button
        label="Strikethrough"
        hint="Strikethrough"
        onClick={() => run("strikeThrough")}
      >
        <span className="fb-strike">ab</span>
      </Button>
      <Button label="Bulleted list" hint="Bulleted list" onClick={() => run("insertUnorderedList")}>
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
        onClick={() => ask("link")}
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
      <Button
        label="Add image"
        hint="Picture, by its web address"
        onClick={() => ask("image")}
      >
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
        <div
          className={`fb-url${anchor ? " is-floating" : ""}`}
          style={anchor ? { top: anchor.top, left: anchor.left } : undefined}
        >
          <input
            className="fb-url-input"
            autoFocus
            value={url}
            placeholder={
              asking === "image" ? "Direct image address…" : "Link address…"
            }
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
                restore();
              }
            }}
          />
          <button
            className="fb-url-ok"
            onClick={submitUrl}
            aria-label={asking === "image" ? "Insert image" : "Insert link"}
            title="Insert — or press Enter"
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path
                d="M3.5 8.5l3 3 6-6.5"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
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
