//! Saved bulk-operation preset queries (`tasks/032.md` §36–§38).
//!
//! A saved preset is *selection and options*, never authorization: listing or
//! loading one performs no work, stores no credentials, and pre-fills the Bulk
//! Operations screen with a freshly composed
//! [`PrepareEnvironmentRequest`](crate::models::PrepareEnvironmentRequest).
//! Execution then runs the ordinary plan/validate/authorize path, so a preset
//! can never bypass the policy engine or the per-target destructive
//! confirmation (`tasks/032.md` §37).

use service_orchestrator_config::{OperationPreset, RuntimeAction};
use service_orchestrator_domain::{OperationPresetId, Permission, ServiceId, WorkspaceId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::{OperationPresetSummary, PrepareEnvironmentRequest};

impl Application {
    /// Lists the saved operation presets that apply to a workspace.
    ///
    /// When `workspace_id` is `None`, every preset is listed. A preset whose
    /// owning workspace is unknown never appears; configuration validation
    /// resolves a preset's workspace from its members, so that cannot happen
    /// for a loaded configuration — it is treated as "applies to any
    /// workspace" instead of failing the whole listing.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] for an unknown
    /// workspace, and [`ApplicationError::PermissionDenied`] without read
    /// permission.
    pub fn list_operation_presets(
        &self,
        workspace_id: Option<&WorkspaceId>,
    ) -> Result<Vec<OperationPresetSummary>, ApplicationError> {
        self.authorize(Permission::Read)?;
        if let Some(workspace_id) = workspace_id {
            self.require_workspace(workspace_id)?;
        }
        self.config()
            .operation_presets
            .iter()
            .filter(|preset| {
                workspace_id.is_none_or(|workspace_id| preset_applies(preset, workspace_id))
            })
            .map(|preset| {
                let workspace_id = preset
                    .workspace
                    .clone()
                    .or_else(|| workspace_id.cloned())
                    .expect("a listed preset always has a workspace");
                Ok(OperationPresetSummary {
                    id: preset.id.clone(),
                    name: preset.name.clone(),
                    description: preset.description.clone(),
                    workspace_id: workspace_id.clone(),
                    default: preset.default,
                    request: self.compose_preset_request(&workspace_id, preset)?,
                })
            })
            .collect()
    }

    /// Composes the environment request a saved preset stands for.
    ///
    /// The preset's explicit services are combined with the members of every
    /// selected profile and group into one explicit service selection, so the
    /// caller reviews exactly the services the preset picks — dependency
    /// closure is still the workflow's job (`tasks/032.md` §23).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] for an unknown
    /// workspace, [`ApplicationError::InvalidOperation`] when the preset
    /// belongs to a different workspace or does not exist, and the profile/group
    /// not-found errors when a referenced member is missing from the loaded
    /// configuration.
    pub fn operation_preset_request(
        &self,
        workspace_id: &WorkspaceId,
        preset_id: &OperationPresetId,
    ) -> Result<PrepareEnvironmentRequest, ApplicationError> {
        self.authorize(Permission::Read)?;
        self.require_workspace(workspace_id)?;
        let preset = self.config().operation_preset(preset_id).ok_or_else(|| {
            ApplicationError::invalid_operation(format!(
                "operation preset '{preset_id}' does not exist"
            ))
        })?;
        if !preset_applies(preset, workspace_id) {
            return Err(ApplicationError::invalid_operation(format!(
                "operation preset '{preset_id}' belongs to workspace '{}', not '{workspace_id}'",
                preset
                    .workspace
                    .as_ref()
                    .map_or("<unknown>", WorkspaceId::as_str)
            )));
        }
        self.compose_preset_request(workspace_id, preset)
    }

    /// Expands a validated preset into a concrete environment request.
    fn compose_preset_request(
        &self,
        workspace_id: &WorkspaceId,
        preset: &OperationPreset,
    ) -> Result<PrepareEnvironmentRequest, ApplicationError> {
        let workspace = self.require_workspace(workspace_id)?;
        let mut services: Vec<ServiceId> = Vec::new();
        let mut push = |service: &ServiceId| {
            if !services.contains(service) {
                services.push(service.clone());
            }
        };
        for service in &preset.services {
            push(service);
        }
        for profile_id in &preset.profiles {
            let profile = workspace
                .workspace
                .profile(profile_id)
                .ok_or_else(|| ApplicationError::ProfileNotFound(profile_id.clone()))?;
            for service in self.profile_service_ids(workspace, profile) {
                push(&service);
            }
        }
        for group_id in &preset.groups {
            let group = workspace
                .workspace
                .group(group_id)
                .ok_or_else(|| ApplicationError::GroupNotFound(group_id.clone()))?;
            for service in &group.services {
                push(service);
            }
        }

        let mut request = PrepareEnvironmentRequest::new(workspace_id.clone());
        request.services = services;
        request.sync = preset.git.enabled;
        request.branch = preset.git.branch.clone();
        request.build_libraries = preset.libraries.enabled;
        // A preset that explicitly disables migration composes no migration,
        // even when it names a preset id: `enabled = false` is the declaration
        // of intent, and the request validator refuses a preset without
        // migration enabled.
        if preset.migration.enabled {
            request.run_liquibase = true;
            request.migration_preset = preset.migration.preset.clone();
        }
        request.build = preset.build.enabled;
        request.build_operation = preset.build.operation;
        match preset.runtime.action {
            RuntimeAction::None => {}
            RuntimeAction::Start => request.start = true,
            RuntimeAction::Restart => request.restart = true,
            RuntimeAction::Stop => request.stop = true,
        }
        request.wait_for_health = preset.health.enabled;
        Ok(request)
    }
}

/// Whether a preset may be used from `workspace_id`.
///
/// A preset without a declared workspace applies anywhere; a preset with one
/// applies only there.
fn preset_applies(preset: &OperationPreset, workspace_id: &WorkspaceId) -> bool {
    preset
        .workspace
        .as_ref()
        .is_none_or(|owned| owned == workspace_id)
}
