//! Profile, group, and environment-workflow use cases behind the desktop
//! commands (`tasks/022.md` §35, §36).
//!
//! These are thin adapters over the shared [`OrchestratorClient`]: the request
//! is validated and mapped here, planning and execution run in the daemon's
//! Application Core, and the aggregated status is read back through the same
//! IPC operations the TUI and MCP use. The adapter never plans dependencies,
//! never schedules steps, and never runs Git, Maven, npm, or Liquibase itself.
//!
//! # Planning versus execution
//!
//! [`plan`] is read-only: it asks the core for the plan of exactly the request
//! [`execute`] would submit. [`execute`] forwards the same request and lets the
//! daemon re-validate every precondition and enforce the shared policy — a
//! plan is never treated as authorization, and no frontend confirmation flag
//! exists on this path at all (`tasks/022.md` §12, §16, §32).

use std::collections::BTreeSet;

use serde::Deserialize;
use service_orchestrator_application::{
    ApplicationError, OperationResult, PrepareEnvironmentRequest,
};
use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::{BuildOperation, Task, Workflow, WorkflowId, WorkspaceId};

use crate::dto::{
    EnvironmentPlanView, GroupView, OperationOutcome, ProfileView, WorkflowStatusView,
    WorkflowSummaryView,
};
use crate::error::CommandError;
use crate::validation::{
    parse_branch, parse_group_id, parse_migration_preset_id, parse_profile_id, parse_service_id,
    parse_workflow_id, parse_workspace_id,
};

/// How many workflows one history request may return by default.
pub const DEFAULT_HISTORY_LIMIT: usize = 20;

/// The hard cap on one history request.
///
/// Bounded on purpose: history is derived from the task engine's retained
/// tasks, and the screen asks for a window rather than an unbounded log
/// (`tasks/022.md` §26).
pub const MAX_HISTORY_LIMIT: usize = 100;

/// How many retained tasks are scanned to discover workflow ids.
///
/// One bounded pass over active and recent tasks: the adapter never asks the
/// daemon for unlimited task history just to build a short workflow list.
const HISTORY_TASK_SCAN: usize = 1_000;

/// The validated, wire-level request the GUI sends for planning and execution.
///
/// The struct is the single mapping point between frontend input and the
/// core's [`PrepareEnvironmentRequest`]: both planning and execution consume
/// the same built request, so a preview can never drift from what execution
/// schedules (`tasks/022.md` §12). Fields omitted by the frontend fall back to
/// [`Default`], whose values are the safe ones — synchronization and dependency
/// inclusion on, migrations, builds, and startup off
/// (`tasks/022.md` §11).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct PrepareEnvironmentInput {
    /// The workspace to prepare.
    pub workspace_id: String,
    /// Explicitly selected service ids. Mutually exclusive with `profile` and
    /// `group`: exactly one selection source must be present.
    pub services: Vec<String>,
    /// An optional profile whose services should be selected.
    pub profile: Option<String>,
    /// An optional group whose members should be selected.
    pub group: Option<String>,
    /// An optional branch to check out after synchronizing. Never defaulted:
    /// no workflow checks out a branch the user did not ask for.
    pub branch: Option<String>,
    /// Whether to synchronize repositories. Defaults to `true`.
    pub sync: bool,
    /// Whether to include the transitive required dependency closure.
    /// Defaults to `true`.
    pub include_dependencies: bool,
    /// Whether to build the required libraries. Defaults to `false`.
    pub build_libraries: bool,
    /// Whether to run Liquibase migrations. Defaults to `false`: enabling it
    /// is an explicit, visible database mutation.
    pub run_liquibase: bool,
    /// An optional named migration preset applied to every migration-enabled
    /// selected service (`tasks/032.md` §21, §26). It never enables migration on
    /// its own; `run_liquibase` must be set as well.
    pub migration_preset: Option<String>,
    /// An optional build operation (`build`, `install`, `test`, …) overriding
    /// each entity's configured default (`tasks/032.md` §27).
    pub build_operation: Option<String>,
    /// Whether to build the selected services. Defaults to `false`.
    pub build: bool,
    /// Whether to start the selected services that are not already running.
    /// Defaults to `false`: the GUI never auto-starts the workspace. Mutually
    /// exclusive with `restart` and `stop`.
    pub start: bool,
    /// Whether to restart the selected services (stop in reverse dependency
    /// order, then build/start as requested). Mutually exclusive with `start`
    /// and `stop` (`tasks/032.md` §28).
    pub restart: bool,
    /// Whether to stop the selected services. Mutually exclusive with `start`
    /// and `restart`.
    pub stop: bool,
    /// Whether to wait for health after starting. Defaults to `true`.
    pub wait_for_health: bool,
}

impl Default for PrepareEnvironmentInput {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            services: Vec::new(),
            profile: None,
            group: None,
            branch: None,
            sync: true,
            include_dependencies: true,
            build_libraries: false,
            run_liquibase: false,
            migration_preset: None,
            build_operation: None,
            build: false,
            start: false,
            restart: false,
            stop: false,
            wait_for_health: true,
        }
    }
}

/// Normalizes an optional identifier: blanks and whitespace-only values are
/// treated as "not selected" so a UI placeholder can never become a lookup.
fn optional_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

/// Maps validated GUI input onto the core's prepare-environment request.
///
/// This is a pure function: it performs no IPC and runs no workflow, so the
/// request mapping can be unit-tested directly (`tasks/022.md` §41).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace, service, profile, group,
/// or branch identifier, for an empty selection, or for more than one selection
/// source — the GUI must never implicitly select the whole workspace
/// (`tasks/022.md` §7).
pub fn build_request(
    input: &PrepareEnvironmentInput,
) -> Result<PrepareEnvironmentRequest, CommandError> {
    let workspace_id = parse_workspace_id(&input.workspace_id)?;

    let services = input
        .services
        .iter()
        .map(|raw| raw.trim())
        .filter(|raw| !raw.is_empty())
        .map(parse_service_id)
        .collect::<Result<Vec<_>, _>>()?;
    let profile = optional_id(input.profile.as_deref())
        .map(parse_profile_id)
        .transpose()?;
    let group = optional_id(input.group.as_deref())
        .map(parse_group_id)
        .transpose()?;

    let sources =
        u8::from(!services.is_empty()) + u8::from(profile.is_some()) + u8::from(group.is_some());
    if sources == 0 {
        return Err(CommandError::invalid_input(
            "select at least one service, or a profile or group, before preparing an environment",
        ));
    }
    if sources > 1 {
        return Err(CommandError::invalid_input(
            "select exactly one source: explicit services, a profile, or a group",
        ));
    }

    let branch = optional_id(input.branch.as_deref())
        .map(parse_branch)
        .transpose()?;

    let actions = u8::from(input.start) + u8::from(input.restart) + u8::from(input.stop);
    if actions > 1 {
        return Err(CommandError::invalid_input(
            "choose at most one runtime action: start, restart, or stop",
        ));
    }

    let migration_preset = optional_id(input.migration_preset.as_deref())
        .map(parse_migration_preset_id)
        .transpose()?;
    let build_operation = optional_id(input.build_operation.as_deref())
        .map(|raw| {
            serde_json::from_value::<BuildOperation>(serde_json::Value::String(raw.to_owned()))
                .map_err(|_| {
                    CommandError::invalid_input(format!(
                        "unknown build operation '{raw}': expected build, install, test, or package"
                    ))
                })
        })
        .transpose()?;

    Ok(PrepareEnvironmentRequest {
        workspace_id,
        services,
        profile,
        group,
        branch,
        sync: input.sync,
        include_dependencies: input.include_dependencies,
        build_libraries: input.build_libraries,
        run_liquibase: input.run_liquibase,
        migration_preset,
        build_operation,
        build: input.build,
        start: input.start,
        restart: input.restart,
        stop: input.stop,
        wait_for_health: input.wait_for_health,
    })
}

/// Builds the read-only plan for a workflow the GUI is about to offer.
///
/// The plan comes from the daemon's own composer, so it describes exactly the
/// steps execution would schedule; nothing is mutated here.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for rejected input, `InvalidOperation` when the
/// selection resolves to nothing, or the daemon's structured error (including
/// an unknown workspace, profile, or group).
pub fn plan(
    client: &dyn OrchestratorClient,
    input: &PrepareEnvironmentInput,
) -> Result<EnvironmentPlanView, CommandError> {
    let request = build_request(input)?;
    let plan = client
        .plan_prepare_environment(&request)
        .map_err(CommandError::from)?;
    Ok(EnvironmentPlanView::from(plan))
}

/// Schedules the environment workflow through the daemon.
///
/// The daemon re-validates the request and enforces the shared policy at
/// execution time; the adapter attaches no authorization of its own.
///
/// # Errors
///
/// Same validation errors as [`build_request`], plus the daemon's structured
/// error — including `PermissionDenied` when a required authorization was not
/// granted.
pub fn execute(
    client: &dyn OrchestratorClient,
    input: &PrepareEnvironmentInput,
) -> Result<OperationOutcome, CommandError> {
    let request = build_request(input)?;
    let result: OperationResult = client
        .prepare_environment(request)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Lists a workspace's configured profiles, with their groups expanded.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error for anything else.
pub fn list_profiles(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<ProfileView>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let workspace = client
        .get_workspace(&workspace_id)
        .map_err(CommandError::from)?;
    let profiles = client
        .list_profiles(&workspace_id)
        .map_err(CommandError::from)?;
    Ok(profiles
        .iter()
        .map(|profile| ProfileView::for_workspace(&workspace, profile))
        .collect())
}

/// Returns one profile, scoped to the workspace it belongs to.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected identifier, `ProfileNotFound` for an
/// unknown profile, `NOT_FOUND` when the profile belongs to another workspace,
/// or the daemon's structured error for anything else.
pub fn get_profile(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    profile_id: &str,
) -> Result<ProfileView, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let profile_id = parse_profile_id(profile_id)?;
    let workspace = client
        .get_workspace(&workspace_id)
        .map_err(CommandError::from)?;
    let profile = client
        .get_profile(&profile_id)
        .map_err(CommandError::from)?;
    if !workspace
        .profiles
        .iter()
        .any(|entry| entry.id == profile.id)
    {
        return Err(CommandError::new(
            "NOT_FOUND",
            format!("profile '{profile_id}' does not belong to workspace '{workspace_id}'"),
        ));
    }
    Ok(ProfileView::for_workspace(&workspace, &profile))
}

/// Lists a workspace's configured groups with their member services.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error for anything else.
pub fn list_groups(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<GroupView>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let workspace = client
        .get_workspace(&workspace_id)
        .map_err(CommandError::from)?;
    let groups = client
        .list_groups(&workspace_id)
        .map_err(CommandError::from)?;
    Ok(groups
        .iter()
        .map(|group| GroupView::for_workspace(&workspace, group))
        .collect())
}

/// Returns one group, scoped to the workspace it belongs to.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected identifier, `GroupNotFound` for an
/// unknown group, `NOT_FOUND` when the group belongs to another workspace, or
/// the daemon's structured error for anything else.
pub fn get_group(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    group_id: &str,
) -> Result<GroupView, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let group_id = parse_group_id(group_id)?;
    let workspace = client
        .get_workspace(&workspace_id)
        .map_err(CommandError::from)?;
    let group = client.get_group(&group_id).map_err(CommandError::from)?;
    if !workspace.groups.iter().any(|entry| entry.id == group.id) {
        return Err(CommandError::new(
            "NOT_FOUND",
            format!("group '{group_id}' does not belong to workspace '{workspace_id}'"),
        ));
    }
    Ok(GroupView::for_workspace(&workspace, &group))
}

/// Returns the aggregated, structured status of one workflow.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workflow id, `WorkflowNotFound` for
/// an unknown workflow, or the daemon's structured error for anything else.
pub fn workflow_status(
    client: &dyn OrchestratorClient,
    workflow_id: &str,
) -> Result<WorkflowStatusView, CommandError> {
    let workflow_id = parse_workflow_id(workflow_id)?;
    let status = client
        .get_workflow_status(&workflow_id)
        .map_err(CommandError::from)?;
    Ok(WorkflowStatusView::from(status))
}

/// Lists a bounded, newest-first window of a workspace's workflow history.
///
/// The daemon keeps no persisted workflow table and no workspace-scoped workflow
/// listing: history is derived from the task engine's active and retained tasks,
/// grouped by their shared workflow id, and each candidate's workspace is read
/// from the resolved parameters of its steps. A workflow task carries a workflow
/// target rather than a service or library one, so the step parameters are the
/// only workspace signal a composed workflow exposes. Workflows that
/// disappeared from retention are skipped rather than failing the list, and
/// nothing is invented across daemon restarts (`tasks/022.md` §26, §34).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error when a listing or status query fails.
pub fn list_workflows(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    limit: Option<usize>,
) -> Result<Vec<WorkflowSummaryView>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let limit = history_limit(limit);

    let mut tasks = client.list_running_tasks();
    tasks.extend(client.list_recent_tasks(HISTORY_TASK_SCAN));

    let mut rows = Vec::new();
    for workflow_id in workflow_ids(&tasks) {
        if rows.len() >= limit {
            break;
        }
        // Retention may have moved on between the task listing and this lookup,
        // in which case the workflow is simply no longer observable.
        let workflow = match client.get_workflow(&workflow_id) {
            Ok(workflow) => workflow,
            Err(ApplicationError::WorkflowNotFound(_)) => continue,
            Err(error) => return Err(CommandError::from(error)),
        };
        if workflow_workspace(&workflow).as_ref() != Some(&workspace_id) {
            continue;
        }
        match client.get_workflow_status(&workflow_id) {
            Ok(status) => rows.push(WorkflowSummaryView::from_status(
                status,
                workspace_id.to_string(),
            )),
            Err(ApplicationError::WorkflowNotFound(_)) => continue,
            Err(error) => return Err(CommandError::from(error)),
        }
    }
    Ok(rows)
}

/// Clamps a requested history window into the supported range.
///
/// A missing limit becomes the default; an absurd one is capped rather than
/// obeyed, so one request can never ask for unbounded history.
fn history_limit(limit: Option<usize>) -> usize {
    limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT)
}

/// The workflow ids observed in `tasks`, first occurrence first.
///
/// The caller's ordering (active tasks first, then recent tasks newest-first)
/// decides the history order, and a workflow is listed once even when several of
/// its tasks are retained.
fn workflow_ids(tasks: &[Task]) -> Vec<WorkflowId> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut ids = Vec::new();
    for task in tasks {
        if let Some(workflow) = &task.workflow {
            if seen.insert(workflow.to_string()) {
                ids.push(workflow.clone());
            }
        }
    }
    ids
}

/// Derives the workspace a workflow belongs to from its steps' parameters.
///
/// Every resolved target encodes its owning workspace under a `*.workspace`
/// key (`git.workspace`, `build.workspace`, `service.workspace`, …), so the
/// first step that carries one identifies the workflow's workspace.
fn workflow_workspace(workflow: &Workflow) -> Option<WorkspaceId> {
    workflow.steps.iter().find_map(|step| {
        step.parameters.iter().find_map(|(key, value)| {
            if key.ends_with(".workspace") {
                WorkspaceId::new(value.clone()).ok()
            } else {
                None
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use service_orchestrator_domain::{
        GroupId, LibraryId, ProfileId, ServiceId, TaskId, TaskKind, WorkflowKind, WorkflowStep,
    };

    use super::*;

    fn input() -> PrepareEnvironmentInput {
        PrepareEnvironmentInput {
            workspace_id: "demo".to_owned(),
            services: vec!["orders-api".to_owned()],
            ..PrepareEnvironmentInput::default()
        }
    }

    #[test]
    fn safe_defaults_are_sync_only() {
        let defaults = PrepareEnvironmentInput::default();
        assert!(defaults.sync);
        assert!(defaults.include_dependencies);
        assert!(defaults.wait_for_health);
        assert!(!defaults.run_liquibase);
        assert!(!defaults.build);
        assert!(!defaults.build_libraries);
        assert!(!defaults.start);
        assert_eq!(defaults.branch, None);

        let request = build_request(&input()).expect("a valid request");
        assert!(request.sync);
        assert!(request.include_dependencies);
        assert!(request.wait_for_health);
        assert!(!request.run_liquibase, "migrations are never a default");
        assert!(!request.build);
        assert!(!request.start);
        assert_eq!(request.branch, None);
    }

    #[test]
    fn every_option_maps_onto_the_core_request() {
        let request = build_request(&PrepareEnvironmentInput {
            branch: Some("develop".to_owned()),
            sync: false,
            include_dependencies: false,
            build_libraries: true,
            run_liquibase: true,
            build: true,
            start: true,
            wait_for_health: false,
            ..input()
        })
        .expect("a valid request");

        assert_eq!(request.branch.expect("a branch").to_string(), "develop");
        assert!(!request.sync);
        assert!(!request.include_dependencies);
        assert!(request.build_libraries);
        assert!(request.run_liquibase);
        assert!(request.build);
        assert!(request.start);
        assert!(!request.wait_for_health);
    }

    #[test]
    fn profile_and_group_selections_map_exactly_once() {
        let by_profile = build_request(&PrepareEnvironmentInput {
            services: Vec::new(),
            profile: Some("full-stack".to_owned()),
            ..input()
        })
        .expect("a valid profile selection");
        assert_eq!(
            by_profile.profile.expect("a profile").to_string(),
            "full-stack"
        );
        assert!(by_profile.services.is_empty());

        let by_group = build_request(&PrepareEnvironmentInput {
            services: Vec::new(),
            group: Some("core".to_owned()),
            ..input()
        })
        .expect("a valid group selection");
        assert_eq!(by_group.group.expect("a group").to_string(), "core");
        assert!(by_group.profile.is_none());
    }

    #[test]
    fn exactly_one_selection_source_is_required() {
        let none = build_request(&PrepareEnvironmentInput {
            workspace_id: "demo".to_owned(),
            services: Vec::new(),
            ..PrepareEnvironmentInput::default()
        })
        .expect_err("no selection");
        assert_eq!(none.code, "INVALID_INPUT");
        assert!(none.message.contains("at least one"));

        // A UI placeholder must never become a lookup of the whole workspace.
        let blank_profile = build_request(&PrepareEnvironmentInput {
            workspace_id: "demo".to_owned(),
            profile: Some("   ".to_owned()),
            ..PrepareEnvironmentInput::default()
        })
        .expect_err("a blank profile is no selection");
        assert_eq!(blank_profile.code, "INVALID_INPUT");

        let both = build_request(&PrepareEnvironmentInput {
            profile: Some("full-stack".to_owned()),
            ..input()
        })
        .expect_err("two sources");
        assert_eq!(both.code, "INVALID_INPUT");
        assert!(both.message.contains("exactly one"));
    }

    #[test]
    fn identifiers_are_validated_before_any_ipc_call() {
        let workspace = build_request(&PrepareEnvironmentInput {
            workspace_id: "bad id".to_owned(),
            ..input()
        })
        .expect_err("a rejected workspace id");
        assert_eq!(workspace.code, "INVALID_INPUT");

        let service = build_request(&PrepareEnvironmentInput {
            services: vec!["bad id".to_owned()],
            ..input()
        })
        .expect_err("a rejected service id");
        assert_eq!(service.code, "INVALID_INPUT");

        let profile = build_request(&PrepareEnvironmentInput {
            services: Vec::new(),
            profile: Some("bad id".to_owned()),
            ..input()
        })
        .expect_err("a rejected profile id");
        assert_eq!(profile.code, "INVALID_INPUT");

        let group = build_request(&PrepareEnvironmentInput {
            services: Vec::new(),
            group: Some("bad id".to_owned()),
            ..input()
        })
        .expect_err("a rejected group id");
        assert_eq!(group.code, "INVALID_INPUT");

        let branch = build_request(&PrepareEnvironmentInput {
            branch: Some("bad branch".to_owned()),
            ..input()
        })
        .expect_err("a rejected branch");
        assert_eq!(branch.code, "INVALID_INPUT");
    }

    #[test]
    fn omitted_fields_deserialize_to_the_safe_defaults() {
        let json = serde_json::json!({
            "workspace_id": "demo",
            "services": ["orders-api"],
        });
        let parsed: PrepareEnvironmentInput =
            serde_json::from_value(json).expect("a partial payload");
        assert!(parsed.sync);
        assert!(parsed.include_dependencies);
        assert!(parsed.wait_for_health);
        assert!(!parsed.run_liquibase);
        assert!(!parsed.start);
        assert_eq!(parsed.profile, None);
        assert_eq!(parsed.group, None);
        assert_eq!(parsed.branch, None);
    }

    #[test]
    fn permission_denial_propagates_with_its_code() {
        let denial: CommandError = ApplicationError::PermissionDenied {
            permission: service_orchestrator_domain::Permission::Destructive,
            message: "a non-local database target requires confirmation".to_owned(),
        }
        .into();
        assert_eq!(denial.code, "PermissionDenied");
        assert!(denial.message.contains("non-local"));
    }

    #[test]
    fn history_is_workspace_scoped_newest_first_and_deduplicated() {
        let task = |id: &str| {
            let mut task = Task::new(
                TaskId::new(format!("task-{id}")).expect("id"),
                TaskKind::GitSync,
            );
            task.workflow = Some(WorkflowId::new(id).expect("id"));
            task
        };

        let tasks = vec![
            task("wf-2"),
            task("wf-2"),
            task("wf-1"),
            task("wf-elsewhere"),
            Task::new(
                TaskId::new("task-standalone").expect("id"),
                TaskKind::GitSync,
            ),
        ];

        // Deduplicated, first occurrence first, and tasks with no workflow are
        // ignored.
        let ids: Vec<String> = workflow_ids(&tasks)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(ids, ["wf-2", "wf-1", "wf-elsewhere"]);

        // A workflow's workspace is read from its steps' resolved target
        // parameters; a step that only carries `*.workspace_root` is not enough.
        let workflow = |workspace: &str| {
            let mut step = WorkflowStep::new(
                service_orchestrator_domain::StepId::new("step-1").expect("id"),
                TaskKind::GitSync,
            );
            step.parameters
                .insert("git.workspace".to_owned(), workspace.to_owned());
            Workflow {
                id: WorkflowId::new("wf-1").expect("id"),
                kind: WorkflowKind::SyncWorkspace,
                steps: vec![step],
            }
        };
        assert_eq!(
            workflow_workspace(&workflow("demo")),
            Some(WorkspaceId::new("demo").expect("id"))
        );
        assert_eq!(
            workflow_workspace(&workflow("other")),
            Some(WorkspaceId::new("other").expect("id"))
        );

        let mut root_only = WorkflowStep::new(
            service_orchestrator_domain::StepId::new("step-1").expect("id"),
            TaskKind::GitSync,
        );
        root_only
            .parameters
            .insert("git.workspace_root".to_owned(), "/home/dev/demo".to_owned());
        let root_only = Workflow {
            id: WorkflowId::new("wf-1").expect("id"),
            kind: WorkflowKind::SyncWorkspace,
            steps: vec![root_only],
        };
        assert_eq!(workflow_workspace(&root_only), None);
    }

    #[test]
    fn plan_and_status_views_render_backend_labels_verbatim() {
        use service_orchestrator_application::{
            EnvironmentPhaseKind, EnvironmentPlan, EnvironmentPlanPhase, EnvironmentPlanStep,
            WorkflowPhaseProgress, WorkflowStatus, WorkflowTaskFailure,
        };
        use service_orchestrator_domain::{Branch, WorkflowKind, WorkflowState};

        let plan = EnvironmentPlan {
            workspace_id: WorkspaceId::new("demo").expect("id"),
            target_branch: Some(Branch::new("develop").expect("a branch")),
            selected_services: vec![ServiceId::new("orders-api").expect("id")],
            resolved_services: vec![
                ServiceId::new("orders-api").expect("id"),
                ServiceId::new("admin-ui").expect("id"),
            ],
            included_dependencies: vec![ServiceId::new("admin-ui").expect("id")],
            required_libraries: vec![LibraryId::new("shared-kit").expect("id")],
            phases: vec![EnvironmentPlanPhase {
                kind: EnvironmentPhaseKind::GitSync,
                steps: vec![EnvironmentPlanStep {
                    phase: EnvironmentPhaseKind::GitSync,
                    task: TaskKind::GitSync,
                    service: Some(ServiceId::new("orders-api").expect("id")),
                    library: None,
                    description: "sync orders-api".to_owned(),
                }],
            }],
            step_count: 1,
            requires_destructive_authorization: true,
            warnings: vec!["library 'shared-kit' has no build command".to_owned()],
        };
        let view = EnvironmentPlanView::from(plan);
        assert_eq!(view.target_branch.as_deref(), Some("develop"));
        assert_eq!(view.included_dependencies, ["admin-ui"]);
        assert_eq!(view.required_libraries, ["shared-kit"]);
        assert_eq!(view.phases.len(), 1);
        assert_eq!(view.phases[0].kind, "git-sync");
        assert_eq!(view.phases[0].steps[0].task, "git_sync");
        assert_eq!(
            view.phases[0].steps[0].service.as_deref(),
            Some("orders-api")
        );
        assert!(view.requires_destructive_authorization);
        assert_eq!(view.warnings.len(), 1);

        let status = WorkflowStatus {
            workflow_id: WorkflowId::new("wf-1").expect("id"),
            kind: WorkflowKind::PrepareEnvironment,
            state: WorkflowState::PartiallySucceeded,
            total: 3,
            succeeded: 1,
            failed: 1,
            cancelled: 0,
            skipped: 0,
            pending: 1,
            started_at: None,
            finished_at: None,
            phases: vec![WorkflowPhaseProgress {
                phase: EnvironmentPhaseKind::Migration,
                total: 1,
                succeeded: 1,
                failed: 0,
                pending: 0,
            }],
            failures: vec![WorkflowTaskFailure {
                task_id: TaskId::new("task-1").expect("id"),
                kind: TaskKind::BuildService,
                service: Some(ServiceId::new("admin-ui").expect("id")),
                library: None,
                code: "BUILD_FAILED".to_owned(),
                message: "maven exited with code 1".to_owned(),
            }],
        };
        let view = WorkflowStatusView::from(status.clone());
        assert_eq!(view.kind, "prepare_environment");
        assert_eq!(view.state, "partially_succeeded");
        assert_eq!(view.total, 3);
        assert_eq!(view.pending, 1);
        assert_eq!(view.phases[0].phase, "migration");
        assert_eq!(view.failures[0].code, "BUILD_FAILED");
        assert_eq!(view.failures[0].service.as_deref(), Some("admin-ui"));

        let summary = WorkflowSummaryView::from_status(status, "demo");
        assert_eq!(summary.workflow_id, "wf-1");
        assert_eq!(summary.workspace_id, "demo");
        assert_eq!(summary.state, "partially_succeeded");
        assert_eq!(summary.succeeded, 1);
    }

    #[test]
    fn history_limits_are_clamped_not_obeyed_blindly() {
        assert_eq!(DEFAULT_HISTORY_LIMIT, 20);
        assert_eq!(MAX_HISTORY_LIMIT, 100);
        assert_eq!(history_limit(None), DEFAULT_HISTORY_LIMIT);
        assert_eq!(history_limit(Some(0)), 1);
        assert_eq!(history_limit(Some(10_000)), MAX_HISTORY_LIMIT);
        assert_eq!(history_limit(Some(5)), 5);

        let parsed = parse_workflow_id("wf-1").expect("a workflow id");
        assert_eq!(parsed.to_string(), "wf-1");
        let profile = ProfileId::new("full-stack").expect("id");
        let group = GroupId::new("core").expect("id");
        assert_eq!(profile.to_string(), "full-stack");
        assert_eq!(group.to_string(), "core");
    }
}
