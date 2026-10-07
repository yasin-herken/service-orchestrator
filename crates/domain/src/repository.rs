//! Source-control repository description.
//!
//! [`Repository`] describes *where* the source lives and *which branch* is the
//! default. It is intentionally provider-independent and contains no Git
//! behaviour; the Git adapter in the infrastructure layer consumes this data to
//! perform fetches, checkouts, and status queries.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;
use crate::paths::RelativePath;

/// A branch name.
///
/// A branch name must be non-empty, must not contain whitespace or control
/// characters, and must not begin with `-` (which could be mistaken for a
/// command-line flag).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Branch(String);

impl Branch {
    /// Validates `value` and constructs a branch name.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the name is empty, starts with
    /// `-`, or contains whitespace or control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DomainError::validation("branch", "must not be empty"));
        }
        if trimmed.starts_with('-') {
            return Err(DomainError::validation("branch", "must not begin with '-'"));
        }
        if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(DomainError::validation(
                "branch",
                "must not contain whitespace or control characters",
            ));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the branch name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Branch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The hosting provider of a repository.
///
/// This is metadata only. The domain does not embed provider-specific APIs or
/// credentials.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryProvider {
    /// A Git repository (hosted anywhere).
    #[default]
    Git,
    /// A provider not modelled explicitly.
    Other(String),
}

impl fmt::Display for RepositoryProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git => f.write_str("git"),
            Self::Other(name) => f.write_str(name),
        }
    }
}

/// A source-control repository used by a service or library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    /// The clone URL. Credentials must never be embedded here.
    pub url: String,
    /// The location of the checkout relative to the workspace root.
    pub local_path: RelativePath,
    /// The branch checked out when no other branch is requested.
    pub default_branch: Branch,
    /// The hosting provider, if known.
    pub provider: RepositoryProvider,
}

impl Repository {
    /// Constructs a repository description.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the URL is empty or contains
    /// embedded credentials, or if the default branch is invalid.
    pub fn new(
        url: impl Into<String>,
        local_path: RelativePath,
        default_branch: Branch,
    ) -> Result<Self, DomainError> {
        let url = url.into();
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(DomainError::validation(
                "repository",
                "url must not be empty",
            ));
        }
        if url_contains_credentials(trimmed) {
            return Err(DomainError::validation(
                "repository",
                "url must not embed credentials; use environment-provided authentication",
            ));
        }
        Ok(Self {
            url: trimmed.to_owned(),
            local_path,
            default_branch,
            provider: RepositoryProvider::Git,
        })
    }

    /// Sets the hosting provider.
    #[must_use]
    pub fn with_provider(mut self, provider: RepositoryProvider) -> Self {
        self.provider = provider;
        self
    }
}

fn url_contains_credentials(url: &str) -> bool {
    // Detect `scheme://user:password@host` and `scheme://token@host`.
    let Some((_, rest)) = url.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    authority.contains('@')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> RelativePath {
        RelativePath::new("backend/auth-service").unwrap()
    }

    #[test]
    fn accepts_typical_branches() {
        assert_eq!(Branch::new("master").unwrap().as_str(), "master");
        assert!(Branch::new("release/1.2").is_ok());
        assert!(Branch::new("feature/ABC-1").is_ok());
    }

    #[test]
    fn rejects_unsafe_branches() {
        assert!(Branch::new("").is_err());
        assert!(Branch::new("-Doption").is_err());
        assert!(Branch::new("has space").is_err());
    }

    #[test]
    fn accepts_a_repository_without_credentials() {
        let repository = Repository::new(
            "git@github.com:acme/auth-service.git",
            path(),
            Branch::new("master").unwrap(),
        )
        .unwrap();
        assert_eq!(repository.provider, RepositoryProvider::Git);
        assert_eq!(repository.default_branch.as_str(), "master");
    }

    #[test]
    fn rejects_urls_with_embedded_credentials() {
        assert!(Repository::new(
            "https://user:token@github.com/acme/auth-service.git",
            path(),
            Branch::new("main").unwrap(),
        )
        .is_err());
        assert!(Repository::new(
            "https://ghp_token@github.com/acme/auth-service.git",
            path(),
            Branch::new("main").unwrap(),
        )
        .is_err());
    }

    #[test]
    fn rejects_empty_url() {
        assert!(Repository::new("  ", path(), Branch::new("main").unwrap()).is_err());
    }
}
