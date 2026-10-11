//! Strongly typed process identifiers.
//!
//! A [`ProcessId`] identifies one managed process for its whole lifetime, from
//! `Created` through whatever terminal state it reaches. It is opaque, stable,
//! and human-readable (`proc-0`, `proc-1`, ...); it carries no structure and is
//! never an array index or a timestamp.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::ProcessError;

/// The maximum length, in characters, of a process identifier.
pub const MAX_PROCESS_ID_LEN: usize = 128;

fn validate(value: &str) -> Result<String, ProcessError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ProcessError::invalid_spec("process id must not be empty"));
    }
    if trimmed.chars().count() > MAX_PROCESS_ID_LEN {
        return Err(ProcessError::invalid_spec(format!(
            "process id must not exceed {MAX_PROCESS_ID_LEN} characters"
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(ProcessError::invalid_spec(
            "process id may only contain ASCII letters, digits, '-', '_', '.', and ':'",
        ));
    }
    Ok(trimmed.to_owned())
}

/// Identifies a single managed process.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProcessId(String);

impl ProcessId {
    /// Validates `value` and constructs the identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidSpec`] if the value is empty, too long, or
    /// contains disallowed characters.
    pub fn new(value: impl Into<String>) -> Result<Self, ProcessError> {
        Ok(Self(validate(&value.into())?))
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ProcessId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for ProcessId {
    type Err = ProcessError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_generated_and_human_identifiers() {
        assert_eq!(ProcessId::new("proc-0").unwrap().as_str(), "proc-0");
        assert!(ProcessId::new("auth-service.process:1").is_ok());
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(ProcessId::new("  proc-7 \n").unwrap().as_str(), "proc-7");
    }

    #[test]
    fn rejects_invalid_identifiers() {
        assert!(ProcessId::new("").is_err());
        assert!(ProcessId::new("   ").is_err());
        assert!(ProcessId::new("proc 1").is_err());
        assert!(ProcessId::new("proc;rm -rf").is_err());
        assert!(ProcessId::new("a".repeat(MAX_PROCESS_ID_LEN + 1)).is_err());
    }
}
