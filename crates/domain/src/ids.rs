//! Strongly typed domain identifiers.
//!
//! The domain prefers a small number of dedicated identifier types over bare
//! `String`s. Passing a `ServiceId` where a `TaskId` is expected becomes a
//! compile error rather than a runtime bug, and every identifier is validated
//! once at construction time.
//!
//! Identifiers are opaque, stable, and human-readable. They are *not* intended
//! to carry structure; a service named `auth-service` has the id
//! `auth-service`, not a composite encoding of its workspace or type.
//!
//! # Validation rules
//!
//! Every identifier must:
//!
//! - be non-empty after trimming surrounding whitespace;
//! - be at most 128 characters long;
//! - contain only ASCII letters, digits, and the characters `-`, `_`, `.`, and
//!   `:` (which covers service names, library coordinates, and generated ids).
//!
//! The allowed alphabet is deliberately conservative so identifiers can be
//! embedded safely in configuration keys, resource URIs, and log lines.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// The maximum length, in characters, of any domain identifier.
pub const MAX_IDENTIFIER_LEN: usize = 128;

fn validate_identifier(value: &str, entity: &'static str) -> Result<String, DomainError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::invalid_identifier(entity, "must not be empty"));
    }
    if trimmed.chars().count() > MAX_IDENTIFIER_LEN {
        return Err(DomainError::invalid_identifier(
            entity,
            format!("must not exceed {MAX_IDENTIFIER_LEN} characters"),
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(DomainError::invalid_identifier(
            entity,
            "may only contain ASCII letters, digits, '-', '_', '.', and ':'",
        ));
    }
    Ok(trimmed.to_owned())
}

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $entity:literal) => {
        $(#[$meta])*
        #[derive(
            Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validates `value` and constructs the identifier.
            ///
            /// # Errors
            ///
            /// Returns a [`DomainError::InvalidIdentifier`] if the value is
            /// empty, too long, or contains disallowed characters.
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                Ok(Self(validate_identifier(&value.into(), $entity)?))
            }

            /// Returns the identifier as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = DomainError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = DomainError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = DomainError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

define_id!(
    /// Identifies a [`Service`](crate::service::Service) within a workspace.
    ServiceId,
    "service"
);
define_id!(
    /// Identifies a [`Library`](crate::library::Library) within a workspace.
    LibraryId,
    "library"
);
define_id!(
    /// Identifies a [`Workspace`](crate::workspace::Workspace).
    WorkspaceId,
    "workspace"
);
define_id!(
    /// Identifies a [`Task`](crate::task::Task).
    TaskId,
    "task"
);
define_id!(
    /// Identifies a [`Workflow`](crate::workflow::Workflow).
    WorkflowId,
    "workflow"
);
define_id!(
    /// Identifies a [`Profile`](crate::profile::Profile).
    ProfileId,
    "profile"
);
define_id!(
    /// Identifies a [`ServiceGroup`](crate::group::ServiceGroup).
    GroupId,
    "service group"
);
define_id!(
    /// Identifies a single step inside a [`Workflow`](crate::workflow::Workflow).
    StepId,
    "workflow step"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_typical_identifiers() {
        assert_eq!(
            ServiceId::new("auth-service").unwrap().as_str(),
            "auth-service"
        );
        assert!(LibraryId::new("common_core").is_ok());
        assert!(WorkspaceId::new("project-x").is_ok());
        assert!(LibraryId::new("com.company:common-core").is_ok());
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let id = ServiceId::new("  auth-service \n").unwrap();
        assert_eq!(id.as_str(), "auth-service");
    }

    #[test]
    fn rejects_empty_identifiers() {
        let error = ServiceId::new("   ").unwrap_err();
        assert!(matches!(error, DomainError::InvalidIdentifier { .. }));
    }

    #[test]
    fn rejects_disallowed_characters() {
        assert!(ServiceId::new("auth service").is_err());
        assert!(ServiceId::new("auth/service").is_err());
        assert!(ServiceId::new("auth;rm -rf").is_err());
    }

    #[test]
    fn rejects_overlong_identifiers() {
        let too_long = "a".repeat(MAX_IDENTIFIER_LEN + 1);
        assert!(TaskId::new(too_long).is_err());
        assert!(TaskId::new("a".repeat(MAX_IDENTIFIER_LEN)).is_ok());
    }

    #[test]
    fn round_trips_through_string_and_fromstr() {
        let id: ProfileId = "backend-development".parse().unwrap();
        assert_eq!(id.to_string(), "backend-development");
        let converted = ProfileId::try_from(String::from("backend-development")).unwrap();
        assert_eq!(id, converted);
        let borrowed = ProfileId::try_from("backend-development").unwrap();
        assert_eq!(id, borrowed);
    }

    #[test]
    fn distinct_id_types_do_not_interchange() {
        // This is primarily a compile-time guarantee; the runtime assertion
        // simply documents the intent.
        let service = ServiceId::new("shared-name").unwrap();
        let task = TaskId::new("shared-name").unwrap();
        assert_eq!(service.as_str(), task.as_str());
    }
}
