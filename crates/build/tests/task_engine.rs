//! Integration tests for running builds through the task engine.
//!
//! These exercise the whole adapter flow required by `tasks/008.md`:
//!
//! ```text
//! TaskDefinition (target parameters)
//!     -> TaskEngine -> BuildTaskExecutor -> BuildManager
//!     -> BuildProvider -> BuildPlan -> BuildRunner -> process manager
//! ```
//!
//! They assert on real task outcomes (succeeded, failed with a structured code)
//! and, crucially, that the *same* flow serves a Maven target and an npm target
//! whenever a command is configured — provider-independent orchestration. The
//! configured command is a tiny local utility so no Maven or npm is invoked.

mod support;

use std::sync::Arc;

use service_orchestrator_build::{register_build_executors, BuildManager};
use service_orchestrator_domain::{BuildOperation, BuildSystem, TaskKind, TaskStatus};
use service_orchestrator_execution::{ExecutorRegistry, TaskDefinition, TaskEngine};
use service_orchestrator_process::ProcessManager;
use support::Workspace;

#[cfg(unix)]
const ECHO: &str = "/bin/echo";
#[cfg(unix)]
const FALSE: &str = "/usr/bin/false";

/// Builds an engine with the build executors registered.
fn engine(max_parallel: usize) -> TaskEngine {
    let manager = Arc::new(BuildManager::new(ProcessManager::new().unwrap()));
    let mut registry = ExecutorRegistry::new();
    register_build_executors(&mut registry, manager);
    TaskEngine::builder()
        .max_parallel_tasks(max_parallel)
        .registry(registry)
        .build()
        .expect("the engine builds inside a Tokio runtime")
}

/// Submits a build task carrying a resolved target and waits for it.
async fn run_build_task(
    engine: &TaskEngine,
    definition: TaskDefinition,
) -> service_orchestrator_domain::Task {
    let id = engine.submit(definition).expect("task is accepted");
    engine.wait_for(&id).await.expect("the task exists")
}

fn build_definition(
    target: &service_orchestrator_application::ResolvedBuildTarget,
) -> TaskDefinition {
    TaskDefinition::new(TaskKind::BuildService).with_parameters(target.to_parameters())
}

#[tokio::test]
#[cfg(unix)]
async fn a_maven_build_succeeds_through_the_engine() {
    let workspace = Workspace::new("auth-service");
    let target =
        workspace.target_with_command(BuildSystem::Maven, BuildOperation::Build, ECHO, &["ok"]);
    let engine = engine(2);

    let task = run_build_task(&engine, build_definition(&target)).await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}

#[tokio::test]
#[cfg(unix)]
async fn provider_independent_orchestration_serves_npm_the_same_way() {
    let workspace = Workspace::new("web-ui");
    let target =
        workspace.target_with_command(BuildSystem::Npm, BuildOperation::Build, ECHO, &["ok"]);
    let engine = engine(2);

    let task = run_build_task(&engine, build_definition(&target)).await;
    assert_eq!(task.status, TaskStatus::Succeeded);
    engine.shutdown().await;
}

#[tokio::test]
#[cfg(unix)]
async fn a_failing_build_is_reported_with_a_structured_code() {
    let workspace = Workspace::new("auth-service");
    let target =
        workspace.target_with_command(BuildSystem::Maven, BuildOperation::Build, FALSE, &[]);
    let engine = engine(2);

    let task = run_build_task(&engine, build_definition(&target)).await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "BUILD_COMMAND_FAILED"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn an_unsupported_operation_fails_the_task() {
    let workspace = Workspace::new("auth-service");
    let target = workspace.target(BuildSystem::Maven, BuildOperation::InstallDependencies);
    let engine = engine(2);

    let task = run_build_task(&engine, build_definition(&target)).await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "BUILD_UNSUPPORTED_OPERATION"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn a_task_without_target_parameters_fails_with_a_structured_code() {
    let engine = engine(2);

    // A build task that carries no resolved target is malformed.
    let task = run_build_task(&engine, TaskDefinition::new(TaskKind::BuildService)).await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "BUILD_MISSING_PARAMETER"
    );
    engine.shutdown().await;
}
