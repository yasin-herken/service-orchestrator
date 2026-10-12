//! End-to-end managed service lifecycle through the [`ServiceManager`].
//!
//! These tests run real processes (a tiny local `sleep`) to exercise start,
//! stop, restart, idempotency, port conflicts, version compatibility,
//! cancellation, and ownership. They never invoke Java, Node, Maven, or npm.

mod support;

use std::sync::Arc;

use service_orchestrator_application::{ServiceRuntimeError, ServiceRuntimeIndex};
use service_orchestrator_domain::{ProcessState, Version};
use service_orchestrator_execution::Cancellation;
use service_orchestrator_runtime::{
    FreePortChecker, NullRuntimeInspector, RuntimeEvent, RuntimeRegistry,
};
use support::{
    fixture, java_runtime, manager, manager_with, script_registry, service_id, BusyPort,
    FixedInspector, ScriptRuntime, SLEEP,
};

#[tokio::test]
#[cfg(unix)]
async fn a_service_starts_and_is_tracked() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));

    let outcome = manager.start(&fixture.target, None).await.unwrap();
    assert!(!outcome.already_running);
    assert!(outcome.pid.is_some());
    assert_eq!(outcome.runtime.label(), "java");

    let info = manager
        .runtime_state(&service_id())
        .expect("the process is tracked");
    assert_eq!(info.state, ProcessState::Running);
    assert!(info.pid.is_some());

    manager.stop(&service_id()).await.unwrap();
}

#[tokio::test]
#[cfg(unix)]
async fn starting_a_running_service_is_idempotent() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));

    let first = manager.start(&fixture.target, None).await.unwrap();
    let second = manager.start(&fixture.target, None).await.unwrap();
    assert!(second.already_running);
    assert_eq!(first.pid, second.pid);

    manager.stop(&service_id()).await.unwrap();
}

#[tokio::test]
#[cfg(unix)]
async fn stopping_a_stopped_service_is_a_no_op() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));

    let outcome = manager.stop(&service_id()).await.unwrap();
    assert!(outcome.already_stopped);

    manager.start(&fixture.target, None).await.unwrap();
    let stopped = manager.stop(&service_id()).await.unwrap();
    assert!(!stopped.already_stopped);
    // Once stopped, the process is no longer tracked.
    assert!(manager.runtime_state(&service_id()).is_none());
}

#[tokio::test]
#[cfg(unix)]
async fn restart_stops_before_starting() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));

    manager.start(&fixture.target, None).await.unwrap();
    let outcome = manager.restart(&fixture.target, None).await.unwrap();
    assert!(!outcome.stopped.already_stopped);
    assert!(!outcome.started.already_running);

    let info = manager
        .runtime_state(&service_id())
        .expect("the replacement process is tracked");
    assert_eq!(info.state, ProcessState::Running);

    manager.stop(&service_id()).await.unwrap();
}

#[tokio::test]
#[cfg(unix)]
async fn a_cancelled_start_never_spawns() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));

    let cancellation = Cancellation::new();
    cancellation.cancel();
    let error = manager
        .start(&fixture.target, Some(cancellation))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ServiceRuntimeError::OperationCancelled { .. }
    ));
    assert!(manager.runtime_state(&service_id()).is_none());
}

#[tokio::test]
#[cfg(unix)]
async fn a_busy_port_prevents_a_start() {
    let fixture = fixture(&[8081], Some(java_runtime()));
    let manager = manager_with(
        Arc::clone(&fixture.config),
        support::sleep_registry(),
        Arc::new(BusyPort(8081)),
        Arc::new(NullRuntimeInspector),
    );

    let error = manager.start(&fixture.target, None).await.unwrap_err();
    assert!(matches!(
        error,
        ServiceRuntimeError::PortAlreadyInUse { port: 8081, .. }
    ));
    assert!(manager.runtime_state(&service_id()).is_none());
}

#[tokio::test]
#[cfg(unix)]
async fn a_failed_start_clears_the_starting_state_so_the_service_is_recoverable() {
    // Regression test for `tasks/017.md` defect D1: the application marks a
    // service `Starting` before it submits the start task, so a failed start
    // must reconcile that state — otherwise every future start is refused
    // with "currently starting" until the daemon restarts.
    use service_orchestrator_application::{InMemoryRuntimeStateStore, RuntimeStateStore};

    let fixture = fixture(&[8081], Some(java_runtime()));
    let state = Arc::new(InMemoryRuntimeStateStore::new());
    let manager = support::manager_with_state(
        Arc::clone(&fixture.config),
        support::sleep_registry(),
        Arc::new(BusyPort(8081)),
        Arc::new(NullRuntimeInspector),
        Arc::clone(&state),
    );

    // The application's pre-task bookkeeping.
    let current = state.service_state(&service_id()).unwrap_or_default();
    state.set_service_state(&service_id(), current.with_process(ProcessState::Starting));

    let error = manager.start(&fixture.target, None).await.unwrap_err();
    assert!(matches!(
        error,
        ServiceRuntimeError::PortAlreadyInUse { .. }
    ));

    let reconciled = state
        .service_state(&service_id())
        .expect("the transition state is recorded");
    assert_eq!(
        reconciled.process,
        ProcessState::Failed,
        "a failed start must not leave the service stuck in Starting"
    );

    // The service is recoverable: with the port free, the next start succeeds.
    let recovered = support::manager_with_state(
        Arc::clone(&fixture.config),
        support::sleep_registry(),
        Arc::new(FreePortChecker),
        Arc::new(NullRuntimeInspector),
        state,
    );
    recovered.start(&fixture.target, None).await.unwrap();
    recovered.stop(&service_id()).await.unwrap();
}

#[tokio::test]
#[cfg(unix)]
async fn a_version_mismatch_prevents_a_start() {
    let pinned = Version::new("27").unwrap();
    let fixture = fixture(
        &[],
        Some(service_orchestrator_domain::Runtime::versioned(
            service_orchestrator_domain::RuntimeKind::Java,
            pinned,
        )),
    );
    let manager = manager_with(
        Arc::clone(&fixture.config),
        support::sleep_registry(),
        Arc::new(FreePortChecker),
        Arc::new(FixedInspector(Some(Version::new("21.0.1").unwrap()))),
    );

    let error = manager.start(&fixture.target, None).await.unwrap_err();
    assert!(matches!(
        error,
        ServiceRuntimeError::RuntimeVersionMismatch { .. }
    ));
    assert!(manager.runtime_state(&service_id()).is_none());
}

#[tokio::test]
async fn a_missing_working_directory_is_rejected() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));

    let mut target = fixture.target.clone();
    target.path = None;
    let error = manager.start(&target, None).await.unwrap_err();
    assert!(matches!(
        error,
        ServiceRuntimeError::WorkingDirectoryInvalid { .. }
    ));
}

#[tokio::test]
async fn an_unknown_runtime_is_unsupported() {
    let fixture = fixture(&[], None);
    let manager = manager_with(
        Arc::clone(&fixture.config),
        RuntimeRegistry::new(),
        Arc::new(FreePortChecker),
        Arc::new(NullRuntimeInspector),
    );

    let error = manager.start(&fixture.target, None).await.unwrap_err();
    assert!(matches!(
        error,
        ServiceRuntimeError::UnsupportedRuntime { .. }
    ));
}

#[tokio::test]
#[cfg(unix)]
async fn lifecycle_events_are_published() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));
    let mut events = manager.subscribe();

    manager.start(&fixture.target, None).await.unwrap();

    // The first two events are the starting/started pair for the service.
    let first = events.recv().await.unwrap();
    assert_eq!(first.service_id(), &service_id());
    let second = events.recv().await.unwrap();
    assert_eq!(second.service_id(), &service_id());

    manager.stop(&service_id()).await.unwrap();
}

#[test]
#[cfg(unix)]
fn the_sleep_utility_exists() {
    // Guards the fixture against a machine without the test utility.
    assert!(std::path::Path::new(SLEEP).is_file());
}

#[tokio::test]
#[cfg(unix)]
async fn concurrent_start_requests_create_one_process() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager(Arc::clone(&fixture.config));

    // Two interfaces (say TUI and MCP) racing to start the same service must
    // serialize behind the per-service lock and produce exactly one process.
    let (first, second) = tokio::join!(
        manager.start(&fixture.target, None),
        manager.start(&fixture.target, None),
    );
    let first = first.unwrap();
    let second = second.unwrap();

    assert_ne!(
        first.already_running, second.already_running,
        "exactly one request spawns and the other observes it"
    );
    assert_eq!(first.pid, second.pid);

    manager.stop(&service_id()).await.unwrap();
}

#[tokio::test]
#[cfg(unix)]
async fn an_unexpected_exit_is_detected_and_queryable() {
    let fixture = fixture(&[], Some(java_runtime()));
    let manager = manager_with(
        Arc::clone(&fixture.config),
        script_registry(ScriptRuntime::new(
            "/bin/sh",
            vec!["-c".to_owned(), "exit 3".to_owned()],
        )),
        Arc::new(FreePortChecker),
        Arc::new(NullRuntimeInspector),
    );
    let mut events = manager.subscribe();

    manager.start(&fixture.target, None).await.unwrap();

    // The exit watcher publishes a terminal event once the process ends.
    let terminal = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            if matches!(
                event,
                RuntimeEvent::ServiceStopped { .. }
                    | RuntimeEvent::ServiceFailed { .. }
                    | RuntimeEvent::ServiceExited { .. }
            ) {
                break event;
            }
        }
    })
    .await
    .expect("a terminal event is published");
    assert_eq!(terminal.service_id(), &service_id());

    // The failure remains queryable through the runtime index.
    let info = manager
        .runtime_state(&service_id())
        .expect("the process is still tracked after exiting");
    assert_ne!(info.state, ProcessState::Running);
}
