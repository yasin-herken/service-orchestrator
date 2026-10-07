//! Task, workflow, and log queries.

use service_orchestrator_domain::{Task, TaskId, Workflow, WorkflowId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::ports::{LogEntry, LogRequest};

impl Application {
    /// Returns a task snapshot from the task engine.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::TaskNotFound`] if the task does not exist.
    pub fn get_task(&self, task_id: &TaskId) -> Result<Task, ApplicationError> {
        self.tasks()
            .task(task_id)
            .map_err(|_| ApplicationError::TaskNotFound(task_id.clone()))
    }

    /// Returns a workflow from the task engine.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkflowNotFound`] if it does not exist.
    pub fn get_workflow(&self, workflow_id: &WorkflowId) -> Result<Workflow, ApplicationError> {
        self.tasks()
            .workflow(workflow_id)
            .map_err(|_| ApplicationError::WorkflowNotFound(workflow_id.clone()))
    }

    /// Returns bounded log entries for a service.
    ///
    /// The service is validated first so an unknown service is reported as
    /// [`ApplicationError::ServiceNotFound`] rather than a log-service error.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or [`ApplicationError::LogService`] if log retrieval fails.
    pub fn get_logs(&self, request: &LogRequest) -> Result<Vec<LogEntry>, ApplicationError> {
        self.locate_service(&request.service_id)?;
        self.logs()
            .query(request)
            .map_err(|error| ApplicationError::LogService {
                message: error.to_string(),
            })
    }
}
