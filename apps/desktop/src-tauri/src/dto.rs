//! Read-only view models handed to the frontend.
//!
//! The DTOs are deliberately slimmer than the core types: the GUI renders a
//! table row and a workspace card, not the full domain aggregate. Mapping
//! happens exactly once, here, so neither the frontend nor the Tauri commands
//! re-derive domain data (`tasks/018.md` §6, §11).

use serde::Serialize;
use service_orchestrator_application::{
    BuildCapabilities, BulkOperationSummary, HealthCheckOutcome, LibraryArtifactVerification,
    LibraryDependent, LibraryStatus, LogEntry, LogPage, MigrationStatus, OperationResult,
    ServiceDiagnostics, ServiceHealth, ServiceStatus, VersionAlignmentPlan, WorkflowCancellation,
    WorkspaceStatus as CoreWorkspaceStatus,
};
use service_orchestrator_domain::{
    ArtifactCoordinate, BranchInfo, BuildOperation, Dependency, DependencyTarget, HealthCheck,
    Library, Repository, RepositoryStatus, Runtime, Service, Task, TaskProgress, Timestamp,
    Workflow, Workspace,
};

/// A workspace as shown in the selector and the dashboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkspaceSummary {
    /// The workspace identifier.
    pub id: String,
    /// The human-readable workspace name.
    pub name: String,
    /// The absolute filesystem root of the workspace.
    pub root: String,
    /// How many services the workspace configures.
    pub service_count: usize,
}

impl From<Workspace> for WorkspaceSummary {
    fn from(workspace: Workspace) -> Self {
        Self {
            id: workspace.id.to_string(),
            name: workspace.name,
            root: workspace.root.to_string(),
            service_count: workspace.services.len(),
        }
    }
}

/// One row of the services table: configuration plus observed state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceSummary {
    /// The service identifier.
    pub id: String,
    /// The human-readable service name.
    pub display_name: String,
    /// `backend`, `frontend`, `library`, or a project-specific category.
    pub kind: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The last observed process state (snake_case).
    pub process_state: String,
    /// The last observed health state (snake_case).
    pub health_state: String,
    /// The last observed Git working-copy state (snake_case).
    pub git_state: String,
    /// The branch currently checked out, when known.
    pub branch: Option<String>,
    /// The first declared port, when the service declares one.
    pub port: Option<u16>,
}

impl From<ServiceStatus> for ServiceSummary {
    fn from(status: ServiceStatus) -> Self {
        Self {
            id: status.service_id.to_string(),
            display_name: status.display_name,
            kind: status.kind.label().to_owned(),
            workspace_id: status.workspace_id.to_string(),
            process_state: enum_value(&status.process_state),
            health_state: enum_value(&status.health_state),
            git_state: enum_value(&status.git_state),
            branch: status.branch.map(|branch| branch.to_string()),
            port: status.ports.first().map(|port| port.value()),
        }
    }
}

/// A workspace with the status of each of its services.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkspaceStatus {
    /// The workspace identifier.
    pub workspace_id: String,
    /// The human-readable workspace name.
    pub name: String,
    /// The absolute filesystem root of the workspace.
    pub root: String,
    /// One row per configured service.
    pub services: Vec<ServiceSummary>,
}

impl From<CoreWorkspaceStatus> for WorkspaceStatus {
    fn from(status: CoreWorkspaceStatus) -> Self {
        Self {
            workspace_id: status.workspace_id.to_string(),
            name: status.name,
            root: status.root.to_string(),
            services: status
                .services
                .into_iter()
                .map(ServiceSummary::from)
                .collect(),
        }
    }
}

/// Serializes a snake_case domain enum to its wire string.
///
/// The core types already declare `#[serde(rename_all = "snake_case")]`, so
/// serde is the single source of truth for the strings the frontend types
/// declare.
fn enum_value<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The normalized outcome of a mutating operation.
///
/// The wire shape is deliberately flatter than the core's
/// [`OperationResult`](service_orchestrator_application::OperationResult): the
/// frontend branches on `status` and follows `task_id`/`workflow_id` to observe
/// real task-engine progress rather than re-deriving meaning from a tagged
/// enum (`tasks/019.md` §11, §25).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationOutcome {
    /// What the daemon scheduled — or that nothing was needed.
    pub status: OperationStatus,
    /// The scheduled task id, when a single task was scheduled.
    pub task_id: Option<String>,
    /// The scheduled workflow id, when a workflow was scheduled.
    pub workflow_id: Option<String>,
    /// The task or workflow kind label (`start_service`, `start_services`, …).
    pub kind: String,
}

/// How an operation resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// A single task was submitted.
    Task,
    /// A workflow (bulk operation) was submitted.
    Workflow,
    /// The target was already in the requested state; nothing was scheduled.
    AlreadyInDesiredState,
}

impl From<OperationResult> for OperationOutcome {
    fn from(result: OperationResult) -> Self {
        match result {
            OperationResult::TaskScheduled { task_id, kind } => Self {
                status: OperationStatus::Task,
                task_id: Some(task_id.to_string()),
                workflow_id: None,
                kind: kind.label().to_owned(),
            },
            OperationResult::WorkflowScheduled { workflow_id, kind } => Self {
                status: OperationStatus::Workflow,
                task_id: None,
                workflow_id: Some(workflow_id.to_string()),
                kind: kind.label().to_owned(),
            },
            OperationResult::AlreadyInDesiredState { kind } => Self {
                status: OperationStatus::AlreadyInDesiredState,
                task_id: None,
                workflow_id: None,
                kind: kind.label().to_owned(),
            },
        }
    }
}

/// A runtime requirement in display form (`java 21`, `node 20.19.4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuntimeView {
    /// The runtime label (`java`, `node`, `maven`, `npm`, or a custom label).
    pub kind: String,
    /// The pinned version, when the service pins one.
    pub version: Option<String>,
}

impl From<&Runtime> for RuntimeView {
    fn from(runtime: &Runtime) -> Self {
        Self {
            kind: runtime.kind.label().to_owned(),
            version: runtime.version.as_ref().map(ToString::to_string),
        }
    }
}

/// The source repository of a service.
///
/// The URL is the validated configuration value; the domain rejects embedded
/// credentials, so no secret can reach the webview (`tasks/019.md` §18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RepositoryView {
    /// The clone URL (never containing credentials).
    pub url: String,
    /// The checkout path relative to the workspace root.
    pub local_path: String,
    /// The branch checked out when no other branch is requested.
    pub default_branch: String,
    /// The hosting provider label.
    pub provider: String,
}

impl From<&Repository> for RepositoryView {
    fn from(repository: &Repository) -> Self {
        Self {
            url: repository.url.clone(),
            local_path: repository.local_path.as_str().to_owned(),
            default_branch: repository.default_branch.to_string(),
            provider: repository.provider.to_string(),
        }
    }
}

/// A declared dependency of a service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DependencyView {
    /// The dependency target's identifier.
    pub target: String,
    /// Whether the target is a `service` or a `library`.
    pub target_kind: String,
    /// Whether the dependency is needed for `build` or `runtime`.
    pub kind: String,
    /// Whether the dependency is mandatory.
    pub required: bool,
}

impl From<&Dependency> for DependencyView {
    fn from(dependency: &Dependency) -> Self {
        let (target_kind, target) = match &dependency.target {
            DependencyTarget::Service(id) => ("service", id.to_string()),
            DependencyTarget::Library(id) => ("library", id.to_string()),
        };
        Self {
            target,
            target_kind: target_kind.to_owned(),
            kind: dependency.kind.to_string(),
            required: dependency.required,
        }
    }
}

/// A configured health check of a service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HealthCheckView {
    /// A display label such as `process`, `port 8081`, or `http <url>`.
    pub kind: String,
    /// Whether the check is required for the service to be considered healthy.
    pub required: bool,
}

impl From<&HealthCheck> for HealthCheckView {
    fn from(check: &HealthCheck) -> Self {
        Self {
            kind: check.kind.to_string(),
            required: check.required,
        }
    }
}

/// The latest observed outcome of one configured health check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HealthOutcomeView {
    /// The probe type (`process`, `tcp`, `http`, or a provider label).
    pub check_type: String,
    /// What was probed, if anything (`localhost:8081`, an HTTP URL).
    pub target: Option<String>,
    /// Whether the check is required.
    pub required: bool,
    /// The observed check state (snake_case `HealthState`).
    pub state: String,
    /// How long the probe took, in milliseconds.
    pub duration_ms: u64,
    /// A short, redacted explanation, present for a failure.
    pub message: Option<String>,
    /// When the check ran, seconds since the Unix epoch.
    pub checked_at: i64,
}

impl From<&HealthCheckOutcome> for HealthOutcomeView {
    fn from(outcome: &HealthCheckOutcome) -> Self {
        Self {
            check_type: outcome.check_type.clone(),
            target: outcome.target.clone(),
            required: outcome.required,
            state: enum_value(&outcome.status),
            duration_ms: outcome.duration_ms,
            message: outcome.message.clone(),
            checked_at: timestamp_seconds(outcome.checked_at),
        }
    }
}

/// A structured, observed failure recorded in a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticFailureView {
    /// Where the failure was observed (`health`, `log`, or `runtime`).
    pub source: String,
    /// A stable, machine-readable code.
    pub code: String,
    /// A human-readable summary.
    pub message: String,
    /// When the failure was observed, seconds since the Unix epoch.
    pub at: Option<i64>,
}

/// A detailed view of one service: configuration plus observed facts.
///
/// The detail never invents a value. Anything the adapter cannot observe yet —
/// for example the current commit, which needs a Git operation deferred to a
/// later task — is simply absent (`tasks/019.md` §18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceDetail {
    /// The service identifier.
    pub id: String,
    /// The human-readable service name.
    pub display_name: String,
    /// `backend`, `frontend`, or a project-specific category.
    pub kind: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The service directory relative to the workspace root, when configured.
    pub path: Option<String>,
    /// The source repository, when configured.
    pub repository: Option<RepositoryView>,
    /// The runtimes the service requires.
    pub runtimes: Vec<RuntimeView>,
    /// The configured build system, when known.
    pub build_system: Option<String>,
    /// The capabilities the service declares (snake_case).
    pub capabilities: Vec<String>,
    /// The last observed process state (snake_case).
    pub process_state: String,
    /// The last observed health state (snake_case).
    pub health_state: String,
    /// The last observed Git working-copy state (snake_case).
    pub git_state: String,
    /// The branch currently checked out, when known.
    pub branch: Option<String>,
    /// Whether the service is currently tracked as a managed process.
    pub managed: bool,
    /// The operating-system process id, while the process is alive.
    pub pid: Option<u32>,
    /// The ports the service declares.
    pub ports: Vec<u16>,
    /// The declared dependencies.
    pub dependencies: Vec<DependencyView>,
    /// The configured health checks.
    pub health_checks: Vec<HealthCheckView>,
    /// The latest outcome of each configured health check.
    pub health_outcomes: Vec<HealthOutcomeView>,
    /// When the service was last checked, seconds since the Unix epoch.
    pub last_checked_at: Option<i64>,
    /// The most recent structured failure, when one was observed.
    pub last_failure: Option<DiagnosticFailureView>,
    /// The most recent error-level log lines for the service.
    pub recent_errors: Vec<String>,
}

impl ServiceDetail {
    /// Combines configuration, observed status, and diagnostics.
    #[must_use]
    pub fn new(service: Service, status: ServiceStatus, diagnostics: ServiceDiagnostics) -> Self {
        Self {
            id: service.id.to_string(),
            display_name: service.display_name,
            kind: service.kind.label().to_owned(),
            workspace_id: status.workspace_id.to_string(),
            path: service.path.as_ref().map(|path| path.as_str().to_owned()),
            repository: service.repository.as_ref().map(RepositoryView::from),
            runtimes: service.runtimes.iter().map(RuntimeView::from).collect(),
            build_system: service.build_system.as_ref().map(ToString::to_string),
            capabilities: service.capabilities.iter().map(enum_value).collect(),
            process_state: enum_value(&status.process_state),
            health_state: enum_value(&status.health_state),
            git_state: enum_value(&status.git_state),
            branch: status.branch.map(|branch| branch.to_string()),
            managed: diagnostics.managed,
            pid: diagnostics.pid,
            ports: status.ports.iter().map(|port| port.value()).collect(),
            dependencies: service
                .dependencies
                .iter()
                .map(DependencyView::from)
                .collect(),
            health_checks: service
                .health_checks
                .iter()
                .map(HealthCheckView::from)
                .collect(),
            health_outcomes: diagnostics
                .health_checks
                .iter()
                .map(HealthOutcomeView::from)
                .collect(),
            last_checked_at: diagnostics.last_checked_at.map(timestamp_seconds),
            last_failure: diagnostics
                .last_failure
                .as_ref()
                .map(|failure| DiagnosticFailureView {
                    source: failure.source.clone(),
                    code: failure.code.clone(),
                    message: failure.message.clone(),
                    at: failure.at.map(timestamp_seconds),
                }),
            recent_errors: diagnostics.recent_errors,
        }
    }
}

/// Converts a domain timestamp to whole seconds since the Unix epoch.
pub(crate) fn timestamp_seconds(timestamp: Timestamp) -> i64 {
    i64::try_from(timestamp.epoch_millis() / 1000).unwrap_or(0)
}

// -- git ----------------------------------------------------------------------

/// One configured repository, as a row on the Git screen.
///
/// The row combines validated configuration — which service owns which
/// repository, its checkout path, and its default branch — with the last
/// *observed* Git state, when there is one. It performs no Git operation: the
/// real working tree is inspected on demand through `get_git_status`
/// (`tasks/020.md` §4, §6, §35).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RepositorySummary {
    /// A stable, workspace-aware repository identifier (`workspace:service`).
    pub id: String,
    /// The service that owns the repository.
    pub service_id: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The human-readable repository name.
    pub name: String,
    /// The service directory relative to the workspace root, when configured.
    pub path: Option<String>,
    /// The source repository, when configured.
    pub repository: Option<RepositoryView>,
    /// The last observed Git working-copy state (snake_case), or `unknown`.
    pub git_state: String,
    /// The branch last observed as checked out, when known.
    pub branch: Option<String>,
}

impl RepositorySummary {
    /// Builds a repository row from configuration and last-observed state.
    #[must_use]
    pub fn new(workspace_id: &str, service: &Service, status: Option<&ServiceStatus>) -> Self {
        Self {
            id: format!("{workspace_id}:{}", service.id),
            service_id: service.id.to_string(),
            workspace_id: workspace_id.to_owned(),
            name: service.display_name.clone(),
            path: service.path.as_ref().map(|path| path.as_str().to_owned()),
            repository: service.repository.as_ref().map(RepositoryView::from),
            git_state: status.map_or_else(
                || "unknown".to_owned(),
                |status| enum_value(&status.git_state),
            ),
            branch: status
                .and_then(|status| status.branch.as_ref())
                .map(ToString::to_string),
        }
    }
}

/// A structured `{ code, message }` error embedded in a per-item result.
///
/// A batch inspection must report one repository's failure without failing the
/// whole batch, so the error is carried as a value rather than a rejected
/// command (`tasks/020.md` §11, §30).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandErrorView {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable, user-safe message.
    pub message: String,
}

impl From<crate::error::CommandError> for CommandErrorView {
    fn from(error: crate::error::CommandError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

/// One repository's on-demand inspection result, success or failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RepositoryInspectionView {
    /// The stable, workspace-aware repository identifier.
    pub repository_id: String,
    /// The service that owns the repository.
    pub service_id: String,
    /// The inspected status, when inspection succeeded.
    pub status: Option<GitStatusView>,
    /// Why inspection failed, when it did.
    pub error: Option<CommandErrorView>,
}

/// A full working-tree inspection of one repository.
///
/// This mirrors [`RepositoryStatus`](service_orchestrator_domain::RepositoryStatus)
/// field for field, so the frontend can present the real ahead/behind, staged,
/// unstaged, untracked, and conflict counts without re-deriving them
/// (`tasks/020.md` §4, §8, §11).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GitStatusView {
    /// The service that owns the repository.
    pub service_id: String,
    /// The lifecycle of the checkout on disk (`available`, `not_cloned`, …).
    pub lifecycle: String,
    /// The checked-out branch, when not detached or unborn.
    pub branch: Option<String>,
    /// The current commit SHA, when there is one.
    pub commit: Option<String>,
    /// Whether `HEAD` is detached.
    pub detached_head: bool,
    /// Whether the branch is unborn (a fresh repository).
    pub unborn: bool,
    /// The configured upstream of the current branch, when any.
    pub upstream: Option<String>,
    /// How many commits the branch is ahead of its upstream, when known.
    pub ahead: Option<u32>,
    /// How many commits the branch is behind its upstream, when known.
    pub behind: Option<u32>,
    /// The number of staged changes.
    pub staged_changes: u32,
    /// The number of unstaged changes.
    pub unstaged_changes: u32,
    /// The number of untracked files.
    pub untracked_files: u32,
    /// The number of unmerged (conflicted) entries.
    pub conflicts: u32,
    /// Whether the working tree has no staged, unstaged, untracked, or
    /// conflicted entries.
    pub clean: bool,
    /// Whether any local change would be at risk from a destructive operation.
    pub has_local_changes: bool,
    /// Whether the branch is both ahead of and behind its upstream.
    pub diverged: bool,
}

impl GitStatusView {
    /// Builds an inspection view from a repository status.
    #[must_use]
    pub fn new(service_id: &str, status: &RepositoryStatus) -> Self {
        Self {
            service_id: service_id.to_owned(),
            lifecycle: status.lifecycle.label().to_owned(),
            branch: status.branch.as_ref().map(ToString::to_string),
            commit: status.commit.clone(),
            detached_head: status.detached_head,
            unborn: status.unborn,
            upstream: status.upstream.clone(),
            ahead: status.ahead,
            behind: status.behind,
            staged_changes: status.staged_changes,
            unstaged_changes: status.unstaged_changes,
            untracked_files: status.untracked_files,
            conflicts: status.conflicts,
            clean: status.is_clean(),
            has_local_changes: status.has_local_changes(),
            diverged: status.is_diverged(),
        }
    }
}

/// One branch in a repository listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BranchView {
    /// The branch name (`main`, `origin/main`).
    pub name: String,
    /// Whether the branch is `local` or `remote`.
    pub kind: String,
    /// Whether the branch is currently checked out.
    pub current: bool,
    /// The upstream branch name, when one is configured.
    pub upstream: Option<String>,
    /// The commit the branch points at, when known.
    pub commit: Option<String>,
}

impl From<&BranchInfo> for BranchView {
    fn from(info: &BranchInfo) -> Self {
        Self {
            name: info.name.clone(),
            kind: enum_value(&info.kind),
            current: info.current,
            upstream: info.upstream.clone(),
            commit: info.commit.clone(),
        }
    }
}

/// A read-only preview of a bulk Git operation over a selection.
///
/// The preview aggregates real status and branch information for each selected
/// repository so the user can see dirty working trees, missing target branches,
/// and diverged histories *before* anything mutates. It never stashes, resets,
/// or discards anything (`tasks/020.md` §7, §8, §9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GitSyncPreview {
    /// The workspace the operation targets.
    pub workspace_id: String,
    /// The target branch.
    pub target_branch: String,
    /// The requested mode (`sync`, `fetch`, or `checkout`).
    pub mode: String,
    /// One entry per selected repository.
    pub entries: Vec<GitSyncPreviewEntry>,
    /// The number of selected repositories.
    pub total: usize,
    /// How many are ready without warnings.
    pub ready: usize,
    /// How many have uncommitted local changes.
    pub dirty: usize,
    /// How many are missing the target branch.
    pub missing_branch: usize,
    /// How many have diverged from their upstream.
    pub diverged: usize,
    /// How many could not be inspected (not cloned, invalid, or unavailable).
    pub unavailable: usize,
}

/// One repository's entry in a bulk Git preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GitSyncPreviewEntry {
    /// The stable repository identifier.
    pub repository_id: String,
    /// The service that owns the repository.
    pub service_id: String,
    /// The human-readable repository name.
    pub name: String,
    /// The branch currently checked out, when known.
    pub current_branch: Option<String>,
    /// Whether the target branch exists locally or as a remote-tracking ref.
    pub target_branch_exists: bool,
    /// Whether the matching target ref is `local` or `remote`.
    pub target_branch_kind: Option<String>,
    /// The lifecycle of the checkout on disk.
    pub lifecycle: String,
    /// The last observed working-copy state.
    pub git_state: String,
    /// Whether the working tree is clean.
    pub clean: bool,
    /// How many commits ahead of upstream, when known.
    pub ahead: Option<u32>,
    /// How many commits behind upstream, when known.
    pub behind: Option<u32>,
    /// Whether the branch has diverged from its upstream.
    pub diverged: bool,
    /// The operations the daemon will run for this repository.
    pub expected_operations: Vec<String>,
    /// Human-readable warnings that do not stop the operation.
    pub warnings: Vec<String>,
    /// Whether the operation cannot safely proceed for this repository.
    pub blocked: bool,
}

// -- build --------------------------------------------------------------------

/// One service row on the Builds screen: configuration plus provider
/// capabilities.
///
/// The capability list is the build adapter's own registry projected read-only
/// through the application core, so the GUI never hard-codes Maven or npm
/// knowledge (`tasks/020.md` §12, §13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildTargetSummary {
    /// The service identifier.
    pub service_id: String,
    /// The human-readable service name.
    pub display_name: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The service category.
    pub kind: String,
    /// The configured build system, when known.
    pub build_system: Option<String>,
    /// The runtimes the service requires.
    pub runtimes: Vec<RuntimeView>,
    /// The operations the build system supports, when a provider is registered.
    pub capability: Option<BuildCapabilitiesView>,
    /// Why capabilities could not be resolved, when they could not.
    pub capability_error: Option<String>,
}

impl BuildTargetSummary {
    /// Builds a build target row from a service and its resolved capabilities.
    #[must_use]
    pub fn new(
        workspace_id: &str,
        service: &Service,
        capability: Option<&BuildCapabilities>,
        capability_error: Option<String>,
    ) -> Self {
        Self {
            service_id: service.id.to_string(),
            display_name: service.display_name.clone(),
            workspace_id: workspace_id.to_owned(),
            kind: service.kind.label().to_owned(),
            build_system: service.build_system.as_ref().map(ToString::to_string),
            runtimes: service.runtimes.iter().map(RuntimeView::from).collect(),
            capability: capability.map(BuildCapabilitiesView::from),
            capability_error,
        }
    }
}

/// The build operations a build system's provider supports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildCapabilitiesView {
    /// The build system the capabilities describe.
    pub build_system: String,
    /// One entry per build-system-neutral operation, in a stable order.
    pub operations: Vec<BuildOperationView>,
    /// Whether the provider supports no operation at all.
    pub empty: bool,
}

impl From<&BuildCapabilities> for BuildCapabilitiesView {
    fn from(capabilities: &BuildCapabilities) -> Self {
        // Enumerate the closed operation set explicitly: the core deliberately
        // exposes no `all()` so a new variant forces a compile error here
        // rather than silently vanishing from the UI.
        let all = [
            BuildOperation::Clean,
            BuildOperation::Build,
            BuildOperation::Test,
            BuildOperation::InstallDependencies,
            BuildOperation::PublishLocalArtifact,
        ];
        let operations = all
            .into_iter()
            .map(|operation| BuildOperationView {
                operation: operation.label().to_owned(),
                supported: capabilities.supports(operation),
                // Only `Build` is reachable through the application core's
                // service build use case today; the rest are reported honestly
                // as unsupported-by-the-application rather than hidden.
                exposed: operation == BuildOperation::Build,
                reason: if capabilities.supports(operation) {
                    (operation != BuildOperation::Build).then(|| {
                        "supported by the provider but not exposed by the application core for services"
                            .to_owned()
                    })
                } else {
                    Some("the configured build provider does not support this operation".to_owned())
                },
            })
            .collect();
        Self {
            build_system: capabilities.build_system.label().to_owned(),
            operations,
            empty: capabilities.is_empty(),
        }
    }
}

/// One build operation in a provider's capability list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildOperationView {
    /// The operation label (`clean`, `build`, `test`, …).
    pub operation: String,
    /// Whether the configured provider supports the operation.
    pub supported: bool,
    /// Whether the application core exposes the operation for a service.
    pub exposed: bool,
    /// Why the operation is disabled, when it is.
    pub reason: Option<String>,
}

// -- library ------------------------------------------------------------------

/// A Maven-style artifact coordinate in display form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArtifactView {
    /// The group identifier.
    pub group_id: String,
    /// The artifact identifier.
    pub artifact_id: String,
    /// The artifact version.
    pub version: String,
    /// The packaging (`jar`, `pom`, …), defaulting to `jar`.
    pub packaging: String,
    /// The classifier, when the artifact has one.
    pub classifier: Option<String>,
    /// The artifact's file name.
    pub file_name: String,
    /// The canonical `group:artifact:version` coordinate.
    pub coordinate: String,
}

impl From<&ArtifactCoordinate> for ArtifactView {
    fn from(artifact: &ArtifactCoordinate) -> Self {
        Self {
            group_id: artifact.group_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            version: artifact.version.to_string(),
            packaging: artifact.packaging_or_default().to_owned(),
            classifier: artifact.classifier.clone(),
            file_name: artifact.file_name(),
            coordinate: artifact.to_string(),
        }
    }
}

/// One library row on the Libraries screen.
///
/// The row combines validated configuration with best-effort observed state
/// (repository branch/commit and Git state) and the services that depend on it.
/// It never rewrites a POM or changes a version (`tasks/020.md` §20, §23, §24).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LibrarySummary {
    /// The library identifier.
    pub id: String,
    /// The human-readable library name.
    pub display_name: String,
    /// The workspace that owns the library.
    pub workspace_id: String,
    /// The configured build system, when known.
    pub build_system: Option<String>,
    /// The configured repository URL, when any (never containing credentials).
    pub repository_url: Option<String>,
    /// The primary artifact coordinate, when configured.
    pub artifact: Option<ArtifactView>,
    /// Every artifact the library produces (primary first).
    pub artifacts: Vec<ArtifactView>,
    /// The branch last observed as checked out, when known.
    pub branch: Option<String>,
    /// The commit last observed, when known.
    pub commit: Option<String>,
    /// The last observed Git working-copy state.
    pub git_state: String,
    /// Why repository state could not be inspected, when it could not.
    pub repository_error: Option<String>,
    /// The configured version of the primary artifact, when known.
    pub configured_version: Option<String>,
    /// The services that depend on the library.
    pub dependents: Vec<LibraryDependentView>,
}

impl LibrarySummary {
    /// Builds a library row from configuration, observed status, and dependents.
    #[must_use]
    pub fn new(
        library: &Library,
        status: Option<&LibraryStatus>,
        dependents: &[LibraryDependent],
    ) -> Self {
        let artifacts = library
            .produced_artifacts()
            .into_iter()
            .map(ArtifactView::from)
            .collect();
        Self {
            id: library.id.to_string(),
            display_name: library.display_name.clone(),
            workspace_id: status
                .map(|status| status.workspace_id.to_string())
                .unwrap_or_default(),
            build_system: library.build_system.as_ref().map(ToString::to_string),
            repository_url: library.repository.as_ref().map(|repo| repo.url.clone()),
            artifact: library.artifact.as_ref().map(ArtifactView::from),
            artifacts,
            branch: status
                .and_then(|status| status.branch.as_ref())
                .map(ToString::to_string),
            commit: status.and_then(|status| status.commit.clone()),
            git_state: status.map_or_else(
                || "unknown".to_owned(),
                |status| enum_value(&status.git_state),
            ),
            repository_error: status.and_then(|status| status.repository_error.clone()),
            configured_version: library
                .artifact
                .as_ref()
                .map(|artifact| artifact.version.to_string()),
            dependents: dependents.iter().map(LibraryDependentView::from).collect(),
        }
    }
}

/// A service that depends on a library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LibraryDependentView {
    /// The dependent service.
    pub service_id: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// Whether the dependency is mandatory.
    pub required: bool,
    /// Whether the dependency is needed for `build` or `runtime`.
    pub kind: String,
}

impl From<&LibraryDependent> for LibraryDependentView {
    fn from(dependent: &LibraryDependent) -> Self {
        Self {
            service_id: dependent.service_id.to_string(),
            workspace_id: dependent.workspace_id.to_string(),
            required: dependent.required,
            kind: dependent.kind.to_string(),
        }
    }
}

/// The result of verifying a library's artifact in the local repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ArtifactVerificationView {
    /// The library identifier.
    pub library_id: String,
    /// The workspace that owns the library.
    pub workspace_id: String,
    /// The artifact that was verified.
    pub artifact: ArtifactView,
    /// Whether the artifact is present in the effective local repository.
    pub present: bool,
    /// The artifact file's absolute path, when it was found.
    pub path: Option<String>,
    /// A human-readable summary.
    pub message: String,
}

impl From<&LibraryArtifactVerification> for ArtifactVerificationView {
    fn from(verification: &LibraryArtifactVerification) -> Self {
        Self {
            library_id: verification.library_id.to_string(),
            workspace_id: verification.workspace_id.to_string(),
            artifact: ArtifactView::from(&verification.artifact),
            present: verification.present,
            path: verification.path.as_ref().map(ToString::to_string),
            message: verification.message.clone(),
        }
    }
}

/// One consumer's alignment with a library's target version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConsumerAlignmentView {
    /// The dependent service.
    pub service_id: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The consumer module's absolute directory, when it could be resolved.
    pub path: Option<String>,
    /// The version the consumer resolves to, when known.
    pub current_version: Option<String>,
    /// The alignment outcome (`aligned`, `mismatch`, `unknown`, `unresolved`).
    pub status: String,
    /// Additional context, for example why a version is unknown.
    pub detail: Option<String>,
}

/// A structured version-alignment view for a library and its consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VersionAlignmentView {
    /// The library identifier.
    pub library_id: String,
    /// The workspace that owns the library.
    pub workspace_id: String,
    /// The coordinate whose version consumers should align with.
    pub target: ArtifactView,
    /// One entry per dependent service.
    pub consumers: Vec<ConsumerAlignmentView>,
    /// How many consumers are aligned with the target version.
    pub aligned_count: usize,
    /// How many consumers were inspected.
    pub total: usize,
    /// Whether every consumer is aligned.
    pub fully_aligned: bool,
}

impl From<&VersionAlignmentPlan> for VersionAlignmentView {
    fn from(plan: &VersionAlignmentPlan) -> Self {
        Self {
            library_id: plan.library_id.to_string(),
            workspace_id: plan.workspace_id.to_string(),
            target: ArtifactView::from(&plan.target),
            consumers: plan
                .consumers
                .iter()
                .map(|consumer| ConsumerAlignmentView {
                    service_id: consumer.service_id.to_string(),
                    workspace_id: consumer.workspace_id.to_string(),
                    path: consumer.path.as_ref().map(ToString::to_string),
                    current_version: consumer.current_version.as_ref().map(ToString::to_string),
                    status: consumer.status.label().to_owned(),
                    detail: consumer.detail.clone(),
                })
                .collect(),
            aligned_count: plan.aligned_count(),
            total: plan.consumers.len(),
            fully_aligned: plan.is_fully_aligned(),
        }
    }
}

// -- tasks, workflows, and logs ----------------------------------------------

/// A task-engine task in display form.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskView {
    /// The task identifier.
    pub id: String,
    /// The task kind label (`build_service`, `git_sync`, …).
    pub kind: String,
    /// The task lifecycle status (`queued`, `running`, `succeeded`, …).
    pub status: String,
    /// The service the task acts on, when any.
    pub service_id: Option<String>,
    /// The library the task acts on, when any.
    pub library_id: Option<String>,
    /// The workflow the task belongs to, when any.
    pub workflow_id: Option<String>,
    /// The workspace that owns the task's service or library, when it could be
    /// resolved from configuration (`tasks/021.md` §10).
    pub workspace_id: Option<String>,
    /// When the task was created, seconds since the Unix epoch.
    pub created_at: i64,
    /// When the task started running, seconds since the Unix epoch.
    pub started_at: Option<i64>,
    /// When the task reached a terminal state, seconds since the Unix epoch.
    pub finished_at: Option<i64>,
    /// How long the task ran, in milliseconds, once it has started.
    pub duration_ms: Option<u64>,
    /// The task's progress.
    pub progress: TaskProgressView,
    /// The task's structured failure, when it failed.
    pub failure: Option<TaskFailureView>,
    /// The ids of the tasks this one depends on.
    pub dependencies: Vec<String>,
}

impl From<&Task> for TaskView {
    fn from(task: &Task) -> Self {
        Self {
            id: task.id.to_string(),
            kind: task.kind.label().to_owned(),
            status: task.status.label().to_owned(),
            service_id: task.service.as_ref().map(ToString::to_string),
            library_id: task.library.as_ref().map(ToString::to_string),
            workflow_id: task.workflow.as_ref().map(ToString::to_string),
            workspace_id: None,
            created_at: timestamp_seconds(task.created_at),
            started_at: task.started_at.map(timestamp_seconds),
            finished_at: task.finished_at.map(timestamp_seconds),
            duration_ms: task_duration_millis(task),
            progress: TaskProgressView::from(&task.progress),
            failure: task.error.as_ref().map(|error| TaskFailureView {
                code: error.code.clone(),
                message: error.message.clone(),
                detail: error.detail.clone(),
            }),
            dependencies: task.dependencies.iter().map(ToString::to_string).collect(),
        }
    }
}

impl TaskView {
    /// Records the resolved owning workspace on a task row.
    #[must_use]
    pub fn with_workspace(mut self, workspace_id: impl Into<String>) -> Self {
        self.workspace_id = Some(workspace_id.into());
        self
    }
}

/// How far along a task is.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TaskProgressView {
    /// The number of completed units of work.
    pub current: u32,
    /// The total number of units, when known.
    pub total: Option<u32>,
    /// An optional human-readable progress message.
    pub message: Option<String>,
    /// The completion fraction in `0.0..=1.0`, when the total is known.
    pub fraction: Option<f64>,
}

impl From<&TaskProgress> for TaskProgressView {
    fn from(progress: &TaskProgress) -> Self {
        Self {
            current: progress.current,
            total: progress.total,
            message: progress.message.clone(),
            fraction: progress.fraction(),
        }
    }
}

/// A structured task failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TaskFailureView {
    /// A stable, machine-readable code.
    pub code: String,
    /// A human-readable summary.
    pub message: String,
    /// Optional additional detail, such as the process exit code.
    pub detail: Option<String>,
}

/// An aggregate of a workflow's per-task outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct BulkSummaryView {
    /// The total number of tasks.
    pub total: u32,
    /// Tasks that completed successfully.
    pub succeeded: u32,
    /// Tasks that failed.
    pub failed: u32,
    /// Tasks that were cancelled.
    pub cancelled: u32,
    /// Tasks that were deliberately skipped.
    pub skipped: u32,
    /// Tasks still queued, blocked, or running.
    pub pending: u32,
    /// Whether no task is still pending.
    pub complete: bool,
}

impl From<BulkOperationSummary> for BulkSummaryView {
    fn from(summary: BulkOperationSummary) -> Self {
        Self {
            total: summary.total,
            succeeded: summary.succeeded,
            failed: summary.failed,
            cancelled: summary.cancelled,
            skipped: summary.skipped,
            pending: summary.pending,
            complete: summary.is_complete(),
        }
    }
}

/// The result of requesting cancellation of a workflow's tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowCancellationView {
    /// How many still-active tasks were asked to cancel.
    pub cancelled_tasks: u32,
    /// Whether every task was already terminal when cancellation was requested.
    pub already_complete: bool,
}

impl From<WorkflowCancellation> for WorkflowCancellationView {
    fn from(cancellation: WorkflowCancellation) -> Self {
        Self {
            cancelled_tasks: cancellation.cancelled_tasks,
            already_complete: cancellation.already_complete,
        }
    }
}

/// A single structured log entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LogEntryView {
    /// When the entry was produced, seconds since the Unix epoch.
    pub timestamp: i64,
    /// The severity (`trace`, `debug`, `info`, `warn`, `error`).
    pub level: String,
    /// The log message, already redacted of obvious secret material.
    pub message: String,
    /// Where the entry originated (`process`, `task`, `build`, …).
    pub source: String,
    /// The originating output stream (`stdout`/`stderr`), for process entries.
    pub stream: Option<String>,
    /// The service the entry belongs to, when known.
    pub service_id: Option<String>,
    /// The task the entry belongs to, when known.
    pub task_id: Option<String>,
    /// The managed process the entry belongs to, when known.
    pub process_id: Option<String>,
    /// The entry's monotonic sequence, used for cursor pagination.
    pub sequence: u64,
}

impl From<&LogEntry> for LogEntryView {
    fn from(entry: &LogEntry) -> Self {
        Self {
            timestamp: timestamp_seconds(entry.timestamp),
            level: enum_value(&entry.level),
            message: entry.message.clone(),
            source: enum_value(&entry.source),
            stream: entry.stream.as_ref().map(enum_value),
            service_id: entry.service_id.as_ref().map(ToString::to_string),
            task_id: entry.task_id.as_ref().map(ToString::to_string),
            process_id: entry.process_id.clone(),
            sequence: entry.sequence,
        }
    }
}

/// A bounded page of log entries with its continuation cursor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LogPageView {
    /// The matched entries, oldest first.
    pub entries: Vec<LogEntryView>,
    /// The cursor to request the next page, or `null` when there is none.
    pub next_cursor: Option<u64>,
    /// Whether more matching entries existed than the limit allowed.
    pub truncated: bool,
}

impl From<LogPage> for LogPageView {
    fn from(page: LogPage) -> Self {
        Self {
            entries: page.entries.iter().map(LogEntryView::from).collect(),
            next_cursor: page.next_cursor,
            truncated: page.truncated,
        }
    }
}

/// A service's migration setup in display form.
///
/// The snapshot is configuration-derived and performs no database operation:
/// [`pending`](Self::pending) and [`locked`](Self::locked) are explicit unknowns
/// (`null` on the wire) until a real provider inspection task answers
/// (`tasks/021.md` §4, §6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MigrationStatusView {
    /// The service whose migrations are described.
    pub service_id: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// Whether migrations are enabled for the service.
    pub enabled: bool,
    /// The configured migration provider (`liquibase`, …).
    pub provider: String,
    /// How the provider is invoked (`cli`, `maven`, …).
    pub execution_mode: String,
    /// The declared environment of the target database (`local`, `staging`, …).
    pub environment: String,
    /// A display-safe identity of the target database (never credentials).
    pub database_identity: String,
    /// Whether applying to the target requires explicit authorization.
    pub requires_authorization: bool,
    /// The configured migration timeout, in seconds.
    pub timeout_seconds: Option<u64>,
    /// The number of pending changesets, when known — always `null` from the
    /// configuration snapshot.
    pub pending: Option<u32>,
    /// Whether the target is locked by another migration, when known — always
    /// `null` from the configuration snapshot.
    pub locked: Option<bool>,
}

impl From<&MigrationStatus> for MigrationStatusView {
    fn from(status: &MigrationStatus) -> Self {
        Self {
            service_id: status.service_id.to_string(),
            workspace_id: status.workspace_id.to_string(),
            enabled: status.enabled,
            provider: enum_value(&status.provider),
            execution_mode: enum_value(&status.execution_mode),
            environment: enum_value(&status.environment),
            database_identity: status.database_identity.clone(),
            requires_authorization: status.requires_authorization,
            timeout_seconds: status.timeout_seconds,
            pending: status.pending,
            locked: status.locked,
        }
    }
}

/// One row of the Migrations table: configuration plus the latest task.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MigrationRowView {
    /// The configuration-derived migration setup.
    pub status: MigrationStatusView,
    /// The most recent migration task observed for the service, when any.
    pub latest_task: Option<TaskView>,
}

// -- workflows ----------------------------------------------------------------

/// One step of a workflow, in display form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowStepView {
    /// The step identifier, unique within its workflow.
    pub id: String,
    /// The step description, when one is configured.
    pub description: Option<String>,
    /// The kind of task the step performs (`migration_apply`, …).
    pub task_kind: String,
    /// The ids of the steps that must complete before this one.
    pub depends_on: Vec<String>,
}

/// A workflow's declared structure, in display form.
///
/// Relationships come from real step ids, task kinds, and dependency edges —
/// never from parsing task names (`tasks/021.md` §16).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowView {
    /// The workflow identifier.
    pub id: String,
    /// The workflow kind label (`prepare_environment`, …).
    pub kind: String,
    /// The declared steps, in order.
    pub steps: Vec<WorkflowStepView>,
}

impl From<&Workflow> for WorkflowView {
    fn from(workflow: &Workflow) -> Self {
        Self {
            id: workflow.id.to_string(),
            kind: workflow.kind.label().to_owned(),
            steps: workflow
                .steps
                .iter()
                .map(|step| WorkflowStepView {
                    id: step.id.to_string(),
                    description: step.description.clone(),
                    task_kind: step.task.label().to_owned(),
                    depends_on: step.depends_on.iter().map(ToString::to_string).collect(),
                })
                .collect(),
        }
    }
}

// -- health -------------------------------------------------------------------

/// One row of the Health table: runtime state plus last observed health.
///
/// Runtime state and health state are deliberately kept distinct: a `running`
/// process may be `unhealthy`, and a starting process is `checking`
/// (`tasks/021.md` §17, §18).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HealthRowView {
    /// The service identifier.
    pub service_id: String,
    /// The human-readable service name.
    pub display_name: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The last observed process state (snake_case).
    pub process_state: String,
    /// The last observed aggregated health state (snake_case).
    pub health_state: String,
    /// When the service was last checked, seconds since the Unix epoch.
    pub checked_at: Option<i64>,
    /// When the service was last observed healthy.
    pub last_healthy_at: Option<i64>,
    /// When the service was last observed unhealthy.
    pub last_unhealthy_at: Option<i64>,
    /// How many consecutive observations have failed.
    pub consecutive_failures: u32,
    /// The latest process-liveness check outcome, when one was observed.
    pub process_check: Option<HealthOutcomeView>,
    /// The latest TCP/port reachability check outcome, when one was observed.
    pub tcp_check: Option<HealthOutcomeView>,
    /// The latest HTTP (or actuator) readiness check outcome, when observed.
    pub http_check: Option<HealthOutcomeView>,
}

impl HealthRowView {
    /// Builds a row from the observed service status and health index entry.
    ///
    /// A service that has never been checked gets an explicit `unknown` health
    /// state rather than an invented one (`tasks/021.md` §22).
    #[must_use]
    pub fn new(status: &ServiceStatus, health: &ServiceHealth) -> Self {
        let check = |kind: &str| {
            health
                .checks
                .iter()
                .find(|outcome| outcome.check_type == kind)
                .map(HealthOutcomeView::from)
        };
        Self {
            service_id: status.service_id.to_string(),
            display_name: status.display_name.clone(),
            workspace_id: status.workspace_id.to_string(),
            process_state: enum_value(&status.process_state),
            health_state: enum_value(&health.state),
            checked_at: health.checked_at.map(timestamp_seconds),
            last_healthy_at: health.last_healthy_at.map(timestamp_seconds),
            last_unhealthy_at: health.last_unhealthy_at.map(timestamp_seconds),
            consecutive_failures: health.consecutive_failures,
            process_check: check("process"),
            tcp_check: check("tcp"),
            http_check: check("http"),
        }
    }
}

/// The detailed health view of one service.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HealthDetailView {
    /// The dashboard row this detail expands.
    pub row: HealthRowView,
    /// The latest outcome of every configured check, with duration and reason.
    pub checks: Vec<HealthOutcomeView>,
    /// Whether the service is currently tracked as a managed process.
    pub managed: bool,
    /// The operating-system process id, while the process is alive.
    pub pid: Option<u32>,
    /// The ports the service declares.
    pub ports: Vec<u16>,
    /// The most recent structured failure, when one was observed.
    pub last_failure: Option<DiagnosticFailureView>,
    /// The most recent error-level log lines for the service, oldest first.
    pub recent_errors: Vec<String>,
}

// -- realtime log streaming ---------------------------------------------------

/// A realtime log-stream event pushed to the frontend.
///
/// The `kind` discriminates: `entry` carries a log line, `lagged` reports
/// entries the daemon dropped for this subscriber, and `closed` ends the
/// stream. A lagging stream is reported honestly rather than silently
/// shrinking (`tasks/021.md` §28).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LogStreamEvent {
    /// The frontend-generated subscription id this event belongs to.
    pub subscription_id: String,
    /// The service whose stream produced the event.
    pub service_id: String,
    /// `entry`, `lagged`, or `closed`.
    pub kind: String,
    /// The log entry, for `entry` events.
    pub entry: Option<LogEntryView>,
    /// How many entries the daemon dropped, for `lagged` events.
    pub dropped: Option<u64>,
    /// A short, human-readable explanation for `lagged` and `closed` events.
    pub message: Option<String>,
}

impl LogStreamEvent {
    /// Creates an `entry` event for one subscription.
    #[must_use]
    pub fn entry(subscription_id: &str, service_id: &str, entry: &LogEntry) -> Self {
        Self {
            subscription_id: subscription_id.to_owned(),
            service_id: service_id.to_owned(),
            kind: "entry".to_owned(),
            entry: Some(LogEntryView::from(entry)),
            dropped: None,
            message: None,
        }
    }

    /// Creates a `lagged` event reporting dropped entries.
    #[must_use]
    pub fn lagged(subscription_id: &str, service_id: &str, dropped: u64) -> Self {
        Self {
            subscription_id: subscription_id.to_owned(),
            service_id: service_id.to_owned(),
            kind: "lagged".to_owned(),
            entry: None,
            dropped: Some(dropped),
            message: Some(format!(
                "the log stream fell behind; {dropped} entries were dropped and this view may be incomplete"
            )),
        }
    }

    /// Creates a `closed` event ending a subscription's stream.
    #[must_use]
    pub fn closed(subscription_id: &str, service_id: &str, message: &str) -> Self {
        Self {
            subscription_id: subscription_id.to_owned(),
            service_id: service_id.to_owned(),
            kind: "closed".to_owned(),
            entry: None,
            dropped: None,
            message: Some(message.to_owned()),
        }
    }
}

/// Returns a task's wall-clock duration in milliseconds, when it has started.
fn task_duration_millis(task: &Task) -> Option<u64> {
    let started = task.started_at?;
    let finished = task.finished_at.unwrap_or_else(Timestamp::now);
    u64::try_from(
        finished
            .epoch_millis()
            .saturating_sub(started.epoch_millis()),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use service_orchestrator_application::{
        AlignmentStatus, ConsumerAlignment, DiagnosticFailure, HealthState, LogLevel, LogSource,
        LogStream,
    };
    use service_orchestrator_domain::{
        ArtifactCoordinate, Branch, BranchInfo, BranchKind, BuildSystem, DependencyKind, GitState,
        HealthCheckKind, Library, LibraryId, Port, ProcessState, RelativePath, RepositoryLifecycle,
        RepositoryStatus, RuntimeKind, ServiceId, ServiceType, Task, TaskFailure, TaskId, TaskKind,
        TaskProgress, TaskStatus, Timestamp, Version, WorkflowId, WorkflowKind, WorkspaceId,
    };

    use super::*;

    fn status() -> ServiceStatus {
        ServiceStatus {
            service_id: ServiceId::new("orders-api").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            display_name: "orders-api".to_owned(),
            kind: ServiceType::Backend,
            process_state: ProcessState::NotRunning,
            health_state: HealthState::Unknown,
            git_state: GitState::Clean,
            branch: Some(Branch::new("main").expect("a valid branch")),
            ports: vec![Port::new(8081).expect("a valid port")],
            capabilities: Default::default(),
        }
    }

    fn diagnostics() -> ServiceDiagnostics {
        ServiceDiagnostics {
            service_id: ServiceId::new("orders-api").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            process_state: ProcessState::Running,
            health_state: HealthState::Unhealthy,
            managed: true,
            process_id: Some("proc-1".to_owned()),
            pid: Some(4321),
            ports: vec![Port::new(8081).expect("a valid port")],
            last_checked_at: Some(Timestamp::now()),
            health_checks: vec![HealthCheckOutcome {
                check_type: "http".to_owned(),
                target: Some("http://localhost:8081/actuator/health".to_owned()),
                required: true,
                status: HealthState::Unhealthy,
                duration_ms: 12,
                message: Some("connection refused".to_owned()),
                checked_at: Timestamp::now(),
            }],
            last_failure: Some(DiagnosticFailure {
                source: "health".to_owned(),
                code: "HTTP_CONNECTION_REFUSED".to_owned(),
                message: "connection refused".to_owned(),
                at: Some(Timestamp::now()),
            }),
            recent_errors: vec!["port 8081 in use".to_owned()],
            notes: Vec::new(),
        }
    }

    fn service() -> Service {
        let mut service = Service::new(
            ServiceId::new("orders-api").expect("a valid id"),
            "orders-api",
            ServiceType::Backend,
        )
        .expect("a valid service");
        service = service
            .with_path(RelativePath::new("backend/orders-api").expect("a valid path"))
            .with_runtime(Runtime::versioned(
                RuntimeKind::Java,
                Version::new("21").expect("a valid version"),
            ))
            .with_dependency(Dependency::runtime_service(
                ServiceId::new("admin-ui").expect("a valid id"),
            ))
            .with_health_check(HealthCheck::new(HealthCheckKind::Process))
            .with_port(Port::new(8081).expect("a valid port"));
        service.build_system = Some(service_orchestrator_domain::BuildSystem::Maven);
        service
    }

    #[test]
    fn service_summaries_carry_the_fields_the_table_renders() {
        let summary = ServiceSummary::from(status());
        assert_eq!(summary.id, "orders-api");
        assert_eq!(summary.display_name, "orders-api");
        assert_eq!(summary.kind, "backend");
        assert_eq!(summary.workspace_id, "demo");
        assert_eq!(summary.process_state, "not_running");
        assert_eq!(summary.health_state, "unknown");
        assert_eq!(summary.git_state, "clean");
        assert_eq!(summary.branch.as_deref(), Some("main"));
        assert_eq!(summary.port, Some(8081));
    }

    #[test]
    fn missing_branch_and_ports_stay_null() {
        let mut status = status();
        status.branch = None;
        status.ports = Vec::new();
        let summary = ServiceSummary::from(status);
        assert_eq!(summary.branch, None);
        assert_eq!(summary.port, None);
    }

    #[test]
    fn open_service_kinds_keep_their_custom_label() {
        let mut status = status();
        status.kind = ServiceType::Other("worker".to_owned());
        assert_eq!(ServiceSummary::from(status).kind, "worker");
    }

    #[test]
    fn workspace_summaries_count_configured_services() {
        let workspace = Workspace::new(
            WorkspaceId::new("demo").expect("a valid id"),
            "Demo Platform",
            service_orchestrator_domain::AbsolutePath::new("/Users/dev/Projects/demo")
                .expect("a valid path"),
        )
        .expect("a valid workspace");
        let summary = WorkspaceSummary::from(workspace);
        assert_eq!(summary.id, "demo");
        assert_eq!(summary.name, "Demo Platform");
        assert_eq!(summary.service_count, 0);
        assert_eq!(summary.root, "/Users/dev/Projects/demo");
    }

    #[test]
    fn process_and_health_states_serialize_snake_case() {
        assert_eq!(enum_value(&ProcessState::NotRunning), "not_running");
        assert_eq!(enum_value(&ProcessState::Running), "running");
        assert_eq!(enum_value(&HealthState::Unhealthy), "unhealthy");
    }

    #[test]
    fn operation_outcomes_flatten_each_variant() {
        let task = OperationOutcome::from(OperationResult::TaskScheduled {
            task_id: TaskId::new("task-1").expect("a valid id"),
            kind: TaskKind::StartService,
        });
        assert_eq!(task.status, OperationStatus::Task);
        assert_eq!(task.task_id.as_deref(), Some("task-1"));
        assert_eq!(task.workflow_id, None);
        assert_eq!(task.kind, "start_service");

        let workflow = OperationOutcome::from(OperationResult::WorkflowScheduled {
            workflow_id: WorkflowId::new("wf-1").expect("a valid id"),
            kind: WorkflowKind::Custom("start_services".to_owned()),
        });
        assert_eq!(workflow.status, OperationStatus::Workflow);
        assert_eq!(workflow.workflow_id.as_deref(), Some("wf-1"));
        assert_eq!(workflow.task_id, None);
        assert_eq!(workflow.kind, "start_services");

        let noop = OperationOutcome::from(OperationResult::AlreadyInDesiredState {
            kind: TaskKind::StopService,
        });
        assert_eq!(noop.status, OperationStatus::AlreadyInDesiredState);
        assert_eq!(noop.task_id, None);
        assert_eq!(noop.workflow_id, None);
        assert_eq!(noop.kind, "stop_service");
    }

    #[test]
    fn operation_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(OperationStatus::AlreadyInDesiredState).expect("serializes"),
            serde_json::json!("already_in_desired_state")
        );
    }

    #[test]
    fn service_detail_combines_configuration_status_and_diagnostics() {
        let mut observed = status();
        observed.process_state = ProcessState::Running;
        observed.health_state = HealthState::Unhealthy;
        let detail = ServiceDetail::new(service(), observed, diagnostics());
        assert_eq!(detail.id, "orders-api");
        assert_eq!(detail.kind, "backend");
        assert_eq!(detail.workspace_id, "demo");
        assert_eq!(detail.path.as_deref(), Some("backend/orders-api"));
        assert_eq!(detail.runtimes.len(), 1);
        assert_eq!(detail.runtimes[0].kind, "java");
        assert_eq!(detail.runtimes[0].version.as_deref(), Some("21"));
        assert_eq!(detail.build_system.as_deref(), Some("maven"));
        assert_eq!(detail.dependencies.len(), 1);
        assert_eq!(detail.dependencies[0].target, "admin-ui");
        assert_eq!(detail.dependencies[0].target_kind, "service");
        assert_eq!(detail.dependencies[0].kind, "runtime");
        assert!(detail.dependencies[0].required);
        assert_eq!(detail.health_checks.len(), 1);
        assert_eq!(detail.health_checks[0].kind, "process");
        assert!(detail.health_checks[0].required);
        assert_eq!(detail.ports, vec![8081]);
        assert!(detail.managed);
        assert_eq!(detail.pid, Some(4321));
        // Observed status wins over the summary's not-running observation.
        assert_eq!(detail.process_state, "running");
        assert_eq!(detail.health_state, "unhealthy");
        assert_eq!(detail.branch.as_deref(), Some("main"));
    }

    #[test]
    fn service_detail_maps_health_outcomes_and_recent_failure() {
        let detail = ServiceDetail::new(service(), status(), diagnostics());
        assert_eq!(detail.health_outcomes.len(), 1);
        let outcome = &detail.health_outcomes[0];
        assert_eq!(outcome.check_type, "http");
        assert_eq!(
            outcome.target.as_deref(),
            Some("http://localhost:8081/actuator/health")
        );
        assert_eq!(outcome.state, "unhealthy");
        assert_eq!(outcome.message.as_deref(), Some("connection refused"));
        assert!(detail.last_checked_at.is_some());

        let failure = detail.last_failure.as_ref().expect("a failure is mapped");
        assert_eq!(failure.source, "health");
        assert_eq!(failure.code, "HTTP_CONNECTION_REFUSED");
        assert!(failure.at.is_some());
        assert_eq!(detail.recent_errors, vec!["port 8081 in use".to_owned()]);
    }

    #[test]
    fn unobserved_detail_fields_stay_absent() {
        let mut bare = service();
        bare.repository = None;
        bare.dependencies.clear();
        bare.health_checks.clear();
        let mut diagnostics = diagnostics();
        diagnostics.health_checks.clear();
        diagnostics.last_failure = None;
        diagnostics.pid = None;
        diagnostics.recent_errors.clear();
        let detail = ServiceDetail::new(bare, status(), diagnostics);
        assert!(detail.repository.is_none());
        assert!(detail.dependencies.is_empty());
        assert!(detail.health_checks.is_empty());
        assert!(detail.health_outcomes.is_empty());
        assert!(detail.last_failure.is_none());
        assert_eq!(detail.pid, None);
        assert!(detail.recent_errors.is_empty());
    }

    // -- git ----------------------------------------------------------------

    fn repository_status() -> RepositoryStatus {
        RepositoryStatus {
            lifecycle: RepositoryLifecycle::Available,
            branch: Some(Branch::new("feature/x").expect("a valid branch")),
            commit: Some("abc123".to_owned()),
            detached_head: false,
            unborn: false,
            upstream: Some("origin/feature/x".to_owned()),
            ahead: Some(2),
            behind: Some(1),
            staged_changes: 1,
            unstaged_changes: 2,
            untracked_files: 3,
            conflicts: 1,
        }
    }

    #[test]
    fn repository_summary_combines_configuration_with_observed_state() {
        let mut service = service();
        service.repository = Some(
            Repository::new(
                "git@github.com:acme/orders-api.git",
                RelativePath::new("backend/orders-api").expect("a valid path"),
                Branch::new("main").expect("a valid branch"),
            )
            .expect("a valid repository"),
        );
        let summary = RepositorySummary::new("demo", &service, Some(&status()));
        assert_eq!(summary.id, "demo:orders-api");
        assert_eq!(summary.service_id, "orders-api");
        assert_eq!(summary.workspace_id, "demo");
        assert_eq!(summary.name, "orders-api");
        assert_eq!(summary.git_state, "clean");
        assert_eq!(summary.branch.as_deref(), Some("main"));
        let repository = summary.repository.expect("mapped");
        assert_eq!(repository.default_branch, "main");
        assert_eq!(repository.local_path, "backend/orders-api");

        // Without an observation the state is honestly unknown.
        let unknown = RepositorySummary::new("demo", &service, None);
        assert_eq!(unknown.git_state, "unknown");
        assert_eq!(unknown.branch, None);
    }

    #[test]
    fn git_status_view_maps_counts_and_safety_flags() {
        let view = GitStatusView::new("orders-api", &repository_status());
        assert_eq!(view.service_id, "orders-api");
        assert_eq!(view.lifecycle, "available");
        assert_eq!(view.branch.as_deref(), Some("feature/x"));
        assert_eq!(view.commit.as_deref(), Some("abc123"));
        assert_eq!(view.upstream.as_deref(), Some("origin/feature/x"));
        assert_eq!(view.ahead, Some(2));
        assert_eq!(view.behind, Some(1));
        assert_eq!(view.staged_changes, 1);
        assert_eq!(view.unstaged_changes, 2);
        assert_eq!(view.untracked_files, 3);
        assert_eq!(view.conflicts, 1);
        assert!(!view.clean);
        assert!(view.has_local_changes);
        assert!(view.diverged);
    }

    #[test]
    fn a_clean_ahead_branch_is_not_reported_as_dirty_or_diverged() {
        let clean = RepositoryStatus {
            lifecycle: RepositoryLifecycle::Available,
            ahead: Some(1),
            ..RepositoryStatus::default()
        };
        let view = GitStatusView::new("orders-api", &clean);
        assert!(view.clean);
        assert!(!view.has_local_changes);
        assert!(!view.diverged);
    }

    #[test]
    fn branch_views_map_local_and_remote_refs() {
        let mut local = BranchInfo::new("main", BranchKind::Local, true);
        local.upstream = Some("origin/main".to_owned());
        local.commit = Some("abc".to_owned());
        let view = BranchView::from(&local);
        assert_eq!(view.name, "main");
        assert_eq!(view.kind, "local");
        assert!(view.current);
        assert_eq!(view.upstream.as_deref(), Some("origin/main"));
        assert_eq!(view.commit.as_deref(), Some("abc"));

        let remote = BranchView::from(&BranchInfo::new("origin/main", BranchKind::Remote, false));
        assert_eq!(remote.kind, "remote");
        assert!(!remote.current);
        assert_eq!(remote.upstream, None);
    }

    #[test]
    fn command_error_views_preserve_code_and_message() {
        let view = CommandErrorView::from(crate::error::CommandError::invalid_input("bad id"));
        assert_eq!(view.code, "INVALID_INPUT");
        assert_eq!(view.message, "bad id");
    }

    // -- build --------------------------------------------------------------

    fn maven_capabilities() -> BuildCapabilities {
        BuildCapabilities {
            build_system: BuildSystem::Maven,
            operations: vec![
                BuildOperation::Clean,
                BuildOperation::Build,
                BuildOperation::Test,
                BuildOperation::PublishLocalArtifact,
            ],
        }
    }

    #[test]
    fn build_capabilities_expose_only_the_service_build_operation() {
        let view = BuildCapabilitiesView::from(&maven_capabilities());
        assert_eq!(view.build_system, "maven");
        assert!(!view.empty);
        // The closed operation set is always reported in full.
        assert_eq!(view.operations.len(), 5);
        let find = |name: &str| {
            view.operations
                .iter()
                .find(|operation| operation.operation == name)
                .expect("present")
        };

        let build = find("build");
        assert!(build.supported);
        assert!(build.exposed);
        assert_eq!(build.reason, None);

        let clean = find("clean");
        assert!(clean.supported);
        assert!(!clean.exposed);
        assert!(clean
            .reason
            .as_deref()
            .expect("a reason")
            .contains("not exposed"));

        let install = find("install_dependencies");
        assert!(!install.supported);
        assert!(!install.exposed);
        assert!(install
            .reason
            .as_deref()
            .expect("a reason")
            .contains("does not support"));
    }

    #[test]
    fn a_provider_with_no_operations_reports_empty() {
        let view = BuildCapabilitiesView::from(&BuildCapabilities {
            build_system: BuildSystem::Npm,
            operations: Vec::new(),
        });
        assert!(view.empty);
        assert!(view.operations.iter().all(|operation| !operation.supported));
    }

    #[test]
    fn build_target_summary_carries_capabilities_or_a_reason() {
        let target = BuildTargetSummary::new("demo", &service(), Some(&maven_capabilities()), None);
        assert_eq!(target.service_id, "orders-api");
        assert_eq!(target.workspace_id, "demo");
        assert_eq!(target.build_system.as_deref(), Some("maven"));
        assert!(target.capability.is_some());
        assert_eq!(target.capability_error, None);
        assert_eq!(
            target.runtimes.first().map(|runtime| runtime.kind.as_str()),
            Some("java")
        );

        let unresolved =
            BuildTargetSummary::new("demo", &service(), None, Some("no provider".to_owned()));
        assert!(unresolved.capability.is_none());
        assert_eq!(unresolved.capability_error.as_deref(), Some("no provider"));
    }

    // -- library ------------------------------------------------------------

    fn coordinate(artifact_id: &str) -> ArtifactCoordinate {
        ArtifactCoordinate::new(
            "com.demo",
            artifact_id,
            Version::new("1.8.0-SNAPSHOT").expect("a valid version"),
        )
        .expect("a valid coordinate")
    }

    #[test]
    fn artifact_views_render_coordinates_files_and_classifiers() {
        let view = ArtifactView::from(
            &coordinate("shared-kit")
                .with_packaging("pom")
                .with_classifier("sources"),
        );
        assert_eq!(view.group_id, "com.demo");
        assert_eq!(view.artifact_id, "shared-kit");
        assert_eq!(view.version, "1.8.0-SNAPSHOT");
        assert_eq!(view.packaging, "pom");
        assert_eq!(view.classifier.as_deref(), Some("sources"));
        assert_eq!(view.file_name, "shared-kit-1.8.0-SNAPSHOT-sources.pom");
        assert_eq!(view.coordinate, "com.demo:shared-kit:1.8.0-SNAPSHOT");

        let default_packaging = ArtifactView::from(&coordinate("shared-kit"));
        assert_eq!(default_packaging.packaging, "jar");
        assert_eq!(default_packaging.file_name, "shared-kit-1.8.0-SNAPSHOT.jar");
    }

    #[test]
    fn library_summaries_collect_artifacts_and_dependents() {
        let primary = coordinate("shared-kit");
        let additional = coordinate("shared-kit-dto");
        let library = Library::new(
            LibraryId::new("shared-kit").expect("a valid id"),
            "Shared Kit",
        )
        .expect("a valid library")
        .with_artifact(primary.clone())
        .with_additional_artifact(additional.clone())
        .with_build_system(BuildSystem::Maven);
        let status = LibraryStatus {
            library_id: LibraryId::new("shared-kit").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            display_name: "Shared Kit".to_owned(),
            build_system: Some(BuildSystem::Maven),
            artifact: Some(primary.clone()),
            artifacts: vec![primary.clone(), additional.clone()],
            repository_url: Some("git@github.com:acme/shared-kit.git".to_owned()),
            branch: Some(Branch::new("develop").expect("a valid branch")),
            commit: Some("deadbeef".to_owned()),
            git_state: GitState::Modified,
            repository_error: None,
            artifact_present: Some(true),
        };
        let dependents = vec![LibraryDependent {
            service_id: ServiceId::new("orders-api").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            required: true,
            kind: DependencyKind::Build,
        }];

        let summary = LibrarySummary::new(&library, Some(&status), &dependents);
        assert_eq!(summary.id, "shared-kit");
        assert_eq!(summary.workspace_id, "demo");
        assert_eq!(summary.build_system.as_deref(), Some("maven"));
        assert_eq!(summary.artifacts.len(), 2);
        assert_eq!(
            summary
                .artifact
                .as_ref()
                .map(|artifact| artifact.artifact_id.clone()),
            Some("shared-kit".to_owned())
        );
        assert_eq!(
            summary.configured_version.as_deref(),
            Some("1.8.0-SNAPSHOT")
        );
        assert_eq!(summary.branch.as_deref(), Some("develop"));
        assert_eq!(summary.commit.as_deref(), Some("deadbeef"));
        assert_eq!(summary.git_state, "modified");
        assert_eq!(summary.dependents.len(), 1);
        assert_eq!(summary.dependents[0].service_id, "orders-api");
        assert_eq!(summary.dependents[0].kind, "build");

        // With no observed status the repository state stays honest.
        let bare = LibrarySummary::new(&library, None, &[]);
        assert_eq!(bare.workspace_id, "");
        assert_eq!(bare.git_state, "unknown");
        assert_eq!(bare.commit, None);
    }

    #[test]
    fn artifact_verification_reports_presence_and_path() {
        let verification = LibraryArtifactVerification {
            library_id: LibraryId::new("shared-kit").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            artifact: coordinate("shared-kit"),
            present: true,
            path: Some(
                service_orchestrator_domain::AbsolutePath::new(
                    "/Users/dev/.m2/repository/com/demo/shared-kit/1.8.0-SNAPSHOT/shared-kit-1.8.0-SNAPSHOT.jar",
                )
                .expect("a valid path"),
            ),
            message: "artifact present".to_owned(),
        };
        let view = ArtifactVerificationView::from(&verification);
        assert_eq!(view.library_id, "shared-kit");
        assert_eq!(view.workspace_id, "demo");
        assert!(view.present);
        assert!(view.path.as_deref().expect("a path").ends_with(".jar"));
        assert_eq!(view.message, "artifact present");
    }

    #[test]
    fn version_alignment_distinguishes_all_four_outcomes() {
        let consumer =
            |service: &str, alignment: AlignmentStatus, version: Option<&str>| ConsumerAlignment {
                service_id: ServiceId::new(service).expect("a valid id"),
                workspace_id: WorkspaceId::new("demo").expect("a valid id"),
                path: None,
                current_version: version.map(|value| Version::new(value).expect("a valid version")),
                status: alignment,
                detail: None,
            };
        let target = coordinate("shared-kit");
        let plan = VersionAlignmentPlan {
            library_id: LibraryId::new("shared-kit").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            target: target.clone(),
            consumers: vec![
                consumer(
                    "auth-service",
                    AlignmentStatus::Aligned,
                    Some("1.8.0-SNAPSHOT"),
                ),
                consumer(
                    "user-service",
                    AlignmentStatus::Mismatch,
                    Some("1.7.0-SNAPSHOT"),
                ),
                consumer("payment-service", AlignmentStatus::Unknown, None),
                consumer("billing-service", AlignmentStatus::Unresolved, None),
            ],
        };
        let view = VersionAlignmentView::from(&plan);
        assert_eq!(view.library_id, "shared-kit");
        assert_eq!(view.target.coordinate, "com.demo:shared-kit:1.8.0-SNAPSHOT");
        assert_eq!(view.total, 4);
        assert_eq!(view.aligned_count, 1);
        assert!(!view.fully_aligned);
        let statuses: Vec<&str> = view
            .consumers
            .iter()
            .map(|consumer| consumer.status.as_str())
            .collect();
        assert_eq!(
            statuses,
            vec!["aligned", "mismatch", "unknown", "unresolved"]
        );

        let empty = VersionAlignmentView::from(&VersionAlignmentPlan {
            library_id: LibraryId::new("shared-kit").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            target,
            consumers: Vec::new(),
        });
        assert!(empty.fully_aligned);
        assert_eq!(empty.total, 0);
    }

    // -- tasks, workflows, and logs -----------------------------------------

    #[test]
    fn task_views_map_lifecycle_progress_and_failure() {
        let mut task = Task::new(
            TaskId::new("task-1").expect("a valid id"),
            TaskKind::BuildService,
        )
        .with_service(ServiceId::new("orders-api").expect("a valid id"))
        .with_library(LibraryId::new("shared-kit").expect("a valid id"))
        .with_workflow(WorkflowId::new("wf-1").expect("a valid id"))
        .depends_on(TaskId::new("task-0").expect("a valid id"));
        task.status = TaskStatus::Failed;
        task.started_at = Some(Timestamp::now());
        task.finished_at = Some(Timestamp::now());
        task.progress = TaskProgress::new(1, Some(2), Some("building".to_owned())).expect("valid");
        task.error = Some(
            TaskFailure::new(
                "MAVEN_BUILD_FAILED",
                "build failed",
                Some("exit 1".to_owned()),
            )
            .expect("valid"),
        );

        let view = TaskView::from(&task);
        assert_eq!(view.id, "task-1");
        assert_eq!(view.kind, "build_service");
        assert_eq!(view.status, "failed");
        assert_eq!(view.service_id.as_deref(), Some("orders-api"));
        assert_eq!(view.library_id.as_deref(), Some("shared-kit"));
        assert_eq!(view.workflow_id.as_deref(), Some("wf-1"));
        assert_eq!(view.progress.current, 1);
        assert_eq!(view.progress.total, Some(2));
        assert_eq!(view.progress.fraction, Some(0.5));
        assert_eq!(view.dependencies, vec!["task-0".to_owned()]);
        assert!(view.duration_ms.is_some());
        let failure = view.failure.expect("mapped");
        assert_eq!(failure.code, "MAVEN_BUILD_FAILED");
        assert_eq!(failure.message, "build failed");
        assert_eq!(failure.detail.as_deref(), Some("exit 1"));
    }

    #[test]
    fn unknown_task_totals_leave_the_fraction_absent() {
        let view = TaskProgressView::from(&TaskProgress::default());
        assert_eq!(view.current, 0);
        assert_eq!(view.total, None);
        assert_eq!(view.fraction, None);
    }

    #[test]
    fn bulk_summary_views_compute_completion() {
        let complete = BulkSummaryView::from(BulkOperationSummary {
            total: 5,
            succeeded: 3,
            failed: 1,
            cancelled: 0,
            skipped: 1,
            pending: 0,
        });
        assert_eq!(complete.total, 5);
        assert_eq!(complete.succeeded, 3);
        assert_eq!(complete.failed, 1);
        assert_eq!(complete.skipped, 1);
        assert!(complete.complete);

        let pending = BulkSummaryView::from(BulkOperationSummary {
            total: 2,
            succeeded: 1,
            pending: 1,
            ..BulkOperationSummary::default()
        });
        assert!(!pending.complete);
    }

    #[test]
    fn workflow_cancellation_views_report_partial_cancellation() {
        let partial = WorkflowCancellationView::from(WorkflowCancellation {
            cancelled_tasks: 3,
            already_complete: false,
        });
        assert_eq!(partial.cancelled_tasks, 3);
        assert!(!partial.already_complete);

        let done = WorkflowCancellationView::from(WorkflowCancellation {
            cancelled_tasks: 0,
            already_complete: true,
        });
        assert!(done.already_complete);
    }

    #[test]
    fn log_entries_and_pages_map_levels_sources_and_streams() {
        let mut entry = LogEntry::new(LogLevel::Error, "build failed")
            .with_source(LogSource::Build)
            .with_stream(LogStream::Stderr)
            .with_service(ServiceId::new("orders-api").expect("a valid id"))
            .with_task(TaskId::new("task-1").expect("a valid id"))
            .with_process("proc-1");
        entry.sequence = 7;

        let view = LogEntryView::from(&entry);
        assert_eq!(view.level, "error");
        assert_eq!(view.source, "build");
        assert_eq!(view.stream.as_deref(), Some("stderr"));
        assert_eq!(view.service_id.as_deref(), Some("orders-api"));
        assert_eq!(view.task_id.as_deref(), Some("task-1"));
        assert_eq!(view.process_id.as_deref(), Some("proc-1"));
        assert_eq!(view.sequence, 7);

        let page = LogPageView::from(LogPage {
            entries: vec![entry],
            next_cursor: Some(7),
            truncated: true,
        });
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.next_cursor, Some(7));
        assert!(page.truncated);
    }

    #[test]
    fn bulk_summary_view_of_empty_workflow_is_complete() {
        let view = BulkSummaryView::from(BulkOperationSummary::default());
        assert_eq!(view.total, 0);
        assert!(view.complete);
    }
}
