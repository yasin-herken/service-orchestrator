//! Read-only view models handed to the frontend.
//!
//! The DTOs are deliberately slimmer than the core types: the GUI renders a
//! table row and a workspace card, not the full domain aggregate. Mapping
//! happens exactly once, here, so neither the frontend nor the Tauri commands
//! re-derive domain data (`tasks/018.md` §6, §11).

use serde::Serialize;
use service_orchestrator_application::{
    HealthCheckOutcome, OperationResult, ServiceDiagnostics, ServiceStatus,
    WorkspaceStatus as CoreWorkspaceStatus,
};
use service_orchestrator_domain::{
    Dependency, DependencyTarget, HealthCheck, Repository, Runtime, Service, Timestamp, Workspace,
};

/// A workspace as shown in the selector and the dashboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkspaceSummary {
    /// The workspace identifier.
    pub id: String,
    /// The human-readable workspace name.
    pub name: String,
    /// The absolute filesystem root of the workspace.
    pub root: String,
    /// How many services the workspace configures.
    pub service_count: usize,
}

impl From<Workspace> for WorkspaceSummary {
    fn from(workspace: Workspace) -> Self {
        Self {
            id: workspace.id.to_string(),
            name: workspace.name,
            root: workspace.root.to_string(),
            service_count: workspace.services.len(),
        }
    }
}

/// One row of the services table: configuration plus observed state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceSummary {
    /// The service identifier.
    pub id: String,
    /// The human-readable service name.
    pub display_name: String,
    /// `backend`, `frontend`, `library`, or a project-specific category.
    pub kind: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The last observed process state (snake_case).
    pub process_state: String,
    /// The last observed health state (snake_case).
    pub health_state: String,
    /// The last observed Git working-copy state (snake_case).
    pub git_state: String,
    /// The branch currently checked out, when known.
    pub branch: Option<String>,
    /// The first declared port, when the service declares one.
    pub port: Option<u16>,
}

impl From<ServiceStatus> for ServiceSummary {
    fn from(status: ServiceStatus) -> Self {
        Self {
            id: status.service_id.to_string(),
            display_name: status.display_name,
            kind: status.kind.label().to_owned(),
            workspace_id: status.workspace_id.to_string(),
            process_state: enum_value(&status.process_state),
            health_state: enum_value(&status.health_state),
            git_state: enum_value(&status.git_state),
            branch: status.branch.map(|branch| branch.to_string()),
            port: status.ports.first().map(|port| port.value()),
        }
    }
}

/// A workspace with the status of each of its services.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkspaceStatus {
    /// The workspace identifier.
    pub workspace_id: String,
    /// The human-readable workspace name.
    pub name: String,
    /// The absolute filesystem root of the workspace.
    pub root: String,
    /// One row per configured service.
    pub services: Vec<ServiceSummary>,
}

impl From<CoreWorkspaceStatus> for WorkspaceStatus {
    fn from(status: CoreWorkspaceStatus) -> Self {
        Self {
            workspace_id: status.workspace_id.to_string(),
            name: status.name,
            root: status.root.to_string(),
            services: status
                .services
                .into_iter()
                .map(ServiceSummary::from)
                .collect(),
        }
    }
}

/// Serializes a snake_case domain enum to its wire string.
///
/// The core types already declare `#[serde(rename_all = "snake_case")]`, so
/// serde is the single source of truth for the strings the frontend types
/// declare.
fn enum_value<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The normalized outcome of a mutating operation.
///
/// The wire shape is deliberately flatter than the core's
/// [`OperationResult`](service_orchestrator_application::OperationResult): the
/// frontend branches on `status` and follows `task_id`/`workflow_id` to observe
/// real task-engine progress rather than re-deriving meaning from a tagged
/// enum (`tasks/019.md` §11, §25).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OperationOutcome {
    /// What the daemon scheduled — or that nothing was needed.
    pub status: OperationStatus,
    /// The scheduled task id, when a single task was scheduled.
    pub task_id: Option<String>,
    /// The scheduled workflow id, when a workflow was scheduled.
    pub workflow_id: Option<String>,
    /// The task or workflow kind label (`start_service`, `start_services`, …).
    pub kind: String,
}

/// How an operation resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    /// A single task was submitted.
    Task,
    /// A workflow (bulk operation) was submitted.
    Workflow,
    /// The target was already in the requested state; nothing was scheduled.
    AlreadyInDesiredState,
}

impl From<OperationResult> for OperationOutcome {
    fn from(result: OperationResult) -> Self {
        match result {
            OperationResult::TaskScheduled { task_id, kind } => Self {
                status: OperationStatus::Task,
                task_id: Some(task_id.to_string()),
                workflow_id: None,
                kind: kind.label().to_owned(),
            },
            OperationResult::WorkflowScheduled { workflow_id, kind } => Self {
                status: OperationStatus::Workflow,
                task_id: None,
                workflow_id: Some(workflow_id.to_string()),
                kind: kind.label().to_owned(),
            },
            OperationResult::AlreadyInDesiredState { kind } => Self {
                status: OperationStatus::AlreadyInDesiredState,
                task_id: None,
                workflow_id: None,
                kind: kind.label().to_owned(),
            },
        }
    }
}

/// A runtime requirement in display form (`java 21`, `node 20.19.4`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuntimeView {
    /// The runtime label (`java`, `node`, `maven`, `npm`, or a custom label).
    pub kind: String,
    /// The pinned version, when the service pins one.
    pub version: Option<String>,
}

impl From<&Runtime> for RuntimeView {
    fn from(runtime: &Runtime) -> Self {
        Self {
            kind: runtime.kind.label().to_owned(),
            version: runtime.version.as_ref().map(ToString::to_string),
        }
    }
}

/// The source repository of a service.
///
/// The URL is the validated configuration value; the domain rejects embedded
/// credentials, so no secret can reach the webview (`tasks/019.md` §18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RepositoryView {
    /// The clone URL (never containing credentials).
    pub url: String,
    /// The checkout path relative to the workspace root.
    pub local_path: String,
    /// The branch checked out when no other branch is requested.
    pub default_branch: String,
    /// The hosting provider label.
    pub provider: String,
}

impl From<&Repository> for RepositoryView {
    fn from(repository: &Repository) -> Self {
        Self {
            url: repository.url.clone(),
            local_path: repository.local_path.as_str().to_owned(),
            default_branch: repository.default_branch.to_string(),
            provider: repository.provider.to_string(),
        }
    }
}

/// A declared dependency of a service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DependencyView {
    /// The dependency target's identifier.
    pub target: String,
    /// Whether the target is a `service` or a `library`.
    pub target_kind: String,
    /// Whether the dependency is needed for `build` or `runtime`.
    pub kind: String,
    /// Whether the dependency is mandatory.
    pub required: bool,
}

impl From<&Dependency> for DependencyView {
    fn from(dependency: &Dependency) -> Self {
        let (target_kind, target) = match &dependency.target {
            DependencyTarget::Service(id) => ("service", id.to_string()),
            DependencyTarget::Library(id) => ("library", id.to_string()),
        };
        Self {
            target,
            target_kind: target_kind.to_owned(),
            kind: dependency.kind.to_string(),
            required: dependency.required,
        }
    }
}

/// A configured health check of a service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HealthCheckView {
    /// A display label such as `process`, `port 8081`, or `http <url>`.
    pub kind: String,
    /// Whether the check is required for the service to be considered healthy.
    pub required: bool,
}

impl From<&HealthCheck> for HealthCheckView {
    fn from(check: &HealthCheck) -> Self {
        Self {
            kind: check.kind.to_string(),
            required: check.required,
        }
    }
}

/// The latest observed outcome of one configured health check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HealthOutcomeView {
    /// The probe type (`process`, `tcp`, `http`, or a provider label).
    pub check_type: String,
    /// What was probed, if anything (`localhost:8081`, an HTTP URL).
    pub target: Option<String>,
    /// Whether the check is required.
    pub required: bool,
    /// The observed check state (snake_case `HealthState`).
    pub state: String,
    /// How long the probe took, in milliseconds.
    pub duration_ms: u64,
    /// A short, redacted explanation, present for a failure.
    pub message: Option<String>,
    /// When the check ran, seconds since the Unix epoch.
    pub checked_at: i64,
}

impl From<&HealthCheckOutcome> for HealthOutcomeView {
    fn from(outcome: &HealthCheckOutcome) -> Self {
        Self {
            check_type: outcome.check_type.clone(),
            target: outcome.target.clone(),
            required: outcome.required,
            state: enum_value(&outcome.status),
            duration_ms: outcome.duration_ms,
            message: outcome.message.clone(),
            checked_at: timestamp_seconds(outcome.checked_at),
        }
    }
}

/// A structured, observed failure recorded in a diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticFailureView {
    /// Where the failure was observed (`health`, `log`, or `runtime`).
    pub source: String,
    /// A stable, machine-readable code.
    pub code: String,
    /// A human-readable summary.
    pub message: String,
    /// When the failure was observed, seconds since the Unix epoch.
    pub at: Option<i64>,
}

/// A detailed view of one service: configuration plus observed facts.
///
/// The detail never invents a value. Anything the adapter cannot observe yet —
/// for example the current commit, which needs a Git operation deferred to a
/// later task — is simply absent (`tasks/019.md` §18).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServiceDetail {
    /// The service identifier.
    pub id: String,
    /// The human-readable service name.
    pub display_name: String,
    /// `backend`, `frontend`, or a project-specific category.
    pub kind: String,
    /// The workspace that owns the service.
    pub workspace_id: String,
    /// The service directory relative to the workspace root, when configured.
    pub path: Option<String>,
    /// The source repository, when configured.
    pub repository: Option<RepositoryView>,
    /// The runtimes the service requires.
    pub runtimes: Vec<RuntimeView>,
    /// The configured build system, when known.
    pub build_system: Option<String>,
    /// The capabilities the service declares (snake_case).
    pub capabilities: Vec<String>,
    /// The last observed process state (snake_case).
    pub process_state: String,
    /// The last observed health state (snake_case).
    pub health_state: String,
    /// The last observed Git working-copy state (snake_case).
    pub git_state: String,
    /// The branch currently checked out, when known.
    pub branch: Option<String>,
    /// Whether the service is currently tracked as a managed process.
    pub managed: bool,
    /// The operating-system process id, while the process is alive.
    pub pid: Option<u32>,
    /// The ports the service declares.
    pub ports: Vec<u16>,
    /// The declared dependencies.
    pub dependencies: Vec<DependencyView>,
    /// The configured health checks.
    pub health_checks: Vec<HealthCheckView>,
    /// The latest outcome of each configured health check.
    pub health_outcomes: Vec<HealthOutcomeView>,
    /// When the service was last checked, seconds since the Unix epoch.
    pub last_checked_at: Option<i64>,
    /// The most recent structured failure, when one was observed.
    pub last_failure: Option<DiagnosticFailureView>,
    /// The most recent error-level log lines for the service.
    pub recent_errors: Vec<String>,
}

impl ServiceDetail {
    /// Combines configuration, observed status, and diagnostics.
    #[must_use]
    pub fn new(service: Service, status: ServiceStatus, diagnostics: ServiceDiagnostics) -> Self {
        Self {
            id: service.id.to_string(),
            display_name: service.display_name,
            kind: service.kind.label().to_owned(),
            workspace_id: status.workspace_id.to_string(),
            path: service.path.as_ref().map(|path| path.as_str().to_owned()),
            repository: service.repository.as_ref().map(RepositoryView::from),
            runtimes: service.runtimes.iter().map(RuntimeView::from).collect(),
            build_system: service.build_system.as_ref().map(ToString::to_string),
            capabilities: service.capabilities.iter().map(enum_value).collect(),
            process_state: enum_value(&status.process_state),
            health_state: enum_value(&status.health_state),
            git_state: enum_value(&status.git_state),
            branch: status.branch.map(|branch| branch.to_string()),
            managed: diagnostics.managed,
            pid: diagnostics.pid,
            ports: status.ports.iter().map(|port| port.value()).collect(),
            dependencies: service
                .dependencies
                .iter()
                .map(DependencyView::from)
                .collect(),
            health_checks: service
                .health_checks
                .iter()
                .map(HealthCheckView::from)
                .collect(),
            health_outcomes: diagnostics
                .health_checks
                .iter()
                .map(HealthOutcomeView::from)
                .collect(),
            last_checked_at: diagnostics.last_checked_at.map(timestamp_seconds),
            last_failure: diagnostics
                .last_failure
                .as_ref()
                .map(|failure| DiagnosticFailureView {
                    source: failure.source.clone(),
                    code: failure.code.clone(),
                    message: failure.message.clone(),
                    at: failure.at.map(timestamp_seconds),
                }),
            recent_errors: diagnostics.recent_errors,
        }
    }
}

/// Converts a domain timestamp to whole seconds since the Unix epoch.
fn timestamp_seconds(timestamp: Timestamp) -> i64 {
    i64::try_from(timestamp.epoch_millis() / 1000).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use service_orchestrator_application::{DiagnosticFailure, HealthState};
    use service_orchestrator_domain::{
        Branch, GitState, HealthCheckKind, Port, ProcessState, RelativePath, RuntimeKind,
        ServiceId, ServiceType, TaskId, TaskKind, Timestamp, Version, WorkflowId, WorkflowKind,
        WorkspaceId,
    };

    use super::*;

    fn status() -> ServiceStatus {
        ServiceStatus {
            service_id: ServiceId::new("orders-api").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            display_name: "orders-api".to_owned(),
            kind: ServiceType::Backend,
            process_state: ProcessState::NotRunning,
            health_state: HealthState::Unknown,
            git_state: GitState::Clean,
            branch: Some(Branch::new("main").expect("a valid branch")),
            ports: vec![Port::new(8081).expect("a valid port")],
            capabilities: Default::default(),
        }
    }

    fn diagnostics() -> ServiceDiagnostics {
        ServiceDiagnostics {
            service_id: ServiceId::new("orders-api").expect("a valid id"),
            workspace_id: WorkspaceId::new("demo").expect("a valid id"),
            process_state: ProcessState::Running,
            health_state: HealthState::Unhealthy,
            managed: true,
            process_id: Some("proc-1".to_owned()),
            pid: Some(4321),
            ports: vec![Port::new(8081).expect("a valid port")],
            last_checked_at: Some(Timestamp::now()),
            health_checks: vec![HealthCheckOutcome {
                check_type: "http".to_owned(),
                target: Some("http://localhost:8081/actuator/health".to_owned()),
                required: true,
                status: HealthState::Unhealthy,
                duration_ms: 12,
                message: Some("connection refused".to_owned()),
                checked_at: Timestamp::now(),
            }],
            last_failure: Some(DiagnosticFailure {
                source: "health".to_owned(),
                code: "HTTP_CONNECTION_REFUSED".to_owned(),
                message: "connection refused".to_owned(),
                at: Some(Timestamp::now()),
            }),
            recent_errors: vec!["port 8081 in use".to_owned()],
            notes: Vec::new(),
        }
    }

    fn service() -> Service {
        let mut service = Service::new(
            ServiceId::new("orders-api").expect("a valid id"),
            "orders-api",
            ServiceType::Backend,
        )
        .expect("a valid service");
        service = service
            .with_path(RelativePath::new("backend/orders-api").expect("a valid path"))
            .with_runtime(Runtime::versioned(
                RuntimeKind::Java,
                Version::new("21").expect("a valid version"),
            ))
            .with_dependency(Dependency::runtime_service(
                ServiceId::new("admin-ui").expect("a valid id"),
            ))
            .with_health_check(HealthCheck::new(HealthCheckKind::Process))
            .with_port(Port::new(8081).expect("a valid port"));
        service.build_system = Some(service_orchestrator_domain::BuildSystem::Maven);
        service
    }

    #[test]
    fn service_summaries_carry_the_fields_the_table_renders() {
        let summary = ServiceSummary::from(status());
        assert_eq!(summary.id, "orders-api");
        assert_eq!(summary.display_name, "orders-api");
        assert_eq!(summary.kind, "backend");
        assert_eq!(summary.workspace_id, "demo");
        assert_eq!(summary.process_state, "not_running");
        assert_eq!(summary.health_state, "unknown");
        assert_eq!(summary.git_state, "clean");
        assert_eq!(summary.branch.as_deref(), Some("main"));
        assert_eq!(summary.port, Some(8081));
    }

    #[test]
    fn missing_branch_and_ports_stay_null() {
        let mut status = status();
        status.branch = None;
        status.ports = Vec::new();
        let summary = ServiceSummary::from(status);
        assert_eq!(summary.branch, None);
        assert_eq!(summary.port, None);
    }

    #[test]
    fn open_service_kinds_keep_their_custom_label() {
        let mut status = status();
        status.kind = ServiceType::Other("worker".to_owned());
        assert_eq!(ServiceSummary::from(status).kind, "worker");
    }

    #[test]
    fn workspace_summaries_count_configured_services() {
        let workspace = Workspace::new(
            WorkspaceId::new("demo").expect("a valid id"),
            "Demo Platform",
            service_orchestrator_domain::AbsolutePath::new("/Users/dev/Projects/demo")
                .expect("a valid path"),
        )
        .expect("a valid workspace");
        let summary = WorkspaceSummary::from(workspace);
        assert_eq!(summary.id, "demo");
        assert_eq!(summary.name, "Demo Platform");
        assert_eq!(summary.service_count, 0);
        assert_eq!(summary.root, "/Users/dev/Projects/demo");
    }

    #[test]
    fn process_and_health_states_serialize_snake_case() {
        assert_eq!(enum_value(&ProcessState::NotRunning), "not_running");
        assert_eq!(enum_value(&ProcessState::Running), "running");
        assert_eq!(enum_value(&HealthState::Unhealthy), "unhealthy");
    }

    #[test]
    fn operation_outcomes_flatten_each_variant() {
        let task = OperationOutcome::from(OperationResult::TaskScheduled {
            task_id: TaskId::new("task-1").expect("a valid id"),
            kind: TaskKind::StartService,
        });
        assert_eq!(task.status, OperationStatus::Task);
        assert_eq!(task.task_id.as_deref(), Some("task-1"));
        assert_eq!(task.workflow_id, None);
        assert_eq!(task.kind, "start_service");

        let workflow = OperationOutcome::from(OperationResult::WorkflowScheduled {
            workflow_id: WorkflowId::new("wf-1").expect("a valid id"),
            kind: WorkflowKind::Custom("start_services".to_owned()),
        });
        assert_eq!(workflow.status, OperationStatus::Workflow);
        assert_eq!(workflow.workflow_id.as_deref(), Some("wf-1"));
        assert_eq!(workflow.task_id, None);
        assert_eq!(workflow.kind, "start_services");

        let noop = OperationOutcome::from(OperationResult::AlreadyInDesiredState {
            kind: TaskKind::StopService,
        });
        assert_eq!(noop.status, OperationStatus::AlreadyInDesiredState);
        assert_eq!(noop.task_id, None);
        assert_eq!(noop.workflow_id, None);
        assert_eq!(noop.kind, "stop_service");
    }

    #[test]
    fn operation_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(OperationStatus::AlreadyInDesiredState).expect("serializes"),
            serde_json::json!("already_in_desired_state")
        );
    }

    #[test]
    fn service_detail_combines_configuration_status_and_diagnostics() {
        let mut observed = status();
        observed.process_state = ProcessState::Running;
        observed.health_state = HealthState::Unhealthy;
        let detail = ServiceDetail::new(service(), observed, diagnostics());
        assert_eq!(detail.id, "orders-api");
        assert_eq!(detail.kind, "backend");
        assert_eq!(detail.workspace_id, "demo");
        assert_eq!(detail.path.as_deref(), Some("backend/orders-api"));
        assert_eq!(detail.runtimes.len(), 1);
        assert_eq!(detail.runtimes[0].kind, "java");
        assert_eq!(detail.runtimes[0].version.as_deref(), Some("21"));
        assert_eq!(detail.build_system.as_deref(), Some("maven"));
        assert_eq!(detail.dependencies.len(), 1);
        assert_eq!(detail.dependencies[0].target, "admin-ui");
        assert_eq!(detail.dependencies[0].target_kind, "service");
        assert_eq!(detail.dependencies[0].kind, "runtime");
        assert!(detail.dependencies[0].required);
        assert_eq!(detail.health_checks.len(), 1);
        assert_eq!(detail.health_checks[0].kind, "process");
        assert!(detail.health_checks[0].required);
        assert_eq!(detail.ports, vec![8081]);
        assert!(detail.managed);
        assert_eq!(detail.pid, Some(4321));
        // Observed status wins over the summary's not-running observation.
        assert_eq!(detail.process_state, "running");
        assert_eq!(detail.health_state, "unhealthy");
        assert_eq!(detail.branch.as_deref(), Some("main"));
    }

    #[test]
    fn service_detail_maps_health_outcomes_and_recent_failure() {
        let detail = ServiceDetail::new(service(), status(), diagnostics());
        assert_eq!(detail.health_outcomes.len(), 1);
        let outcome = &detail.health_outcomes[0];
        assert_eq!(outcome.check_type, "http");
        assert_eq!(
            outcome.target.as_deref(),
            Some("http://localhost:8081/actuator/health")
        );
        assert_eq!(outcome.state, "unhealthy");
        assert_eq!(outcome.message.as_deref(), Some("connection refused"));
        assert!(detail.last_checked_at.is_some());

        let failure = detail.last_failure.as_ref().expect("a failure is mapped");
        assert_eq!(failure.source, "health");
        assert_eq!(failure.code, "HTTP_CONNECTION_REFUSED");
        assert!(failure.at.is_some());
        assert_eq!(detail.recent_errors, vec!["port 8081 in use".to_owned()]);
    }

    #[test]
    fn unobserved_detail_fields_stay_absent() {
        let mut bare = service();
        bare.repository = None;
        bare.dependencies.clear();
        bare.health_checks.clear();
        let mut diagnostics = diagnostics();
        diagnostics.health_checks.clear();
        diagnostics.last_failure = None;
        diagnostics.pid = None;
        diagnostics.recent_errors.clear();
        let detail = ServiceDetail::new(bare, status(), diagnostics);
        assert!(detail.repository.is_none());
        assert!(detail.dependencies.is_empty());
        assert!(detail.health_checks.is_empty());
        assert!(detail.health_outcomes.is_empty());
        assert!(detail.last_failure.is_none());
        assert_eq!(detail.pid, None);
        assert!(detail.recent_errors.is_empty());
    }
}
