//! Choosing the polling interval.
//!
//! The Tasks API has no push channel (docs/api-findings.md §6), so polling is
//! the only option. The cadence is not chosen for quota reasons — 30 s polling
//! of one list is roughly 6% of the 50,000/day courtesy limit — but because
//! Google's own clients take seconds to propagate anyway, and a network
//! wake-up every few seconds is a real idle-power cost on a laptop.
//!
//! ARCHITECTURE §5.3.

use std::time::{Duration, Instant};

pub const ACTIVE: Duration = Duration::from_secs(30);
pub const IDLE: Duration = Duration::from_secs(60);
pub const HIDDEN: Duration = Duration::from_secs(300);

/// How long after an interaction the window still counts as "active".
const ACTIVITY_WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    Active,
    Idle,
    Hidden,
}

impl Cadence {
    pub fn interval(self) -> Duration {
        match self {
            Cadence::Active => ACTIVE,
            Cadence::Idle => IDLE,
            Cadence::Hidden => HIDDEN,
        }
    }
}

pub fn cadence_for(visible: bool, last_activity: Option<Instant>, now: Instant) -> Cadence {
    if !visible {
        return Cadence::Hidden;
    }

    match last_activity {
        Some(at) if now.duration_since(at) < ACTIVITY_WINDOW => Cadence::Active,
        _ => Cadence::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_window_polls_slowly_regardless_of_recent_activity() {
        let now = Instant::now();
        assert_eq!(cadence_for(false, Some(now), now), Cadence::Hidden);
        assert_eq!(cadence_for(false, None, now), Cadence::Hidden);
    }

    #[test]
    fn recent_interaction_means_active() {
        let now = Instant::now();
        let recent = now - Duration::from_secs(10);
        assert_eq!(cadence_for(true, Some(recent), now), Cadence::Active);
    }

    #[test]
    fn activity_decays_to_idle_after_the_window() {
        let now = Instant::now();
        let stale = now - (ACTIVITY_WINDOW + Duration::from_secs(1));
        assert_eq!(cadence_for(true, Some(stale), now), Cadence::Idle);
    }

    #[test]
    fn a_visible_window_with_no_interaction_is_idle_not_active() {
        let now = Instant::now();
        assert_eq!(cadence_for(true, None, now), Cadence::Idle);
    }

    #[test]
    fn intervals_are_ordered_and_within_budget() {
        assert!(Cadence::Active.interval() < Cadence::Idle.interval());
        assert!(Cadence::Idle.interval() < Cadence::Hidden.interval());

        // Nothing faster than 30s — see the module comment for why.
        assert!(Cadence::Active.interval() >= Duration::from_secs(30));

        // Sanity-check the quota claim: one request per tick, one list.
        let per_day = 86_400 / Cadence::Active.interval().as_secs();
        assert!(per_day < 5_000, "{per_day}/day is more than intended");
    }
}
