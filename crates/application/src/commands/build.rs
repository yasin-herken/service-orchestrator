//! Build target resolution.
//!
//! The application never runs a build tool. These helpers resolve a validated
//! configuration into a workspace-aware [`ResolvedBuildTarget`] that a build
//! task carries to the engine. They are shared by the single-entity build
//! commands and the workspace/environment workflows, so resolution is never
//! duplicated (`AGENTS.md` §21).
//!
//! The build *system* is configuration-driven: it is inferred by the
//! configuration layer and validated there, so a target with an unrecognized
//! system is reported as a precise application error before any task is
//! scheduled — an unimplemented system never appears operational.

use service_orchestrator_config::ConfiguredWorkspace;
use service_orchestrator_domain::{
    AbsolutePath, BuildOperation, BuildSystem, CommandKind, CommandSet, Library, RelativePath,
    Service,
};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::ports::{ResolvedBuildCommand, ResolvedBuildTarget};

impl Application {
    /// Resolves the build target for an operation on a service.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::InvalidOperation`] if the service has no
    /// workspace path or no recognized build system.
    pub(crate) fn service_build_target(
        &self,
        workspace: &ConfiguredWorkspace,
        service: &Service,
        operation: BuildOperation,
    ) -> Result<ResolvedBuildTarget, ApplicationError> {
        let context = BuildTargetContext {
            name: service.id.as_str(),
            path: service.path.as_ref(),
            build_system: service.build_system,
            operation,
            command: configured_command(&service.commands, operation),
            entity: "service",
        };
        self.resolve_build_target(workspace, context)
    }

    /// Resolves the build target for an operation on a library.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::InvalidOperation`] if the library has no
    /// workspace path or no recognized build system.
    pub(crate) fn library_build_target(
        &self,
        workspace: &ConfiguredWorkspace,
        library: &Library,
        operation: BuildOperation,
    ) -> Result<ResolvedBuildTarget, ApplicationError> {
        let context = BuildTargetContext {
            name: library.id.as_str(),
            path: library.path.as_ref(),
            build_system: library.build_system,
            operation,
            command: configured_command(&library.commands, operation),
            entity: "library",
        };
        self.resolve_build_target(workspace, context)
    }

    /// Builds a [`ResolvedBuildTarget`], validating that its working directory
    /// stays inside the workspace.
    fn resolve_build_target(
        &self,
        workspace: &ConfiguredWorkspace,
        context: BuildTargetContext<'_>,
    ) -> Result<ResolvedBuildTarget, ApplicationError> {
        let path = context.path.ok_or_else(|| {
            ApplicationError::invalid_operation(format!(
                "{} '{}' has no workspace path configured",
                context.entity, context.name
            ))
        })?;
        let build_system = context.build_system.ok_or_else(|| {
            ApplicationError::invalid_operation(format!(
                "{} '{}' has no recognized build system; declare `build_system` or a Maven/npm runtime or command",
                context.entity, context.name
            ))
        })?;

        let root = workspace.workspace.root.as_str().trim_end_matches('/');
        let path = AbsolutePath::new(format!("{root}/{}", path.as_str())).map_err(|error| {
            ApplicationError::Build {
                code: "BUILD_INVALID_PATH".to_owned(),
                message: error.to_string(),
            }
        })?;
        let target = ResolvedBuildTarget {
            workspace_id: workspace.workspace.id.clone(),
            name: context.name.to_owned(),
            build_system,
            operation: context.operation,
            path,
            workspace_root: workspace.workspace.root.clone(),
            command: context.command,
        };
        if !target.is_inside_workspace() {
            return Err(ApplicationError::Build {
                code: "BUILD_PATH_OUTSIDE_WORKSPACE".to_owned(),
                message: format!(
                    "build target '{}' resolves to '{}', outside workspace root '{}'",
                    target.name, target.path, target.workspace_root
                ),
            });
        }
        Ok(target)
    }
}

/// The resolved inputs for one build target, before construction.
struct BuildTargetContext<'a> {
    name: &'a str,
    path: Option<&'a RelativePath>,
    build_system: Option<BuildSystem>,
    operation: BuildOperation,
    command: Option<ResolvedBuildCommand>,
    entity: &'static str,
}

/// Returns the configured command preferred for `operation`, if the entity
/// declares one.
///
/// Only the build-system-neutral operations that map onto a declared command
/// kind are considered. [`CommandKind::Install`] covers both installing
/// dependencies and publishing a local artifact (`install` means "install
/// dependencies or artifacts"; for Maven its `install` phase publishes to the
/// local repository), so both map onto it. `Clean` has no dedicated command kind
/// and always uses the provider's default plan.
fn configured_command(
    commands: &CommandSet,
    operation: BuildOperation,
) -> Option<ResolvedBuildCommand> {
    let kind = match operation {
        BuildOperation::Build => CommandKind::Build,
        BuildOperation::Test => CommandKind::Test,
        BuildOperation::InstallDependencies | BuildOperation::PublishLocalArtifact => {
            CommandKind::Install
        }
        BuildOperation::Clean => return None,
    };
    commands.get(&kind).map(|spec| ResolvedBuildCommand {
        program: spec.program.clone(),
        args: spec.args.clone(),
        working_dir: spec.working_dir.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::CommandSpec;

    #[test]
    fn only_supported_operations_use_a_declared_command() {
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Build,
            CommandSpec::new("./mvnw", ["clean", "install"]).unwrap(),
        );

        let build = configured_command(&commands, BuildOperation::Build).unwrap();
        assert_eq!(build.program, "./mvnw");
        assert_eq!(build.args, vec!["clean", "install"]);

        assert!(configured_command(&commands, BuildOperation::Clean).is_none());
        assert!(configured_command(&commands, BuildOperation::PublishLocalArtifact).is_none());
    }

    #[test]
    fn install_and_test_operations_map_to_their_kinds() {
        let mut commands = CommandSet::new();
        commands.set(
            CommandKind::Install,
            CommandSpec::new("npm", ["install"]).unwrap(),
        );
        commands.set(
            CommandKind::Test,
            CommandSpec::new("npm", ["test"]).unwrap(),
        );

        assert!(configured_command(&commands, BuildOperation::InstallDependencies).is_some());
        // Publishing a local artifact uses the same `install` command; a
        // library may declare a dedicated install command, and when it does not
        // the provider's default plan is used instead.
        assert!(configured_command(&commands, BuildOperation::PublishLocalArtifact).is_some());
        assert!(configured_command(&commands, BuildOperation::Test).is_some());
    }
}
