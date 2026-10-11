//! The npm build provider.
//!
//! npm is the second build system, used by the frontend services. It exists to
//! prove the abstraction is real: its capabilities, its executable, and its
//! plans differ from Maven's, yet nothing in the manager, the task engine, or
//! the application changes to accommodate it.
//!
//! # Capabilities
//!
//! npm supports [`Build`](BuildOperation::Build), [`Test`](BuildOperation::Test),
//! and [`InstallDependencies`](BuildOperation::InstallDependencies). It does not
//! declare [`Clean`](BuildOperation::Clean) — there is no standard npm clean
//! goal — or [`PublishLocalArtifact`](BuildOperation::PublishLocalArtifact),
//! which is a Maven-repository concept. An unsupported request is rejected with
//! a structured error rather than being approximated.
//!
//! # No expected outputs
//!
//! Where a build writes its output (`dist/`, `build/`, …) is project-specific
//! and not declared by npm itself, so the provider records no expected output
//! rather than guessing. This is exactly the kind of provider-specific judgment
//! the [`BuildProvider`](crate::provider::BuildProvider) abstraction exists to
//! hold.

use service_orchestrator_application::{BuildError, ResolvedBuildTarget};
use service_orchestrator_domain::{BuildOperation, BuildSystem};

use crate::plan::{BuildPlan, EnvironmentRequirement, ExpectedOutput};
use crate::provider::{plan_from_command, resolve_plan, BuildHost, BuildProvider};

/// The npm executable, resolved from `PATH`.
const NPM: &str = "npm";

/// The npm build provider.
#[derive(Clone, Copy, Debug, Default)]
pub struct NpmBuildProvider;

impl NpmBuildProvider {
    /// Creates the provider.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// The npm arguments for a build-system-neutral operation.
    fn args(operation: BuildOperation) -> Option<&'static [&'static str]> {
        match operation {
            BuildOperation::Build => Some(&["run", "build"]),
            BuildOperation::Test => Some(&["test"]),
            BuildOperation::InstallDependencies => Some(&["install"]),
            BuildOperation::Clean | BuildOperation::PublishLocalArtifact => None,
        }
    }

    /// npm has no environment variable it strictly requires; it needs `node`
    /// and `npm` on `PATH`, which are executables rather than variables.
    fn environment() -> Vec<EnvironmentRequirement> {
        Vec::new()
    }

    /// npm output locations are project-specific, so none are declared.
    fn expected_outputs() -> Vec<ExpectedOutput> {
        Vec::new()
    }
}

impl BuildProvider for NpmBuildProvider {
    fn system(&self) -> BuildSystem {
        BuildSystem::Npm
    }

    fn capabilities(&self) -> &[BuildOperation] {
        &[
            BuildOperation::Build,
            BuildOperation::Test,
            BuildOperation::InstallDependencies,
        ]
    }

    fn validate(
        &self,
        target: &ResolvedBuildTarget,
        _host: &dyn BuildHost,
    ) -> Result<(), BuildError> {
        crate::provider::validate_inside_workspace(target)
    }

    fn build_plan(
        &self,
        target: &ResolvedBuildTarget,
        _host: &dyn BuildHost,
    ) -> Result<BuildPlan, BuildError> {
        resolve_plan(
            target,
            || {
                let args =
                    Self::args(target.operation).ok_or(BuildError::UnsupportedOperation {
                        system: BuildSystem::Npm,
                        operation: target.operation,
                    })?;
                Ok(
                    BuildPlan::new(BuildSystem::Npm, target.operation, NPM, target.path.clone())
                        .with_args(args.iter().copied()),
                )
            },
            |command| {
                plan_from_command(
                    target,
                    command,
                    Self::environment(),
                    Self::expected_outputs(),
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::SystemBuildHost;
    use service_orchestrator_domain::{AbsolutePath, WorkspaceId};

    fn target(operation: BuildOperation) -> ResolvedBuildTarget {
        ResolvedBuildTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "web-ui".to_owned(),
            build_system: BuildSystem::Npm,
            operation,
            path: AbsolutePath::new("/w/acme/frontend/web-ui").unwrap(),
            workspace_root: AbsolutePath::new("/w/acme").unwrap(),
            command: None,
        }
    }

    #[test]
    fn declares_its_capabilities() {
        let provider = NpmBuildProvider::new();
        assert_eq!(provider.system(), BuildSystem::Npm);
        assert!(provider.supports(BuildOperation::Build));
        assert!(provider.supports(BuildOperation::Test));
        assert!(provider.supports(BuildOperation::InstallDependencies));
        assert!(!provider.supports(BuildOperation::Clean));
        assert!(!provider.supports(BuildOperation::PublishLocalArtifact));
    }

    #[test]
    fn maps_operations_to_arguments() {
        let provider = NpmBuildProvider::new();
        let host = SystemBuildHost::new();
        for (operation, expected) in [
            (BuildOperation::Build, vec!["run", "build"]),
            (BuildOperation::Test, vec!["test"]),
            (BuildOperation::InstallDependencies, vec!["install"]),
        ] {
            let plan = provider.build_plan(&target(operation), &host).unwrap();
            assert_eq!(plan.program, "npm");
            assert_eq!(plan.args, expected);
        }
    }

    #[test]
    fn rejects_an_unsupported_operation() {
        let provider = NpmBuildProvider::new();
        for operation in [BuildOperation::Clean, BuildOperation::PublishLocalArtifact] {
            let error = provider
                .build_plan(&target(operation), &SystemBuildHost::new())
                .unwrap_err();
            assert_eq!(error.code(), "BUILD_UNSUPPORTED_OPERATION");
        }
    }

    #[test]
    fn declares_no_expected_outputs() {
        let plan = NpmBuildProvider::new()
            .build_plan(&target(BuildOperation::Build), &SystemBuildHost::new())
            .unwrap();
        assert!(plan.expected_outputs.is_empty());
    }
}
