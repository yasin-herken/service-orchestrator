//! Serialization contract for domain values.
//!
//! These tests use `serde_json`, the same format the MCP adapter will use, to
//! prove that domain values round-trip and that secrets are never emitted.

use service_orchestrator_domain::{
    AbsolutePath, Branch, CommandKind, CommandSet, CommandSpec, Environment, EnvironmentVariable,
    HealthCheck, HealthCheckKind, HttpHealthCheck, Port, RelativePath, Repository, Runtime,
    RuntimeKind, Service, ServiceId, ServiceType, Task, TaskId, TaskKind, TaskStatus, Timestamp,
    Version, Workspace, WorkspaceId,
};

fn service() -> Service {
    let mut commands = CommandSet::new();
    commands.set(
        CommandKind::Build,
        CommandSpec::new("./mvnw", ["clean", "install"]).unwrap(),
    );
    commands.set(
        CommandKind::Start,
        CommandSpec::new("./mvnw", ["spring-boot:run"]).unwrap(),
    );

    Service::new(
        ServiceId::new("auth-service").unwrap(),
        "auth-service",
        ServiceType::Backend,
    )
    .unwrap()
    .with_path(RelativePath::new("backend/auth-service").unwrap())
    .with_repository(
        Repository::new(
            "git@github.com:acme/auth-service.git",
            RelativePath::new("backend/auth-service").unwrap(),
            Branch::new("master").unwrap(),
        )
        .unwrap(),
    )
    .with_runtime(Runtime::versioned(
        RuntimeKind::Java,
        Version::new("21").unwrap(),
    ))
    .with_port(Port::new(8081).unwrap())
    .with_health_check(HealthCheck::new(HealthCheckKind::Http(
        HttpHealthCheck::new("http://localhost:8081/actuator/health", Some(200)).unwrap(),
    )))
    .with_commands(commands)
}

#[test]
fn service_round_trips_through_json() {
    let service = service();
    let json = serde_json::to_string(&service).unwrap();
    let restored: Service = serde_json::from_str(&json).unwrap();
    assert_eq!(service, restored);
}

#[test]
fn command_set_serializes_as_a_json_array() {
    let json = serde_json::to_string(&service().commands).unwrap();
    assert!(json.starts_with('['), "expected a JSON array, got {json}");
}

#[test]
fn secret_environment_values_are_never_serialized() {
    let mut environment = Environment::new();
    environment
        .set(EnvironmentVariable::new("APP_ENV", "local").unwrap())
        .unwrap();
    environment
        .set(EnvironmentVariable::secret("DB_PASSWORD", "hunter2").unwrap())
        .unwrap();

    let json = serde_json::to_string(&environment).unwrap();
    assert!(!json.contains("hunter2"), "secret leaked: {json}");
    assert!(json.contains("***"));
    assert!(json.contains("APP_ENV"));
}

#[test]
fn task_round_trips_with_timestamps_and_status() {
    let mut task = Task::new(TaskId::new("task-1").unwrap(), TaskKind::BuildService)
        .with_service(ServiceId::new("auth-service").unwrap())
        .depends_on(TaskId::new("task-0").unwrap());
    task.transition(TaskStatus::Running).unwrap();
    task.transition(TaskStatus::Succeeded).unwrap();

    let json = serde_json::to_string(&task).unwrap();
    let restored: Task = serde_json::from_str(&json).unwrap();
    assert_eq!(task, restored);
    assert!(restored.started_at.is_some());
    assert!(restored.finished_at.is_some());
}

#[test]
fn workspace_round_trips_through_json() {
    let workspace = Workspace::new(
        WorkspaceId::new("project-x").unwrap(),
        "Project X",
        AbsolutePath::new("/Users/dev/projects/project-x").unwrap(),
    )
    .unwrap()
    .with_service(service());

    let json = serde_json::to_string(&workspace).unwrap();
    let restored: Workspace = serde_json::from_str(&json).unwrap();
    assert_eq!(workspace, restored);
}

#[test]
fn timestamp_round_trips() {
    let timestamp = Timestamp::now();
    let json = serde_json::to_string(&timestamp).unwrap();
    let restored: Timestamp = serde_json::from_str(&json).unwrap();
    assert_eq!(timestamp, restored);
}
