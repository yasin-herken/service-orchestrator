//! Read-only repository inspection and project-configuration detection.
//!
//! Both use cases are read-only and go through the
//! [`RepositoryInspector`](crate::ports::RepositoryInspector) boundary: the
//! application core never touches the filesystem itself
//! (`tasks/023.md` §17, §18). Detection is a *suggestion*; the service wizard
//! reviews and edits it before anything is validated and saved.

use service_orchestrator_domain::{AbsolutePath, Permission};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::ports::{
    DetectedRuntime, ProjectConfigurationSuggestion, RepositoryInspection, SuggestedCommand,
};

impl Application {
    /// Inspects a local repository directory, read-only.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::InvalidPath`] when `path` is not an absolute
    /// path, and [`ApplicationError::RepositoryInspection`] when inspection
    /// could not be attempted (`tasks/023.md` §17).
    pub fn inspect_repository(&self, path: &str) -> Result<RepositoryInspection, ApplicationError> {
        self.authorize(Permission::Read)?;
        let path = absolute_inspection_path(path)?;
        self.repository_inspector()
            .inspect(&path)
            .map_err(ApplicationError::inspection)
    }

    /// Detects a suggested service configuration from a repository.
    ///
    /// The suggestion is derived from a fresh [`inspect_repository`](Self::inspect_repository)
    /// result by a pure mapping, so it never executes a project script and never
    /// commits anything (`tasks/023.md` §18, §19, §20).
    ///
    /// # Errors
    ///
    /// As [`inspect_repository`](Self::inspect_repository).
    pub fn detect_project_configuration(
        &self,
        path: &str,
    ) -> Result<ProjectConfigurationSuggestion, ApplicationError> {
        let inspection = self.inspect_repository(path)?;
        Ok(suggest_configuration(inspection))
    }
}

/// Validates that `path` is an absolute path suitable for inspection.
fn absolute_inspection_path(path: &str) -> Result<String, ApplicationError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ApplicationError::invalid_path(
            "an inspection path must not be empty",
        ));
    }
    // Reject a path that tries to escape through a `..` segment before it ever
    // reaches the inspector (`tasks/023.md` §46).
    if trimmed.split(['/', '\\']).any(|segment| segment == "..") {
        return Err(ApplicationError::invalid_path(format!(
            "'{trimmed}' must not contain '..' path segments"
        )));
    }
    let absolute = AbsolutePath::new(trimmed).map_err(|error| {
        ApplicationError::invalid_path(format!("'{trimmed}' is not an absolute path: {error}"))
    })?;
    Ok(absolute.as_str().to_owned())
}

/// Derives a service-configuration suggestion from an inspection.
///
/// The mapping is intentionally conservative: it only proposes a command when
/// the repository clearly supports one, and it warns rather than guessing when
/// a detected technology has no execution provider.
fn suggest_configuration(fresh: RepositoryInspection) -> ProjectConfigurationSuggestion {
    let mut warnings = Vec::new();

    if !fresh.exists {
        warnings.push(format!("'{}' does not exist on this machine", fresh.path));
    } else if !fresh.is_directory {
        warnings.push(format!("'{}' is not a directory", fresh.path));
    }

    // Gradle is detected but has no execution provider (`tasks/023.md` §19).
    for system in &fresh.build_systems {
        if !system.availability.is_operational() {
            warnings.push(format!(
                "Detected: {} — execution provider not installed.",
                system.label
            ));
        }
    }

    let maven = fresh
        .build_systems
        .iter()
        .find(|system| system.system == "maven");
    let npm = fresh
        .build_systems
        .iter()
        .find(|system| system.system == "npm");

    if maven.is_some() && npm.is_some() {
        warnings.push(
            "Both Maven and npm were detected; the suggestion prefers Maven. Adjust it if this \
             is a full-stack directory."
                .to_owned(),
        );
    }

    let (build_system, service_type, build_command, start_command) = if maven.is_some() {
        let uses_wrapper = fresh
            .maven
            .as_ref()
            .is_some_and(|metadata| metadata.wrapper_available);
        let program = if uses_wrapper { "./mvnw" } else { "mvn" };
        let build = SuggestedCommand {
            program: program.to_owned(),
            args: vec!["clean".to_owned(), "install".to_owned()],
        };
        let spring_boot = fresh
            .maven
            .as_ref()
            .is_some_and(|metadata| metadata.spring_boot);
        let start = spring_boot.then(|| SuggestedCommand {
            program: program.to_owned(),
            args: vec!["spring-boot:run".to_owned()],
        });
        (
            Some("maven".to_owned()),
            Some("backend".to_owned()),
            Some(build),
            start,
        )
    } else if npm.is_some() {
        let scripts = fresh
            .node
            .as_ref()
            .map(|node| node.scripts.as_slice())
            .unwrap_or(&[]);
        let build = scripts
            .iter()
            .any(|script| script == "build")
            .then(|| SuggestedCommand {
                program: "npm".to_owned(),
                args: vec!["run".to_owned(), "build".to_owned()],
            });
        let start = if scripts.iter().any(|script| script == "start") {
            Some(SuggestedCommand {
                program: "npm".to_owned(),
                args: vec!["run".to_owned(), "start".to_owned()],
            })
        } else if scripts.iter().any(|script| script == "dev") {
            Some(SuggestedCommand {
                program: "npm".to_owned(),
                args: vec!["run".to_owned(), "dev".to_owned()],
            })
        } else {
            None
        };
        (
            Some("npm".to_owned()),
            Some("frontend".to_owned()),
            build,
            start,
        )
    } else {
        (None, None, None, None)
    };

    if build_system.is_none() && fresh.is_directory {
        warnings.push(
            "No supported build system was detected; configure the build manually.".to_owned(),
        );
    }
    if build_system.is_some() && start_command.is_none() {
        warnings.push(
            "No start command could be inferred; configure one if this service should run."
                .to_owned(),
        );
    }

    ProjectConfigurationSuggestion {
        path: fresh.path.clone(),
        service_type,
        build_system,
        runtimes: suggested_runtimes(&fresh),
        build_command,
        start_command,
        integrations: fresh.integrations.clone(),
        warnings,
        inspection: fresh,
    }
}

/// Returns the runtimes worth pre-filling for a suggestion.
fn suggested_runtimes(inspection: &RepositoryInspection) -> Vec<DetectedRuntime> {
    inspection.runtimes.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{
        DetectedBuildSystem, DetectedIntegration, DetectedRuntime, MavenMetadata, NodeMetadata,
        ProviderAvailability, RepositoryInspection,
    };

    fn base_inspection() -> RepositoryInspection {
        RepositoryInspection {
            path: "/tmp/project".to_owned(),
            exists: true,
            is_directory: true,
            is_git_repository: true,
            branch: Some("main".to_owned()),
            remote: None,
            project_files: vec!["pom.xml".to_owned()],
            build_systems: Vec::new(),
            runtimes: Vec::new(),
            integrations: Vec::new(),
            maven: None,
            node: None,
            notes: Vec::new(),
        }
    }

    #[test]
    fn a_gradle_detection_is_reported_as_not_installed() {
        let mut inspection = base_inspection();
        inspection.build_systems = vec![DetectedBuildSystem {
            system: "gradle".to_owned(),
            label: "Gradle".to_owned(),
            availability: ProviderAvailability::ProviderNotInstalled,
            evidence: vec!["build.gradle".to_owned()],
            note: None,
        }];
        let suggestion = suggest_configuration(inspection);
        assert!(suggestion.build_system.is_none());
        assert!(suggestion
            .warnings
            .iter()
            .any(|warning| warning.contains("execution provider not installed")));
    }

    #[test]
    fn maven_with_spring_boot_suggests_a_wrapper_build_and_start() {
        let mut inspection = base_inspection();
        inspection.maven = Some(MavenMetadata {
            artifact_id: Some("auth-service".to_owned()),
            wrapper_available: true,
            spring_boot: true,
            ..MavenMetadata::default()
        });
        inspection.build_systems = vec![DetectedBuildSystem {
            system: "maven".to_owned(),
            label: "Maven".to_owned(),
            availability: ProviderAvailability::Available,
            evidence: vec!["pom.xml".to_owned(), "mvnw".to_owned()],
            note: None,
        }];
        let suggestion = suggest_configuration(inspection);
        assert_eq!(suggestion.build_system.as_deref(), Some("maven"));
        assert_eq!(suggestion.service_type.as_deref(), Some("backend"));
        let build = suggestion
            .build_command
            .expect("a build command is suggested");
        assert_eq!(build.program, "./mvnw");
        let start = suggestion
            .start_command
            .expect("a start command is suggested");
        assert_eq!(start.args, vec!["spring-boot:run".to_owned()]);
    }

    #[test]
    fn npm_with_a_dev_script_suggests_a_start_command() {
        let mut inspection = base_inspection();
        inspection.build_systems = vec![DetectedBuildSystem {
            system: "npm".to_owned(),
            label: "npm".to_owned(),
            availability: ProviderAvailability::Available,
            evidence: vec!["package.json".to_owned()],
            note: None,
        }];
        inspection.node = Some(NodeMetadata {
            scripts: vec!["build".to_owned(), "dev".to_owned()],
            react: true,
            ..NodeMetadata::default()
        });
        inspection.integrations = vec![DetectedIntegration {
            kind: "react".to_owned(),
            label: "React".to_owned(),
            evidence: vec!["package.json".to_owned()],
        }];
        let suggestion = suggest_configuration(inspection);
        assert_eq!(suggestion.build_system.as_deref(), Some("npm"));
        assert_eq!(suggestion.service_type.as_deref(), Some("frontend"));
        assert_eq!(
            suggestion.build_command.expect("build").args,
            vec!["run".to_owned(), "build".to_owned()]
        );
        assert_eq!(
            suggestion.start_command.expect("start").args,
            vec!["run".to_owned(), "dev".to_owned()]
        );
    }

    #[test]
    fn a_directory_without_a_build_system_warns() {
        let suggestion = suggest_configuration(base_inspection());
        assert!(suggestion.build_system.is_none());
        assert!(suggestion
            .warnings
            .iter()
            .any(|warning| warning.contains("No supported build system")));
    }

    #[test]
    fn a_missing_directory_is_reported_as_a_warning() {
        let mut inspection = base_inspection();
        inspection.exists = false;
        inspection.is_directory = false;
        let suggestion = suggest_configuration(inspection);
        assert!(suggestion
            .warnings
            .iter()
            .any(|warning| warning.contains("does not exist")));
    }

    #[test]
    fn runtimes_are_carried_through_unchanged() {
        let mut inspection = base_inspection();
        inspection.runtimes = vec![DetectedRuntime {
            kind: "java".to_owned(),
            label: "Java".to_owned(),
            required_version: Some("21".to_owned()),
            evidence: vec!["pom.xml".to_owned()],
        }];
        let suggestion = suggest_configuration(inspection);
        assert_eq!(suggestion.runtimes.len(), 1);
        assert_eq!(
            suggestion.runtimes[0].required_version.as_deref(),
            Some("21")
        );
    }
}
