//! Read-only build queries.
//!
//! The application never runs a build tool. This query only projects the build
//! adapter's provider registry so an interface can render which operations a
//! service's configured build system supports — and disable the rest — without
//! embedding provider knowledge (`tasks/020.md` §12, §13). It performs no build
//! and mutates nothing.

use service_orchestrator_domain::{Permission, ServiceId};

use crate::app::Application;
use crate::error::ApplicationError;
use crate::models::BuildCapabilities;

impl Application {
    /// Returns the operations the service's configured build system supports.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::ServiceNotFound`] if the service does not
    /// exist, and [`ApplicationError::Build`] when the service declares no build
    /// system or no provider is registered for the one it declares.
    pub fn get_build_capabilities(
        &self,
        service_id: &ServiceId,
    ) -> Result<BuildCapabilities, ApplicationError> {
        self.authorize(Permission::Read)?;
        let (_workspace, service) = self.locate_service(service_id)?;
        let Some(system) = service.build_system else {
            return Err(ApplicationError::Build {
                code: "BUILD_SYSTEM_UNKNOWN".to_owned(),
                message: format!("service '{service_id}' has no configured build system"),
            });
        };
        self.build_capability_index()
            .capabilities(system)
            .ok_or_else(|| ApplicationError::Build {
                code: "BUILD_UNSUPPORTED_SYSTEM".to_owned(),
                message: format!("no build provider is registered for build system '{system}'"),
            })
    }
}
