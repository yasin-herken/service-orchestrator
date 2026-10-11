//! Health boundary.
//!
//! The application never probes a process, opens a socket, or sends an HTTP
//! request itself. Health is *observed* by the health adapter, which runs the
//! configured checks — as task-engine operations — and publishes the result
//! into an in-memory [`HealthIndex`]. The application reads that index to answer
//! `get_service_health`, `list_service_health`, and `get_service_diagnostics`.
//!
//! # Latest observation, not a probe
//!
//! [`HealthIndex`] is read-only and side-effect free, exactly like
//! [`RuntimeStateStore`](crate::RuntimeStateStore): a status query returns the
//! *last observed* health and never triggers a check. Running a check is a task
//! (`check_service_health`), which keeps the read path predictable and prevents a
//! query from probing anything (`AGENTS.md` §14, `tasks/012.md` §17).
//!
//! # Health versus process state
//!
//! The health state is deliberately separate from the process state. A service
//! whose process is running may be unhealthy, and a service whose process is
//! still starting may be `checking` (`tasks/012.md` §11).
//!
//! # Why a port value and a small trait
//!
//! Health checks are task-driven, so the *mutating* path is a task executor and
//! needs no trait here. The application does need to answer read-oriented health
//! questions from the adapter's memory, so [`HealthIndex`] is a small, read-only
//! port that the adapter implements and [`NullHealthIndex`] keeps the application
//! constructible without it.

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{ServiceId, Timestamp, WorkspaceId};

use super::runtime_state::HealthState;

/// The label of a process health check.
pub const CHECK_PROCESS: &str = "process";
/// The label of a TCP/port health check.
pub const CHECK_TCP: &str = "tcp";
/// The label of an HTTP (or Spring Boot Actuator) health check.
pub const CHECK_HTTP: &str = "http";

/// The outcome of a single health check.
///
/// A check outcome is a *fact*: what was probed, whether it passed, how long it
/// took, and — for a failure — a short, redacted explanation. The `check_type`
/// and `target` are stable, machine-readable labels so an interface can group
/// results without parsing message text (`tasks/012.md` §17).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheckOutcome {
    /// The kind of probe (`process`, `tcp`, `http`, or a provider label).
    pub check_type: String,
    /// What was probed, for example `localhost:8081` or an HTTP URL.
    #[serde(default)]
    pub target: Option<String>,
    /// Whether the check is required for the service to be healthy. A failed
    /// optional check degrades health rather than making it unhealthy.
    pub required: bool,
    /// The observed status of this single check.
    pub status: HealthState,
    /// How long the probe took, in milliseconds.
    pub duration_ms: u64,
    /// A short, redacted explanation, present for a failure.
    #[serde(default)]
    pub message: Option<String>,
    /// When the check ran.
    pub checked_at: Timestamp,
}

impl HealthCheckOutcome {
    /// Returns `true` when the check passed.
    #[must_use]
    pub const fn is_healthy(&self) -> bool {
        matches!(self.status, HealthState::Healthy)
    }
}

/// The aggregated health of a service.
///
/// The aggregate combines the latest individual check outcomes and the
/// history the adapter retains: when it was last checked, when it was last
/// healthy and unhealthy, and how many times in a row it has failed
/// (`tasks/012.md` §17, §18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceHealth {
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The service identifier.
    pub service_id: ServiceId,
    /// The aggregated health state.
    pub state: HealthState,
    /// When the service was last checked, if it ever has been.
    #[serde(default)]
    pub checked_at: Option<Timestamp>,
    /// The latest outcome of each configured check.
    #[serde(default)]
    pub checks: Vec<HealthCheckOutcome>,
    /// How many consecutive observations have failed (required or optional).
    #[serde(default)]
    pub consecutive_failures: u32,
    /// When the service was last observed healthy.
    #[serde(default)]
    pub last_healthy_at: Option<Timestamp>,
    /// When the service was last observed unhealthy.
    #[serde(default)]
    pub last_unhealthy_at: Option<Timestamp>,
}

impl ServiceHealth {
    /// Builds an `unknown` health record for a service that has never been
    /// checked.
    #[must_use]
    pub fn unknown(workspace_id: WorkspaceId, service_id: ServiceId) -> Self {
        Self {
            workspace_id,
            service_id,
            state: HealthState::Unknown,
            checked_at: None,
            checks: Vec::new(),
            consecutive_failures: 0,
            last_healthy_at: None,
            last_unhealthy_at: None,
        }
    }

    /// Returns `true` when the service is fully healthy.
    #[must_use]
    pub const fn is_healthy(&self) -> bool {
        self.state.is_healthy()
    }

    /// Returns `true` when the service is serving (healthy or degraded).
    #[must_use]
    pub const fn is_serving(&self) -> bool {
        self.state.is_serving()
    }
}

/// A read-only index of the last observed health of services.
///
/// The health adapter implements this over its in-memory history; the
/// application reads it so its queries never reach past a port. The
/// [`NullHealthIndex`] answers "nothing has been checked", which keeps the
/// application constructible and its existing tests unchanged when no health
/// adapter is wired.
pub trait HealthIndex: Send + Sync {
    /// Returns the last observed health for `service_id`, if any.
    fn service_health(&self, service_id: &ServiceId) -> Option<ServiceHealth>;

    /// Returns the last observed health of every service the index knows about.
    fn services(&self) -> Vec<ServiceHealth>;
}

/// A [`HealthIndex`] that knows about nothing.
///
/// It is the default so the application is always constructible. Health queries
/// then report [`HealthState::Unknown`] rather than failing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullHealthIndex;

impl HealthIndex for NullHealthIndex {
    fn service_health(&self, _service_id: &ServiceId) -> Option<ServiceHealth> {
        None
    }

    fn services(&self) -> Vec<ServiceHealth> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> ServiceId {
        ServiceId::new("auth-service").unwrap()
    }

    fn workspace() -> WorkspaceId {
        WorkspaceId::new("acme").unwrap()
    }

    #[test]
    fn null_index_reports_nothing() {
        assert!(NullHealthIndex.service_health(&service()).is_none());
        assert!(NullHealthIndex.services().is_empty());
    }

    #[test]
    fn unknown_health_is_not_serving() {
        let health = ServiceHealth::unknown(workspace(), service());
        assert_eq!(health.state, HealthState::Unknown);
        assert!(!health.is_healthy());
        assert!(!health.is_serving());
    }

    #[test]
    fn degraded_health_is_serving_but_not_healthy() {
        let mut health = ServiceHealth::unknown(workspace(), service());
        health.state = HealthState::Degraded;
        assert!(!health.is_healthy());
        assert!(health.is_serving());
    }
}
