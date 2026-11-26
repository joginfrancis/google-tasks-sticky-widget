import { describe, expect, it } from "vitest";
import {
  FIRST_RETRY_MS,
  MAX_RETRY_MS,
  RECHECK_MS,
  STANDBY_MS,
  nextCheckMs,
} from "./updateSchedule";

describe("nextCheckMs", () => {
  /// The bug this exists for: 0.1.6 asked once per launch and swallowed the
  /// answer, so one bad moment cost a whole session.
  it("retries a minute after the first failure", () => {
    expect(nextCheckMs("failed", 1)).toBe(FIRST_RETRY_MS);
  });

  it("doubles the wait for each failure in a row", () => {
    expect(nextCheckMs("failed", 2)).toBe(FIRST_RETRY_MS * 2);
    expect(nextCheckMs("failed", 3)).toBe(FIRST_RETRY_MS * 4);
    expect(nextCheckMs("failed", 4)).toBe(FIRST_RETRY_MS * 8);
  });

  /// An endpoint that has been down all morning is not helped by being asked
  /// every minute until evening.
  it("stops doubling at half an hour", () => {
    expect(nextCheckMs("failed", 10)).toBe(MAX_RETRY_MS);
    expect(nextCheckMs("failed", 500)).toBe(MAX_RETRY_MS);
  });

  /// Defensive: a caller that has not counted yet must not get a zero delay
  /// and spin.
  it("treats an uncounted failure as the first one", () => {
    expect(nextCheckMs("failed", 0)).toBe(FIRST_RETRY_MS);
    expect(nextCheckMs("failed", -1)).toBe(FIRST_RETRY_MS);
  });

  /// The widget is left running for days, so "no update today" is not a
  /// final answer.
  it("asks again hours later when the version is current", () => {
    expect(nextCheckMs("current", 0)).toBe(RECHECK_MS);
  });

  /// A note that is not the checker keeps ticking so it can inherit the job
  /// when the chosen note is closed.
  it("keeps an unchosen note on a short standby", () => {
    expect(nextCheckMs("standby", 0)).toBe(STANDBY_MS);
    expect(STANDBY_MS).toBeLessThan(RECHECK_MS);
  });
});
