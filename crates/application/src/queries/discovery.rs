//! Bounded repository discovery for the bulk-import wizard (`tasks/032.md`
//! §7–§9).
//!
//! The use case is read-only: it resolves the workspace root from the validated
//! configuration, merges the configured discovery bounds with the request
//! overrides, and asks the [`RepositoryDiscovery`](crate::ports::RepositoryDiscovery)
//! boundary for a bounded scan. The application core never walks the filesystem
//! itself and never executes anything (`AGENTS.md` §1, `tasks/032.md` §7).
//!
//! The scan result is annotated with the two facts the wizard needs to render a
//! row: the *suggested* template category (and template, when unambiguous), and
//! the existing configured entities that already occupy the directory. Both are
//! suggestions for review; neither is a silent classification
//! (`tasks/032.md` §8, §9, §13).

use service_orchestrator_config::{
    effective_excludes, MAX_DISCOVERY_DEPTH, MAX_DISCOVERY_REPOSITORIES,
};
use service_orchestrator_domain::Permission;

use crate::app::Application;
use crate::bulk_import::{
    configured_entities_by_path, normalize_relative_path, suggested_category,
    templates_for_category, CategoryMatch, DiscoveredRepositoryView, DiscoveryScanRequest,
    RepositoryDiscoveryReport,
};
use crate::error::ApplicationError;
use crate::ports::{DiscoveryReport, DiscoveryRequest};

impl Application {
    /// Scans a workspace root for repositories, read-only and bounded.
    ///
    /// The effective bounds are the configured [`discovery`] settings, overlaid
    /// with the request overrides, then clamped to the hard caps
    /// ([`MAX_DISCOVERY_DEPTH`], [`MAX_DISCOVERY_REPOSITORIES`]) so a caller can
    /// never turn the scan unbounded (`tasks/032.md` §7, §47).
    ///
    /// [`discovery`]: service_orchestrator_config::DiscoverySettings
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does not
    /// exist, [`ApplicationError::PermissionDenied`] if the caller lacks read
    /// permission, and [`ApplicationError::Discovery`] when the scan cannot be
    /// attempted at all. Findings below an unreadable directory are returned as
    /// warnings on the report, not as errors.
    pub fn discover_repositories(
        &self,
        request: DiscoveryScanRequest,
    ) -> Result<RepositoryDiscoveryReport, ApplicationError> {
        self.authorize(Permission::Read)?;
        let workspace = self.require_workspace(&request.workspace_id)?;
        let settings = &self.config().discovery;

        let max_depth = request
            .max_depth
            .unwrap_or(settings.max_depth)
            .min(MAX_DISCOVERY_DEPTH);
        let max_repositories = request
            .max_repositories
            .unwrap_or(settings.max_repositories)
            .clamp(1, MAX_DISCOVERY_REPOSITORIES);
        let follow_symlinks = request.follow_symlinks.unwrap_or(settings.follow_symlinks);

        let mut exclude = effective_excludes(&settings.exclude);
        for name in &request.exclude {
            let name = name.trim();
            if !name.is_empty() && !exclude.iter().any(|existing| existing == name) {
                exclude.push(name.to_owned());
            }
        }

        let discovery_request = DiscoveryRequest {
            workspace_root: workspace
                .workspace
                .root
                .as_str()
                .trim_end_matches('/')
                .to_owned(),
            max_depth,
            exclude,
            follow_symlinks,
            max_repositories,
        };
        let report = self
            .repository_discovery()
            .discover(&discovery_request)
            .map_err(ApplicationError::discovery)?;
        Ok(self.annotate_discovery(workspace, report))
    }

    /// Annotates a raw discovery report with matching and registration state.
    fn annotate_discovery(
        &self,
        workspace: &service_orchestrator_config::ConfiguredWorkspace,
        report: DiscoveryReport,
    ) -> RepositoryDiscoveryReport {
        let configured = configured_entities_by_path(workspace);
        let templates = &self.config().templates;
        let repositories = report
            .repositories
            .into_iter()
            .map(|repository| {
                let (
                    suggested_category,
                    ambiguous_categories,
                    candidate_templates,
                    suggested_template,
                ) = match suggested_category(&repository.detections) {
                    CategoryMatch::Unique(category) => {
                        let candidates = templates_for_category(templates, &category);
                        if candidates.len() == 1 {
                            (
                                Some(category),
                                Vec::new(),
                                Vec::new(),
                                candidates.into_iter().next(),
                            )
                        } else {
                            (Some(category), Vec::new(), candidates, None)
                        }
                    }
                    CategoryMatch::Ambiguous { categories } => (None, categories, Vec::new(), None),
                    CategoryMatch::None => (None, Vec::new(), Vec::new(), None),
                };
                let existing = configured
                    .get(&normalize_relative_path(&repository.relative_path))
                    .cloned()
                    .unwrap_or_default();
                DiscoveredRepositoryView {
                    repository,
                    suggested_category,
                    ambiguous_categories,
                    suggested_template,
                    candidate_templates,
                    existing,
                }
            })
            .collect();
        RepositoryDiscoveryReport {
            workspace_id: workspace.workspace.id.clone(),
            root: report.workspace_root,
            repositories,
            truncated: report.truncated,
            warnings: report.warnings,
        }
    }
}
