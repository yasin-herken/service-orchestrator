//! The application container.
//!
//! [`Application`] is the single entry point every interface talks to. It holds
//! the already-validated [`Config`] plus the boundaries the use cases need
//! ([`TaskManager`], [`RuntimeStateStore`], [`LogService`]) and the
//! [`PolicyEngine`]. Interfaces never reach past it.
//!
//! Dependencies are injected explicitly through the constructor; there is no
//! framework. The struct is cloneable because its boundaries are shared behind
//! [`Arc`], so the TUI and MCP adapters can each hold an `Application` that
//! observes the same runtime state and submits to the same task engine.

use std::sync::Arc;

use service_orchestrator_config::{
    Config, ConfigManager, ConfiguredWorkspace, LiquibaseConfig, StaticConfigManager,
};
use service_orchestrator_domain::{
    AbsolutePath, GroupId, Library, LibraryId, Permission, Profile, ProfileId, Repository, Service,
    ServiceGroup, ServiceId, WorkspaceId,
};
use service_orchestrator_policy::{Authorization, PolicyEngine};

use crate::error::ApplicationError;
use crate::ports::{
    ArtifactInspector, BuildCapabilityIndex, GitManager, HealthIndex, LogService,
    NullArtifactInspector, NullBuildCapabilityIndex, NullGitManager, NullHealthIndex,
    NullRepositoryInspector, NullServiceRuntimeIndex, NullWorkspaceProbe, RepositoryInspector,
    ResolvedRepository, ResolvedServiceTarget, RuntimeStateStore, ServiceRuntimeIndex,
    WorkspaceProbe,
};

/// The shared application core.
#[derive(Clone)]
pub struct Application {
    config: Arc<Config>,
    configuration: Arc<dyn ConfigManager>,
    tasks: Arc<dyn service_orchestrator_execution::TaskManager>,
    runtime: Arc<dyn RuntimeStateStore>,
    logs: Arc<dyn LogService>,
    git: Arc<dyn GitManager>,
    artifacts: Arc<dyn ArtifactInspector>,
    service_runtime: Arc<dyn ServiceRuntimeIndex>,
    health: Arc<dyn HealthIndex>,
    build_capabilities: Arc<dyn BuildCapabilityIndex>,
    probe: Arc<dyn WorkspaceProbe>,
    repository_inspector: Arc<dyn RepositoryInspector>,
    policy: PolicyEngine,
    authorization: Authorization,
}

impl Application {
    /// Creates an application core from validated configuration and its ports.
    ///
    /// No Git manager, artifact inspector, or service runtime index is wired;
    /// call [`Application::with_git_manager`],
    /// [`Application::with_artifact_inspector`], and
    /// [`Application::with_service_runtime`] to provide them. Until then, Git
    /// operations report [`ApplicationError::Git`], artifact inspection reports
    /// [`ApplicationError::ArtifactInspection`], and runtime-status queries
    /// report only the last observed state store value.
    #[must_use]
    pub fn new(
        config: Config,
        tasks: Arc<dyn service_orchestrator_execution::TaskManager>,
        runtime: Arc<dyn RuntimeStateStore>,
        logs: Arc<dyn LogService>,
    ) -> Self {
        let configuration: Arc<dyn ConfigManager> =
            Arc::new(StaticConfigManager::new(config.clone()));
        Self {
            config: Arc::new(config),
            configuration,
            tasks,
            runtime,
            logs,
            git: Arc::new(NullGitManager),
            artifacts: Arc::new(NullArtifactInspector),
            service_runtime: Arc::new(NullServiceRuntimeIndex),
            health: Arc::new(NullHealthIndex),
            build_capabilities: Arc::new(NullBuildCapabilityIndex),
            probe: Arc::new(NullWorkspaceProbe),
            repository_inspector: Arc::new(NullRepositoryInspector),
            policy: PolicyEngine::new(),
            authorization: Authorization::standard(),
        }
    }

    /// Replaces the Git manager.
    #[must_use]
    pub fn with_git_manager(mut self, git: Arc<dyn GitManager>) -> Self {
        self.git = git;
        self
    }

    /// Replaces the artifact inspector.
    ///
    /// The inspector is provided by the build adapter; it resolves artifacts
    /// against the effective local Maven repository and inspects consumer
    /// dependency versions. Without it, those queries report
    /// [`ApplicationError::ArtifactInspection`].
    #[must_use]
    pub fn with_artifact_inspector(mut self, artifacts: Arc<dyn ArtifactInspector>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Replaces the service runtime index.
    ///
    /// The index is provided by the runtime adapter; it exposes live
    /// managed-process state so `get_service_runtime_status` and
    /// `list_running_services` can report process ids, exit information, and
    /// startup commands. Without it, those queries report only the last observed
    /// state store value.
    #[must_use]
    pub fn with_service_runtime(mut self, service_runtime: Arc<dyn ServiceRuntimeIndex>) -> Self {
        self.service_runtime = service_runtime;
        self
    }

    /// Replaces the health index.
    ///
    /// The index is provided by the health adapter; it exposes the last observed
    /// health of each service so `get_service_health`, `list_service_health`,
    /// and `get_service_diagnostics` can report it. Without it, those queries
    /// report [`HealthState::Unknown`](crate::HealthState).
    #[must_use]
    pub fn with_health_index(mut self, health: Arc<dyn HealthIndex>) -> Self {
        self.health = health;
        self
    }

    /// Replaces the build-capability index.
    ///
    /// The index is provided by the build adapter; it projects the registered
    /// build providers so interfaces can render which operations a service's
    /// build system supports without hard-coding provider knowledge
    /// (`tasks/020.md` §13). Without it, capability queries report that no
    /// provider is registered.
    #[must_use]
    pub fn with_build_capabilities(
        mut self,
        build_capabilities: Arc<dyn BuildCapabilityIndex>,
    ) -> Self {
        self.build_capabilities = build_capabilities;
        self
    }

    /// Replaces the workspace probe.
    ///
    /// The probe is provided by the composition root; it performs the
    /// read-only environment observations behind
    /// [`validate_workspace`](Self::validate_workspace). Without a real probe,
    /// validation assumes every path and program is present.
    #[must_use]
    pub fn with_workspace_probe(mut self, probe: Arc<dyn WorkspaceProbe>) -> Self {
        self.probe = probe;
        self
    }

    /// Replaces the configuration manager.
    ///
    /// The manager owns the validated configuration the application reads and
    /// the transaction operations behind the configuration-management use
    /// cases. The daemon wires a [`FileConfigManager`](service_orchestrator_config::FileConfigManager)
    /// so a committed change is re-loaded and every subsequent read observes it
    /// (`tasks/023.md` §39, §40). Without it, the application keeps the
    /// configuration it was constructed with and mutation operations report
    /// that configuration management is unavailable.
    #[must_use]
    pub fn with_configuration(mut self, configuration: Arc<dyn ConfigManager>) -> Self {
        self.configuration = configuration;
        self.refresh_configuration();
        self
    }

    /// Replaces the repository inspector.
    ///
    /// The inspector is provided by the composition root; it performs the
    /// read-only repository observations behind
    /// [`inspect_repository`](Self::inspect_repository). Without a real
    /// inspector, those queries report that inspection is unavailable.
    #[must_use]
    pub fn with_repository_inspector(
        mut self,
        repository_inspector: Arc<dyn RepositoryInspector>,
    ) -> Self {
        self.repository_inspector = repository_inspector;
        self
    }

    /// Replaces the policy engine.
    #[must_use]
    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
        self.policy = policy;
        self
    }

    /// Sets the caller's authorization, for example after a user confirms a
    /// destructive action.
    #[must_use]
    pub fn with_authorization(mut self, authorization: Authorization) -> Self {
        self.authorization = authorization;
        self
    }

    /// Returns the validated configuration backing this application.
    #[must_use]
    pub fn config(&self) -> &Config {
        self.config.as_ref()
    }

    /// Returns the configuration manager behind the management use cases.
    #[must_use]
    pub fn configuration(&self) -> &dyn ConfigManager {
        self.configuration.as_ref()
    }

    /// Re-reads the configuration manager's current snapshot into this view.
    ///
    /// Called by the daemon after it commits or reloads configuration so a
    /// subsequent request observes the new state (`tasks/023.md` §39, §40).
    pub fn refresh_configuration(&mut self) {
        self.config = self.configuration.current();
    }

    /// Checks `permission` against the current authorization.
    ///
    /// Interfaces call this before a destructive action to decide whether to
    /// prompt for confirmation; mutating use cases call it themselves so the
    /// check cannot be bypassed.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::PermissionDenied`] when the operation
    /// requires confirmation that has not been granted.
    pub fn authorize(&self, permission: Permission) -> Result<(), ApplicationError> {
        self.policy
            .check(permission, self.authorization)
            .map_err(|error| ApplicationError::PermissionDenied {
                permission,
                message: error.to_string(),
            })
    }

    /// Returns the execution settings carried by the configuration.
    pub(crate) fn default_timeout_seconds(&self) -> u64 {
        self.config.execution.default_timeout_seconds
    }

    /// Returns the task manager boundary.
    pub(crate) fn tasks(&self) -> &dyn service_orchestrator_execution::TaskManager {
        self.tasks.as_ref()
    }

    /// Returns the runtime state store.
    pub(crate) fn runtime(&self) -> &dyn RuntimeStateStore {
        self.runtime.as_ref()
    }

    /// Returns the log service.
    pub(crate) fn logs(&self) -> &dyn LogService {
        self.logs.as_ref()
    }

    /// Returns the Git manager boundary.
    pub(crate) fn git(&self) -> &dyn GitManager {
        self.git.as_ref()
    }

    /// Returns the artifact inspector boundary.
    pub(crate) fn artifacts(&self) -> &dyn ArtifactInspector {
        self.artifacts.as_ref()
    }

    /// Returns the service runtime index.
    pub(crate) fn service_runtime(&self) -> &dyn ServiceRuntimeIndex {
        self.service_runtime.as_ref()
    }

    /// Returns the health index.
    pub(crate) fn health(&self) -> &dyn HealthIndex {
        self.health.as_ref()
    }

    /// Returns the build-capability index.
    pub(crate) fn build_capability_index(&self) -> &dyn BuildCapabilityIndex {
        self.build_capabilities.as_ref()
    }

    /// Returns the workspace probe boundary.
    pub(crate) fn probe(&self) -> &dyn WorkspaceProbe {
        self.probe.as_ref()
    }

    /// Returns the repository inspector boundary.
    pub(crate) fn repository_inspector(&self) -> &dyn RepositoryInspector {
        self.repository_inspector.as_ref()
    }

    /// Resolves an entity's workspace-relative directory into an absolute path.
    ///
    /// The domain guarantees the relative path is valid; joining it with the
    /// workspace root cannot escape the workspace because a [`RelativePath`]
    /// contains no `..` segments.
    ///
    /// [`RelativePath`]: service_orchestrator_domain::RelativePath
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::InvalidOperation`] if the joined path cannot
    /// be represented as an absolute path.
    pub(crate) fn absolute_entity_path(
        &self,
        workspace: &ConfiguredWorkspace,
        relative: &service_orchestrator_domain::RelativePath,
    ) -> Result<AbsolutePath, ApplicationError> {
        let root = workspace.workspace.root.as_str().trim_end_matches('/');
        AbsolutePath::new(format!("{root}/{}", relative.as_str())).map_err(|error| {
            ApplicationError::invalid_operation(format!(
                "path '{}' is not a valid absolute path: {error}",
                relative.as_str()
            ))
        })
    }

    // -- repository resolution ------------------------------------------------

    /// Resolves the repository of a service into a workspace-aware value.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist and [`ApplicationError::InvalidOperation`] if it has no repository.
    pub(crate) fn resolve_service_repository(
        &self,
        id: &ServiceId,
    ) -> Result<ResolvedRepository, ApplicationError> {
        let (workspace, service) = self.locate_service(id)?;
        let repository = service.repository.as_ref().ok_or_else(|| {
            ApplicationError::invalid_operation(format!(
                "service '{id}' has no repository configured"
            ))
        })?;
        self.resolve_repository(workspace, id.as_str(), repository)
    }

    /// Resolves the repository of a library into a workspace-aware value.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist and [`ApplicationError::InvalidOperation`] if it has no repository.
    pub(crate) fn resolve_library_repository(
        &self,
        id: &LibraryId,
    ) -> Result<ResolvedRepository, ApplicationError> {
        let (workspace, library) = self.locate_library(id)?;
        let repository = library.repository.as_ref().ok_or_else(|| {
            ApplicationError::invalid_operation(format!(
                "library '{id}' has no repository configured"
            ))
        })?;
        self.resolve_repository(workspace, id.as_str(), repository)
    }

    /// Resolves a service into a workspace-aware lifecycle target.
    ///
    /// The target carries only identity and the working directory; environment
    /// *values* are deliberately excluded, exactly like a build target
    /// (`AGENTS.md` §31). A service without a configured path yields a target
    /// with no path — a start or restart then fails with
    /// [`ServiceRuntimeError::WorkingDirectoryInvalid`](crate::ServiceRuntimeError)
    /// while a stop remains valid.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceRuntime`] if the resolved path escapes
    /// the workspace root or cannot be represented as an absolute path.
    pub(crate) fn service_runtime_target(
        &self,
        workspace: &ConfiguredWorkspace,
        service: &Service,
    ) -> Result<ResolvedServiceTarget, ApplicationError> {
        let path = match &service.path {
            Some(relative) => Some(self.absolute_entity_path(workspace, relative)?),
            None => None,
        };
        let target = ResolvedServiceTarget {
            workspace_id: workspace.workspace.id.clone(),
            service_id: service.id.clone(),
            path,
            workspace_root: workspace.workspace.root.clone(),
        };
        if !target.is_inside_workspace() {
            return Err(ApplicationError::ServiceRuntime {
                code: "RUNTIME_PATH_OUTSIDE_WORKSPACE".to_owned(),
                message: format!(
                    "service '{}' resolves to a working directory outside workspace root '{}'",
                    service.id, target.workspace_root
                ),
            });
        }
        Ok(target)
    }

    /// Builds a [`ResolvedRepository`] from a workspace and a repository,
    /// joining the workspace root with the repository's relative checkout path.
    pub(crate) fn resolve_repository(
        &self,
        workspace: &ConfiguredWorkspace,
        name: &str,
        repository: &Repository,
    ) -> Result<ResolvedRepository, ApplicationError> {
        let root = workspace.workspace.root.as_str().trim_end_matches('/');
        let path = AbsolutePath::new(format!("{root}/{}", repository.local_path.as_str()))
            .map_err(|error| ApplicationError::Git {
                code: "GIT_INVALID_PATH".to_owned(),
                message: error.to_string(),
            })?;
        let resolved = ResolvedRepository {
            workspace_id: workspace.workspace.id.clone(),
            name: name.to_owned(),
            url: repository.url.clone(),
            path,
            workspace_root: workspace.workspace.root.clone(),
            default_branch: repository.default_branch.clone(),
        };
        if !resolved.is_inside_workspace() {
            return Err(ApplicationError::Git {
                code: "GIT_PATH_OUTSIDE_WORKSPACE".to_owned(),
                message: format!(
                    "repository '{name}' resolves to '{}', outside workspace root '{}'",
                    resolved.path, resolved.workspace_root
                ),
            });
        }
        Ok(resolved)
    }

    // -- configuration lookups ------------------------------------------------

    /// Returns a configured workspace by id.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if it does not exist.
    pub(crate) fn require_workspace(
        &self,
        id: &WorkspaceId,
    ) -> Result<&ConfiguredWorkspace, ApplicationError> {
        self.config
            .workspace(id)
            .ok_or_else(|| ApplicationError::WorkspaceNotFound(id.clone()))
    }

    /// Returns the workspace that owns `id`.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if no workspace owns it.
    pub(crate) fn workspace_of_service(
        &self,
        id: &ServiceId,
    ) -> Result<&ConfiguredWorkspace, ApplicationError> {
        self.config
            .workspace_of_service(id)
            .ok_or_else(|| ApplicationError::ServiceNotFound(id.clone()))
    }

    /// Returns a workspace and service pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist.
    pub(crate) fn locate_service(
        &self,
        id: &ServiceId,
    ) -> Result<(&ConfiguredWorkspace, &Service), ApplicationError> {
        let workspace = self.workspace_of_service(id)?;
        let service = workspace
            .workspace
            .service(id)
            .expect("workspace_of_service returned a workspace that owns the service");
        Ok((workspace, service))
    }

    /// Returns a workspace and library pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist.
    pub(crate) fn locate_library(
        &self,
        id: &LibraryId,
    ) -> Result<(&ConfiguredWorkspace, &Library), ApplicationError> {
        for workspace in &self.config.workspaces {
            if let Some(library) = workspace.workspace.library(id) {
                return Ok((workspace, library));
            }
        }
        Err(ApplicationError::LibraryNotFound(id.clone()))
    }

    /// Returns a workspace and profile pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ProfileNotFound`] if the profile does not
    /// exist.
    pub(crate) fn locate_profile(
        &self,
        id: &ProfileId,
    ) -> Result<(&ConfiguredWorkspace, &Profile), ApplicationError> {
        for workspace in &self.config.workspaces {
            if let Some(profile) = workspace.workspace.profile(id) {
                return Ok((workspace, profile));
            }
        }
        Err(ApplicationError::ProfileNotFound(id.clone()))
    }

    /// Returns a workspace and service group pair.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::GroupNotFound`] if the group does not exist.
    pub(crate) fn locate_group(
        &self,
        id: &GroupId,
    ) -> Result<(&ConfiguredWorkspace, &ServiceGroup), ApplicationError> {
        for workspace in &self.config.workspaces {
            if let Some(group) = workspace.workspace.group(id) {
                return Ok((workspace, group));
            }
        }
        Err(ApplicationError::GroupNotFound(id.clone()))
    }

    /// Returns the Liquibase configuration for a service, if any.
    pub(crate) fn liquibase_config<'a>(
        &self,
        workspace: &'a ConfiguredWorkspace,
        service_id: &ServiceId,
    ) -> Option<&'a LiquibaseConfig> {
        workspace.liquibase.get(service_id)
    }
}
