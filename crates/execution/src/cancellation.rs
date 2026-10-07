//! Cooperative cancellation.
//!
//! Every task, the scheduler, and the engine itself own a [`Cancellation`].
//! Cancellation is *cooperative*: a running executor is never killed
//! asynchronously. Instead it is handed a [`Cancellation`] in its
//! [`TaskContext`](crate::TaskContext) and is expected to observe it —
//! periodically awaiting [`Cancellation::cancelled`] or checking
//! [`Cancellation::is_cancelled`] — so it can release resources (a process, a
//! file handle) before returning.
//!
//! The primitive is a one-way [`tokio::sync::watch`] channel: cheap to clone,
//! safe to share across threads, and awaitable from any task.

use tokio::sync::watch;

/// A one-way cancellation signal that can be cloned and shared.
#[derive(Clone, Debug)]
pub struct Cancellation {
    sender: watch::Sender<bool>,
}

impl Cancellation {
    /// Creates a cancellation that is not yet signalled.
    #[must_use]
    pub fn new() -> Self {
        let (sender, _receiver) = watch::channel(false);
        Self { sender }
    }

    /// Signals cancellation.
    ///
    /// Cancellation is idempotent: signalling an already-cancelled token has no
    /// further effect.
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }

    /// Returns `true` if cancellation has been signalled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    /// Waits until cancellation is signalled.
    ///
    /// If cancellation has already been signalled this returns immediately, so
    /// an executor that checks cancellation late still observes it.
    pub async fn cancelled(&self) {
        let mut receiver = self.sender.subscribe();
        if *receiver.borrow() {
            return;
        }
        while receiver.changed().await.is_ok() {
            if *receiver.borrow() {
                return;
            }
        }
    }
}

impl Default for Cancellation {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tokens_are_not_cancelled() {
        let cancellation = Cancellation::new();
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent_and_observed_by_clones() {
        let cancellation = Cancellation::new();
        let clone = cancellation.clone();
        cancellation.cancel();
        cancellation.cancel();
        assert!(cancellation.is_cancelled());
        assert!(clone.is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_returns_immediately_when_already_cancelled() {
        let cancellation = Cancellation::new();
        cancellation.cancel();
        cancellation.cancelled().await;
    }

    #[tokio::test]
    async fn cancelled_waits_for_a_signal() {
        let cancellation = Cancellation::new();
        let signal = cancellation.clone();
        let handle = tokio::spawn(async move {
            signal.cancel();
        });
        cancellation.cancelled().await;
        handle.await.unwrap();
    }
}
