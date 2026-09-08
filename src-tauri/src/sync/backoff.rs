//! Exponential backoff with jitter.
//!
//! ARCHITECTURE §5.4: without this, an expired grant or a flapping connection
//! becomes a hot retry loop that burns quota and can turn a blip into a
//! rate-limited outage longer than the original problem.

use std::time::Duration;

const CEILING: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub fn record_success(&mut self) {
        self.failures = 0;
    }

    pub fn record_failure(&mut self) {
        // Saturate rather than wrap: the delay is already at the ceiling long
        // before this matters, but an overflow here would reset it to zero.
        self.failures = self.failures.saturating_add(1);
    }

    pub fn is_backing_off(&self) -> bool {
        self.failures > 0
    }

    /// Doubles per failure from `base`, capped, then jittered.
    ///
    /// A server-supplied `Retry-After` wins outright — it is a real instruction,
    /// not a guess.
    pub fn delay(&self, base: Duration, retry_after: Option<u64>) -> Duration {
        if let Some(secs) = retry_after {
            return Duration::from_secs(secs).min(CEILING);
        }

        if self.failures == 0 {
            return base;
        }

        let multiplier = 1u64 << self.failures.min(10);
        let scaled = base
            .checked_mul(multiplier as u32)
            .unwrap_or(CEILING)
            .min(CEILING);

        jitter(scaled, self.failures)
    }
}

/// Up to ±12.5%, so several clients that failed together do not all retry on
/// the same tick. Derived from the failure count rather than a RNG — this needs
/// spread, not unpredictability, and a deterministic version is testable.
fn jitter(delay: Duration, seed: u32) -> Duration {
    let millis = delay.as_millis() as u64;
    let spread = millis / 8;
    if spread == 0 {
        return delay;
    }

    let offset = (u64::from(seed).wrapping_mul(2_654_435_761)) % (spread * 2);
    Duration::from_millis(millis.saturating_sub(spread).saturating_add(offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Duration = Duration::from_secs(30);

    #[test]
    fn first_attempt_uses_the_base_interval_exactly() {
        let backoff = Backoff::default();
        assert_eq!(backoff.delay(BASE, None), BASE);
        assert!(!backoff.is_backing_off());
    }

    #[test]
    fn delay_grows_with_consecutive_failures() {
        let mut backoff = Backoff::default();
        let mut previous = backoff.delay(BASE, None);

        for _ in 0..5 {
            backoff.record_failure();
            let next = backoff.delay(BASE, None);
            assert!(next > previous, "delay must grow: {previous:?} -> {next:?}");
            previous = next;
        }
    }

    #[test]
    fn delay_never_exceeds_the_ceiling() {
        let mut backoff = Backoff::default();
        for _ in 0..100 {
            backoff.record_failure();
        }
        assert!(backoff.delay(BASE, None) <= CEILING);
        // And a huge base must not overflow past it either.
        assert!(backoff.delay(Duration::from_secs(3600), None) <= CEILING);
    }

    #[test]
    fn success_resets_the_delay() {
        let mut backoff = Backoff::default();
        for _ in 0..4 {
            backoff.record_failure();
        }
        assert!(backoff.is_backing_off());

        backoff.record_success();
        assert!(!backoff.is_backing_off());
        assert_eq!(backoff.delay(BASE, None), BASE);
    }

    #[test]
    fn retry_after_overrides_and_is_still_capped() {
        let mut backoff = Backoff::default();
        backoff.record_failure();
        assert_eq!(backoff.delay(BASE, Some(45)), Duration::from_secs(45));
        assert_eq!(backoff.delay(BASE, Some(99_999)), CEILING);
    }

    #[test]
    fn jitter_stays_within_an_eighth_either_way() {
        let delay = Duration::from_secs(60);
        for seed in 1..50 {
            let jittered = jitter(delay, seed);
            assert!(jittered >= Duration::from_millis(52_500), "{jittered:?}");
            assert!(jittered <= Duration::from_millis(67_500), "{jittered:?}");
        }
    }
}
