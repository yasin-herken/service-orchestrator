//! The read-only workspace validation use case.
//!
//! [`Application::validate_workspace`] inspects a configured workspace against
//! this machine and returns a structured report. It is **read-only by
//! construction** (`tasks/017.md` §7): it never checks out, pulls, builds,
//! installs, migrates, starts, stops, or rewrites anything. Every observation
//! goes through the [`WorkspaceProbe`] boundary.

use std::collections::BTreeMap;

use service_orchestrator_domain::{
    AbsolutePath, BuildSystem, CommandKind, DependencyTarget, HealthCheckKind, Permission, Port,
    RuntimeKind, ServiceId, ServiceType, WorkspaceId,
};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::ports::WorkspaceProbe;
use crate::validation::{
    ConfiguredPortConflict, OccupiedPort, RepositoryCheckStatus, RepositoryOwner,
    RepositoryValidation, ValidationCode, ValidationDiagnostic, ValidationScope,
    ValidationSeverity, ValidationTotals, WorkspaceValidationReport,
};

impl Application {
    /// Validates a workspace against this machine, read-only.
    ///
    /// The report covers configuration totals, repository checkouts, runtime
    /// and build-tool availability, dependency resolution and ordering, port
    /// configuration conflicts, and actual port occupancy. A service that is
    /// merely stopped is never reported as broken.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::WorkspaceNotFound`] if the workspace does
    /// not exist.
    pub fn validate_workspace(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceValidationReport, ApplicationError> {
        self.authorize(Permission::Read)?;
        let workspace = self.require_workspace(workspace_id)?;
        let probe = self.probe();
        let root = workspace
            .workspace
            .root
            .as_str()
            .trim_end_matches('/')
            .to_owned();

        let mut diagnostics = Vec::new();
        let root_available = probe.exists(&root) && probe.is_dir(&root);
        if !root_available {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Error,
                code: ValidationCode::MissingWorkspaceRoot,
                scope: ValidationScope::Workspace,
                message: format!(
                    "the workspace root '{root}' does not exist or is not a directory on this machine"
                ),
            });
        }

        // -- repositories and working directories --------------------------
        let mut repositories = Vec::new();
        let mut totals = ValidationTotals::default();

        for service in &workspace.workspace.services {
            if let Some(relative) = &service.path {
                check_working_directory(
                    probe,
                    workspace,
                    ValidationScope::Service(service.id.clone()),
                    relative,
                    ValidationCode::MissingServicePath,
                    &mut diagnostics,
                );
            }
            if let Some(repository) = &service.repository {
                let relative = &repository.local_path;
                if let Ok(path) = self.absolute_entity_path(workspace, relative) {
                    let status = check_repository(probe, &path, root_available);
                    tally(&status, &mut totals);
                    push_repository_diagnostic(
                        &status,
                        RepositoryOwner::Service(service.id.clone()),
                        path.as_str(),
                        &mut diagnostics,
                    );
                    repositories.push(RepositoryValidation {
                        owner: RepositoryOwner::Service(service.id.clone()),
                        path: path.as_str().to_owned(),
                        status,
                    });
                }
            }
            check_runtime_tools(
                probe,
                ValidationScope::Service(service.id.clone()),
                &service.runtimes,
                service
                    .path
                    .as_ref()
                    .and_then(|relative| self.absolute_entity_path(workspace, relative).ok()),
                service.build_system,
                &mut diagnostics,
            );
            check_start_directory(
                probe,
                workspace,
                &service.id,
                &service.commands,
                &mut diagnostics,
            );
            check_health_ports(service.id.clone(), service, &mut diagnostics);
        }

        for library in &workspace.workspace.libraries {
            if let Some(relative) = &library.path {
                check_working_directory(
                    probe,
                    workspace,
                    ValidationScope::Library(library.id.clone()),
                    relative,
                    ValidationCode::MissingLibraryPath,
                    &mut diagnostics,
                );
            }
            if let Some(repository) = &library.repository {
                let relative = &repository.local_path;
                if let Ok(path) = self.absolute_entity_path(workspace, relative) {
                    let status = check_repository(probe, &path, root_available);
                    tally(&status, &mut totals);
                    push_repository_diagnostic(
                        &status,
                        RepositoryOwner::Library(library.id.clone()),
                        path.as_str(),
                        &mut diagnostics,
                    );
                    repositories.push(RepositoryValidation {
                        owner: RepositoryOwner::Library(library.id.clone()),
                        path: path.as_str().to_owned(),
                        status,
                    });
                }
            }
            check_runtime_tools(
                probe,
                ValidationScope::Library(library.id.clone()),
                &library.runtimes,
                library
                    .path
                    .as_ref()
                    .and_then(|relative| self.absolute_entity_path(workspace, relative).ok()),
                library.build_system,
                &mut diagnostics,
            );
        }

        // -- dependency resolution and ordering ----------------------------
        check_dependencies(workspace, &mut diagnostics);
        check_dependency_cycles(workspace, &mut diagnostics);

        // -- ports ---------------------------------------------------------
        let mut declared: BTreeMap<Port, Vec<ServiceId>> = BTreeMap::new();
        for service in &workspace.workspace.services {
            for port in &service.ports {
                declared.entry(*port).or_default().push(service.id.clone());
            }
        }
        let mut port_conflicts = Vec::new();
        let mut occupied_ports = Vec::new();
        for (port, services) in &declared {
            let mut services = services.clone();
            services.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            if services.len() > 1 {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Error,
                    code: ValidationCode::DuplicateConfiguredPort,
                    scope: ValidationScope::Workspace,
                    message: format!(
                        "port {port} is declared by multiple services: {}",
                        services
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                });
                port_conflicts.push(ConfiguredPortConflict {
                    port: *port,
                    services,
                });
            } else if probe.is_port_in_use(port.value()) {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Warning,
                    code: ValidationCode::PortCurrentlyOccupied,
                    scope: ValidationScope::Service(services[0].clone()),
                    message: format!(
                        "port {port} currently accepts connections; it may be held by an \
                         unrelated process or by '{}' already running",
                        services[0]
                    ),
                });
                occupied_ports.push(OccupiedPort {
                    port: *port,
                    services,
                });
            }
        }

        totals.services = workspace.workspace.services.len();
        totals.backend_services = workspace
            .workspace
            .services
            .iter()
            .filter(|service| service.kind == ServiceType::Backend)
            .count();
        totals.frontend_services = workspace
            .workspace
            .services
            .iter()
            .filter(|service| service.kind == ServiceType::Frontend)
            .count();
        totals.libraries = workspace.workspace.libraries.len();
        totals.port_conflicts = port_conflicts.len();
        for diagnostic in &diagnostics {
            match diagnostic.severity {
                ValidationSeverity::Error => totals.errors += 1,
                ValidationSeverity::Warning => totals.warnings += 1,
            }
            match diagnostic.code {
                ValidationCode::MissingRuntime | ValidationCode::MissingBuildTool => {
                    totals.missing_runtimes += 1;
                }
                ValidationCode::UnknownServiceDependency
                | ValidationCode::UnknownLibraryDependency
                | ValidationCode::DependencyCycle => {
                    totals.dependency_errors += 1;
                }
                _ => {}
            }
        }
        // "Invalid configurations" = configurations that cannot work as
        // written. Occupied ports and stopped services are warnings at most,
        // never errors, so they never inflate this count.
        totals.invalid_configurations = totals.errors;

        Ok(WorkspaceValidationReport {
            workspace_id: workspace.workspace.id.clone(),
            name: workspace.workspace.name.clone(),
            root,
            totals,
            repositories,
            diagnostics,
            port_conflicts,
            occupied_ports,
        })
    }
}

/// Checks one entity's working directory.
fn check_working_directory(
    probe: &dyn WorkspaceProbe,
    workspace: &service_orchestrator_config::ConfiguredWorkspace,
    scope: ValidationScope,
    relative: &service_orchestrator_domain::RelativePath,
    missing_code: ValidationCode,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let root = workspace.workspace.root.as_str().trim_end_matches('/');
    let path = format!("{root}/{}", relative.as_str());
    if !probe.exists(&path) {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            code: missing_code,
            scope,
            message: format!("the configured working directory '{path}' does not exist"),
        });
    } else if !probe.is_dir(&path) {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            code: ValidationCode::NotADirectory,
            scope,
            message: format!("the configured working directory '{path}' is not a directory"),
        });
    }
}

/// Classifies one repository checkout without touching it.
fn check_repository(
    probe: &dyn WorkspaceProbe,
    path: &AbsolutePath,
    root_available: bool,
) -> RepositoryCheckStatus {
    if !root_available {
        return RepositoryCheckStatus::Missing;
    }
    if !probe.exists(path.as_str()) {
        RepositoryCheckStatus::Missing
    } else if !probe.is_dir(path.as_str()) {
        RepositoryCheckStatus::NotADirectory
    } else if !probe.contains_entry(path.as_str(), ".git") {
        RepositoryCheckStatus::MissingGitMetadata
    } else {
        RepositoryCheckStatus::Valid
    }
}

fn tally(status: &RepositoryCheckStatus, totals: &mut ValidationTotals) {
    match status {
        RepositoryCheckStatus::Valid => totals.repositories_valid += 1,
        RepositoryCheckStatus::Missing => totals.repositories_missing += 1,
        RepositoryCheckStatus::NotADirectory | RepositoryCheckStatus::MissingGitMetadata => {
            totals.repositories_invalid += 1;
        }
    }
}

fn push_repository_diagnostic(
    status: &RepositoryCheckStatus,
    owner: RepositoryOwner,
    path: &str,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let (code, message) = match status {
        RepositoryCheckStatus::Valid => return,
        RepositoryCheckStatus::Missing => (
            ValidationCode::MissingRepository,
            format!("the repository checkout '{path}' does not exist; sync it before building"),
        ),
        RepositoryCheckStatus::NotADirectory => (
            ValidationCode::NotADirectory,
            format!("the repository checkout '{path}' exists but is not a directory"),
        ),
        RepositoryCheckStatus::MissingGitMetadata => (
            ValidationCode::MissingGitMetadata,
            format!(
                "the repository checkout '{path}' has no .git metadata; it is not a Git repository"
            ),
        ),
    };
    let scope = match owner {
        RepositoryOwner::Service(id) => ValidationScope::Service(id),
        RepositoryOwner::Library(id) => ValidationScope::Library(id),
    };
    diagnostics.push(ValidationDiagnostic {
        severity: ValidationSeverity::Error,
        code,
        scope,
        message,
    });
}

/// Checks that declared runtimes and build systems are available locally.
#[allow(clippy::too_many_arguments)]
fn check_runtime_tools(
    probe: &dyn WorkspaceProbe,
    scope: ValidationScope,
    runtimes: &[service_orchestrator_domain::Runtime],
    directory: Option<AbsolutePath>,
    build_system: Option<BuildSystem>,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for runtime in runtimes {
        let (program, label) = match runtime.kind {
            RuntimeKind::Java => ("java", "Java"),
            RuntimeKind::Node => ("node", "Node.js"),
            // Build-tool requirements are checked through the build system
            // below, where the Maven Wrapper preference can be honoured.
            RuntimeKind::Maven | RuntimeKind::Npm => continue,
            // Any other runtime has no universally discoverable binary; there
            // is nothing meaningful to probe without hard-coding tooling.
            RuntimeKind::Other(_) => continue,
        };
        if !probe.has_program(program) {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Warning,
                code: ValidationCode::MissingRuntime,
                scope: scope.clone(),
                message: format!("'{program}' ({label}) was not found on PATH"),
            });
        }
    }

    let wrapper_available = directory
        .as_ref()
        .is_some_and(|path| probe.contains_entry(path.as_str(), "mvnw"));
    match build_system {
        Some(BuildSystem::Maven) if !wrapper_available && !probe.has_program("mvn") => {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Warning,
                code: ValidationCode::MissingBuildTool,
                scope,
                message: "neither a Maven Wrapper (mvnw) nor 'mvn' was found for this Maven build"
                    .to_owned(),
            });
        }
        Some(BuildSystem::Npm) if !probe.has_program("npm") => {
            diagnostics.push(ValidationDiagnostic {
                severity: ValidationSeverity::Warning,
                code: ValidationCode::MissingBuildTool,
                scope,
                message: "'npm' was not found on PATH for this npm build".to_owned(),
            });
        }
        _ => {}
    }
}

/// Checks that a start command's working directory exists.
fn check_start_directory(
    probe: &dyn WorkspaceProbe,
    workspace: &service_orchestrator_config::ConfiguredWorkspace,
    id: &ServiceId,
    commands: &service_orchestrator_domain::CommandSet,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let Some(relative) = commands
        .get(&CommandKind::Start)
        .and_then(|command| command.working_dir.as_ref())
    else {
        return;
    };
    let root = workspace.workspace.root.as_str().trim_end_matches('/');
    let path = format!("{root}/{}", relative.as_str());
    if !probe.is_dir(&path) {
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Warning,
            code: ValidationCode::MissingStartDirectory,
            scope: ValidationScope::Service(id.clone()),
            message: format!(
                "the start command's working directory '{path}' does not exist or is not a directory"
            ),
        });
    }
}

/// Checks that port health checks reference declared ports.
fn check_health_ports(
    id: ServiceId,
    service: &service_orchestrator_domain::Service,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for check in &service.health_checks {
        if let HealthCheckKind::Port(port) = &check.kind {
            if !service.ports.contains(port) {
                diagnostics.push(ValidationDiagnostic {
                    severity: ValidationSeverity::Warning,
                    code: ValidationCode::UndeclaredHealthPort,
                    scope: ValidationScope::Service(id.clone()),
                    message: format!(
                        "a port health check probes {port}, which the service does not declare"
                    ),
                });
            }
        }
    }
}

/// Checks that every dependency target is configured.
fn check_dependencies(
    workspace: &service_orchestrator_config::ConfiguredWorkspace,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    let report_unknown = |owner: ValidationScope,
                          target: &DependencyTarget,
                          required: bool,
                          diagnostics: &mut Vec<ValidationDiagnostic>| {
        let (resolved, code) = match target {
            DependencyTarget::Service(id) => (
                workspace.workspace.service(id).is_some(),
                ValidationCode::UnknownServiceDependency,
            ),
            DependencyTarget::Library(id) => (
                workspace.workspace.library(id).is_some(),
                ValidationCode::UnknownLibraryDependency,
            ),
        };
        if !resolved {
            diagnostics.push(ValidationDiagnostic {
                severity: if required {
                    ValidationSeverity::Error
                } else {
                    ValidationSeverity::Warning
                },
                code,
                scope: owner,
                message: format!("dependency '{target}' is not configured in this workspace"),
            });
        }
    };

    for service in &workspace.workspace.services {
        for dependency in &service.dependencies {
            let owner = ValidationScope::Service(service.id.clone());
            report_unknown(owner, &dependency.target, dependency.required, diagnostics);
        }
    }
    for library in &workspace.workspace.libraries {
        for dependency in &library.dependencies {
            let owner = ValidationScope::Library(library.id.clone());
            report_unknown(owner, &dependency.target, dependency.required, diagnostics);
        }
    }
}

/// Detects cycles among required dependencies using Kahn's algorithm.
fn check_dependency_cycles(
    workspace: &service_orchestrator_config::ConfiguredWorkspace,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    // (0 = service, 1 = library) keeps service and library ids distinct even
    // when they share a name.
    type Node = (u8, String);
    let mut edges: BTreeMap<Node, Vec<Node>> = BTreeMap::new();
    let mut indegree: BTreeMap<Node, usize> = BTreeMap::new();

    fn add_node(
        node: Node,
        edges: &mut BTreeMap<Node, Vec<Node>>,
        indegree: &mut BTreeMap<Node, usize>,
    ) {
        indegree.entry(node.clone()).or_insert(0);
        edges.entry(node).or_default();
    }

    fn add_edge(
        from: Node,
        to: Node,
        edges: &mut BTreeMap<Node, Vec<Node>>,
        indegree: &mut BTreeMap<Node, usize>,
    ) {
        if let Some(list) = edges.get_mut(&from) {
            list.push(to.clone());
        }
        *indegree.entry(to).or_insert(0) += 1;
    }

    for service in &workspace.workspace.services {
        add_node((0, service.id.to_string()), &mut edges, &mut indegree);
    }
    for library in &workspace.workspace.libraries {
        add_node((1, library.id.to_string()), &mut edges, &mut indegree);
    }
    for service in &workspace.workspace.services {
        for dependency in &service.dependencies {
            if !dependency.required {
                continue;
            }
            let to = match &dependency.target {
                DependencyTarget::Service(id) => (0, id.to_string()),
                DependencyTarget::Library(id) => (1, id.to_string()),
            };
            if indegree.contains_key(&to) {
                add_edge((0, service.id.to_string()), to, &mut edges, &mut indegree);
            }
        }
    }
    for library in &workspace.workspace.libraries {
        for dependency in &library.dependencies {
            if !dependency.required {
                continue;
            }
            let to = match &dependency.target {
                DependencyTarget::Service(id) => (0, id.to_string()),
                DependencyTarget::Library(id) => (1, id.to_string()),
            };
            if indegree.contains_key(&to) {
                add_edge((1, library.id.to_string()), to, &mut edges, &mut indegree);
            }
        }
    }

    let mut ready: Vec<Node> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(node, _)| node.clone())
        .collect();
    let mut remaining = indegree.len();
    while let Some(node) = ready.pop() {
        remaining -= 1;
        for next in edges.get(&node).cloned().unwrap_or_default() {
            if let Some(degree) = indegree.get_mut(&next) {
                *degree -= 1;
                if *degree == 0 {
                    ready.push(next);
                }
            }
        }
    }
    if remaining > 0 {
        let cyclic: Vec<String> = indegree
            .iter()
            .filter(|(_, degree)| **degree > 0)
            .map(|(node, _)| node.1.clone())
            .collect();
        diagnostics.push(ValidationDiagnostic {
            severity: ValidationSeverity::Error,
            code: ValidationCode::DependencyCycle,
            scope: ValidationScope::Workspace,
            message: format!(
                "required dependencies form a cycle involving: {}",
                cyclic.join(", ")
            ),
        });
    }
}
