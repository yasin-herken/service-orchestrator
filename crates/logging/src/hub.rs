//! The in-memory log hub.
//!
//! [`LogHub`] is the observability layer's store and the concrete
//! [`LogService`] the application reads through. It unifies the log sources the
//! platform already produces — managed process output, task-engine events, and
//! application messages — into one structured, bounded, queryable stream
//! (`tasks/012.md` §20, §21).
//!
//! # Not a database, not a search engine
//!
//! Entries live in memory, keyed by a strictly increasing `sequence` assigned by
//! the hub. `sequence` is the pagination cursor: because it only ever grows, a
//! page produced while new lines arrive stays stable — new entries always sort
//! after the cursor (`tasks/012.md` §25). The store is deliberately simple:
//! filtering is a bounded linear scan over the retained window, not full-text
//! search (`tasks/012.md` §24).
//!
//! # Bounded retention
//!
//! Every entry is subject to two caps: a per-service cap and a global cap
//! (`tasks/012.md` §26). When a cap is exceeded the oldest entries are evicted
//! from storage as well as from the index, so a chatty service cannot grow the
//! hub without bound. Defaults are documented by [`LogRetention`].
//!
//! # Backpressure
//!
//! Stored entries are independent of live subscribers. Each service has its own
//! bounded broadcast channel; a slow subscriber observes [`LogStreamError::Lagged`]
//! rather than blocking the producer or other subscribers (`tasks/012.md` §22,
//! §27).
//!
//! [`LogStreamError::Lagged`]: service_orchestrator_application::LogStreamError::Lagged

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use service_orchestrator_application::{
    LogEntry, LogLevel, LogPage, LogRequest, LogService, LogServiceError, LogSubscription,
};
use service_orchestrator_domain::ServiceId;
use tokio::sync::broadcast;

use crate::redact::redact;
use crate::subscription::ChannelLogSubscription;

/// The default number of entries retained per service.
pub const DEFAULT_PER_SERVICE_ENTRIES: usize = 2000;

/// The default number of entries retained across every service.
pub const DEFAULT_TOTAL_ENTRIES: usize = 20_000;

/// The default maximum length, in characters, of a stored message.
pub const DEFAULT_MAX_MESSAGE_LEN: usize = 4096;

/// The default per-service realtime subscriber channel capacity.
pub const DEFAULT_SUBSCRIBER_CAPACITY: usize = 1024;

/// The default hard upper bound on the number of entries one query returns.
pub const DEFAULT_MAX_QUERY_LIMIT: usize = 1000;

/// The number of entries returned when a request does not set a limit.
pub const DEFAULT_QUERY_LIMIT: usize = 200;

/// The bounded-retention policy of a [`LogHub`].
///
/// All limits are counts of entries or characters and are normalized to be at
/// least one, so a misconfigured zero can never disable retention or panic the
/// broadcast channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogRetention {
    /// The maximum entries retained for any one service.
    pub per_service: usize,
    /// The maximum entries retained across every service.
    pub total: usize,
    /// The maximum length, in characters, of a stored message.
    pub max_message_len: usize,
    /// The capacity of each per-service realtime subscriber channel.
    pub subscriber_capacity: usize,
    /// The hard upper bound applied to a query's `limit`.
    pub max_query_limit: usize,
}

impl Default for LogRetention {
    fn default() -> Self {
        Self {
            per_service: DEFAULT_PER_SERVICE_ENTRIES,
            total: DEFAULT_TOTAL_ENTRIES,
            max_message_len: DEFAULT_MAX_MESSAGE_LEN,
            subscriber_capacity: DEFAULT_SUBSCRIBER_CAPACITY,
            max_query_limit: DEFAULT_MAX_QUERY_LIMIT,
        }
    }
}

impl LogRetention {
    /// Returns a copy with every limit raised to at least one.
    #[must_use]
    fn normalized(self) -> Self {
        Self {
            per_service: self.per_service.max(1),
            total: self.total.max(1),
            max_message_len: self.max_message_len.max(1),
            subscriber_capacity: self.subscriber_capacity.max(1),
            max_query_limit: self.max_query_limit.max(1),
        }
    }
}

/// The hub's mutable state, guarded by one short-lived mutex.
#[derive(Default)]
struct Inner {
    entries: BTreeMap<u64, LogEntry>,
    by_service: HashMap<ServiceId, VecDeque<u64>>,
    subscribers: HashMap<ServiceId, broadcast::Sender<LogEntry>>,
}

/// A bounded, queryable, streaming store of structured log entries.
pub struct LogHub {
    retention: LogRetention,
    sequence: AtomicU64,
    inner: Mutex<Inner>,
}

impl Default for LogHub {
    fn default() -> Self {
        Self::new()
    }
}

impl LogHub {
    /// Creates a hub with the default retention policy.
    #[must_use]
    pub fn new() -> Self {
        Self::with_retention(LogRetention::default())
    }

    /// Creates a hub with an explicit retention policy.
    #[must_use]
    pub fn with_retention(retention: LogRetention) -> Self {
        Self {
            retention: retention.normalized(),
            sequence: AtomicU64::new(0),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Returns the effective retention policy.
    #[must_use]
    pub fn retention(&self) -> LogRetention {
        self.retention
    }

    /// Stores an entry, returning its assigned sequence.
    ///
    /// The message is redacted and truncated before storage, and the entry is
    /// broadcast to any live subscribers of its service. A hub never refuses an
    /// entry: retention simply evicts the oldest ones.
    pub fn record(&self, mut entry: LogEntry) -> u64 {
        entry.message = self.prepare_message(&entry.message);
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        entry.sequence = sequence;

        let mut inner = self.lock();
        inner.entries.insert(sequence, entry.clone());
        if let Some(service) = entry.service_id.clone() {
            inner
                .by_service
                .entry(service.clone())
                .or_default()
                .push_back(sequence);
            self.trim_service(&mut inner, &service);
        }
        self.trim_total(&mut inner);
        if let Some(sender) = entry
            .service_id
            .as_ref()
            .and_then(|service| inner.subscribers.get(service))
        {
            // A send error only means there are currently no live receivers.
            let _ = sender.send(entry);
        }
        sequence
    }

    /// Records a plain application message with no correlation metadata.
    ///
    /// A convenience for the composition root and tests; richer callers build a
    /// [`LogEntry`] directly.
    pub fn record_message(&self, level: LogLevel, message: impl Into<String>) -> u64 {
        self.record(LogEntry::new(level, message))
    }

    /// Returns the number of entries currently retained.
    #[must_use]
    pub fn retained(&self) -> usize {
        self.lock().entries.len()
    }

    /// Returns the number of entries currently retained for `service_id`.
    #[must_use]
    pub fn retained_for(&self, service_id: &ServiceId) -> usize {
        self.lock()
            .by_service
            .get(service_id)
            .map_or(0, VecDeque::len)
    }

    /// Returns `true` when no entries are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().entries.is_empty()
    }

    /// Returns the number of live realtime subscriber channels.
    #[must_use]
    pub fn subscriber_channels(&self) -> usize {
        self.lock().subscribers.len()
    }

    /// Removes every retained entry, leaving live subscribers connected.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.by_service.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Redacts and truncates a message according to the retention policy.
    fn prepare_message(&self, message: &str) -> String {
        truncate(&redact(message), self.retention.max_message_len)
    }

    /// Evicts the oldest entries of a service beyond its per-service cap.
    fn trim_service(&self, inner: &mut Inner, service: &ServiceId) {
        let capacity = self.retention.per_service;
        let mut evicted = Vec::new();
        if let Some(list) = inner.by_service.get_mut(service) {
            while list.len() > capacity {
                match list.pop_front() {
                    Some(sequence) => evicted.push(sequence),
                    None => break,
                }
            }
        }
        for sequence in evicted {
            inner.entries.remove(&sequence);
        }
        if inner
            .by_service
            .get(service)
            .is_some_and(VecDeque::is_empty)
        {
            inner.by_service.remove(service);
        }
    }

    /// Evicts the globally oldest entries beyond the total cap.
    fn trim_total(&self, inner: &mut Inner) {
        while inner.entries.len() > self.retention.total {
            let Some((&sequence, _)) = inner.entries.iter().next() else {
                break;
            };
            Self::evict(inner, sequence);
        }
    }

    /// Removes one entry and keeps the per-service index consistent.
    fn evict(inner: &mut Inner, sequence: u64) {
        let Some(entry) = inner.entries.remove(&sequence) else {
            return;
        };
        let Some(service) = entry.service_id else {
            return;
        };
        let mut empty = false;
        if let Some(list) = inner.by_service.get_mut(&service) {
            while list.front().is_some_and(|candidate| *candidate <= sequence) {
                list.pop_front();
            }
            empty = list.is_empty();
        }
        if empty {
            inner.by_service.remove(&service);
        }
    }

    /// Returns the effective, bounded limit for a request.
    fn effective_limit(&self, requested: Option<usize>) -> usize {
        requested
            .unwrap_or(DEFAULT_QUERY_LIMIT)
            .clamp(1, self.retention.max_query_limit)
    }
}

impl LogService for LogHub {
    /// Returns matching entries newest first, bounded by the request's limit.
    fn query(&self, request: &LogRequest) -> Result<Vec<LogEntry>, LogServiceError> {
        validate_request(request)?;
        let limit = self.effective_limit(request.limit);
        let inner = self.lock();
        let mut entries = Vec::new();
        for entry in inner.entries.values().rev() {
            if !matches_request(entry, request) {
                continue;
            }
            entries.push(entry.clone());
            if entries.len() >= limit {
                break;
            }
        }
        Ok(entries)
    }

    /// Returns one stable, oldest-first cursor page of matching entries.
    ///
    /// The cursor is the sequence of the last returned entry; because sequences
    /// increase monotonically, entries appended concurrently sort strictly after
    /// it and never disturb the page (`tasks/012.md` §25).
    fn page(&self, request: &LogRequest) -> Result<LogPage, LogServiceError> {
        validate_request(request)?;
        let limit = self.effective_limit(request.limit);
        let after = request.after_sequence.unwrap_or(0);
        let inner = self.lock();
        let mut entries = Vec::new();
        let mut truncated = false;
        for entry in inner.entries.values() {
            if entry.sequence <= after || !matches_request(entry, request) {
                continue;
            }
            if entries.len() >= limit {
                truncated = true;
                break;
            }
            entries.push(entry.clone());
        }
        let next_cursor = if truncated {
            entries.last().map(|entry| entry.sequence)
        } else {
            None
        };
        Ok(LogPage {
            entries,
            next_cursor,
            truncated,
        })
    }

    fn subscribe(
        &self,
        service_id: &ServiceId,
    ) -> Result<Box<dyn LogSubscription>, LogServiceError> {
        let mut inner = self.lock();
        let sender = inner
            .subscribers
            .entry(service_id.clone())
            .or_insert_with(|| broadcast::channel::<LogEntry>(self.retention.subscriber_capacity).0)
            .clone();
        let receiver = sender.subscribe();
        Ok(Box::new(ChannelLogSubscription::new(
            service_id.clone(),
            receiver,
        )))
    }
}

impl std::fmt::Debug for LogHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogHub")
            .field("retention", &self.retention)
            .field("retained", &self.retained())
            .finish_non_exhaustive()
    }
}

/// Returns `true` when an entry satisfies every filter in `request`.
fn matches_request(entry: &LogEntry, request: &LogRequest) -> bool {
    if request
        .service_id
        .as_ref()
        .is_some_and(|service| entry.service_id.as_ref() != Some(service))
    {
        return false;
    }
    if request
        .task_id
        .as_ref()
        .is_some_and(|task| entry.task_id.as_ref() != Some(task))
    {
        return false;
    }
    if request
        .process_id
        .as_ref()
        .is_some_and(|process| entry.process_id.as_deref() != Some(process.as_str()))
    {
        return false;
    }
    if request
        .workspace_id
        .as_ref()
        .is_some_and(|workspace| entry.workspace_id.as_ref() != Some(workspace))
    {
        return false;
    }
    if request.source.is_some_and(|source| entry.source != source) {
        return false;
    }
    if request
        .minimum_level
        .is_some_and(|minimum| entry.level < minimum)
    {
        return false;
    }
    if request
        .text
        .as_ref()
        .is_some_and(|text| !contains_ignore_case(&entry.message, text))
    {
        return false;
    }
    if request.since.is_some_and(|since| entry.timestamp < since) {
        return false;
    }
    if request.until.is_some_and(|until| entry.timestamp > until) {
        return false;
    }
    true
}

/// Validates the structural constraints of a request.
fn validate_request(request: &LogRequest) -> Result<(), LogServiceError> {
    if let (Some(since), Some(until)) = (request.since, request.until) {
        if since > until {
            return Err(LogServiceError::QueryInvalid {
                message: format!("'since' ({since}) is after 'until' ({until})"),
            });
        }
    }
    Ok(())
}

/// Returns `true` when `haystack` contains `needle`, ignoring ASCII case.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Truncates `message` to at most `max_chars` characters, marking the cut.
fn truncate(message: &str, max_chars: usize) -> String {
    if message.chars().count() <= max_chars {
        return message.to_owned();
    }
    let mut truncated: String = message.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_orchestrator_application::LogSource;
    use service_orchestrator_domain::{TaskId, Timestamp, WorkspaceId};

    fn service(name: &str) -> ServiceId {
        ServiceId::new(name).unwrap()
    }

    fn entry(service_id: &ServiceId, message: &str) -> LogEntry {
        LogEntry::new(LogLevel::Info, message).with_service(service_id.clone())
    }

    fn small_hub(per_service: usize, total: usize) -> LogHub {
        LogHub::with_retention(LogRetention {
            per_service,
            total,
            max_message_len: 64,
            subscriber_capacity: 8,
            max_query_limit: 100,
        })
    }

    #[test]
    fn sequences_are_monotonic_and_assigned_by_the_hub() {
        let hub = LogHub::new();
        let first = hub.record(entry(&service("a"), "one"));
        let second = hub.record(entry(&service("a"), "two"));
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        let stored = hub.query(&LogRequest::all()).unwrap();
        assert_eq!(stored[0].sequence, 2);
        assert_eq!(stored[1].sequence, 1);
    }

    #[test]
    fn query_returns_newest_first_and_bounded() {
        let hub = LogHub::new();
        for index in 0..10 {
            hub.record(entry(&service("a"), &format!("line {index}")));
        }
        let newest = hub
            .query(&LogRequest::new(service("a")).with_limit(3))
            .unwrap();
        assert_eq!(newest.len(), 3);
        assert_eq!(newest[0].message, "line 9");
        assert_eq!(newest[2].message, "line 7");
    }

    #[test]
    fn query_filters_by_level_source_and_text() {
        let hub = LogHub::new();
        let mut error = entry(&service("a"), "connection refused");
        error.level = LogLevel::Error;
        error.source = LogSource::Process;
        hub.record(error);
        hub.record(entry(&service("a"), "all good"));

        let filtered = hub
            .query(
                &LogRequest::new(service("a"))
                    .with_minimum_level(LogLevel::Warn)
                    .with_source(LogSource::Process)
                    .matching("REFUSED"),
            )
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "connection refused");
    }

    #[test]
    fn query_filters_by_task_process_and_workspace() {
        let hub = LogHub::new();
        let mut scoped = entry(&service("a"), "scoped");
        scoped.task_id = Some(TaskId::new("t1").unwrap());
        scoped.process_id = Some("proc-0".to_owned());
        scoped.workspace_id = Some(WorkspaceId::new("acme").unwrap());
        hub.record(scoped);
        hub.record(entry(&service("a"), "unscoped"));

        let request = LogRequest::all()
            .for_task(TaskId::new("t1").unwrap())
            .for_process("proc-0")
            .for_workspace(WorkspaceId::new("acme").unwrap());
        assert_eq!(hub.query(&request).unwrap().len(), 1);
    }

    #[test]
    fn pagination_is_stable_and_reports_truncation() {
        let hub = LogHub::new();
        for index in 0..5 {
            hub.record(entry(&service("a"), &format!("line {index}")));
        }
        let first = hub
            .page(&LogRequest::new(service("a")).with_limit(2))
            .unwrap();
        assert_eq!(first.entries.len(), 2);
        assert!(first.truncated);
        assert_eq!(first.entries[0].message, "line 0");
        assert_eq!(first.next_cursor, Some(first.entries[1].sequence));

        let second = hub
            .page(
                &LogRequest::new(service("a"))
                    .with_limit(2)
                    .after(first.next_cursor.unwrap()),
            )
            .unwrap();
        assert_eq!(second.entries[0].message, "line 2");
        assert_eq!(second.entries[1].message, "line 3");
        assert!(second.truncated);

        let third = hub
            .page(
                &LogRequest::new(service("a"))
                    .with_limit(2)
                    .after(second.next_cursor.unwrap()),
            )
            .unwrap();
        assert_eq!(third.entries.len(), 1);
        assert_eq!(third.entries[0].message, "line 4");
        assert!(!third.truncated);
        assert_eq!(third.next_cursor, None);
    }

    #[test]
    fn concurrent_appends_do_not_disturb_an_existing_cursor() {
        let hub = LogHub::new();
        hub.record(entry(&service("a"), "line 0"));
        hub.record(entry(&service("a"), "line 1"));
        let first = hub
            .page(&LogRequest::new(service("a")).with_limit(1))
            .unwrap();
        // A new line arrives between pages and must not shift the cursor.
        hub.record(entry(&service("a"), "line 2"));
        let second = hub
            .page(&LogRequest::new(service("a")).after(first.next_cursor.unwrap()))
            .unwrap();
        assert_eq!(second.entries[0].message, "line 1");
        assert_eq!(second.entries[1].message, "line 2");
    }

    #[test]
    fn queries_are_hard_bounded_by_the_maximum() {
        let hub = small_hub(100, 100);
        for index in 0..50 {
            hub.record(entry(&service("a"), &format!("line {index}")));
        }
        let unbounded = hub.query(&LogRequest::new(service("a"))).unwrap();
        assert_eq!(unbounded.len(), 50);
        let absurd = hub
            .query(&LogRequest::new(service("a")).with_limit(usize::MAX))
            .unwrap();
        // The request's limit is clamped to `max_query_limit` (100 here), which
        // is more than the retained count.
        assert_eq!(absurd.len(), 50);
    }

    #[test]
    fn per_service_retention_evicts_oldest() {
        let hub = small_hub(2, 100);
        for index in 0..4 {
            hub.record(entry(&service("a"), &format!("line {index}")));
        }
        assert_eq!(hub.retained_for(&service("a")), 2);
        assert_eq!(hub.retained(), 2);
        let retained = hub.query(&LogRequest::new(service("a"))).unwrap();
        assert_eq!(retained[0].message, "line 3");
        assert_eq!(retained[1].message, "line 2");
    }

    #[test]
    fn total_retention_bounds_every_service() {
        let hub = small_hub(100, 3);
        hub.record(entry(&service("a"), "a0"));
        hub.record(entry(&service("b"), "b0"));
        hub.record(entry(&service("c"), "c0"));
        hub.record(entry(&service("d"), "d0"));
        assert_eq!(hub.retained(), 3);
        assert_eq!(hub.retained_for(&service("a")), 0);
        assert_eq!(hub.retained_for(&service("b")), 1);
    }

    #[test]
    fn messages_are_redacted_and_truncated_on_record() {
        let hub = small_hub(10, 10);
        hub.record(entry(&service("a"), "DB_PASSWORD=hunter2"));
        let stored = hub.query(&LogRequest::new(service("a"))).unwrap();
        assert!(!stored[0].message.contains("hunter2"));

        let long = "x".repeat(200);
        hub.record(entry(&service("a"), &long));
        let stored = hub.query(&LogRequest::new(service("a"))).unwrap();
        assert!(stored[0].message.chars().count() <= 64);
        assert!(stored[0].message.ends_with('…'));
    }

    #[test]
    fn invalid_time_ranges_are_rejected() {
        let hub = LogHub::new();
        let later = Timestamp::now();
        let earlier = Timestamp::from_offset(later.as_offset() - time::Duration::seconds(1));
        let request = LogRequest::all().between(later, earlier);
        assert!(matches!(
            hub.page(&request),
            Err(LogServiceError::QueryInvalid { .. })
        ));
        assert!(matches!(
            hub.query(&request),
            Err(LogServiceError::QueryInvalid { .. })
        ));
    }

    #[test]
    fn zero_limits_and_zero_retention_are_normalized() {
        let hub = LogHub::with_retention(LogRetention {
            per_service: 0,
            total: 0,
            max_message_len: 0,
            subscriber_capacity: 0,
            max_query_limit: 0,
        });
        assert_eq!(hub.retention().per_service, 1);
        assert_eq!(hub.retention().subscriber_capacity, 1);
        hub.record(entry(&service("a"), "kept"));
        assert_eq!(hub.retained(), 1);
    }

    #[test]
    fn clear_drops_history_but_keeps_subscribers_connected() {
        let hub = LogHub::new();
        hub.record(entry(&service("a"), "line"));
        assert!(!hub.is_empty());
        hub.clear();
        assert!(hub.is_empty());
        assert_eq!(hub.subscriber_channels(), 0);
    }

    #[tokio::test]
    async fn subscribers_receive_live_entries() {
        let hub = LogHub::new();
        let mut subscription = hub.subscribe(&service("a")).unwrap();
        hub.record(entry(&service("a"), "live"));
        let received = subscription.recv().await.unwrap();
        assert_eq!(received.message, "live");
    }

    #[tokio::test]
    async fn every_subscriber_receives_each_entry() {
        let hub = LogHub::new();
        let mut first = hub.subscribe(&service("a")).unwrap();
        let mut second = hub.subscribe(&service("a")).unwrap();
        assert_eq!(hub.subscriber_channels(), 1);
        hub.record(entry(&service("a"), "broadcast"));
        assert_eq!(first.recv().await.unwrap().message, "broadcast");
        assert_eq!(second.recv().await.unwrap().message, "broadcast");
    }

    #[tokio::test]
    async fn subscriptions_are_scoped_to_one_service() {
        let hub = LogHub::new();
        let mut subscription = hub.subscribe(&service("a")).unwrap();
        hub.record(entry(&service("b"), "other"));
        hub.record(entry(&service("a"), "mine"));
        assert_eq!(subscription.recv().await.unwrap().message, "mine");
    }
}
