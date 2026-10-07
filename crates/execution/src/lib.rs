//! # Execution
//!
//! The task and workflow engine.
//!
//! The engine owns task state, dependency-aware scheduling, bounded
//! concurrency, cancellation, retry, timeout, progress reporting, and an
//! in-memory task registry. It depends only on `service-orchestrator-domain`
//! (plus `tokio` for asynchronous execution) and never on interfaces or
//! concrete infrastructure. Concrete task executors are supplied by outer
//! layers through the [`ExecutorRegistry`].
//!
//! # Architecture
//!
//! ```text
//! TUI ─────┐
//!          │
//! MCP ─────┼──> Application use case
//!          │
//! CLI ─────┘
//!               ↓
//!         TaskDefinition / WorkflowDefinition
//!               ↓
//!           TaskEngine  ──> TaskStore (in-memory registry)
//!               ↓              ↑
//!           Scheduler ─────────┘
//!               ↓
//!        TaskExecutor (registry lookup)
//!               ↓
//!         Infrastructure
//! ```
//!
//! The [`TaskManager`] trait is the small contract the application depends on.
//! [`TaskEngine`] implements it and adds the richer query and event API
//! (`list_tasks`, `list_running_tasks`, `list_recent_tasks`, `subscribe`,
//! `wait_for`) that the TUI and MCP will use.
//!
//! # Boundaries
//!
//! - The scheduler never knows how Git, Maven, npm, or a process works; it only
//!   knows how to schedule generic tasks.
//! - An executor never knows about queues, dependencies, or concurrency.
//! - The engine never spawns a process, runs a build, or opens a socket.
//!
//! See `docs/task-engine.md` and `docs/adr/005-task-engine.md`.

#![deny(missing_docs)]

mod cancellation;
mod context;
mod engine;
mod error;
mod events;
mod executor;
mod graph;
mod registry;
mod retry;
mod scheduler;
mod store;
mod task_manager;

pub use cancellation::Cancellation;
pub use context::{ProgressReporter, TaskContext};
pub use engine::{EngineConfig, TaskEngine, TaskEngineBuilder};
pub use error::TaskEngineError;
pub use events::TaskEvent;
pub use executor::{TaskExecutor, TaskExecutorError, TaskOutcome};
pub use registry::ExecutorRegistry;
pub use retry::RetryPolicy;
pub use store::TaskStore;
pub use task_manager::{
    DependencyPolicy, TaskDefinition, TaskManager, TaskManagerError, TaskTarget, WorkflowDefinition,
};
