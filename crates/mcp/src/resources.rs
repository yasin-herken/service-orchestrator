//! MCP resources: read-only, structured state exposed as documents.
//!
//! Resources are the read side of the protocol (`AGENTS.md` §25). They let an
//! agent fetch a workspace, service, or task as a structured JSON document
//! instead of parsing tool output. Every resource is backed by an existing
//! application query; none of them mutates state, and none of them reaches past
//! the application core.
//!
//! Only workspaces are enumerated concretely. Workspace-scoped views, services,
//! and tasks are offered as RFC 6570 **templates** so the resource list stays
//! bounded no matter how many services or tasks exist (`tasks/014.md` §24, §34).
//!
//! URIs:
//!
//! - `orchestrator://server` — server metadata;
//! - `workspace://{workspaceId}` — a workspace;
//! - `workspace://{workspaceId}/status` — a workspace status snapshot;
//! - `workspace://{workspaceId}/services` — a workspace's services;
//! - `workspace://{workspaceId}/libraries` — a workspace's libraries;
//! - `service://{serviceId}` — a service status;
//! - `service://{serviceId}/diagnostics` — a service's diagnostics;
//! - `task://{taskId}` — a task snapshot.

use rmcp::model::{ErrorData, Resource, ResourceContents, ResourceTemplate};
use serde_json::{json, Value};
use service_orchestrator_application::ApplicationError;
use service_orchestrator_client::OrchestratorClient;

use crate::{
    parse_service_id, parse_task_id, parse_workspace_id, response, ToolError, MIME_JSON, SERVER_URI,
};

/// The concrete resources the server advertises.
///
/// This is bounded: one static resource plus the (small, configuration-driven)
/// set of workspaces.
pub(crate) fn list(client: &dyn OrchestratorClient) -> Vec<Resource> {
    let mut resources = vec![Resource::new(SERVER_URI, "server")
        .with_description("Service Orchestrator MCP server metadata.")
        .with_mime_type(MIME_JSON)];

    for workspace in client.list_workspaces().unwrap_or_default() {
        let id = workspace.id.to_string();
        resources.push(
            Resource::new(format!("workspace://{id}"), format!("workspace:{id}"))
                .with_description(format!("The configured workspace '{id}'."))
                .with_mime_type(MIME_JSON),
        );
    }

    resources
}

/// The resource URI templates the server advertises.
pub(crate) fn templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new("workspace://{workspaceId}", "workspace")
            .with_description("A workspace by id.")
            .with_mime_type(MIME_JSON),
        ResourceTemplate::new("workspace://{workspaceId}/status", "workspace-status")
            .with_description("A workspace status snapshot.")
            .with_mime_type(MIME_JSON),
        ResourceTemplate::new("workspace://{workspaceId}/services", "workspace-services")
            .with_description("The services configured in a workspace.")
            .with_mime_type(MIME_JSON),
        ResourceTemplate::new("workspace://{workspaceId}/libraries", "workspace-libraries")
            .with_description("The libraries configured in a workspace.")
            .with_mime_type(MIME_JSON),
        ResourceTemplate::new("service://{serviceId}", "service-status")
            .with_description("The structured status of a service.")
            .with_mime_type(MIME_JSON),
        ResourceTemplate::new("service://{serviceId}/diagnostics", "service-diagnostics")
            .with_description("Structured diagnostics for a service.")
            .with_mime_type(MIME_JSON),
        ResourceTemplate::new("task://{taskId}", "task")
            .with_description("A task snapshot, including status and progress.")
            .with_mime_type(MIME_JSON),
    ]
}

/// Builds the protocol contents for one resource URI.
pub(crate) async fn read(
    client: &dyn OrchestratorClient,
    uri: &str,
) -> Result<ResourceContents, ErrorData> {
    let value = read_value(client, uri).await?;
    let text = serde_json::to_string_pretty(&value).map_err(|error| {
        ErrorData::internal_error(format!("failed to encode resource: {error}"), None)
    })?;
    Ok(ResourceContents::text(text, uri).with_mime_type(MIME_JSON))
}

/// Resolves a resource URI into its structured value.
async fn read_value(client: &dyn OrchestratorClient, uri: &str) -> Result<Value, ErrorData> {
    if uri == SERVER_URI {
        return Ok(server_info());
    }

    if let Some(rest) = uri.strip_prefix("workspace://") {
        let mut parts = rest.splitn(2, '/');
        let id = parts.next().unwrap_or_default();
        let sub = parts.next();
        let workspace_id = parse_workspace_id(id).map_err(to_error_data)?;
        return match sub {
            None => query(client.get_workspace(&workspace_id)),
            Some("status") => query(client.get_workspace_status(&workspace_id)),
            Some("services") => query(client.list_services(&workspace_id)),
            Some("libraries") => query(client.list_libraries(&workspace_id)),
            Some(other) => Err(ErrorData::resource_not_found(
                format!("unknown workspace sub-resource '{other}'"),
                None,
            )),
        };
    }

    if let Some(rest) = uri.strip_prefix("service://") {
        let mut parts = rest.splitn(2, '/');
        let id = parts.next().unwrap_or_default();
        let sub = parts.next();
        let service_id = parse_service_id(id).map_err(to_error_data)?;
        return match sub {
            None => query(client.get_service_status(&service_id)),
            Some("diagnostics") => query(client.get_service_diagnostics(&service_id)),
            Some(other) => Err(ErrorData::resource_not_found(
                format!("unknown service sub-resource '{other}'"),
                None,
            )),
        };
    }

    if let Some(rest) = uri.strip_prefix("task://") {
        if rest.contains('/') {
            return Err(ErrorData::resource_not_found(
                format!("unknown task sub-resource in '{uri}'"),
                None,
            ));
        }
        let task_id = parse_task_id(rest).map_err(to_error_data)?;
        return query(client.get_task(&task_id));
    }

    Err(ErrorData::resource_not_found(
        format!("unrecognized resource URI '{uri}'"),
        None,
    ))
}

/// Serializes an application query result into JSON.
fn query<T: serde::Serialize>(result: Result<T, ApplicationError>) -> Result<Value, ErrorData> {
    let value = result.map_err(ToolError::from).map_err(to_error_data)?;
    serde_json::to_value(value).map_err(|error| {
        ErrorData::internal_error(format!("failed to encode resource: {error}"), None)
    })
}

/// Static metadata about this server.
fn server_info() -> Value {
    json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "transport": "stdio",
        "kind": "service-orchestrator",
    })
}

/// Maps a tool-layer error into a resource-read protocol error.
///
/// A resource read has no success/failure envelope, so every application error
/// becomes either "not found" (for a missing or malformed identifier) or an
/// internal error. The message is the agent-facing summary; no lower-level
/// detail is leaked.
fn to_error_data(error: ToolError) -> ErrorData {
    match error {
        ToolError::Protocol(error) => error,
        ToolError::InvalidArgument { message } => ErrorData::resource_not_found(message, None),
        ToolError::Application(error) => match error {
            ApplicationError::WorkspaceNotFound(id) => {
                ErrorData::resource_not_found(format!("workspace '{id}' was not found"), None)
            }
            ApplicationError::ServiceNotFound(id) => {
                ErrorData::resource_not_found(format!("service '{id}' was not found"), None)
            }
            ApplicationError::TaskNotFound(id) => {
                ErrorData::resource_not_found(format!("task '{id}' was not found"), None)
            }
            other => {
                let body = response::error_body(&other);
                ErrorData::internal_error(body.message, Some(json!({ "code": body.code })))
            }
        },
    }
}
