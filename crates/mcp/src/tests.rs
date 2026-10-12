//! Adapter unit tests.
//!
//! These drive the real adapter (catalog, dispatch, resources, prompts, and the
//! response envelope) over a real [`Application`] core wired to a real task
//! engine, but with **no executors registered** and no infrastructure ports: a
//! scheduled task is created and observable but never runs, so no Git command,
//! Maven build, Liquibase migration, or operating-system process is ever
//! started (`tasks/014.md` §36).
//!
//! The end-to-end stdio test lives at the repository root (`tests/mcp_stdio.rs`)
//! and exercises the same behavior through the real binary.

use std::collections::HashSet;
use std::sync::Arc;

use rmcp::model::{ErrorCode, ResourceContents};
use serde_json::{json, Value};
use service_orchestrator_application::{
    Application, ApplicationError, InMemoryRuntimeStateStore, NullLogService, OperationResult,
};
use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_config::load_from_str;
use service_orchestrator_domain::{Permission, TaskId, TaskKind};
use service_orchestrator_execution::TaskEngine;

use crate::server::OrchestratorServer;
use crate::{prompts, resources, response, TaskMonitor, ToolError};

/// The fixture configuration shared by these tests.
const CONFIG: &str = include_str!("../tests/fixtures/config.toml");

/// The adapter under test, sharing one task engine across every call.
struct Harness {
    server: OrchestratorServer,
}

impl Harness {
    /// Builds the adapter over the fixture configuration and a real task
    /// engine.
    fn new() -> Self {
        let config = load_from_str(CONFIG, "<test>").expect("the fixture configuration is valid");
        let engine = Arc::new(
            TaskEngine::builder()
                .max_parallel_tasks(4)
                .build()
                .expect("a Tokio runtime is active in these tests"),
        );
        let application = Application::new(
            config,
            engine.clone(),
            Arc::new(InMemoryRuntimeStateStore::new()),
            Arc::new(NullLogService),
        );
        let monitor: Arc<dyn TaskMonitor> = engine.clone();
        let client: Arc<dyn OrchestratorClient> = Arc::new(application);
        Self {
            server: OrchestratorServer::new(client, monitor),
        }
    }

    /// Calls a tool by name with the given JSON arguments.
    ///
    /// `Value::Null` means "no arguments"; an object is passed through.
    async fn call(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        let arguments = match arguments {
            Value::Null => None,
            Value::Object(map) => Some(map),
            _ => panic!("tool arguments must be an object or null"),
        };
        self.server.dispatch(name, arguments).await
    }

    /// Calls a tool and expects success, returning its `data` payload.
    async fn call_ok(&self, name: &str, arguments: Value) -> Value {
        match self.call(name, arguments).await {
            Ok(value) => value,
            Err(error) => panic!("expected '{name}' to succeed, got {error:?}"),
        }
    }

    /// Calls a tool and expects an application-level failure envelope.
    async fn call_app_error(&self, name: &str, arguments: Value) -> response::ErrorBody {
        match self.call(name, arguments).await {
            Err(ToolError::Application(error)) => response::error_body(&error),
            other => {
                panic!("expected '{name}' to fail with an application error, got {other:?}")
            }
        }
    }

    /// Reads one resource and returns its raw contents.
    async fn read_resource(&self, uri: &str) -> ResourceContents {
        let client = self.server.client_for_test();
        resources::read(client.as_ref(), uri)
            .await
            .unwrap_or_else(|error| panic!("reading '{uri}' failed: {error:?}"))
    }
}

/// Extracts the `text` field from a text resource.
fn resource_text(contents: &ResourceContents) -> String {
    serde_json::to_value(contents)
        .expect("resource contents serialize")
        .get("text")
        .and_then(Value::as_str)
        .expect("a text resource")
        .to_owned()
}

// -- discovery and schemas ---------------------------------------------------

#[test]
fn catalog_exposes_unique_well_formed_tools() {
    let tools = crate::catalog::tools();
    assert!(tools.len() >= 40, "the catalog is substantial");

    let mut names = HashSet::new();
    for tool in &tools {
        assert!(
            names.insert(tool.name.to_string()),
            "tool '{}' is advertised twice",
            tool.name
        );
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(
            !description.trim().is_empty(),
            "tool '{}' has no description",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type").and_then(Value::as_str),
            Some("object"),
            "tool '{}' does not advertise an object schema",
            tool.name
        );
        assert!(
            tool.annotations.is_some(),
            "tool '{}' has no risk annotations",
            tool.name
        );
    }
}

#[test]
fn read_only_tools_declare_read_only_and_destructive_tools_declare_destructive() {
    let tools = crate::catalog::tools();
    let find = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("tool '{name}' is advertised"))
            .annotations
            .clone()
            .expect("annotations are present")
    };

    let status = find("git_status");
    assert_eq!(status.read_only_hint, Some(true));
    assert_eq!(status.destructive_hint, Some(false));

    let build = find("build_service");
    assert_eq!(build.read_only_hint, Some(false));
    assert_eq!(build.destructive_hint, Some(false));

    let reset = find("git_reset_hard");
    assert_eq!(reset.read_only_hint, Some(false));
    assert_eq!(reset.destructive_hint, Some(true));

    // Planning an environment preparation is read-only; cancelling a workflow
    // is a plain (non-destructive) write.
    let plan = find("plan_prepare_environment");
    assert_eq!(plan.read_only_hint, Some(true));
    assert_eq!(plan.destructive_hint, Some(false));

    let cancel = find("cancel_workflow");
    assert_eq!(cancel.read_only_hint, Some(false));
    assert_eq!(cancel.destructive_hint, Some(false));
}

#[tokio::test]
async fn every_advertised_tool_routes_to_a_dispatch_arm() {
    // A tool that is advertised but has no dispatch arm would answer with
    // METHOD_NOT_FOUND. Every real tool answers with either success or a
    // validation error instead; none may be unroutable.
    let harness = Harness::new();
    for tool in crate::catalog::tools() {
        match harness.call(&tool.name, Value::Null).await {
            Err(ToolError::Protocol(error)) => assert_ne!(
                error.code,
                ErrorCode::METHOD_NOT_FOUND,
                "tool '{}' is advertised but not dispatched",
                tool.name
            ),
            Ok(_) | Err(ToolError::Application(_)) | Err(ToolError::InvalidArgument { .. }) => {}
        }
    }
}

#[tokio::test]
async fn unknown_tool_is_a_protocol_error() {
    let harness = Harness::new();
    match harness.call("no_such_tool", Value::Null).await {
        Err(ToolError::Protocol(error)) => {
            assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
        }
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

// -- structured responses ----------------------------------------------------

#[tokio::test]
async fn list_workspaces_returns_real_fixture_state() {
    let harness = Harness::new();
    let data = harness.call_ok("list_workspaces", Value::Null).await;
    let workspaces = data.as_array().expect("a list of workspaces");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(
        workspaces[0].get("id").and_then(Value::as_str),
        Some("demo")
    );
}

#[tokio::test]
async fn get_service_returns_the_structured_service() {
    let harness = Harness::new();
    let data = harness
        .call_ok("get_service", json!({ "serviceId": "orders-api" }))
        .await;
    assert_eq!(data.get("id").and_then(Value::as_str), Some("orders-api"));
    assert_eq!(
        data.get("display_name").and_then(Value::as_str),
        Some("orders-api")
    );
}

#[tokio::test]
async fn get_workspace_status_is_structured() {
    let harness = Harness::new();
    let data = harness
        .call_ok("get_workspace_status", json!({ "workspaceId": "demo" }))
        .await;
    assert_eq!(
        data.get("workspace_id").and_then(Value::as_str),
        Some("demo")
    );
    assert!(data.get("services").is_some());
}

#[tokio::test]
async fn validate_workspace_is_a_read_only_structured_report() {
    let harness = Harness::new();
    let data = harness
        .call_ok("validate_workspace", json!({ "workspaceId": "demo" }))
        .await;
    assert_eq!(
        data.get("workspace_id").and_then(Value::as_str),
        Some("demo")
    );

    let totals = data
        .get("totals")
        .expect("the report carries aggregate totals");
    assert_eq!(totals.get("services").and_then(Value::as_u64), Some(2));
    assert_eq!(
        totals.get("backend_services").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        totals.get("frontend_services").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(totals.get("libraries").and_then(Value::as_u64), Some(1));

    for field in [
        "repositories",
        "diagnostics",
        "port_conflicts",
        "occupied_ports",
    ] {
        assert!(
            data.get(field).and_then(Value::as_array).is_some(),
            "the report exposes '{field}'"
        );
    }
}

#[tokio::test]
async fn validate_workspace_of_an_unknown_workspace_reports_not_found() {
    let harness = Harness::new();
    let body = harness
        .call_app_error("validate_workspace", json!({ "workspaceId": "missing" }))
        .await;
    assert_eq!(body.code, "WORKSPACE_NOT_FOUND");
}

// -- structured errors -------------------------------------------------------

#[tokio::test]
async fn a_valid_but_unknown_service_reports_service_not_found() {
    let harness = Harness::new();
    let body = harness
        .call_app_error("get_service", json!({ "serviceId": "missing" }))
        .await;
    assert_eq!(body.code, "SERVICE_NOT_FOUND");
    assert!(!body.message.is_empty());
}

#[tokio::test]
async fn a_malformed_identifier_reports_invalid_argument() {
    let harness = Harness::new();
    match harness
        .call("get_service", json!({ "serviceId": "not a valid id!" }))
        .await
    {
        Err(ToolError::InvalidArgument { message }) => assert!(!message.is_empty()),
        other => panic!("expected an invalid-argument error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_wrongly_typed_argument_is_a_protocol_error() {
    // The JSON shape is wrong, so the request never routed.
    let harness = Harness::new();
    match harness.call("get_service", json!({ "serviceId": 5 })).await {
        Err(ToolError::Protocol(error)) => assert_eq!(error.code, ErrorCode::INVALID_PARAMS),
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_argument_is_rejected() {
    // `deny_unknown_fields` means an unmodelled option cannot sneak past
    // validation.
    let harness = Harness::new();
    match harness
        .call(
            "get_service",
            json!({ "serviceId": "orders-api", "force": true }),
        )
        .await
    {
        Err(ToolError::Protocol(error)) => assert_eq!(error.code, ErrorCode::INVALID_PARAMS),
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

// -- policy enforcement ------------------------------------------------------

#[tokio::test]
async fn a_destructive_operation_requires_confirmation() {
    let harness = Harness::new();
    let body = harness
        .call_app_error("git_reset_hard", json!({ "serviceId": "orders-api" }))
        .await;
    assert_eq!(body.code, "CONFIRMATION_REQUIRED");
}

#[test]
fn error_codes_distinguish_confirmation_from_denial() {
    let destructive = ApplicationError::PermissionDenied {
        permission: Permission::Destructive,
        message: "confirmation required".to_owned(),
    };
    assert_eq!(
        response::error_body(&destructive).code,
        "CONFIRMATION_REQUIRED"
    );

    let forbidden = ApplicationError::PermissionDenied {
        permission: Permission::SafeWrite,
        message: "not permitted".to_owned(),
    };
    assert_eq!(response::error_body(&forbidden).code, "PERMISSION_DENIED");
}

// -- tasks -------------------------------------------------------------------

#[tokio::test]
async fn a_safe_write_schedules_a_queryable_task() {
    let harness = Harness::new();
    let data = harness
        .call_ok("build_service", json!({ "serviceId": "orders-api" }))
        .await;
    assert_eq!(
        data.get("outcome").and_then(Value::as_str),
        Some("task_scheduled")
    );
    let task_id = data
        .get("taskId")
        .and_then(Value::as_str)
        .expect("a task id is returned");
    assert!(
        data.get("status").and_then(Value::as_str).is_some(),
        "a live status is attached to a scheduled task"
    );

    // The returned id refers to a real task in the shared engine.
    let snapshot = harness
        .call_ok("get_task", json!({ "taskId": task_id }))
        .await;
    assert_eq!(snapshot.get("id").and_then(Value::as_str), Some(task_id));
    assert!(snapshot.get("status").and_then(Value::as_str).is_some());
    assert!(snapshot.get("progress").is_some());

    // ...and it is listed by the task monitor.
    let tasks = harness.call_ok("list_tasks", Value::Null).await;
    assert!(tasks
        .as_array()
        .expect("a list of tasks")
        .iter()
        .any(|task| task.get("id").and_then(Value::as_str) == Some(task_id)));

    // Cancelling is a real engine operation.
    let cancelled = harness
        .call_ok("cancel_task", json!({ "taskId": task_id }))
        .await;
    assert_eq!(
        cancelled.get("taskId").and_then(Value::as_str),
        Some(task_id)
    );
}

#[tokio::test]
async fn get_task_reports_task_not_found_for_an_unknown_task() {
    let harness = Harness::new();
    let body = harness
        .call_app_error("get_task", json!({ "taskId": "task-does-not-exist" }))
        .await;
    assert_eq!(body.code, "TASK_NOT_FOUND");
}

// -- environment workflows ---------------------------------------------------

#[tokio::test]
async fn plan_prepare_environment_returns_a_structured_plan() {
    let harness = Harness::new();
    let data = harness
        .call_ok(
            "plan_prepare_environment",
            json!({
                "workspaceId": "demo",
                "serviceIds": ["admin-ui"],
                "build": true,
                "start": true
            }),
        )
        .await;

    assert_eq!(
        data.get("workspace_id").and_then(Value::as_str),
        Some("demo")
    );
    let selected = data["selected_services"]
        .as_array()
        .expect("a list of selected services");
    assert!(selected.iter().any(|id| id.as_str() == Some("admin-ui")));

    // admin-ui depends on orders-api, so the dependency closure includes it.
    let resolved: Vec<&str> = data["resolved_services"]
        .as_array()
        .expect("a list of resolved services")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(resolved.contains(&"orders-api"));

    assert!(data["phases"]
        .as_array()
        .is_some_and(|phases| !phases.is_empty()));
    // The fixture's only migration target is local, so no confirmation is due.
    assert_eq!(
        data.get("requires_destructive_authorization")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[tokio::test]
async fn prepare_environment_schedules_a_queryable_workflow() {
    let harness = Harness::new();
    let data = harness
        .call_ok(
            "prepare_environment",
            json!({
                "workspaceId": "demo",
                "serviceIds": ["orders-api"],
                "build": true,
                "start": true
            }),
        )
        .await;
    assert_eq!(
        data.get("outcome").and_then(Value::as_str),
        Some("workflow_scheduled")
    );
    let workflow_id = data
        .get("workflowId")
        .and_then(Value::as_str)
        .expect("a workflow id");

    // The workflow is immediately observable through the shared engine.
    let status = harness
        .call_ok("get_workflow_status", json!({ "workflowId": workflow_id }))
        .await;
    assert_eq!(
        status.get("workflow_id").and_then(Value::as_str),
        Some(workflow_id)
    );
    assert!(status.get("state").and_then(Value::as_str).is_some());
    assert!(status.get("phases").and_then(Value::as_array).is_some());
    assert!(status.get("total").and_then(Value::as_u64).is_some());

    // Cancellation is a real engine operation and reports a structured result.
    let cancelled = harness
        .call_ok("cancel_workflow", json!({ "workflowId": workflow_id }))
        .await;
    assert!(cancelled
        .get("already_complete")
        .and_then(Value::as_bool)
        .is_some());
    assert!(cancelled
        .get("cancelled_tasks")
        .and_then(Value::as_u64)
        .is_some());
}

#[tokio::test]
async fn get_workflow_status_reports_workflow_not_found() {
    let harness = Harness::new();
    let body = harness
        .call_app_error("get_workflow_status", json!({ "workflowId": "missing" }))
        .await;
    assert_eq!(body.code, "WORKFLOW_NOT_FOUND");
}

#[test]
fn operation_result_renders_a_live_status() {
    let result = OperationResult::TaskScheduled {
        task_id: TaskId::new("task-1").expect("valid id"),
        kind: TaskKind::BuildService,
    };
    let value = response::operation_result(&result, Some("running"));
    assert_eq!(
        value.get("outcome").and_then(Value::as_str),
        Some("task_scheduled")
    );
    assert_eq!(value.get("status").and_then(Value::as_str), Some("running"));
}

// -- secret redaction --------------------------------------------------------

#[tokio::test]
async fn secret_environment_values_are_redacted_in_tool_output() {
    let harness = Harness::new();
    let data = harness
        .call_ok("get_service", json!({ "serviceId": "orders-api" }))
        .await;
    let environment = data
        .get("environment")
        .and_then(Value::as_object)
        .expect("the service exposes its environment");

    let password = environment
        .get("DB_PASSWORD")
        .expect("the secret variable is present");
    assert_eq!(
        password.get("value").and_then(Value::as_str),
        Some("***"),
        "a secret value must never be serialized"
    );

    let serialized = data.to_string();
    assert!(
        !serialized.contains("super-secret-value"),
        "the raw secret must not appear anywhere in the response"
    );
}

// -- resources ---------------------------------------------------------------

#[test]
fn resource_templates_are_advertised() {
    let templates = resources::templates();
    assert!(!templates.is_empty());
    let uris: HashSet<&str> = templates
        .iter()
        .map(|template| template.uri_template.as_str())
        .collect();
    assert!(uris.contains("workspace://{workspaceId}"));
    assert!(uris.contains("service://{serviceId}"));
    assert!(uris.contains("task://{taskId}"));
}

#[tokio::test]
async fn concrete_resources_include_the_server_and_every_workspace() {
    let harness = Harness::new();
    let client = harness.server.client_for_test();
    let listed = resources::list(client.as_ref());
    let uris: HashSet<&str> = listed
        .iter()
        .map(|resource| resource.uri.as_str())
        .collect();
    assert!(uris.contains(crate::SERVER_URI));
    assert!(uris.contains("workspace://demo"));
}

#[tokio::test]
async fn reading_a_workspace_resource_returns_structured_state() {
    let harness = Harness::new();
    let contents = harness.read_resource("workspace://demo").await;
    let value: Value =
        serde_json::from_str(&resource_text(&contents)).expect("resource text is JSON");
    assert_eq!(value.get("id").and_then(Value::as_str), Some("demo"));
}

#[tokio::test]
async fn reading_a_service_resource_returns_its_status() {
    let harness = Harness::new();
    let contents = harness.read_resource("service://orders-api").await;
    let value: Value = serde_json::from_str(&resource_text(&contents)).expect("valid JSON");
    assert_eq!(
        value.get("service_id").and_then(Value::as_str),
        Some("orders-api")
    );
}

#[tokio::test]
async fn reading_an_unknown_resource_is_a_protocol_error() {
    let harness = Harness::new();
    let client = harness.server.client_for_test();
    match resources::read(client.as_ref(), "workspace://demo/nope").await {
        Err(error) => assert_eq!(error.code, ErrorCode::RESOURCE_NOT_FOUND),
        Ok(_) => panic!("an unknown sub-resource must not resolve"),
    }
}

// -- prompts -----------------------------------------------------------------

#[test]
fn prompts_are_advertised_with_required_arguments() {
    let prompts = prompts::list();
    assert!(!prompts.is_empty());
    let prepare = prompts
        .iter()
        .find(|prompt| prompt.name == "prepare-development-environment")
        .expect("the prepare prompt exists");
    let arguments = prepare.arguments.as_ref().expect("it declares arguments");
    assert!(arguments
        .iter()
        .any(|argument| argument.required == Some(true)));
}

#[test]
fn a_prompt_renders_guidance_and_requires_its_argument() {
    let result = prompts::get(
        "debug-service",
        json!({ "serviceId": "orders-api" }).as_object().cloned(),
    )
    .expect("the prompt renders");
    assert!(!result.messages.is_empty());

    match prompts::get("debug-service", None) {
        Err(error) => assert_eq!(error.code, ErrorCode::INVALID_PARAMS),
        Ok(_) => panic!("a missing required argument must fail"),
    }
}

// -- envelope shape ----------------------------------------------------------

#[test]
fn the_response_envelope_has_the_documented_shape() {
    let ok = response::success(json!({ "answer": 42 }));
    assert_eq!(ok.get("success").and_then(Value::as_bool), Some(true));
    assert_eq!(ok.pointer("/data/answer").and_then(Value::as_i64), Some(42));

    let failure = response::failure(response::ErrorBody::new("SERVICE_NOT_FOUND", "gone"));
    assert_eq!(failure.get("success").and_then(Value::as_bool), Some(false));
    assert_eq!(
        failure.pointer("/error/code").and_then(Value::as_str),
        Some("SERVICE_NOT_FOUND")
    );
}
