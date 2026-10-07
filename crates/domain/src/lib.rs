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
//! Domain concepts that will live here include services, workspaces,
//! repositories, libraries, profiles, service groups, tasks, workflows,
//! dependencies, runtimes, and state and status values.
