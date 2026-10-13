//! Unified log retrieval behind the desktop Logs screen.
//!
//! One command answers three scopes through the existing Observability
//! boundary: a task's logs, one service's logs, or the workspace-wide
//! application log (`tasks/021.md` §23, §27). Every request is bounded by a
//! limit and a cursor; the GUI never reads files and the adapter never stores
//! logs of its own.
//!
//! # Workspace scope is resolved from configuration
//!
//! Producers rarely know which workspace an entry belongs to — a task line
//! carries its task and service, a process line its process and service — so
//! filtering on a workspace tag alone would silently return nothing. A
//! workspace-scoped query therefore resolves the workspace's services and keeps
//! the entries that carry the workspace tag, belong to one of those services,
//! or carry no service at all (application-level lines, which are
//! workspace-agnostic).

use std::collections::HashSet;

use service_orchestrator_application::{LogRequest, LogSource};
use service_orchestrator_client::OrchestratorClient;

use crate::dto::{LogEntryView, LogPageView};
use crate::error::CommandError;
use crate::tasks::parse_level;
use crate::validation::{parse_service_id, parse_task_id, parse_workspace_id};

/// The default number of entries per unified page.
const DEFAULT_LOG_LIMIT: usize = 500;

/// The hard cap on entries per unified page.
const MAX_LOG_LIMIT: usize = 1_000;

/// A bounded, filterable query for the unified Logs screen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogQuery {
    /// Scope to one task's logs, when set (takes precedence over `service_id`).
    pub task_id: Option<String>,
    /// Scope to one service's logs, when set.
    pub service_id: Option<String>,
    /// Scope to one workspace's entries, when set.
    pub workspace_id: Option<String>,
    /// Only return entries from this source (`process`, `task`, `build`, …).
    pub source: Option<String>,
    /// Only return entries at or above this severity.
    pub minimum_level: Option<String>,
    /// Only return entries whose message contains this text.
    pub text: Option<String>,
    /// Return entries after this sequence cursor (follow/pagination).
    pub after: Option<u64>,
    /// The maximum number of entries to return.
    pub limit: Option<usize>,
}

/// Answers one bounded, cursor-paginated log query.
///
/// Task scope delegates to the task-log page (stable cursor); service scope
/// asks the service's page; everything else is a bounded workspace-scoped
/// query whose continuation cursor is the highest scanned sequence. Workspace
/// membership is resolved from configuration, because producers rarely tag an
/// entry with a workspace (see the module documentation).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected identifier or severity, or the
/// daemon's structured error for anything else.
pub fn get_logs(
    client: &dyn OrchestratorClient,
    query: &LogQuery,
) -> Result<LogPageView, CommandError> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LOG_LIMIT)
        .clamp(1, MAX_LOG_LIMIT);

    if let Some(task_id) = query.task_id.as_deref().filter(|id| !id.trim().is_empty()) {
        parse_task_id(task_id)?;
        return crate::tasks::get_task_logs(
            client,
            task_id,
            &crate::tasks::TaskLogsQuery {
                after: query.after,
                limit: Some(limit),
                minimum_level: query.minimum_level.clone(),
                text: query.text.clone(),
            },
        );
    }

    if let Some(service_id) = query
        .service_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
    {
        let service_id = parse_service_id(service_id)?;
        let request = build_request(query, limit)?;
        let page = client
            .get_service_logs(&service_id, &request)
            .map_err(CommandError::from)?;
        return Ok(LogPageView::from(page));
    }

    let request = build_request(query, limit)?;
    let mut entries = client.get_logs(&request).map_err(CommandError::from)?;
    // A full scanned window may mean more entries exist; the cursor is the
    // newest scanned sequence, so the frontend pages forward without assuming
    // completeness (`tasks/021.md` §27).
    let truncated = entries.len() >= limit;
    let next_cursor = entries.first().map(|entry| entry.sequence);
    if let Some(workspace_id) = query.workspace_id.as_deref() {
        let workspace_id = parse_workspace_id(workspace_id)?;
        let services = client
            .list_services(&workspace_id)
            .map_err(CommandError::from)?
            .iter()
            .map(|service| service.id.to_string())
            .collect::<HashSet<_>>();
        entries.retain(|entry| match (&entry.workspace_id, &entry.service_id) {
            (Some(tagged), _) => tagged == &workspace_id,
            (None, Some(service)) => services.contains(service.as_str()),
            // An application-level line carries no service and is not scoped to
            // one workspace, so every workspace view may show it.
            (None, None) => true,
        });
    }
    // `LogService::query` answers newest-first; the viewer reads oldest-first.
    entries.reverse();
    Ok(LogPageView {
        entries: entries.iter().map(LogEntryView::from).collect(),
        next_cursor,
        truncated,
    })
}

/// Builds the filterable part of a log request from a query.
fn build_request(query: &LogQuery, limit: usize) -> Result<LogRequest, CommandError> {
    let mut request = LogRequest::all().with_limit(limit);
    if let Some(source) = query.source.as_deref().filter(|s| !s.trim().is_empty()) {
        request = request.with_source(parse_source(source)?);
    }
    if let Some(level) = query
        .minimum_level
        .as_deref()
        .filter(|level| !level.trim().is_empty())
    {
        request = request.with_minimum_level(parse_level(level)?);
    }
    if let Some(text) = query.text.as_deref().filter(|text| !text.is_empty()) {
        request = request.matching(text.to_owned());
    }
    if let Some(after) = query.after {
        request = request.after(after);
    }
    Ok(request)
}

/// Parses a source label into a [`LogSource`].
///
/// # Errors
///
/// Returns `INVALID_INPUT` for an unknown source label.
fn parse_source(raw: &str) -> Result<LogSource, CommandError> {
    match raw.trim() {
        "process" => Ok(LogSource::Process),
        "task" => Ok(LogSource::Task),
        "runtime" => Ok(LogSource::Runtime),
        "git" => Ok(LogSource::Git),
        "build" => Ok(LogSource::Build),
        "library" => Ok(LogSource::Library),
        "migration" => Ok(LogSource::Migration),
        "application" => Ok(LogSource::Application),
        other => Err(CommandError::invalid_input(format!(
            "unknown log source '{other}' (expected process, task, runtime, git, build, library, migration, or application)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use service_orchestrator_application::LogLevel;

    use super::*;

    #[test]
    fn known_sources_and_levels_parse_and_unknown_are_rejected() {
        assert_eq!(parse_source("process").expect("valid"), LogSource::Process);
        assert_eq!(
            parse_source("migration").expect("valid"),
            LogSource::Migration
        );
        let error = parse_source("database").expect_err("unknown source");
        assert_eq!(error.code, "INVALID_INPUT");

        assert_eq!(parse_level("error").expect("valid"), LogLevel::Error);
        assert_eq!(parse_level("warn").expect("valid"), LogLevel::Warn);
    }

    #[test]
    fn the_request_carries_every_filter() {
        let query = LogQuery {
            task_id: None,
            service_id: Some("orders-api".to_owned()),
            workspace_id: None,
            source: Some("process".to_owned()),
            minimum_level: Some("warn".to_owned()),
            text: Some("timeout".to_owned()),
            after: Some(42),
            limit: Some(100),
        };
        let request = build_request(&query, 100).expect("a request");
        assert_eq!(request.source, Some(LogSource::Process));
        assert_eq!(request.minimum_level, Some(LogLevel::Warn));
        assert_eq!(request.text.as_deref(), Some("timeout"));
        assert_eq!(request.after_sequence, Some(42));
        assert_eq!(request.limit, Some(100));
    }

    #[test]
    fn identifiers_are_validated() {
        assert_eq!(
            parse_service_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
    }
}
