//! Task, workflow, and log read/write use cases behind the desktop commands.
//!
//! Git, build, and library operations all return task or workflow ids; this
//! module is the one place the GUI follows those ids. It reads a task, aggregates
//! a workflow, pages a task's structured logs, and requests cancellation through
//! the existing Task Engine APIs — the GUI never fabricates progress or claims a
//! build succeeded on its own (`tasks/020.md` §10, §17, §18, §27, §29).
//!
//! Logs are always bounded: a request carries a limit and a cursor, so the
//! frontend keeps a bounded buffer and follows a task with stable pagination
//! rather than unbounded memory (`tasks/020.md` §18, §35).

use service_orchestrator_application::{LogLevel, LogRequest};
use service_orchestrator_client::OrchestratorClient;

use crate::dto::{BulkSummaryView, LogPageView, TaskView, WorkflowCancellationView};
use crate::error::CommandError;
use crate::validation::{parse_task_id, parse_workflow_id};

/// The largest number of log entries a single page may return.
///
/// The core enforces its own hard upper bound; this keeps the adapter's default
/// small enough for a responsive viewer (`tasks/020.md` §35).
const MAX_LOG_LIMIT: usize = 1_000;

/// The default number of log entries per page.
const DEFAULT_LOG_LIMIT: usize = 500;

/// A bounded, filterable request for a task's logs.
///
/// The frontend supplies the cursor for follow mode, a severity floor for error
/// filtering, and a substring for search; every field is optional so the same
/// command serves an initial load and an incremental poll.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskLogsQuery {
    /// Return entries after this sequence cursor.
    pub after: Option<u64>,
    /// The maximum number of entries to return.
    pub limit: Option<usize>,
    /// Only return entries at or above this severity (`trace` … `error`).
    pub minimum_level: Option<String>,
    /// Only return entries whose message contains this text (case-insensitive).
    pub text: Option<String>,
}

/// Returns one task by id.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected task id, or the daemon's structured
/// error (for example an unknown task) for anything else.
pub fn get_task(client: &dyn OrchestratorClient, task_id: &str) -> Result<TaskView, CommandError> {
    let task_id = parse_task_id(task_id)?;
    let task = client.get_task(&task_id).map_err(CommandError::from)?;
    Ok(TaskView::from(&task))
}

/// Aggregates a workflow's tasks into a bulk-operation summary.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workflow id, or the daemon's
/// structured error for anything else.
pub fn summarize_workflow(
    client: &dyn OrchestratorClient,
    workflow_id: &str,
) -> Result<BulkSummaryView, CommandError> {
    let workflow_id = parse_workflow_id(workflow_id)?;
    let summary = client
        .summarize_workflow(&workflow_id)
        .map_err(CommandError::from)?;
    Ok(BulkSummaryView::from(summary))
}

/// Returns one bounded page of a task's structured logs.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected task id or severity, or the daemon's
/// structured error for anything else.
pub fn get_task_logs(
    client: &dyn OrchestratorClient,
    task_id: &str,
    query: &TaskLogsQuery,
) -> Result<LogPageView, CommandError> {
    let task_id = parse_task_id(task_id)?;
    let level = query
        .minimum_level
        .as_deref()
        .map(parse_level)
        .transpose()?;
    let limit = query.limit.unwrap_or(DEFAULT_LOG_LIMIT).min(MAX_LOG_LIMIT);

    let mut request = LogRequest::all()
        .for_task(task_id.clone())
        .with_limit(limit);
    if let Some(after) = query.after {
        request = request.after(after);
    }
    if let Some(level) = level {
        request = request.with_minimum_level(level);
    }
    if let Some(text) = query.text.as_ref().filter(|text| !text.is_empty()) {
        request = request.matching(text.clone());
    }

    let page = client
        .get_task_logs(&task_id, &request)
        .map_err(CommandError::from)?;
    Ok(LogPageView::from(page))
}

/// Requests cancellation of one task and returns its latest observed state.
///
/// Cancellation is best-effort and never rolls back work already done, so the
/// returned task reflects whatever the engine observed — the GUI reports partial
/// completion accurately rather than assuming a rollback
/// (`tasks/020.md` §29).
///
/// # Errors
///
/// Same as [`get_task`], plus the daemon's structured error when cancellation is
/// rejected.
pub fn cancel_task(
    client: &dyn OrchestratorClient,
    task_id: &str,
) -> Result<TaskView, CommandError> {
    let task_id = parse_task_id(task_id)?;
    client.cancel_task(&task_id).map_err(CommandError::from)?;
    let task = client.get_task(&task_id).map_err(CommandError::from)?;
    Ok(TaskView::from(&task))
}

/// Requests cancellation of every active task in a workflow.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workflow id, or the daemon's
/// structured error for anything else.
pub fn cancel_workflow(
    client: &dyn OrchestratorClient,
    workflow_id: &str,
) -> Result<WorkflowCancellationView, CommandError> {
    let workflow_id = parse_workflow_id(workflow_id)?;
    let cancellation = client
        .cancel_workflow(&workflow_id)
        .map_err(CommandError::from)?;
    Ok(WorkflowCancellationView::from(cancellation))
}

/// Parses a severity label into a [`LogLevel`].
///
/// # Errors
///
/// Returns `INVALID_INPUT` for an unknown severity.
fn parse_level(raw: &str) -> Result<LogLevel, CommandError> {
    match raw.trim() {
        "trace" => Ok(LogLevel::Trace),
        "debug" => Ok(LogLevel::Debug),
        "info" => Ok(LogLevel::Info),
        "warn" => Ok(LogLevel::Warn),
        "error" => Ok(LogLevel::Error),
        other => Err(CommandError::invalid_input(format!(
            "unknown log level '{other}' (expected trace, debug, info, warn, or error)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_levels_parse_and_unknown_are_rejected() {
        assert_eq!(parse_level("error").expect("valid"), LogLevel::Error);
        assert_eq!(parse_level(" warn ").expect("valid"), LogLevel::Warn);
        let error = parse_level("verbose").expect_err("unknown level");
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn invalid_ids_are_rejected_before_any_ipc_call() {
        assert_eq!(parse_task_id("").expect_err("empty").code, "INVALID_INPUT");
        assert_eq!(
            parse_workflow_id("bad id").expect_err("space").code,
            "INVALID_INPUT"
        );
    }
}
