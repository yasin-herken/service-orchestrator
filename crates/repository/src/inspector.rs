//! The real, read-only [`RepositoryInspector`] the composition root wires into
//! the application core.
//!
//! This adapter is the only place that reads a developer's repository
//! directory for inspection purposes. It performs **reads only**: it never runs
//! a build, a script, `npm install`, or a Git command, and it never writes
//! (`tasks/023.md` §17, §20). Every observation is a *suggestion* for the
//! service wizard, not a committed fact.
//!
//! The logic is intentionally bounded. It reads well-known files (and at most a
//! small prefix of any one of them), looks for `.git/HEAD` and `.git/config`,
//! and greps a `pom.xml` for a handful of tags. Anything that would require
//! effective-POM evaluation or a dependency resolver is left unresolved rather
//! than guessed (`tasks/023.md` §21).

use std::path::{Path, PathBuf};

use service_orchestrator_application::{
    DetectedBuildSystem, DetectedIntegration, DetectedRuntime, MavenMetadata, NodeMetadata,
    ProviderAvailability, RepositoryInspection, RepositoryInspectionError, RepositoryInspector,
};

/// The largest prefix of a file the inspector will read.
///
/// It bounds work on a hostile or accidentally huge `pom.xml`/`package.json`
/// without needing to stat the file first.
const MAX_TEXT_BYTES: u64 = 1024 * 1024;

/// The well-known files the inspector reports when present, in a stable order.
const PROJECT_FILES: &[&str] = &[
    "pom.xml",
    "mvnw",
    "build.gradle",
    "build.gradle.kts",
    "gradlew",
    "package.json",
    "package-lock.json",
    "application.yml",
    "application.yaml",
    "application.properties",
    "liquibase.properties",
];

/// The production repository inspector.
///
/// It reads a local directory through the standard library and produces the
/// structured [`RepositoryInspection`] the application core consumes.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRepositoryInspector;

impl RepositoryInspector for SystemRepositoryInspector {
    fn inspect(&self, path: &str) -> Result<RepositoryInspection, RepositoryInspectionError> {
        let root = PathBuf::from(path);
        let metadata = match std::fs::metadata(&root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RepositoryInspection::missing(path));
            }
            Err(error) => {
                return Err(RepositoryInspectionError::new(
                    "INSPECTION_FAILED",
                    format!("could not inspect '{path}': {error}"),
                ));
            }
        };

        let mut inspection = RepositoryInspection::missing(path);
        inspection.exists = true;
        inspection.is_directory = metadata.is_dir();
        if !metadata.is_dir() {
            return Ok(inspection);
        }

        detect_git(&root, &mut inspection);
        detect_project_files(&root, &mut inspection);
        detect_maven(&root, &mut inspection);
        detect_node(&root, &mut inspection);
        detect_integrations(&root, &mut inspection);
        Ok(inspection)
    }
}

/// Reads `.git/HEAD` and `.git/config` for the branch and `origin` remote.
fn detect_git(root: &Path, inspection: &mut RepositoryInspection) {
    let git = root.join(".git");
    if !git.exists() {
        return;
    }
    inspection.is_git_repository = true;
    // A `.git` file means this is a linked worktree; the real directory is
    // elsewhere. The branch and remote are then left unresolved rather than
    // guessed (`tasks/023.md` §17).
    if !git.is_dir() {
        inspection.notes.push(
            "This is a linked worktree or submodule; branch and remote are reported by Git."
                .to_owned(),
        );
        return;
    }
    if let Some(head) = read_bounded(&git.join("HEAD")) {
        if let Some(reference) = head.trim().strip_prefix("ref: refs/heads/") {
            inspection.branch = Some(reference.to_owned());
        }
    }
    if let Some(config) = read_bounded(&git.join("config")) {
        inspection.remote = origin_url(&config);
    }
}

/// Extracts the `url` of the `[remote "origin"]` section from `.git/config`.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
            continue;
        }
        if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "url" {
                    return Some(value.trim().to_owned());
                }
            }
        }
    }
    None
}

/// Records the well-known project files that are present.
fn detect_project_files(root: &Path, inspection: &mut RepositoryInspection) {
    for file in PROJECT_FILES {
        if root.join(file).exists() {
            inspection.project_files.push((*file).to_owned());
        }
    }
}

/// Detects Maven and Gradle build systems and their metadata.
fn detect_maven(root: &Path, inspection: &mut RepositoryInspection) {
    let wrapper = root.join("mvnw").exists();
    let pom = root.join("pom.xml");
    let gradle_files: Vec<&str> = ["build.gradle", "build.gradle.kts", "gradlew"]
        .into_iter()
        .filter(|file| root.join(file).exists())
        .collect();

    if !gradle_files.is_empty() {
        // Gradle is reported as detected but *not operational*: no execution
        // provider is implemented (`tasks/023.md` §19). The wizard surfaces the
        // note instead of offering a build.
        inspection.build_systems.push(DetectedBuildSystem {
            system: "gradle".to_owned(),
            label: "Gradle".to_owned(),
            availability: ProviderAvailability::ProviderNotInstalled,
            evidence: gradle_files.iter().map(|file| (*file).to_owned()).collect(),
            note: Some("Detected: Gradle — execution provider not installed.".to_owned()),
        });
    }

    if !pom.exists() {
        return;
    }

    inspection.build_systems.push(DetectedBuildSystem {
        system: "maven".to_owned(),
        label: "Maven".to_owned(),
        availability: ProviderAvailability::Available,
        evidence: if wrapper {
            vec!["pom.xml".to_owned(), "mvnw".to_owned()]
        } else {
            vec!["pom.xml".to_owned()]
        },
        note: None,
    });

    let Some(xml) = read_bounded(&pom) else {
        inspection
            .notes
            .push("pom.xml could not be read; Maven metadata is unavailable.".to_owned());
        return;
    };

    let artifact_id = first_tag(&xml, "artifactId");
    let group_id = first_tag(&xml, "groupId");
    let version = first_tag(&xml, "version");
    let packaging = first_tag(&xml, "packaging");
    let spring_boot = xml.contains("spring-boot") || xml.contains("springframework.boot");
    let java_version = first_tag(&xml, "java.version")
        .or_else(|| first_tag(&xml, "maven.compiler.release"))
        .or_else(|| first_tag(&xml, "maven.compiler.source"))
        .or_else(|| first_tag(&xml, "maven.compiler.target"));

    let metadata = MavenMetadata {
        group_id,
        artifact_id,
        version,
        packaging,
        java_version,
        modules: all_tags(&xml, "module"),
        wrapper_available: wrapper,
        spring_boot,
    };

    inspection.notes.push(
        "Maven metadata comes from this POM only; inherited or resolved values are left unresolved."
            .to_owned(),
    );
    inspection.runtimes.push(DetectedRuntime {
        kind: "java".to_owned(),
        label: "Java".to_owned(),
        required_version: metadata.java_version.clone(),
        evidence: vec!["pom.xml".to_owned()],
    });
    inspection.maven = Some(metadata);
}

/// Detects npm and its metadata from `package.json` and version files.
fn detect_node(root: &Path, inspection: &mut RepositoryInspection) {
    let package_json = root.join("package.json");
    if !package_json.exists() {
        return;
    }

    inspection.build_systems.push(DetectedBuildSystem {
        system: "npm".to_owned(),
        label: "npm".to_owned(),
        availability: ProviderAvailability::Available,
        evidence: if root.join("package-lock.json").exists() {
            vec!["package.json".to_owned(), "package-lock.json".to_owned()]
        } else {
            vec!["package.json".to_owned()]
        },
        note: None,
    });

    let mut metadata = NodeMetadata::default();
    if let Some(text) = read_bounded(&package_json) {
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(json) => {
                metadata.name = json
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                metadata.version = json
                    .get("version")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                metadata.required_node_version = json
                    .get("engines")
                    .and_then(|engines| engines.get("node"))
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
                metadata.package_manager = json
                    .get("packageManager")
                    .and_then(|value| value.as_str())
                    .map(|value| value.split('@').next().unwrap_or(value).to_owned());
                if let Some(scripts) = json.get("scripts").and_then(|value| value.as_object()) {
                    metadata.scripts = scripts.keys().cloned().collect();
                }
                metadata.react = has_dependency(&json, "react")
                    || has_dependency(&json, "react-dom")
                    || has_dependency(&json, "next");
            }
            Err(_) => inspection
                .notes
                .push("package.json could not be parsed; Node metadata is unavailable.".to_owned()),
        }
    } else {
        inspection
            .notes
            .push("package.json could not be read; Node metadata is unavailable.".to_owned());
    }

    // `.nvmrc`/`.node-version` take precedence over `engines.node`.
    for file in [".nvmrc", ".node-version"] {
        if let Some(version) = read_bounded(&root.join(file)) {
            let version = version.trim();
            if !version.is_empty() {
                metadata.required_node_version = Some(version.to_owned());
                break;
            }
        }
    }

    inspection.runtimes.push(DetectedRuntime {
        kind: "node".to_owned(),
        label: "Node.js".to_owned(),
        required_version: metadata.required_node_version.clone(),
        evidence: vec!["package.json".to_owned()],
    });
    inspection.node = Some(metadata);
}

/// Detects Spring Boot, Liquibase, and React integrations.
fn detect_integrations(root: &Path, inspection: &mut RepositoryInspection) {
    let spring_boot = inspection
        .maven
        .as_ref()
        .is_some_and(|maven| maven.spring_boot)
        || root.join("application.yml").exists()
        || root.join("application.yaml").exists()
        || root.join("application.properties").exists();
    if spring_boot {
        inspection.integrations.push(DetectedIntegration {
            kind: "spring_boot".to_owned(),
            label: "Spring Boot".to_owned(),
            evidence: vec!["application.yml".to_owned()],
        });
    }

    if root.join("liquibase.properties").exists() || root.join("db/changelog").exists() {
        inspection.integrations.push(DetectedIntegration {
            kind: "liquibase".to_owned(),
            label: "Liquibase".to_owned(),
            evidence: vec!["liquibase.properties".to_owned(), "db/changelog".to_owned()],
        });
    }

    if inspection.node.as_ref().is_some_and(|node| node.react) {
        inspection.integrations.push(DetectedIntegration {
            kind: "react".to_owned(),
            label: "React".to_owned(),
            evidence: vec!["package.json".to_owned()],
        });
    }
}

/// Returns whether `json` declares `name` in any dependency section.
fn has_dependency(json: &serde_json::Value, name: &str) -> bool {
    ["dependencies", "devDependencies", "peerDependencies"]
        .into_iter()
        .any(|section| {
            json.get(section)
                .and_then(|value| value.as_object())
                .is_some_and(|dependencies| dependencies.contains_key(name))
        })
}

/// Reads a file's prefix as UTF-8, returning `None` when it cannot be read or
/// is not valid UTF-8.
fn read_bounded(path: &Path) -> Option<String> {
    use std::io::Read;

    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_TEXT_BYTES).read_to_end(&mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

/// Returns the first `<name>...</name>` value in `xml`, trimmed.
fn first_tag(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let value = xml[start..end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Returns every `<name>...</name>` value in `xml`, trimmed and non-empty.
fn all_tags(xml: &str, name: &str) -> Vec<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut rest = xml;
    let mut values = Vec::new();
    while let Some(start) = rest.find(&open) {
        let after = start + open.len();
        let Some(end) = rest[after..].find(&close) else {
            break;
        };
        let value = rest[after..after + end].trim();
        if !value.is_empty() {
            values.push(value.to_owned());
        }
        rest = &rest[after + end + close.len()..];
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "service-orchestrator-inspector-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir is created");
        dir
    }

    #[test]
    fn a_missing_directory_is_reported_as_missing_not_an_error() {
        let path = temp_dir("missing").join("nope");
        let inspection = SystemRepositoryInspector
            .inspect(path.to_str().expect("a path"))
            .expect("inspection does not fail on a missing directory");
        assert!(!inspection.exists);
        assert!(inspection.build_systems.is_empty());
    }

    #[test]
    fn a_maven_project_is_detected_with_metadata() {
        let dir = temp_dir("maven");
        std::fs::write(
            dir.join("pom.xml"),
            "<project><groupId>com.acme</groupId><artifactId>auth-service</artifactId>\
             <version>1.0.0</version><packaging>jar</packaging>\
             <properties><java.version>21</java.version></properties>\
             <dependencies><dependency><groupId>org.springframework.boot</groupId>\
             <artifactId>spring-boot-starter-web</artifactId></dependency></dependencies></project>",
        )
        .expect("the pom is written");
        std::fs::write(dir.join("mvnw"), "#!/bin/sh\n").expect("the wrapper is written");

        let inspection = SystemRepositoryInspector
            .inspect(dir.to_str().expect("a path"))
            .expect("inspection succeeds");
        assert!(inspection.exists);
        let maven = inspection
            .maven
            .clone()
            .expect("Maven metadata is detected");
        assert_eq!(maven.artifact_id.as_deref(), Some("auth-service"));
        assert_eq!(maven.group_id.as_deref(), Some("com.acme"));
        assert_eq!(maven.java_version.as_deref(), Some("21"));
        assert!(maven.wrapper_available);
        assert!(maven.spring_boot);
        assert_eq!(
            inspection
                .operational_build_system()
                .map(|s| s.system.as_str()),
            Some("maven")
        );
        assert!(inspection
            .integrations
            .iter()
            .any(|integration| integration.kind == "spring_boot"));
    }

    #[test]
    fn an_npm_react_project_is_detected() {
        let dir = temp_dir("npm");
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"web-ui","version":"2.0.0",
                "engines":{"node":">=20"},
                "scripts":{"build":"vite build","dev":"vite"},
                "dependencies":{"react":"^18.0.0"}}"#,
        )
        .expect("the manifest is written");
        std::fs::write(dir.join(".nvmrc"), "20.x\n").expect("the nvmrc is written");

        let inspection = SystemRepositoryInspector
            .inspect(dir.to_str().expect("a path"))
            .expect("inspection succeeds");
        let node = inspection.node.clone().expect("Node metadata is detected");
        assert_eq!(node.name.as_deref(), Some("web-ui"));
        assert_eq!(node.required_node_version.as_deref(), Some("20.x"));
        assert!(node.scripts.contains(&"build".to_owned()));
        assert!(node.react);
        assert_eq!(
            inspection
                .operational_build_system()
                .map(|s| s.system.as_str()),
            Some("npm")
        );
    }

    #[test]
    fn gradle_is_detected_but_not_operational() {
        let dir = temp_dir("gradle");
        std::fs::write(dir.join("build.gradle.kts"), "plugins {}\n")
            .expect("the script is written");

        let inspection = SystemRepositoryInspector
            .inspect(dir.to_str().expect("a path"))
            .expect("inspection succeeds");
        let gradle = inspection
            .build_systems
            .iter()
            .find(|system| system.system == "gradle")
            .expect("Gradle is detected");
        assert_eq!(
            gradle.availability,
            ProviderAvailability::ProviderNotInstalled
        );
        assert!(inspection.operational_build_system().is_none());
        assert!(gradle
            .note
            .as_deref()
            .is_some_and(|note| note.contains("not installed")));
    }

    #[test]
    fn git_branch_and_origin_are_read_from_the_git_directory() {
        let dir = temp_dir("git");
        let git = dir.join(".git");
        std::fs::create_dir_all(&git).expect("the git dir is created");
        std::fs::write(git.join("HEAD"), "ref: refs/heads/feature/widget\n")
            .expect("HEAD is written");
        std::fs::write(
            git.join("config"),
            "[remote \"origin\"]\n\turl = git@example.com:acme/widget.git\n",
        )
        .expect("config is written");

        let inspection = SystemRepositoryInspector
            .inspect(dir.to_str().expect("a path"))
            .expect("inspection succeeds");
        assert!(inspection.is_git_repository);
        assert_eq!(inspection.branch.as_deref(), Some("feature/widget"));
        assert_eq!(
            inspection.remote.as_deref(),
            Some("git@example.com:acme/widget.git")
        );
    }

    #[test]
    fn tag_helpers_extract_values_boundedly() {
        let xml = "<project><modules><module>a</module><module>b</module></modules></project>";
        assert_eq!(
            all_tags(xml, "module"),
            vec!["a".to_owned(), "b".to_owned()]
        );
        assert_eq!(first_tag(xml, "module").as_deref(), Some("a"));
        assert_eq!(first_tag(xml, "missing"), None);
    }
}
