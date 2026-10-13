//! Structured command errors surfaced to the frontend.
//!
//! Tauri commands return [`CommandError`], which serializes to the exact
//! `{ code, message }` shape the typed desktop API declares. Codes are stable
//! and machine-readable; messages are human-readable and never contain stack
//! traces or secrets (`tasks/018.md` §20).

use std::fmt;

use serde::Serialize;
use service_orchestrator_application::ApplicationError;

/// The structured error returned by every command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommandError {
    /// Stable machine-readable code — the daemon's IPC code (for example
    /// `DAEMON_UNAVAILABLE` or `PROTOCOL_INCOMPATIBLE`), the application
    /// error's variant name (for example `ServiceNotFound`), or one of the
    /// adapter codes `INVALID_INPUT`, `NOT_FOUND`, and `INTERNAL`.
    pub code: String,
    /// Human-readable, user-safe message.
    pub message: String,
}

impl CommandError {
    /// Creates an error with an explicit code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Creates an `INVALID_INPUT` error for input the domain rejected.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("INVALID_INPUT", message)
    }

    /// Creates an `INTERNAL` error for adapter-side failures.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("INTERNAL", message)
    }

    /// Appends an actionable hint to the message, keeping the code.
    #[must_use]
    pub fn with_hint(self, hint: impl AsRef<str>) -> Self {
        Self {
            code: self.code,
            message: format!("{} ({})", self.message, hint.as_ref()),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

impl From<ApplicationError> for CommandError {
    fn from(error: ApplicationError) -> Self {
        // Errors that already carry a stable wire code promote it so the
        // frontend branches on the specific condition (DAEMON_UNAVAILABLE,
        // CONFIG_CONFLICT, CONFIG_ENTITY_NOT_FOUND, INSPECTION_FAILED, …)
        // rather than an outer variant name.
        match &error {
            ApplicationError::RuntimeUnavailable { code, message }
            | ApplicationError::Configuration { code, message }
            | ApplicationError::RepositoryInspection { code, message } => {
                return Self::new(code.clone(), message.clone());
            }
            _ => {}
        }
        // Everything else maps to its externally tagged variant name, which is
        // stable, specific, and requires no hand-maintained match arms.
        let code = variant_code(&error);
        Self::new(code, error.to_string())
    }
}

/// Extracts the externally tagged variant name of an application error.
fn variant_code(error: &ApplicationError) -> String {
    match serde_json::to_value(error) {
        Ok(serde_json::Value::Object(map)) if !map.is_empty() => map
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "INTERNAL".to_owned()),
        // A unit variant serializes to its bare name.
        Ok(serde_json::Value::String(name)) => name,
        _ => "INTERNAL".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_client::codes;

    #[test]
    fn runtime_unavailable_promotes_its_wire_code() {
        let error = ApplicationError::RuntimeUnavailable {
            code: codes::DAEMON_UNAVAILABLE.to_owned(),
            message: "the daemon is not running".to_owned(),
        };
        let command: CommandError = error.into();
        assert_eq!(command.code, codes::DAEMON_UNAVAILABLE);
        assert_eq!(command.message, "the daemon is not running");
    }

    #[test]
    fn protocol_incompatibility_keeps_its_code() {
        let error = ApplicationError::RuntimeUnavailable {
            code: codes::PROTOCOL_INCOMPATIBLE.to_owned(),
            message: "protocol mismatch".to_owned(),
        };
        let command: CommandError = error.into();
        assert_eq!(command.code, codes::PROTOCOL_INCOMPATIBLE);
    }

    #[test]
    fn other_application_errors_use_their_variant_name() {
        let service =
            service_orchestrator_domain::ServiceId::new("orders-api").expect("a valid service id");
        let command: CommandError = ApplicationError::ServiceNotFound(service).into();
        assert_eq!(command.code, "ServiceNotFound");
        assert!(!command.message.is_empty());
        assert!(!command.message.contains("stack"), "no internals leak");
    }

    #[test]
    fn configuration_codes_promote_their_specific_code() {
        let error = ApplicationError::Configuration {
            code: "CONFIG_CONFLICT".to_owned(),
            message: "the file changed".to_owned(),
        };
        let command: CommandError = error.into();
        assert_eq!(command.code, "CONFIG_CONFLICT");

        let inspection = ApplicationError::RepositoryInspection {
            code: "INSPECTION_FAILED".to_owned(),
            message: "unreadable".to_owned(),
        };
        assert_eq!(CommandError::from(inspection).code, "INSPECTION_FAILED");
    }

    #[test]
    fn adapter_codes_are_stable() {
        assert_eq!(CommandError::invalid_input("bad id").code, "INVALID_INPUT");
        assert_eq!(CommandError::internal("boom").code, "INTERNAL");
        assert_eq!(
            CommandError::new("NOT_FOUND", "gone")
                .with_hint("see the log")
                .message,
            "gone (see the log)"
        );
    }

    #[test]
    fn errors_render_readably() {
        assert_eq!(CommandError::new("X", "y").to_string(), "[X] y");
    }
}
