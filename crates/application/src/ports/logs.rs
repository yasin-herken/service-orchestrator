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
//!
//! # Structured entries
//!
//! A [`LogEntry`] carries the source, the optional correlation identifiers
//! (workspace, service, task, process), the originating stream for process
//! output, and a strictly increasing `sequence` used for stable cursor
//! pagination. Not every entry has every identifier: an application log has no
//! process, and a process log has no task (`tasks/012.md` §21). Adapters must
//! not fabricate metadata they do not have.
//!
//! # Streaming
//!
//! [`LogService::subscribe`] returns an interface-neutral [`LogSubscription`].
//! The Ratatui TUI, the MCP server, and a future CLI each consume the same
//! subscription; delivery is bounded and lossy under backpressure, so a slow
//! consumer never blocks a managed process's `stdout`/`stderr` reader
//! (`tasks/012.md` §22, §27).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{ServiceId, TaskId, Timestamp, WorkspaceId};
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

impl LogLevel {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Where a log entry originated.
///
/// The source lets an interface filter "show me build output" separately from
/// "show me application errors" without parsing message text
/// (`tasks/012.md` §24).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogSource {
    /// Output captured from a managed process.
    Process,
    /// A message emitted by a task executor.
    Task,
    /// A service runtime lifecycle event.
    Runtime,
    /// A Git operation.
    Git,
    /// A build operation.
    Build,
    /// A library operation.
    Library,
    /// A database migration operation.
    Migration,
    /// An application-level message that belongs to no other source.
    #[default]
    Application,
}

impl LogSource {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Task => "task",
            Self::Runtime => "runtime",
            Self::Git => "git",
            Self::Build => "build",
            Self::Library => "library",
            Self::Migration => "migration",
            Self::Application => "application",
        }
    }
}

/// Which process output stream a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

impl LogStream {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// A single structured log entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// When the entry was produced.
    pub timestamp: Timestamp,
    /// The severity of the entry.
    pub level: LogLevel,
    /// The log message, already redacted of obvious secret material.
    pub message: String,
    /// The workspace the entry belongs to, when known.
    #[serde(default)]
    pub workspace_id: Option<WorkspaceId>,
    /// The service the entry belongs to, when known.
    #[serde(default)]
    pub service_id: Option<ServiceId>,
    /// The task the entry belongs to, when known.
    #[serde(default)]
    pub task_id: Option<TaskId>,
    /// The managed process the entry belongs to, when known.
    #[serde(default)]
    pub process_id: Option<String>,
    /// Where the entry originated.
    #[serde(default)]
    pub source: LogSource,
    /// The originating output stream, for process entries.
    #[serde(default)]
    pub stream: Option<LogStream>,
    /// A strictly increasing sequence assigned when the entry is stored.
    ///
    /// It is the cursor for [`LogPage`] pagination and is assigned by the log
    /// store, never by the producer, so it is monotonic even under concurrency.
    #[serde(default)]
    pub sequence: u64,
}

impl LogEntry {
    /// Creates a log entry with only the mandatory fields set.
    #[must_use]
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            timestamp: Timestamp::now(),
            level,
            message: message.into(),
            workspace_id: None,
            service_id: None,
            task_id: None,
            process_id: None,
            source: LogSource::Application,
            stream: None,
            sequence: 0,
        }
    }

    /// Sets the workspace the entry belongs to.
    #[must_use]
    pub fn with_workspace(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    /// Sets the service the entry belongs to.
    #[must_use]
    pub fn with_service(mut self, service_id: ServiceId) -> Self {
        self.service_id = Some(service_id);
        self
    }

    /// Sets the task the entry belongs to.
    #[must_use]
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Sets the process the entry belongs to.
    #[must_use]
    pub fn with_process(mut self, process_id: impl Into<String>) -> Self {
        self.process_id = Some(process_id.into());
        self
    }

    /// Sets the entry's source.
    #[must_use]
    pub fn with_source(mut self, source: LogSource) -> Self {
        self.source = source;
        self
    }

    /// Sets the originating output stream.
    #[must_use]
    pub fn with_stream(mut self, stream: LogStream) -> Self {
        self.stream = Some(stream);
        self
    }
}

/// A bounded, filterable request for log entries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRequest {
    /// Restrict to one service, when set.
    #[serde(default)]
    pub service_id: Option<ServiceId>,
    /// Restrict to one task, when set.
    #[serde(default)]
    pub task_id: Option<TaskId>,
    /// Restrict to one managed process, when set.
    #[serde(default)]
    pub process_id: Option<String>,
    /// Restrict to one workspace, when set.
    #[serde(default)]
    pub workspace_id: Option<WorkspaceId>,
    /// Restrict to one source, when set.
    #[serde(default)]
    pub source: Option<LogSource>,
    /// Only return entries at or above this severity.
    #[serde(default)]
    pub minimum_level: Option<LogLevel>,
    /// A case-insensitive substring the message must contain, when set.
    #[serde(default)]
    pub text: Option<String>,
    /// Only return entries at or after this time.
    #[serde(default)]
    pub since: Option<Timestamp>,
    /// Only return entries at or before this time.
    #[serde(default)]
    pub until: Option<Timestamp>,
    /// Return only entries with a sequence strictly greater than this cursor.
    ///
    /// This is the stable cursor for [`LogPage`] pagination; it is ignored by
    /// [`LogService::query`].
    #[serde(default)]
    pub after_sequence: Option<u64>,
    /// The maximum number of entries to return. Implementations enforce a hard
    /// upper bound regardless of this value.
    #[serde(default)]
    pub limit: Option<usize>,
}

impl LogRequest {
    /// Creates a request that selects a service's logs.
    #[must_use]
    pub fn new(service_id: ServiceId) -> Self {
        Self {
            service_id: Some(service_id),
            ..Self::default()
        }
    }

    /// Creates a request with no selection; every entry is eligible.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Restricts the request to a service.
    #[must_use]
    pub fn for_service(mut self, service_id: ServiceId) -> Self {
        self.service_id = Some(service_id);
        self
    }

    /// Restricts the request to a task.
    #[must_use]
    pub fn for_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Restricts the request to a managed process.
    #[must_use]
    pub fn for_process(mut self, process_id: impl Into<String>) -> Self {
        self.process_id = Some(process_id.into());
        self
    }

    /// Restricts the request to a workspace.
    #[must_use]
    pub fn for_workspace(mut self, workspace_id: WorkspaceId) -> Self {
        self.workspace_id = Some(workspace_id);
        self
    }

    /// Restricts the request to a source.
    #[must_use]
    pub fn with_source(mut self, source: LogSource) -> Self {
        self.source = Some(source);
        self
    }

    /// Limits the number of returned entries.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Requests entries after the given cursor.
    #[must_use]
    pub fn after(mut self, sequence: u64) -> Self {
        self.after_sequence = Some(sequence);
        self
    }

    /// Filters to entries at or above `level`.
    #[must_use]
    pub fn with_minimum_level(mut self, level: LogLevel) -> Self {
        self.minimum_level = Some(level);
        self
    }

    /// Filters to entries whose message contains `text`.
    #[must_use]
    pub fn matching(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Filters to entries within the given time range.
    #[must_use]
    pub fn between(mut self, since: Timestamp, until: Timestamp) -> Self {
        self.since = Some(since);
        self.until = Some(until);
        self
    }
}

/// A stable page of log entries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPage {
    /// The matched entries, oldest first.
    pub entries: Vec<LogEntry>,
    /// The cursor to pass as [`LogRequest::after_sequence`] for the next page,
    /// or `None` when no further matching entries exist.
    pub next_cursor: Option<u64>,
    /// Whether more matching entries existed than the limit allowed.
    pub truncated: bool,
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

    /// The query is not valid.
    #[error("invalid log query: {message}")]
    QueryInvalid {
        /// What is wrong with the query.
        message: String,
    },

    /// The requested log source does not exist.
    #[error("log source '{name}' was not found")]
    SourceNotFound {
        /// The missing source.
        name: String,
    },

    /// The subscriber fell too far behind and lines were dropped.
    #[error("log retention limit reached: {message}")]
    RetentionExceeded {
        /// A human-readable description.
        message: String,
    },

    /// The log service does not support the requested operation.
    #[error("the log service does not support this operation")]
    Unsupported,

    /// The log service failed internally.
    #[error("log service error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

/// Implements a [`LogSubscription`].
#[async_trait]
pub trait LogSubscription: Send {
    /// Receives the next log entry.
    ///
    /// # Errors
    ///
    /// Returns [`LogStreamError::Closed`] when the producer has shut down and
    /// [`LogStreamError::Lagged`] when the subscriber fell behind and entries
    /// were dropped.
    async fn recv(&mut self) -> Result<LogEntry, LogStreamError>;
}

/// Errors produced while consuming a [`LogSubscription`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LogStreamError {
    /// The subscriber fell behind and entries were dropped.
    #[error("log subscription lagged; {dropped} entries were dropped")]
    Lagged {
        /// How many entries were dropped.
        dropped: u64,
    },
    /// The producer has shut down.
    #[error("log subscription closed")]
    Closed,
}

/// Retrieves and streams bounded, structured logs.
pub trait LogService: Send + Sync {
    /// Returns log entries matching `request`, newest first.
    ///
    /// # Errors
    ///
    /// Returns a [`LogServiceError`] if the logs cannot be retrieved.
    fn query(&self, request: &LogRequest) -> Result<Vec<LogEntry>, LogServiceError>;

    /// Returns one stable page of entries matching `request`.
    ///
    /// The default implementation wraps [`LogService::query`] and reports that
    /// no further cursor is available; a real store overrides it.
    ///
    /// # Errors
    ///
    /// Returns a [`LogServiceError`] if the page cannot be retrieved.
    fn page(&self, request: &LogRequest) -> Result<LogPage, LogServiceError> {
        let mut entries = self.query(request)?;
        entries.reverse();
        Ok(LogPage {
            entries,
            next_cursor: None,
            truncated: false,
        })
    }

    /// Subscribes to realtime entries for `service_id`.
    ///
    /// # Errors
    ///
    /// Returns [`LogServiceError::Unsupported`] when streaming is unavailable.
    fn subscribe(
        &self,
        service_id: &ServiceId,
    ) -> Result<Box<dyn LogSubscription>, LogServiceError> {
        let _ = service_id;
        Err(LogServiceError::Unsupported)
    }
}

/// A [`LogService`] that returns no logs and supports no streaming.
///
/// Useful as a default when log capture has not been wired up, and in tests
/// that do not exercise logging.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullLogService;

impl LogService for NullLogService {
    fn query(&self, _request: &LogRequest) -> Result<Vec<LogEntry>, LogServiceError> {
        Ok(Vec::new())
    }

    fn page(&self, _request: &LogRequest) -> Result<LogPage, LogServiceError> {
        Ok(LogPage::default())
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
        assert!(service.page(&request).unwrap().entries.is_empty());
        assert!(matches!(
            service.subscribe(&ServiceId::new("auth-service").unwrap()),
            Err(LogServiceError::Unsupported)
        ));
    }

    #[test]
    fn log_requests_are_built_with_filters() {
        let request = LogRequest::new(ServiceId::new("auth-service").unwrap())
            .with_limit(100)
            .with_minimum_level(LogLevel::Warn);
        assert_eq!(
            request.service_id.as_ref().map(ServiceId::as_str),
            Some("auth-service")
        );
        assert_eq!(request.limit, Some(100));
        assert_eq!(request.minimum_level, Some(LogLevel::Warn));
    }

    #[test]
    fn log_levels_are_ordered_by_severity() {
        assert!(LogLevel::Trace < LogLevel::Info);
        assert!(LogLevel::Warn < LogLevel::Error);
    }

    #[test]
    fn entries_are_built_with_correlation_metadata() {
        let entry = LogEntry::new(LogLevel::Info, "started")
            .with_service(ServiceId::new("auth-service").unwrap())
            .with_stream(LogStream::Stdout)
            .with_source(LogSource::Process);
        assert_eq!(entry.source, LogSource::Process);
        assert_eq!(entry.stream, Some(LogStream::Stdout));
        assert_eq!(
            entry.service_id.as_ref().map(ServiceId::as_str),
            Some("auth-service")
        );
    }

    #[test]
    fn task_requests_select_a_task() {
        let request = LogRequest::all().for_task(TaskId::new("task-1").unwrap());
        assert_eq!(request.task_id.as_ref().map(TaskId::as_str), Some("task-1"));
        assert!(request.service_id.is_none());
    }
}
