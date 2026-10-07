//! Log retrieval boundary.
//!
//! Service logs are produced by running processes and captured by the process
//! infrastructure. The application does not read files or pipe output itself;
//! it asks a [`LogService`] for a bounded, structured slice of a service's log
//! and returns it to the caller.
//!
//! Log retrieval is a read operation: it must never start, stop, or probe a
//! service (`AGENTS.md` §35). Results are always bounded so a query cannot
//! return unbounded content through MCP.

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{ServiceId, Timestamp};
use thiserror::Error;

/// The severity of a log entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Fine-grained diagnostic output.
    Trace,
    /// Development diagnostic output.
    Debug,
    /// Normal operational output.
    Info,
    /// A recoverable problem.
    Warn,
    /// A failure.
    Error,
}

/// A single structured log entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// When the entry was produced.
    pub timestamp: Timestamp,
    /// The severity of the entry.
    pub level: LogLevel,
    /// The log message.
    pub message: String,
}

/// A bounded request for a service's logs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRequest {
    /// The service whose logs are requested.
    pub service_id: ServiceId,
    /// The maximum number of entries to return, newest first.
    pub limit: Option<usize>,
    /// Only return entries at or above this severity.
    pub minimum_level: Option<LogLevel>,
}

impl LogRequest {
    /// Creates a request for a service's logs with no limit or level filter.
    #[must_use]
    pub fn new(service_id: ServiceId) -> Self {
        Self {
            service_id,
            limit: None,
            minimum_level: None,
        }
    }

    /// Limits the number of returned entries.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Filters to entries at or above `level`.
    #[must_use]
    pub fn with_minimum_level(mut self, level: LogLevel) -> Self {
        self.minimum_level = Some(level);
        self
    }
}

/// Errors returned by a [`LogService`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LogServiceError {
    /// No logs are available for the service.
    #[error("no logs available for service '{service_id}'")]
    NoLogs {
        /// The service whose logs were requested.
        service_id: ServiceId,
    },

    /// The log service failed internally.
    #[error("log service error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

/// Retrieves bounded, structured logs for a service.
pub trait LogService: Send + Sync {
    /// Returns log entries matching `request`.
    ///
    /// # Errors
    ///
    /// Returns a [`LogServiceError`] if the logs cannot be retrieved.
    fn query(&self, request: &LogRequest) -> Result<Vec<LogEntry>, LogServiceError>;
}

/// A [`LogService`] that returns no logs.
///
/// Useful as a default when log capture has not been wired up, and in tests
/// that do not exercise logging.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullLogService;

impl LogService for NullLogService {
    fn query(&self, _request: &LogRequest) -> Result<Vec<LogEntry>, LogServiceError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_log_service_returns_nothing() {
        let service = NullLogService;
        let request = LogRequest::new(ServiceId::new("auth-service").unwrap());
        assert!(service.query(&request).unwrap().is_empty());
    }

    #[test]
    fn log_requests_are_built_with_filters() {
        let request = LogRequest::new(ServiceId::new("auth-service").unwrap())
            .with_limit(100)
            .with_minimum_level(LogLevel::Warn);
        assert_eq!(request.limit, Some(100));
        assert_eq!(request.minimum_level, Some(LogLevel::Warn));
    }

    #[test]
    fn log_levels_are_ordered_by_severity() {
        assert!(LogLevel::Trace < LogLevel::Info);
        assert!(LogLevel::Warn < LogLevel::Error);
    }
}
