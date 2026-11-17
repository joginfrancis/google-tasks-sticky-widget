import { describe, expect, it } from "vitest";
import { pickChecker, MAIN_LABEL } from "./window";

describe("pickChecker", () => {
  /// The bug: a restored arrangement has no `main` window, so a rule of
  /// "only `main` checks" meant nothing checked and an update sat on GitHub
  /// unoffered.
  it("picks a note when there is no main window", () => {
    const labels = ["note-3", "note-0", "note-4", "note-1", "note-2"];
    expect(pickChecker(labels)).toBe("note-0");
  });

  it("prefers main when it is open, because that is a fresh install", () => {
    expect(pickChecker(["note-2", MAIN_LABEL, "note-0"])).toBe(MAIN_LABEL);
  });

  /// Sorted as numbers. As strings "note-10" comes before "note-9", and two
  /// windows would each believe the other had the job.
  it("orders notes numerically, not alphabetically", () => {
    expect(pickChecker(["note-10", "note-9"])).toBe("note-9");
  });

  /// Every window runs this over the same list, so exactly one may agree.
  it("names the same window whatever order the list arrives in", () => {
    const labels = ["note-4", "note-1", "note-3"];
    const reversed = [...labels].reverse();
    expect(pickChecker(labels)).toBe(pickChecker(reversed));
  });

  /// A task opened in its own window is not a note and must not take the job,
  /// since it closes again as soon as the task is done.
  it("ignores page windows", () => {
    expect(pickChecker(["page-abc", "note-2"])).toBe("note-2");
    expect(pickChecker(["page-abc"])).toBeNull();
  });

  it("returns null when there is nothing to choose", () => {
    expect(pickChecker([])).toBeNull();
  });
});
