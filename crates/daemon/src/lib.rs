//! # Service Orchestrator daemon
//!
//! The local shared runtime. It owns the single authoritative application core,
//! task engine, process registry, and log store, and serves them to interface
//! clients over the versioned Unix domain socket protocol defined in
//! `service-orchestrator-client`.
//!
//! The daemon is deliberately thin: it validates protocol versions, routes each
//! request to the shared [`Application`](service_orchestrator_application::Application),
//! and fans events out to clients. It contains no business logic, so the TUI and
//! MCP interfaces observe exactly the same behavior whether they run in-process
//! or over IPC (`tasks/015.md` §4, §9, §10).
//!
//! See `docs/local-daemon.md` and `docs/adr/015-shared-local-runtime.md`.

#![deny(missing_docs)]

mod dispatch;
mod error;
mod lock;
mod server;

pub use error::DaemonError;
pub use server::{Daemon, DaemonOptions};
