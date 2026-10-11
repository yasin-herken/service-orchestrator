//! Structured health errors.
//!
//! [`HealthError`] is the health adapter's error vocabulary. It is deliberately
//! structured: every variant carries the service and a stable, machine-readable
//! [`code`](HealthError::code) so the task executor can map a probe failure to a
//! task failure without parsing a message, and an interface (or a future MCP
//! tool) can branch on the code (`tasks/012.md` §35).
//!
//! A probe that merely observes "not healthy" is **not** an error: the health
//! manager reports it as a [`HealthState`](service_orchestrator_application::HealthState).
//! An error means the check could not be executed at all — the service is
//! unknown, the configuration is invalid, the operation was cancelled, or the
//! adapter failed internally.

use service_orchestrator_domain::ServiceId;
use thiserror::Error;

/// An error produced while running a health check.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HealthError {
    /// The requested service does not exist in the configuration.
    #[error("service '{service}' was not found")]
    ServiceNotFound {
        /// The service that was requested.
        service: ServiceId,
    },

    /// A health check exceeded its timeout.
    #[error(
        "the {check_type} health check for service '{service}' timed out after {timeout_ms} ms"
    )]
    HealthCheckTimeout {
        /// The service being checked.
        service: ServiceId,
        /// The kind of check that timed out (`process`, `tcp`, `http`).
        check_type: String,
        /// The timeout that elapsed, in milliseconds.
        timeout_ms: u64,
    },

    /// A health check could not reach the service.
    #[error(
        "the {check_type} health check for service '{service}' could not connect to '{target}': {message}"
    )]
    HealthCheckConnectionRefused {
        /// The service being checked.
        service: ServiceId,
        /// The kind of check that failed.
        check_type: String,
        /// What was probed, for example `127.0.0.1:8081`.
        target: String,
        /// A short explanation.
        message: String,
    },

    /// A health check returned a response that could not be interpreted.
    #[error("the {check_type} health check for service '{service}' returned an invalid response: {message}")]
    HealthCheckInvalidResponse {
        /// The service being checked.
        service: ServiceId,
        /// The kind of check that failed.
        check_type: String,
        /// Why the response was invalid.
        message: String,
    },

    /// No checker is registered for the configured check kind.
    #[error("the '{check_type}' health check kind is not supported for service '{service}'")]
    HealthCheckUnsupported {
        /// The service being checked.
        service: ServiceId,
        /// The unsupported check kind.
        check_type: String,
    },

    /// The service's health configuration is not usable.
    #[error("the health configuration for service '{service}' is invalid: {message}")]
    HealthConfigurationInvalid {
        /// The service whose configuration is invalid.
        service: ServiceId,
        /// What is wrong with the configuration.
        message: String,
    },

    /// The health operation observed cancellation and stopped cooperatively.
    #[error("the health operation for service '{service}' was cancelled")]
    OperationCancelled {
        /// The service whose operation was cancelled.
        service: ServiceId,
    },

    /// A readiness check did not observe a ready service before its deadline.
    #[error("service '{service}' did not become ready: {message}")]
    HealthNotReady {
        /// The service that did not become ready.
        service: ServiceId,
        /// The last observed state and why it was not ready.
        message: String,
    },

    /// The health adapter failed for an internal reason.
    #[error("health manager error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

impl HealthError {
    /// Returns a stable, machine-readable code for the error.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ServiceNotFound { .. } => "HEALTH_SERVICE_NOT_FOUND",
            Self::HealthCheckTimeout { .. } => "HEALTH_CHECK_TIMEOUT",
            Self::HealthCheckConnectionRefused { .. } => "HEALTH_CHECK_CONNECTION_REFUSED",
            Self::HealthCheckInvalidResponse { .. } => "HEALTH_CHECK_INVALID_RESPONSE",
            Self::HealthCheckUnsupported { .. } => "HEALTH_CHECK_UNSUPPORTED",
            Self::HealthConfigurationInvalid { .. } => "HEALTH_CONFIGURATION_INVALID",
            Self::OperationCancelled { .. } => "HEALTH_OPERATION_CANCELLED",
            Self::HealthNotReady { .. } => "HEALTH_NOT_READY",
            Self::Internal { .. } => "HEALTH_INTERNAL_ERROR",
        }
    }

    /// Returns the service the error refers to, when it has one.
    #[must_use]
    pub const fn service(&self) -> Option<&ServiceId> {
        match self {
            Self::ServiceNotFound { service }
            | Self::HealthCheckTimeout { service, .. }
            | Self::HealthCheckConnectionRefused { service, .. }
            | Self::HealthCheckInvalidResponse { service, .. }
            | Self::HealthCheckUnsupported { service, .. }
            | Self::HealthConfigurationInvalid { service, .. }
            | Self::OperationCancelled { service }
            | Self::HealthNotReady { service, .. } => Some(service),
            Self::Internal { .. } => None,
        }
    }

    /// Returns `true` when the error is a configuration problem rather than a
    /// runtime probe outcome.
    ///
    /// A configuration error means the check could not be attempted at all, so
    /// the manager reports it as a failed task rather than as an unhealthy
    /// service.
    #[must_use]
    pub const fn is_configuration(&self) -> bool {
        matches!(
            self,
            Self::HealthCheckUnsupported { .. } | Self::HealthConfigurationInvalid { .. }
        )
    }

    /// Builds a [`HealthError::Internal`].
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> ServiceId {
        ServiceId::new("auth-service").unwrap()
    }

    #[test]
    fn codes_are_stable() {
        assert_eq!(
            HealthError::HealthCheckTimeout {
                service: service(),
                check_type: "http".to_owned(),
                timeout_ms: 5000,
            }
            .code(),
            "HEALTH_CHECK_TIMEOUT"
        );
        assert_eq!(
            HealthError::HealthNotReady {
                service: service(),
                message: "still starting".to_owned(),
            }
            .code(),
            "HEALTH_NOT_READY"
        );
    }

    #[test]
    fn configuration_errors_are_classified() {
        assert!(HealthError::HealthConfigurationInvalid {
            service: service(),
            message: "no checks".to_owned(),
        }
        .is_configuration());
        assert!(!HealthError::OperationCancelled { service: service() }.is_configuration());
    }

    #[test]
    fn errors_expose_their_service() {
        assert_eq!(
            HealthError::ServiceNotFound { service: service() }
                .service()
                .map(ServiceId::as_str),
            Some("auth-service")
        );
        assert!(HealthError::internal("boom").service().is_none());
    }
}
