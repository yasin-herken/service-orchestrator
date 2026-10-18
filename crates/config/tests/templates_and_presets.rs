//! Integration tests for service templates, named migration presets, saved
//! operation presets, and repository-discovery settings (`tasks/032.md`).
//!
//! These exercise the new configuration entities through the real loader: the
//! raw TOML schema, normalization into validated values, cross-entity
//! reference checks, and the safety invariants around destructive presets.

use service_orchestrator_config::{
    load_from_str, Config, ConfigError, ValidationErrors, DEFAULT_DISCOVERY_MAX_DEPTH,
    DEFAULT_DISCOVERY_MAX_REPOSITORIES,
};
use service_orchestrator_domain::{
    LiquibaseExecutionMode, MigrationProviderType, MigrationTargetEnvironment, ServiceType,
};

const ORIGIN: &str = "<test>";

fn load(text: &str) -> Result<Config, ConfigError> {
    load_from_str(text, ORIGIN)
}

fn validation_errors(error: &ConfigError) -> &ValidationErrors {
    match error {
        ConfigError::Validation(errors) => errors,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

const BASE: &str = r#"
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

[services.liquibase]
enabled = true

[services.liquibase.database]
environment = "local"
"#;

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

const TEMPLATE: &str = r#"
[[templates]]
id = "spring-boot-backend"
name = "Spring Boot Backend"
category = "spring_boot_backend"
type = "backend"
default_branch = "master"
build_system = "maven"
build_operation = "build"
timeout_seconds = 900

[templates.runtime]
type = "java"
version = "21"

[templates.commands]
build = "./mvnw clean install -Dservice=${serviceName}"
start = "./mvnw spring-boot:run -Droot=${workspaceRoot}"

[[templates.health_checks]]
type = "http"
url = "http://localhost:${port}/actuator/health"
expected_status = 200

[[templates.health_checks]]
type = "process"

[templates.liquibase]
enabled = true
provider = "liquibase"
execution_mode = "maven"
preset = "update"

[templates.environment]
APP_ENV = "local"
SERVICE_HOME = "${repositoryPath}"
"#;

#[test]
fn loads_a_template_with_variables() {
    let text = format!("{BASE}\n{TEMPLATE}");
    let config = load(&text).expect("the template loads");
    let template = config
        .template(&"spring-boot-backend".parse().unwrap())
        .expect("the template exists");

    assert_eq!(template.name, "Spring Boot Backend");
    assert_eq!(
        template.category,
        Some(service_orchestrator_config::TemplateCategory::SpringBootBackend)
    );
    assert_eq!(template.service_type, Some(ServiceType::Backend));
    assert_eq!(template.resolved_service_type(), Some(ServiceType::Backend));
    assert_eq!(template.timeout_seconds, Some(900));
    assert_eq!(template.runtimes.len(), 1);
    assert_eq!(template.health_checks.len(), 2);

    let migration = template.migration.as_ref().expect("migration defaults");
    assert!(migration.enabled);
    assert_eq!(migration.provider, MigrationProviderType::Liquibase);
    assert_eq!(migration.execution_mode, LiquibaseExecutionMode::Maven);
    assert_eq!(
        migration.preset.as_ref().map(|id| id.as_str()),
        Some("update")
    );

    let build = template
        .commands
        .get(&service_orchestrator_domain::CommandKind::Build)
        .expect("the build command exists");
    assert_eq!(
        build.args,
        vec!["clean", "install", "-Dservice=${serviceName}"]
    );
}

#[test]
fn a_template_category_suggests_a_service_type() {
    let text =
        format!("{BASE}\n[[templates]]\nid = \"shared\"\ncategory = \"shared_maven_library\"\n");
    let config = load(&text).expect("the template loads");
    let template = config.template(&"shared".parse().unwrap()).unwrap();
    assert_eq!(template.service_type, None);
    assert_eq!(template.resolved_service_type(), Some(ServiceType::Library));
}

#[test]
fn rejects_an_unknown_template_variable() {
    let text = format!(
        "{BASE}\n[[templates]]\nid = \"bad\"\n[templates.commands]\nbuild = \"./mvnw clean -Dx=${{bogus}}\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(
        errors
            .errors()
            .iter()
            .any(|e| e.location.contains("templates[0].commands.build")
                && e.problem.contains("bogus")),
        "expected an unknown-variable error, got {:?}",
        errors.errors()
    );
}

#[test]
fn rejects_a_variable_in_a_template_program() {
    let text = format!(
        "{BASE}\n[[templates]]\nid = \"bad\"\n[templates.commands]\nbuild = \"${{repositoryPath}}/mvnw clean\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(
        errors
            .errors()
            .iter()
            .any(|e| e.problem.contains("program cannot use")),
        "expected a program-variable error, got {:?}",
        errors.errors()
    );
}

#[test]
fn rejects_a_malformed_template_variable() {
    let text = format!(
        "{BASE}\n[[templates]]\nid = \"bad\"\n[templates.commands]\nbuild = \"./mvnw ${{serviceName\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("unterminated")));
}

#[test]
fn rejects_a_fixed_port_health_check_in_a_template() {
    let text = format!(
        "{BASE}\n[[templates]]\nid = \"bad\"\n[[templates.health_checks]]\ntype = \"port\"\nport = 8081\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location.contains("health_checks[0].type")
            && e.problem.contains("cannot declare a fixed port")));
}

#[test]
fn rejects_an_unknown_preset_reference_from_a_template() {
    let text =
        format!("{BASE}\n[[templates]]\nid = \"bad\"\n[templates.liquibase]\npreset = \"ghost\"\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location.contains("templates[0].liquibase.preset")
            && e.problem.contains("unknown migration preset 'ghost'")));
}

#[test]
fn rejects_an_unsupported_template_provider() {
    let text = format!(
        "{BASE}\n[[templates]]\nid = \"bad\"\n[templates.liquibase]\nprovider = \"flyway\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("'flyway' is not supported")));
}

#[test]
fn rejects_a_branch_with_variables_in_a_template() {
    let text = format!(
        "{BASE}\n[[templates]]\nid = \"bad\"\ndefault_branch = \"release/${{serviceName}}\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("cannot use ${...} variables")));
}

#[test]
fn rejects_duplicate_template_ids() {
    let text = format!("{BASE}\n[[templates]]\nid = \"dup\"\n[[templates]]\nid = \"dup\"\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("duplicate service template id")));
}

// ---------------------------------------------------------------------------
// Migration presets
// ---------------------------------------------------------------------------

#[test]
fn built_in_migration_presets_are_always_available() {
    let config = load(BASE).expect("the base configuration loads");
    let presets = config.all_migration_presets();
    let ids: Vec<&str> = presets.iter().map(|preset| preset.id.as_str()).collect();
    assert_eq!(ids, vec!["status", "update", "reset_local"]);

    let reset = config
        .migration_preset(&"reset_local".parse().unwrap())
        .expect("the reset preset exists");
    assert!(reset.destructive);
    assert!(!reset.read_only);
    assert_eq!(reset.execution_mode, LiquibaseExecutionMode::Maven);
    assert_eq!(
        reset.goals,
        vec![
            "liquibase:dropAll".to_owned(),
            "liquibase:update".to_owned()
        ]
    );
    assert_eq!(
        reset.allowed_environments,
        vec![MigrationTargetEnvironment::Local]
    );
    assert!(reset.builtin);
}

#[test]
fn a_user_preset_overrides_a_built_in_by_id() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"update\"\ngoals = [\"liquibase:updateSQL\"]\ncontexts = [\"sql\", \"onlyDev\"]\n"
    );
    let config = load(&text).expect("the preset loads");
    let update = config.migration_preset(&"update".parse().unwrap()).unwrap();
    assert_eq!(update.goals, vec!["liquibase:updateSQL".to_owned()]);
    assert_eq!(
        update.contexts,
        vec!["sql".to_owned(), "onlyDev".to_owned()]
    );
    assert!(!update.builtin);
}

#[test]
fn loads_a_destructive_preset_with_allowed_environments() {
    let text = format!(
        r#"{BASE}
[[migration_presets]]
id = "reset_dev"
goals = ["liquibase:dropAll", "liquibase:update"]
contexts = ["sql", "data", "onlyDev"]
destructive = true
allowed_environments = ["local", "shared_development"]
timeout_seconds = 600
"#
    );
    let config = load(&text).expect("the preset loads");
    let preset = config
        .migration_preset(&"reset_dev".parse().unwrap())
        .unwrap();
    assert!(preset.destructive);
    assert!(preset.allows_environment(&MigrationTargetEnvironment::Local));
    assert!(preset.allows_environment(&MigrationTargetEnvironment::SharedDevelopment));
    assert!(!preset.allows_environment(&MigrationTargetEnvironment::Production));
    assert!(preset.contexts_for(&["ignored".to_owned()]).len() == 3);
}

#[test]
fn rejects_drop_all_without_destructive() {
    let text =
        format!("{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"liquibase:dropAll\"]\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("destroys database objects")));
}

#[test]
fn rejects_a_destructive_preset_without_allowed_environments() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"liquibase:dropAll\"]\ndestructive = true\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("must declare the environments")));
}

#[test]
fn rejects_unknown_in_allowed_environments() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"liquibase:dropAll\"]\ndestructive = true\nallowed_environments = [\"unknown\"]\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("never be allowed")));
}

#[test]
fn rejects_allowed_environments_on_a_non_destructive_preset() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"liquibase:update\"]\nallowed_environments = [\"local\"]\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("only applies to a destructive preset")));
}

#[test]
fn rejects_read_only_and_destructive_together() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"liquibase:dropAll\"]\ndestructive = true\nread_only = true\nallowed_environments = [\"local\"]\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("cannot both be true")));
}

#[test]
fn rejects_a_cli_execution_mode_for_a_preset() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"liquibase:update\"]\nexecution_mode = \"cli\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("only `maven` execution is supported")));
}

#[test]
fn rejects_a_non_liquibase_preset_provider() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"migrate\"]\nprovider = \"dbmate\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("'dbmate' is not supported")));
}

#[test]
fn rejects_an_option_as_a_goal() {
    let text = format!(
        "{BASE}\n[[migration_presets]]\nid = \"oops\"\ngoals = [\"-Dliquibase.contexts=sql\"]\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("is an option, not a goal")));
}

#[test]
fn rejects_an_empty_goal_list() {
    let text = format!("{BASE}\n[[migration_presets]]\nid = \"oops\"\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("at least one goal")));
}

// ---------------------------------------------------------------------------
// Service preset assignment and destructive safety
// ---------------------------------------------------------------------------

#[test]
fn a_service_may_use_a_built_in_preset() {
    let document = r#"
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

[services.liquibase]
enabled = true
preset = "update"

[services.liquibase.database]
environment = "local"
"#;
    let config = load(document).expect("the service loads");
    let workspace = config.workspace(&"acme".parse().unwrap()).unwrap();
    let settings = workspace
        .liquibase
        .get(&"auth-service".parse().unwrap())
        .expect("liquibase settings exist");
    assert_eq!(
        settings.preset.as_ref().map(|id| id.as_str()),
        Some("update")
    );
}

#[test]
fn rejects_an_unknown_service_preset() {
    let document = BASE.replace(
        "[services.liquibase.database]",
        "preset = \"ghost\"\n\n[services.liquibase.database]",
    );
    let error = load(&document).unwrap_err();
    let errors = validation_errors(&error);
    assert!(
        errors
            .errors()
            .iter()
            .any(|e| e.location.contains("services[0].liquibase.preset")
                && e.problem.contains("unknown migration preset 'ghost'")),
        "unexpected errors: {:?}",
        errors.errors()
    );
}

#[test]
fn a_destructive_service_preset_requires_an_allowed_declared_environment() {
    // The service declares `local`, the destructive preset allows `local`:
    // valid.
    let allowed = BASE.replace(
        "[services.liquibase.database]",
        "preset = \"reset_local\"\n\n[services.liquibase.database]",
    );
    load(&allowed).expect("an allowed environment passes");

    // The service declares `shared_development`: the precondition fails.
    let disallowed = allowed.replace(
        "environment = \"local\"",
        "environment = \"shared_development\"",
    );
    let error = load(&disallowed).unwrap_err();
    let errors = validation_errors(&error);
    assert!(
        errors
            .errors()
            .iter()
            .any(|e| e.problem.contains("may only target [local]")),
        "unexpected errors: {:?}",
        errors.errors()
    );

    // The service declares nothing: unknown can never be allowed.
    let undeclared = allowed.replace("environment = \"local\"", "");
    let error = load(&undeclared).unwrap_err();
    let errors = validation_errors(&error);
    assert!(
        errors
            .errors()
            .iter()
            .any(|e| e.problem.contains("declares no database environment")),
        "unexpected errors: {:?}",
        errors.errors()
    );
}

// ---------------------------------------------------------------------------
// Operation presets
// ---------------------------------------------------------------------------

#[test]
fn loads_a_saved_operation_preset() {
    let text = format!(
        r#"{BASE}
[[operation_presets]]
id = "backend-local"
name = "Backend Local Development"
workspace = "acme"
default = true
services = ["auth-service"]

[operation_presets.git]
enabled = true
branch = "master"

[operation_presets.libraries]
enabled = true

[operation_presets.migration]
enabled = true
preset = "update"

[operation_presets.build]
enabled = true
operation = "install_dependencies"

[operation_presets.runtime]
action = "start"

[operation_presets.health]
enabled = true
"#
    );
    let config = load(&text).expect("the operation preset loads");
    let preset = config
        .operation_preset(&"backend-local".parse().unwrap())
        .expect("the preset exists");
    assert_eq!(preset.name, "Backend Local Development");
    assert!(preset.default);
    assert_eq!(
        preset.workspace.as_ref().map(|id| id.as_str()),
        Some("acme")
    );
    assert_eq!(preset.services.len(), 1);
    assert!(preset.git.enabled);
    assert_eq!(
        preset.git.branch.as_ref().map(|b| b.as_str()),
        Some("master")
    );
    assert!(preset.libraries.enabled);
    assert!(preset.migration.enabled);
    assert_eq!(
        preset.migration.preset.as_ref().map(|id| id.as_str()),
        Some("update")
    );
    assert!(preset.build.enabled);
    assert_eq!(
        preset.build.operation,
        Some(service_orchestrator_domain::BuildOperation::InstallDependencies)
    );
    assert_eq!(
        preset.runtime.action,
        service_orchestrator_config::RuntimeAction::Start
    );
    assert!(preset.health.enabled);
}

#[test]
fn an_operation_preset_infers_its_workspace_from_members() {
    let text = format!(
        "{BASE}\n[[operation_presets]]\nid = \"inferred\"\nservices = [\"auth-service\"]\n[operation_presets.git]\nenabled = true\n"
    );
    let config = load(&text).expect("the preset loads");
    let preset = config
        .operation_preset(&"inferred".parse().unwrap())
        .unwrap();
    assert_eq!(
        preset.workspace.as_ref().map(|id| id.as_str()),
        Some("acme")
    );
    assert!(!preset.migration.enabled, "an omitted phase stays off");
}

#[test]
fn rejects_an_operation_preset_without_a_selection() {
    let text = format!("{BASE}\n[[operation_presets]]\nid = \"empty\"\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors.errors().iter().any(|e| e
        .problem
        .contains("selects no services, profiles, or groups")));
}

#[test]
fn rejects_an_operation_preset_with_an_unknown_service() {
    let text = format!(
        "{BASE}\n[[operation_presets]]\nid = \"bad\"\nworkspace = \"acme\"\nservices = [\"ghost\"]\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("unknown service 'ghost'")));
}

#[test]
fn rejects_an_operation_preset_with_an_unknown_migration_preset() {
    let text = format!(
        "{BASE}\n[[operation_presets]]\nid = \"bad\"\nservices = [\"auth-service\"]\n[operation_presets.migration]\nenabled = true\npreset = \"ghost\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("unknown migration preset 'ghost'")));
}

#[test]
fn rejects_two_default_presets_in_one_workspace() {
    let text = format!(
        r#"{BASE}
[[operation_presets]]
id = "one"
workspace = "acme"
default = true
services = ["auth-service"]

[[operation_presets]]
id = "two"
workspace = "acme"
default = true
services = ["auth-service"]
"#
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.problem.contains("already has a default operation preset")));
}

#[test]
fn rejects_an_unknown_runtime_action() {
    let text = format!(
        "{BASE}\n[[operation_presets]]\nid = \"bad\"\nservices = [\"auth-service\"]\n[operation_presets.runtime]\naction = \"explode\"\n"
    );
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors.errors().iter().any(|e| e
        .problem
        .contains("'explode' is not a known runtime action")));
}

// ---------------------------------------------------------------------------
// Discovery settings
// ---------------------------------------------------------------------------

#[test]
fn discovery_settings_default_safely() {
    let config = load(BASE).expect("the base configuration loads");
    assert_eq!(config.discovery.max_depth, DEFAULT_DISCOVERY_MAX_DEPTH);
    assert_eq!(
        config.discovery.max_repositories,
        DEFAULT_DISCOVERY_MAX_REPOSITORIES
    );
    assert!(config.discovery.follow_symlinks);
    assert!(config.discovery.exclude.contains(&".git".to_owned()));
    assert!(config
        .discovery
        .exclude
        .contains(&"node_modules".to_owned()));
}

#[test]
fn discovery_settings_merge_configured_excludes() {
    let text = format!(
        "{BASE}\n[discovery]\nmax_depth = 3\nfollow_symlinks = false\nexclude = [\"vendor\", \".git\"]\n"
    );
    let config = load(&text).expect("discovery settings load");
    assert_eq!(config.discovery.max_depth, 3);
    assert!(!config.discovery.follow_symlinks);
    // The built-in excludes always apply; configured extras are appended and
    // duplicates are removed.
    assert!(config.discovery.exclude.contains(&"vendor".to_owned()));
    assert_eq!(
        config
            .discovery
            .exclude
            .iter()
            .filter(|name| *name == ".git")
            .count(),
        1
    );
    assert!(config.discovery.exclude.contains(&"target".to_owned()));
}

#[test]
fn rejects_an_unbounded_discovery_depth() {
    let text = format!("{BASE}\n[discovery]\nmax_depth = 5000\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location == "discovery.max_depth"));
}

#[test]
fn rejects_a_zero_repository_cap() {
    let text = format!("{BASE}\n[discovery]\nmax_repositories = 0\n");
    let error = load(&text).unwrap_err();
    let errors = validation_errors(&error);
    assert!(errors
        .errors()
        .iter()
        .any(|e| e.location == "discovery.max_repositories"));
}
