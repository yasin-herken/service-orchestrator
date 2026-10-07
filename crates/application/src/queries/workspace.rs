//! Workspace, group, and profile queries.

use service_orchestrator_domain::{
    GroupId, Profile, ProfileId, ServiceGroup, Workspace, WorkspaceId,
};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::WorkspaceStatus;

impl Application {
    /// Lists all configured workspaces.
    #[must_use]
    pub fn list_workspaces(&self) -> Vec<Workspace> {
        self.config()
            .workspaces
            .iter()
            .map(|workspace| workspace.workspace.clone())
            .collect()
    }

    /// Returns a workspace by id.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if it does not exist.
    pub fn get_workspace(&self, workspace_id: &WorkspaceId) -> Result<Workspace, ApplicationError> {
        Ok(self.require_workspace(workspace_id)?.workspace.clone())
    }

    /// Returns a structured status snapshot of a workspace and its services.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if it does not exist.
    pub fn get_workspace_status(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceStatus, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        let services = workspace
            .workspace
            .services
            .iter()
            .map(|service| self.build_service_status(workspace, service))
            .collect();
        Ok(WorkspaceStatus {
            workspace_id: workspace.workspace.id.clone(),
            name: workspace.workspace.name.clone(),
            root: workspace.workspace.root.clone(),
            services,
        })
    }

    /// Lists the service groups in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does
    /// not exist.
    pub fn list_groups(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ServiceGroup>, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        Ok(workspace.workspace.groups.clone())
    }

    /// Returns a service group by id.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::GroupNotFound`] if it does not exist.
    pub fn get_group(&self, group_id: &GroupId) -> Result<ServiceGroup, ApplicationError> {
        let (_workspace, group) = self.locate_group(group_id)?;
        Ok(group.clone())
    }

    /// Lists the developer profiles in a workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does
    /// not exist.
    pub fn list_profiles(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<Profile>, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        Ok(workspace.workspace.profiles.clone())
    }

    /// Returns a developer profile by id.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ProfileNotFound`] if it does not exist.
    pub fn get_profile(&self, profile_id: &ProfileId) -> Result<Profile, ApplicationError> {
        let (_workspace, profile) = self.locate_profile(profile_id)?;
        Ok(profile.clone())
    }
}
