//! Configuration loading.
//!
//! The loading boundary is deliberately small. [`ConfigLoader`] is the single
//! trait the application depends on; [`TomlFileLoader`] is the initial
//! implementation. Future sources (environment variables, CLI overrides,
//! team-shared or remote configuration) would implement the same trait and be
//! composed here, but none are implemented yet.
//!
//! The pipeline is:
//!
//! 1. read the file;
//! 2. parse it into a `toml::Value` so the schema version can be inspected and
//!    migrated before structural deserialization;
//! 3. deserialize the [`RawConfig`] model (rejecting unknown fields);
//! 4. normalize and validate into [`Config`].

use std::path::{Path, PathBuf};

use crate::error::{ConfigError, ValidationError, ValidationErrors};
use crate::migration;
use crate::model::{Config, DEFAULT_CONFIG_VERSION, SUPPORTED_CONFIG_VERSION};
use crate::normalize;
use crate::raw::RawConfig;

/// Loads configuration from some source into a validated [`Config`].
pub trait ConfigLoader {
    /// Loads and validates configuration from `path`.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] if the file cannot be read, cannot be parsed,
    /// declares an unsupported version, or fails validation.
    fn load(&self, path: &Path) -> Result<Config, ConfigError>;
}

/// The default TOML file loader.
///
/// TOML parsing is contained to this crate; no other layer reads configuration
/// files directly.
#[derive(Debug, Default, Clone, Copy)]
pub struct TomlFileLoader;

impl ConfigLoader for TomlFileLoader {
    fn load(&self, path: &Path) -> Result<Config, ConfigError> {
        let origin = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: origin.clone(),
            source,
        })?;
        load_from_str(&text, &origin)
    }
}

/// Loads configuration from a file path using [`TomlFileLoader`].
///
/// # Errors
///
/// Returns a [`ConfigError`] as described by [`ConfigLoader::load`].
pub fn load_from_path(path: &Path) -> Result<Config, ConfigError> {
    TomlFileLoader.load(path)
}

/// Loads configuration from the default location, `~/.service-orchestrator/config.toml`.
///
/// # Errors
///
/// Returns a [`ConfigError`] as described by [`ConfigLoader::load`].
pub fn load_default() -> Result<Config, ConfigError> {
    load_from_path(&default_config_file())
}

/// Loads and validates configuration from an in-memory TOML string.
///
/// `origin` names the source in error messages (for example a file path or
/// `"<memory>"`). This is primarily useful to interface adapters and tests that
/// need to validate a document without touching the filesystem.
///
/// # Errors
///
/// Returns a [`ConfigError`] if the document cannot be parsed, declares an
/// unsupported version, or fails validation.
pub fn load_from_str(text: &str, origin: &str) -> Result<Config, ConfigError> {
    let value: toml::Value = toml::from_str(text).map_err(|error| ConfigError::Parse {
        path: origin.to_owned(),
        message: error.to_string(),
    })?;

    let version = read_version(&value, origin)?;
    let value = migration::migrate(value, version, origin)?;
    let raw: RawConfig = value
        .try_into()
        .map_err(|error: toml::de::Error| ConfigError::Parse {
            path: origin.to_owned(),
            message: error.to_string(),
        })?;

    normalize::normalize(raw, origin)
}

/// Reads the declared schema version, defaulting to [`DEFAULT_CONFIG_VERSION`].
fn read_version(value: &toml::Value, origin: &str) -> Result<u32, ConfigError> {
    match value.get("config_version") {
        None => Ok(DEFAULT_CONFIG_VERSION),
        Some(toml::Value::Integer(found)) => {
            if *found == i64::from(SUPPORTED_CONFIG_VERSION) {
                Ok(SUPPORTED_CONFIG_VERSION)
            } else {
                Err(ConfigError::UnsupportedVersion {
                    path: origin.to_owned(),
                    found: *found,
                    supported: SUPPORTED_CONFIG_VERSION,
                })
            }
        }
        Some(other) => Err(ConfigError::Validation(ValidationErrors::single(
            ValidationError::new(
                "config_version",
                format!(
                    "config_version must be an integer, found {}",
                    other.type_str()
                ),
            )
            .expected(format!("the integer {SUPPORTED_CONFIG_VERSION}")),
        ))),
    }
}

/// The default configuration directory, `~/.service-orchestrator`.
///
/// Falls back to `.service-orchestrator` in the current directory when `HOME`
/// is not set, which keeps the function total without inventing a home.
#[must_use]
pub fn default_config_dir() -> PathBuf {
    home_dir().join(".service-orchestrator")
}

/// The default configuration file, `~/.service-orchestrator/config.toml`.
#[must_use]
pub fn default_config_file() -> PathBuf {
    default_config_dir().join("config.toml")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
