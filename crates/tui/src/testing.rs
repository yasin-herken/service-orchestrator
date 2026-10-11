//! Test support: in-memory fakes for the two engine-facing boundaries.
//!
//! This module exists so interface tests — including integration tests in
//! `tests/` — can drive [`crate::App`] without a real task engine, a real Git
//! repository, or a real process. Nothing here is used by the production event
//! loop; it is compiled unconditionally only because integration tests cannot
//! reach `#[cfg(test)]` items.
//!
//! [`FakeTasks`] implements both the application's
//! [`TaskManager`](service_orchestrator_execution::TaskManager) boundary and the
//! interface's [`TaskObserver`] boundary, so a single fake backs both the
//! application core and the interface under test.

use std::collections::BTreeMap;
use std::sync::Mutex;

use service_orchestrator_domain::{
    Task, TaskFailure, TaskId, TaskStatus, Workflow, WorkflowId, WorkflowStep,
};
use service_orchestrator_execution::{
    TaskDefinition, TaskEvent, TaskManager, TaskManagerError, TaskTarget, WorkflowDefinition,
};
use tokio::sync::broadcast;

use crate::observer::TaskObserver;

/// Shared mutable state behind a fake engine.
struct Inner {
    tasks: Vec<Task>,
    workflows: BTreeMap<WorkflowId, (Workflow, Vec<TaskId>)>,
    counter: u64,
    workflow_counter: u64,
}

/// An in-memory [`TaskManager`] and [`TaskObserver`].
pub struct FakeTasks {
    inner: Mutex<Inner>,
    events: broadcast::Sender<TaskEvent>,
}

impl Default for FakeTasks {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeTasks {
    /// Creates an empty fake engine.
    #[must_use]
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            inner: Mutex::new(Inner {
                tasks: Vec::new(),
                workflows: BTreeMap::new(),
                counter: 0,
                workflow_counter: 0,
            }),
            events,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn next_task_id(inner: &mut Inner) -> TaskId {
        let value = inner.counter;
        inner.counter += 1;
        TaskId::new(format!("task-{value}")).expect("generated task ids are valid")
    }

    /// Publishes an event to subscribers.
    ///
    /// A send error only means there are currently no subscribers.
    pub fn publish(&self, event: TaskEvent) {
        let _ = self.events.send(event);
    }

    /// Inserts a task and publishes its creation event.
    pub fn insert(&self, task: Task) {
        {
            let mut inner = self.lock();
            inner.tasks.push(task.clone());
        }
        self.publish(TaskEvent::TaskCreated {
            task: Box::new(task),
        });
    }

    /// Moves a task to a new status, recording timestamps and publishing the
    /// matching event.
    ///
    /// # Panics
    ///
    /// Panics if the task does not exist.
    pub fn set_status(&self, id: &TaskId, status: TaskStatus) {
        let mut inner = self.lock();
        let task = inner
            .tasks
            .iter_mut()
            .find(|task| &task.id == id)
            .expect("task exists");
        let from = task.status;
        task.status = status;
        match status {
            TaskStatus::Running => {
                if task.started_at.is_none() {
                    task.started_at = Some(service_orchestrator_domain::Timestamp::now());
                }
            }
            TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Cancelled
            | TaskStatus::Skipped => {
                task.finished_at = Some(service_orchestrator_domain::Timestamp::now());
            }
            TaskStatus::Queued | TaskStatus::Blocked => {}
        }
        let task = task.clone();
        drop(inner);
        let _ = from;
        match status {
            TaskStatus::Running => self.publish(TaskEvent::TaskStarted {
                id: task.id.clone(),
                kind: task.kind.clone(),
            }),
            TaskStatus::Succeeded => self.publish(TaskEvent::TaskSucceeded {
                id: task.id.clone(),
            }),
            TaskStatus::Failed => self.publish(TaskEvent::TaskFailed {
                id: task.id.clone(),
                failure: TaskFailure::new("FAKE_FAILURE", "the fake task failed", None)
                    .expect("static failure is valid"),
            }),
            TaskStatus::Cancelled => {
                self.publish(TaskEvent::TaskCancelled {
                    id: task.id.clone(),
                });
            }
            TaskStatus::Skipped => self.publish(TaskEvent::TaskSkipped {
                id: task.id.clone(),
            }),
            TaskStatus::Blocked => self.publish(TaskEvent::TaskBlocked {
                id: task.id.clone(),
            }),
            TaskStatus::Queued => {}
        }
    }

    /// Moves a task to `running`.
    pub fn mark_running(&self, id: &TaskId) {
        self.set_status(id, TaskStatus::Running);
    }

    /// Moves a task to `succeeded`.
    pub fn mark_succeeded(&self, id: &TaskId) {
        self.set_status(id, TaskStatus::Succeeded);
    }

    /// Moves a task to `failed`.
    pub fn mark_failed(&self, id: &TaskId) {
        self.set_status(id, TaskStatus::Failed);
    }

    /// Returns the identifiers of every task, oldest first.
    #[must_use]
    pub fn task_ids(&self) -> Vec<TaskId> {
        self.lock()
            .tasks
            .iter()
            .map(|task| task.id.clone())
            .collect()
    }
}

impl TaskManager for FakeTasks {
    fn submit(&self, definition: TaskDefinition) -> Result<TaskId, TaskManagerError> {
        let mut inner = self.lock();
        let id = Self::next_task_id(&mut inner);
        let mut task = Task::new(id.clone(), definition.kind.clone());
        match &definition.target {
            TaskTarget::Service(service) => task = task.with_service(service.clone()),
            TaskTarget::Library(library) => task = task.with_library(library.clone()),
            TaskTarget::Workflow(workflow) => task = task.with_workflow(workflow.clone()),
            TaskTarget::None => {}
        }
        if let Some(description) = definition.description.clone() {
            task.progress.message = Some(description);
        }
        inner.tasks.push(task.clone());
        drop(inner);
        self.publish(TaskEvent::TaskCreated {
            task: Box::new(task),
        });
        Ok(id)
    }

    fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskManagerError> {
        if definition.steps.is_empty() {
            return Err(TaskManagerError::Rejected {
                reason: "workflow must have at least one step".to_owned(),
            });
        }
        let mut inner = self.lock();
        let workflow_value = inner.workflow_counter;
        inner.workflow_counter += 1;
        let workflow_id =
            WorkflowId::new(format!("workflow-{workflow_value}")).expect("valid workflow id");
        let mut task_ids = Vec::new();
        let mut tasks = Vec::new();
        for step in &definition.steps {
            let id = Self::next_task_id(&mut inner);
            let mut task =
                Task::new(id.clone(), step.task.clone()).with_workflow(workflow_id.clone());
            if let Some(description) = step.description.clone() {
                task.progress.message = Some(description);
            }
            task_ids.push(id);
            tasks.push(task);
        }
        inner.tasks.extend(tasks.iter().cloned());
        let workflow = Workflow {
            id: workflow_id.clone(),
            kind: definition.kind,
            steps: definition.steps,
        };
        inner
            .workflows
            .insert(workflow_id.clone(), (workflow.clone(), task_ids));
        drop(inner);
        self.publish(TaskEvent::WorkflowCreated {
            workflow: Box::new(workflow),
        });
        for task in tasks {
            self.publish(TaskEvent::TaskCreated {
                task: Box::new(task),
            });
        }
        Ok(workflow_id)
    }

    fn task(&self, id: &TaskId) -> Result<Task, TaskManagerError> {
        self.lock()
            .tasks
            .iter()
            .find(|task| &task.id == id)
            .cloned()
            .ok_or_else(|| TaskManagerError::NotFound {
                entity: "task",
                id: id.to_string(),
            })
    }

    fn workflow(&self, id: &WorkflowId) -> Result<Workflow, TaskManagerError> {
        self.lock()
            .workflows
            .get(id)
            .map(|(workflow, _)| workflow.clone())
            .ok_or_else(|| TaskManagerError::NotFound {
                entity: "workflow",
                id: id.to_string(),
            })
    }

    fn workflow_tasks(&self, id: &WorkflowId) -> Result<Vec<Task>, TaskManagerError> {
        let inner = self.lock();
        let (_, ids) = inner
            .workflows
            .get(id)
            .ok_or_else(|| TaskManagerError::NotFound {
                entity: "workflow",
                id: id.to_string(),
            })?;
        Ok(inner
            .tasks
            .iter()
            .filter(|task| ids.contains(&task.id))
            .cloned()
            .collect())
    }

    fn cancel(&self, id: &TaskId) -> Result<(), TaskManagerError> {
        let mut inner = self.lock();
        let task = inner
            .tasks
            .iter_mut()
            .find(|task| &task.id == id)
            .ok_or_else(|| TaskManagerError::NotFound {
                entity: "task",
                id: id.to_string(),
            })?;
        if !task.status.is_terminal() {
            task.status = TaskStatus::Cancelled;
            task.finished_at = Some(service_orchestrator_domain::Timestamp::now());
        }
        drop(inner);
        self.publish(TaskEvent::TaskCancelled { id: id.clone() });
        Ok(())
    }
}

impl TaskObserver for FakeTasks {
    fn tasks(&self) -> Vec<Task> {
        self.lock().tasks.clone()
    }

    fn running_tasks(&self) -> Vec<Task> {
        self.lock()
            .tasks
            .iter()
            .filter(|task| task.status.is_active())
            .cloned()
            .collect()
    }

    fn recent_tasks(&self, limit: usize) -> Vec<Task> {
        let inner = self.lock();
        inner
            .tasks
            .iter()
            .filter(|task| task.status.is_terminal())
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.events.subscribe()
    }
}

/// A convenience for building a workflow step.
#[must_use]
pub fn step(name: &str, task: service_orchestrator_domain::TaskKind) -> WorkflowStep {
    WorkflowStep::new(
        service_orchestrator_domain::StepId::new(name).expect("valid step id"),
        task,
    )
}

/// Renders one frame of the interface to an in-memory buffer.
///
/// Returns the rendered screen as text with each row on its own line. This lets
/// integration tests assert on what a developer would see without requiring a
/// real interactive terminal.
///
/// # Panics
///
/// Panics if the in-memory terminal cannot be created or drawn, which indicates
/// a programming error rather than a runtime condition.
#[must_use]
pub fn render(app: &crate::App, width: u16, height: u16) -> String {
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    fn text(buffer: &Buffer, width: u16) -> String {
        let width = usize::from(width).max(1);
        buffer
            .content()
            .chunks(width)
            .map(|row| row.iter().map(ratatui::buffer::Cell::symbol).collect())
            .collect::<Vec<String>>()
            .join("\n")
    }

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("an in-memory terminal is always created");
    terminal
        .draw(|frame| crate::router::draw(frame, app))
        .expect("drawing to an in-memory buffer cannot fail");
    text(terminal.backend().buffer(), width)
}
