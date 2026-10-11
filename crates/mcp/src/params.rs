//! Strict, typed input schemas for every MCP tool.
//!
//! Each struct in this module is the *only* accepted shape for one tool's
//! `arguments`. They are deliberately strict:
//!
//! - `deny_unknown_fields` rejects any property the tool does not model, so a
//!   client cannot smuggle an unmodelled option past validation;
//! - identifiers stay as [`String`] here and are parsed into validated domain
//!   ids at dispatch time, so a malformed identifier becomes a structured
//!   `INVALID_ARGUMENT` error rather than a protocol failure;
//! - enums are modelled as enums, never as free-form strings
//!   (`tasks/014.md` §22, §23).
//!
//! The schemas are generated from these types by `schemars` and advertised
//! through `tools/list`, so the schema a client sees and the shape the adapter
//! accepts can never drift apart.

use schemars::JsonSchema;
use serde::Deserialize;
use service_orchestrator_application::{
    HealthCheckScope, LibraryPrepareOptions, LogLevel, LogSource,
};
use service_orchestrator_domain::{LibraryId, WorkspaceId};

/// The argument shape of a tool that takes no arguments.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Empty {}

/// `workspaceId` only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceRef {
    /// The workspace identifier.
    pub(crate) workspace_id: String,
}

/// `serviceId` only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServiceRef {
    /// The service identifier.
    pub(crate) service_id: String,
}

/// `libraryId` only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LibraryRef {
    /// The library identifier.
    pub(crate) library_id: String,
}

/// `groupId` only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GroupRef {
    /// The service-group identifier.
    pub(crate) group_id: String,
}

/// `profileId` only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProfileRef {
    /// The developer-profile identifier.
    pub(crate) profile_id: String,
}

/// `taskId` only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskRef {
    /// The task identifier.
    pub(crate) task_id: String,
}

/// `workflowId` only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkflowRef {
    /// The workflow identifier.
    pub(crate) workflow_id: String,
}

/// A workspace and an explicit list of its services.
///
/// An empty `serviceIds` list is passed through unchanged: in the application
/// core an empty selection means "already satisfied" for state-changing bulk
/// commands, and the adapter must not reinterpret it as "every service"
/// (`tasks/014.md` §20).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceServices {
    /// The workspace identifier.
    pub(crate) workspace_id: String,
    /// The selected service identifiers.
    #[serde(default)]
    pub(crate) service_ids: Vec<String>,
}

/// A workspace, an explicit service list, and a branch.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkspaceServicesBranch {
    /// The workspace identifier.
    pub(crate) workspace_id: String,
    /// The selected service identifiers.
    #[serde(default)]
    pub(crate) service_ids: Vec<String>,
    /// The branch to check out.
    pub(crate) branch: String,
}

/// A service and a branch.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServiceBranch {
    /// The service identifier.
    pub(crate) service_id: String,
    /// The branch to check out.
    pub(crate) branch: String,
}

/// A workspace and one of its service groups.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GroupAction {
    /// The workspace identifier.
    pub(crate) workspace_id: String,
    /// The service-group identifier.
    pub(crate) group_id: String,
}

/// A workspace and one of its developer profiles.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProfileAction {
    /// The workspace identifier.
    pub(crate) workspace_id: String,
    /// The developer-profile identifier.
    pub(crate) profile_id: String,
}

/// A service and a read-only Git diff selection.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServiceDiff {
    /// The service identifier.
    pub(crate) service_id: String,
    /// Whether to diff staged changes instead of unstaged changes.
    #[serde(default)]
    pub(crate) staged: bool,
    /// The maximum number of patch bytes to return.
    #[serde(default)]
    pub(crate) max_bytes: Option<usize>,
}

/// Health-check request.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HealthCheckRequest {
    /// The service identifier.
    pub(crate) service_id: String,
    /// Which configured checks to run. Defaults to every check.
    #[serde(default)]
    pub(crate) scope: HealthScope,
}

/// The health-check scope, as accepted over MCP.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HealthScope {
    /// Run every configured health check.
    #[default]
    All,
    /// Run only process checks.
    Process,
    /// Run only TCP/port checks.
    Tcp,
    /// Run only HTTP/actuator checks.
    Http,
}

/// A log severity filter, as accepted over MCP.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Level {
    /// Trace and above.
    Trace,
    /// Debug and above.
    Debug,
    /// Info and above.
    Info,
    /// Warn and above.
    Warn,
    /// Error only.
    Error,
}

/// A log source filter, as accepted over MCP.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    /// Output captured from a managed process.
    Process,
    /// A task executor message.
    Task,
    /// A runtime lifecycle event.
    Runtime,
    /// A Git operation.
    Git,
    /// A build operation.
    Build,
    /// A library operation.
    Library,
    /// A database migration operation.
    Migration,
    /// An application-level message.
    Application,
}

/// Service log query.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ServiceLogsRequest {
    /// The service identifier.
    pub(crate) service_id: String,
    /// Restrict to one workspace.
    #[serde(default)]
    pub(crate) workspace_id: Option<String>,
    /// Only return entries at or above this severity.
    #[serde(default)]
    pub(crate) level: Option<Level>,
    /// Restrict to one source.
    #[serde(default)]
    pub(crate) source: Option<Source>,
    /// A case-insensitive substring the message must contain.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Return only entries after this cursor.
    #[serde(default)]
    pub(crate) after: Option<u64>,
    /// The maximum number of entries to return.
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

/// Task log query.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskLogsRequest {
    /// The task identifier.
    pub(crate) task_id: String,
    /// Restrict to one workspace.
    #[serde(default)]
    pub(crate) workspace_id: Option<String>,
    /// Only return entries at or above this severity.
    #[serde(default)]
    pub(crate) level: Option<Level>,
    /// Restrict to one source.
    #[serde(default)]
    pub(crate) source: Option<Source>,
    /// A case-insensitive substring the message must contain.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Return only entries after this cursor.
    #[serde(default)]
    pub(crate) after: Option<u64>,
    /// The maximum number of entries to return.
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

/// General log query.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LogsRequest {
    /// Restrict to one task.
    #[serde(default)]
    pub(crate) task_id: Option<String>,
    /// Restrict to one workspace.
    #[serde(default)]
    pub(crate) workspace_id: Option<String>,
    /// Only return entries at or above this severity.
    #[serde(default)]
    pub(crate) level: Option<Level>,
    /// Restrict to one source.
    #[serde(default)]
    pub(crate) source: Option<Source>,
    /// A case-insensitive substring the message must contain.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Return only entries after this cursor.
    #[serde(default)]
    pub(crate) after: Option<u64>,
    /// The maximum number of entries to return.
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

/// A `list_tasks` bound.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListTasksRequest {
    /// Return at most this many most-recent tasks.
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

impl From<Level> for LogLevel {
    fn from(level: Level) -> Self {
        match level {
            Level::Trace => Self::Trace,
            Level::Debug => Self::Debug,
            Level::Info => Self::Info,
            Level::Warn => Self::Warn,
            Level::Error => Self::Error,
        }
    }
}

impl From<Source> for LogSource {
    fn from(source: Source) -> Self {
        match source {
            Source::Process => Self::Process,
            Source::Task => Self::Task,
            Source::Runtime => Self::Runtime,
            Source::Git => Self::Git,
            Source::Build => Self::Build,
            Source::Library => Self::Library,
            Source::Migration => Self::Migration,
            Source::Application => Self::Application,
        }
    }
}

impl From<HealthScope> for HealthCheckScope {
    fn from(scope: HealthScope) -> Self {
        match scope {
            HealthScope::All => Self::All,
            HealthScope::Process => Self::Process,
            HealthScope::Tcp => Self::Tcp,
            HealthScope::Http => Self::Http,
        }
    }
}

/// The phases of a library preparation, inlined on both prepare tools.
///
/// Inlined rather than `#[serde(flatten)]`ed because serde forbids `flatten`
/// together with `deny_unknown_fields`.
///
/// A phase that is absent keeps the application default (`true`); an explicit
/// `false` disables it.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LibraryPhases {
    /// Whether to synchronize the repository first. Defaults to `true`.
    #[serde(default)]
    pub(crate) sync: Option<bool>,
    /// A branch to check out after synchronizing.
    #[serde(default)]
    pub(crate) branch: Option<String>,
    /// Whether to build the library. Defaults to `true`.
    #[serde(default)]
    pub(crate) build: Option<bool>,
    /// Whether to install the artifact locally. Defaults to `true`.
    #[serde(default)]
    pub(crate) install: Option<bool>,
    /// Whether to verify the artifact afterwards. Defaults to `true`.
    #[serde(default)]
    pub(crate) verify: Option<bool>,
}

impl LibraryPhases {
    /// Materializes the phase selection into application options.
    pub(crate) fn into_options(self) -> Result<LibraryPrepareOptions, crate::ToolError> {
        let mut options = LibraryPrepareOptions::default();
        if let Some(sync) = self.sync {
            options = options.with_sync(sync);
        }
        if let Some(build) = self.build {
            options = options.with_build(build);
        }
        if let Some(install) = self.install {
            options = options.with_install(install);
        }
        if let Some(verify) = self.verify {
            options = options.with_verify(verify);
        }
        if let Some(branch) = self.branch {
            options = options.with_branch(crate::parse_branch(&branch)?);
        }
        Ok(options)
    }
}

/// `prepare_library` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareLibraryRequest {
    /// The library identifier.
    pub(crate) library_id: String,
    /// Whether to synchronize the repository first. Defaults to `true`.
    #[serde(default)]
    pub(crate) sync: Option<bool>,
    /// A branch to check out after synchronizing.
    #[serde(default)]
    pub(crate) branch: Option<String>,
    /// Whether to build the library. Defaults to `true`.
    #[serde(default)]
    pub(crate) build: Option<bool>,
    /// Whether to install the artifact locally. Defaults to `true`.
    #[serde(default)]
    pub(crate) install: Option<bool>,
    /// Whether to verify the artifact afterwards. Defaults to `true`.
    #[serde(default)]
    pub(crate) verify: Option<bool>,
}

impl PrepareLibraryRequest {
    /// Splits the request into a library id and phase options.
    pub(crate) fn into_parts(self) -> Result<(LibraryId, LibraryPrepareOptions), crate::ToolError> {
        let library_id = crate::parse_library_id(&self.library_id)?;
        let options = LibraryPhases {
            sync: self.sync,
            branch: self.branch,
            build: self.build,
            install: self.install,
            verify: self.verify,
        }
        .into_options()?;
        Ok((library_id, options))
    }
}

/// `prepare_selected_libraries` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareSelectedLibrariesRequest {
    /// The workspace identifier.
    pub(crate) workspace_id: String,
    /// The libraries to prepare. Empty selects every library in the workspace.
    #[serde(default)]
    pub(crate) library_ids: Vec<String>,
    /// Whether to synchronize the repository first. Defaults to `true`.
    #[serde(default)]
    pub(crate) sync: Option<bool>,
    /// A branch to check out after synchronizing.
    #[serde(default)]
    pub(crate) branch: Option<String>,
    /// Whether to build the libraries. Defaults to `true`.
    #[serde(default)]
    pub(crate) build: Option<bool>,
    /// Whether to install the artifacts locally. Defaults to `true`.
    #[serde(default)]
    pub(crate) install: Option<bool>,
    /// Whether to verify the artifacts afterwards. Defaults to `true`.
    #[serde(default)]
    pub(crate) verify: Option<bool>,
}

impl PrepareSelectedLibrariesRequest {
    /// Splits the request into a workspace, library ids, and phase options.
    #[allow(clippy::type_complexity)]
    pub(crate) fn into_parts(
        self,
    ) -> Result<(WorkspaceId, Vec<LibraryId>, LibraryPrepareOptions), crate::ToolError> {
        let workspace_id = crate::parse_workspace_id(&self.workspace_id)?;
        let libraries = self
            .library_ids
            .iter()
            .map(|id| crate::parse_library_id(id))
            .collect::<Result<Vec<_>, _>>()?;
        let options = LibraryPhases {
            sync: self.sync,
            branch: self.branch,
            build: self.build,
            install: self.install,
            verify: self.verify,
        }
        .into_options()?;
        Ok((workspace_id, libraries, options))
    }
}

/// `prepare_environment` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PrepareEnvironmentRequest {
    /// The workspace identifier.
    pub(crate) workspace_id: String,
    /// The services to include. Empty selects every service in the workspace.
    #[serde(default)]
    pub(crate) service_ids: Vec<String>,
    /// A branch to check out before building.
    #[serde(default)]
    pub(crate) branch: Option<String>,
    /// Whether to run Liquibase migrations.
    #[serde(default)]
    pub(crate) run_liquibase: bool,
    /// Whether to build libraries and services.
    #[serde(default)]
    pub(crate) build: bool,
    /// Whether to start the selected services.
    #[serde(default)]
    pub(crate) start: bool,
}
