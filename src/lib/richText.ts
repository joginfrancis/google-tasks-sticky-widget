/**
 * The little formatting a plain-text description can carry.
 *
 * Google Tasks stores descriptions as plain text (docs/ui-parity.md §1), so
 * nothing here is real formatting — it is Markdown's own conventions, written
 * by hand and rendered on the way out. The characters are what travels: the
 * phone and the web app show `**done**`, this widget shows it in bold, and
 * neither is lying about what is stored.
 *
 * Deliberately a small subset. Everything here is something people already
 * type into notes without being asked — asterisks for emphasis, dashes for
 * lists, a pasted URL — so an existing description formats itself.
 */

export type Inline =
  | { kind: "text"; text: string }
  | { kind: "bold"; text: string }
  | { kind: "italic"; text: string }
  | { kind: "code"; text: string }
  | { kind: "link"; text: string; url: string }
  | { kind: "image"; url: string; alt: string };

export type Block =
  | { kind: "paragraph"; content: Inline[] }
  | { kind: "bullet"; content: Inline[] }
  | { kind: "numbered"; number: number; content: Inline[] }
  | { kind: "blank" };

/** `- item`, `* item` or `• item`. */
const BULLET = /^\s*[-*•]\s+(.*)$/;
/** `1. item`, `2) item`. */
const NUMBERED = /^\s*(\d{1,3})[.)]\s+(.*)$/;

/** Splits a description into lines that know what kind of line they are. */
export function parseBlocks(text: string): Block[] {
  return text.split("\n").map((line) => {
    if (line.trim() === "") return { kind: "blank" };

    const numbered = NUMBERED.exec(line);
    if (numbered) {
      return {
        kind: "numbered",
        number: Number(numbered[1]),
        content: parseInline(numbered[2]),
      };
    }

    const bullet = BULLET.exec(line);
    if (bullet) return { kind: "bullet", content: parseInline(bullet[1]) };

    return { kind: "paragraph", content: parseInline(line) };
  });
}

/**
 * Order matters and is the whole design here.
 *
 * Links are found before emphasis, so an underscore or asterisk inside a URL
 * is part of the address rather than a formatting mark — plenty of real links
 * contain both. `**` is found before `*` for the same reason.
 */
const PATTERNS: {
  re: RegExp;
  build: (m: RegExpExecArray) => Inline;
}[] = [
  // ![alt](url) — a picture. Bare image URLs are caught further down.
  {
    re: /!\[([^\]]*)\]\((https?:\/\/[^\s)]+)\)/g,
    build: (m) => ({ kind: "image", url: m[2], alt: m[1] }),
  },
  // [text](url) — a link wearing a name, as pasting over a selection makes.
  {
    re: /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g,
    build: (m) => ({ kind: "link", text: m[1], url: m[2] }),
  },
  // A bare URL. Trailing punctuation is sentence punctuation, not address.
  {
    re: /https?:\/\/[^\s<>"']+[^\s<>"'.,;:!?)\]]/g,
    build: (m) => {
      const url = m[0];
      return isImageUrl(url)
        ? { kind: "image", url, alt: "" }
        : { kind: "link", text: url, url };
    },
  },
  { re: /`([^`\n]+)`/g, build: (m) => ({ kind: "code", text: m[1] }) },
  { re: /\*\*([^*\n]+)\*\*/g, build: (m) => ({ kind: "bold", text: m[1] }) },
  { re: /__([^_\n]+)__/g, build: (m) => ({ kind: "bold", text: m[1] }) },
  { re: /\*([^*\n]+)\*/g, build: (m) => ({ kind: "italic", text: m[1] }) },
  // `_word_` only between spaces: snake_case_names are not italics.
  {
    re: /(?<![A-Za-z0-9])_([^_\n]+)_(?![A-Za-z0-9])/g,
    build: (m) => ({ kind: "italic", text: m[1] }),
  },
];

export function parseInline(line: string): Inline[] {
  if (line === "") return [];

  // Earliest match wins; ties go to the pattern listed first, which is what
  // makes the ordering above meaningful.
  let best: { start: number; end: number; token: Inline } | null = null;
  for (const { re, build } of PATTERNS) {
    re.lastIndex = 0;
    const m = re.exec(line);
    if (!m) continue;
    if (!best || m.index < best.start) {
      best = { start: m.index, end: m.index + m[0].length, token: build(m) };
    }
  }

  if (!best) return [{ kind: "text", text: line }];

  const out: Inline[] = [];
  if (best.start > 0) out.push({ kind: "text", text: line.slice(0, best.start) });
  out.push(best.token);
  out.push(...parseInline(line.slice(best.end)));
  return out;
}

/** Whether a URL is worth showing as a picture rather than as a link. */
export function isImageUrl(url: string): boolean {
  const withoutQuery = url.split(/[?#]/)[0];
  return /\.(png|jpe?g|gif|webp|bmp|avif|svg)$/i.test(withoutQuery);
}

/** True if the whole string is one link and nothing else — what a paste is. */
export function isUrl(text: string): boolean {
  return /^https?:\/\/\S+$/.test(text.trim());
}

/**
 * What pasting a URL should leave behind.
 *
 * Over selected text it becomes a named link, as in Notion: the words stay
 * readable and the address goes out of the way. With nothing selected the URL
 * is simply inserted — wrapping it in brackets there would only add noise to
 * something already readable.
 */
export function pasteLink(
  value: string,
  start: number,
  end: number,
  url: string,
): { text: string; caret: number } {
  const selected = value.slice(start, end);
  const insert = selected ? `[${selected}](${url})` : url;
  return {
    text: value.slice(0, start) + insert + value.slice(end),
    caret: start + insert.length,
  };
}

/**
 * Wraps or unwraps a selection in a marker — what Ctrl+B and Ctrl+I do.
 *
 * Pressing it again on text that already carries the marker takes it off, so
 * the shortcut is a toggle rather than a one-way trip into `****bold****`.
 */
export function toggleMarker(
  value: string,
  start: number,
  end: number,
  marker: string,
): { text: string; start: number; end: number } {
  const selected = value.slice(start, end);

  if (
    selected.startsWith(marker) &&
    selected.endsWith(marker) &&
    selected.length > marker.length * 2
  ) {
    const inner = selected.slice(marker.length, selected.length - marker.length);
    return { text: value.slice(0, start) + inner + value.slice(end), start, end: start + inner.length };
  }

  const before = value.slice(Math.max(0, start - marker.length), start);
  const after = value.slice(end, end + marker.length);
  if (before === marker && after === marker) {
    return {
      text:
        value.slice(0, start - marker.length) + selected + value.slice(end + marker.length),
      start: start - marker.length,
      end: end - marker.length,
    };
  }

  const wrapped = `${marker}${selected}${marker}`;
  return {
    text: value.slice(0, start) + wrapped + value.slice(end),
    start: start + marker.length,
    end: end + marker.length,
  };
}
