//! Retry policy.
//!
//! Retry is **opt-in**. The default policy runs a task exactly once, so a
//! failure is reported immediately rather than being silently repeated.
//! Destructive operations must not be retried automatically, so the engine
//! never retries on its own — a caller has to ask for it.
//!
//! When a retry policy is set, a failed *attempt* is retried until
//! [`RetryPolicy::max_attempts`] is reached. All attempts happen within the
//! same `running` episode: the domain state machine has no
//! `failed -> queued` edge, and the engine deliberately does not invent one.
//! Cancellation and timeout are honoured between and during attempts.

use std::time::Duration;

/// How many times a task's executor may be invoked before a failure is final.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The total number of attempts, including the first. `1` means no retry.
    pub max_attempts: u32,
    /// The delay between attempts.
    pub backoff: Duration,
}

impl RetryPolicy {
    /// A policy that runs the task exactly once.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            max_attempts: 1,
            backoff: Duration::from_secs(0),
        }
    }

    /// A policy that runs the task up to `max_attempts` times.
    ///
    /// A value below `1` is clamped to `1`, so a task is always attempted at
    /// least once.
    #[must_use]
    pub const fn attempts(max_attempts: u32) -> Self {
        Self {
            max_attempts: if max_attempts < 1 { 1 } else { max_attempts },
            backoff: Duration::from_secs(0),
        }
    }

    /// Sets the delay between attempts.
    #[must_use]
    pub const fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    /// Returns `true` if a failed `attempt` number may be retried.
    ///
    /// `attempt` is one-based: the first attempt has number `1`.
    #[must_use]
    pub const fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_attempts
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_never_retries() {
        let policy = RetryPolicy::none();
        assert_eq!(policy.max_attempts, 1);
        assert!(!policy.should_retry(1));
    }

    #[test]
    fn attempts_controls_the_retry_budget() {
        let policy = RetryPolicy::attempts(3);
        assert!(policy.should_retry(1));
        assert!(policy.should_retry(2));
        assert!(!policy.should_retry(3));
    }

    #[test]
    fn attempts_are_clamped_to_at_least_one() {
        assert_eq!(RetryPolicy::attempts(0).max_attempts, 1);
    }

    #[test]
    fn backoff_is_configurable() {
        let policy = RetryPolicy::attempts(2).with_backoff(Duration::from_millis(50));
        assert_eq!(policy.backoff, Duration::from_millis(50));
    }
}
