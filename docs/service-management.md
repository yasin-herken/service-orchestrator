# Service Management

A **service** is a buildable and/or runnable unit: a backend, a frontend, or an
`other:<name>` type. This guide covers the runtime lifecycle — start, stop, and
restart — plus ports, process ownership, and startup failures. Building is
covered in [`builds.md`](builds.md); health in [`health.md`](health.md).

## 1. Service types and capabilities

A service's behaviour is derived from its `type` and its declared
**capabilities**, never from its name.

- `backend` — typically a Spring Boot or similar HTTP service.
- `frontend` — typically a Node/npm application; a frontend must declare a
  runtime.
- `other:<name>` — an escape hatch for a technology that is not modelled yet.
- A `library` is **not** a service type in `[[services]]`; libraries are a
  separate entity ([`libraries.md`](libraries.md)).

Capabilities (`build`, `run`, `migrate`, `provide_artifact`, `serve_http`)
declare what a service can do. A service that can `build` must configure a build
command; a service that can `run` must configure a start command.

## 2. Runtime configuration

```toml
[services.runtime]
type = "java"        # java | node | maven | npm | other:<name>
version = "21"       # required for java and node

[services.commands]
install = "./mvnw dependency:resolve"
build   = "./mvnw clean install"
start   = "./mvnw spring-boot:run"
```

- `java` and `node` runtimes are the initial set; a service declares the runtime
  requirement it needs, and `validate_workspace` reports whether it resolves.
- Commands are structured: a string is tokenized **without a shell**, and a
  `sh -c` style command is rejected at configuration time. Prefer the structured
  form for anything non-trivial.

See [`configuration.md`](configuration.md) §5.4–5.5.

## 3. Start, stop, and restart

Every lifecycle operation is an asynchronous **task** submitted to the one task
engine the daemon owns. The call returns immediately with a task id; progress
and the result are observed separately.

| Operation | TUI | GUI | MCP |
| --- | --- | --- | --- |
| Start current | `s` | Services → Start | `start_service { serviceId }` |
| Stop current | `x` | Services → Stop | `stop_service { serviceId }` |
| Restart current | `r` | Services → Restart | `restart_service { serviceId }` |
| Start selection | `S` | bulk Start | `start_services { workspaceId, serviceIds }` |
| Stop selection | `X` | bulk Stop | `stop_services { … }` |
| Restart selection | *no bulk-restart key* | bulk Restart | `restart_services { … }` |

The **TUI** multi-select applies the *uppercase* key to the selection and the
*lowercase* key to the row under the cursor ([`tui.md`](tui.md)). The **GUI**
uses a selection toolbar. In **MCP**, an empty `serviceIds` list is **not** "all
services": it schedules nothing and reports `already_in_desired_state` — pass
explicit ids, or use a group/profile workflow.

Also available: `start_group`/`stop_group` and `start_profile`/`stop_profile`,
which start or stop every service in a configured group or profile
([`workflows.md`](workflows.md)).

## 4. Idempotency

Lifecycle operations are idempotent:

- Starting an already-running service returns an "already in desired state"
  result rather than spawning a second process.
- Stopping a stopped service is a no-op.
- A concurrent start of the same service is serialized by a per-service lock, so
  two clients cannot race two processes into existence.

This is why two interfaces cannot start the same service twice: they see the
same runtime state in the daemon.

## 5. Process ownership

**The daemon owns every managed process.** This is a core rule
([`local-daemon.md`](local-daemon.md), [`security.md`](security.md) §8).

- A service started from any interface is managed by the daemon and is therefore
  visible and controllable from every other interface.
- Processes are spawned in their own process group; a requested stop is graceful
  (SIGTERM) first and escalates to SIGKILL after a grace period. Children are
  cleaned up, not orphaned.
- The daemon **never** signals a process by PID or port. A process that merely
  occupies a configured port — or that reused a PID — is never harmed.
- Runtime state is **in memory**. After a crash or restart the daemon adopts
  nothing, reports zero managed services, and does not re-adopt surviving
  processes. This is a known limitation
  ([`known-limitations.md`](known-limitations.md)).

Closing an interface never stops the daemon or the services. Shutting the daemon
down is explicit: `service-orchestrator daemon stop`.

## 6. Runtime status vs health

`get_service_runtime_status` / the runtime column reports the **process** state
(`Stopped`, `Starting`, `Running`, `Stopping`, `Exited`, …). It is **not**
health. A running process can be unhealthy, and a healthy service can still be
starting. See [`health.md`](health.md).

## 7. Ports

```toml
[[services.ports]]
port = 8081
protocol = "tcp"
```

- Ports are declared per service and validated (`1..=65535`, `tcp`/`udp`).
- Two services declaring the same port is a **configuration conflict** reported
  by `validate_workspace`.
- A declared port currently occupied by **something else** is a **warning**; the
  occupying process is left alone.
- Starting a service whose port is already in use fails with
  `RUNTIME_PORT_ALREADY_IN_USE` — the daemon never kills the occupant.

## 8. Startup failures

| Code | Meaning | Action |
| --- | --- | --- |
| `RUNTIME_PORT_ALREADY_IN_USE` | The configured port is occupied. | Free the port or change it; the occupant is not killed. |
| `RUNTIME_PROCESS_START_FAILED` | The process could not be spawned. | Check the start command, working directory, and executable. |
| `RUNTIME_PROCESS_EXITED` | The process exited immediately or during startup. | Read the service logs — usually a misconfiguration or a dependency that is not ready. |
| `RUNTIME_START_COMMAND_MISSING` | The service has `run` capability but no start command. | Add `commands.start`. |
| `RUNTIME_VERSION_MISMATCH` | The installed runtime version differs from the pin. | Install the pinned version or adjust the pin. |
| `RUNTIME_ALREADY_STARTING` | A start is already in progress. | Wait; starts are serialized per service. |
| `RUNTIME_UNSUPPORTED` | The runtime type is not implemented. | Use `java`/`node`, or a supported `other:` runtime with commands. |

Diagnose a failing service with `get_service_diagnostics`, which composes the
runtime state, the managed process, the latest health result, recent error logs,
and the declared ports — without inventing a root cause.

## 9. Service detail

The **TUI** service detail view (`Enter` on Services) and the **GUI** service
detail panel show identity, workspace, repository path/URL/branch, Git state,
runtime requirements, process state, health, ports, dependencies, the last task,
and recent errors. See [`tui.md`](tui.md) and [`desktop-gui.md`](desktop-gui.md).

## 10. Limitations

- Managed-process recovery after a daemon crash/restart is not implemented.
- There is no force-stop from the TUI/GUI; the daemon's graceful-then-forced
  policy applies.
- Kubernetes and Docker targets are not implemented.

See [`known-limitations.md`](known-limitations.md).
