# Health Management

The Health layer determines whether a locally managed service is **running,
reachable, ready, healthy, or unhealthy**. It turns the health checks declared in
validated configuration into structured probes, runs them as task-engine
operations, aggregates their outcomes deterministically, and records the result
where the application core — and therefore the TUI and MCP — can read it.

This document describes the `service-orchestrator-health` infrastructure crate:
the health manager, the provider abstraction, the process/TCP/HTTP providers, the
state model, readiness semantics, configuration, polling, events, history,
diagnostics, and security. The decisions behind it are recorded in
`docs/adr/012-health-observability.md`. The companion observability layer is in
`docs/observability.md`.

---

## 1. Where it sits

The health adapter is **infrastructure**. It implements the health task kinds
and a small read-only application port. It owns probe mechanics and bookkeeping,
never process lifecycle, scheduling, or business rules.

```text
TUI / MCP / CLI
       │
       ▼
Application core  (check_service_health, check_service_readiness,
       │           get_service_health, list_service_health,
       │           get_service_diagnostics)
       ▼
Task Engine ──> HealthTaskExecutor
       │
       ▼
Health Manager ──> HealthCheckerRegistry ──> ProcessHealthChecker
       │                                       TcpHealthChecker
       │                                       HttpHealthChecker
       ▼
HealthStore (HealthIndex)  +  HealthEventStream
       │
       └── mirrors the aggregate into the application's RuntimeStateStore
```

Four seams stay separate (`AGENTS.md` §3, §16, `tasks/012.md` §3):

| Seam | Owns |
| --- | --- |
| **Process Manager / Runtime** | process lifecycle, PID tracking, `stdout`/`stderr`, exit detection |
| **Health Manager** | probe interpretation, aggregation, history, events, mirroring health state |
| **Task Engine** | scheduling, task lifecycle, cancellation, timeouts, task events |
| **Application Core** | the use cases on top, the read-only `HealthIndex` port, policy |

No checker spawns a process. No checker schedules a task. The process manager
remains the single process owner, and the health manager never scans the
operating system for arbitrary processes.

### Health versus process state

Health state is deliberately **separate** from process state
(`tasks/012.md` §11). These are all valid combinations:

```text
Process: Running   Health: Unhealthy    (process alive, endpoint refused)
Process: Running   Health: Checking     (process alive, still starting)
Process: Stopped   Health: Unknown      (never checked / not managed)
```

A live PID does **not** imply a healthy service.

---

## 2. The health manager

`HealthManager` is the coordinator. It holds validated `Config` plus the
read-only ports it needs, resolves providers through a registry, runs each
selected probe under its own timeout, aggregates the outcomes, records them, and
publishes events.

```rust
pub struct HealthManager { /* config, service_runtime, runtime_state,
                              registry, store, events, defaults */ }
```

Construction registers the three standard providers:

```rust
HealthManager::new(config, service_runtime_index, runtime_state_store)
```

- the **process** provider reads managed-process state from the
  `ServiceRuntimeIndex` the runtime adapter implements;
- the **TCP** and **HTTP** providers are stateless.

The manager is cloneable behind an `Arc` and shared by the task executor and the
periodic monitor. `with_checkers`, `with_store`, `with_check_timeout`, and
`with_poll_interval` replace its collaborators and defaults; `store()` and
`subscribe()` expose the read path and the event stream.

### Defaults

| Constant | Value | Meaning |
| --- | --- | --- |
| `DEFAULT_CHECK_TIMEOUT_SECONDS` | 5 | per-check timeout when a check configures none |
| `DEFAULT_POLL_INTERVAL_MILLIS` | 500 | readiness poll interval when no check configures one |
| `DEFAULT_READINESS_TIMEOUT_SECONDS` | 60 | readiness task deadline when the task configures none |
| `DEFAULT_HISTORY_CAPACITY` | 50 | retained health samples per service |
| `DEFAULT_MAX_CONCURRENCY` | 4 | concurrent checks in the monitor |
| event bus capacity | 256 | bounded health-event broadcast |

### A probe result is not an error

Every **ordinary** observation is returned as a `CheckResult`, including a
failure — a closed port, a stopped process, an endpoint that reports `DOWN`.
Those are facts about the service, and the manager records them as a
`HealthState`. A `HealthError` is returned only when the check **could not be
attempted at all**: the service is unknown, no check matches the scope, no
provider is registered for the kind, or a provider reports a configuration
failure (`tasks/012.md` §35).

---

## 3. Provider abstraction

A provider answers one question about one configured check. The trait is small
and asynchronous; the provider performs no timing, retries, storage, or event
publication.

```rust
#[async_trait]
pub trait HealthChecker: Send + Sync {
    fn check_type(&self) -> CheckType;
    async fn check(&self, context: &HealthCheckContext<'_>)
        -> Result<CheckResult, HealthError>;
}
```

- `HealthCheckContext` borrows the workspace id, service id, the configured
  check, and the effective timeout, so a provider carries no owned
  configuration.
- `CheckResult` carries an observed `status`, a display-safe `target`, and an
  optional `message`. The constructors `healthy`, `unhealthy`, `unreachable`,
  and `unknown` make intent explicit.
- `CheckType` is `Process | Tcp | Http`; `CheckType::from_kind` maps a configured
  `HealthCheckKind` onto a provider, and returns `None` for a project-specific
  `Other`, which the manager reports as `HealthCheckUnsupported`.
- `check_target` produces a stable description (a URL or `127.0.0.1:8081`) so the
  manager can describe a check even when its future timed out.

The registry is a small `BTreeMap<CheckType, Arc<dyn HealthChecker>>`, **not a
plugin framework**: no discovery, no dynamic loading. `HealthCheckerRegistry::standard`
registers the three providers; adding a fourth is one implementation and one
registry line (`tasks/012.md` §5, §6).

Docker, Kubernetes, and gRPC providers are explicitly **out of scope** for this
task.

---

## 4. Implemented providers

### Process

`ProcessHealthChecker` asks exactly one question: *is the service's managed
process alive?* It reads the `ServiceRuntimeIndex` the runtime adapter
implements, which tracks only processes Service Orchestrator started
(`tasks/012.md` §7).

- a tracked, running process → `healthy`;
- a tracked process in any other state → `unhealthy` with the state in the
  message;
- no tracked process at all → `unhealthy` ("not managed by the process
  manager").

It never scans the OS and never treats an unrelated process using the same port
as a managed service.

### TCP

`TcpHealthChecker` attempts a connection to a local port with the effective
timeout (`tasks/012.md` §8, §12).

- an accepted connection → `healthy` (this is **reachability**, not readiness);
- `ConnectionRefused` → `unreachable` ("the connection was refused");
- another I/O error → `unreachable` with the error, and a timeout → `unreachable`
  with the elapsed time.

Only the loopback interface (`127.0.0.1`) is probed, and the port comes from
validated configuration. The provider does not offer arbitrary host/port
requests. A zero timeout is normalised to one millisecond so a check never hangs
or panics.

Because TCP reachability is only reachability, aggregation does **not** report a
service fully healthy on TCP alone when a stronger (HTTP/actuator) required check
is configured.

### HTTP / Spring Boot Actuator

`HttpHealthChecker` sends a single `GET` over a plain TCP socket and interprets
the response. Spring Boot Actuator is supported through the same provider: when
the JSON body contains a `status` field it is **authoritative**
(`tasks/012.md` §9, §10).

| Body `status` | Result |
| --- | --- |
| `UP` | `healthy` |
| `DOWN` | `unhealthy` |
| `OUT_OF_SERVICE` | `unhealthy` |
| any other (e.g. `UNKNOWN`) | `unknown` |

So an HTTP `200` with `"status":"DOWN"` is reported **unhealthy** — a `200` does
not automatically mean the application is up. When there is no Actuator status
in the body, the response is judged by status code: the configured
`expected_status` if set, otherwise any `2xx`.

`actuator_status` exposes the status mapping for documentation and tests.

#### Deliberate limits

- **Only `http://` is supported.** `https://` requires TLS, which the built-in
  provider does not implement; a configured HTTPS URL is rejected as a
  configuration error rather than silently downgraded.
- **Redirects are never followed.** A redirect is just an unexpected status, so
  credentials are never forwarded to another endpoint (`tasks/012.md` §9, §34).
- The response body is read with a hard cap (`64 KiB`), so a hostile or broken
  endpoint cannot exhaust memory.
- No credentials or custom headers are sent: only `Host`, `Accept:
  application/json`, and `Connection: close`. Userinfo in a URL is stripped and
  never transmitted (`tasks/012.md` §28).

A malformed response (no header terminator, no status line, a non-numeric code)
is reported as `HealthCheckInvalidResponse`, a configuration-class error, rather
than a state.

---

## 5. Health state model

The health state is an application value (`HealthState`), separate from
`ProcessState`:

| State | Meaning |
| --- | --- |
| `Unknown` | health has not been observed |
| `Checking` | a probe is in flight and the result is not yet known |
| `Healthy` | every required check succeeded |
| `Unhealthy` | at least one required check failed |
| `Degraded` | only optional checks failed; the service is still serving |
| `Unreachable` | the process may be alive, but the service is not answering a network probe |

`is_healthy()` is true only for `Healthy`; `is_serving()` is true for `Healthy`
and `Degraded`.

`ServiceHealth` is the aggregate record the manager stores and the application
reads. It carries the workspace and service ids, the aggregate `state`, when it
was last `checked_at`, the latest outcome of each configured check, how many
`consecutive_failures` there have been, and the `last_healthy_at` /
`last_unhealthy_at` instants.

---

## 6. Aggregation

`aggregate_health` is deterministic and never averages statuses into a
percentage (`tasks/012.md` §14). Each configured check is either **required**
(the default) or **optional** (`required = false`). The rule, in order:

1. no checks → `Unknown`;
2. any **required** check `Unhealthy` or `Degraded` → `Unhealthy`;
3. otherwise any **required** check `Unreachable` → `Unreachable`;
4. otherwise any **required** check `Unknown` or `Checking` → `Checking`;
5. otherwise any **optional** check failed → `Degraded`;
6. otherwise → `Healthy`.

A required failure dominates an optional one, and an unreachable result
dominates an undetermined one. A failing optional check degrades health rather
than making the service unhealthy, so a non-critical probe cannot take a working
service out of the healthy set.

The `checking` state is only meaningful while a probe is in flight. Once a run
completes, the manager records a settled state; a service never stays stuck in
`checking` after a run.

---

## 7. Readiness vs liveness

A service may have started its process but still be initializing. The stages are
represented by the configured **required** checks, not by a hard-coded sequence
(`tasks/012.md` §12, §16):

```text
Starting
   │
   ▼
Process running      (required process check)
   │
   ▼
Port open            (required TCP check)
   │
   ▼
HTTP readiness UP    (required HTTP/actuator check)
   │
   ▼
Ready
```

Not every service requires all stages; the validated configuration determines
them. A service is **ready** when the aggregate of its required checks is
serving (`Healthy` or `Degraded`).

`HealthManager::wait_until_ready` polls a service's required checks until it is
ready or `timeout` elapses:

- it returns `HealthNotReady` if the deadline passes first;
- it returns `OperationCancelled` if cancellation is observed, including while
  sleeping between polls (`tokio::select!` on the sleep and the cancellation
  token);
- the poll interval is the smallest configured `interval_seconds` among the
  required checks, or `DEFAULT_POLL_INTERVAL_MILLIS`.

`check_service_readiness` schedules a `ReadinessCheck` task with a 60-second
default deadline, so a readiness wait is an explicit, bounded, cancellable task
rather than a long-running startup task. **Starting** a service and **waiting
for readiness** are distinct: a start task completes once the process is
spawned, while a readiness check represents the wait until the service is
serving. The startup task lifecycle from Task 010 is unchanged; readiness is an
additional, separate operation.

---

## 8. Configuration

Health checks use the existing configuration system (`tasks/012.md` §13); there
is no competing format. A service declares checks in
`[[services.health_checks]]`:

```toml
[[services.health_checks]]
type = "process"
required = true

[[services.health_checks]]
type = "port"
port = 8081
timeout_seconds = 2
interval_seconds = 1
required = true

[[services.health_checks]]
type = "http"
url = "http://localhost:8081/actuator/health"
expected_status = 200
timeout_seconds = 5
interval_seconds = 10
required = true
```

| Key | Applies to | Meaning |
| --- | --- | --- |
| `type` | all | `process`, `port`, or `http` |
| `port` | `port` | the TCP port to probe (required for `port`) |
| `url` | `http` | an absolute `http://` or `https://` URL (required for `http`) |
| `expected_status` | `http` | the HTTP status treated as healthy; optional |
| `timeout_seconds` | all | per-probe timeout; defaults to 5s |
| `interval_seconds` | all | poll interval; defaults to 500ms |
| `required` | all | whether the check is critical; defaults to **`true`** |

Validation rejects an unknown `type`, a `port` check without a `port`, an `http`
check without a `url`, an out-of-range port, and an out-of-range
`expected_status`, each with a structured, fix-suggesting error. A `Port` check
whose port is not declared by the service is rejected by `Service::validate`.
Duplicate check kinds on one service are rejected.

---

## 9. Polling and cancellation

`HealthMonitor` is a lightweight coordinator around the manager
(`tasks/012.md` §15, §33). It is **not** a second scheduler: it submits no tasks
and owns no scheduling.

- **Opt-in, never everything by default.** A service is polled only once it is
  `watch`ed, so an idle workspace is never scanned continuously.
- **No duplicate loops.** `start` is idempotent; a second call while running is a
  no-op and returns `false`.
- **Bounded concurrency.** At most `DEFAULT_MAX_CONCURRENCY` (4) checks run at
  once, enforced with a `tokio::sync::Semaphore`. The bound is adjustable with
  `with_max_concurrency`.
- **Cooperative shutdown.** `stop` signals cancellation; `shutdown` also awaits
  the loop, so a stopped monitor leaves no task behind.
- **No busy polling.** The loop sleeps for the interval and wakes early only on
  cancellation.

`watch` / `unwatch` / `is_watching` / `watched` manage the watched set, and
clones of a monitor share the watched set, cancellation token, and running state
so two interfaces observe and control the same loop.

Task-driven checks (from `check_service_health` and `check_service_readiness`)
use the task engine's own cancellation and timeout, so they compose with the
rest of the system.

---

## 10. History

`HealthStore` is the manager's memory and the concrete `HealthIndex` the
application reads through. It retains, per service:

- the latest `ServiceHealth` (with the latest per-check outcomes);
- a bounded ring of compact `HealthSample`s (state, timestamp, cumulative
  consecutive failures), capped at `DEFAULT_HISTORY_CAPACITY` (50) per service;
- the running `consecutive_failures` count and the `last_healthy_at` /
  `last_unhealthy_at` instants.

History is **in memory, never a database**, and bounded per service, so a service
polled for days cannot exhaust memory. `record` owns the bookkeeping; the store
never re-derives an aggregate.

The manager mirrors the aggregate into the application's `RuntimeStateStore`, so
`get_service_status` and diagnostics see the same health value the health index
holds.

---

## 11. Events

The manager publishes interface-neutral `HealthEvent`s on a bounded broadcast
bus:

| Event | Published |
| --- | --- |
| `HealthCheckStarted` | before a probe runs |
| `HealthCheckSucceeded` | a probe passed |
| `HealthCheckFailed` | a probe did not pass (carries the observed status, a redacted message, and the duration) |
| `ServiceHealthChanged` | the aggregate state is different from the previous observation |

`ServiceHealthChanged` is emitted **only on a real transition**, so a service
that stays healthy while polled does not produce an event storm
(`tasks/012.md` §19). The "became healthy" / "became unhealthy" transitions are
recovered from the event's `previous` and `current` fields rather than duplicated
as separate variants. Delivery is bounded: a subscriber that falls behind observes
a `Lagged` error and can resynchronise from the `HealthIndex`.

---

## 12. Task-engine integration

Every health operation is a task. `HealthTaskExecutor` decodes the
workspace-aware `ResolvedServiceTarget` from the task's parameters, maps the task
kind onto a check scope, delegates to the manager, and maps the result back to
the engine. No probe meaning lives in the executor.

| Task kind | Scope |
| --- | --- |
| `HealthCheck` | all configured checks |
| `ProcessHealthCheck` | process checks |
| `TcpHealthCheck` | TCP/port checks |
| `HttpHealthCheck` | HTTP checks |
| `ReadinessCheck` | the required-check readiness wait |

A completed run maps to a task outcome as follows:

| Aggregate | Task outcome |
| --- | --- |
| `Healthy` / `Degraded` | **succeeded** (degraded is still a successful observation) |
| `Unhealthy` | failed, code `HEALTH_CHECK_FAILED` |
| `Unreachable` | failed, code `HEALTH_UNREACHABLE` |
| `Unknown` / `Checking` | failed, code `HEALTH_UNDETERMINED` |

A failure attaches the first failing check's message as structured detail, so an
interface can branch on the code without parsing text. Cancellation maps to the
engine's `Cancelled`. `register_health_executors` registers one executor for all
five health kinds; the composition root calls it once with the same manager the
application reads through.

---

## 13. Application core integration

The application never probes anything itself. It reads the last observed health
through the read-only `HealthIndex` port and schedules probes as tasks.

| Use case | Kind | Notes |
| --- | --- | --- |
| `check_service_health(service, scope)` | command | schedules the matching health task; `Permission::Read`; fails if the service has no checks |
| `check_service_readiness(service)` | command | schedules a `ReadinessCheck`; `Permission::Read`; fails if the service has no required checks |
| `get_service_health(service)` | query | last observed health, or `Unknown` if never checked; side-effect free |
| `list_service_health(workspace)` | query | the last observed health of every service in a workspace |
| `get_service_diagnostics(service)` | query | the structured diagnostic (see below) |

The `HealthIndex` port has two methods — `service_health(service_id)` and
`services()` — and a `NullHealthIndex` default keeps the application
constructible when no health adapter is wired (queries then report `Unknown`
instead of failing). Constructing an `Application` with the manager's store via
`with_health_index` wires the real observations in.

**Reads are side-effect free.** A status query returns the *last observed* health
and never triggers a probe; running a probe is a task.

---

## 14. Diagnostics

`get_service_diagnostics(service_id)` combines **observed facts only**
(`tasks/012.md` §30, §31):

- the last runtime state and the managed process (id, pid, state, command);
- the latest health aggregate and each check's outcome;
- the service's declared ports;
- the most recent structured failure, derived from a failing check or the latest
  error-level log line;
- recent error-level log lines for the service, oldest first;
- notes such as "no health check has been observed yet" or "the service has no
  health checks configured".

The diagnostic performs no destructive or mutating operation and **never
fabricates a root cause**: a failure is reported as the structured code and
message the observing subsystem produced. Stable failure codes map check types
to `PROCESS_NOT_RUNNING`, `TCP_CONNECTION_REFUSED`, or `HTTP_CHECK_FAILED`. The
result is a machine-readable `ServiceDiagnostics` value — never a pre-formatted
terminal string — so a future MCP tool can serialize it directly.

---

## 15. Errors

`HealthError` is structured: every variant carries the service and a stable,
machine-readable code.

| Code | Meaning |
| --- | --- |
| `HEALTH_SERVICE_NOT_FOUND` | the requested service is not in the configuration |
| `HEALTH_CHECK_TIMEOUT` | a check exceeded its timeout |
| `HEALTH_CHECK_CONNECTION_REFUSED` | a check could not reach the service |
| `HEALTH_CHECK_INVALID_RESPONSE` | a response could not be interpreted |
| `HEALTH_CHECK_UNSUPPORTED` | no provider is registered for the check kind |
| `HEALTH_CONFIGURATION_INVALID` | the health configuration is not usable |
| `HEALTH_OPERATION_CANCELLED` | cancellation was observed |
| `HEALTH_NOT_READY` | a readiness wait reached its deadline |
| `HEALTH_INTERNAL_ERROR` | an internal failure |

`service()` returns the related service where one exists, and `is_configuration()`
distinguishes a "could not attempt" configuration problem from a runtime probe
outcome.

---

## 16. Security considerations

- **Configuration-sourced endpoints only.** Health URLs and ports come from
  validated configuration. The TCP provider probes only loopback; the HTTP
  provider sends only to the configured host.
- **No HTTPS.** TLS is not implemented by the built-in provider; an HTTPS URL is
  rejected, not downgraded.
- **No redirects.** A redirect is treated as an unexpected status, so credentials
  are never forwarded off-origin (`tasks/012.md` §34).
- **No credentials.** The HTTP provider sends no `Authorization` header and
  strips URL userinfo; it never transmits userinfo.
- **Bounded reads.** Response bodies are capped, so a hostile endpoint cannot
  exhaust memory.
- **No broad probing surface.** There is no generic "check this arbitrary URL"
  capability; the only probes are the configured ones, and a future MCP tool must
  not expose unrestricted HTTP requests (`tasks/012.md` §34).
- **Redacted messages.** Check messages and log lines are redacted before they
  are stored or emitted (see `docs/observability.md`).

---

## 17. Testing

Unit tests cover provider selection, the process/TCP/HTTP providers, Actuator
status parsing, aggregation, scope selection, history bookkeeping, event
context, and the manager's mirroring. Integration tests
(`crates/health/tests/health_manager.rs`) use loopback fixtures and a fake
runtime index — no company services and no external network — to cover:

- a healthy, an unhealthy, and an unreachable HTTP endpoint;
- TCP reachability and refusal;
- a managed process exiting, and an unmanaged service;
- an optional check failing as only `degraded`;
- readiness becoming ready after a delayed startup, timing out, and honouring
  cancellation;
- polling recording health and shutting down cleanly;
- health events reporting transitions without repeating them;
- health being mirrored into the runtime state store;
- an unknown service producing a structured error.

See `docs/observability.md` for the log/streaming tests and the ADR for the
decisions.
