//! Configuration schema migration.
//!
//! Only version 1 exists today, so migration is the identity function. The
//! module exists so that when version 2 arrives the upgrade path has an obvious
//! home and the loader does not need to change shape:
//!
//! ```text
//! value  ──upgrade_to(2)──> value  ──upgrade_to(3)──> value
//! ```
//!
//! Each future `upgrade_to_n` function will take the TOML value for version
//! `n - 1`, produce the shape expected by version `n`, and rewrite
//! `config_version`. Migrations must be pure and deterministic.

use crate::error::ConfigError;
use crate::model::SUPPORTED_CONFIG_VERSION;

/// Upgrades `value` from `from` to [`SUPPORTED_CONFIG_VERSION`].
///
/// # Errors
///
/// Returns [`ConfigError::UnsupportedVersion`] if `from` is newer than the
/// supported version. Older versions are upgraded step by step.
pub(crate) fn migrate(
    value: toml::Value,
    from: u32,
    path: &str,
) -> Result<toml::Value, ConfigError> {
    if from == SUPPORTED_CONFIG_VERSION {
        return Ok(value);
    }
    if from > SUPPORTED_CONFIG_VERSION {
        return Err(ConfigError::UnsupportedVersion {
            path: path.to_owned(),
            found: i64::from(from),
            supported: SUPPORTED_CONFIG_VERSION,
        });
    }

    // No older versions exist yet. When version 2 is introduced, this becomes:
    //
    //   let mut value = value;
    //   if from < 1 { value = upgrade_to_1(value)?; }
    //   if from < 2 { value = upgrade_to_2(value)?; }
    //
    // and the guard below disappears.
    let _ = value;
    Err(ConfigError::UnsupportedVersion {
        path: path.to_owned(),
        found: i64::from(from),
        supported: SUPPORTED_CONFIG_VERSION,
    })
}
