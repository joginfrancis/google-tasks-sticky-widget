import { describe, expect, it } from "vitest";
import { applySelectClick, emptySelection, pruneSelection } from "./selection";

const order = ["a", "b", "c", "d", "e"];
const toggle = { toggle: true, range: false };
const range = { toggle: false, range: true };
const addRange = { toggle: true, range: true };
const ids = (s: { ids: Set<string> }) => [...s.ids].sort();

describe("applySelectClick", () => {
  it("toggles a row on and off with Ctrl+click", () => {
    const on = applySelectClick(emptySelection, order, "b", toggle);
    expect(ids(on)).toEqual(["b"]);
    const off = applySelectClick(on, order, "b", toggle);
    expect(ids(off)).toEqual([]);
  });

  it("adds to the selection with further Ctrl+clicks", () => {
    let s = applySelectClick(emptySelection, order, "a", toggle);
    s = applySelectClick(s, order, "d", toggle);
    expect(ids(s)).toEqual(["a", "d"]);
  });

  it("selects the whole run from the anchor with Shift+click", () => {
    let s = applySelectClick(emptySelection, order, "b", toggle);
    s = applySelectClick(s, order, "d", range);
    expect(ids(s)).toEqual(["b", "c", "d"]);
  });

  it("selects a run upwards as well as downwards", () => {
    let s = applySelectClick(emptySelection, order, "d", toggle);
    s = applySelectClick(s, order, "b", range);
    expect(ids(s)).toEqual(["b", "c", "d"]);
  });

  // As in a file manager: Shift re-measures from the anchor, it does not grow.
  it("replaces the previous range rather than adding to it", () => {
    let s = applySelectClick(emptySelection, order, "c", toggle);
    s = applySelectClick(s, order, "e", range);
    s = applySelectClick(s, order, "a", range);
    expect(ids(s)).toEqual(["a", "b", "c"]);
  });

  it("adds a range to the selection when Ctrl is held too", () => {
    let s = applySelectClick(emptySelection, order, "a", toggle);
    s = applySelectClick(s, order, "c", toggle);
    s = applySelectClick(s, order, "e", addRange);
    expect(ids(s)).toEqual(["a", "c", "d", "e"]);
  });

  it("starts fresh when Shift+clicking with nothing selected", () => {
    const s = applySelectClick(emptySelection, order, "c", range);
    expect(ids(s)).toEqual(["c"]);
    expect(s.anchor).toBe("c");
  });

  it("starts fresh when the anchor is no longer shown", () => {
    const stale = { ids: new Set(["x"]), anchor: "x" };
    const s = applySelectClick(stale, order, "c", range);
    expect(ids(s)).toEqual(["c"]);
  });
});

describe("pruneSelection", () => {
  it("drops rows that are no longer shown", () => {
    const s = pruneSelection({ ids: new Set(["a", "z"]), anchor: "a" }, order);
    expect(ids(s)).toEqual(["a"]);
    expect(s.anchor).toBe("a");
  });

  it("clears an anchor that is no longer shown", () => {
    const s = pruneSelection({ ids: new Set(["a"]), anchor: "z" }, order);
    expect(s.anchor).toBeNull();
  });

  // Returning the same object lets React skip a re-render on every sync.
  it("returns the same selection when nothing changed", () => {
    const before = { ids: new Set(["a", "b"]), anchor: "b" };
    expect(pruneSelection(before, order)).toBe(before);
  });
});
