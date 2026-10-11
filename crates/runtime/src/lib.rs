//! # Runtime
//!
//! The service runtime adapter: a **runtime-agnostic** Service Manager that
//! turns a validated service into a structured start plan and drives the process
//! manager to run, stop, and restart it.
//!
//! The adapter is organised around a small [`ServiceRuntime`] abstraction. A
//! runtime knows *how* one kind of service starts — which executable it needs,
//! which working directory and environment apply, and how it should be shut down
//! — and nothing else. The [`ServiceManager`] knows *which* runtime to use and
//! applies the common rules (idempotency, validation, version compatibility, port
//! conflicts, ownership, lifecycle tracking, event publication) uniformly.
//! Neither spawns a process itself: the process manager owns execution, and the
//! task engine owns scheduling (`AGENTS.md` §11, §15, §21).
//!
//! # Architecture
//!
//! ```text
//! Application use case
//!        │  (lifecycle op as a TaskDefinition with target parameters)
//!        ▼
//! Task Engine ──> ServiceRuntimeTaskExecutor ──┐
//!                                             ▼
//!                                       Service Manager ──> RuntimeRegistry
//!                                             │                 ├── JavaRuntime
//!                                             │                 └── NodeRuntime
//!                                             ▼
//!                                        Process Manager
//! ```
//!
//! # Startup task versus managed process
//!
//! A start operation spawns the process, confirms it is running, records it in
//! the managed-process registry, and returns. The startup task therefore
//! *completes* immediately while the managed process keeps running under the
//! process manager's supervision. Cancelling a completed startup task never
//! stops the service; stopping requires an explicit `StopService` operation
//! (`tasks/010.md` §19, §26).
//!
//! # Secrets
//!
//! A service's environment may carry secret values. Lifecycle tasks never carry
//! environment values — only identity and working directory — so nothing secret
//! reaches a task result or an interface. The manager reads the full environment
//! from validated configuration at execution time and hands it straight to the
//! process manager, which redacts secret values whenever a specification is
//! displayed or serialized (`AGENTS.md` §31).
//!
//! See `docs/service-runtime.md` and `docs/adr/010-service-runtime.md`.

mod coordinator;
mod events;
mod executor;
mod java;
mod managed;
mod manager;
mod node;
mod port_check;
mod registry;
mod runtime;

pub use events::{RuntimeEvent, RuntimeEventStream};
pub use executor::{
    register_service_executors, RestartServiceExecutor, StartServiceExecutor, StopServiceExecutor,
};
pub use java::JavaRuntime;
pub use manager::{RestartOutcome, ServiceManager, StartOutcome, StopOutcome};
pub use node::NodeRuntime;
pub use port_check::{FreePortChecker, PortChecker, SystemPortChecker};
pub use registry::RuntimeRegistry;
pub use runtime::{
    basename, extract_version, major_version, plan_from_service, validate_service,
    NullRuntimeInspector, ProcessRuntimeInspector, RuntimeHost, RuntimeInfo, RuntimeInspector,
    RuntimeType, ServiceRuntime, StartPlan, SystemRuntimeHost,
};

// Re-export the structured error and index the application defines, so a caller
// of this adapter has a single import.
pub use service_orchestrator_application::{ServiceRuntimeError, ServiceRuntimeIndex};
