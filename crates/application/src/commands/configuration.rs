//! Configuration commit and reload commands.
//!
//! These are the only use cases that change configuration. A commit validates
//! the whole proposed file before writing it atomically, and a reload re-reads
//! and validates the file. Neither ever mutates a running process; a touched
//! running service is reported for the developer to act on
//! (`tasks/023.md` §8, §9, §29).

use service_orchestrator_config::ConfigChange;
use service_orchestrator_domain::Permission;

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::{ConfigurationCommit, ConfigurationCounts, ConfigurationReload};

impl Application {
    /// Validates and persists a configuration change.
    ///
    /// A create or update requires [`Permission::SafeWrite`]; a removal requires
    /// [`Permission::Destructive`] because it can orphan a running service's
    /// configuration, so it must be explicitly confirmed through the same policy
    /// model as any other destructive operation (`tasks/023.md` §29, §30). The
    /// write is atomic and keeps a backup; the current configuration is replaced
    /// only after the file validates (`tasks/023.md` §33, §37).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::PermissionDenied`] when a removal is not
    /// confirmed, [`ApplicationError::ConfigurationInvalid`] when the proposal
    /// does not validate, and [`ApplicationError::Configuration`] with code
    /// `CONFIG_CONFLICT` when the file changed since it was read.
    pub fn commit_configuration(
        &self,
        change: &ConfigChange,
        expected_revision: &str,
    ) -> Result<ConfigurationCommit, ApplicationError> {
        let permission = if change.action() == "remove" {
            Permission::Destructive
        } else {
            Permission::SafeWrite
        };
        self.authorize(permission)?;
        let config = self
            .configuration()
            .commit(change, expected_revision)
            .map_err(ApplicationError::config)?;
        Ok(ConfigurationCommit {
            revision: self.configuration().revision(),
            description: change.description(),
            counts: ConfigurationCounts::of(&config),
        })
    }

    /// Re-reads and validates the configuration file from disk.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ConfigurationInvalid`] when the file is
    /// invalid and [`ApplicationError::Configuration`] when it cannot be read.
    pub fn reload_configuration(&self) -> Result<ConfigurationReload, ApplicationError> {
        self.authorize(Permission::Read)?;
        let config = self
            .configuration()
            .reload()
            .map_err(ApplicationError::config)?;
        Ok(ConfigurationReload {
            revision: self.configuration().revision(),
            counts: ConfigurationCounts::of(&config),
        })
    }
}
