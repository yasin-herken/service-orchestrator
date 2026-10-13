//! Stable method names for the local IPC protocol.
//!
//! A method name is a first-class part of the wire contract: the daemon routes
//! on it and the [`RemoteClient`](crate::RemoteClient) sends it. Defining the
//! names once here means the client and the daemon can never disagree about a
//! spelling, and it keeps the routing table in the dispatcher a plain match
//! (`tasks/015.md` §8, §9).
//!
//! The names intentionally mirror the application use case they dispatch to;
//! they are not a second API surface. Parameters cross the wire as a JSON tuple
//! of the operation's arguments and results cross as the operation's structured
//! return value.

// -- diagnostics -------------------------------------------------------------

/// Returns the daemon's operational status snapshot.
pub const DAEMON_STATUS: &str = "daemon_status";
/// Requests a graceful daemon shutdown.
pub const DAEMON_SHUTDOWN: &str = "daemon_shutdown";
/// Returns the read-only configuration summary.
pub const CONFIG_SUMMARY: &str = "config_summary";

// -- task listing and streaming ---------------------------------------------

/// Lists every task the engine knows about.
pub const LIST_TASKS: &str = "list_tasks";
/// Lists the tasks that are currently active.
pub const LIST_RUNNING_TASKS: &str = "list_running_tasks";
/// Lists the most recently finished tasks.
pub const LIST_RECENT_TASKS: &str = "list_recent_tasks";
/// Subscribes to realtime logs for a service.
pub const SUBSCRIBE_SERVICE_LOGS: &str = "subscribe_service_logs";
/// Stops streaming realtime logs for a service.
pub const UNSUBSCRIBE_SERVICE_LOGS: &str = "unsubscribe_service_logs";

// -- workspaces, groups, profiles -------------------------------------------

/// Lists every configured workspace.
pub const LIST_WORKSPACES: &str = "list_workspaces";
/// Returns a workspace by id.
pub const GET_WORKSPACE: &str = "get_workspace";
/// Returns a workspace status snapshot.
pub const GET_WORKSPACE_STATUS: &str = "get_workspace_status";
/// Validates a workspace against this machine, read-only.
pub const VALIDATE_WORKSPACE: &str = "validate_workspace";
/// Lists a workspace's service groups.
pub const LIST_GROUPS: &str = "list_groups";
/// Returns a service group by id.
pub const GET_GROUP: &str = "get_group";
/// Lists a workspace's developer profiles.
pub const LIST_PROFILES: &str = "list_profiles";
/// Returns a developer profile by id.
pub const GET_PROFILE: &str = "get_profile";

// -- services and libraries -------------------------------------------------

/// Lists a workspace's services.
pub const LIST_SERVICES: &str = "list_services";
/// Returns a service by id.
pub const GET_SERVICE: &str = "get_service";
/// Returns a service status snapshot.
pub const GET_SERVICE_STATUS: &str = "get_service_status";
/// Returns the managed runtime status of a service.
pub const GET_SERVICE_RUNTIME_STATUS: &str = "get_service_runtime_status";
/// Lists the running services in a workspace.
pub const LIST_RUNNING_SERVICES: &str = "list_running_services";
/// Lists a workspace's libraries.
pub const LIST_LIBRARIES: &str = "list_libraries";
/// Returns a library by id.
pub const GET_LIBRARY: &str = "get_library";
/// Inspects a library's configuration and repository state.
pub const GET_LIBRARY_STATUS: &str = "get_library_status";
/// Lists the services that depend on a library.
pub const FIND_LIBRARY_DEPENDENTS: &str = "find_library_dependents";
/// Verifies a library's artifact in the local repository.
pub const VERIFY_LIBRARY_ARTIFACT: &str = "verify_library_artifact";
/// Inspects a library's consumer version alignment.
pub const INSPECT_LIBRARY_VERSION_ALIGNMENT: &str = "inspect_library_version_alignment";

// -- git (read-only) --------------------------------------------------------

/// Inspects a service's repository status.
pub const GET_GIT_STATUS: &str = "get_git_status";
/// Lists a service's repository branches.
pub const LIST_BRANCHES: &str = "list_branches";
/// Returns a bounded diff of a service's repository.
pub const GET_REPOSITORY_DIFF: &str = "get_repository_diff";

// -- migrations, health, diagnostics, logs ----------------------------------

/// Returns a service's configuration-derived migration status.
pub const MIGRATION_STATUS: &str = "migration_status";
/// Returns the last observed health of a service.
pub const GET_SERVICE_HEALTH: &str = "get_service_health";
/// Lists the last observed health of every service in a workspace.
pub const LIST_SERVICE_HEALTH: &str = "list_service_health";
/// Produces a structured diagnostic of a service's observed state.
pub const GET_SERVICE_DIAGNOSTICS: &str = "get_service_diagnostics";
/// Returns one stable page of a service's logs.
pub const GET_SERVICE_LOGS: &str = "get_service_logs";
/// Returns one stable page of a task's logs.
pub const GET_TASK_LOGS: &str = "get_task_logs";
/// Returns bounded log entries matching a request.
pub const GET_LOGS: &str = "get_logs";

// -- tasks and workflows ----------------------------------------------------

/// Returns a task by id.
pub const GET_TASK: &str = "get_task";
/// Returns a workflow by id.
pub const GET_WORKFLOW: &str = "get_workflow";
/// Aggregates a workflow's tasks into a bulk-operation summary.
pub const SUMMARIZE_WORKFLOW: &str = "summarize_workflow";
/// Requests cancellation of a task.
pub const CANCEL_TASK: &str = "cancel_task";

// -- git commands -----------------------------------------------------------

/// Synchronizes a service's repository.
pub const SYNC_SERVICE: &str = "sync_service";
/// Synchronizes a service's repository (the legacy Git entry point).
pub const SYNC_REPOSITORY: &str = "sync_repository";
/// Fetches a service's repository.
pub const FETCH_REPOSITORY: &str = "fetch_repository";
/// Pulls a service's repository.
pub const PULL_REPOSITORY: &str = "pull_repository";
/// Checks out a branch in a service's repository.
pub const CHECKOUT_REPOSITORY: &str = "checkout_repository";
/// Clones a service's repository.
pub const CLONE_REPOSITORY: &str = "clone_repository";
/// Hard-resets a service's repository (destructive).
pub const RESET_REPOSITORY_HARD: &str = "reset_repository_hard";
/// Cleans a service's repository (destructive).
pub const CLEAN_REPOSITORY: &str = "clean_repository";
/// Discards local changes in a service's repository (destructive).
pub const DISCARD_LOCAL_CHANGES: &str = "discard_local_changes";
/// Synchronizes the selected repositories in a workspace.
pub const SYNC_REPOSITORIES: &str = "sync_repositories";
/// Fetches the selected repositories in a workspace.
pub const FETCH_REPOSITORIES: &str = "fetch_repositories";
/// Checks out a branch in the selected repositories of a workspace.
pub const CHECKOUT_REPOSITORIES: &str = "checkout_repositories";

// -- builds and libraries ---------------------------------------------------

/// Returns the build operations a service's build system supports.
pub const GET_BUILD_CAPABILITIES: &str = "get_build_capabilities";
/// Builds a service.
pub const BUILD_SERVICE: &str = "build_service";
/// Builds a workspace's services.
pub const BUILD_WORKSPACE: &str = "build_workspace";
/// Builds a library.
pub const BUILD_LIBRARY: &str = "build_library";
/// Installs a library's artifact locally.
pub const INSTALL_LIBRARY: &str = "install_library";
/// Synchronizes a library's repository.
pub const SYNC_LIBRARY: &str = "sync_library";
/// Runs a library prepare workflow.
pub const PREPARE_LIBRARY: &str = "prepare_library";
/// Prepares several libraries in dependency order.
pub const PREPARE_SELECTED_LIBRARIES: &str = "prepare_selected_libraries";

// -- service runtime --------------------------------------------------------

/// Starts a service.
pub const START_SERVICE: &str = "start_service";
/// Stops a service.
pub const STOP_SERVICE: &str = "stop_service";
/// Restarts a service.
pub const RESTART_SERVICE: &str = "restart_service";
/// Starts the selected services in a workspace.
pub const START_SERVICES: &str = "start_services";
/// Stops the selected services in a workspace.
pub const STOP_SERVICES: &str = "stop_services";
/// Restarts the selected services in a workspace.
pub const RESTART_SERVICES: &str = "restart_services";

// -- migrations -------------------------------------------------------------

/// Validates migrations for a service.
pub const VALIDATE_MIGRATION: &str = "validate_migration";
/// Reports migration status for a service.
pub const GET_MIGRATION_STATUS: &str = "get_migration_status";
/// Inspects pending changesets for a service.
pub const INSPECT_MIGRATION_PENDING: &str = "inspect_migration_pending";
/// Previews migrations for a service.
pub const PREVIEW_MIGRATION: &str = "preview_migration";
/// Applies migrations for a service.
pub const APPLY_MIGRATION: &str = "apply_migration";
/// Applies migrations for the selected services in a workspace.
pub const APPLY_SELECTED_MIGRATIONS: &str = "apply_selected_migrations";
/// Runs Liquibase for a service.
pub const RUN_LIQUIBASE: &str = "run_liquibase";

// -- health -----------------------------------------------------------------

/// Runs the selected health checks for a service.
pub const CHECK_SERVICE_HEALTH: &str = "check_service_health";
/// Runs only the required readiness checks for a service.
pub const CHECK_SERVICE_READINESS: &str = "check_service_readiness";

// -- workflows --------------------------------------------------------------

/// Synchronizes every repository in a workspace.
pub const SYNC_WORKSPACE: &str = "sync_workspace";
/// Prepares a development environment.
pub const PREPARE_ENVIRONMENT: &str = "prepare_environment";
/// Plans a development environment preparation without changing anything.
pub const PLAN_PREPARE_ENVIRONMENT: &str = "plan_prepare_environment";
/// Returns an aggregated status for a workflow.
pub const GET_WORKFLOW_STATUS: &str = "get_workflow_status";
/// Requests cancellation of every active task in a workflow.
pub const CANCEL_WORKFLOW: &str = "cancel_workflow";
/// Starts every service in a group.
pub const START_GROUP: &str = "start_group";
/// Stops every service in a group.
pub const STOP_GROUP: &str = "stop_group";
/// Starts every service in a profile.
pub const START_PROFILE: &str = "start_profile";
/// Stops every service in a profile.
pub const STOP_PROFILE: &str = "stop_profile";
