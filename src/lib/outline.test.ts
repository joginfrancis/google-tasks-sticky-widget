import { describe, expect, it } from "vitest";
import { isOutlinePaste, parseOutline } from "./outline";

describe("parseOutline", () => {
  it("makes one task per line", () => {
    expect(parseOutline("milk\neggs\nbread")).toEqual([
      { title: "milk", depth: 0 },
      { title: "eggs", depth: 0 },
      { title: "bread", depth: 0 },
    ]);
  });

  it("makes an indented line a subtask of the line above", () => {
    expect(parseOutline("shopping\n  milk\n  eggs\nbank")).toEqual([
      { title: "shopping", depth: 0 },
      { title: "milk", depth: 1 },
      { title: "eggs", depth: 1 },
      { title: "bank", depth: 0 },
    ]);
  });

  it("treats tabs and spaces alike", () => {
    expect(parseOutline("a\n\tb")).toEqual(parseOutline("a\n    b"));
  });

  it("accepts any consistent indent width", () => {
    const twoSpace = parseOutline("a\n  b\nc");
    const eightSpace = parseOutline("a\n        b\nc");
    expect(twoSpace).toEqual(eightSpace);
  });

  // Google Tasks is one level deep; a subtask cannot have subtasks.
  it("flattens deeper nesting rather than dropping it", () => {
    expect(parseOutline("a\n  b\n    c\n      d")).toEqual([
      { title: "a", depth: 0 },
      { title: "b", depth: 1 },
      { title: "c", depth: 1 },
      { title: "d", depth: 1 },
    ]);
  });

  it("ignores blank lines", () => {
    expect(parseOutline("a\n\n\nb")).toEqual([
      { title: "a", depth: 0 },
      { title: "b", depth: 0 },
    ]);
  });

  it("handles CRLF, which is what Notepad pastes", () => {
    expect(parseOutline("a\r\n  b")).toEqual([
      { title: "a", depth: 0 },
      { title: "b", depth: 1 },
    ]);
  });

  it("strips bullets and numbering", () => {
    expect(parseOutline("- a\n* b\n1. c\n2) d\n• e")).toEqual([
      { title: "a", depth: 0 },
      { title: "b", depth: 0 },
      { title: "c", depth: 0 },
      { title: "d", depth: 0 },
      { title: "e", depth: 0 },
    ]);
  });

  it("strips bullets without losing the indent that follows them", () => {
    expect(parseOutline("- a\n  - b")).toEqual([
      { title: "a", depth: 0 },
      { title: "b", depth: 1 },
    ]);
  });

  // Text copied out of an already-indented block would otherwise become one
  // long chain of subtasks under its first line.
  it("takes the first line's indent as the baseline", () => {
    expect(parseOutline("    a\n    b\n    c")).toEqual([
      { title: "a", depth: 0 },
      { title: "b", depth: 0 },
      { title: "c", depth: 0 },
    ]);
  });

  it("returns to top level when the indent does", () => {
    expect(parseOutline("a\n  b\nc\n  d")).toEqual([
      { title: "a", depth: 0 },
      { title: "b", depth: 1 },
      { title: "c", depth: 0 },
      { title: "d", depth: 1 },
    ]);
  });

  it("never starts with a subtask, which would have no parent", () => {
    const entries = parseOutline("  a\n    b");
    expect(entries[0].depth).toBe(0);
  });

  it("drops lines that are only a bullet", () => {
    expect(parseOutline("a\n-\nb")).toEqual([
      { title: "a", depth: 0 },
      { title: "-", depth: 0 },
      { title: "b", depth: 0 },
    ]);
  });

  it("is empty for empty or whitespace-only text", () => {
    expect(parseOutline("")).toEqual([]);
    expect(parseOutline("   \n\t\n  ")).toEqual([]);
  });
});

describe("isOutlinePaste", () => {
  it("is false for a single line, which is an ordinary paste", () => {
    expect(isOutlinePaste("just a task")).toBe(false);
    expect(isOutlinePaste("trailing newline\n")).toBe(false);
  });

  it("is true once there is more than one task in it", () => {
    expect(isOutlinePaste("a\nb")).toBe(true);
  });
});
