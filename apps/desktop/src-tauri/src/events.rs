//! Event names and payload mapping for events pushed to the frontend.
//!
//! The adapter forwards two event streams into the webview
//! (`tasks/018.md` §12): daemon connection transitions (the adapter's own
//! state machine) and task-engine events observed on the client's
//! subscription. Payload shaping lives here as pure functions so it is unit
//! testable without a webview.

use serde_json::{json, Value};
use service_orchestrator_execution::TaskEvent;

/// Event name carrying a [`ConnectionSnapshot`](crate::connection::ConnectionSnapshot).
pub const CONNECTION_EVENT: &str = "daemon://connection";

/// Event name carrying a task-engine event payload.
///
/// The payload keeps the daemon's externally tagged wire shape
/// (`{"TaskCreated": { … }}`), plus a synthetic `Lagged` variant when the
/// local subscription fell behind.
pub const TASK_EVENT: &str = "daemon://task";

/// Maps a task event onto its JSON payload.
///
/// Returns [`None`] only if serialization fails, which would indicate the
/// event shape drifted from its declared wire contract; the forwarder skips
/// such an event rather than emitting malformed data.
pub fn task_event_payload(event: &TaskEvent) -> Option<Value> {
    serde_json::to_value(event).ok()
}

/// Builds the synthetic payload emitted when the subscription lagged.
///
/// Dropping events is visible to the frontend instead of silently shrinking
/// the activity feed (`tasks/018.md` §22).
pub fn lagged_payload(dropped: u64) -> Value {
    json!({ "Lagged": { "dropped": dropped } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_domain::TaskId;

    #[test]
    fn task_events_keep_their_externally_tagged_shape() {
        let event = TaskEvent::TaskCancelled {
            id: TaskId::new("task-1").expect("a valid task id"),
        };
        let payload = task_event_payload(&event).expect("the event serializes");
        assert_eq!(payload, json!({ "TaskCancelled": { "id": "task-1" } }));
    }

    #[test]
    fn lagged_events_are_reported_as_drops() {
        assert_eq!(lagged_payload(7), json!({ "Lagged": { "dropped": 7 } }));
    }

    #[test]
    fn event_names_match_the_frontend_contract() {
        assert_eq!(CONNECTION_EVENT, "daemon://connection");
        assert_eq!(TASK_EVENT, "daemon://task");
    }
}
