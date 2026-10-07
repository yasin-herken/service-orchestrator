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
    AbsolutePath, Branch, GitState, Port, ProcessState, ServiceCapability, ServiceId, ServiceType,
    TaskId, TaskKind, WorkflowId, WorkflowKind, WorkspaceId,
};

use crate::ports::HealthState;

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
