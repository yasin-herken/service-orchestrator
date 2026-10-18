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
    /// The declared reusable service templates (`tasks/032.md` §3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<RawTemplate>,
    /// The declared named migration presets (`tasks/032.md` §16).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub migration_presets: Vec<RawMigrationPreset>,
    /// The declared saved bulk-operation presets (`tasks/032.md` §36).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_presets: Vec<RawOperationPreset>,
    /// Global task execution settings.
    #[serde(default)]
    pub execution: RawExecution,
    /// Bounded repository-discovery settings.
    #[serde(default)]
    pub discovery: RawDiscovery,
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
    /// The default build operation (`clean`, `build`, `test`, …); workflows use
    /// it when no request overrides it (`tasks/032.md` §3, §12).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_operation: Option<String>,
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
    /// The default build operation (`clean`, `build`, `test`, …); workflows use
    /// it when no request overrides it (`tasks/032.md` §3, §12).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_operation: Option<String>,
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
    /// The default named migration preset for this service (`tasks/032.md`
    /// §21). The preset only defines *how* migration runs; the target database
    /// stays declared here and never moves into a preset or template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
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

/// Bounded repository-discovery settings (`tasks/032.md` §7).
///
/// Discovery walks a workspace root looking for Git repositories. Traversal is
/// deliberately bounded: a depth limit, an exclusion list, an optional
/// repository cap, and an explicit symlink policy keep a scan of a large or
/// hostile tree from running away. None of these settings executes anything;
/// discovery only reads well-known files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawDiscovery {
    /// The maximum directory depth below the workspace root. Defaults to 6.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
    /// Directory names excluded from traversal, in addition to the built-in
    /// defaults such as `.git` and `node_modules`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Whether symlinked directories may be followed. Defaults to `true`;
    /// followed links must still resolve inside the workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_symlinks: Option<bool>,
    /// The maximum number of repositories reported. Defaults to 200.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_repositories: Option<usize>,
}

/// A raw reusable service template (`tasks/032.md` §3).
///
/// A template is a set of *defaults* for the existing validated service model;
/// it is never a second source of truth for a service. Every string value may
/// use the documented `${...}` variables, which are substituted when the
/// template is resolved for a concrete repository. A template deliberately has
/// no database credentials, no ports, and no repository URL: those stay on the
/// service.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawTemplate {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// An optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The template category, for example `spring_boot_backend`,
    /// `react_frontend`, or `shared_maven_library`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The suggested service type (`backend`, `frontend`, `other:<name>`).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub service_type: Option<String>,
    /// The default branch checked out for a matching repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    /// The build system (`maven` or `npm`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_system: Option<String>,
    /// The build operation used for the service build phase (`build`, `test`,
    /// or another supported operation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_operation: Option<String>,
    /// A single runtime requirement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RawRuntime>,
    /// Additional runtime requirements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<RawRuntime>,
    /// The configured commands (build, start, …); values may use variables.
    #[serde(default, skip_serializing_if = "raw_commands_is_empty")]
    pub commands: RawCommands,
    /// Optional migration defaults; credentials and database targets are never
    /// part of a template (`tasks/032.md` §21).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liquibase: Option<RawTemplateMigration>,
    /// Health checks (for example `http` on `${port}` or a URL).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health_checks: Vec<RawHealthCheck>,
    /// The operation timeout, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    /// Environment variables; values may use variables. Secrets are still
    /// marked explicitly and redacted when displayed.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub environment: BTreeMap<String, RawEnvValue>,
    /// Extra capabilities beyond the type defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// The migration defaults a template may carry.
///
/// Only *how* migration runs is templated: whether it is enabled, the provider,
/// the execution mode, and the default named preset. The target database and
/// its credentials always come from the service itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawTemplateMigration {
    /// Whether migrations should run for a service created from the template.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The migration provider (`liquibase` by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The Liquibase execution mode (`maven` by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    /// The default named migration preset for the service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// A raw named migration preset (`tasks/032.md` §16–§19).
///
/// A preset names a structured set of Liquibase goals, an optional context
/// list, and its safety classification. It is deliberately **not** an
/// unrestricted command: goals are validated tokens composed into a Maven
/// argument vector, never a shell line. A destructive preset must declare a
/// non-empty `allowed_environments` list, and that list never grants
/// authorization by itself — a destructive preset still requires explicit,
/// interactive authorization through the application core at execution time.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawMigrationPreset {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// An optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The migration provider (`liquibase` by default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The execution mode. Only `maven` is accepted for a preset: a preset
    /// composes Maven goals and cannot express a CLI invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    /// The Liquibase goals, for example `["liquibase:update"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<String>,
    /// The Liquibase contexts; when omitted, the service's declared contexts
    /// apply (`tasks/032.md` §17).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contexts: Vec<String>,
    /// Whether the preset destroys database objects. Destructive presets
    /// require explicit per-target authorization before every execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive: Option<bool>,
    /// Whether the preset only inspects the database (for example `status`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// The environments a destructive preset may target. Required and
    /// non-empty for a destructive preset; never a substitute for the
    /// execution-time authorization (`tasks/032.md` §19, §37).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_environments: Vec<String>,
    /// The migration timeout, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
}

/// A raw saved bulk-operation preset (`tasks/032.md` §36).
///
/// A preset stores the *selection* of operations a developer commonly runs. It
/// never stores credentials and never carries authorization: loading one
/// pre-fills the Bulk Operations screen, and execution still runs the full
/// validation and policy of a freshly composed request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOperationPreset {
    /// The stable identifier.
    pub id: String,
    /// The human-readable name (defaults to the id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// An optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The owning workspace; inferred from the selection when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Whether this is the default preset for its workspace. At most one
    /// preset per workspace may set it.
    #[serde(rename = "default", skip_serializing_if = "Option::is_none")]
    pub default_preset: Option<bool>,
    /// Explicitly selected services.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<String>,
    /// Selected profiles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    /// Selected groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Git options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<RawOperationGit>,
    /// Library options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libraries: Option<RawOperationLibraries>,
    /// Migration options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration: Option<RawOperationMigration>,
    /// Build options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<RawOperationBuild>,
    /// Runtime options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RawOperationRuntime>,
    /// Health options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<RawOperationHealth>,
}

/// The Git options of a saved operation preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOperationGit {
    /// Whether repositories are synchronized (fetch + fast-forward pull).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The branch that is checked out when set; otherwise each service keeps
    /// its configured branch (`tasks/032.md` §24).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// The library options of a saved operation preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOperationLibraries {
    /// Whether the required libraries are built in dependency order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// The migration options of a saved operation preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOperationMigration {
    /// Whether migrations run at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The named migration preset; defaults to each service's configured
    /// preset when omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// The build options of a saved operation preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOperationBuild {
    /// Whether services are built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// The build operation applied to every selected service, for example
    /// `build` or `test`. Provider-incompatible choices are rejected by the
    /// application, not silently ignored (`tasks/032.md` §27).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

/// The runtime options of a saved operation preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOperationRuntime {
    /// The lifecycle action: `none`, `start`, `restart`, or `stop`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// The health options of a saved operation preset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOperationHealth {
    /// Whether configured health checks run after startup/restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}
