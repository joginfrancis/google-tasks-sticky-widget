/**
 * Multi-select rules, kept apart from the component that uses them.
 *
 * Ctrl+click and Shift+click follow the same conventions as a file manager, so
 * they should feel familiar rather than be learned. The range rule is where the
 * subtle cases live — what the anchor is after a toggle, what happens when the
 * anchor has gone — and those are easier to get right, and keep right, as a
 * pure function than inside a click handler.
 */

export interface Selection {
  ids: Set<string>;
  /** Where a Shift+click range starts: the last row Ctrl- or plain-selected. */
  anchor: string | null;
}

export const emptySelection: Selection = { ids: new Set(), anchor: null };

/**
 * Applies one modified click to a selection.
 *
 * `order` is the rows as they are shown, which is what a range is measured in.
 *
 * - Ctrl+click toggles one row and makes it the anchor.
 * - Shift+click selects every row from the anchor to the one clicked,
 *   replacing the current selection — as a file manager does. With Ctrl held
 *   as well, the range is added to what is already selected.
 * - Shift+click with no usable anchor selects just that row and anchors there.
 */
export function applySelectClick(
  selection: Selection,
  order: string[],
  id: string,
  mode: { toggle: boolean; range: boolean },
): Selection {
  if (mode.range) {
    const from = selection.anchor ? order.indexOf(selection.anchor) : -1;
    const to = order.indexOf(id);
    if (to === -1) return selection;
    // An anchor that has since disappeared — completed, deleted, moved away —
    // cannot start a range, so the click starts a fresh one here.
    if (from === -1) return { ids: new Set([id]), anchor: id };

    const [lo, hi] = from <= to ? [from, to] : [to, from];
    const range = order.slice(lo, hi + 1);
    const ids = mode.toggle ? new Set([...selection.ids, ...range]) : new Set(range);
    // The anchor stays put, so a second Shift+click re-measures from the same
    // start rather than from wherever the last range ended.
    return { ids, anchor: selection.anchor };
  }

  const ids = new Set(selection.ids);
  if (ids.has(id)) ids.delete(id);
  else ids.add(id);
  return { ids, anchor: id };
}

/** Drops rows that are no longer shown, keeping the anchor only if it still is. */
export function pruneSelection(selection: Selection, order: string[]): Selection {
  const visible = new Set(order);
  const ids = new Set([...selection.ids].filter((id) => visible.has(id)));
  if (ids.size === selection.ids.size && (!selection.anchor || visible.has(selection.anchor))) {
    return selection;
  }
  return {
    ids,
    anchor: selection.anchor && visible.has(selection.anchor) ? selection.anchor : null,
  };
}
