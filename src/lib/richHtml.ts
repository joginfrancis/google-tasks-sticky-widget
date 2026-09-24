/**
 * Markdown in, editable HTML out — and back again.
 *
 * The description is stored as plain text with Markdown conventions, because
 * that is all Google Tasks can hold. But nobody wants to *write* `**like**`
 * `*this*`: in the editor the formatting should simply look like formatting,
 * as it does in Notion or Docs.
 *
 * So the editor is a contenteditable showing real bold, real links and real
 * pictures, and these two functions are the border between that and what gets
 * saved. `toHtml` is what the editor starts from; `toMarkdown` is what comes
 * out and goes to Google.
 *
 * Everything is escaped on the way in: a description is user text that has
 * been round-tripped through an API, and none of it is trusted as markup.
 */

import { parseBlocks, type Inline } from "./richText";

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function inlineToHtml(tokens: Inline[]): string {
  return tokens
    .map((token) => {
      switch (token.kind) {
        case "bold":
          return `<strong>${escapeHtml(token.text)}</strong>`;
        case "italic":
          return `<em>${escapeHtml(token.text)}</em>`;
        case "strike":
          return `<s>${escapeHtml(token.text)}</s>`;
        case "code":
          return `<code>${escapeHtml(token.text)}</code>`;
        case "link":
          return `<a href="${escapeHtml(token.url)}">${escapeHtml(token.text)}</a>`;
        case "image":
          return `<img src="${escapeHtml(token.url)}" alt="${escapeHtml(token.alt)}">`;
        default:
          return escapeHtml(token.text);
      }
    })
    .join("");
}

/** One `<div>` per line, which is what a contenteditable produces itself. */
export function toHtml(markdown: string): string {
  if (!markdown) return "";

  return parseBlocks(markdown)
    .map((block) => {
      if (block.kind === "blank") return "<div><br></div>";
      if (block.kind === "bullet") {
        return `<ul><li>${inlineToHtml(block.content)}</li></ul>`;
      }
      if (block.kind === "numbered") {
        return `<ol start="${block.number}"><li>${inlineToHtml(block.content)}</li></ol>`;
      }
      return `<div>${inlineToHtml(block.content)}</div>`;
    })
    .join("");
}

/**
 * Reads whatever the browser's editing left behind and writes the Markdown for
 * it. Browsers are inventive here — `<b>` or `<strong>`, `<div>` or `<br>`,
 * spans full of inline styles — so this asks what an element *means* rather
 * than matching a list of tags it hopes to see.
 */
export function toMarkdown(root: HTMLElement): string {
  const lines: string[] = [];
  let current = "";

  const flush = () => {
    lines.push(current.replace(/[ \t]+$/g, ""));
    current = "";
  };

  const walk = (node: Node, marker: string) => {
    if (node.nodeType === Node.TEXT_NODE) {
      // A newline inside the markup is layout, not content.
      current += (node.textContent ?? "").replace(/\n/g, " ");
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;

    const el = node as HTMLElement;
    const tag = el.tagName.toLowerCase();

    if (tag === "br") {
      flush();
      return;
    }

    if (tag === "img") {
      const src = el.getAttribute("src") ?? "";
      const alt = el.getAttribute("alt") ?? "";
      if (src) current += `![${alt}](${src})`;
      return;
    }

    if (tag === "a") {
      const href = el.getAttribute("href") ?? "";
      const text = el.textContent ?? "";
      if (!href) current += text;
      else if (text === href) current += href;
      else current += `[${text}](${href})`;
      return;
    }

    const style = el.style;
    const weight = style.fontWeight;
    const bold =
      tag === "b" || tag === "strong" || weight === "bold" || Number(weight) >= 600;
    const italic = tag === "i" || tag === "em" || style.fontStyle === "italic";
    const strike =
      tag === "s" ||
      tag === "strike" ||
      tag === "del" ||
      style.textDecorationLine?.includes("line-through") ||
      style.textDecoration?.includes("line-through");
    const code = tag === "code";

    // Block-level elements each start a line; everything else is inline.
    const block = ["div", "p", "li", "ul", "ol", "blockquote", "h1", "h2", "h3"].includes(
      tag,
    );

    if (block && current.trim() !== "") flush();

    const childMarker = tag === "li" ? "- " : tag === "ul" || tag === "ol" ? marker : marker;
    if (tag === "li") current += "- ";

    const open = (bold ? "**" : "") + (italic ? "*" : "") + (strike ? "~~" : "") + (code ? "`" : "");
    const close = (code ? "`" : "") + (strike ? "~~" : "") + (italic ? "*" : "") + (bold ? "**" : "");

    const before = current.length;
    current += open;
    el.childNodes.forEach((child) => walk(child, childMarker));

    // Nothing inside: drop the markers rather than leaving `****` behind.
    if (current.length === before + open.length) current = current.slice(0, before);
    else current += close;

    // Only if the element actually produced something: a `<ul>` whose items
    // have each already ended their own line would otherwise add a blank one
    // after every list. A genuinely empty line arrives as `<br>`, which flushes
    // on its own account above.
    if (block && current.trim() !== "") flush();
  };

  root.childNodes.forEach((child) => walk(child, ""));
  if (current.trim() !== "") flush();

  return lines
    .join("\n")
    // A contenteditable leaves a trail of empty divs; two blank lines in a row
    // is a deliberate gap, three is debris.
    .replace(/\n{3,}/g, "\n\n")
    .replace(/ /g, " ")
    .trim();
}
