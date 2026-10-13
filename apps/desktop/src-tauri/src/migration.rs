//! Database-migration use cases behind the desktop commands.
//!
//! These are thin adapters over the shared [`OrchestratorClient`]: the
//! configuration snapshot, the inspection/preview/apply tasks, and the
//! coordinated bulk-apply workflow all run in the daemon's Application Core —
//! the GUI never executes Liquibase, never touches a database, and never
//! schedules anything on its own (`tasks/021.md` §5, §8, §33).
//!
//! # Authorization
//!
//! Applying to a non-local target requires the daemon's destructive
//! authorization. The frontend's confirmation only *expresses user intent*: it
//! is forwarded as the request's `allow_destructive` flag, and the daemon
//! honors it only for an interactive client connected as the same user — the
//! policy engine still decides, and a denial surfaces as an actionable
//! `PermissionDenied` error (`tasks/021.md` §8, §9).

use service_orchestrator_application::{Authorization, MigrationStatus};
use service_orchestrator_client::OrchestratorClient;
use service_orchestrator_domain::Task;

use crate::dto::{MigrationRowView, MigrationStatusView, OperationOutcome, TaskView};
use crate::error::CommandError;
use crate::validation::{parse_service_id, parse_service_ids, parse_workspace_id};

/// The task kinds that count as a service's latest migration operation.
const MIGRATION_KINDS: [&str; 5] = [
    "migration_validate",
    "migration_status",
    "migration_pending",
    "migration_preview",
    "migration_apply",
];

/// How many recent tasks are scanned for a service's latest migration task.
///
/// Bounded on purpose: the screen shows the latest outcome, not an unbounded
/// history (`tasks/021.md` §15).
const RECENT_TASK_SCAN: usize = 200;

/// Lists a workspace's migration-enabled services with their latest task.
///
/// Services without any Liquibase configuration are skipped: they have no
/// migration setup to show. Services whose configuration exists but is
/// disabled are listed with `enabled = false` so the screen reflects the real
/// configuration (`tasks/021.md` §4, §5).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected workspace id, or the daemon's
/// structured error for anything else.
pub fn list_migrations(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
) -> Result<Vec<MigrationRowView>, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let services = client
        .list_services(&workspace_id)
        .map_err(CommandError::from)?;
    let recent = client.list_recent_tasks(RECENT_TASK_SCAN);

    let mut rows = Vec::new();
    for service in &services {
        match client.migration_status(&service.id) {
            Ok(status) => rows.push(MigrationRowView {
                status: MigrationStatusView::from(&status),
                // The row is already scoped to one workspace, so its task row
                // is resolved to the same workspace for cross-screen navigation.
                latest_task: latest_migration_task(&recent, service.id.as_str())
                    .map(|task| task.with_workspace(workspace_id.as_str())),
            }),
            // No migration configuration at all: the service simply has no
            // migration row. Any other failure is reported, not hidden.
            Err(service_orchestrator_application::ApplicationError::InvalidOperation {
                ..
            }) => {}
            Err(error) => return Err(CommandError::from(error)),
        }
    }
    Ok(rows)
}

/// Returns one service's configuration-derived migration snapshot.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected service id, `InvalidOperation` when
/// the service has no migration configuration, or the daemon's structured
/// error for anything else.
pub fn get_migration_status(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<MigrationStatusView, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let status: MigrationStatus = client
        .migration_status(&service_id)
        .map_err(CommandError::from)?;
    Ok(MigrationStatusView::from(&status))
}

/// Schedules a migration-configuration validation task.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected service id, or the daemon's
/// structured error (including a permission denial) for anything else.
pub fn validate_migration(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .validate_migration(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Schedules a migration-status inspection task.
///
/// # Errors
///
/// Same as [`validate_migration`].
pub fn inspect_status(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .get_migration_status(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Schedules a pending-changeset inspection task.
///
/// # Errors
///
/// Same as [`validate_migration`].
pub fn inspect_pending(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .inspect_migration_pending(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Schedules a read-only SQL preview task.
///
/// # Errors
///
/// Same as [`validate_migration`].
pub fn preview(
    client: &dyn OrchestratorClient,
    service_id: &str,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = client
        .preview_migration(&service_id)
        .map_err(CommandError::from)?;
    Ok(OperationOutcome::from(result))
}

/// Applies pending migrations for one service.
///
/// With `confirmed` the request carries the destructive-authorization intent;
/// the daemon independently verifies the client kind and peer identity before
/// honoring it, so the confirmation boolean is never a trusted boundary. A
/// denial surfaces as an actionable `PermissionDenied` error
/// (`tasks/021.md` §8, §9).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected service id, `PermissionDenied` when
/// the target's environment was not authorized, or the daemon's structured
/// error for anything else.
pub fn apply(
    client: &dyn OrchestratorClient,
    service_id: &str,
    confirmed: bool,
) -> Result<OperationOutcome, CommandError> {
    let service_id = parse_service_id(service_id)?;
    let result = submit_apply(client, confirmed, |authorized| {
        authorized.apply_migration(&service_id)
    })?;
    Ok(OperationOutcome::from(result))
}

/// Applies migrations for the selected services as one coordinated workflow.
///
/// Bulk migration runs only through the backend's existing workflow, which
/// serializes steps that share a database target; the GUI never loops over
/// services independently (`tasks/021.md` §9).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for a rejected identifier or an empty selection,
/// `PermissionDenied` when a non-local target was not authorized, or the
/// daemon's structured error for anything else.
pub fn apply_selected(
    client: &dyn OrchestratorClient,
    workspace_id: &str,
    service_ids: &[String],
    confirmed: bool,
) -> Result<OperationOutcome, CommandError> {
    let workspace_id = parse_workspace_id(workspace_id)?;
    let service_ids = parse_service_ids(service_ids)?;
    let result = submit_apply(client, confirmed, |authorized| {
        authorized.apply_selected_migrations(&workspace_id, &service_ids)
    })?;
    Ok(OperationOutcome::from(result))
}

/// Submits an apply operation, attaching destructive intent when confirmed.
fn submit_apply(
    client: &dyn OrchestratorClient,
    confirmed: bool,
    operation: impl FnOnce(
        &dyn OrchestratorClient,
    ) -> Result<
        service_orchestrator_application::OperationResult,
        service_orchestrator_application::ApplicationError,
    >,
) -> Result<service_orchestrator_application::OperationResult, CommandError> {
    if confirmed {
        let authorized = client.with_authorization(Authorization::allow_destructive());
        operation(authorized.as_ref()).map_err(CommandError::from)
    } else {
        // No confirmation: the standard authorization applies, so a local
        // target still works while a non-local one is refused by the daemon.
        operation(client).map_err(CommandError::from)
    }
}

/// Returns the newest migration task of a service, if one was observed.
fn latest_migration_task(recent: &[Task], service_id: &str) -> Option<TaskView> {
    recent
        .iter()
        .filter(|task| {
            task.service
                .as_ref()
                .is_some_and(|id| id.as_str() == service_id)
                && MIGRATION_KINDS.contains(&task.kind.label())
        })
        .max_by_key(|task| task.created_at)
        .map(TaskView::from)
}

#[cfg(test)]
mod tests {
    use service_orchestrator_domain::{ServiceId, Task, TaskId, TaskKind, TaskStatus, Timestamp};

    use super::*;

    fn migration_task(service: &str, kind: TaskKind) -> Task {
        let mut task = Task::new(TaskId::new("task-1").expect("a valid id"), kind)
            .with_service(ServiceId::new(service).expect("a valid id"));
        // `Timestamp::now()` has nanosecond resolution, so sequential tasks
        // have distinct creation times.
        task.created_at = Timestamp::now();
        task.status = TaskStatus::Succeeded;
        task
    }

    #[test]
    fn the_latest_migration_task_is_the_newest_of_the_right_kind_and_service() {
        let older = migration_task("orders-api", TaskKind::MigrationApply);
        let newer = migration_task("orders-api", TaskKind::MigrationPreview);
        let other_service = migration_task("user-service", TaskKind::MigrationApply);
        let other_kind = migration_task("orders-api", TaskKind::BuildService);
        let recent = vec![older, newer, other_service, other_kind];

        let latest = latest_migration_task(&recent, "orders-api").expect("a task");
        assert_eq!(latest.kind, "migration_preview");
        assert_eq!(latest.service_id.as_deref(), Some("orders-api"));

        assert!(latest_migration_task(&recent, "admin-ui").is_none());
    }

    #[test]
    fn identifiers_are_validated_before_any_ipc_call() {
        assert_eq!(
            parse_service_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
    }
}
