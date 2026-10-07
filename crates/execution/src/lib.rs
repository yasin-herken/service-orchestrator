//! # Execution
//!
//! Task and workflow engine.
//!
//! Owns task state, dependency-aware scheduling, bounded concurrency,
//! cancellation, retry, timeout, and progress reporting. It depends only on
//! `service-orchestrator-domain` and never on interfaces or concrete
//! infrastructure. Concrete task handlers are supplied by outer layers.
//!
//! This crate currently defines the engine's **contract** — the
//! [`TaskManager`] trait, the [`TaskDefinition`] and [`WorkflowDefinition`]
//! requests, and the [`TaskManagerError`] type — but not the scheduler. The
//! application layer submits work through this contract; the scheduler that
//! implements it arrives in a later task.
//!
//! See `docs/application-core.md` and `docs/adr/004-application-core.md`.

mod task_manager;

pub use task_manager::{
    TaskDefinition, TaskManager, TaskManagerError, TaskTarget, WorkflowDefinition,
};
