# The MCP Server

Service Orchestrator can expose its whole capability set to AI agents through
the Model Context Protocol:

```bash
service-orchestrator mcp
```

`mcp` is an **interface adapter**, not a second implementation. It reaches the
same application core the Ratatui TUI reaches — in production both are clients
of one shared local daemon — so an agent and a human driving the TUI see
identical behavior, identical policies, and identical error semantics, and an
operation either starts is immediately visible to the other (`AGENTS.md` §1,
§21; `docs/adr/001-core-architecture.md`, `docs/local-daemon.md`). It is
entirely optional: the TUI remains fully functional with no LLM and no MCP
client (`AGENTS.md` §2).

---

## 1. Connection model

The server uses the **stdio transport**, which is the primary local transport for
MCP:

```text
MCP client (Claude, an IDE, an agent framework, ...)
        │  stdin / stdout
        ▼
service-orchestrator mcp ──► OrchestratorClient ──► (IPC) ──► daemon
                                                              ──► application
                                                              ──► policy
                                                              ──► task engine
```

- The client launches the process and speaks newline-delimited JSON-RPC over its
  stdin/stdout.
- **stdout carries protocol messages only.** Every diagnostic the adapter emits
  goes to **stderr** as a single-line JSON record (§7). A stray `println!` would
  corrupt the stream, so none exists (`tasks/014.md` §6).
- There is no HTTP server and no custom JSON-RPC layer; the protocol is
  implemented by the official Rust SDK.
- The adapter is a **client of the shared local daemon** over a Unix domain
  socket. Starting `mcp` connects to a running daemon or starts one if needed;
  because the daemon owns the one task engine and process registry, a task an
  agent starts here is visible and controllable from the TUI (and vice versa).
  Closing the MCP session never stops the daemon
  (`docs/local-daemon.md`, `docs/adr/015-shared-local-runtime.md`).

### Launching it

```bash
service-orchestrator mcp
service-orchestrator mcp --config /path/to/config.toml
service-orchestrator mcp --config=/path/to/config.toml
```

Configuration resolution is the same as the TUI: the command line wins, then the
`SERVICE_ORCHESTRATOR_CONFIG` environment variable, then
`~/.service-orchestrator/config.toml`. The binary has one composition root
(`src/main.rs`). The **daemon** owns the wired core: `bootstrap()` loads and
validates configuration, builds the infrastructure adapters, registers their task
executors, builds the task engine, spawns the log ingestors, and wires the
application core. The `tui` and `mcp` commands do not build a core of their own;
they resolve the same config path, derive the socket location from it, connect a
`RemoteClient` to the daemon (starting one if necessary), and hand that client to
their adapter (`tasks/015.md` §4, §5).

A typical client registration is a stdio server whose command is the binary with
the `mcp` subcommand:

```json
{
  "mcpServers": {
    "service-orchestrator": {
      "command": "/usr/local/bin/service-orchestrator",
      "args": ["mcp", "--config", "/Users/me/.service-orchestrator/config.toml"]
    }
  }
}
```

### The SDK

The adapter is built on the official Rust SDK, [`rmcp`](https://crates.io/crates/rmcp)
**3.5.1**, pinned in the workspace manifest:

```toml
rmcp = { version = "3.5.1", default-features = false, features = ["server", "transport-io"] }
schemars = "1.0"
```

- `server` supplies `ServerHandler`, the model types, and `schema_for_input`.
- `transport-io` supplies `rmcp::transport::stdio()` and the newline-delimited
  JSON framing.
- `default-features = false` keeps the dependency surface minimal; no HTTP,
  WebSocket, or child-process transport is built.
- `schemars` is used directly to derive each tool's input schema from its
  parameter struct (`schema_for_input::<T>()`).
- `rmcp` 3.5.1 requires Rust 1.88 and edition 2024, so the workspace
  `rust-version` is `1.88.0`.

The `service-orchestrator-mcp` crate is a leaf: it depends only on `rmcp`,
`schemars`, `serde`/`serde_json`, `thiserror`, `tokio`, and the `application`,
`client`, `domain`, and `execution` crates. It never depends on an
infrastructure crate (`git`, `build`, `process`, `liquibase`, `health`,
`logging`), so the adapter boundary is enforced by the manifest rather than by
convention. It reaches the core through the `client` boundary, so it does not
know whether that core is local or behind the daemon socket
(`tasks/015.md` §13, §21).

---

## 2. Architecture

```text
crates/mcp/src/
├── lib.rs        serve_stdio, ServeError, ToolError, parse helpers, constants
├── server.rs     OrchestratorServer: ServerHandler impl + the tool dispatcher
├── catalog.rs    the advertised tool catalog (names, descriptions, schemas, annotations)
├── params.rs     strict, typed input structs (one per tool shape)
├── response.rs   the success/failure envelope, error mapping, camelCase views
├── resources.rs  read-only resources and RFC 6570 templates
├── prompts.rs    standardized developer workflows (guidance only)
├── monitor.rs    the read-only TaskMonitor boundary over the task engine
├── logging.rs    the stderr audit log
└── tests.rs      adapter unit tests (discovery, envelopes, policy, redaction)
```

The flow is one-directional, exactly as in the TUI:

```text
tools/call ─▶ validate args ─▶ Application use case ─▶ policy ─▶ task engine
```

`OrchestratorServer` implements `rmcp::ServerHandler` by hand (rather than using
the `#[tool]` macros) so it controls the structured envelope and the `isError`
flag precisely. `call_tool` deserializes the arguments into a typed request,
`dispatch` matches on the tool name, calls exactly one application method, and
`response.rs` renders the result.

### Two kinds of failure

The protocol separates a request that could not be *routed* from one that ran and
*failed*, and the adapter follows that split exactly (`tasks/014.md` §16, §28):

| Situation | Wire result |
|---|---|
| Unknown tool, malformed arguments (wrong shape, wrong type, unknown property) | JSON-RPC **protocol error** (`Err(ErrorData)`) |
| Valid route, invalid domain value (e.g. an illegal identifier) | tool result, `isError: true`, envelope `code: "INVALID_ARGUMENT"` |
| Valid route, application refusal or failure (including policy denial) | tool result, `isError: true`, structured envelope |

A malformed call never reaches the application. A valid-but-invalid *domain
value* does route, so the caller gets a readable error rather than an opaque
protocol error.

---

## 3. Tools

Every tool is a thin, named projection of exactly **one** application use case.
The adapter never invents behavior, never shells out, and never keeps a second
engine. The granularity follows `AGENTS.md` §24: **primitive** (Git, migration),
**service** (build, start, restart), and **workflow** (prepare, sync, build).

Input schemas are strict and typed. Each tool's schema is derived from a
`params.rs` struct declared with `#[serde(rename_all = "camelCase",
deny_unknown_fields)]`, so an unmodelled option cannot sneak past validation. An
agent never has to construct a shell command: there is deliberately **no
`run_shell`** tool (`AGENTS.md` §23).

### Workspace, group, and profile

| Tool | Permission | Returns |
|---|---|---|
| `list_workspaces` | read | every configured workspace |
| `get_workspace` | read | one workspace |
| `get_workspace_status` | read | a workspace status snapshot (services + state) |
| `list_groups` / `get_group` | read | service groups |
| `list_profiles` / `get_profile` | read | developer profiles |

### Services

| Tool | Permission | Returns |
|---|---|---|
| `list_services` | read | services in a workspace |
| `get_service` | read | one service definition |
| `get_service_status` | read | structured status (Git, runtime, health) |
| `get_service_runtime_status` | read | last observed process/runtime state |
| `list_running_services` | read | currently running services |
| `build_service` | safe write | task id |
| `start_service` / `stop_service` / `restart_service` | safe write | task id |
| `start_services` / `stop_services` / `restart_services` | safe write | workflow id |

> `start_services`, `stop_services`, and `restart_services` operate on exactly
> the supplied `serviceIds`. An **empty** list is not "all services": it produces
> `already_in_desired_state` with nothing scheduled. Pass explicit ids, or use a
> group/profile workflow to act on a defined set.

### Git

| Tool | Permission | Returns |
|---|---|---|
| `git_status` | read | repository status |
| `git_list_branches` | read | branches |
| `git_diff` | read | repository diff |
| `git_fetch` / `git_pull` | safe write | task id |
| `git_checkout` | safe write | task id |
| `git_sync` | safe write | task id (fetch + checkout against the configured branch) |
| `git_clone` | safe write | task id |
| `git_sync_repositories` / `git_fetch_repositories` / `git_checkout_repositories` | safe write | workflow id |
| `git_reset_hard` | **destructive** | `CONFIRMATION_REQUIRED` under the MCP server |
| `git_clean` | **destructive** | `CONFIRMATION_REQUIRED` |
| `git_discard_changes` | **destructive** | `CONFIRMATION_REQUIRED` |

### Builds, libraries, and migrations

| Tool | Permission | Returns |
|---|---|---|
| `build_workspace` | safe write | workflow id |
| `build_library` / `install_library` / `prepare_library` | safe write | task id |
| `prepare_selected_libraries` | safe write | workflow id |
| `list_libraries` / `get_library` | read | libraries |
| `get_library_status` | read | configuration + cached Git state |
| `find_library_dependents` | read | services/libraries that consume a library |
| `verify_library_artifact` | read | local artifact presence |
| `inspect_library_version_alignment` | read | consumer version alignment |
| `migration_status` / `validate_migration` | read | configured migration summary |
| `get_migration_status` | read (async) | a live migration-status task result |
| `inspect_migration_pending` | read (async) | a live pending-changeset task result |
| `preview_migration` | read | a preview task id |
| `apply_migration` | safe write | task id |
| `apply_selected_migrations` | safe write | workflow id |

> Migration reads that need a live database probe are scheduled tasks
> (`get_migration_status`, `inspect_migration_pending`). `apply_migration`
> mutates the database and flows through the same policy path as every other
> operation.

### Runtime, health, logs, tasks, workflows

| Tool | Permission | Returns |
|---|---|---|
| `check_service_health` | safe write | task id |
| `check_service_readiness` | safe write | task id |
| `get_service_health` | read | last observed health |
| `list_service_health` | read | health for every service |
| `get_service_diagnostics` | read | structured diagnostics |
| `get_service_logs` / `get_task_logs` | read | a bounded log page |
| `get_logs` | read | a bounded list of log entries |
| `get_task` | read | a task snapshot (status, progress, failure) |
| `list_tasks` | read | running tasks + recent history |
| `cancel_task` | safe write | cancellation request |
| `summarize_workflow` | read | aggregated per-step outcomes |
| `sync_workspace` | safe write | workflow id |
| `prepare_environment` | safe write | workflow id |
| `start_group` / `stop_group` | safe write | workflow id |
| `start_profile` / `stop_profile` | safe write | workflow id |

Tool descriptions state what each one **does**, whether it **mutates** state, and
what it **returns**, so an agent can plan without guessing.

### Documented gaps

Tools are advertised only where an application use case already exists.
`AGENTS.md` §19 forbids inventing business logic inside the interface, so three
operations an agent might expect are deliberately **absent** and documented
instead of faked:

- **`install_dependencies`** — the application exposes no "resolve dependencies"
  use case. A service installs through `build_service`; a library through
  `install_library` / `prepare_library`.
- **`test_service`** — there is no test use case.
- **`build_selected_services`** (as one workflow) — the application has no
  combined bulk-build use case. Call `build_service` per service, or use
  `build_workspace` to build the whole workspace.

See `docs/adr/014-mcp-integration.md` for the reasoning.

---

## 4. Resources

Resources are the read side of the protocol (`AGENTS.md` §25). They let an agent
fetch structured state as a document instead of parsing tool output. Every
resource is backed by an existing application query; none mutates state and none
reaches past the application core.

Concrete resources are bounded: one static metadata resource plus the
(configuration-driven, small) set of workspaces.

| URI | Contents |
|---|---|
| `orchestrator://server` | server name, version, transport |
| `workspace://{workspaceId}` | one workspace |

Workspace-scoped views, services, and tasks are offered as **RFC 6570
templates**, so the list stays bounded no matter how many services or tasks
exist (`tasks/014.md` §24, §34):

| Template | Contents |
|---|---|
| `workspace://{workspaceId}/status` | a workspace status snapshot |
| `workspace://{workspaceId}/services` | the workspace's services |
| `workspace://{workspaceId}/libraries` | the workspace's libraries |
| `service://{serviceId}` | a service status |
| `service://{serviceId}/diagnostics` | structured diagnostics |
| `task://{taskId}` | a task snapshot (status, progress, failure) |

A read has no success/failure envelope, so a missing or malformed identifier
becomes a `resource not found` protocol error and any other failure becomes an
internal error; the message is the agent-facing summary and no lower-level detail
is leaked.

---

## 5. Prompts

Prompts are standardized starting points for common developer workflows. A prompt
returns a single user message that names the tools to call and the order to call
them in. It contains **no business logic**; it only guides an agent toward the
application capabilities that already exist (`AGENTS.md` §26).

| Prompt | Arguments | Guides toward |
|---|---|---|
| `prepare-development-environment` | `workspaceId` (required), `serviceIds` | `get_workspace_status` → `prepare_environment` → poll `get_task` |
| `sync-workspace` | `workspaceId` (required) | `git_sync_repositories` / `sync_workspace` → `summarize_workflow` |
| `debug-service` | `serviceId` (required) | status, runtime, health, diagnostics, logs |
| `inspect-unhealthy-service` | `serviceId` (required) | `check_service_health` → diagnostics + error logs |
| `inspect-failed-build` | `serviceId` (required) | status → filtered build logs → `git_sync` → `build_service` |

---

## 6. Permissions, policy, and safety

MCP input is untrusted and is treated as such. The adapter cannot bypass any
safety rule the application enforces (`AGENTS.md` §14, §29, §30, §31):

- **The policy engine is the authority.** The daemon wires the `PolicyEngine`
  and runs it for every request. The MCP client is a **non-interactive** peer,
  so the daemon never grants it destructive authorization: MCP can read and stage
  but cannot perform a destructive operation. A caller-supplied confirmation is
  not part of the protocol and would be ignored — only an interactive TUI peer
  can carry the `allow_destructive` flag, and even then the policy engine still
  decides.
- **A destructive tool is denied, not confirmed.** `git_reset_hard`, `git_clean`,
  and `git_discard_changes` return a structured `CONFIRMATION_REQUIRED` error.
  There is **no `confirmed` flag** for an agent to set: accepting a
  caller-supplied `confirmed: true` would let an agent self-authorize, which the
  policy model forbids. A human must approve destructive work interactively
  (`tasks/015.md` §23).
- **Every input is validated.** Identifiers are parsed into strongly typed domain
  values; a value with an illegal character is a structured `INVALID_ARGUMENT`,
  and an unmodelled option is rejected by `deny_unknown_fields`.
- **Secrets are never exposed.** Service environment values marked secret
  serialize as `***` in the domain, so they cannot leak through a tool result, a
  resource, a log, or an error (`AGENTS.md` §31).
- **No arbitrary shell.** There is no `run_shell` tool; a mutation is always an
  explicit, named use case.
- **Structured errors are authoritative.** An agent should read `error.code`
  rather than retrying blindly; the failure envelope is the contract.

The same rules apply to the TUI. Because both interfaces call the same
application core, an operation a human sees in the TUI behaves identically
through MCP.

---

## 7. Observability

The MCP transport reserves stdout for the protocol, so the adapter writes a
single-line JSON audit record to **stderr** for each lifecycle event and tool
invocation:

```json
{"event":"mcp_server_started","transport":"stdio"}
{"event":"mcp_tool_invocation","tool":"build_service","outcome":"success","durationMs":3,"errorCode":null}
{"event":"mcp_tool_invocation","tool":"git_reset_hard","outcome":"application_error","durationMs":0,"errorCode":"CONFIRMATION_REQUIRED"}
{"event":"mcp_server_stopped","reason":"Closed"}
```

A record contains the tool **name**, the **outcome** (`success`,
`application_error`, `protocol_error`), the **duration**, and — on failure — a
stable **error code**. No argument value is ever logged, so no secret can reach
the log this way.

Application logs are separate and richer: they flow through the shared
`LogService`, exactly as they do for the TUI, and are read back through
`get_service_logs`, `get_task_logs`, and `get_logs` (bounded pages).

---

## 8. Long-running work

Every mutating tool returns **immediately** with the identifier the application
core produced — the adapter never invents one and never blocks on execution:

```json
{
  "success": true,
  "data": { "outcome": "task_scheduled", "taskId": "42", "kind": "build", "status": "queued" }
}
```

An agent then polls:

- `get_task` (or reads `task://{taskId}`) for status, progress, duration, and any
  failure;
- `get_task_logs` for the task's output;
- `summarize_workflow` for the aggregated outcome of a workflow's steps.

The task view is `camelCase` and reports `status`, `progress.current`/`total`/
`fraction`/`message`, `durationMs`, and a structured `error { code, message,
detail }`.

### State is shared through the daemon

The task engine and runtime state live in the **daemon**, not in the MCP
process. Every `mcp` and `tui` client connects to the same daemon over a Unix
domain socket, so a task id created through one session is visible and
controllable through another, and through the TUI
(`docs/local-daemon.md`, `docs/adr/015-shared-local-runtime.md`). Starting a
second MCP session does not create a second engine; if it cannot reach the
daemon it fails with a structured `DAEMON_UNAVAILABLE`, never a silent private
engine.

The daemon owns the state for its own lifetime. It is shut down explicitly with
`service-orchestrator daemon stop`; closing an MCP client disconnects only that
client and leaves the daemon and the developer's services running.

### `migration_status` vs `get_migration_status`

`migration_status` reports configuration and the last observed values, so its
`pending`/`locked` fields may be `null`. The live database probe is a scheduled
task: call `get_migration_status` or `inspect_migration_pending`, then read the
resulting task.

---

## 9. Example agent workflows

**Prepare an environment**

1. `get_workspace_status` with `{ "workspaceId": "acme" }`.
2. `prepare_environment` with `{ "workspaceId": "acme", "runLiquibase": true,
   "build": true, "start": true }` → `workflowId`.
3. Poll `summarize_workflow` until the steps finish; read `get_task` for any
   failed task's structured error.

**Debug an unhealthy service**

1. `check_service_health` with `{ "serviceId": "auth-service", "scope": "all" }`.
2. `get_service_diagnostics` and `get_service_runtime_status`.
3. `get_service_logs` with `{ "serviceId": "auth-service", "level": "error" }`.
4. Report which signal (process, port, HTTP) failed and why.

**Investigate a failed build**

1. `get_service_status` to confirm the last build failed.
2. `get_service_logs` with `{ "serviceId": "web-ui", "source": "build", "level":
   "error" }`.
3. If the repository may be stale, `git_status` then `git_sync`.
4. `build_service` to rebuild → `taskId`; poll `get_task`.

**A destructive request**

1. `git_reset_hard` returns `{ "success": false, "error": { "code":
   "CONFIRMATION_REQUIRED", ... } }`.
2. The agent must stop and ask a human to perform the reset interactively. It
   cannot confirm on its own.

---

## 10. Testing

- **Adapter unit tests** (`crates/mcp/src/tests.rs`) drive the real adapter over
  a real `Application` wired to a real task engine (with **no executors
  registered**, so a scheduled task is observable but never runs). They cover:
  tool discovery and schema shape, unique names, risk annotations, dispatch
  coverage for every advertised tool, structured responses, structured errors
  (not-found, invalid argument, protocol error, unknown property), policy
  denial → `CONFIRMATION_REQUIRED`, real task scheduling + query +
  cancellation, resource templates and reads, prompt rendering and required
  arguments, the envelope shape, and **secret redaction**.
- **End-to-end stdio test** (`tests/mcp_stdio.rs`) launches the **real binary**
  as a child process against an isolated daemon it also owns, and speaks
  newline-delimited JSON-RPC over the client's stdin/stdout against an isolated,
  generated configuration. It performs the `initialize` handshake, capability
  discovery, `tools/list`, `resources/templates/list`, `prompts/list`, a
  read-only call, a mutating call that returns a real task id, a task query, a
  structured application error, a protocol error for a malformed request, a
  `CONFIRMATION_REQUIRED` denial, and a clean shutdown — asserting that **every
  line on stdout is valid JSON**, that the startup record is on stderr, that the
  **daemon keeps running after the MCP client exits**, and that an explicit
  `daemon stop` shuts it down (`tasks/015.md` §29, §30).

No test starts a real Git, Maven, npm, or Liquibase process, and no test touches
a real repository.

---

## 11. Limitations

- The interface is read-only where the application exposes no safe update path.
  It never edits configuration files, and it never rewrites Maven POMs or
  migration files.
- The documented gaps above (`install_dependencies`, `test_service`, a combined
  bulk build) are intentional: no business logic is added to the adapter.
- Task and runtime state are shared through the daemon (§8); the adapter keeps no
  engine of its own. If the daemon cannot be reached, an operation fails with the
  structured `DAEMON_UNAVAILABLE` rather than silently running against a private
  engine. Kubernetes and Docker remain out of scope for this interface.
- Simple resource snapshots (workspace, service, task) are serialized with the
  application's native field names, while the envelope, task, and log views are
  `camelCase`. A client should read the field names the schema advertises rather
  than assume one convention everywhere.
- The interface returns bounded log pages; it never streams an unbounded log.

See `docs/adr/014-mcp-integration.md` for the design rationale and the
alternatives considered.
