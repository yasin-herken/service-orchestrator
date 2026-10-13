//! Re-export of the shared domain redaction rule.
//!
//! The "secrets must never be exposed" rule is a domain invariant
//! (`AGENTS.md` §31) reused by the log hub, the build and migration adapters,
//! and the process description. The canonical implementation lives in
//! [`service_orchestrator_domain::redact`]; this module keeps the historical
//! `service_orchestrator_logging::redact` path working for existing callers.

pub use service_orchestrator_domain::redact::{is_clean, redact};
