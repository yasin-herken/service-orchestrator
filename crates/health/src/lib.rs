//! # Health
//!
//! Infrastructure adapter for health checks.
//!
//! Supports configurable checks such as process, port, and HTTP/actuator
//! probes. A live process does not imply a healthy service. It depends on the
//! domain and the application core.
//!
//! # Architecture
//!
//! ```text
//! Application use case
//!        │  (health op as a TaskDefinition with target parameters)
//!        ▼
//! Task Engine ──> HealthTaskExecutor ──┐
//!                                     ▼
//!                               HealthManager ──> HealthCheckerRegistry
//!                                     │                 ├── ProcessHealthChecker
//!                                     │                 ├── TcpHealthChecker
//!                                     │                 └── HttpHealthChecker
//!                                     ▼
//!                       HealthStore (HealthIndex) + HealthEventStream
//! ```
//!
//! The [`HealthManager`] owns interpretation and bookkeeping; the three
//! providers own the probe mechanics; the [`HealthMonitor`] optionally polls
//! watched services; and the application reads the latest observation through
//! the [`HealthStore`], which implements the read-only
//! [`HealthIndex`](service_orchestrator_application::HealthIndex) port.
//!
//! See `docs/health-manager.md` and `docs/adr/012-health-observability.md`.

#![deny(missing_docs)]

pub mod checker;
pub mod error;
pub mod events;
pub mod executor;
pub mod history;
pub mod http;
pub mod manager;
pub mod monitor;
pub mod process;
pub mod registry;
pub mod tcp;

pub use checker::{check_target, CheckResult, CheckType, HealthCheckContext, HealthChecker};
pub use error::HealthError;
pub use events::{HealthEvent, HealthEventStream};
pub use executor::{
    register_health_executors, HealthTaskExecutor, DEFAULT_READINESS_TIMEOUT_SECONDS,
};
pub use history::{HealthSample, HealthStore, DEFAULT_HISTORY_CAPACITY};
pub use http::{actuator_status, HttpHealthChecker};
pub use manager::{
    aggregate_health, scope_includes, HealthManager, DEFAULT_CHECK_TIMEOUT_SECONDS,
    DEFAULT_POLL_INTERVAL_MILLIS,
};
pub use monitor::{HealthMonitor, DEFAULT_MAX_CONCURRENCY};
pub use process::ProcessHealthChecker;
pub use registry::HealthCheckerRegistry;
pub use tcp::TcpHealthChecker;
