//! The Tauri commands exposed to the webview.
//!
//! Every command is a thin adapter: validate input, delegate to a query or
//! mutation over the shared [`OrchestratorClient`], and funnel failures through
//! the connection tracker (`tasks/018.md` §7, §11; `tasks/019.md` §3, §10).
//!
//! The command set:
//!
//! - `get_daemon_status` — daemon process facts for the dashboard;
//! - `list_workspaces` — workspace summaries for the selector;
//! - `list_services` — service rows for a workspace;
//! - `get_workspace_status` — one workspace with all service statuses;
//! - `get_service_status` — one service, validated against its workspace;
//! - `get_service_detail` — configuration, observed state, and diagnostics;
//! - `start_service` / `stop_service` / `restart_service` — one service;
//! - `start_services` / `stop_services` / `restart_services` — bulk workflow;
//! - `check_service_health` — run a service's configured health checks;
//! - `get_connection_state` — the adapter's current connection snapshot.
//!
//! Task 020 adds the Git, build, and library management commands plus the shared
//! task/workflow/log commands every operation observes through
//! (`tasks/020.md` §4–§29).

use std::sync::Arc;

use service_orchestrator_client::{DaemonStatus, RemoteClient};
use tauri::State;

use crate::connection::ConnectionSnapshot;
use crate::dto::{
    ArtifactVerificationView, BranchView, BuildCapabilitiesView, BuildTargetSummary,
    BulkSummaryView, GitStatusView, GitSyncPreview, LibraryDependentView, LibrarySummary,
    LogPageView, OperationOutcome, RepositoryInspectionView, RepositorySummary, ServiceDetail,
    ServiceSummary, TaskView, VersionAlignmentView, WorkflowCancellationView, WorkspaceStatus,
    WorkspaceSummary,
};
use crate::error::CommandError;
use crate::state::AppState;
use crate::tasks::TaskLogsQuery;
use crate::{build, git, library, mutations, queries, tasks};

/// Runs one blocking query against the shared client.
///
/// On success the connection state is confirmed (`Connected`), which lets an
/// explicit Retry clear the banner as soon as the daemon answers. On failure
/// connection-shaped errors update the pushed connection state, while input
/// and not-found errors stay local to the command.
async fn query<T, F>(state: &AppState, operation: F) -> Result<T, CommandError>
where
    F: FnOnce(Arc<RemoteClient>) -> Result<T, CommandError> + Send + 'static,
    T: Send + 'static,
{
    let client = state.ensure_client().await?;
    let outcome = tauri::async_runtime::spawn_blocking(move || operation(client))
        .await
        .map_err(|error| CommandError::internal(format!("query task failed: {error}")))?;
    match outcome {
        Ok(value) => {
            state.push(&crate::connection::ProbeOutcome::Connected);
            Ok(value)
        }
        Err(error) => {
            state.observe(&error);
            Err(error)
        }
    }
}

/// Returns the daemon's diagnostic status for the dashboard.
///
/// # Errors
///
/// A structured `{ code, message }` error when the daemon cannot be reached.
#[tauri::command]
pub async fn get_daemon_status(state: State<'_, AppState>) -> Result<DaemonStatus, CommandError> {
    query(state.inner(), |client| {
        client.daemon_status().map_err(CommandError::from)
    })
    .await
}

/// Lists every configured workspace as selector and dashboard summaries.
///
/// # Errors
///
/// A structured `{ code, message }` error when the daemon cannot be reached.
#[tauri::command]
pub async fn list_workspaces(
    state: State<'_, AppState>,
) -> Result<Vec<WorkspaceSummary>, CommandError> {
    query(state.inner(), |client| {
        queries::list_workspaces(client.as_ref())
    })
    .await
}

/// Lists a workspace's services as table rows.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn list_services(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<ServiceSummary>, CommandError> {
    query(state.inner(), move |client| {
        queries::list_services(client.as_ref(), &workspace_id)
    })
    .await
}

/// Returns one workspace with the observed status of each of its services.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn get_workspace_status(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<WorkspaceStatus, CommandError> {
    query(state.inner(), move |client| {
        queries::workspace_status(client.as_ref(), &workspace_id)
    })
    .await
}

/// Returns one service's status, validated against its workspace.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier, `NOT_FOUND` when the service
/// belongs to a different workspace, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn get_service_status(
    state: State<'_, AppState>,
    workspace_id: String,
    service_id: String,
) -> Result<ServiceSummary, CommandError> {
    query(state.inner(), move |client| {
        queries::service_status(client.as_ref(), &workspace_id, &service_id)
    })
    .await
}

/// Returns a detailed view of one service, validated against its workspace.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier, `NOT_FOUND` when the service
/// belongs to a different workspace, otherwise a structured `{ code, message }`
/// error.
#[tauri::command]
pub async fn get_service_detail(
    state: State<'_, AppState>,
    workspace_id: String,
    service_id: String,
) -> Result<ServiceDetail, CommandError> {
    query(state.inner(), move |client| {
        queries::service_detail(client.as_ref(), &workspace_id, &service_id)
    })
    .await
}

/// Starts one service through the daemon.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected service id, otherwise a structured
/// `{ code, message }` error (including a permission denial).
#[tauri::command]
pub async fn start_service(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        mutations::start_service(client.as_ref(), &service_id)
    })
    .await
}

/// Stops one service through the daemon.
///
/// # Errors
///
/// Same as [`start_service`].
#[tauri::command]
pub async fn stop_service(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        mutations::stop_service(client.as_ref(), &service_id)
    })
    .await
}

/// Restarts one service through the daemon.
///
/// # Errors
///
/// Same as [`start_service`].
#[tauri::command]
pub async fn restart_service(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        mutations::restart_service(client.as_ref(), &service_id)
    })
    .await
}

/// Starts the selected services of a workspace as one workflow.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier or an empty selection, otherwise a
/// structured `{ code, message }` error.
#[tauri::command]
pub async fn start_services(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        mutations::start_services(client.as_ref(), &workspace_id, &service_ids)
    })
    .await
}

/// Stops the selected services of a workspace as one workflow.
///
/// # Errors
///
/// Same as [`start_services`].
#[tauri::command]
pub async fn stop_services(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        mutations::stop_services(client.as_ref(), &workspace_id, &service_ids)
    })
    .await
}

/// Restarts the selected services of a workspace as one workflow.
///
/// # Errors
///
/// Same as [`start_services`].
#[tauri::command]
pub async fn restart_services(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        mutations::restart_services(client.as_ref(), &workspace_id, &service_ids)
    })
    .await
}

/// Runs every configured health check for one service.
///
/// # Errors
///
/// Same as [`start_service`].
#[tauri::command]
pub async fn check_service_health(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        mutations::check_service_health(client.as_ref(), &service_id)
    })
    .await
}

/// Returns the adapter's current connection snapshot.
///
/// The frontend pulls this once on startup and then follows the pushed
/// `daemon://connection` events; the pull is only a bootstrap.
#[tauri::command]
pub fn get_connection_state(state: State<'_, AppState>) -> ConnectionSnapshot {
    state.connection_snapshot()
}

// -- git ----------------------------------------------------------------------

/// Lists a workspace's configured repositories as Git rows.
///
/// The row set is configuration-derived and performs no Git operation, so the
/// screen opens immediately even with 100+ repositories
/// (`tasks/020.md` §4, §35).
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn list_repositories(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<RepositorySummary>, CommandError> {
    query(state.inner(), move |client| {
        git::list_repositories(client.as_ref(), &workspace_id)
    })
    .await
}

/// Inspects one repository's working-tree status on demand.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected service id, `ServiceNotFound` for an unknown
/// service, `InvalidOperation` when it has no repository, or a `Git` error when
/// the repository cannot be inspected.
#[tauri::command]
pub async fn get_git_status(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<GitStatusView, CommandError> {
    query(state.inner(), move |client| {
        git::get_git_status(client.as_ref(), &service_id)
    })
    .await
}

/// Inspects several repositories, reporting each result independently.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace or service identifier.
#[tauri::command]
pub async fn inspect_repositories(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
) -> Result<Vec<RepositoryInspectionView>, CommandError> {
    query(state.inner(), move |client| {
        git::inspect_repositories(client.as_ref(), &workspace_id, &service_ids)
    })
    .await
}

/// Lists a repository's local and remote-tracking branches.
///
/// # Errors
///
/// Same as [`get_git_status`].
#[tauri::command]
pub async fn list_branches(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<Vec<BranchView>, CommandError> {
    query(state.inner(), move |client| {
        git::list_branches(client.as_ref(), &service_id)
    })
    .await
}

/// Builds a read-only preview of a bulk Git operation over a selection.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace, service, branch, or mode.
#[tauri::command]
pub async fn preview_git_sync(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
    target_branch: String,
    mode: String,
) -> Result<GitSyncPreview, CommandError> {
    query(state.inner(), move |client| {
        git::preview_git_sync(
            client.as_ref(),
            &workspace_id,
            &service_ids,
            &target_branch,
            &mode,
        )
    })
    .await
}

/// Fetches one repository.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected service id, otherwise a structured
/// `{ code, message }` error (including a permission denial).
#[tauri::command]
pub async fn fetch_repository(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        git::fetch_repository(client.as_ref(), &service_id)
    })
    .await
}

/// Fast-forward pulls one repository.
///
/// # Errors
///
/// Same as [`fetch_repository`].
#[tauri::command]
pub async fn pull_repository(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        git::pull_repository(client.as_ref(), &service_id)
    })
    .await
}

/// Checks out a branch in one repository.
///
/// # Errors
///
/// Same as [`fetch_repository`], plus `INVALID_INPUT` for a rejected branch.
#[tauri::command]
pub async fn checkout_repository(
    state: State<'_, AppState>,
    service_id: String,
    branch: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        git::checkout_repository(client.as_ref(), &service_id, &branch)
    })
    .await
}

/// Synchronizes one repository (fetch, checkout, fast-forward).
///
/// # Errors
///
/// Same as [`fetch_repository`].
#[tauri::command]
pub async fn sync_repository(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        git::sync_repository(client.as_ref(), &service_id)
    })
    .await
}

/// Fetches the selected repositories as one workflow.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier or an empty selection, otherwise a
/// structured `{ code, message }` error.
#[tauri::command]
pub async fn fetch_repositories(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        git::fetch_repositories(client.as_ref(), &workspace_id, &service_ids)
    })
    .await
}

/// Synchronizes the selected repositories as one workflow.
///
/// # Errors
///
/// Same as [`fetch_repositories`].
#[tauri::command]
pub async fn sync_repositories(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        git::sync_repositories(client.as_ref(), &workspace_id, &service_ids)
    })
    .await
}

/// Checks out a branch in the selected repositories as one workflow.
///
/// # Errors
///
/// Same as [`fetch_repositories`], plus `INVALID_INPUT` for a rejected branch.
#[tauri::command]
pub async fn checkout_repositories(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
    branch: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        git::checkout_repositories(client.as_ref(), &workspace_id, &service_ids, &branch)
    })
    .await
}

// -- build --------------------------------------------------------------------

/// Lists a workspace's build targets with their provider capabilities.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn list_build_targets(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<BuildTargetSummary>, CommandError> {
    query(state.inner(), move |client| {
        build::list_build_targets(client.as_ref(), &workspace_id)
    })
    .await
}

/// Returns the build operations one service's build system supports.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected service id, `ServiceNotFound` for an unknown
/// service, or a `Build` error when no provider is registered.
#[tauri::command]
pub async fn get_build_capabilities(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<BuildCapabilitiesView, CommandError> {
    query(state.inner(), move |client| {
        build::get_build_capabilities(client.as_ref(), &service_id)
    })
    .await
}

/// Builds one service.
///
/// # Errors
///
/// Same as [`get_build_capabilities`], plus the daemon's structured error when
/// the build is rejected.
#[tauri::command]
pub async fn build_service(
    state: State<'_, AppState>,
    service_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        build::build_service(client.as_ref(), &service_id)
    })
    .await
}

/// Builds the selected services as one workflow.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier or an empty selection, otherwise a
/// structured `{ code, message }` error.
#[tauri::command]
pub async fn build_services(
    state: State<'_, AppState>,
    workspace_id: String,
    service_ids: Vec<String>,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        build::build_services(client.as_ref(), &workspace_id, &service_ids)
    })
    .await
}

// -- library ------------------------------------------------------------------

/// Lists a workspace's libraries with observed state and dependents.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workspace id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn list_libraries(
    state: State<'_, AppState>,
    workspace_id: String,
) -> Result<Vec<LibrarySummary>, CommandError> {
    query(state.inner(), move |client| {
        library::list_libraries(client.as_ref(), &workspace_id)
    })
    .await
}

/// Lists the services that depend on one library.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected library id or `LibraryNotFound` for an
/// unknown library.
#[tauri::command]
pub async fn get_library_dependents(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<Vec<LibraryDependentView>, CommandError> {
    query(state.inner(), move |client| {
        library::get_library_dependents(client.as_ref(), &library_id)
    })
    .await
}

/// Verifies a library's artifact in the effective local repository.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected library id or `LibraryNotFound` for an
/// unknown library.
#[tauri::command]
pub async fn verify_library_artifact(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<ArtifactVerificationView, CommandError> {
    query(state.inner(), move |client| {
        library::verify_library_artifact(client.as_ref(), &library_id)
    })
    .await
}

/// Inspects a library's consumer dependency version alignment.
///
/// # Errors
///
/// Same as [`verify_library_artifact`].
#[tauri::command]
pub async fn inspect_version_alignment(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<VersionAlignmentView, CommandError> {
    query(state.inner(), move |client| {
        library::inspect_version_alignment(client.as_ref(), &library_id)
    })
    .await
}

/// Builds one library.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected library id, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn build_library(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        library::build_library(client.as_ref(), &library_id)
    })
    .await
}

/// Installs one library's artifact into the local repository.
///
/// # Errors
///
/// Same as [`build_library`].
#[tauri::command]
pub async fn install_library(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        library::install_library(client.as_ref(), &library_id)
    })
    .await
}

/// Synchronizes one library's repository.
///
/// # Errors
///
/// Same as [`build_library`].
#[tauri::command]
pub async fn sync_library(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        library::sync_library(client.as_ref(), &library_id)
    })
    .await
}

/// Runs the prepare-library workflow for one library.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected library id or branch, otherwise a structured
/// `{ code, message }` error.
#[tauri::command]
pub async fn prepare_library(
    state: State<'_, AppState>,
    library_id: String,
    sync: bool,
    branch: Option<String>,
    build: bool,
    install: bool,
    verify: bool,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        library::prepare_library(
            client.as_ref(),
            &library_id,
            sync,
            branch.as_deref(),
            build,
            install,
            verify,
        )
    })
    .await
}

/// Prepares several libraries as one workflow, in dependency order.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected identifier or an empty selection, otherwise a
/// structured `{ code, message }` error.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn prepare_libraries(
    state: State<'_, AppState>,
    workspace_id: String,
    library_ids: Vec<String>,
    sync: bool,
    branch: Option<String>,
    build: bool,
    install: bool,
    verify: bool,
) -> Result<OperationOutcome, CommandError> {
    query(state.inner(), move |client| {
        library::prepare_libraries(
            client.as_ref(),
            &workspace_id,
            &library_ids,
            sync,
            branch.as_deref(),
            build,
            install,
            verify,
        )
    })
    .await
}

// -- tasks, workflows, and logs ----------------------------------------------

/// Returns one task by id.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected task id, or the daemon's structured error.
#[tauri::command]
pub async fn get_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<TaskView, CommandError> {
    query(state.inner(), move |client| {
        tasks::get_task(client.as_ref(), &task_id)
    })
    .await
}

/// Aggregates a workflow's tasks into a bulk-operation summary.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workflow id, or the daemon's structured error.
#[tauri::command]
pub async fn summarize_workflow(
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<BulkSummaryView, CommandError> {
    query(state.inner(), move |client| {
        tasks::summarize_workflow(client.as_ref(), &workflow_id)
    })
    .await
}

/// Returns one bounded page of a task's structured logs.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected task id or severity, or the daemon's
/// structured error.
#[tauri::command]
pub async fn get_task_logs(
    state: State<'_, AppState>,
    task_id: String,
    after: Option<u64>,
    limit: Option<usize>,
    minimum_level: Option<String>,
    text: Option<String>,
) -> Result<LogPageView, CommandError> {
    let query_params = TaskLogsQuery {
        after,
        limit,
        minimum_level,
        text,
    };
    query(state.inner(), move |client| {
        tasks::get_task_logs(client.as_ref(), &task_id, &query_params)
    })
    .await
}

/// Requests cancellation of one task and returns its latest observed state.
///
/// # Errors
///
/// Same as [`get_task`].
#[tauri::command]
pub async fn cancel_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<TaskView, CommandError> {
    query(state.inner(), move |client| {
        tasks::cancel_task(client.as_ref(), &task_id)
    })
    .await
}

/// Requests cancellation of every active task in a workflow.
///
/// # Errors
///
/// `INVALID_INPUT` for a rejected workflow id, or the daemon's structured error.
#[tauri::command]
pub async fn cancel_workflow(
    state: State<'_, AppState>,
    workflow_id: String,
) -> Result<WorkflowCancellationView, CommandError> {
    query(state.inner(), move |client| {
        tasks::cancel_workflow(client.as_ref(), &workflow_id)
    })
    .await
}
