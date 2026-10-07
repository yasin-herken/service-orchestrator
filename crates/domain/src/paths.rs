//! Provider-independent path value objects.
//!
//! The domain must not depend on the filesystem or on OS-specific path types.
//! These value objects capture only the *shape* and safety of a path: whether it
//! is relative to a workspace or absolute, and whether it could escape its
//! intended location. Resolving a path against a real filesystem is the job of
//! the infrastructure layer.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

fn has_parent_traversal(value: &str) -> bool {
    value.split(['/', '\\']).any(|segment| segment == "..")
}

fn has_empty_segments(value: &str) -> bool {
    // An empty interior segment (`a//b`) is ambiguous; a single trailing slash
    // is tolerated because it is harmless.
    value.contains("//")
}

/// A path relative to a workspace root.
///
/// Relative paths are used for repository checkouts, command working
/// directories, and other locations that are meaningful only inside a
/// workspace. A relative path must not be absolute, must not contain a `..`
/// segment, and must not begin with `~`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RelativePath(String);

impl RelativePath {
    /// Validates `value` and constructs a relative path.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::InvalidPath`] if the path is empty, absolute,
    /// starts with `~`, or contains a `..` segment.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidPath {
                reason: "path must not be empty".to_owned(),
            });
        }
        if trimmed.starts_with('/') || trimmed.starts_with('\\') {
            return Err(DomainError::InvalidPath {
                reason: format!("'{trimmed}' must be relative, not absolute"),
            });
        }
        if trimmed.starts_with('~') {
            return Err(DomainError::InvalidPath {
                reason: "path must not begin with '~'".to_owned(),
            });
        }
        if has_parent_traversal(trimmed) {
            return Err(DomainError::InvalidPath {
                reason: format!("'{trimmed}' must not contain a '..' segment"),
            });
        }
        if has_empty_segments(trimmed) {
            return Err(DomainError::InvalidPath {
                reason: format!("'{trimmed}' must not contain empty path segments"),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the path as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An absolute filesystem path.
///
/// Used for workspace roots. It must be absolute and must not contain a `..`
/// segment; `~` is intentionally rejected because the domain never performs
/// shell-style home-directory expansion.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AbsolutePath(String);

impl AbsolutePath {
    /// Validates `value` and constructs an absolute path.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::InvalidPath`] if the path is empty, relative,
    /// starts with `~`, or contains a `..` segment.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidPath {
                reason: "path must not be empty".to_owned(),
            });
        }
        if !trimmed.starts_with('/') {
            return Err(DomainError::InvalidPath {
                reason: format!("'{trimmed}' must be an absolute path"),
            });
        }
        if trimmed.starts_with('~') {
            return Err(DomainError::InvalidPath {
                reason: "path must not begin with '~'".to_owned(),
            });
        }
        if has_parent_traversal(trimmed) {
            return Err(DomainError::InvalidPath {
                reason: format!("'{trimmed}' must not contain a '..' segment"),
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the path as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AbsolutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_relative_paths() {
        assert_eq!(
            RelativePath::new("backend/auth-service").unwrap().as_str(),
            "backend/auth-service"
        );
        assert!(RelativePath::new("./mvnw").is_ok());
    }

    #[test]
    fn rejects_absolute_relative_paths() {
        assert!(RelativePath::new("/etc/passwd").is_err());
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(RelativePath::new("../secrets").is_err());
        assert!(RelativePath::new("backend/../../secrets").is_err());
    }

    #[test]
    fn rejects_home_expansion() {
        assert!(RelativePath::new("~/projects").is_err());
        assert!(AbsolutePath::new("~/projects").is_err());
    }

    #[test]
    fn accepts_absolute_workspace_root() {
        assert_eq!(
            AbsolutePath::new("/Users/dev/projects/project-x")
                .unwrap()
                .as_str(),
            "/Users/dev/projects/project-x"
        );
    }

    #[test]
    fn rejects_relative_absolute_path() {
        assert!(AbsolutePath::new("projects/project-x").is_err());
    }

    #[test]
    fn rejects_absolute_parent_traversal() {
        assert!(AbsolutePath::new("/Users/dev/../root").is_err());
    }
}
