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
//! This module is the only place in the crate that is shaped by TOML. Nothing
//! outside the crate ever sees these types.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The complete raw configuration file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawConfig {
    /// The schema version. Defaulted so a file that omits it is treated as v1;
    /// unknown versions are rejected by the loader.
    #[serde(default = "default_version")]
    pub(crate) config_version: u32,
    /// The declared workspaces.
    #[serde(default)]
    pub(crate) workspaces: Vec<RawWorkspace>,
    /// The declared services (across all workspaces).
    #[serde(default)]
    pub(crate) services: Vec<RawService>,
    /// The declared libraries (across all workspaces).
    #[serde(default)]
    pub(crate) libraries: Vec<RawLibrary>,
    /// The declared service groups.
    #[serde(default)]
    pub(crate) groups: Vec<RawGroup>,
    /// The declared developer profiles.
    #[serde(default)]
    pub(crate) profiles: Vec<RawProfile>,
    /// Global task execution settings.
    #[serde(default)]
    pub(crate) execution: RawExecution,
}

fn default_version() -> u32 {
    crate::model::DEFAULT_CONFIG_VERSION
}

/// A raw workspace.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawWorkspace {
    /// The stable identifier.
    pub(crate) id: String,
    /// The human-readable name (defaults to the id).
    pub(crate) name: Option<String>,
    /// The workspace root, which may begin with `~`.
    pub(crate) root: String,
    /// Workspace-wide environment variables.
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, RawEnvValue>,
}

/// A raw service.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawService {
    /// The stable identifier.
    pub(crate) id: String,
    /// The human-readable name (defaults to the id).
    pub(crate) name: Option<String>,
    /// `backend`, `frontend`, `library`, or `other:<name>`.
    #[serde(rename = "type")]
    pub(crate) service_type: String,
    /// The id of the owning workspace.
    pub(crate) workspace: String,
    /// The path relative to the workspace root.
    pub(crate) path: Option<String>,
    /// The branch checked out by default.
    pub(crate) default_branch: Option<String>,
    /// The source repository.
    pub(crate) repository: Option<RawRepository>,
    /// A single runtime requirement.
    pub(crate) runtime: Option<RawRuntime>,
    /// Additional runtime requirements.
    #[serde(default)]
    pub(crate) runtimes: Vec<RawRuntime>,
    /// The configured commands.
    #[serde(default)]
    pub(crate) commands: RawCommands,
    /// Optional Liquibase configuration.
    pub(crate) liquibase: Option<RawLiquibase>,
    /// Health checks.
    #[serde(default)]
    pub(crate) health_checks: Vec<RawHealthCheck>,
    /// Exposed ports.
    #[serde(default)]
    pub(crate) ports: Vec<RawPort>,
    /// Declared dependencies.
    #[serde(default)]
    pub(crate) dependencies: RawDependencies,
    /// Service environment variables.
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, RawEnvValue>,
    /// Extra capabilities beyond the type defaults.
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
}

/// A raw library.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLibrary {
    /// The stable identifier.
    pub(crate) id: String,
    /// The human-readable name (defaults to the id).
    pub(crate) name: Option<String>,
    /// The id of the owning workspace.
    pub(crate) workspace: String,
    /// The path relative to the workspace root.
    pub(crate) path: Option<String>,
    /// The branch checked out by default.
    pub(crate) default_branch: Option<String>,
    /// The artifact technology. Defaults to `maven`.
    #[serde(rename = "type")]
    pub(crate) library_type: Option<String>,
    /// The source repository.
    pub(crate) repository: Option<RawRepository>,
    /// The published artifact coordinate.
    pub(crate) artifact: Option<RawArtifact>,
    /// A single runtime requirement.
    pub(crate) runtime: Option<RawRuntime>,
    /// Additional runtime requirements.
    #[serde(default)]
    pub(crate) runtimes: Vec<RawRuntime>,
    /// The configured commands.
    #[serde(default)]
    pub(crate) commands: RawCommands,
    /// Declared dependencies (libraries only).
    #[serde(default)]
    pub(crate) dependencies: RawDependencies,
    /// Library environment variables.
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, RawEnvValue>,
}

/// A raw repository.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRepository {
    /// The clone URL.
    pub(crate) url: String,
    /// The default branch; falls back to the entity's `default_branch`.
    pub(crate) default_branch: Option<String>,
    /// The hosting provider.
    pub(crate) provider: Option<String>,
}

/// A raw runtime requirement.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawRuntime {
    /// `java`, `node`, `maven`, `npm`, or `other:<name>`.
    #[serde(rename = "type")]
    pub(crate) kind: String,
    /// The pinned version.
    pub(crate) version: Option<String>,
}

/// A raw artifact coordinate.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawArtifact {
    /// The group identifier.
    pub(crate) group_id: String,
    /// The artifact identifier.
    pub(crate) artifact_id: String,
    /// The artifact version.
    pub(crate) version: String,
}

/// The raw command table.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCommands {
    /// The install command.
    pub(crate) install: Option<RawCommand>,
    /// The build command.
    pub(crate) build: Option<RawCommand>,
    /// The start command.
    pub(crate) start: Option<RawCommand>,
    /// The stop command.
    pub(crate) stop: Option<RawCommand>,
    /// The restart command.
    pub(crate) restart: Option<RawCommand>,
    /// The test command.
    pub(crate) test: Option<RawCommand>,
    /// The Liquibase command.
    pub(crate) liquibase: Option<RawCommand>,
    /// Project-specific commands, keyed by name.
    #[serde(default)]
    pub(crate) custom: BTreeMap<String, RawCommand>,
}

/// A command, in either short string or structured form.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawCommand {
    /// A command line such as `"./mvnw clean install"`.
    Line(String),
    /// A structured command with an explicit program and arguments.
    Structured(RawStructuredCommand),
}

/// A structured command.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawStructuredCommand {
    /// The executable.
    pub(crate) program: String,
    /// The arguments.
    #[serde(default)]
    pub(crate) args: Vec<String>,
    /// The working directory, relative to the workspace.
    pub(crate) working_dir: Option<String>,
    /// Command-specific environment variables.
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, RawEnvValue>,
}

/// An environment variable value.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawEnvValue {
    /// A literal, non-secret value.
    Literal(String),
    /// A literal value with an explicit secrecy flag.
    Structured(RawEnvVariable),
}

/// A structured environment variable.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawEnvVariable {
    /// The literal value.
    pub(crate) value: String,
    /// Whether the value must be redacted when displayed.
    #[serde(default)]
    pub(crate) secret: bool,
}

/// Optional Liquibase configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawLiquibase {
    /// Whether Liquibase should run. Defaults to `true` when the block is
    /// present.
    pub(crate) enabled: Option<bool>,
    /// The migration command.
    pub(crate) command: Option<RawCommand>,
    /// The migration timeout.
    pub(crate) timeout_seconds: Option<u64>,
    /// Liquibase environment variables.
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, RawEnvValue>,
}

/// A raw health check.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawHealthCheck {
    /// `process`, `port`, or `http`.
    #[serde(rename = "type")]
    pub(crate) kind: String,
    /// The URL for an HTTP check.
    pub(crate) url: Option<String>,
    /// The expected HTTP status.
    pub(crate) expected_status: Option<u16>,
    /// The port for a port check.
    pub(crate) port: Option<u16>,
    /// The probe timeout.
    pub(crate) timeout_seconds: Option<u64>,
    /// The polling interval.
    pub(crate) interval_seconds: Option<u64>,
}

/// A raw port.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawPort {
    /// The port number.
    pub(crate) port: u16,
    /// `tcp` or `udp`. Defaults to `tcp`.
    pub(crate) protocol: Option<String>,
}

/// The raw dependency tables.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDependencies {
    /// Dependencies on other services.
    #[serde(default)]
    pub(crate) services: Vec<RawDependencyRef>,
    /// Dependencies on libraries.
    #[serde(default)]
    pub(crate) libraries: Vec<RawDependencyRef>,
}

/// A dependency reference, in short string or detailed form.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawDependencyRef {
    /// A bare identifier such as `"auth-service"`.
    Id(String),
    /// A reference with an explicit kind and required flag.
    Detailed(RawDetailedDependency),
}

/// A detailed dependency reference.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDetailedDependency {
    /// The target identifier.
    pub(crate) id: String,
    /// `build` or `runtime`.
    pub(crate) kind: Option<String>,
    /// Whether the dependency is mandatory.
    pub(crate) required: Option<bool>,
}

/// A raw service group.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawGroup {
    /// The stable identifier.
    pub(crate) id: String,
    /// The human-readable name (defaults to the id).
    pub(crate) name: Option<String>,
    /// An optional description.
    pub(crate) description: Option<String>,
    /// The owning workspace; inferred from members when omitted.
    pub(crate) workspace: Option<String>,
    /// The services in the group.
    #[serde(default)]
    pub(crate) services: Vec<String>,
}

/// A raw developer profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProfile {
    /// The stable identifier.
    pub(crate) id: String,
    /// The human-readable name (defaults to the id).
    pub(crate) name: Option<String>,
    /// An optional description.
    pub(crate) description: Option<String>,
    /// The owning workspace; inferred from members when omitted.
    pub(crate) workspace: Option<String>,
    /// The services in the profile.
    #[serde(default)]
    pub(crate) services: Vec<String>,
    /// The groups in the profile.
    #[serde(default)]
    pub(crate) groups: Vec<String>,
}

/// Global execution settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawExecution {
    /// The maximum number of parallel tasks.
    pub(crate) max_parallel_tasks: Option<usize>,
    /// The default operation timeout.
    pub(crate) default_timeout_seconds: Option<u64>,
}
