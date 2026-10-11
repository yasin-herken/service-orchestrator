//! Ports the application layer depends on.
//!
//! A port is a boundary the application needs in order to reach something it
//! does not own. Only boundaries that exist today are declared here; there is
//! deliberately no `BuildService`, `ProcessService`, `LiquibaseService`, or
//! `HealthService` trait yet, because the application does not call those
//! subsystems directly — it requests tasks and the execution engine reaches the
//! adapters. Git is the exception: read-only inspection and authorized
//! destructive operations are exposed directly through `GitManager`, while
//! mutations are scheduled as tasks. Builds are entirely task-driven, so the
//! build boundary declares only the workspace-aware [`ResolvedBuildTarget`]
//! value that a task executor decodes.
//!
//! The task-engine contract itself lives in the `execution` core service (the
//! application depends *inward* on the engine); see [`execution::TaskManager`].
//!
//! Service lifecycle is the same shape as builds: mutations are task-driven, so
//! the boundary declares the workspace-aware [`ResolvedServiceTarget`] a task
//! executor decodes, plus a small read-only [`ServiceRuntimeIndex`] the adapter
//! implements so runtime-status queries can read live managed-process state.
//!
//! [`execution::TaskManager`]: service_orchestrator_execution::TaskManager

pub(crate) mod artifact;
pub(crate) mod build;
pub(crate) mod git;
pub(crate) mod logs;
pub(crate) mod runtime_state;
pub(crate) mod service_runtime;

pub use artifact::{
    parameter as artifact_parameter, ArtifactInspectionError, ArtifactInspector, ArtifactPresence,
    DependencyVersionInspection, NullArtifactInspector, ResolvedArtifact, ResolvedDependency,
};
pub use build::{
    parameter as build_parameter, BuildError, ResolvedBuildCommand, ResolvedBuildTarget,
};
pub use git::{
    parameter, DiffOptions, GitError, GitManager, GitOptions, NullGitManager, ResolvedRepository,
};
pub use logs::{LogEntry, LogLevel, LogRequest, LogService, LogServiceError, NullLogService};
pub use runtime_state::{
    HealthState, InMemoryRuntimeStateStore, RuntimeStateStore, ServiceRuntimeState,
};
pub use service_runtime::{
    parameter as service_runtime_parameter, NullServiceRuntimeIndex, ResolvedServiceTarget,
    ServiceProcessInfo, ServiceRuntimeError, ServiceRuntimeIndex,
};
