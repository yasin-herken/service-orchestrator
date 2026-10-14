//! The versioned, structured local IPC protocol.
//!
//! The protocol is deliberately small. Every client message is a
//! [`RequestEnvelope`] carrying a stable method name and a JSON parameter
//! object; every server message is a [`ServerMessage`], either a
//! [`ResponseEnvelope`] correlated by `request_id` or a pushed [`ServerEvent`].
//!
//! Frames are length-prefixed: a big-endian `u32` byte length followed by that
//! many bytes of UTF-8 JSON. There is no shell, no command string, and no public
//! network port — the transport is a Unix domain socket owned by the daemon
//! (`tasks/015.md` §7, §8, §9).
//!
//! The protocol is versioned with [`PROTOCOL_VERSION`]. A client and daemon that
//! disagree are refused with an actionable error rather than silently
//! misinterpreted (`tasks/015.md` §25).

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use service_orchestrator_application::{ApplicationError, DiffOptions, LogEntry};
use service_orchestrator_domain::ServiceId;
use service_orchestrator_execution::TaskEvent;

/// The current IPC protocol version.
///
/// Bump this whenever the wire shape changes incompatibly. The daemon rejects a
/// request whose `protocol` differs, returning
/// [`ApplicationError::RuntimeUnavailable`] with the code
/// [`codes::PROTOCOL_INCOMPATIBLE`](crate::codes::PROTOCOL_INCOMPATIBLE).
pub const PROTOCOL_VERSION: u32 = 1;

/// The largest frame the transport will read or write, in bytes.
///
/// It bounds an untrusted request so a malformed or hostile client cannot make
/// the daemon allocate unbounded memory (`tasks/015.md` §24).
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Which interface a client is.
///
/// The daemon uses this to decide whether a request may carry an
/// `allow_destructive` flag: only an interactive (`Tui` or `Desktop`) client
/// can, and even then the request still passes through the policy engine. MCP
/// and non-interactive CLI clients can never self-confirm (`tasks/015.md` §23;
/// `tasks/021.md` §23).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    /// The Ratatui terminal interface.
    Tui,
    /// The MCP stdio server.
    Mcp,
    /// A command-line client (for example `daemon status`).
    Cli,
    /// The Tauri desktop GUI adapter.
    Desktop,
    /// An interface kind this build does not recognize.
    ///
    /// Deserialized through `#[serde(other)]` so a peer never fails to *parse*
    /// a request envelope merely because the sender declares an interface kind
    /// this build predates. The variant is deliberately non-interactive, so an
    /// unrecognized kind can never carry a destructive confirmation
    /// (`tasks/015.md` §23, §25, `tasks/031.md` §15).
    ///
    /// Without this, adding an interface kind (as `Desktop` was) is a breaking
    /// wire change: an older daemon rejects the whole envelope and drops the
    /// connection, which a newer client can only observe as a bare
    /// "the daemon connection was closed".
    #[serde(other)]
    Unknown,
}

impl ClientKind {
    /// Returns a short, stable label used in logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Mcp => "mcp",
            Self::Cli => "cli",
            Self::Desktop => "desktop",
            Self::Unknown => "unknown",
        }
    }

    /// Returns `true` if the interface is interactive and may ask for a
    /// destructive confirmation.
    #[must_use]
    pub const fn is_interactive(self) -> bool {
        matches!(self, Self::Tui | Self::Desktop)
    }
}

/// A client request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestEnvelope {
    /// The protocol version the client speaks.
    pub protocol: u32,
    /// A unique, client-generated request identifier.
    pub request_id: String,
    /// The interface making the request.
    pub client: ClientKind,
    /// Whether the client is asking to permit a destructive operation.
    ///
    /// It is honored only for an interactive client; the daemon still runs the
    /// shared policy engine (`tasks/015.md` §23).
    pub allow_destructive: bool,
    /// The stable method name, for example `start_service`.
    pub method: String,
    /// The method's parameters, as a JSON value.
    pub params: serde_json::Value,
}

/// A server response correlated to a request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// The `request_id` of the request this answers.
    pub request_id: String,
    /// The operation's structured result: `Ok` carries the serialized value,
    /// `Err` carries the structured application error.
    pub result: Result<serde_json::Value, ApplicationError>,
}

/// A message pushed from the server to a client.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerMessage {
    /// A response to a request.
    Response(ResponseEnvelope),
    /// An asynchronous event.
    Event(ServerEvent),
}

/// An asynchronous event pushed to every connected client.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// A task-engine state change.
    Task {
        /// The engine event.
        event: TaskEvent,
    },
    /// A realtime log entry for a service the client subscribed to.
    Log {
        /// The service the entry belongs to.
        service_id: ServiceId,
        /// The entry.
        entry: LogEntry,
    },
    /// The active configuration changed.
    ///
    /// Emitted after the daemon commits or reloads configuration, so every
    /// connected client — TUI, MCP, and the desktop GUI — refreshes its view
    /// and observes the same revision (`tasks/023.md` §39, §40).
    ConfigurationChanged(ConfigurationChanged),
    /// The daemon is shutting down.
    Shutdown {
        /// A human-readable reason.
        reason: String,
    },
}

/// The payload of a [`ServerEvent::ConfigurationChanged`] event.
///
/// It carries the new revision and a short, human-readable description of what
/// changed. It never carries the configuration itself: a client re-reads the
/// snapshot it needs, so there is no second cache to keep coherent
/// (`tasks/023.md` §39, §40).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationChanged {
    /// The revision of the file after the change.
    pub revision: String,
    /// A short description, for example `create service 'auth-service'`.
    pub description: String,
}

/// A serializable projection of
/// [`DiffOptions`](service_orchestrator_application::DiffOptions) for IPC.
///
/// `DiffOptions` carries a live cancellation token and a `Duration`, neither of
/// which round-trips meaningfully over the socket. The daemon owns cancellation,
/// so only the numeric fields cross the wire and the daemon rebuilds the options
/// (`tasks/015.md` §9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffParams {
    /// Whether to diff staged (`true`) or unstaged (`false`) changes.
    pub staged: bool,
    /// The maximum number of patch bytes to return.
    pub max_bytes: usize,
    /// The optional timeout, in milliseconds.
    pub timeout_millis: Option<u64>,
}

impl DiffParams {
    /// Projects diff options onto their wire form, dropping live-only fields.
    #[must_use]
    pub fn from_options(options: &DiffOptions) -> Self {
        Self {
            staged: options.staged,
            max_bytes: options.max_bytes,
            timeout_millis: options
                .timeout
                .map(|timeout| u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)),
        }
    }

    /// Rebuilds diff options on the daemon side.
    ///
    /// The cancellation token is intentionally left unset: cancellation is
    /// daemon-owned and is never supplied by a client (`tasks/015.md` §9, §22).
    /// The options are rebuilt through the builder so `max_bytes` is clamped to
    /// the application's maximum even when a client bypasses the MCP layer and
    /// sends a raw IPC request (`AGENTS.md` §35, §10).
    #[must_use]
    pub fn into_options(self) -> DiffOptions {
        let mut options = DiffOptions::new()
            .staged(self.staged)
            .with_max_bytes(self.max_bytes);
        if let Some(millis) = self.timeout_millis {
            options = options.with_timeout(std::time::Duration::from_millis(millis));
        }
        options
    }
}

/// Stable error codes exchanged over IPC.
pub mod codes {
    /// The daemon could not be reached, or the connection dropped.
    pub const DAEMON_UNAVAILABLE: &str = "DAEMON_UNAVAILABLE";
    /// The client and daemon speak incompatible protocol versions.
    pub const PROTOCOL_INCOMPATIBLE: &str = "PROTOCOL_INCOMPATIBLE";
    /// The request was malformed or exceeded the transport's limits.
    pub const INVALID_REQUEST: &str = "INVALID_REQUEST";
    /// The requested operation is not recognized by the daemon.
    pub const UNKNOWN_OPERATION: &str = "UNKNOWN_OPERATION";
}

/// Builds a [`ApplicationError::RuntimeUnavailable`] with `code`.
#[must_use]
pub fn unavailable(code: &str, message: impl Into<String>) -> ApplicationError {
    ApplicationError::RuntimeUnavailable {
        code: code.to_owned(),
        message: message.into(),
    }
}

/// Serializes `value` as a length-prefixed frame and writes it.
///
/// # Errors
///
/// Returns an [`io::Error`] when serialization fails or the write fails.
pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame exceeds the maximum size",
        ));
    }
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame is too large"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

/// Reads one length-prefixed frame and deserializes it.
///
/// Returns `Ok(None)` on a clean end of stream (the peer closed the connection
/// between frames). A truncated frame is an error.
///
/// # Errors
///
/// Returns an [`io::Error`] when the stream fails, a frame is truncated, a frame
/// exceeds [`MAX_FRAME_BYTES`], or the payload is not valid JSON for `T`.
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<Option<T>> {
    let mut length_bytes = [0u8; 4];
    match reader.read_exact(&mut length_bytes) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(length_bytes) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds the maximum size",
        ));
    }
    let mut buffer = vec![0u8; length];
    reader.read_exact(&mut buffer)?;
    let value = serde_json::from_slice(&buffer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let request = RequestEnvelope {
            protocol: PROTOCOL_VERSION,
            request_id: "req-1".to_owned(),
            client: ClientKind::Tui,
            allow_destructive: false,
            method: "list_workspaces".to_owned(),
            params: serde_json::json!({}),
        };
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &request).expect("a frame is written");
        let decoded: RequestEnvelope = read_frame(&mut buffer.as_slice())
            .expect("the frame reads")
            .expect("a frame is present");
        assert_eq!(decoded.method, "list_workspaces");
        assert_eq!(decoded.client, ClientKind::Tui);
    }

    #[test]
    fn clean_eof_reads_none() {
        let decoded: Option<RequestEnvelope> =
            read_frame(&mut std::io::empty()).expect("an empty stream is not an error");
        assert!(decoded.is_none());
    }

    #[test]
    fn a_truncated_frame_is_an_error() {
        let buffer = vec![0, 0, 0, 8, b'{'];
        let result = read_frame::<_, RequestEnvelope>(&mut buffer.as_slice());
        assert!(result.is_err());
    }

    #[test]
    fn an_oversized_frame_is_rejected() {
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&(u32::MAX).to_be_bytes());
        let result = read_frame::<_, RequestEnvelope>(&mut buffer.as_slice());
        assert!(result.is_err());
    }

    #[test]
    fn responses_round_trip_with_an_application_error() {
        let response = ResponseEnvelope {
            request_id: "req-2".to_owned(),
            result: Err(ApplicationError::ServiceNotFound(
                ServiceId::new("ghost").unwrap(),
            )),
        };
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &response).expect("a frame is written");
        let decoded: ResponseEnvelope = read_frame(&mut buffer.as_slice())
            .expect("the frame reads")
            .expect("a frame is present");
        assert_eq!(decoded.request_id, "req-2");
        assert!(matches!(
            decoded.result,
            Err(ApplicationError::ServiceNotFound(_))
        ));
    }

    #[test]
    fn client_kinds_are_labeled_and_only_interactive_interfaces_confirm() {
        assert_eq!(ClientKind::Tui.label(), "tui");
        assert_eq!(ClientKind::Mcp.label(), "mcp");
        assert_eq!(ClientKind::Cli.label(), "cli");
        assert_eq!(ClientKind::Desktop.label(), "desktop");
        assert_eq!(ClientKind::Unknown.label(), "unknown");
        assert!(ClientKind::Tui.is_interactive());
        assert!(ClientKind::Desktop.is_interactive());
        assert!(!ClientKind::Mcp.is_interactive());
        assert!(!ClientKind::Cli.is_interactive());
        assert!(
            !ClientKind::Unknown.is_interactive(),
            "an unrecognized kind can never self-confirm a destructive operation"
        );
    }

    #[test]
    fn an_unknown_client_kind_still_parses_the_envelope() {
        // A newer peer may declare an interface kind this build predates. The
        // envelope must still parse (as `Unknown`, non-interactive) rather than
        // fail and drop the connection (`tasks/031.md` §15).
        let value = serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "request_id": "req-unknown",
            "client": "holo-deck",
            "allow_destructive": true,
            "method": "daemon_status",
            "params": [],
        });
        let decoded: RequestEnvelope =
            serde_json::from_value(value).expect("an unknown kind still parses");
        assert_eq!(decoded.client, ClientKind::Unknown);
        assert!(!decoded.client.is_interactive());
    }

    #[test]
    fn the_desktop_kind_serializes_as_a_stable_wire_label() {
        let request = RequestEnvelope {
            protocol: PROTOCOL_VERSION,
            request_id: "req-5".to_owned(),
            client: ClientKind::Desktop,
            allow_destructive: true,
            method: "apply_migration".to_owned(),
            params: serde_json::json!(["orders-api"]),
        };
        let value = serde_json::to_value(&request).expect("a request serializes");
        assert_eq!(value["client"], serde_json::json!("desktop"));
        let decoded: RequestEnvelope =
            serde_json::from_value(value).expect("the request round-trips");
        assert_eq!(decoded.client, ClientKind::Desktop);
        assert!(decoded.client.is_interactive());
    }

    #[test]
    fn diff_params_drop_live_only_fields() {
        let options = DiffOptions {
            staged: true,
            max_bytes: 4096,
            timeout: Some(std::time::Duration::from_millis(2500)),
            cancellation: Some(service_orchestrator_execution::Cancellation::new()),
        };
        let params = DiffParams::from_options(&options);
        assert!(params.staged);
        assert_eq!(params.max_bytes, 4096);
        assert_eq!(params.timeout_millis, Some(2500));

        let rebuilt = params.into_options();
        assert!(
            rebuilt.cancellation.is_none(),
            "cancellation is daemon-owned"
        );
        assert_eq!(rebuilt.max_bytes, 4096);
    }

    #[test]
    fn an_oversized_diff_request_is_clamped_when_rebuilt() {
        // A client that bypasses the MCP layer and sends a raw IPC request
        // cannot ask for an unbounded patch.
        let params = DiffParams {
            staged: false,
            max_bytes: usize::MAX,
            timeout_millis: None,
        };
        let rebuilt = params.into_options();
        assert_eq!(
            rebuilt.max_bytes,
            service_orchestrator_application::MAX_DIFF_BYTES
        );
    }

    #[test]
    fn server_events_are_tagged_by_event_type() {
        let event = ServerEvent::Shutdown {
            reason: "test".to_owned(),
        };
        let value = serde_json::to_value(&event).expect("an event serializes");
        assert_eq!(value["event_type"], serde_json::json!("shutdown"));

        let message = ServerMessage::Event(event);
        let value = serde_json::to_value(&message).expect("a message serializes");
        assert_eq!(value["kind"], serde_json::json!("event"));
    }

    #[test]
    fn configuration_changed_events_are_tagged_and_carry_a_revision() {
        let event = ServerEvent::ConfigurationChanged(ConfigurationChanged {
            revision: "abc123".to_owned(),
            description: "create service 'auth-service'".to_owned(),
        });
        let value = serde_json::to_value(&event).expect("an event serializes");
        assert_eq!(
            value["event_type"],
            serde_json::json!("configuration_changed")
        );
        assert_eq!(value["revision"], serde_json::json!("abc123"));
        let decoded: ServerEvent = serde_json::from_value(value).expect("the event round-trips");
        assert!(matches!(
            decoded,
            ServerEvent::ConfigurationChanged(ConfigurationChanged { revision, .. })
                if revision == "abc123"
        ));
    }

    #[test]
    fn runtime_unavailable_code_crosses_the_wire() {
        let error = unavailable(codes::PROTOCOL_INCOMPATIBLE, "version mismatch");
        let response = ResponseEnvelope {
            request_id: "req-3".to_owned(),
            result: Err(error),
        };
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &response).expect("a frame is written");
        let decoded: ResponseEnvelope = read_frame(&mut buffer.as_slice())
            .expect("the frame reads")
            .expect("a frame is present");
        match decoded.result {
            Err(ApplicationError::RuntimeUnavailable { code, message }) => {
                assert_eq!(code, codes::PROTOCOL_INCOMPATIBLE);
                assert_eq!(message, "version mismatch");
            }
            other => panic!("expected a runtime-unavailable error, got {other:?}"),
        }
    }

    #[test]
    fn request_envelope_carries_the_routing_fields() {
        let request = RequestEnvelope {
            protocol: PROTOCOL_VERSION,
            request_id: "req-4".to_owned(),
            client: ClientKind::Mcp,
            allow_destructive: true,
            method: "start_service".to_owned(),
            params: serde_json::json!(["auth-service"]),
        };
        let value = serde_json::to_value(&request).expect("a request serializes");
        assert_eq!(value["protocol"], serde_json::json!(PROTOCOL_VERSION));
        assert_eq!(value["client"], serde_json::json!("mcp"));
        assert_eq!(value["allow_destructive"], serde_json::json!(true));
        assert_eq!(value["method"], serde_json::json!("start_service"));
        assert_eq!(value["params"], serde_json::json!(["auth-service"]));
    }
}
