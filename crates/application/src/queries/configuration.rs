//! Configuration snapshot, validation, and preview queries.
//!
//! These are read-only: they never write the configuration file. A preview
//! *proposes* a change by applying it to an in-memory copy and re-validating it,
//! and a validation re-reads and checks the current file
//! (`tasks/023.md` §37, §38). Only [`commit_configuration`](crate::Application::commit_configuration)
//! writes.

use std::collections::BTreeSet;

use service_orchestrator_config::{
    ConfigChange, ConfigError, ConfigurationPreview, ConfigurationSnapshot, EntityKind,
    RunningServiceImpact,
};
use service_orchestrator_domain::Permission;

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::ConfigurationValidation;

impl Application {
    /// Returns a read-only raw snapshot of the configuration for editing.
    ///
    /// # Errors
    ///
    /// Returns an [`ApplicationError::Configuration`] when the file cannot be
    /// read or parsed, or when no configuration file is available.
    pub fn configuration_snapshot(&self) -> Result<ConfigurationSnapshot, ApplicationError> {
        self.authorize(Permission::Read)?;
        self.configuration()
            .snapshot()
            .map_err(ApplicationError::config)
    }

    /// Validates the current configuration file without changing anything.
    ///
    /// The current serving configuration is never updated, even if the file is
    /// invalid (`tasks/023.md` §38).
    ///
    /// # Errors
    ///
    /// Returns an [`ApplicationError::Configuration`] when the file cannot be
    /// read or parsed at all. A file that parses but is invalid is reported
    /// through [`ConfigurationValidation::valid`] and its issues.
    pub fn validate_configuration(&self) -> Result<ConfigurationValidation, ApplicationError> {
        self.authorize(Permission::Read)?;
        let revision = self.configuration().revision();
        match self.configuration().validate() {
            Ok(()) => Ok(ConfigurationValidation {
                revision,
                valid: true,
                issues: Vec::new(),
            }),
            Err(ConfigError::Validation(errors)) => Ok(ConfigurationValidation {
                revision,
                valid: false,
                issues: errors.errors().to_vec(),
            }),
            Err(other) => Err(ApplicationError::config(other)),
        }
    }

    /// Previews a change without committing it.
    ///
    /// The returned preview carries the structured diff, the validation result,
    /// the references a removal would break, and — filled in here — the impact
    /// on services that are currently running
    /// (`tasks/023.md` §9, §29, §37).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::Configuration`] with code `CONFIG_CONFLICT`
    /// when the file no longer matches `expected_revision`, and a structural
    /// error when the change cannot be applied.
    pub fn preview_configuration(
        &self,
        change: &ConfigChange,
        expected_revision: &str,
    ) -> Result<ConfigurationPreview, ApplicationError> {
        self.authorize(Permission::Read)?;
        let mut preview = self
            .configuration()
            .preview(change, expected_revision)
            .map_err(ApplicationError::config)?;
        preview.running_impact = self.running_impact(change, &preview);
        Ok(preview)
    }

    /// Computes the impact of a proposed change on currently running services.
    ///
    /// A running service that a change touches is reported with the action the
    /// developer must take; the change itself never restarts or stops a process
    /// (`tasks/023.md` §9, §29).
    fn running_impact(
        &self,
        change: &ConfigChange,
        preview: &ConfigurationPreview,
    ) -> Vec<RunningServiceImpact> {
        let running = self.running_service_ids();
        let mut impact = Vec::new();

        for updated in &preview.diff.updated {
            if updated.kind == EntityKind::Service && running.contains(&updated.id) {
                impact.push(RunningServiceImpact {
                    service_id: updated.id.clone(),
                    running: true,
                    change: "updated".to_owned(),
                    action_required: format!(
                        "restart '{}' to apply the new configuration",
                        updated.id
                    ),
                });
            }
        }
        for removed in &preview.diff.removed {
            if removed.kind == EntityKind::Service && running.contains(&removed.id) {
                impact.push(RunningServiceImpact {
                    service_id: removed.id.clone(),
                    running: true,
                    change: "removed".to_owned(),
                    action_required: format!(
                        "stop '{}'; it is no longer part of the configuration",
                        removed.id
                    ),
                });
            }
        }
        // Removing a workspace orphans every service it owns.
        if change.kind() == EntityKind::Workspace && change.action() == "remove" {
            if let Some(id) = change.target_id() {
                for service_id in self.services_of_workspace(id) {
                    if running.contains(&service_id)
                        && !impact.iter().any(|entry| entry.service_id == service_id)
                    {
                        impact.push(RunningServiceImpact {
                            service_id: service_id.clone(),
                            running: true,
                            change: "removed".to_owned(),
                            action_required: format!(
                                "stop '{service_id}'; its workspace '{id}' is being removed"
                            ),
                        });
                    }
                }
            }
        }
        impact.sort_by(|a, b| a.service_id.cmp(&b.service_id));
        impact
    }

    /// Returns the ids of every service currently tracked as running.
    fn running_service_ids(&self) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        for workspace in &self.config().workspaces {
            if let Ok(running) = self.list_running_services(&workspace.workspace.id) {
                for status in running {
                    ids.insert(status.service_id.to_string());
                }
            }
        }
        ids
    }

    /// Returns the service ids declared in a workspace.
    fn services_of_workspace(&self, workspace_id: &str) -> Vec<String> {
        self.config()
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace.id.as_str() == workspace_id)
            .map(|workspace| {
                workspace
                    .workspace
                    .services
                    .iter()
                    .map(|service| service.id.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}
