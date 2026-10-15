import { describe, it, expect } from "vitest";

import { report } from "./layout";

describe("report", () => {
  it("says the plain count when everything moved", () => {
    expect(report("Arranged", { moved: 5, total: 5, skipped: [] })).toBe(
      "Arranged 5 notes",
    );
    expect(report("Aligned", { moved: 1, total: 1, skipped: [] })).toBe(
      "Aligned 1 note",
    );
  });

  it("names what was left behind", () => {
    expect(
      report("Arranged", {
        moved: 4,
        total: 5,
        skipped: ["1 note whose list is gone"],
      }),
    ).toBe("Arranged 4 of 5 notes — left 1 note whose list is gone");
  });

  it("reads every reason, not just the first", () => {
    expect(
      report("Moved", {
        moved: 2,
        total: 5,
        skipped: ["1 note whose list is gone", "2 notes with no screen"],
      }),
    ).toBe(
      "Moved 2 of 5 notes — left 1 note whose list is gone, 2 notes with no screen",
    );
  });
});
