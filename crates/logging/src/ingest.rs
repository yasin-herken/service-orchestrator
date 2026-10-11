//! Ingestors that feed existing event sources into the log hub.
//!
//! The observability layer does not replace the process manager, the task
//! engine, or the runtime events they already publish; it *correlates* them.
//! These ingestors translate the events those components emit into structured
//! [`LogEntry`] values and record them in a [`LogHub`], so historical queries
//! and diagnostics can see them alongside application messages
//! (`tasks/012.md` §20, §29).
//!
//! # Correlation, not fabrication
//!
//! A process line carries a process id but not a service; the ingestor resolves
//! the service through the process manager's registry rather than guessing. A
//! task event carries the task id, and only `TaskCreated` carries the service —
//! the ingestor caches that mapping so later events for the same task inherit it
//! (`tasks/012.md` §21, §29). Workspace metadata is never invented.
//!
//! # Delivery is lossy and non-blocking
//!
//! Both ingestors consume bounded broadcast streams. If the ingestor itself
//! falls behind, it observes `Lagged`, skips the lost events, and continues; it
//! never blocks the producer (`tasks/012.md` §22, §27).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use service_orchestrator_application::{LogEntry, LogLevel, LogSource, LogStream};
use service_orchestrator_domain::{ServiceId, TaskId, WorkspaceId};
use service_orchestrator_execution::{Cancellation, TaskEvent};
use service_orchestrator_process::{LogLine, LogStreamKind, ProcessManager};
use tokio::sync::broadcast;

use crate::hub::LogHub;

/// Builds a log entry from a captured process line.
///
/// The severity is left at [`LogLevel::Info`] because process output does not
/// carry one; the originating stream is preserved instead, so a consumer can
/// distinguish `stdout` from `stderr` without the hub inventing a level.
#[must_use]
pub fn entry_from_process_line(
    line: &LogLine,
    service_id: Option<ServiceId>,
    workspace_id: Option<WorkspaceId>,
) -> LogEntry {
    let mut entry = LogEntry::new(LogLevel::Info, line.line.clone())
        .with_process(line.process.as_str())
        .with_source(LogSource::Process)
        .with_stream(stream_of(line.stream));
    entry.timestamp = line.timestamp;
    if let Some(service) = service_id {
        entry = entry.with_service(service);
    }
    if let Some(workspace) = workspace_id {
        entry = entry.with_workspace(workspace);
    }
    entry
}

/// Builds a log entry from a task-engine event.
///
/// `service_id` is the service the task belongs to, when known; it is resolved
/// by the [`TaskLogIngestor`] from its task-to-service cache.
#[must_use]
pub fn entry_from_task_event(event: &TaskEvent, service_id: Option<ServiceId>) -> LogEntry {
    let (task_id, level, message) = describe_task_event(event);
    let mut entry = LogEntry::new(level, message).with_source(LogSource::Task);
    if let Some(task_id) = task_id {
        entry = entry.with_task(task_id);
    }
    if let Some(service) = service_id {
        entry = entry.with_service(service);
    }
    entry
}

/// Returns the service a task event concerns, when the event carries one.
///
/// Only `TaskCreated` includes the owning service in the event payload.
#[must_use]
pub fn task_event_service(event: &TaskEvent) -> Option<ServiceId> {
    match event {
        TaskEvent::TaskCreated { task } => task.service.clone(),
        _ => None,
    }
}

/// Returns the task id a task event concerns.
#[must_use]
pub fn task_event_id(event: &TaskEvent) -> Option<&TaskId> {
    match event {
        TaskEvent::TaskCreated { task } => Some(&task.id),
        TaskEvent::TaskStarted { id, .. }
        | TaskEvent::TaskProgress { id, .. }
        | TaskEvent::TaskMessage { id, .. }
        | TaskEvent::TaskRetrying { id, .. }
        | TaskEvent::TaskSucceeded { id }
        | TaskEvent::TaskFailed { id, .. }
        | TaskEvent::TaskCancelled { id }
        | TaskEvent::TaskSkipped { id }
        | TaskEvent::TaskBlocked { id } => Some(id),
        TaskEvent::WorkflowCreated { .. } => None,
    }
}

/// Maps the rich task event into an optional id, a severity, and a message.
///
/// A workflow event carries no task, so its id is `None`; metadata is never
/// fabricated to fill the field.
fn describe_task_event(event: &TaskEvent) -> (Option<TaskId>, LogLevel, String) {
    match event {
        TaskEvent::TaskCreated { task } => (
            Some(task.id.clone()),
            LogLevel::Info,
            format!("task '{}' queued ({})", task.id, task.kind),
        ),
        TaskEvent::TaskStarted { id, kind } => (
            Some(id.clone()),
            LogLevel::Info,
            format!("task '{id}' started ({kind})"),
        ),
        TaskEvent::TaskProgress { id, progress } => {
            let message = progress
                .message
                .clone()
                .unwrap_or_else(|| match progress.total {
                    Some(total) => format!("task '{id}' progress {}/{}", progress.current, total),
                    None => format!("task '{id}' progress {}", progress.current),
                });
            (Some(id.clone()), LogLevel::Debug, message)
        }
        TaskEvent::TaskMessage { id, message } => {
            (Some(id.clone()), LogLevel::Info, message.clone())
        }
        TaskEvent::TaskRetrying { id, attempt } => (
            Some(id.clone()),
            LogLevel::Warn,
            format!("task '{id}' retrying (attempt {attempt})"),
        ),
        TaskEvent::TaskSucceeded { id } => (
            Some(id.clone()),
            LogLevel::Info,
            format!("task '{id}' succeeded"),
        ),
        TaskEvent::TaskFailed { id, failure } => (
            Some(id.clone()),
            LogLevel::Error,
            format!("task '{id}' failed ({}): {}", failure.code, failure.message),
        ),
        TaskEvent::TaskCancelled { id } => (
            Some(id.clone()),
            LogLevel::Warn,
            format!("task '{id}' cancelled"),
        ),
        TaskEvent::TaskSkipped { id } => (
            Some(id.clone()),
            LogLevel::Warn,
            format!("task '{id}' skipped"),
        ),
        TaskEvent::TaskBlocked { id } => (
            Some(id.clone()),
            LogLevel::Debug,
            format!("task '{id}' blocked on dependencies"),
        ),
        TaskEvent::WorkflowCreated { workflow } => (
            None,
            LogLevel::Info,
            format!(
                "workflow '{}' created with {} step(s)",
                workflow.id,
                workflow.len()
            ),
        ),
    }
}

/// Maps a process stream kind onto the interface-neutral log stream.
fn stream_of(kind: LogStreamKind) -> LogStream {
    match kind {
        LogStreamKind::Stdout => LogStream::Stdout,
        LogStreamKind::Stderr => LogStream::Stderr,
    }
}

/// Records managed-process output in a log hub.
pub struct ProcessLogIngestor {
    hub: Arc<LogHub>,
    manager: ProcessManager,
}

impl ProcessLogIngestor {
    /// Creates an ingestor over a hub and the process manager.
    #[must_use]
    pub fn new(hub: Arc<LogHub>, manager: ProcessManager) -> Self {
        Self { hub, manager }
    }

    /// Runs until the process manager shuts down or `cancellation` fires.
    pub async fn run(self, cancellation: Option<Cancellation>) {
        let mut stream = self.manager.subscribe_logs();
        loop {
            let next = match &cancellation {
                Some(cancellation) => tokio::select! {
                    () = cancellation.cancelled() => return,
                    line = stream.recv() => line,
                },
                None => stream.recv().await,
            };
            match next {
                Ok(line) => {
                    let service = self
                        .manager
                        .snapshot(&line.process)
                        .and_then(|snapshot| snapshot.service);
                    self.hub
                        .record(entry_from_process_line(&line, service, None));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

impl std::fmt::Debug for ProcessLogIngestor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessLogIngestor").finish_non_exhaustive()
    }
}

/// Records task-engine events in a log hub, tracking each task's service.
pub struct TaskLogIngestor {
    hub: Arc<LogHub>,
    services: Mutex<HashMap<TaskId, ServiceId>>,
}

impl TaskLogIngestor {
    /// Creates an ingestor over a hub.
    #[must_use]
    pub fn new(hub: Arc<LogHub>) -> Self {
        Self {
            hub,
            services: Mutex::new(HashMap::new()),
        }
    }

    /// Translates one event, resolving its service, and records it.
    ///
    /// Returns the assigned sequence.
    pub fn handle(&self, event: &TaskEvent) -> u64 {
        let service = self.resolve_service(event);
        self.hub.record(entry_from_task_event(event, service))
    }

    /// Returns the number of cached task-to-service associations.
    #[must_use]
    pub fn cached_services(&self) -> usize {
        self.services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Runs until the task event stream closes.
    pub async fn run(self, mut receiver: broadcast::Receiver<TaskEvent>) {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    self.handle(&event);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }

    /// Resolves and caches the service associated with an event's task.
    fn resolve_service(&self, event: &TaskEvent) -> Option<ServiceId> {
        if let Some(service) = task_event_service(event) {
            if let Some(id) = task_event_id(event) {
                self.services
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(id.clone(), service.clone());
            }
            return Some(service);
        }
        task_event_id(event).and_then(|id| {
            self.services
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(id)
                .cloned()
        })
    }
}

impl std::fmt::Debug for TaskLogIngestor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskLogIngestor")
            .field("cached_services", &self.cached_services())
            .finish_non_exhaustive()
    }
}

/// Spawns a [`ProcessLogIngestor`] on the current runtime.
///
/// # Panics
///
/// Panics if called outside a Tokio runtime context.
#[must_use]
pub fn spawn_process_ingestor(
    hub: Arc<LogHub>,
    manager: ProcessManager,
    cancellation: Option<Cancellation>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(ProcessLogIngestor::new(hub, manager).run(cancellation))
}

/// Spawns a [`TaskLogIngestor`] on the current runtime.
///
/// # Panics
///
/// Panics if called outside a Tokio runtime context.
#[must_use]
pub fn spawn_task_ingestor(
    hub: Arc<LogHub>,
    receiver: broadcast::Receiver<TaskEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(TaskLogIngestor::new(hub).run(receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::{LogRequest, LogService};
    use service_orchestrator_domain::{Task, TaskFailure, TaskKind, TaskProgress, Timestamp};
    use service_orchestrator_process::ProcessId;

    fn service() -> ServiceId {
        ServiceId::new("auth-service").unwrap()
    }

    #[test]
    fn process_lines_become_entries_with_correlation() {
        let line = LogLine {
            process: ProcessId::new("proc-0").unwrap(),
            stream: LogStreamKind::Stderr,
            line: "boom".to_owned(),
            timestamp: Timestamp::now(),
        };
        let entry = entry_from_process_line(&line, Some(service()), None);
        assert_eq!(entry.source, LogSource::Process);
        assert_eq!(entry.stream, Some(LogStream::Stderr));
        assert_eq!(entry.process_id.as_deref(), Some("proc-0"));
        assert_eq!(
            entry.service_id.as_ref().map(ServiceId::as_str),
            Some("auth-service")
        );
        assert_eq!(entry.level, LogLevel::Info);
    }

    #[test]
    fn task_events_become_entries() {
        let created = TaskEvent::TaskCreated {
            task: Box::new(
                Task::new(TaskId::new("t1").unwrap(), TaskKind::BuildService)
                    .with_service(service()),
            ),
        };
        let entry = entry_from_task_event(&created, task_event_service(&created));
        assert_eq!(entry.source, LogSource::Task);
        assert_eq!(entry.level, LogLevel::Info);
        assert_eq!(entry.task_id.as_ref().map(TaskId::as_str), Some("t1"));
        assert!(entry.message.contains("queued"));

        let failed = TaskEvent::TaskFailed {
            id: TaskId::new("t1").unwrap(),
            failure: TaskFailure::new("BUILD_FAILED", "boom", None).unwrap(),
        };
        let entry = entry_from_task_event(&failed, None);
        assert_eq!(entry.level, LogLevel::Error);
        assert!(entry.message.contains("BUILD_FAILED"));
    }

    #[test]
    fn progress_without_message_is_described() {
        let event = TaskEvent::TaskProgress {
            id: TaskId::new("t1").unwrap(),
            progress: TaskProgress::new(2, Some(4), None).unwrap(),
        };
        let entry = entry_from_task_event(&event, None);
        assert_eq!(entry.level, LogLevel::Debug);
        assert!(entry.message.contains("2/4"));
    }

    #[test]
    fn the_task_ingestor_caches_task_to_service() {
        let hub = Arc::new(LogHub::new());
        let ingestor = TaskLogIngestor::new(Arc::clone(&hub));

        ingestor.handle(&TaskEvent::TaskCreated {
            task: Box::new(
                Task::new(TaskId::new("t1").unwrap(), TaskKind::BuildService)
                    .with_service(service()),
            ),
        });
        assert_eq!(ingestor.cached_services(), 1);

        let sequence = ingestor.handle(&TaskEvent::TaskSucceeded {
            id: TaskId::new("t1").unwrap(),
        });
        assert!(sequence > 0);

        let entries = hub.query(&LogRequest::new(service())).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn the_task_ingestor_consumes_a_broadcast_stream() {
        let hub = Arc::new(LogHub::new());
        let (sender, receiver) = broadcast::channel(8);
        let handle = tokio::spawn(TaskLogIngestor::new(Arc::clone(&hub)).run(receiver));
        sender
            .send(TaskEvent::TaskCreated {
                task: Box::new(
                    Task::new(TaskId::new("t9").unwrap(), TaskKind::GitSync)
                        .with_service(service()),
                ),
            })
            .unwrap();
        // Give the ingestor a turn.
        tokio::task::yield_now().await;
        drop(sender);
        handle.await.unwrap();
        assert_eq!(hub.query(&LogRequest::new(service())).unwrap().len(), 1);
    }

    #[test]
    fn workflow_creation_is_logged_without_a_task() {
        let workflow = service_orchestrator_domain::Workflow::new(
            service_orchestrator_domain::WorkflowId::new("w1").unwrap(),
            service_orchestrator_domain::WorkflowKind::PrepareEnvironment,
        );
        let event = TaskEvent::WorkflowCreated {
            workflow: Box::new(workflow),
        };
        assert!(task_event_id(&event).is_none());
        let entry = entry_from_task_event(&event, None);
        assert!(entry.message.contains("workflow 'w1'"));
    }
}
