//! # Logging
//!
//! Infrastructure for structured logging and bounded, filterable log retrieval.
//!
//! The observability layer unifies the log sources the platform already
//! produces — managed process output, task-engine events, and application
//! messages — into a single structured model. It is interface-neutral: the
//! Ratatui TUI, the MCP server, and a future CLI all read the same
//! [`LogService`](service_orchestrator_application::LogService) and the same
//! realtime [`LogSubscription`](service_orchestrator_application::LogSubscription).
//!
//! # Architecture
//!
//! ```text
//! ProcessManager ──> ProcessLogIngestor ─┐
//! TaskEngine     ──> TaskLogIngestor    ─┼──> LogHub ──> LogService (query/page)
//! Application    ──> LogHub::record     ─┘         │
//!                                                  └──> per-service broadcast
//!                                                       (LogSubscription)
//! ```
//!
//! The [`LogHub`] is the in-memory store: it assigns monotonic sequences,
//! redacts and truncates messages, enforces bounded retention, answers filtered
//! queries and cursor pages, and broadcasts live entries to subscribers. The
//! ingestors only translate existing events; they never duplicate the process
//! manager's or the task engine's responsibilities (`tasks/012.md` §20, §27,
//! §28).
//!
//! See `docs/observability.md` and `docs/adr/012-health-observability.md`.

#![deny(missing_docs)]

pub mod hub;
pub mod ingest;
pub mod redact;
pub mod subscription;

pub use hub::{
    LogHub, LogRetention, DEFAULT_MAX_MESSAGE_LEN, DEFAULT_MAX_QUERY_LIMIT,
    DEFAULT_PER_SERVICE_ENTRIES, DEFAULT_QUERY_LIMIT, DEFAULT_SUBSCRIBER_CAPACITY,
    DEFAULT_TOTAL_ENTRIES,
};
pub use ingest::{
    entry_from_process_line, entry_from_task_event, spawn_process_ingestor, spawn_task_ingestor,
    task_event_id, task_event_service, ProcessLogIngestor, TaskLogIngestor,
};
pub use redact::{is_clean, redact};
pub use subscription::ChannelLogSubscription;
