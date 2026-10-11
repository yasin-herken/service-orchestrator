//! # Git
//!
//! The Git infrastructure adapter: a production-quality Git manager that drives
//! the installed Git CLI through the process manager and implements the
//! application's [`GitManager`] port.
//!
//! It contains no business rules. It depends on the domain (for the values it
//! produces), the application core (for the port it implements), the execution
//! service (for the task-engine boundary), and the process manager (for running
//! Git). It never spawns an operating-system process itself and never depends on
//! Ratatui or MCP.
//!
//! # Architecture
//!
//! ```text
//! Application use case
//!        │  (mutating op as a TaskDefinition with repository parameters)
//!        ▼
//! Task Engine ──> GitTaskExecutor ──┐
//!        │                          │
//!        │  (read-only op, direct)  │
//!        ▼                          ▼
//!                CliGitManager (GitManager port)
//!                        │
//!            ┌───────────┼──────────────┬───────────────┐
//!            ▼           ▼              ▼               ▼
//!      command.rs   coordinator.rs   runner.rs      status.rs
//!    (pure argv)   (per-repo locks)  (process mgr)  (porcelain v2)
//!                                        │
//!                                        ▼
//!                                  Process Manager
//!                                        │
//!                                        ▼
//!                                      git CLI
//! ```
//!
//! Read-only operations (`status`, `branches`, `diff`) are called directly on
//! [`CliGitManager`] by the application; mutating operations are submitted to
//! the task engine, which invokes [`GitTaskExecutor`]. Both paths reach the same
//! manager, so validation, safety checks, and repository-scoped coordination are
//! never duplicated.
//!
//! # Reuse
//!
//! The composition root wires the adapter with
//! [`CliGitManager::new`] and [`register_git_executors`]. The manager is cheaply
//! cloneable, and the application holds it behind `Arc<dyn GitManager>`.
//!
//! See `docs/git-manager.md` and `docs/adr/007-git-manager.md`.

#![deny(missing_docs)]

mod command;
mod coordinator;
mod executor;
mod manager;
mod runner;
mod status;

pub use coordinator::RepositoryCoordinator;
pub use executor::{register_git_executors, GitTaskExecutor};
pub use manager::CliGitManager;
