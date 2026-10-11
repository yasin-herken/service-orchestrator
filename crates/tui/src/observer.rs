//! The task-engine boundary the interface observes through.
//!
//! The application's [`TaskManager`](service_orchestrator_execution::TaskManager)
//! contract exposes task *submission* and lookup by id, but not the richer list
//! and event API the interface needs. That API exists on the concrete
//! [`TaskEngine`]. Rather than widen the application boundary, the interface
//! depends on this small, interface-facing trait — which the engine implements
//! and which a test can implement with a fake.
//!
//! [`TaskEngine`]: service_orchestrator_execution::TaskEngine

use std::sync::Arc;

use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::Task;
use service_orchestrator_execution::{TaskEngine, TaskEvent};
use tokio::sync::broadcast;

/// A read-only, event-publishing view of the task engine.
pub trait TaskObserver: Send + Sync {
    /// Returns every task, oldest first.
    fn tasks(&self) -> Vec<Task>;

    /// Returns the tasks that are currently running.
    fn running_tasks(&self) -> Vec<Task>;

    /// Returns up to `limit` most recently finished tasks, newest first.
    fn recent_tasks(&self, limit: usize) -> Vec<Task>;

    /// Subscribes to task events.
    fn subscribe(&self) -> broadcast::Receiver<TaskEvent>;
}

impl TaskObserver for TaskEngine {
    fn tasks(&self) -> Vec<Task> {
        TaskEngine::list_tasks(self)
    }

    fn running_tasks(&self) -> Vec<Task> {
        TaskEngine::list_running_tasks(self)
    }

    fn recent_tasks(&self, limit: usize) -> Vec<Task> {
        TaskEngine::list_recent_tasks(self, limit)
    }

    fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        TaskEngine::subscribe(self)
    }
}

/// A [`TaskObserver`] backed by the shared-runtime client.
///
/// The daemon owns the task engine, so a remote interface observes tasks by
/// asking the client for the running and recent lists and subscribing to the
/// overlay events the client republishes (`tasks/015.md` §19, §20).
pub struct RemoteTaskObserver {
    client: Arc<dyn OrchestratorClient>,
}

impl RemoteTaskObserver {
    /// Builds an observer over a shared-runtime client.
    #[must_use]
    pub fn new(client: Arc<dyn OrchestratorClient>) -> Self {
        Self { client }
    }
}

impl TaskObserver for RemoteTaskObserver {
    fn tasks(&self) -> Vec<Task> {
        self.client.list_tasks()
    }

    fn running_tasks(&self) -> Vec<Task> {
        self.client.list_running_tasks()
    }

    fn recent_tasks(&self, limit: usize) -> Vec<Task> {
        self.client.list_recent_tasks(limit)
    }

    fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.client.subscribe_task_events()
    }
}
