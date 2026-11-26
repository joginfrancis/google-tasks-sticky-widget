/**
 * When to look for a new version again.
 *
 * 0.1.6 looked exactly once, twenty seconds after launch, and threw the result
 * away if it failed. 0.1.8 then sat on GitHub for nine days, offered to nobody
 * on this desk, and the log says almost nothing about why: one failed check on
 * 8 November, and silence either side of it, because a success and a failure
 * wrote the same nothing. The timing lives here, apart from the hook, so it can
 * be read and tested without a webview.
 */

/** The first look. Late enough that the arrangement has finished settling. */
export const FIRST_CHECK_MS = 20_000;

/** After a failed look. Doubles per consecutive failure, up to the ceiling. */
export const FIRST_RETRY_MS = 60_000;
export const MAX_RETRY_MS = 30 * 60_000;

/** After a look that found nothing. The widget stays up for days. */
export const RECHECK_MS = 6 * 60 * 60_000;

/** For a note that is not the chosen checker, waiting to inherit the job. */
export const STANDBY_MS = 15 * 60_000;

export type CheckOutcome =
  /** The check threw: no network, no release, a GitHub hiccup. */
  | "failed"
  /** The check answered, and this is the newest version. */
  | "current"
  /** Another note is doing the checking. */
  | "standby";

/**
 * How long to wait before the next look.
 *
 * `consecutiveFailures` counts the failures since the last clean answer, so a
 * first failure retries in a minute and a run of them backs off to half an
 * hour rather than hammering an endpoint that is plainly not well.
 */
export function nextCheckMs(outcome: CheckOutcome, consecutiveFailures: number): number {
  switch (outcome) {
    case "current":
      return RECHECK_MS;
    case "standby":
      return STANDBY_MS;
    case "failed": {
      const doublings = Math.max(0, consecutiveFailures - 1);
      return Math.min(FIRST_RETRY_MS * 2 ** doublings, MAX_RETRY_MS);
    }
  }
}
