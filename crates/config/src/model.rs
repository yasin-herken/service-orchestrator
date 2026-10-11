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
    CommandSpec, Environment, GroupId, Library, LibraryId, LiquibaseExecutionMode,
    MigrationOperation, MigrationProviderType, MigrationTargetEnvironment, Profile, ProfileId,
    Service, ServiceId, Workspace, WorkspaceId,
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
/// enabled flag, provider, execution mode, timeout, database target, or
/// dedicated environment, so those live here (`tasks/011.md` §8, §9).
///
/// The database *environments* (and any connection metadata) are declared here;
/// credentials never are. Secrets belong in the environment, referenced by name,
/// and are redacted by [`Environment`] whenever they are displayed
/// (`AGENTS.md` §31, `tasks/011.md` §11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiquibaseConfig {
    /// Whether migrations should run for the service.
    pub enabled: bool,
    /// The migration provider/tooling.
    pub provider: MigrationProviderType,
    /// How Liquibase is invoked, when the provider is Liquibase.
    pub execution_mode: LiquibaseExecutionMode,
    /// The commands configured per migration operation.
    pub commands: MigrationCommandSet,
    /// The migration timeout, in seconds.
    pub timeout_seconds: Option<u64>,
    /// Liquibase-specific environment variables.
    pub environment: Environment,
    /// The declared target database and its environment.
    pub database: MigrationDatabaseConfig,
}

/// The migration commands a service may configure, one per operation.
///
/// An unset command means the provider's default applies (for example the
/// conventional Maven Liquibase goals). A configured command is authoritative
/// and used verbatim (`tasks/011.md` §6, §8).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigrationCommandSet {
    /// The validation command.
    pub validate: Option<CommandSpec>,
    /// The status command.
    pub status: Option<CommandSpec>,
    /// The pending-changes command.
    pub pending: Option<CommandSpec>,
    /// The SQL-preview command.
    pub preview: Option<CommandSpec>,
    /// The apply (update) command.
    pub update: Option<CommandSpec>,
}

impl MigrationCommandSet {
    /// Returns the command configured for `operation`, if any.
    #[must_use]
    pub fn get(&self, operation: MigrationOperation) -> Option<&CommandSpec> {
        match operation {
            MigrationOperation::Validate => self.validate.as_ref(),
            MigrationOperation::Status => self.status.as_ref(),
            MigrationOperation::Pending => self.pending.as_ref(),
            MigrationOperation::Preview => self.preview.as_ref(),
            MigrationOperation::Apply => self.update.as_ref(),
        }
    }

    /// Sets the command for `operation`, replacing any existing value.
    pub fn set(&mut self, operation: MigrationOperation, command: Option<CommandSpec>) {
        match operation {
            MigrationOperation::Validate => self.validate = command,
            MigrationOperation::Status => self.status = command,
            MigrationOperation::Pending => self.pending = command,
            MigrationOperation::Preview => self.preview = command,
            MigrationOperation::Apply => self.update = command,
        }
    }

    /// Returns `true` when no command is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.validate.is_none()
            && self.status.is_none()
            && self.pending.is_none()
            && self.preview.is_none()
            && self.update.is_none()
    }
}

/// The declared target database of a service's migrations.
///
/// This is *identification*, not connection: the environment, type, name, host,
/// and schema are display-safe metadata used to distinguish one target from
/// another and to classify risk. It never carries a password or a full
/// credential-bearing JDBC URL (`tasks/011.md` §9, §11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationDatabaseConfig {
    /// The declared environment; [`MigrationTargetEnvironment::Unknown`] when
    /// nothing is declared.
    pub environment: MigrationTargetEnvironment,
    /// The database type, for example `postgresql`.
    pub database_type: Option<String>,
    /// The database name.
    pub name: Option<String>,
    /// The host identity (never a credential).
    pub host: Option<String>,
    /// The schema.
    pub schema: Option<String>,
    /// Liquibase contexts active for the target.
    pub contexts: Vec<String>,
    /// Liquibase labels active for the target.
    pub labels: Vec<String>,
}

impl Default for MigrationDatabaseConfig {
    fn default() -> Self {
        Self {
            environment: MigrationTargetEnvironment::Unknown,
            database_type: None,
            name: None,
            host: None,
            schema: None,
            contexts: Vec::new(),
            labels: Vec::new(),
        }
    }
}

impl MigrationDatabaseConfig {
    /// Returns a display-safe identity for the target.
    ///
    /// The identity is assembled from the declared environment, database type,
    /// name, host, and schema. It never contains a password or a full
    /// credential-bearing JDBC URL, so it is safe to log, attach to a task, or
    /// surface through an interface (`tasks/011.md` §9, §11, §29). Missing parts
    /// are rendered as `?` rather than omitted so the shape stays readable and a
    /// target with nothing declared is visibly unidentified.
    #[must_use]
    pub fn display_identity(&self) -> String {
        let kind = self.database_type.as_deref().unwrap_or("?");
        let name = self.name.as_deref().unwrap_or("?");
        let host = self.host.as_deref().unwrap_or("?");
        let schema = self.schema.as_deref().unwrap_or("?");
        format!(
            "{kind}/{name}@{host}/{schema} ({})",
            self.environment.label()
        )
    }

    /// Returns a stable key identifying the physical database target.
    ///
    /// Two services that declare the same database type, host, name, and schema
    /// are treated as the same target: their migrations are serialized so two
    /// writers never race over one database (`tasks/011.md` §15). The
    /// environment is **not** part of the key — it is a safety classification,
    /// not part of a database's identity — so two services that point at the
    /// same physical database but label it differently still coordinate.
    ///
    /// A target with no declared connection identity yields the same (mostly
    /// empty) key for every such service, which conservatively serializes every
    /// unidentified migration. That is intentional: when the system cannot prove
    /// two targets are distinct, it must assume they may be the same database.
    #[must_use]
    pub fn target_key(&self) -> String {
        format!(
            "type={};host={};name={};schema={}",
            self.database_type.as_deref().unwrap_or(""),
            self.host.as_deref().unwrap_or(""),
            self.name.as_deref().unwrap_or(""),
            self.schema.as_deref().unwrap_or(""),
        )
    }
}
