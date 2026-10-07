//! Structured service commands.
//!
//! Commands are modelled structurally (a program plus an argument vector),
//! never as an opaque shell string. This is a security boundary: structured
//! arguments cannot be re-interpreted by a shell, so values that originate from
//! configuration or user input cannot smuggle in `;`, `&&`, pipes, or command
//! substitution (see `AGENTS.md` §32).
//!
//! The command *set* is an ordered list of [`CommandEntry`] values rather than a
//! map keyed by an enum, so that it serializes cleanly to JSON for the MCP and
//! configuration boundaries and remains open to future command kinds.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::environment::Environment;
use crate::error::DomainError;
use crate::paths::RelativePath;

/// The semantic role of a command.
///
/// The `Other` variant allows project-specific commands (for example `package`
/// or `lint`) without changing this enum or the rest of the domain.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Compile or assemble the service or library.
    Build,
    /// Start the service.
    Start,
    /// Stop the service gracefully.
    Stop,
    /// Restart the service.
    Restart,
    /// Install dependencies or artifacts.
    Install,
    /// Run the test suite.
    Test,
    /// Run database migrations.
    Liquibase,
    /// A project-specific command.
    Other(String),
}

impl CommandKind {
    /// Returns a short, human-readable label.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Build => "build",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Install => "install",
            Self::Test => "test",
            Self::Liquibase => "liquibase",
            Self::Other(name) => name,
        }
    }
}

impl fmt::Display for CommandKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

fn validate_program(program: &str) -> Result<(), DomainError> {
    if program.trim().is_empty() {
        return Err(DomainError::InvalidCommand {
            reason: "program must not be empty".to_owned(),
        });
    }
    if program.chars().any(char::is_whitespace) {
        return Err(DomainError::InvalidCommand {
            reason: format!("program '{program}' must not contain whitespace"),
        });
    }
    if !program
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '+' | ':' | '\\'))
    {
        return Err(DomainError::InvalidCommand {
            reason: format!(
                "program '{program}' may only contain letters, digits, '.', '_', '-', '/', '+', ':', and '\\'"
            ),
        });
    }
    Ok(())
}

/// A single executable invocation.
///
/// `program` names the executable (for example `mvn`, `npm`, `./mvnw`), `args`
/// are passed verbatim as separate arguments, and the optional `working_dir` is
/// resolved relative to the service's repository root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    /// The executable to run.
    pub program: String,
    /// The arguments passed to the executable, in order.
    pub args: Vec<String>,
    /// The directory the command runs in, relative to the workspace.
    pub working_dir: Option<RelativePath>,
    /// Additional environment variables for this command.
    pub environment: Environment,
}

impl CommandSpec {
    /// Constructs a command from a program and its arguments.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::InvalidCommand`] if the program is empty or
    /// contains unsafe characters.
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Result<Self, DomainError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let program = program.into();
        validate_program(&program)?;
        Ok(Self {
            program,
            args: args.into_iter().map(Into::into).collect(),
            working_dir: None,
            environment: Environment::new(),
        })
    }

    /// Sets the working directory for this command.
    #[must_use]
    pub fn with_working_dir(mut self, working_dir: RelativePath) -> Self {
        self.working_dir = Some(working_dir);
        self
    }

    /// Adds an environment variable to this command.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::Validation`] if the variable is invalid.
    pub fn with_environment(
        mut self,
        variable: crate::environment::EnvironmentVariable,
    ) -> Result<Self, DomainError> {
        self.environment.set(variable)?;
        Ok(self)
    }

    /// Validates the command specification.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::InvalidCommand`] if the program is invalid.
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_program(&self.program)
    }

    /// Returns `true` if this command is a full command line passed to a shell.
    ///
    /// Structured commands are the norm; this helper exists so the policy layer
    /// can flag an explicitly shell-based command for extra scrutiny.
    #[must_use]
    pub fn invokes_shell(&self) -> bool {
        let program = self.program.rsplit('/').next().unwrap_or(&self.program);
        matches!(program, "sh" | "bash" | "zsh" | "fish") && self.args.iter().any(|arg| arg == "-c")
    }
}

/// A [`CommandKind`] paired with its [`CommandSpec`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEntry {
    /// The semantic role of the command.
    pub kind: CommandKind,
    /// The command to run.
    pub spec: CommandSpec,
}

impl CommandEntry {
    /// Creates a command entry.
    #[must_use]
    pub fn new(kind: CommandKind, spec: CommandSpec) -> Self {
        Self { kind, spec }
    }
}

/// The set of commands declared for a service or library.
///
/// A command set holds at most one entry per [`CommandKind`]. The vector
/// representation serializes to a JSON array, which keeps the configuration and
/// MCP representations straightforward and free of invalid map keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommandSet {
    entries: Vec<CommandEntry>,
}

impl CommandSet {
    /// Creates an empty command set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or replaces the command for `kind`, returning the previous
    /// specification if one existed.
    pub fn set(&mut self, kind: CommandKind, spec: CommandSpec) -> Option<CommandSpec> {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.kind == kind) {
            return Some(std::mem::replace(&mut entry.spec, spec));
        }
        self.entries.push(CommandEntry::new(kind, spec));
        None
    }

    /// Returns the command for `kind`, if present.
    #[must_use]
    pub fn get(&self, kind: &CommandKind) -> Option<&CommandSpec> {
        self.entries
            .iter()
            .find(|entry| &entry.kind == kind)
            .map(|entry| &entry.spec)
    }

    /// Returns `true` if a command exists for `kind`.
    #[must_use]
    pub fn contains(&self, kind: &CommandKind) -> bool {
        self.get(kind).is_some()
    }

    /// Removes the command for `kind`, returning it if present.
    pub fn remove(&mut self, kind: &CommandKind) -> Option<CommandSpec> {
        let index = self.entries.iter().position(|entry| &entry.kind == kind)?;
        Some(self.entries.remove(index).spec)
    }

    /// Returns the number of commands.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if there are no commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over the command entries in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &CommandEntry> {
        self.entries.iter()
    }

    /// Validates every command in the set.
    ///
    /// # Errors
    ///
    /// Returns a [`DomainError::InvalidCommand`] if any command is malformed.
    pub fn validate(&self) -> Result<(), DomainError> {
        for entry in &self.entries {
            entry.spec.validate()?;
        }
        Ok(())
    }
}

impl fmt::Display for CommandSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rendered: Vec<String> = self
            .entries
            .iter()
            .map(|entry| {
                let mut command = entry.spec.program.clone();
                for arg in &entry.spec.args {
                    command.push(' ');
                    command.push_str(arg);
                }
                format!("{}: {command}", entry.kind)
            })
            .collect();
        f.write_str(&rendered.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_structured_commands() {
        let spec = CommandSpec::new("./mvnw", ["clean", "install"]).unwrap();
        assert_eq!(spec.program, "./mvnw");
        assert_eq!(spec.args, vec!["clean", "install"]);
        assert!(!spec.invokes_shell());
    }

    #[test]
    fn rejects_unsafe_programs() {
        assert!(CommandSpec::new("mvn; rm -rf /", Vec::<String>::new()).is_err());
        assert!(CommandSpec::new("mvn && evil", Vec::<String>::new()).is_err());
        assert!(CommandSpec::new("", Vec::<String>::new()).is_err());
        assert!(CommandSpec::new("mvn build", Vec::<String>::new()).is_err());
    }

    #[test]
    fn flags_shell_invocations() {
        let spec = CommandSpec::new("sh", ["-c", "echo hi > file"]).unwrap();
        assert!(spec.invokes_shell());
        let direct = CommandSpec::new("sh", ["script.sh"]).unwrap();
        assert!(!direct.invokes_shell());
    }

    #[test]
    fn command_set_replaces_and_removes() {
        let mut commands = CommandSet::new();
        let build = CommandSpec::new("mvn", ["clean", "install"]).unwrap();
        assert!(commands.set(CommandKind::Build, build).is_none());
        assert!(commands.contains(&CommandKind::Build));

        let replacement = CommandSpec::new("mvn", ["package"]).unwrap();
        let previous = commands.set(CommandKind::Build, replacement).unwrap();
        assert_eq!(previous.args, vec!["clean", "install"]);
        assert_eq!(commands.len(), 1);

        assert!(commands.remove(&CommandKind::Build).is_some());
        assert!(commands.is_empty());
    }

    #[test]
    fn command_set_validates_members() {
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Start,
            CommandSpec::new("npm", ["run", "start"]).unwrap(),
        );
        assert!(commands.validate().is_ok());
    }

    #[test]
    fn supports_project_specific_command_kinds() {
        let spec = CommandSpec::new("npm", ["run", "lint"]).unwrap();
        let kind = CommandKind::Other("lint".to_owned());
        let mut commands = CommandSet::new();
        commands.set(kind.clone(), spec);
        assert!(commands.contains(&kind));
    }
}
