//! Provider selection, capability handling, and plan generation.
//!
//! These tests exercise the `BuildProvider` abstraction directly — selection
//! through the registry and planning through a controlled host — without
//! starting a process. They are the deterministic core required by
//! `tasks/008.md`: Maven selection, npm selection, unknown-provider rejection,
//! unsupported-operation handling, plan generation, and proof that planning
//! never spawns anything.

mod support;

use service_orchestrator_build::{
    BuildPlan, BuildProviderRegistry, ExpectedOutputKind, SystemBuildHost,
};
use service_orchestrator_domain::{BuildOperation, BuildSystem};
use support::{FakeHost, Workspace};

fn provider(system: BuildSystem) -> std::sync::Arc<dyn service_orchestrator_build::BuildProvider> {
    BuildProviderRegistry::standard()
        .resolve(system)
        .expect("the standard registry resolves its systems")
        .clone()
}

fn plan(
    system: BuildSystem,
    operation: BuildOperation,
    host: &dyn service_orchestrator_build::BuildHost,
) -> BuildPlan {
    let workspace = Workspace::new("entity");
    provider(system)
        .build_plan(&workspace.target(system, operation), host)
        .expect("the plan is generated")
}

// -- selection ---------------------------------------------------------------

#[test]
fn maven_provider_selection() {
    assert_eq!(provider(BuildSystem::Maven).system(), BuildSystem::Maven);
}

#[test]
fn npm_provider_selection() {
    assert_eq!(provider(BuildSystem::Npm).system(), BuildSystem::Npm);
}

#[test]
fn unknown_provider_rejection() {
    // No provider registered for the system.
    let error = BuildProviderRegistry::new()
        .resolve(BuildSystem::Maven)
        .err()
        .expect("an empty registry has no providers");
    assert_eq!(error.code(), "BUILD_UNSUPPORTED_SYSTEM");

    // A build system outside the modelled set is rejected by the domain.
    assert!(BuildSystem::parse("gradle").is_none());
    assert!(BuildSystem::parse("bazel").is_none());
}

// -- plan generation ---------------------------------------------------------

#[test]
fn a_maven_plan_is_generated() {
    let workspace = Workspace::new("auth-service");
    let plan = provider(BuildSystem::Maven)
        .build_plan(
            &workspace.target(BuildSystem::Maven, BuildOperation::Build),
            &FakeHost::new(),
        )
        .unwrap();
    assert_eq!(plan.program, "mvn");
    assert_eq!(plan.args, vec!["-B", "package"]);
    assert_eq!(
        plan.working_dir.as_str(),
        workspace.entity.to_string_lossy().as_ref()
    );
    assert_eq!(plan.expected_outputs.len(), 1);
    assert_eq!(plan.expected_outputs[0].path, "target");
    assert_eq!(plan.expected_outputs[0].kind, ExpectedOutputKind::Directory);
    assert_eq!(plan.system, BuildSystem::Maven);
    assert_eq!(plan.operation, BuildOperation::Build);
}

#[test]
fn a_maven_plan_prefers_the_wrapper() {
    let plan = plan(
        BuildSystem::Maven,
        BuildOperation::Build,
        &FakeHost::new().with_file("mvnw"),
    );
    assert_eq!(plan.program, "./mvnw");
}

#[test]
fn an_npm_plan_is_generated() {
    let plan = plan(
        BuildSystem::Npm,
        BuildOperation::Build,
        &SystemBuildHost::new(),
    );
    assert_eq!(plan.program, "npm");
    assert_eq!(plan.args, vec!["run", "build"]);
    assert!(plan.expected_outputs.is_empty());
    assert_eq!(plan.system, BuildSystem::Npm);
}

#[test]
fn a_configured_command_is_authoritative() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target_with_command(
        BuildSystem::Maven,
        BuildOperation::Build,
        "./mvnw",
        &["clean", "verify"],
    );
    let plan = provider(BuildSystem::Maven)
        .build_plan(&target, &FakeHost::new())
        .unwrap();
    assert_eq!(plan.program, "./mvnw");
    assert_eq!(plan.args, vec!["clean", "verify"]);
}

// -- capabilities ------------------------------------------------------------

#[test]
fn unsupported_operations_are_rejected_by_maven() {
    let error = provider(BuildSystem::Maven)
        .build_plan(
            &Workspace::new("entity")
                .target(BuildSystem::Maven, BuildOperation::InstallDependencies),
            &FakeHost::new(),
        )
        .unwrap_err();
    assert_eq!(error.code(), "BUILD_UNSUPPORTED_OPERATION");
}

#[test]
fn unsupported_operations_are_rejected_by_npm() {
    for operation in [BuildOperation::Clean, BuildOperation::PublishLocalArtifact] {
        let error = provider(BuildSystem::Npm)
            .build_plan(
                &Workspace::new("entity").target(BuildSystem::Npm, operation),
                &SystemBuildHost::new(),
            )
            .unwrap_err();
        assert_eq!(error.code(), "BUILD_UNSUPPORTED_OPERATION");
    }
}

#[test]
fn capabilities_differ_between_providers() {
    let maven = provider(BuildSystem::Maven);
    let npm = provider(BuildSystem::Npm);
    assert!(maven.supports(BuildOperation::PublishLocalArtifact));
    assert!(!npm.supports(BuildOperation::PublishLocalArtifact));
    assert!(npm.supports(BuildOperation::InstallDependencies));
    assert!(!maven.supports(BuildOperation::InstallDependencies));
}

// -- providers are pure ------------------------------------------------------

#[test]
fn planning_does_not_spawn_a_process() {
    // The configured executable does not exist anywhere on the machine. A
    // provider that tried to run it would fail here; because providers only
    // produce data, planning succeeds.
    let workspace = Workspace::new("entity");
    let target = workspace.target_with_command(
        BuildSystem::Maven,
        BuildOperation::Build,
        "definitely-not-a-real-build-tool-xyz",
        &["package"],
    );
    let plan = provider(BuildSystem::Maven)
        .build_plan(&target, &FakeHost::new())
        .unwrap();
    assert_eq!(plan.program, "definitely-not-a-real-build-tool-xyz");
}

#[test]
fn planning_is_repeatable() {
    let workspace = Workspace::new("entity");
    let target = workspace.target(BuildSystem::Npm, BuildOperation::Build);
    let provider = provider(BuildSystem::Npm);
    let host = SystemBuildHost::new();
    let first = provider.build_plan(&target, &host).unwrap();
    let second = provider.build_plan(&target, &host).unwrap();
    assert_eq!(first, second);
}
