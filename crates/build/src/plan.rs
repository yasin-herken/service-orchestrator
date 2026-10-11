//! Structured build plans.
//!
//! A [`BuildPlan`] is *data*, not behaviour. A build provider constructs one and
//! the process manager executes it; nothing in this module spawns a process,
//! touches the filesystem, or schedules a task (`tasks/008.md`). Keeping the
//! plan a plain value is what lets the provider stay a pure, testable mapping
//! from a validated [`ResolvedBuildTarget`](service_orchestrator_application::ResolvedBuildTarget)
//! to a program, an argument vector, a working directory, its requirements, and
//! its expected outputs.
//!
//! # Environment requirements, not values
//!
//! A plan declares the environment variables a build *expects* by name only
//! ([`EnvironmentRequirement`]). It never carries a variable's value, so a
//! plan can be logged, surfaced through a task, or returned over MCP without
//! risking a credential leak (`AGENTS.md` §31). The value, when there is one,
//! comes from the developer's process environment at execution time.
//!
//! # Expected outputs
//!
//! Where a build system has a conventional output location (Maven's `target`
//! directory, for example) the provider records it as an [`ExpectedOutput`] so
//! interfaces can describe what a build produced. Providers whose outputs are
//! not well known record none rather than guessing.

use std::fmt;
use std::time::Duration;

use service_orchestrator_domain::{AbsolutePath, BuildOperation, BuildSystem};

/// An environment variable a build plan expects to be present.
///
/// Only the variable's *name* is recorded (`AGENTS.md` §31). `required` marks a
/// variable without which the build must not start; an optional requirement is
/// advisory and never blocks execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentRequirement {
    /// The variable name, for example `JAVA_HOME`.
    pub name: String,
    /// Whether the build must not start when the variable is absent.
    pub required: bool,
}

impl EnvironmentRequirement {
    /// Declares a variable the build cannot run without.
    #[must_use]
    pub fn required(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            required: true,
        }
    }

    /// Declares a variable the build prefers but does not require.
    #[must_use]
    pub fn optional(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            required: false,
        }
    }
}

/// The kind of output a plan expects to produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedOutputKind {
    /// A directory of generated output.
    Directory,
    /// A single generated file.
    File,
    /// An assembled or published artifact.
    Artifact,
}

impl fmt::Display for ExpectedOutputKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Directory => "directory",
            Self::File => "file",
            Self::Artifact => "artifact",
        })
    }
}

/// A location a plan expects to produce.
///
/// The path is relative to the plan's working directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedOutput {
    /// The location relative to the working directory.
    pub path: String,
    /// The kind of output.
    pub kind: ExpectedOutputKind,
}

impl ExpectedOutput {
    /// Constructs an expected output.
    #[must_use]
    pub fn new(path: impl Into<String>, kind: ExpectedOutputKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

/// A structured, build-system-neutral description of one build invocation.
///
/// The process manager turns a plan into a [`ProcessSpec`]. The plan itself is
/// inert: constructing one never runs anything.
///
/// [`ProcessSpec`]: service_orchestrator_process::ProcessSpec
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildPlan {
    /// The executable to run, for example `mvn`, `./mvnw`, or `npm`.
    pub program: String,
    /// The arguments passed to the executable, in order.
    pub args: Vec<String>,
    /// The directory the build runs in.
    pub working_dir: AbsolutePath,
    /// The environment variables the build expects, by name.
    pub environment: Vec<EnvironmentRequirement>,
    /// An optional maximum runtime declared by the provider.
    pub timeout: Option<Duration>,
    /// The outputs the build is expected to produce.
    pub expected_outputs: Vec<ExpectedOutput>,
    /// The build system the plan targets.
    pub system: BuildSystem,
    /// The build-system-neutral operation the plan performs.
    pub operation: BuildOperation,
}

impl BuildPlan {
    /// Creates a plan for `system`/`operation` running `program`.
    #[must_use]
    pub fn new(
        system: BuildSystem,
        operation: BuildOperation,
        program: impl Into<String>,
        working_dir: AbsolutePath,
    ) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            working_dir,
            environment: Vec::new(),
            timeout: None,
            expected_outputs: Vec::new(),
            system,
            operation,
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

    /// Adds an environment requirement.
    #[must_use]
    pub fn with_environment(mut self, requirement: EnvironmentRequirement) -> Self {
        self.environment.push(requirement);
        self
    }

    /// Sets the declared maximum runtime.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Adds an expected output.
    #[must_use]
    pub fn with_expected_output(mut self, output: ExpectedOutput) -> Self {
        self.expected_outputs.push(output);
        self
    }

    /// Iterates over the environment variables the build cannot run without.
    pub fn required_environment(&self) -> impl Iterator<Item = &str> {
        self.environment
            .iter()
            .filter(|requirement| requirement.required)
            .map(|requirement| requirement.name.as_str())
    }

    /// Returns a redacted, human-readable rendering of the command.
    ///
    /// Only the program and arguments are shown; environment requirements are
    /// names and carry no values.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> BuildPlan {
        BuildPlan::new(
            BuildSystem::Maven,
            BuildOperation::Build,
            "mvn",
            AbsolutePath::new("/w/auth-service").unwrap(),
        )
        .with_args(["-B", "package"])
        .with_environment(EnvironmentRequirement::required("JAVA_HOME"))
        .with_environment(EnvironmentRequirement::optional("MAVEN_OPTS"))
        .with_expected_output(ExpectedOutput::new("target", ExpectedOutputKind::Directory))
    }

    #[test]
    fn renders_the_command_without_environment_values() {
        assert_eq!(plan().command(), "mvn -B package");
    }

    #[test]
    fn required_environment_excludes_optional_names() {
        let plan = plan();
        let required: Vec<&str> = plan.required_environment().collect();
        assert_eq!(required, vec!["JAVA_HOME"]);
    }

    #[test]
    fn environment_requirement_constructors_set_requiredness() {
        assert!(EnvironmentRequirement::required("A").required);
        assert!(!EnvironmentRequirement::optional("B").required);
    }

    #[test]
    fn expected_output_kind_has_a_stable_label() {
        assert_eq!(ExpectedOutputKind::Directory.to_string(), "directory");
        assert_eq!(ExpectedOutputKind::Artifact.to_string(), "artifact");
    }
}
