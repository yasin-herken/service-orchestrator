//! Health, diagnostics, and log queries.
//!
//! These queries are read-only: they combine validated configuration with the
//! last observed runtime state, the managed process index, the health index,
//! and stored logs. They never probe a process or a socket — refreshing health
//! is a command that schedules a task (`check_service_health`).

use service_orchestrator_domain::{ServiceId, TaskId, WorkspaceId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::{DiagnosticFailure, ServiceDiagnostics};
use crate::ports::{
    LogLevel, LogPage, LogRequest, LogServiceError, LogSubscription, ServiceHealth,
};

impl Application {
    /// Returns the last observed health of a service.
    ///
    /// A service that has never been checked reports
    /// [`HealthState::Unknown`] rather than an error.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist.
    pub fn get_service_health(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceHealth, ApplicationError> {
        let (workspace, service) = self.locate_service(service_id)?;
        Ok(self.health().service_health(service_id).unwrap_or_else(|| {
            ServiceHealth::unknown(workspace.workspace.id.clone(), service.id.clone())
        }))
    }

    /// Lists the last observed health of every service in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does not
    /// exist.
    pub fn list_service_health(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceHealth>, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        Ok(workspace
            .workspace
            .services
            .iter()
            .map(|service| {
                self.health()
                    .service_health(&service.id)
                    .unwrap_or_else(|| {
                        ServiceHealth::unknown(workspace.workspace.id.clone(), service.id.clone())
                    })
            })
            .collect())
    }

    /// Produces a structured diagnostic of a service's observed state.
    ///
    /// The diagnostic is assembled from facts the system has actually observed:
    /// the last runtime state, the managed process, the latest health result and
    /// its checks, the service's declared ports, and recent error logs. It
    /// performs no destructive or mutating operation and never fabricates a root
    /// cause (`tasks/012.md` §30).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist.
    pub fn get_service_diagnostics(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceDiagnostics, ApplicationError> {
        let (workspace, service) = self.locate_service(service_id)?;
        let workspace_id = workspace.workspace.id.clone();

        let state = self.runtime().service_state(service_id).unwrap_or_default();
        let info = self.service_runtime().runtime_state(service_id);
        let health = self
            .health()
            .service_health(service_id)
            .unwrap_or_else(|| ServiceHealth::unknown(workspace_id.clone(), service_id.clone()));

        let recent_errors = self.recent_error_logs(service_id, 20);
        let last_failure = self.latest_failure(&health, &recent_errors);

        let mut notes = Vec::new();
        if !service.health_checks.is_empty() && health.checked_at.is_none() {
            notes.push("no health check has been observed yet".to_owned());
        }
        if service.health_checks.is_empty() {
            notes.push("the service has no health checks configured".to_owned());
        }

        Ok(ServiceDiagnostics {
            service_id: service_id.clone(),
            workspace_id,
            process_state: info.as_ref().map_or(state.process, |info| info.state),
            health_state: health.state,
            managed: info.is_some(),
            process_id: info.as_ref().and_then(|info| info.process_id.clone()),
            pid: info.as_ref().and_then(|info| info.pid),
            ports: service.ports.clone(),
            last_checked_at: health.checked_at,
            health_checks: health.checks.clone(),
            last_failure,
            recent_errors,
            notes,
        })
    }

    /// Returns one stable page of a service's logs.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or a structured log error if the page cannot be retrieved.
    pub fn get_service_logs(
        &self,
        service_id: &ServiceId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError> {
        self.locate_service(service_id)?;
        let request = request.clone().for_service(service_id.clone());
        self.logs().page(&request).map_err(map_log_error)
    }

    /// Returns one stable page of a task's logs.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::TaskNotFound`] if the task does not exist, or
    /// a structured log error if the page cannot be retrieved.
    pub fn get_task_logs(
        &self,
        task_id: &TaskId,
        request: &LogRequest,
    ) -> Result<LogPage, ApplicationError> {
        self.get_task(task_id)?;
        let request = request.clone().for_task(task_id.clone());
        self.logs().page(&request).map_err(map_log_error)
    }

    /// Subscribes to realtime logs for a service.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, or a structured log error if streaming is unavailable.
    pub fn subscribe_service_logs(
        &self,
        service_id: &ServiceId,
    ) -> Result<Box<dyn LogSubscription>, ApplicationError> {
        self.locate_service(service_id)?;
        self.logs().subscribe(service_id).map_err(map_log_error)
    }

    /// Returns the most recent error-level lines for a service.
    ///
    /// `LogService::query` returns entries newest first; the result is reversed
    /// so it is chronological (oldest first), making the latest error the last
    /// element. This is deliberately not `page`, whose forward cursor selects
    /// the *oldest* matching window.
    fn recent_error_logs(&self, service_id: &ServiceId, limit: usize) -> Vec<String> {
        let request = LogRequest::new(service_id.clone())
            .with_minimum_level(LogLevel::Error)
            .with_limit(limit);
        match self.logs().query(&request) {
            Ok(mut entries) => {
                entries.reverse();
                entries.into_iter().map(|entry| entry.message).collect()
            }
            Err(_) => Vec::new(),
        }
    }

    /// Derives the most recent observed failure from health and logs.
    fn latest_failure(
        &self,
        health: &ServiceHealth,
        recent_errors: &[String],
    ) -> Option<DiagnosticFailure> {
        if let Some(check) = health.checks.iter().find(|check| !check.is_healthy()) {
            return Some(DiagnosticFailure {
                source: "health".to_owned(),
                code: failure_code(&check.check_type),
                message: check.message.clone().unwrap_or_else(|| {
                    format!("the {} health check did not pass", check.check_type)
                }),
                at: Some(check.checked_at),
            });
        }
        recent_errors.last().map(|line| DiagnosticFailure {
            source: "log".to_owned(),
            code: "LOG_ERROR".to_owned(),
            message: line.clone(),
            at: health.checked_at,
        })
    }
}

/// Maps a log-service failure into an application error.
fn map_log_error(error: LogServiceError) -> ApplicationError {
    match error {
        LogServiceError::QueryInvalid { message } => ApplicationError::LogQueryInvalid { message },
        other => ApplicationError::LogService {
            message: other.to_string(),
        },
    }
}

/// Returns a stable failure code for a failing check type.
fn failure_code(check_type: &str) -> String {
    match check_type {
        "process" => "PROCESS_NOT_RUNNING".to_owned(),
        "tcp" => "TCP_CONNECTION_REFUSED".to_owned(),
        "http" => "HTTP_CHECK_FAILED".to_owned(),
        other => format!("{}_CHECK_FAILED", other.to_ascii_uppercase()),
    }
}
