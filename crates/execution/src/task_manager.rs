//! The task engine contract used by the application layer.
//!
//! Every meaningful operation is a *task*, and every composed operation is a
//! *workflow*. The application layer does not execute work itself: it describes
//! the work as a [`TaskDefinition`] or [`WorkflowDefinition`] and submits it to
//! a [`TaskManager`]. A concrete engine — scheduling, bounded concurrency,
//! cancellation, retry, timeout, and progress reporting — implements the trait
//! in a later task.
//!
//! This module deliberately contains **no scheduling logic**. Its purpose is to
//! fix the boundary between the application layer (which requests work) and the
//! task engine (which executes it), so the application can be built and tested
//! with an in-memory fake long before the engine exists.
//!
//! # Why the contract lives in `execution`
//!
//! The accepted architecture (`docs/architecture.md`, ADR 001) makes
//! `execution` a core service that the application depends on, and the engine
//! depends only on the domain. Placing the contract here keeps that direction
//! intact: the engine owns its API, and the application consumes it. It also
//! means a future engine is implemented in the same crate, with no dependency
//! back-edge from execution to application.

use service_orchestrator_domain::{
    LibraryId, ServiceId, Task, TaskId, TaskKind, Workflow, WorkflowId, WorkflowKind, WorkflowStep,
};
use thiserror::Error;

/// What a task definition acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskTarget {
    /// The task acts on no particular entity.
    None,
    /// The task acts on a service.
    Service(ServiceId),
    /// The task acts on a library.
    Library(LibraryId),
    /// The task belongs to a workflow.
    Workflow(WorkflowId),
}

/// A request to run a single unit of work.
///
/// A `TaskDefinition` is the application's *intent*: it is submitted to a
/// [`TaskManager`], which assigns a [`TaskId`] and owns scheduling. It carries
/// only domain identifiers and plain data — never process handles, commands to
/// execute directly, or interface types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskDefinition {
    /// The kind of work to perform.
    pub kind: TaskKind,
    /// The entity the task acts on.
    pub target: TaskTarget,
    /// Tasks that must complete first.
    pub dependencies: Vec<TaskId>,
    /// An optional per-task timeout, in seconds.
    pub timeout_seconds: Option<u64>,
    /// An optional human-readable description.
    pub description: Option<String>,
}

impl TaskDefinition {
    /// Creates a task definition with no target or dependencies.
    #[must_use]
    pub fn new(kind: TaskKind) -> Self {
        Self {
            kind,
            target: TaskTarget::None,
            dependencies: Vec::new(),
            timeout_seconds: None,
            description: None,
        }
    }

    /// Sets the service the task acts on.
    #[must_use]
    pub fn for_service(mut self, service: ServiceId) -> Self {
        self.target = TaskTarget::Service(service);
        self
    }

    /// Sets the library the task acts on.
    #[must_use]
    pub fn for_library(mut self, library: LibraryId) -> Self {
        self.target = TaskTarget::Library(library);
        self
    }

    /// Associates the task with a workflow.
    #[must_use]
    pub fn in_workflow(mut self, workflow: WorkflowId) -> Self {
        self.target = TaskTarget::Workflow(workflow);
        self
    }

    /// Adds a dependency on another task.
    #[must_use]
    pub fn depends_on(mut self, task: TaskId) -> Self {
        self.dependencies.push(task);
        self
    }

    /// Sets a per-task timeout, in seconds.
    #[must_use]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout_seconds = Some(seconds);
        self
    }

    /// Sets a human-readable description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// A request to run a composed sequence of tasks.
///
/// The workflow's shape (steps and their prerequisites) is validated by the
/// domain before submission. The [`TaskManager`] assigns the [`WorkflowId`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowDefinition {
    /// The high-level purpose of the workflow.
    pub kind: WorkflowKind,
    /// The steps that make up the workflow.
    pub steps: Vec<WorkflowStep>,
}

impl WorkflowDefinition {
    /// Creates an empty workflow definition.
    #[must_use]
    pub fn new(kind: WorkflowKind) -> Self {
        Self {
            kind,
            steps: Vec::new(),
        }
    }

    /// Adds a step.
    #[must_use]
    pub fn with_step(mut self, step: WorkflowStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Returns the number of steps.
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Returns `true` if the workflow has no steps.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Errors returned by a [`TaskManager`].
///
/// These carry enough context for the application layer to map a failure into
/// its own structured error without leaking scheduler internals.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TaskManagerError {
    /// The referenced task or workflow does not exist.
    #[error("{entity} '{id}' was not found")]
    NotFound {
        /// The kind of entity that was looked up (`task` or `workflow`).
        entity: &'static str,
        /// The identifier that could not be resolved.
        id: String,
    },

    /// The engine refused to accept the submitted work.
    #[error("task was rejected: {reason}")]
    Rejected {
        /// Why the work was rejected.
        reason: String,
    },

    /// The engine failed for an internal reason.
    #[error("task engine error: {message}")]
    Internal {
        /// A human-readable description of the failure.
        message: String,
    },
}

/// The application-facing boundary of the task engine.
///
/// The application submits work and observes or cancels it; it never executes
/// anything itself. Implementations are supplied by the composition root once
/// the concrete engine exists.
pub trait TaskManager: Send + Sync {
    /// Submits a single task and returns its assigned identifier.
    ///
    /// # Errors
    ///
    /// Returns a [`TaskManagerError`] if the engine rejects the work.
    fn submit(&self, definition: TaskDefinition) -> Result<TaskId, TaskManagerError>;

    /// Submits a workflow and returns its assigned identifier.
    ///
    /// # Errors
    ///
    /// Returns a [`TaskManagerError`] if the engine rejects the workflow.
    fn submit_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskManagerError>;

    /// Returns a snapshot of a task.
    ///
    /// # Errors
    ///
    /// Returns [`TaskManagerError::NotFound`] if the task does not exist.
    fn task(&self, id: &TaskId) -> Result<Task, TaskManagerError>;

    /// Returns a workflow by identifier.
    ///
    /// # Errors
    ///
    /// Returns [`TaskManagerError::NotFound`] if the workflow does not exist.
    fn workflow(&self, id: &WorkflowId) -> Result<Workflow, TaskManagerError>;

    /// Requests cancellation of a task.
    ///
    /// # Errors
    ///
    /// Returns [`TaskManagerError::NotFound`] if the task does not exist.
    fn cancel(&self, id: &TaskId) -> Result<(), TaskManagerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_definitions_are_built_with_targets_and_dependencies() {
        let definition = TaskDefinition::new(TaskKind::BuildService)
            .for_service(ServiceId::new("auth-service").unwrap())
            .depends_on(TaskId::new("task-0").unwrap())
            .with_timeout(120)
            .with_description("build auth-service");
        assert_eq!(
            definition.target,
            TaskTarget::Service(ServiceId::new("auth-service").unwrap())
        );
        assert_eq!(definition.dependencies.len(), 1);
        assert_eq!(definition.timeout_seconds, Some(120));
        assert_eq!(
            definition.description.as_deref(),
            Some("build auth-service")
        );
    }

    #[test]
    fn library_targets_are_supported() {
        let definition = TaskDefinition::new(TaskKind::BuildLibrary)
            .for_library(LibraryId::new("common-core").unwrap());
        assert_eq!(
            definition.target,
            TaskTarget::Library(LibraryId::new("common-core").unwrap())
        );
    }

    #[test]
    fn workflow_definitions_accumulate_steps() {
        let definition =
            WorkflowDefinition::new(WorkflowKind::PrepareEnvironment).with_step(WorkflowStep::new(
                service_orchestrator_domain::StepId::new("sync").unwrap(),
                TaskKind::GitSync,
            ));
        assert_eq!(definition.len(), 1);
        assert!(!definition.is_empty());
    }
}
