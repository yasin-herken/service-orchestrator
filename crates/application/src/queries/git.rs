//! Read-only Git queries.
//!
//! These queries inspect a repository through the [`GitManager`] and never
//! mutate it. They are `async` because they reach the process manager through
//! the port; the mutating Git operations are commands that return a task id
//! instead (see [`crate::commands`]).
//!
//! [`GitManager`]: crate::ports::GitManager

use service_orchestrator_domain::{BranchInfo, RepoDiff, RepositoryStatus, ServiceId, WorkflowId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::BulkOperationSummary;
use crate::ports::{DiffOptions, GitError, GitOptions};

/// Maps a structured [`GitError`] into an application error.
pub(crate) fn map_git_error(error: GitError) -> ApplicationError {
    ApplicationError::Git {
        code: error.code().to_owned(),
        message: error.to_string(),
    }
}

impl Application {
    /// Inspects a service's repository status.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, [`ApplicationError::InvalidOperation`] if it has no repository, or
    /// [`ApplicationError::Git`] if the repository cannot be inspected.
    pub async fn get_git_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<RepositoryStatus, ApplicationError> {
        self.authorize(service_orchestrator_domain::Permission::Read)?;
        let repository = self.resolve_service_repository(service_id)?;
        let options = GitOptions::new().with_timeout(self.git_read_timeout());
        self.git()
            .status(&repository, &options)
            .await
            .map_err(map_git_error)
    }

    /// Lists the local and remote-tracking branches of a service's repository.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, [`ApplicationError::InvalidOperation`] if it has no repository, or
    /// [`ApplicationError::Git`] if the repository cannot be inspected.
    pub async fn list_branches(
        &self,
        service_id: &ServiceId,
    ) -> Result<Vec<BranchInfo>, ApplicationError> {
        self.authorize(service_orchestrator_domain::Permission::Read)?;
        let repository = self.resolve_service_repository(service_id)?;
        let options = GitOptions::new().with_timeout(self.git_read_timeout());
        self.git()
            .branches(&repository, &options)
            .await
            .map_err(map_git_error)
    }

    /// Returns a bounded diff of a service's repository.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, [`ApplicationError::InvalidOperation`] if it has no repository, or
    /// [`ApplicationError::Git`] if the repository cannot be inspected.
    pub async fn get_repository_diff(
        &self,
        service_id: &ServiceId,
        options: &DiffOptions,
    ) -> Result<RepoDiff, ApplicationError> {
        self.authorize(service_orchestrator_domain::Permission::Read)?;
        let repository = self.resolve_service_repository(service_id)?;
        self.git()
            .diff(&repository, options)
            .await
            .map_err(map_git_error)
    }

    /// Aggregates the tasks of a workflow into a bulk-operation summary.
    ///
    /// Interfaces call this after a multi-repository command to report how many
    /// repositories succeeded, failed, were cancelled, or are still running.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkflowNotFound`] if the workflow does not
    /// exist.
    pub fn summarize_workflow(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<BulkOperationSummary, ApplicationError> {
        let tasks = self
            .tasks()
            .workflow_tasks(workflow_id)
            .map_err(|_| ApplicationError::WorkflowNotFound(workflow_id.clone()))?;
        Ok(BulkOperationSummary::from_tasks(&tasks))
    }

    /// The timeout applied to read-only Git queries.
    fn git_read_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.default_timeout_seconds())
    }
}
