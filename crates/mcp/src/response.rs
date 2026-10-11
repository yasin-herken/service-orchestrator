//! The MCP response envelope and its typed views.
//!
//! Every tool result is wrapped in one of two envelopes (`tasks/014.md` §16,
//! §28):
//!
//! ```json
//! { "success": true,  "data": { ... } }
//! { "success": false, "error": { "code": "...", "message": "...", "details": { ... } } }
//! ```
//!
//! A success envelope is returned as a normal tool result; a failure envelope is
//! returned with `isError: true`. Neither is a protocol error: the request was
//! well formed and the application ran, it simply refused or failed. Protocol
//! errors are reserved for requests that could not be routed at all
//! (`server.rs`).
//!
//! Only a handful of values get an explicit view. The task asks for `taskId` and
//! a live `status` on a scheduled task, so [`operation_result`] and [`task`]
//! render a stable, `camelCase` view. Every other result is the application's or
//! domain's own serialization, passed through unchanged — the adapter adds no
//! business meaning (`tasks/014.md` §2, §19).

use serde::Serialize;
use serde_json::{json, Value};
use service_orchestrator_application::{
    ApplicationError, LogEntry, LogPage, LogStream, OperationResult,
};
use service_orchestrator_domain::{Permission, Task, Timestamp};

/// Wraps a successful payload in the success envelope.
pub(crate) fn success(value: Value) -> Value {
    json!({ "success": true, "data": value })
}

/// Wraps an error body in the failure envelope.
pub(crate) fn failure(error: ErrorBody) -> Value {
    json!({ "success": false, "error": error })
}

/// The structured body of a failure envelope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorBody {
    /// A stable, machine-readable error code.
    pub(crate) code: String,
    /// A human-readable summary.
    pub(crate) message: String,
    /// Optional structured context, such as the expected and found versions of
    /// an artifact mismatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) details: Option<Value>,
}

impl ErrorBody {
    /// Builds a body with no extra details.
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    /// Attaches structured details.
    pub(crate) fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// Maps an [`ApplicationError`] into a structured MCP error body.
///
/// The mapping reuses the application's own stable codes where the application
/// already defines one (Git, build, migration, health, artifact), and gives
/// every other variant a dedicated code. A destructive-permission denial is
/// reported as `CONFIRMATION_REQUIRED`: the request is valid and policy is the
/// only obstacle (`tasks/014.md` §15, §25).
pub(crate) fn error_body(error: &ApplicationError) -> ErrorBody {
    match error {
        ApplicationError::WorkspaceNotFound(_) => {
            ErrorBody::new("WORKSPACE_NOT_FOUND", error.to_string())
        }
        ApplicationError::ServiceNotFound(_) => {
            ErrorBody::new("SERVICE_NOT_FOUND", error.to_string())
        }
        ApplicationError::LibraryNotFound(_) => {
            ErrorBody::new("LIBRARY_NOT_FOUND", error.to_string())
        }
        ApplicationError::ProfileNotFound(_) => {
            ErrorBody::new("PROFILE_NOT_FOUND", error.to_string())
        }
        ApplicationError::GroupNotFound(_) => ErrorBody::new("GROUP_NOT_FOUND", error.to_string()),
        ApplicationError::TaskNotFound(_) => ErrorBody::new("TASK_NOT_FOUND", error.to_string()),
        ApplicationError::WorkflowNotFound(_) => {
            ErrorBody::new("WORKFLOW_NOT_FOUND", error.to_string())
        }
        ApplicationError::InvalidOperation { message } => {
            ErrorBody::new("INVALID_OPERATION", message.clone())
        }
        ApplicationError::DependencyNotFound { owner, target } => {
            ErrorBody::new("DEPENDENCY_NOT_FOUND", error.to_string())
                .with_details(json!({ "owner": owner, "target": target }))
        }
        ApplicationError::ConfigurationInvalid { message } => {
            ErrorBody::new("CONFIGURATION_INVALID", message.clone())
        }
        ApplicationError::WorkflowInvalid { message } => {
            ErrorBody::new("WORKFLOW_INVALID", message.clone())
        }
        ApplicationError::PermissionDenied {
            permission,
            message,
        } => {
            let code = if *permission == Permission::Destructive {
                "CONFIRMATION_REQUIRED"
            } else {
                "PERMISSION_DENIED"
            };
            ErrorBody::new(code, message.clone())
        }
        ApplicationError::TaskManager { message } => {
            ErrorBody::new("TASK_ENGINE_ERROR", message.clone())
        }
        ApplicationError::LogService { message } => {
            ErrorBody::new("LOG_SERVICE_ERROR", message.clone())
        }
        ApplicationError::Git { code, message } | ApplicationError::Build { code, message } => {
            ErrorBody::new(code.clone(), message.clone())
        }
        ApplicationError::LibraryConfigurationInvalid { library, message } => {
            ErrorBody::new("LIBRARY_CONFIGURATION_INVALID", message.clone())
                .with_details(json!({ "library": library }))
        }
        ApplicationError::BuildProviderUnsupported { library, operation } => {
            ErrorBody::new("BUILD_PROVIDER_UNSUPPORTED", error.to_string())
                .with_details(json!({ "library": library, "operation": operation }))
        }
        ApplicationError::ArtifactNotFound {
            library,
            coordinate,
        } => ErrorBody::new("ARTIFACT_NOT_FOUND", error.to_string())
            .with_details(json!({ "library": library, "coordinate": coordinate })),
        ApplicationError::ArtifactVersionMismatch {
            library,
            expected,
            found,
        } => ErrorBody::new("ARTIFACT_VERSION_MISMATCH", error.to_string())
            .with_details(json!({ "library": library, "expected": expected, "found": found })),
        ApplicationError::DependencyResolution { library, message } => {
            ErrorBody::new("DEPENDENCY_RESOLUTION_FAILED", message.clone())
                .with_details(json!({ "library": library }))
        }
        ApplicationError::DependencyCycle { libraries } => {
            ErrorBody::new("DEPENDENCY_CYCLE", error.to_string())
                .with_details(json!({ "libraries": libraries }))
        }
        ApplicationError::ServiceDependencyCycle { services } => {
            ErrorBody::new("SERVICE_DEPENDENCY_CYCLE", error.to_string())
                .with_details(json!({ "services": services }))
        }
        ApplicationError::VersionInspectionUnsupported { library, message } => {
            ErrorBody::new("VERSION_INSPECTION_UNSUPPORTED", message.clone())
                .with_details(json!({ "library": library }))
        }
        ApplicationError::ArtifactInspection { code, message }
        | ApplicationError::ServiceRuntime { code, message }
        | ApplicationError::Migration { code, message }
        | ApplicationError::Health { code, message } => {
            ErrorBody::new(code.clone(), message.clone())
        }
        ApplicationError::DiagnosticsUnavailable { service, message } => {
            ErrorBody::new("DIAGNOSTICS_UNAVAILABLE", message.clone())
                .with_details(json!({ "service": service }))
        }
        ApplicationError::LogQueryInvalid { message } => {
            ErrorBody::new("INVALID_LOG_QUERY", message.clone())
        }
        ApplicationError::RuntimeUnavailable { code, message } => {
            ErrorBody::new(code.clone(), message.clone())
        }
    }
}

/// Renders an [`OperationResult`] as `camelCase`, attaching the live status of a
/// freshly scheduled task when one is available.
pub(crate) fn operation_result(result: &OperationResult, status: Option<&str>) -> Value {
    match result {
        OperationResult::TaskScheduled { task_id, kind } => {
            let mut value = json!({
                "outcome": "task_scheduled",
                "taskId": task_id.to_string(),
                "kind": kind.label(),
            });
            if let Some(status) = status {
                value["status"] = Value::String(status.to_owned());
            }
            value
        }
        OperationResult::WorkflowScheduled { workflow_id, kind } => json!({
            "outcome": "workflow_scheduled",
            "workflowId": workflow_id.to_string(),
            "kind": kind.label(),
        }),
        OperationResult::AlreadyInDesiredState { kind } => json!({
            "outcome": "already_in_desired_state",
            "kind": kind.label(),
            "message": "the target was already in the requested state; nothing was scheduled",
        }),
    }
}

/// A `camelCase` view of a task, tailored for agent consumption.
///
/// It reports the stable `kind`/`status` labels, the correlation identifiers,
/// timestamps, an elapsed `durationMs`, progress, and any failure — everything a
/// client needs to track a long-running operation (`tasks/014.md` §17, §30, §31).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskView<'a> {
    id: String,
    kind: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    library_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workflow_id: Option<String>,
    created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<i64>,
    progress: ProgressView,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<FailureView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<&'a str>,
}

/// A `camelCase` view of task progress.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressView {
    current: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fraction: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

/// A `camelCase` view of a task failure.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureView {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// Renders a task as a [`TaskView`].
pub(crate) fn task(task: &Task) -> Value {
    let duration_ms = task.started_at.map(|start| {
        let end = task.finished_at.unwrap_or_else(Timestamp::now);
        i64::try_from((end - start).whole_milliseconds()).unwrap_or(i64::MAX)
    });

    let view = TaskView {
        id: task.id.to_string(),
        kind: task.kind.label().to_owned(),
        status: task.status.label(),
        service_id: task.service.as_ref().map(ToString::to_string),
        library_id: task.library.as_ref().map(ToString::to_string),
        workflow_id: task.workflow.as_ref().map(ToString::to_string),
        created_at: task.created_at.to_string(),
        started_at: task.started_at.map(|at| at.to_string()),
        finished_at: task.finished_at.map(|at| at.to_string()),
        duration_ms,
        progress: ProgressView {
            current: task.progress.current,
            total: task.progress.total,
            fraction: task.progress.total.and_then(|total| {
                if total == 0 {
                    None
                } else {
                    Some(f64::from(task.progress.current) / f64::from(total))
                }
            }),
            message: task.progress.message.clone(),
        },
        error: task.error.as_ref().map(|failure| FailureView {
            code: failure.code.clone(),
            message: failure.message.clone(),
            detail: failure.detail.clone(),
        }),
        dependencies: task.dependencies.iter().map(|id| id.as_str()).collect(),
    };

    serde_json::to_value(view).unwrap_or(Value::Null)
}

/// Renders a bounded [`LogPage`] as `camelCase`.
pub(crate) fn log_page(page: &LogPage) -> Value {
    let view = LogPageView {
        entries: page.entries.iter().map(log_entry).collect(),
        next_cursor: page.next_cursor,
        truncated: page.truncated,
    };
    serde_json::to_value(view).unwrap_or(Value::Null)
}

/// Renders a list of log entries as a `camelCase` array.
///
/// Used by the general `get_logs` query, which returns entries directly rather
/// than a page.
pub(crate) fn log_entries(entries: &[LogEntry]) -> Value {
    let view: Vec<LogEntryView> = entries.iter().map(log_entry).collect();
    serde_json::to_value(view).unwrap_or(Value::Null)
}

/// A `camelCase` view of a log page.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogPageView {
    entries: Vec<LogEntryView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<u64>,
    truncated: bool,
}

/// A `camelCase` view of a log entry.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEntryView {
    timestamp: String,
    level: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    process_id: Option<String>,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<&'static str>,
    sequence: u64,
}

/// Builds a [`LogEntryView`] from a log entry.
fn log_entry(entry: &LogEntry) -> LogEntryView {
    LogEntryView {
        timestamp: entry.timestamp.to_string(),
        level: entry.level.label(),
        message: entry.message.clone(),
        workspace_id: entry.workspace_id.as_ref().map(ToString::to_string),
        service_id: entry.service_id.as_ref().map(ToString::to_string),
        task_id: entry.task_id.as_ref().map(ToString::to_string),
        process_id: entry.process_id.clone(),
        source: entry.source.label(),
        stream: entry.stream.map(LogStream::label),
        sequence: entry.sequence,
    }
}
