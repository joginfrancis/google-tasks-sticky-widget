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

  // Whether this text marks its tasks with bullets.
  //
  // `formatTasksAsText` writes titles as bullets and a task's details as plain
  // indented lines beneath them, so in such a text an unbulleted line that is
  // indented under a bullet is detail, not a task. Text typed by hand usually
  // has no bullets at all, where an indented line *is* a subtask — so the
  // document decides, rather than a rule that would break one of the two.
  const bulleted = lines.some((line) => LIST_MARKER.test(line.trim()));

  const entries: OutlineEntry[] = [];
  // Indent of the most recent top-level line, so a subtask is recognised by
  // being deeper than its parent rather than deeper than some fixed column.
  let topLevelIndent: number | null = null;
  /** Indent of the last line that was a task, for spotting its details. */
  let lastTaskIndent: number | null = null;

  for (const line of lines) {
    const columns = indentColumns(line);
    const isBullet = LIST_MARKER.test(line.trim());
    const title = line.trim().replace(LIST_MARKER, "").trim();
    if (title === "") continue;

    // A detail line under a bulleted task: part of its description, which a
    // paste cannot carry, rather than a task in its own right.
    if (
      bulleted &&
      !isBullet &&
      lastTaskIndent !== null &&
      columns > lastTaskIndent
    ) {
      continue;
    }

    lastTaskIndent = columns;

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

/** A task with everything worth writing down about it. */
export interface DetailedTask {
  id: string;
  title: string;
  parentId: string | null;
  notes: string | null;
  /** RFC 3339, as Google gives it. */
  due?: string | null;
}

/** Month names written out, so copied text does not depend on a locale. */
const MONTHS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

/**
 * A due date as it should read in a document someone else may open next week.
 *
 * Absolute, not "Tomorrow": the relative form the rows use is right on screen
 * and wrong the moment the text is pasted somewhere and read later.
 */
function dueLine(due: string): string {
  const date = new Date(due);
  if (Number.isNaN(date.getTime())) return "";
  return `Due ${date.getUTCDate()} ${MONTHS[date.getUTCMonth()]} ${date.getUTCFullYear()}`;
}

/**
 * Writes tasks out as text, keeping the hierarchy and the details.
 *
 * ```
 * - Book the venue
 *   Deposit is due on signing
 *   Due 20 Oct 2026
 *   - Call about parking
 *     Ask for the side gate
 * ```
 *
 * Titles carry a bullet and details do not, which is the distinction
 * `parseOutline` reads: pasting this back into a note rebuilds the same tasks
 * and the same nesting. Details are dropped on the way back, because a pasted
 * line can only become a title — better that than inventing a task called
 * "Deposit is due on signing".
 *
 * A selected parent brings its subtasks. A subtask selected without its parent
 * is written at the top level rather than indented under whatever happens to
 * precede it.
 */
export function formatTasksAsText(
  tasks: DetailedTask[],
  selectedIds: Set<string>,
): string {
  const lines: string[] = [];

  for (const task of tasks) {
    const parentSelected =
      task.parentId !== null && selectedIds.has(task.parentId);
    if (!selectedIds.has(task.id) && !parentSelected) continue;

    const indent = parentSelected ? "  " : "";
    lines.push(`${indent}- ${task.title}`);

    const detail = `${indent}  `;

    // Notes keep their own line breaks: a description written as three bullet
    // points should arrive as three bullet points.
    if (task.notes) {
      for (const line of task.notes.split(/\r\n|\r|\n/)) {
        if (line.trim() === "") continue;
        lines.push(`${detail}${line.trim()}`);
      }
    }

    if (task.due) {
      const written = dueLine(task.due);
      if (written) lines.push(`${detail}${written}`);
    }
  }

  return lines.join("\n");
}
