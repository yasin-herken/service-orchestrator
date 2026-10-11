//! The read-only task-engine view the MCP adapter observes through.
//!
//! The application's [`TaskManager`](service_orchestrator_execution::TaskManager)
//! contract exposes task *submission* and lookup by id, but not the richer list
//! API the interface needs. That API exists on the concrete
//! [`TaskEngine`](service_orchestrator_execution::TaskEngine). Rather than widen
//! the application boundary, the interface depends on this small,
//! interface-facing trait — which the engine implements and which a test can
//! implement with a fake.
//!
//! This mirrors the TUI's `TaskObserver` exactly and exists for the same reason:
//! the interface must be able to answer `list_tasks` without holding a tool
//! request open and without reaching past the application core
//! (`tasks/014.md` §17, §30).

use std::sync::Arc;

use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::Task;
use service_orchestrator_execution::TaskEngine;

/// A read-only, list-oriented view of the task engine.
pub trait TaskMonitor: Send + Sync {
    /// Returns every known task, oldest first.
    fn tasks(&self) -> Vec<Task>;

    /// Returns up to `limit` most recently created tasks, newest first.
    fn recent_tasks(&self, limit: usize) -> Vec<Task>;
}

impl TaskMonitor for TaskEngine {
    fn tasks(&self) -> Vec<Task> {
        Self::list_tasks(self)
    }

    fn recent_tasks(&self, limit: usize) -> Vec<Task> {
        Self::list_recent_tasks(self, limit)
    }
}

/// A [`TaskMonitor`] backed by the shared-runtime client.
///
/// The daemon owns the task engine, so an MCP client cannot reach it directly.
/// This adapter forwards the two read-only list queries over IPC, keeping the
/// adapter unaware of transport (`tasks/015.md` §13, §19).
pub struct RemoteTaskMonitor {
    client: Arc<dyn OrchestratorClient>,
}

impl RemoteTaskMonitor {
    /// Builds a monitor over a shared-runtime client.
    #[must_use]
    pub fn new(client: Arc<dyn OrchestratorClient>) -> Self {
        Self { client }
    }
}

impl TaskMonitor for RemoteTaskMonitor {
    fn tasks(&self) -> Vec<Task> {
        self.client.list_tasks()
    }

    fn recent_tasks(&self, limit: usize) -> Vec<Task> {
        self.client.list_recent_tasks(limit)
    }
}
