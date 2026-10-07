//! Configuration error model.
//!
//! Configuration failures are split into three actionable categories:
//!
//! - [`ConfigError::Read`] — the file could not be read;
//! - [`ConfigError::Parse`] — the file was read but is not valid TOML or does
//!   not match the raw schema (including unknown fields);
//! - [`ConfigError::Validation`] — the file parsed but is semantically invalid.
//!
//! Validation errors are deliberately rich. A developer staring at
//! `~/.service-orchestrator/config.toml` needs to know *which* field is wrong
//! and *how* to fix it, so every [`ValidationError`] carries the file, a
//! location such as `services[2].runtime.version`, the problem, the
//! expectation, and a suggested fix.

use std::fmt;

use thiserror::Error;

/// A single, actionable configuration problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// The configuration file the problem was found in, if known.
    pub file: Option<String>,
    /// A dotted/indexed location such as `services[2].runtime.version`.
    pub location: String,
    /// What is wrong.
    pub problem: String,
    /// What a valid value would look like, if that is expressible.
    pub expected: Option<String>,
    /// A concrete change that would resolve the problem, if known.
    pub suggested_fix: Option<String>,
}

impl ValidationError {
    /// Creates a validation error for `location` with `problem`.
    #[must_use]
    pub fn new(location: impl Into<String>, problem: impl Into<String>) -> Self {
        Self {
            file: None,
            location: location.into(),
            problem: problem.into(),
            expected: None,
            suggested_fix: None,
        }
    }

    /// Attaches the expected shape of a valid value.
    #[must_use]
    pub fn expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    /// Attaches a suggested fix.
    #[must_use]
    pub fn suggested_fix(mut self, fix: impl Into<String>) -> Self {
        self.suggested_fix = Some(fix.into());
        self
    }

    /// Attaches the originating file.
    #[must_use]
    pub fn in_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Location:")?;
        writeln!(f, "{}", self.location)?;
        writeln!(f)?;
        writeln!(f, "Problem:")?;
        writeln!(f, "{}", self.problem)?;
        if let Some(expected) = &self.expected {
            writeln!(f)?;
            writeln!(f, "Expected:")?;
            writeln!(f, "{expected}")?;
        }
        if let Some(fix) = &self.suggested_fix {
            writeln!(f)?;
            writeln!(f, "Suggested fix:")?;
            write!(f, "{fix}")?;
        }
        Ok(())
    }
}

/// A non-empty collection of [`ValidationError`] values for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationErrors {
    file: Option<String>,
    errors: Vec<ValidationError>,
}

impl ValidationErrors {
    /// Creates a collection from one or more errors.
    ///
    /// # Panics
    ///
    /// Panics if `errors` is empty. Configuration code only constructs this
    /// after checking that at least one error was collected.
    #[must_use]
    pub fn new(errors: Vec<ValidationError>) -> Self {
        assert!(
            !errors.is_empty(),
            "ValidationErrors must contain at least one error"
        );
        Self { file: None, errors }
    }

    /// Creates a collection containing a single error.
    #[must_use]
    pub fn single(error: ValidationError) -> Self {
        Self {
            file: error.file.clone(),
            errors: vec![error],
        }
    }

    /// Attaches the originating file to the collection.
    #[must_use]
    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Returns the contained errors.
    #[must_use]
    pub fn errors(&self) -> &[ValidationError] {
        &self.errors
    }

    /// Returns `true` if there are no errors (never true for a constructed
    /// value).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Configuration validation failed.")?;
        if let Some(file) = self
            .file
            .as_ref()
            .or_else(|| self.errors.iter().find_map(|error| error.file.as_ref()))
        {
            writeln!(f)?;
            writeln!(f, "File:")?;
            writeln!(f, "{file}")?;
        }
        for (index, error) in self.errors.iter().enumerate() {
            writeln!(f)?;
            if self.errors.len() > 1 {
                writeln!(f, "({})", index + 1)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

/// Errors produced while loading configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("failed to read configuration file '{path}': {source}")]
    Read {
        /// The file that could not be read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The configuration file is not valid TOML or does not match the schema.
    #[error("failed to parse configuration file '{path}': {message}")]
    Parse {
        /// The file that failed to parse.
        path: String,
        /// The parser message, including line/column information when present.
        message: String,
    },

    /// The file declares a configuration version this build does not support.
    #[error(
        "unsupported configuration version {found} in '{path}'; this build supports version {supported}"
    )]
    UnsupportedVersion {
        /// The file whose version is unsupported.
        path: String,
        /// The version found in the file.
        found: i64,
        /// The version this build supports.
        supported: u32,
    },

    /// The file parsed but is semantically invalid.
    #[error("{0}")]
    Validation(ValidationErrors),
}

impl From<ValidationErrors> for ConfigError {
    fn from(errors: ValidationErrors) -> Self {
        Self::Validation(errors)
    }
}
