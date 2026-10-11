//! The shared orchestrator client abstraction.
//!
//! [`OrchestratorClient`] is the single boundary the Ratatui TUI and the MCP
//! server talk to. It mirrors the application core's use cases exactly, so an
//! interface references intent (a method call) rather than transport. Two
//! implementations exist:
//!
//! - [`Application`] itself, which serves in-process tests and any future
//!   in-process embedding; and
//! - [`RemoteClient`](crate::RemoteClient), which proxies each call over the
//!   local IPC protocol to the daemon.
//!
//! Interfaces remain unaware of which implementation they hold. That keeps the
//! daemon an implementation detail of the transport, never of the interface
//! (`tasks/015.md` §11, §13).
//!
//! # Task listing and events
//!
//! The application's [`TaskManager`](service_orchestrator_execution::TaskManager)
//! contract exposes submission and lookup by id but not the richer list and
//! event API an interface needs. Those are declared here as the defaultable
//! [`OrchestratorClient::list_tasks`] family and
//! [`OrchestratorClient::subscribe_task_events`]. The daemon-backed client
//! overrides them; the in-process [`Application`] implementation leaves the
//! defaults (an interface using the in-process client observes tasks through the
//! engine directly).

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use service_orchestrator_application::{
    Application, ApplicationError, Authorization, BulkOperationSummary, HealthCheckScope,
    LibraryArtifactVerification, LibraryDependent, LibraryPrepareOptions, LibraryStatus, LogEntry,
    LogPage, LogRequest, LogSubscription, MigrationStatus, OperationResult,
    PrepareEnvironmentRequest, PrepareSelectedLibrariesRequest, ServiceDiagnostics, ServiceHealth,
    ServiceRuntimeStatus, ServiceStatus, WorkspaceStatus,
};
use service_orchestrator_application::{DiffOptions, HealthState};
use service_orchestrator_domain::{
    Branch, BranchInfo, GroupId, Library, LibraryId, Profile, ProfileId, RepoDiff,
    RepositoryStatus, Service, ServiceGroup, ServiceId, Task, TaskId, Workflow, WorkflowId,
    Workspace, WorkspaceId,
};
use service_orchestrator_execution::TaskEvent;
use tokio::sync::broadcast;

/// The read-only slice of configuration an interface needs.
///
/// The daemon owns the validated configuration; a client holds a refreshable
/// summary rather than a second copy, so it never mutates configuration and
/// never disagrees with the daemon about it (`tasks/015.md` §26).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSummary {
    /// The configuration schema version.
    pub version: u32,
    /// The configured maximum number of tasks executed in parallel.
    pub max_parallel_tasks: usize,
    /// The configured default operation timeout, in seconds.
    pub default_timeout_seconds: u64,
}

/// The shared client boundary over the application core.
///
/// Read and safe-write calls are synchronous because the interface state machine
/// ([`App`](service_orchestrator_tui::App)) is synchronous; the handful of
/// queries that reach an external process are `async`. The method set mirrors
/// the application's public use cases one-to-one: a client adds no business
/// logic (`tasks/015.md` §11, §21).
#[async_trait]
pub trait OrchestratorClient: Send + Sync {
    /// Returns the read-only configuration summary.
    fn config_summary(&self) -> ConfigSummary;

    /// Returns a client that performs subsequent calls with `authorization`.
    ///
    /// For a daemon-backed client this does not grant anything on its own: the
    /// daemon honors a destructive request only from an interactive peer and
    /// still runs the shared policy engine (`tasks/015.md` §23).
    fn with_authorization(&self, authorization: Authorization) -> Arc<dyn OrchestratorClient>;

    /// Returns every known task, oldest first.
    fn list_tasks(&self) -> Vec<Task> {
        Vec::new()
    }

    /// Returns the tasks that are currently active.
    fn list_running_tasks(&self) -> Vec<Task> {
        Vec::new()
    }

    /// Returns up to `limit` most recently finished tasks, newest first.
    fn list_recent_tasks(&self, _limit: usize) -> Vec<Task> {
        Vec::new()
    }

    /// Subscribes to task-engine events.
    ///
    /// The default is an immediately closed stream.
    fn subscribe_task_events(&self) -> broadcast::Receiver<TaskEvent> {
        let (_sender, receiver) = broadcast::channel(1);
        receiver
    }

    // -- workspaces, groups, profiles ---------------------------------------

    /// Lists every configured workspace.
    fn list_workspaces(&self) -> Result<Vec<Workspace>, ApplicationError>;

    /// Returns a workspace by id.
    fn get_workspace(&self, workspace_id: &WorkspaceId) -> Result<Workspace, ApplicationError>;

    /// Returns a workspace status snapshot.
    fn get_workspace_status(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceStatus, ApplicationError>;

    /// Lists a workspace's service groups.
    fn list_groups(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceGroup>, ApplicationError>;

    /// Returns a service group by id.
    fn get_group(&self, group_id: &GroupId) -> Result<ServiceGroup, ApplicationError>;

    /// Lists a workspace's developer profiles.
    fn list_profiles(&self, workspace_id: &WorkspaceId) -> Result<Vec<Profile>, ApplicationError>;

    /// Returns a developer profile by id.
    fn get_profile(&self, profile_id: &ProfileId) -> Result<Profile, ApplicationError>;

    // -- services and libraries ---------------------------------------------

    /// Lists a workspace's services.
    fn list_services(&self, workspace_id: &WorkspaceId) -> Result<Vec<Service>, ApplicationError>;

    /// Returns a service by id.
    fn get_service(&self, service_id: &ServiceId) -> Result<Service, ApplicationError>;

    /// Returns a service status snapshot.
    fn get_service_status(&self, service_id: &ServiceId)
        -> Result<ServiceStatus, ApplicationError>;

    /// Returns the managed runtime status of a service.
    fn get_service_runtime_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceRuntimeStatus, ApplicationError>;

    /// Lists the running services in a workspace.
    fn list_running_services(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceRuntimeStatus>, ApplicationError>;

    /// Lists a workspace's libraries.
    fn list_libraries(&self, workspace_id: &WorkspaceId) -> Result<Vec<Library>, ApplicationError>;

    /// Returns a library by id.
    fn get_library(&self, library_id: &LibraryId) -> Result<Library, ApplicationError>;

    /// Inspects a library's configuration and repository state.
    async fn get_library_status(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryStatus, ApplicationError>;

    /// Lists the services that depend on a library.
    fn find_library_dependents(
        &self,
        library_id: &LibraryId,
    ) -> Result<Vec<LibraryDependent>, ApplicationError>;

    /// Verifies a library's artifact in the local repository.
    async fn verify_library_artifact(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryArtifactVerification, ApplicationError>;

    /// Inspects a library's consumer version alignment.
    async fn inspect_library_version_alignment(
        &self,
        library_id: &LibraryId,
    ) -> Result<service_orchestrator_application::VersionAlignmentPlan, ApplicationError>;

    // -- git (read-only) ----------------------------------------------------

    /// Inspects a service's repository status.
    async fn get_git_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<RepositoryStatus, ApplicationError>;

    /// Lists a service's repository branches.
    async fn list_branches(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<BranchInfo>, ApplicationError>;

    /// Returns a bounded diff of a service's repository.
    async fn get_repository_diff(
        &self,
        service_id: &ServiceId,
        options: &DiffOptions,
    ) -> Result<RepoDiff, ApplicationError>;

    // -- migrations, health, diagnostics, logs ------------------------------

    /// Returns a service's configuration-derived migration status.
    fn migration_status(&self, service_id: &ServiceId)
        -> Result<MigrationStatus, ApplicationError>;

    /// Returns the last observed health of a service.
    fn get_service_health(&self, service_id: &ServiceId)
        -> Result<ServiceHealth, ApplicationError>;

    /// Lists the last observed health of every service in a workspace.
    fn list_service_health(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceHealth>, ApplicationError>;

    /// Produces a structured diagnostic of a service's observed state.
    fn get_service_diagnostics(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceDiagnostics, ApplicationError>;

    /// Returns one stable page of a service's logs.
    fn get_service_logs(
        &self,
        service_id: &ServiceId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError>;

    /// Returns one stable page of a task's logs.
    fn get_task_logs(
        &self,
        task_id: &TaskId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError>;

    /// Returns bounded log entries matching a request.
    fn get_logs(&self, request: &LogRequest) -> Result<Vec<LogEntry>, ApplicationError>;

    /// Subscribes to realtime logs for a service.
    fn subscribe_service_logs(
        &self,
        service_id: &ServiceId,
    ) -> Result<Box<dyn LogSubscription>, ApplicationError>;

    // -- tasks and workflows ------------------------------------------------

    /// Returns a task by id.
    fn get_task(&self, task_id: &TaskId) -> Result<Task, ApplicationError>;

    /// Returns a workflow by id.
    fn get_workflow(&self, workflow_id: &WorkflowId) -> Result<Workflow, ApplicationError>;

    /// Aggregates a workflow's tasks into a bulk-operation summary.
    fn summarize_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<BulkOperationSummary, ApplicationError>;

    /// Requests cancellation of a task.
    fn cancel_task(&self, task_id: &TaskId) -> Result<(), ApplicationError>;

    // -- git commands -------------------------------------------------------

    /// Synchronizes a service's repository.
    fn sync_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Synchronizes a service's repository (the legacy Git entry point).
    fn sync_repository(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Fetches a service's repository.
    fn fetch_repository(&self, service_id: &ServiceId)
        -> Result<OperationResult, ApplicationError>;
    /// Pulls a service's repository.
    fn pull_repository(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Checks out a branch in a service's repository.
    fn checkout_repository(
        &self,
        service_id: &ServiceId,
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError>;
    /// Clones a service's repository.
    fn clone_repository(&self, service_id: &ServiceId)
        -> Result<OperationResult, ApplicationError>;
    /// Hard-resets a service's repository (destructive).
    fn reset_repository_hard(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Cleans a service's repository (destructive).
    fn clean_repository(&self, service_id: &ServiceId)
        -> Result<OperationResult, ApplicationError>;
    /// Discards local changes in a service's repository (destructive).
    fn discard_local_changes(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Synchronizes the selected repositories in a workspace.
    fn sync_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError>;
    /// Fetches the selected repositories in a workspace.
    fn fetch_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError>;
    /// Checks out a branch in the selected repositories of a workspace.
    fn checkout_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError>;

    // -- builds and libraries -----------------------------------------------

    /// Builds a service.
    fn build_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Builds a service's repository (alias).
    fn build_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Builds a library.
    fn build_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError>;
    /// Installs a library's artifact locally.
    fn install_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError>;
    /// Synchronizes a library's repository.
    fn sync_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError>;
    /// Runs a library prepare workflow.
    fn prepare_library(
        &self,
        library_id: &LibraryId,
        options: &LibraryPrepareOptions,
    ) -> Result<OperationResult, ApplicationError>;
    /// Prepares several libraries in dependency order.
    fn prepare_selected_libraries(
        &self,
        request: PrepareSelectedLibrariesRequest,
    ) -> Result<OperationResult, ApplicationError>;

    // -- service runtime ----------------------------------------------------

    /// Starts a service.
    fn start_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Stops a service.
    fn stop_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Restarts a service.
    fn restart_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Starts the selected services in a workspace.
    fn start_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError>;
    /// Stops the selected services in a workspace.
    fn stop_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError>;
    /// Restarts the selected services in a workspace.
    fn restart_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError>;

    // -- migrations ---------------------------------------------------------

    /// Validates migrations for a service.
    fn validate_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Reports migration status for a service.
    fn get_migration_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Inspects pending changesets for a service.
    fn inspect_migration_pending(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Previews migrations for a service.
    fn preview_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Applies migrations for a service.
    fn apply_migration(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;
    /// Applies migrations for the selected services in a workspace.
    fn apply_selected_migrations(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError>;
    /// Runs Liquibase for a service.
    fn run_liquibase(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError>;

    // -- health -------------------------------------------------------------

    /// Runs the selected health checks for a service.
    fn check_service_health(
        &self,
        service_id: &ServiceId,
        scope: HealthCheckScope,
    ) -> Result<OperationResult, ApplicationError>;
    /// Runs only the required readiness checks for a service.
    fn check_service_readiness(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError>;

    // -- workflows ----------------------------------------------------------

    /// Synchronizes every repository in a workspace.
    fn sync_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Builds the selected services in a workspace.
    fn prepare_environment(
        &self,
        request: PrepareEnvironmentRequest,
    ) -> Result<OperationResult, ApplicationError>;
    /// Starts every service in a group.
    fn start_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Stops every service in a group.
    fn stop_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Starts every service in a profile.
    fn start_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError>;
    /// Stops every service in a profile.
    fn stop_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError>;
}

/// Makes the application core usable as an in-process client.
///
/// This is what keeps the TUI and MCP adapter tests on the real core: they wrap
/// an [`Application`] in an `Arc<dyn OrchestratorClient>` and exercise exactly
/// the same method calls a remote client would receive.
#[async_trait]
impl OrchestratorClient for Application {
    fn config_summary(&self) -> ConfigSummary {
        let config = self.config();
        ConfigSummary {
            version: config.version,
            max_parallel_tasks: config.execution.max_parallel_tasks,
            default_timeout_seconds: config.execution.default_timeout_seconds,
        }
    }

    fn with_authorization(&self, authorization: Authorization) -> Arc<dyn OrchestratorClient> {
        Arc::new(self.clone().with_authorization(authorization))
    }

    fn list_workspaces(&self) -> Result<Vec<Workspace>, ApplicationError> {
        Ok(Application::list_workspaces(self))
    }

    fn get_workspace(&self, workspace_id: &WorkspaceId) -> Result<Workspace, ApplicationError> {
        Application::get_workspace(self, workspace_id)
    }

    fn get_workspace_status(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceStatus, ApplicationError> {
        Application::get_workspace_status(self, workspace_id)
    }

    fn list_groups(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceGroup>, ApplicationError> {
        Application::list_groups(self, workspace_id)
    }

    fn get_group(&self, group_id: &GroupId) -> Result<ServiceGroup, ApplicationError> {
        Application::get_group(self, group_id)
    }

    fn list_profiles(&self, workspace_id: &WorkspaceId) -> Result<Vec<Profile>, ApplicationError> {
        Application::list_profiles(self, workspace_id)
    }

    fn get_profile(&self, profile_id: &ProfileId) -> Result<Profile, ApplicationError> {
        Application::get_profile(self, profile_id)
    }

    fn list_services(&self, workspace_id: &WorkspaceId) -> Result<Vec<Service>, ApplicationError> {
        Application::list_services(self, workspace_id)
    }

    fn get_service(&self, service_id: &ServiceId) -> Result<Service, ApplicationError> {
        Application::get_service(self, service_id)
    }

    fn get_service_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceStatus, ApplicationError> {
        Application::get_service_status(self, service_id)
    }

    fn get_service_runtime_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceRuntimeStatus, ApplicationError> {
        Application::get_service_runtime_status(self, service_id)
    }

    fn list_running_services(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceRuntimeStatus>, ApplicationError> {
        Application::list_running_services(self, workspace_id)
    }

    fn list_libraries(&self, workspace_id: &WorkspaceId) -> Result<Vec<Library>, ApplicationError> {
        Application::list_libraries(self, workspace_id)
    }

    fn get_library(&self, library_id: &LibraryId) -> Result<Library, ApplicationError> {
        Application::get_library(self, library_id)
    }

    async fn get_library_status(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryStatus, ApplicationError> {
        Application::get_library_status(self, library_id).await
    }

    fn find_library_dependents(
        &self,
        library_id: &LibraryId,
    ) -> Result<Vec<LibraryDependent>, ApplicationError> {
        Application::find_library_dependents(self, library_id)
    }

    async fn verify_library_artifact(
        &self,
        library_id: &LibraryId,
    ) -> Result<LibraryArtifactVerification, ApplicationError> {
        Application::verify_library_artifact(self, library_id).await
    }

    async fn inspect_library_version_alignment(
        &self,
        library_id: &LibraryId,
    ) -> Result<service_orchestrator_application::VersionAlignmentPlan, ApplicationError> {
        Application::inspect_library_version_alignment(self, library_id).await
    }

    async fn get_git_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<RepositoryStatus, ApplicationError> {
        Application::get_git_status(self, service_id).await
    }

    async fn list_branches(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<BranchInfo>, ApplicationError> {
        Application::list_branches(self, service_id).await
    }

    async fn get_repository_diff(
        &self,
        service_id: &ServiceId,
        options: &DiffOptions,
    ) -> Result<RepoDiff, ApplicationError> {
        Application::get_repository_diff(self, service_id, options).await
    }

    fn migration_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<MigrationStatus, ApplicationError> {
        Application::migration_status(self, service_id)
    }

    fn get_service_health(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceHealth, ApplicationError> {
        Application::get_service_health(self, service_id)
    }

    fn list_service_health(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceHealth>, ApplicationError> {
        Application::list_service_health(self, workspace_id)
    }

    fn get_service_diagnostics(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceDiagnostics, ApplicationError> {
        Application::get_service_diagnostics(self, service_id)
    }

    fn get_service_logs(
        &self,
        service_id: &ServiceId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError> {
        Application::get_service_logs(self, service_id, request)
    }

    fn get_task_logs(
        &self,
        task_id: &TaskId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError> {
        Application::get_task_logs(self, task_id, request)
    }

    fn get_logs(&self, request: &LogRequest) -> Result<Vec<LogEntry>, ApplicationError> {
        Application::get_logs(self, request)
    }

    fn subscribe_service_logs(
        &self,
        service_id: &ServiceId,
    ) -> Result<Box<dyn LogSubscription>, ApplicationError> {
        Application::subscribe_service_logs(self, service_id)
    }

    fn get_task(&self, task_id: &TaskId) -> Result<Task, ApplicationError> {
        Application::get_task(self, task_id)
    }

    fn get_workflow(&self, workflow_id: &WorkflowId) -> Result<Workflow, ApplicationError> {
        Application::get_workflow(self, workflow_id)
    }

    fn summarize_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<BulkOperationSummary, ApplicationError> {
        Application::summarize_workflow(self, workflow_id)
    }

    fn cancel_task(&self, task_id: &TaskId) -> Result<(), ApplicationError> {
        Application::cancel_task(self, task_id)
    }

    fn sync_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::sync_service(self, service_id)
    }

    fn sync_repository(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::sync_repository(self, service_id)
    }

    fn fetch_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::fetch_repository(self, service_id)
    }

    fn pull_repository(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::pull_repository(self, service_id)
    }

    fn checkout_repository(
        &self,
        service_id: &ServiceId,
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError> {
        Application::checkout_repository(self, service_id, branch)
    }

    fn clone_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::clone_repository(self, service_id)
    }

    fn reset_repository_hard(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::reset_repository_hard(self, service_id)
    }

    fn clean_repository(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::clean_repository(self, service_id)
    }

    fn discard_local_changes(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::discard_local_changes(self, service_id)
    }

    fn sync_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        Application::sync_repositories(self, workspace_id, services)
    }

    fn fetch_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        Application::fetch_repositories(self, workspace_id, services)
    }

    fn checkout_repositories(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
        branch: &Branch,
    ) -> Result<OperationResult, ApplicationError> {
        Application::checkout_repositories(self, workspace_id, services, branch)
    }

    fn build_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::build_service(self, service_id)
    }

    fn build_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::build_workspace(self, workspace_id)
    }

    fn build_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError> {
        Application::build_library(self, library_id)
    }

    fn install_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError> {
        Application::install_library(self, library_id)
    }

    fn sync_library(&self, library_id: &LibraryId) -> Result<OperationResult, ApplicationError> {
        Application::sync_library(self, library_id)
    }

    fn prepare_library(
        &self,
        library_id: &LibraryId,
        options: &LibraryPrepareOptions,
    ) -> Result<OperationResult, ApplicationError> {
        Application::prepare_library(self, library_id, options)
    }

    fn prepare_selected_libraries(
        &self,
        request: PrepareSelectedLibrariesRequest,
    ) -> Result<OperationResult, ApplicationError> {
        Application::prepare_selected_libraries(self, request)
    }

    fn start_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::start_service(self, service_id)
    }

    fn stop_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::stop_service(self, service_id)
    }

    fn restart_service(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::restart_service(self, service_id)
    }

    fn start_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        Application::start_services(self, workspace_id, services)
    }

    fn stop_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        Application::stop_services(self, workspace_id, services)
    }

    fn restart_services(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        Application::restart_services(self, workspace_id, services)
    }

    fn validate_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::validate_migration(self, service_id)
    }

    fn get_migration_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::get_migration_status(self, service_id)
    }

    fn inspect_migration_pending(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::inspect_migration_pending(self, service_id)
    }

    fn preview_migration(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::preview_migration(self, service_id)
    }

    fn apply_migration(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::apply_migration(self, service_id)
    }

    fn apply_selected_migrations(
        &self,
        workspace_id: &WorkspaceId,
        services: &[ServiceId],
    ) -> Result<OperationResult, ApplicationError> {
        Application::apply_selected_migrations(self, workspace_id, services)
    }

    fn run_liquibase(&self, service_id: &ServiceId) -> Result<OperationResult, ApplicationError> {
        Application::run_liquibase(self, service_id)
    }

    fn check_service_health(
        &self,
        service_id: &ServiceId,
        scope: HealthCheckScope,
    ) -> Result<OperationResult, ApplicationError> {
        Application::check_service_health(self, service_id, scope)
    }

    fn check_service_readiness(
        &self,
        service_id: &ServiceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::check_service_readiness(self, service_id)
    }

    fn sync_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::sync_workspace(self, workspace_id)
    }

    fn prepare_environment(
        &self,
        request: PrepareEnvironmentRequest,
    ) -> Result<OperationResult, ApplicationError> {
        Application::prepare_environment(self, request)
    }

    fn start_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::start_group(self, workspace_id, group_id)
    }

    fn stop_group(
        &self,
        workspace_id: &WorkspaceId,
        group_id: &GroupId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::stop_group(self, workspace_id, group_id)
    }

    fn start_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::start_profile(self, workspace_id, profile_id)
    }

    fn stop_profile(
        &self,
        workspace_id: &WorkspaceId,
        profile_id: &ProfileId,
    ) -> Result<OperationResult, ApplicationError> {
        Application::stop_profile(self, workspace_id, profile_id)
    }
}

/// Silences an unused-import warning when the `HealthState` re-export is not
/// otherwise referenced by a downstream crate.
#[allow(unused_imports)]
use HealthState as _HealthState;
