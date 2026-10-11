//! The Node.js service runtime.
//!
//! Front-end and Node services are started through a configured command —
//! typically `npm run start` or a package-script equivalent. As with Java, this
//! runtime never hard-codes the command; it maps the validated configuration into
//! a [`StartPlan`] and declares the runtime kind (`tasks/010.md` §8).
//!
//! Resolving `node` across version managers (`nvm`, `fnm`, `volta`) and verifying
//! the installed version is the manager's job, through the
//! [`RuntimeHost`](crate::RuntimeHost) and [`RuntimeInspector`](crate::RuntimeInspector)
//! seams (`tasks/010.md` §12).

use service_orchestrator_application::ServiceRuntimeError;
use service_orchestrator_domain::{AbsolutePath, Environment, Service};

use crate::runtime::{plan_from_service, validate_service, RuntimeType, ServiceRuntime, StartPlan};

/// The Node.js service runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeRuntime;

impl NodeRuntime {
    /// Creates a Node runtime.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ServiceRuntime for NodeRuntime {
    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Node
    }

    fn validate(&self, service: &Service) -> Result<(), ServiceRuntimeError> {
        validate_service(service)
    }

    fn prepare_start(
        &self,
        service: &Service,
        working_dir: &AbsolutePath,
        environment: Environment,
    ) -> Result<StartPlan, ServiceRuntimeError> {
        plan_from_service(RuntimeType::Node, service, working_dir, environment)
    }
}
