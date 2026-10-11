# ADR 012: The Health Manager and Observability Layer

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

A developer environment is only useful once its services are actually *serving*,
and debugging a partially running environment needs the evidence the platform
already produces. ADR 001 and ADR 004 placed health and logging adapters in the
infrastructure layer and fixed the inward dependency rule; ADR 005 established
the task engine and its executor registry; ADR 006 established the process
manager as the only component allowed to run an operating-system process and made
the point that a live pid does not imply a healthy service (`AGENTS.md` §15, §16,
§32); ADR 010 established the runtime adapter and its managed-process index.

The amendment in `tasks/012.md` shaped this decision with hard requirements:

- health must be a **separate concern from process state**; the two are
  independently valid in every combination;
- the initial providers are **process, TCP, and HTTP**, with **Spring Boot
  Actuator supported through the HTTP provider** rather than a dedicated one;
- providers must return **structured results**, and an ordinary "not healthy"
  observation is a fact, not an error;
- aggregation must be **deterministic** and must never average into a
  percentage;
- readiness must be **distinguishable** from process liveness and network
  reachability, and must not leave a startup task running indefinitely;
- polling must be **opt-in**, cancellable, bounded, and free of duplicate loops;
- the observability layer must **reuse the existing log sources** and correlate
  them, not rewrite the process manager or task engine;
- logs must be **bounded**, **streamable** to multiple slow-tolerant
  subscribers, **filterable**, **paginated**, and **redacted**;
- diagnostics must report **observed facts only** and never fabricate a root
  cause;
- Ratatui, MCP, Kubernetes, Docker, auto-restart, and telemetry exporters are
  explicitly out of scope.

## Decision

Implement a health adapter (`service-orchestrator-health`) and an observability
adapter (`service-orchestrator-logging`), both infrastructure, both reading and
writing through existing application ports.

```text
                Application core
                 │            │
      HealthIndex│            │LogService / LogSubscription
                 ▼            ▼
          HealthManager      LogHub
                 │            ▲
     HealthCheckerRegistry    │ ingestors
        │      │      │       │
   Process   TCP    HTTP   ProcessManager + TaskEngine events
```

### Health and process state are deliberately separate

Process state answers "is the OS process we started alive?" Health state answers
"is the service actually reachable, ready, and serving?". They are modelled by
different types (`ProcessState` and `HealthState`) and never merged, so all of
these remain representable:

```text
Process: Running   Health: Unhealthy    (process alive, endpoint refused)
Process: Running   Health: Checking     (process alive, still starting)
Process: Stopped   Health: Unknown      (never checked / not managed)
```

The manager contains a `HealthState` variant for each meaningful observation:
`Unknown`, `Checking`, `Healthy`, `Unhealthy`, `Degraded`, `Unreachable`.

### Process, TCP, and HTTP are the initial providers

They are the three checks the current Java/Spring Boot and Node services actually
need, and they cover the three independent stages a service passes through
(`tasks/012.md` §6, §12):

- **process** — liveness, read from the runtime adapter's managed-process index
  (the process manager is the only owner of that knowledge; the health adapter
  never scans the OS);
- **TCP** — network reachability, a loopback connection from a configured port;
- **HTTP** — application readiness, a single `GET` judged by status and body.

They are looked up through a small `BTreeMap<CheckType, Arc<dyn HealthChecker>>`,
**not** a plugin framework: no discovery, no dynamic loading. Docker,
Kubernetes, and gRPC providers are deliberately deferred; adding one is a new
implementation and one registry line.

### Actuator is supported through the HTTP provider

Spring Boot Actuator exposes health over plain HTTP at a configurable path
(`/actuator/health`, `/actuator/health/readiness`). A dedicated provider would
reimplement the HTTP transport, timeout, and response handling for a single
vendor's URL convention, so instead the HTTP provider treats an Actuator `status`
field in the JSON body as **authoritative** when present:

```text
UP               -> healthy
DOWN             -> unhealthy
OUT_OF_SERVICE   -> unhealthy
other / UNKNOWN  -> unknown
```

This means an HTTP `200` carrying `"status":"DOWN"` is correctly reported
unhealthy. A service without Actuator is still checked by status code, and
Actuator is never assumed to be enabled.

### A probe result is not an error

A closed port, a stopped process, and an endpoint that reports `DOWN` are facts
about the service, returned as an `Ok(CheckResult)`. `HealthError` is reserved for
the cases where the check could not be attempted at all — an unknown service, an
invalid configuration, an unsupported check kind, a malformed response, or an
internal failure. This keeps the manager's bookkeeping honest: it records an
observed state and never mistakes "could not run the probe" for "the service is
unhealthy".

### Aggregation is deterministic

The rule is fixed and ordered: a required failure dominates; an unreachable
result dominates an undetermined one; a failing optional check only degrades.

```text
no checks                                       -> unknown
any required unhealthy/degraded                 -> unhealthy
else any required unreachable                   -> unreachable
else any required unknown/checking              -> checking
else optional check failed                      -> degraded
else                                            -> healthy
```

A check is `required` by default and optional only when configuration says so
(`required = false`). This is the single place a check's criticality is
expressed, so a non-critical probe can never take a working service out of the
healthy set.

### Readiness is explicit, bounded, and separate from starting

Readiness is represented by the configured **required** checks, not a hard-coded
stage sequence. A service is ready when the aggregate of its required checks is
serving. `wait_until_ready` polls until ready or a deadline, honouring
cancellation while sleeping between polls, and returns `HealthNotReady` on
timeout. It is exposed as its own `ReadinessCheck` task with a 60-second default
deadline, so a **start** task still completes when the process is spawned and a
**readiness** wait is a separate, cancellable, bounded operation. The Task 010
startup lifecycle is unchanged.

### Polling is opt-in and adding it is not a second scheduler

`HealthMonitor` is a lightweight coordinator around the manager. A service is
polled only once it is explicitly watched, so an idle workspace is never scanned;
`start` is idempotent so there are no duplicate loops; concurrency is bounded by
a semaphore; and `shutdown` awaits the loop so nothing is left behind. It submits
no tasks and owns no scheduling — the task engine remains the only scheduler
(`AGENTS.md` §41).

### Observability reuses the existing log sources

The platform already captures managed `stdout`/`stderr`, publishes process and
runtime events, and emits task-engine events. The observability layer does not
replace any of that; two thin ingestors translate those events into a common
`LogEntry` and store them in one `LogHub`:

- **correlation, not fabrication** — process lines resolve their service through
  the process manager's registry, and task events inherit the service cached from
  `TaskCreated`; no metadata is invented;
- **lossy and non-blocking** — an ingestor that falls behind skips the lost
  events and continues, never blocking a producer.

Application messages are recorded directly. The result is one structured,
queryable stream over sources that remain the single owners of their own
behaviour.

### Bounded retention and backpressure are required

A managed service can emit output continuously for days. Unbounded in-memory
accumulation would eventually exhaust memory, so `LogRetention` caps entries
per service (2000), across every service (20000), message length (4096
characters), subscriber channel capacity (1024), and the query limit (1000).
Eviction drops the oldest entries from storage and the index.

Stored history and live subscribers are independent. Each service has its own
bounded broadcast channel, so a slow subscriber drops entries for **itself**
only and observes `LogStreamError::Lagged { dropped }`; it never blocks the
producer or the other subscribers, and it never stalls a managed process's
`stdout`/`stderr` reader.

### Cursor pagination, not offsets

The hub assigns a strictly increasing `sequence` when it stores an entry.
`query` returns newest first; `page` walks oldest first using the sequence as a
forward cursor. Because sequences only grow, an entry appended between pages
always sorts strictly after the cursor, so a page is never shifted or
duplicated. This is the shape a future MCP tool returns, so a client can walk a
whole history under concurrent appends without instability.

### Diagnostics are observed facts

`get_service_diagnostics` composes the runtime state, the managed process, the
latest health aggregate and its checks, the declared ports, the most recent
structured failure, and recent error-level logs. It performs no destructive or
mutating operation and never fabricates a root cause: a failure is reported as
the structured code and message the observing subsystem produced. The result is a
machine-readable value, so a future MCP tool consumes it directly.

### Redaction is best-effort and honest

Every stored or streamed message passes through a pattern-based scrub targeting
obvious shapes — secret-looking `key=value` pairs, URL userinfo, and
`Bearer`/`Basic` tokens — replacing them with the domain's `REDACTED`
placeholder. There is no reliable way to detect every secret in free text, so
this is documented as imperfect, and the system additionally avoids logging
sensitive inputs in the first place.

### A documented dependency exception

Both adapters depend on `execution`: the health adapter implements a
`TaskExecutor` (so checks are task-engine operations) and a small read-only
`HealthIndex` port, and the logging adapter consumes the engine's event stream.
The health adapter also reads validated `config` (to resolve a service's checks)
and a read-only `ServiceRuntimeIndex`; the logging adapter depends on `process`
to subscribe to captured output and resolve a process's service. These are the
same infrastructure-to-infrastructure exceptions ADR 007–011 recorded, applied
deliberately and narrow: the edges point outward from the domain, and neither
`execution`, `process`, nor `config` depends on `health` or `logging`.

## Alternatives considered

1. **Merge health into process state.**
   Rejected: a live process and a serving service are different facts
   (`AGENTS.md` §16). Merging would lose "running but unhealthy" and "starting but
   not yet ready", which are exactly the states a developer debugs.

2. **Scan the operating system for processes or ports.**
   Rejected: the health adapter must not duplicate the process manager's
   ownership or treat an unrelated process as a managed service
   (`tasks/012.md` §7). The process provider reads the managed-process index
   instead.

3. **A dedicated Spring Boot Actuator provider.**
   Rejected: Actuator health is HTTP plus a `status` field. One HTTP provider that
   recognises the field avoids reimplementing transport, timeout, and response
   handling, and still handles services without Actuator
   (`tasks/012.md` §10).

4. **Implement TLS so `https://` health URLs work.**
   Rejected for MVP: it pulls in a TLS stack for a local-development tool whose
   services bind loopback over plain HTTP. An HTTPS URL is rejected as a
   configuration error rather than silently downgraded, and a future provider can
   add TLS deliberately.

5. **Follow HTTP redirects.**
   Rejected: following a redirect can forward credentials to another origin
   (`tasks/012.md` §34). A redirect is treated as an unexpected status.

6. **Treat a probe failure as an error instead of an observation.**
   Rejected: it would conflate "the service is down" with "the probe could not
   run", producing misleading failures. Failures are recorded as states; only an
   un-attemptable check is an error (`tasks/012.md` §35).

7. **Average check outcomes into a health percentage.**
   Rejected: percentages hide which required check failed and invite arbitrary
   thresholds (`tasks/012.md` §14). The deterministic, ordered rule is
   predictable and documents exactly one answer per input.

8. **Poll every configured service continuously.**
   Rejected: it would scan idle workspaces with no benefit
   (`tasks/012.md` §15, §38). Polling is opt-in through the watched set, and
   task-driven checks are explicit operations.

9. **Let the health monitor schedule its own tasks.**
   Rejected: it would create a second scheduler. The monitor calls the manager
   directly; task-driven checks use the engine, which remains the only scheduler.

10. **A new logging pipeline replacing the process manager's capture.**
    Rejected: it would duplicate a working component and risk blocking
    `stdout`/`stderr` (`tasks/012.md` §20). Thin ingestors correlate the existing
    sources instead.

11. **Offset-based or time-based pagination.**
    Rejected: offsets shift when entries are appended and evicted, and timestamps
    are not unique (`tasks/012.md` §25). A monotonic sequence is a stable cursor.

12. **Unbounded in-memory logs, or blocking subscribers.**
    Rejected: a long-running service would exhaust memory, and a slow UI would
    stall a process (`tasks/012.md` §26, §27). Retention is bounded and delivery
    is lossy per subscriber.

13. **A full-text search engine or a database for logs.**
    Rejected as over-engineering for MVP: a bounded in-memory store with a
    linear, filterable scan is sufficient and keeps the layer dependency-free.

14. **Claim perfect secret detection in logs.**
    Rejected: it is not achievable. Redaction is pattern-based, documented as
    imperfect, and paired with avoiding sensitive logging
    (`tasks/012.md` §28).

## Consequences

### Positive

- Process state, health state, and readiness are separate, so every meaningful
  combination is representable and no state is lost.
- Probe mechanics are isolated in three small providers; the manager, task
  engine, application, and interfaces are provider-agnostic.
- Aggregation is deterministic and documented, so a given set of outcomes always
  yields the same state.
- Readiness is an explicit, bounded, cancellable task; a startup task is never
  left running indefinitely.
- Health checks reuse the existing task engine, process manager, runtime index,
  and configuration — no separate scheduler or process owner.
- The observability layer reuses the existing log sources and adds correlation,
  bounded retention, stable pagination, streaming with explicit backpressure,
  and best-effort redaction, all behind existing application use cases.
- Both health and logs reach the TUI and MCP through the same application core,
  so behaviour cannot drift between interfaces.

### Negative / trade-offs

- The `health -> execution`, `health -> config`, `logging -> execution`, and
  `logging -> process` edges are explicit infrastructure-to-infrastructure
  exceptions; they must be applied deliberately to future adapters, not copied
  casually.
- The built-in HTTP provider supports only `http://` and does not follow
  redirects; a service behind TLS or a redirect needs a future, explicitly
  authorised provider.
- Redaction is pattern-based and cannot guarantee that no secret ever appears in
  a log line.
- The log hub is in-memory only: history does not survive a restart, and file
  retention is not implemented.
- Health history is bounded, so very old samples are evicted; it is a trend aid,
  not an audit trail.
- The monitor's watched set is in-process; two separate Service Orchestrator
  instances do not share watched services.
- `Degraded` counts as a required failure in aggregation when a required check
  itself reports degraded, which is a conservative choice documented here.

## Follow-up

- Implement Docker and Kubernetes health providers and gRPC checks on the same
  provider/registry pattern when a service needs them.
- Compose a readiness step into the `PrepareEnvironment` and service-start
  workflows so a service is only reported ready after its required checks pass
  (`tasks/012.md` §16, §33).
- Surface health, logs, and diagnostics in the TUI and MCP strictly through the
  application use cases (`get_service_health`, `list_service_health`,
  `get_service_diagnostics`, `get_service_logs`, `get_task_logs`,
  `subscribe_service_logs`).
- Add a CLI diagnostic command that renders the same structured values.
- Consider optional file-backed log retention where a durable history is wanted,
  keeping in-memory retention as the bounded default.
