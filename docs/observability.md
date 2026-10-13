# Observability

The Observability layer unifies the log and event sources the platform already
produces — managed process output, task-engine events, and application messages —
into one structured, bounded, queryable, and streamable model. It is
interface-neutral: the Ratatui TUI, the MCP server, and a future CLI all read the
same application use cases and the same realtime subscription.

This document describes the `service-orchestrator-logging` infrastructure crate:
the structured log model, the log hub, ingestion and correlation, historical
retrieval, filtering, cursor pagination, retention, realtime streaming,
backpressure, redaction, and diagnostics. The health manager is in
`docs/health-manager.md`; the decisions behind both are in
`docs/adr/012-health-observability.md`.

---

## 1. Where it sits

The observability layer is **infrastructure**. It reuses the sources that already
exist — it does not rewrite the process manager's capture, the runtime's events,
or the task engine's events. It *correlates* them.

```text
ProcessManager ──> ProcessLogIngestor ─┐
TaskEngine     ──> TaskLogIngestor    ─┼──> LogHub ──> LogService (query / page)
Application    ──> LogHub::record     ─┘         │
                                                 └──> per-service broadcast
                                                      (LogSubscription)
                                                          │
                                                          ▼
                                          Application use cases -> TUI / MCP / CLI
```

The `LogHub` is the in-memory store and the concrete `LogService` the application
reads through. The ingestors only translate existing events; they never duplicate
the process manager's or the task engine's responsibilities
(`tasks/012.md` §20, §27, §28).

---

## 2. Structured log model

A `LogEntry` is one structured record. Not every entry has every identifier: an
application log has no process, and a process log has no task
(`tasks/012.md` §21`). Adapters must **not fabricate metadata** they do not have.

| Field | Type | Notes |
| --- | --- | --- |
| `timestamp` | `Timestamp` | when the entry was produced |
| `level` | `LogLevel` | `trace`, `debug`, `info`, `warn`, `error` (ordered by severity) |
| `message` | `String` | the (redacted, truncated) text |
| `workspace_id` | `Option<WorkspaceId>` | when known |
| `service_id` | `Option<ServiceId>` | when known |
| `task_id` | `Option<TaskId>` | when known |
| `process_id` | `Option<String>` | the managed process, when known |
| `source` | `LogSource` | where the entry originated |
| `stream` | `Option<LogStream>` | `stdout` / `stderr`, for process output |
| `sequence` | `u64` | assigned by the store; the pagination cursor |

`LogSource` is `Process | Task | Runtime | Git | Build | Library | Migration |
Application`; the default is `Application`. The source lets an interface filter
"show me build output" separately from "show me application errors" without
parsing message text (`tasks/012.md` §24).

`sequence` is assigned by the hub when an entry is **stored**, never by the
producer, so it is strictly increasing even under concurrency. It is the stable
cursor for pagination.

---

## 3. The log hub

`LogHub` is deliberately simple: a `BTreeMap<u64, LogEntry>` keyed by sequence,
a per-service index of sequences, and one bounded broadcast channel per service
that has had subscribers. It holds no database and is not a search engine.

`record` is the single write path:

1. redacts and truncates the message according to the retention policy;
2. assigns the next monotonic sequence;
3. inserts the entry and indexes it by service;
4. evicts the oldest entries beyond the per-service cap, then beyond the global
   cap;
5. broadcasts the entry to any live subscribers of its service.

A hub never refuses an entry: retention simply evicts the oldest.

`record_message(level, message)` is a convenience for the composition root and
tests; richer callers build a `LogEntry` directly. `retained`,
`retained_for(service)`, `is_empty`, `subscriber_channels`, and `clear` support
introspection and tests.

---

## 4. Ingestion and correlation

Two ingestors translate existing events. They consume **bounded broadcast**
streams; if an ingestor itself falls behind it observes `Lagged`, skips the lost
events, and continues — it never blocks the producer (`tasks/012.md` §22, §27).

### Process output

`ProcessLogIngestor` subscribes to the process manager's log stream
(`ProcessManager::subscribe_logs`). A `LogLine` carries a process id but **not** a
service, so the ingestor resolves the service through the process manager's
registry (`snapshot(process_id).service`) rather than guessing. The resulting
entry has `source = Process`, the originating `stream` preserved, and
`level = Info` — process output carries no severity, so none is invented; a
consumer distinguishes `stdout` from `stderr` through the `stream` field
(`tasks/012.md` §21, §29).

### Task events

`TaskLogIngestor` translates task-engine events into entries with
`source = Task`. Only `TaskCreated` carries the task's owning service, so the
ingestor caches the task-to-service mapping and later events for the same task
inherit it (`tasks/012.md` §21, §29). Each event maps to a level and message:

| Event | Level | Message |
| --- | --- | --- |
| `TaskCreated` | info | `task '<id>' queued (<kind>)` |
| `TaskStarted` | info | `task '<id>' started (<kind>)` |
| `TaskProgress` | debug | the progress message, or `current/total` |
| `TaskMessage` | info | the message |
| `TaskRetrying` | warn | `task '<id>' retrying (attempt N)` |
| `TaskSucceeded` | info | `task '<id>' succeeded` |
| `TaskFailed` | error | `task '<id>' failed (<code>): <message>` |
| `TaskCancelled` | warn | `task '<id>' cancelled` |
| `TaskSkipped` | warn | `task '<id>' skipped` |
| `TaskBlocked` | debug | `task '<id>' blocked on dependencies` |
| `WorkflowCreated` | info | `workflow '<id>' created with N step(s)` |

A workflow event carries no task, so its task id is `None`; metadata is never
fabricated to fill the field. `spawn_process_ingestor` and `spawn_task_ingestor`
start the loops on the current runtime.

### Task correlation

Because task events and process lines are stored with their `task_id` /
`process_id`, the application can retrieve everything relevant to a task without
a global text search (`tasks/012.md` §29`): `get_task_logs` filters the hub by
task id.

---

## 5. Historical retrieval and filtering

`LogService::query` returns matching entries **newest first**, bounded by the
request's limit. `LogService::page` returns one stable page **oldest first** with
a forward cursor.

`LogRequest` supports these filters, all optional (`tasks/012.md` §24):

| Filter | Field |
| --- | --- |
| service | `service_id` |
| task | `task_id` |
| process | `process_id` |
| workspace | `workspace_id` |
| severity | `minimum_level` (at or above) |
| source | `source` |
| text | `text` (case-insensitive substring) |
| time range | `since` / `until` |
| cursor | `after_sequence` (honoured by `page`, ignored by `query`) |
| limit | `limit` |

Filtering is a bounded linear scan over the retained window — a simple bounded
mechanism, not full-text search. A request whose `since` is after its `until` is
rejected with `LogServiceError::QueryInvalid`. A working example of a diagnostic
query is "the most recent error-level lines for a service":
`minimum_level = Error`, limited, read newest-first and reversed to chronological.

---

## 6. Pagination

`page` uses the monotonic `sequence` as a **stable cursor**
(`tasks/012.md` §25):

- entries are returned oldest first, with `next_cursor` set to the sequence of
  the last returned entry when more matched than the limit allowed (`truncated`);
- because sequences only grow, an entry appended between pages always sorts
  strictly after the cursor, so a page produced while new lines arrive is never
  shifted or duplicated;
- `next_cursor` is `None` when no further matching entries exist.

`LogPage` carries `entries`, `next_cursor`, and `truncated`. This is the shape a
future MCP `get_service_logs` / `get_task_logs` tool returns, so a client can
walk a whole history without ever receiving unbounded content.

---

## 7. Retention

`LogRetention` bounds the hub so a continuously running service cannot consume
unlimited memory (`tasks/012.md` §26):

| Limit | Default | Meaning |
| --- | --- | --- |
| `per_service` | 2000 | entries retained per service |
| `total` | 20000 | entries retained across every service |
| `max_message_len` | 4096 | characters retained per message |
| `subscriber_capacity` | 1024 | per-service realtime channel capacity |
| `max_query_limit` | 1000 | hard upper bound on one query's `limit` |

Every limit is normalised to at least one, so a misconfigured zero can never
disable retention or panic the broadcast channel. Eviction removes the oldest
entries from storage as well as from the index. When a request sets no limit,
`DEFAULT_QUERY_LIMIT` (200) is used, and any request is clamped to
`max_query_limit` regardless of the value supplied.

---

## 8. Realtime streaming and backpressure

`LogService::subscribe(service_id)` returns an interface-neutral
`LogSubscription` (`ChannelLogSubscription`), backed by a per-service bounded
`tokio::sync::broadcast` channel. Multiple subscribers each receive every entry
for their service (`tasks/012.md` §22).

Backpressure is explicit (`tasks/012.md` §27`):

- a slow subscriber does **not** block the producer or the other subscribers;
  the channel drops the oldest entries for that subscriber only;
- the dropped count is surfaced as `LogStreamError::Lagged { dropped }`, so a
  consumer can detect the gap and resynchronise from stored history;
- `LogStreamError::Closed` is returned when the producer has shut down.

The stored history and the live subscription are independent: a subscriber never
blocks a managed process's `stdout`/`stderr` reader.

---

## 9. Secret redaction

Logs are a common place for a credential to leak by accident. `redact` applies a
conservative, pattern-based scrub before an entry is stored or streamed
(`tasks/012.md` §28`):

- the value of a secret-looking `key=value` pair (`password`, `passwd`, `pwd`,
  `secret`, `token`, `apikey`/`api_key`/`api-key`, `access_key`/`accesskey`,
  `private_key`/`privatekey`, `credential`, `authorization`, `bearer`);
- the userinfo of a URL (`scheme://user:pass@host` → `scheme://***@host`);
- a token following a `Bearer` / `Basic` scheme keyword.

Redacted values are replaced with the domain's `REDACTED` placeholder (`***`), so
the marker is consistent with the environment model (`AGENTS.md` §31`).
Whitespace and non-secret assignments are preserved.

The rule is a domain invariant (`service_orchestrator_domain::redact`) shared by
the log hub, the build and migration adapters, and the process description;
`service_orchestrator_logging::redact` re-exports it (`docs/security.md` §7).

**It is best-effort and does not pretend otherwise.** There is no reliable way to
detect every secret in free text, so redaction targets obvious shapes, and the
system additionally avoids logging sensitive inputs in the first place. This is
documented as a limitation in the ADR.

---

## 10. Diagnostics

The application composes a structured diagnostic from observed facts only
(`tasks/012.md` §30, §31`). `get_service_diagnostics(service_id)` combines:

- the service runtime state and the managed process (id, pid, state, command);
- the latest health aggregate and each check outcome
  (`docs/health-manager.md`);
- the service's declared ports;
- the most recent structured failure (`last_failure`), derived from a failing
  health check or the newest error-level log line, with its source (`health` /
  `log`) and a stable code;
- `recent_errors`: the most recent error-level log lines for the service, oldest
  first (the newest error is the last element);
- `notes`: observed caveats such as "no health check has been observed yet".

The diagnostic is a `ServiceDiagnostics` value — never a formatted string — and
performs no destructive or mutating operation. It never fabricates a root cause:
a failure is the structured code and message the observing subsystem produced.

---

## 11. Application core integration

| Use case | Kind | Notes |
| --- | --- | --- |
| `get_service_logs(service, request)` | query | one stable page of a service's logs; the request is scoped to the service |
| `get_task_logs(task, request)` | query | one stable page of a task's logs; fails if the task is unknown |
| `subscribe_service_logs(service)` | query | a realtime subscription for a service |
| `get_service_diagnostics(service)` | query | the structured diagnostic |

All are read-only and bounded. `LogServiceError::QueryInvalid` maps to
`ApplicationError::LogQueryInvalid`; other log failures map to
`ApplicationError::LogService`. A `NullLogService` default keeps the application
constructible when logging is not wired; `LogHub` is the concrete implementation
in production.

`Application::recent_error_logs` (used by diagnostics) deliberately calls
`query` — which returns entries **newest first** — and reverses them to
chronological order. It does not use `page`, whose forward cursor selects the
*oldest* matching window, which would report stale errors as recent.

---

## 12. Security considerations

- **Redaction before storage.** Every stored or streamed message passes through
  `redact` and is truncated to the retention limit.
- **No secrets in queries or errors.** Structured log errors carry no message
  content; diagnostics carry only already-redacted lines.
- **Bounded everything.** Retention, per-query limits, message length, and
  subscriber channel capacity are all bounded, so neither a chatty service nor an
  eager client can exhaust memory through log accumulation
  (`tasks/012.md` §26, §27).
- **Lossy, non-blocking streaming.** A slow consumer drops entries for itself
  rather than stalling a managed process.
- **Honest limits.** Redaction is pattern-based and documented as imperfect; the
  system also avoids logging sensitive inputs in the first place.

---

## 13. Testing

Unit tests cover sequence assignment, newest-first and bounded queries, filtering
by level/source/text/task/process/workspace, stable cursor pagination, per-service
and total retention, redaction and truncation, time-range validation, zero-limit
normalisation, live broadcast, and multi-subscriber delivery. Integration tests
(`crates/logging/tests/observability.rs`) use loopback and `/bin/sh` fixtures —
no company services and no external network — to cover:

- multiple subscribers each receiving live entries;
- a slow subscriber lagging without blocking the producer;
- bounded, filterable historical retrieval;
- pagination walking the whole history without duplicates;
- per-service and total retention capping a chatty service;
- recorded messages being redacted;
- the task ingestor correlating events with their service;
- real managed-process output flowing into the hub;
- application queries and diagnostics reading the real hub.

The health side (providers, aggregation, readiness, polling, events) is tested in
`crates/health/tests/health_manager.rs`; see `docs/health-manager.md` §17.
