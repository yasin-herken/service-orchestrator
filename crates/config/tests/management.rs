//! Configuration management integration tests.
//!
//! These exercise the mutation pipeline end to end against real temporary
//! files: format-preserving edits, validation-before-write, atomic persistence
//! with backups, revision conflicts, path safety, and diffing.

use std::path::{Path, PathBuf};

use service_orchestrator_config::management::{
    AffectedReference, ConfigChange, ConfigRepository, ConfigurationDiff, EntityChange, EntityKind,
};
use service_orchestrator_config::raw::{RawGroup, RawService, RawTemplate, RawWorkspace};
use service_orchestrator_config::ConfigError;

const BASE: &str = r#"# acme workspace configuration
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

/// Creates an isolated temporary directory for one test.
struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "so-config-management-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        path.push(unique);
        std::fs::create_dir_all(&path).expect("the temporary directory is created");
        Self { path }
    }

    fn config_path(&self) -> PathBuf {
        self.path.join("config.toml")
    }

    fn write_config(&self, text: &str) -> PathBuf {
        let path = self.config_path();
        std::fs::write(&path, text).expect("the fixture config is written");
        path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn raw_service(id: &str, workspace: &str) -> RawService {
    toml::from_str(&format!(
        r#"
        id = "{id}"
        type = "backend"
        workspace = "{workspace}"
        [commands]
        build = "./mvnw clean install"
        start = "./mvnw spring-boot:run"
        "#
    ))
    .expect("the raw service parses")
}

fn raw_workspace(id: &str, root: &str) -> RawWorkspace {
    toml::from_str(&format!(
        r#"
        id = "{id}"
        root = "{root}"
        "#
    ))
    .expect("the raw workspace parses")
}

fn raw_group(id: &str, services: &[&str]) -> RawGroup {
    let members = services
        .iter()
        .map(|service| format!("\"{service}\""))
        .collect::<Vec<_>>()
        .join(", ");
    toml::from_str(&format!(
        r#"
        id = "{id}"
        services = [{members}]
        "#
    ))
    .expect("the raw group parses")
}

fn service_change(entity: RawService) -> ConfigChange {
    ConfigChange::service(EntityChange::Create { entity })
}

fn raw_template(id: &str, category: &str) -> RawTemplate {
    toml::from_str(&format!(
        r#"
        id = "{id}"
        category = "{category}"
        "#
    ))
    .expect("the raw template parses")
}

fn assert_no_pending_temp(dir: &Path) {
    for entry in std::fs::read_dir(dir).expect("the directory is readable") {
        let name = entry.expect("the entry is readable").file_name();
        let name = name.to_string_lossy();
        assert!(
            !name.contains(".tmp-"),
            "a temporary file was left behind: {name}"
        );
    }
}

// ---------------------------------------------------------------------------

#[test]
fn create_appends_and_preserves_comments() {
    let dir = TestDir::new("create");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());

    let snapshot = repository.snapshot().expect("a snapshot is read");
    let change = service_change(raw_service("payment-service", "acme"));

    let preview = repository
        .preview(&change, &snapshot.revision)
        .expect("the change previews");
    assert!(preview.valid, "the proposal is valid: {:?}", preview.issues);
    assert_eq!(preview.diff.added.len(), 1);
    assert_eq!(preview.diff.added[0].id, "payment-service");

    let config = repository
        .commit(&change, &snapshot.revision)
        .expect("the change commits");
    assert!(config
        .workspace(&service_orchestrator_domain::WorkspaceId::new("acme").unwrap())
        .is_some());

    let written = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(
        written.contains("# acme workspace configuration"),
        "the leading comment is preserved:\n{written}"
    );
    assert!(written.contains("payment-service"));

    let backup = path.with_extension("toml.bak");
    assert!(backup.exists(), "a backup of the previous file is kept");
    let original = std::fs::read_to_string(&backup).expect("the backup is readable");
    assert!(!original.contains("payment-service"));

    assert_no_pending_temp(&dir.path);
}

#[test]
fn stale_revision_is_rejected() {
    let dir = TestDir::new("stale");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());

    let change = service_change(raw_service("payment-service", "acme"));
    let error = repository
        .preview(&change, "deadbeefdeadbeef")
        .expect_err("a stale revision is rejected");
    assert!(
        matches!(error, ConfigError::Conflict { .. }),
        "expected a conflict, got {error:?}"
    );

    let error = repository
        .commit(&change, "deadbeefdeadbeef")
        .expect_err("a stale commit is rejected");
    assert!(matches!(error, ConfigError::Conflict { .. }));

    let unchanged = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(!unchanged.contains("payment-service"));
}

#[test]
fn an_invalid_change_is_not_persisted() {
    let dir = TestDir::new("invalid");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    // The service references a workspace that does not exist.
    let change = service_change(raw_service("payment-service", "ghost"));

    let preview = repository
        .preview(&change, &revision)
        .expect("the change previews");
    assert!(!preview.valid, "the proposal must be reported invalid");
    assert!(!preview.issues.is_empty());

    let error = repository
        .commit(&change, &revision)
        .expect_err("an invalid change is rejected");
    assert!(
        matches!(error, ConfigError::Validation(_)),
        "expected a validation error, got {error:?}"
    );

    let unchanged = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(
        !unchanged.contains("payment-service"),
        "the file is untouched after an invalid change"
    );
}

#[test]
fn update_must_not_rename_the_entity() {
    let dir = TestDir::new("rename");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path);
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let change = ConfigChange::service(EntityChange::Update {
        id: "auth-service".to_owned(),
        entity: raw_service("renamed-service", "acme"),
    });
    let error = repository
        .preview(&change, &revision)
        .expect_err("a rename is rejected");
    assert!(
        matches!(error, ConfigError::InvalidChange { .. }),
        "expected an invalid-change error, got {error:?}"
    );
}

#[test]
fn update_replaces_only_the_targeted_entity() {
    let dir = TestDir::new("update");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let mut entity = raw_service("auth-service", "acme");
    entity.name = Some("Auth Service".to_owned());
    entity.default_branch = Some("main".to_owned());
    let change = ConfigChange::service(EntityChange::Update {
        id: "auth-service".to_owned(),
        entity,
    });

    let preview = repository
        .preview(&change, &revision)
        .expect("the change previews");
    assert!(preview.valid);
    assert_eq!(preview.diff.updated.len(), 1);
    let updated = &preview.diff.updated[0];
    assert_eq!(updated.id, "auth-service");
    assert!(updated
        .fields
        .iter()
        .any(|field| field.field == "default_branch"));

    repository
        .commit(&change, &revision)
        .expect("the change commits");
    let written = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(written.contains("Auth Service"));
    assert!(written.contains("# acme workspace configuration"));
}

#[test]
fn removing_a_service_reports_affected_references() {
    let dir = TestDir::new("remove");
    let group = raw_group("core", &["auth-service"]);
    let group_toml = toml::to_string(&group).expect("the group serializes");
    let text = format!("{BASE}\n[[groups]]\n{group_toml}");
    let path = dir.write_config(&text);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let change = ConfigChange::service(EntityChange::Remove {
        id: "auth-service".to_owned(),
    });
    let preview = repository
        .preview(&change, &revision)
        .expect("the change previews");

    assert_eq!(preview.diff.removed.len(), 1);
    assert!(
        preview
            .affected_references
            .iter()
            .any(
                |reference: &AffectedReference| reference.source_kind == EntityKind::Group
                    && reference.source_id == "core"
            ),
        "the referring group is reported: {:?}",
        preview.affected_references
    );
    assert!(
        !preview.valid,
        "removing a referenced service is not directly valid"
    );
}

#[test]
fn duplicate_create_is_rejected() {
    let dir = TestDir::new("duplicate");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path);
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let change = service_change(raw_service("auth-service", "acme"));
    let error = repository
        .preview(&change, &revision)
        .expect_err("a duplicate id is rejected");
    assert!(
        matches!(error, ConfigError::EntityExists { .. }),
        "expected an entity-exists error, got {error:?}"
    );
}

#[test]
fn creating_a_workspace_round_trips() {
    let dir = TestDir::new("workspace");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let change = ConfigChange::workspace(EntityChange::Create {
        entity: raw_workspace("beta", "/tmp/beta"),
    });
    let preview = repository
        .preview(&change, &revision)
        .expect("the change previews");
    assert!(preview.valid, "{:?}", preview.issues);

    repository
        .commit(&change, &revision)
        .expect("the change commits");
    let snapshot = repository.snapshot().expect("a snapshot is read");
    assert!(snapshot.workspaces.iter().any(|ws| ws.id == "beta"));
}

#[test]
fn the_diff_is_empty_for_a_no_op_update() {
    let dir = TestDir::new("noop");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path);
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let change = ConfigChange::service(EntityChange::Update {
        id: "auth-service".to_owned(),
        entity: raw_service("auth-service", "acme"),
    });
    let preview = repository
        .preview(&change, &revision)
        .expect("the change previews");
    assert!(preview.valid);
    assert_eq!(preview.diff, ConfigurationDiff::default());
    assert_eq!(preview.diff.change_count(), 0);
}

#[test]
fn revision_changes_when_the_file_changes() {
    let dir = TestDir::new("revision");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let before = repository.snapshot().expect("a snapshot is read").revision;
    std::fs::write(&path, format!("{BASE}\n# trailing comment\n")).expect("the file is rewritten");
    let after = repository.snapshot().expect("a snapshot is read").revision;
    assert_ne!(before, after, "an out-of-band edit changes the revision");
}

#[test]
fn a_valid_batch_commits_every_change_atomically() {
    let dir = TestDir::new("batch-valid");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let changes = vec![
        service_change(raw_service("payment-service", "acme")),
        service_change(raw_service("billing-service", "acme")),
    ];
    let preview = repository
        .preview_batch(&changes, &revision)
        .expect("the batch previews");
    assert!(preview.valid, "{:?}", preview.issues);
    assert_eq!(preview.diff.added.len(), 2);

    repository
        .commit_batch(&changes, &revision)
        .expect("the batch commits");

    let snapshot = repository.snapshot().expect("a snapshot is read");
    for id in ["payment-service", "billing-service"] {
        assert!(
            snapshot.services.iter().any(|service| service.id == id),
            "{id} is committed with the batch"
        );
    }
    assert_no_pending_temp(&dir.path);
}

#[test]
fn an_invalid_change_aborts_the_whole_batch() {
    let dir = TestDir::new("batch-invalid");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    // The second change references a workspace that does not exist.
    let changes = vec![
        service_change(raw_service("payment-service", "acme")),
        service_change(raw_service("billing-service", "ghost")),
    ];
    let preview = repository
        .preview_batch(&changes, &revision)
        .expect("the batch previews");
    assert!(!preview.valid, "the invalid member invalidates the batch");

    let error = repository
        .commit_batch(&changes, &revision)
        .expect_err("an invalid batch is rejected");
    assert!(
        matches!(error, ConfigError::Validation(_)),
        "expected a validation error, got {error:?}"
    );

    let unchanged = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(
        !unchanged.contains("payment-service") && !unchanged.contains("billing-service"),
        "not even the valid member is persisted:\n{unchanged}"
    );
    assert_no_pending_temp(&dir.path);
}

#[test]
fn a_duplicate_create_aborts_the_whole_batch() {
    let dir = TestDir::new("batch-duplicate");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let changes = vec![
        service_change(raw_service("payment-service", "acme")),
        service_change(raw_service("payment-service", "acme")),
    ];
    let error = repository
        .commit_batch(&changes, &revision)
        .expect_err("a duplicate id in a batch is rejected");
    assert!(
        matches!(error, ConfigError::EntityExists { .. }),
        "expected an entity-exists error, got {error:?}"
    );

    let unchanged = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(!unchanged.contains("payment-service"));
}

#[test]
fn a_template_change_round_trips() {
    let dir = TestDir::new("template");
    let path = dir.write_config(BASE);
    let repository = ConfigRepository::new(path.clone());
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let create = ConfigChange::template(EntityChange::Create {
        entity: raw_template("spring-boot-backend", "spring_boot_backend"),
    });
    let preview = repository
        .preview(&create, &revision)
        .expect("the template previews");
    assert!(preview.valid, "{:?}", preview.issues);
    assert_eq!(preview.diff.added[0].kind, EntityKind::Template);

    repository
        .commit(&create, &revision)
        .expect("the template commits");
    let snapshot = repository.snapshot().expect("a snapshot is read");
    assert!(snapshot
        .templates
        .iter()
        .any(|template| template.id == "spring-boot-backend"));

    // Renaming is modeled as create + remove: an update must keep the id.
    let revision = snapshot.revision.clone();
    let mut updated = raw_template("spring-boot-backend", "spring_boot_backend");
    updated.name = Some("Spring Boot Backend".to_owned());
    let update = ConfigChange::template(EntityChange::Update {
        id: "spring-boot-backend".to_owned(),
        entity: updated,
    });
    repository
        .commit(&update, &revision)
        .expect("the template update commits");
    let written = std::fs::read_to_string(&path).expect("the file is readable");
    assert!(written.contains("Spring Boot Backend"));

    let revision = repository.snapshot().expect("a snapshot is read").revision;
    let remove = ConfigChange::template(EntityChange::Remove {
        id: "spring-boot-backend".to_owned(),
    });
    repository
        .commit(&remove, &revision)
        .expect("the template removal commits");
    let snapshot = repository.snapshot().expect("a snapshot is read");
    assert!(snapshot.templates.is_empty());
}

#[test]
fn removing_a_migration_preset_reports_its_references() {
    let text = format!(
        r#"{BASE}
[services.liquibase]
preset = "custom"

[[migration_presets]]
id = "custom"
goals = ["liquibase:update"]
"#
    );
    let dir = TestDir::new("preset-refs");
    let path = dir.write_config(&text);
    let repository = ConfigRepository::new(path);
    let revision = repository.snapshot().expect("a snapshot is read").revision;

    let remove = ConfigChange::migration_preset(EntityChange::Remove {
        id: "custom".to_owned(),
    });
    let preview = repository
        .preview(&remove, &revision)
        .expect("the removal previews");

    assert!(
        preview.affected_references.iter().any(|reference| {
            reference.source_kind == EntityKind::Service
                && reference.source_id == "auth-service"
                && reference.relation == "migration_preset"
        }),
        "the referring service is reported: {:?}",
        preview.affected_references
    );
    assert!(
        !preview.valid,
        "removing a referenced preset leaves the configuration invalid"
    );
}
