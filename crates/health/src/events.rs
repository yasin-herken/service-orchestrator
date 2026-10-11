//! Health events.
//!
//! The manager publishes an interface-neutral [`HealthEvent`] for every
//! meaningful observation: a check started, a check finished, or a service's
//! aggregate state changed. The vocabulary mirrors the process and runtime buses
//! so a single subscription can drive a TUI health panel, an MCP status feed, or
//! future telemetry (`tasks/012.md` §19).
//!
//! # No repeated state-change events
//!
//! [`HealthEvent::ServiceHealthChanged`] is only published when the aggregate
//! state actually differs from the previous observation, so a service that stays
//! healthy while polled does not emit an event storm (`tasks/012.md` §19). The
//! `became healthy` / `became unhealthy` transitions are recovered from the
//! event's `previous` and `current` fields rather than duplicated as their own
//! variants.
//!
//! Delivery is in-process and bounded. A subscriber that falls behind observes a
//! `Lagged` error and can resynchronise from the [`HealthIndex`] the store
//! implements.
//!
//! [`HealthIndex`]: service_orchestrator_application::HealthIndex

use tokio::sync::broadcast;

use service_orchestrator_application::HealthState;
use service_orchestrator_domain::{ServiceId, WorkspaceId};

/// A notable health observation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthEvent {
    /// A single check began.
    HealthCheckStarted {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service being checked.
        service_id: ServiceId,
        /// The provider label (`process`, `tcp`, `http`).
        check_type: String,
    },
    /// A single check passed.
    HealthCheckSucceeded {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service that was checked.
        service_id: ServiceId,
        /// The provider label.
        check_type: String,
        /// How long the probe took, in milliseconds.
        duration_ms: u64,
    },
    /// A single check did not pass.
    HealthCheckFailed {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service that was checked.
        service_id: ServiceId,
        /// The provider label.
        check_type: String,
        /// The observed status of the check.
        status: HealthState,
        /// A short, redacted explanation.
        message: String,
        /// How long the probe took, in milliseconds.
        duration_ms: u64,
    },
    /// A service's aggregate health changed.
    ///
    /// This is the only event published on a state *transition*; when a service
    /// is polled and its state is unchanged, no event is emitted.
    ServiceHealthChanged {
        /// The owning workspace.
        workspace_id: WorkspaceId,
        /// The service whose health changed.
        service_id: ServiceId,
        /// The previous aggregate state.
        previous: HealthState,
        /// The new aggregate state.
        current: HealthState,
    },
}

impl HealthEvent {
    /// Returns the service this event concerns.
    #[must_use]
    pub fn service_id(&self) -> &ServiceId {
        match self {
            Self::HealthCheckStarted { service_id, .. }
            | Self::HealthCheckSucceeded { service_id, .. }
            | Self::HealthCheckFailed { service_id, .. }
            | Self::ServiceHealthChanged { service_id, .. } => service_id,
        }
    }

    /// Returns the provider label for a per-check event, if this is one.
    #[must_use]
    pub fn check_type(&self) -> Option<&str> {
        match self {
            Self::HealthCheckStarted { check_type, .. }
            | Self::HealthCheckSucceeded { check_type, .. }
            | Self::HealthCheckFailed { check_type, .. } => Some(check_type),
            Self::ServiceHealthChanged { .. } => None,
        }
    }

    /// Returns `true` when this event reports a state transition.
    #[must_use]
    pub const fn is_state_change(&self) -> bool {
        matches!(self, Self::ServiceHealthChanged { .. })
    }
}

/// The manager's bounded broadcast bus.
#[derive(Clone)]
pub(crate) struct HealthEventBus {
    sender: broadcast::Sender<HealthEvent>,
}

impl HealthEventBus {
    /// Creates a bus with the given channel capacity (at least one).
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    /// Publishes an event, ignoring the absence of subscribers.
    pub(crate) fn publish(&self, event: HealthEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribes to future events.
    pub(crate) fn subscribe(&self) -> HealthEventStream {
        HealthEventStream {
            receiver: self.sender.subscribe(),
        }
    }
}

/// A bounded, async view of health events.
pub struct HealthEventStream {
    receiver: broadcast::Receiver<HealthEvent>,
}

impl HealthEventStream {
    /// Receives the next event.
    ///
    /// # Errors
    ///
    /// Returns a [`broadcast::error::RecvError`] when the consumer fell behind
    /// or the manager has shut down.
    pub async fn recv(&mut self) -> Result<HealthEvent, broadcast::error::RecvError> {
        self.receiver.recv().await
    }
}

impl std::fmt::Debug for HealthEventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthEventStream").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_expose_their_context() {
        let event = HealthEvent::HealthCheckFailed {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new("auth-service").unwrap(),
            check_type: "http".to_owned(),
            status: HealthState::Unhealthy,
            message: "refused".to_owned(),
            duration_ms: 3,
        };
        assert_eq!(event.service_id().as_str(), "auth-service");
        assert_eq!(event.check_type(), Some("http"));
        assert!(!event.is_state_change());

        let change = HealthEvent::ServiceHealthChanged {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new("auth-service").unwrap(),
            previous: HealthState::Unknown,
            current: HealthState::Healthy,
        };
        assert!(change.is_state_change());
        assert_eq!(change.check_type(), None);
    }

    #[tokio::test]
    async fn the_bus_delivers_to_subscribers() {
        let bus = HealthEventBus::new(4);
        let mut stream = bus.subscribe();
        bus.publish(HealthEvent::ServiceHealthChanged {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: ServiceId::new("auth-service").unwrap(),
            previous: HealthState::Unknown,
            current: HealthState::Healthy,
        });
        let event = stream.recv().await.unwrap();
        assert!(event.is_state_change());
    }
}
