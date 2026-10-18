//! Template variables: the small, explicit substitution system used by service
//! templates (`tasks/032.md` §3–§4).
//!
//! A service template is a set of *defaults* for the validated service model.
//! Some of its values depend on the concrete repository the template is being
//! resolved for — the service name, the repository directory, the port assigned
//! during a bulk import — so those values may contain a fixed, validated set of
//! `${...}` placeholders.
//!
//! The system is deliberately not an expression language:
//!
//! - the variable set is closed and small;
//! - substitution is a single textual replacement pass;
//! - an unknown, empty, or unterminated placeholder is a hard error;
//! - a *bare* `$` that is not followed by `{` is ordinary text.
//!
//! Resolution never decides anything by itself: a resolved value still flows
//! through the normal configuration validation (path checks included), so a
//! variable cannot bypass path validation (`tasks/032.md` §4).

use std::fmt;

use serde::{Deserialize, Serialize};

/// One of the supported template variables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TemplateVariable {
    /// The id of the service (or library) being created.
    ServiceName,
    /// The repository directory name.
    RepositoryName,
    /// The repository's workspace-relative path.
    RepositoryPath,
    /// The absolute workspace root.
    WorkspaceRoot,
    /// The workspace-relative path currently being resolved.
    ///
    /// For a repository-level value this equals
    /// [`TemplateVariable::RepositoryPath`]; it exists as a separate name so a
    /// future nested resolution (for example a module directory inside a
    /// repository) can supply a narrower path without changing the vocabulary.
    RelativePath,
    /// The port assigned to the service during the import or override flow.
    ///
    /// This is only available once a port has actually been assigned; a value
    /// that references it fails to resolve otherwise.
    Port,
}

impl TemplateVariable {
    /// Every supported variable, in documentation order.
    pub const ALL: [TemplateVariable; 6] = [
        TemplateVariable::ServiceName,
        TemplateVariable::RepositoryName,
        TemplateVariable::RepositoryPath,
        TemplateVariable::WorkspaceRoot,
        TemplateVariable::RelativePath,
        TemplateVariable::Port,
    ];

    /// The variable name, without the `${` and `}` delimiters.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ServiceName => "serviceName",
            Self::RepositoryName => "repositoryName",
            Self::RepositoryPath => "repositoryPath",
            Self::WorkspaceRoot => "workspaceRoot",
            Self::RelativePath => "relativePath",
            Self::Port => "port",
        }
    }

    /// The full placeholder, for example `${serviceName}`.
    #[must_use]
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::ServiceName => "${serviceName}",
            Self::RepositoryName => "${repositoryName}",
            Self::RepositoryPath => "${repositoryPath}",
            Self::WorkspaceRoot => "${workspaceRoot}",
            Self::RelativePath => "${relativePath}",
            Self::Port => "${port}",
        }
    }

    /// Parses a variable name from inside a placeholder.
    ///
    /// Matching is exact and case-sensitive: `serviceName` is a variable,
    /// `servicename` is not. Returns `None` for an unknown name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|var| var.name() == name)
    }
}

impl fmt::Display for TemplateVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A template text error: an invalid placeholder.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// A placeholder names a variable that does not exist.
    #[error(
        "unknown template variable '${{{name}}}'; supported variables are: serviceName, \
         repositoryName, repositoryPath, workspaceRoot, relativePath, port"
    )]
    UnknownVariable {
        /// The unrecognized variable name.
        name: String,
    },
    /// A placeholder is empty (`${}`).
    #[error("empty template placeholder '${{}}'")]
    EmptyPlaceholder,
    /// A `${` is not closed by a `}`.
    #[error("unterminated template placeholder; every '${{' needs a matching '}}'")]
    UnterminatedPlaceholder,
}

/// Returns every variable referenced by `text`, in order of appearance.
///
/// The text is fully parsed: an unknown, empty, or unterminated placeholder is
/// an error rather than being skipped.
///
/// # Errors
///
/// Returns a [`TemplateError`] for the first invalid placeholder.
pub fn template_variables(text: &str) -> Result<Vec<TemplateVariable>, TemplateError> {
    let mut variables = Vec::new();
    for_each_placeholder(text, |name| {
        let variable = TemplateVariable::parse(name).ok_or_else(|| {
            if name.is_empty() {
                TemplateError::EmptyPlaceholder
            } else {
                TemplateError::UnknownVariable {
                    name: name.to_owned(),
                }
            }
        })?;
        variables.push(variable);
        Ok(())
    })?;
    Ok(variables)
}

/// Validates that every `${...}` placeholder in `text` is supported.
///
/// # Errors
///
/// Returns a [`TemplateError`] for the first invalid placeholder.
pub fn validate_template_text(text: &str) -> Result<(), TemplateError> {
    template_variables(text).map(|_| ())
}

/// Returns `true` when `text` references the `${port}` variable.
///
/// # Errors
///
/// Returns a [`TemplateError`] when the text contains an invalid placeholder.
pub fn requires_port(text: &str) -> Result<bool, TemplateError> {
    Ok(template_variables(text)?.contains(&TemplateVariable::Port))
}

/// The values a template is resolved against.
///
/// The context is assembled by the caller from the concrete repository being
/// imported and the port assigned to it; it is plain data, not a lookup into
/// configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateContext {
    /// The service (or library) id being created.
    pub service_name: String,
    /// The repository directory name.
    pub repository_name: String,
    /// The repository's workspace-relative path.
    pub repository_path: String,
    /// The absolute workspace root.
    pub workspace_root: String,
    /// The workspace-relative path currently being resolved.
    pub relative_path: String,
    /// The assigned port, when one exists.
    pub port: Option<u16>,
}

impl TemplateContext {
    /// Resolves one variable to its textual value.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::UnknownVariable`] when the variable carries no
    /// value in this context — the only case today is `${port}` without an
    /// assigned port.
    pub fn value(&self, variable: TemplateVariable) -> Result<String, TemplateError> {
        match variable {
            TemplateVariable::ServiceName => Ok(self.service_name.clone()),
            TemplateVariable::RepositoryName => Ok(self.repository_name.clone()),
            TemplateVariable::RepositoryPath => Ok(self.repository_path.clone()),
            TemplateVariable::WorkspaceRoot => Ok(self.workspace_root.clone()),
            TemplateVariable::RelativePath => Ok(self.relative_path.clone()),
            TemplateVariable::Port => self.port.map(|port| port.to_string()).ok_or_else(|| {
                // Reuse the unknown-variable error with an actionable message:
                // the variable exists but has no value here.
                TemplateError::UnknownVariable {
                    name: "port (no port has been assigned yet)".to_owned(),
                }
            }),
        }
    }
}

/// Substitutes every placeholder in `text` with its value from `context`.
///
/// Substitution is a single pass: a value that itself contains `${...}` is
/// never re-expanded.
///
/// # Errors
///
/// Returns a [`TemplateError`] for the first invalid or unresolvable
/// placeholder.
pub fn resolve_template_text(
    text: &str,
    context: &TemplateContext,
) -> Result<String, TemplateError> {
    let mut resolved = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        resolved.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(TemplateError::UnterminatedPlaceholder);
        };
        let name = &after[..end];
        let variable = TemplateVariable::parse(name).ok_or_else(|| {
            if name.is_empty() {
                TemplateError::EmptyPlaceholder
            } else {
                TemplateError::UnknownVariable {
                    name: name.to_owned(),
                }
            }
        })?;
        resolved.push_str(&context.value(variable)?);
        rest = &after[end + 1..];
    }
    resolved.push_str(rest);
    Ok(resolved)
}

/// Calls `visit` with every placeholder name in `text`.
fn for_each_placeholder(
    text: &str,
    mut visit: impl FnMut(&str) -> Result<(), TemplateError>,
) -> Result<(), TemplateError> {
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(TemplateError::UnterminatedPlaceholder);
        };
        visit(&after[..end])?;
        rest = &after[end + 1..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> TemplateContext {
        TemplateContext {
            service_name: "auth-service".to_owned(),
            repository_name: "auth".to_owned(),
            repository_path: "backend/auth".to_owned(),
            workspace_root: "/Users/dev/project-x".to_owned(),
            relative_path: "backend/auth".to_owned(),
            port: Some(8081),
        }
    }

    #[test]
    fn parses_every_supported_variable() {
        for variable in TemplateVariable::ALL {
            assert_eq!(TemplateVariable::parse(variable.name()), Some(variable));
            assert_eq!(variable.placeholder(), format!("${{{}}}", variable.name()));
        }
        assert_eq!(TemplateVariable::parse("service"), None);
        assert_eq!(TemplateVariable::parse("ServiceName"), None);
    }

    #[test]
    fn collects_variables_in_order() {
        let text = "${relativePath}/mvnw -Dname=${serviceName} --port ${port}";
        assert_eq!(
            template_variables(text).unwrap(),
            vec![
                TemplateVariable::RelativePath,
                TemplateVariable::ServiceName,
                TemplateVariable::Port,
            ]
        );
    }

    #[test]
    fn rejects_unknown_empty_and_unterminated_placeholders() {
        assert_eq!(
            validate_template_text("${bogus}"),
            Err(TemplateError::UnknownVariable {
                name: "bogus".to_owned()
            })
        );
        assert_eq!(
            validate_template_text("${}"),
            Err(TemplateError::EmptyPlaceholder)
        );
        assert_eq!(
            validate_template_text("prefix ${serviceName"),
            Err(TemplateError::UnterminatedPlaceholder)
        );
    }

    #[test]
    fn a_bare_dollar_is_ordinary_text() {
        assert_eq!(validate_template_text("costs $5, or `$HOME`"), Ok(()));
        assert_eq!(
            resolve_template_text("costs $5", &context()).unwrap(),
            "costs $5"
        );
    }

    #[test]
    fn resolves_placeholders_once() {
        let resolved = resolve_template_text(
            "${repositoryPath}/mvnw -Dservice=${serviceName} -Droot=${workspaceRoot}",
            &context(),
        )
        .unwrap();
        assert_eq!(
            resolved,
            "backend/auth/mvnw -Dservice=auth-service -Droot=/Users/dev/project-x"
        );
    }

    #[test]
    fn substitution_is_not_recursive() {
        let mut context = context();
        context.service_name = "${workspaceRoot}".to_owned();
        let resolved = resolve_template_text("name=${serviceName}", &context).unwrap();
        assert_eq!(resolved, "name=${workspaceRoot}");
    }

    #[test]
    fn port_requires_an_assigned_port() {
        let mut context_without_port = context();
        context_without_port.port = None;
        assert_eq!(
            resolve_template_text("http://localhost:${port}/health", &context_without_port)
                .unwrap_err(),
            TemplateError::UnknownVariable {
                name: "port (no port has been assigned yet)".to_owned()
            }
        );
        let resolved =
            resolve_template_text("http://localhost:${port}/health", &context()).unwrap();
        assert_eq!(resolved, "http://localhost:8081/health");
    }

    #[test]
    fn requires_port_detects_the_variable() {
        assert!(requires_port("http://localhost:${port}/health").unwrap());
        assert!(!requires_port("http://localhost:8081/health").unwrap());
    }
}
