//! Application request and response objects.
//!
//! These types are the machine-readable contract between the application core
//! and every interface. They are structured — never pre-formatted strings — so
//! the TUI and MCP adapters can render or serialize them independently without
//! re-deriving business meaning.
//!
//! They derive `serde` for the same reason domain values do: the values
//! genuinely cross the configuration and interface boundaries. Deriving it here
//! is not MCP-specific; the MCP adapter still owns its own response envelope.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{
    AbsolutePath, ArtifactCoordinate, Branch, BuildOperation, BuildSystem, DependencyKind,
    GitState, GroupId, LibraryId, LiquibaseExecutionMode, MigrationProviderType,
    MigrationTargetEnvironment, Port, ProcessState, ProfileId, ServiceCapability, ServiceId,
    ServiceType, Task, TaskId, TaskKind, TaskStatus, Timestamp, Version, WorkflowId, WorkflowKind,
    WorkflowState, WorkspaceId,
};

use crate::ports::{HealthCheckOutcome, HealthState};

/// The result of submitting a command.
///
/// Long-running operations return a task or workflow identifier rather than
/// blocking. A command that finds the target already in the desired state
/// returns [`OperationResult::AlreadyInDesiredState`] instead of scheduling
/// duplicate work, which is how idempotency is expressed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum OperationResult {
    /// A single task was submitted.
    TaskScheduled {
        /// The assigned task identifier.
        task_id: TaskId,
        /// The kind of task that was submitted.
        kind: TaskKind,
    },
    /// A workflow was submitted.
    WorkflowScheduled {
        /// The assigned workflow identifier.
        workflow_id: WorkflowId,
        /// The kind of workflow that was submitted.
        kind: WorkflowKind,
    },
    /// The target was already in the requested state; nothing was scheduled.
    AlreadyInDesiredState {
        /// The operation that was skipped.
        kind: TaskKind,
    },
}

impl OperationResult {
    /// Returns `true` if a task or workflow was actually scheduled.
    #[must_use]
    pub const fn is_scheduled(&self) -> bool {
        !matches!(self, Self::AlreadyInDesiredState { .. })
    }

    /// Returns the scheduled task identifier, if a single task was scheduled.
    #[must_use]
    pub const fn task_id(&self) -> Option<&TaskId> {
        match self {
            Self::TaskScheduled { task_id, .. } => Some(task_id),
            Self::WorkflowScheduled { .. } | Self::AlreadyInDesiredState { .. } => None,
        }
    }

    /// Returns the scheduled workflow identifier, if a workflow was scheduled.
    #[must_use]
    pub const fn workflow_id(&self) -> Option<&WorkflowId> {
        match self {
            Self::WorkflowScheduled { workflow_id, .. } => Some(workflow_id),
            Self::TaskScheduled { .. } | Self::AlreadyInDesiredState { .. } => None,
        }
    }
}

/// A request to prepare a development environment.
///
/// The request is explicit and structured so it maps cleanly onto an MCP tool
/// schema, and so a caller avoids a long positional argument list. Every option
/// has a documented, safe default: there is no implicit database mutation and no
/// destructive Git operation.
///
/// # Selection
///
/// The services to include are determined in this order: an explicit
/// [`services`](Self::services) list, otherwise a [`profile`](Self::profile),
/// otherwise a [`group`](Self::group), otherwise every service in the workspace.
/// Required dependencies are added automatically unless
/// [`include_dependencies`](Self::include_dependencies) is disabled.
///
/// # Defaults
///
/// A request created with [`PrepareEnvironmentRequest::new`] synchronizes Git,
/// builds nothing, applies no migrations, starts nothing, and waits for health
/// only if services are started. The `with_*`/`build`/`start` builders opt in to
/// the more expensive phases explicitly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareEnvironmentRequest {
    /// The workspace to prepare.
    pub workspace_id: WorkspaceId,
    /// The services to include. When empty, selection falls back to the
    /// [`profile`](Self::profile), then the [`group`](Self::group), then the whole
    /// workspace.
    pub services: Vec<ServiceId>,
    /// An optional development profile whose services should be selected.
    #[serde(default)]
    pub profile: Option<ProfileId>,
    /// An optional service group whose members should be selected.
    #[serde(default)]
    pub group: Option<GroupId>,
    /// An optional branch to check out before building.
    #[serde(default)]
    pub branch: Option<Branch>,
    /// Whether to synchronize repositories (fetch, optional checkout,
    /// fast-forward pull). Defaults to `true`.
    #[serde(default = "default_true")]
    pub sync: bool,
    /// Whether to include the transitive closure of required service and library
    /// dependencies. Defaults to `true`.
    #[serde(default = "default_true")]
    pub include_dependencies: bool,
    /// Whether to build the required libraries (`BuildLibrary`). Defaults to
    /// `false`.
    #[serde(default)]
    pub build_libraries: bool,
    /// Whether to run Liquibase migrations for migration-enabled services.
    /// Defaults to `false`; enabling it is an explicit database mutation.
    pub run_liquibase: bool,
    /// Whether to build the selected services. Defaults to `false`.
    pub build: bool,
    /// Whether to start the selected services. Defaults to `false`.
    pub start: bool,
    /// Whether to run configured health checks after starting services. Defaults
    /// to `true`; it has an effect only when [`start`](Self::start) is set.
    #[serde(default = "default_true")]
    pub wait_for_health: bool,
}

/// The `serde` default for boolean options that default to `true`.
const fn default_true() -> bool {
    true
}

impl PrepareEnvironmentRequest {
    /// Creates a request that selects services and only synchronizes Git.
    ///
    /// `sync` and `include_dependencies` default to `true`; library and service
    /// builds, migrations, and startup default to `false`.
    #[must_use]
    pub fn new(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            services: Vec::new(),
            profile: None,
            group: None,
            branch: None,
            sync: true,
            include_dependencies: true,
            build_libraries: false,
            run_liquibase: false,
            build: false,
            start: false,
            wait_for_health: true,
        }
    }

    /// Selects the services to include.
    #[must_use]
    pub fn with_services<I>(mut self, services: I) -> Self
    where
        I: IntoIterator<Item = ServiceId>,
    {
        self.services = services.into_iter().collect();
        self
    }

    /// Selects services through a development profile.
    #[must_use]
    pub fn with_profile(mut self, profile: ProfileId) -> Self {
        self.profile = Some(profile);
        self
    }

    /// Selects services through a service group.
    #[must_use]
    pub fn with_group(mut self, group: GroupId) -> Self {
        self.group = Some(group);
        self
    }

    /// Sets the branch to check out.
    #[must_use]
    pub fn with_branch(mut self, branch: Branch) -> Self {
        self.branch = Some(branch);
        self
    }

    /// Enables or disables Git synchronization.
    #[must_use]
    pub const fn with_sync(mut self, sync: bool) -> Self {
        self.sync = sync;
        self
    }

    /// Enables or disables automatic dependency inclusion.
    #[must_use]
    pub const fn with_dependencies(mut self, include: bool) -> Self {
        self.include_dependencies = include;
        self
    }

    /// Enables Liquibase migrations.
    #[must_use]
    pub const fn run_liquibase(mut self) -> Self {
        self.run_liquibase = true;
        self
    }

    /// Enables building libraries and services.
    ///
    /// This is the convenience switch used by callers that want both; use
    /// [`build_libraries`](Self::build_libraries) to build libraries without
    /// building services.
    #[must_use]
    pub const fn build(mut self) -> Self {
        self.build = true;
        self.build_libraries = true;
        self
    }

    /// Enables building libraries.
    #[must_use]
    pub const fn build_libraries(mut self) -> Self {
        self.build_libraries = true;
        self
    }

    /// Enables starting the selected services.
    #[must_use]
    pub const fn start(mut self) -> Self {
        self.start = true;
        self
    }

    /// Enables or disables waiting for health after startup.
    #[must_use]
    pub const fn with_wait_for_health(mut self, wait: bool) -> Self {
        self.wait_for_health = wait;
        self
    }
}

/// A phase of environment preparation.
///
/// Phases give a workflow a stable, human-meaningful grouping independent of the
/// individual task kinds: a `GitSync` phase contains fetch, checkout, and pull
/// steps alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentPhaseKind {
    /// Workspace and precondition validation.
    Validate,
    /// Repository synchronization.
    GitSync,
    /// Required-library preparation.
    LibraryPrepare,
    /// Database migrations.
    Migration,
    /// Service builds.
    ServiceBuild,
    /// Service startup.
    ServiceStart,
    /// Health and readiness checks.
    HealthCheck,
}

impl EnvironmentPhaseKind {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::GitSync => "git-sync",
            Self::LibraryPrepare => "library-prepare",
            Self::Migration => "migration",
            Self::ServiceBuild => "service-build",
            Self::ServiceStart => "service-start",
            Self::HealthCheck => "health-check",
        }
    }

    /// Maps a task kind onto the phase it belongs to.
    #[must_use]
    pub const fn from_task_kind(kind: &TaskKind) -> Option<Self> {
        match kind {
            TaskKind::Validate => Some(Self::Validate),
            TaskKind::GitSync
            | TaskKind::GitFetch
            | TaskKind::GitPull
            | TaskKind::GitCheckout
            | TaskKind::GitClone => Some(Self::GitSync),
            TaskKind::BuildLibrary | TaskKind::LibraryInstall | TaskKind::ArtifactVerification => {
                Some(Self::LibraryPrepare)
            }
            TaskKind::Liquibase
            | TaskKind::MigrationValidate
            | TaskKind::MigrationStatus
            | TaskKind::MigrationPending
            | TaskKind::MigrationPreview
            | TaskKind::MigrationApply => Some(Self::Migration),
            TaskKind::BuildService | TaskKind::MavenBuild | TaskKind::NpmBuild => {
                Some(Self::ServiceBuild)
            }
            TaskKind::StartService | TaskKind::RestartService | TaskKind::StopService => {
                Some(Self::ServiceStart)
            }
            TaskKind::HealthCheck
            | TaskKind::ProcessHealthCheck
            | TaskKind::TcpHealthCheck
            | TaskKind::HttpHealthCheck
            | TaskKind::ReadinessCheck => Some(Self::HealthCheck),
            _ => None,
        }
    }
}

impl std::fmt::Display for EnvironmentPhaseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One planned step of an environment preparation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPlanStep {
    /// The phase the step belongs to.
    pub phase: EnvironmentPhaseKind,
    /// The task the step performs.
    pub task: TaskKind,
    /// The service the step acts on, if any.
    #[serde(default)]
    pub service: Option<ServiceId>,
    /// The library the step acts on, if any.
    #[serde(default)]
    pub library: Option<LibraryId>,
    /// A human-readable description.
    pub description: String,
}

/// A planned phase and the steps it contains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPlanPhase {
    /// The phase.
    pub kind: EnvironmentPhaseKind,
    /// The steps in the phase, in planned order.
    pub steps: Vec<EnvironmentPlanStep>,
}

/// A read-only plan for preparing a development environment.
///
/// Planning never mutates repositories, databases, or processes. It is the
/// authoritative description of what [`Application::prepare_environment`] would
/// schedule, computed from a single request so it cannot drift from execution.
///
/// [`Application::prepare_environment`]: crate::Application::prepare_environment
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPlan {
    /// The workspace the plan applies to.
    pub workspace_id: WorkspaceId,
    /// The branch that would be checked out, if any.
    #[serde(default)]
    pub target_branch: Option<Branch>,
    /// The services the caller explicitly selected.
    pub selected_services: Vec<ServiceId>,
    /// Every service that would participate, including dependencies.
    pub resolved_services: Vec<ServiceId>,
    /// The services added automatically as required dependencies.
    pub included_dependencies: Vec<ServiceId>,
    /// The libraries that would be prepared, in dependency order.
    pub required_libraries: Vec<LibraryId>,
    /// The planned phases, in execution order.
    pub phases: Vec<EnvironmentPlanPhase>,
    /// The total number of steps across every phase.
    pub step_count: u32,
    /// Whether any step requires [`Permission::Destructive`].
    ///
    /// [`Permission::Destructive`]: service_orchestrator_domain::Permission::Destructive
    pub requires_destructive_authorization: bool,
    /// Non-fatal observations, for example explicitly skipped migrations.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Per-phase progress within a workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPhaseProgress {
    /// The phase.
    pub phase: EnvironmentPhaseKind,
    /// The number of tasks in the phase.
    pub total: u32,
    /// Tasks that succeeded.
    pub succeeded: u32,
    /// Tasks that failed.
    pub failed: u32,
    /// Tasks that are still queued, blocked, or running.
    pub pending: u32,
}

/// A structured failure within a workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTaskFailure {
    /// The failed task.
    pub task_id: TaskId,
    /// The task kind.
    pub kind: TaskKind,
    /// The service the task acted on, if any.
    #[serde(default)]
    pub service: Option<ServiceId>,
    /// The library the task acted on, if any.
    #[serde(default)]
    pub library: Option<LibraryId>,
    /// A stable, machine-readable error code.
    pub code: String,
    /// A human-readable summary.
    pub message: String,
}

/// An aggregated, structured status for a workflow.
///
/// It carries counts, phase progress, and failures — never unbounded logs. A
/// shared [`workflow_id`](Self::workflow_id) correlates every underlying task, so
/// the TUI and MCP observe the same authoritative state through the daemon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStatus {
    /// The workflow identifier.
    pub workflow_id: WorkflowId,
    /// The high-level purpose of the workflow.
    pub kind: WorkflowKind,
    /// The aggregate state.
    pub state: WorkflowState,
    /// The total number of tasks.
    pub total: u32,
    /// Tasks that succeeded.
    pub succeeded: u32,
    /// Tasks that failed.
    pub failed: u32,
    /// Tasks that were cancelled.
    pub cancelled: u32,
    /// Tasks that were deliberately skipped.
    pub skipped: u32,
    /// Tasks that are still queued, blocked, or running.
    pub pending: u32,
    /// When the workflow's first task started, if any.
    #[serde(default)]
    pub started_at: Option<Timestamp>,
    /// When the workflow's last task finished, if it has.
    #[serde(default)]
    pub finished_at: Option<Timestamp>,
    /// Progress per phase, in canonical phase order.
    pub phases: Vec<WorkflowPhaseProgress>,
    /// Every failed task, with its structured error.
    #[serde(default)]
    pub failures: Vec<WorkflowTaskFailure>,
}

/// The result of requesting cancellation of a workflow's tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCancellation {
    /// How many still-active tasks were asked to cancel.
    pub cancelled_tasks: u32,
    /// Whether every task was already terminal when cancellation was requested.
    pub already_complete: bool,
}

/// A structured snapshot of a service's configuration and runtime state.
///
/// The fields are raw application values; interfaces format them. For example,
/// the TUI might render `"auth-service — running — :8081"`, but that string is
/// never produced here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceStatus {
    /// The service identifier.
    pub service_id: ServiceId,
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The human-readable service name.
    pub display_name: String,
    /// The service category.
    pub kind: ServiceType,
    /// The last observed process state.
    pub process_state: ProcessState,
    /// The last observed health state.
    pub health_state: HealthState,
    /// The last observed Git state.
    pub git_state: GitState,
    /// The branch currently checked out, if known.
    pub branch: Option<Branch>,
    /// The ports the service declares.
    pub ports: Vec<Port>,
    /// The capabilities the service declares.
    pub capabilities: BTreeSet<ServiceCapability>,
}

// -- build management ---------------------------------------------------------

/// The build-system-neutral operations a build provider supports.
///
/// This is a read-only projection of the build adapter's provider registry. It
/// lets an interface render exactly the operations a configured build system can
/// perform — and disable the rest with an explanation — without hard-coding
/// Maven (or npm) knowledge into the UI (`tasks/020.md` §13). Producing it
/// performs no build and mutates nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildCapabilities {
    /// The build system these capabilities describe.
    pub build_system: BuildSystem,
    /// The operations the provider supports, in a stable order.
    pub operations: Vec<BuildOperation>,
}

impl BuildCapabilities {
    /// Returns `true` when the provider supports `operation`.
    #[must_use]
    pub fn supports(&self, operation: BuildOperation) -> bool {
        self.operations.contains(&operation)
    }

    /// Returns `true` when the provider supports no operation.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

/// A structured snapshot of a service's managed runtime.
///
/// Unlike [`ServiceStatus`], which summarizes broad configuration and observed
/// state, this focuses on the managed process: its identity, the runtime that
/// started it, the (redacted) startup command, and its exit information. It is
/// the machine-readable answer to `get_service_runtime_status` and
/// `list_running_services`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRuntimeStatus {
    /// The service identifier.
    pub service_id: ServiceId,
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The last observed logical process state.
    pub process_state: ProcessState,
    /// Whether the service is currently tracked as a managed process.
    pub managed: bool,
    /// The runtime label that started the process, when known.
    pub runtime: Option<String>,
    /// The managed process identifier, when one exists.
    pub process_id: Option<String>,
    /// The operating-system process id, while the process is alive.
    pub pid: Option<u32>,
    /// A redacted identity of the startup command.
    pub command: Option<String>,
    /// When the current execution started.
    pub started_at: Option<Timestamp>,
    /// The exit code of the last execution, once it has ended.
    pub exit_code: Option<i32>,
    /// How the last execution ended, once it has ended.
    pub exit_reason: Option<String>,
    /// A structured explanation of the last failure, when there was one.
    pub last_error: Option<String>,
}

/// An aggregate of a multi-repository operation's per-repository outcomes.
///
/// A bulk Git command submits one independent step per repository. Each step
/// settles on its own, so the summary is computed from the workflow's tasks
/// rather than from a single all-or-nothing result: one repository's failure
/// never changes another's outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkOperationSummary {
    /// The total number of repositories in the operation.
    pub total: u32,
    /// The number of repositories that completed successfully.
    pub succeeded: u32,
    /// The number of repositories that failed.
    pub failed: u32,
    /// The number of repositories that were cancelled.
    pub cancelled: u32,
    /// The number of repositories that were deliberately skipped.
    pub skipped: u32,
    /// The number of repositories that are still queued, blocked, or running.
    pub pending: u32,
}

impl BulkOperationSummary {
    /// Builds a summary from a workflow's tasks.
    #[must_use]
    pub fn from_tasks(tasks: &[Task]) -> Self {
        let mut summary = Self {
            total: tasks.len() as u32,
            ..Self::default()
        };
        for task in tasks {
            match task.status {
                TaskStatus::Succeeded => summary.succeeded += 1,
                TaskStatus::Failed => summary.failed += 1,
                TaskStatus::Cancelled => summary.cancelled += 1,
                TaskStatus::Skipped => summary.skipped += 1,
                TaskStatus::Queued | TaskStatus::Running | TaskStatus::Blocked => {
                    summary.pending += 1;
                }
            }
        }
        summary
    }

    /// Returns `true` when no repository is still pending.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.pending == 0
    }
}

/// A structured snapshot of a workspace and its services.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStatus {
    /// The workspace identifier.
    pub workspace_id: WorkspaceId,
    /// The human-readable workspace name.
    pub name: String,
    /// The workspace root.
    pub root: AbsolutePath,
    /// The status of each service in the workspace.
    pub services: Vec<ServiceStatus>,
}

// -- library management -------------------------------------------------------

/// A structured snapshot of a library's configuration and repository state.
///
/// The snapshot combines validated configuration with the last observed Git
/// state. It performs no build and no artifact write; local artifact presence is
/// reported separately and definitively by
/// [`Application::verify_library_artifact`](crate::Application::verify_library_artifact).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryStatus {
    /// The library identifier.
    pub library_id: LibraryId,
    /// The workspace that owns the library.
    pub workspace_id: WorkspaceId,
    /// The human-readable library name.
    pub display_name: String,
    /// The build system used to build the library, when known.
    pub build_system: Option<service_orchestrator_domain::BuildSystem>,
    /// The primary artifact coordinate, when configured.
    pub artifact: Option<ArtifactCoordinate>,
    /// Every artifact the library produces (primary first).
    pub artifacts: Vec<ArtifactCoordinate>,
    /// The library's configured repository URL, if it has one.
    pub repository_url: Option<String>,
    /// The branch currently checked out, when observed.
    pub branch: Option<Branch>,
    /// The current commit, when observed.
    pub commit: Option<String>,
    /// The last observed Git state, when a repository is configured and
    /// inspected.
    pub git_state: GitState,
    /// A structured description of why repository state could not be inspected,
    /// when it could not.
    pub repository_error: Option<String>,
    /// Local artifact availability, when known. This is `None` unless a caller
    /// has inspected the repository; the definitive answer is
    /// [`Application::verify_library_artifact`](crate::Application::verify_library_artifact).
    pub artifact_present: Option<bool>,
}

/// A dependent of a library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryDependent {
    /// The dependent service.
    pub service_id: ServiceId,
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// Whether the dependency is mandatory.
    pub required: bool,
    /// Whether the dependency is needed for building or for running.
    pub kind: DependencyKind,
}

/// The result of verifying a library's artifact in the local repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryArtifactVerification {
    /// The library identifier.
    pub library_id: LibraryId,
    /// The workspace that owns the library.
    pub workspace_id: WorkspaceId,
    /// The artifact that was verified.
    pub artifact: ArtifactCoordinate,
    /// Whether the artifact is present in the effective local repository.
    pub present: bool,
    /// The artifact file's absolute path, when it was found.
    pub path: Option<AbsolutePath>,
    /// A human-readable summary.
    pub message: String,
}

/// How a consumer's dependency compares with a library's target version.
///
/// The four outcomes are deliberately distinct: the use case never guesses a
/// version Maven configuration prevents it from resolving (`tasks/009.md` §14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignmentStatus {
    /// The consumer references the target version.
    Aligned,
    /// The consumer references a different, resolved version.
    Mismatch,
    /// A version exists but could not be resolved reliably.
    Unknown,
    /// The consumer does not declare a dependency on the artifact.
    Unresolved,
}

impl AlignmentStatus {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Aligned => "aligned",
            Self::Mismatch => "mismatch",
            Self::Unknown => "unknown",
            Self::Unresolved => "unresolved",
        }
    }
}

/// One consumer's alignment with a library's target version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerAlignment {
    /// The dependent service.
    pub service_id: ServiceId,
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The consumer module's absolute directory, when it could be resolved.
    pub path: Option<AbsolutePath>,
    /// The version the consumer resolves to, when known.
    pub current_version: Option<Version>,
    /// The alignment outcome.
    pub status: AlignmentStatus,
    /// Additional context, for example why a version is unknown.
    pub detail: Option<String>,
}

/// A structured version-alignment plan for a library and its consumers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionAlignmentPlan {
    /// The library identifier.
    pub library_id: LibraryId,
    /// The workspace that owns the library.
    pub workspace_id: WorkspaceId,
    /// The coordinate whose version consumers should align with.
    pub target: ArtifactCoordinate,
    /// One entry per dependent service.
    pub consumers: Vec<ConsumerAlignment>,
}

impl VersionAlignmentPlan {
    /// Returns the number of consumers aligned with the target version.
    #[must_use]
    pub fn aligned_count(&self) -> usize {
        self.consumers
            .iter()
            .filter(|consumer| consumer.status == AlignmentStatus::Aligned)
            .count()
    }

    /// Returns `true` when every consumer is aligned.
    ///
    /// An empty consumer set is trivially aligned.
    #[must_use]
    pub fn is_fully_aligned(&self) -> bool {
        self.consumers
            .iter()
            .all(|consumer| consumer.status == AlignmentStatus::Aligned)
    }
}

/// Options for a prepare-library workflow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryPrepareOptions {
    /// Whether to synchronize the library's repository first.
    ///
    /// Synchronization is optional so a developer can build the current local
    /// branch with uncommitted changes (`tasks/009.md` §20).
    pub sync: bool,
    /// An optional branch to check out after synchronizing. When absent, the
    /// configured branch is left untouched.
    pub branch: Option<Branch>,
    /// Whether to build the library.
    pub build: bool,
    /// Whether to install the artifact into the local repository.
    pub install: bool,
    /// Whether to verify the artifact after installation.
    pub verify: bool,
}

impl Default for LibraryPrepareOptions {
    fn default() -> Self {
        Self {
            sync: true,
            branch: None,
            build: true,
            install: true,
            verify: true,
        }
    }
}

impl LibraryPrepareOptions {
    /// Creates the default options: synchronize, build, install, and verify.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or disables repository synchronization.
    #[must_use]
    pub const fn with_sync(mut self, sync: bool) -> Self {
        self.sync = sync;
        self
    }

    /// Sets the branch to check out after synchronizing.
    #[must_use]
    pub fn with_branch(mut self, branch: Branch) -> Self {
        self.branch = Some(branch);
        self
    }

    /// Enables or disables building the library.
    #[must_use]
    pub const fn with_build(mut self, build: bool) -> Self {
        self.build = build;
        self
    }

    /// Enables or disables installing the artifact locally.
    #[must_use]
    pub const fn with_install(mut self, install: bool) -> Self {
        self.install = install;
        self
    }

    /// Enables or disables artifact verification.
    #[must_use]
    pub const fn with_verify(mut self, verify: bool) -> Self {
        self.verify = verify;
        self
    }
}

/// A request to prepare several libraries in dependency order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareSelectedLibrariesRequest {
    /// The workspace the libraries belong to.
    pub workspace_id: WorkspaceId,
    /// The libraries to prepare. When empty, every library in the workspace is
    /// selected.
    pub libraries: Vec<LibraryId>,
    /// The phases to run.
    pub options: LibraryPrepareOptions,
}

impl PrepareSelectedLibrariesRequest {
    /// Creates a request with default options.
    #[must_use]
    pub fn new(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            libraries: Vec::new(),
            options: LibraryPrepareOptions::new(),
        }
    }

    /// Selects the libraries to prepare.
    #[must_use]
    pub fn with_libraries<I>(mut self, libraries: I) -> Self
    where
        I: IntoIterator<Item = LibraryId>,
    {
        self.libraries = libraries.into_iter().collect();
        self
    }

    /// Replaces the phase options.
    #[must_use]
    pub fn with_options(mut self, options: LibraryPrepareOptions) -> Self {
        self.options = options;
        self
    }
}

// -- migrations ---------------------------------------------------------------

/// A configuration-derived snapshot of a service's migration setup.
///
/// This is deliberately *configuration*, not live database state. Inspecting
/// migration status or pending changesets requires running the provider, which
/// is a task (`migration_status` / `migration_pending`); a query that returned a
/// live answer would have to execute Liquibase, which read-only queries must not
/// do. The [`pending`](Self::pending) and [`locked`](Self::locked) fields are
/// therefore explicit unknowns (`None`) here, never a guess
/// (`tasks/011.md` §7, §14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationStatus {
    /// The service whose migrations are described.
    pub service_id: ServiceId,
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// Whether migrations are enabled for the service.
    pub enabled: bool,
    /// The configured migration provider.
    pub provider: MigrationProviderType,
    /// How the provider is invoked.
    pub execution_mode: LiquibaseExecutionMode,
    /// The declared environment of the target database.
    pub environment: MigrationTargetEnvironment,
    /// A display-safe identity of the target database (never credentials).
    pub database_identity: String,
    /// Whether applying to the target requires explicit authorization.
    pub requires_authorization: bool,
    /// The configured migration timeout, in seconds.
    pub timeout_seconds: Option<u64>,
    /// The number of pending changesets, when known.
    ///
    /// Always `None` from this query: determining it runs the provider as a task.
    pub pending: Option<u32>,
    /// Whether the target database is currently locked by another migration,
    /// when known.
    ///
    /// Always `None` from this query, for the same reason as
    /// [`pending`](Self::pending).
    pub locked: Option<bool>,
}

// -- health and diagnostics ---------------------------------------------------

/// Which configured health checks a check operation should run.
///
/// The scope lets a caller probe only the level it cares about — for example a
/// process liveness check while debugging, or a readiness HTTP check during
/// startup — without running every configured probe (`tasks/012.md` §12, §33).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckScope {
    /// Run every configured health check.
    #[default]
    All,
    /// Run only process checks (is the managed process alive?).
    Process,
    /// Run only TCP/port checks (is the port reachable?).
    Tcp,
    /// Run only HTTP/actuator checks (is the application ready?).
    Http,
}

impl HealthCheckScope {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Process => "process",
            Self::Tcp => "tcp",
            Self::Http => "http",
        }
    }
}

/// A structured diagnostic of a service's observed state.
///
/// The diagnostic combines configuration with *observed facts* only: the last
/// runtime state, the managed process, the latest health result, its checks,
/// declared ports, and recent error logs. It never mutates anything and never
/// fabricates a root cause; a failure is reported as the structured code and
/// message the observing subsystem produced (`tasks/012.md` §30, §31).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDiagnostics {
    /// The service identifier.
    pub service_id: ServiceId,
    /// The workspace that owns the service.
    pub workspace_id: WorkspaceId,
    /// The last observed process state.
    pub process_state: ProcessState,
    /// The last observed aggregated health state.
    pub health_state: HealthState,
    /// Whether the service is currently tracked as a managed process.
    pub managed: bool,
    /// The managed process identifier, when one exists.
    #[serde(default)]
    pub process_id: Option<String>,
    /// The operating-system process id, while the process is alive.
    #[serde(default)]
    pub pid: Option<u32>,
    /// The ports the service declares.
    #[serde(default)]
    pub ports: Vec<Port>,
    /// When the service was last checked, if it ever has been.
    #[serde(default)]
    pub last_checked_at: Option<Timestamp>,
    /// The latest outcome of each health check.
    #[serde(default)]
    pub health_checks: Vec<HealthCheckOutcome>,
    /// The most recent structured failure, when one was observed.
    #[serde(default)]
    pub last_failure: Option<DiagnosticFailure>,
    /// The most recent error-level log lines for the service, oldest first.
    #[serde(default)]
    pub recent_errors: Vec<String>,
    /// Additional observed facts that do not fit the fields above.
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A structured, observed failure recorded in a diagnostic.
///
/// A failure is never invented: it is derived from a health check outcome or an
/// error-level log line, and it carries the source that produced it
/// (`tasks/012.md` §30, §35).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticFailure {
    /// Where the failure was observed (`health`, `log`, or `runtime`).
    pub source: String,
    /// A stable, machine-readable code, for example `HTTP_CONNECTION_REFUSED`.
    pub code: String,
    /// A human-readable summary.
    pub message: String,
    /// When the failure was observed, if known.
    #[serde(default)]
    pub at: Option<Timestamp>,
}
