//! Service and library queries.
//!
//! Queries are side-effect free: they read the validated configuration and the
//! last observed runtime state. They never fetch, check out, build, probe, or
//! start anything.

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{Library, LibraryId, Service, ServiceId, WorkspaceId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::ServiceStatus;

impl Application {
    /// Lists the services in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does
    /// not exist.
    pub fn list_services(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<Service>, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        Ok(workspace.workspace.services.clone())
    }

    /// Returns a service by id.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist.
    pub fn get_service(&self, service_id: &ServiceId) -> Result<Service, ApplicationError> {
        let (_workspace, service) = self.locate_service(service_id)?;
        Ok(service.clone())
    }

    /// Returns a structured status snapshot for a service.
    ///
    /// The status combines configuration with the last observed runtime state;
    /// it performs no probes and mutates nothing.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist.
    pub fn get_service_status(
        &self,
        service_id: &ServiceId,
    ) -> Result<ServiceStatus, ApplicationError> {
        let (workspace, service) = self.locate_service(service_id)?;
        Ok(self.build_service_status(workspace, service))
    }

    /// Lists the libraries in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does
    /// not exist.
    pub fn list_libraries(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<Library>, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        Ok(workspace.workspace.libraries.clone())
    }

    /// Returns a library by id.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist.
    pub fn get_library(&self, library_id: &LibraryId) -> Result<Library, ApplicationError> {
        let (_workspace, library) = self.locate_library(library_id)?;
        Ok(library.clone())
    }

    /// Builds a [`ServiceStatus`] from a workspace and service.
    pub(crate) fn build_service_status(
        &self,
        workspace: &ConfiguredWorkspace,
        service: &Service,
    ) -> ServiceStatus {
        let state = self
            .runtime()
            .service_state(&service.id)
            .unwrap_or_default();
        ServiceStatus {
            service_id: service.id.clone(),
            workspace_id: workspace.workspace.id.clone(),
            display_name: service.display_name.clone(),
            kind: service.kind.clone(),
            process_state: state.process,
            health_state: state.health,
            git_state: state.git,
            branch: state.branch,
            ports: service.ports.clone(),
            capabilities: service.capabilities.clone(),
        }
    }
}
