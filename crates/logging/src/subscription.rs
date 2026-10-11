//! A realtime log subscription backed by a bounded broadcast channel.
//!
//! [`ChannelLogSubscription`] is the concrete [`LogSubscription`] the
//! [`LogHub`](crate::LogHub) hands out. It is deliberately thin: the hub owns
//! the channel, assigns sequences, and enforces retention; the subscription only
//! surfaces the next entry an interface should render (`tasks/012.md` §22).
//!
//! # Backpressure
//!
//! The underlying channel is a bounded [`tokio::sync::broadcast`] channel. A
//! subscriber that cannot keep up does not block the producer: the channel drops
//! the oldest entries for that subscriber and reports how many were lost as
//! [`LogStreamError::Lagged`]. The consumer can then resynchronise from stored
//! history (`tasks/012.md` §27).

use async_trait::async_trait;
use service_orchestrator_application::{LogEntry, LogStreamError, LogSubscription};
use service_orchestrator_domain::ServiceId;
use tokio::sync::broadcast;

/// A [`LogSubscription`] over a per-service broadcast channel.
pub struct ChannelLogSubscription {
    service_id: ServiceId,
    receiver: broadcast::Receiver<LogEntry>,
}

impl ChannelLogSubscription {
    /// Creates a subscription for `service_id` over `receiver`.
    pub(crate) fn new(service_id: ServiceId, receiver: broadcast::Receiver<LogEntry>) -> Self {
        Self {
            service_id,
            receiver,
        }
    }

    /// Returns the service this subscription observes.
    #[must_use]
    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }
}

#[async_trait]
impl LogSubscription for ChannelLogSubscription {
    async fn recv(&mut self) -> Result<LogEntry, LogStreamError> {
        match self.receiver.recv().await {
            Ok(entry) => Ok(entry),
            Err(broadcast::error::RecvError::Lagged(dropped)) => {
                Err(LogStreamError::Lagged { dropped })
            }
            Err(broadcast::error::RecvError::Closed) => Err(LogStreamError::Closed),
        }
    }
}

impl std::fmt::Debug for ChannelLogSubscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelLogSubscription")
            .field("service", &self.service_id.as_str())
            .finish_non_exhaustive()
    }
}
