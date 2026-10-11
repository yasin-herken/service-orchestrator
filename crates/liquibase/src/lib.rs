//! # Liquibase
//!
//! The database-migration infrastructure adapter: a **provider-based** Migration
//! Manager that turns a validated migration target into a structured migration
//! plan and executes it through the process manager.
//!
//! The adapter is organised around a small [`MigrationProvider`] abstraction. A
//! provider knows *how* one migration tool works — its capabilities, how to
//! validate a request, how to resolve its executable, and how to construct its
//! plans — and nothing else. The [`MigrationManager`] knows *which* provider to
//! use and applies the common rules (configuration lookup, environment safety,
//! capability checks, planning, database-target coordination, execution, failure
//! classification) uniformly. Neither spawns a process: the process manager owns
//! execution, and the task engine owns scheduling (`AGENTS.md` §9, §15, §21).
//!
//! The initial concrete provider is Liquibase. Flyway and custom tools are
//! planned; adding one is a new provider plus a registry entry, not a change to
//! orchestration (`tasks/011.md` §2, §5).
//!
//! # Architecture
//!
//! ```text
//! Application use case
//!        │  (migration op as a TaskDefinition with target parameters)
//!        ▼
//! Task Engine ──> MigrationTaskExecutor ──┐
//!                                        ▼
//!                                  MigrationManager
//!                                 ┌──────┴───────┐
//!                                 ▼              ▼
//!                 MigrationProviderRegistry  MigrationRunner
//!                                 │              │
//!                                 ▼              ▼
//!                        LiquibaseProvider   Process Manager
//!                                 │              │
//!                                 └─ MigrationPlan ─┘
//!                                                 ▼
//!                                          Liquibase / Maven
//! ```
//!
//! # Boundaries
//!
//! - The domain names the migration *provider*, *operation*, and *target
//!   environment* ([`MigrationProviderType`](service_orchestrator_domain::MigrationProviderType),
//!   [`MigrationOperation`](service_orchestrator_domain::MigrationOperation),
//!   [`MigrationTargetEnvironment`](service_orchestrator_domain::MigrationTargetEnvironment));
//!   the adapter never teaches the domain about executables or arguments.
//! - The application resolves a workspace-aware
//!   [`ResolvedMigrationTarget`](service_orchestrator_application::ResolvedMigrationTarget)
//!   and schedules a task; it never runs a migration tool.
//! - Providers are pure: they generate plans through a [`BuildHost`](crate::BuildHost)
//!   seam and never spawn processes or schedule tasks (`tasks/011.md` §4).
//! - The manager adds no concurrency of its own beyond database-target
//!   coordination; the engine's `max_parallel_tasks` remains the only global
//!   bound (`AGENTS.md` §41).
//! - No credential is ever stored on a task, logged, or returned: task
//!   parameters hold identity only, and captured output is redacted
//!   (`AGENTS.md` §31, `tasks/011.md` §11).
//!
//! # Extending
//!
//! Adding a provider (Flyway, for example) is: implement [`MigrationProvider`],
//! register it in [`MigrationProviderRegistry::standard`], extend configuration
//! validation, and add provider-specific tests. Nothing in the manager, the task
//! engine, the application core, or the interfaces changes.
//!
//! See `docs/migration-manager.md` and `docs/adr/011-migration-manager.md`.

#![deny(missing_docs)]

mod context;
mod coordinator;
mod executor;
mod liquibase;
mod manager;
mod plan;
mod provider;
mod registry;
mod runner;

pub use context::MigrationContext;
pub use coordinator::DatabaseCoordinator;
pub use executor::{register_migration_executors, MigrationTaskExecutor};
pub use liquibase::LiquibaseProvider;
pub use manager::{MigrationManager, MigrationOutcome};
pub use plan::{MigrationCapabilities, MigrationPlan, MigrationSafety};
pub use provider::{BuildHost, MigrationProvider, SystemBuildHost};
pub use registry::MigrationProviderRegistry;

// The structured migration error vocabulary is defined by the application (the
// target and its error are one boundary); re-export it so callers of the
// migration adapter can name the errors it returns without a second import.
pub use service_orchestrator_application::MigrationError;
