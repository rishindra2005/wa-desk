//! Classification and escalation of transient failures.
//!
//! Short lock waits on the bridge databases surface as `Timeout` or `Busy`
//! and normally clear on the next retry. Reporting each of them as a problem
//! is noise, so callers feed their outcomes into a [`FailureTracker`] and only
//! escalate once a failure has persisted. Time is passed in, so tests drive
//! the tracker with a fake clock.

use std::time::{Duration, Instant};

use crate::ErrorKind;

/// A transient failure must last at least this long before it is escalated.
pub const ESCALATE_AFTER: Duration = Duration::from_secs(60);
/// ... and span at least this many consecutive failed attempts.
pub const ESCALATE_ATTEMPTS: u32 = 5;

/// True for kinds that mean "try again shortly" rather than a real fault.
pub fn is_transient_kind(kind: ErrorKind) -> bool {
    matches!(kind, ErrorKind::Timeout | ErrorKind::Busy)
}

/// Counts consecutive failures and says when a transient streak is persistent.
#[derive(Debug, Default, Clone)]
pub struct FailureTracker {
    first: Option<Instant>,
    attempts: u32,
    escalated: bool,
}

impl FailureTracker {
    /// New tracker with no failures.
    pub fn new() -> Self {
        Self::default()
    }

    /// Consecutive failures so far.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Whether the current streak has already been escalated.
    pub fn escalated(&self) -> bool {
        self.escalated
    }

    /// Record a failure at `now`. Returns `true` exactly once per streak: the
    /// first time it has lasted [`ESCALATE_AFTER`] over [`ESCALATE_ATTEMPTS`]
    /// attempts.
    pub fn failure(&mut self, now: Instant) -> bool {
        let first = *self.first.get_or_insert(now);
        self.attempts = self.attempts.saturating_add(1);
        if self.escalated
            || self.attempts < ESCALATE_ATTEMPTS
            || now.saturating_duration_since(first) < ESCALATE_AFTER
        {
            return false;
        }
        self.escalated = true;
        true
    }

    /// Record a success; returns the number of failures it ended and whether
    /// that streak had been escalated.
    pub fn success(&mut self) -> (u32, bool) {
        let out = (self.attempts, self.escalated);
        *self = Self::default();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert!(is_transient_kind(ErrorKind::Timeout));
        assert!(is_transient_kind(ErrorKind::Busy));
        assert!(!is_transient_kind(ErrorKind::Internal));
        assert!(!is_transient_kind(ErrorKind::BridgeDown));
    }

    #[test]
    fn needs_both_time_and_attempts() {
        let t0 = Instant::now();
        let mut t = FailureTracker::new();
        // Many attempts, too quick.
        for i in 0..10 {
            assert!(!t.failure(t0 + Duration::from_secs(i)));
        }
        // Long enough now: escalates once.
        assert!(t.failure(t0 + Duration::from_secs(60)));
        assert!(!t.failure(t0 + Duration::from_secs(90)));
        assert_eq!(t.success(), (12, true));
        assert_eq!(t.attempts(), 0);
    }

    #[test]
    fn long_but_few_attempts_does_not_escalate() {
        let t0 = Instant::now();
        let mut t = FailureTracker::new();
        assert!(!t.failure(t0));
        assert!(!t.failure(t0 + Duration::from_secs(300)));
        assert!(!t.failure(t0 + Duration::from_secs(600)));
        assert!(!t.failure(t0 + Duration::from_secs(900)));
        assert!(t.failure(t0 + Duration::from_secs(1200)));
    }
}
