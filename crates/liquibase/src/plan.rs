//! Structured migration plans.
//!
//! A [`MigrationPlan`] is inert data: a program, an argument vector, a working
//! directory, the environment to apply, an optional timeout, and the
//! classification of the target. A provider constructs one and the process
//! manager executes it; nothing in this module spawns a process or schedules a
//! task (`AGENTS.md` §9, §21, `tasks/011.md` §3).
//!
//! # Capabilities
//!
//! [`MigrationCapabilities`] declares which provider-neutral operations a
//! provider supports. Liquibase supports all of them; a future provider need
//! not. An unsupported request is rejected with a structured error rather than
//! silently ignored (`tasks/011.md` §7).
//!
//! # Debug is redacted
//!
//! The plan holds the environment *values* that will be applied to the child
//! process, which may include secrets. Its [`Debug`] implementation prints only
//! display-safe fields (`AGENTS.md` §31, `tasks/011.md` §11).

use std::fmt;
use std::time::Duration;

use service_orchestrator_domain::{
    AbsolutePath, Environment, MigrationOperation, MigrationProviderType,
};

/// The provider-neutral operations a migration provider supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MigrationCapabilities {
    /// Whether configuration validation is supported.
    pub validate: bool,
    /// Whether status inspection is supported.
    pub status: bool,
    /// Whether pending-changeset inspection is supported.
    pub pending: bool,
    /// Whether SQL preview generation is supported.
    pub preview: bool,
    /// Whether applying migrations is supported.
    pub apply: bool,
}

impl MigrationCapabilities {
    /// All operations supported.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            validate: true,
            status: true,
            pending: true,
            preview: true,
            apply: true,
        }
    }

    /// No operations supported.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            validate: false,
            status: false,
            pending: false,
            preview: false,
            apply: false,
        }
    }

    /// Returns `true` when `operation` is supported.
    #[must_use]
    pub const fn supports(&self, operation: MigrationOperation) -> bool {
        match operation {
            MigrationOperation::Validate => self.validate,
            MigrationOperation::Status => self.status,
            MigrationOperation::Pending => self.pending,
            MigrationOperation::Preview => self.preview,
            MigrationOperation::Apply => self.apply,
        }
    }
}

/// The risk classification of a planned migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MigrationSafety {
    /// A read-only inspection; no database mutation.
    ReadOnly,
    /// A mutation of an explicitly configured local database.
    LocalWrite,
    /// A mutation of a non-local database the caller authorized.
    AuthorizedWrite,
}

impl MigrationSafety {
    /// Returns a short, stable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::LocalWrite => "local_write",
            Self::AuthorizedWrite => "authorized_write",
        }
    }

    /// Returns `true` when the plan mutates a database.
    #[must_use]
    pub const fn is_write(self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

impl fmt::Display for MigrationSafety {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A structured, provider-neutral description of one migration invocation.
#[derive(Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    /// The executable to run, for example `mvn`, `./mvnw`, or `liquibase`.
    pub program: String,
    /// The arguments passed to the executable, in order.
    pub args: Vec<String>,
    /// The directory the migration runs in.
    pub working_dir: AbsolutePath,
    /// The environment variables applied to the child process.
    ///
    /// Values may be secrets; they are never rendered by this type's [`Debug`].
    pub environment: Environment,
    /// An optional maximum runtime declared by the provider.
    pub timeout: Option<Duration>,
    /// The provider the plan targets.
    pub provider: MigrationProviderType,
    /// The provider-neutral operation the plan performs.
    pub operation: MigrationOperation,
    /// A display-safe identity of the target database.
    pub database_identity: String,
    /// The stable key identifying the physical database target.
    pub target_key: String,
    /// The risk classification of the plan.
    pub safety: MigrationSafety,
}

impl MigrationPlan {
    /// Creates a plan for `provider`/`operation` running `program`.
    #[must_use]
    pub fn new(
        provider: MigrationProviderType,
        operation: MigrationOperation,
        program: impl Into<String>,
        working_dir: AbsolutePath,
    ) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            working_dir,
            environment: Environment::new(),
            timeout: None,
            provider,
            operation,
            database_identity: String::new(),
            target_key: String::new(),
            safety: MigrationSafety::ReadOnly,
        }
    }

    /// Replaces the argument vector.
    #[must_use]
    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Replaces the environment applied to the process.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// Sets the declared maximum runtime.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the display-safe target identity and its coordination key.
    #[must_use]
    pub fn with_target(mut self, identity: impl Into<String>, key: impl Into<String>) -> Self {
        self.database_identity = identity.into();
        self.target_key = key.into();
        self
    }

    /// Sets the risk classification.
    #[must_use]
    pub fn with_safety(mut self, safety: MigrationSafety) -> Self {
        self.safety = safety;
        self
    }

    /// Returns a redacted, human-readable rendering of the command.
    ///
    /// Environment values are never included.
    #[must_use]
    pub fn command(&self) -> String {
        let mut rendered = self.program.clone();
        for arg in &self.args {
            rendered.push(' ');
            rendered.push_str(arg);
        }
        rendered
    }
}

impl fmt::Debug for MigrationPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MigrationPlan")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("working_dir", &self.working_dir)
            .field("environment_variables", &self.environment.len())
            .field("provider", &self.provider)
            .field("operation", &self.operation)
            .field("database_identity", &self.database_identity)
            .field("safety", &self.safety)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::EnvironmentVariable;

    #[test]
    fn capabilities_report_supported_operations() {
        let all = MigrationCapabilities::all();
        for operation in [
            MigrationOperation::Validate,
            MigrationOperation::Status,
            MigrationOperation::Pending,
            MigrationOperation::Preview,
            MigrationOperation::Apply,
        ] {
            assert!(all.supports(operation));
        }
        let none = MigrationCapabilities::none();
        assert!(!none.supports(MigrationOperation::Validate));
        assert!(!none.supports(MigrationOperation::Apply));
    }

    #[test]
    fn safety_labels_are_stable() {
        assert_eq!(MigrationSafety::ReadOnly.label(), "read_only");
        assert_eq!(MigrationSafety::LocalWrite.label(), "local_write");
        assert_eq!(MigrationSafety::AuthorizedWrite.label(), "authorized_write");
        assert!(!MigrationSafety::ReadOnly.is_write());
        assert!(MigrationSafety::LocalWrite.is_write());
        assert!(MigrationSafety::AuthorizedWrite.is_write());
    }

    #[test]
    fn renders_the_command_without_environment_values() {
        let plan = MigrationPlan::new(
            MigrationProviderType::Liquibase,
            MigrationOperation::Apply,
            "mvn",
            AbsolutePath::new("/w/auth-service").unwrap(),
        )
        .with_args(["-B", "liquibase:update"]);
        assert_eq!(plan.command(), "mvn -B liquibase:update");
    }

    #[test]
    fn debug_output_never_contains_a_secret() {
        let mut environment = Environment::new();
        environment
            .set(EnvironmentVariable::secret("DB_PASSWORD", "hunter2").unwrap())
            .unwrap();
        let plan = MigrationPlan::new(
            MigrationProviderType::Liquibase,
            MigrationOperation::Apply,
            "mvn",
            AbsolutePath::new("/w/auth-service").unwrap(),
        )
        .with_environment(environment);
        let rendered = format!("{plan:?}");
        assert!(!rendered.contains("hunter2"));
        assert!(rendered.contains("environment_variables"));
    }
}
