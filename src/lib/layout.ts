/**
 * Reading the result of a layout command out loud.
 *
 * The commands used to answer with a bare count, which meant a note that did
 * not move was reported as a smaller number and nothing said why. They now
 * answer with what moved, out of how many, and a phrase per reason — and this
 * turns that into the one line the panel shows.
 */

/** What a layout command did, as the Rust side reports it. */
export type LayoutOutcome = {
  /** Notes that were given a new place. */
  moved: number;
  /** Notes the operation was asked about, moved or not. */
  total: number;
  /** One phrase per reason, ready to read: "1 note whose list is gone". */
  skipped: string[];
  /** Said instead of the count, when a count is not the answer. */
  message?: string | null;
};

/**
 * "Arranged 5 notes", or "Arranged 4 of 5 — left 1 note whose list is gone".
 *
 * The "of" only appears when something was left behind: "Arranged 5 of 5" is
 * a worse sentence than "Arranged 5 notes" and says the same thing.
 */
export function report(verb: string, out: LayoutOutcome): string {
  // "They are already on that screen" is the whole answer; a count of nothing
  // moved is not.
  if (out.message) return out.message;

  const plural = out.moved === 1 ? " note" : " notes";
  const count =
    out.moved === out.total
      ? out.moved + plural
      : out.moved + " of " + out.total + plural;

  return out.skipped.length > 0
    ? verb + " " + count + " — left " + out.skipped.join(", ")
    : verb + " " + count;
}
