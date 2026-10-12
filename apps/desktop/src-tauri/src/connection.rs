//! Daemon connection state machine.
//!
//! The tracker is the single source of truth for the GUI's connection state
//! (`tasks/018.md` §9): every probe result and every command-side failure is
//! recorded here, and transitions are pushed to the frontend as
//! `daemon://connection` events. The frontend never re-derives state on its
//! own, which is what keeps the TUI, MCP, and GUI agreeing on daemon health.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use service_orchestrator_application::ApplicationError;
use service_orchestrator_client::{codes, DaemonStatus};

/// How the adapter's connection to the daemon is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// The first connection attempt has not completed yet.
    Connecting,
    /// The daemon answered and requests are succeeding.
    Connected,
    /// The daemon went away; no automatic restart is currently in flight.
    Disconnected,
    /// The daemon is unreachable and retries are being made.
    Reconnecting,
    /// The connection cannot proceed without intervention (protocol
    /// mismatch or an unexpected failure).
    Error,
}

/// A snapshot of the connection state pushed to the frontend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConnectionSnapshot {
    /// The current state.
    pub state: ConnectionState,
    /// A user-facing explanation, when there is one.
    pub message: Option<String>,
    /// Unix time (seconds) the current state was entered.
    pub since_unix: u64,
}

/// What a status probe found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The daemon answered.
    Connected,
    /// The socket exists but is not reachable (daemon stopped or busy).
    Unreachable(String),
    /// The daemon speaks an incompatible protocol version.
    Incompatible(String),
    /// The probe failed for another reason.
    Failed(String),
}

/// How many consecutive unreachable probes keep the state at
/// [`ConnectionState::Reconnecting`] before falling back to
/// [`ConnectionState::Disconnected`].
///
/// The streak makes "the daemon stopped" read as a short reconnection window
/// first, then as a stable disconnected banner, without the frontend counting
/// anything itself (`tasks/018.md` §9).
pub const RECONNECT_ATTEMPTS: u32 = 5;

/// Classifies a `daemon_status` probe result.
#[must_use]
pub fn classify_probe(result: Result<DaemonStatus, ApplicationError>) -> ProbeOutcome {
    match result {
        Ok(_) => ProbeOutcome::Connected,
        Err(ApplicationError::RuntimeUnavailable { code, message })
            if code == codes::DAEMON_UNAVAILABLE =>
        {
            ProbeOutcome::Unreachable(message)
        }
        Err(ApplicationError::RuntimeUnavailable { code, message })
            if code == codes::PROTOCOL_INCOMPATIBLE =>
        {
            ProbeOutcome::Incompatible(message)
        }
        Err(ApplicationError::RuntimeUnavailable { code, message }) => {
            ProbeOutcome::Failed(format!("[{code}] {message}"))
        }
        Err(other) => ProbeOutcome::Failed(other.to_string()),
    }
}

/// The tracked state behind a [`ConnectionSnapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tracked {
    state: ConnectionState,
    message: Option<String>,
    since_unix: u64,
    failures: u32,
}

/// Thread-safe connection tracker that reports state transitions.
///
/// One instance lives in the shared application state; probes, command
/// failures, and reconnection attempts all record into it, and only actual
/// transitions are emitted to the frontend.
pub struct ConnectionTracker {
    inner: Mutex<Tracked>,
}

impl ConnectionTracker {
    /// Creates a tracker starting in [`ConnectionState::Connecting`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Tracked {
                state: ConnectionState::Connecting,
                message: None,
                since_unix: now_unix(),
                failures: 0,
            }),
        }
    }

    /// Returns the current snapshot.
    #[must_use]
    pub fn snapshot(&self) -> ConnectionSnapshot {
        let inner = self.inner.lock().expect("tracker lock poisoned");
        ConnectionSnapshot {
            state: inner.state,
            message: inner.message.clone(),
            since_unix: inner.since_unix,
        }
    }

    /// Records a probe or command outcome.
    ///
    /// Returns `true` when the observable state changed, so the caller emits
    /// exactly one event per transition.
    pub fn record(&self, outcome: &ProbeOutcome) -> bool {
        let mut inner = self.inner.lock().expect("tracker lock poisoned");
        let (state, message) = match outcome {
            ProbeOutcome::Connected => {
                inner.failures = 0;
                (ConnectionState::Connected, None)
            }
            ProbeOutcome::Unreachable(reason) => {
                inner.failures = inner.failures.saturating_add(1);
                let state = if inner.failures >= RECONNECT_ATTEMPTS {
                    ConnectionState::Disconnected
                } else {
                    ConnectionState::Reconnecting
                };
                (state, Some(reason.clone()))
            }
            ProbeOutcome::Incompatible(reason) => (ConnectionState::Error, Some(reason.clone())),
            ProbeOutcome::Failed(reason) => (ConnectionState::Error, Some(reason.clone())),
        };
        if inner.state == state && inner.message == message {
            return false;
        }
        inner.state = state;
        inner.message = message;
        inner.since_unix = now_unix();
        true
    }
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Current unix time in seconds, clamped to zero before the epoch.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_client::DaemonStatus;

    fn daemon_status() -> DaemonStatus {
        DaemonStatus {
            pid: 4242,
            socket_path: "/tmp/daemon.sock".into(),
            protocol_version: 1,
            clients: 1,
            active_tasks: 0,
            managed_services: 1,
            started_at_unix: 1_700_000_000,
        }
    }

    fn unavailable() -> ApplicationError {
        ApplicationError::RuntimeUnavailable {
            code: codes::DAEMON_UNAVAILABLE.to_owned(),
            message: "the daemon is not running".to_owned(),
        }
    }

    #[test]
    fn probes_classify_into_the_four_outcomes() {
        assert_eq!(classify_probe(Ok(daemon_status())), ProbeOutcome::Connected);
        assert_eq!(
            classify_probe(Err(unavailable())),
            ProbeOutcome::Unreachable("the daemon is not running".to_owned())
        );
        let incompatible = ApplicationError::RuntimeUnavailable {
            code: codes::PROTOCOL_INCOMPATIBLE.to_owned(),
            message: "protocol 9 is not supported".to_owned(),
        };
        assert_eq!(
            classify_probe(Err(incompatible)),
            ProbeOutcome::Incompatible("protocol 9 is not supported".to_owned())
        );
        let other = ApplicationError::RuntimeUnavailable {
            code: "SOMETHING_ELSE".to_owned(),
            message: "surprise".to_owned(),
        };
        assert_eq!(
            classify_probe(Err(other)),
            ProbeOutcome::Failed("[SOMETHING_ELSE] surprise".to_owned())
        );
    }

    #[test]
    fn starts_connecting_and_moves_to_connected() {
        let tracker = ConnectionTracker::new();
        assert_eq!(tracker.snapshot().state, ConnectionState::Connecting);
        assert!(tracker.record(&ProbeOutcome::Connected));
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.state, ConnectionState::Connected);
        assert_eq!(snapshot.message, None);
        // A repeated success is not a transition.
        assert!(!tracker.record(&ProbeOutcome::Connected));
    }

    #[test]
    fn unreachable_streak_reaches_disconnected_after_the_budget() {
        let tracker = ConnectionTracker::new();
        tracker.record(&ProbeOutcome::Connected);

        let probe = ProbeOutcome::Unreachable("socket gone".to_owned());
        // The first failure transitions into reconnection…
        assert!(tracker.record(&probe));
        assert_eq!(tracker.snapshot().state, ConnectionState::Reconnecting);
        // …further failures keep the same visible state, so they emit nothing…
        for _ in 0..RECONNECT_ATTEMPTS - 2 {
            assert!(!tracker.record(&probe));
            assert_eq!(tracker.snapshot().state, ConnectionState::Reconnecting);
        }
        // …until the budget runs out and the stable disconnected banner appears.
        assert!(tracker.record(&probe));
        assert_eq!(tracker.snapshot().state, ConnectionState::Disconnected);
        // Further failures do not re-emit the same state.
        assert!(!tracker.record(&probe));
        assert_eq!(tracker.snapshot().message.as_deref(), Some("socket gone"));
    }

    #[test]
    fn a_success_after_failures_resets_the_streak() {
        let tracker = ConnectionTracker::new();
        for _ in 0..RECONNECT_ATTEMPTS {
            tracker.record(&ProbeOutcome::Unreachable("socket gone".to_owned()));
        }
        assert_eq!(tracker.snapshot().state, ConnectionState::Disconnected);

        assert!(tracker.record(&ProbeOutcome::Connected));
        assert_eq!(tracker.snapshot().state, ConnectionState::Connected);
        // The streak restarts: one new failure is reconnection, not disconnection.
        tracker.record(&ProbeOutcome::Unreachable("socket gone".to_owned()));
        assert_eq!(tracker.snapshot().state, ConnectionState::Reconnecting);
    }

    #[test]
    fn incompatibility_and_failures_become_error_states() {
        let tracker = ConnectionTracker::new();
        assert!(tracker.record(&ProbeOutcome::Incompatible("v9".to_owned())));
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.state, ConnectionState::Error);
        assert_eq!(snapshot.message.as_deref(), Some("v9"));

        assert!(tracker.record(&ProbeOutcome::Failed("boom".to_owned())));
        assert_eq!(tracker.snapshot().state, ConnectionState::Error);
        assert_eq!(tracker.snapshot().message.as_deref(), Some("boom"));
        // Same state and same message is not a transition.
        assert!(!tracker.record(&ProbeOutcome::Failed("boom".to_owned())));
    }

    #[test]
    fn snapshots_carry_the_entered_time() {
        let before = now_unix();
        let tracker = ConnectionTracker::new();
        let snapshot = tracker.snapshot();
        assert!(snapshot.since_unix >= before);
        assert_eq!(
            serde_json::to_value(&snapshot).expect("snapshot serializes"),
            serde_json::json!({
                "state": "connecting",
                "message": null,
                "since_unix": snapshot.since_unix,
            })
        );
    }
}
