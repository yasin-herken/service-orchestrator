//! The Maven build provider.
//!
//! Maven is the primary build system of the current backend services
//! (`AGENTS.md` §4), so this is the reference implementation of
//! [`BuildProvider`]. It is still fully build-system neutral from the outside:
//! it declares its own capabilities, resolves its own executable or wrapper, and
//! emits a structured [`BuildPlan`].
//!
//! # Wrapper resolution
//!
//! A Maven project usually ships the Maven Wrapper (`mvnw`), which pins the
//! Maven version and lets a developer build without a system-wide install. The
//! provider prefers `./mvnw` when it exists in the module directory and falls
//! back to `mvn` from `PATH` otherwise. This is the whole of the provider's
//! filesystem observation, and it is asked through the [`BuildHost`] so tests
//! can control it.
//!
//! # Capabilities
//!
//! Maven supports [`Clean`](BuildOperation::Clean), [`Build`](BuildOperation::Build),
//! [`Test`](BuildOperation::Test), and
//! [`PublishLocalArtifact`](BuildOperation::PublishLocalArtifact). It does not
//! declare [`InstallDependencies`](BuildOperation::InstallDependencies): Maven's
//! `install` phase *publishes* the module to the local repository, which is a
//! different operation from resolving dependencies, so conflating them would
//! hide the distinction the operation vocabulary exists to express.

use service_orchestrator_application::{BuildError, ResolvedBuildTarget};
use service_orchestrator_domain::{BuildOperation, BuildSystem};

use crate::plan::{BuildPlan, EnvironmentRequirement, ExpectedOutput, ExpectedOutputKind};
use crate::provider::{plan_from_command, resolve_plan, BuildHost, BuildProvider};

/// The Maven Wrapper script, preferred over a system-wide Maven.
const WRAPPER: &str = "mvnw";

/// The system-wide Maven executable, resolved from `PATH`.
const MAVEN: &str = "mvn";

/// The Maven build provider.
#[derive(Clone, Copy, Debug, Default)]
pub struct MavenBuildProvider;

impl MavenBuildProvider {
    /// Creates the provider.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// The Maven goals for a build-system-neutral operation.
    ///
    /// `-B` runs Maven in batch mode so it never waits for an interactive
    /// prompt an interface cannot answer (`AGENTS.md` §9).
    fn goals(operation: BuildOperation) -> Option<&'static [&'static str]> {
        match operation {
            BuildOperation::Clean => Some(&["-B", "clean"]),
            BuildOperation::Build => Some(&["-B", "package"]),
            BuildOperation::Test => Some(&["-B", "test"]),
            BuildOperation::PublishLocalArtifact => Some(&["-B", "install"]),
            BuildOperation::InstallDependencies => None,
        }
    }

    /// The environment Maven prefers.
    ///
    /// `JAVA_HOME` is optional: a Maven build works when `java` is on `PATH`,
    /// so its absence is recorded but never blocks the build.
    fn environment() -> Vec<EnvironmentRequirement> {
        vec![EnvironmentRequirement::optional("JAVA_HOME")]
    }

    /// The outputs Maven conventionally produces for `operation`.
    fn expected_outputs(operation: BuildOperation) -> Vec<ExpectedOutput> {
        match operation {
            BuildOperation::Build | BuildOperation::PublishLocalArtifact => {
                vec![ExpectedOutput::new("target", ExpectedOutputKind::Directory)]
            }
            BuildOperation::Clean | BuildOperation::Test | BuildOperation::InstallDependencies => {
                Vec::new()
            }
        }
    }

    /// Resolves the Maven executable or wrapper for `target`.
    fn resolve_program(target: &ResolvedBuildTarget, host: &dyn BuildHost) -> String {
        if host.file_exists(&target.path, WRAPPER) {
            format!("./{WRAPPER}")
        } else {
            MAVEN.to_owned()
        }
    }
}

impl BuildProvider for MavenBuildProvider {
    fn system(&self) -> BuildSystem {
        BuildSystem::Maven
    }

    fn capabilities(&self) -> &[BuildOperation] {
        &[
            BuildOperation::Clean,
            BuildOperation::Build,
            BuildOperation::Test,
            BuildOperation::PublishLocalArtifact,
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
        host: &dyn BuildHost,
    ) -> Result<BuildPlan, BuildError> {
        resolve_plan(
            target,
            || {
                let goals =
                    Self::goals(target.operation).ok_or(BuildError::UnsupportedOperation {
                        system: BuildSystem::Maven,
                        operation: target.operation,
                    })?;
                Ok(BuildPlan::new(
                    BuildSystem::Maven,
                    target.operation,
                    Self::resolve_program(target, host),
                    target.path.clone(),
                )
                .with_args(goals.iter().copied())
                .with_environment(EnvironmentRequirement::optional("JAVA_HOME"))
                .with_expected_output(ExpectedOutput::new("target", ExpectedOutputKind::Directory)))
            },
            |command| {
                plan_from_command(
                    target,
                    command,
                    Self::environment(),
                    Self::expected_outputs(target.operation),
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{AbsolutePath, WorkspaceId};

    /// A host whose answers the test controls.
    struct FakeHost {
        wrapper: bool,
    }

    impl FakeHost {
        fn without_wrapper() -> Self {
            Self { wrapper: false }
        }

        fn with_wrapper() -> Self {
            Self { wrapper: true }
        }
    }

    impl BuildHost for FakeHost {
        fn file_exists(&self, _directory: &AbsolutePath, name: &str) -> bool {
            self.wrapper && name == WRAPPER
        }

        fn environment_has(&self, _name: &str) -> bool {
            false
        }
    }

    fn target(operation: BuildOperation) -> ResolvedBuildTarget {
        ResolvedBuildTarget {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            name: "auth-service".to_owned(),
            build_system: BuildSystem::Maven,
            operation,
            path: AbsolutePath::new("/w/acme/backend/auth-service").unwrap(),
            workspace_root: AbsolutePath::new("/w/acme").unwrap(),
            command: None,
        }
    }

    #[test]
    fn declares_its_capabilities() {
        let provider = MavenBuildProvider::new();
        assert_eq!(provider.system(), BuildSystem::Maven);
        assert!(provider.supports(BuildOperation::Clean));
        assert!(provider.supports(BuildOperation::Build));
        assert!(provider.supports(BuildOperation::Test));
        assert!(provider.supports(BuildOperation::PublishLocalArtifact));
        assert!(!provider.supports(BuildOperation::InstallDependencies));
    }

    #[test]
    fn prefers_the_wrapper_when_it_exists() {
        let provider = MavenBuildProvider::new();
        let plan = provider
            .build_plan(&target(BuildOperation::Build), &FakeHost::with_wrapper())
            .unwrap();
        assert_eq!(plan.program, "./mvnw");
    }

    #[test]
    fn falls_back_to_the_system_maven() {
        let provider = MavenBuildProvider::new();
        let plan = provider
            .build_plan(&target(BuildOperation::Build), &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.program, "mvn");
    }

    #[test]
    fn maps_operations_to_goals() {
        let provider = MavenBuildProvider::new();
        let host = FakeHost::without_wrapper();
        for (operation, expected) in [
            (BuildOperation::Clean, vec!["-B", "clean"]),
            (BuildOperation::Build, vec!["-B", "package"]),
            (BuildOperation::Test, vec!["-B", "test"]),
            (BuildOperation::PublishLocalArtifact, vec!["-B", "install"]),
        ] {
            let plan = provider.build_plan(&target(operation), &host).unwrap();
            assert_eq!(plan.args, expected);
        }
    }

    #[test]
    fn rejects_an_unsupported_operation() {
        let provider = MavenBuildProvider::new();
        let error = provider
            .build_plan(
                &target(BuildOperation::InstallDependencies),
                &FakeHost::without_wrapper(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "BUILD_UNSUPPORTED_OPERATION");
    }

    #[test]
    fn a_build_declares_the_target_directory() {
        let provider = MavenBuildProvider::new();
        let plan = provider
            .build_plan(&target(BuildOperation::Build), &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.expected_outputs.len(), 1);
        assert_eq!(plan.expected_outputs[0].path, "target");
    }

    #[test]
    fn planning_does_not_spawn_a_process() {
        // The program does not exist anywhere; if planning tried to run it, this
        // would fail. It cannot, because the provider is a pure mapping.
        let target = ResolvedBuildTarget {
            command: Some(service_orchestrator_application::ResolvedBuildCommand {
                program: "definitely-not-a-real-build-tool".to_owned(),
                args: vec!["package".to_owned()],
                working_dir: None,
            }),
            ..target(BuildOperation::Build)
        };
        let plan = MavenBuildProvider::new()
            .build_plan(&target, &FakeHost::without_wrapper())
            .unwrap();
        assert_eq!(plan.program, "definitely-not-a-real-build-tool");
    }
}
