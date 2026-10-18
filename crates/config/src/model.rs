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

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{
    Branch, BuildOperation, BuildSystem, CommandSet, CommandSpec, Environment, GroupId,
    HealthCheck, Library, LibraryId, LiquibaseExecutionMode, MigrationOperation, MigrationPresetId,
    MigrationProviderType, MigrationTargetEnvironment, OperationPresetId, Profile, ProfileId,
    Runtime, Service, ServiceCapability, ServiceId, ServiceType, TemplateId, Workspace,
    WorkspaceId,
};

/// The configuration schema version understood by this build.
pub const SUPPORTED_CONFIG_VERSION: u32 = 1;

/// The version assumed when a file omits `config_version`.
pub const DEFAULT_CONFIG_VERSION: u32 = SUPPORTED_CONFIG_VERSION;

/// The default number of tasks executed in parallel.
pub const DEFAULT_MAX_PARALLEL_TASKS: usize = 4;

/// The default operation timeout, in seconds.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 1800;

/// The default directory depth below a workspace root that discovery walks.
pub const DEFAULT_DISCOVERY_MAX_DEPTH: usize = 6;

/// The default cap on the number of repositories discovery reports.
pub const DEFAULT_DISCOVERY_MAX_REPOSITORIES: usize = 200;

/// The deepest traversal discovery may be configured with.
///
/// The bound exists so a typo cannot turn a scan into an effectively unlimited
/// tree walk (`tasks/032.md` §7).
pub const MAX_DISCOVERY_DEPTH: usize = 64;

/// The largest repository cap discovery may be configured with.
pub const MAX_DISCOVERY_REPOSITORIES: usize = 10_000;

/// Directories discovery never descends into, regardless of configuration.
///
/// These are build outputs, dependency caches, and version-control metadata:
/// traversing them is both useless (nested repositories inside them are not
/// projects a developer wants to manage) and expensive.
pub const DEFAULT_DISCOVERY_EXCLUDE: [&str; 6] =
    [".git", "node_modules", "target", "build", "dist", "out"];

/// A fully validated configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The schema version the file declared.
    pub version: u32,
    /// Global task execution settings.
    pub execution: ExecutionSettings,
    /// Bounded repository-discovery settings.
    pub discovery: DiscoverySettings,
    /// The reusable service templates (`tasks/032.md` §3).
    pub templates: Vec<ServiceTemplate>,
    /// The user-configured named migration presets. The built-in presets
    /// (`status`, `update`, `reset_local`) are merged in by
    /// [`Config::migration_presets`] and may be overridden by id.
    pub migration_presets: Vec<MigrationPreset>,
    /// The saved bulk-operation presets (`tasks/032.md` §36).
    pub operation_presets: Vec<OperationPreset>,
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

    /// Returns the template with the given id, if present.
    #[must_use]
    pub fn template(&self, id: &TemplateId) -> Option<&ServiceTemplate> {
        self.templates.iter().find(|template| &template.id == id)
    }

    /// Returns every migration preset: the built-ins merged with the
    /// user-configured presets, where a user preset with a built-in id
    /// overrides that built-in (`tasks/032.md` §16).
    #[must_use]
    pub fn all_migration_presets(&self) -> Vec<MigrationPreset> {
        merge_migration_presets(&self.migration_presets)
    }

    /// Returns the named migration preset with the given id, if present.
    ///
    /// A user-configured preset of the same id overrides the built-in.
    #[must_use]
    pub fn migration_preset(&self, id: &MigrationPresetId) -> Option<MigrationPreset> {
        self.all_migration_presets()
            .into_iter()
            .find(|preset| &preset.id == id)
    }

    /// Returns the saved operation preset with the given id, if present.
    #[must_use]
    pub fn operation_preset(&self, id: &OperationPresetId) -> Option<&OperationPreset> {
        self.operation_presets
            .iter()
            .find(|preset| &preset.id == id)
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
    /// The named migration preset applied when neither the request nor the
    /// operation preset selects one (`tasks/032.md` §21).
    pub preset: Option<MigrationPresetId>,
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

// ---------------------------------------------------------------------------
// Repository discovery (`tasks/032.md` §7)
// ---------------------------------------------------------------------------

/// Bounded repository-discovery settings.
///
/// Discovery only reads well-known files; nothing here executes anything. Every
/// setting exists to keep a scan of a large or hostile tree predictable: a
/// depth limit, an exclusion list, an explicit symlink policy, and a cap on the
/// number of repositories reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverySettings {
    /// The maximum directory depth below the workspace root.
    pub max_depth: usize,
    /// The effective directory names excluded from traversal: the built-in
    /// [`DEFAULT_DISCOVERY_EXCLUDE`] entries plus the configured extras.
    pub exclude: Vec<String>,
    /// Whether symlinked directories may be followed. A followed link must
    /// still resolve inside the workspace root.
    pub follow_symlinks: bool,
    /// The maximum number of repositories reported.
    pub max_repositories: usize,
}

impl Default for DiscoverySettings {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_DISCOVERY_MAX_DEPTH,
            exclude: DEFAULT_DISCOVERY_EXCLUDE
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            follow_symlinks: true,
            max_repositories: DEFAULT_DISCOVERY_MAX_REPOSITORIES,
        }
    }
}

/// Returns the effective exclusion list: the built-in directories followed by
/// the configured extras, in order, without duplicates.
#[must_use]
pub fn effective_excludes(configured: &[String]) -> Vec<String> {
    let mut excludes: Vec<String> = DEFAULT_DISCOVERY_EXCLUDE
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    for name in configured {
        let name = name.trim();
        if !name.is_empty() && !excludes.iter().any(|existing| existing == name) {
            excludes.push(name.to_owned());
        }
    }
    excludes
}

// ---------------------------------------------------------------------------
// Service templates (`tasks/032.md` §3)
// ---------------------------------------------------------------------------

/// The category of a service template.
///
/// The category is what automatic matching (`tasks/032.md` §9) keys on: a
/// detected technology suggests a category, and a template of that category is
/// proposed. A template without a category is never matched automatically.
///
/// It serializes as its canonical label string so interfaces can round-trip it
/// through the client/daemon protocol without sharing the config crate's model
/// types (`tasks/032.md` §9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum TemplateCategory {
    /// A Spring Boot backend service.
    SpringBootBackend,
    /// A React frontend application.
    ReactFrontend,
    /// A shared Maven library.
    SharedMavenLibrary,
    /// A project-specific category. It is never matched automatically.
    Custom(String),
}

impl From<String> for TemplateCategory {
    fn from(value: String) -> Self {
        TemplateCategory::parse(&value).unwrap_or(TemplateCategory::Custom(value))
    }
}

impl From<TemplateCategory> for String {
    fn from(category: TemplateCategory) -> Self {
        category.label().to_owned()
    }
}

impl TemplateCategory {
    /// Returns the canonical, machine-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::SpringBootBackend => "spring_boot_backend",
            Self::ReactFrontend => "react_frontend",
            Self::SharedMavenLibrary => "shared_maven_library",
            Self::Custom(name) => name,
        }
    }

    /// Parses a category label.
    ///
    /// Accepts a small set of aliases for the built-in categories; any other
    /// non-empty value becomes [`TemplateCategory::Custom`], which is only ever
    /// used for manual selection.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "" => None,
            "spring_boot_backend" | "spring-boot-backend" | "spring_boot" => {
                Some(Self::SpringBootBackend)
            }
            "react_frontend" | "react-frontend" | "react" => Some(Self::ReactFrontend),
            "shared_maven_library" | "shared-maven-library" | "maven_library" | "maven-library" => {
                Some(Self::SharedMavenLibrary)
            }
            other => Some(Self::Custom(other.to_owned())),
        }
    }

    /// The service type this category suggests when a template does not declare
    /// one explicitly.
    ///
    /// A shared Maven library resolves to [`ServiceType::Library`], which the
    /// bulk-import planner maps to a `[[libraries]]` entry rather than a
    /// `[[services]]` one.
    #[must_use]
    pub const fn suggested_service_type(&self) -> Option<ServiceType> {
        match self {
            Self::SpringBootBackend => Some(ServiceType::Backend),
            Self::ReactFrontend => Some(ServiceType::Frontend),
            Self::SharedMavenLibrary => Some(ServiceType::Library),
            Self::Custom(_) => None,
        }
    }

    /// Returns `true` for the three built-in categories.
    #[must_use]
    pub const fn is_builtin(&self) -> bool {
        !matches!(self, Self::Custom(_))
    }
}

/// The migration defaults a template may carry.
///
/// Only *how* migration runs is templated. The target database, its
/// environment, and every credential stay on the service (`tasks/032.md` §21).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateMigration {
    /// Whether migrations should run for a service created from the template.
    pub enabled: bool,
    /// The migration provider.
    pub provider: MigrationProviderType,
    /// How Liquibase is invoked, when the provider is Liquibase.
    pub execution_mode: LiquibaseExecutionMode,
    /// The named migration preset applied to created services.
    pub preset: Option<MigrationPresetId>,
}

impl Default for TemplateMigration {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: MigrationProviderType::Liquibase,
            execution_mode: LiquibaseExecutionMode::Maven,
            preset: None,
        }
    }
}

/// A reusable set of defaults for a service (`tasks/032.md` §3).
///
/// A template is *never* a second source of truth: it holds only values that
/// resolve into the existing validated service model, and the bulk-import
/// planner composes a candidate service that is validated by the same loader
/// and validator as a hand-written one. Precedence is deterministic
/// (`tasks/032.md` §6):
///
/// ```text
/// template defaults → detected repository metadata → explicit user overrides
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceTemplate {
    /// The stable identifier.
    pub id: TemplateId,
    /// The human-readable name.
    pub name: String,
    /// An optional description.
    pub description: Option<String>,
    /// The category, when the template may be matched automatically.
    pub category: Option<TemplateCategory>,
    /// The suggested service type. Falls back to the category's suggestion.
    pub service_type: Option<ServiceType>,
    /// The default branch checked out for a matching repository.
    pub default_branch: Option<Branch>,
    /// The build system.
    pub build_system: Option<BuildSystem>,
    /// The build operation used for the service build phase.
    pub build_operation: Option<BuildOperation>,
    /// Runtime requirements.
    pub runtimes: Vec<Runtime>,
    /// The configured commands; string values may use template variables.
    pub commands: CommandSet,
    /// Migration defaults; the database target stays on the service.
    pub migration: Option<TemplateMigration>,
    /// Health checks; an HTTP URL may use template variables.
    pub health_checks: Vec<HealthCheck>,
    /// The operation timeout, in seconds.
    pub timeout_seconds: Option<u64>,
    /// Environment variables; values may use template variables.
    pub environment: Environment,
    /// Extra capabilities beyond the type defaults.
    pub capabilities: Vec<ServiceCapability>,
}

impl ServiceTemplate {
    /// Returns the service type the template implies, if any.
    #[must_use]
    pub fn resolved_service_type(&self) -> Option<ServiceType> {
        self.service_type.clone().or_else(|| {
            self.category
                .as_ref()
                .and_then(TemplateCategory::suggested_service_type)
        })
    }
}

// ---------------------------------------------------------------------------
// Named migration presets (`tasks/032.md` §16)
// ---------------------------------------------------------------------------

/// The identifiers of the built-in migration presets.
pub const BUILTIN_MIGRATION_PRESETS: [&str; 3] = ["status", "update", "reset_local"];

/// A named, reusable migration preset.
///
/// A preset is deliberately **not** an unrestricted command: it names validated
/// Liquibase goals that the provider composes into a structured Maven argument
/// vector, plus the contexts to run with. It can be destructive, in which case
/// it must declare the environments it may target — and that declaration is
/// *not* authorization (`tasks/032.md` §19, §37). A destructive preset still
/// requires explicit, interactive authorization for the concrete database
/// target at execution time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationPreset {
    /// The stable identifier.
    pub id: MigrationPresetId,
    /// The human-readable name.
    pub name: String,
    /// An optional description.
    pub description: Option<String>,
    /// The migration provider.
    pub provider: MigrationProviderType,
    /// The execution mode. Only [`LiquibaseExecutionMode::Maven`] is accepted:
    /// a preset composes Maven goals.
    pub execution_mode: LiquibaseExecutionMode,
    /// The validated Liquibase goals, for example `["liquibase:update"]`.
    pub goals: Vec<String>,
    /// The Liquibase contexts. Empty means the service's declared contexts
    /// apply.
    pub contexts: Vec<String>,
    /// Whether the preset destroys database objects.
    pub destructive: bool,
    /// Whether the preset only inspects the database.
    pub read_only: bool,
    /// The environments a destructive preset may target. Required and
    /// non-empty for a destructive preset; empty otherwise.
    pub allowed_environments: Vec<MigrationTargetEnvironment>,
    /// The migration timeout, in seconds.
    pub timeout_seconds: Option<u64>,
    /// Whether this preset is one of the built-ins (false for a user preset
    /// that overrides a built-in id).
    pub builtin: bool,
}

/// Merges the built-in migration presets with the configured ones.
///
/// The built-ins come first, in their canonical order. A configured preset
/// with a built-in id replaces that built-in in place; a configured preset with
/// a new id is appended (`tasks/032.md` §16).
#[must_use]
pub fn merge_migration_presets(configured: &[MigrationPreset]) -> Vec<MigrationPreset> {
    let mut presets = MigrationPreset::builtins();
    for configured in configured {
        match presets.iter_mut().find(|preset| preset.id == configured.id) {
            Some(existing) => *existing = configured.clone(),
            None => presets.push(configured.clone()),
        }
    }
    presets
}

impl MigrationPreset {
    /// Returns the built-in presets: `status`, `update`, and `reset_local`.
    ///
    /// The built-ins are the baseline vocabulary every installation shares; a
    /// user preset with the same id overrides the built-in entirely
    /// ([`Config::migration_presets`]).
    #[must_use]
    pub fn builtins() -> Vec<MigrationPreset> {
        let id = |value: &str| {
            MigrationPresetId::new(value).expect("built-in migration preset ids are valid")
        };
        vec![
            MigrationPreset {
                id: id("status"),
                name: "Status".to_owned(),
                description: Some(
                    "Show pending Liquibase change sets without modifying the database.".to_owned(),
                ),
                provider: MigrationProviderType::Liquibase,
                execution_mode: LiquibaseExecutionMode::Maven,
                goals: vec!["liquibase:status".to_owned()],
                contexts: Vec::new(),
                destructive: false,
                read_only: true,
                allowed_environments: Vec::new(),
                timeout_seconds: None,
                builtin: true,
            },
            MigrationPreset {
                id: id("update"),
                name: "Update".to_owned(),
                description: Some("Apply pending Liquibase change sets.".to_owned()),
                provider: MigrationProviderType::Liquibase,
                execution_mode: LiquibaseExecutionMode::Maven,
                goals: vec!["liquibase:update".to_owned()],
                contexts: Vec::new(),
                destructive: false,
                read_only: false,
                allowed_environments: Vec::new(),
                timeout_seconds: None,
                builtin: true,
            },
            MigrationPreset {
                id: id("reset_local"),
                name: "Reset Local Database".to_owned(),
                description: Some(
                    "Drop every object in a local database and re-apply all change sets. \
                     Destructive: requires explicit authorization for each target."
                        .to_owned(),
                ),
                provider: MigrationProviderType::Liquibase,
                execution_mode: LiquibaseExecutionMode::Maven,
                goals: vec![
                    "liquibase:dropAll".to_owned(),
                    "liquibase:update".to_owned(),
                ],
                contexts: Vec::new(),
                destructive: true,
                read_only: false,
                allowed_environments: vec![MigrationTargetEnvironment::Local],
                timeout_seconds: None,
                builtin: true,
            },
        ]
    }

    /// Returns `true` when `id` is the id of a built-in preset.
    #[must_use]
    pub fn is_builtin_id(id: &MigrationPresetId) -> bool {
        BUILTIN_MIGRATION_PRESETS.contains(&id.as_str())
    }

    /// Returns `true` when the preset may target `environment`.
    ///
    /// A non-destructive preset has no environment restriction. A destructive
    /// preset only runs against a declared environment on its list; this is a
    /// *precondition*, never authorization.
    #[must_use]
    pub fn allows_environment(&self, environment: &MigrationTargetEnvironment) -> bool {
        !self.destructive || self.allowed_environments.contains(environment)
    }

    /// The contexts to run with, falling back to `service_contexts` when the
    /// preset declares none (`tasks/032.md` §17).
    #[must_use]
    pub fn contexts_for<'a>(&'a self, service_contexts: &'a [String]) -> &'a [String] {
        if self.contexts.is_empty() {
            service_contexts
        } else {
            &self.contexts
        }
    }
}

// ---------------------------------------------------------------------------
// Saved bulk-operation presets (`tasks/032.md` §36)
// ---------------------------------------------------------------------------

/// The runtime lifecycle action a saved operation preset selects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuntimeAction {
    /// Services are not started, restarted, or stopped.
    #[default]
    None,
    /// Start the selected services in dependency order.
    Start,
    /// Restart the selected services.
    Restart,
    /// Stop the selected services in reverse dependency order.
    Stop,
}

impl RuntimeAction {
    /// Returns the canonical, machine-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Start => "start",
            Self::Restart => "restart",
            Self::Stop => "stop",
        }
    }

    /// Parses an action label.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "none" => Some(Self::None),
            "start" => Some(Self::Start),
            "restart" => Some(Self::Restart),
            "stop" => Some(Self::Stop),
            _ => None,
        }
    }

    /// Returns `true` when the action changes process state.
    #[must_use]
    pub const fn is_lifecycle(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// The Git options of a saved operation preset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationGitOptions {
    /// Whether repositories are synchronized (fetch and fast-forward pull).
    pub enabled: bool,
    /// The branch that is checked out when set; otherwise each service keeps
    /// its configured branch.
    pub branch: Option<Branch>,
}

/// The library options of a saved operation preset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationLibraryOptions {
    /// Whether the required libraries are built in dependency order.
    pub enabled: bool,
}

/// The migration options of a saved operation preset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationMigrationOptions {
    /// Whether migrations run at all.
    pub enabled: bool,
    /// The named migration preset applied to every selected service.
    pub preset: Option<MigrationPresetId>,
}

/// The build options of a saved operation preset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationBuildOptions {
    /// Whether services are built.
    pub enabled: bool,
    /// The build operation applied to every selected service.
    pub operation: Option<BuildOperation>,
}

/// The runtime options of a saved operation preset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationRuntimeOptions {
    /// The lifecycle action.
    pub action: RuntimeAction,
}

/// The health options of a saved operation preset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperationHealthOptions {
    /// Whether configured health checks run after startup.
    pub enabled: bool,
}

/// A saved bulk-operation selection (`tasks/032.md` §36).
///
/// A preset stores *what a developer commonly runs*: the selected entities and
/// the phase options. It never stores credentials and never carries
/// authorization; loading one pre-fills the Bulk Operations screen and
/// execution still runs the full validation and policy of a freshly composed
/// request (`tasks/032.md` §37).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationPreset {
    /// The stable identifier.
    pub id: OperationPresetId,
    /// The human-readable name.
    pub name: String,
    /// An optional description.
    pub description: Option<String>,
    /// The owning workspace, when declared.
    pub workspace: Option<WorkspaceId>,
    /// Whether this is the default preset for its workspace.
    pub default: bool,
    /// Explicitly selected services.
    pub services: Vec<ServiceId>,
    /// Selected profiles.
    pub profiles: Vec<ProfileId>,
    /// Selected groups.
    pub groups: Vec<GroupId>,
    /// Git options.
    pub git: OperationGitOptions,
    /// Library options.
    pub libraries: OperationLibraryOptions,
    /// Migration options.
    pub migration: OperationMigrationOptions,
    /// Build options.
    pub build: OperationBuildOptions,
    /// Runtime options.
    pub runtime: OperationRuntimeOptions,
    /// Health options.
    pub health: OperationHealthOptions,
}
