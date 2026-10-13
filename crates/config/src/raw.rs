//! Raw TOML configuration models.
//!
//! These types are a faithful, unvalidated mirror of the on-disk schema. They
//! deliberately contain `String`s and `Option`s rather than domain value
//! objects so that every failure can be reported with a useful location by
//! [`crate::normalize`] instead of surfacing as an opaque deserialization
//! error.
//!
//! Every struct uses `#[serde(deny_unknown_fields)]`: an unknown or misspelled
//! key is an error, never silently ignored. This is an intentional choice — a
//! typo such as `max_paralel_tasks` should fail loudly (see
//! `docs/configuration.md`).
//!
//! The types are also `Serialize` so the configuration management layer can
//! round-trip a single entity back to TOML when an edit is applied
//! (`tasks/023.md` §37). `Option` and empty collections are skipped when
//! serializing so an unchanged entity never gains noise such as
//! `environment = {}`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The complete raw configuration file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    /// The schema version. Defaulted so a file that omits it is treated as v1;
    /// unknown versions are rejected by the loader.
    #[serde(default = "default_version")]
    pub config_version: u32,
    /// The declared workspaces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<RawWorkspace>,
    /// The declared services (across all workspaces).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<RawService>,
    /// The declared libraries (across all workspaces).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<RawLibrary>,
    /// The declared service groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<RawGroup>,
    /// The declared developer profiles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<RawProfile>,
    /// Global task execution settings.
    #[serde(default)]
    pub execution: RawExecution,
}

fn default_version() -> u32 {
    crate::model::DEFAULT_CONFIG_VERSION
}

/// A raw workspace.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawWorkspace {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The workspace root, which may begin with `~`.
    pub root: String,
    /// Workspace-wide environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, RawEnvValue>,
}

/// A raw service.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawService {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `backend`, `frontend`, `library`, or `other:<name>`.
    #[serde(rename = "type")]
    pub service_type: String,
    /// The id of the owning workspace.
    pub workspace: String,
    /// The path relative to the workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The branch checked out by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// The source repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<RawRepository>,
    /// A single runtime requirement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RawRuntime>,
    /// Additional runtime requirements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<RawRuntime>,
    /// The configured commands.
    #[serde(default, skip_serializing_if = "raw_commands_is_empty")]
    pub commands: RawCommands,
    /// The build system (`maven` or `npm`). When omitted it is inferred from
    /// the runtimes and commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_system: Option<String>,
    /// Optional Liquibase configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquibase: Option<RawLiquibase>,
    /// Health checks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health_checks: Vec<RawHealthCheck>,
    /// Exposed ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<RawPort>,
    /// Declared dependencies.
    #[serde(default, skip_serializing_if = "raw_dependencies_is_empty")]
    pub dependencies: RawDependencies,
    /// Service environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, RawEnvValue>,
    /// Extra capabilities beyond the type defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// A raw library.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawLibrary {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The id of the owning workspace.
    pub workspace: String,
    /// The path relative to the workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The branch checked out by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// The artifact technology. Defaults to `maven`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub library_type: Option<String>,
    /// The source repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<RawRepository>,
    /// The published artifact coordinate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<RawArtifact>,
    /// Additional artifacts a multi-module (reactor) library produces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<RawArtifact>,
    /// A single runtime requirement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RawRuntime>,
    /// Additional runtime requirements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<RawRuntime>,
    /// The configured commands.
    #[serde(default, skip_serializing_if = "raw_commands_is_empty")]
    pub commands: RawCommands,
    /// The build system (`maven` or `npm`). When omitted it is inferred from
    /// the library `type`, runtimes, and commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_system: Option<String>,
    /// Declared dependencies (libraries only).
    #[serde(default, skip_serializing_if = "raw_dependencies_is_empty")]
    pub dependencies: RawDependencies,
    /// Library environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, RawEnvValue>,
}

/// A raw repository.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawRepository {
    /// The clone URL.
    pub url: String,
    /// The default branch; falls back to the entity's `default_branch`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// The hosting provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// A raw runtime requirement.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawRuntime {
    /// `java`, `node`, `maven`, `npm`, or `other:<name>`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The pinned version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// A raw artifact coordinate.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawArtifact {
    /// The group identifier.
    pub group_id: String,
    /// The artifact identifier.
    pub artifact_id: String,
    /// The artifact version.
    pub version: String,
    /// The packaging (for example `jar` or `pom`). Defaults to `jar`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packaging: Option<String>,
    /// The classifier (for example `sources`), when the artifact has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classifier: Option<String>,
}

/// The raw command table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawCommands {
    /// The install command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install: Option<RawCommand>,
    /// The build command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<RawCommand>,
    /// The start command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<RawCommand>,
    /// The stop command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<RawCommand>,
    /// The restart command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<RawCommand>,
    /// The test command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<RawCommand>,
    /// The Liquibase command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquibase: Option<RawCommand>,
    /// Project-specific commands, keyed by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub custom: BTreeMap<String, RawCommand>,
}

/// Returns `true` when every command is unset.
fn raw_commands_is_empty(commands: &RawCommands) -> bool {
    commands.install.is_none()
        && commands.build.is_none()
        && commands.start.is_none()
        && commands.stop.is_none()
        && commands.restart.is_none()
        && commands.test.is_none()
        && commands.liquibase.is_none()
        && commands.custom.is_empty()
}

/// A command, in either short string or structured form.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RawCommand {
    /// A command line such as `"./mvnw clean install"`.
    Line(String),
    /// A structured command with an explicit program and arguments.
    Structured(RawStructuredCommand),
}

/// A structured command.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawStructuredCommand {
    /// The executable.
    pub program: String,
    /// The arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// The working directory, relative to the workspace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Command-specific environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, RawEnvValue>,
}

/// An environment variable value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RawEnvValue {
    /// A literal, non-secret value.
    Literal(String),
    /// A literal value with an explicit secrecy flag.
    Structured(RawEnvVariable),
}

/// A structured environment variable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawEnvVariable {
    /// The literal value.
    pub value: String,
    /// Whether the value must be redacted when displayed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub secret: bool,
}

/// Returns `true` when the flag is `false` (used to skip it when serializing).
fn is_false(value: &bool) -> bool {
    !*value
}

/// Optional Liquibase configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawLiquibase {
    /// Whether Liquibase should run. Defaults to `true` when the block is
    /// present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The legacy single migration command; mapped to `commands.update`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<RawCommand>,
    /// The migration provider (`liquibase` by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The Liquibase execution mode (`maven` by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    /// The per-operation migration commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commands: Option<RawMigrationCommands>,
    /// The migration timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    /// Liquibase environment variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, RawEnvValue>,
    /// The declared target database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<RawMigrationDatabase>,
}

/// The raw per-operation migration command table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawMigrationCommands {
    /// The validation command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validate: Option<RawCommand>,
    /// The status command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RawCommand>,
    /// The pending-changes command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<RawCommand>,
    /// The SQL-preview command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<RawCommand>,
    /// The apply (update) command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<RawCommand>,
}

/// The raw declared target database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawMigrationDatabase {
    /// The declared environment name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// The database type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database_type: Option<String>,
    /// The database name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The host identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Liquibase contexts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contexts: Vec<String>,
    /// Liquibase labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// A raw health check.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawHealthCheck {
    /// `process`, `port`, or `http`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The URL for an HTTP check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The expected HTTP status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_status: Option<u16>,
    /// The port for a port check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// The probe timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    /// The polling interval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_seconds: Option<u64>,
    /// Whether the check is required for the service to be healthy. Defaults
    /// to `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// A raw port.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawPort {
    /// The port number.
    pub port: u16,
    /// `tcp` or `udp`. Defaults to `tcp`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
}

/// The raw dependency tables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawDependencies {
    /// Dependencies on other services.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<RawDependencyRef>,
    /// Dependencies on libraries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<RawDependencyRef>,
}

/// Returns `true` when no dependency is declared.
fn raw_dependencies_is_empty(dependencies: &RawDependencies) -> bool {
    dependencies.services.is_empty() && dependencies.libraries.is_empty()
}

/// A dependency reference, in short string or detailed form.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum RawDependencyRef {
    /// A bare identifier such as `"auth-service"`.
    Id(String),
    /// A reference with an explicit kind and required flag.
    Detailed(RawDetailedDependency),
}

/// A detailed dependency reference.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawDetailedDependency {
    /// The target identifier.
    pub id: String,
    /// `build` or `runtime`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Whether the dependency is mandatory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// A raw service group.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawGroup {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// An optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The owning workspace; inferred from members when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// The services in the group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
}

/// A raw developer profile.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawProfile {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// An optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The owning workspace; inferred from members when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// The services in the profile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    /// The groups in the profile.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
}

/// Global execution settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawExecution {
    /// The maximum number of parallel tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallel_tasks: Option<usize>,
    /// The default operation timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_timeout_seconds: Option<u64>,
}
