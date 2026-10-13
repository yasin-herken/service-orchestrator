//! Per-service fan-out of the daemon's realtime log streams.
//!
//! The shared IPC client allows exactly one log subscription per service
//! (`RemoteClient` keeps a single sender per service id), while the Logs
//! screen may hold several frontend subscriptions at once — for example a
//! followed service and a task-log pane. This module multiplexes: one reader
//! task per subscribed service owns the single IPC subscription and forwards
//! every entry to each frontend subscription of that service
//! (`tasks/021.md` §24, §28).
//!
//! # Lifecycle safety
//!
//! Dropping the IPC subscription sends the daemon `unsubscribe_service_logs`
//! and removes the client's per-service sender, and the daemon itself treats a
//! subscribe for an already-subscribed service as a no-op. A new subscribe for
//! a service must therefore never interleave with the old subscription's
//! teardown. This is guaranteed by three rules, all taken under the same lock
//! order (per-service lifecycle lock, then fan-out state lock):
//!
//! 1. every teardown (dropping the IPC subscription, notifying subscribers,
//!    publishing the `done` signal) happens under both locks;
//! 2. a subscribe that finds a stopping channel waits for `done` before
//!    opening a fresh IPC subscription;
//! 3. the last unsubscribe only signals the reader — it never removes the
//!    channel — so a stopping channel is always visible to a racing subscribe
//!    (`tasks/021.md` §24).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use service_orchestrator_application::{LogStreamError, LogSubscription};
use service_orchestrator_client::{OrchestratorClient, RemoteClient};
use tauri::{AppHandle, Emitter};

use crate::dto::LogStreamEvent;
use crate::error::CommandError;
use crate::events::LOG_EVENT;
use crate::validation::parse_service_id;

/// One IPC log stream fanned out to every frontend subscription of a service.
struct ServiceChannel {
    /// The frontend subscription ids currently attached.
    subscribers: HashSet<String>,
    /// Signals the reader task to stop; the last unsubscribe triggers it.
    stop: tokio::sync::watch::Sender<bool>,
    /// Flips to `true` once the reader's teardown (including the daemon
    /// unsubscribe) has completed, so a re-subscribe can wait for it.
    done: Arc<tokio::sync::watch::Sender<bool>>,
    /// Whether the reader has been asked to stop.
    stopping: bool,
}

#[derive(Default)]
struct FanoutState {
    /// The channel of each subscribed service (live or stopping).
    channels: HashMap<String, ServiceChannel>,
    /// Which service each frontend subscription is attached to.
    subscriptions: HashMap<String, String>,
    /// Per-service teardown/subscribe serialization.
    lifecycles: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

/// The adapter-side log multiplexer shared by the subscribe/unsubscribe
/// commands.
#[derive(Default)]
pub struct LogFanout {
    state: tokio::sync::Mutex<FanoutState>,
}

impl LogFanout {
    /// Attaches a frontend subscription to a service's realtime log stream.
    ///
    /// The first subscriber opens the single IPC subscription; later
    /// subscribers attach to the existing reader. Each event is emitted as a
    /// `daemon://log` payload tagged with the frontend's subscription id, so
    /// listeners filter their own stream and never receive a neighbour's
    /// entries (`tasks/021.md` §24, §34).
    ///
    /// # Errors
    ///
    /// Returns `INVALID_INPUT` for a rejected service id or an empty/duplicate
    /// subscription id, or the daemon's structured error when the IPC
    /// subscription fails.
    pub async fn subscribe(
        self: &Arc<Self>,
        app: AppHandle,
        client: Arc<RemoteClient>,
        subscription_id: String,
        service_id: String,
    ) -> Result<(), CommandError> {
        let service = parse_service_id(&service_id)?;
        if subscription_id.trim().is_empty() {
            return Err(CommandError::invalid_input(
                "log subscription id must not be empty",
            ));
        }

        /// What the state scan decided to do next.
        enum Next {
            /// Attached to a live channel.
            Attached,
            /// A previous reader is still tearing down; wait for its `done`.
            Wait(Arc<tokio::sync::watch::Sender<bool>>),
            /// No channel exists; open the IPC subscription under both locks.
            Open(Arc<tokio::sync::Mutex<()>>),
        }

        loop {
            let next = {
                let mut state = self.state.lock().await;
                if state.subscriptions.contains_key(&subscription_id) {
                    return Err(CommandError::invalid_input(format!(
                        "log subscription '{subscription_id}' is already active"
                    )));
                }
                match state.channels.get_mut(&service_id) {
                    Some(channel) if !channel.stopping => {
                        channel.subscribers.insert(subscription_id.clone());
                        state
                            .subscriptions
                            .insert(subscription_id.clone(), service_id.clone());
                        Next::Attached
                    }
                    Some(channel) => Next::Wait(Arc::clone(&channel.done)),
                    None => {
                        let lifecycle = state
                            .lifecycles
                            .entry(service_id.clone())
                            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                            .clone();
                        Next::Open(lifecycle)
                    }
                }
            };

            match next {
                Next::Attached => return Ok(()),
                Next::Wait(done) => {
                    // Wait outside every lock: the reader's teardown needs the
                    // lifecycle and state locks to publish `done`.
                    await_done(done).await;
                }
                Next::Open(lifecycle) => {
                    // Serialize with any in-flight teardown of this service.
                    let guard = lifecycle.lock().await;
                    let wait_done = {
                        let mut state = self.state.lock().await;
                        if state.subscriptions.contains_key(&subscription_id) {
                            return Err(CommandError::invalid_input(format!(
                                "log subscription '{subscription_id}' is already active"
                            )));
                        }
                        match state.channels.get_mut(&service_id) {
                            Some(channel) if !channel.stopping => {
                                channel.subscribers.insert(subscription_id.clone());
                                state
                                    .subscriptions
                                    .insert(subscription_id.clone(), service_id.clone());
                                None
                            }
                            Some(channel) => Some(Arc::clone(&channel.done)),
                            None => {
                                let subscribe_client = Arc::clone(&client);
                                let target = service.clone();
                                let subscription =
                                    tauri::async_runtime::spawn_blocking(move || {
                                        subscribe_client.subscribe_service_logs(&target)
                                    })
                                    .await
                                    .map_err(|error| {
                                        CommandError::internal(format!(
                                            "log subscription task failed: {error}"
                                        ))
                                    })?
                                    .map_err(CommandError::from)?;

                                let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
                                let (done_tx, _) = tokio::sync::watch::channel(false);
                                state.channels.insert(
                                    service_id.clone(),
                                    ServiceChannel {
                                        subscribers: HashSet::from([subscription_id.clone()]),
                                        stop: stop_tx,
                                        done: Arc::new(done_tx),
                                        stopping: false,
                                    },
                                );
                                state
                                    .subscriptions
                                    .insert(subscription_id.clone(), service_id.clone());
                                drop(state);

                                tokio::spawn(run_reader(
                                    Arc::clone(self),
                                    app.clone(),
                                    service_id.clone(),
                                    subscription,
                                    stop_rx,
                                    Arc::clone(&lifecycle),
                                ));
                                None
                            }
                        }
                    };
                    if let Some(done) = wait_done {
                        // A teardown started while we waited for the lock; it
                        // needs the lock to finish, so release it first.
                        drop(guard);
                        await_done(done).await;
                        continue;
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Detaches a frontend subscription; the last one asks the service's
    /// reader to stop, and the reader's teardown notifies the daemon.
    ///
    /// Unknown subscription ids are ignored, so cleanup on unmount is
    /// idempotent (`tasks/021.md` §24, §34).
    pub async fn unsubscribe(&self, subscription_id: &str) {
        let mut state = self.state.lock().await;
        let Some(service_id) = state.subscriptions.remove(subscription_id) else {
            return;
        };
        if let Some(channel) = state.channels.get_mut(&service_id) {
            channel.subscribers.remove(subscription_id);
            if channel.subscribers.is_empty() && !channel.stopping {
                channel.stopping = true;
                let _ = channel.stop.send(true);
            }
        }
    }
}

/// Waits until a channel's teardown publishes `done`.
///
/// If the sender disappears without publishing (which cannot happen while the
/// caller holds a clone, but is handled defensively), the wait ends so a
/// subscription can never hang on a stale channel.
async fn await_done(done: Arc<tokio::sync::watch::Sender<bool>>) {
    let mut receiver = done.subscribe();
    while !*receiver.borrow() {
        if receiver.changed().await.is_err() {
            break;
        }
    }
}

/// Forwards one service's IPC log stream to every attached subscription.
///
/// The loop ends when the stream closes or every subscriber has left. The
/// teardown — removing the channel, notifying the subscribers, dropping the
/// IPC subscription (which sends the daemon `unsubscribe_service_logs`), and
/// publishing `done` — runs under the lifecycle and state locks so a fresh
/// subscribe for the same service can never interleave with it
/// (`tasks/021.md` §24).
async fn run_reader(
    fanout: Arc<LogFanout>,
    app: AppHandle,
    service_id: String,
    mut subscription: Box<dyn LogSubscription>,
    mut stop: tokio::sync::watch::Receiver<bool>,
    lifecycle: Arc<tokio::sync::Mutex<()>>,
) {
    loop {
        let event = tokio::select! {
            result = subscription.recv() => match result {
                Ok(entry) => Some(LogStreamEvent::entry("", &service_id, &entry)),
                Err(LogStreamError::Lagged { dropped }) => {
                    Some(LogStreamEvent::lagged("", &service_id, dropped))
                }
                Err(LogStreamError::Closed) => None,
            },
            changed = stop.changed() => {
                let _ = changed;
                None
            }
        };
        let Some(event) = event else {
            break;
        };
        let state = fanout.state.lock().await;
        let Some(channel) = state.channels.get(&service_id) else {
            break;
        };
        for subscriber in &channel.subscribers {
            let mut payload = event.clone();
            payload.subscription_id = subscriber.clone();
            let _ = app.emit(LOG_EVENT, &payload);
        }
    }

    // Teardown: under both locks, so a re-subscribe cannot interleave.
    let _guard = lifecycle.lock().await;
    let mut state = fanout.state.lock().await;
    state.subscriptions.retain(|_, sid| sid != &service_id);
    let done = if let Some(removed) = state.channels.remove(&service_id) {
        for subscriber in &removed.subscribers {
            let payload = LogStreamEvent::closed(
                subscriber,
                &service_id,
                "the daemon closed this log stream",
            );
            let _ = app.emit(LOG_EVENT, &payload);
        }
        Some(removed.done)
    } else {
        None
    };
    drop(subscription); // sends the daemon `unsubscribe_service_logs`
    drop(state);
    if let Some(done) = done {
        let _ = done.send(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unsubscribing_an_unknown_subscription_is_idempotent() {
        let fanout = LogFanout::default();
        fanout.unsubscribe("never-subscribed").await;
        let state = fanout.state.lock().await;
        assert!(state.channels.is_empty());
        assert!(state.subscriptions.is_empty());
    }

    #[tokio::test]
    async fn the_last_unsubscribe_marks_the_channel_stopping_for_teardown() {
        let fanout = Arc::new(LogFanout::default());
        {
            let mut state = fanout.state.lock().await;
            let (stop, _stop_rx) = tokio::sync::watch::channel(false);
            let (done, _done_rx) = tokio::sync::watch::channel(false);
            state.channels.insert(
                "orders-api".to_owned(),
                ServiceChannel {
                    subscribers: HashSet::from(["sub-1".to_owned()]),
                    stop,
                    done: Arc::new(done),
                    stopping: false,
                },
            );
            state
                .subscriptions
                .insert("sub-1".to_owned(), "orders-api".to_owned());
        }

        fanout.unsubscribe("sub-1").await;

        // The channel stays visible (as stopping) until the reader's teardown
        // removes it, so a racing re-subscribe waits instead of double-
        // subscribing.
        let state = fanout.state.lock().await;
        assert!(state.subscriptions.is_empty());
        let channel = state
            .channels
            .get("orders-api")
            .expect("the stopping channel remains visible");
        assert!(channel.stopping);
        assert!(channel.subscribers.is_empty());
    }
}
