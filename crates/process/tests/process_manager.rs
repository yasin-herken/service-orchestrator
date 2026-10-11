//! End-to-end tests for the process manager.
//!
//! Every test drives the public API and uses small, deterministic helper
//! executables (`/bin/sh`, `printf`, `sleep`) that are standard on the target
//! platform. No external software such as Maven, Git, npm, or Liquibase is
//! involved, and no test depends on a developer repository.
//!
//! Log capture is made deterministic by subscribing to the manager's event
//! stream *before* spawning: a process that exits immediately still has its
//! lines buffered for a subscriber that already exists.

use std::time::Duration;

use service_orchestrator_domain::ServiceId;
use service_orchestrator_process::{
    ExitReason, LogLine, LogStreamKind, ProcessError, ProcessEvent, ProcessEventStream,
    ProcessManager, ProcessManagerConfig, ProcessSpec, ProcessStatus, RestartPolicy,
    ShutdownStrategy, TerminationSignal,
};
use tokio::time::timeout;

/// A `sh -c "<script>"` process specification.
fn sh(script: &str) -> ProcessSpec {
    ProcessSpec::new("/bin/sh")
        .expect("program is valid")
        .args(["-c", script])
}

/// A manager with a short manager-level shutdown grace, for fast tests.
fn manager() -> ProcessManager {
    ProcessManager::with_config(ProcessManagerConfig {
        shutdown_grace: Duration::from_secs(2),
        ..ProcessManagerConfig::default()
    })
    .expect("runtime is available")
}

/// Waits until `handle` is running, or panics after a bounded period.
async fn await_running(handle: &service_orchestrator_process::ProcessHandle) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !handle.is_running() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "process never reached running: {:?}",
            handle.status()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Drains buffered log lines for one process until the stream goes quiet.
async fn drain_logs(
    logs: &mut service_orchestrator_process::LogStream,
    process: &service_orchestrator_process::ProcessId,
) -> Vec<LogLine> {
    let mut lines = Vec::new();
    loop {
        match timeout(Duration::from_millis(300), logs.recv()).await {
            Ok(Ok(line)) if &line.process == process => lines.push(line),
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => break,
        }
    }
    lines
}

/// Drains buffered events until the stream goes quiet.
async fn drain_events(events: &mut ProcessEventStream) -> Vec<ProcessEvent> {
    let mut out = Vec::new();
    while let Ok(Ok(event)) = timeout(Duration::from_millis(300), events.recv()).await {
        out.push(event);
    }
    out
}

/// Waits until a specific line is observed for a process.
async fn await_log_line(
    logs: &mut service_orchestrator_process::LogStream,
    process: &service_orchestrator_process::ProcessId,
    needle: &str,
) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "did not observe line '{needle}' for {process}"
        );
        match timeout(Duration::from_millis(500), logs.recv()).await {
            Ok(Ok(line)) if &line.process == process && line.line == needle => return,
            Ok(Ok(_)) => {}
            Ok(Err(_)) => panic!("log stream closed before '{needle}' was observed"),
            Err(_) => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawns_a_process_and_detects_a_clean_exit() {
    let manager = manager();
    let handle = manager.spawn(sh("exit 0")).expect("spawn succeeds");
    assert!(handle.pid().is_some());

    let snapshot = handle.wait().await.expect("wait succeeds");
    assert_eq!(snapshot.status, ProcessStatus::Exited);
    let exit = snapshot.exit.expect("exit information is recorded");
    assert_eq!(exit.code, Some(0));
    assert_eq!(exit.reason, ExitReason::Exited);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captures_stdout_lines_independently() {
    let manager = manager();
    let mut logs = manager.subscribe_logs();
    let handle = manager
        .spawn(sh("printf 'first\\nsecond\\n'"))
        .expect("spawn succeeds");
    handle.wait().await.expect("wait succeeds");

    let lines = drain_logs(&mut logs, handle.id()).await;
    let stdout: Vec<&str> = lines
        .iter()
        .filter(|line| line.stream == LogStreamKind::Stdout)
        .map(|line| line.line.as_str())
        .collect();
    assert_eq!(stdout, vec!["first", "second"]);
    assert!(lines
        .iter()
        .all(|line| line.stream == LogStreamKind::Stdout));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captures_stderr_separately_from_stdout() {
    let manager = manager();
    let mut logs = manager.subscribe_logs();
    let handle = manager
        .spawn(sh("printf 'out\\n'; printf 'err\\n' 1>&2"))
        .expect("spawn succeeds");
    handle.wait().await.expect("wait succeeds");

    let lines = drain_logs(&mut logs, handle.id()).await;
    let stdout: Vec<&str> = lines
        .iter()
        .filter(|line| line.stream == LogStreamKind::Stdout)
        .map(|line| line.line.as_str())
        .collect();
    let stderr: Vec<&str> = lines
        .iter()
        .filter(|line| line.stream == LogStreamKind::Stderr)
        .map(|line| line.line.as_str())
        .collect();
    assert_eq!(stdout, vec!["out"]);
    assert_eq!(stderr, vec!["err"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn records_a_non_zero_exit_as_failure() {
    let manager = manager();
    let handle = manager.spawn(sh("exit 3")).expect("spawn succeeds");
    let snapshot = handle.wait().await.expect("wait succeeds");
    assert_eq!(snapshot.status, ProcessStatus::Failed);
    let exit = snapshot.exit.expect("exit information is recorded");
    assert_eq!(exit.code, Some(3));
    assert_eq!(exit.reason, ExitReason::Failed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stops_a_process_gracefully() {
    let manager = manager();
    let mut logs = manager.subscribe_logs();
    // SIGINT is chosen over SIGTERM defensively: a test runner may have
    // inherited an ignored SIGTERM, and POSIX forbids a non-interactive shell
    // from trapping a signal that was already ignored when it started.
    //
    // The body loops over a short sleep rather than one long one. There is an
    // unavoidable OS race between the shell printing `ready` and reaching its
    // `wait` on the foreground child: a signal delivered in that window is
    // deferred until the current foreground command returns. With `sleep 30`
    // that deferral can exceed the grace period and force a `SIGKILL`; with a
    // one-second loop the deferred trap always runs well inside the grace
    // period, so the test is deterministic.
    let spec = sh("trap 'exit 0' INT; echo ready; while :; do sleep 1; done")
        .with_shutdown(ShutdownStrategy::graceful().with_signal(TerminationSignal::Int));
    let handle = manager.spawn(spec).expect("spawn succeeds");
    await_running(&handle).await;
    await_log_line(&mut logs, handle.id(), "ready").await;

    handle.stop().expect("stop succeeds");
    let snapshot = handle.wait().await.expect("wait succeeds");
    let exit = snapshot.exit.clone().expect("exit recorded");
    assert_eq!(snapshot.status, ProcessStatus::Stopped, "exit={exit:?}");
    assert_eq!(exit.reason, ExitReason::Terminated);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escalates_to_sigkill_when_a_process_ignores_term() {
    let manager = manager();
    let mut logs = manager.subscribe_logs();
    let spec = sh("trap '' TERM; printf 'ready\\n'; sleep 30")
        .with_shutdown(ShutdownStrategy::graceful().with_grace_period(Duration::from_millis(150)));
    let handle = manager.spawn(spec).expect("spawn succeeds");
    await_running(&handle).await;
    // Wait until the shell has installed the trap before signalling it.
    await_log_line(&mut logs, handle.id(), "ready").await;

    handle.stop().expect("stop succeeds");
    let snapshot = handle.wait().await.expect("wait succeeds");
    assert_eq!(snapshot.status, ProcessStatus::Killed);
    assert_eq!(
        snapshot.exit.expect("exit recorded").reason,
        ExitReason::Killed
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_kills_a_running_process() {
    let manager = manager();
    let handle = manager.spawn(sh("sleep 30")).expect("spawn succeeds");
    await_running(&handle).await;

    handle.kill().expect("kill succeeds");
    let snapshot = handle.wait().await.expect("wait succeeds");
    assert_eq!(snapshot.status, ProcessStatus::Killed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enforces_a_timeout() {
    let manager = manager();
    let spec = sh("sleep 30").with_timeout(Duration::from_millis(150));
    let handle = manager.spawn(spec).expect("spawn succeeds");
    let snapshot = handle.wait().await.expect("wait succeeds");
    assert_eq!(snapshot.status, ProcessStatus::Killed);
    assert_eq!(
        snapshot.exit.expect("exit recorded").reason,
        ExitReason::TimedOut
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarts_into_a_new_execution_instance() {
    let manager = manager();
    let first = manager.spawn(sh("sleep 30")).expect("spawn succeeds");
    await_running(&first).await;
    let first_id = first.id().clone();

    let second = manager.restart(&first_id).await.expect("restart succeeds");
    assert_ne!(second.id(), &first_id);
    assert_eq!(first.status(), ProcessStatus::Stopped);
    await_running(&second).await;

    second.kill().expect("kill succeeds");
    second.wait().await.expect("wait succeeds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatically_restarts_after_a_failure() {
    let manager = manager();
    let mut events = manager.subscribe_events();
    let spec = sh("exit 1").with_restart(RestartPolicy::OnFailure {
        max_restarts: 1,
        backoff: Duration::from_millis(20),
    });
    let handle = manager.spawn(spec).expect("spawn succeeds");
    let snapshot = handle.wait().await.expect("wait succeeds");
    assert_eq!(snapshot.status, ProcessStatus::Failed);

    let events = drain_events(&mut events).await;
    let restarts = events
        .iter()
        .filter(|event| matches!(event, ProcessEvent::Restarting { .. }))
        .count();
    assert_eq!(restarts, 1, "expected exactly one automatic restart");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integrates_with_cooperative_cancellation() {
    let manager = manager();
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = manager
        .spawn_with_cancellation(sh("sleep 30"), async move {
            let _ = cancel_rx.await;
        })
        .expect("spawn succeeds");
    await_running(&handle).await;

    cancel_tx.send(()).expect("cancellation is delivered");
    let snapshot = handle.wait().await.expect("wait succeeds");
    assert_eq!(snapshot.status, ProcessStatus::Stopped);
    assert_eq!(
        snapshot.exit.expect("exit recorded").reason,
        ExitReason::Cancelled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registry_supports_lookup_and_listing() {
    let manager = manager();
    let service = ServiceId::new("auth-service").unwrap();
    let spec = sh("sleep 30").for_service(service.clone());
    let handle = manager.spawn(spec).expect("spawn succeeds");
    await_running(&handle).await;

    assert!(manager.handle(handle.id()).is_some());
    assert!(manager.snapshot(handle.id()).is_some());
    assert!(manager.find_by_service(&service).len() == 1);
    assert_eq!(manager.list_active().len(), 1);
    assert_eq!(manager.list().len(), 1);
    assert_eq!(manager.len(), 1);

    handle.kill().expect("kill succeeds");
    handle.wait().await.expect("wait succeeds");
    assert!(manager.list_active().is_empty());
    // Finished processes remain queryable until forgotten.
    assert!(manager.forget(handle.id()));
    assert!(manager.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handles_many_concurrent_processes() {
    let manager = manager();
    let handles: Vec<_> = (0..8)
        .map(|_| manager.spawn(sh("printf 'x\\n'")).expect("spawn succeeds"))
        .collect();

    for handle in &handles {
        let snapshot = handle.wait().await.expect("wait succeeds");
        assert_eq!(snapshot.status, ProcessStatus::Exited);
    }
    assert_eq!(manager.list().len(), 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_a_spawn_failure() {
    let manager = manager();
    let spec = ProcessSpec::new("/nonexistent/service-orchestrator-binary").expect("valid spec");
    let error = manager.spawn(spec).unwrap_err();
    assert!(matches!(error, ProcessError::Spawn { .. }));
    // The failed process is still recorded for inspection.
    let failed = manager.list();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].status, ProcessStatus::Failed);
    assert!(failed[0].last_error.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_stops_every_active_process() {
    let manager = manager();
    let handles: Vec<_> = (0..3)
        .map(|_| manager.spawn(sh("sleep 30")).expect("spawn succeeds"))
        .collect();
    for handle in &handles {
        await_running(handle).await;
    }

    manager.shutdown().await;
    for handle in &handles {
        assert!(
            handle.is_terminal(),
            "process {} survived shutdown: {:?}",
            handle.id(),
            handle.status()
        );
    }
    assert!(manager.is_shutdown());
    assert!(manager.spawn(sh("exit 0")).is_err());
}
