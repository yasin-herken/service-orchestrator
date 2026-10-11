//! Integration tests for running service lifecycle through the task engine.
//!
//! These exercise the whole adapter flow required by `tasks/010.md`:
//!
//! ```text
//! TaskDefinition (target parameters)
//!     -> TaskEngine -> StartServiceExecutor / StopServiceExecutor
//!     -> ServiceManager -> ServiceRuntime -> process manager
//! ```
//!
//! They assert on real task outcomes and that a malformed task fails with a
//! structured code, without ever invoking a real toolchain.

mod support;

use std::sync::Arc;

use service_orchestrator_application::ResolvedServiceTarget;
use service_orchestrator_domain::{ProcessState, Task, TaskKind, TaskStatus};
use service_orchestrator_execution::{ExecutorRegistry, TaskDefinition, TaskEngine};
use service_orchestrator_runtime::{
    register_service_executors, ServiceManager, ServiceRuntimeIndex,
};
use support::{fixture, java_runtime};

fn engine(manager: Arc<ServiceManager>) -> TaskEngine {
    let mut registry = ExecutorRegistry::new();
    register_service_executors(&mut registry, manager);
    TaskEngine::builder()
        .max_parallel_tasks(2)
        .registry(registry)
        .build()
        .expect("the engine builds inside a Tokio runtime")
}

async fn run(engine: &TaskEngine, definition: TaskDefinition) -> Task {
    let id = engine.submit(definition).expect("task is accepted");
    engine.wait_for(&id).await.expect("the task exists")
}

fn definition(kind: TaskKind, target: &ResolvedServiceTarget) -> TaskDefinition {
    TaskDefinition::new(kind).with_parameters(target.to_parameters())
}

#[tokio::test]
#[cfg(unix)]
async fn start_and_stop_succeed_through_the_engine() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = Arc::new(support::manager(Arc::clone(&fixture.config)));
    let engine = engine(Arc::clone(&manager));

    let started = run(&engine, definition(TaskKind::StartService, &fixture.target)).await;
    assert_eq!(started.status, TaskStatus::Succeeded);

    // The startup task is complete, but the managed process keeps running: a
    // completed/cancelled startup task must never stop the service
    // (`tasks/010.md` §19, §26).
    let info = manager
        .runtime_state(&fixture.target.service_id)
        .expect("the service process outlives its startup task");
    assert_eq!(info.state, ProcessState::Running);

    let stopped = run(&engine, definition(TaskKind::StopService, &fixture.target)).await;
    assert_eq!(stopped.status, TaskStatus::Succeeded);

    engine.shutdown().await;
}

#[tokio::test]
async fn a_task_without_target_parameters_fails_with_a_structured_code() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = Arc::new(support::manager(Arc::clone(&fixture.config)));
    let engine = engine(manager);

    // A lifecycle task that carries no resolved target is malformed.
    let task = run(&engine, TaskDefinition::new(TaskKind::StartService)).await;
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("failed tasks carry an error").code,
        "RUNTIME_MISSING_PARAMETER"
    );

    engine.shutdown().await;
}
