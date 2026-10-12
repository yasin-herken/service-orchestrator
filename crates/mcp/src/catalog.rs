//! The MCP tool catalog.
//!
//! Every tool is a thin, named projection of exactly one application use case.
//! This module owns only the *advertisement* — name, description, annotation,
//! and input schema — never behavior; `server.rs` dispatches each name to the
//! application core.
//!
//! # Granularity
//!
//! The catalog follows `AGENTS.md` §24:
//!
//! - **primitive** — `git_fetch`, `git_checkout`, `migration_status`, …;
//! - **service** — `build_service`, `start_service`, `restart_service`, …;
//! - **workflow** — `prepare_environment`, `sync_workspace`, `build_workspace`,
//!   `start_group`, `start_profile`, ….
//!
//! # Authorization annotations
//!
//! A tool is annotated with the same permission class the application core
//! enforces: `readOnly` for side-effect-free queries, a plain (non-destructive)
//! write for staging operations, and `destructive` for the operations that pass
//! through [`Permission::Destructive`](service_orchestrator_domain::Permission).
//! The annotation is a hint; the application is still the authority
//! (`tasks/014.md` §15, §25).
//!
//! # Not implemented
//!
//! Tools are advertised only where a use case already exists. There is
//! deliberately no `install_dependencies`, `test_service`, or generic
//! "build selected services" tool, because the application core exposes no such
//! use case; see `docs/mcp.md` for the documented gaps (`tasks/014.md` §19).

use rmcp::handler::server::tool::schema_for_input;
use rmcp::model::{Tool, ToolAnnotations};

use crate::params;

/// Builds the full tool list in a stable order.
pub(crate) fn tools() -> Vec<Tool> {
    vec![
        // Workspace, group, and profile discovery.
        tool::<params::Empty>(
            "list_workspaces",
            "List every configured workspace.",
            read(),
        ),
        tool::<params::WorkspaceRef>(
            "get_workspace",
            "Return one configured workspace by id.",
            read(),
        ),
        tool::<params::WorkspaceRef>(
            "get_workspace_status",
            "Return a structured status snapshot of a workspace and its services.",
            read(),
        ),
        tool::<params::WorkspaceRef>(
            "validate_workspace",
            "Validate a workspace against this machine, read-only: configuration, \
             repository checkouts, runtime and build-tool availability, dependency \
             resolution, port conflicts, and port occupancy. Never checks out, pulls, \
             builds, starts, or rewrites anything.",
            read(),
        ),
        tool::<params::WorkspaceRef>(
            "list_groups",
            "List the service groups defined in a workspace.",
            read(),
        ),
        tool::<params::GroupRef>(
            "get_group",
            "Return one service group by id.",
            read(),
        ),
        tool::<params::WorkspaceRef>(
            "list_profiles",
            "List the developer profiles defined in a workspace.",
            read(),
        ),
        tool::<params::ProfileRef>(
            "get_profile",
            "Return one developer profile by id.",
            read(),
        ),
        // Services.
        tool::<params::WorkspaceRef>(
            "list_services",
            "List the services configured in a workspace.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "get_service",
            "Return one service by id.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "get_service_status",
            "Return the structured status of a service.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "get_service_runtime_status",
            "Return the managed-process runtime status of a service.",
            read(),
        ),
        tool::<params::WorkspaceRef>(
            "list_running_services",
            "List the services with a managed running process in a workspace.",
            read(),
        ),
        // Libraries.
        tool::<params::WorkspaceRef>(
            "list_libraries",
            "List the libraries configured in a workspace.",
            read(),
        ),
        tool::<params::LibraryRef>(
            "get_library",
            "Return one library by id.",
            read(),
        ),
        tool::<params::LibraryRef>(
            "get_library_status",
            "Return the repository and artifact status of a library.",
            read(),
        ),
        tool::<params::LibraryRef>(
            "find_library_dependents",
            "List the libraries and services that consume a library.",
            read(),
        ),
        tool::<params::LibraryRef>(
            "verify_library_artifact",
            "Verify that a library's artifact is present in the local repository.",
            read(),
        ),
        tool::<params::LibraryRef>(
            "inspect_library_version_alignment",
            "Inspect whether consumers align with a library's artifact version.",
            read(),
        ),
        // Git, read-only.
        tool::<params::ServiceRef>(
            "git_status",
            "Return the Git working-tree status of a service's repository.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "git_list_branches",
            "List the local branches of a service's repository.",
            read(),
        ),
        tool::<params::ServiceDiff>(
            "git_diff",
            "Return a bounded diff of a service's repository (staged or unstaged).",
            read(),
        ),
        // Git, safe writes.
        tool::<params::ServiceRef>(
            "git_fetch",
            "Fetch a service's repository. Returns a task id.",
            write(),
        ),
        tool::<params::ServiceRef>(
            "git_pull",
            "Pull a service's repository. Returns a task id.",
            write(),
        ),
        tool::<params::ServiceBranch>(
            "git_checkout",
            "Check out a branch in a service's repository. Returns a task id.",
            write(),
        ),
        tool::<params::ServiceRef>(
            "git_sync",
            "Synchronize a service's repository (fetch and pull the configured branch). Returns a task id.",
            write(),
        ),
        tool::<params::ServiceRef>(
            "git_clone",
            "Clone a service's repository when it is not yet present. Returns a task id.",
            write(),
        ),
        tool::<params::WorkspaceServices>(
            "git_sync_repositories",
            "Synchronize several repositories in a workspace. Returns a workflow id.",
            write(),
        ),
        tool::<params::WorkspaceServices>(
            "git_fetch_repositories",
            "Fetch several repositories in a workspace. Returns a workflow id.",
            write(),
        ),
        tool::<params::WorkspaceServicesBranch>(
            "git_checkout_repositories",
            "Check out a branch in several repositories of a workspace. Returns a workflow id.",
            write(),
        ),
        // Git, destructive: always require confirmation.
        tool::<params::ServiceRef>(
            "git_reset_hard",
            "Hard-reset a service's repository, discarding local changes. Requires explicit confirmation.",
            destructive(),
        ),
        tool::<params::ServiceRef>(
            "git_clean",
            "Remove untracked files from a service's repository. Requires explicit confirmation.",
            destructive(),
        ),
        tool::<params::ServiceRef>(
            "git_discard_changes",
            "Discard local changes in a service's repository. Requires explicit confirmation.",
            destructive(),
        ),
        // Builds.
        tool::<params::ServiceRef>(
            "build_service",
            "Build a service. Returns a task id.",
            write(),
        ),
        tool::<params::WorkspaceRef>(
            "build_workspace",
            "Build every service in a workspace. Returns a workflow id.",
            write(),
        ),
        tool::<params::LibraryRef>(
            "build_library",
            "Build a library. Returns a task id.",
            write(),
        ),
        tool::<params::LibraryRef>(
            "install_library",
            "Install a library's artifact into the local repository. Returns a task id.",
            write(),
        ),
        tool::<params::PrepareLibraryRequest>(
            "prepare_library",
            "Synchronize, build, install, and verify one library. Returns a workflow id.",
            write(),
        ),
        tool::<params::PrepareSelectedLibrariesRequest>(
            "prepare_selected_libraries",
            "Prepare several libraries in dependency order. Returns a workflow id.",
            write(),
        ),
        // Service runtime.
        tool::<params::ServiceRef>(
            "start_service",
            "Start a service. Returns a task id.",
            write(),
        ),
        tool::<params::ServiceRef>(
            "stop_service",
            "Stop a service. Returns a task id.",
            write(),
        ),
        tool::<params::ServiceRef>(
            "restart_service",
            "Restart a service. Returns a task id.",
            write(),
        ),
        tool::<params::WorkspaceServices>(
            "start_services",
            "Start several services of a workspace. Returns a workflow id.",
            write(),
        ),
        tool::<params::WorkspaceServices>(
            "stop_services",
            "Stop several services of a workspace. Returns a workflow id.",
            write(),
        ),
        tool::<params::WorkspaceServices>(
            "restart_services",
            "Restart several services of a workspace. Returns a workflow id.",
            write(),
        ),
        // Migrations.
        tool::<params::ServiceRef>(
            "migration_status",
            "Return the configured migration setup of a service (configuration snapshot; no database access).",
            read(),
        ),
        tool::<params::ServiceRef>(
            "validate_migration",
            "Validate a service's migration configuration against the database. Returns a task id.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "get_migration_status",
            "Inspect the live migration status of a service's database. Returns a task id.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "inspect_migration_pending",
            "List the pending changesets of a service's database. Returns a task id.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "preview_migration",
            "Preview the SQL a service's migration would run. Returns a task id.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "apply_migration",
            "Apply a service's migrations. Requires confirmation for non-local targets. Returns a task id.",
            write(),
        ),
        tool::<params::WorkspaceServices>(
            "apply_selected_migrations",
            "Apply migrations for several services in a workspace. Returns a workflow id.",
            write(),
        ),
        // Health and diagnostics.
        tool::<params::HealthCheckRequest>(
            "check_service_health",
            "Run a service's configured health checks. Returns a task id.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "check_service_readiness",
            "Poll a service until it is ready or the deadline elapses. Returns a task id.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "get_service_health",
            "Return the last observed health of a service.",
            read(),
        ),
        tool::<params::WorkspaceRef>(
            "list_service_health",
            "Return the last observed health of every service in a workspace.",
            read(),
        ),
        tool::<params::ServiceRef>(
            "get_service_diagnostics",
            "Return structured diagnostics for a service.",
            read(),
        ),
        // Logs.
        tool::<params::ServiceLogsRequest>(
            "get_service_logs",
            "Return a bounded page of a service's logs.",
            read(),
        ),
        tool::<params::TaskLogsRequest>(
            "get_task_logs",
            "Return a bounded page of a task's logs.",
            read(),
        ),
        tool::<params::LogsRequest>(
            "get_logs",
            "Return a bounded page of logs matching the given filters.",
            read(),
        ),
        // Tasks.
        tool::<params::TaskRef>(
            "get_task",
            "Return the current snapshot of a task, including status and progress.",
            read(),
        ),
        tool::<params::ListTasksRequest>(
            "list_tasks",
            "List recent tasks, newest first.",
            read(),
        ),
        tool::<params::TaskRef>(
            "cancel_task",
            "Request cancellation of a queued or running task.",
            write(),
        ),
        tool::<params::WorkflowRef>(
            "summarize_workflow",
            "Summarize the aggregate outcome of a workflow's tasks.",
            read(),
        ),
        // Workflows.
        tool::<params::WorkspaceRef>(
            "sync_workspace",
            "Synchronize every repository in a workspace. Returns a workflow id.",
            write(),
        ),
        tool::<params::PrepareEnvironmentRequest>(
            "prepare_environment",
            "Prepare a development environment: sync, optionally migrate, build, and start. Returns a workflow id.",
            write(),
        ),
        tool::<params::PrepareEnvironmentRequest>(
            "plan_prepare_environment",
            "Plan a development environment preparation without changing anything. Returns the resolved phases and steps.",
            read(),
        ),
        tool::<params::WorkflowRef>(
            "get_workflow_status",
            "Return an aggregated status for a workflow: state, per-phase progress, and structured failures.",
            read(),
        ),
        tool::<params::WorkflowRef>(
            "cancel_workflow",
            "Request cancellation of every still-active task in a workflow.",
            write(),
        ),
        tool::<params::GroupAction>(
            "start_group",
            "Start every service in a group. Returns a workflow id.",
            write(),
        ),
        tool::<params::GroupAction>(
            "stop_group",
            "Stop every service in a group. Returns a workflow id.",
            write(),
        ),
        tool::<params::ProfileAction>(
            "start_profile",
            "Start every service in a developer profile. Returns a workflow id.",
            write(),
        ),
        tool::<params::ProfileAction>(
            "stop_profile",
            "Stop every service in a developer profile. Returns a workflow id.",
            write(),
        ),
    ]
}

/// Builds one tool from its parameter type.
///
/// A schema failure is a programming error, not a runtime condition: the
/// parameter types are static, so a failure can only mean a broken `JsonSchema`
/// derivation. It is surfaced loudly at startup rather than hidden.
fn tool<T: schemars::JsonSchema + 'static>(
    name: &'static str,
    description: &'static str,
    annotations: ToolAnnotations,
) -> Tool {
    let schema = schema_for_input::<T>()
        .unwrap_or_else(|error| panic!("invalid JSON schema for MCP tool '{name}': {error}"));
    Tool::new(name, description, schema).with_annotations(annotations)
}

/// Annotations for a side-effect-free query.
fn read() -> ToolAnnotations {
    ToolAnnotations::new().read_only(true).destructive(false)
}

/// Annotations for a staging write that never destroys local state.
fn write() -> ToolAnnotations {
    ToolAnnotations::new().read_only(false).destructive(false)
}

/// Annotations for an operation that requires explicit confirmation.
fn destructive() -> ToolAnnotations {
    ToolAnnotations::new().read_only(false).destructive(true)
}
