//! The real, bounded [`RepositoryDiscovery`] walker used by the composition
//! root.
//!
//! The application core never walks the filesystem itself (`AGENTS.md` §1): it
//! asks the [`RepositoryDiscovery`] boundary, and this module is the
//! production implementation. It is intentionally conservative
//! (`tasks/032.md` §7, §8):
//!
//! - traversal is bounded by a depth and a repository count, both clamped to the
//!   hard caps the configuration module defines;
//! - excluded directory names (build outputs, dependency caches, `.git`) are
//!   never entered;
//! - a symlinked directory is followed only when it resolves back inside the
//!   workspace root, and each real directory is visited once;
//! - an unreadable directory below the root is a warning, not a failure;
//! - nothing is written, executed, built, cloned, or installed. The only work
//!   per candidate is the read-only [`SystemRepositoryInspector`] pass.
//!
//! A directory is a *candidate repository* when it directly contains one of the
//! well-known project markers below. Traversal continues through a candidate,
//! so nested repositories (a frontend inside a backend checkout, for example)
//! are still discovered.

use std::collections::BTreeSet;
use std::fs::DirEntry;
use std::path::{Path, PathBuf};

use service_orchestrator_application::{
    DiscoveredRepository, DiscoveryError, DiscoveryReport, DiscoveryRequest, RepositoryDiscovery,
    RepositoryInspector, TechnologyDetection,
};
use service_orchestrator_config::MAX_DISCOVERY_DEPTH;

use crate::inspector::SystemRepositoryInspector;

/// The files whose direct presence makes a directory a candidate repository.
///
/// They mirror the project files the inspector knows about: version-control
/// metadata, build manifests and wrappers, and Liquibase configuration.
const REPOSITORY_MARKERS: &[&str] = &[
    ".git",
    "pom.xml",
    "mvnw",
    "build.gradle",
    "build.gradle.kts",
    "gradlew",
    "package.json",
    "liquibase.properties",
];

/// The production repository discovery walker.
///
/// Constructed once in the composition root and handed to the application core
/// through [`Application::with_repository_discovery`], so the same bounded
/// scan backs the TUI, the desktop app, and MCP.
///
/// [`Application::with_repository_discovery`]:
///     service_orchestrator_application::Application::with_repository_discovery
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRepositoryDiscovery {
    inspector: SystemRepositoryInspector,
}

impl SystemRepositoryDiscovery {
    /// Creates a walker over `inspector`.
    #[must_use]
    pub const fn new(inspector: SystemRepositoryInspector) -> Self {
        Self { inspector }
    }
}

impl RepositoryDiscovery for SystemRepositoryDiscovery {
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryReport, DiscoveryError> {
        let root = PathBuf::from(&request.workspace_root);
        let metadata = std::fs::metadata(&root).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                DiscoveryError::new(
                    "DISCOVERY_ROOT_MISSING",
                    format!(
                        "the workspace root '{}' does not exist",
                        request.workspace_root
                    ),
                )
            } else {
                DiscoveryError::new(
                    "DISCOVERY_ROOT_UNREADABLE",
                    format!(
                        "the workspace root '{}' could not be read: {error}",
                        request.workspace_root
                    ),
                )
            }
        })?;
        if !metadata.is_dir() {
            return Err(DiscoveryError::new(
                "DISCOVERY_ROOT_NOT_A_DIRECTORY",
                format!(
                    "the workspace root '{}' is not a directory",
                    request.workspace_root
                ),
            ));
        }

        let mut walk = Walk {
            inspector: self.inspector,
            request,
            root: root.clone(),
            root_canonical: std::fs::canonicalize(&root).ok(),
            max_depth: request.max_depth.min(MAX_DISCOVERY_DEPTH),
            max_repositories: request.max_repositories.max(1),
            visited: BTreeSet::new(),
            repositories: Vec::new(),
            warnings: Vec::new(),
            truncated: false,
        };
        if let Some(canonical) = &walk.root_canonical {
            walk.visited.insert(canonical.clone());
        } else {
            walk.warnings.push(
                "the workspace root could not be resolved to a real path; symlinked directories \
                 are not followed"
                    .to_owned(),
            );
        }
        walk.visit(&root, 0, "");

        Ok(DiscoveryReport {
            workspace_root: request.workspace_root.clone(),
            repositories: walk.repositories,
            truncated: walk.truncated,
            warnings: walk.warnings,
        })
    }
}

/// The mutable state of one bounded scan.
struct Walk<'a> {
    inspector: SystemRepositoryInspector,
    request: &'a DiscoveryRequest,
    root: PathBuf,
    root_canonical: Option<PathBuf>,
    max_depth: usize,
    max_repositories: usize,
    /// Canonical paths of the directories already visited, so a directory
    /// reached twice (through a symlink or a link chain) is reported once.
    visited: BTreeSet<PathBuf>,
    repositories: Vec<DiscoveredRepository>,
    warnings: Vec<String>,
    truncated: bool,
}

impl Walk<'_> {
    /// Visits one directory at `depth`, recording the candidate repositories
    /// among its children.
    fn visit(&mut self, dir: &Path, depth: usize, relative: &str) {
        if self.truncated || depth >= self.max_depth {
            return;
        }
        let read = match std::fs::read_dir(dir) {
            Ok(read) => read,
            Err(error) => {
                self.warnings.push(format!(
                    "directory '{}' could not be read and was skipped: {error}",
                    display_relative(relative)
                ));
                return;
            }
        };
        let mut entries: Vec<DirEntry> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(DirEntry::file_name);

        for entry in entries {
            if self.truncated {
                return;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy().into_owned();
            if self
                .request
                .exclude
                .iter()
                .any(|excluded| excluded == &name)
            {
                continue;
            }
            let path = entry.path();
            let child_relative = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };

            let is_symlink = match entry.file_type() {
                Ok(file_type) => file_type.is_symlink(),
                Err(error) => {
                    self.warnings.push(format!(
                        "'{child_relative}' could not be inspected and was skipped: {error}"
                    ));
                    continue;
                }
            };
            // `entry.metadata()` does not follow a symlink; `fs::metadata` does.
            // Only directories are interesting, whether linked or not.
            let is_directory = if is_symlink {
                match std::fs::metadata(&path) {
                    Ok(metadata) => metadata.is_dir(),
                    Err(error) => {
                        self.warnings.push(format!(
                            "symlink '{child_relative}' could not be resolved and was skipped: \
                             {error}"
                        ));
                        continue;
                    }
                }
            } else {
                match entry.metadata() {
                    Ok(metadata) => metadata.is_dir(),
                    Err(error) => {
                        self.warnings.push(format!(
                            "'{child_relative}' could not be inspected and was skipped: {error}"
                        ));
                        continue;
                    }
                }
            };
            if !is_directory {
                continue;
            }

            if is_symlink {
                if !self.follows_symlink(&path, &child_relative) {
                    continue;
                }
            } else if let Ok(canonical) = std::fs::canonicalize(&path) {
                // A regular directory that is the target of an earlier symlink
                // has already been walked; report it once.
                if !self.visited.insert(canonical) {
                    continue;
                }
            }

            self.record(&path, &name, &child_relative);
            self.visit(&path, depth + 1, &child_relative);
        }
    }

    /// Applies the symlink policy: traversal may be disabled, the link must
    /// resolve inside the workspace root, and the resolved directory must not
    /// have been visited already.
    fn follows_symlink(&mut self, path: &Path, relative: &str) -> bool {
        if !self.request.follow_symlinks {
            self.warnings.push(format!(
                "symlink '{relative}' was skipped because symlink traversal is disabled"
            ));
            return false;
        }
        let Some(root_canonical) = &self.root_canonical else {
            self.warnings.push(format!(
                "symlink '{relative}' was skipped because the workspace root could not be \
                 resolved"
            ));
            return false;
        };
        match std::fs::canonicalize(path) {
            Ok(canonical) => {
                if !canonical.starts_with(root_canonical) {
                    self.warnings.push(format!(
                        "symlink '{relative}' resolves outside '{}' and was not followed",
                        self.root.display()
                    ));
                    return false;
                }
                if !self.visited.insert(canonical) {
                    // The same real directory was already walked.
                    return false;
                }
                true
            }
            Err(error) => {
                self.warnings.push(format!(
                    "symlink '{relative}' could not be resolved and was skipped: {error}"
                ));
                false
            }
        }
    }

    /// Records the directory as a repository when it carries a project marker.
    fn record(&mut self, path: &Path, name: &str, relative: &str) {
        if !is_repository(path) {
            return;
        }
        if self.repositories.len() >= self.max_repositories {
            self.truncated = true;
            self.warnings.push(format!(
                "repository discovery stopped after {} repositories; raise \
                 discovery.max_repositories or narrow the scan",
                self.max_repositories
            ));
            return;
        }
        let Some(absolute) = path.to_str() else {
            self.warnings.push(format!(
                "'{relative}' has a path that is not valid UTF-8 and was skipped"
            ));
            return;
        };
        match self.inspector.inspect(absolute) {
            Ok(inspection) => {
                let detections = TechnologyDetection::detect(&inspection);
                self.repositories.push(DiscoveredRepository {
                    name: name.to_owned(),
                    relative_path: relative.to_owned(),
                    inspection,
                    detections,
                });
            }
            Err(error) => self.warnings.push(format!(
                "'{relative}' matched a project marker but could not be inspected: {}",
                error.message
            )),
        }
    }
}

/// Returns `true` when the directory directly contains a project marker.
fn is_repository(dir: &Path) -> bool {
    REPOSITORY_MARKERS
        .iter()
        .any(|marker| dir.join(marker).symlink_metadata().is_ok())
}

/// Renders a relative directory for a warning; the root is spelled out.
fn display_relative(relative: &str) -> &str {
    if relative.is_empty() {
        "<workspace root>"
    } else {
        relative
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "service-orchestrator-discovery-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir is created");
        dir
    }

    fn walker() -> SystemRepositoryDiscovery {
        SystemRepositoryDiscovery::new(SystemRepositoryInspector)
    }

    fn request(root: &Path) -> DiscoveryRequest {
        let mut request = DiscoveryRequest::new(root.to_str().expect("a UTF-8 path"));
        // Tests create their fixtures; only the built-in exclusions apply.
        request.exclude = vec![
            ".git".to_owned(),
            "node_modules".to_owned(),
            "target".to_owned(),
        ];
        request
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the parent directory is created");
        }
        std::fs::write(path, contents).expect("the fixture file is written");
    }

    fn relative_paths(report: &DiscoveryReport) -> Vec<String> {
        report
            .repositories
            .iter()
            .map(|repository| repository.relative_path.clone())
            .collect()
    }

    #[test]
    fn a_missing_root_is_an_error_not_an_empty_report() {
        let root = temp_dir("missing").join("nope");
        let error = walker()
            .discover(&request(&root))
            .expect_err("a missing root is an error");
        assert_eq!(error.code, "DISCOVERY_ROOT_MISSING");
    }

    #[test]
    fn a_file_root_is_not_a_directory() {
        let root = temp_dir("file-root");
        let file = root.join("config.toml");
        write(&file, "config_version = 1\n");
        let error = walker()
            .discover(&request(&file))
            .expect_err("a file root is an error");
        assert_eq!(error.code, "DISCOVERY_ROOT_NOT_A_DIRECTORY");
    }

    #[test]
    fn maven_and_node_repositories_are_found_with_detections() {
        let root = temp_dir("find");
        write(
            &root.join("services/catalog-service/pom.xml"),
            "<project><artifactId>catalog-service</artifactId></project>",
        );
        write(
            &root.join("frontend/storefront/package.json"),
            r#"{"name":"storefront","dependencies":{"react":"^18.0.0"}}"#,
        );
        // Excluded and marker-less directories never become repositories.
        write(&root.join("node_modules/dep/package.json"), "{}");
        write(&root.join("target/classes/pom.xml"), "<project/>");
        std::fs::create_dir_all(root.join("services/notes")).expect("a marker-less directory");

        let report = walker()
            .discover(&request(&root))
            .expect("the scan succeeds");
        assert!(!report.truncated);
        assert_eq!(
            relative_paths(&report),
            vec![
                "frontend/storefront".to_owned(),
                "services/catalog-service".to_owned()
            ]
        );
        let catalog = &report.repositories[1];
        assert_eq!(catalog.name, "catalog-service");
        assert!(TechnologyDetection::find(&catalog.detections, "maven").is_some());
        let storefront = &report.repositories[0];
        assert!(TechnologyDetection::find(&storefront.detections, "npm").is_some());
        assert!(TechnologyDetection::find(&storefront.detections, "react").is_some());
        assert_eq!(report.workspace_root, root.to_str().expect("a UTF-8 path"));
    }

    #[test]
    fn traversal_stops_at_the_configured_depth() {
        let root = temp_dir("depth");
        write(&root.join("level-one/level-two/pom.xml"), "<project/>");

        let mut shallow = request(&root);
        shallow.max_depth = 1;
        let report = walker().discover(&shallow).expect("the scan succeeds");
        assert!(report.repositories.is_empty());

        let mut deep = request(&root);
        deep.max_depth = 2;
        let report = walker().discover(&deep).expect("the scan succeeds");
        assert_eq!(
            relative_paths(&report),
            vec!["level-one/level-two".to_owned()]
        );
    }

    #[test]
    fn nested_repositories_are_reported_and_git_worktrees_are_markers() {
        let root = temp_dir("nested");
        std::fs::create_dir_all(root.join("platform/.git")).expect("the git directory");
        write(&root.join("platform/pom.xml"), "<project/>");
        write(&root.join("platform/web/.git"), "gitdir: /elsewhere\n");
        write(&root.join("platform/web/package.json"), r#"{"name":"web"}"#);

        let report = walker()
            .discover(&request(&root))
            .expect("the scan succeeds");
        assert_eq!(
            relative_paths(&report),
            vec!["platform".to_owned(), "platform/web".to_owned()]
        );
        assert!(report.repositories[0].is_git_repository());
        assert!(report.repositories[1].is_git_repository());
    }

    #[test]
    fn the_repository_cap_truncates_the_scan_with_a_warning() {
        let root = temp_dir("cap");
        for name in ["a", "b", "c"] {
            write(&root.join(format!("{name}/pom.xml")), "<project/>");
        }
        let mut request = request(&root);
        request.max_repositories = 2;

        let report = walker().discover(&request).expect("the scan succeeds");
        assert!(report.truncated);
        assert_eq!(
            relative_paths(&report),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("stopped after 2 repositories")));
    }

    #[test]
    fn paths_with_spaces_and_unicode_are_discovered() {
        let root = temp_dir("unicode");
        write(&root.join("my services/api service/pom.xml"), "<project/>");
        write(&root.join("components/über-service/package.json"), "{}");

        let report = walker()
            .discover(&request(&root))
            .expect("the scan succeeds");
        assert_eq!(
            relative_paths(&report),
            vec![
                "components/über-service".to_owned(),
                "my services/api service".to_owned()
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_is_followed_inside_the_root_and_reported_once() {
        let root = temp_dir("symlink-inside");
        write(&root.join("real/pom.xml"), "<project/>");
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).expect("the symlink");

        let report = walker()
            .discover(&request(&root))
            .expect("the scan succeeds");
        assert_eq!(relative_paths(&report), vec!["link".to_owned()]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_outside_the_workspace_root_is_not_followed() {
        let root = temp_dir("symlink-outside-root");
        let outside = temp_dir("symlink-outside-target");
        write(&outside.join("secret/pom.xml"), "<project/>");
        std::os::unix::fs::symlink(&outside, root.join("escape")).expect("the symlink");

        let report = walker()
            .discover(&request(&root))
            .expect("the scan succeeds");
        assert!(report.repositories.is_empty());
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("resolves outside")));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_traversal_can_be_disabled() {
        let root = temp_dir("symlink-disabled");
        write(&root.join("real/pom.xml"), "<project/>");
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).expect("the symlink");
        let mut request = request(&root);
        request.follow_symlinks = false;

        let report = walker().discover(&request).expect("the scan succeeds");
        assert_eq!(relative_paths(&report), vec!["real".to_owned()]);
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("symlink traversal is disabled")));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_is_a_warning() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_dir("unreadable");
        write(&root.join("hidden/pom.xml"), "<project/>");
        write(&root.join("visible/pom.xml"), "<project/>");
        let locked = root.join("locked");
        std::fs::create_dir_all(&locked).expect("the directory");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o0)).expect("the mode");

        // Running with enough privileges to bypass the mode makes the fixture
        // meaningless; skip rather than assert something that cannot happen.
        if std::fs::read_dir(&locked).is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).ok();
            return;
        }

        let report = walker()
            .discover(&request(&root))
            .expect("the scan succeeds");
        assert_eq!(
            relative_paths(&report),
            vec!["hidden".to_owned(), "visible".to_owned()]
        );
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("locked") && warning.contains("could not be read")));

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).ok();
    }

    #[test]
    fn the_depth_and_repository_caps_are_clamped_to_the_hard_limits() {
        let root = temp_dir("clamp");
        write(&root.join("one/pom.xml"), "<project/>");
        write(&root.join("two/pom.xml"), "<project/>");
        let mut request = request(&root);
        request.max_depth = usize::MAX;
        request.max_repositories = 0;

        // `usize::MAX` depths must not recurse without bound and a zero
        // repository cap must behave as one, never as unlimited.
        let report = walker().discover(&request).expect("the scan succeeds");
        assert!(report.truncated);
        assert_eq!(relative_paths(&report), vec!["one".to_owned()]);
    }
}
