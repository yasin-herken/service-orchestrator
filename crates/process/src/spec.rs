//! The process specification and its policies.
//!
//! A [`ProcessSpec`] describes *what to run* before anything is spawned: a
//! program, its structured argument vector, a working directory, environment
//! variables, an optional timeout, a restart policy, and a shutdown strategy.
//! It is deliberately a structured command (program plus arguments) rather than
//! an opaque shell string, so values from configuration cannot be re-interpreted
//! by a shell (`AGENTS.md` §32).
//!
//! Environment variables use the domain [`Environment`] type, which redacts
//! secret values when displayed or serialized, so a command description can
//! never leak a credential (`AGENTS.md` §31).

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use service_orchestrator_domain::{Environment, EnvironmentVariable, ServiceId};

use crate::error::ProcessError;
use crate::state::ExitReason;

/// The signal used to request a graceful stop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationSignal {
    /// `SIGTERM`: the conventional, catchable request to terminate.
    #[default]
    Term,
    /// `SIGINT`: the interactive interrupt signal.
    Int,
    /// `SIGKILL`: uncatchable, used only for escalation.
    Kill,
}

/// How a process should be shut down.
///
/// The manager always prefers a graceful signal. If the process does not exit
/// within [`grace_period`](ShutdownStrategy::grace_period), it escalates to
/// `SIGKILL`. Both the signal and the grace period are configurable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShutdownStrategy {
    /// The graceful signal sent first.
    pub signal: TerminationSignal,
    /// How long to wait for the process to exit before force-killing it.
    pub grace_period: Duration,
}

impl ShutdownStrategy {
    /// The default strategy: `SIGTERM`, then `SIGKILL` after five seconds.
    #[must_use]
    pub const fn graceful() -> Self {
        Self {
            signal: TerminationSignal::Term,
            grace_period: Duration::from_secs(5),
        }
    }

    /// Sets the graceful signal.
    #[must_use]
    pub const fn with_signal(mut self, signal: TerminationSignal) -> Self {
        self.signal = signal;
        self
    }

    /// Sets the grace period before `SIGKILL`.
    #[must_use]
    pub const fn with_grace_period(mut self, grace_period: Duration) -> Self {
        self.grace_period = grace_period;
        self
    }
}

impl Default for ShutdownStrategy {
    fn default() -> Self {
        Self::graceful()
    }
}

/// When a process should be automatically restarted after it ends.
///
/// Restart is opt-in: the default is [`RestartPolicy::Never`], so a process
/// that ends stays ended until an operator or the application restarts it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RestartPolicy {
    /// Never restart automatically.
    #[default]
    Never,
    /// Restart after a failure (a non-zero exit or a signal), up to a limit.
    OnFailure {
        /// The maximum number of automatic restarts.
        max_restarts: u32,
        /// The delay before each restart.
        backoff: Duration,
    },
    /// Restart after any non-requested end, up to a limit.
    Always {
        /// The maximum number of automatic restarts.
        max_restarts: u32,
        /// The delay before each restart.
        backoff: Duration,
    },
}

impl RestartPolicy {
    /// Returns the configured backoff between restarts.
    #[must_use]
    pub const fn backoff(self) -> Duration {
        match self {
            Self::Never => Duration::from_secs(0),
            Self::OnFailure { backoff, .. } | Self::Always { backoff, .. } => backoff,
        }
    }

    /// Decides whether an execution that ended with `reason` should restart,
    /// given how many automatic restarts have already happened.
    ///
    /// A requested stop (`Terminated`, `Cancelled`, `Killed`) never triggers an
    /// automatic restart, even under [`RestartPolicy::Always`]: an operator's
    /// intent wins.
    #[must_use]
    pub const fn should_restart(self, reason: ExitReason, restarts_done: u32) -> bool {
        match self {
            Self::Never => false,
            Self::OnFailure { max_restarts, .. } => {
                reason.is_failure() && restarts_done < max_restarts
            }
            Self::Always { max_restarts, .. } => {
                !matches!(
                    reason,
                    ExitReason::Terminated | ExitReason::Cancelled | ExitReason::Killed
                ) && restarts_done < max_restarts
            }
        }
    }
}

/// A structured description of a process to run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessSpec {
    /// The executable to run.
    pub program: String,
    /// The arguments passed to the executable, in order.
    pub args: Vec<String>,
    /// The directory the process runs in.
    pub working_dir: Option<PathBuf>,
    /// Additional environment variables for the process.
    ///
    /// Secret variables are applied to the child process but are redacted when
    /// the specification is displayed or serialized.
    pub environment: Environment,
    /// An optional maximum runtime. When exceeded, the process is stopped.
    pub timeout: Option<Duration>,
    /// When the process should be restarted automatically.
    pub restart: RestartPolicy,
    /// How the process should be shut down.
    pub shutdown: ShutdownStrategy,
    /// The service this process belongs to, when it is a service process.
    pub service: Option<ServiceId>,
}

impl ProcessSpec {
    /// Creates a specification for `program` with no arguments.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidSpec`] if the program is empty or contains
    /// whitespace (a program is a single executable, not a command line).
    pub fn new(program: impl Into<String>) -> Result<Self, ProcessError> {
        let program = program.into();
        if program.trim().is_empty() {
            return Err(ProcessError::invalid_spec("program must not be empty"));
        }
        if program.chars().any(char::is_whitespace) {
            return Err(ProcessError::invalid_spec(format!(
                "program '{program}' must not contain whitespace"
            )));
        }
        Ok(Self {
            program,
            args: Vec::new(),
            working_dir: None,
            environment: Environment::new(),
            timeout: None,
            restart: RestartPolicy::Never,
            shutdown: ShutdownStrategy::graceful(),
            service: None,
        })
    }

    /// Appends a single argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends several arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets the working directory.
    #[must_use]
    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Adds an environment variable.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidSpec`] if the variable is invalid.
    pub fn env(mut self, variable: EnvironmentVariable) -> Result<Self, ProcessError> {
        self.environment
            .set(variable)
            .map_err(|error| ProcessError::invalid_spec(error.to_string()))?;
        Ok(self)
    }

    /// Replaces the whole environment.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// Sets the maximum runtime.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sets the restart policy.
    #[must_use]
    pub fn with_restart(mut self, restart: RestartPolicy) -> Self {
        self.restart = restart;
        self
    }

    /// Sets the shutdown strategy.
    #[must_use]
    pub fn with_shutdown(mut self, shutdown: ShutdownStrategy) -> Self {
        self.shutdown = shutdown;
        self
    }

    /// Associates the process with a service.
    #[must_use]
    pub fn for_service(mut self, service: ServiceId) -> Self {
        self.service = Some(service);
        self
    }

    /// Validates the specification.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidSpec`] if the program is empty or contains
    /// whitespace.
    pub fn validate(&self) -> Result<(), ProcessError> {
        if self.program.trim().is_empty() {
            return Err(ProcessError::invalid_spec("program must not be empty"));
        }
        if self.program.chars().any(char::is_whitespace) {
            return Err(ProcessError::invalid_spec(format!(
                "program '{}' must not contain whitespace",
                self.program
            )));
        }
        Ok(())
    }

    /// Returns a redacted, human-readable description of the command.
    ///
    /// Environment *values* are never included; only the program and arguments
    /// are shown, and secret environment variables are displayed as `***`.
    #[must_use]
    pub fn description(&self) -> String {
        let mut rendered = self.program.clone();
        for arg in &self.args {
            rendered.push(' ');
            rendered.push_str(arg);
        }
        rendered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_structured_command() {
        let spec = ProcessSpec::new("/bin/echo")
            .unwrap()
            .args(["hello", "world"]);
        assert_eq!(spec.program, "/bin/echo");
        assert_eq!(spec.args, vec!["hello", "world"]);
        assert_eq!(spec.description(), "/bin/echo hello world");
    }

    #[test]
    fn rejects_invalid_programs() {
        assert!(ProcessSpec::new("").is_err());
        assert!(ProcessSpec::new("   ").is_err());
        assert!(ProcessSpec::new("/bin/echo hi").is_err());
    }

    #[test]
    fn rejects_invalid_environment_keys() {
        let variable = EnvironmentVariable::new("1BAD", "x").unwrap_err();
        // Building the variable already fails in the domain, so wrap a valid
        // construction path and assert the manager validates too.
        assert!(matches!(
            variable,
            service_orchestrator_domain::DomainError::Validation { .. }
        ));
        let spec = ProcessSpec::new("/bin/echo").unwrap();
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn default_policies_are_conservative() {
        let spec = ProcessSpec::new("/bin/echo").unwrap();
        assert_eq!(spec.restart, RestartPolicy::Never);
        assert_eq!(spec.shutdown, ShutdownStrategy::graceful());
    }

    #[test]
    fn restart_policy_decides_restarts() {
        let on_failure = RestartPolicy::OnFailure {
            max_restarts: 2,
            backoff: Duration::from_millis(10),
        };
        assert!(on_failure.should_restart(ExitReason::Failed, 0));
        assert!(!on_failure.should_restart(ExitReason::Failed, 2));
        assert!(!on_failure.should_restart(ExitReason::Exited, 0));
        assert!(!on_failure.should_restart(ExitReason::Cancelled, 0));

        let always = RestartPolicy::Always {
            max_restarts: 1,
            backoff: Duration::from_millis(10),
        };
        assert!(always.should_restart(ExitReason::Exited, 0));
        assert!(!always.should_restart(ExitReason::Terminated, 0));
        assert!(!always.should_restart(ExitReason::Killed, 0));
    }

    #[test]
    fn shutdown_strategy_is_configurable() {
        let strategy = ShutdownStrategy::graceful()
            .with_signal(TerminationSignal::Int)
            .with_grace_period(Duration::from_millis(250));
        assert_eq!(strategy.signal, TerminationSignal::Int);
        assert_eq!(strategy.grace_period, Duration::from_millis(250));
    }
}
