/**
 * Turning pasted text into tasks.
 *
 * Pasting a list from a notepad or a document should produce that list, with
 * indented lines becoming subtasks of the line above them. Kept as a pure
 * function because the alternative — deciding structure inside a paste handler
 * — is exactly the kind of index arithmetic that has been wrong three times in
 * this codebase already, and this way it can be tested without a UI.
 */

/** One task to create, in the order it must be created. */
export interface OutlineEntry {
  title: string;
  /** 0 for a top-level task, 1 for a subtask of the entry above it. */
  depth: 0 | 1;
}

/**
 * Leading whitespace is measured in columns, with a tab counting as this many.
 *
 * Four matches how editors render a tab by default; the exact number barely
 * matters because only the *ordering* of indents is used, never the absolute
 * width.
 */
const TAB_COLUMNS = 4;

/** Bullet and numbering marks to strip; a pasted list should not keep them. */
const LIST_MARKER = /^(?:[-*+•‣▪]|\d+[.)]|[a-z][.)])\s+/i;

function indentColumns(line: string): number {
  let columns = 0;
  for (const ch of line) {
    if (ch === " ") columns += 1;
    else if (ch === "\t") columns += TAB_COLUMNS;
    else break;
  }
  return columns;
}

/**
 * Parses pasted text into a flat, ordered list of tasks to create.
 *
 * Google Tasks is one level deep — a subtask cannot itself have subtasks — so
 * anything indented beyond the first level is flattened onto it rather than
 * dropped. Losing lines silently would be worse than nesting them less deeply
 * than they were written.
 *
 * A line more indented than the one before it is a subtask of the nearest
 * top-level line above. Absolute widths are never compared against a fixed
 * size, so two spaces, four spaces or a tab all work as long as the text is
 * self-consistent.
 */
export function parseOutline(text: string): OutlineEntry[] {
  const lines = text
    .split(/\r\n|\r|\n/)
    // Blank lines are separators in prose and mean nothing as tasks.
    .filter((line) => line.trim() !== "");

  const entries: OutlineEntry[] = [];
  // Indent of the most recent top-level line, so a subtask is recognised by
  // being deeper than its parent rather than deeper than some fixed column.
  let topLevelIndent: number | null = null;

  for (const line of lines) {
    const columns = indentColumns(line);
    const title = line.trim().replace(LIST_MARKER, "").trim();
    if (title === "") continue;

    // The first line sets the baseline: text pasted from an already-indented
    // block should not become one long chain of subtasks.
    if (topLevelIndent === null || columns <= topLevelIndent) {
      topLevelIndent = columns;
      entries.push({ title, depth: 0 });
      continue;
    }

    // Deeper than the current top level — a subtask. With no top-level task
    // yet it has nothing to hang from, so it becomes one itself.
    entries.push({ title, depth: entries.length === 0 ? 0 : 1 });
  }

  return entries;
}

/**
 * Whether pasted text should be treated as an outline at all.
 *
 * A single line is an ordinary paste into the field being typed in, and
 * hijacking that would make pasting a task title impossible.
 */
export function isOutlinePaste(text: string): boolean {
  return parseOutline(text).length > 1;
}
