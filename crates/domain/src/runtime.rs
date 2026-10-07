//! Runtime requirements.
//!
//! A [`Runtime`] describes *what a service or library needs* (for example Java
//! 21 or Node 20.19.4). It deliberately says nothing about *how* that runtime is
//! discovered, installed, or executed; environment discovery and executable
//! resolution belong to the infrastructure layer.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// A version string such as `21`, `20.19.4`, or `1.8.0-SNAPSHOT`.
///
/// Versions are kept opaque. The domain does not attempt to parse or compare
/// them semantically; that is the responsibility of the build and configuration
/// layers, which know the conventions of each ecosystem.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Version(String);

impl Version {
    /// Validates `value` and constructs a version.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the value is empty or contains
    /// characters outside ASCII letters, digits, `.`, `_`, `-`, and `+`.
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DomainError::validation("version", "must not be empty"));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
        {
            return Err(DomainError::validation(
                "version",
                "may only contain ASCII letters, digits, '.', '_', '-', and '+'",
            ));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the version as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The category of a runtime requirement.
///
/// The `Other` variant keeps the model open to future runtimes (for example
/// Python, Go, or a container toolchain) without changing the rest of the
/// domain.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// A Java virtual machine.
    Java,
    /// A Node.js runtime.
    Node,
    /// The Maven build tool.
    Maven,
    /// The npm package manager.
    Npm,
    /// Any runtime not modelled explicitly.
    Other(String),
}

impl RuntimeKind {
    /// Returns a short, human-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Java => "java",
            Self::Node => "node",
            Self::Maven => "maven",
            Self::Npm => "npm",
            Self::Other(name) => name,
        }
    }
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A runtime requirement: a kind plus an optional version.
///
/// Example requirements:
///
/// ```text
/// Java 21
/// Node 20.19.4
/// Maven (any version)
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Runtime {
    /// The kind of runtime required.
    pub kind: RuntimeKind,
    /// The required version, if the service pins one.
    pub version: Option<Version>,
}

impl Runtime {
    /// Creates a runtime requirement without a pinned version.
    #[must_use]
    pub fn any(kind: RuntimeKind) -> Self {
        Self {
            kind,
            version: None,
        }
    }

    /// Creates a runtime requirement with a pinned version.
    #[must_use]
    pub fn versioned(kind: RuntimeKind, version: Version) -> Self {
        Self {
            kind,
            version: Some(version),
        }
    }

    /// Returns `true` if `self` requires the same runtime kind as `other`.
    #[must_use]
    pub fn is_kind(&self, kind: &RuntimeKind) -> bool {
        &self.kind == kind
    }
}

impl fmt::Display for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.version {
            Some(version) => write!(f, "{} {}", self.kind, version),
            None => write!(f, "{}", self.kind),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_versions() {
        assert!(Version::new("21").is_ok());
        assert!(Version::new("20.19.4").is_ok());
        assert!(Version::new("1.8.0-SNAPSHOT").is_ok());
    }

    #[test]
    fn rejects_empty_or_invalid_versions() {
        assert!(Version::new("   ").is_err());
        assert!(Version::new("21 rc1").is_err());
    }

    #[test]
    fn distinguishes_runtime_kinds() {
        let java = Runtime::versioned(RuntimeKind::Java, Version::new("21").unwrap());
        assert!(java.is_kind(&RuntimeKind::Java));
        assert!(!java.is_kind(&RuntimeKind::Node));
        assert_eq!(java.to_string(), "java 21");
    }

    #[test]
    fn supports_open_ended_runtimes() {
        let maven = Runtime::any(RuntimeKind::Maven);
        assert_eq!(maven.to_string(), "maven");
        let other = Runtime::any(RuntimeKind::Other("python".to_owned()));
        assert_eq!(other.kind.label(), "python");
    }
}
