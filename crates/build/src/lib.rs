//! # Build
//!
//! The build infrastructure adapter: a **build-system-agnostic** Build Manager
//! that turns a validated build target into a structured build plan and executes
//! it through the process manager.
//!
//! The adapter is organised around a small [`BuildProvider`] abstraction. A
//! provider knows *how* one build system works — its capabilities, how to
//! resolve its executable or wrapper, and how to construct its plans — and
//! nothing else. The [`BuildManager`] knows *which* provider to use and applies
//! the common rules (capability checks, validation, environment enforcement,
//! execution, failure classification) uniformly. Neither spawns a process: the
//! process manager owns execution, and the task engine owns scheduling.
//!
//! # Architecture
//!
//! ```text
//! Application use case
//!        │  (build op as a TaskDefinition with target parameters)
//!        ▼
//! Task Engine ──> BuildTaskExecutor ──┐
//!                                    ▼
//!                              BuildManager
//!                             ┌──────┴───────┐
//!                             ▼              ▼
//!                   BuildProviderRegistry  BuildRunner
//!                             │              │
//!                 ┌───────────┴───────┐      ▼
//!                 ▼                   ▼   Process Manager
//!         MavenBuildProvider   NpmBuildProvider     │
//!                 │                   │             ▼
//!                 └──────── BuildPlan ─┘        build tool
//! ```
//!
//! # Boundaries
//!
//! - The domain names the build *system* and the *operation*
//!   ([`BuildSystem`](service_orchestrator_domain::BuildSystem),
//!   [`BuildOperation`](service_orchestrator_domain::BuildOperation)); the
//!   adapter never teaches the domain about executables or arguments.
//! - The application resolves a workspace-aware
//!   [`ResolvedBuildTarget`](service_orchestrator_application::ResolvedBuildTarget)
//!   and schedules a task; it never runs a build tool.
//! - Providers are pure: they generate plans through a [`BuildHost`] seam and
//!   never spawn processes or schedule tasks (`tasks/008.md`).
//! - The manager adds no concurrency of its own; the engine's
//!   `max_parallel_tasks` remains the only global bound (`AGENTS.md` §41).
//!
//! # Extending
//!
//! Adding a build system (Gradle, for example) is: implement [`BuildProvider`],
//! register it in [`BuildProviderRegistry::standard`], extend configuration
//! validation, and add provider-specific tests. Nothing in the manager, the task
//! engine, the application core, or the interfaces changes.
//!
//! See `docs/build-manager.md` and `docs/adr/008-build-manager.md`.

#![deny(missing_docs)]

mod artifact;
mod coordinator;
mod executor;
mod library_executor;
mod manager;
mod maven;
mod npm;
mod plan;
mod provider;
mod registry;
mod runner;

pub use artifact::{
    FixedLocalRepository, LocalRepositoryProvider, MavenArtifactInspector,
    MavenLocalRepositoryProvider,
};
pub use coordinator::ArtifactCoordinator;
pub use executor::{register_build_executors, BuildTaskExecutor};
pub use library_executor::{register_library_executors, ArtifactVerificationExecutor};
pub use manager::{BuildManager, BuildOutcome};
pub use maven::MavenBuildProvider;
pub use npm::NpmBuildProvider;
pub use plan::{BuildPlan, EnvironmentRequirement, ExpectedOutput, ExpectedOutputKind};
pub use provider::{BuildHost, BuildProvider, SystemBuildHost};
pub use registry::BuildProviderRegistry;

// The structured build error vocabulary is defined by the application (the
// target and its error are one boundary); re-export it so callers of the build
// adapter can name the errors it returns without a second import.
pub use service_orchestrator_application::BuildError;
