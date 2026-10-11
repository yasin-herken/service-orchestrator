//! Maven artifact inspection.
//!
//! This module implements the application's [`ArtifactInspector`] port for
//! Maven: it locates an artifact in the *effective* local repository and reads a
//! consumer's declared dependency version from its POM. It is the only place in
//! the build adapter that reads Maven-specific files, and it **never resolves**
//! artifacts itself (`tasks/009.md` §7, §12).
//!
//! # Effective local repository
//!
//! Maven does not always use `~/.m2/repository`: the location can be overridden
//! by settings, `MAVEN_OPTS`, or command-line properties. Rather than hard-code
//! a path, [`MavenLocalRepositoryProvider`] asks Maven for the effective value
//! (`help:evaluate -Dexpression=settings.localRepository`) through the same
//! [`BuildRunner`] every other build uses. When the value cannot be determined,
//! inspection fails explicitly ([`ArtifactInspectionError::LocalRepositoryUnknown`])
//! instead of guessing.
//!
//! # Conservative version inspection
//!
//! A Maven dependency version can come from a direct element, a property, a
//! parent POM, dependency management, or an imported BOM (`tasks/009.md` §12).
//! [`MavenArtifactInspector::dependency_version`] resolves only what it can read
//! with certainty — a literal version or a property defined in the same POM — and
//! reports everything else as
//! [`DependencyVersionInspection::Unknown`]. It never rewrites a POM and never
//! runs a text replacement.
//!
//! [`BuildRunner`]: crate::runner::BuildRunner

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use service_orchestrator_application::{
    ArtifactInspectionError, ArtifactInspector, ArtifactPresence, DependencyVersionInspection,
    ResolvedArtifact, ResolvedDependency,
};
use service_orchestrator_domain::{AbsolutePath, BuildOperation, BuildSystem, Version};
use service_orchestrator_process::ProcessManager;

use crate::plan::BuildPlan;
use crate::runner::BuildRunner;

/// The maximum time an artifact inspection waits for Maven.
const MAVEN_TIMEOUT: Duration = Duration::from_secs(120);

/// Resolves the effective local repository for a module.
///
/// A provider is a seam: production uses [`MavenLocalRepositoryProvider`], while
/// tests and configuration may inject a [`FixedLocalRepository`]. The application
/// never sees this type — it reaches the inspector through the
/// [`ArtifactInspector`] port.
#[async_trait]
pub trait LocalRepositoryProvider: Send + Sync {
    /// Returns the effective local repository for a module directory.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactInspectionError::LocalRepositoryUnknown`] when the
    /// location cannot be determined.
    async fn local_repository(
        &self,
        working_dir: &AbsolutePath,
    ) -> Result<AbsolutePath, ArtifactInspectionError>;
}

/// A [`LocalRepositoryProvider`] that always returns a fixed path.
///
/// This is used by tests and by callers that already know the local repository
/// (for example from an explicit configuration value).
#[derive(Clone, Debug)]
pub struct FixedLocalRepository {
    root: AbsolutePath,
}

impl FixedLocalRepository {
    /// Creates a provider over an explicit repository root.
    #[must_use]
    pub fn new(root: AbsolutePath) -> Self {
        Self { root }
    }
}

#[async_trait]
impl LocalRepositoryProvider for FixedLocalRepository {
    async fn local_repository(
        &self,
        _working_dir: &AbsolutePath,
    ) -> Result<AbsolutePath, ArtifactInspectionError> {
        Ok(self.root.clone())
    }
}

/// Asks Maven for the effective local repository, caching the answer per module.
///
/// The value is a property of the developer's Maven configuration, not of the
/// module, but Maven resolves it relative to the working directory, so the cache
/// is keyed by directory. A failure is never cached: a transient problem must not
/// permanently disable inspection.
///
/// By default Maven reads the developer's own settings, which is how the
/// *effective* value is discovered. [`MavenLocalRepositoryProvider::with_settings`]
/// pins a specific settings file instead, which hermetic tests and CI systems
/// with a non-default settings layout use.
pub struct MavenLocalRepositoryProvider {
    runner: BuildRunner,
    cache: std::sync::Mutex<BTreeMap<String, AbsolutePath>>,
    settings: Option<AbsolutePath>,
}

impl MavenLocalRepositoryProvider {
    /// Creates a provider that evaluates Maven configuration through `processes`.
    #[must_use]
    pub fn new(processes: ProcessManager) -> Self {
        Self {
            runner: BuildRunner::new(processes),
            cache: std::sync::Mutex::new(BTreeMap::new()),
            settings: None,
        }
    }

    /// Pins the Maven settings file used to evaluate the local repository.
    ///
    /// Without this, Maven reads the developer's default settings. Pinning a
    /// settings file keeps the evaluation independent of the ambient Maven
    /// configuration.
    #[must_use]
    pub fn with_settings(mut self, settings: AbsolutePath) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Runs `mvn help:evaluate` and parses the effective repository.
    async fn evaluate(
        &self,
        working_dir: &AbsolutePath,
    ) -> Result<AbsolutePath, ArtifactInspectionError> {
        let mut args = vec!["-B".to_owned(), "-q".to_owned(), "-DforceStdout".to_owned()];
        if let Some(settings) = &self.settings {
            args.push("-s".to_owned());
            args.push(settings.as_str().to_owned());
        }
        args.push("help:evaluate".to_owned());
        args.push("-Dexpression=settings.localRepository".to_owned());

        let plan = BuildPlan::new(
            BuildSystem::Maven,
            BuildOperation::Build,
            "mvn",
            working_dir.clone(),
        )
        .with_args(args);

        let output = self
            .runner
            .run(&plan, Some(MAVEN_TIMEOUT), None)
            .await
            .map_err(|error| ArtifactInspectionError::LocalRepositoryUnknown {
                message: format!("could not run Maven to evaluate the local repository: {error}"),
            })?;

        if !output.succeeded() {
            return Err(ArtifactInspectionError::LocalRepositoryUnknown {
                message: "Maven did not report the effective local repository".to_owned(),
            });
        }

        let value = output
            .stdout
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('['))
            .ok_or_else(|| ArtifactInspectionError::LocalRepositoryUnknown {
                message: "Maven reported an empty local repository".to_owned(),
            })?;

        AbsolutePath::new(value.to_owned()).map_err(|error| {
            ArtifactInspectionError::LocalRepositoryUnknown {
                message: format!("Maven reported an invalid local repository '{value}': {error}"),
            }
        })
    }
}

#[async_trait]
impl LocalRepositoryProvider for MavenLocalRepositoryProvider {
    async fn local_repository(
        &self,
        working_dir: &AbsolutePath,
    ) -> Result<AbsolutePath, ArtifactInspectionError> {
        let key = working_dir.as_str().to_owned();
        if let Some(cached) = self
            .cache
            .lock()
            .expect("the local-repository cache is never poisoned")
            .get(&key)
        {
            return Ok(cached.clone());
        }
        let resolved = self.evaluate(working_dir).await?;
        self.cache
            .lock()
            .expect("the local-repository cache is never poisoned")
            .insert(key, resolved.clone());
        Ok(resolved)
    }
}

impl std::fmt::Debug for MavenLocalRepositoryProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MavenLocalRepositoryProvider")
            .finish_non_exhaustive()
    }
}

/// The Maven [`ArtifactInspector`].
///
/// It locates artifacts using the standard Maven repository layout and reads
/// consumer POMs conservatively. It performs no dependency resolution and
/// rewrites nothing.
pub struct MavenArtifactInspector {
    local_repository: std::sync::Arc<dyn LocalRepositoryProvider>,
}

impl MavenArtifactInspector {
    /// Creates an inspector that evaluates the local repository through Maven.
    #[must_use]
    pub fn new(processes: ProcessManager) -> Self {
        let provider = MavenLocalRepositoryProvider::new(processes);
        Self::with_local_repository(std::sync::Arc::new(provider))
    }

    /// Creates an inspector over an explicit local-repository provider.
    #[must_use]
    pub fn with_local_repository(
        local_repository: std::sync::Arc<dyn LocalRepositoryProvider>,
    ) -> Self {
        Self { local_repository }
    }

    /// Creates an inspector that reads a fixed local repository.
    #[must_use]
    pub fn with_fixed_repository(root: AbsolutePath) -> Self {
        Self::with_local_repository(std::sync::Arc::new(FixedLocalRepository::new(root)))
    }
}

impl std::fmt::Debug for MavenArtifactInspector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MavenArtifactInspector")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ArtifactInspector for MavenArtifactInspector {
    async fn locate(
        &self,
        artifact: &ResolvedArtifact,
    ) -> Result<ArtifactPresence, ArtifactInspectionError> {
        let root = self
            .local_repository
            .local_repository(&artifact.path)
            .await?;
        let full = join_absolute(&root, &artifact.coordinate.repository_path());
        let path =
            AbsolutePath::new(full.clone()).map_err(|error| ArtifactInspectionError::Io {
                path: full.clone(),
                message: error.to_string(),
            })?;
        match std::fs::metadata(full.as_str()) {
            Ok(metadata) if metadata.is_file() => Ok(ArtifactPresence::Found {
                path,
                size_bytes: Some(metadata.len()),
            }),
            _ => Ok(ArtifactPresence::Missing {
                expected_path: Some(path),
            }),
        }
    }

    async fn dependency_version(
        &self,
        dependency: &ResolvedDependency,
    ) -> Result<DependencyVersionInspection, ArtifactInspectionError> {
        let pom = join_absolute(&dependency.path, "pom.xml");
        let xml = std::fs::read_to_string(&pom).map_err(|error| ArtifactInspectionError::Io {
            path: pom.clone(),
            message: error.to_string(),
        })?;
        let document = parse_xml(&xml);
        Ok(inspect_dependency(
            &document,
            &dependency.coordinate.group_id,
            &dependency.coordinate.artifact_id,
        ))
    }
}

/// Joins a repository or module root with a relative path.
fn join_absolute(root: &AbsolutePath, relative: &str) -> String {
    format!(
        "{}/{}",
        root.as_str().trim_end_matches('/'),
        relative.trim_start_matches('/')
    )
}

// -- POM reading --------------------------------------------------------------

/// A minimal XML element tree, sufficient to read a Maven POM.
///
/// This is deliberately not a general XML parser: it builds a name/text/children
/// tree and ignores attributes, namespaces, and entity expansion. Maven POMs are
/// simple, well-formed documents, and a conservative reader that cannot be
/// confused by a malformed POM is safer than a permissive one.
#[derive(Debug, Default)]
struct Element {
    name: String,
    text: String,
    children: Vec<Element>,
}

impl Element {
    fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |child| child.name == name)
    }

    fn text(&self) -> &str {
        self.text.trim()
    }

    fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(Element::text)
            .filter(|value| !value.is_empty())
    }
}

/// Parses `xml` into a synthetic root whose children are the document elements.
fn parse_xml(xml: &str) -> Element {
    let mut root = Element::default();
    let mut stack: Vec<Element> = Vec::new();
    let bytes = xml.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'<' {
            let start = index;
            while index < bytes.len() && bytes[index] != b'<' {
                index += 1;
            }
            let text = &xml[start..index];
            if !text.trim().is_empty() {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(text);
                }
            }
            continue;
        }

        if xml[index..].starts_with("<!--") {
            index += xml[index + 4..].find("-->").map_or(0, |end| end + 7);
            continue;
        }
        if xml[index..].starts_with("<![CDATA[") {
            let rest = &xml[index + 9..];
            if let Some(end) = rest.find("]]>") {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&rest[..end]);
                }
                index += end + 12;
                continue;
            }
            break;
        }
        if xml[index..].starts_with("<?") || xml[index..].starts_with("<!") {
            match xml[index..].find('>') {
                Some(end) => index += end + 1,
                None => break,
            }
            continue;
        }

        let Some(end) = xml[index..].find('>') else {
            break;
        };
        let inner = xml[index + 1..index + end].trim();
        index += end + 1;

        if let Some(name) = inner.strip_prefix('/') {
            let name = name.trim();
            while let Some(element) = stack.pop() {
                let matched = element.name == name;
                attach(&mut stack, &mut root, element);
                if matched {
                    break;
                }
            }
        } else {
            let self_closing = inner.ends_with('/');
            let name = inner
                .trim_end_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned();
            if name.is_empty() {
                continue;
            }
            let element = Element {
                name,
                ..Element::default()
            };
            if self_closing {
                attach(&mut stack, &mut root, element);
            } else {
                stack.push(element);
            }
        }
    }

    while let Some(element) = stack.pop() {
        attach(&mut stack, &mut root, element);
    }
    root
}

/// Attaches a completed element to its parent (or the synthetic root).
fn attach(stack: &mut [Element], root: &mut Element, element: Element) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(element),
        None => root.children.push(element),
    }
}

/// One declared dependency's coordinates and optional version.
struct DeclaredDependency {
    group: String,
    artifact: String,
    version: Option<String>,
    packaging: Option<String>,
    scope: Option<String>,
}

impl DeclaredDependency {
    fn matches(&self, group: &str, artifact: &str) -> bool {
        self.group == group && self.artifact == artifact
    }
}

/// Reads the `<dependency>` entries directly inside a `<dependencies>` element.
fn read_dependencies(dependencies: &Element) -> Vec<DeclaredDependency> {
    dependencies
        .children_named("dependency")
        .map(|dependency| DeclaredDependency {
            group: dependency.child_text("groupId").unwrap_or("").to_owned(),
            artifact: dependency.child_text("artifactId").unwrap_or("").to_owned(),
            version: dependency.child_text("version").map(str::to_owned),
            packaging: dependency.child_text("type").map(str::to_owned),
            scope: dependency.child_text("scope").map(str::to_owned),
        })
        .collect()
}

/// Reads `project/properties` into a name→value map.
fn read_properties(project: &Element) -> BTreeMap<String, String> {
    let mut properties = BTreeMap::new();
    if let Some(element) = project.child("properties") {
        for property in &element.children {
            let value = property.text();
            if !value.is_empty() {
                properties.insert(property.name.clone(), value.to_owned());
            }
        }
    }
    properties
}

/// Inspects `project` for a declared version of `group:artifact`.
fn inspect_dependency(
    document: &Element,
    group: &str,
    artifact: &str,
) -> DependencyVersionInspection {
    let project = document.child("project").unwrap_or(document);
    let properties = read_properties(project);

    let direct = project
        .child("dependencies")
        .map(read_dependencies)
        .unwrap_or_default();
    let managed = project
        .child("dependencyManagement")
        .and_then(|management| management.child("dependencies"))
        .map(read_dependencies)
        .unwrap_or_default();

    if let Some(dependency) = direct.iter().find(|entry| entry.matches(group, artifact)) {
        if let Some(version) = dependency.version.as_deref() {
            return resolve_version(version, &properties, "the declared dependency");
        }
        if let Some(managed) = managed.iter().find(|entry| entry.matches(group, artifact)) {
            if let Some(version) = managed.version.as_deref() {
                return resolve_version(version, &properties, "dependency management");
            }
        }
        return DependencyVersionInspection::Unknown {
            reason: format!(
                "'{group}:{artifact}' is declared without a version; it may be supplied by a parent or imported BOM"
            ),
        };
    }

    if managed.iter().any(|entry| entry.matches(group, artifact)) {
        return DependencyVersionInspection::Unknown {
            reason: format!("'{group}:{artifact}' is managed but not declared as a dependency"),
        };
    }

    if project.child("parent").is_some() || imports_a_bom(&managed) {
        return DependencyVersionInspection::Unknown {
            reason: format!(
                "'{group}:{artifact}' is not declared directly; its version may come from a parent POM or an imported BOM"
            ),
        };
    }

    DependencyVersionInspection::NotDeclared
}

/// Returns `true` when dependency management imports at least one BOM.
fn imports_a_bom(managed: &[DeclaredDependency]) -> bool {
    managed.iter().any(|dependency| {
        dependency.packaging.as_deref() == Some("pom")
            && dependency.scope.as_deref() == Some("import")
    })
}

/// Resolves a version string against the POM's properties.
fn resolve_version(
    version: &str,
    properties: &BTreeMap<String, String>,
    source: &str,
) -> DependencyVersionInspection {
    let version = version.trim();
    if let Some(name) = property_reference(version) {
        return match properties.get(name) {
            Some(value) if !value.contains("${") => match Version::new(value.clone()) {
                Ok(version) => DependencyVersionInspection::Declared { version },
                Err(_) => DependencyVersionInspection::Unknown {
                    reason: format!(
                        "the property '{name}' from {source} is not a valid version: '{value}'"
                    ),
                },
            },
            Some(_) => DependencyVersionInspection::Unknown {
                reason: format!("the property '{name}' from {source} is itself an expression"),
            },
            None => DependencyVersionInspection::Unknown {
                reason: format!(
                    "the version from {source} references '{name}', which is not defined in this POM"
                ),
            },
        };
    }
    if version.contains("${") {
        return DependencyVersionInspection::Unknown {
            reason: format!(
                "the version from {source} is a Maven expression that cannot be resolved statically"
            ),
        };
    }
    match Version::new(version.to_owned()) {
        Ok(version) => DependencyVersionInspection::Declared { version },
        Err(_) => DependencyVersionInspection::Unknown {
            reason: format!("the version from {source} is not a valid Maven version: '{version}'"),
        },
    }
}

/// Returns the property name of a simple `${name}` reference.
fn property_reference(version: &str) -> Option<&str> {
    let inner = version.strip_prefix("${")?.strip_suffix('}')?;
    if inner.is_empty() || inner.contains("${") {
        None
    } else {
        Some(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::{ArtifactCoordinate, LibraryId, WorkspaceId};

    fn write_pom(dir: &std::path::Path, xml: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("pom.xml"), xml).unwrap();
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "so-artifact-{label}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_nested_elements_and_text() {
        let xml = "<project><properties><x>1</x></properties><dependencies>\
                   <dependency><groupId>g</groupId><artifactId>a</artifactId>\
                   <version>2.0</version></dependency></dependencies></project>";
        let document = parse_xml(xml);
        let project = document.child("project").unwrap();
        assert_eq!(
            project.child("properties").unwrap().child_text("x"),
            Some("1")
        );
        let deps = read_dependencies(project.child("dependencies").unwrap());
        assert_eq!(deps.len(), 1);
        assert!(deps[0].matches("g", "a"));
        assert_eq!(deps[0].version.as_deref(), Some("2.0"));
    }

    #[test]
    fn ignores_comments_and_the_xml_declaration() {
        let xml = "<?xml version=\"1.0\"?><!-- hi --><project><artifactId>a</artifactId></project>";
        let document = parse_xml(xml);
        assert_eq!(
            document.child("project").unwrap().child_text("artifactId"),
            Some("a")
        );
    }

    #[test]
    fn reads_self_closing_elements() {
        let xml = "<project><parent><groupId>g</groupId><version>1</version></parent>\
                   <artifactId>a</artifactId></project>";
        let document = parse_xml(xml);
        let project = document.child("project").unwrap();
        assert!(project.child("parent").is_some());
        assert_eq!(project.child_text("artifactId"), Some("a"));
    }

    #[test]
    fn a_literal_version_is_declared() {
        let xml = "<project><dependencies><dependency><groupId>com.acme</groupId>\
                   <artifactId>core</artifactId><version>1.8.0-SNAPSHOT</version></dependency>\
                   </dependencies></project>";
        let outcome = inspect_dependency(&parse_xml(xml), "com.acme", "core");
        match outcome {
            DependencyVersionInspection::Declared { version } => {
                assert_eq!(version.to_string(), "1.8.0-SNAPSHOT");
            }
            other => panic!("expected a declared version, got {other:?}"),
        }
    }

    #[test]
    fn a_property_version_is_resolved() {
        let xml = "<project><properties><core.version>1.8.0-SNAPSHOT</core.version></properties>\
                   <dependencies><dependency><groupId>com.acme</groupId>\
                   <artifactId>core</artifactId><version>${core.version}</version></dependency>\
                   </dependencies></project>";
        let outcome = inspect_dependency(&parse_xml(xml), "com.acme", "core");
        assert!(matches!(
            outcome,
            DependencyVersionInspection::Declared { .. }
        ));
    }

    #[test]
    fn an_undefined_property_is_unknown() {
        let xml = "<project><dependencies><dependency><groupId>com.acme</groupId>\
                   <artifactId>core</artifactId><version>${missing}</version></dependency>\
                   </dependencies></project>";
        let outcome = inspect_dependency(&parse_xml(xml), "com.acme", "core");
        assert!(matches!(
            outcome,
            DependencyVersionInspection::Unknown { .. }
        ));
    }

    #[test]
    fn a_version_without_a_direct_element_is_unknown_when_a_parent_exists() {
        let xml = "<project><parent><groupId>g</groupId><version>1</version></parent>\
                   <dependencies><dependency><groupId>com.acme</groupId>\
                   <artifactId>core</artifactId></dependency></dependencies></project>";
        let outcome = inspect_dependency(&parse_xml(xml), "com.acme", "core");
        assert!(matches!(
            outcome,
            DependencyVersionInspection::Unknown { .. }
        ));
    }

    #[test]
    fn an_absent_artifact_is_not_declared() {
        let xml = "<project><dependencies><dependency><groupId>other</groupId>\
                   <artifactId>thing</artifactId><version>1</version></dependency>\
                   </dependencies></project>";
        let outcome = inspect_dependency(&parse_xml(xml), "com.acme", "core");
        assert_eq!(outcome, DependencyVersionInspection::NotDeclared);
    }

    #[test]
    fn a_managed_version_is_not_treated_as_declared() {
        let xml = "<project><dependencyManagement><dependencies><dependency>\
                   <groupId>com.acme</groupId><artifactId>core</artifactId>\
                   <version>1.8.0</version></dependency></dependencies></dependencyManagement>\
                   </project>";
        let outcome = inspect_dependency(&parse_xml(xml), "com.acme", "core");
        assert!(matches!(
            outcome,
            DependencyVersionInspection::Unknown { .. }
        ));
    }

    #[tokio::test]
    async fn a_fixed_repository_locates_a_present_artifact() {
        let root = temp_dir("present");
        let coordinate =
            ArtifactCoordinate::new("com.acme", "core", Version::new("1.8.0").unwrap()).unwrap();
        let artifact_path = root.join(coordinate.repository_path());
        std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        std::fs::write(&artifact_path, b"jar").unwrap();

        let inspector = MavenArtifactInspector::with_fixed_repository(
            AbsolutePath::new(root.to_string_lossy().into_owned()).unwrap(),
        );
        let presence = inspector
            .locate(&ResolvedArtifact {
                workspace_id: WorkspaceId::new("acme").unwrap(),
                library: LibraryId::new("core").unwrap(),
                path: AbsolutePath::new(root.to_string_lossy().into_owned()).unwrap(),
                coordinate,
            })
            .await
            .unwrap();
        assert!(presence.is_found());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_fixed_repository_reports_a_missing_artifact() {
        let root = temp_dir("missing");
        let inspector = MavenArtifactInspector::with_fixed_repository(
            AbsolutePath::new(root.to_string_lossy().into_owned()).unwrap(),
        );
        let presence = inspector
            .locate(&ResolvedArtifact {
                workspace_id: WorkspaceId::new("acme").unwrap(),
                library: LibraryId::new("core").unwrap(),
                path: AbsolutePath::new(root.to_string_lossy().into_owned()).unwrap(),
                coordinate: ArtifactCoordinate::new(
                    "com.acme",
                    "core",
                    Version::new("9.9.9").unwrap(),
                )
                .unwrap(),
            })
            .await
            .unwrap();
        assert!(!presence.is_found());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn dependency_version_reads_a_consumer_pom() {
        let consumer = temp_dir("consumer");
        write_pom(
            &consumer,
            "<project><properties><core.version>1.8.0-SNAPSHOT</core.version></properties>\
             <dependencies><dependency><groupId>com.acme</groupId>\
             <artifactId>core</artifactId><version>${core.version}</version></dependency>\
             </dependencies></project>",
        );
        let inspector = MavenArtifactInspector::with_fixed_repository(
            AbsolutePath::new(consumer.to_string_lossy().into_owned()).unwrap(),
        );
        let outcome = inspector
            .dependency_version(&ResolvedDependency {
                path: AbsolutePath::new(consumer.to_string_lossy().into_owned()).unwrap(),
                coordinate: ArtifactCoordinate::new(
                    "com.acme",
                    "core",
                    Version::new("1.8.0-SNAPSHOT").unwrap(),
                )
                .unwrap(),
            })
            .await
            .unwrap();
        assert!(matches!(
            outcome,
            DependencyVersionInspection::Declared { .. }
        ));
        let _ = std::fs::remove_dir_all(&consumer);
    }

    #[tokio::test]
    async fn a_missing_consumer_pom_is_reported_as_io() {
        let consumer = temp_dir("no-pom");
        let inspector = MavenArtifactInspector::with_fixed_repository(
            AbsolutePath::new(consumer.to_string_lossy().into_owned()).unwrap(),
        );
        let error = inspector
            .dependency_version(&ResolvedDependency {
                path: AbsolutePath::new(consumer.to_string_lossy().into_owned()).unwrap(),
                coordinate: ArtifactCoordinate::new(
                    "com.acme",
                    "core",
                    Version::new("1.8.0").unwrap(),
                )
                .unwrap(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "ARTIFACT_IO_ERROR");
        let _ = std::fs::remove_dir_all(&consumer);
    }
}
