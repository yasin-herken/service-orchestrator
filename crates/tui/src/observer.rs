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
