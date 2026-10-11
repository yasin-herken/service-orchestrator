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
    AbsolutePath, ArtifactCoordinate, Branch, DependencyKind, GitState, LibraryId,
    LiquibaseExecutionMode, MigrationProviderType, MigrationTargetEnvironment, Port, ProcessState,
    ServiceCapability, ServiceId, ServiceType, Task, TaskId, TaskKind, TaskStatus, Timestamp,
    Version, WorkflowId, WorkflowKind, WorkspaceId,
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
/// schema later, and so a caller avoids a long positional argument list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareEnvironmentRequest {
    /// The workspace to prepare.
    pub workspace_id: WorkspaceId,
    /// The services to include. When empty, every service in the workspace is
    /// included.
    pub services: Vec<ServiceId>,
    /// An optional branch to check out before building.
    pub branch: Option<Branch>,
    /// Whether to run Liquibase migrations.
    pub run_liquibase: bool,
    /// Whether to build libraries and services.
    pub build: bool,
    /// Whether to start the selected services.
    pub start: bool,
}

impl PrepareEnvironmentRequest {
    /// Creates a request that selects the given services and does nothing else.
    #[must_use]
    pub fn new(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            services: Vec::new(),
            branch: None,
            run_liquibase: false,
            build: false,
            start: false,
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

    /// Sets the branch to check out.
    #[must_use]
    pub fn with_branch(mut self, branch: Branch) -> Self {
        self.branch = Some(branch);
        self
    }

    /// Enables Liquibase migrations.
    #[must_use]
    pub fn run_liquibase(mut self) -> Self {
        self.run_liquibase = true;
        self
    }

    /// Enables building libraries and services.
    #[must_use]
    pub fn build(mut self) -> Self {
        self.build = true;
        self
    }

    /// Enables starting the selected services.
    #[must_use]
    pub fn start(mut self) -> Self {
        self.start = true;
        self
    }
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
