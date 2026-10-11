//! Library commands.

use service_orchestrator_domain::{BuildOperation, CommandKind, LibraryId, Permission, TaskKind};
use service_orchestrator_execution::TaskDefinition;

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::OperationResult;

impl Application {
    /// Builds a library.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LibraryNotFound`] if the library does not
    /// exist, or [`ApplicationError::InvalidOperation`] if it has no build
    /// command or no path.
    pub fn build_library(
        &self,
        library_id: &LibraryId,
    ) -> Result<OperationResult, ApplicationError> {
        self.authorize(Permission::SafeWrite)?;
        let (workspace, library) = self.locate_library(library_id)?;
        if !library.commands.contains(&CommandKind::Build) {
            return Err(ApplicationError::invalid_operation(format!(
                "library '{library_id}' has no build command configured"
            )));
        }
        let target = self.library_build_target(workspace, library, BuildOperation::Build)?;
        self.schedule(
            TaskDefinition::new(TaskKind::BuildLibrary)
                .for_library(library_id.clone())
                .with_parameters(target.to_parameters())
                .with_timeout(self.default_timeout_seconds()),
        )
    }
}
