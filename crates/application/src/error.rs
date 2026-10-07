//! The application error model.
//!
//! [`ApplicationError`] is the single error type returned by the application
//! use cases. It is deliberately independent of any interface: the TUI and MCP
//! adapters map it into their own response shapes but neither can observe a
//! lower-level detail it does not understand.
//!
//! The variants fall into three groups:
//!
//! - **not found** — a referenced workspace, service, library, profile, group,
//!   task, or workflow does not exist;
//! - **precondition / validation** — the operation is not valid for the current
//!   configuration or runtime state;
//! - **boundary** — a port (task engine, log service) failed, with its context
//!   preserved but no low-level leak.

use service_orchestrator_domain::Permission;
use service_orchestrator_domain::{
    GroupId, LibraryId, ProfileId, ServiceId, TaskId, WorkflowId, WorkspaceId,
};
use thiserror::Error;

/// Errors produced by application use cases.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApplicationError {
    /// The requested workspace does not exist.
    #[error("workspace '{0}' was not found")]
    WorkspaceNotFound(WorkspaceId),

    /// The requested service does not exist.
    #[error("service '{0}' was not found")]
    ServiceNotFound(ServiceId),

    /// The requested library does not exist.
    #[error("library '{0}' was not found")]
    LibraryNotFound(LibraryId),

    /// The requested profile does not exist.
    #[error("profile '{0}' was not found")]
    ProfileNotFound(ProfileId),

    /// The requested service group does not exist.
    #[error("service group '{0}' was not found")]
    GroupNotFound(GroupId),

    /// The requested task does not exist.
    #[error("task '{0}' was not found")]
    TaskNotFound(TaskId),

    /// The requested workflow does not exist.
    #[error("workflow '{0}' was not found")]
    WorkflowNotFound(WorkflowId),

    /// A precondition for the operation was not met.
    #[error("invalid operation: {message}")]
    InvalidOperation {
        /// Why the operation cannot proceed.
        message: String,
    },

    /// A referenced dependency could not be resolved.
    #[error("'{owner}' depends on '{target}', which was not found")]
    DependencyNotFound {
        /// The entity that declared the dependency.
        owner: String,
        /// The dependency target that could not be resolved.
        target: String,
    },

    /// The configuration is not valid for the requested operation.
    #[error("configuration is invalid: {message}")]
    ConfigurationInvalid {
        /// What is wrong with the configuration.
        message: String,
    },

    /// A constructed workflow is not well formed.
    #[error("workflow is invalid: {message}")]
    WorkflowInvalid {
        /// What is wrong with the workflow.
        message: String,
    },

    /// The operation requires explicit confirmation and was denied.
    #[error("permission denied: {message}")]
    PermissionDenied {
        /// The permission that was required.
        permission: Permission,
        /// A human-readable explanation.
        message: String,
    },

    /// The task engine refused or failed the operation.
    #[error("task engine error: {message}")]
    TaskManager {
        /// The underlying engine message.
        message: String,
    },

    /// The log service failed.
    #[error("log service error: {message}")]
    LogService {
        /// The underlying log service message.
        message: String,
    },
}

impl ApplicationError {
    /// Builds an [`ApplicationError::InvalidOperation`].
    #[must_use]
    pub(crate) fn invalid_operation(message: impl Into<String>) -> Self {
        Self::InvalidOperation {
            message: message.into(),
        }
    }
}
