//! The TCP health provider.
//!
//! A TCP check attempts a connection to a local port with a configurable
//! timeout. A successful connection indicates **reachability**, not
//! application-level readiness: a socket that accepts a connection may still be
//! initializing (`tasks/012.md` §8, §12). When a stronger check (HTTP/actuator)
//! is configured, aggregation does not report the service fully healthy on TCP
//! reachability alone.
//!
//! Only the loopback interface is probed. A port check comes from validated
//! configuration, and the provider deliberately does not offer arbitrary
//! host/port requests (`tasks/012.md` §34).

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use tokio::net::TcpStream;

use crate::checker::{CheckResult, CheckType, HealthCheckContext, HealthChecker};
use crate::error::HealthError;

/// The TCP provider.
#[derive(Clone, Copy, Debug, Default)]
pub struct TcpHealthChecker;

impl TcpHealthChecker {
    /// Creates a TCP provider.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl HealthChecker for TcpHealthChecker {
    fn check_type(&self) -> CheckType {
        CheckType::Tcp
    }

    async fn check(&self, context: &HealthCheckContext<'_>) -> Result<CheckResult, HealthError> {
        let Some(port) = context.check.kind.port() else {
            return Err(HealthError::HealthConfigurationInvalid {
                service: context.service_id.clone(),
                message: "a tcp health check must declare a port".to_owned(),
            });
        };
        let port = port.value();
        let target = format!("127.0.0.1:{port}");
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let timeout = effective_timeout(context.timeout);

        match tokio::time::timeout(timeout, TcpStream::connect(address)).await {
            Ok(Ok(_stream)) => Ok(CheckResult::healthy(Some(target))),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => Ok(
                CheckResult::unreachable(Some(target), "the connection was refused"),
            ),
            Ok(Err(error)) => Ok(CheckResult::unreachable(Some(target), error.to_string())),
            Err(_elapsed) => Ok(CheckResult::unreachable(
                Some(target),
                format!("the connection timed out after {} ms", timeout.as_millis()),
            )),
        }
    }
}

/// Returns a usable timeout even when a caller supplies zero.
fn effective_timeout(timeout: Duration) -> Duration {
    if timeout.is_zero() {
        Duration::from_millis(1)
    } else {
        timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::HealthState;
    use service_orchestrator_domain::{HealthCheck, HealthCheckKind, Port, ServiceId, WorkspaceId};

    fn parts(check: HealthCheck) -> (WorkspaceId, ServiceId, HealthCheck) {
        (
            WorkspaceId::new("acme").unwrap(),
            ServiceId::new("auth-service").unwrap(),
            check,
        )
    }

    #[tokio::test]
    async fn reports_an_open_port_as_reachable() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (workspace, service, check) = parts(HealthCheck::new(HealthCheckKind::Port(
            Port::new(port).unwrap(),
        )));
        let result = TcpHealthChecker::new()
            .check(&HealthCheckContext {
                workspace_id: &workspace,
                service_id: &service,
                check: &check,
                timeout: Duration::from_secs(1),
            })
            .await
            .unwrap();
        assert_eq!(result.status, HealthState::Healthy);
    }

    #[tokio::test]
    async fn reports_a_closed_port_as_unreachable() {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let (workspace, service, check) = parts(HealthCheck::new(HealthCheckKind::Port(
            Port::new(port).unwrap(),
        )));
        let result = TcpHealthChecker::new()
            .check(&HealthCheckContext {
                workspace_id: &workspace,
                service_id: &service,
                check: &check,
                timeout: Duration::from_millis(500),
            })
            .await
            .unwrap();
        assert_eq!(result.status, HealthState::Unreachable);
    }

    #[tokio::test]
    async fn a_check_without_a_port_is_a_configuration_error() {
        let (workspace, service, check) = parts(HealthCheck::new(HealthCheckKind::Process));
        let error = TcpHealthChecker::new()
            .check(&HealthCheckContext {
                workspace_id: &workspace,
                service_id: &service,
                check: &check,
                timeout: Duration::from_secs(1),
            })
            .await
            .unwrap_err();
        assert!(error.is_configuration());
    }
}
