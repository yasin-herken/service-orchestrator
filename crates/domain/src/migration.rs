//! Database-migration vocabulary.
//!
//! The domain names *which* migration provider or tool an entity uses, *which*
//! migration operation is requested, and *which* environment a target database
//! represents. It deliberately says nothing about *how* a migration runs:
//! executables, Maven goals, CLI arguments, output parsing, and process
//! execution belong to the infrastructure `liquibase` adapter.
//!
//! Keeping this vocabulary in the domain (rather than in the adapter or in
//! configuration) is what lets the application, the migration adapter, and the
//! interfaces agree on a closed set of providers, operations, and target
//! environments while remaining provider-agnostic. Liquibase is the initial
//! provider, but nothing in the domain or the application is coupled to it:
//! adding Flyway is a new enum variant plus a new provider, not a change to
//! orchestration (`tasks/011.md` §3, §4, §5).
//!
//! # Why target environment is a first-class concept
//!
//! Applying migrations is a database mutation, and a migration aimed at a
//! production database is categorically more dangerous than one aimed at a
//! local container. The domain therefore models the *declared* environment of a
//! migration target so the application and policy layers can classify risk
//! without guessing. The environment is never inferred from a hostname
//! (`tasks/011.md` §10): an unrecognized or absent declaration is
//! [`MigrationTargetEnvironment::Unknown`], which is treated as non-local.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::task::TaskKind;

/// A database-migration provider (tooling).
///
/// The set is open at the edges: [`MigrationProviderType::Custom`] carries a
/// project-specific provider name so configuration can name a tool the domain
/// does not model yet. A provider that is named but not registered is rejected
/// with a structured `MigrationProviderNotFound` error at execution time rather
/// than silently ignored (`tasks/011.md` §5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationProviderType {
    /// Liquibase (the initial concrete provider).
    Liquibase,
    /// Flyway. Planned; not implemented in this task.
    Flyway,
    /// A project-specific provider, for example `custom:dbmate`.
    Custom(String),
}

impl MigrationProviderType {
    /// Returns the canonical, machine-readable label.
    ///
    /// The label is the spelling used in configuration files and in task
    /// parameters, so it is stable and round-trips through
    /// [`MigrationProviderType::parse`].
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Liquibase => "liquibase",
            Self::Flyway => "flyway",
            Self::Custom(name) => name,
        }
    }

    /// Parses a provider label.
    ///
    /// Any non-empty, unrecognized value becomes a [`Self::Custom`] provider so
    /// configuration remains forward-compatible; whether a provider is actually
    /// usable is decided by the provider registry, not by parsing.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        match value {
            "liquibase" => Some(Self::Liquibase),
            "flyway" => Some(Self::Flyway),
            other => Some(Self::Custom(other.to_owned())),
        }
    }

    /// Returns `true` when this is a provider the domain models explicitly.
    #[must_use]
    pub fn is_builtin(&self) -> bool {
        !matches!(self, Self::Custom(_))
    }
}

impl fmt::Display for MigrationProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A provider-neutral migration operation.
///
/// These operations have genuinely shared *semantics* across providers, but not
/// every provider supports every operation. A provider declares the subset it
/// supports and an unsupported request is rejected with a structured error
/// (`tasks/011.md` §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationOperation {
    /// Validate the migration configuration without touching the database.
    Validate,
    /// Inspect migration status where the provider supports it.
    Status,
    /// Inspect pending changesets where the provider supports it.
    Pending,
    /// Generate SQL for a planned update without applying it.
    Preview,
    /// Apply pending migrations.
    Apply,
}

impl MigrationOperation {
    /// Returns the canonical, machine-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Status => "status",
            Self::Pending => "pending",
            Self::Preview => "preview",
            Self::Apply => "apply",
        }
    }

    /// Parses an operation label.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "validate" => Some(Self::Validate),
            "status" => Some(Self::Status),
            "pending" => Some(Self::Pending),
            "preview" => Some(Self::Preview),
            "apply" => Some(Self::Apply),
            _ => None,
        }
    }

    /// Returns the operation implied by a task kind, if any.
    ///
    /// The legacy [`TaskKind::Liquibase`] variant is retained as an alias for
    /// [`MigrationOperation::Apply`] so workflows and callers written before the
    /// migration manager existed keep working (`tasks/011.md` §8).
    #[must_use]
    pub const fn from_task_kind(kind: &TaskKind) -> Option<Self> {
        match kind {
            TaskKind::MigrationValidate => Some(Self::Validate),
            TaskKind::MigrationStatus => Some(Self::Status),
            TaskKind::MigrationPending => Some(Self::Pending),
            TaskKind::MigrationPreview => Some(Self::Preview),
            TaskKind::MigrationApply | TaskKind::Liquibase => Some(Self::Apply),
            _ => None,
        }
    }

    /// Returns `true` when the operation may modify the database.
    ///
    /// Only [`MigrationOperation::Apply`] mutates; every other operation is a
    /// read-only inspection and needs no confirmation (`tasks/011.md` §27).
    #[must_use]
    pub const fn is_mutating(self) -> bool {
        matches!(self, Self::Apply)
    }
}

impl fmt::Display for MigrationOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// How Liquibase is invoked for a service.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum LiquibaseExecutionMode {
    /// Through the Maven Liquibase plugin, resolved with the Maven Wrapper or a
    /// system-wide Maven. This is the default for the current backend services.
    #[default]
    Maven,
    /// Through a Liquibase CLI executable.
    Cli,
}

impl LiquibaseExecutionMode {
    /// Returns the canonical, machine-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Maven => "maven",
            Self::Cli => "cli",
        }
    }

    /// Parses an execution-mode label.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "maven" => Some(Self::Maven),
            "cli" => Some(Self::Cli),
            _ => None,
        }
    }
}

impl fmt::Display for LiquibaseExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The declared environment of a migration's target database.
///
/// Only [`Self::Local`] may migrate automatically (`tasks/011.md` §10). Every
/// other value — including [`Self::Unknown`], the safe default when nothing is
/// declared — requires explicit authorization because it may be shared.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationTargetEnvironment {
    /// An explicitly configured local development database.
    Local,
    /// A shared development database used by more than one developer.
    SharedDevelopment,
    /// A staging (pre-production) database.
    Staging,
    /// A production database.
    Production,
    /// A project-specific environment name.
    Other(String),
    /// No environment was declared or it could not be recognized.
    ///
    /// This is the safe default: an unknown target is treated as non-local and
    /// requires authorization.
    #[default]
    Unknown,
}

impl MigrationTargetEnvironment {
    /// Returns the canonical, machine-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Local => "local",
            Self::SharedDevelopment => "shared_development",
            Self::Staging => "staging",
            Self::Production => "production",
            Self::Other(name) => name,
            Self::Unknown => "unknown",
        }
    }

    /// Parses an environment label.
    ///
    /// Returns [`Self::Unknown`] for an empty value and [`Self::Other`] for any
    /// unrecognized non-empty value, so a declared-but-unmodelled environment is
    /// never silently treated as local.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "local" => Self::Local,
            "shared_development" | "shared-dev" | "shared" => Self::SharedDevelopment,
            "staging" | "stage" => Self::Staging,
            "production" | "prod" => Self::Production,
            "" | "unknown" => Self::Unknown,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Returns `true` when this environment may migrate without explicit
    /// authorization.
    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    /// Returns `true` when migrating this environment requires explicit
    /// authorization.
    #[must_use]
    pub const fn requires_authorization(&self) -> bool {
        !self.is_local()
    }
}

impl fmt::Display for MigrationTargetEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_round_trip_through_their_label() {
        for provider in [
            MigrationProviderType::Liquibase,
            MigrationProviderType::Flyway,
        ] {
            assert_eq!(
                MigrationProviderType::parse(provider.label()),
                Some(provider)
            );
        }
        assert!(MigrationProviderType::Liquibase.is_builtin());
        assert!(!MigrationProviderType::Custom("dbmate".to_owned()).is_builtin());
    }

    #[test]
    fn an_unknown_provider_parses_as_custom() {
        assert_eq!(
            MigrationProviderType::parse("dbmate"),
            Some(MigrationProviderType::Custom("dbmate".to_owned()))
        );
        assert_eq!(MigrationProviderType::parse("   "), None);
    }

    #[test]
    fn operations_round_trip_through_their_label() {
        for operation in [
            MigrationOperation::Validate,
            MigrationOperation::Status,
            MigrationOperation::Pending,
            MigrationOperation::Preview,
            MigrationOperation::Apply,
        ] {
            assert_eq!(
                MigrationOperation::parse(operation.label()),
                Some(operation)
            );
        }
        assert_eq!(MigrationOperation::parse("rollback"), None);
        assert!(MigrationOperation::Apply.is_mutating());
        assert!(!MigrationOperation::Preview.is_mutating());
    }

    #[test]
    fn task_kinds_map_to_operations() {
        assert_eq!(
            MigrationOperation::from_task_kind(&TaskKind::MigrationValidate),
            Some(MigrationOperation::Validate)
        );
        assert_eq!(
            MigrationOperation::from_task_kind(&TaskKind::MigrationStatus),
            Some(MigrationOperation::Status)
        );
        assert_eq!(
            MigrationOperation::from_task_kind(&TaskKind::MigrationPending),
            Some(MigrationOperation::Pending)
        );
        assert_eq!(
            MigrationOperation::from_task_kind(&TaskKind::MigrationPreview),
            Some(MigrationOperation::Preview)
        );
        assert_eq!(
            MigrationOperation::from_task_kind(&TaskKind::MigrationApply),
            Some(MigrationOperation::Apply)
        );
        // The legacy kind is retained as an apply alias.
        assert_eq!(
            MigrationOperation::from_task_kind(&TaskKind::Liquibase),
            Some(MigrationOperation::Apply)
        );
        assert_eq!(
            MigrationOperation::from_task_kind(&TaskKind::BuildService),
            None
        );
    }

    #[test]
    fn execution_modes_round_trip_through_their_label() {
        for mode in [LiquibaseExecutionMode::Maven, LiquibaseExecutionMode::Cli] {
            assert_eq!(LiquibaseExecutionMode::parse(mode.label()), Some(mode));
        }
        assert_eq!(
            LiquibaseExecutionMode::default(),
            LiquibaseExecutionMode::Maven
        );
        assert_eq!(LiquibaseExecutionMode::parse("docker"), None);
    }

    #[test]
    fn only_local_environments_are_authorization_free() {
        assert!(MigrationTargetEnvironment::Local.is_local());
        assert!(!MigrationTargetEnvironment::Local.requires_authorization());

        for environment in [
            MigrationTargetEnvironment::SharedDevelopment,
            MigrationTargetEnvironment::Staging,
            MigrationTargetEnvironment::Production,
            MigrationTargetEnvironment::Other("qa".to_owned()),
            MigrationTargetEnvironment::Unknown,
        ] {
            assert!(!environment.is_local());
            assert!(environment.requires_authorization());
        }
    }

    #[test]
    fn environment_parsing_never_guesses_local() {
        assert_eq!(
            MigrationTargetEnvironment::parse("local"),
            MigrationTargetEnvironment::Local
        );
        assert_eq!(
            MigrationTargetEnvironment::parse(""),
            MigrationTargetEnvironment::Unknown
        );
        assert_eq!(
            MigrationTargetEnvironment::parse("qa"),
            MigrationTargetEnvironment::Other("qa".to_owned())
        );
        // A hostname-like value is not an environment and must not be local.
        let parsed = MigrationTargetEnvironment::parse("localhost");
        assert!(!parsed.is_local());
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(MigrationProviderType::Liquibase.to_string(), "liquibase");
        assert_eq!(MigrationOperation::Apply.to_string(), "apply");
        assert_eq!(LiquibaseExecutionMode::Cli.to_string(), "cli");
        assert_eq!(MigrationTargetEnvironment::Staging.to_string(), "staging");
    }
}
