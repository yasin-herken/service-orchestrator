//! # Config
//!
//! Configuration loading, parsing, validation, versioning, and migration.
//!
//! Configuration is external and versioned; services are never hard-coded in
//! source. This crate is infrastructure: it produces domain configuration
//! values and depends only on the domain. It performs no orchestration and no
//! runtime execution (no Git, Maven, npm, Liquibase, or process spawning).
//!
//! # Pipeline
//!
//! ```text
//! config.toml
//!     │  read
//!     ▼
//! TOML value  ──  version gate  ──  (future) migrations
//!     │  deserialize
//!     ▼
//! raw configuration model        (`raw`)
//!     │  validate + normalize
//!     ▼
//! validated configuration        ([`Config`])
//!     │  build
//!     ▼
//! domain `Workspace`             (`service-orchestrator-domain`)
//! ```
//!
//! TOML-specific details live only in the [`raw`] module and the loader. The
//! rest of the application sees [`Config`], which is composed of domain
//! [`Workspace`](service_orchestrator_domain::Workspace) aggregates and a small
//! number of configuration-only values (execution settings, workspace
//! environment, Liquibase settings) that have no domain representation yet.
//!
//! # Design notes
//!
//! - **Versioned.** Every file declares `config_version`. Unsupported versions
//!   are rejected with an actionable error; a migration hook exists for future
//!   schema changes.
//! - **Strict.** Unknown TOML fields are rejected (`deny_unknown_fields`) so a
//!   typo such as `max_paralel_tasks` fails loudly instead of being ignored.
//! - **Actionable errors.** Validation failures carry a file, a location, the
//!   problem, the expectation, and a suggested fix.
//! - **No filesystem mutation.** Paths are normalized lexically; nothing is
//!   created, deleted, or resolved against the real filesystem.
//!
//! See `docs/configuration.md` and `docs/adr/003-configuration-system.md`.

mod error;
mod loader;
mod migration;
mod model;
mod normalize;
mod raw;
mod validation;

pub use error::{ConfigError, ValidationError, ValidationErrors};
pub use loader::{
    default_config_dir, default_config_file, load_default, load_from_path, load_from_str,
    ConfigLoader, TomlFileLoader,
};
pub use model::{
    Config, ConfiguredWorkspace, ExecutionSettings, LiquibaseConfig, DEFAULT_CONFIG_VERSION,
    DEFAULT_MAX_PARALLEL_TASKS, DEFAULT_TIMEOUT_SECONDS, SUPPORTED_CONFIG_VERSION,
};
