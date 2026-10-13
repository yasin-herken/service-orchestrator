//! The request router.
//!
//! Every wire method maps to exactly one application use case. The router holds
//! no business logic: it decodes the request's parameters, calls the shared
//! [`Application`], and encodes the structured result. Authorization is applied
//! here — the daemon rebuilds an [`Application`] carrying the caller's
//! authorization so the shared policy engine always runs (`tasks/015.md` §9,
//! §23).

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use service_orchestrator_application::{
    Application, ApplicationError, Authorization, HealthCheckScope, LibraryPrepareOptions,
    LogRequest, PrepareEnvironmentRequest, PrepareSelectedLibrariesRequest,
};
use service_orchestrator_client::ops;
use service_orchestrator_client::{codes, unavailable, DiffParams, RequestEnvelope};
use service_orchestrator_config::ConfigChange;
use service_orchestrator_domain::{
    Branch, GroupId, LibraryId, ProfileId, ServiceId, TaskId, WorkspaceId,
};
use service_orchestrator_execution::{clamp_task_limit, TaskEngine, MAX_TASK_LIST_LIMIT};

/// Returns whether a request may act as an interactive destructive confirmation.
///
/// Trust is threefold: the request must ask for it, must declare an
/// interactive client kind, *and* the connection's kernel-verified peer UID
/// must match the daemon's. The wire-declared `ClientKind` alone is never
/// enough (`tasks/017.md` §27, finding H-IPC-1).
pub(crate) const fn destructive_allowed(request: &RequestEnvelope, same_user: bool) -> bool {
    request.allow_destructive && request.client.is_interactive() && same_user
}

/// Dispatches one application operation to the shared core.
///
/// `same_user` is the kernel-verified identity of the connecting peer: a
/// destructive authorization request is honored only when the request asks for
/// it, the peer declares itself an interactive client, *and* the peer runs as
/// the same user as the daemon (`tasks/017.md` §27, finding H-IPC-1).
///
/// # Errors
///
/// Returns a structured [`ApplicationError`] for unknown methods, invalid
/// parameters, or a failed use case.
pub(crate) async fn route(
    application: Application,
    engine: &Arc<TaskEngine>,
    request: &RequestEnvelope,
    same_user: bool,
) -> Result<Value, ApplicationError> {
    let allow_destructive = destructive_allowed(request, same_user);
    let authorization = if allow_destructive {
        Authorization::allow_destructive()
    } else {
        Authorization::standard()
    };
    // The caller already holds an owned snapshot, so authorization is applied in
    // place instead of cloning the application a second time.
    let app = application.with_authorization(authorization);
    let params = request.params.clone();

    match request.method.as_str() {
        // -- diagnostics and task listing ----------------------------------
        ops::CONFIG_SUMMARY => {
            let config = app.config();
            encode(service_orchestrator_client::ConfigSummary {
                version: config.version,
                max_parallel_tasks: config.execution.max_parallel_tasks,
                default_timeout_seconds: config.execution.default_timeout_seconds,
            })
        }
        // A task listing is bounded: an unbounded request would serialize the
        // entire task history into one IPC frame. `list_tasks` keeps the newest
        // tasks (oldest first within the retained window) so its ordering is
        // unchanged for a bounded history (`AGENTS.md` §35).
        ops::LIST_TASKS => {
            let mut tasks = engine.list_tasks();
            if tasks.len() > MAX_TASK_LIST_LIMIT {
                let excess = tasks.len() - MAX_TASK_LIST_LIMIT;
                tasks.drain(..excess);
            }
            encode(tasks)
        }
        ops::LIST_RUNNING_TASKS => encode(engine.list_running_tasks()),
        ops::LIST_RECENT_TASKS => {
            let (limit,): (usize,) = decode(params)?;
            encode(engine.list_recent_tasks(clamp_task_limit(limit)))
        }

        // -- configuration management (read-only) ---------------------------
        // Commit and reload are handled by the server, which owns the write
        // lock and the change-event broadcast (`tasks/023.md` §38).
        ops::CONFIGURATION_SNAPSHOT => encode(app.configuration_snapshot()?),
        ops::VALIDATE_CONFIGURATION => encode(app.validate_configuration()?),
        ops::PREVIEW_CONFIGURATION => {
            let (change, revision): (ConfigChange, String) = decode(params)?;
            encode(app.preview_configuration(&change, &revision)?)
        }
        ops::INSPECT_REPOSITORY => {
            let (path,): (String,) = decode(params)?;
            encode(app.inspect_repository(&path)?)
        }
        ops::DETECT_PROJECT_CONFIGURATION => {
            let (path,): (String,) = decode(params)?;
            encode(app.detect_project_configuration(&path)?)
        }

        // -- workspaces, groups, profiles ----------------------------------
        ops::LIST_WORKSPACES => encode(app.list_workspaces()),
        ops::GET_WORKSPACE => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.get_workspace(&id)?)
        }
        ops::GET_WORKSPACE_STATUS => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.get_workspace_status(&id)?)
        }
        ops::VALIDATE_WORKSPACE => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.validate_workspace(&id)?)
        }
        ops::LIST_GROUPS => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.list_groups(&id)?)
        }
        ops::GET_GROUP => {
            let (id,): (GroupId,) = decode(params)?;
            encode(app.get_group(&id)?)
        }
        ops::LIST_PROFILES => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.list_profiles(&id)?)
        }
        ops::GET_PROFILE => {
            let (id,): (ProfileId,) = decode(params)?;
            encode(app.get_profile(&id)?)
        }

        // -- services and libraries ----------------------------------------
        ops::LIST_SERVICES => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.list_services(&id)?)
        }
        ops::GET_SERVICE => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.get_service(&id)?)
        }
        ops::GET_SERVICE_STATUS => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.get_service_status(&id)?)
        }
        ops::GET_SERVICE_RUNTIME_STATUS => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.get_service_runtime_status(&id)?)
        }
        ops::LIST_RUNNING_SERVICES => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.list_running_services(&id)?)
        }
        ops::LIST_LIBRARIES => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.list_libraries(&id)?)
        }
        ops::GET_LIBRARY => {
            let (id,): (LibraryId,) = decode(params)?;
            encode(app.get_library(&id)?)
        }
        ops::GET_LIBRARY_STATUS => {
            let (id,): (LibraryId,) = decode(params)?;
            encode(app.get_library_status(&id).await?)
        }
        ops::FIND_LIBRARY_DEPENDENTS => {
            let (id,): (LibraryId,) = decode(params)?;
            encode(app.find_library_dependents(&id)?)
        }
        ops::VERIFY_LIBRARY_ARTIFACT => {
            let (id,): (LibraryId,) = decode(params)?;
            encode(app.verify_library_artifact(&id).await?)
        }
        ops::INSPECT_LIBRARY_VERSION_ALIGNMENT => {
            let (id,): (LibraryId,) = decode(params)?;
            encode(app.inspect_library_version_alignment(&id).await?)
        }

        // -- git (read-only) ------------------------------------------------
        ops::GET_GIT_STATUS => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.get_git_status(&id).await?)
        }
        ops::LIST_BRANCHES => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.list_branches(&id).await?)
        }
        ops::GET_REPOSITORY_DIFF => {
            let (id, diff): (ServiceId, DiffParams) = decode(params)?;
            encode(app.get_repository_diff(&id, &diff.into_options()).await?)
        }

        // -- migrations, health, diagnostics, logs --------------------------
        ops::MIGRATION_STATUS => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.migration_status(&id)?)
        }
        ops::GET_SERVICE_HEALTH => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.get_service_health(&id)?)
        }
        ops::LIST_SERVICE_HEALTH => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.list_service_health(&id)?)
        }
        ops::GET_SERVICE_DIAGNOSTICS => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.get_service_diagnostics(&id)?)
        }
        ops::GET_SERVICE_LOGS => {
            let (id, request): (ServiceId, LogRequest) = decode(params)?;
            encode(app.get_service_logs(&id, &request)?)
        }
        ops::GET_TASK_LOGS => {
            let (id, request): (TaskId, LogRequest) = decode(params)?;
            encode(app.get_task_logs(&id, &request)?)
        }
        ops::GET_LOGS => {
            let (request,): (LogRequest,) = decode(params)?;
            encode(app.get_logs(&request)?)
        }

        // -- tasks and workflows --------------------------------------------
        ops::GET_TASK => {
            let (id,): (TaskId,) = decode(params)?;
            encode(app.get_task(&id)?)
        }
        ops::GET_WORKFLOW => {
            let (id,): (service_orchestrator_domain::WorkflowId,) = decode(params)?;
            encode(app.get_workflow(&id)?)
        }
        ops::SUMMARIZE_WORKFLOW => {
            let (id,): (service_orchestrator_domain::WorkflowId,) = decode(params)?;
            encode(app.summarize_workflow(&id)?)
        }
        ops::CANCEL_TASK => {
            let (id,): (TaskId,) = decode(params)?;
            encode(app.cancel_task(&id)?)
        }

        // -- git commands ----------------------------------------------------
        ops::SYNC_SERVICE => unary_service(params, |id| app.sync_service(id)),
        ops::SYNC_REPOSITORY => unary_service(params, |id| app.sync_repository(id)),
        ops::FETCH_REPOSITORY => unary_service(params, |id| app.fetch_repository(id)),
        ops::PULL_REPOSITORY => unary_service(params, |id| app.pull_repository(id)),
        ops::CLONE_REPOSITORY => unary_service(params, |id| app.clone_repository(id)),
        ops::RESET_REPOSITORY_HARD => unary_service(params, |id| app.reset_repository_hard(id)),
        ops::CLEAN_REPOSITORY => unary_service(params, |id| app.clean_repository(id)),
        ops::DISCARD_LOCAL_CHANGES => unary_service(params, |id| app.discard_local_changes(id)),
        ops::CHECKOUT_REPOSITORY => {
            let (id, branch): (ServiceId, Branch) = decode(params)?;
            encode(app.checkout_repository(&id, &branch)?)
        }
        ops::SYNC_REPOSITORIES => {
            let (workspace, services): (WorkspaceId, Vec<ServiceId>) = decode(params)?;
            encode(app.sync_repositories(&workspace, &services)?)
        }
        ops::FETCH_REPOSITORIES => {
            let (workspace, services): (WorkspaceId, Vec<ServiceId>) = decode(params)?;
            encode(app.fetch_repositories(&workspace, &services)?)
        }
        ops::CHECKOUT_REPOSITORIES => {
            let (workspace, services, branch): (WorkspaceId, Vec<ServiceId>, Branch) =
                decode(params)?;
            encode(app.checkout_repositories(&workspace, &services, &branch)?)
        }

        // -- builds and libraries -------------------------------------------
        ops::GET_BUILD_CAPABILITIES => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.get_build_capabilities(&id)?)
        }
        ops::BUILD_SERVICE => unary_service(params, |id| app.build_service(id)),
        ops::BUILD_WORKSPACE => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.build_workspace(&id)?)
        }
        ops::BUILD_LIBRARY => unary_library(params, |id| app.build_library(id)),
        ops::INSTALL_LIBRARY => unary_library(params, |id| app.install_library(id)),
        ops::SYNC_LIBRARY => unary_library(params, |id| app.sync_library(id)),
        ops::PREPARE_LIBRARY => {
            let (id, options): (LibraryId, LibraryPrepareOptions) = decode(params)?;
            encode(app.prepare_library(&id, &options)?)
        }
        ops::PREPARE_SELECTED_LIBRARIES => {
            let (request,): (PrepareSelectedLibrariesRequest,) = decode(params)?;
            encode(app.prepare_selected_libraries(request)?)
        }

        // -- service runtime -------------------------------------------------
        ops::START_SERVICE => unary_service(params, |id| app.start_service(id)),
        ops::STOP_SERVICE => unary_service(params, |id| app.stop_service(id)),
        ops::RESTART_SERVICE => unary_service(params, |id| app.restart_service(id)),
        ops::START_SERVICES => {
            let (workspace, services): (WorkspaceId, Vec<ServiceId>) = decode(params)?;
            encode(app.start_services(&workspace, &services)?)
        }
        ops::STOP_SERVICES => {
            let (workspace, services): (WorkspaceId, Vec<ServiceId>) = decode(params)?;
            encode(app.stop_services(&workspace, &services)?)
        }
        ops::RESTART_SERVICES => {
            let (workspace, services): (WorkspaceId, Vec<ServiceId>) = decode(params)?;
            encode(app.restart_services(&workspace, &services)?)
        }

        // -- migrations ------------------------------------------------------
        ops::VALIDATE_MIGRATION => unary_service(params, |id| app.validate_migration(id)),
        ops::GET_MIGRATION_STATUS => unary_service(params, |id| app.get_migration_status(id)),
        ops::INSPECT_MIGRATION_PENDING => {
            unary_service(params, |id| app.inspect_migration_pending(id))
        }
        ops::PREVIEW_MIGRATION => unary_service(params, |id| app.preview_migration(id)),
        ops::APPLY_MIGRATION => unary_service(params, |id| app.apply_migration(id)),
        ops::RUN_LIQUIBASE => unary_service(params, |id| app.run_liquibase(id)),
        ops::APPLY_SELECTED_MIGRATIONS => {
            let (workspace, services): (WorkspaceId, Vec<ServiceId>) = decode(params)?;
            encode(app.apply_selected_migrations(&workspace, &services)?)
        }

        // -- health ----------------------------------------------------------
        ops::CHECK_SERVICE_HEALTH => {
            let (id, scope): (ServiceId, HealthCheckScope) = decode(params)?;
            encode(app.check_service_health(&id, scope)?)
        }
        ops::CHECK_SERVICE_READINESS => {
            let (id,): (ServiceId,) = decode(params)?;
            encode(app.check_service_readiness(&id)?)
        }

        // -- workflows -------------------------------------------------------
        ops::SYNC_WORKSPACE => {
            let (id,): (WorkspaceId,) = decode(params)?;
            encode(app.sync_workspace(&id)?)
        }
        ops::PREPARE_ENVIRONMENT => {
            let (request,): (PrepareEnvironmentRequest,) = decode(params)?;
            encode(app.prepare_environment(request)?)
        }
        ops::PLAN_PREPARE_ENVIRONMENT => {
            let (request,): (PrepareEnvironmentRequest,) = decode(params)?;
            encode(app.plan_prepare_environment(&request)?)
        }
        ops::GET_WORKFLOW_STATUS => {
            let (id,): (service_orchestrator_domain::WorkflowId,) = decode(params)?;
            encode(app.get_workflow_status(&id)?)
        }
        ops::CANCEL_WORKFLOW => {
            let (id,): (service_orchestrator_domain::WorkflowId,) = decode(params)?;
            encode(app.cancel_workflow(&id)?)
        }
        ops::START_GROUP => {
            let (workspace, group): (WorkspaceId, GroupId) = decode(params)?;
            encode(app.start_group(&workspace, &group)?)
        }
        ops::STOP_GROUP => {
            let (workspace, group): (WorkspaceId, GroupId) = decode(params)?;
            encode(app.stop_group(&workspace, &group)?)
        }
        ops::START_PROFILE => {
            let (workspace, profile): (WorkspaceId, ProfileId) = decode(params)?;
            encode(app.start_profile(&workspace, &profile)?)
        }
        ops::STOP_PROFILE => {
            let (workspace, profile): (WorkspaceId, ProfileId) = decode(params)?;
            encode(app.stop_profile(&workspace, &profile)?)
        }

        other => Err(unavailable(
            codes::UNKNOWN_OPERATION,
            format!("the daemon does not recognize the operation '{other}'"),
        )),
    }
}

/// Routes a method whose only argument is a [`ServiceId`] and whose result is an
/// [`OperationResult`].
fn unary_service(
    params: Value,
    operation: impl FnOnce(
        &ServiceId,
    ) -> Result<
        service_orchestrator_application::OperationResult,
        ApplicationError,
    >,
) -> Result<Value, ApplicationError> {
    let (id,): (ServiceId,) = decode(params)?;
    encode(operation(&id)?)
}

/// Routes a method whose only argument is a [`LibraryId`] and whose result is an
/// [`OperationResult`].
fn unary_library(
    params: Value,
    operation: impl FnOnce(
        &LibraryId,
    ) -> Result<
        service_orchestrator_application::OperationResult,
        ApplicationError,
    >,
) -> Result<Value, ApplicationError> {
    let (id,): (LibraryId,) = decode(params)?;
    encode(operation(&id)?)
}

/// Decodes a request's parameter tuple.
fn decode<T: DeserializeOwned>(params: Value) -> Result<T, ApplicationError> {
    serde_json::from_value(params).map_err(|error| {
        unavailable(
            codes::INVALID_REQUEST,
            format!("the operation's parameters were invalid: {error}"),
        )
    })
}

/// Encodes an operation's result.
fn encode<T: Serialize>(value: T) -> Result<Value, ApplicationError> {
    serde_json::to_value(value).map_err(|error| {
        unavailable(
            codes::INVALID_REQUEST,
            format!("the operation's result could not be serialized: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_client::ClientKind;

    fn request(client: ClientKind, allow_destructive: bool) -> RequestEnvelope {
        RequestEnvelope {
            protocol: 1,
            request_id: "req-1".to_owned(),
            client,
            allow_destructive,
            method: "git_reset_hard".to_owned(),
            params: serde_json::json!([]),
        }
    }

    #[test]
    fn destructive_authorization_requires_all_three_trust_signals() {
        assert!(
            destructive_allowed(&request(ClientKind::Tui, true), true),
            "an interactive peer that asks for it is trusted"
        );
        assert!(
            destructive_allowed(&request(ClientKind::Desktop, true), true),
            "the desktop GUI is an interactive peer with a human at the keyboard"
        );
        assert!(
            !destructive_allowed(&request(ClientKind::Desktop, true), false),
            "a desktop peer from another user is never trusted"
        );
        assert!(
            !destructive_allowed(&request(ClientKind::Tui, true), false),
            "a peer whose UID does not match the daemon is never trusted"
        );
        assert!(
            !destructive_allowed(&request(ClientKind::Mcp, true), true),
            "a non-interactive client cannot self-confirm, even from the same user"
        );
        assert!(
            !destructive_allowed(&request(ClientKind::Tui, false), true),
            "an unconfirmed request stays non-destructive"
        );
    }
}
