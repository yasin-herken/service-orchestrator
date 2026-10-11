//! MCP prompts: standardized developer workflows for an agent to start from.
//!
//! A prompt returns a single user message that names the tools to call and the
//! order to call them in. It contains **no business logic**: it is guidance for
//! an agent to drive the same application use cases a human would drive through
//! the TUI (`AGENTS.md` §26, `tasks/014.md` §29).
//!
//! The set is deliberately small and only references tools that exist.

use rmcp::model::{
    ErrorData, GetPromptResult, JsonObject, Prompt, PromptArgument, PromptMessage, Role,
};
use serde_json::Value;

/// The prompts the server advertises.
pub(crate) fn list() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "prepare-development-environment",
            Some("Bring a workspace to a ready state: synchronize, migrate, build, and start."),
            Some(vec![
                PromptArgument::new("workspaceId")
                    .with_description("The workspace to prepare.")
                    .with_required(true),
                PromptArgument::new("serviceIds")
                    .with_description("Comma-separated services to include; omit for all."),
            ]),
        ),
        Prompt::new(
            "sync-workspace",
            Some("Synchronize every repository in a workspace."),
            Some(vec![PromptArgument::new("workspaceId")
                .with_description("The workspace to synchronize.")
                .with_required(true)]),
        ),
        Prompt::new(
            "debug-service",
            Some("Gather the state needed to debug a service: status, health, diagnostics, and logs."),
            Some(vec![PromptArgument::new("serviceId")
                .with_description("The service to debug.")
                .with_required(true)]),
        ),
        Prompt::new(
            "inspect-unhealthy-service",
            Some("Investigate a service whose health check is failing."),
            Some(vec![PromptArgument::new("serviceId")
                .with_description("The service to inspect.")
                .with_required(true)]),
        ),
        Prompt::new(
            "inspect-failed-build",
            Some("Investigate a service whose build failed."),
            Some(vec![PromptArgument::new("serviceId")
                .with_description("The service whose build failed.")
                .with_required(true)]),
        ),
    ]
}

/// Builds the message for one prompt.
pub(crate) fn get(name: &str, arguments: Option<JsonObject>) -> Result<GetPromptResult, ErrorData> {
    let arguments = arguments.unwrap_or_default();
    let text = match name {
        "prepare-development-environment" => {
            let workspace = required(&arguments, "workspaceId")?;
            let selected = match arguments.get("serviceIds").and_then(Value::as_str) {
                Some(list) if !list.trim().is_empty() => {
                    let ids: Vec<String> = list
                        .split(',')
                        .map(|id| format!("\"{}\"", id.trim()))
                        .collect();
                    format!(" Use serviceIds [{}].", ids.join(", "))
                }
                _ => String::new(),
            };
            format!(
                "Prepare the '{workspace}' workspace for development.{selected} \
Call `get_workspace_status` first to see the current state, then call \
`prepare_environment` with workspaceId \"{workspace}\" and the desired flags \
(runLiquibase, build, start). It returns a workflowId; poll `get_task` on the \
workflow's tasks or read `get_task_logs` until it finishes. If a task fails, \
call `get_task` to read its structured error before retrying."
            )
        }
        "sync-workspace" => {
            let workspace = required(&arguments, "workspaceId")?;
            format!(
                "Synchronize the '{workspace}' workspace. Call `git_sync_repositories` \
with workspaceId \"{workspace}\" and the serviceIds you want, or `sync_workspace` \
to synchronize them all. Both return a workflowId; read `summarize_workflow` to \
see the aggregate outcome once the tasks finish."
            )
        }
        "debug-service" => {
            let service = required(&arguments, "serviceId")?;
            format!(
                "Collect the state needed to debug service '{service}'. Call \
`get_service_status`, `get_service_runtime_status`, `get_service_health`, and \
`get_service_diagnostics` for the service, then call `check_service_health` or \
`check_service_readiness` to refresh the live health, and `get_service_logs` to \
read a bounded page of its output. Report what is wrong and which signal \
(process, port, HTTP) failed."
            )
        }
        "inspect-unhealthy-service" => {
            let service = required(&arguments, "serviceId")?;
            format!(
                "Service '{service}' is unhealthy. Call `check_service_health` with \
scope \"all\" to refresh every configured check, then `get_service_diagnostics` \
and `get_service_logs` (level \"error\"). Compare the process state from \
`get_service_runtime_status` with the health result and explain which check \
failed and why."
            )
        }
        "inspect-failed-build" => {
            let service = required(&arguments, "serviceId")?;
            format!(
                "The build of service '{service}' failed. Call `get_service_status` \
to confirm, then `get_service_logs` filtered to source \"build\" and level \
\"error\" to read the failure. If the repository may be stale, call \
`git_status` and `git_sync` before rebuilding with `build_service`."
            )
        }
        other => {
            return Err(ErrorData::invalid_params(
                format!("unknown prompt '{other}'"),
                None,
            ));
        }
    };

    Ok(GetPromptResult::new(vec![PromptMessage::new_text(
        Role::User,
        text,
    )]))
}

/// Returns a required string argument or an `invalid_params` error.
fn required(arguments: &JsonObject, name: &str) -> Result<String, ErrorData> {
    match arguments.get(name).and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => Ok(value.to_owned()),
        _ => Err(ErrorData::invalid_params(
            format!("missing required argument '{name}'"),
            None,
        )),
    }
}
