//! Domain error model.
//!
//! This module defines the small set of errors that belong to the domain
//! itself: identifier validation, invariant violations, missing or duplicate
//! entities, invalid task state transitions, dependency cycles, and invalid
//! commands or paths.
//!
//! It deliberately does **not** define a single global error enum for the whole
//! application. The layering described in `docs/architecture.md` expects each
//! layer to own its own error category:
//!
//! - **validation / domain errors** live here, in [`DomainError`];
//! - **application errors** (use-case failures, missing ports inputs) will live
//!   in the `application` crate;
//! - **infrastructure errors** (Git, process, build, Liquibase, health) will
//!   live in their respective infrastructure crates and be mapped into the
//!   application error at the boundary.
//!
//! Keeping the domain error small and meaningful keeps error handling honest:
//! a caller in the domain can only produce a domain failure, never an I/O or
//! transport failure that it has no way to fix.

use thiserror::Error;

/// Errors produced while constructing or validating domain values.
///
/// `DomainError` is intentionally flat and descriptive instead of nested. Each
/// variant carries the entity name and a human-readable reason so that both the
/// TUI and MCP adapters can surface an actionable message without knowing which
/// domain type failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    /// A strongly typed identifier failed validation.
    #[error("invalid {entity} identifier: {reason}")]
    InvalidIdentifier {
        /// The entity the identifier belongs to, for example `service`.
        entity: &'static str,
        /// Why the identifier was rejected.
        reason: String,
    },

    /// A domain object violated one of its invariants.
    #[error("invalid {entity}: {reason}")]
    Validation {
        /// The entity that was being validated, for example `service`.
        entity: &'static str,
        /// Why the value was rejected.
        reason: String,
    },

    /// An entity was referenced but does not exist in the surrounding scope.
    #[error("{entity} '{id}' was not found")]
    NotFound {
        /// The entity type that was looked up.
        entity: &'static str,
        /// The identifier that could not be resolved.
        id: String,
    },

    /// Two entities of the same kind shared an identifier.
    #[error("duplicate {entity} identifier '{id}'")]
    Duplicate {
        /// The entity type that was duplicated.
        entity: &'static str,
        /// The identifier that appeared more than once.
        id: String,
    },

    /// A task was asked to move to a status it cannot reach.
    #[error("task '{id}' cannot transition from '{from}' to '{to}'")]
    InvalidTaskTransition {
        /// The task whose state transition was rejected.
        id: String,
        /// The current task status.
        from: &'static str,
        /// The requested task status.
        to: &'static str,
    },

    /// A dependency graph contained a cycle and therefore cannot be ordered.
    #[error("dependency cycle detected involving: {nodes}")]
    DependencyCycle {
        /// A comma-separated list of the identifiers that form the cycle.
        nodes: String,
    },

    /// A command specification was malformed or unsafe.
    #[error("invalid command: {reason}")]
    InvalidCommand {
        /// Why the command was rejected.
        reason: String,
    },

    /// A workspace path was malformed or escaped its root.
    #[error("invalid path: {reason}")]
    InvalidPath {
        /// Why the path was rejected.
        reason: String,
    },
}

impl DomainError {
    /// Builds an [`DomainError::InvalidIdentifier`] with a static reason.
    #[must_use]
    pub(crate) fn invalid_identifier(entity: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidIdentifier {
            entity,
            reason: reason.into(),
        }
    }

    /// Builds a [`DomainError::Validation`] with the given entity and reason.
    #[must_use]
    pub(crate) fn validation(entity: &'static str, reason: impl Into<String>) -> Self {
        Self::Validation {
            entity,
            reason: reason.into(),
        }
    }

    /// Builds a [`DomainError::NotFound`] for the given entity and identifier.
    #[must_use]
    pub(crate) fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            entity,
            id: id.into(),
        }
    }

    /// Builds a [`DomainError::Duplicate`] for the given entity and identifier.
    #[must_use]
    pub(crate) fn duplicate(entity: &'static str, id: impl Into<String>) -> Self {
        Self::Duplicate {
            entity,
            id: id.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_include_entity_and_reason() {
        let error = DomainError::validation("service", "name must not be empty");
        assert_eq!(error.to_string(), "invalid service: name must not be empty");
    }

    #[test]
    fn invalid_identifier_message_includes_entity() {
        let error = DomainError::invalid_identifier("task", "must not be empty");
        assert!(error.to_string().contains("task"));
        assert!(error.to_string().contains("must not be empty"));
    }
}
