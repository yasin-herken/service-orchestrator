//! The task engine.
//!
//! [`TaskEngine`] implements the application-facing
//! [`TaskManager`](crate::TaskManager) contract and adds the richer query and
//! event API that the TUI and MCP will use. It is a thin façade over shared
//! [`EngineInner`] state; all scheduling lives in [`crate::scheduler`].
//!
//! # Ownership and runtime
//!
//! The engine does **not** own a Tokio runtime. It is built inside a runtime
//! context and captures a [`Handle`], so the composition root decides how the
//! runtime is created and kept alive. `submit` and `cancel` are synchronous and
//! can be called from a TUI event loop or an MCP handler; the scheduler and
//! task executions run on the captured runtime.
//!
//! # Workflow mapping
//!
//! The application submits a [`TaskDefinition`] or a [`WorkflowDefinition`]. A
//! workflow is expanded eagerly into one task per step, with step prerequisites
//! translated into [`TaskId`] dependencies. The same dependency-aware scheduler
//! then runs both.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_domain::{
    StepId, Task, TaskId, TaskKind, TaskProgress, TaskStatus, Workflow, WorkflowId,
};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, Notify, Semaphore};

use crate::cancellation::Cancellation;
use crate::context::ProgressSink;
use crate::error::TaskEngineError;
use crate::events::{TaskEvent, TaskEventBus};
use crate::executor::TaskExecutor;
use crate::registry::ExecutorRegistry;
use crate::store::{StatusChange, TaskRecord, TaskStore};
use crate::task_manager::{
    DependencyPolicy, TaskDefinition, TaskManager, TaskManagerError, TaskTarget, WorkflowDefinition,
};

/// Runtime configuration for the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// The maximum number of tasks that may run at once.
    pub max_parallel_tasks: usize,
    /// An optional cap on the number of non-terminal tasks.
    pub max_queued_tasks: Option<usize>,
    /// An optional default timeout applied to tasks that do not set one.
    pub default_timeout: Option<Duration>,
    /// The event broadcast channel capacity.
    pub event_capacity: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_parallel_tasks: 4,
            max_queued_tasks: None,
            default_timeout: None,
            event_capacity: 256,
        }
    }
}

/// Builds a [`TaskEngine`].
///
/// Executors are registered by task kind; a task whose kind has no executor
/// fails with a structured `EXECUTOR_NOT_FOUND` error when it runs.
#[derive(Default)]
pub struct TaskEngineBuilder {
    config: EngineConfig,
    executors: ExecutorRegistry,
}

impl TaskEngineBuilder {
    /// Creates a builder with default configuration and no executors.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the engine configuration.
    #[must_use]
    pub fn config(mut self, config: EngineConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the maximum number of tasks that may run concurrently.
    #[must_use]
    pub fn max_parallel_tasks(mut self, max: usize) -> Self {
        self.config.max_parallel_tasks = max.max(1);
        self
    }

    /// Sets the cap on non-terminal tasks.
    #[must_use]
    pub fn max_queued_tasks(mut self, max: usize) -> Self {
        self.config.max_queued_tasks = Some(max);
        self
    }

    /// Sets the default per-task timeout.
    #[must_use]
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.config.default_timeout = Some(timeout);
        self
    }

    /// Registers an executor for a task kind.
    #[must_use]
    pub fn executor(mut self, kind: TaskKind, executor: Arc<dyn TaskExecutor>) -> Self {
        self.executors.register(kind, executor);
        self
    }

    /// Builds the engine inside the current Tokio runtime context.
    ///
    /// # Errors
    ///
    /// Returns [`TaskEngineError::NoRuntime`] when called outside a Tokio
    /// runtime.
    pub fn build(self) -> Result<TaskEngine, TaskEngineError> {
        let handle = Handle::try_current().map_err(|_| TaskEngineError::NoRuntime)?;
        let config = self.config;
        let max = config.max_parallel_tasks.max(1);
        let inner = Arc::new(EngineInner {
            store: TaskStore::new(),
            executors: self.executors,
            events: TaskEventBus::new(config.event_capacity),
            config: EngineConfig {
                max_parallel_tasks: max,
                ..config
            },
            notify: Notify::new(),
            scheduler_cancel: Cancellation::new(),
            semaphore: Arc::new(Semaphore::new(max)),
            task_counter: AtomicU64::new(0),
            workflow_counter: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            handle: handle.clone(),
        });

        handle.spawn(crate::scheduler::run(inner.clone()));
        Ok(TaskEngine { inner })
    }
}

/// Shared engine state, behind a single [`Arc`].
pub(crate) struct EngineInner {
    pub(crate) store: TaskStore,
    pub(crate) executors: ExecutorRegistry,
    pub(crate) events: TaskEventBus,
    pub(crate) config: EngineConfig,
    pub(crate) notify: Notify,
    pub(crate) scheduler_cancel: Cancellation,
    pub(crate) semaphore: Arc<Semaphore>,
    task_counter: AtomicU64,
    workflow_counter: AtomicU64,
    shutdown: AtomicBool,
    pub(crate) handle: Handle,
}

impl EngineInner {
    /// Returns `true` once shutdown has begun.
    pub(crate) fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Publishes an event.
    pub(crate) fn publish(&self, event: TaskEvent) {
        self.events.publish(event);
    }

    /// Publishes the event that corresponds to a recorded status change.
    pub(crate) fn publish_status_change(&self, change: &StatusChange) {
        match change.to {
            TaskStatus::Running => {
                let kind = self
                    .store
                    .task(&change.id)
                    .map_or(TaskKind::Custom("unknown".to_owned()), |task| task.kind);
                self.publish(TaskEvent::TaskStarted {
                    id: change.id.clone(),
                    kind,
                });
            }
            TaskStatus::Succeeded => self.publish(TaskEvent::TaskSucceeded {
                id: change.id.clone(),
            }),
            TaskStatus::Failed => {
                let failure = change.failure.clone().unwrap_or_else(|| {
                    service_orchestrator_domain::TaskFailure::new(
                        "TASK_FAILED",
                        "task failed",
                        None,
                    )
                    .expect("static failure is valid")
                });
                self.publish(TaskEvent::TaskFailed {
                    id: change.id.clone(),
                    failure,
                });
            }
            TaskStatus::Cancelled => self.publish(TaskEvent::TaskCancelled {
                id: change.id.clone(),
            }),
            TaskStatus::Skipped => self.publish(TaskEvent::TaskSkipped {
                id: change.id.clone(),
            }),
            TaskStatus::Blocked => self.publish(TaskEvent::TaskBlocked {
                id: change.id.clone(),
            }),
            TaskStatus::Queued => {}
        }
    }

    /// Generates the next task identifier.
    pub(crate) fn next_task_id(&self) -> TaskId {
        let value = self.task_counter.fetch_add(1, Ordering::SeqCst);
        TaskId::new(format!("task-{value}")).expect("generated task ids are valid")
    }

    fn next_workflow_id(&self) -> WorkflowId {
        let value = self.workflow_counter.fetch_add(1, Ordering::SeqCst);
        WorkflowId::new(format!("workflow-{value}")).expect("generated workflow ids are valid")
    }

    /// Enforces the configured backpressure limit before accepting new work.
    fn check_capacity(&self, incoming: usize) -> Result<(), TaskEngineError> {
        if self.is_shutdown() {
            return Err(TaskEngineError::ShuttingDown);
        }
        if let Some(limit) = self.config.max_queued_tasks {
            if self.store.active_count() + incoming > limit {
                return Err(TaskEngineError::QueueFull { limit });
            }
        }
        Ok(())
    }

    /// Builds a task record from a definition.
    fn build_record(
        &self,
        id: TaskId,
        definition: TaskDefinition,
        workflow: Option<WorkflowId>,
    ) -> TaskRecord {
        let mut task = Task::new(id, definition.kind.clone());
        match &definition.target {
            TaskTarget::Service(service) => task = task.with_service(service.clone()),
            TaskTarget::Library(library) => task = task.with_library(library.clone()),
            TaskTarget::Workflow(id) => task = task.with_workflow(id.clone()),
            TaskTarget::None => {}
        }
        if let Some(workflow) = workflow {
            task = task.with_workflow(workflow);
        }
        task.dependencies = definition.dependencies.clone();
        if let Some(description) = definition.description.clone() {
            // Seed the task's progress message with its description so callers
            // see *why* the task exists before the executor reports anything.
            task.progress.message = Some(description);
        }

        TaskRecord {
            task,
            target: definition.target,
            retry: definition.retry,
            dependency_policy: definition.dependency_policy,
            timeout: definition
                .timeout_seconds
                .map(Duration::from_secs)
                .or(self.config.default_timeout),
            attempt: 0,
            sequence: 0,
            cancellation: Cancellation::new(),
        }
    }
}

impl ProgressSink for EngineInner {
    fn set_progress(
        &self,
        id: &TaskId,
        current: u32,
        total: Option<u32>,
        message: Option<String>,
    ) -> Result<(), TaskEngineError> {
        let progress = TaskProgress::new(current, total, message)?;
        self.store.set_progress(id, progress.clone())?;
        self.publish(TaskEvent::TaskProgress {
            id: id.clone(),
            progress,
        });
        Ok(())
    }

    fn set_message(&self, id: &TaskId, message: String) {
        if self.store.set_message(id, message.clone()).is_ok() {
            self.publish(TaskEvent::TaskMessage {
                id: id.clone(),
                message,
            });
        }
    }
}

/// The task engine.
///
/// Not `Clone`: a single instance owns the engine's lifetime. Share it behind
/// an `Arc<dyn TaskManager>` (or `Arc<TaskEngine>`) when multiple components
/// need it.
pub struct TaskEngine {
    inner: Arc<EngineInner>,
}

impl TaskEngine {
    /// Creates a builder.
    #[must_use]
    pub fn builder() -> TaskEngineBuilder {
        TaskEngineBuilder::new()
    }

    /// Submits a single task and returns its identifier.
    ///
    /// # Errors
    ///
    /// Returns [`TaskEngineError`] if the engine is shutting down, the queue is
    /// full, a dependency is unknown, or the task is invalid.
    pub fn submit(&self, definition: TaskDefinition) -> Result<TaskId, TaskEngineError> {
        let id = self.inner.next_task_id();
        for dependency in &definition.dependencies {
            if !self.inner.store.contains(dependency) {
                return Err(TaskEngineError::DependencyNotFound {
                    task: id,
                    dependency: dependency.clone(),
                });
            }
        }
        self.inner.check_capacity(1)?;

        let record = self.inner.build_record(id.clone(), definition, None);
        let (task, change) = self.inner.store.insert_task(record)?;

        self.inner.publish(TaskEvent::TaskCreated {
            task: Box::new(task),
        });
        if let Some(change) = change {
            self.inner.publish_status_change(&change);
        }
        self.inner.notify.notify_one();
        Ok(id)
    }

    /// Submits a workflow and returns its identifier.
    ///
    /// # Errors
    ///
    /// Returns [`TaskEngineError`] if the workflow is empty, malformed, cyclic,
    /// or the engine is shutting down or full.
    pub fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskEngineError> {
        if definition.steps.is_empty() {
            return Err(TaskEngineError::InvalidWorkflow {
                message: "workflow must have at least one step".to_owned(),
            });
        }
        self.inner.check_capacity(definition.steps.len())?;

        let workflow_id = self.inner.next_workflow_id();
        let workflow = Workflow {
            id: workflow_id.clone(),
            kind: definition.kind.clone(),
            steps: definition.steps.clone(),
        };
        // The domain validates uniqueness, reference resolution, and acyclicity
        // of *step* prerequisites before the engine maps them to task ids.
        workflow.validate().map_err(map_workflow_validation)?;

        let mut step_to_task: std::collections::BTreeMap<StepId, TaskId> =
            std::collections::BTreeMap::new();
        for step in &workflow.steps {
            step_to_task.insert(step.id.clone(), self.inner.next_task_id());
        }

        let mut records = Vec::with_capacity(workflow.steps.len());
        let mut edges = std::collections::BTreeMap::new();
        for step in &workflow.steps {
            let task_id = step_to_task[&step.id].clone();
            let mut dependencies = Vec::with_capacity(step.depends_on.len());
            for prerequisite in &step.depends_on {
                if let Some(task) = step_to_task.get(prerequisite) {
                    dependencies.push(task.clone());
                }
            }
            edges.insert(task_id.clone(), dependencies.clone());

            let definition = TaskDefinition {
                kind: step.task.clone(),
                target: TaskTarget::Workflow(workflow_id.clone()),
                dependencies,
                timeout_seconds: None,
                description: step.description.clone(),
                retry: crate::retry::RetryPolicy::none(),
                dependency_policy: DependencyPolicy::AllSucceeded,
            };
            records.push(
                self.inner
                    .build_record(task_id, definition, Some(workflow_id.clone())),
            );
        }

        let nodes: Vec<TaskId> = records
            .iter()
            .map(|record| record.task.id.clone())
            .collect();
        let exists = |id: &TaskId| self.inner.store.contains(id);
        crate::graph::validate(&nodes, &edges, &exists)?;

        let task_ids = nodes.clone();
        let mut initial_changes = Vec::new();
        for record in records {
            let (_task, change) = self.inner.store.insert_task(record)?;
            if let Some(change) = change {
                initial_changes.push(change);
            }
        }
        self.inner.store.insert_workflow(workflow.clone(), task_ids);
        self.inner.publish(TaskEvent::WorkflowCreated {
            workflow: Box::new(workflow),
        });

        // Publish a created event for every task, then any initial blocked or
        // skipped transitions.
        for task in self.inner.store.list() {
            if task.workflow.as_ref() == Some(&workflow_id) {
                self.inner.publish(TaskEvent::TaskCreated {
                    task: Box::new(task),
                });
            }
        }
        for change in initial_changes {
            self.inner.publish_status_change(&change);
        }
        self.inner.notify.notify_one();
        Ok(workflow_id)
    }

    /// Returns a snapshot of a task.
    ///
    /// # Errors
    ///
    /// Returns [`TaskEngineError::TaskNotFound`] if the task does not exist.
    pub fn get_task(&self, id: &TaskId) -> Result<Task, TaskEngineError> {
        self.inner
            .store
            .task(id)
            .ok_or_else(|| TaskEngineError::TaskNotFound(id.clone()))
    }

    /// Returns a workflow.
    ///
    /// # Errors
    ///
    /// Returns [`TaskEngineError::WorkflowNotFound`] if it does not exist.
    pub fn get_workflow(&self, id: &WorkflowId) -> Result<Workflow, TaskEngineError> {
        self.inner
            .store
            .workflow(id)
            .ok_or_else(|| TaskEngineError::WorkflowNotFound(id.clone()))
    }

    /// Lists every task, oldest first.
    #[must_use]
    pub fn list_tasks(&self) -> Vec<Task> {
        self.inner.store.list()
    }

    /// Lists the tasks that are currently running.
    #[must_use]
    pub fn list_running_tasks(&self) -> Vec<Task> {
        self.inner.store.running()
    }

    /// Lists up to `limit` most recently finished tasks, newest first.
    #[must_use]
    pub fn list_recent_tasks(&self, limit: usize) -> Vec<Task> {
        self.inner.store.recent(limit)
    }

    /// Subscribes to task events.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.inner.events.subscribe()
    }

    /// Requests cancellation of a task.
    ///
    /// Cancelling an already-terminal task is a no-op and returns `Ok`.
    ///
    /// # Errors
    ///
    /// Returns [`TaskEngineError::TaskNotFound`] if the task does not exist.
    pub fn cancel(&self, id: &TaskId) -> Result<(), TaskEngineError> {
        let task = self
            .inner
            .store
            .task(id)
            .ok_or_else(|| TaskEngineError::TaskNotFound(id.clone()))?;
        if task.status.is_terminal() {
            return Ok(());
        }
        self.inner.store.signal_cancel(id)?;
        self.inner.notify.notify_one();
        Ok(())
    }

    /// Waits until `id` reaches a terminal state and returns its snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`TaskEngineError::TaskNotFound`] if the task does not exist.
    pub async fn wait_for(&self, id: &TaskId) -> Result<Task, TaskEngineError> {
        loop {
            let (task, mut revision) = self
                .inner
                .store
                .task_and_revision(id)
                .ok_or_else(|| TaskEngineError::TaskNotFound(id.clone()))?;
            if task.status.is_terminal() {
                return Ok(task);
            }
            if revision.changed().await.is_err() {
                // The store was dropped; re-read once and return.
                return self.get_task(id);
            }
        }
    }

    /// Returns `true` once shutdown has begun.
    #[must_use]
    pub fn is_shutdown(&self) -> bool {
        self.inner.is_shutdown()
    }

    /// Gracefully shuts the engine down.
    ///
    /// New submissions are rejected, the scheduler stops, running tasks are
    /// asked to cancel cooperatively, and this waits (bounded) for them to
    /// finish. Executors that ignore cancellation are not force-killed.
    pub async fn shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        self.inner.scheduler_cancel.cancel();
        self.inner.notify.notify_one();

        for task in self.inner.store.running() {
            let _ = self.inner.store.signal_cancel(&task.id);
        }

        let permits = self.inner.config.max_parallel_tasks as u32;
        let wait = self.inner.semaphore.acquire_many(permits);
        let _ = tokio::time::timeout(Duration::from_secs(5), wait).await;
    }
}

impl std::fmt::Debug for TaskEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskEngine")
            .field("config", &self.inner.config)
            .field("tasks", &self.inner.store.list().len())
            .field("shutdown", &self.is_shutdown())
            .finish()
    }
}

impl TaskManager for TaskEngine {
    fn submit(&self, definition: TaskDefinition) -> Result<TaskId, TaskManagerError> {
        TaskEngine::submit(self, definition).map_err(Into::into)
    }

    fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskManagerError> {
        TaskEngine::submit_workflow(self, definition).map_err(Into::into)
    }

    fn task(&self, id: &TaskId) -> Result<Task, TaskManagerError> {
        TaskEngine::get_task(self, id).map_err(Into::into)
    }

    fn workflow(&self, id: &WorkflowId) -> Result<Workflow, TaskManagerError> {
        TaskEngine::get_workflow(self, id).map_err(Into::into)
    }

    fn cancel(&self, id: &TaskId) -> Result<(), TaskManagerError> {
        TaskEngine::cancel(self, id).map_err(Into::into)
    }
}

impl Drop for TaskEngine {
    fn drop(&mut self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        self.inner.scheduler_cancel.cancel();
        self.inner.notify.notify_one();
        for task in self.inner.store.running() {
            let _ = self.inner.store.signal_cancel(&task.id);
        }
    }
}

fn map_workflow_validation(error: service_orchestrator_domain::DomainError) -> TaskEngineError {
    use service_orchestrator_domain::DomainError;
    match error {
        DomainError::DependencyCycle { nodes } => TaskEngineError::DependencyCycle { nodes },
        other => TaskEngineError::InvalidWorkflow {
            message: other.to_string(),
        },
    }
}
