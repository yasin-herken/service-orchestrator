//! Environment variables for services and commands.
//!
//! The domain distinguishes between ordinary values and secret values. Secrets
//! must never be exposed through the TUI, MCP, logs, task results, or errors
//! (see `AGENTS.md` §31). [`EnvironmentVariable`] therefore renders itself as a
//! redacted placeholder when it is a secret, so simply formatting a service or
//! command cannot leak a credential.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// The placeholder used when rendering a secret value.
pub const REDACTED: &str = "***";

fn validate_key(key: &str) -> Result<(), DomainError> {
    if key.is_empty() {
        return Err(DomainError::validation(
            "environment variable",
            "key must not be empty",
        ));
    }
    let mut chars = key.chars();
    let first = chars.next().expect("key is non-empty");
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(DomainError::validation(
            "environment variable",
            format!("key '{key}' must start with a letter or underscore"),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DomainError::validation(
            "environment variable",
            format!("key '{key}' may only contain letters, digits, and underscore"),
        ));
    }
    Ok(())
}

/// A single environment variable.
///
/// `Serialize` is implemented manually rather than derived: serializing a
/// secret variable emits [`REDACTED`] instead of its value, so that writing a
/// service to configuration output, logs, or an MCP response can never leak a
/// credential (`AGENTS.md` §31). Deserialization reads the value as given, so
/// secrets supplied at runtime still work.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct EnvironmentVariable {
    /// The variable name.
    pub key: String,
    /// The variable value. May be a secret.
    pub value: String,
    /// Whether the value is a secret and must be redacted when displayed.
    pub secret: bool,
}

impl serde::Serialize for EnvironmentVariable {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("EnvironmentVariable", 3)?;
        state.serialize_field("key", &self.key)?;
        if self.secret {
            state.serialize_field("value", REDACTED)?;
        } else {
            state.serialize_field("value", &self.value)?;
        }
        state.serialize_field("secret", &self.secret)?;
        state.end()
    }
}

impl EnvironmentVariable {
    /// Creates a non-secret environment variable.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the key is not a valid variable
    /// name.
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Result<Self, DomainError> {
        Self::with_secrecy(key, value, false)
    }

    /// Creates a secret environment variable.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the key is not a valid variable
    /// name.
    pub fn secret(key: impl Into<String>, value: impl Into<String>) -> Result<Self, DomainError> {
        Self::with_secrecy(key, value, true)
    }

    /// Creates an environment variable with explicit secrecy control.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the key is not a valid variable
    /// name.
    pub fn with_secrecy(
        key: impl Into<String>,
        value: impl Into<String>,
        secret: bool,
    ) -> Result<Self, DomainError> {
        let key = key.into();
        validate_key(&key)?;
        Ok(Self {
            key,
            value: value.into(),
            secret,
        })
    }
}

impl fmt::Display for EnvironmentVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.secret {
            write!(f, "{}={REDACTED}", self.key)
        } else {
            write!(f, "{}={}", self.key, self.value)
        }
    }
}

/// An ordered set of environment variables keyed by variable name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Environment {
    variables: BTreeMap<String, EnvironmentVariable>,
}

impl Environment {
    /// Creates an empty environment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces a variable, returning the previous value.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the variable is invalid.
    pub fn set(
        &mut self,
        variable: EnvironmentVariable,
    ) -> Result<Option<EnvironmentVariable>, DomainError> {
        validate_key(&variable.key)?;
        Ok(self.variables.insert(variable.key.clone(), variable))
    }

    /// Returns the variable with the given key, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&EnvironmentVariable> {
        self.variables.get(key)
    }

    /// Returns `true` if a variable with the given key exists.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.variables.contains_key(key)
    }

    /// Returns the number of variables.
    #[must_use]
    pub fn len(&self) -> usize {
        self.variables.len()
    }

    /// Returns `true` if there are no variables.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    /// Iterates over the variables in key order.
    pub fn iter(&self) -> impl Iterator<Item = &EnvironmentVariable> {
        self.variables.values()
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered: Vec<String> = self.variables.values().map(ToString::to_string).collect();
        f.write_str(&rendered.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_values_when_displayed() {
        let variable = EnvironmentVariable::secret("DB_PASSWORD", "hunter2").unwrap();
        let rendered = variable.to_string();
        assert_eq!(rendered, "DB_PASSWORD=***");
        assert!(!rendered.contains("hunter2"));
    }

    #[test]
    fn renders_plain_values_in_full() {
        let variable = EnvironmentVariable::new("APP_ENV", "local").unwrap();
        assert_eq!(variable.to_string(), "APP_ENV=local");
    }

    #[test]
    fn rejects_invalid_keys() {
        assert!(EnvironmentVariable::new("1BAD", "x").is_err());
        assert!(EnvironmentVariable::new("BAD-KEY", "x").is_err());
        assert!(EnvironmentVariable::new("", "x").is_err());
    }

    #[test]
    fn environment_redacts_all_secrets_when_displayed() {
        let mut environment = Environment::new();
        environment
            .set(EnvironmentVariable::new("APP_ENV", "local").unwrap())
            .unwrap();
        environment
            .set(EnvironmentVariable::secret("TOKEN", "abc123").unwrap())
            .unwrap();
        let rendered = environment.to_string();
        assert!(rendered.contains("APP_ENV=local"));
        assert!(rendered.contains("TOKEN=***"));
        assert!(!rendered.contains("abc123"));
    }

    #[test]
    fn set_replaces_existing_variable() {
        let mut environment = Environment::new();
        environment
            .set(EnvironmentVariable::new("APP_ENV", "local").unwrap())
            .unwrap();
        let previous = environment
            .set(EnvironmentVariable::new("APP_ENV", "staging").unwrap())
            .unwrap();
        assert_eq!(previous.unwrap().value, "local");
        assert_eq!(environment.get("APP_ENV").unwrap().value, "staging");
        assert_eq!(environment.len(), 1);
    }
}
