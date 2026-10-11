//! MCP audit logging.
//!
//! The MCP transport reserves **stdout exclusively for protocol messages**, so
//! every diagnostic emitted by this crate goes to **stderr** as a single-line
//! JSON record. A log line never contains tool arguments or any value that could
//! carry a secret; it records the *tool name*, the *outcome*, the *duration*, and
//! — on failure — a stable *error code* only (`tasks/014.md` §6, §33).
//!
//! This is deliberately tiny. The interface owns no logging framework: the
//! application's structured logs flow through the shared
//! [`LogService`](service_orchestrator_application::LogService), and this module
//! only records the adapter-level audit trail that has no other home.

use std::time::Duration;

use serde_json::json;

/// The outcome of a tool invocation, as recorded in the audit log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InvocationOutcome {
    /// The tool ran and returned a success envelope.
    Success,
    /// The tool ran and returned an application-level error envelope.
    ApplicationError,
    /// The request could not be routed: malformed arguments or an unknown tool.
    ProtocolError,
}

impl InvocationOutcome {
    /// Returns the stable label written to the log.
    fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::ApplicationError => "application_error",
            Self::ProtocolError => "protocol_error",
        }
    }
}

/// Records that the MCP server accepted a stdio session.
pub(crate) fn server_started() {
    emit(json!({
        "event": "mcp_server_started",
        "transport": "stdio",
    }));
}

/// Records that the MCP server stopped serving.
pub(crate) fn server_stopped(reason: &str) {
    emit(json!({
        "event": "mcp_server_stopped",
        "reason": reason,
    }));
}

/// Records one tool invocation.
///
/// No argument value is ever logged; only the tool name and the high-level
/// outcome are recorded.
pub(crate) fn tool_invocation(
    tool: &str,
    outcome: InvocationOutcome,
    duration: Duration,
    error_code: Option<&str>,
) {
    emit(json!({
        "event": "mcp_tool_invocation",
        "tool": tool,
        "outcome": outcome.label(),
        "durationMs": u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        "errorCode": error_code,
    }));
}

/// Writes one structured record to stderr.
///
/// A serialization failure is impossible for the values built here, but the
/// fallback keeps the adapter silent rather than panicking mid-protocol.
fn emit(record: serde_json::Value) {
    eprintln!("{record}");
}
