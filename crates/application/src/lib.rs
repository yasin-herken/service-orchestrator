//! # Application
//!
//! The shared application core: use cases, orchestration, and the ports that
//! infrastructure implements.
//!
//! Every interface — the Ratatui TUI, the MCP server, and any future CLI or HTTP
//! API — calls the same use cases here. Neither interface contains business
//! orchestration logic, and this crate knows nothing about either:
//!
//! - it never depends on `ratatui`, `rmcp`, or any interface type;
//! - it never returns pre-formatted strings; every result is structured.
//!
//! # Shape
//!
//! [`Application`] is the entry point. It holds validated
//! [`Config`](service_orchestrator_config::Config) plus the boundaries the use
//! cases need:
//!
//! - the task engine ([`TaskManager`](service_orchestrator_execution::TaskManager),
//!   owned by the `execution` core service);
//! - the runtime-state store ([`RuntimeStateStore`]);
//! - the log service ([`LogService`]);
//! - the [policy engine](service_orchestrator_policy::PolicyEngine), applied to
//!   every mutating command.
//!
//! Use cases are split into [`Application`] **queries** (side-effect free) and
//! **commands** (which validate preconditions, apply policy, and submit tasks).
//! Long-running commands return a [`TaskId`](service_orchestrator_domain::TaskId)
//! or workflow id through an [`OperationResult`] rather than blocking.
//!
//! # Example
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use service_orchestrator_application::{Application, InMemoryRuntimeStateStore, NullLogService};
//! # use service_orchestrator_config::Config;
//! # use service_orchestrator_execution::TaskManager;
//! # fn build(config: Config, tasks: Arc<dyn TaskManager>) -> Application {
//! Application::new(
//!     config,
//!     tasks,
//!     Arc::new(InMemoryRuntimeStateStore::new()),
//!     Arc::new(NullLogService),
//! )
//! # }
//! ```
//!
//! See `docs/application-core.md` and `docs/adr/004-application-core.md`.

mod app;
mod commands;
mod error;
mod models;
mod ports;
mod queries;

pub use app::Application;
pub use error::ApplicationError;
pub use models::{OperationResult, PrepareEnvironmentRequest, ServiceStatus, WorkspaceStatus};
pub use ports::{
    HealthState, InMemoryRuntimeStateStore, LogEntry, LogLevel, LogRequest, LogService,
    LogServiceError, NullLogService, RuntimeStateStore, ServiceRuntimeState,
};
