//! Semantic parsing and validation helpers.
//!
//! The raw model is intentionally stringly typed. This module converts those
//! strings into domain value objects and performs the checks that are specific
//! to configuration rather than to the domain:
//!
//! - known enum spellings (`backend`, `java`, `http`, …) and `other:<name>`
//!   escape hatches;
//! - command-line tokenization;
//! - environment variable construction;
//! - port ranges and health-check required fields;
//! - consistency between a service's declared capabilities and its commands
//!   and runtime, and between a library's technology and its artifact metadata.
//!
//! Every helper follows the same shape: it reports failures by pushing a
//! [`ValidationError`] with a precise location and returns `None`, so the
//! caller can keep collecting problems from the rest of the file.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Duration;

use service_orchestrator_domain::{
    ArtifactCoordinate, CommandKind, CommandSpec, DependencyKind, Environment, EnvironmentVariable,
    HealthCheck, HealthCheckKind, HttpHealthCheck, Library, Port, RelativePath, RepositoryProvider,
    Runtime, RuntimeKind, Service, ServiceCapability, ServiceType, Version,
};

use crate::error::ValidationError;
use crate::raw::{
    RawArtifact, RawCommand, RawDependencyRef, RawDetailedDependency, RawEnvValue, RawHealthCheck,
    RawPort, RawRepository, RawRuntime, RawStructuredCommand,
};

/// The command kinds that may be declared by name in a configuration file.
const NAMED_COMMAND_KINDS: [(&str, CommandKind); 7] = [
    ("install", CommandKind::Install),
    ("build", CommandKind::Build),
    ("start", CommandKind::Start),
    ("stop", CommandKind::Stop),
    ("restart", CommandKind::Restart),
    ("test", CommandKind::Test),
    ("liquibase", CommandKind::Liquibase),
];

/// Parses a strongly typed identifier, reporting a [`ValidationError`] on
/// failure.
pub(crate) fn parse_id<T>(
    location: &str,
    value: &str,
    errors: &mut Vec<ValidationError>,
) -> Option<T>
where
    T: FromStr<Err = service_orchestrator_domain::DomainError>,
{
    match value.parse::<T>() {
        Ok(parsed) => Some(parsed),
        Err(error) => {
            errors.push(
                ValidationError::new(location, error.to_string())
                    .expected("an identifier using letters, digits, '-', '_', '.', and ':'")
                    .suggested_fix(format!(
                        "Rename '{value}' so it contains no whitespace or path separators."
                    )),
            );
            None
        }
    }
}

/// Parses a semantic version string.
pub(crate) fn parse_version(
    location: &str,
    value: &str,
    errors: &mut Vec<ValidationError>,
) -> Option<Version> {
    match Version::new(value) {
        Ok(version) => Some(version),
        Err(error) => {
            errors.push(
                ValidationError::new(location, error.to_string())
                    .expected("a non-empty version such as '21', '20.19.4', or '1.8.0-SNAPSHOT'"),
            );
            None
        }
    }
}

/// Parses a service type.
pub(crate) fn parse_service_type(
    location: &str,
    value: &str,
    errors: &mut Vec<ValidationError>,
) -> Option<ServiceType> {
    match value {
        "backend" => Some(ServiceType::Backend),
        "frontend" => Some(ServiceType::Frontend),
        "library" => Some(ServiceType::Library),
        other => match other.strip_prefix("other:") {
            Some(name) if !name.trim().is_empty() => {
                Some(ServiceType::Other(name.trim().to_owned()))
            }
            Some(_) => {
                errors.push(
                    ValidationError::new(location, "the 'other:' service type needs a name")
                        .expected("'other:<name>', for example 'other:worker'"),
                );
                None
            }
            None => {
                errors.push(
                    ValidationError::new(
                        location,
                        format!("'{value}' is not a known service type"),
                    )
                    .expected("one of: backend, frontend, library, other:<name>"),
                );
                None
            }
        },
    }
}

/// Parses a runtime kind.
pub(crate) fn parse_runtime_kind(
    location: &str,
    value: &str,
    errors: &mut Vec<ValidationError>,
) -> Option<RuntimeKind> {
    match value {
        "java" => Some(RuntimeKind::Java),
        "node" => Some(RuntimeKind::Node),
        "maven" => Some(RuntimeKind::Maven),
        "npm" => Some(RuntimeKind::Npm),
        other => match other.strip_prefix("other:") {
            Some(name) if !name.trim().is_empty() => {
                Some(RuntimeKind::Other(name.trim().to_owned()))
            }
            Some(_) => {
                errors.push(
                    ValidationError::new(location, "the 'other:' runtime type needs a name")
                        .expected("'other:<name>', for example 'other:python'"),
                );
                None
            }
            None => {
                errors.push(
                    ValidationError::new(location, format!("'{value}' is not a known runtime"))
                        .expected("one of: java, node, maven, npm, other:<name>"),
                );
                None
            }
        },
    }
}

/// Parses a service capability.
pub(crate) fn parse_capability(
    location: &str,
    value: &str,
    errors: &mut Vec<ValidationError>,
) -> Option<ServiceCapability> {
    match value {
        "build" => Some(ServiceCapability::Build),
        "run" => Some(ServiceCapability::Run),
        "migrate" => Some(ServiceCapability::Migrate),
        "provide_artifact" => Some(ServiceCapability::ProvideArtifact),
        "serve_http" => Some(ServiceCapability::ServeHttp),
        other => {
            errors.push(
                ValidationError::new(
                    location,
                    format!("'{other}' is not a known service capability"),
                )
                .expected("one of: build, run, migrate, provide_artifact, serve_http"),
            );
            None
        }
    }
}

/// Parses a dependency kind, falling back to `default` when unspecified.
pub(crate) fn parse_dependency_kind(
    location: &str,
    value: Option<&str>,
    default: DependencyKind,
    errors: &mut Vec<ValidationError>,
) -> Option<DependencyKind> {
    match value {
        None => Some(default),
        Some("build") => Some(DependencyKind::Build),
        Some("runtime") => Some(DependencyKind::Runtime),
        Some(other) => {
            errors.push(
                ValidationError::new(
                    location,
                    format!("'{other}' is not a known dependency kind"),
                )
                .expected("one of: build, runtime"),
            );
            None
        }
    }
}

/// Parses a runtime requirement, validating its pinned version.
pub(crate) fn parse_runtime(
    location: &str,
    raw: &RawRuntime,
    errors: &mut Vec<ValidationError>,
) -> Option<Runtime> {
    let kind = parse_runtime_kind(&format!("{location}.type"), &raw.kind, errors)?;
    match &raw.version {
        Some(version) => {
            let version = parse_version(&format!("{location}.version"), version, errors)?;
            Some(Runtime::versioned(kind, version))
        }
        None => Some(Runtime::any(kind)),
    }
}

/// Parses an artifact coordinate.
pub(crate) fn parse_artifact(
    location: &str,
    raw: &RawArtifact,
    errors: &mut Vec<ValidationError>,
) -> Option<ArtifactCoordinate> {
    let version = parse_version(&format!("{location}.version"), &raw.version, errors)?;
    match ArtifactCoordinate::new(raw.group_id.clone(), raw.artifact_id.clone(), version) {
        Ok(artifact) => Some(artifact),
        Err(error) => {
            errors.push(ValidationError::new(location, error.to_string()));
            None
        }
    }
}

/// Parses a repository description.
pub(crate) fn parse_repository(
    location: &str,
    raw: &RawRepository,
    local_path: &RelativePath,
    fallback_branch: Option<&str>,
    errors: &mut Vec<ValidationError>,
) -> Option<service_orchestrator_domain::Repository> {
    let branch = raw
        .default_branch
        .as_deref()
        .or(fallback_branch)
        .map(|value| {
            service_orchestrator_domain::Branch::new(value).map_err(|error| {
                ValidationError::new(format!("{location}.default_branch"), error.to_string())
                    .expected("a branch name without whitespace, such as 'main' or 'release/1.2'")
            })
        })
        .transpose();
    let branch = match branch {
        Ok(branch) => branch,
        Err(error) => {
            errors.push(error);
            return None;
        }
    };
    let Some(branch) = branch else {
        errors.push(
            ValidationError::new(
                location,
                "repository has no default branch and the entity declares none either",
            )
            .expected("a default branch, for example 'main'")
            .suggested_fix(
                "Set the branch on the repository or the service/library: default_branch = \"main\"."
                    .to_owned(),
            ),
        );
        return None;
    };

    let repository = match service_orchestrator_domain::Repository::new(
        raw.url.clone(),
        local_path.clone(),
        branch,
    ) {
        Ok(repository) => repository,
        Err(error) => {
            errors.push(ValidationError::new(location, error.to_string()));
            return None;
        }
    };

    let repository = match &raw.provider {
        None => repository,
        Some(provider) if provider == "git" => repository.with_provider(RepositoryProvider::Git),
        Some(provider) => repository.with_provider(RepositoryProvider::Other(provider.clone())),
    };
    Some(repository)
}

/// Parses a port, validating the protocol.
pub(crate) fn parse_port(
    location: &str,
    raw: &RawPort,
    errors: &mut Vec<ValidationError>,
) -> Option<Port> {
    if let Some(protocol) = &raw.protocol {
        if protocol != "tcp" && protocol != "udp" {
            errors.push(
                ValidationError::new(
                    format!("{location}.protocol"),
                    format!("'{protocol}' is not a known protocol"),
                )
                .expected("one of: tcp, udp"),
            );
            return None;
        }
    }
    match Port::new(raw.port) {
        Ok(port) => Some(port),
        Err(error) => {
            errors.push(
                ValidationError::new(format!("{location}.port"), error.to_string())
                    .expected("a port between 1 and 65535"),
            );
            None
        }
    }
}

/// Parses a health check, validating required fields for its kind.
pub(crate) fn parse_health_check(
    location: &str,
    raw: &RawHealthCheck,
    errors: &mut Vec<ValidationError>,
) -> Option<HealthCheck> {
    let kind = match raw.kind.as_str() {
        "process" => HealthCheckKind::Process,
        "port" => {
            let Some(port) = raw.port else {
                errors.push(
                    ValidationError::new(
                        format!("{location}.port"),
                        "a port health check needs a port",
                    )
                    .expected("an integer port between 1 and 65535")
                    .suggested_fix("Add `port = 8081` to the health check.".to_owned()),
                );
                return None;
            };
            match Port::new(port) {
                Ok(port) => HealthCheckKind::Port(port),
                Err(error) => {
                    errors.push(
                        ValidationError::new(format!("{location}.port"), error.to_string())
                            .expected("a port between 1 and 65535"),
                    );
                    return None;
                }
            }
        }
        "http" => {
            let Some(url) = raw.url.as_deref() else {
                errors.push(
                    ValidationError::new(
                        format!("{location}.url"),
                        "an http health check needs a url",
                    )
                    .expected("an absolute http:// or https:// URL")
                    .suggested_fix(
                        "Add `url = \"http://localhost:8081/actuator/health\"`.".to_owned(),
                    ),
                );
                return None;
            };
            match HttpHealthCheck::new(url, raw.expected_status) {
                Ok(check) => HealthCheckKind::Http(check),
                Err(error) => {
                    errors.push(ValidationError::new(
                        format!("{location}.url"),
                        error.to_string(),
                    ));
                    return None;
                }
            }
        }
        other => {
            errors.push(
                ValidationError::new(
                    format!("{location}.type"),
                    format!("'{other}' is not a known health check type"),
                )
                .expected("one of: process, port, http"),
            );
            return None;
        }
    };

    let mut check = HealthCheck::new(kind);
    if let Some(seconds) = raw.timeout_seconds {
        check = check.with_timeout(Duration::from_secs(seconds));
    }
    if let Some(seconds) = raw.interval_seconds {
        check = check.with_interval(Duration::from_secs(seconds));
    }
    Some(check)
}

/// Parses a command, in either string or structured form.
pub(crate) fn parse_command(
    location: &str,
    raw: &RawCommand,
    errors: &mut Vec<ValidationError>,
) -> Option<CommandSpec> {
    match raw {
        RawCommand::Line(line) => {
            let tokens = match split_command_line(line) {
                Ok(tokens) => tokens,
                Err(reason) => {
                    errors.push(
                        ValidationError::new(location, reason)
                            .expected("a command such as './mvnw clean install'"),
                    );
                    return None;
                }
            };
            let Some((program, args)) = tokens.split_first() else {
                errors.push(
                    ValidationError::new(location, "command must not be empty")
                        .expected("a non-empty command line"),
                );
                return None;
            };
            match CommandSpec::new(program.clone(), args.to_vec()) {
                Ok(spec) => Some(spec),
                Err(error) => {
                    errors.push(
                        ValidationError::new(location, error.to_string()).suggested_fix(
                            "Use a structured command for a program name that contains spaces."
                                .to_owned(),
                        ),
                    );
                    None
                }
            }
        }
        RawCommand::Structured(structured) => {
            parse_structured_command(location, structured, errors)
        }
    }
}

fn parse_structured_command(
    location: &str,
    structured: &RawStructuredCommand,
    errors: &mut Vec<ValidationError>,
) -> Option<CommandSpec> {
    let mut spec = match CommandSpec::new(structured.program.clone(), structured.args.clone()) {
        Ok(spec) => spec,
        Err(error) => {
            errors.push(ValidationError::new(
                format!("{location}.program"),
                error.to_string(),
            ));
            return None;
        }
    };
    if let Some(working_dir) = &structured.working_dir {
        match RelativePath::new(working_dir.clone()) {
            Ok(path) => spec = spec.with_working_dir(path),
            Err(error) => {
                errors.push(ValidationError::new(
                    format!("{location}.working_dir"),
                    error.to_string(),
                ));
                return None;
            }
        }
    }
    let (environment, mut env_errors) =
        parse_environment(&format!("{location}.environment"), &structured.environment);
    errors.append(&mut env_errors);
    spec.environment = environment;
    Some(spec)
}

/// Parses an environment variable map.
pub(crate) fn parse_environment(
    location: &str,
    raw: &BTreeMap<String, RawEnvValue>,
) -> (Environment, Vec<ValidationError>) {
    let mut environment = Environment::new();
    let mut errors = Vec::new();
    for (key, value) in raw {
        let (value, secret) = match value {
            RawEnvValue::Literal(value) => (value.clone(), false),
            RawEnvValue::Structured(variable) => (variable.value.clone(), variable.secret),
        };
        match EnvironmentVariable::with_secrecy(key.clone(), value, secret) {
            Ok(variable) => {
                // The variable was already validated, so `set` cannot fail.
                let _ = environment.set(variable);
            }
            Err(error) => {
                errors.push(ValidationError::new(
                    format!("{location}.{key}"),
                    error.to_string(),
                ));
            }
        }
    }
    (environment, errors)
}

/// Parses the named commands of a raw command table.
pub(crate) fn parse_commands(
    location: &str,
    raw: &crate::raw::RawCommands,
    errors: &mut Vec<ValidationError>,
) -> service_orchestrator_domain::CommandSet {
    let mut commands = service_orchestrator_domain::CommandSet::new();
    for (name, kind) in NAMED_COMMAND_KINDS {
        let raw_command = match name {
            "install" => raw.install.as_ref(),
            "build" => raw.build.as_ref(),
            "start" => raw.start.as_ref(),
            "stop" => raw.stop.as_ref(),
            "restart" => raw.restart.as_ref(),
            "test" => raw.test.as_ref(),
            "liquibase" => raw.liquibase.as_ref(),
            _ => None,
        };
        if let Some(raw_command) = raw_command {
            if let Some(spec) = parse_command(&format!("{location}.{name}"), raw_command, errors) {
                commands.set(kind, spec);
            }
        }
    }
    for (name, raw_command) in &raw.custom {
        if name.trim().is_empty() {
            errors.push(ValidationError::new(
                format!("{location}.custom"),
                "project-specific command names must not be empty",
            ));
            continue;
        }
        if let Some(spec) = parse_command(&format!("{location}.custom.{name}"), raw_command, errors)
        {
            commands.set(CommandKind::Other(name.clone()), spec);
        }
    }
    commands
}

/// Parses a dependency reference into a domain dependency.
pub(crate) fn parse_dependency(
    location: &str,
    raw: &RawDependencyRef,
    default_kind: DependencyKind,
    target_is_library: bool,
    errors: &mut Vec<ValidationError>,
) -> Option<service_orchestrator_domain::Dependency> {
    let (id, kind, required) = match raw {
        RawDependencyRef::Id(id) => (id.clone(), default_kind, true),
        RawDependencyRef::Detailed(RawDetailedDependency { id, kind, required }) => {
            let kind = parse_dependency_kind(
                &format!("{location}.kind"),
                kind.as_deref(),
                default_kind,
                errors,
            )?;
            (id.clone(), kind, required.unwrap_or(true))
        }
    };

    let target = if target_is_library {
        let id: service_orchestrator_domain::LibraryId = parse_id(location, &id, errors)?;
        service_orchestrator_domain::DependencyTarget::Library(id)
    } else {
        let id: service_orchestrator_domain::ServiceId = parse_id(location, &id, errors)?;
        service_orchestrator_domain::DependencyTarget::Service(id)
    };

    Some(if required {
        service_orchestrator_domain::Dependency::new(target, kind)
    } else {
        service_orchestrator_domain::Dependency::optional(target, kind)
    })
}

/// Validates the consistency between a service's type, commands, and runtime.
pub(crate) fn validate_service_consistency(
    location: &str,
    service: &Service,
    errors: &mut Vec<ValidationError>,
) {
    if service.supports(ServiceCapability::Build) && !service.has_build_command() {
        errors.push(
            ValidationError::new(
                format!("{location}.commands.build"),
                format!(
                    "service '{}' can be built but has no build command",
                    service.id
                ),
            )
            .expected("a build command")
            .suggested_fix(
                "Add a [services.commands] entry, for example `build = \"./mvnw clean install\"`."
                    .to_owned(),
            ),
        );
    }
    if service.supports(ServiceCapability::Run) && !service.has_start_command() {
        errors.push(
            ValidationError::new(
                format!("{location}.commands.start"),
                format!("service '{}' can be run but has no start command", service.id),
            )
            .expected("a start command")
            .suggested_fix(
                "Add a [services.commands] entry, for example `start = \"./mvnw spring-boot:run\"`."
                    .to_owned(),
            ),
        );
    }
    if service.kind == ServiceType::Frontend && service.runtimes.is_empty() {
        errors.push(
            ValidationError::new(
                format!("{location}.runtime"),
                format!("frontend service '{}' declares no runtime", service.id),
            )
            .expected("a runtime such as Node 20.19.4")
            .suggested_fix(
                "Add [services.runtime] with `type = \"node\"` and `version = \"20.19.4\"`."
                    .to_owned(),
            ),
        );
    }
    for runtime in &service.runtimes {
        if matches!(runtime.kind, RuntimeKind::Java | RuntimeKind::Node)
            && runtime.version.is_none()
        {
            errors.push(
                ValidationError::new(
                    format!("{location}.runtime.version"),
                    format!("{} runtime version is missing", runtime.kind.label()),
                )
                .expected("a non-empty semantic/version-like string")
                .suggested_fix(
                    "Add `version = \"20.19.4\"` (or the required version) to the runtime."
                        .to_owned(),
                ),
            );
        }
    }
}

/// Validates a library's build command and artifact metadata.
pub(crate) fn validate_library_consistency(
    location: &str,
    library_type: &str,
    library: &Library,
    errors: &mut Vec<ValidationError>,
) {
    if !library.commands.contains(&CommandKind::Build) {
        errors.push(
            ValidationError::new(
                format!("{location}.commands.build"),
                format!("library '{}' has no build command", library.id),
            )
            .expected("a build command")
            .suggested_fix(
                "Add `build = \"./mvnw clean install\"` to [libraries.commands].".to_owned(),
            ),
        );
    }
    if library_type == "maven" && library.artifact.is_none() {
        errors.push(
            ValidationError::new(
                format!("{location}.artifact"),
                format!("maven library '{}' has no artifact coordinate", library.id),
            )
            .expected("group_id, artifact_id, and version")
            .suggested_fix(
                "Add [libraries.artifact] with `group_id`, `artifact_id`, and `version`."
                    .to_owned(),
            ),
        );
    }
}

/// Splits a command line into a program and arguments.
///
/// This is intentionally small and dependency-free. It honours single and
/// double quotes and backslash escapes so arguments such as
/// `-Dflag="a b"` survive, but it is **not** a shell: there is no expansion,
/// globbing, piping, or command substitution.
pub(crate) fn split_command_line(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some(active) => {
                if ch == active {
                    quote = None;
                } else if ch == '\\' && active == '"' {
                    match chars.next() {
                        Some(escaped) => current.push(escaped),
                        None => {
                            return Err("command ends with a dangling backslash".to_owned());
                        }
                    }
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    has_token = true;
                }
                '\\' => match chars.next() {
                    Some(escaped) => {
                        current.push(escaped);
                        has_token = true;
                    }
                    None => return Err("command ends with a dangling backslash".to_owned()),
                },
                ch if ch.is_whitespace() => {
                    if has_token {
                        tokens.push(std::mem::take(&mut current));
                        has_token = false;
                    }
                }
                ch => {
                    current.push(ch);
                    has_token = true;
                }
            },
        }
    }

    if let Some(active) = quote {
        return Err(format!("command has an unterminated {active} quote"));
    }
    if has_token {
        tokens.push(current);
    }
    if tokens.is_empty() {
        return Err("command must not be empty".to_owned());
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_command_lines() {
        assert_eq!(
            split_command_line("./mvnw clean install").unwrap(),
            vec!["./mvnw", "clean", "install"]
        );
        assert_eq!(
            split_command_line("  npm   run   build  ").unwrap(),
            vec!["npm", "run", "build"]
        );
    }

    #[test]
    fn honours_quotes_and_escapes() {
        assert_eq!(
            split_command_line("echo 'a b' \"c d\"").unwrap(),
            vec!["echo", "a b", "c d"]
        );
        assert_eq!(
            split_command_line("cmd -Dx=\"a b\"").unwrap(),
            vec!["cmd", "-Dx=a b"]
        );
    }

    #[test]
    fn rejects_empty_and_unterminated_commands() {
        assert!(split_command_line("   ").is_err());
        assert!(split_command_line("echo 'oops").is_err());
        assert!(split_command_line("echo \\").is_err());
    }

    #[test]
    fn parses_service_types_and_capabilities() {
        let mut errors = Vec::new();
        assert_eq!(
            parse_service_type("t", "backend", &mut errors),
            Some(ServiceType::Backend)
        );
        assert_eq!(
            parse_service_type("t", "other:worker", &mut errors),
            Some(ServiceType::Other("worker".to_owned()))
        );
        assert!(parse_service_type("t", "banana", &mut errors).is_none());
        assert!(errors.iter().any(|e| e.location == "t"));

        let mut errors = Vec::new();
        assert_eq!(
            parse_capability("t", "serve_http", &mut errors),
            Some(ServiceCapability::ServeHttp)
        );
        assert!(parse_capability("t", "teleport", &mut errors).is_none());
    }
}
