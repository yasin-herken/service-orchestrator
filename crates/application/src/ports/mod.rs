//! Ports the application layer depends on.
//!
//! A port is a boundary the application needs in order to reach something it
//! does not own. Only boundaries that exist today are declared here; there is
//! deliberately no `GitService`, `BuildService`, `ProcessService`,
//! `LiquibaseService`, or `HealthService` trait yet, because the application
//! does not call those subsystems directly — it requests tasks and the
//! execution engine reaches the adapters.
//!
//! The task-engine contract itself lives in the `execution` core service (the
//! application depends *inward* on the engine); see [`execution::TaskManager`].
//!
//! [`execution::TaskManager`]: service_orchestrator_execution::TaskManager

pub(crate) mod logs;
pub(crate) mod runtime_state;

pub use logs::{LogEntry, LogLevel, LogRequest, LogService, LogServiceError, NullLogService};
pub use runtime_state::{
    HealthState, InMemoryRuntimeStateStore, RuntimeStateStore, ServiceRuntimeState,
};
