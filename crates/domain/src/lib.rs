//! # Domain
//!
//! Pure business concepts and rules for Service Orchestrator.
//!
//! The domain is the innermost layer and depends on no other workspace crate.
//! It must never depend on:
//!
//! - interface adapters (`tui`, `mcp`)
//! - infrastructure (`git`, `process`, `build`, `liquibase`, `health`, `logging`)
//! - concrete runtime, filesystem, process, or terminal libraries
//!
//! The crate depends only on `serde` (for crossing the configuration and
//! interface boundaries), `thiserror` (for structured errors), and `time` (for
//! a consistent timestamp type). None of these pull in an async runtime,
//! network stack, or OS process API.
//!
//! # What lives here
//!
//! - **Identity**: strongly typed [`ServiceId`], [`TaskId`], and friends, plus
//!   validated value objects such as [`Port`], [`Version`], [`Branch`],
//!   [`RelativePath`], and [`AbsolutePath`].
//! - **Configuration vocabulary**: [`Workspace`], [`Service`], [`Library`],
//!   [`Repository`], [`Runtime`], [`CommandSet`], [`Profile`], [`ServiceGroup`],
//!   [`Dependency`], and [`HealthCheck`].
//! - **Runtime vocabulary**: [`Task`], [`TaskStatus`], [`Workflow`],
//!   [`ProcessState`], [`GitState`], and [`Permission`].
//! - **Errors**: the small, typed [`DomainError`].
//!
//! See `docs/domain-model.md` for the full model and its invariants, and
//! `docs/adr/002-domain-model.md` for the reasoning behind it.
//!
//! # What does not live here
//!
//! The domain defines *what* things are, never *how* they are done. There is no
//! Git access, no process spawning, no Maven or npm invocation, no Liquibase,
//! no scheduling, no TUI, and no MCP. Those belong to infrastructure, the
//! execution engine, policy, and the interface adapters respectively.

mod graph;

pub mod command;
pub mod dependency;
pub mod environment;
pub mod error;
pub mod git;
pub mod group;
pub mod health;
pub mod ids;
pub mod library;
pub mod paths;
pub mod permission;
pub mod port;
pub mod process;
pub mod profile;
pub mod repository;
pub mod runtime;
pub mod service;
pub mod task;
pub mod timestamp;
pub mod workflow;
pub mod workspace;

pub use command::{CommandEntry, CommandKind, CommandSet, CommandSpec};
pub use dependency::{Dependency, DependencyKind, DependencyTarget};
pub use environment::{Environment, EnvironmentVariable, REDACTED};
pub use error::DomainError;
pub use git::GitState;
pub use group::ServiceGroup;
pub use health::{HealthCheck, HealthCheckKind, HttpHealthCheck};
pub use ids::{GroupId, LibraryId, ProfileId, ServiceId, StepId, TaskId, WorkflowId, WorkspaceId};
pub use library::{ArtifactCoordinate, Library};
pub use paths::{AbsolutePath, RelativePath};
pub use permission::Permission;
pub use port::Port;
pub use process::ProcessState;
pub use profile::Profile;
pub use repository::{Branch, Repository, RepositoryProvider};
pub use runtime::{Runtime, RuntimeKind, Version};
pub use service::{Service, ServiceCapability, ServiceType};
pub use task::{Task, TaskFailure, TaskKind, TaskProgress, TaskStatus};
pub use timestamp::Timestamp;
pub use workflow::{Workflow, WorkflowKind, WorkflowStep};
pub use workspace::Workspace;
