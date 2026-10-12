//! Domain-backed validation of command inputs.
//!
//! Every identifier that reaches the adapter is validated through the same
//! domain value types the rest of the platform uses, so the GUI cannot smuggle
//! a malformed workspace or service id across the IPC boundary
//! (`tasks/019.md` §35). Validation lives here, once, and is reused by the
//! read queries and the mutating commands alike.

use service_orchestrator_domain::{ServiceId, WorkspaceId};

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
}
