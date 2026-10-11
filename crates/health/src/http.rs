//! The HTTP health provider.
//!
//! A single `GET` request is sent over a plain TCP socket and the response is
//! interpreted. Spring Boot Actuator is supported through the same provider:
//! when the body contains an Actuator `status` field it is authoritative, so an
//! HTTP `200` with `"status":"DOWN"` is reported as unhealthy
//! (`tasks/012.md` §9, §10).
//!
//! # Deliberate limits
//!
//! - Only `http://` is supported. `https://` requires TLS, which the built-in
//!   provider does not implement; a configured HTTPS URL is rejected as a
//!   configuration error rather than silently downgraded.
//! - Redirects are never followed: a redirect is just an unexpected status, so
//!   credentials are never forwarded to another endpoint (`tasks/012.md` §9, §34).
//! - The response body is read with a hard cap, so a hostile or broken endpoint
//!   cannot exhaust memory.
//! - No credentials or custom headers are sent. The provider sends only `Host`,
//!   `Connection: close`, and `Accept` (`tasks/012.md` §28).

use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_application::HealthState;
use service_orchestrator_domain::HealthCheckKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::checker::{CheckResult, CheckType, HealthCheckContext, HealthChecker};
use crate::error::HealthError;

/// The maximum number of response bytes read from an endpoint.
const MAX_BODY_BYTES: usize = 64 * 1024;

/// The HTTP provider.
#[derive(Clone, Copy, Debug)]
pub struct HttpHealthChecker {
    /// The default status code treated as healthy when none is configured and
    /// the body carries no Actuator status.
    default_status: u16,
}

impl HttpHealthChecker {
    /// Creates an HTTP provider that treats `200` as healthy by default.
    #[must_use]
    pub fn new() -> Self {
        Self {
            default_status: 200,
        }
    }

    /// Overrides the default expected status code.
    #[must_use]
    pub fn with_default_status(mut self, status: u16) -> Self {
        self.default_status = status;
        self
    }
}

impl Default for HttpHealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HealthChecker for HttpHealthChecker {
    fn check_type(&self) -> CheckType {
        CheckType::Http
    }

    async fn check(&self, context: &HealthCheckContext<'_>) -> Result<CheckResult, HealthError> {
        let HealthCheckKind::Http(config) = &context.check.kind else {
            return Err(HealthError::HealthConfigurationInvalid {
                service: context.service_id.clone(),
                message: "an http health check must declare a url".to_owned(),
            });
        };
        let target = parse_http_url(&config.url).map_err(|message| {
            HealthError::HealthConfigurationInvalid {
                service: context.service_id.clone(),
                message,
            }
        })?;

        let timeout = if context.timeout.is_zero() {
            Duration::from_millis(1)
        } else {
            context.timeout
        };

        let exchange = async {
            let mut stream = TcpStream::connect((target.host.as_str(), target.port)).await?;
            let request = build_request(&target);
            stream.write_all(request.as_bytes()).await?;
            stream.flush().await?;
            let mut body = Vec::new();
            let mut limited = stream.take(MAX_BODY_BYTES as u64);
            limited.read_to_end(&mut body).await?;
            Ok::<Vec<u8>, std::io::Error>(body)
        };

        let bytes = match tokio::time::timeout(timeout, exchange).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                return Ok(CheckResult::unreachable(
                    Some(config.url.clone()),
                    "the connection was refused",
                ));
            }
            Ok(Err(error)) => {
                return Ok(CheckResult::unreachable(
                    Some(config.url.clone()),
                    error.to_string(),
                ));
            }
            Err(_elapsed) => {
                return Ok(CheckResult::unreachable(
                    Some(config.url.clone()),
                    format!("the request timed out after {} ms", timeout.as_millis()),
                ));
            }
        };

        let (status_code, body_status) =
            parse_response(&bytes).map_err(|message| HealthError::HealthCheckInvalidResponse {
                service: context.service_id.clone(),
                check_type: CheckType::Http.label().to_owned(),
                message,
            })?;

        let expected = config.expected_status.or(Some(self.default_status));
        Ok(evaluate(
            &config.url,
            status_code,
            body_status.as_deref(),
            expected,
        ))
    }
}

/// A parsed HTTP endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HttpTarget {
    host: String,
    port: u16,
    path: String,
}

/// Parses an absolute `http://` URL into a host, port, and path.
///
/// Only the pieces the provider needs are extracted; userinfo, query strings,
/// and fragments are preserved in the path but no userinfo is ever sent.
fn parse_http_url(url: &str) -> Result<HttpTarget, String> {
    let trimmed = url.trim();
    let Some(rest) = trimmed.strip_prefix("http://") else {
        if trimmed.starts_with("https://") {
            return Err(
                "https health urls are not supported by the built-in http provider".to_owned(),
            );
        }
        return Err(format!("url '{trimmed}' must start with http://"));
    };
    if rest.is_empty() {
        return Err("url has no host".to_owned());
    }
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    // Strip any userinfo; the provider sends no credentials.
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((host, port)) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("'{port}' is not a valid port"))?;
            (host.to_owned(), port)
        }
        None => (host_port.to_owned(), 80),
    };
    if host.is_empty() {
        return Err("url has no host".to_owned());
    }
    Ok(HttpTarget {
        host,
        port,
        path: path.to_owned(),
    })
}

/// Builds the minimal request the provider sends.
fn build_request(target: &HttpTarget) -> String {
    let host_header = if target.port == 80 {
        target.host.clone()
    } else {
        format!("{}:{}", target.host, target.port)
    };
    format!(
        "GET {} HTTP/1.1\r\nHost: {host_header}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        target.path
    )
}

/// Extracts the status code and an optional Actuator `status` from a response.
fn parse_response(bytes: &[u8]) -> Result<(u16, Option<String>), String> {
    let split_at =
        find_header_end(bytes).ok_or_else(|| "response had no header terminator".to_owned())?;
    let head = String::from_utf8_lossy(&bytes[..split_at]);
    let body = &bytes[split_at + 4..];

    let status_line = head
        .lines()
        .next()
        .ok_or_else(|| "response had no status line".to_owned())?;
    let code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "response status line had no code".to_owned())?
        .parse::<u16>()
        .map_err(|_| format!("'{status_line}' did not contain a numeric status"))?;

    let body_status = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("status")?.as_str().map(str::to_owned));
    Ok((code, body_status))
}

/// Returns the index of the `\r\n\r\n` header terminator.
fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Decides the check status from the status code and any Actuator status.
fn evaluate(
    url: &str,
    status_code: u16,
    body_status: Option<&str>,
    expected: Option<u16>,
) -> CheckResult {
    let target = Some(url.to_owned());
    if let Some(status) = body_status {
        return match status.to_ascii_uppercase().as_str() {
            "UP" => CheckResult::healthy(target).with_message("the endpoint reports UP"),
            "DOWN" | "OUT_OF_SERVICE" => {
                CheckResult::unhealthy(target, format!("the endpoint reports {status}"))
            }
            other => CheckResult::unknown(target, format!("the endpoint reports {other}")),
        };
    }
    match expected {
        Some(expected) if status_code == expected => {
            CheckResult::healthy(target).with_message(format!("status {status_code}"))
        }
        Some(expected) => CheckResult::unhealthy(
            target,
            format!("expected status {expected}, received {status_code}"),
        ),
        None if (200..300).contains(&status_code) => {
            CheckResult::healthy(target).with_message(format!("status {status_code}"))
        }
        None => CheckResult::unhealthy(target, format!("received status {status_code}")),
    }
}

/// Parses an Actuator status value into a health state.
///
/// Exposed for tests and documentation of the mapping; the provider uses
/// [`evaluate`] directly.
#[must_use]
pub fn actuator_status(value: &str) -> HealthState {
    match value.to_ascii_uppercase().as_str() {
        "UP" => HealthState::Healthy,
        "DOWN" | "OUT_OF_SERVICE" => HealthState::Unhealthy,
        _ => HealthState::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{
        HealthCheck, HealthCheckKind, HttpHealthCheck, ServiceId, WorkspaceId,
    };
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    fn parts(url: &str, expected: Option<u16>) -> (WorkspaceId, ServiceId, HealthCheck) {
        (
            WorkspaceId::new("acme").unwrap(),
            ServiceId::new("auth-service").unwrap(),
            HealthCheck::new(HealthCheckKind::Http(
                HttpHealthCheck::new(url, expected).unwrap(),
            )),
        )
    }

    async fn serve(response: &'static [u8]) -> u16 {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buffer = [0u8; 1024];
                let _ = socket.read(&mut buffer).await;
                let _ = socket.write_all(response).await;
                let _ = socket.shutdown().await;
            }
        });
        port
    }

    #[test]
    fn parses_http_urls() {
        let target = parse_http_url("http://localhost:8081/actuator/health").unwrap();
        assert_eq!(target.host, "localhost");
        assert_eq!(target.port, 8081);
        assert_eq!(target.path, "/actuator/health");
    }

    #[test]
    fn rejects_https_and_relative_urls() {
        assert!(parse_http_url("https://localhost:8081/health").is_err());
        assert!(parse_http_url("/actuator/health").is_err());
    }

    #[test]
    fn strips_userinfo() {
        let target = parse_http_url("http://user:pass@localhost:9000/health").unwrap();
        assert_eq!(target.host, "localhost");
        assert_eq!(target.port, 9000);
        assert!(!build_request(&target).contains("pass"));
    }

    #[test]
    fn parses_actuator_status_from_the_body() {
        let (code, status) = parse_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"UP\"}",
        )
        .unwrap();
        assert_eq!(code, 200);
        assert_eq!(status.as_deref(), Some("UP"));
    }

    #[test]
    fn maps_actuator_status_values() {
        assert_eq!(actuator_status("UP"), HealthState::Healthy);
        assert_eq!(actuator_status("DOWN"), HealthState::Unhealthy);
        assert_eq!(actuator_status("OUT_OF_SERVICE"), HealthState::Unhealthy);
        assert_eq!(actuator_status("UNKNOWN"), HealthState::Unknown);
    }

    #[test]
    fn a_200_with_a_down_body_is_unhealthy() {
        let result = evaluate("http://localhost/health", 200, Some("DOWN"), Some(200));
        assert_eq!(result.status, HealthState::Unhealthy);
    }

    #[test]
    fn an_unexpected_status_is_unhealthy() {
        let result = evaluate("http://localhost/health", 503, None, Some(200));
        assert_eq!(result.status, HealthState::Unhealthy);
    }

    #[tokio::test]
    async fn a_healthy_endpoint_is_reported_healthy() {
        let port = serve(b"HTTP/1.1 200 OK\r\n\r\n{\"status\":\"UP\"}").await;
        let (workspace, service, check) =
            parts(&format!("http://localhost:{port}/actuator/health"), None);
        let result = HttpHealthChecker::new()
            .check(&HealthCheckContext {
                workspace_id: &workspace,
                service_id: &service,
                check: &check,
                timeout: Duration::from_secs(2),
            })
            .await
            .unwrap();
        assert_eq!(result.status, HealthState::Healthy);
    }

    #[tokio::test]
    async fn an_unhealthy_endpoint_is_reported_unhealthy() {
        let port = serve(b"HTTP/1.1 200 OK\r\n\r\n{\"status\":\"DOWN\"}").await;
        let (workspace, service, check) =
            parts(&format!("http://localhost:{port}/actuator/health"), None);
        let result = HttpHealthChecker::new()
            .check(&HealthCheckContext {
                workspace_id: &workspace,
                service_id: &service,
                check: &check,
                timeout: Duration::from_secs(2),
            })
            .await
            .unwrap();
        assert_eq!(result.status, HealthState::Unhealthy);
    }

    #[tokio::test]
    async fn a_refused_connection_is_unreachable() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let (workspace, service, check) = parts(&format!("http://localhost:{port}/health"), None);
        let result = HttpHealthChecker::new()
            .check(&HealthCheckContext {
                workspace_id: &workspace,
                service_id: &service,
                check: &check,
                timeout: Duration::from_secs(1),
            })
            .await
            .unwrap();
        assert_eq!(result.status, HealthState::Unreachable);
    }

    #[tokio::test]
    async fn an_https_url_is_a_configuration_error() {
        let (workspace, service, check) = parts("https://localhost:8443/health", None);
        let error = HttpHealthChecker::new()
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
