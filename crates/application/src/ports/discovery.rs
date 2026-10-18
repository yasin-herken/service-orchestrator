//! Bounded, read-only repository discovery (`tasks/032.md` §7, §8).
//!
//! When a developer imports an existing workspace they point the application at
//! a workspace root and expect it to find the repositories below it. The
//! application core never walks the filesystem itself (`AGENTS.md` §1): it
//! reaches the tree only through [`RepositoryDiscovery`], which returns
//! *observations* — never mutations, and never by executing anything.
//!
//! Discovery is deliberately conservative and bounded:
//!
//! - traversal is capped by a depth and a repository count;
//! - directories on the exclusion list (build outputs, dependency caches,
//!   version-control metadata) are never entered;
//! - a followed symlink must still resolve inside the workspace root;
//! - an unreadable directory is reported as a warning, not a failure;
//! - no project script, build, or Git command is ever run
//!   (`tasks/032.md` §7, §8, §50).
//!
//! Each discovered repository carries the underlying [`RepositoryInspection`]
//! plus [`TechnologyDetection`] facts with an explicit
//! [`DetectionConfidence`], because a detection is a *suggestion* the user
//! reviews, never a silent classification (`tasks/032.md` §8, §9).

use serde::{Deserialize, Serialize};

use crate::ports::repository::RepositoryInspection;

/// How strongly the evidence supports a detected technology.
///
/// Confidence is explicit so an interface can say "probably Maven, verify"
/// instead of presenting an inference as a fact (`tasks/032.md` §8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionConfidence {
    /// The evidence is weak or indirect; the finding must be reviewed.
    Uncertain,
    /// A plausible inference from indirect evidence.
    Low,
    /// A strong inference from project structure.
    Medium,
    /// Direct, unambiguous evidence.
    High,
}

impl DetectionConfidence {
    /// Returns the canonical, machine-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Uncertain => "uncertain",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// One detected technology with the evidence and confidence behind it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TechnologyDetection {
    /// The stable machine-readable kind, for example `maven`, `npm`,
    /// `spring_boot`, or `react`.
    pub technology: String,
    /// A human-readable label, for example `Maven`.
    pub label: String,
    /// How strongly the evidence supports the detection.
    pub confidence: DetectionConfidence,
    /// The files or facts that led to the detection.
    pub evidence: Vec<String>,
    /// An optional caveat, for example "execution provider not installed".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TechnologyDetection {
    /// Builds a detection.
    #[must_use]
    pub fn new(
        technology: impl Into<String>,
        label: impl Into<String>,
        confidence: DetectionConfidence,
        evidence: Vec<String>,
    ) -> Self {
        Self {
            technology: technology.into(),
            label: label.into(),
            confidence,
            evidence,
            note: None,
        }
    }

    /// Attaches a caveat.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Derives the detected technologies from a repository inspection.
    ///
    /// This is the single definition of *what counts as a detection* and how
    /// strong each signal is; adapters and tests share it, so the
    /// confidence rules cannot drift between the real filesystem walker and a
    /// fixture (`tasks/032.md` §8, §45).
    #[must_use]
    pub fn detect(inspection: &RepositoryInspection) -> Vec<TechnologyDetection> {
        let mut detections = Vec::new();

        for system in &inspection.build_systems {
            let confidence = DetectionConfidence::High;
            let detection = match system.system.as_str() {
                "maven" => TechnologyDetection::new(
                    "maven",
                    system.label.clone(),
                    confidence,
                    system.evidence.clone(),
                ),
                "gradle" => TechnologyDetection::new(
                    "gradle",
                    system.label.clone(),
                    confidence,
                    system.evidence.clone(),
                )
                .with_note(
                    system
                        .note
                        .clone()
                        .unwrap_or_else(|| "detected, but no execution provider".to_owned()),
                ),
                "npm" => TechnologyDetection::new(
                    "npm",
                    system.label.clone(),
                    confidence,
                    system.evidence.clone(),
                ),
                _ => TechnologyDetection::new(
                    system.system.clone(),
                    system.label.clone(),
                    confidence,
                    system.evidence.clone(),
                ),
            };
            detections.push(detection);
        }

        if inspection
            .maven
            .as_ref()
            .is_some_and(|maven| maven.wrapper_available)
        {
            detections.push(TechnologyDetection::new(
                "maven_wrapper",
                "Maven Wrapper",
                DetectionConfidence::High,
                vec!["mvnw".to_owned()],
            ));
        }

        for runtime in &inspection.runtimes {
            let confidence = match (runtime.kind.as_str(), &runtime.required_version) {
                ("java", Some(_)) => DetectionConfidence::High,
                ("node", Some(_)) => DetectionConfidence::High,
                _ => DetectionConfidence::Medium,
            };
            detections.push(TechnologyDetection::new(
                runtime.kind.clone(),
                runtime.label.clone(),
                confidence,
                runtime.evidence.clone(),
            ));
        }

        let maven_spring_boot = inspection
            .maven
            .as_ref()
            .is_some_and(|maven| maven.spring_boot);
        for integration in &inspection.integrations {
            let detection = match integration.kind.as_str() {
                "spring_boot" => TechnologyDetection::new(
                    "spring_boot",
                    integration.label.clone(),
                    if maven_spring_boot {
                        DetectionConfidence::High
                    } else {
                        DetectionConfidence::Medium
                    },
                    integration.evidence.clone(),
                ),
                "react" => TechnologyDetection::new(
                    "react",
                    integration.label.clone(),
                    // React is detected from a declared dependency, which is
                    // direct evidence.
                    DetectionConfidence::High,
                    integration.evidence.clone(),
                ),
                "liquibase" => TechnologyDetection::new(
                    "liquibase",
                    integration.label.clone(),
                    // The presence of `liquibase.properties` or a changelog
                    // directory suggests the integration without proving it.
                    DetectionConfidence::Medium,
                    integration.evidence.clone(),
                ),
                _ => TechnologyDetection::new(
                    integration.kind.clone(),
                    integration.label.clone(),
                    DetectionConfidence::Medium,
                    integration.evidence.clone(),
                ),
            };
            detections.push(detection);
        }

        detections
    }

    /// Returns the detection for `technology`, if any.
    #[must_use]
    pub fn find<'a>(
        detections: &'a [TechnologyDetection],
        technology: &str,
    ) -> Option<&'a TechnologyDetection> {
        detections
            .iter()
            .find(|detection| detection.technology == technology)
    }
}

/// A repository found below a workspace root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredRepository {
    /// The directory name of the repository.
    pub name: String,
    /// The path relative to the workspace root, using `/` separators.
    pub relative_path: String,
    /// The read-only inspection of the directory.
    pub inspection: RepositoryInspection,
    /// The detected technologies with their confidence.
    pub detections: Vec<TechnologyDetection>,
}

impl DiscoveredRepository {
    /// Returns `true` when the directory is a Git checkout.
    #[must_use]
    pub fn is_git_repository(&self) -> bool {
        self.inspection.is_git_repository
    }

    /// Returns the `origin` remote URL, when readable.
    #[must_use]
    pub fn remote(&self) -> Option<&str> {
        self.inspection.remote.as_deref()
    }
}

/// The outcome of one bounded discovery scan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryReport {
    /// The workspace root that was scanned.
    pub workspace_root: String,
    /// The repositories found, in deterministic traversal order.
    pub repositories: Vec<DiscoveredRepository>,
    /// Whether the repository cap stopped the scan early.
    pub truncated: bool,
    /// Non-fatal observations: unreadable directories, skipped symlinks
    /// escaping the workspace root, and similar.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A request to discover repositories below a workspace root.
///
/// `exclude` is the *effective* exclusion list: the caller merges the built-in
/// exclusions, the configured extras, and any request-specific extras before
/// calling [`RepositoryDiscovery::discover`], so the port has a single,
/// explicit contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRequest {
    /// The absolute workspace root to scan.
    pub workspace_root: String,
    /// The maximum directory depth below the root.
    pub max_depth: usize,
    /// The effective directory names never entered.
    pub exclude: Vec<String>,
    /// Whether symlinked directories are followed. A followed link must still
    /// resolve inside the workspace root.
    pub follow_symlinks: bool,
    /// The maximum number of repositories reported.
    pub max_repositories: usize,
}

impl DiscoveryRequest {
    /// Creates a request with the default bounds.
    #[must_use]
    pub fn new(workspace_root: impl Into<String>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            max_depth: service_orchestrator_config::DEFAULT_DISCOVERY_MAX_DEPTH,
            exclude: service_orchestrator_config::DEFAULT_DISCOVERY_EXCLUDE
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            follow_symlinks: true,
            max_repositories: service_orchestrator_config::DEFAULT_DISCOVERY_MAX_REPOSITORIES,
        }
    }
}

/// An error raised when a scan cannot be attempted at all.
///
/// A workspace root that does not exist or is not a directory is an error; an
/// unreadable directory *below* a valid root is a warning on the report.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("repository discovery error ({code}): {message}")]
pub struct DiscoveryError {
    /// A stable, machine-readable code such as `DISCOVERY_UNAVAILABLE`.
    pub code: String,
    /// A human-readable explanation.
    pub message: String,
}

impl DiscoveryError {
    /// Builds an error with `code` and `message`.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// A read-only, bounded discovery boundary over a workspace root.
///
/// Implementations must be side-effect free: reads only, no script execution,
/// no build, no install, no checkout (`tasks/032.md` §7, §50).
pub trait RepositoryDiscovery: Send + Sync {
    /// Scans the workspace root described by `request`.
    ///
    /// # Errors
    ///
    /// Returns [`DiscoveryError`] only when the scan could not be attempted at
    /// all, for example a missing workspace root. Partial findings are returned
    /// on the report with warnings.
    fn discover(&self, request: &DiscoveryRequest) -> Result<DiscoveryReport, DiscoveryError>;
}

/// A [`RepositoryDiscovery`] that refuses to scan anything.
///
/// Used when the application core is built without a filesystem walker. Every
/// call fails with `DISCOVERY_UNAVAILABLE` rather than fabricating a result.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullRepositoryDiscovery;

impl RepositoryDiscovery for NullRepositoryDiscovery {
    fn discover(&self, _request: &DiscoveryRequest) -> Result<DiscoveryReport, DiscoveryError> {
        Err(DiscoveryError::new(
            "DISCOVERY_UNAVAILABLE",
            "repository discovery is not available without a filesystem walker",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::repository::{
        DetectedBuildSystem, DetectedIntegration, DetectedRuntime, MavenMetadata, NodeMetadata,
        ProviderAvailability,
    };

    fn inspection() -> RepositoryInspection {
        let mut inspection = RepositoryInspection::missing("/projects/acme/auth-service");
        inspection.exists = true;
        inspection.is_directory = true;
        inspection.is_git_repository = true;
        inspection.build_systems.push(DetectedBuildSystem {
            system: "maven".to_owned(),
            label: "Maven".to_owned(),
            availability: ProviderAvailability::Available,
            evidence: vec!["pom.xml".to_owned()],
            note: None,
        });
        inspection.maven = Some(MavenMetadata {
            artifact_id: Some("auth-service".to_owned()),
            wrapper_available: true,
            spring_boot: true,
            ..MavenMetadata::default()
        });
        inspection.runtimes.push(DetectedRuntime {
            kind: "java".to_owned(),
            label: "Java".to_owned(),
            required_version: Some("21".to_owned()),
            evidence: vec!["pom.xml".to_owned()],
        });
        inspection.integrations.push(DetectedIntegration {
            kind: "spring_boot".to_owned(),
            label: "Spring Boot".to_owned(),
            evidence: vec!["application.yml".to_owned()],
        });
        inspection
    }

    #[test]
    fn derives_detections_with_confidence_and_evidence() {
        let detections = TechnologyDetection::detect(&inspection());
        let maven = TechnologyDetection::find(&detections, "maven").expect("Maven is detected");
        assert_eq!(maven.confidence, DetectionConfidence::High);
        assert_eq!(maven.evidence, vec!["pom.xml".to_owned()]);
        let wrapper =
            TechnologyDetection::find(&detections, "maven_wrapper").expect("the wrapper is known");
        assert_eq!(wrapper.confidence, DetectionConfidence::High);
        let spring =
            TechnologyDetection::find(&detections, "spring_boot").expect("Spring Boot is known");
        assert_eq!(spring.confidence, DetectionConfidence::High);
        let java = TechnologyDetection::find(&detections, "java").expect("Java is detected");
        assert_eq!(java.confidence, DetectionConfidence::High);
    }

    #[test]
    fn a_gradle_project_is_detected_but_not_operational() {
        let mut inspection = RepositoryInspection::missing("/projects/acme/legacy");
        inspection.exists = true;
        inspection.is_directory = true;
        inspection.build_systems.push(DetectedBuildSystem {
            system: "gradle".to_owned(),
            label: "Gradle".to_owned(),
            availability: ProviderAvailability::ProviderNotInstalled,
            evidence: vec!["build.gradle".to_owned()],
            note: Some("Detected: Gradle — execution provider not installed.".to_owned()),
        });
        let detections = TechnologyDetection::detect(&inspection);
        let gradle = TechnologyDetection::find(&detections, "gradle").expect("Gradle is detected");
        assert_eq!(gradle.confidence, DetectionConfidence::High);
        assert!(gradle
            .note
            .as_deref()
            .is_some_and(|note| note.contains("not installed")));
    }

    #[test]
    fn an_indirect_spring_boot_signal_is_only_medium_confidence() {
        let mut inspection = RepositoryInspection::missing("/projects/acme/web");
        inspection.exists = true;
        inspection.is_directory = true;
        inspection.integrations.push(DetectedIntegration {
            kind: "spring_boot".to_owned(),
            label: "Spring Boot".to_owned(),
            evidence: vec!["application.yml".to_owned()],
        });
        let detections = TechnologyDetection::detect(&inspection);
        let spring =
            TechnologyDetection::find(&detections, "spring_boot").expect("Spring Boot is known");
        assert_eq!(spring.confidence, DetectionConfidence::Medium);
    }

    #[test]
    fn a_react_dependency_is_high_confidence() {
        let mut inspection = RepositoryInspection::missing("/projects/acme/web-ui");
        inspection.exists = true;
        inspection.is_directory = true;
        inspection.node = Some(NodeMetadata {
            react: true,
            required_node_version: Some("20.x".to_owned()),
            ..NodeMetadata::default()
        });
        inspection.runtimes.push(DetectedRuntime {
            kind: "node".to_owned(),
            label: "Node.js".to_owned(),
            required_version: Some("20.x".to_owned()),
            evidence: vec!["package.json".to_owned()],
        });
        inspection.integrations.push(DetectedIntegration {
            kind: "react".to_owned(),
            label: "React".to_owned(),
            evidence: vec!["package.json".to_owned()],
        });
        let detections = TechnologyDetection::detect(&inspection);
        let react = TechnologyDetection::find(&detections, "react").expect("React is detected");
        assert_eq!(react.confidence, DetectionConfidence::High);
        let node = TechnologyDetection::find(&detections, "node").expect("Node is detected");
        assert_eq!(node.confidence, DetectionConfidence::High);
    }

    #[test]
    fn the_null_walker_never_fabricates_a_report() {
        let error = NullRepositoryDiscovery
            .discover(&DiscoveryRequest::new("/projects/acme"))
            .expect_err("the null walker refuses");
        assert_eq!(error.code, "DISCOVERY_UNAVAILABLE");
    }

    #[test]
    fn default_bounds_match_the_configuration_defaults() {
        let request = DiscoveryRequest::new("/projects/acme");
        assert_eq!(request.max_depth, 6);
        assert_eq!(request.max_repositories, 200);
        assert!(request.follow_symlinks);
        assert!(request.exclude.contains(&".git".to_owned()));
        assert!(request.exclude.contains(&"node_modules".to_owned()));
    }
}
