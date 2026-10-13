# Health

Health is a **separate concern** from process state. A service with a live
process is not necessarily healthy, and a healthy service may still be starting.
Service Orchestrator answers "is this service actually serving?" with configured
probes and reports the result as a structured state. The model is in
[`health-manager.md`](health-manager.md).

## 1. Health states

| State | Meaning |
| --- | --- |
| `unknown` | No recent observation. |
| `checking` | A probe is running. |
| `healthy` | All required checks passed. |
| `degraded` | A required check passed but an optional check failed. |
| `unhealthy` | A required check failed. |
| `unreachable` | The probe could not be attempted at all (e.g. connection refused). |

`HealthState` is distinct from `ProcessState`, so "running but unhealthy" and
"running but still starting" are both representable.

## 2. Check types

```toml
[[services.health_checks]]
type = "http"
url = "http://localhost:8081/actuator/health"
expected_status = 200
timeout_seconds = 10
interval_seconds = 5
required = true            # default true

[[services.health_checks]]
type = "port"
port = 8081

[[services.health_checks]]
type = "process"
```

| Type | What it probes |
| --- | --- |
| `process` | Whether the daemon's managed process for the service is running (reads the runtime index, never scans the OS). |
| `port` | A loopback TCP connect to a declared port. |
| `http` | A single `GET`; for Spring Boot Actuator, a `status` field in the JSON body is authoritative. |

- A `port` check must target a port the service declares.
- A `port`/`http` check requires its `port`/`url`; `expected_status` must be
  `100..=599`.
- Checks marked `required = true` (the default) dominate the aggregate; a failing
  **optional** check only **degrades**.

Aggregation is deterministic and ordered: required failures dominate,
unreachable dominates undetermined, and statuses are never averaged into a
percentage.

## 3. Manual checks

- **TUI** — Health screen, `Enter`/`h` to check the current service, `H` to check
  the selection, `l` for logs.
- **GUI** — Health screen; each row shows runtime and health side by side; open a
  row for per-check detail and run a manual check or readiness check.
- **MCP** — `check_service_health { serviceId, scope? }` where `scope` is `all`
  (default), `process`, `tcp`, or `http`; `get_service_health { serviceId }` for
  the last observation; `list_service_health { workspaceId }` for all.

A check that *runs and observes "not healthy"* is a normal observation, not an
error. A `HealthError` means the check could not be attempted at all.

## 4. Readiness vs liveness

- **Liveness/health** — the checks above: is the service serving *now*?
- **Readiness** — `check_service_readiness` polls the required checks with
  cancellation until the service is serving or the deadline passes (a
  `ReadinessCheck` task with a 60-second default). Starting a service and waiting
  for readiness are distinct operations; a readiness check never hides as a
  startup action.

Health checks run after startup in an environment workflow only when
`start = true` and `wait_for_health = true` (the default)
([`workflows.md`](workflows.md)).

## 5. Diagnostics

`get_service_diagnostics` composes the runtime state, the managed process, the
latest health result, recent error logs, and the declared ports into one
structured report. It reports **observed facts** and never fabricates a root
cause.

## 6. Common check failures

| Code | Meaning | Action |
| --- | --- | --- |
| `HEALTH_CHECK_TIMEOUT` | The probe exceeded its timeout. | Raise `timeout_seconds`; check service responsiveness. |
| `HEALTH_CHECK_CONNECTION_REFUSED` | Nothing is listening on the port. | Start the service; check the port. |
| `HEALTH_CHECK_INVALID_RESPONSE` | The HTTP response was not parseable / unexpected. | Check `url` and `expected_status`. |
| `HEALTH_CHECK_UNSUPPORTED` | The check type is not implemented. | Use `process`/`port`/`http`. |
| `HEALTH_CONFIGURATION_INVALID` | The check configuration is invalid. | Fix the check. |
| `HEALTH_NOT_READY` | Readiness did not pass before the deadline. | Inspect logs and dependencies. |
| `HEALTH_UNREACHABLE` | The service could not be probed. | Check that it is running and reachable. |
| `HEALTH_UNDETERMINED` | The result could not be determined. | Re-run the check; inspect diagnostics. |

If a **process** is running but the service is unhealthy, the cause is usually a
dependency that is not up (a database, another service), a wrong port, or an
application startup failure. Check `get_service_diagnostics` and the error logs.

## 7. Limitations

- Health history is **observation-derived**: the core keeps the latest
  observation and transition timestamps, not a per-check time series. The GUI
  reports history as unavailable when nothing was observed rather than
  fabricating one.
- HTTP health URLs are validated for shape but **not allow-listed**; a configured
  URL may point at an arbitrary host ([`security.md`](security.md) §11).
- Polling is opt-in; the daemon does not continuously scan every service.
