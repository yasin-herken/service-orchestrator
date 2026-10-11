//! The MCP server handler.
//!
//! [`OrchestratorServer`] is the whole adapter. It implements
//! [`ServerHandler`] by mapping each request to an application use case:
//!
//! ```text
//! tools/call ─▶ validate args ─▶ Application use case ─▶ policy ─▶ task engine
//! ```
//!
//! Nothing here touches Git, Maven, npm, Liquibase, the filesystem, or a
//! process. Every long-running operation returns the identifier the application
//! core already produced; the adapter never invents one (`tasks/014.md` §2, §3).
//!
//! # Two kinds of failure
//!
//! The protocol distinguishes a request that could not be routed from one that
//! ran and failed. This adapter follows that split exactly:
//!
//! - a **malformed** call (unknown tool, wrong JSON shape) is a JSON-RPC
//!   protocol error (`Err(ErrorData)`);
//! - a **valid** call that the application refuses or that fails — including a
//!   policy denial — is a normal tool result with `isError: true` and a
//!   structured `{ success: false, error }` envelope.
//!
//! A valid-but-invalid *domain value* (say, a service id with an illegal
//! character) is the second kind: the request routed, so the caller gets a
//! readable `INVALID_ARGUMENT` envelope rather than an opaque protocol error
//! (`tasks/014.md` §16, §28).

use std::sync::Arc;
use std::time::Instant;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, ServerCapabilities,
    ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer};
use rmcp::ErrorData;
use serde::Serialize;
use serde_json::Value;
use service_orchestrator_application::{
    ApplicationError, DiffOptions, LogRequest, PrepareEnvironmentRequest,
    PrepareSelectedLibrariesRequest,
};
use service_orchestrator_client::OrchestratorClient;

use crate::{
    catalog, logging, params, parse_branch, parse_group_id, parse_library_id, parse_profile_id,
    parse_service_id, parse_task_id, parse_workflow_id, parse_workspace_id, prompts, resources,
    response, TaskMonitor, ToolError,
};

/// The `initialize` instructions shown to a connecting client.
const INSTRUCTIONS: &str = "\
Service Orchestrator exposes a local developer environment — Git, Maven/npm \
builds, shared-library management, Liquibase migrations, service processes, \
health checks, logs, and dependency-aware workflows.

Every mutating tool returns a task or workflow identifier immediately and is \
executed asynchronously; poll `get_task` (or read `task://{taskId}`) and \
`get_task_logs` to follow progress. Reads never mutate state.

All operations pass through the same policy engine as the terminal interface. \
Destructive operations (for example `git_reset_hard`) are denied with a \
structured `CONFIRMATION_REQUIRED` error: an agent cannot self-confirm, and a \
human must approve the action interactively. Treat every failure envelope as \
authoritative rather than retrying blindly.";

/// The MCP adapter over the shared application client.
pub(crate) struct OrchestratorServer {
    client: Arc<dyn OrchestratorClient>,
    monitor: Arc<dyn TaskMonitor>,
    tools: Vec<Tool>,
}

impl OrchestratorServer {
    /// Builds the server and its frozen tool catalog.
    pub(crate) fn new(client: Arc<dyn OrchestratorClient>, monitor: Arc<dyn TaskMonitor>) -> Self {
        Self {
            client,
            monitor,
            tools: catalog::tools(),
        }
    }

    /// Returns the application client, for in-crate tests that read resources
    /// directly.
    #[cfg(test)]
    pub(crate) fn client_for_test(&self) -> Arc<dyn OrchestratorClient> {
        Arc::clone(&self.client)
    }

    /// Dispatches one tool call to the application core.
    ///
    /// Returns the `data` payload of a success envelope, or a [`ToolError`].
    pub(crate) async fn dispatch(
        &self,
        name: &str,
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> Result<Value, ToolError> {
        match name {
            // -- workspaces, groups, profiles -------------------------------
            "list_workspaces" => {
                parse::<params::Empty>(arguments)?;
                query(self.client.list_workspaces())
            }
            "get_workspace" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .get_workspace(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "get_workspace_status" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .get_workspace_status(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "list_groups" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .list_groups(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "get_group" => {
                let p = parse::<params::GroupRef>(arguments)?;
                query(self.client.get_group(&parse_group_id(&p.group_id)?))
            }
            "list_profiles" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .list_profiles(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "get_profile" => {
                let p = parse::<params::ProfileRef>(arguments)?;
                query(self.client.get_profile(&parse_profile_id(&p.profile_id)?))
            }
            // -- services ---------------------------------------------------
            "list_services" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .list_services(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "get_service" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(self.client.get_service(&parse_service_id(&p.service_id)?))
            }
            "get_service_status" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(
                    self.client
                        .get_service_status(&parse_service_id(&p.service_id)?),
                )
            }
            "get_service_runtime_status" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(
                    self.client
                        .get_service_runtime_status(&parse_service_id(&p.service_id)?),
                )
            }
            "list_running_services" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .list_running_services(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            // -- libraries --------------------------------------------------
            "list_libraries" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .list_libraries(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "get_library" => {
                let p = parse::<params::LibraryRef>(arguments)?;
                query(self.client.get_library(&parse_library_id(&p.library_id)?))
            }
            "get_library_status" => {
                let p = parse::<params::LibraryRef>(arguments)?;
                query(
                    self.client
                        .get_library_status(&parse_library_id(&p.library_id)?)
                        .await,
                )
            }
            "find_library_dependents" => {
                let p = parse::<params::LibraryRef>(arguments)?;
                query(
                    self.client
                        .find_library_dependents(&parse_library_id(&p.library_id)?),
                )
            }
            "verify_library_artifact" => {
                let p = parse::<params::LibraryRef>(arguments)?;
                query(
                    self.client
                        .verify_library_artifact(&parse_library_id(&p.library_id)?)
                        .await,
                )
            }
            "inspect_library_version_alignment" => {
                let p = parse::<params::LibraryRef>(arguments)?;
                query(
                    self.client
                        .inspect_library_version_alignment(&parse_library_id(&p.library_id)?)
                        .await,
                )
            }
            // -- git, read-only ---------------------------------------------
            "git_status" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(
                    self.client
                        .get_git_status(&parse_service_id(&p.service_id)?)
                        .await,
                )
            }
            "git_list_branches" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(
                    self.client
                        .list_branches(&parse_service_id(&p.service_id)?)
                        .await,
                )
            }
            "git_diff" => {
                let p = parse::<params::ServiceDiff>(arguments)?;
                let mut options = DiffOptions::new().staged(p.staged);
                if let Some(max_bytes) = p.max_bytes {
                    options = options.with_max_bytes(max_bytes);
                }
                query(
                    self.client
                        .get_repository_diff(&parse_service_id(&p.service_id)?, &options)
                        .await,
                )
            }
            // -- git, safe writes -------------------------------------------
            "git_fetch" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .fetch_repository(&parse_service_id(&p.service_id)?),
                )
            }
            "git_pull" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .pull_repository(&parse_service_id(&p.service_id)?),
                )
            }
            "git_checkout" => {
                let p = parse::<params::ServiceBranch>(arguments)?;
                self.operation(self.client.checkout_repository(
                    &parse_service_id(&p.service_id)?,
                    &parse_branch(&p.branch)?,
                ))
            }
            "git_sync" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(self.client.sync_service(&parse_service_id(&p.service_id)?))
            }
            "git_clone" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .clone_repository(&parse_service_id(&p.service_id)?),
                )
            }
            "git_sync_repositories" => {
                let p = parse::<params::WorkspaceServices>(arguments)?;
                self.operation(self.client.sync_repositories(
                    &parse_workspace_id(&p.workspace_id)?,
                    &service_ids(&p.service_ids)?,
                ))
            }
            "git_fetch_repositories" => {
                let p = parse::<params::WorkspaceServices>(arguments)?;
                self.operation(self.client.fetch_repositories(
                    &parse_workspace_id(&p.workspace_id)?,
                    &service_ids(&p.service_ids)?,
                ))
            }
            "git_checkout_repositories" => {
                let p = parse::<params::WorkspaceServicesBranch>(arguments)?;
                self.operation(self.client.checkout_repositories(
                    &parse_workspace_id(&p.workspace_id)?,
                    &service_ids(&p.service_ids)?,
                    &parse_branch(&p.branch)?,
                ))
            }
            // -- git, destructive -------------------------------------------
            "git_reset_hard" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .reset_repository_hard(&parse_service_id(&p.service_id)?),
                )
            }
            "git_clean" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .clean_repository(&parse_service_id(&p.service_id)?),
                )
            }
            "git_discard_changes" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .discard_local_changes(&parse_service_id(&p.service_id)?),
                )
            }
            // -- builds -----------------------------------------------------
            "build_service" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(self.client.build_service(&parse_service_id(&p.service_id)?))
            }
            "build_workspace" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                self.operation(
                    self.client
                        .build_workspace(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "build_library" => {
                let p = parse::<params::LibraryRef>(arguments)?;
                self.operation(self.client.build_library(&parse_library_id(&p.library_id)?))
            }
            "install_library" => {
                let p = parse::<params::LibraryRef>(arguments)?;
                self.operation(
                    self.client
                        .install_library(&parse_library_id(&p.library_id)?),
                )
            }
            "prepare_library" => {
                let p = parse::<params::PrepareLibraryRequest>(arguments)?;
                let (library_id, options) = p.into_parts()?;
                self.operation(self.client.prepare_library(&library_id, &options))
            }
            "prepare_selected_libraries" => {
                let p = parse::<params::PrepareSelectedLibrariesRequest>(arguments)?;
                let (workspace_id, libraries, options) = p.into_parts()?;
                let request = PrepareSelectedLibrariesRequest::new(workspace_id)
                    .with_libraries(libraries)
                    .with_options(options);
                self.operation(self.client.prepare_selected_libraries(request))
            }
            // -- service runtime --------------------------------------------
            "start_service" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(self.client.start_service(&parse_service_id(&p.service_id)?))
            }
            "stop_service" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(self.client.stop_service(&parse_service_id(&p.service_id)?))
            }
            "restart_service" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .restart_service(&parse_service_id(&p.service_id)?),
                )
            }
            "start_services" => {
                let p = parse::<params::WorkspaceServices>(arguments)?;
                self.operation(self.client.start_services(
                    &parse_workspace_id(&p.workspace_id)?,
                    &service_ids(&p.service_ids)?,
                ))
            }
            "stop_services" => {
                let p = parse::<params::WorkspaceServices>(arguments)?;
                self.operation(self.client.stop_services(
                    &parse_workspace_id(&p.workspace_id)?,
                    &service_ids(&p.service_ids)?,
                ))
            }
            "restart_services" => {
                let p = parse::<params::WorkspaceServices>(arguments)?;
                self.operation(self.client.restart_services(
                    &parse_workspace_id(&p.workspace_id)?,
                    &service_ids(&p.service_ids)?,
                ))
            }
            // -- migrations -------------------------------------------------
            "migration_status" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(
                    self.client
                        .migration_status(&parse_service_id(&p.service_id)?),
                )
            }
            "validate_migration" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .validate_migration(&parse_service_id(&p.service_id)?),
                )
            }
            "get_migration_status" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .get_migration_status(&parse_service_id(&p.service_id)?),
                )
            }
            "inspect_migration_pending" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .inspect_migration_pending(&parse_service_id(&p.service_id)?),
                )
            }
            "preview_migration" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .preview_migration(&parse_service_id(&p.service_id)?),
                )
            }
            "apply_migration" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .apply_migration(&parse_service_id(&p.service_id)?),
                )
            }
            "apply_selected_migrations" => {
                let p = parse::<params::WorkspaceServices>(arguments)?;
                self.operation(self.client.apply_selected_migrations(
                    &parse_workspace_id(&p.workspace_id)?,
                    &service_ids(&p.service_ids)?,
                ))
            }
            // -- health and diagnostics -------------------------------------
            "check_service_health" => {
                let p = parse::<params::HealthCheckRequest>(arguments)?;
                self.operation(
                    self.client
                        .check_service_health(&parse_service_id(&p.service_id)?, p.scope.into()),
                )
            }
            "check_service_readiness" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                self.operation(
                    self.client
                        .check_service_readiness(&parse_service_id(&p.service_id)?),
                )
            }
            "get_service_health" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(
                    self.client
                        .get_service_health(&parse_service_id(&p.service_id)?),
                )
            }
            "list_service_health" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                query(
                    self.client
                        .list_service_health(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "get_service_diagnostics" => {
                let p = parse::<params::ServiceRef>(arguments)?;
                query(
                    self.client
                        .get_service_diagnostics(&parse_service_id(&p.service_id)?),
                )
            }
            // -- logs -------------------------------------------------------
            "get_service_logs" => {
                let p = parse::<params::ServiceLogsRequest>(arguments)?;
                let service_id = parse_service_id(&p.service_id)?;
                let request =
                    log_request(p.workspace_id, p.level, p.source, p.text, p.after, p.limit)?;
                let page = self
                    .client
                    .get_service_logs(&service_id, &request)
                    .map_err(ToolError::from)?;
                Ok(response::log_page(&page))
            }
            "get_task_logs" => {
                let p = parse::<params::TaskLogsRequest>(arguments)?;
                let task_id = parse_task_id(&p.task_id)?;
                let request =
                    log_request(p.workspace_id, p.level, p.source, p.text, p.after, p.limit)?;
                let page = self
                    .client
                    .get_task_logs(&task_id, &request)
                    .map_err(ToolError::from)?;
                Ok(response::log_page(&page))
            }
            "get_logs" => {
                let p = parse::<params::LogsRequest>(arguments)?;
                let mut request =
                    log_request(p.workspace_id, p.level, p.source, p.text, p.after, p.limit)?;
                if let Some(task_id) = p.task_id {
                    request = request.for_task(parse_task_id(&task_id)?);
                }
                let entries = self.client.get_logs(&request).map_err(ToolError::from)?;
                Ok(response::log_entries(&entries))
            }
            // -- tasks ------------------------------------------------------
            "get_task" => {
                let p = parse::<params::TaskRef>(arguments)?;
                let task = self
                    .client
                    .get_task(&parse_task_id(&p.task_id)?)
                    .map_err(ToolError::from)?;
                Ok(response::task(&task))
            }
            "list_tasks" => {
                let p = parse::<params::ListTasksRequest>(arguments)?;
                let tasks = match p.limit {
                    Some(limit) => self.monitor.recent_tasks(limit),
                    None => self.monitor.tasks(),
                };
                Ok(Value::Array(tasks.iter().map(response::task).collect()))
            }
            "cancel_task" => {
                let p = parse::<params::TaskRef>(arguments)?;
                let task_id = parse_task_id(&p.task_id)?;
                self.client.cancel_task(&task_id).map_err(ToolError::from)?;
                Ok(serde_json::json!({
                    "taskId": task_id.to_string(),
                    "cancellation": "requested",
                }))
            }
            "summarize_workflow" => {
                let p = parse::<params::WorkflowRef>(arguments)?;
                query(
                    self.client
                        .summarize_workflow(&parse_workflow_id(&p.workflow_id)?),
                )
            }
            // -- workflows --------------------------------------------------
            "sync_workspace" => {
                let p = parse::<params::WorkspaceRef>(arguments)?;
                self.operation(
                    self.client
                        .sync_workspace(&parse_workspace_id(&p.workspace_id)?),
                )
            }
            "prepare_environment" => {
                let p = parse::<params::PrepareEnvironmentRequest>(arguments)?;
                let mut request =
                    PrepareEnvironmentRequest::new(parse_workspace_id(&p.workspace_id)?)
                        .with_services(service_ids(&p.service_ids)?);
                if let Some(branch) = p.branch {
                    request = request.with_branch(parse_branch(&branch)?);
                }
                if p.run_liquibase {
                    request = request.run_liquibase();
                }
                if p.build {
                    request = request.build();
                }
                if p.start {
                    request = request.start();
                }
                self.operation(self.client.prepare_environment(request))
            }
            "start_group" => {
                let p = parse::<params::GroupAction>(arguments)?;
                self.operation(self.client.start_group(
                    &parse_workspace_id(&p.workspace_id)?,
                    &parse_group_id(&p.group_id)?,
                ))
            }
            "stop_group" => {
                let p = parse::<params::GroupAction>(arguments)?;
                self.operation(self.client.stop_group(
                    &parse_workspace_id(&p.workspace_id)?,
                    &parse_group_id(&p.group_id)?,
                ))
            }
            "start_profile" => {
                let p = parse::<params::ProfileAction>(arguments)?;
                self.operation(self.client.start_profile(
                    &parse_workspace_id(&p.workspace_id)?,
                    &parse_profile_id(&p.profile_id)?,
                ))
            }
            "stop_profile" => {
                let p = parse::<params::ProfileAction>(arguments)?;
                self.operation(self.client.stop_profile(
                    &parse_workspace_id(&p.workspace_id)?,
                    &parse_profile_id(&p.profile_id)?,
                ))
            }
            _ => Err(ToolError::Protocol(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool '{name}'"),
                None,
            ))),
        }
    }

    /// Converts an [`OperationResult`] into a success payload, attaching the
    /// live status of a just-scheduled task when one exists.
    fn operation(
        &self,
        result: Result<service_orchestrator_application::OperationResult, ApplicationError>,
    ) -> Result<Value, ToolError> {
        let result = result.map_err(ToolError::from)?;
        let status = result
            .task_id()
            .and_then(|task_id| self.client.get_task(task_id).ok())
            .map(|task| task.status.label().to_owned());
        Ok(response::operation_result(&result, status.as_deref()))
    }
}

impl rmcp::ServerHandler for OrchestratorServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new(
            "service-orchestrator",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(INSTRUCTIONS)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools
            .iter()
            .find(|tool| tool.name.as_ref() == name)
            .cloned()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let started = Instant::now();
        let name = request.name.as_ref().to_owned();
        let outcome = self.dispatch(&name, request.arguments).await;

        match outcome {
            Ok(value) => {
                logging::tool_invocation(
                    &name,
                    logging::InvocationOutcome::Success,
                    started.elapsed(),
                    None,
                );
                Ok(CallToolResponse::Complete(CallToolResult::structured(
                    response::success(value),
                )))
            }
            Err(ToolError::Application(error)) => {
                let body = response::error_body(&error);
                logging::tool_invocation(
                    &name,
                    logging::InvocationOutcome::ApplicationError,
                    started.elapsed(),
                    Some(&body.code),
                );
                Ok(CallToolResponse::Complete(
                    CallToolResult::structured_error(response::failure(body)),
                ))
            }
            Err(ToolError::InvalidArgument { message }) => {
                logging::tool_invocation(
                    &name,
                    logging::InvocationOutcome::ApplicationError,
                    started.elapsed(),
                    Some("INVALID_ARGUMENT"),
                );
                Ok(CallToolResponse::Complete(
                    CallToolResult::structured_error(response::failure(response::ErrorBody::new(
                        "INVALID_ARGUMENT",
                        message,
                    ))),
                ))
            }
            Err(ToolError::Protocol(error)) => {
                let code = error.code.0.to_string();
                logging::tool_invocation(
                    &name,
                    logging::InvocationOutcome::ProtocolError,
                    started.elapsed(),
                    Some(&code),
                );
                Err(error)
            }
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(resources::list(
            self.client.as_ref(),
        )))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(
            resources::templates(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let contents = resources::read(self.client.as_ref(), &request.uri).await?;
        Ok(ReadResourceResult::new(vec![contents]).into())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(prompts::list()))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let result: GetPromptResult = prompts::get(&request.name, request.arguments)?;
        Ok(result.into())
    }

    async fn on_initialized(&self, _context: NotificationContext<RoleServer>) {
        logging::server_started();
    }
}

/// Deserializes tool arguments into a typed request.
///
/// A shape mismatch (missing required property, unknown property, wrong type)
/// is a protocol `invalid_params` error: the request never routed.
fn parse<T: serde::de::DeserializeOwned>(
    arguments: Option<serde_json::Map<String, Value>>,
) -> Result<T, ToolError> {
    let value = Value::Object(arguments.unwrap_or_default());
    serde_json::from_value(value).map_err(|error| {
        ToolError::Protocol(ErrorData::invalid_params(
            format!("invalid arguments: {error}"),
            None,
        ))
    })
}

/// Serializes a value into the envelope's `data` payload.
fn to_value<T: Serialize>(value: T) -> Result<Value, ToolError> {
    serde_json::to_value(value).map_err(|error| {
        ToolError::Protocol(ErrorData::internal_error(
            format!("failed to serialize response: {error}"),
            None,
        ))
    })
}

/// Serializes the result of an application query.
fn query<T: Serialize>(result: Result<T, ApplicationError>) -> Result<Value, ToolError> {
    to_value(result.map_err(ToolError::from)?)
}

/// Parses a list of service identifiers.
fn service_ids(raw: &[String]) -> Result<Vec<service_orchestrator_domain::ServiceId>, ToolError> {
    raw.iter().map(|id| parse_service_id(id)).collect()
}

/// Builds a bounded [`LogRequest`] from the shared filter fields.
///
/// Every field is optional; the application (and the log store behind it)
/// enforces the hard upper bound on `limit`.
fn log_request(
    workspace_id: Option<String>,
    level: Option<params::Level>,
    source: Option<params::Source>,
    text: Option<String>,
    after: Option<u64>,
    limit: Option<usize>,
) -> Result<LogRequest, ToolError> {
    let mut request = LogRequest::default();
    if let Some(workspace_id) = workspace_id {
        request = request.for_workspace(parse_workspace_id(&workspace_id)?);
    }
    if let Some(level) = level {
        request = request.with_minimum_level(level.into());
    }
    if let Some(source) = source {
        request = request.with_source(source.into());
    }
    if let Some(text) = text {
        request = request.matching(text);
    }
    if let Some(after) = after {
        request = request.after(after);
    }
    if let Some(limit) = limit {
        request = request.with_limit(limit);
    }
    Ok(request)
}
