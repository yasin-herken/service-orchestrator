//! End-to-end tests for the health manager.
//!
//! These tests drive the real [`HealthManager`], the real process/TCP/HTTP
//! providers, and the real retention and event machinery. The only fakes are
//! the managed-process index (so a process lifecycle can be simulated without
//! spawning anything) and small loopback TCP servers that stand in for a
//! service endpoint. Nothing here needs Maven, Java, Node, Git, or the internet
//! (`tasks/012.md` §37).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_application::{
    HealthCheckScope, HealthIndex, HealthState, InMemoryRuntimeStateStore, NullServiceRuntimeIndex,
    RuntimeStateStore, ServiceProcessInfo, ServiceRuntimeIndex,
};
use service_orchestrator_config::{load_from_str, Config};
use service_orchestrator_domain::{ProcessState, ServiceId, WorkspaceId};
use service_orchestrator_execution::Cancellation;
use service_orchestrator_health::{HealthError, HealthEvent, HealthManager, HealthMonitor};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

// -- fixtures ----------------------------------------------------------------

/// Builds a valid configuration whose only service is `auth-service`, with
/// `extra` appended to the service table (ports and health checks).
fn config_with(extra: &str) -> Config {
    let text = format!(
        r#"
config_version = 1

[[workspaces]]
id = "acme"
root = "/tmp/acme"

[[services]]
id = "auth-service"
type = "backend"
workspace = "acme"
path = "backend/auth-service"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"
{extra}
"#
    );
    load_from_str(&text, "<test>").unwrap()
}

/// A configuration with a single HTTP/actuator check.
fn http_config(port: u16, required: bool) -> Config {
    let required = if required { "" } else { "required = false\n" };
    config_with(&format!(
        r#"
[[services.health_checks]]
type = "http"
url = "http://localhost:{port}/actuator/health"
timeout_seconds = 2
{required}"#
    ))
}

/// A configuration with a single TCP/port check on a declared port.
fn tcp_config(port: u16) -> Config {
    config_with(&format!(
        r#"
[[services.ports]]
port = {port}

[[services.health_checks]]
type = "port"
port = {port}
timeout_seconds = 2
"#
    ))
}

/// A configuration with a single process check.
fn process_config() -> Config {
    config_with(
        r#"
[[services.health_checks]]
type = "process"
"#,
    )
}

/// A configuration with a required HTTP check and an optional process check.
fn mixed_config(port: u16) -> Config {
    config_with(&format!(
        r#"
[[services.health_checks]]
type = "http"
url = "http://localhost:{port}/actuator/health"
timeout_seconds = 2

[[services.health_checks]]
type = "process"
required = false
"#
    ))
}

fn service() -> ServiceId {
    ServiceId::new("auth-service").unwrap()
}

/// A runtime index whose managed process can be flipped between running and
/// exited.
struct FakeRuntime {
    running: AtomicBool,
    managed: bool,
}

impl FakeRuntime {
    fn running() -> Self {
        Self {
            running: AtomicBool::new(true),
            managed: true,
        }
    }

    fn unmanaged() -> Self {
        Self {
            running: AtomicBool::new(false),
            managed: false,
        }
    }

    fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::SeqCst);
    }
}

impl ServiceRuntimeIndex for FakeRuntime {
    fn runtime_state(&self, service_id: &ServiceId) -> Option<ServiceProcessInfo> {
        if !self.managed {
            return None;
        }
        let running = self.running.load(Ordering::SeqCst);
        Some(ServiceProcessInfo {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            service_id: service_id.clone(),
            process_id: Some("proc-1".to_owned()),
            pid: Some(4242),
            state: if running {
                ProcessState::Running
            } else {
                ProcessState::Stopped
            },
            runtime: Some("java".to_owned()),
            command: None,
            started_at: None,
            exit_code: None,
            exit_reason: None,
            last_error: None,
        })
    }

    fn managed_services(&self) -> Vec<ServiceProcessInfo> {
        Vec::new()
    }
}

fn manager(config: Config, runtime: Arc<dyn ServiceRuntimeIndex>) -> Arc<HealthManager> {
    Arc::new(HealthManager::new(
        Arc::new(config),
        runtime,
        Arc::new(InMemoryRuntimeStateStore::new()),
    ))
}

fn null_runtime_manager(config: Config) -> Arc<HealthManager> {
    manager(config, Arc::new(NullServiceRuntimeIndex))
}

// -- a tiny HTTP server ------------------------------------------------------

/// Serves one actuator response per accepted connection, in order.
///
/// Each response carries `{"status":"<value>"}`. Once the list is exhausted the
/// listener is dropped, so a further connection is refused.
async fn serve_statuses(statuses: Vec<&'static str>) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        for status in statuses {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let mut buffer = [0u8; 2048];
            let _ = socket.read(&mut buffer).await;
            let body = format!("{{\"status\":\"{status}\"}}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    port
}

/// An always-ready manager with a fast poll interval, for readiness tests.
fn readiness_manager(config: Config) -> Arc<HealthManager> {
    Arc::new(
        HealthManager::new(
            Arc::new(config),
            Arc::new(NullServiceRuntimeIndex),
            Arc::new(InMemoryRuntimeStateStore::new()),
        )
        .with_poll_interval(Duration::from_millis(20)),
    )
}

// -- tests -------------------------------------------------------------------

#[tokio::test]
async fn a_healthy_http_endpoint_is_reported_healthy() {
    let port = serve_statuses(vec!["UP"]).await;
    let manager = null_runtime_manager(http_config(port, true));

    let health = manager
        .check(&service(), HealthCheckScope::All)
        .await
        .unwrap();
    assert_eq!(health.state, HealthState::Healthy);
    assert_eq!(health.checks.len(), 1);
    assert!(health.checks[0].is_healthy());
    assert_eq!(health.consecutive_failures, 0);

    // The latest observation is queryable through the store.
    assert_eq!(
        manager.store().service_health(&service()).unwrap().state,
        HealthState::Healthy
    );
}

#[tokio::test]
async fn an_unhealthy_http_endpoint_is_reported_unhealthy() {
    let port = serve_statuses(vec!["DOWN"]).await;
    let manager = null_runtime_manager(http_config(port, true));

    let health = manager
        .check(&service(), HealthCheckScope::All)
        .await
        .unwrap();
    assert_eq!(health.state, HealthState::Unhealthy);
    assert_eq!(health.consecutive_failures, 1);
    assert!(health.last_unhealthy_at.is_some());
}

#[tokio::test]
async fn an_unreachable_http_endpoint_is_reported_unreachable() {
    // Bind and immediately drop a listener so the port is closed.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let manager = null_runtime_manager(http_config(port, true));
    let health = manager
        .check(&service(), HealthCheckScope::All)
        .await
        .unwrap();
    assert_eq!(health.state, HealthState::Unreachable);
}

#[tokio::test]
async fn tcp_reachability_and_refusal_are_distinguished() {
    // Reachable: the listener stays bound and accepts into its backlog.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let open_port = listener.local_addr().unwrap().port();
    let manager = null_runtime_manager(tcp_config(open_port));
    let health = manager
        .check(&service(), HealthCheckScope::Tcp)
        .await
        .unwrap();
    assert_eq!(health.state, HealthState::Healthy);

    // Refused: a closed port.
    let closed = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let manager = null_runtime_manager(tcp_config(closed_port));
    let health = manager
        .check(&service(), HealthCheckScope::Tcp)
        .await
        .unwrap();
    assert_eq!(health.state, HealthState::Unreachable);
}

#[tokio::test]
async fn a_managed_process_exit_changes_health() {
    let runtime = Arc::new(FakeRuntime::running());
    let manager = manager(
        process_config(),
        Arc::clone(&runtime) as Arc<dyn ServiceRuntimeIndex>,
    );

    let healthy = manager
        .check(&service(), HealthCheckScope::Process)
        .await
        .unwrap();
    assert_eq!(healthy.state, HealthState::Healthy);

    // The managed process exits: health must follow, independently of anything
    // else the service might be doing.
    runtime.set_running(false);
    let unhealthy = manager
        .check(&service(), HealthCheckScope::Process)
        .await
        .unwrap();
    assert_eq!(unhealthy.state, HealthState::Unhealthy);
    assert_eq!(unhealthy.consecutive_failures, 1);
}

#[tokio::test]
async fn an_unmanaged_process_is_unhealthy() {
    let manager = manager(
        process_config(),
        Arc::new(FakeRuntime::unmanaged()) as Arc<dyn ServiceRuntimeIndex>,
    );
    let health = manager
        .check(&service(), HealthCheckScope::Process)
        .await
        .unwrap();
    assert_eq!(health.state, HealthState::Unhealthy);
}

#[tokio::test]
async fn an_optional_check_failure_is_only_degraded() {
    let port = serve_statuses(vec!["UP"]).await;
    let manager = manager(
        mixed_config(port),
        Arc::new(FakeRuntime::unmanaged()) as Arc<dyn ServiceRuntimeIndex>,
    );

    let health = manager
        .check(&service(), HealthCheckScope::All)
        .await
        .unwrap();
    // The required HTTP check passed; only the optional process check failed.
    assert_eq!(health.state, HealthState::Degraded);
}

#[tokio::test]
async fn readiness_becomes_ready_after_delayed_startup() {
    // The endpoint reports DOWN once, then UP: a service that is still
    // initializing must not be declared ready prematurely, and must eventually
    // be declared ready.
    let port = serve_statuses(vec!["DOWN", "UP"]).await;
    let manager = readiness_manager(http_config(port, true));

    let health = manager
        .wait_until_ready(&service(), Duration::from_secs(5), None)
        .await
        .unwrap();
    assert!(health.state.is_serving());
}

#[tokio::test]
async fn readiness_times_out_when_the_service_never_becomes_ready() {
    let port = serve_statuses(vec!["DOWN"; 200]).await;
    let manager = readiness_manager(http_config(port, true));

    let error = manager
        .wait_until_ready(&service(), Duration::from_millis(250), None)
        .await
        .unwrap_err();
    assert!(matches!(error, HealthError::HealthNotReady { .. }));
}

#[tokio::test]
async fn readiness_honours_cancellation() {
    let port = serve_statuses(vec!["DOWN"; 200]).await;
    let manager = readiness_manager(http_config(port, true));
    let cancellation = Cancellation::new();
    cancellation.cancel();

    let error = manager
        .wait_until_ready(&service(), Duration::from_secs(5), Some(cancellation))
        .await
        .unwrap_err();
    assert!(matches!(error, HealthError::OperationCancelled { .. }));
}

#[tokio::test]
async fn polling_records_health_and_shuts_down_cleanly() {
    let port = serve_statuses(vec!["UP"; 200]).await;
    let manager = readiness_manager(http_config(port, true));
    let store = manager.store();
    let monitor = HealthMonitor::new(Arc::clone(&manager), Duration::from_millis(20));
    assert!(monitor.watch(&service()));
    assert!(monitor.start());
    assert!(!monitor.start(), "a second start must not spawn a loop");

    let recorded = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if store.service_health(&service()).is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(recorded.is_ok(), "the monitor never recorded health");

    monitor.shutdown().await;
    assert!(!monitor.is_running());
}

#[tokio::test]
async fn health_events_report_transitions_without_repeating_them() {
    let runtime = Arc::new(FakeRuntime::running());
    let manager = manager(
        process_config(),
        Arc::clone(&runtime) as Arc<dyn ServiceRuntimeIndex>,
    );
    let mut events = manager.subscribe();

    // First observation: unknown -> healthy.
    manager
        .check(&service(), HealthCheckScope::Process)
        .await
        .unwrap();
    // Second, identical observation must not emit another transition.
    manager
        .check(&service(), HealthCheckScope::Process)
        .await
        .unwrap();

    let mut transitions = Vec::new();
    while let Ok(event) = tokio::time::timeout(Duration::from_millis(50), events.recv()).await {
        let Ok(event) = event else { break };
        if let HealthEvent::ServiceHealthChanged {
            previous, current, ..
        } = event
        {
            transitions.push((previous, current));
        }
    }
    assert_eq!(
        transitions,
        vec![(HealthState::Unknown, HealthState::Healthy)]
    );
}

#[tokio::test]
async fn health_is_mirrored_into_the_runtime_state_store() {
    let store = Arc::new(InMemoryRuntimeStateStore::new());
    let manager = Arc::new(HealthManager::new(
        Arc::new(process_config()),
        Arc::new(FakeRuntime::running()) as Arc<dyn ServiceRuntimeIndex>,
        Arc::clone(&store) as Arc<dyn RuntimeStateStore>,
    ));
    manager
        .check(&service(), HealthCheckScope::Process)
        .await
        .unwrap();
    assert_eq!(
        store.service_state(&service()).unwrap().health,
        HealthState::Healthy
    );
}

#[tokio::test]
async fn an_unknown_service_is_a_structured_error() {
    let manager = null_runtime_manager(process_config());
    let error = manager
        .check(&ServiceId::new("missing").unwrap(), HealthCheckScope::All)
        .await
        .unwrap_err();
    assert!(matches!(error, HealthError::ServiceNotFound { .. }));
}
