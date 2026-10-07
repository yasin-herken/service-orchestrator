//! Raw-to-domain normalization and reference validation.
//!
//! This module turns the stringly-typed raw model into validated domain values.
//! It is the only place that knows how a configuration file maps onto the
//! domain aggregate, and the only place that performs reference resolution,
//! identifier uniqueness, and path normalization.
//!
//! Errors are accumulated rather than short-circuited so a developer can fix
//! several problems in one pass. Deep reference checks only run once every
//! entity has been parsed successfully, to avoid cascades of "unknown
//! reference" errors caused by an earlier syntax problem.

use std::collections::{BTreeMap, BTreeSet};

use service_orchestrator_domain::{
    AbsolutePath, DependencyKind, DependencyTarget, Environment, GroupId, Library, LibraryId,
    Profile, ProfileId, RelativePath, Service, ServiceGroup, ServiceId, ServiceType, Workspace,
    WorkspaceId,
};

use crate::error::{ConfigError, ValidationError, ValidationErrors};
use crate::model::{
    Config, ConfiguredWorkspace, ExecutionSettings, LiquibaseConfig, DEFAULT_MAX_PARALLEL_TASKS,
    DEFAULT_TIMEOUT_SECONDS,
};
use crate::raw::{RawConfig, RawLibrary, RawService, RawWorkspace};
use crate::validation::{
    parse_artifact, parse_capability, parse_commands, parse_dependency, parse_environment,
    parse_health_check, parse_id, parse_port, parse_repository, parse_runtime, parse_service_type,
    validate_library_consistency, validate_service_consistency,
};

/// A `(workspace id, entity id)` pair used for within-workspace lookups.
type EntityKey = (String, String);

/// Maps an entity id to the set of workspaces that declare it.
type WorkspaceSets = BTreeMap<String, BTreeSet<String>>;

/// A workspace under construction.
struct Build {
    id: WorkspaceId,
    name: String,
    root: AbsolutePath,
    environment: Environment,
    services: Vec<(String, Service)>,
    libraries: Vec<(String, Library)>,
    groups: Vec<(String, ServiceGroup)>,
    profiles: Vec<(String, Profile)>,
    liquibase: BTreeMap<ServiceId, LiquibaseConfig>,
}

/// Normalizes a parsed configuration file into a validated [`Config`].
///
/// # Errors
///
/// Returns [`ConfigError::Validation`] containing every problem found.
pub(crate) fn normalize(raw: RawConfig, origin: &str) -> Result<Config, ConfigError> {
    let mut errors: Vec<ValidationError> = Vec::new();

    let (mut builds, workspace_index, roots) = build_workspaces(&raw, &mut errors);
    let (service_set, service_workspaces) =
        build_services_and_libraries(&raw, &workspace_index, &roots, &mut builds, &mut errors);

    if !errors.is_empty() {
        return Err(validation_error(errors, origin));
    }

    let library_set = collect_library_set(&builds);
    let group_workspaces = build_groups(
        &raw,
        &workspace_index,
        &service_set,
        &service_workspaces,
        &mut builds,
        &mut errors,
    );
    build_profiles(
        &raw,
        &workspace_index,
        &service_set,
        &group_workspaces,
        &service_workspaces,
        &mut builds,
        &mut errors,
    );

    validate_dependency_references(&builds, &service_set, &library_set, &mut errors);
    let execution = build_execution(&raw, &mut errors);

    if !errors.is_empty() {
        return Err(validation_error(errors, origin));
    }

    let mut workspaces = Vec::with_capacity(builds.len());
    for build in builds {
        if let Some(workspace) = finalize_workspace(build, &mut errors) {
            workspaces.push(workspace);
        }
    }

    if !errors.is_empty() {
        return Err(validation_error(errors, origin));
    }

    Ok(Config {
        version: raw.config_version,
        execution,
        workspaces,
    })
}

fn validation_error(errors: Vec<ValidationError>, origin: &str) -> ConfigError {
    ConfigError::Validation(ValidationErrors::new(errors).with_file(origin))
}

// ---------------------------------------------------------------------------
// Workspaces
// ---------------------------------------------------------------------------

fn build_workspaces(
    raw: &RawConfig,
    errors: &mut Vec<ValidationError>,
) -> (
    Vec<Build>,
    BTreeMap<String, usize>,
    BTreeMap<String, AbsolutePath>,
) {
    let mut builds = Vec::new();
    let mut index = BTreeMap::new();
    let mut roots = BTreeMap::new();
    let mut seen = BTreeSet::new();

    for (i, workspace) in raw.workspaces.iter().enumerate() {
        let location = format!("workspaces[{i}]");
        if !seen.insert(workspace.id.trim().to_owned()) {
            errors.push(
                ValidationError::new(
                    format!("{location}.id"),
                    format!("duplicate workspace id '{}'", workspace.id),
                )
                .suggested_fix("Give each [[workspaces]] entry a unique id.".to_owned()),
            );
            continue;
        }

        let Some(id) = parse_id::<WorkspaceId>(&format!("{location}.id"), &workspace.id, errors)
        else {
            continue;
        };
        let Some(root) = resolve_workspace_root(&format!("{location}.root"), workspace, errors)
        else {
            continue;
        };
        let (environment, mut env_errors) =
            parse_environment(&format!("{location}.environment"), &workspace.environment);
        errors.append(&mut env_errors);

        let name = match &workspace.name {
            Some(name) if name.trim().is_empty() => {
                errors.push(
                    ValidationError::new(
                        format!("{location}.name"),
                        "workspace name must not be empty",
                    )
                    .suggested_fix("Remove the field or set a non-empty name.".to_owned()),
                );
                continue;
            }
            Some(name) => name.clone(),
            None => workspace.id.clone(),
        };

        roots.insert(workspace.id.trim().to_owned(), root.clone());
        index.insert(workspace.id.trim().to_owned(), builds.len());
        builds.push(Build {
            id,
            name,
            root,
            environment,
            services: Vec::new(),
            libraries: Vec::new(),
            groups: Vec::new(),
            profiles: Vec::new(),
            liquibase: BTreeMap::new(),
        });
    }

    (builds, index, roots)
}

fn resolve_workspace_root(
    location: &str,
    workspace: &RawWorkspace,
    errors: &mut Vec<ValidationError>,
) -> Option<AbsolutePath> {
    let expanded = match expand_home(&workspace.root) {
        Ok(expanded) => expanded,
        Err(error) => {
            errors.push(ValidationError::new(location, error).suggested_fix(
                "Set the HOME environment variable or use an absolute path.".to_owned(),
            ));
            return None;
        }
    };
    match AbsolutePath::new(expanded) {
        Ok(path) => Some(path),
        Err(error) => {
            errors.push(
                ValidationError::new(location, error.to_string())
                    .expected("an absolute path such as ~/Projects/project-x")
                    .suggested_fix(
                        "Workspace roots must be absolute; '~' expands using HOME.".to_owned(),
                    ),
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Services and libraries
// ---------------------------------------------------------------------------

/// Builds services and libraries, returning lookup sets used for reference
/// validation.
fn build_services_and_libraries(
    raw: &RawConfig,
    workspace_index: &BTreeMap<String, usize>,
    roots: &BTreeMap<String, AbsolutePath>,
    builds: &mut [Build],
    errors: &mut Vec<ValidationError>,
) -> (BTreeSet<EntityKey>, WorkspaceSets) {
    let mut service_set = BTreeSet::new();
    let mut service_workspaces: WorkspaceSets = BTreeMap::new();

    for (i, raw_service) in raw.services.iter().enumerate() {
        let location = format!("services[{i}]");
        let workspace_id = raw_service.workspace.trim().to_owned();
        let Some(index) = resolve_workspace(&location, &workspace_id, workspace_index, errors)
        else {
            continue;
        };
        let service_id = raw_service.id.trim().to_owned();
        if !service_set.insert((workspace_id.clone(), service_id.clone())) {
            errors.push(
                ValidationError::new(
                    format!("{location}.id"),
                    format!("duplicate service id '{}'", raw_service.id),
                )
                .suggested_fix("Service ids must be unique within a workspace.".to_owned()),
            );
            continue;
        }
        service_workspaces
            .entry(service_id)
            .or_default()
            .insert(workspace_id.clone());

        let root = roots.get(&workspace_id).expect("workspace root exists");
        let Some(service) = build_service(&location, raw_service, root, errors) else {
            continue;
        };
        if let Some(config) = build_liquibase(&location, raw_service, errors) {
            builds[index].liquibase.insert(service.id.clone(), config);
        }
        builds[index].services.push((location, service));
    }

    for (i, raw_library) in raw.libraries.iter().enumerate() {
        let location = format!("libraries[{i}]");
        let workspace_id = raw_library.workspace.trim().to_owned();
        let Some(index) = resolve_workspace(&location, &workspace_id, workspace_index, errors)
        else {
            continue;
        };
        let library_id = raw_library.id.trim().to_owned();
        let duplicate = builds[index]
            .libraries
            .iter()
            .any(|(_, library)| library.id.as_str() == library_id);
        if duplicate {
            errors.push(
                ValidationError::new(
                    format!("{location}.id"),
                    format!("duplicate library id '{}'", raw_library.id),
                )
                .suggested_fix("Library ids must be unique within a workspace.".to_owned()),
            );
            continue;
        }
        let root = roots.get(&workspace_id).expect("workspace root exists");
        if let Some(library) = build_library(&location, raw_library, root, errors) {
            builds[index].libraries.push((location, library));
        }
    }

    (service_set, service_workspaces)
}

fn collect_library_set(builds: &[Build]) -> BTreeSet<EntityKey> {
    let mut set = BTreeSet::new();
    for build in builds {
        for (_, library) in &build.libraries {
            set.insert((build.id.to_string(), library.id.to_string()));
        }
    }
    set
}

fn resolve_workspace(
    location: &str,
    workspace: &str,
    workspace_index: &BTreeMap<String, usize>,
    errors: &mut Vec<ValidationError>,
) -> Option<usize> {
    if let Some(index) = workspace_index.get(workspace) {
        Some(*index)
    } else {
        errors.push(
            ValidationError::new(
                format!("{location}.workspace"),
                format!("unknown workspace '{workspace}'"),
            )
            .suggested_fix(format!(
                "Declare [[workspaces]] with id = \"{workspace}\" or point at an existing one."
            )),
        );
        None
    }
}

fn build_service(
    location: &str,
    raw: &RawService,
    workspace_root: &AbsolutePath,
    errors: &mut Vec<ValidationError>,
) -> Option<Service> {
    let id = parse_id::<ServiceId>(&format!("{location}.id"), &raw.id, errors)?;
    let kind = parse_service_type(&format!("{location}.type"), &raw.service_type, errors)?;
    if kind == ServiceType::Library {
        errors.push(
            ValidationError::new(
                format!("{location}.type"),
                format!("service '{}' is typed as a library", raw.id),
            )
            .expected("a service type such as 'backend' or 'frontend'")
            .suggested_fix(
                "Declare shared artifacts under [[libraries]] instead of [[services]].".to_owned(),
            ),
        );
        return None;
    }

    let name = resolve_name(location, &raw.name, &raw.id, errors)?;
    let mut service = Service::new(id, name, kind).ok()?;

    let path = match &raw.path {
        Some(path) => Some(resolve_entity_path(
            &format!("{location}.path"),
            path,
            workspace_root,
            errors,
        )?),
        None => None,
    };
    if let Some(path) = path.clone() {
        service = service.with_path(path);
    }

    if let Some(repository) = &raw.repository {
        let Some(local_path) = path.clone() else {
            errors.push(
                ValidationError::new(
                    format!("{location}.path"),
                    "a service with a repository must declare a path",
                )
                .suggested_fix(
                    "Add `path = \"backend/auth-service\"` so the checkout location is known."
                        .to_owned(),
                ),
            );
            return None;
        };
        if let Some(repository) = parse_repository(
            &format!("{location}.repository"),
            repository,
            &local_path,
            raw.default_branch.as_deref(),
            errors,
        ) {
            service = service.with_repository(repository);
        }
    }

    if let Some(runtime) = &raw.runtime {
        if let Some(runtime) = parse_runtime(&format!("{location}.runtime"), runtime, errors) {
            service = service.with_runtime(runtime);
        }
    }
    for (j, runtime) in raw.runtimes.iter().enumerate() {
        if let Some(runtime) = parse_runtime(&format!("{location}.runtimes[{j}]"), runtime, errors)
        {
            service = service.with_runtime(runtime);
        }
    }

    service = service.with_commands(parse_commands(
        &format!("{location}.commands"),
        &raw.commands,
        errors,
    ));

    for (j, health_check) in raw.health_checks.iter().enumerate() {
        if let Some(health_check) = parse_health_check(
            &format!("{location}.health_checks[{j}]"),
            health_check,
            errors,
        ) {
            service = service.with_health_check(health_check);
        }
    }

    for (j, port) in raw.ports.iter().enumerate() {
        if let Some(port) = parse_port(&format!("{location}.ports[{j}]"), port, errors) {
            service = service.with_port(port);
        }
    }

    let (environment, mut env_errors) =
        parse_environment(&format!("{location}.environment"), &raw.environment);
    errors.append(&mut env_errors);
    service = service.with_environment(environment);

    for (j, capability) in raw.capabilities.iter().enumerate() {
        if let Some(capability) =
            parse_capability(&format!("{location}.capabilities[{j}]"), capability, errors)
        {
            service = service.with_capability(capability);
        }
    }

    for (j, dependency) in raw.dependencies.services.iter().enumerate() {
        if let Some(dependency) = parse_dependency(
            &format!("{location}.dependencies.services[{j}]"),
            dependency,
            DependencyKind::Runtime,
            false,
            errors,
        ) {
            service = service.with_dependency(dependency);
        }
    }
    for (j, dependency) in raw.dependencies.libraries.iter().enumerate() {
        if let Some(dependency) = parse_dependency(
            &format!("{location}.dependencies.libraries[{j}]"),
            dependency,
            DependencyKind::Build,
            true,
            errors,
        ) {
            service = service.with_dependency(dependency);
        }
    }

    validate_service_consistency(location, &service, errors);
    Some(service)
}

fn build_library(
    location: &str,
    raw: &RawLibrary,
    workspace_root: &AbsolutePath,
    errors: &mut Vec<ValidationError>,
) -> Option<Library> {
    let id = parse_id::<LibraryId>(&format!("{location}.id"), &raw.id, errors)?;
    let name = resolve_name(location, &raw.name, &raw.id, errors)?;
    let mut library = Library::new(id, name).ok()?;

    let library_type = raw
        .library_type
        .clone()
        .unwrap_or_else(|| "maven".to_owned());
    if library_type.trim().is_empty() {
        errors.push(ValidationError::new(
            format!("{location}.type"),
            "library type must not be empty when specified",
        ));
        return None;
    }

    let path = match &raw.path {
        Some(path) => Some(resolve_entity_path(
            &format!("{location}.path"),
            path,
            workspace_root,
            errors,
        )?),
        None => None,
    };
    if let Some(path) = path.clone() {
        library = library.with_path(path);
    }

    if let Some(repository) = &raw.repository {
        let Some(local_path) = path.clone() else {
            errors.push(
                ValidationError::new(
                    format!("{location}.path"),
                    "a library with a repository must declare a path",
                )
                .suggested_fix("Add `path = \"libraries/common-core\"`.".to_owned()),
            );
            return None;
        };
        if let Some(repository) = parse_repository(
            &format!("{location}.repository"),
            repository,
            &local_path,
            raw.default_branch.as_deref(),
            errors,
        ) {
            library = library.with_repository(repository);
        }
    }

    if let Some(artifact) = &raw.artifact {
        if let Some(artifact) = parse_artifact(&format!("{location}.artifact"), artifact, errors) {
            library = library.with_artifact(artifact);
        }
    }

    if let Some(runtime) = &raw.runtime {
        if let Some(runtime) = parse_runtime(&format!("{location}.runtime"), runtime, errors) {
            library = library.with_runtime(runtime);
        }
    }
    for (j, runtime) in raw.runtimes.iter().enumerate() {
        if let Some(runtime) = parse_runtime(&format!("{location}.runtimes[{j}]"), runtime, errors)
        {
            library = library.with_runtime(runtime);
        }
    }

    library = library.with_commands(parse_commands(
        &format!("{location}.commands"),
        &raw.commands,
        errors,
    ));

    if !raw.dependencies.services.is_empty() {
        errors.push(
            ValidationError::new(
                format!("{location}.dependencies.services"),
                format!("library '{}' cannot depend on services", raw.id),
            )
            .expected("library-to-library dependencies only")
            .suggested_fix("Move the dependency under `dependencies.libraries`.".to_owned()),
        );
    }
    for (j, dependency) in raw.dependencies.libraries.iter().enumerate() {
        if let Some(dependency) = parse_dependency(
            &format!("{location}.dependencies.libraries[{j}]"),
            dependency,
            DependencyKind::Build,
            true,
            errors,
        ) {
            library = library.with_dependency(dependency);
        }
    }

    let (environment, mut env_errors) =
        parse_environment(&format!("{location}.environment"), &raw.environment);
    errors.append(&mut env_errors);
    library = library.with_environment(environment);

    validate_library_consistency(location, &library_type, &library, errors);
    Some(library)
}

fn build_liquibase(
    location: &str,
    raw: &RawService,
    errors: &mut Vec<ValidationError>,
) -> Option<LiquibaseConfig> {
    let raw_liquibase = raw.liquibase.as_ref()?;
    let location = format!("{location}.liquibase");
    let command = raw_liquibase
        .command
        .as_ref()
        .and_then(|command| crate::validation::parse_command(&location, command, errors));
    let (environment, mut env_errors) = parse_environment(
        &format!("{location}.environment"),
        &raw_liquibase.environment,
    );
    errors.append(&mut env_errors);
    Some(LiquibaseConfig {
        enabled: raw_liquibase.enabled.unwrap_or(true),
        command,
        timeout_seconds: raw_liquibase.timeout_seconds,
        environment,
    })
}

fn resolve_name(
    location: &str,
    name: &Option<String>,
    fallback: &str,
    errors: &mut Vec<ValidationError>,
) -> Option<String> {
    match name {
        Some(name) if name.trim().is_empty() => {
            errors.push(
                ValidationError::new(format!("{location}.name"), "display name must not be empty")
                    .suggested_fix("Remove the field to default to the id.".to_owned()),
            );
            None
        }
        Some(name) => Some(name.clone()),
        None => Some(fallback.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Groups and profiles
// ---------------------------------------------------------------------------

fn build_groups(
    raw: &RawConfig,
    workspace_index: &BTreeMap<String, usize>,
    service_set: &BTreeSet<EntityKey>,
    service_workspaces: &WorkspaceSets,
    builds: &mut [Build],
    errors: &mut Vec<ValidationError>,
) -> WorkspaceSets {
    let mut group_workspaces: WorkspaceSets = BTreeMap::new();
    let mut seen: BTreeSet<EntityKey> = BTreeSet::new();

    for (i, group) in raw.groups.iter().enumerate() {
        let location = format!("groups[{i}]");
        let Some(id) = parse_id::<GroupId>(&format!("{location}.id"), &group.id, errors) else {
            continue;
        };

        if group.services.is_empty() {
            errors.push(
                ValidationError::new(
                    format!("{location}.services"),
                    format!("group '{}' must contain at least one service", group.id),
                )
                .suggested_fix("List one or more service ids.".to_owned()),
            );
            continue;
        }

        let candidates = service_candidates(group.services.iter(), service_workspaces);
        let Some(workspace) = choose_workspace(
            &location,
            group.workspace.as_deref(),
            candidates,
            workspace_index,
            errors,
        ) else {
            continue;
        };

        if !seen.insert((workspace.clone(), group.id.trim().to_owned())) {
            errors.push(
                ValidationError::new(
                    format!("{location}.id"),
                    format!("duplicate group id '{}'", group.id),
                )
                .suggested_fix("Group ids must be unique within a workspace.".to_owned()),
            );
            continue;
        }

        let Some(name) = resolve_name(&location, &group.name, &group.id, errors) else {
            continue;
        };
        let mut service_group = match ServiceGroup::new(id, name) {
            Ok(service_group) => service_group,
            Err(error) => {
                errors.push(ValidationError::new(&location, error.to_string()));
                continue;
            }
        };
        if let Some(description) = &group.description {
            service_group = service_group.with_description(description.clone());
        }

        for (j, service_id) in group.services.iter().enumerate() {
            let Some(service_id) =
                parse_id::<ServiceId>(&format!("{location}.services[{j}]"), service_id, errors)
            else {
                continue;
            };
            if !service_set.contains(&(workspace.clone(), service_id.to_string())) {
                errors.push(
                    ValidationError::new(
                        format!("{location}.services[{j}]"),
                        format!(
                            "group '{}' references unknown service '{service_id}' in workspace '{workspace}'",
                            group.id
                        ),
                    )
                    .suggested_fix("Reference a declared service in the same workspace.".to_owned()),
                );
                continue;
            }
            service_group = service_group.with_service(service_id);
        }

        let index = workspace_index.get(&workspace).expect("workspace exists");
        builds[*index].groups.push((location, service_group));
        group_workspaces
            .entry(group.id.trim().to_owned())
            .or_default()
            .insert(workspace);
    }

    group_workspaces
}

fn build_profiles(
    raw: &RawConfig,
    workspace_index: &BTreeMap<String, usize>,
    service_set: &BTreeSet<EntityKey>,
    group_workspaces: &WorkspaceSets,
    service_workspaces: &WorkspaceSets,
    builds: &mut [Build],
    errors: &mut Vec<ValidationError>,
) {
    let mut seen: BTreeSet<EntityKey> = BTreeSet::new();

    for (i, profile) in raw.profiles.iter().enumerate() {
        let location = format!("profiles[{i}]");
        let Some(id) = parse_id::<ProfileId>(&format!("{location}.id"), &profile.id, errors) else {
            continue;
        };

        if profile.services.is_empty() && profile.groups.is_empty() {
            errors.push(
                ValidationError::new(
                    format!("{location}.services"),
                    format!(
                        "profile '{}' must reference at least one service or group",
                        profile.id
                    ),
                )
                .suggested_fix("Add a service or a group reference.".to_owned()),
            );
            continue;
        }

        let mut candidates = service_candidates(profile.services.iter(), service_workspaces);
        for group_id in &profile.groups {
            if let Some(workspaces) = group_workspaces.get(group_id.trim()) {
                candidates.extend(workspaces.iter().cloned());
            }
        }

        let Some(workspace) = choose_workspace(
            &location,
            profile.workspace.as_deref(),
            candidates,
            workspace_index,
            errors,
        ) else {
            continue;
        };

        if !seen.insert((workspace.clone(), profile.id.trim().to_owned())) {
            errors.push(
                ValidationError::new(
                    format!("{location}.id"),
                    format!("duplicate profile id '{}'", profile.id),
                )
                .suggested_fix("Profile ids must be unique within a workspace.".to_owned()),
            );
            continue;
        }

        let Some(name) = resolve_name(&location, &profile.name, &profile.id, errors) else {
            continue;
        };
        let mut developer_profile = match Profile::new(id, name) {
            Ok(developer_profile) => developer_profile,
            Err(error) => {
                errors.push(ValidationError::new(&location, error.to_string()));
                continue;
            }
        };
        if let Some(description) = &profile.description {
            developer_profile = developer_profile.with_description(description.clone());
        }

        for (j, service_id) in profile.services.iter().enumerate() {
            let Some(service_id) =
                parse_id::<ServiceId>(&format!("{location}.services[{j}]"), service_id, errors)
            else {
                continue;
            };
            if !service_set.contains(&(workspace.clone(), service_id.to_string())) {
                errors.push(
                    ValidationError::new(
                        format!("{location}.services[{j}]"),
                        format!(
                            "profile '{}' references unknown service '{service_id}' in workspace '{workspace}'",
                            profile.id
                        ),
                    )
                    .suggested_fix("Reference a declared service in the same workspace.".to_owned()),
                );
                continue;
            }
            developer_profile = developer_profile.with_service(service_id);
        }

        for (j, group_id) in profile.groups.iter().enumerate() {
            let Some(group_id) =
                parse_id::<GroupId>(&format!("{location}.groups[{j}]"), group_id, errors)
            else {
                continue;
            };
            if !group_workspaces
                .get(group_id.as_str())
                .is_some_and(|workspaces| workspaces.contains(&workspace))
            {
                errors.push(
                    ValidationError::new(
                        format!("{location}.groups[{j}]"),
                        format!(
                            "profile '{}' references unknown group '{group_id}' in workspace '{workspace}'",
                            profile.id
                        ),
                    )
                    .suggested_fix("Reference a declared group in the same workspace.".to_owned()),
                );
                continue;
            }
            developer_profile = developer_profile.with_group(group_id);
        }

        let index = workspace_index.get(&workspace).expect("workspace exists");
        builds[*index].profiles.push((location, developer_profile));
    }
}

/// Collects the workspaces that declared services belong to.
fn service_candidates<'a>(
    services: impl Iterator<Item = &'a String>,
    service_workspaces: &WorkspaceSets,
) -> BTreeSet<String> {
    let mut candidates = BTreeSet::new();
    for service in services {
        if let Some(workspaces) = service_workspaces.get(service.trim()) {
            candidates.extend(workspaces.iter().cloned());
        }
    }
    candidates
}

/// Chooses the workspace for a group or profile.
///
/// An explicit `workspace` is honored and must exist. Otherwise the candidates
/// inferred from members must contain exactly one workspace.
fn choose_workspace(
    location: &str,
    explicit: Option<&str>,
    candidates: BTreeSet<String>,
    workspace_index: &BTreeMap<String, usize>,
    errors: &mut Vec<ValidationError>,
) -> Option<String> {
    if let Some(workspace) = explicit {
        if workspace_index.contains_key(workspace) {
            return Some(workspace.to_owned());
        }
        errors.push(
            ValidationError::new(
                format!("{location}.workspace"),
                format!("unknown workspace '{workspace}'"),
            )
            .suggested_fix(format!(
                "Declare [[workspaces]] with id = \"{workspace}\" or point at an existing one."
            )),
        );
        return None;
    }

    match candidates.len() {
        1 => candidates.into_iter().next(),
        0 => {
            errors.push(
                ValidationError::new(
                    format!("{location}.workspace"),
                    "cannot determine the owning workspace from the referenced entities",
                )
                .suggested_fix(
                    "Set `workspace = \"<id>\"` explicitly or reference declared entities."
                        .to_owned(),
                ),
            );
            None
        }
        _ => {
            errors.push(
                ValidationError::new(
                    format!("{location}.workspace"),
                    "references span multiple workspaces",
                )
                .suggested_fix(
                    "Set `workspace = \"<id>\"` explicitly or keep members within one workspace."
                        .to_owned(),
                ),
            );
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Dependencies and execution
// ---------------------------------------------------------------------------

fn validate_dependency_references(
    builds: &[Build],
    service_set: &BTreeSet<EntityKey>,
    library_set: &BTreeSet<EntityKey>,
    errors: &mut Vec<ValidationError>,
) {
    for build in builds {
        let workspace = build.id.to_string();
        for (location, service) in &build.services {
            for dependency in &service.dependencies {
                match &dependency.target {
                    DependencyTarget::Service(target) => {
                        if !service_set.contains(&(workspace.clone(), target.to_string())) {
                            errors.push(unknown_dependency(
                                &format!("{location}.dependencies"),
                                service.id.as_str(),
                                "service",
                                target.as_str(),
                                &workspace,
                            ));
                        }
                    }
                    DependencyTarget::Library(target) => {
                        if !library_set.contains(&(workspace.clone(), target.to_string())) {
                            errors.push(unknown_dependency(
                                &format!("{location}.dependencies"),
                                service.id.as_str(),
                                "library",
                                target.as_str(),
                                &workspace,
                            ));
                        }
                    }
                }
            }
        }
        for (location, library) in &build.libraries {
            for dependency in &library.dependencies {
                if let DependencyTarget::Library(target) = &dependency.target {
                    if !library_set.contains(&(workspace.clone(), target.to_string())) {
                        errors.push(unknown_dependency(
                            &format!("{location}.dependencies"),
                            library.id.as_str(),
                            "library",
                            target.as_str(),
                            &workspace,
                        ));
                    }
                }
            }
        }
    }
}

fn unknown_dependency(
    location: &str,
    owner: &str,
    kind: &str,
    target: &str,
    workspace: &str,
) -> ValidationError {
    ValidationError::new(
        location,
        format!("'{owner}' depends on unknown {kind} '{target}' in workspace '{workspace}'"),
    )
    .suggested_fix(format!(
        "Declare {kind} '{target}' or remove the dependency."
    ))
}

fn build_execution(raw: &RawConfig, errors: &mut Vec<ValidationError>) -> ExecutionSettings {
    let max_parallel_tasks = raw
        .execution
        .max_parallel_tasks
        .unwrap_or(DEFAULT_MAX_PARALLEL_TASKS);
    if max_parallel_tasks == 0 {
        errors.push(
            ValidationError::new(
                "execution.max_parallel_tasks",
                "max_parallel_tasks must be at least 1",
            )
            .expected("an integer of 1 or greater")
            .suggested_fix(format!(
                "Set `max_parallel_tasks = {DEFAULT_MAX_PARALLEL_TASKS}`."
            )),
        );
    }
    let default_timeout_seconds = raw
        .execution
        .default_timeout_seconds
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    if default_timeout_seconds == 0 {
        errors.push(
            ValidationError::new(
                "execution.default_timeout_seconds",
                "default_timeout_seconds must be at least 1",
            )
            .expected("an integer of 1 or greater")
            .suggested_fix(format!(
                "Set `default_timeout_seconds = {DEFAULT_TIMEOUT_SECONDS}`."
            )),
        );
    }
    ExecutionSettings {
        max_parallel_tasks,
        default_timeout_seconds,
    }
}

// ---------------------------------------------------------------------------
// Finalization
// ---------------------------------------------------------------------------

fn finalize_workspace(
    build: Build,
    errors: &mut Vec<ValidationError>,
) -> Option<ConfiguredWorkspace> {
    let mut workspace = match Workspace::new(build.id.clone(), build.name, build.root) {
        Ok(workspace) => workspace,
        Err(error) => {
            errors.push(ValidationError::new(
                format!("workspace '{}'", build.id),
                error.to_string(),
            ));
            return None;
        }
    };

    for (_, service) in build.services {
        workspace = workspace.with_service(service);
    }
    for (_, library) in build.libraries {
        workspace = workspace.with_library(library);
    }
    for (_, group) in build.groups {
        workspace = workspace.with_group(group);
    }
    for (_, profile) in build.profiles {
        workspace = workspace.with_profile(profile);
    }

    if let Err(error) = workspace.validate() {
        errors.push(
            ValidationError::new(
                format!("workspace '{}'", workspace.id),
                error.to_string(),
            )
            .suggested_fix(
                "Fix the reported entity; dependency graphs must be acyclic and every reference must resolve."
                    .to_owned(),
            ),
        );
        return None;
    }

    Some(ConfiguredWorkspace {
        workspace,
        environment: build.environment,
        liquibase: build.liquibase,
    })
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Expands a leading `~` using `HOME`.
fn expand_home(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed == "~" {
        let home = std::env::var("HOME")
            .map_err(|_| "cannot expand '~' because HOME is not set".to_owned())?;
        return Ok(home);
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .map_err(|_| "cannot expand '~' because HOME is not set".to_owned())?;
        return Ok(format!("{}/{}", home.trim_end_matches('/'), rest));
    }
    Ok(trimmed.to_owned())
}

/// Resolves an entity path against a workspace root.
///
/// Relative paths are kept as-is. Absolute paths (including `~`-relative ones)
/// must live inside the workspace; they are rewritten as workspace-relative
/// paths. No filesystem access is performed.
fn resolve_entity_path(
    location: &str,
    value: &str,
    workspace_root: &AbsolutePath,
    errors: &mut Vec<ValidationError>,
) -> Option<RelativePath> {
    let expanded = match expand_home(value) {
        Ok(expanded) => expanded,
        Err(error) => {
            errors.push(ValidationError::new(location, error));
            return None;
        }
    };

    let relative = if expanded.starts_with('/') {
        let root = workspace_root.as_str().trim_end_matches('/');
        if expanded == root {
            errors.push(
                ValidationError::new(location, "path must point inside the workspace, not at it")
                    .expected("a path below the workspace root"),
            );
            return None;
        }
        match expanded.strip_prefix(&format!("{root}/")) {
            Some(rest) if !rest.is_empty() => rest.to_owned(),
            Some(_) => {
                errors.push(
                    ValidationError::new(
                        location,
                        "path must point inside the workspace, not at it",
                    )
                    .expected("a path below the workspace root"),
                );
                return None;
            }
            None => {
                errors.push(
                    ValidationError::new(
                        location,
                        format!("'{expanded}' is outside the workspace root '{root}'"),
                    )
                    .expected("a path inside the workspace"),
                );
                return None;
            }
        }
    } else {
        expanded
    };

    match RelativePath::new(relative) {
        Ok(path) => Some(path),
        Err(error) => {
            errors.push(
                ValidationError::new(location, error.to_string())
                    .expected("a workspace-relative path such as 'backend/auth-service'"),
            );
            None
        }
    }
}
