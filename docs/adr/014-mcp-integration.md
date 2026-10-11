# ADR 014: MCP Server & AI Agent Integration

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

ADR 001 established the core-first architecture and named an MCP server as one of
two interfaces over a single application core; ADR 004 defined that core, and
ADRs 005–013 delivered the task engine, process manager, Git manager, build
manager, library manager, service runtime, migration manager, health and
observability layers, and the Ratatui TUI. Every capability an AI agent needs
already existed behind the application core, but nothing exposed it over a
protocol an agent speaks.

`tasks/014.md` asked for a production-quality, standards-compliant MCP server
that exposes those capabilities, with hard constraints:

- the server is an **adapter**: it validates input, applies policy, and forwards
  calls to application use cases; it must not run Git, Maven, npm, Liquibase, or
  OS processes, or duplicate any business logic;
- the **stdio** transport is primary, and **stdout is reserved exclusively for
  protocol messages**; no HTTP server, daemon, or custom JSON-RPC;
- long-running operations must return real task identifiers and stay
  asynchronous;
- every operation must pass through the same policy engine as the TUI, and
  destructive operations must require explicit human authorization;
- tool inputs are untrusted; strict typed schemas, structured
  `{success, data}` / `{success: false, error}` envelopes, and structured errors
  are required;
- resources (`workspace://`, `service://`, `task://`) and a small set of prompts
  are expected, but only what is implemented may be advertised;
- Kubernetes, Docker, a shell tool, a second engine/manager, and new Ratatui
  screens are out of scope.

## Decision

Add a `service-orchestrator-mcp` crate and wire it into the
`service-orchestrator` binary as `service-orchestrator mcp`, built on the
official Rust SDK `rmcp` **3.5.1** over the stdio transport.

```text
tools/call ─▶ validate args ─▶ Application use case ─▶ policy ─▶ task engine
```

The crate depends only on `rmcp`, `schemars`, `serde`/`serde_json`, `thiserror`,
`tokio`, and the `application`, `domain`, and `execution` crates. It never
depends on an infrastructure crate, so the adapter boundary is enforced by the
manifest rather than by convention.

### Why `rmcp`

`rmcp` is the official Model Context Protocol SDK for Rust. It provides the
server handler trait, the protocol model types, JSON-Schema generation, and the
stdio transport with newline-delimited JSON framing. Using it means the adapter
implements the protocol by implementing a trait rather than by hand — no bespoke
JSON-RPC layer — which is exactly the "standards-compliant, not a custom
protocol" requirement. It is pinned at `3.5.1` with `default-features = false,
features = ["server", "transport-io"]`, so no HTTP, WebSocket, or child-process
transport is compiled in. `rmcp` 3.5.1 requires Rust 1.88 / edition 2024, so the
workspace `rust-version` was raised to `1.88.0`.

### The `ServerHandler` is implemented by hand, not with `#[tool]` macros

The SDK's `#[tool]` macros generate a dispatcher, but they do not give precise
control over two things this task requires: a **structured success/failure
envelope** returned as an `isError` tool result (rather than a protocol error),
and a **distinct protocol-error path** for requests that never routed. The
adapter therefore implements `ServerHandler` manually: it builds each tool with
`Tool::new(name, description, schema_for_input::<T>())`, dispatches `call_tool`
by name, and renders the envelope itself. The result is a small, readable
adapter with an exact error contract.

### stdout is protocol-only; diagnostics go to stderr

On stdio, a single stray `println!` corrupts the stream. The adapter therefore
writes **no** diagnostic to stdout. Lifecycle events and tool invocations are
emitted to stderr as single-line JSON records (server started/stopped, and one
record per tool invocation carrying the tool name, outcome, duration, and error
code — never an argument value). Application logs continue to flow through the
shared `LogService` and are read back through the log tools. Nothing in the
crate logs a secret.

### The core is shared, not duplicated

`src/main.rs` was refactored into a shared `Runtime`: `bootstrap()` loads and
validates configuration, constructs the infrastructure adapters, registers their
executors, builds one task engine, spawns the log ingestors, wires the
application core, and returns it. `run_tui` and `run_mcp` each bootstrap, hand
the same core to their adapter, and shut the core down in the same order
(ingestors, engine, processes). `Application::new` already defaults to standard
authorization, so the MCP server inherits the policy model with no special
wiring, and an operation an agent drives behaves identically to one a human
drives in the TUI.

### Structured envelopes and two failure kinds

Every tool result is one of:

```json
{ "success": true,  "data": { ... } }
{ "success": false, "error": { "code": "...", "message": "...", "details": { ... } } }
```

A success is a normal result; a failure is a result with `isError: true`. A
**protocol error** (`Err(ErrorData)`) is reserved for requests that could not be
routed at all: an unknown tool, or arguments whose JSON shape is wrong (a wrong
type, a missing required property, or — because every request struct sets
`deny_unknown_fields` — an unmodelled option). A valid route that the application
refuses or that fails produces the structured envelope instead, so a valid but
invalid *domain value* (say, an identifier with an illegal character) yields a
readable `INVALID_ARGUMENT` rather than an opaque protocol error.

`ApplicationError` is mapped to a stable code, reusing the application's own code
where it already defines one (Git, build, migration, health, artifact) and giving
every other variant a dedicated one. The mapping is exhaustive with no catch-all,
so a new error variant cannot silently degrade to a generic code.

### Policy: destructive work cannot be self-confirmed

The MCP server always runs with `Authorization::standard`. Destructive tools are
advertised (so agents can discover them and so the behavior is real and
testable) but the application denies them with
`ApplicationError::PermissionDenied { permission: Destructive, .. }`, which the
adapter reports as `CONFIRMATION_REQUIRED`. There is deliberately **no
`confirmed` parameter**: accepting a caller-supplied confirmation would let an
agent authorize its own destructive action, which `AGENTS.md` §30 forbids. A
human must approve destructive work interactively.

### Resources and prompts

Resources are read-only and backed by existing application queries. Concrete
resources are bounded (one metadata resource plus the configured workspaces);
workspace-scoped views, services, and tasks are RFC 6570 templates so the list
does not grow with the number of services or tasks. Prompts return guidance only
— which tools to call, in what order — and reference only tools that exist; they
carry no business logic.

### Only what is implemented is advertised

Tools are advertised only where an application use case exists. There is
deliberately no `install_dependencies`, `test_service`, or combined
"build selected services" tool, because the application core exposes no such use
case; the gaps are documented in `docs/mcp.md` rather than faked (`AGENTS.md`
§19).

### Listing tasks through a narrow observer

`list_tasks` needs the engine's list/event API, which the application boundary
does not expose. Following the TUI's `TaskObserver` precedent, the adapter
defines a small interface-facing `TaskMonitor` trait (`tasks()`,
`recent_tasks(limit)`), implemented by the concrete `TaskEngine` and injected by
the composition root. The application boundary stays narrow.

## Alternatives considered

- **Build the protocol by hand.** Rejected: it would reimplement framing,
  capability negotiation, and the model types — a large surface for defects, and
  the opposite of "standards-compliant, not a custom JSON-RPC layer".
- **Use the SDK's `#[tool]` macros.** Rejected: they obscure the structured
  envelope and the protocol-error/`isError` distinction the task requires, and
  make the error contract implicit.
- **A `run_shell` escape hatch for unsupported operations.** Rejected by
  `AGENTS.md` §23: an agent must not have to construct shell commands, and the
  adapter must not execute arbitrary processes.
- **Implement `install_dependencies` / `test_service` in the adapter.** Rejected:
  it would put business logic in the interface and duplicate or invent behavior.
  The gaps are documented instead.
- **Accept a `confirmed: true` argument to allow destructive operations.**
  Rejected: the policy engine must remain the sole authority, and an interface
  must never be able to self-authorize a destructive action.
- **Return tasks synchronously.** Rejected: it would block the transport and
  duplicate the engine's scheduling. Every mutation returns the task or workflow
  id the application already produced.
- **Expose the engine's full API to the adapter.** Rejected: a narrow
  `TaskMonitor` boundary keeps the application core authoritative and mirrors the
  existing `TaskObserver`.
- **Log tool arguments for observability.** Rejected: an argument can carry a
  secret; the audit log records only the tool name, outcome, duration, and error
  code.

## Consequences

### Positive

- One application core, two interfaces, no duplicated behavior: an agent and a
  human observe identical state, policies, and error semantics.
- The adapter boundary is enforced by the dependency graph: adding a Git or Maven
  call would require an impossible dependency edge.
- A destructive operation cannot be triggered by an agent; the policy engine is
  the single authority and confirmation stays with a human.
- stdout is guaranteed to carry protocol messages only, and diagnostics are
  structured on stderr.
- Long-running work is asynchronous and observable: every mutation returns a real
  task or workflow id, and `get_task` / `task://{taskId}` expose status,
  progress, and structured failure.

### Negative / trade-offs

- The `TaskMonitor` trait is a second, narrow boundary in addition to
  `TaskManager`; it exists because the engine's list API is genuinely needed by
  a read-only observer.
- Implementing `ServerHandler` by hand is more code than the macros, accepted in
  exchange for an exact, testable error contract.
- Task and runtime state are **per-process**. Two MCP sessions are two engines;
  a task id from one is not visible in the other. A client should use a single
  long-lived session per workspace. This mirrors the TUI and is documented, not
  worked around.
- The workspace raises `rust-version` to `1.88.0` to satisfy `rmcp` 3.5.1.
- Simple resource snapshots use the application's native field names while the
  envelope, task, and log views are `camelCase`; the inconsistency is documented
  rather than papering a re-serialization over every model.

## Follow-up

- If the application gains `install_dependencies`, `test_service`, or a combined
  bulk-build use case, the corresponding tools can be added without touching the
  adapter's structure.
- If a cross-process or persisted runtime state store replaces the in-memory one,
  `get_task` and the task resources can reflect state beyond a single process.
- Other MCP transports (for example streamable HTTP) can be added behind the
  existing `serve_stdio` seam without changing the adapter.
