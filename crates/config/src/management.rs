//! Safe configuration mutation, validation, diffing, and persistence.
//!
//! This module extends the read-only configuration pipeline with *controlled
//! transactions* (`tasks/023.md` §5–§8, §33–§37). A caller expresses a change
//! as a typed [`ConfigChange`]; the [`ConfigRepository`] applies it to a
//! format-preserving `toml_edit` document, re-validates the whole file with the
//! existing validator, computes a structured diff, and only then persists it
//! atomically. Nothing is written until validation succeeds, and a stale
//! revision is rejected rather than silently overwriting a newer edit.
//!
//! # Format preservation
//!
//! Editing goes through `toml_edit`, so only the changed array of tables is
//! rewritten; comments, blank lines, and formatting elsewhere in the file are
//! preserved (`tasks/023.md` §37). An entity that *is* rewritten is rendered in
//! a canonical form (nested tables become inline tables), which is the
//! documented limitation of this approach.
//!
//! # Single source of truth
//!
//! The daemon owns the only writer. [`FileConfigManager`] keeps the validated
//! [`Config`] the rest of the application reads, and updates it only after a
//! commit or reload succeeds. There is no second configuration model: the
//! on-disk TOML remains authoritative and every edit round-trips through the
//! existing loader and validator.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::error::{ConfigError, ValidationError};
use crate::loader::{load_from_str, ConfigLoader, TomlFileLoader};
use crate::model::Config;
use crate::raw::{
    RawConfig, RawDiscovery, RawExecution, RawGroup, RawLibrary, RawMigrationPreset,
    RawOperationPreset, RawProfile, RawService, RawTemplate, RawWorkspace,
};

// ---------------------------------------------------------------------------
// Entity identification
// ---------------------------------------------------------------------------

/// The kind of configuration entity a change targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// A workspace.
    Workspace,
    /// A service.
    Service,
    /// A shared library.
    Library,
    /// A developer profile.
    Profile,
    /// A service group.
    Group,
    /// A reusable service template (`tasks/032.md` §3).
    Template,
    /// A named migration preset (`tasks/032.md` §16).
    MigrationPreset,
    /// A saved bulk-operation preset (`tasks/032.md` §36).
    OperationPreset,
}

impl EntityKind {
    /// Every entity kind, in configuration-file order.
    #[must_use]
    pub const fn all() -> [EntityKind; 8] {
        [
            EntityKind::Workspace,
            EntityKind::Service,
            EntityKind::Library,
            EntityKind::Profile,
            EntityKind::Group,
            EntityKind::Template,
            EntityKind::MigrationPreset,
            EntityKind::OperationPreset,
        ]
    }

    /// The stable wire/identifier spelling, for example `service`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            EntityKind::Workspace => "workspace",
            EntityKind::Service => "service",
            EntityKind::Library => "library",
            EntityKind::Profile => "profile",
            EntityKind::Group => "group",
            EntityKind::Template => "template",
            EntityKind::MigrationPreset => "migration_preset",
            EntityKind::OperationPreset => "operation_preset",
        }
    }

    /// The singular label used in error messages.
    #[must_use]
    pub const fn label(self) -> &'static str {
        self.as_str()
    }

    /// The TOML array-of-tables key that holds this kind.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            EntityKind::Workspace => "workspaces",
            EntityKind::Service => "services",
            EntityKind::Library => "libraries",
            EntityKind::Profile => "profiles",
            EntityKind::Group => "groups",
            EntityKind::Template => "templates",
            EntityKind::MigrationPreset => "migration_presets",
            EntityKind::OperationPreset => "operation_presets",
        }
    }
}

/// A create/update/remove change to a single raw entity of type `T`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EntityChange<T> {
    /// Adds a new entity. Fails if its id already exists.
    Create {
        /// The complete raw entity.
        entity: T,
    },
    /// Replaces an existing entity. `id` names the target; the payload must
    /// carry the same id.
    Update {
        /// The existing entity id.
        id: String,
        /// The complete replacement entity.
        entity: T,
    },
    /// Removes an existing entity.
    Remove {
        /// The entity id to remove.
        id: String,
    },
}

impl<T> EntityChange<T> {
    /// The stable action spelling: `create`, `update`, or `remove`.
    #[must_use]
    pub const fn action(&self) -> &'static str {
        match self {
            EntityChange::Create { .. } => "create",
            EntityChange::Update { .. } => "update",
            EntityChange::Remove { .. } => "remove",
        }
    }

    /// The id the change targets, if it targets an existing entity.
    #[must_use]
    pub fn target_id(&self) -> Option<&str> {
        match self {
            EntityChange::Create { .. } => None,
            EntityChange::Update { id, .. } | EntityChange::Remove { id } => Some(id.as_str()),
        }
    }
}

/// A single configuration change, tagged by entity kind.
///
/// The JSON wire form is `{"kind": "service", "change": {...}}`, which lets the
/// desktop adapter decode a change without a second schema
/// (`tasks/023.md` §41).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "change", rename_all = "snake_case")]
pub enum ConfigChange {
    /// A workspace change.
    Workspace(Box<EntityChange<RawWorkspace>>),
    /// A service change.
    Service(Box<EntityChange<RawService>>),
    /// A library change.
    Library(Box<EntityChange<RawLibrary>>),
    /// A profile change.
    Profile(Box<EntityChange<RawProfile>>),
    /// A group change.
    Group(Box<EntityChange<RawGroup>>),
    /// A service-template change.
    Template(Box<EntityChange<RawTemplate>>),
    /// A migration-preset change.
    MigrationPreset(Box<EntityChange<RawMigrationPreset>>),
    /// A saved-operation-preset change.
    OperationPreset(Box<EntityChange<RawOperationPreset>>),
}

impl ConfigChange {
    /// The kind of entity this change targets.
    #[must_use]
    pub const fn kind(&self) -> EntityKind {
        match self {
            ConfigChange::Workspace(_) => EntityKind::Workspace,
            ConfigChange::Service(_) => EntityKind::Service,
            ConfigChange::Library(_) => EntityKind::Library,
            ConfigChange::Profile(_) => EntityKind::Profile,
            ConfigChange::Group(_) => EntityKind::Group,
            ConfigChange::Template(_) => EntityKind::Template,
            ConfigChange::MigrationPreset(_) => EntityKind::MigrationPreset,
            ConfigChange::OperationPreset(_) => EntityKind::OperationPreset,
        }
    }

    /// The stable action spelling.
    #[must_use]
    pub fn action(&self) -> &'static str {
        match self {
            ConfigChange::Workspace(change) => change.action(),
            ConfigChange::Service(change) => change.action(),
            ConfigChange::Library(change) => change.action(),
            ConfigChange::Profile(change) => change.action(),
            ConfigChange::Group(change) => change.action(),
            ConfigChange::Template(change) => change.action(),
            ConfigChange::MigrationPreset(change) => change.action(),
            ConfigChange::OperationPreset(change) => change.action(),
        }
    }

    /// The id the change targets, if it targets an existing entity.
    #[must_use]
    pub fn target_id(&self) -> Option<&str> {
        match self {
            ConfigChange::Workspace(change) => change.target_id(),
            ConfigChange::Service(change) => change.target_id(),
            ConfigChange::Library(change) => change.target_id(),
            ConfigChange::Profile(change) => change.target_id(),
            ConfigChange::Group(change) => change.target_id(),
            ConfigChange::Template(change) => change.target_id(),
            ConfigChange::MigrationPreset(change) => change.target_id(),
            ConfigChange::OperationPreset(change) => change.target_id(),
        }
    }

    /// A short human-readable description, for logs and events.
    ///
    /// A create names the entity being added (via its id), so the description
    /// always identifies the affected entity (`tasks/023.md` §37, §40).
    #[must_use]
    pub fn description(&self) -> String {
        let id = match self {
            ConfigChange::Workspace(change) => change_id(change, |entity| entity.id.as_str()),
            ConfigChange::Service(change) => change_id(change, |entity| entity.id.as_str()),
            ConfigChange::Library(change) => change_id(change, |entity| entity.id.as_str()),
            ConfigChange::Profile(change) => change_id(change, |entity| entity.id.as_str()),
            ConfigChange::Group(change) => change_id(change, |entity| entity.id.as_str()),
            ConfigChange::Template(change) => change_id(change, |entity| entity.id.as_str()),
            ConfigChange::MigrationPreset(change) => change_id(change, |entity| entity.id.as_str()),
            ConfigChange::OperationPreset(change) => change_id(change, |entity| entity.id.as_str()),
        };
        match id {
            Some(id) => format!("{} {} '{}'", self.action(), self.kind().label(), id),
            None => format!("{} {}", self.action(), self.kind().label()),
        }
    }
}

/// Returns the id a change carries, including the id of a created entity.
///
/// [`EntityChange::target_id`] deliberately returns `None` for a create (there
/// is no *existing* target); a description still needs to name the new entity.
fn change_id<'a, T>(
    change: &'a EntityChange<T>,
    id_of: impl Fn(&'a T) -> &'a str,
) -> Option<&'a str> {
    match change {
        EntityChange::Create { entity } => Some(id_of(entity)),
        EntityChange::Update { id, .. } | EntityChange::Remove { id } => Some(id.as_str()),
    }
}

/// Convenience constructors that box the entity payload.
impl ConfigChange {
    /// Wraps a workspace change.
    #[must_use]
    pub fn workspace(change: EntityChange<RawWorkspace>) -> Self {
        Self::Workspace(Box::new(change))
    }

    /// Wraps a service change.
    #[must_use]
    pub fn service(change: EntityChange<RawService>) -> Self {
        Self::Service(Box::new(change))
    }

    /// Wraps a library change.
    #[must_use]
    pub fn library(change: EntityChange<RawLibrary>) -> Self {
        Self::Library(Box::new(change))
    }

    /// Wraps a profile change.
    #[must_use]
    pub fn profile(change: EntityChange<RawProfile>) -> Self {
        Self::Profile(Box::new(change))
    }

    /// Wraps a group change.
    #[must_use]
    pub fn group(change: EntityChange<RawGroup>) -> Self {
        Self::Group(Box::new(change))
    }

    /// Wraps a service-template change.
    #[must_use]
    pub fn template(change: EntityChange<RawTemplate>) -> Self {
        Self::Template(Box::new(change))
    }

    /// Wraps a migration-preset change.
    #[must_use]
    pub fn migration_preset(change: EntityChange<RawMigrationPreset>) -> Self {
        Self::MigrationPreset(Box::new(change))
    }

    /// Wraps a saved-operation-preset change.
    #[must_use]
    pub fn operation_preset(change: EntityChange<RawOperationPreset>) -> Self {
        Self::OperationPreset(Box::new(change))
    }
}

// ---------------------------------------------------------------------------
// Diff and preview
// ---------------------------------------------------------------------------

/// A short identity for an entity in a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitySummary {
    /// The entity kind.
    pub kind: EntityKind,
    /// The entity id.
    pub id: String,
    /// The display name, when declared.
    pub name: Option<String>,
}

/// One changed field of an updated entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    /// The dotted field path, for example `runtime.version`.
    pub field: String,
    /// The previous value, rendered for display; `None` when the field was
    /// added. Secret values are redacted.
    pub before: Option<String>,
    /// The new value, rendered for display; `None` when the field was removed.
    /// Secret values are redacted.
    pub after: Option<String>,
}

/// An entity that changed in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatedEntity {
    /// The entity kind.
    pub kind: EntityKind,
    /// The entity id.
    pub id: String,
    /// The display name, when declared.
    pub name: Option<String>,
    /// The changed fields.
    pub fields: Vec<FieldChange>,
}

/// A structured, human-readable configuration diff.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationDiff {
    /// Entities created by the change.
    pub added: Vec<EntitySummary>,
    /// Entities changed in place.
    pub updated: Vec<UpdatedEntity>,
    /// Entities removed by the change.
    pub removed: Vec<EntitySummary>,
}

impl ConfigurationDiff {
    /// Returns `true` when the change is a no-op.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }

    /// The total number of affected entities.
    #[must_use]
    pub fn change_count(&self) -> usize {
        self.added.len() + self.updated.len() + self.removed.len()
    }

    /// The ids of every service the change touches.
    #[must_use]
    pub fn touched_service_ids(&self, removed: bool) -> Vec<String> {
        let mut ids = Vec::new();
        if !removed {
            for summary in &self.added {
                if summary.kind == EntityKind::Service {
                    ids.push(summary.id.clone());
                }
            }
        }
        for updated in &self.updated {
            if updated.kind == EntityKind::Service {
                ids.push(updated.id.clone());
            }
        }
        if removed {
            for summary in &self.removed {
                if summary.kind == EntityKind::Service {
                    ids.push(summary.id.clone());
                }
            }
        }
        ids
    }
}

/// A reference from another configured entity that points at a change target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AffectedReference {
    /// The kind of entity that holds the reference.
    pub source_kind: EntityKind,
    /// The id of the entity that holds the reference.
    pub source_id: String,
    /// The relation, for example `profile`, `group`, or `dependency`.
    pub relation: String,
    /// The referenced id.
    pub target: String,
}

/// The impact of a configuration change on long-running processes.
///
/// A configuration change never restarts or stops a service by itself
/// (`tasks/023.md` §9, §29): it only *future* operations. This records which
/// running services are affected so an interface can surface the required
/// action instead of silently doing it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningServiceImpact {
    /// The affected service id.
    pub service_id: String,
    /// Whether the service is currently running.
    pub running: bool,
    /// The change affecting it: `added`, `updated`, or `removed`.
    pub change: String,
    /// A short, actionable description.
    pub action_required: String,
}

/// The result of previewing a change without committing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigurationPreview {
    /// The revision the change was based on.
    pub revision: String,
    /// The revision the file would have after the change.
    pub proposed_revision: String,
    /// Whether the proposed configuration passes validation.
    pub valid: bool,
    /// The structured diff.
    pub diff: ConfigurationDiff,
    /// Validation problems; empty when [`ConfigurationPreview::valid`].
    pub issues: Vec<ValidationError>,
    /// References that would be affected by the change (mainly removals).
    pub affected_references: Vec<AffectedReference>,
    /// The impact on running processes. Filled in by the application layer,
    /// which knows the live process state; empty at the configuration layer.
    #[serde(default)]
    pub running_impact: Vec<RunningServiceImpact>,
}

/// A read-only snapshot of the raw configuration, suitable for editing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigurationSnapshot {
    /// The revision the snapshot was read at.
    pub revision: String,
    /// The declared schema version.
    pub config_version: u32,
    /// The raw workspaces.
    pub workspaces: Vec<RawWorkspace>,
    /// The raw services.
    pub services: Vec<RawService>,
    /// The raw libraries.
    pub libraries: Vec<RawLibrary>,
    /// The raw profiles.
    pub profiles: Vec<RawProfile>,
    /// The raw groups.
    pub groups: Vec<RawGroup>,
    /// The raw service templates.
    pub templates: Vec<RawTemplate>,
    /// The raw migration presets.
    pub migration_presets: Vec<RawMigrationPreset>,
    /// The raw saved-operation presets.
    pub operation_presets: Vec<RawOperationPreset>,
    /// The global execution settings.
    pub execution: RawExecution,
    /// The repository-discovery settings.
    pub discovery: RawDiscovery,
}

// ---------------------------------------------------------------------------
// The repository
// ---------------------------------------------------------------------------

/// A format-preserving view of the configuration file.
struct ConfigDocument {
    text: String,
    revision: String,
    doc: DocumentMut,
}

impl ConfigDocument {
    fn parse(text: String) -> Result<Self, ConfigError> {
        let revision = revision_of(&text);
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|error| ConfigError::Parse {
                path: "<memory>".to_owned(),
                message: error.to_string(),
            })?;
        Ok(Self {
            text,
            revision,
            doc,
        })
    }

    fn render(&self) -> String {
        self.doc.to_string()
    }

    fn raw(&self) -> Result<RawConfig, ConfigError> {
        toml::from_str(&self.text).map_err(|error| ConfigError::Parse {
            path: "<memory>".to_owned(),
            message: error.to_string(),
        })
    }

    fn apply(&mut self, change: &ConfigChange) -> Result<(), ConfigError> {
        match change {
            ConfigChange::Workspace(change) => {
                apply_entity(&mut self.doc, EntityKind::Workspace, change)
            }
            ConfigChange::Service(change) => {
                apply_entity(&mut self.doc, EntityKind::Service, change)
            }
            ConfigChange::Library(change) => {
                apply_entity(&mut self.doc, EntityKind::Library, change)
            }
            ConfigChange::Profile(change) => {
                apply_entity(&mut self.doc, EntityKind::Profile, change)
            }
            ConfigChange::Group(change) => apply_entity(&mut self.doc, EntityKind::Group, change),
            ConfigChange::Template(change) => {
                apply_entity(&mut self.doc, EntityKind::Template, change)
            }
            ConfigChange::MigrationPreset(change) => {
                apply_entity(&mut self.doc, EntityKind::MigrationPreset, change)
            }
            ConfigChange::OperationPreset(change) => {
                apply_entity(&mut self.doc, EntityKind::OperationPreset, change)
            }
        }
    }
}

/// Applies one entity change to the document.
fn apply_entity<T: RawEntity + Serialize>(
    doc: &mut DocumentMut,
    kind: EntityKind,
    change: &EntityChange<T>,
) -> Result<(), ConfigError> {
    match change {
        EntityChange::Create { entity } => {
            let element = entity_to_table(entity)?;
            let id = entity.id().trim().to_owned();
            let array = array_of_tables_mut(doc, kind, true)?;
            if find_index(array, &id).is_some() {
                return Err(ConfigError::EntityExists {
                    kind: kind.label(),
                    id,
                });
            }
            array.push(element);
            Ok(())
        }
        EntityChange::Update { id, entity } => {
            let new_id = entity.id().trim();
            if new_id != id.trim() {
                return Err(ConfigError::InvalidChange {
                    message: format!(
                        "an update must not rename {} '{}' to '{}'; remove and re-create it instead",
                        kind.label(),
                        id,
                        new_id
                    ),
                });
            }
            let element = entity_to_table(entity)?;
            let array = array_of_tables_mut(doc, kind, false)?;
            let index = find_index(array, id).ok_or_else(|| ConfigError::EntityNotFound {
                kind: kind.label(),
                id: id.clone(),
            })?;
            if let Some(slot) = array.get_mut(index) {
                *slot = element;
            }
            Ok(())
        }
        EntityChange::Remove { id } => {
            let array = array_of_tables_mut(doc, kind, false)?;
            let index = find_index(array, id).ok_or_else(|| ConfigError::EntityNotFound {
                kind: kind.label(),
                id: id.clone(),
            })?;
            array.remove(index);
            Ok(())
        }
    }
}

/// Returns the array of tables for `kind`, optionally creating it.
fn array_of_tables_mut(
    doc: &mut DocumentMut,
    kind: EntityKind,
    create: bool,
) -> Result<&mut ArrayOfTables, ConfigError> {
    let table = kind.table();
    if !doc.as_table().contains_key(table) {
        if !create {
            return Err(ConfigError::EntityNotFound {
                kind: kind.label(),
                id: String::new(),
            });
        }
        doc[table] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    doc[table]
        .as_array_of_tables_mut()
        .ok_or_else(|| ConfigError::Document {
            message: format!(
                "the '{table}' entry is not an array of tables; use [[{table}]] syntax instead"
            ),
        })
}

/// Finds the index of an entity with `id` in an array of tables.
fn find_index(array: &ArrayOfTables, id: &str) -> Option<usize> {
    let id = id.trim();
    array.iter().position(|table| {
        table
            .get("id")
            .and_then(Item::as_str)
            .map(str::trim)
            .is_some_and(|found| found == id)
    })
}

/// Serializes a typed entity into a TOML table.
fn entity_to_table<T: Serialize>(entity: &T) -> Result<Table, ConfigError> {
    toml_edit::ser::to_document(entity)
        .map(DocumentMut::into_table)
        .map_err(|error| ConfigError::Document {
            message: error.to_string(),
        })
}

// ---------------------------------------------------------------------------
// Raw entity access
// ---------------------------------------------------------------------------

/// The identity of a raw entity, used to diff and index them.
trait RawEntity {
    /// The declared id.
    fn id(&self) -> &str;
    /// The optional display name.
    fn name(&self) -> Option<&str>;
}

macro_rules! raw_entity {
    ($type:ty) => {
        impl RawEntity for $type {
            fn id(&self) -> &str {
                &self.id
            }
            fn name(&self) -> Option<&str> {
                self.name.as_deref()
            }
        }
    };
}

raw_entity!(RawWorkspace);
raw_entity!(RawService);
raw_entity!(RawLibrary);
raw_entity!(RawProfile);
raw_entity!(RawGroup);
raw_entity!(RawTemplate);
raw_entity!(RawMigrationPreset);
raw_entity!(RawOperationPreset);

// ---------------------------------------------------------------------------
// Diffing
// ---------------------------------------------------------------------------

/// Computes a structured diff between two raw configurations.
fn compute_diff(old: &RawConfig, new: &RawConfig) -> ConfigurationDiff {
    let mut diff = ConfigurationDiff::default();
    diff_kind(
        EntityKind::Workspace,
        &old.workspaces,
        &new.workspaces,
        &mut diff,
    );
    diff_kind(EntityKind::Service, &old.services, &new.services, &mut diff);
    diff_kind(
        EntityKind::Library,
        &old.libraries,
        &new.libraries,
        &mut diff,
    );
    diff_kind(EntityKind::Profile, &old.profiles, &new.profiles, &mut diff);
    diff_kind(EntityKind::Group, &old.groups, &new.groups, &mut diff);
    diff_kind(
        EntityKind::Template,
        &old.templates,
        &new.templates,
        &mut diff,
    );
    diff_kind(
        EntityKind::MigrationPreset,
        &old.migration_presets,
        &new.migration_presets,
        &mut diff,
    );
    diff_kind(
        EntityKind::OperationPreset,
        &old.operation_presets,
        &new.operation_presets,
        &mut diff,
    );
    diff
}

fn diff_kind<T: RawEntity + Serialize + PartialEq>(
    kind: EntityKind,
    old: &[T],
    new: &[T],
    diff: &mut ConfigurationDiff,
) {
    let old_map: BTreeMap<&str, &T> = old
        .iter()
        .map(|entity| (entity.id().trim(), entity))
        .collect();
    let new_map: BTreeMap<&str, &T> = new
        .iter()
        .map(|entity| (entity.id().trim(), entity))
        .collect();

    for (id, entity) in &new_map {
        match old_map.get(id) {
            None => diff.added.push(summary(kind, *entity)),
            Some(previous) => {
                if previous != entity {
                    diff.updated.push(UpdatedEntity {
                        kind,
                        id: (*id).to_owned(),
                        name: entity.name().map(str::to_owned),
                        fields: field_diff(*previous, *entity),
                    });
                }
            }
        }
    }
    for (id, entity) in &old_map {
        if !new_map.contains_key(id) {
            diff.removed.push(summary(kind, *entity));
        }
    }
}

fn summary<T: RawEntity>(kind: EntityKind, entity: &T) -> EntitySummary {
    EntitySummary {
        kind,
        id: entity.id().trim().to_owned(),
        name: entity.name().map(str::to_owned),
    }
}

/// Computes the changed fields between two entities of the same type.
fn field_diff<T: Serialize>(old: &T, new: &T) -> Vec<FieldChange> {
    let before = entity_fields(old);
    let after = entity_fields(new);
    let mut keys: Vec<&String> = before.keys().chain(after.keys()).collect();
    keys.sort();
    keys.dedup();

    let mut changes = Vec::new();
    for key in keys {
        let old_value = before.get(key);
        let new_value = after.get(key);
        if old_value != new_value {
            changes.push(FieldChange {
                field: key.clone(),
                before: old_value.cloned(),
                after: new_value.cloned(),
            });
        }
    }
    changes
}

/// Flattens an entity into dotted field paths with display values, redacting
/// secret values (`tasks/023.md` §34, §46).
fn entity_fields<T: Serialize>(entity: &T) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    if let Ok(value) = toml::Value::try_from(entity) {
        flatten("", &redact(value), &mut fields);
    }
    fields
}

/// Recursively redacts values marked `secret = true`.
fn redact(value: toml::Value) -> toml::Value {
    match value {
        toml::Value::Table(table) => {
            let is_secret = matches!(table.get("secret"), Some(toml::Value::Boolean(true)));
            let redacted = table
                .into_iter()
                .map(|(key, value)| {
                    let value = if is_secret && key == "value" {
                        toml::Value::String("***".to_owned())
                    } else {
                        redact(value)
                    };
                    (key, value)
                })
                .collect();
            toml::Value::Table(redacted)
        }
        toml::Value::Array(items) => toml::Value::Array(items.into_iter().map(redact).collect()),
        other => other,
    }
}

/// Flattens a TOML value into dotted paths.
fn flatten(prefix: &str, value: &toml::Value, out: &mut BTreeMap<String, String>) {
    match value {
        toml::Value::Table(table) => {
            if table.is_empty() {
                out.insert(prefix.to_owned(), "{}".to_owned());
            }
            for (key, value) in table {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, value, out);
            }
        }
        toml::Value::Array(items) => {
            if items.is_empty() {
                out.insert(prefix.to_owned(), "[]".to_owned());
            }
            for (index, item) in items.iter().enumerate() {
                flatten(&format!("{prefix}[{index}]"), item, out);
            }
        }
        other => {
            out.insert(prefix.to_owned(), render_scalar(other));
        }
    }
}

/// Renders a scalar TOML value for display.
fn render_scalar(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => text.clone(),
        toml::Value::Integer(number) => number.to_string(),
        toml::Value::Float(number) => number.to_string(),
        toml::Value::Boolean(flag) => flag.to_string(),
        toml::Value::Datetime(datetime) => datetime.to_string(),
        toml::Value::Array(_) | toml::Value::Table(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Affected references
// ---------------------------------------------------------------------------

/// Finds the references that point at `(kind, id)` in `raw`.
fn affected_references(raw: &RawConfig, kind: EntityKind, id: &str) -> Vec<AffectedReference> {
    let id = id.trim();
    let mut references = Vec::new();
    match kind {
        EntityKind::Workspace => {
            for service in &raw.services {
                if service.workspace.trim() == id {
                    references.push(reference(EntityKind::Service, &service.id, "workspace", id));
                }
            }
            for library in &raw.libraries {
                if library.workspace.trim() == id {
                    references.push(reference(EntityKind::Library, &library.id, "workspace", id));
                }
            }
            for profile in &raw.profiles {
                if profile.workspace.as_deref().map(str::trim) == Some(id) {
                    references.push(reference(EntityKind::Profile, &profile.id, "workspace", id));
                }
            }
            for group in &raw.groups {
                if group.workspace.as_deref().map(str::trim) == Some(id) {
                    references.push(reference(EntityKind::Group, &group.id, "workspace", id));
                }
            }
            for preset in &raw.operation_presets {
                if preset.workspace.as_deref().map(str::trim) == Some(id) {
                    references.push(reference(
                        EntityKind::OperationPreset,
                        &preset.id,
                        "workspace",
                        id,
                    ));
                }
            }
        }
        EntityKind::Service => {
            for group in &raw.groups {
                if group.services.iter().any(|member| member.trim() == id) {
                    references.push(reference(EntityKind::Group, &group.id, "service", id));
                }
            }
            for profile in &raw.profiles {
                if profile.services.iter().any(|member| member.trim() == id) {
                    references.push(reference(EntityKind::Profile, &profile.id, "service", id));
                }
            }
            for service in &raw.services {
                if service
                    .dependencies
                    .services
                    .iter()
                    .any(|dependency| dependency_id(dependency) == Some(id))
                {
                    references.push(reference(
                        EntityKind::Service,
                        &service.id,
                        "dependency",
                        id,
                    ));
                }
            }
            for preset in &raw.operation_presets {
                if preset.services.iter().any(|member| member.trim() == id) {
                    references.push(reference(
                        EntityKind::OperationPreset,
                        &preset.id,
                        "service",
                        id,
                    ));
                }
            }
        }
        EntityKind::Library => {
            for service in &raw.services {
                if service
                    .dependencies
                    .libraries
                    .iter()
                    .any(|dependency| dependency_id(dependency) == Some(id))
                {
                    references.push(reference(
                        EntityKind::Service,
                        &service.id,
                        "dependency",
                        id,
                    ));
                }
            }
            for library in &raw.libraries {
                if library
                    .dependencies
                    .libraries
                    .iter()
                    .any(|dependency| dependency_id(dependency) == Some(id))
                {
                    references.push(reference(
                        EntityKind::Library,
                        &library.id,
                        "dependency",
                        id,
                    ));
                }
            }
        }
        EntityKind::Group => {
            for profile in &raw.profiles {
                if profile.groups.iter().any(|member| member.trim() == id) {
                    references.push(reference(EntityKind::Profile, &profile.id, "group", id));
                }
            }
            for preset in &raw.operation_presets {
                if preset.groups.iter().any(|member| member.trim() == id) {
                    references.push(reference(
                        EntityKind::OperationPreset,
                        &preset.id,
                        "group",
                        id,
                    ));
                }
            }
        }
        EntityKind::Profile => {
            for preset in &raw.operation_presets {
                if preset.profiles.iter().any(|member| member.trim() == id) {
                    references.push(reference(
                        EntityKind::OperationPreset,
                        &preset.id,
                        "profile",
                        id,
                    ));
                }
            }
        }
        EntityKind::MigrationPreset => {
            for service in &raw.services {
                if service
                    .liquibase
                    .as_ref()
                    .and_then(|liquibase| liquibase.preset.as_deref())
                    .map(str::trim)
                    == Some(id)
                {
                    references.push(reference(
                        EntityKind::Service,
                        &service.id,
                        "migration_preset",
                        id,
                    ));
                }
            }
            for template in &raw.templates {
                if template
                    .liquibase
                    .as_ref()
                    .and_then(|liquibase| liquibase.preset.as_deref())
                    .map(str::trim)
                    == Some(id)
                {
                    references.push(reference(
                        EntityKind::Template,
                        &template.id,
                        "migration_preset",
                        id,
                    ));
                }
            }
            for preset in &raw.operation_presets {
                if preset
                    .migration
                    .as_ref()
                    .and_then(|migration| migration.preset.as_deref())
                    .map(str::trim)
                    == Some(id)
                {
                    references.push(reference(
                        EntityKind::OperationPreset,
                        &preset.id,
                        "migration_preset",
                        id,
                    ));
                }
            }
        }
        // Nothing references a template or an operation preset: both are
        // entry points a developer chooses, never targets of a reference.
        EntityKind::Template | EntityKind::OperationPreset => {}
    }
    references
}

fn reference(
    source_kind: EntityKind,
    source_id: &str,
    relation: &str,
    target: &str,
) -> AffectedReference {
    AffectedReference {
        source_kind,
        source_id: source_id.trim().to_owned(),
        relation: relation.to_owned(),
        target: target.to_owned(),
    }
}

/// Extracts the target id of a dependency reference.
fn dependency_id(dependency: &crate::raw::RawDependencyRef) -> Option<&str> {
    match dependency {
        crate::raw::RawDependencyRef::Id(id) => Some(id.trim()),
        crate::raw::RawDependencyRef::Detailed(detailed) => Some(detailed.id.trim()),
    }
}

// ---------------------------------------------------------------------------
// Revision
// ---------------------------------------------------------------------------

/// Hashes configuration bytes with FNV-1a, producing a stable revision string.
///
/// The hash is deliberately cheap and non-cryptographic: it only needs to
/// detect that the file changed between a read and a write, not resist
/// tampering.
#[must_use]
pub fn revision_of(text: &str) -> String {
    revision_of_bytes(text.as_bytes())
}

fn revision_of_bytes(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

// ---------------------------------------------------------------------------
// ConfigRepository
// ---------------------------------------------------------------------------

/// Reads, validates, and safely persists a TOML configuration file.
///
/// The repository is the only component that writes the file. Every mutation is
/// validated in memory first, the on-disk revision is re-checked immediately
/// before the atomic rename, and a `.bak` copy is kept.
#[derive(Debug, Clone)]
pub struct ConfigRepository {
    path: PathBuf,
}

impl ConfigRepository {
    /// Creates a repository over `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The configuration file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the current file revision, treating a missing file as empty.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Read`] when the file exists but cannot be read.
    pub fn revision(&self) -> Result<String, ConfigError> {
        read_revision(&self.path)
    }

    /// Reads a raw snapshot of the configuration for editing.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] when the file cannot be read or parsed.
    pub fn snapshot(&self) -> Result<ConfigurationSnapshot, ConfigError> {
        let text = self.read_text()?;
        let revision = revision_of(&text);
        let raw: RawConfig = toml::from_str(&text).map_err(|error| ConfigError::Parse {
            path: self.path.display().to_string(),
            message: error.to_string(),
        })?;
        Ok(ConfigurationSnapshot {
            revision,
            config_version: raw.config_version,
            workspaces: raw.workspaces,
            services: raw.services,
            libraries: raw.libraries,
            profiles: raw.profiles,
            groups: raw.groups,
            templates: raw.templates,
            migration_presets: raw.migration_presets,
            operation_presets: raw.operation_presets,
            execution: raw.execution,
            discovery: raw.discovery,
        })
    }

    /// Applies `change` in memory and returns the diff and validation result
    /// without writing anything.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Conflict`] when the file no longer matches
    /// `expected_revision`, and a structural [`ConfigError`] when the change
    /// cannot be applied (unknown entity, duplicate id, uneditable document).
    /// A change that applies but fails *validation* is reported through
    /// [`ConfigurationPreview::valid`] and
    /// [`ConfigurationPreview::issues`] instead of as an error.
    pub fn preview(
        &self,
        change: &ConfigChange,
        expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError> {
        self.preview_batch(std::slice::from_ref(change), expected_revision)
    }

    /// Applies a batch of changes in memory and returns the diff and validation
    /// result without writing anything.
    ///
    /// The batch is applied in order to a single document, so the preview and
    /// the subsequent commit see exactly the same result. This is what makes a
    /// bulk import atomic at the persistence boundary
    /// (`tasks/032.md` §15): the whole batch either validates and commits, or
    /// nothing is written.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] as described by [`ConfigRepository::preview`].
    /// A structural failure in any change aborts the whole batch.
    pub fn preview_batch(
        &self,
        changes: &[ConfigChange],
        expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError> {
        let mut document = self.read_document()?;
        ensure_revision(&self.path, &document.revision, expected_revision)?;
        let old_raw = document.raw()?;
        for change in changes {
            document.apply(change)?;
        }
        let rendered = document.render();
        let proposed_revision = revision_of(&rendered);
        let origin = self.path.display().to_string();
        let (valid, issues) = validate_rendered(&rendered, &origin);
        let new_raw = toml::from_str::<RawConfig>(&rendered).unwrap_or_else(|_| old_raw.clone());
        let diff = compute_diff(&old_raw, &new_raw);
        let mut affected = Vec::new();
        for change in changes {
            if let ("remove", Some(id)) = (change.action(), change.target_id()) {
                affected.extend(affected_references(&old_raw, change.kind(), id));
            }
        }
        Ok(ConfigurationPreview {
            revision: document.revision,
            proposed_revision,
            valid,
            diff,
            issues,
            affected_references: affected,
            running_impact: Vec::new(),
        })
    }

    /// Validates and persists `change`, returning the reloaded configuration.
    ///
    /// Nothing is written until the whole proposed file validates. The revision
    /// is re-checked immediately before the atomic rename so a concurrent edit
    /// is never lost.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Conflict`] for a stale revision,
    /// [`ConfigError::Validation`] when the proposal is invalid, and a
    /// structural or I/O [`ConfigError`] otherwise.
    pub fn commit(
        &self,
        change: &ConfigChange,
        expected_revision: &str,
    ) -> Result<Config, ConfigError> {
        self.commit_batch(std::slice::from_ref(change), expected_revision)
    }

    /// Validates and persists a batch of changes atomically, returning the
    /// reloaded configuration.
    ///
    /// The changes are applied in order to one document; the whole proposal is
    /// validated, and only then is the file written atomically. A failure at
    /// any point leaves the existing configuration untouched
    /// (`tasks/032.md` §15).
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] as described by [`ConfigRepository::commit`].
    pub fn commit_batch(
        &self,
        changes: &[ConfigChange],
        expected_revision: &str,
    ) -> Result<Config, ConfigError> {
        let mut document = self.read_document()?;
        ensure_revision(&self.path, &document.revision, expected_revision)?;
        for change in changes {
            document.apply(change)?;
        }
        let rendered = document.render();
        let origin = self.path.display().to_string();
        let config = load_from_str(&rendered, &origin)?;
        self.persist(&rendered, expected_revision)?;
        Ok(config)
    }

    /// Re-reads and validates the file from disk.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] when the file cannot be read, parsed, or
    /// validated.
    pub fn load(&self) -> Result<Config, ConfigError> {
        TomlFileLoader.load(&self.path)
    }

    /// Reads the file text, treating a missing file as empty.
    fn read_text(&self) -> Result<String, ConfigError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(source) => Err(ConfigError::Read {
                path: self.path.display().to_string(),
                source,
            }),
        }
    }

    fn read_document(&self) -> Result<ConfigDocument, ConfigError> {
        ConfigDocument::parse(self.read_text()?)
    }

    /// Writes `rendered` atomically, keeping a backup and preserving
    /// permissions.
    fn persist(&self, rendered: &str, expected_revision: &str) -> Result<(), ConfigError> {
        // Re-check just before the write so a concurrent writer is detected.
        let actual = read_revision(&self.path)?;
        ensure_revision(&self.path, &actual, expected_revision)?;

        let directory = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(directory).map_err(|source| ConfigError::Write {
            path: directory.display().to_string(),
            source,
        })?;

        let file_name = self
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config.toml".to_owned());
        let temp = self
            .path
            .with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));

        let write = || -> std::io::Result<()> {
            let mut file = fs::File::create(&temp)?;
            file.write_all(rendered.as_bytes())?;
            file.sync_all()?;
            Ok(())
        };
        write().map_err(|source| ConfigError::Write {
            path: temp.display().to_string(),
            source,
        })?;

        // Preserve the existing file's permissions, or restrict a new file to
        // the owner because configuration may reference secrets by name.
        match fs::metadata(&self.path) {
            Ok(metadata) => {
                let _ = fs::set_permissions(&temp, metadata.permissions());
            }
            Err(_) => set_owner_only(&temp),
        }

        // Keep a backup of the previous good file. The backup may carry secret
        // environment values (only by reference in the default model, but a
        // user may have inlined one), so it is restricted to the owner rather
        // than inheriting a potentially world-readable mode (`AGENTS.md` §31,
        // `tasks/026.md` §34).
        if self.path.exists() {
            let backup = self.path.with_extension("toml.bak");
            if fs::copy(&self.path, &backup).is_ok() {
                set_owner_only(&backup);
            }
        }

        fs::rename(&temp, &self.path).map_err(|source| {
            let _ = fs::remove_file(&temp);
            ConfigError::Write {
                path: self.path.display().to_string(),
                source,
            }
        })
    }
}

/// Restricts a newly created configuration file to the owner.
fn set_owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Reads a file's revision, treating a missing file as the empty revision.
fn read_revision(path: &Path) -> Result<String, ConfigError> {
    match fs::read(path) {
        Ok(bytes) => Ok(revision_of_bytes(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(revision_of_bytes(b"")),
        Err(source) => Err(ConfigError::Read {
            path: path.display().to_string(),
            source,
        }),
    }
}

/// Fails with [`ConfigError::Conflict`] when the revisions differ.
fn ensure_revision(path: &Path, actual: &str, expected: &str) -> Result<(), ConfigError> {
    if actual == expected {
        Ok(())
    } else {
        Err(ConfigError::Conflict {
            path: path.display().to_string(),
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
    }
}

/// Validates rendered text, returning `(valid, issues)`.
fn validate_rendered(rendered: &str, origin: &str) -> (bool, Vec<ValidationError>) {
    match load_from_str(rendered, origin) {
        Ok(_) => (true, Vec::new()),
        Err(ConfigError::Validation(errors)) => (false, errors.errors().to_vec()),
        Err(other) => (
            false,
            vec![ValidationError::new("config", other.to_string())],
        ),
    }
}

// ---------------------------------------------------------------------------
// ConfigManager
// ---------------------------------------------------------------------------

/// The configuration boundary the application core depends on.
///
/// It exposes the validated configuration the application reads plus the
/// transaction operations an interface drives. The daemon owns the only
/// implementation that writes a file ([`FileConfigManager`]); tests and
/// in-process embeds use [`StaticConfigManager`].
pub trait ConfigManager: Send + Sync {
    /// Returns the current validated configuration.
    fn current(&self) -> Arc<Config>;

    /// Returns the current on-disk revision.
    fn revision(&self) -> String;

    /// Reads a raw snapshot for editing.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] when the file cannot be read or parsed.
    fn snapshot(&self) -> Result<ConfigurationSnapshot, ConfigError>;

    /// Previews a change without committing it.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] as described by [`ConfigRepository::preview`].
    fn preview(
        &self,
        change: &ConfigChange,
        expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError>;

    /// Previews a batch of changes without committing any of them.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] as described by
    /// [`ConfigRepository::preview_batch`].
    fn preview_batch(
        &self,
        changes: &[ConfigChange],
        expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError>;

    /// Validates, persists, and reloads a change.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] as described by [`ConfigRepository::commit`].
    fn commit(&self, change: &ConfigChange, expected_revision: &str)
        -> Result<Config, ConfigError>;

    /// Validates, persists, and reloads a batch of changes atomically.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] as described by
    /// [`ConfigRepository::commit_batch`].
    fn commit_batch(
        &self,
        changes: &[ConfigChange],
        expected_revision: &str,
    ) -> Result<Config, ConfigError>;

    /// Re-reads and validates the file, updating the current configuration.
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] when the file cannot be read, parsed, or
    /// validated.
    fn reload(&self) -> Result<Arc<Config>, ConfigError>;

    /// Reads and validates the file **without** updating the current
    /// configuration.
    ///
    /// Used by `validate_configuration` so a read-only validation never changes
    /// the serving configuration (`tasks/023.md` §38).
    ///
    /// # Errors
    ///
    /// Returns a [`ConfigError`] when the file cannot be read, parsed, or
    /// validated.
    fn validate(&self) -> Result<(), ConfigError>;
}

/// The file-backed configuration manager: the daemon's configuration writer.
pub struct FileConfigManager {
    repository: ConfigRepository,
    current: RwLock<Arc<Config>>,
}

impl FileConfigManager {
    /// Creates a manager over `path` with the already-loaded `config`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, config: Config) -> Self {
        Self {
            repository: ConfigRepository::new(path),
            current: RwLock::new(Arc::new(config)),
        }
    }

    /// The underlying repository.
    #[must_use]
    pub fn repository(&self) -> &ConfigRepository {
        &self.repository
    }
}

impl ConfigManager for FileConfigManager {
    fn current(&self) -> Arc<Config> {
        Arc::clone(
            &self
                .current
                .read()
                .expect("configuration lock is not poisoned"),
        )
    }

    fn revision(&self) -> String {
        self.repository.revision().unwrap_or_default()
    }

    fn snapshot(&self) -> Result<ConfigurationSnapshot, ConfigError> {
        self.repository.snapshot()
    }

    fn preview(
        &self,
        change: &ConfigChange,
        expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError> {
        self.repository.preview(change, expected_revision)
    }

    fn preview_batch(
        &self,
        changes: &[ConfigChange],
        expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError> {
        self.repository.preview_batch(changes, expected_revision)
    }

    fn commit(
        &self,
        change: &ConfigChange,
        expected_revision: &str,
    ) -> Result<Config, ConfigError> {
        let config = self.repository.commit(change, expected_revision)?;
        *self
            .current
            .write()
            .expect("configuration lock is not poisoned") = Arc::new(config.clone());
        Ok(config)
    }

    fn commit_batch(
        &self,
        changes: &[ConfigChange],
        expected_revision: &str,
    ) -> Result<Config, ConfigError> {
        let config = self.repository.commit_batch(changes, expected_revision)?;
        *self
            .current
            .write()
            .expect("configuration lock is not poisoned") = Arc::new(config.clone());
        Ok(config)
    }

    fn reload(&self) -> Result<Arc<Config>, ConfigError> {
        let config = Arc::new(self.repository.load()?);
        *self
            .current
            .write()
            .expect("configuration lock is not poisoned") = Arc::clone(&config);
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        self.repository.load().map(|_| ())
    }
}

/// A configuration manager over a fixed, in-memory configuration.
///
/// Used by tests and in-process embeds that do not own a file. Mutation
/// operations report that configuration management is unavailable rather than
/// pretending to succeed.
pub struct StaticConfigManager {
    config: Arc<Config>,
}

impl StaticConfigManager {
    /// Wraps an already-validated configuration.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl ConfigManager for StaticConfigManager {
    fn current(&self) -> Arc<Config> {
        Arc::clone(&self.config)
    }

    fn revision(&self) -> String {
        String::new()
    }

    fn snapshot(&self) -> Result<ConfigurationSnapshot, ConfigError> {
        Err(ConfigError::Document {
            message: "configuration management is not available without a configuration file"
                .to_owned(),
        })
    }

    fn preview(
        &self,
        _change: &ConfigChange,
        _expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError> {
        Err(ConfigError::Document {
            message: "configuration management is not available without a configuration file"
                .to_owned(),
        })
    }

    fn preview_batch(
        &self,
        _changes: &[ConfigChange],
        _expected_revision: &str,
    ) -> Result<ConfigurationPreview, ConfigError> {
        Err(ConfigError::Document {
            message: "configuration management is not available without a configuration file"
                .to_owned(),
        })
    }

    fn commit(
        &self,
        _change: &ConfigChange,
        _expected_revision: &str,
    ) -> Result<Config, ConfigError> {
        Err(ConfigError::Document {
            message: "configuration management is not available without a configuration file"
                .to_owned(),
        })
    }

    fn commit_batch(
        &self,
        _changes: &[ConfigChange],
        _expected_revision: &str,
    ) -> Result<Config, ConfigError> {
        Err(ConfigError::Document {
            message: "configuration management is not available without a configuration file"
                .to_owned(),
        })
    }

    fn reload(&self) -> Result<Arc<Config>, ConfigError> {
        Ok(Arc::clone(&self.config))
    }

    fn validate(&self) -> Result<(), ConfigError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_is_stable_and_changes_with_content() {
        assert_eq!(revision_of("a"), revision_of("a"));
        assert_ne!(revision_of("a"), revision_of("b"));
    }

    #[test]
    fn redacts_secret_environment_values() {
        let entity = RawService {
            id: "svc".to_owned(),
            name: None,
            service_type: "backend".to_owned(),
            workspace: "ws".to_owned(),
            path: None,
            default_branch: None,
            repository: None,
            runtime: None,
            runtimes: Vec::new(),
            commands: Default::default(),
            build_system: None,
            build_operation: None,
            liquibase: None,
            health_checks: Vec::new(),
            ports: Vec::new(),
            dependencies: Default::default(),
            environment: [(
                "TOKEN".to_owned(),
                crate::raw::RawEnvValue::Structured(crate::raw::RawEnvVariable {
                    value: "super-secret".to_owned(),
                    secret: true,
                }),
            )]
            .into_iter()
            .collect(),
            capabilities: Vec::new(),
        };
        let fields = entity_fields(&entity);
        assert_eq!(
            fields.get("environment.TOKEN.value").map(String::as_str),
            Some("***")
        );
        assert!(!fields.values().any(|value| value.contains("super-secret")));
    }

    #[test]
    fn entity_kind_tables_match_the_schema() {
        assert_eq!(EntityKind::Workspace.table(), "workspaces");
        assert_eq!(EntityKind::Service.table(), "services");
        assert_eq!(EntityKind::Library.table(), "libraries");
        assert_eq!(EntityKind::Profile.table(), "profiles");
        assert_eq!(EntityKind::Group.table(), "groups");
        assert_eq!(EntityKind::Template.table(), "templates");
        assert_eq!(EntityKind::MigrationPreset.table(), "migration_presets");
        assert_eq!(EntityKind::OperationPreset.table(), "operation_presets");
        assert_eq!(EntityKind::all().len(), 8);
        for kind in EntityKind::all() {
            assert_eq!(kind.label(), kind.as_str());
        }
    }
}
