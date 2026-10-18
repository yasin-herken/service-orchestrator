//! Domain-backed validation of command inputs.
//!
//! Every identifier that reaches the adapter is validated through the same
//! domain value types the rest of the platform uses, so the GUI cannot smuggle
//! a malformed workspace or service id across the IPC boundary
//! (`tasks/019.md` §35). Validation lives here, once, and is reused by the
//! read queries and the mutating commands alike.

use service_orchestrator_domain::{
    Branch, GroupId, LibraryId, MigrationPresetId, OperationPresetId, ProfileId, ServiceId, TaskId,
    TemplateId, WorkflowId, WorkspaceId,
};

use crate::error::CommandError;

/// Validates a workspace identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_workspace_id(raw: &str) -> Result<WorkspaceId, CommandError> {
    WorkspaceId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid workspace id: {error}")))
}

/// Validates a service identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_service_id(raw: &str) -> Result<ServiceId, CommandError> {
    ServiceId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid service id: {error}")))
}

/// Validates a non-empty batch of service identifiers.
///
/// A bulk operation with no members is rejected here rather than being sent to
/// the daemon and answered with a no-op, so the frontend gets an immediate,
/// truthful error (`tasks/019.md` §8, §28).
///
/// # Errors
///
/// Returns `INVALID_INPUT` for an empty selection or any rejected identifier.
pub(crate) fn parse_service_ids(raw: &[String]) -> Result<Vec<ServiceId>, CommandError> {
    if raw.is_empty() {
        return Err(CommandError::invalid_input(
            "select at least one service before running a bulk operation",
        ));
    }
    raw.iter().map(|id| parse_service_id(id)).collect()
}

/// Validates a library identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_library_id(raw: &str) -> Result<LibraryId, CommandError> {
    LibraryId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid library id: {error}")))
}

/// Validates a non-empty batch of library identifiers.
///
/// # Errors
///
/// Returns `INVALID_INPUT` for an empty selection or any rejected identifier.
pub(crate) fn parse_library_ids(raw: &[String]) -> Result<Vec<LibraryId>, CommandError> {
    if raw.is_empty() {
        return Err(CommandError::invalid_input(
            "select at least one library before running a bulk operation",
        ));
    }
    raw.iter().map(|id| parse_library_id(id)).collect()
}

/// Validates a branch name through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the branch name.
pub(crate) fn parse_branch(raw: &str) -> Result<Branch, CommandError> {
    Branch::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid branch: {error}")))
}

/// Validates a profile identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_profile_id(raw: &str) -> Result<ProfileId, CommandError> {
    ProfileId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid profile id: {error}")))
}

/// Validates a group identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_group_id(raw: &str) -> Result<GroupId, CommandError> {
    GroupId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid group id: {error}")))
}

/// Validates a task identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_task_id(raw: &str) -> Result<TaskId, CommandError> {
    TaskId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid task id: {error}")))
}

/// Validates a workflow identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_workflow_id(raw: &str) -> Result<WorkflowId, CommandError> {
    WorkflowId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid workflow id: {error}")))
}

/// Validates a template identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_template_id(raw: &str) -> Result<TemplateId, CommandError> {
    TemplateId::new(raw)
        .map_err(|error| CommandError::invalid_input(format!("invalid template id: {error}")))
}

/// Validates a migration-preset identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_migration_preset_id(raw: &str) -> Result<MigrationPresetId, CommandError> {
    MigrationPresetId::new(raw).map_err(|error| {
        CommandError::invalid_input(format!("invalid migration preset id: {error}"))
    })
}

/// Validates a saved-operation-preset identifier through the domain.
///
/// # Errors
///
/// Returns `INVALID_INPUT` when the domain rejects the identifier.
pub(crate) fn parse_operation_preset_id(raw: &str) -> Result<OperationPresetId, CommandError> {
    OperationPresetId::new(raw).map_err(|error| {
        CommandError::invalid_input(format!("invalid operation preset id: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_identifiers_are_rejected_by_the_domain() {
        let error = parse_workspace_id("  ").expect_err("empty is invalid");
        assert_eq!(error.code, "INVALID_INPUT");
        assert!(!error.message.is_empty());

        let error = parse_service_id("").expect_err("empty is invalid");
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn whitespace_identifiers_are_rejected_by_the_domain() {
        let error = parse_service_id("two words").expect_err("spaces are invalid");
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn valid_identifiers_parse() {
        assert_eq!(
            parse_workspace_id("demo").expect("valid").to_string(),
            "demo"
        );
        assert_eq!(
            parse_service_id("orders-api").expect("valid").to_string(),
            "orders-api"
        );
    }

    #[test]
    fn profile_and_group_ids_validate() {
        assert_eq!(
            parse_profile_id("full-stack").expect("valid").to_string(),
            "full-stack"
        );
        assert_eq!(
            parse_profile_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
        assert_eq!(parse_group_id("core").expect("valid").to_string(), "core");
        assert_eq!(
            parse_group_id("bad id").expect_err("space").code,
            "INVALID_INPUT"
        );
    }

    #[test]
    fn an_empty_selection_is_rejected() {
        let error = parse_service_ids(&[]).expect_err("empty is invalid");
        assert_eq!(error.code, "INVALID_INPUT");
        assert!(error.message.contains("at least one"));
    }

    #[test]
    fn a_batch_is_validated_member_by_member() {
        let ids = parse_service_ids(&["orders-api".to_owned(), "admin-ui".to_owned()])
            .expect("both are valid");
        assert_eq!(ids.len(), 2);

        let error = parse_service_ids(&["orders-api".to_owned(), "bad id".to_owned()])
            .expect_err("one member is invalid");
        assert_eq!(error.code, "INVALID_INPUT");
    }

    #[test]
    fn library_ids_batches_and_task_workflow_ids_validate() {
        assert_eq!(
            parse_library_id("shared-kit").expect("valid").to_string(),
            "shared-kit"
        );
        assert_eq!(
            parse_library_id("").expect_err("empty is invalid").code,
            "INVALID_INPUT"
        );

        let ids = parse_library_ids(&["shared-kit".to_owned()]).expect("one is valid");
        assert_eq!(ids.len(), 1);
        assert_eq!(
            parse_library_ids(&[]).expect_err("empty selection").code,
            "INVALID_INPUT"
        );

        assert_eq!(parse_branch("main").expect("valid").to_string(), "main");
        assert_eq!(parse_branch("").expect_err("empty").code, "INVALID_INPUT");

        assert_eq!(
            parse_task_id("task-1").expect("valid").to_string(),
            "task-1"
        );
        assert_eq!(parse_task_id("").expect_err("empty").code, "INVALID_INPUT");

        assert_eq!(
            parse_workflow_id("wf-1").expect("valid").to_string(),
            "wf-1"
        );
        assert_eq!(
            parse_workflow_id("").expect_err("empty").code,
            "INVALID_INPUT"
        );
    }
}
