//! The validated configuration produced by the loader.
//!
//! [`Config`] is the only configuration type the rest of the application sees.
//! It is deliberately built from domain values ([`Workspace`] and friends) plus
//! a small number of configuration-only values that the domain does not model
//! yet:
//!
//! - global task [`ExecutionSettings`];
//! - workspace-level [`Environment`];
//! - per-service [`LiquibaseConfig`].
//!
//! Keeping these here rather than inventing domain types keeps the domain
//! untouched while still giving the application a single validated object.

use std::collections::BTreeMap;

use service_orchestrator_domain::{
    CommandSpec, Environment, GroupId, Library, LibraryId, Profile, ProfileId, Service, ServiceId,
    Workspace, WorkspaceId,
};

/// The configuration schema version understood by this build.
pub const SUPPORTED_CONFIG_VERSION: u32 = 1;

/// The version assumed when a file omits `config_version`.
pub const DEFAULT_CONFIG_VERSION: u32 = SUPPORTED_CONFIG_VERSION;

/// The default number of tasks executed in parallel.
pub const DEFAULT_MAX_PARALLEL_TASKS: usize = 4;

/// The default operation timeout, in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 1800;

/// A fully validated configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The schema version the file declared.
    pub version: u32,
    /// Global task execution settings.
    pub execution: ExecutionSettings,
    /// The configured workspaces.
    pub workspaces: Vec<ConfiguredWorkspace>,
}

impl Config {
    /// Returns the workspace with the given id, if present.
    #[must_use]
    pub fn workspace(&self, id: &WorkspaceId) -> Option<&ConfiguredWorkspace> {
        self.workspaces
            .iter()
            .find(|workspace| &workspace.workspace.id == id)
    }

    /// Returns the service with the given id across all workspaces.
    #[must_use]
    pub fn service(&self, id: &ServiceId) -> Option<&Service> {
        self.workspaces
            .iter()
            .find_map(|workspace| workspace.workspace.service(id))
    }

    /// Returns the library with the given id across all workspaces.
    #[must_use]
    pub fn library(&self, id: &LibraryId) -> Option<&Library> {
        self.workspaces
            .iter()
            .find_map(|workspace| workspace.workspace.library(id))
    }

    /// Returns the group with the given id across all workspaces.
    #[must_use]
    pub fn group(&self, id: &GroupId) -> Option<&service_orchestrator_domain::ServiceGroup> {
        self.workspaces
            .iter()
            .find_map(|workspace| workspace.workspace.group(id))
    }

    /// Returns the profile with the given id across all workspaces.
    #[must_use]
    pub fn profile(&self, id: &ProfileId) -> Option<&Profile> {
        self.workspaces
            .iter()
            .find_map(|workspace| workspace.workspace.profile(id))
    }

    /// Returns the workspace containing the given service, if any.
    #[must_use]
    pub fn workspace_of_service(&self, id: &ServiceId) -> Option<&ConfiguredWorkspace> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.workspace.service(id).is_some())
    }
}

/// A workspace plus the configuration-only values attached to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfiguredWorkspace {
    /// The validated domain workspace.
    pub workspace: Workspace,
    /// Workspace-wide environment variables.
    pub environment: Environment,
    /// Liquibase settings, keyed by service id.
    pub liquibase: BTreeMap<ServiceId, LiquibaseConfig>,
}

/// Global task execution settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionSettings {
    /// The maximum number of tasks executed in parallel.
    pub max_parallel_tasks: usize,
    /// The default operation timeout, in seconds.
    pub default_timeout_seconds: u64,
}

impl Default for ExecutionSettings {
    fn default() -> Self {
        Self {
            max_parallel_tasks: DEFAULT_MAX_PARALLEL_TASKS,
            default_timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
        }
    }
}

/// Per-service Liquibase configuration.
///
/// The domain models the *command* (`CommandKind::Liquibase`) but not the
/// enabled flag, timeout, or dedicated environment, so those live here until
/// the Liquibase adapter is built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiquibaseConfig {
    /// Whether Liquibase should run for the service.
    pub enabled: bool,
    /// The migration command, if configured.
    pub command: Option<CommandSpec>,
    /// The migration timeout, in seconds.
    pub timeout_seconds: Option<u64>,
    /// Liquibase-specific environment variables.
    pub environment: Environment,
}
