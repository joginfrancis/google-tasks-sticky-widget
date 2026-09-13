import { describe, expect, it } from "vitest";
import { resolveDrop, type DropRow } from "./dropTarget";

/** `a`, `b` with children `b1`/`b2`, then `c` — the list as drawn. */
const rows: DropRow[] = [
  { id: "a", parentId: null },
  { id: "b", parentId: null },
  { id: "b1", parentId: "b" },
  { id: "b2", parentId: "b" },
  { id: "c", parentId: null },
];

describe("resolveDrop — top level", () => {
  it("drops above everything", () => {
    expect(resolveDrop(rows, "c", 0, false)).toEqual({
      parent: null,
      previous: null,
    });
  });

  it("drops between two top-level tasks", () => {
    expect(resolveDrop(rows, "c", 1, false)).toEqual({
      parent: null,
      previous: "a",
    });
  });

  // The row directly above is b2, a subtask — but at top level the task to
  // follow is its parent, not it.
  it("follows the parent, not its last child, when landing past children", () => {
    expect(resolveDrop(rows, "a", 4, false)).toEqual({
      parent: null,
      previous: "b",
    });
  });

  it("drops below everything", () => {
    expect(resolveDrop(rows, "a", 5, false)).toEqual({
      parent: null,
      previous: "c",
    });
  });
});

describe("resolveDrop — into a parent", () => {
  it("nests under the task above when asked", () => {
    expect(resolveDrop(rows, "c", 2, true)).toEqual({
      parent: "b",
      previous: null,
    });
  });

  it("joins the parent of the subtask it lands under", () => {
    // Landing under b1, which is itself a child of b.
    expect(resolveDrop(rows, "c", 3, false)).toEqual({
      parent: "b",
      previous: "b1",
    });
  });

  it("appends after the last existing child when asked to nest", () => {
    expect(resolveDrop(rows, "c", 4, true)).toEqual({
      parent: "b",
      previous: "b2",
    });
  });

  // The same slot without that intent is the boundary *after* b's children,
  // which is top level — this pair is the whole reason `asChild` exists.
  it("stays top level at the same slot when not nesting", () => {
    expect(resolveDrop(rows, "c", 4, false)).toEqual({
      parent: null,
      previous: "b",
    });
  });
});

describe("resolveDrop — out of a parent", () => {
  it("promotes a subtask dropped above everything", () => {
    expect(resolveDrop(rows, "b1", 0, false)).toEqual({
      parent: null,
      previous: null,
    });
  });

  // Dragged at child indentation, so the intent is to stay a child.
  it("reorders a subtask within its parent", () => {
    // b1 moving below b2: rows without b1 are a, b, b2, c.
    expect(resolveDrop(rows, "b1", 4, true)).toEqual({
      parent: "b",
      previous: "b2",
    });
  });

  // The same drag pulled left: out of b entirely, and into the top level just
  // after it.
  it("promotes a subtask dragged out to top level", () => {
    expect(resolveDrop(rows, "b1", 4, false)).toEqual({
      parent: null,
      previous: "b",
    });
  });
});

describe("resolveDrop — what the API cannot express", () => {
  it("keeps a task that has children at top level", () => {
    // b owns b1 and b2, so it cannot become a child of a: that needs two
    // levels, and Google Tasks has one.
    expect(resolveDrop(rows, "b", 1, true)).toEqual({
      parent: null,
      previous: "a",
    });
  });

  it("never nests a task under its own subtask", () => {
    const target = resolveDrop(rows, "b", 3, true);
    expect(target?.parent).not.toBe("b1");
    expect(target?.parent).toBeNull();
  });
});

describe("resolveDrop — edges", () => {
  it("returns null for a task that is not in the list", () => {
    expect(resolveDrop(rows, "nope", 0, false)).toBeNull();
  });

  it("clamps a slot past the end", () => {
    expect(resolveDrop(rows, "a", 99, false)).toEqual({
      parent: null,
      previous: "c",
    });
  });

  it("clamps a negative slot", () => {
    expect(resolveDrop(rows, "c", -5, false)).toEqual({
      parent: null,
      previous: null,
    });
  });

  it("handles a single-task list", () => {
    expect(resolveDrop([{ id: "only", parentId: null }], "only", 1, false)).toEqual(
      { parent: null, previous: null },
    );
  });

  // Moving a row onto its own position must not try to follow itself.
  it("never returns the moved task as its own previous", () => {
    for (let slot = 0; slot <= rows.length; slot += 1) {
      for (const asChild of [false, true]) {
        for (const row of rows) {
          const target = resolveDrop(rows, row.id, slot, asChild);
          expect(target?.previous).not.toBe(row.id);
          expect(target?.parent).not.toBe(row.id);
        }
      }
    }
  });
});
