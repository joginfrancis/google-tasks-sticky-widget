import { useEffect, useRef } from "react";
import { toHtml, toMarkdown } from "../../lib/richHtml";
import { isImageUrl, isUrl } from "../../lib/richText";

/**
 * The description editor: what you see is what the note shows.
 *
 * Formatting appears as formatting while you type — bold is bold, a link is a
 * link, a picture is a picture — rather than as the `**markers**` that are
 * actually stored. Those markers are still what reaches Google, written on the
 * way out by `toMarkdown`, so the phone and the web app see plain text and
 * this widget sees a formatted note. Neither is being lied to.
 *
 * Uncontrolled on purpose. A contenteditable driven by React state fights the
 * caret on every keystroke: re-rendering the HTML puts the cursor back at the
 * start. The DOM owns the text while the editor is open, and it is read once
 * when the edit finishes.
 */
interface Props {
  /** Markdown to start from. */
  value: string;
  /** Character offset to put the caret at, or null to leave it at the end. */
  caret: number | null;
  onCommit: (markdown: string) => void;
  onCancel: () => void;
  /** Tab out of the editor — the date button is the next stop. */
  onTabOut: () => void;
  /** Shift+Tab, which walks back to the title. */
  onTabBack: () => void;
  /** The content changed shape, so the note may need to grow. */
  onInput: () => void;
  editorRef: React.RefObject<HTMLDivElement | null>;
}

export function NotesEditor({
  value,
  caret,
  onCommit,
  onCancel,
  onTabOut,
  onTabBack,
  onInput,
  editorRef,
}: Props) {
  /** Set while cancelling, so the blur that follows does not save it anyway. */
  const cancelled = useRef(false);

  useEffect(() => {
    const host = editorRef.current;
    if (!host) return;

    host.innerHTML = toHtml(value);
    host.focus();
    placeCaret(host, caret);
    // Only ever on open: re-running this would wipe out what is being typed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const commit = () => {
    const host = editorRef.current;
    if (!host || cancelled.current) return;
    onCommit(toMarkdown(host));
  };

  return (
    <div
      ref={editorRef}
      className="task-edit is-notes is-rich"
      contentEditable
      suppressContentEditableWarning
      role="textbox"
      aria-multiline="true"
      aria-label="Description"
      data-placeholder="Add details…"
      onInput={onInput}
      onBlur={(event) => {
        // Focus moving into the toolbar is not the user leaving the editor —
        // it is them about to format what they just selected. Typing a link
        // address into the bar's field was closing the editor underneath it,
        // which is why the link and image buttons appeared to do nothing.
        const next = event.relatedTarget as HTMLElement | null;
        if (next?.closest(".format-bar")) return;
        commit();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          cancelled.current = true;
          onCancel();
          return;
        }

        if (event.key === "Tab") {
          event.preventDefault();
          commit();
          if (event.shiftKey) onTabBack();
          else onTabOut();
          return;
        }

        // Enter is a line break here, because a description is prose.
        // Ctrl+Enter is what finishes it, as it was with the plain editor.
        if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
          event.preventDefault();
          commit();
          return;
        }

        if ((event.ctrlKey || event.metaKey) && !event.altKey) {
          const key = event.key.toLowerCase();
          const command =
            key === "b" ? "bold" : key === "i" ? "italic" : key === "u" ? null : null;
          if (command) {
            event.preventDefault();
            document.execCommand(command);
            onInput();
          }
        }
      }}
      onPaste={(event) => {
        // Never paste other applications' HTML: a description that arrives
        // with a stylesheet attached is not a description any more.
        event.preventDefault();
        const text = event.clipboardData.getData("text/plain");
        if (!text) return;

        const selection = window.getSelection();
        const hasSelection = selection ? !selection.isCollapsed : false;

        if (isUrl(text) && isImageUrl(text) && !hasSelection) {
          // A pasted picture address becomes the picture, not a line of text
          // about one.
          document.execCommand("insertImage", false, text.trim());
        } else if (isUrl(text) && hasSelection) {
          // Pasting a link over words names the link with those words.
          document.execCommand("createLink", false, text.trim());
        } else if (isUrl(text)) {
          document.execCommand("createLink", false, text.trim());
          // execCommand with nothing selected does nothing, so write the
          // address in first and link that.
          if (!(window.getSelection()?.anchorNode as HTMLElement | null)?.parentElement?.closest("a")) {
            document.execCommand("insertHTML", false, anchorHtml(text.trim()));
          }
        } else {
          document.execCommand("insertText", false, text);
        }
        onInput();
      }}
      onClick={(event) => {
        // A link in an editor is text being edited, not something to follow;
        // Ctrl+click opens it, as everywhere else.
        const anchor = (event.target as HTMLElement).closest("a");
        if (anchor && (event.ctrlKey || event.metaKey)) {
          event.preventDefault();
          const href = anchor.getAttribute("href");
          if (href) void openLink(href);
        }
      }}
    />
  );
}

/** The address as its own link. Escaped: user input going into markup. */
function anchorHtml(url: string): string {
  const safe = url
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
  return `<a href="${safe}">${safe}</a>`;
}

async function openLink(url: string) {
  const { invoke } = await import("@tauri-apps/api/core");
  void invoke("open_link", { url }).catch(() => {});
}

/**
 * Puts the caret at a character offset, counting the text as the reader sees
 * it rather than as it is stored — which is what a click on the rendered
 * description gives us.
 */
function placeCaret(host: HTMLElement, offset: number | null) {
  const selection = window.getSelection();
  if (!selection) return;

  const range = document.createRange();

  if (offset === null) {
    range.selectNodeContents(host);
    range.collapse(false);
  } else {
    let remaining = offset;
    let target: Node = host;
    let position = 0;

    const walker = document.createTreeWalker(host, NodeFilter.SHOW_TEXT);
    let node = walker.nextNode();
    while (node) {
      const length = node.textContent?.length ?? 0;
      if (remaining <= length) {
        target = node;
        position = remaining;
        break;
      }
      remaining -= length;
      node = walker.nextNode();
      if (!node) {
        target = host;
        position = host.childNodes.length;
      }
    }

    try {
      range.setStart(target, position);
      range.collapse(true);
    } catch {
      range.selectNodeContents(host);
      range.collapse(false);
    }
  }

  selection.removeAllRanges();
  selection.addRange(range);
}
