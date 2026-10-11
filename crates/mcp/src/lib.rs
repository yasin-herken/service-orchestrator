//! # MCP
//!
//! MCP interface adapter (stdio transport).
//!
//! A thin adapter that validates input, applies policy, and forwards calls to
//! application use cases. It must not implement business logic or call
//! infrastructure directly. It depends only on the application core.
//!
//! # What this crate is
//!
//! `service-orchestrator mcp` speaks the Model Context Protocol over stdio so an
//! AI agent can drive the same operations a human drives through the TUI. The
//! adapter is deliberately thin:
//!
//! ```text
//! MCP request ─▶ validation ─▶ Application use case ─▶ policy ─▶ task engine
//! ```
//!
//! It never runs `git`, Maven, npm, Liquibase, or a process itself, and it never
//! contains a second engine, scheduler, or workflow manager. Every mutation it
//! accepts becomes a task or workflow the shared [`TaskEngine`] executes
//! (`AGENTS.md` §1, §21; `tasks/014.md` §2, §3).
//!
//! # Wire discipline
//!
//! On stdio, **stdout carries protocol messages only**. Every diagnostic this
//! crate emits goes to stderr as a single-line JSON record (`logging`), and no
//! tool argument is ever logged. A stray `println!` would corrupt the stream, so
//! none exists (`tasks/014.md` §6).
//!
//! # Authorization
//!
//! The server always runs with [`Authorization::standard`]: it can read and
//! stage, but an operation that requires [`Permission::Destructive`] is denied
//! with a structured `CONFIRMATION_REQUIRED` error. An agent cannot self-confirm
//! — there is no `confirmed` flag to set — so a human must approve destructive
//! work interactively (`AGENTS.md` §29, §30; `tasks/014.md` §25).
//!
//! [`TaskEngine`]: service_orchestrator_execution::TaskEngine
//! [`Authorization::standard`]: service_orchestrator_application::Authorization::standard
//! [`Permission::Destructive`]: service_orchestrator_domain::Permission::Destructive

use std::sync::Arc;

use rmcp::ErrorData;
use service_orchestrator_application::{Application, ApplicationError};
use service_orchestrator_domain::{
    Branch, DomainError, GroupId, LibraryId, ProfileId, ServiceId, TaskId, WorkflowId, WorkspaceId,
};

mod catalog;
mod logging;
mod monitor;
mod params;
mod prompts;
mod resources;
mod response;
mod server;

#[cfg(test)]
mod tests;

pub use monitor::TaskMonitor;

/// The static `initialize`/metadata resource URI.
pub(crate) const SERVER_URI: &str = "orchestrator://server";

/// The MIME type every structured resource and payload is served as.
pub(crate) const MIME_JSON: &str = "application/json";

/// A failure while serving the MCP transport.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The client never completed the `initialize` handshake, or the transport
    /// failed while negotiating it.
    ///
    /// The SDK error is boxed: it is large, and this enum is returned by value.
    #[error("failed to start the MCP server: {0}")]
    Initialize(Box<rmcp::service::ServerInitializeError>),

    /// The serving task stopped before a graceful shutdown.
    #[error("the MCP server stopped unexpectedly: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl From<rmcp::service::ServerInitializeError> for ServeError {
    fn from(error: rmcp::service::ServerInitializeError) -> Self {
        Self::Initialize(Box::new(error))
    }
}

/// Serves the MCP protocol on stdin/stdout until the client disconnects.
///
/// The application core is shared with whatever else the composition root wired
/// (the same task engine, runtime state, and log service), so an MCP-driven
/// operation observes and is observed by the same state a human would see. The
/// `monitor` is the read-only task-engine view backing `list_tasks`
/// (`tasks/014.md` §3, §17).
///
/// # Errors
///
/// Returns [`ServeError`] when the transport cannot be initialized or when the
/// serving task fails. A normal client disconnect is not an error.
pub async fn serve_stdio(
    application: Application,
    monitor: Arc<dyn TaskMonitor>,
) -> Result<(), ServeError> {
    let handler = server::OrchestratorServer::new(application, monitor);
    let running = rmcp::serve_server(handler, rmcp::transport::stdio()).await?;
    let reason = running.waiting().await?;
    logging::server_stopped(&format!("{reason:?}"));
    Ok(())
}

/// An error raised while routing or running one tool call.
///
/// The three variants exist because the protocol separates them: a shape
/// mismatch never routed (protocol error), a valid-but-invalid domain value
/// routed but was rejected (`INVALID_ARGUMENT`), and an application refusal or
/// failure produced a structured error envelope.
#[derive(Debug)]
pub(crate) enum ToolError {
    /// The request could not be routed: malformed arguments or an unknown tool.
    Protocol(ErrorData),
    /// The request routed, but an identifier or enum value was not valid.
    InvalidArgument {
        /// Why the value was rejected.
        message: String,
    },
    /// The application core refused or failed the operation.
    Application(ApplicationError),
}

impl ToolError {
    /// Builds an [`ToolError::InvalidArgument`].
    pub(crate) fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
        }
    }
}

impl From<ApplicationError> for ToolError {
    fn from(error: ApplicationError) -> Self {
        Self::Application(error)
    }
}

impl From<DomainError> for ToolError {
    fn from(error: DomainError) -> Self {
        Self::invalid_argument(error.to_string())
    }
}

/// Parses a workspace identifier from an untrusted string.
pub(crate) fn parse_workspace_id(raw: &str) -> Result<WorkspaceId, ToolError> {
    WorkspaceId::new(raw).map_err(ToolError::from)
}

/// Parses a service identifier from an untrusted string.
pub(crate) fn parse_service_id(raw: &str) -> Result<ServiceId, ToolError> {
    ServiceId::new(raw).map_err(ToolError::from)
}

/// Parses a library identifier from an untrusted string.
pub(crate) fn parse_library_id(raw: &str) -> Result<LibraryId, ToolError> {
    LibraryId::new(raw).map_err(ToolError::from)
}

/// Parses a group identifier from an untrusted string.
pub(crate) fn parse_group_id(raw: &str) -> Result<GroupId, ToolError> {
    GroupId::new(raw).map_err(ToolError::from)
}

/// Parses a profile identifier from an untrusted string.
pub(crate) fn parse_profile_id(raw: &str) -> Result<ProfileId, ToolError> {
    ProfileId::new(raw).map_err(ToolError::from)
}

/// Parses a task identifier from an untrusted string.
pub(crate) fn parse_task_id(raw: &str) -> Result<TaskId, ToolError> {
    TaskId::new(raw).map_err(ToolError::from)
}

/// Parses a workflow identifier from an untrusted string.
pub(crate) fn parse_workflow_id(raw: &str) -> Result<WorkflowId, ToolError> {
    WorkflowId::new(raw).map_err(ToolError::from)
}

/// Parses a branch name from an untrusted string.
pub(crate) fn parse_branch(raw: &str) -> Result<Branch, ToolError> {
    Branch::new(raw).map_err(ToolError::from)
}
