//! The Java service runtime.
//!
//! Services in this ecosystem are Spring Boot applications built with Maven and
//! started through a configured command — typically `./mvnw spring-boot:run` for
//! development or `java -jar …` for a packaged artifact. This runtime never
//! hard-codes either: the start command comes from validated configuration
//! (`tasks/010.md` §8). Its job is to declare the runtime kind, validate that the
//! service can run, and turn the configured command into a [`StartPlan`].
//!
//! Whether a compatible JDK is installed is an environment question answered by
//! the manager through a [`RuntimeInspector`](crate::RuntimeInspector), not here.

use service_orchestrator_application::ServiceRuntimeError;
use service_orchestrator_domain::{AbsolutePath, Environment, Service};

use crate::runtime::{plan_from_service, validate_service, RuntimeType, ServiceRuntime, StartPlan};

/// The Java service runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct JavaRuntime;

impl JavaRuntime {
    /// Creates a Java runtime.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ServiceRuntime for JavaRuntime {
    fn runtime_type(&self) -> RuntimeType {
        RuntimeType::Java
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
        plan_from_service(RuntimeType::Java, service, working_dir, environment)
    }
}
