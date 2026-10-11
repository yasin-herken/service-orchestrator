//! The artifact-verification task executor.
//!
//! [`ArtifactVerificationExecutor`] adapts the `ArtifactVerification` task to the
//! task engine. It is deliberately thin: it decodes the
//! [`ResolvedArtifact`] from the task's parameters, asks the
//! [`ArtifactInspector`] whether the artifact is present, and maps the structured
//! answer onto a [`TaskOutcome`] or [`TaskExecutorError`].
//!
//! A missing artifact is a **task failure**, not an inspection success: the
//! `PrepareLibrary` workflow must stop before a consumer is built against an
//! artifact that was never installed. The failure carries the stable code
//! `ARTIFACT_NOT_FOUND` (`tasks/009.md` §19, §24).
//!
//! [`register_library_executors`] wires the executor into an
//! [`ExecutorRegistry`]; the composition root calls it once at startup with the
//! same [`ArtifactInspector`] the application uses.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use service_orchestrator_application::{
    ArtifactInspectionError, ArtifactInspector, ArtifactPresence, ResolvedArtifact,
};
use service_orchestrator_domain::TaskKind;
use service_orchestrator_execution::{
    ExecutorRegistry, TaskContext, TaskExecutor, TaskExecutorError, TaskOutcome,
};

/// The stable code for a verified-but-missing artifact.
const ARTIFACT_NOT_FOUND: &str = "ARTIFACT_NOT_FOUND";

/// Verifies an artifact's presence on behalf of the task engine.
pub struct ArtifactVerificationExecutor {
    inspector: Arc<dyn ArtifactInspector>,
}

impl ArtifactVerificationExecutor {
    /// Creates an executor over an artifact inspector.
    #[must_use]
    pub fn new(inspector: Arc<dyn ArtifactInspector>) -> Self {
        Self { inspector }
    }

    /// Verifies the artifact encoded by `parameters`.
    ///
    /// This holds all of the executor's behaviour; the [`TaskExecutor`]
    /// implementation only adds the progress report the engine's context
    /// provides.
    ///
    /// # Errors
    ///
    /// Returns a structured failure when the parameters are malformed, the
    /// inspector reports the artifact missing, or inspection itself fails.
    pub async fn verify(
        &self,
        parameters: &BTreeMap<String, String>,
    ) -> Result<TaskOutcome, TaskExecutorError> {
        let artifact = ResolvedArtifact::from_parameters(parameters).map_err(to_executor_error)?;
        self.verify_artifact(&artifact).await
    }

    /// Verifies a decoded artifact.
    async fn verify_artifact(
        &self,
        artifact: &ResolvedArtifact,
    ) -> Result<TaskOutcome, TaskExecutorError> {
        match self.inspector.locate(artifact).await {
            Ok(ArtifactPresence::Found { path, size_bytes }) => {
                let mut outcome = TaskOutcome::succeeded(Some(format!(
                    "artifact '{}' is present",
                    artifact.coordinate
                )))
                .with_metadata("library", artifact.library.to_string())
                .with_metadata("coordinate", artifact.coordinate.to_string())
                .with_metadata("path", path.to_string());
                if let Some(size) = size_bytes {
                    outcome = outcome.with_metadata("size_bytes", size.to_string());
                }
                Ok(outcome)
            }
            Ok(ArtifactPresence::Missing { expected_path }) => {
                let message = match expected_path {
                    Some(path) => format!(
                        "artifact '{}' was not found in the local repository at '{path}'",
                        artifact.coordinate
                    ),
                    None => format!(
                        "artifact '{}' was not found in the local repository",
                        artifact.coordinate
                    ),
                };
                Err(TaskExecutorError::failed(ARTIFACT_NOT_FOUND, message))
            }
            Err(error) => Err(TaskExecutorError::failed(error.code(), error.to_string())),
        }
    }
}

#[async_trait]
impl TaskExecutor for ArtifactVerificationExecutor {
    async fn execute(&self, context: TaskContext) -> Result<TaskOutcome, TaskExecutorError> {
        let artifact =
            ResolvedArtifact::from_parameters(context.parameters()).map_err(to_executor_error)?;
        let _ = context
            .progress()
            .message(format!("verify artifact '{}'", artifact.coordinate));
        self.verify_artifact(&artifact).await
    }
}

impl std::fmt::Debug for ArtifactVerificationExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactVerificationExecutor")
            .finish_non_exhaustive()
    }
}

/// Registers the artifact-verification executor for the `ArtifactVerification`
/// task kind.
///
/// The composition root calls this once, passing the same registry it hands to
/// the engine and the same inspector it wires into the application.
pub fn register_library_executors(
    registry: &mut ExecutorRegistry,
    inspector: Arc<dyn ArtifactInspector>,
) {
    let executor: Arc<dyn TaskExecutor> = Arc::new(ArtifactVerificationExecutor::new(inspector));
    registry.register(TaskKind::ArtifactVerification, executor);
}

/// Maps a decode failure onto the engine's error vocabulary.
fn to_executor_error(error: ArtifactInspectionError) -> TaskExecutorError {
    match error {
        ArtifactInspectionError::MissingParameter { .. }
        | ArtifactInspectionError::InvalidParameter { .. } => {
            TaskExecutorError::internal(error.to_string())
        }
        other => TaskExecutorError::failed(other.code(), other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::{DependencyVersionInspection, ResolvedDependency};
    use service_orchestrator_domain::{
        AbsolutePath, ArtifactCoordinate, LibraryId, Version, WorkspaceId,
    };

    /// An inspector whose answer the test controls.
    struct FakeInspector {
        presence: ArtifactPresence,
        unavailable: bool,
    }

    #[async_trait]
    impl ArtifactInspector for FakeInspector {
        async fn locate(
            &self,
            _artifact: &ResolvedArtifact,
        ) -> Result<ArtifactPresence, ArtifactInspectionError> {
            if self.unavailable {
                return Err(ArtifactInspectionError::Unavailable {
                    message: "no inspector".to_owned(),
                });
            }
            Ok(self.presence.clone())
        }

        async fn dependency_version(
            &self,
            _dependency: &ResolvedDependency,
        ) -> Result<DependencyVersionInspection, ArtifactInspectionError> {
            Ok(DependencyVersionInspection::NotDeclared)
        }
    }

    fn artifact() -> ResolvedArtifact {
        ResolvedArtifact {
            workspace_id: WorkspaceId::new("acme").unwrap(),
            library: LibraryId::new("core").unwrap(),
            path: AbsolutePath::new("/w/acme/core").unwrap(),
            coordinate: ArtifactCoordinate::new("com.acme", "core", Version::new("1.8.0").unwrap())
                .unwrap(),
        }
    }

    fn executor(presence: ArtifactPresence, unavailable: bool) -> ArtifactVerificationExecutor {
        ArtifactVerificationExecutor::new(Arc::new(FakeInspector {
            presence,
            unavailable,
        }))
    }

    #[test]
    fn parameter_failures_are_internal() {
        let error = ArtifactInspectionError::MissingParameter {
            key: "artifact.path".to_owned(),
        };
        assert!(matches!(
            to_executor_error(error),
            TaskExecutorError::Internal { .. }
        ));
    }

    #[tokio::test]
    async fn missing_parameters_decode_to_an_internal_error() {
        let error = executor(
            ArtifactPresence::Missing {
                expected_path: None,
            },
            false,
        )
        .verify(&BTreeMap::new())
        .await
        .unwrap_err();
        assert!(matches!(error, TaskExecutorError::Internal { .. }));
    }

    #[tokio::test]
    async fn a_present_artifact_succeeds_with_metadata() {
        let outcome = executor(
            ArtifactPresence::Found {
                path: AbsolutePath::new("/r/com/acme/core/1.8.0/core-1.8.0.jar").unwrap(),
                size_bytes: Some(42),
            },
            false,
        )
        .verify(&artifact().to_parameters())
        .await
        .unwrap();
        assert!(outcome.success);
        assert_eq!(
            outcome.metadata.get("size_bytes").map(String::as_str),
            Some("42")
        );
        assert_eq!(
            outcome.metadata.get("coordinate").map(String::as_str),
            Some("com.acme:core:1.8.0")
        );
    }

    #[tokio::test]
    async fn a_missing_artifact_fails_with_a_stable_code() {
        let error = executor(
            ArtifactPresence::Missing {
                expected_path: Some(
                    AbsolutePath::new("/r/com/acme/core/1.8.0/core-1.8.0.jar").unwrap(),
                ),
            },
            false,
        )
        .verify(&artifact().to_parameters())
        .await
        .unwrap_err();
        match error {
            TaskExecutorError::Failed { code, .. } => assert_eq!(code, ARTIFACT_NOT_FOUND),
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unavailable_inspector_fails_with_its_code() {
        let error = executor(
            ArtifactPresence::Missing {
                expected_path: None,
            },
            true,
        )
        .verify(&artifact().to_parameters())
        .await
        .unwrap_err();
        match error {
            TaskExecutorError::Failed { code, .. } => {
                assert_eq!(code, "ARTIFACT_INSPECTION_UNAVAILABLE");
            }
            other => panic!("expected a structured failure, got {other:?}"),
        }
    }

    #[test]
    fn registering_the_library_executors_claims_the_verification_kind() {
        let mut registry = ExecutorRegistry::new();
        register_library_executors(
            &mut registry,
            Arc::new(FakeInspector {
                presence: ArtifactPresence::Missing {
                    expected_path: None,
                },
                unavailable: false,
            }),
        );
        assert!(registry.kinds().contains(&TaskKind::ArtifactVerification));
    }
}
