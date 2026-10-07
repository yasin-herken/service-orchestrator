//! The task model.
//!
//! Every meaningful operation in the platform is represented as a [`Task`].
//! Tasks are *data*: they describe what should happen and record what happened.
//! The execution engine (the `execution` crate) is responsible for actually
//! running them, scheduling them by dependency, and enforcing concurrency.
//!
//! This module therefore defines the task vocabulary and the legal state
//! machine, but performs no execution.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::ids::{LibraryId, ServiceId, TaskId, WorkflowId};
use crate::timestamp::Timestamp;

/// The kind of work a task performs.
///
/// This is the shared vocabulary used by service-level operations, workflow
/// steps, and MCP tool results. The `Custom` variant keeps it open to
/// project-specific tasks.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Validate configuration or preconditions.
    Validate,
    /// Synchronize a repository (fetch plus fast-forward).
    GitSync,
    /// Fetch remote refs.
    GitFetch,
    /// Check out a branch.
    GitCheckout,
    /// Pull the current branch.
    GitPull,
    /// Build a library.
    BuildLibrary,
    /// Run a Maven build.
    MavenBuild,
    /// Install Node dependencies.
    NpmInstall,
    /// Run a Node build.
    NpmBuild,
    /// Run database migrations.
    Liquibase,
    /// Build a service.
    BuildService,
    /// Start a service.
    StartService,
    /// Stop a service.
    StopService,
    /// Restart a service.
    RestartService,
    /// Run health checks.
    HealthCheck,
    /// A project-specific task.
    Custom(String),
}

impl TaskKind {
    /// Returns a short, human-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Validate => "validate",
            Self::GitSync => "git_sync",
            Self::GitFetch => "git_fetch",
            Self::GitCheckout => "git_checkout",
            Self::GitPull => "git_pull",
            Self::BuildLibrary => "build_library",
            Self::MavenBuild => "maven_build",
            Self::NpmInstall => "npm_install",
            Self::NpmBuild => "npm_build",
            Self::Liquibase => "liquibase",
            Self::BuildService => "build_service",
            Self::StartService => "start_service",
            Self::StopService => "stop_service",
            Self::RestartService => "restart_service",
            Self::HealthCheck => "health_check",
            Self::Custom(name) => name,
        }
    }
}

impl fmt::Display for TaskKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The lifecycle status of a task.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Waiting to be scheduled.
    #[default]
    Queued,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Succeeded,
    /// Finished with an error.
    Failed,
    /// Cancelled before completion.
    Cancelled,
    /// Deliberately not run.
    Skipped,
    /// Waiting on a dependency.
    Blocked,
}

impl TaskStatus {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
            Self::Blocked => "blocked",
        }
    }

    /// Returns `true` if the task has reached a state it cannot leave.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped
        )
    }

    /// Returns `true` if the task is either queued or running.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running | Self::Blocked)
    }

    /// Returns `true` if the transition `self -> next` is legal.
    ///
    /// The legal transitions are:
    ///
    /// ```text
    /// queued  -> running | blocked | cancelled | skipped
    /// blocked -> queued | running | failed | cancelled | skipped
    /// running -> succeeded | failed | cancelled
    /// terminal states never change
    /// ```
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Queued => matches!(
                next,
                Self::Running | Self::Blocked | Self::Cancelled | Self::Skipped
            ),
            Self::Blocked => matches!(
                next,
                Self::Queued | Self::Running | Self::Failed | Self::Cancelled | Self::Skipped
            ),
            Self::Running => matches!(next, Self::Succeeded | Self::Failed | Self::Cancelled),
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped => false,
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Progress information for a running task.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskProgress {
    /// The number of completed units of work.
    pub current: u32,
    /// The total number of units, if known.
    pub total: Option<u32>,
    /// An optional human-readable progress message.
    pub message: Option<String>,
}

impl TaskProgress {
    /// Creates progress information.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if `current` exceeds `total`.
    pub fn new(
        current: u32,
        total: Option<u32>,
        message: Option<String>,
    ) -> Result<Self, DomainError> {
        if let Some(total) = total {
            if current > total {
                return Err(DomainError::validation(
                    "task progress",
                    format!("current ({current}) cannot exceed total ({total})"),
                ));
            }
        }
        Ok(Self {
            current,
            total,
            message,
        })
    }

    /// Returns the completion fraction in `0.0..=1.0`, if the total is known
    /// and non-zero.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total?;
        if total == 0 {
            return None;
        }
        Some(f64::from(self.current) / f64::from(total))
    }
}

/// A structured task failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskFailure {
    /// A stable, machine-readable error code, for example `MAVEN_BUILD_FAILED`.
    pub code: String,
    /// A human-readable summary.
    pub message: String,
    /// Optional additional detail, such as the process exit code.
    pub detail: Option<String>,
}

impl TaskFailure {
    /// Creates a task failure.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the code or message is empty.
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        detail: Option<String>,
    ) -> Result<Self, DomainError> {
        let code = code.into();
        let message = message.into();
        if code.trim().is_empty() {
            return Err(DomainError::validation(
                "task failure",
                "code must not be empty",
            ));
        }
        if message.trim().is_empty() {
            return Err(DomainError::validation(
                "task failure",
                "message must not be empty",
            ));
        }
        Ok(Self {
            code,
            message,
            detail,
        })
    }
}

/// A unit of work with a lifecycle and observable progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// The stable identifier of the task.
    pub id: TaskId,
    /// The kind of work the task performs.
    pub kind: TaskKind,
    /// The current lifecycle status.
    pub status: TaskStatus,
    /// The service the task acts on, if any.
    #[serde(default)]
    pub service: Option<ServiceId>,
    /// The library the task acts on, if any.
    #[serde(default)]
    pub library: Option<LibraryId>,
    /// The workflow the task belongs to, if any.
    #[serde(default)]
    pub workflow: Option<WorkflowId>,
    /// When the task was created.
    pub created_at: Timestamp,
    /// When the task started running, if it has.
    pub started_at: Option<Timestamp>,
    /// When the task reached a terminal state, if it has.
    pub finished_at: Option<Timestamp>,
    /// The task's progress.
    pub progress: TaskProgress,
    /// The task's failure, if it failed.
    pub error: Option<TaskFailure>,
    /// Tasks that must complete before this one may run.
    pub dependencies: Vec<TaskId>,
}

impl Task {
    /// Creates a queued task.
    #[must_use]
    pub fn new(id: TaskId, kind: TaskKind) -> Self {
        Self {
            id,
            kind,
            status: TaskStatus::Queued,
            service: None,
            library: None,
            workflow: None,
            created_at: Timestamp::now(),
            started_at: None,
            finished_at: None,
            progress: TaskProgress::default(),
            error: None,
            dependencies: Vec::new(),
        }
    }

    /// Associates the task with a service.
    #[must_use]
    pub fn with_service(mut self, service: ServiceId) -> Self {
        self.service = Some(service);
        self
    }

    /// Associates the task with a library.
    #[must_use]
    pub fn with_library(mut self, library: LibraryId) -> Self {
        self.library = Some(library);
        self
    }

    /// Associates the task with a workflow.
    #[must_use]
    pub fn with_workflow(mut self, workflow: WorkflowId) -> Self {
        self.workflow = Some(workflow);
        self
    }

    /// Adds a dependency on another task.
    #[must_use]
    pub fn depends_on(mut self, task: TaskId) -> Self {
        self.dependencies.push(task);
        self
    }

    /// Sets the task's progress.
    pub fn set_progress(&mut self, progress: TaskProgress) {
        self.progress = progress;
    }

    /// Attempts to move the task to `next`.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::InvalidTaskTransition`] if the transition is
    /// not legal. Timestamps are recorded automatically for the `running` and
    /// terminal states.
    pub fn transition(&mut self, next: TaskStatus) -> Result<(), DomainError> {
        if !self.status.can_transition_to(next) {
            return Err(DomainError::InvalidTaskTransition {
                id: self.id.to_string(),
                from: self.status.label(),
                to: next.label(),
            });
        }
        self.status = next;
        match next {
            TaskStatus::Running => {
                if self.started_at.is_none() {
                    self.started_at = Some(Timestamp::now());
                }
            }
            TaskStatus::Succeeded
            | TaskStatus::Failed
            | TaskStatus::Cancelled
            | TaskStatus::Skipped => {
                self.finished_at = Some(Timestamp::now());
            }
            TaskStatus::Queued | TaskStatus::Blocked => {}
        }
        Ok(())
    }

    /// Marks the task as failed with a structured error.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::InvalidTaskTransition`] if the task is not
    /// currently running.
    pub fn fail(&mut self, failure: TaskFailure) -> Result<(), DomainError> {
        self.transition(TaskStatus::Failed)?;
        self.error = Some(failure);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task::new(TaskId::new("task-1").unwrap(), TaskKind::BuildService)
    }

    #[test]
    fn new_tasks_are_queued() {
        let task = task();
        assert_eq!(task.status, TaskStatus::Queued);
        assert!(task.started_at.is_none());
        assert!(task.finished_at.is_none());
        assert!(task.library.is_none());
    }

    #[test]
    fn tasks_can_target_a_library() {
        let task = Task::new(TaskId::new("task-2").unwrap(), TaskKind::BuildLibrary)
            .with_library(LibraryId::new("common-core").unwrap());
        assert_eq!(
            task.library.as_ref().map(LibraryId::as_str),
            Some("common-core")
        );
        assert!(task.service.is_none());
    }

    #[test]
    fn running_records_start_time() {
        let mut task = task();
        task.transition(TaskStatus::Running).unwrap();
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.started_at.is_some());
    }

    #[test]
    fn success_records_finish_time() {
        let mut task = task();
        task.transition(TaskStatus::Running).unwrap();
        task.transition(TaskStatus::Succeeded).unwrap();
        assert!(task.finished_at.is_some());
        assert!(task.status.is_terminal());
    }

    #[test]
    fn rejects_illegal_transitions() {
        let mut task = task();
        let error = task.transition(TaskStatus::Succeeded).unwrap_err();
        assert!(matches!(error, DomainError::InvalidTaskTransition { .. }));
    }

    #[test]
    fn terminal_tasks_cannot_transition() {
        let mut task = task();
        task.transition(TaskStatus::Running).unwrap();
        task.transition(TaskStatus::Succeeded).unwrap();
        assert!(task.transition(TaskStatus::Running).is_err());
        assert!(task.transition(TaskStatus::Failed).is_err());
    }

    #[test]
    fn blocked_task_can_return_to_queued() {
        let mut task = task();
        task.transition(TaskStatus::Blocked).unwrap();
        task.transition(TaskStatus::Queued).unwrap();
        assert_eq!(task.status, TaskStatus::Queued);
    }

    #[test]
    fn failure_stores_structured_error() {
        let mut task = task();
        task.transition(TaskStatus::Running).unwrap();
        task.fail(
            TaskFailure::new(
                "MAVEN_BUILD_FAILED",
                "Maven build failed",
                Some("exit 1".into()),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(task.status, TaskStatus::Failed);
        assert_eq!(task.error.as_ref().unwrap().code, "MAVEN_BUILD_FAILED");
    }

    #[test]
    fn progress_reports_fraction() {
        let progress = TaskProgress::new(2, Some(4), None).unwrap();
        assert_eq!(progress.fraction(), Some(0.5));
        let unknown = TaskProgress::new(2, None, None).unwrap();
        assert_eq!(unknown.fraction(), None);
    }

    #[test]
    fn progress_rejects_current_beyond_total() {
        assert!(TaskProgress::new(5, Some(4), None).is_err());
    }

    #[test]
    fn task_failure_requires_code_and_message() {
        assert!(TaskFailure::new("", "boom", None).is_err());
        assert!(TaskFailure::new("CODE", "  ", None).is_err());
    }

    #[test]
    fn transition_rules_match_documented_machine() {
        assert!(TaskStatus::Queued.can_transition_to(TaskStatus::Running));
        assert!(TaskStatus::Queued.can_transition_to(TaskStatus::Blocked));
        assert!(!TaskStatus::Queued.can_transition_to(TaskStatus::Succeeded));
        assert!(TaskStatus::Running.can_transition_to(TaskStatus::Failed));
        assert!(!TaskStatus::Succeeded.can_transition_to(TaskStatus::Running));
    }
}
