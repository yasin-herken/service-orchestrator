//! Periodic health monitoring.
//!
//! [`HealthMonitor`] is a lightweight coordinator around [`HealthManager`]. It
//! runs one background loop that, on a configurable interval, checks every
//! *explicitly watched* service concurrently up to a fixed bound, then waits
//! before the next round. It is not a second scheduler: it submits no tasks and
//! owns no cancellation — it simply calls the manager
//! (`tasks/012.md` §15, §33).
//!
//! # Properties
//!
//! - **Opt-in, never everything by default.** A service is polled only once it
//!   is watched, so an idle workspace is not scanned continuously
//!   (`tasks/012.md` §15, §38).
//! - **No duplicate loops.** [`HealthMonitor::start`] is idempotent; a second
//!   call while running is a no-op.
//! - **Bounded concurrency.** At most [`DEFAULT_MAX_CONCURRENCY`] checks run at
//!   once, enforced with a [`tokio::sync::Semaphore`].
//! - **Cooperative shutdown.** [`HealthMonitor::stop`] signals cancellation and
//!   [`HealthMonitor::shutdown`] also awaits the loop, so a stopped monitor
//!   leaves no task behind.
//! - **No busy polling.** The loop sleeps for the interval and wakes early only
//!   on cancellation.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use service_orchestrator_application::HealthCheckScope;
use service_orchestrator_domain::ServiceId;
use service_orchestrator_execution::Cancellation;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use crate::manager::HealthManager;

/// The default number of checks that may run concurrently.
pub const DEFAULT_MAX_CONCURRENCY: usize = 4;

/// A cloneable handle to a background health monitor.
///
/// Cloning shares the same watched set, cancellation token, and running state,
/// so two interfaces observe and control the same loop.
#[derive(Clone)]
pub struct HealthMonitor {
    inner: Arc<MonitorInner>,
}

impl HealthMonitor {
    /// Creates a monitor for `manager` that polls every `interval`.
    #[must_use]
    pub fn new(manager: Arc<HealthManager>, interval: Duration) -> Self {
        Self {
            inner: Arc::new(MonitorInner {
                manager,
                interval,
                max_concurrency: AtomicUsize::new(DEFAULT_MAX_CONCURRENCY),
                services: Mutex::new(BTreeSet::new()),
                cancellation: Cancellation::new(),
                running: AtomicBool::new(false),
                handle: Mutex::new(None),
            }),
        }
    }

    /// Sets the maximum number of concurrent checks.
    ///
    /// The bound is shared state, so it may be adjusted before or after clones
    /// are made.
    #[must_use]
    pub fn with_max_concurrency(self, max_concurrency: usize) -> Self {
        self.inner
            .max_concurrency
            .store(max_concurrency.max(1), Ordering::SeqCst);
        self
    }

    /// Starts watching a service, returning `true` if it was newly added.
    pub fn watch(&self, service_id: &ServiceId) -> bool {
        self.inner
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(service_id.clone())
    }

    /// Stops watching a service, returning `true` if it was being watched.
    pub fn unwatch(&self, service_id: &ServiceId) -> bool {
        self.inner
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(service_id)
    }

    /// Returns `true` if a service is being watched.
    #[must_use]
    pub fn is_watching(&self, service_id: &ServiceId) -> bool {
        self.inner
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(service_id)
    }

    /// Returns the watched services, in identifier order.
    #[must_use]
    pub fn watched(&self) -> Vec<ServiceId> {
        self.inner
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Returns `true` while the polling loop is running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::SeqCst)
    }

    /// Returns the polling interval.
    #[must_use]
    pub fn interval(&self) -> Duration {
        self.inner.interval
    }

    /// Starts the polling loop.
    ///
    /// Returns `true` if the loop was started and `false` if one was already
    /// running, so a caller cannot accidentally create duplicate loops
    /// (`tasks/012.md` §15).
    pub fn start(&self) -> bool {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        let inner = Arc::clone(&self.inner);
        let handle = tokio::spawn(run_loop(inner));
        *self
            .inner
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handle);
        true
    }

    /// Signals the polling loop to stop.
    ///
    /// The loop observes cancellation cooperatively; use
    /// [`HealthMonitor::shutdown`] to also wait for it to finish.
    pub fn stop(&self) {
        self.inner.running.store(false, Ordering::SeqCst);
        self.inner.cancellation.cancel();
    }

    /// Stops the loop and waits for it to finish.
    pub async fn shutdown(&self) {
        self.stop();
        let handle = self
            .inner
            .handle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}

impl std::fmt::Debug for HealthMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HealthMonitor")
            .field("interval", &self.inner.interval)
            .field(
                "max_concurrency",
                &self.inner.max_concurrency.load(Ordering::SeqCst),
            )
            .field("running", &self.is_running())
            .field("watched", &self.watched())
            .finish_non_exhaustive()
    }
}

/// The shared monitor state.
struct MonitorInner {
    manager: Arc<HealthManager>,
    interval: Duration,
    max_concurrency: AtomicUsize,
    services: Mutex<BTreeSet<ServiceId>>,
    cancellation: Cancellation,
    running: AtomicBool,
    handle: Mutex<Option<JoinHandle<()>>>,
}

/// The polling loop.
async fn run_loop(inner: Arc<MonitorInner>) {
    let semaphore = Arc::new(Semaphore::new(inner.max_concurrency.load(Ordering::SeqCst)));
    loop {
        if inner.cancellation.is_cancelled() {
            break;
        }

        let services = inner
            .services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect::<Vec<_>>();

        let mut handles = Vec::with_capacity(services.len());
        for service_id in services {
            let Ok(permit) = Arc::clone(&semaphore).acquire_owned().await else {
                break;
            };
            let manager = Arc::clone(&inner.manager);
            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let _ = manager.check(&service_id, HealthCheckScope::All).await;
            }));
        }
        for handle in handles {
            let _ = handle.await;
        }

        tokio::select! {
            () = tokio::time::sleep(inner.interval) => {}
            () = inner.cancellation.cancelled() => break,
        }
    }
    inner.running.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    use service_orchestrator_application::{
        HealthIndex, InMemoryRuntimeStateStore, ServiceProcessInfo, ServiceRuntimeIndex,
    };
    use service_orchestrator_config::load_from_str;
    use service_orchestrator_domain::{ProcessState, ServiceId, WorkspaceId};

    const CONFIG: &str = r#"
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

[[services.health_checks]]
type = "process"
"#;

    /// A runtime index that always reports a running managed process.
    struct RunningIndex;

    impl ServiceRuntimeIndex for RunningIndex {
        fn runtime_state(&self, service_id: &ServiceId) -> Option<ServiceProcessInfo> {
            Some(ServiceProcessInfo {
                workspace_id: WorkspaceId::new("acme").unwrap(),
                service_id: service_id.clone(),
                process_id: Some("proc-1".to_owned()),
                pid: Some(4242),
                state: ProcessState::Running,
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

    fn manager() -> Arc<HealthManager> {
        let config = load_from_str(CONFIG, "<test>").unwrap();
        Arc::new(HealthManager::new(
            Arc::new(config),
            Arc::new(RunningIndex),
            Arc::new(InMemoryRuntimeStateStore::new()),
        ))
    }

    fn service() -> ServiceId {
        ServiceId::new("auth-service").unwrap()
    }

    #[test]
    fn watching_is_opt_in_and_idempotent() {
        let monitor = HealthMonitor::new(manager(), Duration::from_millis(50));
        assert!(monitor.watched().is_empty());
        assert!(monitor.watch(&service()));
        assert!(!monitor.watch(&service()));
        assert!(monitor.is_watching(&service()));
        assert_eq!(monitor.watched(), vec![service()]);
        assert!(monitor.unwatch(&service()));
        assert!(!monitor.unwatch(&service()));
    }

    #[tokio::test]
    async fn starting_twice_does_not_create_duplicate_loops() {
        let monitor = HealthMonitor::new(manager(), Duration::from_millis(20));
        assert!(monitor.start());
        assert!(!monitor.start());
        monitor.shutdown().await;
        assert!(!monitor.is_running());
    }

    #[tokio::test]
    async fn polling_records_health_and_stops_cleanly() {
        let manager = manager();
        let store = manager.store();
        let monitor = HealthMonitor::new(Arc::clone(&manager), Duration::from_millis(20));
        assert!(monitor.watch(&service()));
        assert!(monitor.start());

        let recorded = tokio::time::timeout(Duration::from_secs(2), async {
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
}
