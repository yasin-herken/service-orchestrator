//! The process health provider.
//!
//! A process check answers exactly one question: is the service's **managed**
//! process alive? It reads the [`ServiceRuntimeIndex`] the runtime adapter
//! implements, which tracks only processes Service Orchestrator started
//! (`tasks/012.md` §7). It never scans the operating system for arbitrary
//! processes, and it never treats an unrelated process using the same port as a
//! managed service.

use std::sync::Arc;

use async_trait::async_trait;
use service_orchestrator_application::ServiceRuntimeIndex;

use crate::checker::{CheckResult, CheckType, HealthCheckContext, HealthChecker};
use crate::error::HealthError;

/// The process provider, backed by the runtime adapter's managed-process index.
pub struct ProcessHealthChecker {
    runtime: Arc<dyn ServiceRuntimeIndex>,
}

impl ProcessHealthChecker {
    /// Creates a process provider over the managed-process index.
    #[must_use]
    pub fn new(runtime: Arc<dyn ServiceRuntimeIndex>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl HealthChecker for ProcessHealthChecker {
    fn check_type(&self) -> CheckType {
        CheckType::Process
    }

    async fn check(&self, context: &HealthCheckContext<'_>) -> Result<CheckResult, HealthError> {
        match self.runtime.runtime_state(context.service_id) {
            Some(info) if info.state.is_running() => {
                Ok(CheckResult::healthy(None).with_message("the managed process is running"))
            }
            Some(info) => Ok(CheckResult::unhealthy(
                None,
                format!("the managed process is {}", info.state),
            )),
            None => Ok(CheckResult::unhealthy(
                None,
                "the service is not managed by the process manager",
            )),
        }
    }
}

impl std::fmt::Debug for ProcessHealthChecker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHealthChecker")
            .finish_non_exhaustive()
    }
}
