//! Bulk import planning and commit (`tasks/032.md` §10–§15).
//!
//! Planning is **read-only**: it inspects each selected repository, resolves
//! the template, composes the exact raw entity that would be written, and asks
//! the configuration manager to preview the whole proposed document without
//! persisting anything. Committing re-runs the plan — so the persisted entities
//! are always produced from fresh observations — and then applies every change
//! through the shared configuration manager's atomic batch transaction.
//!
//! Precedence is deterministic (`tasks/032.md` §6):
//!
//! ```text
//! per-service overrides → bulk overrides → template defaults → detected metadata
//! ```
//!
//! Nothing here executes a project script, builds, migrates, or clones; the
//! only side effect of a commit is the atomic configuration write, followed by
//! the reload and broadcast the daemon performs (`tasks/032.md` §15, §50).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_config::raw::{
    RawArtifact, RawCommand, RawCommands, RawDependencies, RawEnvValue, RawEnvVariable,
    RawHealthCheck, RawLibrary, RawLiquibase, RawMigrationDatabase, RawPort, RawRepository,
    RawRuntime, RawService, RawStructuredCommand,
};
use service_orchestrator_config::{
    Config, ConfigChange, ConfigurationSnapshot, ConfiguredWorkspace, EntityChange, EntityKind,
    ServiceTemplate, ValidationError,
};
use service_orchestrator_domain::{
    resolve_template_text, BuildSystem, CommandKind, CommandSpec, HealthCheckKind, LibraryId,
    MigrationTargetEnvironment, Permission, Port, ServiceCapability, ServiceId, ServiceType,
    TemplateContext, TemplateError,
};

use crate::app::Application;
use crate::bulk_import::{
    configured_entities_by_path, normalize_relative_path, suggested_category,
    template_requires_port, templates_for_category, BulkImportCommit, BulkImportEntityRef,
    BulkImportIssue, BulkImportOverrides, BulkImportPlan, BulkImportProposal, BulkImportRequest,
    BulkImportRow, BulkImportRowAction, BulkImportSelection, CategoryMatch, ConfiguredEntityMatch,
    ExistingEntityPolicy, TemplateSource,
};
use crate::error::ApplicationError;
use crate::models::ConfigurationCounts;
use crate::ports::{RepositoryInspection, RepositoryInspector, TechnologyDetection};

impl Application {
    /// Plans a bulk import without changing anything.
    ///
    /// The plan contains one row per selected repository in deterministic
    /// (path-sorted) order, the composed entity each row would write, the
    /// whole-document preview, and per-row errors attributed back from
    /// configuration validation (`tasks/032.md` §10, §14).
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] for an unknown
    /// workspace, [`ApplicationError::PermissionDenied`] without read
    /// permission, and [`ApplicationError::Configuration`] (code
    /// `CONFIG_CONFLICT`) when the configuration file changes while planning.
    /// Per-repository problems are *not* errors: they are reported on the rows
    /// so the wizard can show them.
    pub fn plan_bulk_import(
        &self,
        request: BulkImportRequest,
    ) -> Result<BulkImportPlan, ApplicationError> {
        self.authorize(Permission::Read)?;
        let workspace = self.require_workspace(&request.workspace_id)?;
        let snapshot = self
            .configuration()
            .snapshot()
            .map_err(ApplicationError::config)?;
        let mut planner = Planner::new(self, workspace, &snapshot);

        let mut warnings = Vec::new();
        let selections = dedupe_selections(&request.repositories, &mut warnings);

        // Pass 1: inspect and prepare every row.
        let mut rows: Vec<PreparedRow> = selections
            .into_iter()
            .map(|selection| planner.prepare_row(selection, &request.bulk))
            .collect();

        // Pass 2: assign ports deterministically in path order.
        planner.assign_ports(&mut rows, &request);

        // Pass 3: compose the raw entity for every importable row.
        for row in &mut rows {
            planner.compose_row(row);
        }

        // Pass 4: build the batch, preview it, and attribute validation errors.
        let (changes, index_map) = planner.build_changes(&rows);
        let preview = if changes.is_empty() {
            None
        } else {
            Some(
                self.configuration()
                    .preview_batch(&changes, &snapshot.revision)
                    .map_err(ApplicationError::config)?,
            )
        };
        if let Some(preview) = &preview {
            warnings.extend(attribute_issues(
                &mut rows,
                &index_map,
                &snapshot,
                &preview.issues,
            ));
        }

        let rows: Vec<BulkImportRow> = rows.into_iter().map(PreparedRow::finish).collect();
        let creates = rows
            .iter()
            .filter(|row| {
                matches!(row.action, BulkImportRowAction::Create) && row.errors.is_empty()
            })
            .count();
        let updates = rows
            .iter()
            .filter(|row| {
                matches!(row.action, BulkImportRowAction::Update { .. }) && row.errors.is_empty()
            })
            .count();
        let skips = rows
            .iter()
            .filter(|row| matches!(row.action, BulkImportRowAction::Skip { .. }))
            .count();
        let rejects = rows.len() - creates - updates - skips;
        let valid = !rows.is_empty()
            && rejects == 0
            && preview.as_ref().is_none_or(|preview| preview.valid);

        Ok(BulkImportPlan {
            workspace_id: request.workspace_id,
            revision: snapshot.revision,
            rows,
            valid,
            creates,
            updates,
            skips,
            rejects,
            preview,
            warnings,
        })
    }

    /// Commits a bulk import atomically.
    ///
    /// The request is re-planned against the current configuration, the whole
    /// proposed document is validated again, and every change is applied in one
    /// atomic batch transaction that refuses to run if the file changed since
    /// the caller previewed it (`tasks/032.md` §15): `expected_revision` is the
    /// revision of the plan the user confirmed. A stale revision is reported as
    /// a conflict instead of silently importing onto a different configuration.
    ///
    /// # Errors
    ///
    /// Requires [`Permission::SafeWrite`], or [`Permission::Destructive`] when
    /// any row replaces an existing entity, because an update overwrites
    /// hand-written configuration (`tasks/032.md` §13). Returns
    /// [`ApplicationError::ConfigurationInvalid`] with an actionable summary
    /// when the plan is not valid, and [`ApplicationError::Configuration`]
    /// (code `CONFIG_CONFLICT`) when the file changed concurrently.
    pub fn commit_bulk_import(
        &self,
        request: BulkImportRequest,
        expected_revision: &str,
    ) -> Result<BulkImportCommit, ApplicationError> {
        let permission = if request
            .repositories
            .iter()
            .any(|selection| selection.existing == ExistingEntityPolicy::Update)
        {
            Permission::Destructive
        } else {
            Permission::SafeWrite
        };
        self.authorize(permission)?;

        let plan = self.plan_bulk_import(request)?;
        if !plan.valid {
            return Err(ApplicationError::ConfigurationInvalid {
                message: summarize_plan_problems(&plan),
            });
        }
        let changes = changes_for_plan(&plan);
        let config = if changes.is_empty() {
            // Every row was skipped, so there is nothing to persist.
            Arc::clone(&self.configuration().current())
        } else {
            Arc::new(
                self.configuration()
                    .commit_batch(&changes, expected_revision)
                    .map_err(ApplicationError::config)?,
            )
        };

        let mut created_services = Vec::new();
        let mut created_libraries = Vec::new();
        let mut updated = Vec::new();
        let mut skipped = Vec::new();
        for row in &plan.rows {
            match (&row.action, &row.proposal) {
                (BulkImportRowAction::Create, Some(BulkImportProposal::Service { .. })) => {
                    created_services.extend(row.entity_id.clone());
                }
                (BulkImportRowAction::Create, Some(BulkImportProposal::Library { .. })) => {
                    created_libraries.extend(row.entity_id.clone());
                }
                (BulkImportRowAction::Update { existing, kind }, _) => {
                    updated.push(BulkImportEntityRef {
                        kind: *kind,
                        id: existing.clone(),
                    });
                }
                (BulkImportRowAction::Skip { existing, kind }, _) => {
                    skipped.push(BulkImportEntityRef {
                        kind: *kind,
                        id: existing.clone(),
                    });
                }
                _ => {}
            }
        }

        Ok(BulkImportCommit {
            workspace_id: plan.workspace_id,
            revision: self.configuration().revision(),
            created_services,
            created_libraries,
            updated,
            skipped,
            counts: ConfigurationCounts::of(&config),
        })
    }
}

// ---------------------------------------------------------------------------
// Planning internals
// ---------------------------------------------------------------------------

/// Shared, read-only state one planning pass works against.
struct Planner<'a> {
    inspector: &'a dyn RepositoryInspector,
    config: &'a Config,
    workspace: &'a ConfiguredWorkspace,
    existing_by_path: BTreeMap<String, Vec<ConfiguredEntityMatch>>,
    /// Port owners across the whole configuration, for conflict reporting.
    port_owners: BTreeMap<u16, String>,
    snapshot_service_count: usize,
    snapshot_library_count: usize,
    snapshot_service_ids: Vec<String>,
    snapshot_library_ids: Vec<String>,
}

impl<'a> Planner<'a> {
    fn new(
        app: &'a Application,
        workspace: &'a ConfiguredWorkspace,
        snapshot: &ConfigurationSnapshot,
    ) -> Self {
        let mut port_owners = BTreeMap::new();
        for configured in &app.config().workspaces {
            for service in &configured.workspace.services {
                for port in &service.ports {
                    port_owners
                        .entry(port.value())
                        .or_insert_with(|| format!("service '{}'", service.id));
                }
            }
        }
        Self {
            inspector: app.repository_inspector(),
            config: app.config(),
            workspace,
            existing_by_path: configured_entities_by_path(workspace),
            port_owners,
            snapshot_service_count: snapshot.services.len(),
            snapshot_library_count: snapshot.libraries.len(),
            snapshot_service_ids: snapshot
                .services
                .iter()
                .map(|service| service.id.clone())
                .collect(),
            snapshot_library_ids: snapshot
                .libraries
                .iter()
                .map(|library| library.id.clone())
                .collect(),
        }
    }

    /// Normalizes and resolves one selection.
    fn prepare_row(
        &self,
        selection: &BulkImportSelection,
        bulk: &BulkImportOverrides,
    ) -> PreparedRow {
        let relative_path = normalize_relative_path(&selection.relative_path);
        let root = self.workspace.workspace.root.as_str().trim_end_matches('/');
        let absolute_path = format!("{root}/{relative_path}");

        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        let inspection = match self.inspector.inspect(&absolute_path) {
            Ok(inspection) => inspection,
            Err(error) => {
                errors.push(BulkImportIssue::new(
                    "INSPECTION_FAILED",
                    format!("the repository could not be inspected: {}", error.message),
                ));
                RepositoryInspection::missing(&absolute_path)
            }
        };
        if !inspection.exists || !inspection.is_directory {
            errors.push(BulkImportIssue::new(
                "REPOSITORY_MISSING",
                format!("'{relative_path}' is not a directory below the workspace root"),
            ));
        }
        if inspection.exists && !inspection.is_git_repository {
            warnings.push(format!(
                "'{relative_path}' is not a Git repository; no origin remote could be read."
            ));
        }
        for system in &inspection.build_systems {
            if !system.availability.is_operational() {
                warnings.push(format!(
                    "Detected: {} — execution provider not installed; the entity will not be \
                     buildable until a supported build system is configured.",
                    system.label
                ));
            }
        }

        let detections = TechnologyDetection::detect(&inspection);
        let effective = bulk.merged_with(&selection.overrides);

        // Resolve the template: explicit, or an unambiguous automatic match.
        let (template, template_source) =
            self.resolve_template(selection, &detections, &mut errors, &mut warnings);

        // Resolve the entity type and kind.
        let service_type = effective
            .service_type
            .clone()
            .or_else(|| {
                template
                    .as_ref()
                    .and_then(ServiceTemplate::resolved_service_type)
            })
            .or_else(|| detected_service_type(&detections));
        if service_type.is_none() {
            errors.push(BulkImportIssue::new(
                "SERVICE_TYPE_REQUIRED",
                format!(
                    "no entity type could be determined for '{relative_path}'; set a type in the \
                     template or a per-service override"
                ),
            ));
        }
        let entity_kind = service_type.as_ref().map(|service_type| {
            if service_type == &ServiceType::Library {
                EntityKind::Library
            } else {
                EntityKind::Service
            }
        });

        // Match existing configured entities by path, not by display name.
        let existing = self
            .existing_by_path
            .get(&relative_path)
            .cloned()
            .unwrap_or_default();
        let mut action = match selection.existing {
            ExistingEntityPolicy::Skip if !existing.is_empty() => {
                let matched = &existing[0];
                BulkImportRowAction::Skip {
                    existing: matched.id.clone(),
                    kind: matched.kind,
                }
            }
            ExistingEntityPolicy::Update if !existing.is_empty() => match &entity_kind {
                Some(kind) => {
                    let matching: Vec<&ConfiguredEntityMatch> = existing
                        .iter()
                        .filter(|candidate| candidate.kind == *kind)
                        .collect();
                    match matching.as_slice() {
                        [] => BulkImportRowAction::Create,
                        [only] => BulkImportRowAction::Update {
                            existing: only.id.clone(),
                            kind: *kind,
                        },
                        _ => {
                            errors.push(BulkImportIssue::new(
                                "DUPLICATE_EXISTING_TARGET",
                                format!(
                                    "several existing {} entities occupy '{relative_path}'; \
                                     update them individually instead",
                                    kind.label()
                                ),
                            ));
                            BulkImportRowAction::Reject
                        }
                    }
                }
                None => BulkImportRowAction::Reject,
            },
            _ if entity_kind.is_some() => BulkImportRowAction::Create,
            _ => BulkImportRowAction::Reject,
        };

        // Decide the entity id: an explicit override, the existing id for an
        // update, or the sanitized repository directory name.
        let existing_id = match &action {
            BulkImportRowAction::Update { existing, .. } => Some(existing.clone()),
            _ => None,
        };
        let kind_for_id = entity_kind.or_else(|| existing.first().map(|matched| matched.kind));
        let id = kind_for_id.and_then(|kind| {
            resolve_id(
                &effective,
                &relative_path,
                kind,
                existing_id.as_deref(),
                &mut errors,
            )
        });

        // Update implies replacing exactly the matched entity; a rename is a
        // different operation (create + remove) and is refused here.
        if let (BulkImportRowAction::Update { existing, .. }, Some(resolved)) = (&action, &id) {
            if resolved != existing {
                errors.push(BulkImportIssue::new(
                    "RENAME_NOT_SUPPORTED",
                    format!(
                        "an update cannot rename '{existing}' to '{resolved}'; create the new \
                         entity and remove the old one instead"
                    ),
                ));
                action = BulkImportRowAction::Reject;
            }
        }

        // Branch precedence: per-row → bulk → template → detected checkout.
        let branch = effective
            .default_branch
            .clone()
            .map(|branch| branch.as_str().to_owned())
            .or_else(|| {
                template.as_ref().and_then(|template| {
                    template
                        .default_branch
                        .as_ref()
                        .map(|branch| branch.as_str().to_owned())
                })
            })
            .or_else(|| {
                inspection
                    .branch
                    .clone()
                    .filter(|branch| matches!(branch.as_str(), "master" | "main" | "develop"))
            });
        if branch.is_none()
            && matches!(
                action,
                BulkImportRowAction::Create | BulkImportRowAction::Update { .. }
            )
        {
            errors.push(BulkImportIssue::new(
                "BRANCH_REQUIRED",
                format!(
                    "no default branch could be determined for '{relative_path}'; set a branch \
                     in the template or a bulk override"
                ),
            ));
        }

        // Whether a port is required by any templated value.
        let mut requires_port = false;
        if let Some(template) = &template {
            match template_requires_port(template) {
                Ok(port_needed) => requires_port = port_needed,
                Err(error) => errors.push(BulkImportIssue::new(
                    "TEMPLATE_VARIABLE_INVALID",
                    format!(
                        "template '{}' has an invalid variable: {error}",
                        template.id
                    ),
                )),
            }
        }
        for value in [
            effective.build_command.as_deref(),
            effective.start_command.as_deref(),
            effective.health_url.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            match service_orchestrator_domain::requires_port(value) {
                Ok(true) => requires_port = true,
                Ok(false) => {}
                Err(error) => errors.push(BulkImportIssue::new(
                    "TEMPLATE_VARIABLE_INVALID",
                    format!("the override value '{value}' has an invalid variable: {error}"),
                )),
            }
        }

        // Skip rows keep their match and are not composed further; nothing is
        // written for them, so composition problems do not apply.
        if matches!(action, BulkImportRowAction::Skip { .. }) {
            errors.clear();
        } else if !errors.is_empty() {
            action = BulkImportRowAction::Reject;
        }

        // Warn when Liquibase files were detected but the row configures no
        // migration, so the developer decides rather than the planner.
        let detected_liquibase = TechnologyDetection::find(&detections, "liquibase").is_some();
        let migration_configured = effective.migration_enabled.is_some()
            || effective.migration_preset.is_some()
            || template
                .as_ref()
                .is_some_and(|template| template.migration.is_some());
        if detected_liquibase && !migration_configured {
            warnings.push(
                "Liquibase files were detected but no migration preset is assigned.".to_owned(),
            );
        }

        PreparedRow {
            relative_path,
            absolute_path,
            inspection,
            detections,
            template,
            template_source,
            effective,
            errors,
            warnings,
            entity_kind,
            service_type,
            id,
            branch,
            requires_port,
            assigned_port: None,
            proposal: None,
            action,
        }
    }

    /// Resolves the template for a selection: explicit or automatic.
    ///
    /// An automatic match is accepted only when the detections suggest exactly
    /// one category *and* exactly one template declares it; anything else is
    /// surfaced for the user to decide (`tasks/032.md` §9).
    fn resolve_template(
        &self,
        selection: &BulkImportSelection,
        detections: &[TechnologyDetection],
        errors: &mut Vec<BulkImportIssue>,
        warnings: &mut Vec<String>,
    ) -> (Option<ServiceTemplate>, TemplateSource) {
        if let Some(id) = &selection.template {
            return match self.config.template(id) {
                Some(template) => (Some(template.clone()), TemplateSource::Explicit),
                None => {
                    errors.push(BulkImportIssue::new(
                        "TEMPLATE_UNKNOWN",
                        format!("template '{id}' does not exist"),
                    ));
                    (None, TemplateSource::Explicit)
                }
            };
        }
        match suggested_category(detections) {
            CategoryMatch::Unique(category) => {
                let candidates = templates_for_category(&self.config.templates, &category);
                match candidates.as_slice() {
                    [] => {
                        warnings.push(format!(
                            "no template of category '{}' exists; assign one explicitly",
                            category.label()
                        ));
                        (None, TemplateSource::None)
                    }
                    [only] => {
                        let template = self
                            .config
                            .template(only)
                            .expect("the template list came from the configuration")
                            .clone();
                        (Some(template), TemplateSource::Matched)
                    }
                    many => {
                        errors.push(BulkImportIssue::new(
                            "TEMPLATE_AMBIGUOUS",
                            format!(
                                "{} templates of category '{}' match; assign one explicitly",
                                many.len(),
                                category.label()
                            ),
                        ));
                        (None, TemplateSource::None)
                    }
                }
            }
            CategoryMatch::Ambiguous { categories } => {
                warnings.push(format!(
                    "detections are ambiguous between {}; assign a template explicitly",
                    categories
                        .iter()
                        .map(|category| format!("'{}'", category.label()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                (None, TemplateSource::None)
            }
            CategoryMatch::None => (None, TemplateSource::None),
        }
    }

    /// Assigns ports deterministically in path order.
    ///
    /// Explicit per-row and bulk ports always win and are checked for
    /// conflicts. Rows that need a port but have none either take the next
    /// free port from the caller's base or are rejected with an actionable
    /// error (`tasks/032.md` §12, §14).
    fn assign_ports(&mut self, rows: &mut [PreparedRow], request: &BulkImportRequest) {
        let has_port_base = request.port_base.is_some();
        let mut next_port = request.port_base.map(Port::value);

        // Explicit ports first: they reserve their number for their row and
        // are validated against existing configuration and each other.
        for row in rows.iter_mut() {
            if !importable(row) {
                continue;
            }
            let mut seen_in_row = BTreeSet::new();
            for port in row.effective.ports.clone() {
                let value = port.value();
                if !seen_in_row.insert(value) {
                    continue;
                }
                if let Some(owner) = self.port_owners.get(&value) {
                    row.errors.push(BulkImportIssue::new(
                        "PORT_CONFLICT",
                        format!("port {value} is already used by {owner}"),
                    ));
                } else {
                    self.port_owners
                        .insert(value, format!("row '{}'", row.relative_path));
                }
            }
            if let Some(port) = row.effective.ports.first() {
                row.assigned_port = Some(port.value());
            }
            if !row.errors.is_empty() {
                row.action = BulkImportRowAction::Reject;
            }
        }

        for row in rows.iter_mut() {
            if !importable(row) {
                continue;
            }
            if row.assigned_port.is_some() {
                continue;
            }
            let wants_port = row.requires_port
                || (has_port_base && row.entity_kind == Some(EntityKind::Service));
            if !wants_port {
                continue;
            }
            let Some(mut candidate) = next_port else {
                row.errors.push(BulkImportIssue::new(
                    "PORT_REQUIRED",
                    format!(
                        "'{}' needs a port but none is assigned; set a bulk port base or a \
                         per-service port",
                        row.relative_path
                    ),
                ));
                row.action = BulkImportRowAction::Reject;
                continue;
            };
            while candidate < u16::MAX && self.port_owners.contains_key(&candidate) {
                candidate += 1;
            }
            if self.port_owners.contains_key(&candidate) {
                row.errors.push(BulkImportIssue::new(
                    "PORT_RANGE_EXHAUSTED",
                    "no free port is available at or above the configured port base",
                ));
                row.action = BulkImportRowAction::Reject;
                continue;
            }
            row.assigned_port = Some(candidate);
            self.port_owners
                .insert(candidate, format!("row '{}'", row.relative_path));
            next_port = candidate.checked_add(1).filter(|next| *next < u16::MAX);
        }
    }

    /// Composes the raw entity for an importable row.
    fn compose_row(&self, row: &mut PreparedRow) {
        if !importable(row) {
            return;
        }
        let (Some(template), Some(id), Some(branch)) = (&row.template, &row.id, &row.branch) else {
            row.action = BulkImportRowAction::Reject;
            return;
        };
        let context = TemplateContext {
            service_name: id.clone(),
            repository_name: repository_name(&row.relative_path),
            repository_path: row.relative_path.clone(),
            workspace_root: self
                .workspace
                .workspace
                .root
                .as_str()
                .trim_end_matches('/')
                .to_owned(),
            relative_path: row.relative_path.clone(),
            port: row.assigned_port,
        };
        let resolved_ports: Vec<Port> = match row.assigned_port {
            Some(assigned) => row
                .effective
                .ports
                .clone()
                .into_iter()
                .chain(Port::new(assigned).ok())
                .collect::<BTreeSet<Port>>()
                .into_iter()
                .collect(),
            None => row.effective.ports.clone(),
        };
        let input = ComposeInput {
            template,
            overrides: &row.effective,
            inspection: &row.inspection,
            relative_path: &row.relative_path,
            id,
            branch,
            ports: &resolved_ports,
            context: &context,
        };
        let result =
            match &row.service_type {
                Some(ServiceType::Library) => compose_library(self.config, self.workspace, &input)
                    .map(|library| BulkImportProposal::Library {
                        library: Box::new(library),
                    }),
                Some(service_type) => {
                    compose_service(self.config, self.workspace, service_type, &input).map(
                        |service| BulkImportProposal::Service {
                            service: Box::new(service),
                        },
                    )
                }
                None => Err(BulkImportIssue::new(
                    "SERVICE_TYPE_REQUIRED",
                    "no entity type could be determined for this repository".to_owned(),
                )),
            };
        match result {
            Ok(proposal) => {
                row.proposal = Some(proposal);
            }
            Err(issue) => {
                row.errors.push(issue);
                row.action = BulkImportRowAction::Reject;
            }
        }
    }

    /// Builds the change list and the index → row mapping used to attribute
    /// validation errors.
    fn build_changes(&self, rows: &[PreparedRow]) -> (Vec<ConfigChange>, IndexMap) {
        let mut changes = Vec::new();
        let mut index_map = IndexMap {
            service_rows: BTreeMap::new(),
            library_rows: BTreeMap::new(),
        };
        let mut service_next = self.snapshot_service_count;
        let mut library_next = self.snapshot_library_count;
        for (row_index, row) in rows.iter().enumerate() {
            let Some(proposal) = &row.proposal else {
                continue;
            };
            match (&row.action, proposal) {
                (BulkImportRowAction::Create, BulkImportProposal::Service { service }) => {
                    index_map.service_rows.insert(service_next, row_index);
                    service_next += 1;
                    changes.push(ConfigChange::Service(Box::new(EntityChange::Create {
                        entity: service.as_ref().clone(),
                    })));
                }
                (BulkImportRowAction::Create, BulkImportProposal::Library { library }) => {
                    index_map.library_rows.insert(library_next, row_index);
                    library_next += 1;
                    changes.push(ConfigChange::Library(Box::new(EntityChange::Create {
                        entity: library.as_ref().clone(),
                    })));
                }
                (
                    BulkImportRowAction::Update { existing, .. },
                    BulkImportProposal::Service { service },
                ) => {
                    if let Some(index) = self.snapshot_service_index(existing) {
                        index_map.service_rows.insert(index, row_index);
                    }
                    changes.push(ConfigChange::Service(Box::new(EntityChange::Update {
                        id: existing.clone(),
                        entity: service.as_ref().clone(),
                    })));
                }
                (
                    BulkImportRowAction::Update { existing, .. },
                    BulkImportProposal::Library { library },
                ) => {
                    if let Some(index) = self.snapshot_library_index(existing) {
                        index_map.library_rows.insert(index, row_index);
                    }
                    changes.push(ConfigChange::Library(Box::new(EntityChange::Update {
                        id: existing.clone(),
                        entity: library.as_ref().clone(),
                    })));
                }
                _ => {}
            }
        }
        (changes, index_map)
    }

    fn snapshot_service_index(&self, id: &str) -> Option<usize> {
        self.snapshot_service_ids
            .iter()
            .position(|existing| existing == id)
    }

    fn snapshot_library_index(&self, id: &str) -> Option<usize> {
        self.snapshot_library_ids
            .iter()
            .position(|existing| existing == id)
    }
}

/// A row under construction.
struct PreparedRow {
    relative_path: String,
    absolute_path: String,
    inspection: RepositoryInspection,
    detections: Vec<TechnologyDetection>,
    template: Option<ServiceTemplate>,
    template_source: TemplateSource,
    effective: BulkImportOverrides,
    errors: Vec<BulkImportIssue>,
    warnings: Vec<String>,
    entity_kind: Option<EntityKind>,
    service_type: Option<ServiceType>,
    id: Option<String>,
    branch: Option<String>,
    requires_port: bool,
    assigned_port: Option<u16>,
    proposal: Option<BulkImportProposal>,
    action: BulkImportRowAction,
}

impl PreparedRow {
    fn finish(self) -> BulkImportRow {
        let ports = match self.assigned_port {
            Some(port) => Port::new(port).ok().into_iter().collect(),
            None => self.effective.ports.clone(),
        };
        BulkImportRow {
            relative_path: self.relative_path,
            absolute_path: self.absolute_path,
            template: self.template.as_ref().map(|template| template.id.clone()),
            template_source: self.template_source,
            entity_kind: self.entity_kind,
            entity_id: self.id,
            action: self.action,
            detections: self.detections,
            ports,
            proposal: self.proposal,
            warnings: self.warnings,
            errors: self.errors,
        }
    }
}

/// Whether a row will be composed and possibly written.
fn importable(row: &PreparedRow) -> bool {
    matches!(
        row.action,
        BulkImportRowAction::Create | BulkImportRowAction::Update { .. }
    )
}

/// The directory name of a repository path.
fn repository_name(relative_path: &str) -> String {
    relative_path
        .rsplit('/')
        .next()
        .unwrap_or(relative_path)
        .to_owned()
}

/// Removes duplicate selections, keeping the first occurrence in path order
/// and reporting what was dropped.
fn dedupe_selections<'a>(
    selections: &'a [BulkImportSelection],
    warnings: &mut Vec<String>,
) -> Vec<&'a BulkImportSelection> {
    let mut ordered: Vec<&BulkImportSelection> = selections.iter().collect();
    ordered.sort_by(|left, right| {
        normalize_relative_path(&left.relative_path)
            .cmp(&normalize_relative_path(&right.relative_path))
    });
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for selection in ordered {
        let key = normalize_relative_path(&selection.relative_path);
        if seen.insert(key.clone()) {
            deduped.push(selection);
        } else {
            warnings.push(format!("duplicate selection '{key}' was ignored"));
        }
    }
    deduped
}

/// Attributes whole-document validation errors back to the rows that caused
/// them, using the deterministic index arithmetic of the batch.
///
/// Returns the issues that could not be attributed to a row, so the plan can
/// still surface them.
fn attribute_issues(
    rows: &mut [PreparedRow],
    index_map: &IndexMap,
    snapshot: &ConfigurationSnapshot,
    issues: &[ValidationError],
) -> Vec<String> {
    let mut unattributed = Vec::new();
    for issue in issues {
        let Some((table, index)) = parse_location(&issue.location) else {
            unattributed.push(render_issue(issue));
            continue;
        };
        let row_index = match table {
            "services" => index_map
                .service_rows
                .get(&index)
                .copied()
                .or_else(|| existing_row_for_service(rows, snapshot, index)),
            "libraries" => index_map
                .library_rows
                .get(&index)
                .copied()
                .or_else(|| existing_row_for_library(rows, snapshot, index)),
            _ => None,
        };
        match row_index {
            Some(row_index) => rows[row_index].errors.push(BulkImportIssue::new(
                "CONFIGURATION_INVALID",
                render_issue(issue),
            )),
            None => unattributed.push(render_issue(issue)),
        }
    }
    unattributed
}

/// Renders a validation error as one actionable line.
fn render_issue(issue: &ValidationError) -> String {
    match (&issue.expected, &issue.suggested_fix) {
        (_, Some(fix)) => format!("{}: {} (fix: {fix})", issue.location, issue.problem),
        (Some(expected), None) => {
            format!(
                "{}: {} (expected: {expected})",
                issue.location, issue.problem
            )
        }
        (None, None) => format!("{}: {}", issue.location, issue.problem),
    }
}

/// Finds the row whose update targets the existing service at `index`.
fn existing_row_for_service(
    rows: &[PreparedRow],
    snapshot: &ConfigurationSnapshot,
    index: usize,
) -> Option<usize> {
    let id = snapshot.services.get(index)?.id.as_str();
    rows.iter().position(|row| {
        matches!(
            &row.action,
            BulkImportRowAction::Update { existing, kind }
                if kind == &EntityKind::Service && existing == id
        )
    })
}

/// Finds the row whose update targets the existing library at `index`.
fn existing_row_for_library(
    rows: &[PreparedRow],
    snapshot: &ConfigurationSnapshot,
    index: usize,
) -> Option<usize> {
    let id = snapshot.libraries.get(index)?.id.as_str();
    rows.iter().position(|row| {
        matches!(
            &row.action,
            BulkImportRowAction::Update { existing, kind }
                if kind == &EntityKind::Library && existing == id
        )
    })
}

/// Parses a configuration location such as `services[4].runtime.version`.
fn parse_location(location: &str) -> Option<(&str, usize)> {
    let open = location.find('[')?;
    let close = location[open..].find(']')? + open;
    let table = &location[..open];
    let index = location[open + 1..close].parse().ok()?;
    Some((table, index))
}

/// Maps a planned row back into the configuration changes that write it.
fn changes_for_plan(plan: &BulkImportPlan) -> Vec<ConfigChange> {
    let mut changes = Vec::new();
    for row in &plan.rows {
        match (&row.action, &row.proposal) {
            (BulkImportRowAction::Create, Some(BulkImportProposal::Service { service })) => {
                changes.push(ConfigChange::Service(Box::new(EntityChange::Create {
                    entity: service.as_ref().clone(),
                })));
            }
            (BulkImportRowAction::Create, Some(BulkImportProposal::Library { library })) => {
                changes.push(ConfigChange::Library(Box::new(EntityChange::Create {
                    entity: library.as_ref().clone(),
                })));
            }
            (
                BulkImportRowAction::Update { existing, .. },
                Some(BulkImportProposal::Service { service }),
            ) => {
                changes.push(ConfigChange::Service(Box::new(EntityChange::Update {
                    id: existing.clone(),
                    entity: service.as_ref().clone(),
                })));
            }
            (
                BulkImportRowAction::Update { existing, .. },
                Some(BulkImportProposal::Library { library }),
            ) => {
                changes.push(ConfigChange::Library(Box::new(EntityChange::Update {
                    id: existing.clone(),
                    entity: library.as_ref().clone(),
                })));
            }
            _ => {}
        }
    }
    changes
}

/// Renders the row problems of an invalid plan into one actionable message.
fn summarize_plan_problems(plan: &BulkImportPlan) -> String {
    let mut problems = Vec::new();
    for row in &plan.rows {
        for issue in &row.errors {
            problems.push(format!("{}: {}", row.relative_path, issue.message));
        }
    }
    if problems.is_empty() {
        problems.push("the proposed configuration did not validate".to_owned());
    }
    problems.truncate(5);
    format!("the bulk import is not valid: {}", problems.join("; "))
}

/// The index → row mapping used to attribute validation errors.
struct IndexMap {
    service_rows: BTreeMap<usize, usize>,
    library_rows: BTreeMap<usize, usize>,
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

/// The resolved inputs one composed entity depends on.
struct ComposeInput<'a> {
    template: &'a ServiceTemplate,
    overrides: &'a BulkImportOverrides,
    inspection: &'a RepositoryInspection,
    relative_path: &'a str,
    id: &'a str,
    branch: &'a str,
    ports: &'a [Port],
    context: &'a TemplateContext,
}

/// Composes a `[[services]]` entry from a template, detections, and overrides.
fn compose_service(
    config: &Config,
    workspace: &ConfiguredWorkspace,
    service_type: &ServiceType,
    input: &ComposeInput<'_>,
) -> Result<RawService, BulkImportIssue> {
    let url = repository_url(input)?;
    let mut commands = commands_from_template(input.template, input.inspection, input.context)?;
    apply_command_override(
        &mut commands,
        CommandKind::Build,
        input.overrides.build_command.as_deref(),
        input.context,
    )?;
    apply_command_override(
        &mut commands,
        CommandKind::Start,
        input.overrides.start_command.as_deref(),
        input.context,
    )?;
    let build_system = input
        .overrides
        .build_system
        .or(input.template.build_system)
        .or_else(|| detected_build_system(input.inspection));
    Ok(RawService {
        id: input.id.to_owned(),
        name: None,
        service_type: service_type_string(service_type),
        workspace: workspace.workspace.id.to_string(),
        path: Some(input.relative_path.to_owned()),
        default_branch: Some(input.branch.to_owned()),
        repository: Some(RawRepository {
            url,
            default_branch: None,
            provider: None,
        }),
        runtime: None,
        runtimes: resolve_runtimes(input.template, input.overrides, input.inspection),
        commands,
        build_system: build_system.map(|system| system.label().to_owned()),
        build_operation: input
            .overrides
            .build_operation
            .or(input.template.build_operation)
            .map(|operation| operation.label().to_owned()),
        liquibase: compose_liquibase(config, input.template, input.overrides)?,
        health_checks: resolve_health_checks(input)?,
        ports: input
            .ports
            .iter()
            .map(|port| RawPort {
                port: port.value(),
                protocol: None,
            })
            .collect(),
        dependencies: RawDependencies::default(),
        environment: resolve_environment(input.template, input.context)?,
        capabilities: resolve_capabilities(input.template, input.overrides),
    })
}

/// Composes a `[[libraries]]` entry from a template, detections, and overrides.
fn compose_library(
    _config: &Config,
    workspace: &ConfiguredWorkspace,
    input: &ComposeInput<'_>,
) -> Result<RawLibrary, BulkImportIssue> {
    let url = repository_url(input)?;
    let mut commands = commands_from_template(input.template, input.inspection, input.context)?;
    apply_command_override(
        &mut commands,
        CommandKind::Build,
        input.overrides.build_command.as_deref(),
        input.context,
    )?;
    let artifact = resolve_artifact(input.overrides, input.inspection)?;
    let build_system = input
        .overrides
        .build_system
        .or(input.template.build_system)
        .or_else(|| detected_build_system(input.inspection));
    Ok(RawLibrary {
        id: input.id.to_owned(),
        name: None,
        workspace: workspace.workspace.id.to_string(),
        path: Some(input.relative_path.to_owned()),
        default_branch: Some(input.branch.to_owned()),
        library_type: Some("maven".to_owned()),
        repository: Some(RawRepository {
            url,
            default_branch: None,
            provider: None,
        }),
        artifact: Some(artifact),
        artifacts: Vec::new(),
        runtime: None,
        runtimes: resolve_runtimes(input.template, input.overrides, input.inspection),
        commands,
        build_system: build_system.map(|system| system.label().to_owned()),
        build_operation: input
            .overrides
            .build_operation
            .or(input.template.build_operation)
            .map(|operation| operation.label().to_owned()),
        dependencies: RawDependencies::default(),
        environment: resolve_environment(input.template, input.context)?,
    })
}

/// The clone URL of a composed entity: the override, then the detected origin.
fn repository_url(input: &ComposeInput<'_>) -> Result<String, BulkImportIssue> {
    input
        .overrides
        .repository_url
        .clone()
        .or_else(|| input.inspection.remote.clone())
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| {
            BulkImportIssue::new(
                "REPOSITORY_URL_REQUIRED",
                format!(
                    "no origin remote was found for '{}'; provide a repository URL override",
                    input.relative_path
                ),
            )
        })
}

/// The raw service-type spelling of a domain type.
fn service_type_string(service_type: &ServiceType) -> String {
    match service_type {
        ServiceType::Backend => "backend".to_owned(),
        ServiceType::Frontend => "frontend".to_owned(),
        ServiceType::Library => "library".to_owned(),
        ServiceType::Other(name) => format!("other:{name}"),
    }
}

/// The type implied by detections alone, when no template declares one.
fn detected_service_type(detections: &[TechnologyDetection]) -> Option<ServiceType> {
    let spring_boot = TechnologyDetection::find(detections, "spring_boot").is_some();
    let react = TechnologyDetection::find(detections, "react").is_some();
    let npm = TechnologyDetection::find(detections, "npm").is_some();
    if spring_boot {
        Some(ServiceType::Backend)
    } else if react || npm {
        Some(ServiceType::Frontend)
    } else {
        // A plain Maven project is deliberately ambiguous between a backend
        // service and a library; a template (or the user) decides.
        None
    }
}

/// The build system implied by detections: Maven first, then npm.
fn detected_build_system(inspection: &RepositoryInspection) -> Option<BuildSystem> {
    if inspection
        .build_systems
        .iter()
        .any(|system| system.system == "maven")
    {
        Some(BuildSystem::Maven)
    } else if inspection
        .build_systems
        .iter()
        .any(|system| system.system == "npm")
    {
        Some(BuildSystem::Npm)
    } else {
        None
    }
}

/// Resolves runtime requirements: overrides, then template, then detections.
fn resolve_runtimes(
    template: &ServiceTemplate,
    overrides: &BulkImportOverrides,
    inspection: &RepositoryInspection,
) -> Vec<RawRuntime> {
    if !overrides.runtimes.is_empty() {
        return overrides
            .runtimes
            .iter()
            .map(|runtime| RawRuntime {
                kind: runtime.kind.label().to_owned(),
                version: runtime
                    .version
                    .as_ref()
                    .map(|version| version.as_str().to_owned()),
            })
            .collect();
    }
    if !template.runtimes.is_empty() {
        return template
            .runtimes
            .iter()
            .map(|runtime| RawRuntime {
                kind: runtime.kind.label().to_owned(),
                version: runtime
                    .version
                    .as_ref()
                    .map(|version| version.as_str().to_owned()),
            })
            .collect();
    }
    let mut seen = BTreeSet::new();
    inspection
        .runtimes
        .iter()
        .filter(|runtime| matches!(runtime.kind.as_str(), "java" | "node" | "maven" | "npm"))
        .filter(|runtime| seen.insert(runtime.kind.clone()))
        .map(|runtime| RawRuntime {
            kind: runtime.kind.clone(),
            version: runtime.required_version.clone(),
        })
        .collect()
}

/// Resolves the configured commands of a template and fills gaps from
/// detections.
fn commands_from_template(
    template: &ServiceTemplate,
    inspection: &RepositoryInspection,
    context: &TemplateContext,
) -> Result<RawCommands, BulkImportIssue> {
    let mut commands = RawCommands::default();
    for entry in template.commands.iter() {
        let resolved = resolve_command(&entry.spec, context)?;
        insert_raw_command(&mut commands, &entry.kind, RawCommand::Structured(resolved));
    }
    if commands.build.is_none() {
        commands.build = detected_build_command(inspection);
    }
    if commands.start.is_none() {
        commands.start = detected_start_command(inspection);
    }
    Ok(commands)
}

/// Resolves one configured command's variable-bearing values.
fn resolve_command(
    spec: &CommandSpec,
    context: &TemplateContext,
) -> Result<RawStructuredCommand, BulkImportIssue> {
    let arg_error = |error: TemplateError| {
        BulkImportIssue::new(
            "TEMPLATE_RESOLUTION_FAILED",
            format!("command argument could not be resolved: {error}"),
        )
    };
    let args = spec
        .args
        .iter()
        .map(|arg| resolve_template_text(arg, context).map_err(arg_error))
        .collect::<Result<Vec<_>, _>>()?;
    let working_dir = spec
        .working_dir
        .as_ref()
        .map(|dir| resolve_template_text(dir.as_str(), context))
        .transpose()
        .map_err(|error| {
            BulkImportIssue::new(
                "TEMPLATE_RESOLUTION_FAILED",
                format!("working directory could not be resolved: {error}"),
            )
        })?;
    let mut environment = BTreeMap::new();
    for variable in spec.environment.iter() {
        let value = resolve_template_text(&variable.value, context).map_err(|error| {
            BulkImportIssue::new(
                "TEMPLATE_RESOLUTION_FAILED",
                format!(
                    "environment value '{}' could not be resolved: {error}",
                    variable.key
                ),
            )
        })?;
        environment.insert(
            variable.key.clone(),
            RawEnvValue::Structured(RawEnvVariable {
                value,
                secret: variable.secret,
            }),
        );
    }
    Ok(RawStructuredCommand {
        program: spec.program.clone(),
        args,
        working_dir,
        environment,
    })
}

/// Inserts a raw command under its semantic kind.
fn insert_raw_command(commands: &mut RawCommands, kind: &CommandKind, command: RawCommand) {
    match kind {
        CommandKind::Build => commands.build = Some(command),
        CommandKind::Start => commands.start = Some(command),
        CommandKind::Stop => commands.stop = Some(command),
        CommandKind::Restart => commands.restart = Some(command),
        CommandKind::Install => commands.install = Some(command),
        CommandKind::Test => commands.test = Some(command),
        CommandKind::Liquibase => commands.liquibase = Some(command),
        CommandKind::Other(name) => {
            commands.custom.insert(name.clone(), command);
        }
    }
}

/// Applies a command-line override, resolving its variables.
fn apply_command_override(
    commands: &mut RawCommands,
    kind: CommandKind,
    value: Option<&str>,
    context: &TemplateContext,
) -> Result<(), BulkImportIssue> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let resolved = resolve_template_text(value, context).map_err(|error| {
        BulkImportIssue::new(
            "TEMPLATE_RESOLUTION_FAILED",
            format!(
                "the {} command override could not be resolved: {error}",
                kind.label()
            ),
        )
    })?;
    insert_raw_command(commands, &kind, RawCommand::Line(resolved));
    Ok(())
}

/// A detected Maven or npm build command.
fn detected_build_command(inspection: &RepositoryInspection) -> Option<RawCommand> {
    if inspection
        .build_systems
        .iter()
        .any(|system| system.system == "maven")
    {
        let program = if inspection
            .maven
            .as_ref()
            .is_some_and(|maven| maven.wrapper_available)
        {
            "./mvnw"
        } else {
            "mvn"
        };
        return Some(structured_command(program, &["clean", "install"]));
    }
    if inspection
        .build_systems
        .iter()
        .any(|system| system.system == "npm")
        && inspection
            .node
            .as_ref()
            .is_some_and(|node| node.scripts.iter().any(|script| script == "build"))
    {
        return Some(structured_command("npm", &["run", "build"]));
    }
    None
}

/// A detected Maven Spring Boot or npm start command.
fn detected_start_command(inspection: &RepositoryInspection) -> Option<RawCommand> {
    let maven_spring_boot = inspection
        .maven
        .as_ref()
        .is_some_and(|maven| maven.spring_boot);
    if maven_spring_boot
        && inspection
            .build_systems
            .iter()
            .any(|system| system.system == "maven")
    {
        let program = if inspection
            .maven
            .as_ref()
            .is_some_and(|maven| maven.wrapper_available)
        {
            "./mvnw"
        } else {
            "mvn"
        };
        return Some(structured_command(program, &["spring-boot:run"]));
    }
    let scripts = inspection
        .node
        .as_ref()
        .map(|node| node.scripts.as_slice())
        .unwrap_or(&[]);
    for script in ["start", "dev"] {
        if scripts.iter().any(|candidate| candidate == script) {
            return Some(structured_command("npm", &["run", script]));
        }
    }
    None
}

/// Builds a structured raw command without a working directory.
fn structured_command(program: &str, args: &[&str]) -> RawCommand {
    RawCommand::Structured(RawStructuredCommand {
        program: program.to_owned(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        working_dir: None,
        environment: BTreeMap::new(),
    })
}

/// Resolves health checks: the override URL, then the template's checks.
fn resolve_health_checks(input: &ComposeInput<'_>) -> Result<Vec<RawHealthCheck>, BulkImportIssue> {
    if let Some(url) = input
        .overrides
        .health_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        let resolved = resolve_template_text(url, input.context).map_err(|error| {
            BulkImportIssue::new(
                "TEMPLATE_RESOLUTION_FAILED",
                format!("the health check URL could not be resolved: {error}"),
            )
        })?;
        return Ok(vec![RawHealthCheck {
            kind: "http".to_owned(),
            url: Some(resolved),
            expected_status: None,
            port: None,
            timeout_seconds: None,
            interval_seconds: None,
            required: Some(true),
        }]);
    }
    input
        .template
        .health_checks
        .iter()
        .map(|check| {
            let (kind, url, expected_status, port) = match &check.kind {
                HealthCheckKind::Process => ("process".to_owned(), None, None, None),
                HealthCheckKind::Http(http) => {
                    let resolved =
                        resolve_template_text(&http.url, input.context).map_err(|error| {
                            BulkImportIssue::new(
                                "TEMPLATE_RESOLUTION_FAILED",
                                format!("the health check URL could not be resolved: {error}"),
                            )
                        })?;
                    (
                        "http".to_owned(),
                        Some(resolved),
                        http.expected_status,
                        None,
                    )
                }
                HealthCheckKind::Port(port) => ("port".to_owned(), None, None, Some(port.value())),
                HealthCheckKind::Other(name) => (name.clone(), None, None, None),
            };
            Ok(RawHealthCheck {
                kind,
                url,
                expected_status,
                port,
                timeout_seconds: check.timeout.map(duration_seconds),
                interval_seconds: check.interval.map(duration_seconds),
                required: Some(check.required),
            })
        })
        .collect()
}

/// Whole seconds of a duration, saturating.
fn duration_seconds(duration: Duration) -> u64 {
    duration.as_secs()
}

/// Resolves the template's environment values.
fn resolve_environment(
    template: &ServiceTemplate,
    context: &TemplateContext,
) -> Result<BTreeMap<String, RawEnvValue>, BulkImportIssue> {
    let mut environment = BTreeMap::new();
    for variable in template.environment.iter() {
        let value = resolve_template_text(&variable.value, context).map_err(|error| {
            BulkImportIssue::new(
                "TEMPLATE_RESOLUTION_FAILED",
                format!(
                    "environment value '{}' could not be resolved: {error}",
                    variable.key
                ),
            )
        })?;
        environment.insert(
            variable.key.clone(),
            RawEnvValue::Structured(RawEnvVariable {
                value,
                secret: variable.secret,
            }),
        );
    }
    Ok(environment)
}

/// The capability labels of a composed entity: template plus overrides.
fn resolve_capabilities(
    template: &ServiceTemplate,
    overrides: &BulkImportOverrides,
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut labels = Vec::new();
    for capability in template
        .capabilities
        .iter()
        .chain(overrides.capabilities.iter())
    {
        if let Some(label) = capability_label(capability) {
            if seen.insert(label) {
                labels.push(label.to_owned());
            }
        }
    }
    labels
}

/// The configuration spelling of a capability.
fn capability_label(capability: &ServiceCapability) -> Option<&'static str> {
    match capability {
        ServiceCapability::Build => Some("build"),
        ServiceCapability::Run => Some("run"),
        ServiceCapability::Migrate => Some("migrate"),
        ServiceCapability::ProvideArtifact => Some("provide_artifact"),
        ServiceCapability::ServeHttp => Some("serve_http"),
    }
}

/// Composes the migration block of a service.
///
/// Only the template and the overrides decide *how* migration runs; the target
/// database stays declared on the service. A destructive preset is refused
/// unless the row declares a concrete environment the preset allows — and that
/// declaration is not authorization: execution still requires the interactive,
/// per-target confirmation (`tasks/032.md` §19, §21).
fn compose_liquibase(
    config: &Config,
    template: &ServiceTemplate,
    overrides: &BulkImportOverrides,
) -> Result<Option<RawLiquibase>, BulkImportIssue> {
    let template_migration = template.migration.as_ref();
    let requested = overrides.migration_enabled.is_some()
        || overrides.migration_preset.is_some()
        || overrides.database_environment.is_some()
        || overrides.database_type.is_some()
        || overrides.database_name.is_some()
        || template_migration.is_some();
    if !requested {
        return Ok(None);
    }
    let enabled = overrides
        .migration_enabled
        .or_else(|| template_migration.map(|migration| migration.enabled))
        .unwrap_or(true);
    let provider = template_migration
        .map(|migration| migration.provider.label().to_owned())
        .unwrap_or_else(|| "liquibase".to_owned());
    let execution_mode = template_migration
        .map(|migration| migration.execution_mode.label().to_owned())
        .unwrap_or_else(|| "maven".to_owned());
    let preset = overrides
        .migration_preset
        .clone()
        .or_else(|| template_migration.and_then(|migration| migration.preset.clone()));
    if let Some(preset_id) = &preset {
        let resolved = config.migration_preset(preset_id).ok_or_else(|| {
            BulkImportIssue::new(
                "MIGRATION_PRESET_UNKNOWN",
                format!("migration preset '{preset_id}' does not exist or is not usable"),
            )
        })?;
        if resolved.destructive {
            let environment = overrides
                .database_environment
                .clone()
                .unwrap_or(MigrationTargetEnvironment::Unknown);
            let allowed = environment != MigrationTargetEnvironment::Unknown
                && resolved.allowed_environments.contains(&environment);
            if !allowed {
                return Err(BulkImportIssue::new(
                    "DESTRUCTIVE_TARGET_NOT_AUTHORIZED",
                    format!(
                        "destructive migration preset '{preset_id}' requires a declared database \
                         environment that its `allowed_environments` list contains; declare one \
                         explicitly for this service"
                    ),
                ));
            }
        }
    }
    let declares_database = overrides.database_environment.is_some()
        || overrides.database_type.is_some()
        || overrides.database_name.is_some();
    let database = declares_database.then(|| RawMigrationDatabase {
        environment: overrides
            .database_environment
            .as_ref()
            .map(|environment| environment.label().to_owned()),
        database_type: overrides.database_type.clone(),
        name: overrides.database_name.clone(),
        ..RawMigrationDatabase::default()
    });
    Ok(Some(RawLiquibase {
        enabled: Some(enabled),
        command: None,
        provider: Some(provider),
        execution_mode: Some(execution_mode),
        commands: None,
        timeout_seconds: None,
        preset: preset.map(|preset| preset.to_string()),
        environment: BTreeMap::new(),
        database,
    }))
}

/// Resolves a library artifact coordinate: the override, then detected Maven
/// metadata.
fn resolve_artifact(
    overrides: &BulkImportOverrides,
    inspection: &RepositoryInspection,
) -> Result<RawArtifact, BulkImportIssue> {
    if let Some(artifact) = &overrides.artifact {
        return Ok(RawArtifact {
            group_id: artifact.group_id.trim().to_owned(),
            artifact_id: artifact.artifact_id.trim().to_owned(),
            version: artifact.version.trim().to_owned(),
            packaging: artifact.packaging.clone(),
            classifier: None,
        });
    }
    let detected = inspection.maven.as_ref().and_then(|maven| {
        Some(RawArtifact {
            group_id: maven.group_id.clone()?,
            artifact_id: maven.artifact_id.clone()?,
            version: maven.version.clone()?,
            packaging: maven.packaging.clone(),
            classifier: None,
        })
    });
    detected.ok_or_else(|| {
        BulkImportIssue::new(
            "ARTIFACT_REQUIRED",
            format!(
                "no complete Maven artifact coordinate could be detected for '{}'; provide a \
                 group id, artifact id, and version override",
                inspection.path
            ),
        )
    })
}

/// Resolves the entity id: an explicit override, the existing id for an
/// update, or the sanitized repository directory name.
fn resolve_id(
    overrides: &BulkImportOverrides,
    relative_path: &str,
    kind: EntityKind,
    existing_id: Option<&str>,
    errors: &mut Vec<BulkImportIssue>,
) -> Option<String> {
    if let Some(name) = overrides
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let validated = match kind {
            EntityKind::Library => LibraryId::new(name).map(|id| id.to_string()),
            _ => ServiceId::new(name).map(|id| id.to_string()),
        };
        return match validated {
            Ok(id) => Some(id),
            Err(error) => {
                errors.push(BulkImportIssue::new(
                    "INVALID_NAME",
                    format!(
                        "the name '{name}' is not a valid {} id: {error}",
                        kind.label()
                    ),
                ));
                None
            }
        };
    }
    if let Some(existing) = existing_id {
        return Some(existing.to_owned());
    }
    match sanitize_entity_id(&repository_name(relative_path)) {
        Some(id) => Some(id),
        None => {
            errors.push(BulkImportIssue::new(
                "NAME_REQUIRED",
                format!(
                    "no valid entity id can be derived from '{relative_path}'; set an explicit \
                     name"
                ),
            ));
            None
        }
    }
}

/// Derives an identifier from a repository directory name: lowercased ASCII
/// alphanumerics, `_`, `.`, and `:`, with every other character collapsing to
/// a single dash. Returns `None` when nothing valid remains.
fn sanitize_entity_id(raw: &str) -> Option<String> {
    let mut id = String::new();
    let mut last_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':') {
            id.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !id.is_empty() && !last_dash {
            id.push('-');
            last_dash = true;
        }
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{DetectedBuildSystem, DetectedRuntime, MavenMetadata, ProviderAvailability};

    fn inspection() -> RepositoryInspection {
        let mut inspection = RepositoryInspection::missing("/projects/acme/auth-service");
        inspection.exists = true;
        inspection.is_directory = true;
        inspection.is_git_repository = true;
        inspection.branch = Some("main".to_owned());
        inspection.remote = Some("git@example.com:acme/auth-service.git".to_owned());
        inspection.build_systems.push(DetectedBuildSystem {
            system: "maven".to_owned(),
            label: "Maven".to_owned(),
            availability: ProviderAvailability::Available,
            evidence: vec!["pom.xml".to_owned()],
            note: None,
        });
        inspection
    }

    #[test]
    fn sanitizes_repository_directory_names() {
        assert_eq!(
            sanitize_entity_id("Auth Service"),
            Some("auth-service".to_owned())
        );
        assert_eq!(
            sanitize_entity_id("auth_service"),
            Some("auth_service".to_owned())
        );
        assert_eq!(sanitize_entity_id("my.app"), Some("my.app".to_owned()));
        assert_eq!(sanitize_entity_id("a__b"), Some("a__b".to_owned()));
        assert_eq!(sanitize_entity_id("--weird--"), Some("weird".to_owned()));
        assert_eq!(sanitize_entity_id("Über"), Some("ber".to_owned()));
        assert_eq!(sanitize_entity_id("日本語"), None);
        assert_eq!(sanitize_entity_id("   "), None);
    }

    #[test]
    fn parses_configuration_locations() {
        assert_eq!(parse_location("services[4].runtime"), Some(("services", 4)));
        assert_eq!(
            parse_location("libraries[0].artifact"),
            Some(("libraries", 0))
        );
        assert_eq!(parse_location("discovery.max_depth"), None);
        assert_eq!(parse_location("services[abc]"), None);
    }

    #[test]
    fn names_are_derived_from_the_last_path_segment() {
        assert_eq!(repository_name("backend/auth-service"), "auth-service");
        assert_eq!(repository_name("auth-service"), "auth-service");
    }

    #[test]
    fn spring_boot_and_react_detections_suggest_types() {
        let spring = TechnologyDetection::new(
            "spring_boot",
            "Spring Boot",
            crate::ports::DetectionConfidence::High,
            Vec::new(),
        );
        let react = TechnologyDetection::new(
            "react",
            "React",
            crate::ports::DetectionConfidence::High,
            Vec::new(),
        );
        assert_eq!(detected_service_type(&[spring]), Some(ServiceType::Backend));
        assert_eq!(detected_service_type(&[react]), Some(ServiceType::Frontend));
        assert_eq!(detected_service_type(&[]), None);
    }

    #[test]
    fn detected_commands_use_the_wrapper_when_present() {
        let mut inspection = inspection();
        inspection.maven = Some(MavenMetadata {
            wrapper_available: true,
            spring_boot: true,
            ..MavenMetadata::default()
        });
        let build = detected_build_command(&inspection).expect("a build command");
        let RawCommand::Structured(build) = build else {
            panic!("a structured command");
        };
        assert_eq!(build.program, "./mvnw");
        assert_eq!(build.args, vec!["clean".to_owned(), "install".to_owned()]);
        let start = detected_start_command(&inspection).expect("a start command");
        let RawCommand::Structured(start) = start else {
            panic!("a structured command");
        };
        assert_eq!(start.args, vec!["spring-boot:run".to_owned()]);
    }

    #[test]
    fn detected_runtimes_fall_back_to_inspection_facts() {
        let mut inspection = inspection();
        inspection.runtimes.push(DetectedRuntime {
            kind: "java".to_owned(),
            label: "Java".to_owned(),
            required_version: Some("21".to_owned()),
            evidence: vec!["pom.xml".to_owned()],
        });
        inspection.runtimes.push(DetectedRuntime {
            kind: "java".to_owned(),
            label: "Java".to_owned(),
            required_version: Some("21".to_owned()),
            evidence: vec!["pom.xml".to_owned()],
        });
        let template = template();
        let runtimes = resolve_runtimes(&template, &BulkImportOverrides::default(), &inspection);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].kind, "java");
        assert_eq!(runtimes[0].version.as_deref(), Some("21"));
    }

    fn template() -> ServiceTemplate {
        use service_orchestrator_domain::{CommandSet, CommandSpec, TemplateId};
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Build,
            CommandSpec::new("./mvnw", ["clean", "install"]).expect("a valid command"),
        );
        ServiceTemplate {
            id: TemplateId::new("backend").expect("a valid id"),
            name: "backend".to_owned(),
            description: None,
            category: None,
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
    fn template_runtimes_win_over_detected_ones() {
        use service_orchestrator_domain::{Runtime, RuntimeKind, Version};
        let mut inspection = inspection();
        inspection.runtimes.push(DetectedRuntime {
            kind: "java".to_owned(),
            label: "Java".to_owned(),
            required_version: Some("17".to_owned()),
            evidence: vec!["pom.xml".to_owned()],
        });
        let mut template = template();
        template.runtimes.push(Runtime::versioned(
            RuntimeKind::Java,
            Version::new("21").expect("a valid version"),
        ));
        let runtimes = resolve_runtimes(&template, &BulkImportOverrides::default(), &inspection);
        assert_eq!(runtimes.len(), 1);
        assert_eq!(runtimes[0].version.as_deref(), Some("21"));
    }
}
