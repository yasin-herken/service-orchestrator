//! End-to-end tests for the observability layer.
//!
//! These tests exercise the real [`LogHub`], its ingestion helpers, and a real
//! managed process. The process is a tiny `/bin/sh` one-liner that prints a
//! line, so the test proves that the process-provider output actually reaches
//! the hub and is correlated with its service (`tasks/012.md` §22, §23, §26,
//! §27, §29).

use std::sync::Arc;
use std::time::Duration;

use service_orchestrator_application::{
    Application, InMemoryRuntimeStateStore, LogEntry, LogLevel, LogRequest, LogService,
    LogStreamError,
};
use service_orchestrator_config::load_from_str;
use service_orchestrator_domain::{ServiceId, Task, TaskId, TaskKind, Workflow, WorkflowId};
use service_orchestrator_execution::{
    Cancellation, TaskDefinition, TaskEvent, TaskManager, TaskManagerError, WorkflowDefinition,
};
use service_orchestrator_logging::{LogHub, LogRetention, ProcessLogIngestor, TaskLogIngestor};
use service_orchestrator_process::{ProcessManager, ProcessSpec};

const APP_CONFIG: &str = r#"
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
"#;

/// A task manager that is never exercised by the read-only queries here.
struct NoopTaskManager;

impl TaskManager for NoopTaskManager {
    fn submit(&self, _definition: TaskDefinition) -> Result<TaskId, TaskManagerError> {
        Ok(TaskId::new("task-0").unwrap())
    }

    fn submit_workflow(
        &self,
        _definition: WorkflowDefinition,
    ) -> Result<WorkflowId, TaskManagerError> {
        Ok(WorkflowId::new("wf-0").unwrap())
    }

    fn task(&self, id: &TaskId) -> Result<Task, TaskManagerError> {
        Err(TaskManagerError::NotFound {
            entity: "task",
            id: id.to_string(),
        })
    }

    fn workflow(&self, id: &WorkflowId) -> Result<Workflow, TaskManagerError> {
        Err(TaskManagerError::NotFound {
            entity: "workflow",
            id: id.to_string(),
        })
    }

    fn workflow_tasks(&self, id: &WorkflowId) -> Result<Vec<Task>, TaskManagerError> {
        Err(TaskManagerError::NotFound {
            entity: "workflow",
            id: id.to_string(),
        })
    }

    fn cancel(&self, id: &TaskId) -> Result<(), TaskManagerError> {
        Err(TaskManagerError::NotFound {
            entity: "task",
            id: id.to_string(),
        })
    }
}

fn app_with_logs(hub: Arc<LogHub>) -> Application {
    let config = load_from_str(APP_CONFIG, "<test>").unwrap();
    Application::new(
        config,
        Arc::new(NoopTaskManager) as Arc<dyn TaskManager>,
        Arc::new(InMemoryRuntimeStateStore::new()),
        hub as Arc<dyn LogService>,
    )
}

fn service() -> ServiceId {
    ServiceId::new("auth-service").unwrap()
}

fn small_hub(per_service: usize, total: usize, subscriber_capacity: usize) -> LogHub {
    LogHub::with_retention(LogRetention {
        per_service,
        total,
        max_message_len: 512,
        subscriber_capacity,
        max_query_limit: 100,
    })
}

fn entry(message: &str) -> service_orchestrator_application::LogEntry {
    service_orchestrator_application::LogEntry::new(LogLevel::Info, message).with_service(service())
}

#[tokio::test]
async fn multiple_subscribers_each_receive_live_entries() {
    let hub = LogHub::new();
    let mut first = hub.subscribe(&service()).unwrap();
    let mut second = hub.subscribe(&service()).unwrap();

    hub.record(entry("first line"));
    hub.record(entry("second line"));

    assert_eq!(first.recv().await.unwrap().message, "first line");
    assert_eq!(second.recv().await.unwrap().message, "first line");
    assert_eq!(first.recv().await.unwrap().message, "second line");
    assert_eq!(second.recv().await.unwrap().message, "second line");
}

#[tokio::test]
async fn a_slow_subscriber_lags_without_blocking_the_producer() {
    let hub = small_hub(100, 100, 2);
    let mut subscription = hub.subscribe(&service()).unwrap();

    // The producer races ahead of the consumer: five entries into a two-slot
    // channel. `record` is synchronous and never awaits, so it cannot block.
    for index in 0..5 {
        hub.record(entry(&format!("line {index}")));
    }

    let first = subscription.recv().await;
    assert!(matches!(first, Err(LogStreamError::Lagged { .. })));

    // After resynchronising, the subscriber still receives later entries.
    let recovered = subscription.recv().await.unwrap();
    assert!(recovered.message.starts_with("line "));
}

#[test]
fn historical_retrieval_is_bounded_and_filterable() {
    let hub = small_hub(100, 50, 8);
    for index in 0..20 {
        hub.record(entry(&format!("message {index}")));
    }
    let mut error = entry("a failure");
    error.level = LogLevel::Error;
    hub.record(error);

    // Newest first, bounded by the request.
    let page = hub
        .query(&LogRequest::new(service()).with_limit(5))
        .unwrap();
    assert_eq!(page.len(), 5);
    assert_eq!(page[0].message, "a failure");

    // Only errors.
    let errors = hub
        .query(&LogRequest::new(service()).with_minimum_level(LogLevel::Error))
        .unwrap();
    assert_eq!(errors.len(), 1);
}

#[test]
fn pagination_walks_the_whole_history_without_duplicates() {
    let hub = LogHub::new();
    for index in 0..25 {
        hub.record(entry(&format!("line {index}")));
    }

    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let request = match cursor {
            Some(cursor) => LogRequest::new(service()).with_limit(10).after(cursor),
            None => LogRequest::new(service()).with_limit(10),
        };
        let page = hub.page(&request).unwrap();
        seen.extend(page.entries.iter().map(|entry| entry.message.clone()));
        match page.next_cursor {
            Some(next) if page.truncated => cursor = Some(next),
            _ => break,
        }
    }

    assert_eq!(seen.len(), 25);
    assert_eq!(seen.first().map(String::as_str), Some("line 0"));
    assert_eq!(seen.last().map(String::as_str), Some("line 24"));
    // Strictly chronological, no repeats.
    let mut unique = seen.clone();
    unique.dedup();
    assert_eq!(unique.len(), seen.len());
}

#[test]
fn per_service_retention_caps_a_chatty_service() {
    let hub = small_hub(3, 100, 8);
    for index in 0..10 {
        hub.record(entry(&format!("line {index}")));
    }
    assert_eq!(hub.retained_for(&service()), 3);
    assert_eq!(hub.retained(), 3);

    let retained = hub.query(&LogRequest::new(service())).unwrap();
    assert_eq!(retained.len(), 3);
    assert_eq!(retained[0].message, "line 9");
}

#[test]
fn total_retention_caps_the_whole_hub() {
    let hub = small_hub(100, 4, 8);
    let other = ServiceId::new("payment-service").unwrap();
    for index in 0..3 {
        hub.record(entry(&format!("auth {index}")));
        hub.record(
            service_orchestrator_application::LogEntry::new(
                LogLevel::Info,
                format!("payment {index}"),
            )
            .with_service(other.clone()),
        );
    }
    assert_eq!(hub.retained(), 4);
}

#[test]
fn recorded_messages_are_redacted() {
    let hub = LogHub::new();
    hub.record(entry("connect DATABASE_URL=postgres://u:p@localhost/db"));
    let stored = hub.query(&LogRequest::new(service())).unwrap();
    assert!(!stored[0].message.contains(":p@localhost"));
}

#[test]
fn the_task_ingestor_correlates_events_with_their_service() {
    let hub = Arc::new(LogHub::new());
    let ingestor = TaskLogIngestor::new(Arc::clone(&hub));
    let task = TaskId::new("task-7").unwrap();

    ingestor.handle(&TaskEvent::TaskCreated {
        task: Box::new(Task::new(task.clone(), TaskKind::BuildService).with_service(service())),
    });
    ingestor.handle(&TaskEvent::TaskStarted {
        id: task.clone(),
        kind: TaskKind::BuildService,
    });
    ingestor.handle(&TaskEvent::TaskFailed {
        id: task.clone(),
        failure: service_orchestrator_domain::TaskFailure::new(
            "BUILD_FAILED",
            "maven exited 1",
            None,
        )
        .unwrap(),
    });

    let entries = hub.query(&LogRequest::new(service())).unwrap();
    assert_eq!(entries.len(), 3);
    // Newest first: the failure is the most recent entry.
    assert_eq!(entries[0].level, LogLevel::Error);
    assert!(entries[0].message.contains("BUILD_FAILED"));
    assert_eq!(entries[0].task_id.as_ref(), Some(&task));

    // Every entry inherited the service even though only the first event
    // carried it.
    assert!(entries
        .iter()
        .all(|entry| entry.service_id.as_ref() == Some(&service())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn managed_process_output_flows_into_the_hub() {
    let manager = ProcessManager::new().expect("runtime is available");
    let hub = Arc::new(LogHub::new());
    let cancellation = Cancellation::new();

    let ingestor = ProcessLogIngestor::new(Arc::clone(&hub), manager.clone());
    let running = ingestor.run(Some(cancellation.clone()));
    let handle = tokio::spawn(running);
    // Let the ingestor subscribe before the process starts emitting.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let process = manager
        .spawn(
            ProcessSpec::new("/bin/sh")
                .expect("program is valid")
                .args(["-c", "printf 'ingested line\\n'"])
                .for_service(service()),
        )
        .expect("spawn succeeds");
    process.wait().await.expect("wait succeeds");

    let recorded = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let entries = hub.query(&LogRequest::new(service())).unwrap();
            if let Some(entry) = entries
                .into_iter()
                .find(|entry| entry.message == "ingested line")
            {
                break entry;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the process line was never ingested");

    assert_eq!(
        recorded.source,
        service_orchestrator_application::LogSource::Process
    );
    assert!(recorded.process_id.is_some());

    cancellation.cancel();
    let _ = handle.await;
}

#[tokio::test]
async fn application_queries_and_diagnostics_use_the_real_hub() {
    let hub = Arc::new(LogHub::new());
    hub.record(
        LogEntry::new(LogLevel::Error, "connection refused on port 8081").with_service(service()),
    );
    hub.record(LogEntry::new(LogLevel::Info, "started").with_service(service()));
    let app = app_with_logs(Arc::clone(&hub));

    // The diagnostic combines stored logs with the (empty) health and runtime
    // state, and reports the observed error without inventing a root cause.
    let diagnostics = app.get_service_diagnostics(&service()).unwrap();
    assert!(diagnostics
        .recent_errors
        .iter()
        .any(|line| line.contains("connection refused")));
    let failure = diagnostics.last_failure.expect("a failure is reported");
    assert_eq!(failure.source, "log");
    assert!(failure.message.contains("connection refused"));

    // The oldest-first history page is served by the hub.
    let page = app
        .get_service_logs(&service(), &LogRequest::new(service()))
        .unwrap();
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].message, "connection refused on port 8081");

    // Realtime streaming is wired through the same application boundary.
    let mut subscription = app.subscribe_service_logs(&service()).unwrap();
    hub.record(LogEntry::new(LogLevel::Warn, "still degraded").with_service(service()));
    assert_eq!(subscription.recv().await.unwrap().message, "still degraded");
}
