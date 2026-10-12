//! Read-only view models handed to the frontend.
//!
//! The DTOs are deliberately slimmer than the core types: the GUI renders a
//! table row and a workspace card, not the full domain aggregate. Mapping
//! happens exactly once, here, so neither the frontend nor the Tauri commands
//! re-derive domain data (`tasks/018.md` §6, §11).

use serde::Serialize;
use service_orchestrator_application::{ServiceStatus, WorkspaceStatus as CoreWorkspaceStatus};
use service_orchestrator_domain::{ProcessState, Workspace};

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

/// Returns a process state as its wire string (used by tests and diagnostics).
#[must_use]
pub fn process_state_value(state: ProcessState) -> String {
    enum_value(&state)
}

#[cfg(test)]
mod tests {
    use service_orchestrator_application::HealthState;
    use service_orchestrator_domain::{
        Branch, GitState, Port, ServiceId, ServiceType, WorkspaceId,
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
        let workspace = service_orchestrator_domain::Workspace::new(
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
    fn process_states_serialize_snake_case() {
        assert_eq!(process_state_value(ProcessState::NotRunning), "not_running");
        assert_eq!(process_state_value(ProcessState::Running), "running");
    }
}
