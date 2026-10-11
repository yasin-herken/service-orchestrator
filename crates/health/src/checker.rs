//! The health-checker abstraction.
//!
//! A [`HealthChecker`] answers a single question about one configured check:
//! *is the managed process alive?*, *does the port accept a connection?*, *does
//! the HTTP endpoint report a healthy response?*. It is deliberately small and
//! synchronous-looking: the manager owns timing, timeouts, cancellation,
//! aggregation, storage, and events.
//!
//! # A probe result is not an error
//!
//! A checker returns [`CheckResult`] for **every** ordinary observation,
//! including a failure: the port is closed, the process is not running, the
//! endpoint answered `DOWN`. Those are facts about the service, and the manager
//! records them as a [`HealthState`]. A [`HealthError`] is reserved for the
//! cases where the check could not be attempted at all — a malformed URL, an
//! unsupported scheme, a missing port (`tasks/012.md` §35).
//!
//! # Providers are not a plugin framework
//!
//! There are three providers — process, TCP, and HTTP — and they are looked up
//! by [`CheckType`] in a small registry, not loaded dynamically
//! (`tasks/012.md` §5, §6). Docker, Kubernetes, and gRPC providers are
//! explicitly out of scope for this task.

use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_application::HealthState;
use service_orchestrator_domain::{HealthCheck, HealthCheckKind, ServiceId, WorkspaceId};

use crate::error::HealthError;

/// The kind of probe a [`HealthChecker`] performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CheckType {
    /// The service's managed process is alive.
    Process,
    /// A TCP port accepts a connection.
    Tcp,
    /// An HTTP endpoint reports a healthy response.
    Http,
}

impl CheckType {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Tcp => "tcp",
            Self::Http => "http",
        }
    }

    /// Maps a configured check kind onto the provider that handles it.
    ///
    /// Returns `None` for a project-specific [`HealthCheckKind::Other`], which
    /// the manager reports as [`HealthError::HealthCheckUnsupported`].
    #[must_use]
    pub fn from_kind(kind: &HealthCheckKind) -> Option<Self> {
        match kind {
            HealthCheckKind::Process => Some(Self::Process),
            HealthCheckKind::Port(_) => Some(Self::Tcp),
            HealthCheckKind::Http(_) => Some(Self::Http),
            HealthCheckKind::Other(_) => None,
        }
    }
}

/// A stable, display-safe description of what a check probes.
///
/// Used for the check outcome's `target` and for the timeout message, so the
/// manager can describe a check even when its future did not complete.
#[must_use]
pub fn check_target(kind: &HealthCheckKind) -> Option<String> {
    match kind {
        HealthCheckKind::Process | HealthCheckKind::Other(_) => None,
        HealthCheckKind::Port(port) => Some(format!("127.0.0.1:{}", port.value())),
        HealthCheckKind::Http(check) => Some(check.url.clone()),
    }
}

/// Everything a checker needs to perform one probe.
///
/// The context borrows the configured check so the checker is cheap to clone and
/// carries no owned configuration.
pub struct HealthCheckContext<'a> {
    /// The workspace that owns the service.
    pub workspace_id: &'a WorkspaceId,
    /// The service being checked.
    pub service_id: &'a ServiceId,
    /// The configured check to perform.
    pub check: &'a HealthCheck,
    /// How long the probe may run before the manager times it out.
    pub timeout: Duration,
}

/// The observed result of a single probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckResult {
    /// The observed status of the probe.
    pub status: HealthState,
    /// What was probed, for example a URL or `127.0.0.1:8081`.
    pub target: Option<String>,
    /// A short, display-safe explanation, present for a failure.
    pub message: Option<String>,
}

impl CheckResult {
    /// A probe that passed.
    #[must_use]
    pub fn healthy(target: Option<String>) -> Self {
        Self {
            status: HealthState::Healthy,
            target,
            message: None,
        }
    }

    /// A probe that failed: a required check in this state makes the service
    /// unhealthy.
    #[must_use]
    pub fn unhealthy(target: Option<String>, message: impl Into<String>) -> Self {
        Self {
            status: HealthState::Unhealthy,
            target,
            message: Some(message.into()),
        }
    }

    /// A probe that could not reach the service: the process may be alive, but
    /// nothing is answering yet.
    #[must_use]
    pub fn unreachable(target: Option<String>, message: impl Into<String>) -> Self {
        Self {
            status: HealthState::Unreachable,
            target,
            message: Some(message.into()),
        }
    }

    /// A probe that could not determine health.
    #[must_use]
    pub fn unknown(target: Option<String>, message: impl Into<String>) -> Self {
        Self {
            status: HealthState::Unknown,
            target,
            message: Some(message.into()),
        }
    }

    /// Attaches a message to a passing result.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// Performs one kind of health probe.
#[async_trait]
pub trait HealthChecker: Send + Sync {
    /// The provider's check type.
    fn check_type(&self) -> CheckType;

    /// Performs the probe described by `context`.
    ///
    /// # Errors
    ///
    /// Returns a [`HealthError`] only when the probe could not be attempted
    /// (a configuration or internal failure). An ordinary unhealthy or
    /// unreachable observation is returned as an `Ok` [`CheckResult`].
    async fn check(&self, context: &HealthCheckContext<'_>) -> Result<CheckResult, HealthError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{HealthCheckKind, HttpHealthCheck, Port};

    #[test]
    fn maps_check_kinds_to_providers() {
        assert_eq!(
            CheckType::from_kind(&HealthCheckKind::Process),
            Some(CheckType::Process)
        );
        assert_eq!(
            CheckType::from_kind(&HealthCheckKind::Port(Port::new(8081).unwrap())),
            Some(CheckType::Tcp)
        );
        assert_eq!(
            CheckType::from_kind(&HealthCheckKind::Http(
                HttpHealthCheck::new("http://localhost:8081/actuator/health", None).unwrap()
            )),
            Some(CheckType::Http)
        );
        assert_eq!(
            CheckType::from_kind(&HealthCheckKind::Other("docker".to_owned())),
            None
        );
    }

    #[test]
    fn describes_probe_targets() {
        assert_eq!(check_target(&HealthCheckKind::Process), None);
        assert_eq!(
            check_target(&HealthCheckKind::Port(Port::new(8081).unwrap())).as_deref(),
            Some("127.0.0.1:8081")
        );
        assert_eq!(
            check_target(&HealthCheckKind::Http(
                HttpHealthCheck::new("http://localhost:9000/health", None).unwrap()
            ))
            .as_deref(),
            Some("http://localhost:9000/health")
        );
    }

    #[test]
    fn check_results_carry_status_and_message() {
        let result = CheckResult::unreachable(Some("127.0.0.1:8081".to_owned()), "refused");
        assert_eq!(result.status, HealthState::Unreachable);
        assert_eq!(result.message.as_deref(), Some("refused"));
        assert_eq!(CheckResult::healthy(None).status, HealthState::Healthy);
    }
}
