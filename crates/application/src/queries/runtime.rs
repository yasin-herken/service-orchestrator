//! Service runtime queries.
//!
//! Runtime-status queries are read-only: they combine validated configuration
//! with live managed-process state exposed by the [`ServiceRuntimeIndex`] port.
//! When no runtime adapter is wired the index answers "nothing is managed", so a
//! query falls back to the last observed
//! [`RuntimeStateStore`](crate::RuntimeStateStore) value — never probing a
//! process, a port, or a socket.

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{CommandKind, Service, ServiceId, WorkspaceId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::ServiceRuntimeStatus;
use crate::ports::ServiceProcessInfo;

impl Application {
    /// Returns a structured runtime status for a service.
    ///
    /// The status reports the managed process when the runtime adapter is
    /// tracking one (process id, pid, runtime, redacted command, exit
    /// information) and otherwise the last observed logical process state.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist.
    pub fn get_service_runtime_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceRuntimeStatus, ApplicationError> {
        let (workspace, service) = self.locate_service(service_id)?;
        let info = self.service_runtime().runtime_state(service_id);
        Ok(self.build_runtime_status(workspace, service, info))
    }

    /// Lists the running (or transitioning) services in a workspace.
    ///
    /// A service is included when the runtime adapter reports an active managed
    /// process, or — when no adapter is wired — when the last observed process
    /// state is active. Services that are merely configured are never started
    /// automatically and therefore never appear here unless they were explicitly
    /// started.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does not
    /// exist.
    pub fn list_running_services(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceRuntimeStatus>, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        let mut statuses = Vec::new();
        for service in &workspace.workspace.services {
            let info = self.service_runtime().runtime_state(&service.id);
            let active = match &info {
                Some(info) => info.state.is_active(),
                None => self
                    .runtime()
                    .service_state(&service.id)
                    .unwrap_or_default()
                    .process
                    .is_active(),
            };
            if active {
                statuses.push(self.build_runtime_status(workspace, service, info));
            }
        }
        Ok(statuses)
    }

    /// Builds a [`ServiceRuntimeStatus`] from configuration and optional managed
    /// process detail.
    pub(crate) fn build_runtime_status(
        &self,
        workspace: &ConfiguredWorkspace,
        service: &Service,
        info: Option<ServiceProcessInfo>,
    ) -> ServiceRuntimeStatus {
        let workspace_id = workspace.workspace.id.clone();
        match info {
            Some(info) => ServiceRuntimeStatus {
                service_id: service.id.clone(),
                workspace_id,
                process_state: info.state,
                managed: true,
                runtime: info.runtime,
                process_id: info.process_id,
                pid: info.pid,
                command: info.command,
                started_at: info.started_at,
                exit_code: info.exit_code,
                exit_reason: info.exit_reason,
                last_error: info.last_error,
            },
            None => {
                let state = self
                    .runtime()
                    .service_state(&service.id)
                    .unwrap_or_default();
                ServiceRuntimeStatus {
                    service_id: service.id.clone(),
                    workspace_id,
                    process_state: state.process,
                    managed: false,
                    runtime: None,
                    process_id: None,
                    pid: None,
                    command: service
                        .commands
                        .get(&CommandKind::Start)
                        .map(|command| redacted_command(&command.program, &command.args)),
                    started_at: None,
                    exit_code: None,
                    exit_reason: None,
                    last_error: None,
                }
            }
        }
    }
}

/// Renders a command identity without any environment values.
///
/// Only the program and its structured arguments are shown, so a startup
/// command can be surfaced through an interface without leaking a credential
/// (`AGENTS.md` §31).
fn redacted_command(program: &str, args: &[String]) -> String {
    let mut rendered = program.to_owned();
    for arg in args {
        rendered.push(' ');
        rendered.push_str(arg);
    }
    rendered
}
