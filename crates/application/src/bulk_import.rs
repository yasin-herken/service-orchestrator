//! Bulk service import models and template matching (`tasks/032.md` §7–§15).
//!
//! This module holds the interface contract for the import flow:
//!
//! - the discovery report the wizard reviews;
//! - the selection and override values the wizard sends back;
//! - the per-repository plan rows and the whole-document preview;
//! - the pure template-matching rule that turns detected technologies into a
//!   *suggested* template — never a silent classification
//!   (`tasks/032.md` §8, §9).
//!
//! The use cases live in
//! [`queries::discovery`](crate::queries) and [`commands::bulk_import`]:
//! planning is read-only, committing validates the whole proposed document and
//! writes it atomically through the shared configuration manager.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use service_orchestrator_config::{
    ConfigurationPreview, ConfiguredWorkspace, EntityKind, ServiceTemplate, TemplateCategory,
};
use service_orchestrator_domain::{
    Branch, BuildOperation, BuildSystem, MigrationPresetId, MigrationTargetEnvironment, Port,
    RelativePath, Repository, Runtime, ServiceCapability, ServiceType, TemplateError, TemplateId,
    WorkspaceId,
};

use crate::ports::{DetectionConfidence, DiscoveredRepository, TechnologyDetection};

// ---------------------------------------------------------------------------
// Template matching (`tasks/032.md` §9)
// ---------------------------------------------------------------------------

/// The outcome of turning detections into a template category.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "match")]
pub enum CategoryMatch {
    /// Exactly one category is the strongest signal.
    Unique(TemplateCategory),
    /// Several categories tie; the user must choose.
    Ambiguous {
        /// The tied categories, in a stable order.
        categories: Vec<TemplateCategory>,
    },
    /// No category could be inferred from the detections.
    None,
}

impl CategoryMatch {
    /// Returns the unique category, if there is one.
    #[must_use]
    pub fn category(&self) -> Option<&TemplateCategory> {
        match self {
            Self::Unique(category) => Some(category),
            Self::Ambiguous { .. } | Self::None => None,
        }
    }
}

/// Infers the template category a repository's detections suggest.
///
/// The rule is deliberately simple and conservative
/// (`tasks/032.md` §9):
///
/// - a detected Spring Boot integration suggests [`TemplateCategory::SpringBootBackend`];
/// - a detected React integration suggests [`TemplateCategory::ReactFrontend`];
/// - a Maven project that is neither suggests [`TemplateCategory::SharedMavenLibrary`];
/// - Gradle is ignored for matching: it is detected and reported, but no
///   execution provider exists, so it must never select an executable template;
/// - a tie between equally strong signals yields
///   [`CategoryMatch::Ambiguous`] instead of an arbitrary pick.
#[must_use]
pub fn suggested_category(detections: &[TechnologyDetection]) -> CategoryMatch {
    let spring_boot = TechnologyDetection::find(detections, "spring_boot");
    let react = TechnologyDetection::find(detections, "react");
    let maven = TechnologyDetection::find(detections, "maven");

    let mut candidates: Vec<(TemplateCategory, DetectionConfidence)> = Vec::new();
    if let Some(detection) = spring_boot {
        candidates.push((TemplateCategory::SpringBootBackend, detection.confidence));
    }
    if let Some(detection) = react {
        candidates.push((TemplateCategory::ReactFrontend, detection.confidence));
    }
    if spring_boot.is_none() {
        if let Some(detection) = maven {
            candidates.push((TemplateCategory::SharedMavenLibrary, detection.confidence));
        }
    }

    let Some(best) = candidates.iter().map(|(_, confidence)| *confidence).max() else {
        return CategoryMatch::None;
    };
    let mut strongest: Vec<TemplateCategory> = candidates
        .into_iter()
        .filter(|(_, confidence)| *confidence == best)
        .map(|(category, _)| category)
        .collect();
    match strongest.len() {
        0 => CategoryMatch::None,
        1 => CategoryMatch::Unique(strongest.remove(0)),
        _ => CategoryMatch::Ambiguous {
            categories: strongest,
        },
    }
}

/// The templates that declare `category`.
#[must_use]
pub fn templates_for_category(
    templates: &[ServiceTemplate],
    category: &TemplateCategory,
) -> Vec<TemplateId> {
    templates
        .iter()
        .filter(|template| template.category.as_ref() == Some(category))
        .map(|template| template.id.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Discovery report (`tasks/032.md` §7, §9, §13)
// ---------------------------------------------------------------------------

/// An existing configured entity that occupies a discovered path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredEntityMatch {
    /// Whether the entity is a service or a library.
    pub kind: EntityKind,
    /// The configured entity id.
    pub id: String,
}

/// A discovered repository annotated with matching and registration state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredRepositoryView {
    /// The discovered repository and its detections.
    pub repository: DiscoveredRepository,
    /// The category the detections suggest, when unambiguous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_category: Option<TemplateCategory>,
    /// Equally strong categories that tied; the user must choose.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ambiguous_categories: Vec<TemplateCategory>,
    /// The template proposed for import, when the match is unique.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_template: Option<TemplateId>,
    /// Every template that shares the suggested category; more than one means
    /// the user must pick.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_templates: Vec<TemplateId>,
    /// Existing configured entities at the same workspace path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub existing: Vec<ConfiguredEntityMatch>,
}

impl DiscoveredRepositoryView {
    /// Returns `true` when the repository is already configured.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        !self.existing.is_empty()
    }
}

/// Indexes the configured services and libraries of `workspace` by their
/// workspace-relative directory.
///
/// Both entity `path` and the repository checkout path participate: a
/// discovered directory is *already configured* when either matches, because
/// both point into the same workspace tree (`tasks/032.md` §13). The index is
/// keyed on the normalized relative path so trailing separators do not defeat
/// the comparison.
#[must_use]
pub(crate) fn configured_entities_by_path(
    workspace: &ConfiguredWorkspace,
) -> BTreeMap<String, Vec<ConfiguredEntityMatch>> {
    let mut index: BTreeMap<String, Vec<ConfiguredEntityMatch>> = BTreeMap::new();
    for service in &workspace.workspace.services {
        if let Some(path) = entity_directory(service.path.as_ref(), service.repository.as_ref()) {
            index.entry(path).or_default().push(ConfiguredEntityMatch {
                kind: EntityKind::Service,
                id: service.id.to_string(),
            });
        }
    }
    for library in &workspace.workspace.libraries {
        if let Some(path) = entity_directory(library.path.as_ref(), library.repository.as_ref()) {
            index.entry(path).or_default().push(ConfiguredEntityMatch {
                kind: EntityKind::Library,
                id: library.id.to_string(),
            });
        }
    }
    index
}

/// The workspace-relative directory of an entity: its working `path` when set,
/// otherwise the repository checkout path. Returns `None` when neither exists.
fn entity_directory(
    path: Option<&RelativePath>,
    repository: Option<&Repository>,
) -> Option<String> {
    let raw = path
        .map(RelativePath::as_str)
        .or_else(|| repository.map(|repository| repository.local_path.as_str()))?;
    Some(normalize_relative_path(raw))
}

/// Normalizes a workspace-relative path for comparison: `/` separators,
/// trailing separators removed, empty segments dropped.
#[must_use]
pub(crate) fn normalize_relative_path(raw: &str) -> String {
    raw.split(['/', '\\'])
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// A request to scan a workspace root for repositories.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryScanRequest {
    /// The workspace whose root is scanned.
    pub workspace_id: WorkspaceId,
    /// Overrides the configured maximum depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
    /// Extra directory names to exclude, on top of the built-in and configured
    /// exclusions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Overrides the configured symlink policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follow_symlinks: Option<bool>,
    /// Overrides the configured repository cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_repositories: Option<usize>,
}

impl DiscoveryScanRequest {
    /// Creates a scan request with the configured bounds.
    #[must_use]
    pub fn new(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            max_depth: None,
            exclude: Vec::new(),
            follow_symlinks: None,
            max_repositories: None,
        }
    }
}

/// The bounded result of a discovery scan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryDiscoveryReport {
    /// The scanned workspace.
    pub workspace_id: WorkspaceId,
    /// The workspace root that was scanned.
    pub root: String,
    /// The repositories, in deterministic traversal order.
    pub repositories: Vec<DiscoveredRepositoryView>,
    /// Whether the repository cap stopped the scan early.
    pub truncated: bool,
    /// Non-fatal observations from the scan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Bulk import (`tasks/032.md` §10–§15)
// ---------------------------------------------------------------------------

/// How an existing configured entity at a selected path is handled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingEntityPolicy {
    /// Leave the existing entity untouched; the row is skipped.
    #[default]
    Skip,
    /// Replace the existing entity. This overwrites configuration, so it
    /// requires explicit authorization (`tasks/032.md` §13).
    Update,
}

/// A library artifact coordinate supplied as a per-service override.
///
/// The values are plain strings so an invalid coordinate is reported as an
/// actionable per-repository error instead of a deserialization failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkImportArtifact {
    /// The Maven group id.
    pub group_id: String,
    /// The Maven artifact id.
    pub artifact_id: String,
    /// The artifact version.
    pub version: String,
    /// The packaging, for example `jar` or `pom`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packaging: Option<String>,
}

/// Bulk or per-service overrides for one imported repository.
///
/// The same type is used for the shared "bulk overrides" (wizard step 6) and
/// for the per-repository exceptions (wizard step 7). Precedence is
/// deterministic (`tasks/032.md` §6):
///
/// ```text
/// template defaults → detected repository metadata → bulk overrides → per-service overrides
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkImportOverrides {
    /// The entity id. Defaults to the sanitized directory name. A shared `name`
    /// that applies to several repositories is rejected as a duplicate id, and
    /// a per-service value disambiguates it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The service type, overriding the template's and the detected one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_type: Option<ServiceType>,
    /// The branch checked out by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<Branch>,
    /// The repository clone URL. Falls back to the `origin` remote detected in
    /// the checkout. A URL that embeds credentials is rejected by configuration
    /// validation (`AGENTS.md` §31).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_url: Option<String>,
    /// The build system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_system: Option<BuildSystem>,
    /// The build operation used for the build phase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_operation: Option<BuildOperation>,
    /// Runtime requirements; when non-empty they replace the template's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<Runtime>,
    /// Exposed ports; when non-empty they replace the assigned ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// The build command line, replacing the template's build command. Values
    /// may use the documented template variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_command: Option<String>,
    /// The start command line, replacing the template's start command. Values
    /// may use the documented template variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_command: Option<String>,
    /// Whether migrations run for this entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_enabled: Option<bool>,
    /// The named migration preset assigned to the entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_preset: Option<MigrationPresetId>,
    /// The declared database environment. Required for a destructive preset:
    /// the preset's `allowed_environments` must contain it, and it must never
    /// be inferred (`tasks/032.md` §19, §21).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_environment: Option<MigrationTargetEnvironment>,
    /// The declared database type, for display and preset scoping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<String>,
    /// The declared database name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_name: Option<String>,
    /// Replaces the template's health checks with a single HTTP check on this
    /// URL. Values may use the documented template variables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_url: Option<String>,
    /// The library artifact coordinate. Falls back to the detected Maven
    /// metadata for a Maven library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<BulkImportArtifact>,
    /// Extra capabilities beyond the type defaults.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<ServiceCapability>,
}

impl BulkImportOverrides {
    /// Merges `overrides` over these overrides: every field `overrides` sets
    /// wins, every field it leaves unset keeps this value
    /// (`tasks/032.md` §6, §11, §12).
    #[must_use]
    pub fn merged_with(&self, overrides: &BulkImportOverrides) -> BulkImportOverrides {
        BulkImportOverrides {
            name: overrides.name.clone().or_else(|| self.name.clone()),
            service_type: overrides
                .service_type
                .clone()
                .or_else(|| self.service_type.clone()),
            default_branch: overrides
                .default_branch
                .clone()
                .or_else(|| self.default_branch.clone()),
            repository_url: overrides
                .repository_url
                .clone()
                .or_else(|| self.repository_url.clone()),
            build_system: overrides.build_system.or(self.build_system),
            build_operation: overrides.build_operation.or(self.build_operation),
            runtimes: if overrides.runtimes.is_empty() {
                self.runtimes.clone()
            } else {
                overrides.runtimes.clone()
            },
            ports: if overrides.ports.is_empty() {
                self.ports.clone()
            } else {
                overrides.ports.clone()
            },
            build_command: overrides
                .build_command
                .clone()
                .or_else(|| self.build_command.clone()),
            start_command: overrides
                .start_command
                .clone()
                .or_else(|| self.start_command.clone()),
            migration_enabled: overrides.migration_enabled.or(self.migration_enabled),
            migration_preset: overrides
                .migration_preset
                .clone()
                .or_else(|| self.migration_preset.clone()),
            database_environment: overrides
                .database_environment
                .clone()
                .or_else(|| self.database_environment.clone()),
            database_type: overrides
                .database_type
                .clone()
                .or_else(|| self.database_type.clone()),
            database_name: overrides
                .database_name
                .clone()
                .or_else(|| self.database_name.clone()),
            health_url: overrides
                .health_url
                .clone()
                .or_else(|| self.health_url.clone()),
            artifact: overrides.artifact.clone().or_else(|| self.artifact.clone()),
            capabilities: if overrides.capabilities.is_empty() {
                self.capabilities.clone()
            } else {
                overrides.capabilities.clone()
            },
        }
    }
}

/// Returns `true` when any resolvable value of `template` references
/// `${port}`.
///
/// The scan covers exactly the values a template may use variables in:
/// command arguments, command working directories, command environment
/// values, HTTP health-check URLs, and template environment values. Command
/// *programs* cannot use variables, so they are not scanned
/// (`tasks/032.md` §3, §4).
///
/// # Errors
///
/// Returns a [`TemplateError`] when a scanned value contains an invalid
/// placeholder.
pub(crate) fn template_requires_port(template: &ServiceTemplate) -> Result<bool, TemplateError> {
    use service_orchestrator_domain::{requires_port, HealthCheckKind};
    for entry in template.commands.iter() {
        let spec = &entry.spec;
        for arg in &spec.args {
            if requires_port(arg)? {
                return Ok(true);
            }
        }
        if let Some(working_dir) = &spec.working_dir {
            if requires_port(working_dir.as_str())? {
                return Ok(true);
            }
        }
        for variable in spec.environment.iter() {
            if requires_port(&variable.value)? {
                return Ok(true);
            }
        }
    }
    for check in &template.health_checks {
        if let HealthCheckKind::Http(http) = &check.kind {
            if requires_port(&http.url)? {
                return Ok(true);
            }
        }
    }
    for variable in template.environment.iter() {
        if requires_port(&variable.value)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One repository selected for import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkImportSelection {
    /// The repository path relative to the workspace root.
    pub relative_path: String,
    /// The explicitly assigned template. When unset, the planner matches one
    /// automatically and only accepts an unambiguous match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateId>,
    /// How an existing configured entity at the same path is handled.
    #[serde(default)]
    pub existing: ExistingEntityPolicy,
    /// Per-service overrides.
    #[serde(default)]
    pub overrides: BulkImportOverrides,
}

/// A bulk import request: the selection plus the shared overrides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkImportRequest {
    /// The workspace the repositories are imported into.
    pub workspace_id: WorkspaceId,
    /// The selected repositories, in the caller's order.
    pub repositories: Vec<BulkImportSelection>,
    /// Shared overrides applied to every selected repository.
    #[serde(default)]
    pub bulk: BulkImportOverrides,
    /// The first port assigned to rows that need one. Ports are assigned
    /// sequentially in deterministic (path-sorted) order, skipping ports
    /// already configured in the workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_base: Option<Port>,
}

/// Where a row's template came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateSource {
    /// The caller assigned the template explicitly.
    Explicit,
    /// The template was matched automatically from the detections.
    Matched,
    /// No template is assigned or could be matched.
    None,
}

/// What the planner will do with a row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum BulkImportRowAction {
    /// A new entity is created.
    Create,
    /// An existing configured entity is replaced with the imported
    /// configuration.
    Update {
        /// The existing entity id.
        existing: String,
        /// The existing entity kind.
        kind: EntityKind,
    },
    /// The repository is already configured and is left untouched.
    Skip {
        /// The existing entity id.
        existing: String,
        /// The existing entity kind.
        kind: EntityKind,
    },
    /// The row cannot be imported; `errors` explains why.
    Reject,
}

/// A structured, actionable problem with one repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkImportIssue {
    /// A stable, machine-readable code such as `TEMPLATE_REQUIRED`.
    pub code: String,
    /// A human-readable, actionable explanation.
    pub message: String,
}

impl BulkImportIssue {
    /// Builds an issue.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// The proposed entity configuration for one row, rendered for review.
///
/// The value is the *raw* configuration entity the importer would write: the
/// same schema a hand-written `[[services]]`/`[[libraries]]` entry uses, so the
/// review shows exactly what will be persisted (`tasks/032.md` §10 step 7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "entity")]
pub enum BulkImportProposal {
    /// A proposed `[[services]]` entry.
    Service {
        /// The raw service entry.
        service: Box<service_orchestrator_config::raw::RawService>,
    },
    /// A proposed `[[libraries]]` entry.
    Library {
        /// The raw library entry.
        library: Box<service_orchestrator_config::raw::RawLibrary>,
    },
}

impl BulkImportProposal {
    /// The proposed entity id.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Service { service } => &service.id,
            Self::Library { library } => &library.id,
        }
    }

    /// The proposed entity kind.
    #[must_use]
    pub const fn kind(&self) -> EntityKind {
        match self {
            Self::Service { .. } => EntityKind::Service,
            Self::Library { .. } => EntityKind::Library,
        }
    }
}

/// The plan for one selected repository.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BulkImportRow {
    /// The repository path relative to the workspace root.
    pub relative_path: String,
    /// The absolute path that was inspected.
    pub absolute_path: String,
    /// The template applied, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<TemplateId>,
    /// Where the template came from.
    pub template_source: TemplateSource,
    /// The proposed entity kind, when one was determined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_kind: Option<EntityKind>,
    /// The proposed (or existing) entity id, when one was determined.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// What the planner will do with the row.
    pub action: BulkImportRowAction,
    /// The detected technologies, for review.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detections: Vec<TechnologyDetection>,
    /// The proposed ports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
    /// The proposed configuration, when the row is importable or skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<BulkImportProposal>,
    /// What the user must review before committing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Why the row cannot be imported; a non-empty list rejects the row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<BulkImportIssue>,
}

impl BulkImportRow {
    /// Returns `true` when the row can be imported.
    #[must_use]
    pub fn is_acceptable(&self) -> bool {
        self.errors.is_empty() && !matches!(self.action, BulkImportRowAction::Reject)
    }
}

/// The read-only plan for a bulk import.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BulkImportPlan {
    /// The workspace the import targets.
    pub workspace_id: WorkspaceId,
    /// The configuration revision the plan was computed against.
    pub revision: String,
    /// The per-repository rows, in deterministic (path-sorted) order.
    pub rows: Vec<BulkImportRow>,
    /// Whether every row is importable *and* the whole proposed document
    /// validates.
    pub valid: bool,
    /// The number of rows that would create an entity.
    pub creates: usize,
    /// The number of rows that would replace an existing entity.
    pub updates: usize,
    /// The number of rows that are skipped because they already exist.
    pub skips: usize,
    /// The number of rows that cannot be imported.
    pub rejects: usize,
    /// The whole-document preview: the same structured diff and validation the
    /// configuration screen shows (`tasks/032.md` §10 step 9, §14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<ConfigurationPreview>,
    /// Non-fatal observations about the batch itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl BulkImportPlan {
    /// The repository paths that would be imported.
    #[must_use]
    pub fn imported_paths(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|row| {
                matches!(
                    row.action,
                    BulkImportRowAction::Create | BulkImportRowAction::Update { .. }
                )
            })
            .map(|row| row.relative_path.as_str())
            .collect()
    }
}

/// A reference to an entity the import touched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkImportEntityRef {
    /// The entity kind.
    pub kind: EntityKind,
    /// The entity id.
    pub id: String,
}

/// The result of committing a bulk import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkImportCommit {
    /// The imported workspace.
    pub workspace_id: WorkspaceId,
    /// The configuration revision after the commit.
    pub revision: String,
    /// The services that were created.
    pub created_services: Vec<String>,
    /// The libraries that were created.
    pub created_libraries: Vec<String>,
    /// The existing entities that were replaced.
    pub updated: Vec<BulkImportEntityRef>,
    /// The existing entities that were skipped.
    pub skipped: Vec<BulkImportEntityRef>,
    /// The configuration counts after the commit.
    pub counts: crate::models::ConfigurationCounts,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::DetectionConfidence;
    use service_orchestrator_domain::{
        BuildSystem, CommandKind, CommandSet, CommandSpec, TemplateId,
    };

    fn detection(technology: &str, confidence: DetectionConfidence) -> TechnologyDetection {
        TechnologyDetection::new(technology, technology, confidence, Vec::new())
    }

    fn template(id: &str, category: Option<TemplateCategory>) -> ServiceTemplate {
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Build,
            CommandSpec::new("./mvnw", ["clean", "install"]).expect("a valid command"),
        );
        ServiceTemplate {
            id: TemplateId::new(id).expect("a valid id"),
            name: id.to_owned(),
            description: None,
            category,
            service_type: None,
            default_branch: None,
            build_system: Some(BuildSystem::Maven),
            build_operation: None,
            runtimes: Vec::new(),
            commands,
            migration: None,
            health_checks: Vec::new(),
            timeout_seconds: None,
            environment: service_orchestrator_domain::Environment::new(),
            capabilities: Vec::new(),
        }
    }

    #[test]
    fn spring_boot_suggests_the_backend_category() {
        let detections = [
            detection("maven", DetectionConfidence::High),
            detection("spring_boot", DetectionConfidence::High),
        ];
        assert_eq!(
            suggested_category(&detections),
            CategoryMatch::Unique(TemplateCategory::SpringBootBackend)
        );
    }

    #[test]
    fn react_suggests_the_frontend_category() {
        let detections = [
            detection("npm", DetectionConfidence::High),
            detection("react", DetectionConfidence::High),
        ];
        assert_eq!(
            suggested_category(&detections),
            CategoryMatch::Unique(TemplateCategory::ReactFrontend)
        );
    }

    #[test]
    fn pure_maven_suggests_the_library_category() {
        let detections = [detection("maven", DetectionConfidence::High)];
        assert_eq!(
            suggested_category(&detections),
            CategoryMatch::Unique(TemplateCategory::SharedMavenLibrary)
        );
    }

    #[test]
    fn equally_strong_signals_are_ambiguous_rather_than_guessed() {
        let detections = [
            detection("spring_boot", DetectionConfidence::High),
            detection("react", DetectionConfidence::High),
        ];
        assert_eq!(
            suggested_category(&detections),
            CategoryMatch::Ambiguous {
                categories: vec![
                    TemplateCategory::SpringBootBackend,
                    TemplateCategory::ReactFrontend
                ]
            }
        );
    }

    #[test]
    fn a_stronger_signal_wins_over_a_weaker_one() {
        let detections = [
            detection("spring_boot", DetectionConfidence::Medium),
            detection("react", DetectionConfidence::High),
        ];
        assert_eq!(
            suggested_category(&detections),
            CategoryMatch::Unique(TemplateCategory::ReactFrontend)
        );
    }

    #[test]
    fn gradle_alone_selects_nothing() {
        let detections = [detection("gradle", DetectionConfidence::High)];
        assert_eq!(suggested_category(&detections), CategoryMatch::None);
    }

    #[test]
    fn nothing_detected_selects_nothing() {
        assert_eq!(suggested_category(&[]), CategoryMatch::None);
    }

    #[test]
    fn a_spring_boot_maven_project_never_suggests_the_library_category() {
        let detections = [
            detection("maven", DetectionConfidence::High),
            detection("spring_boot", DetectionConfidence::Medium),
        ];
        assert_eq!(
            suggested_category(&detections),
            CategoryMatch::Unique(TemplateCategory::SpringBootBackend)
        );
    }

    #[test]
    fn templates_are_matched_within_their_category() {
        let templates = [
            template("backend", Some(TemplateCategory::SpringBootBackend)),
            template("frontend", Some(TemplateCategory::ReactFrontend)),
            template("uncategorized", None),
        ];
        assert_eq!(
            templates_for_category(&templates, &TemplateCategory::SpringBootBackend),
            vec![TemplateId::new("backend").expect("a valid id")]
        );
        assert!(
            templates_for_category(&templates, &TemplateCategory::SharedMavenLibrary).is_empty()
        );
    }

    #[test]
    fn the_plan_tracks_imported_paths() {
        let plan = BulkImportPlan {
            workspace_id: WorkspaceId::new("acme").expect("a valid id"),
            revision: "abc".to_owned(),
            rows: vec![
                BulkImportRow {
                    relative_path: "backend/auth".to_owned(),
                    absolute_path: "/projects/acme/backend/auth".to_owned(),
                    template: None,
                    template_source: TemplateSource::Matched,
                    entity_kind: Some(EntityKind::Service),
                    entity_id: Some("auth".to_owned()),
                    action: BulkImportRowAction::Create,
                    detections: Vec::new(),
                    ports: Vec::new(),
                    proposal: None,
                    warnings: Vec::new(),
                    errors: Vec::new(),
                },
                BulkImportRow {
                    relative_path: "backend/legacy".to_owned(),
                    absolute_path: "/projects/acme/backend/legacy".to_owned(),
                    template: None,
                    template_source: TemplateSource::None,
                    entity_kind: None,
                    entity_id: None,
                    action: BulkImportRowAction::Reject,
                    detections: Vec::new(),
                    ports: Vec::new(),
                    proposal: None,
                    warnings: Vec::new(),
                    errors: vec![BulkImportIssue::new(
                        "TEMPLATE_REQUIRED",
                        "assign a template",
                    )],
                },
            ],
            valid: false,
            creates: 1,
            updates: 0,
            skips: 0,
            rejects: 1,
            preview: None,
            warnings: Vec::new(),
        };
        assert_eq!(plan.imported_paths(), vec!["backend/auth"]);
    }
}
