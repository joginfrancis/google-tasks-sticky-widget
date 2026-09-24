import { invoke } from "@tauri-apps/api/core";

/**
 * Making the note big enough for what just opened inside it.
 *
 * A note is small on purpose, which means anything that expands — a long
 * description, a menu, a date picker — can run past the bottom of the window.
 * Clipping it would be the worst answer: the thing exists, it is just
 * invisible, and there is no scrollbar on a popup to hint otherwise.
 *
 * Several things can want the window big at once (a menu opened over an
 * already-expanded task), so growth is reference counted here rather than each
 * caller shrinking the window out from under the others. The window goes back
 * to its own size when the last of them lets go.
 */

let holders = 0;

async function release() {
  holders = Math.max(0, holders - 1);
  if (holders > 0) return;
  document.body.classList.remove("is-tall");
  await invoke("note_set_tall", { tall: false }).catch(() => {});
}

/** Full screen height, for a description that needs all of it. */
export function growFull(): () => void {
  holders += 1;
  void invoke("note_set_tall", { tall: true })
    .then(() => document.body.classList.add("is-tall"))
    .catch(() => {});
  return () => void release();
}

/**
 * Just enough for `element` to fit, or nothing at all if it already does.
 *
 * Returns the function that gives the height back. Calling it when nothing was
 * taken is harmless, which keeps the caller free of "did it grow?" bookkeeping.
 */
export function growToFit(element: HTMLElement, margin = 10): () => void {
  const rect = element.getBoundingClientRect();
  const overflow = rect.bottom + margin - window.innerHeight;
  if (overflow <= 0) return () => {};

  holders += 1;
  // CSS pixels are not screen pixels on a scaled display, and the window API
  // speaks in screen pixels.
  const extra = Math.ceil(overflow * window.devicePixelRatio);
  void invoke("note_grow", { extra }).catch(() => {});
  return () => void release();
}

/**
 * Enough room for something `heightCss` tall to sit inside the window, when
 * the thing is positioned by hand rather than laid out — the date picker
 * clamps itself to the window, so it reports no overflow to measure.
 */
export function growForHeight(heightCss: number, margin = 10): () => void {
  const missing = heightCss + margin * 2 - window.innerHeight;
  if (missing <= 0) return () => {};

  holders += 1;
  const extra = Math.ceil(missing * window.devicePixelRatio);
  void invoke("note_grow", { extra }).catch(() => {});
  return () => void release();
}
