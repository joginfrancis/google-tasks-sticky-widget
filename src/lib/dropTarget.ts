/**
 * Turning a drop position into what the move API actually wants.
 *
 * Google Tasks positions a task by `parent` and `previous` — the task it should
 * follow *within that parent* — never by an index. Every ordering bug in this
 * project has come from two places counting the same list differently, and
 * subtasks add another way to count, so the arithmetic lives here, pure and
 * tested, rather than inside a pointer handler.
 *
 * Rows are taken exactly as the list draws them: a flat sequence of parents each
 * followed by their children, which is what the user is pointing at.
 */

export interface DropRow {
  id: string;
  /** null for a top-level task. */
  parentId: string | null;
}

export interface DropTarget {
  /** The parent to move into, or null for top level. */
  parent: string | null;
  /** The task to follow within that parent, or null to go first. */
  previous: string | null;
}

/**
 * Resolves a drop to a `parent` and `previous`.
 *
 * `rows` is the visible order *including* the task being moved — that is what
 * the list draws and what the pointer was measured against. It is removed here,
 * so callers never have to hold two frames in mind.
 *
 * `slot` is an insertion point in that same visible sequence: 0 is above
 * everything, `rows.length` below everything.
 *
 * `asChild` is the caller's reading of intent, normally from how far right the
 * pointer is. It only decides genuinely ambiguous drops — see below.
 */
export function resolveDrop(
  rows: DropRow[],
  taskId: string,
  slot: number,
  asChild: boolean,
): DropTarget | null {
  const moving = rows.find((r) => r.id === taskId);
  if (!moving) return null;

  // Its own children travel with it, so they cannot be its new neighbours.
  const others = rows.filter((r) => r.id !== taskId && r.parentId !== taskId);

  // The slot counts rows that are about to leave the sequence, so convert it
  // into the frame `others` is in.
  const clamped = Math.max(0, Math.min(slot, rows.length));
  const removedBefore = rows
    .slice(0, clamped)
    .filter((r) => r.id === taskId || r.parentId === taskId).length;
  const index = Math.max(0, Math.min(clamped - removedBefore, others.length));

  const before = index > 0 ? others[index - 1] : null;
  const after = index < others.length ? others[index] : null;

  // Above everything is top level; there is nothing above to nest under.
  if (!before) return { parent: null, previous: null };

  // Asked of `rows`, not `others`: the children were filtered out of `others`
  // precisely because they belong to this task, so asking there is always false.
  const hasChildren = rows.some((r) => r.parentId === taskId);

  const parent = chooseParent({ before, after, asChild, hasChildren });

  // `previous` is the last task *at the destination level* at or above the
  // drop, which is often not the row directly above: dropping at top level just
  // past a parent's children must follow the parent, not its last child.
  let previous: string | null = null;
  for (let i = index - 1; i >= 0; i -= 1) {
    if ((others[i].parentId ?? null) === parent) {
      previous = others[i].id;
      break;
    }
  }

  return { parent, previous };
}

function chooseParent(args: {
  before: DropRow;
  after: DropRow | null;
  asChild: boolean;
  hasChildren: boolean;
}): string | null {
  const { before, after, asChild, hasChildren } = args;

  // A task that owns children must stay top level. Nesting it would need two
  // levels of hierarchy and the API has one, so the drop is honoured as far as
  // it can be rather than refused.
  if (hasChildren) return null;

  // Strictly inside one parent's run of children — the rows above and below
  // both belong to it. There is nothing ambiguous about this drop, so intent
  // does not get a say: landing between two of b's children means joining b.
  const beforeParent = before.parentId;
  if (beforeParent !== null && after?.parentId === beforeParent) {
    return beforeParent;
  }

  // Everywhere else is a boundary — after a parent, after its last child, at
  // the end of the list — and genuinely ambiguous. Intent decides.
  if (!asChild) return null;

  // Nest under the row above, or alongside it if that row is itself a subtask.
  return beforeParent !== null ? beforeParent : before.id;
}
