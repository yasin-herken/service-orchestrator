//! The in-memory task store.
//!
//! The store is the engine's runtime registry: it owns every task's state, the
//! dependency index, and the workflows that have been submitted. It is a plain
//! in-memory structure guarded by a single `std::sync::Mutex`; there is no
//! database and no persistence. The MVP deliberately keeps completed tasks so
//! they remain queryable ("recent tasks"); eviction/retention policy is future
//! work.
//!
//! # Locking
//!
//! Short critical sections only. The lock is never held across an `.await`, and
//! the engine clones everything it needs out of the store before spawning work.
//! A separate [`tokio::sync::watch`] revision counter lets waiters observe
//! status changes without polling.
//!
//! # State transitions
//!
//! The store defers to the domain [`Task`] state machine for every transition,
//! so an illegal transition is rejected here exactly as it would be anywhere
//! else. Its only scheduling decision is the initial status of a task
//! (`queued`, `blocked`, or `skipped`) and the state of dependent tasks once a
//! dependency settles.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use service_orchestrator_domain::{
    Task, TaskFailure, TaskId, TaskKind, TaskProgress, TaskStatus, Workflow, WorkflowId,
};
use tokio::sync::watch;

use crate::cancellation::Cancellation;
use crate::error::TaskEngineError;
use crate::retry::RetryPolicy;
use crate::task_manager::{DependencyPolicy, TaskTarget};

/// The internal record for one task.
pub(crate) struct TaskRecord {
    pub(crate) task: Task,
    pub(crate) target: TaskTarget,
    pub(crate) retry: RetryPolicy,
    pub(crate) dependency_policy: DependencyPolicy,
    pub(crate) timeout: Option<Duration>,
    pub(crate) attempt: u32,
    pub(crate) sequence: u64,
    pub(crate) cancellation: Cancellation,
}

/// A snapshot of everything needed to run a task attempt.
pub(crate) struct ExecutionSpec {
    pub(crate) id: TaskId,
    pub(crate) kind: TaskKind,
    pub(crate) target: TaskTarget,
    pub(crate) retry: RetryPolicy,
    pub(crate) timeout: Option<Duration>,
    pub(crate) cancellation: Cancellation,
}

/// A recorded status change, used to emit events after a store mutation.
#[derive(Clone, Debug)]
pub(crate) struct StatusChange {
    pub(crate) id: TaskId,
    pub(crate) to: TaskStatus,
    pub(crate) failure: Option<TaskFailure>,
}

struct Inner {
    tasks: BTreeMap<TaskId, TaskRecord>,
    order: BTreeMap<u64, TaskId>,
    dependents: BTreeMap<TaskId, Vec<TaskId>>,
    workflows: BTreeMap<WorkflowId, Workflow>,
    workflow_tasks: BTreeMap<WorkflowId, Vec<TaskId>>,
    next_sequence: u64,
    revision: watch::Sender<u64>,
}

/// The engine's in-memory task and workflow registry.
pub struct TaskStore {
    inner: Mutex<Inner>,
}

impl TaskStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        let (revision, _receiver) = watch::channel(0);
        Self {
            inner: Mutex::new(Inner {
                tasks: BTreeMap::new(),
                order: BTreeMap::new(),
                dependents: BTreeMap::new(),
                workflows: BTreeMap::new(),
                workflow_tasks: BTreeMap::new(),
                next_sequence: 0,
                revision,
            }),
        }
    }

    /// Returns a snapshot of a task.
    #[must_use]
    pub fn task(&self, id: &TaskId) -> Option<Task> {
        self.inner
            .lock()
            .expect("task store mutex poisoned")
            .tasks
            .get(id)
            .map(|record| record.task.clone())
    }

    /// Returns a snapshot of a task together with a revision subscriber.
    ///
    /// Reading the task and subscribing to the revision happen under one lock,
    /// so a waiter that observes a non-terminal task is guaranteed to be
    /// notified of the next change.
    pub(crate) fn task_and_revision(&self, id: &TaskId) -> Option<(Task, watch::Receiver<u64>)> {
        let inner = self.inner.lock().expect("task store mutex poisoned");
        let task = inner.tasks.get(id)?.task.clone();
        Some((task, inner.revision.subscribe()))
    }

    /// Returns a workflow.
    #[must_use]
    pub fn workflow(&self, id: &WorkflowId) -> Option<Workflow> {
        self.inner
            .lock()
            .expect("task store mutex poisoned")
            .workflows
            .get(id)
            .cloned()
    }

    /// Returns `true` if a task with `id` exists.
    #[must_use]
    pub fn contains(&self, id: &TaskId) -> bool {
        self.inner
            .lock()
            .expect("task store mutex poisoned")
            .tasks
            .contains_key(id)
    }

    /// Returns every task, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<Task> {
        let inner = self.inner.lock().expect("task store mutex poisoned");
        inner
            .order
            .values()
            .filter_map(|id| inner.tasks.get(id))
            .map(|record| record.task.clone())
            .collect()
    }

    /// Returns every task that is currently running.
    #[must_use]
    pub fn running(&self) -> Vec<Task> {
        self.list()
            .into_iter()
            .filter(|task| task.status == TaskStatus::Running)
            .collect()
    }

    /// Returns up to `limit` most recently finished tasks, newest first.
    #[must_use]
    pub fn recent(&self, limit: usize) -> Vec<Task> {
        let inner = self.inner.lock().expect("task store mutex poisoned");
        inner
            .order
            .values()
            .rev()
            .filter_map(|id| inner.tasks.get(id))
            .filter(|record| record.task.status.is_terminal())
            .take(limit)
            .map(|record| record.task.clone())
            .collect()
    }

    /// Returns the ids of tasks that directly depend on `id`.
    #[must_use]
    pub fn dependents_of(&self, id: &TaskId) -> Vec<TaskId> {
        self.inner
            .lock()
            .expect("task store mutex poisoned")
            .dependents
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns the number of non-terminal tasks.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.inner
            .lock()
            .expect("task store mutex poisoned")
            .tasks
            .values()
            .filter(|record| record.task.status.is_active())
            .count()
    }

    // -- mutation ------------------------------------------------------------

    /// Inserts a task and computes its initial status.
    ///
    /// A task with no dependencies starts `queued`. A task whose dependencies
    /// are still pending starts `blocked`. A task whose dependencies have all
    /// settled but do not satisfy its policy starts `skipped`.
    pub(crate) fn insert_task(
        &self,
        mut record: TaskRecord,
    ) -> Result<(Task, Option<StatusChange>), TaskEngineError> {
        let mut inner = self.inner.lock().expect("task store mutex poisoned");
        if inner.tasks.contains_key(&record.task.id) {
            return Err(TaskEngineError::DuplicateTask(record.task.id));
        }

        record.sequence = inner.next_sequence;
        inner.next_sequence += 1;

        let id = record.task.id.clone();
        let dependencies = record.task.dependencies.clone();
        for dependency in &dependencies {
            inner
                .dependents
                .entry(dependency.clone())
                .or_default()
                .push(id.clone());
        }

        let initial = initial_status(&record, &inner.tasks);
        if initial != TaskStatus::Queued {
            record.task.transition(initial)?;
        }

        let snapshot = record.task.clone();
        inner.order.insert(record.sequence, id.clone());
        inner.tasks.insert(id.clone(), record);
        bump_revision(&mut inner);

        let change = if initial == TaskStatus::Queued {
            None
        } else {
            Some(StatusChange {
                id,
                to: initial,
                failure: None,
            })
        };
        Ok((snapshot, change))
    }

    /// Inserts a workflow and its task ids.
    pub(crate) fn insert_workflow(&self, workflow: Workflow, task_ids: Vec<TaskId>) {
        let mut inner = self.inner.lock().expect("task store mutex poisoned");
        inner.workflow_tasks.insert(workflow.id.clone(), task_ids);
        inner.workflows.insert(workflow.id.clone(), workflow);
    }

    /// Transitions a task to `next`.
    pub(crate) fn transition(
        &self,
        id: &TaskId,
        next: TaskStatus,
    ) -> Result<StatusChange, TaskEngineError> {
        self.mutate(id, |task| {
            task.transition(next)?;
            Ok(StatusChange {
                id: id.clone(),
                to: next,
                failure: None,
            })
        })
    }

    /// Transitions a task to `failed` with a structured failure.
    pub(crate) fn fail(
        &self,
        id: &TaskId,
        failure: TaskFailure,
    ) -> Result<StatusChange, TaskEngineError> {
        self.mutate(id, |task| {
            task.fail(failure.clone())?;
            Ok(StatusChange {
                id: id.clone(),
                to: TaskStatus::Failed,
                failure: Some(failure),
            })
        })
    }

    /// Records progress for a task.
    pub(crate) fn set_progress(
        &self,
        id: &TaskId,
        progress: TaskProgress,
    ) -> Result<(), TaskEngineError> {
        self.mutate(id, |task| {
            task.set_progress(progress);
            Ok(())
        })
    }

    /// Records a message for a task without changing its progress counters.
    pub(crate) fn set_message(&self, id: &TaskId, message: String) -> Result<(), TaskEngineError> {
        self.mutate(id, |task| {
            task.progress.message = Some(message);
            Ok(())
        })
    }

    /// Increments and returns the attempt counter for a task.
    ///
    /// The attempt counter is engine bookkeeping rather than domain state, so
    /// it lives on the record instead of the [`Task`].
    pub(crate) fn bump_attempt(&self, id: &TaskId) -> Result<u32, TaskEngineError> {
        let mut inner = self.inner.lock().expect("task store mutex poisoned");
        let record = inner
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskEngineError::TaskNotFound(id.clone()))?;
        record.attempt += 1;
        Ok(record.attempt)
    }

    /// Returns an execution snapshot for a task.
    pub(crate) fn execution_spec(&self, id: &TaskId) -> Option<ExecutionSpec> {
        let inner = self.inner.lock().expect("task store mutex poisoned");
        let record = inner.tasks.get(id)?;
        Some(ExecutionSpec {
            id: id.clone(),
            kind: record.task.kind.clone(),
            target: record.target.clone(),
            retry: record.retry,
            timeout: record.timeout,
            cancellation: record.cancellation.clone(),
        })
    }

    /// Cancels a task's cancellation token.
    pub(crate) fn signal_cancel(&self, id: &TaskId) -> Result<(), TaskEngineError> {
        let inner = self.inner.lock().expect("task store mutex poisoned");
        let record = inner
            .tasks
            .get(id)
            .ok_or_else(|| TaskEngineError::TaskNotFound(id.clone()))?;
        record.cancellation.cancel();
        Ok(())
    }

    /// Returns the next task that is queued and whose dependencies are met.
    ///
    /// Selection is FIFO by insertion sequence, which keeps scheduling
    /// predictable and starvation-free for the MVP.
    #[must_use]
    pub(crate) fn next_ready(&self) -> Option<TaskId> {
        let inner = self.inner.lock().expect("task store mutex poisoned");
        for id in inner.order.values() {
            let record = match inner.tasks.get(id) {
                Some(record) => record,
                None => continue,
            };
            if record.task.status == TaskStatus::Queued
                && dependencies_satisfied(record, &inner.tasks)
            {
                return Some(id.clone());
            }
        }
        None
    }

    /// Settles tasks that depend on `completed` now that it is terminal.
    ///
    /// Returns the transitions applied, so the engine can emit events and wake
    /// the scheduler.
    pub(crate) fn settle_dependents(&self, completed: &TaskId) -> Vec<StatusChange> {
        let mut inner = self.inner.lock().expect("task store mutex poisoned");
        let dependents = inner.dependents.get(completed).cloned().unwrap_or_default();
        let mut changes = Vec::new();

        for dependent in dependents {
            let next = {
                let record = match inner.tasks.get(&dependent) {
                    Some(record) => record,
                    None => continue,
                };
                if record.task.status != TaskStatus::Blocked {
                    continue;
                }
                if !all_dependencies_terminal(record, &inner.tasks) {
                    continue;
                }
                if dependencies_satisfied(record, &inner.tasks) {
                    TaskStatus::Queued
                } else {
                    TaskStatus::Skipped
                }
            };

            if let Some(record) = inner.tasks.get_mut(&dependent) {
                if let Err(error) = record.task.transition(next) {
                    // A rejected transition here would indicate a logic error;
                    // surface it as an internal failure rather than panicking.
                    return vec![StatusChange {
                        id: dependent,
                        to: next,
                        failure: Some(TaskFailure {
                            code: "INVALID_TASK_TRANSITION".to_owned(),
                            message: error.to_string(),
                            detail: None,
                        }),
                    }];
                }
            }
            changes.push(StatusChange {
                id: dependent,
                to: next,
                failure: None,
            });
        }

        if !changes.is_empty() {
            bump_revision(&mut inner);
        }
        changes
    }
}

impl Default for TaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TaskStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().expect("task store mutex poisoned");
        f.debug_struct("TaskStore")
            .field("tasks", &inner.tasks.len())
            .field("workflows", &inner.workflows.len())
            .finish()
    }
}

fn bump_revision(inner: &mut Inner) {
    let next = inner.revision.borrow().wrapping_add(1);
    inner.revision.send_replace(next);
}

fn initial_status(record: &TaskRecord, tasks: &BTreeMap<TaskId, TaskRecord>) -> TaskStatus {
    if record.task.dependencies.is_empty() {
        return TaskStatus::Queued;
    }
    if !all_dependencies_terminal(record, tasks) {
        return TaskStatus::Blocked;
    }
    if dependencies_satisfied(record, tasks) {
        TaskStatus::Queued
    } else {
        TaskStatus::Skipped
    }
}

fn all_dependencies_terminal(record: &TaskRecord, tasks: &BTreeMap<TaskId, TaskRecord>) -> bool {
    record.task.dependencies.iter().all(|dependency| {
        tasks
            .get(dependency)
            .is_some_and(|task| task.task.status.is_terminal())
    })
}

fn dependencies_satisfied(record: &TaskRecord, tasks: &BTreeMap<TaskId, TaskRecord>) -> bool {
    match record.dependency_policy {
        DependencyPolicy::AllSucceeded => record.task.dependencies.iter().all(|dependency| {
            tasks
                .get(dependency)
                .is_some_and(|task| task.task.status == TaskStatus::Succeeded)
        }),
        DependencyPolicy::AnyCompleted => record.task.dependencies.iter().all(|dependency| {
            tasks
                .get(dependency)
                .is_some_and(|task| task.task.status.is_terminal())
        }),
    }
}

impl TaskStore {
    fn mutate<T>(
        &self,
        id: &TaskId,
        f: impl FnOnce(&mut Task) -> Result<T, TaskEngineError>,
    ) -> Result<T, TaskEngineError> {
        let mut inner = self.inner.lock().expect("task store mutex poisoned");
        let record = inner
            .tasks
            .get_mut(id)
            .ok_or_else(|| TaskEngineError::TaskNotFound(id.clone()))?;
        let result = f(&mut record.task)?;
        bump_revision(&mut inner);
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, dependencies: Vec<TaskId>, policy: DependencyPolicy) -> TaskRecord {
        let id = TaskId::new(id).unwrap();
        let mut task = Task::new(id, TaskKind::GitFetch);
        task.dependencies = dependencies;
        TaskRecord {
            task,
            target: TaskTarget::None,
            retry: RetryPolicy::none(),
            dependency_policy: policy,
            timeout: None,
            attempt: 0,
            sequence: 0,
            cancellation: Cancellation::new(),
        }
    }

    fn tid(value: &str) -> TaskId {
        TaskId::new(value).unwrap()
    }

    #[test]
    fn inserts_and_reads_tasks() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        assert!(store.contains(&tid("a")));
        assert_eq!(store.task(&tid("a")).unwrap().status, TaskStatus::Queued);
        assert_eq!(store.list().len(), 1);
        assert!(store.task(&tid("missing")).is_none());
    }

    #[test]
    fn rejects_duplicate_ids() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        let error = store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap_err();
        assert!(matches!(error, TaskEngineError::DuplicateTask(_)));
    }

    #[test]
    fn tasks_with_pending_dependencies_start_blocked() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        store
            .insert_task(record("b", vec![tid("a")], DependencyPolicy::AllSucceeded))
            .unwrap();
        assert_eq!(store.task(&tid("b")).unwrap().status, TaskStatus::Blocked);
    }

    #[test]
    fn transitioning_updates_status_and_revision() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        let (_task, revision) = store.task_and_revision(&tid("a")).unwrap();
        store.transition(&tid("a"), TaskStatus::Running).unwrap();
        assert_eq!(store.task(&tid("a")).unwrap().status, TaskStatus::Running);
        assert!(revision.has_changed().unwrap());
    }

    #[test]
    fn rejects_illegal_transitions() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        let error = store
            .transition(&tid("a"), TaskStatus::Succeeded)
            .unwrap_err();
        assert!(matches!(error, TaskEngineError::InvalidTransition { .. }));
    }

    #[test]
    fn failing_records_a_structured_failure() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        store.transition(&tid("a"), TaskStatus::Running).unwrap();
        let failure = TaskFailure::new("BOOM", "it broke", None).unwrap();
        store.fail(&tid("a"), failure).unwrap();
        let task = store.task(&tid("a")).unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error.unwrap().code, "BOOM");
    }

    #[test]
    fn next_ready_respects_dependencies_and_fifo() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        store
            .insert_task(record("b", vec![tid("a")], DependencyPolicy::AllSucceeded))
            .unwrap();
        assert_eq!(store.next_ready(), Some(tid("a")));
        store.transition(&tid("a"), TaskStatus::Running).unwrap();
        assert_eq!(store.next_ready(), None);
        store.transition(&tid("a"), TaskStatus::Succeeded).unwrap();
        store.settle_dependents(&tid("a"));
        assert_eq!(store.task(&tid("b")).unwrap().status, TaskStatus::Queued);
        assert_eq!(store.next_ready(), Some(tid("b")));
    }

    #[test]
    fn skipped_dependency_skips_dependents_by_default() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        store
            .insert_task(record("b", vec![tid("a")], DependencyPolicy::AllSucceeded))
            .unwrap();
        store.transition(&tid("a"), TaskStatus::Running).unwrap();
        store
            .fail(&tid("a"), TaskFailure::new("BOOM", "failed", None).unwrap())
            .unwrap();
        let changes = store.settle_dependents(&tid("a"));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].to, TaskStatus::Skipped);
        assert_eq!(store.task(&tid("b")).unwrap().status, TaskStatus::Skipped);
    }

    #[test]
    fn any_completed_policy_runs_after_a_failure() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        store
            .insert_task(record("b", vec![tid("a")], DependencyPolicy::AnyCompleted))
            .unwrap();
        store.transition(&tid("a"), TaskStatus::Running).unwrap();
        store
            .fail(&tid("a"), TaskFailure::new("BOOM", "failed", None).unwrap())
            .unwrap();
        let changes = store.settle_dependents(&tid("a"));
        assert_eq!(changes[0].to, TaskStatus::Queued);
        assert_eq!(store.task(&tid("b")).unwrap().status, TaskStatus::Queued);
    }

    #[test]
    fn recent_returns_terminal_tasks_newest_first() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        store
            .insert_task(record("b", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        store.transition(&tid("a"), TaskStatus::Running).unwrap();
        store.transition(&tid("a"), TaskStatus::Succeeded).unwrap();
        let recent = store.recent(10);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, tid("a"));
    }

    #[test]
    fn attempt_counter_increments() {
        let store = TaskStore::new();
        store
            .insert_task(record("a", vec![], DependencyPolicy::AllSucceeded))
            .unwrap();
        assert_eq!(store.bump_attempt(&tid("a")).unwrap(), 1);
        assert_eq!(store.bump_attempt(&tid("a")).unwrap(), 2);
    }
}
