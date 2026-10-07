//! Configuration system integration tests.
//!
//! These exercise the whole pipeline — parse, version gate, deserialize,
//! validate, normalize — against realistic valid documents and a battery of
//! invalid ones. They assert on the shape of successful configuration and on
//! the *kind* and *location* of failures.

use service_orchestrator_config::{
    load_from_str, ConfigError, ValidationErrors, DEFAULT_MAX_PARALLEL_TASKS,
    DEFAULT_TIMEOUT_SECONDS, SUPPORTED_CONFIG_VERSION,
};
use service_orchestrator_domain::{
    CommandKind, DependencyKind, DependencyTarget, ServiceType, WorkspaceId,
};

const ORIGIN: &str = "<test>";

fn load(text: &str) -> Result<service_orchestrator_config::Config, ConfigError> {
    load_from_str(text, ORIGIN)
}

fn validation_errors(error: &ConfigError) -> &ValidationErrors {
    match error {
        ConfigError::Validation(errors) => errors,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// A minimal but complete document: one workspace and one buildable/runnable
/// service.
const MINIMAL: &str = r#"
config_version = 1

[[workspaces]]
id = "acme"
root = "/tmp/acme"

[[services]]
id = "auth-service"
type = "backend"
workspace = "acme"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"
"#;

fn full_config() -> String {
    include_str!("../../../examples/config.toml").to_owned()
}

// ---------------------------------------------------------------------------
// Valid configuration
// ---------------------------------------------------------------------------

#[test]
fn loads_a_minimal_configuration() {
    let config = load(MINIMAL).expect("minimal configuration should load");
    assert_eq!(config.version, SUPPORTED_CONFIG_VERSION);
    assert_eq!(config.workspaces.len(), 1);
    assert_eq!(config.workspaces[0].workspace.services.len(), 1);
}

#[test]
fn loads_the_full_example_configuration() {
    let config = load(&full_config()).expect("example configuration should load");
    assert_eq!(config.workspaces.len(), 1);
    let workspace = &config.workspaces[0].workspace;
    assert_eq!(workspace.id, WorkspaceId::new("acme").unwrap());
    assert_eq!(workspace.services.len(), 2);
    assert_eq!(workspace.libraries.len(), 1);
    assert_eq!(workspace.groups.len(), 1);
    assert_eq!(workspace.profiles.len(), 1);
    assert_eq!(config.execution.max_parallel_tasks, 4);
    assert_eq!(config.execution.default_timeout_seconds, 1800);
}

#[test]
fn maps_services_onto_domain_values() {
    let config = load(&full_config()).unwrap();
    let auth = config
        .service(&"auth-service".parse().unwrap())
        .expect("auth-service exists");
    assert_eq!(auth.kind, ServiceType::Backend);
    assert!(auth.has_build_command());
    assert!(auth.has_start_command());
    assert_eq!(auth.ports.len(), 1);
    assert_eq!(auth.ports[0].value(), 8081);
    assert!(auth.repository.is_some());

    let web = config
        .service(&"web-ui".parse().unwrap())
        .expect("web-ui exists");
    assert_eq!(web.kind, ServiceType::Frontend);
    assert!(web.requires_runtime(&service_orchestrator_domain::RuntimeKind::Node));
}

#[test]
fn records_liquibase_configuration() {
    let config = load(&full_config()).unwrap();
    let workspace = &config.workspaces[0];
    let auth_id = "auth-service".parse().unwrap();
    let liquibase = workspace
        .liquibase
        .get(&auth_id)
        .expect("auth-service has liquibase config");
    assert!(liquibase.enabled);
    assert_eq!(liquibase.timeout_seconds, Some(300));
    assert!(liquibase.command.is_some());
}

#[test]
fn supports_multiple_workspaces_and_services() {
    let text = r#"
config_version = 1

[[workspaces]]
id = "one"
root = "/tmp/one"
[[workspaces]]
id = "two"
root = "/tmp/two"

[[services]]
id = "a"
type = "backend"
workspace = "one"
[services.commands]
build = "mvn build"
start = "mvn start"

[[services]]
id = "b"
type = "backend"
workspace = "two"
[services.commands]
build = "mvn build"
start = "mvn start"
"#;
    let config = load(text).unwrap();
    assert_eq!(config.workspaces.len(), 2);
    assert!(config.service(&"a".parse().unwrap()).is_some());
    assert!(config.service(&"b".parse().unwrap()).is_some());
}

#[test]
fn resolves_libraries_groups_and_profiles() {
    let config = load(&full_config()).unwrap();
    let workspace = &config.workspaces[0].workspace;
    assert!(workspace.library(&"common-core".parse().unwrap()).is_some());
    let group = workspace
        .group(&"customer".parse().unwrap())
        .expect("customer group exists");
    assert_eq!(group.services.len(), 2);
    let profile = workspace
        .profile(&"full-stack".parse().unwrap())
        .expect("full-stack profile exists");
    assert_eq!(profile.services.len(), 1);
    assert_eq!(profile.groups.len(), 1);
}

#[test]
fn applies_default_dependency_kinds() {
    let config = load(&full_config()).unwrap();
    let auth = config.service(&"auth-service".parse().unwrap()).unwrap();
    assert_eq!(auth.dependencies.len(), 1);
    assert_eq!(auth.dependencies[0].kind, DependencyKind::Build);
    assert!(matches!(
        auth.dependencies[0].target,
        DependencyTarget::Library(_)
    ));

    let web = config.service(&"web-ui".parse().unwrap()).unwrap();
    assert_eq!(web.dependencies[0].kind, DependencyKind::Runtime);
    assert!(matches!(
        web.dependencies[0].target,
        DependencyTarget::Service(_)
    ));
}

#[test]
fn struct_commands_and_custom_commands_are_supported() {
    let text = r#"
config_version = 1

[[workspaces]]
id = "acme"
root = "/tmp/acme"

[[services]]
id = "svc"
type = "backend"
workspace = "acme"

[services.commands]
build = { program = "./mvnw", args = ["clean", "install"] }
start = "./mvnw spring-boot:run"
[services.commands.custom]
lint = "npm run lint"
"#;
    let config = load(text).unwrap();
    let service = config.service(&"svc".parse().unwrap()).unwrap();
    let build = service.commands.get(&CommandKind::Build).unwrap();
    assert_eq!(build.program, "./mvnw");
    assert_eq!(build.args, vec!["clean", "install"]);
    assert!(service
        .commands
        .contains(&CommandKind::Other("lint".to_owned())));
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

#[test]
fn config_version_defaults_to_supported() {
    let text = MINIMAL.replace("config_version = 1", "");
    let config = load(&text).unwrap();
    assert_eq!(config.version, SUPPORTED_CONFIG_VERSION);
}

#[test]
fn execution_settings_default_safely() {
    let text = MINIMAL.replace("config_version = 1", "");
    let config = load(&text).unwrap();
    assert_eq!(
        config.execution.max_parallel_tasks,
        DEFAULT_MAX_PARALLEL_TASKS
    );
    assert_eq!(
        config.execution.default_timeout_seconds,
        DEFAULT_TIMEOUT_SECONDS
    );
}

#[test]
fn display_name_defaults_to_id() {
    let config = load(MINIMAL).unwrap();
    let service = config.service(&"auth-service".parse().unwrap()).unwrap();
    assert_eq!(service.display_name, "auth-service");
}

// ---------------------------------------------------------------------------
// Path handling
// ---------------------------------------------------------------------------

#[test]
fn keeps_relative_service_paths() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[services]]
id = "svc"
type = "backend"
workspace = "acme"
path = "backend/svc"
[services.commands]
build = "mvn build"
start = "mvn start"
"#;
    let config = load(text).unwrap();
    let service = config.service(&"svc".parse().unwrap()).unwrap();
    assert_eq!(service.path.as_ref().unwrap().as_str(), "backend/svc");
}

#[test]
fn rewrites_absolute_paths_inside_the_workspace() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[services]]
id = "svc"
type = "backend"
workspace = "acme"
path = "/tmp/acme/backend/svc"
[services.commands]
build = "mvn build"
start = "mvn start"
"#;
    let config = load(text).unwrap();
    let service = config.service(&"svc".parse().unwrap()).unwrap();
    assert_eq!(service.path.as_ref().unwrap().as_str(), "backend/svc");
}

#[test]
fn expands_home_relative_workspace_root() {
    let home = std::env::var("HOME").expect("HOME is set");
    let text = MINIMAL.replace("/tmp/acme", "~/Projects/acme");
    let config = load(&text).unwrap();
    let root = config.workspaces[0].workspace.root.as_str();
    assert_eq!(root, format!("{home}/Projects/acme"));
}

#[test]
fn expands_home_relative_service_path_against_the_root() {
    let home = std::env::var("HOME").expect("HOME is set");
    let text = format!(
        r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "{home}/ws"
[[services]]
id = "svc"
type = "backend"
workspace = "acme"
path = "~/ws/backend/svc"
[services.commands]
build = "mvn build"
start = "mvn start"
"#
    );
    let config = load(&text).unwrap();
    let service = config.service(&"svc".parse().unwrap()).unwrap();
    assert_eq!(service.path.as_ref().unwrap().as_str(), "backend/svc");
}

#[test]
fn rejects_paths_outside_the_workspace() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[services]]
id = "svc"
type = "backend"
workspace = "acme"
path = "/tmp/other/svc"
[services.commands]
build = "mvn build"
start = "mvn start"
"#;
    let error = load(text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("outside the workspace")));
}

#[test]
fn rejects_parent_traversal_in_paths() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[services]]
id = "svc"
type = "backend"
workspace = "acme"
path = "../escape"
[services.commands]
build = "mvn build"
start = "mvn start"
"#;
    let error = load(text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors.errors().iter().any(|e| e.location.contains("path")));
}

// ---------------------------------------------------------------------------
// Invalid configuration
// ---------------------------------------------------------------------------

#[test]
fn rejects_a_missing_required_field() {
    let text = MINIMAL.replace("root = \"/tmp/acme\"", "");
    let error = load(&text).unwrap_err();
    assert!(matches!(error, ConfigError::Parse { .. }));
    assert!(error.to_string().contains("root"));
}

#[test]
fn rejects_unknown_fields() {
    let text = format!("{MINIMAL}\n[execution]\nmax_paralel_tasks = 4\n");
    let error = load(&text).unwrap_err();
    assert!(matches!(error, ConfigError::Parse { .. }));
    assert!(error.to_string().contains("max_paralel_tasks"));
}

#[test]
fn rejects_an_invalid_service_type() {
    let text = MINIMAL.replace("type = \"backend\"", "type = \"banana\"");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("not a known service type")));
}

#[test]
fn rejects_a_library_typed_as_a_service() {
    let text = MINIMAL.replace("type = \"backend\"", "type = \"library\"");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors.errors().iter().any(|e| e
        .suggested_fix
        .as_deref()
        .unwrap_or("")
        .contains("[[libraries]]")));
}

#[test]
fn rejects_a_frontend_without_a_runtime() {
    let text = MINIMAL.replace("type = \"backend\"", "type = \"frontend\"");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("declares no runtime")));
}

#[test]
fn rejects_a_node_runtime_without_a_version() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[services]]
id = "web"
type = "frontend"
workspace = "acme"
[services.runtime]
type = "node"
[services.commands]
build = "npm run build"
start = "npm start"
"#;
    let error = load(text).unwrap_err();
    let errors = validation_errors(&error);
    let runtime_error = errors
        .errors()
        .iter()
        .find(|e| e.location.ends_with("runtime.version"))
        .expect("expected a runtime version error");
    assert!(runtime_error.problem.contains("version is missing"));
    assert!(runtime_error.expected.is_some());
}

#[test]
fn rejects_an_invalid_runtime_kind() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[services]]
id = "svc"
type = "backend"
workspace = "acme"
[services.runtime]
type = "cobol"
version = "1"
[services.commands]
build = "mvn build"
start = "mvn start"
"#;
    let error = load(text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("not a known runtime")));
}

#[test]
fn rejects_an_unknown_workspace() {
    let text = MINIMAL.replace("workspace = \"acme\"", "workspace = \"missing\"");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("unknown workspace")));
}

#[test]
fn rejects_an_unknown_service_dependency() {
    let text = format!("{MINIMAL}\n[services.dependencies]\nservices = [\"nope\"]\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("unknown service 'nope'")));
}

#[test]
fn rejects_an_unknown_library_dependency() {
    let text = format!("{MINIMAL}\n[services.dependencies]\nlibraries = [\"nope\"]\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("unknown library 'nope'")));
}

#[test]
fn rejects_duplicate_service_ids() {
    let text = format!(
        r#"{MINIMAL}

[[services]]
id = "auth-service"
type = "backend"
workspace = "acme"
[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"
"#
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("duplicate service id")));
}

#[test]
fn rejects_duplicate_workspace_ids() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[workspaces]]
id = "acme"
root = "/tmp/acme2"
"#;
    let error = load(text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("duplicate workspace id")));
}

#[test]
fn rejects_invalid_ports() {
    let text = format!("{MINIMAL}\n[[services.ports]]\nport = 0\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location.contains("ports[0]")));
}

#[test]
fn rejects_duplicate_ports_within_a_service() {
    let text =
        format!("{MINIMAL}\n[[services.ports]]\nport = 8080\n[[services.ports]]\nport = 8080\n");
    let error = load(&text).unwrap_err();
    assert!(!validation_errors(&error).errors().is_empty());
}

#[test]
fn rejects_self_dependencies() {
    let text = format!("{MINIMAL}\n[services.dependencies]\nservices = [\"auth-service\"]\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("depend on itself")));
}

#[test]
fn rejects_duplicate_dependency_references() {
    let text = format!(
        "{MINIMAL}\n[services.dependencies]\nservices = [\"other\", \"other\"]\n\n[[services]]\nid = \"other\"\ntype = \"backend\"\nworkspace = \"acme\"\n[services.commands]\nbuild = \"mvn build\"\nstart = \"mvn start\"\n"
    );
    let error = load(&text).unwrap_err();
    assert!(!validation_errors(&error).errors().is_empty());
}

#[test]
fn rejects_an_unsupported_config_version() {
    let text = MINIMAL.replace("config_version = 1", "config_version = 2");
    let error = load(&text).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::UnsupportedVersion { found: 2, .. }
    ));
}

#[test]
fn rejects_a_non_integer_config_version() {
    let text = MINIMAL.replace("config_version = 1", "config_version = \"one\"");
    let error = load(&text).unwrap_err();
    assert!(matches!(error, ConfigError::Validation(_)));
}

#[test]
fn rejects_an_empty_command() {
    let text = MINIMAL.replace("build = \"./mvnw clean install\"", "build = \"\"");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location.contains("commands.build")));
}

#[test]
fn rejects_a_missing_build_command_for_a_buildable_service() {
    let text = MINIMAL.replace("build = \"./mvnw clean install\"", "");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("has no build command")));
}

#[test]
fn rejects_a_health_check_without_its_required_field() {
    let text = format!("{MINIMAL}\n[[services.health_checks]]\ntype = \"http\"\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location.contains("health_checks[0].url")));
}

#[test]
fn rejects_zero_parallel_tasks() {
    let text = format!("{MINIMAL}\n[execution]\nmax_parallel_tasks = 0\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location == "execution.max_parallel_tasks"));
}

#[test]
fn reports_multiple_problems_at_once() {
    let text = r#"
config_version = 1
[[workspaces]]
id = "acme"
root = "/tmp/acme"
[[services]]
id = "one"
type = "banana"
workspace = "acme"
[[services]]
id = "two"
type = "frontend"
workspace = "acme"
[services.commands]
build = "npm run build"
start = "npm start"
"#;
    let error = load(text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors.errors().len() >= 2, "expected several problems");
    assert!(error
        .to_string()
        .contains("Configuration validation failed"));
}
