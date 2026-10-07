//! Generic, provider-independent health check model.
//!
//! The model describes *what* should be probed, not *how*. The concrete
//! implementations (process inspection, TCP connect, HTTP request, Spring
//! Boot actuator, Docker, Kubernetes) live in the `health` infrastructure
//! crate. Future check kinds are added by extending [`HealthCheckKind`],
//! which is why it carries an `Other` variant.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::port::Port;

/// An HTTP health endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpHealthCheck {
    /// The absolute URL to probe.
    pub url: String,
    /// The HTTP status code treated as healthy, if the probe requires one.
    pub expected_status: Option<u16>,
}

impl HttpHealthCheck {
    /// Constructs an HTTP health check.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the URL is not an absolute
    /// `http://` or `https://` URL, or if the expected status is outside the
    /// valid HTTP status range.
    pub fn new(url: impl Into<String>, expected_status: Option<u16>) -> Result<Self, DomainError> {
        let url = url.into();
        let trimmed = url.trim();
        if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
            return Err(DomainError::validation(
                "health check",
                format!("url '{trimmed}' must start with http:// or https://"),
            ));
        }
        if let Some(status) = expected_status {
            if !(100..=599).contains(&status) {
                return Err(DomainError::validation(
                    "health check",
                    format!("expected status {status} is not a valid HTTP status code"),
                ));
            }
        }
        Ok(Self {
            url: trimmed.to_owned(),
            expected_status,
        })
    }
}

/// The kind of probe a health check performs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckKind {
    /// The service's process is alive.
    Process,
    /// A TCP port is accepting connections.
    Port(Port),
    /// An HTTP endpoint returns a healthy response.
    Http(HttpHealthCheck),
    /// A future or project-specific check.
    Other(String),
}

impl HealthCheckKind {
    /// Returns a short, human-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Process => "process",
            Self::Port(_) => "port",
            Self::Http(_) => "http",
            Self::Other(name) => name,
        }
    }

    /// Returns the port a `Port` check probes, if this is a port check.
    #[must_use]
    pub fn port(&self) -> Option<Port> {
        match self {
            Self::Port(port) => Some(*port),
            _ => None,
        }
    }
}

impl fmt::Display for HealthCheckKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Process | Self::Other(_) => f.write_str(self.label()),
            Self::Port(port) => write!(f, "port {port}"),
            Self::Http(check) => write!(f, "http {}", check.url),
        }
    }
}

/// A configured health check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthCheck {
    /// The probe performed.
    pub kind: HealthCheckKind,
    /// How long a single probe may take before it is considered failed.
    pub timeout: Option<Duration>,
    /// How long to wait between probes when polling.
    pub interval: Option<Duration>,
}

impl HealthCheck {
    /// Creates a health check with default (unspecified) timeout and interval.
    #[must_use]
    pub fn new(kind: HealthCheckKind) -> Self {
        Self {
            kind,
            timeout: None,
            interval: None,
        }
    }

    /// Sets the probe timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the polling interval.
    #[must_use]
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = Some(interval);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_absolute_http_urls() {
        let check =
            HttpHealthCheck::new("http://localhost:8081/actuator/health", Some(200)).unwrap();
        assert_eq!(check.expected_status, Some(200));
    }

    #[test]
    fn rejects_relative_http_urls() {
        assert!(HttpHealthCheck::new("/actuator/health", None).is_err());
    }

    #[test]
    fn rejects_out_of_range_status() {
        assert!(HttpHealthCheck::new("http://localhost/health", Some(99)).is_err());
        assert!(HttpHealthCheck::new("http://localhost/health", Some(600)).is_err());
    }

    #[test]
    fn exposes_port_checks() {
        let check = HealthCheck::new(HealthCheckKind::Port(Port::new(8081).unwrap()));
        assert_eq!(check.kind.port().map(Port::value), Some(8081));
        assert_eq!(check.kind.label(), "port");
    }

    #[test]
    fn process_check_has_no_port() {
        let check = HealthCheck::new(HealthCheckKind::Process);
        assert!(check.kind.port().is_none());
    }
}
