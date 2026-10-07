//! The application container.
//!
//! [`Application`] is the single entry point every interface talks to. It holds
//! the already-validated [`Config`] plus the boundaries the use cases need
//! ([`TaskManager`], [`RuntimeStateStore`], [`LogService`]) and the
//! [`PolicyEngine`]. Interfaces never reach past it.
//!
//! Dependencies are injected explicitly through the constructor; there is no
//! framework. The struct is cloneable because its boundaries are shared behind
//! [`Arc`], so the TUI and MCP adapters can each hold an `Application` that
//! observes the same runtime state and submits to the same task engine.

use std::sync::Arc;

use service_orchestrator_config::{Config, ConfiguredWorkspace, LiquibaseConfig};
use service_orchestrator_domain::{
    GroupId, Library, LibraryId, Permission, Profile, ProfileId, Service, ServiceGroup, ServiceId,
    WorkspaceId,
};
use service_orchestrator_policy::{Authorization, PolicyEngine};

use crate::error::ApplicationError;
use crate::ports::{LogService, RuntimeStateStore};

/// The shared application core.
#[derive(Clone)]
pub struct Application {
    config: Config,
    tasks: Arc<dyn service_orchestrator_execution::TaskManager>,
    runtime: Arc<dyn RuntimeStateStore>,
    logs: Arc<dyn LogService>,
    policy: PolicyEngine,
    authorization: Authorization,
}

impl Application {
    /// Creates an application core from validated configuration and its ports.
    #[must_use]
    pub fn new(
        config: Config,
        tasks: Arc<dyn service_orchestrator_execution::TaskManager>,
        runtime: Arc<dyn RuntimeStateStore>,
        logs: Arc<dyn LogService>,
    ) -> Self {
        Self {
            config,
            tasks,
            runtime,
            logs,
            policy: PolicyEngine::new(),
            authorization: Authorization::standard(),
        }
    }

    /// Replaces the policy engine.
    #[must_use]
    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
        self.policy = policy;
        self
    }

    /// Sets the caller's authorization, for example after a user confirms a
    /// destructive action.
    #[must_use]
    pub fn with_authorization(mut self, authorization: Authorization) -> Self {
        self.authorization = authorization;
        self
    }

    /// Returns the validated configuration backing this application.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Checks `permission` against the current authorization.
    ///
    /// Interfaces call this before a destructive action to decide whether to
    /// prompt for confirmation; mutating use cases call it themselves so the
    /// check cannot be bypassed.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::PermissionDenied`] when the operation
    /// requires confirmation that has not been granted.
    pub fn authorize(&self, permission: Permission) -> Result<(), ApplicationError> {
        self.policy
            .check(permission, self.authorization)
            .map_err(|error| ApplicationError::PermissionDenied {
                permission,
                message: error.to_string(),
            })
    }

    /// Returns the execution settings carried by the configuration.
    pub(crate) fn default_timeout_seconds(&self) -> u64 {
        self.config.execution.default_timeout_seconds
    }

    /// Returns the task manager boundary.
    pub(crate) fn tasks(&self) -> &dyn service_orchestrator_execution::TaskManager {
        self.tasks.as_ref()
    }

    /// Returns the runtime state store.
    pub(crate) fn runtime(&self) -> &dyn RuntimeStateStore {
        self.runtime.as_ref()
    }

    /// Returns the log service.
    pub(crate) fn logs(&self) -> &dyn LogService {
        self.logs.as_ref()
    }

    // -- configuration lookups ------------------------------------------------

    /// Returns a configured workspace by id.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if it does not exist.
    pub(crate) fn require_workspace(
        &self,
        id: &WorkspaceId,
    ) -> Result<&ConfiguredWorkspace, ApplicationError> {
        self.config
            .workspace(id)
            .ok_or_else(|| ApplicationError::WorkspaceNotFound(id.clone()))
    }

    /// Returns the workspace that owns `id`.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if no workspace owns it.
    pub(crate) fn workspace_of_service(
        &self,
        id: &ServiceId,
    ) -> Result<&ConfiguredWorkspace, ApplicationError> {
        self.config
            .workspace_of_service(id)
            .ok_or_else(|| ApplicationError::ServiceNotFound(id.clone()))
    }

    /// Returns a workspace and service pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist.
    pub(crate) fn locate_service(
        &self,
        id: &ServiceId,
    ) -> Result<(&ConfiguredWorkspace, &Service), ApplicationError> {
        let workspace = self.workspace_of_service(id)?;
        let service = workspace
            .workspace
            .service(id)
            .expect("workspace_of_service returned a workspace that owns the service");
        Ok((workspace, service))
    }

    /// Returns a workspace and library pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist.
    pub(crate) fn locate_library(
        &self,
        id: &LibraryId,
    ) -> Result<(&ConfiguredWorkspace, &Library), ApplicationError> {
        for workspace in &self.config.workspaces {
            if let Some(library) = workspace.workspace.library(id) {
                return Ok((workspace, library));
            }
        }
        Err(ApplicationError::LibraryNotFound(id.clone()))
    }

    /// Returns a workspace and profile pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ProfileNotFound`] if the profile does not
    /// exist.
    pub(crate) fn locate_profile(
        &self,
        id: &ProfileId,
    ) -> Result<(&ConfiguredWorkspace, &Profile), ApplicationError> {
        for workspace in &self.config.workspaces {
            if let Some(profile) = workspace.workspace.profile(id) {
                return Ok((workspace, profile));
            }
        }
        Err(ApplicationError::ProfileNotFound(id.clone()))
    }

    /// Returns a workspace and service group pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::GroupNotFound`] if the group does not exist.
    pub(crate) fn locate_group(
        &self,
        id: &GroupId,
    ) -> Result<(&ConfiguredWorkspace, &ServiceGroup), ApplicationError> {
        for workspace in &self.config.workspaces {
            if let Some(group) = workspace.workspace.group(id) {
                return Ok((workspace, group));
            }
        }
        Err(ApplicationError::GroupNotFound(id.clone()))
    }

    /// Returns the Liquibase configuration for a service, if any.
    pub(crate) fn liquibase_config<'a>(
        &self,
        workspace: &'a ConfiguredWorkspace,
        service_id: &ServiceId,
    ) -> Option<&'a LiquibaseConfig> {
        workspace.liquibase.get(service_id)
    }
}
