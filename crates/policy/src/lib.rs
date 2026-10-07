//! # Policy
//!
//! Permission model and policy engine.
//!
//! Every operation exposed through an interface is classified with a
//! [`Permission`] (`Read`, `SafeWrite`, `Destructive`). The [`PolicyEngine`]
//! turns that classification plus the caller's [`Authorization`] into a
//! [`PolicyDecision`], so that no interface — including MCP — can bypass the
//! safety rules.
//!
//! The engine is deliberately small and pure: it performs no I/O and depends
//! only on the domain's [`Permission`] vocabulary. The application layer calls
//! the engine before scheduling any mutating work
//! (`docs/application-core.md`); a future TUI/MCP adapter uses
//! [`PolicyEngine::evaluate`] to decide whether to ask the user to confirm a
//! destructive operation first.
//!
//! Depends only on `service-orchestrator-domain`.

use service_orchestrator_domain::Permission;
use thiserror::Error;

/// The caller's authorization to perform risky operations.
///
/// An interface starts from [`Authorization::standard`], in which destructive
/// operations are not permitted. It is upgraded explicitly — typically after a
/// user has confirmed an action — via [`Authorization::allow_destructive`].
/// Because authorization is passed per operation rather than stored globally,
/// a single confirmation cannot silently unlock unrelated future operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Authorization {
    allow_destructive: bool,
}

impl Authorization {
    /// Authorization for read and safe-write operations only.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            allow_destructive: false,
        }
    }

    /// Authorization that additionally permits destructive operations.
    #[must_use]
    pub const fn allow_destructive() -> Self {
        Self {
            allow_destructive: true,
        }
    }

    /// Returns `true` if destructive operations are permitted.
    #[must_use]
    pub const fn permits_destructive(self) -> bool {
        self.allow_destructive
    }
}

/// The outcome of evaluating a permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The operation may proceed immediately.
    Allowed,
    /// The operation is destructive and the caller has not yet authorized it.
    /// The interface should ask for confirmation and retry with
    /// [`Authorization::allow_destructive`].
    RequiresConfirmation,
}

impl PolicyDecision {
    /// Returns `true` if the operation may proceed.
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }
}

/// An error produced when a policy check denies an operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolicyError {
    /// A destructive operation was attempted without authorization.
    #[error("permission '{permission}' requires explicit confirmation")]
    ConfirmationRequired {
        /// The permission that requires confirmation.
        permission: Permission,
    },
}

/// A stateless policy engine.
///
/// The engine holds no configuration today; it exists as a distinct,
/// replaceable component so that richer rules (per-operation risk overrides,
/// allow-lists, audit hooks) can be added without changing call sites.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PolicyEngine;

impl PolicyEngine {
    /// Creates a policy engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Evaluates `permission` under `authorization`.
    ///
    /// Read and safe-write operations are always allowed. Destructive
    /// operations require an authorization that permits them.
    #[must_use]
    pub const fn evaluate(
        &self,
        permission: Permission,
        authorization: Authorization,
    ) -> PolicyDecision {
        if permission.requires_confirmation() && !authorization.permits_destructive() {
            PolicyDecision::RequiresConfirmation
        } else {
            PolicyDecision::Allowed
        }
    }

    /// Checks `permission`, returning an error when it must be confirmed first.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::ConfirmationRequired`] when `permission` is
    /// destructive and `authorization` does not permit it.
    pub const fn check(
        &self,
        permission: Permission,
        authorization: Authorization,
    ) -> Result<(), PolicyError> {
        match self.evaluate(permission, authorization) {
            PolicyDecision::Allowed => Ok(()),
            PolicyDecision::RequiresConfirmation => {
                Err(PolicyError::ConfirmationRequired { permission })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_operations_are_allowed() {
        let engine = PolicyEngine::new();
        assert!(engine
            .check(Permission::Read, Authorization::standard())
            .is_ok());
    }

    #[test]
    fn safe_write_operations_are_allowed() {
        let engine = PolicyEngine::new();
        assert!(engine
            .check(Permission::SafeWrite, Authorization::standard())
            .is_ok());
        assert_eq!(
            engine.evaluate(Permission::SafeWrite, Authorization::standard()),
            PolicyDecision::Allowed
        );
    }

    #[test]
    fn destructive_operations_require_confirmation_by_default() {
        let engine = PolicyEngine::new();
        let decision = engine.evaluate(Permission::Destructive, Authorization::standard());
        assert_eq!(decision, PolicyDecision::RequiresConfirmation);
        let error = engine
            .check(Permission::Destructive, Authorization::standard())
            .unwrap_err();
        assert!(matches!(
            error,
            PolicyError::ConfirmationRequired {
                permission: Permission::Destructive
            }
        ));
    }

    #[test]
    fn destructive_operations_are_allowed_once_authorized() {
        let engine = PolicyEngine::new();
        assert!(engine
            .check(Permission::Destructive, Authorization::allow_destructive())
            .is_ok());
    }

    #[test]
    fn standard_authorization_does_not_permit_destructive() {
        let authorization = Authorization::standard();
        assert!(!authorization.permits_destructive());
        assert!(!Authorization::default().permits_destructive());
        assert!(Authorization::allow_destructive().permits_destructive());
    }
}
