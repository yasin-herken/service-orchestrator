# Development

How to work on Service Orchestrator: the repository layout, how to run each
interface, how the pieces fit, and how to add a new operation. For the design
rationale, read [`architecture.md`](architecture.md) and the ADRs in
[`adr/`](adr/).

## 1. Repository layout

```text
Cargo.toml                 # workspace root + the composition-root package
rust-toolchain.toml        # stable + clippy + rustfmt
crates/
├── domain/                # pure business concepts and rules
├── execution/             # task and workflow engine
├── policy/                # permission model and policy engine
├── config/                # versioned configuration (+ management/inspection)
├── application/           # use cases, orchestration, ports
├── git/                   # infrastructure: Git adapter
├── process/               # infrastructure: OS process owner
├── build/                 # infrastructure: Maven/npm build provider
├── runtime/               # infrastructure: Java/Node service runtime
├── liquibase/             # infrastructure: Liquibase migration provider
├── health/                # infrastructure: process/TCP/HTTP health
├── logging/               # infrastructure: unified log hub
├── client/                # OrchestratorClient boundary + IPC transport
├── daemon/                # shared local runtime (IPC server)
├── mcp/                   # interface: MCP stdio server
└── tui/                   # interface: Ratatui TUI
apps/desktop/              # interface: Tauri 2 GUI (own Cargo workspace)
src/                       # composition root (service-orchestrator binary)
examples/config.toml       # example configuration
scripts/                   # test and validation scripts
docs/                      # documentation
tests/                     # cross-crate end-to-end tests (mcp_stdio.rs)
```

## 2. The Cargo workspace

- The root package is the **composition root**. It parses the command line,
  builds the core for `daemon`, and hands a client to an interface for `tui`/`mcp`
  (`src/main.rs`). No business logic lives here.
- `crates/*` are workspace members; internal dependencies are declared once under
  `[workspace.dependencies]` so boundaries are visible in one place.
- `apps/desktop/src-tauri` is **excluded** from the root workspace: Tauri drags in
  a large dependency tree the backend should not pay for. It consumes the
  workspace crates as path dependencies.
- Shared lints (`unsafe_code = "forbid"`, `missing_docs`, `rust_2018_idioms`,
  `unreachable_pub`) are defined once and opted into by every crate.

The dependency direction is enforced by the manifests
([`architecture.md`](architecture.md) §5): arrows point inward toward `domain`;
the core never depends on an interface.

## 3. Running each interface

```bash
# Build everything.
cargo build

# Composition root.
cargo run --bin service-orchestrator -- --help
cargo run --bin service-orchestrator -- daemon          # foreground daemon
cargo run --bin service-orchestrator -- daemon status
cargo run --bin service-orchestrator -- tui             # terminal UI
cargo run --bin service-orchestrator -- mcp             # MCP on stdio

# Desktop GUI (its own workspace).
cd apps/desktop
npm install
npm run tauri dev            # preflight builds the daemon binary, then opens the window
npm run dev                  # frontend only (Vite on :5173)
```

All three interfaces connect to the same daemon. `tui`/`mcp` auto-start it; the
GUI's adapter locates the binary and the client auto-starts it.

## 4. The application core

`service-orchestrator-application` is the one API every interface calls. It holds
the validated `Config` plus its boundaries (`TaskManager`, `RuntimeStateStore`,
`LogService`, `PolicyEngine`) and exposes **commands** (mutating) and **queries**
(read-only). Every command authorizes via the policy engine before scheduling.
See [`application-core.md`](application-core.md).

Key ideas:

- Results are structured values, never pre-formatted strings.
- Long-running operations are submitted to the task engine and return a task id
  or workflow id immediately.
- System facts (filesystem, `PATH`, ports, Git) arrive through **ports**, so the
  core never touches the OS directly. The composition root supplies the real
  implementations.

## 5. Infrastructure managers

Each manager follows the same shape:

- A small, pure **provider/runtime/checker** abstraction selects an
  implementation (Maven/npm; Java/Node; process/TCP/HTTP; Liquibase) and emits a
  structured **plan** (program + typed args + env + working dir).
- The manager hands the plan to the **process manager** — the single process
  owner — which runs it supervised, cancellable, and timeout-bounded.
- Mutating operations are **tasks**; the crate registers a `TaskExecutor` against
  the relevant `TaskKind`.

This is why adding a provider does not require touching the engine, the
application, or the interfaces. Details: [`build-manager.md`](build-manager.md),
[`service-runtime.md`](service-runtime.md),
[`migration-manager.md`](migration-manager.md),
[`health-manager.md`](health-manager.md), [`git-manager.md`](git-manager.md).

## 6. The client, daemon, and IPC contract

- `service-orchestrator-client` defines `OrchestratorClient`, a one-to-one mirror
  of the application's use cases. `Application` implements it in-process (so
  interface tests need no daemon); `RemoteClient` implements it over the socket.
- The **daemon** (`service-orchestrator-daemon`) binds a Unix domain socket,
  validates the protocol version, routes each request to the shared
  `Application`, and fans task/log/configuration events to clients.
- The wire protocol is **length-prefixed JSON** (big-endian `u32` length + UTF-8
  JSON), capped at 8 MiB, versioned (`PROTOCOL_VERSION`). Requests are explicit,
  versioned operations — the daemon never proxies a generic Rust surface.
- The MCP and TUI adapters depend only on `client` (plus `domain`/`execution`
  value types), never on the daemon or an infrastructure crate.

See [`local-daemon.md`](local-daemon.md) for the protocol and lifecycle.

## 7. Adding a new operation

The path for a new capability, innermost out:

1. **Domain** — add any new value types or state the operation needs.
2. **Application** — add a use case (a command in `commands/` or a query in
   `queries/`), classify its `Permission`, and apply the policy check. If it is
   long-running, submit a `TaskDefinition`; if it needs a system fact, add a port.
3. **Infrastructure** — if the operation runs a tool, implement it behind the
   relevant provider abstraction and register a `TaskExecutor`; route process
   execution through the process manager.
4. **Client** — add the method to `OrchestratorClient` and its wire operation
   name; implement it for `Application` and `RemoteClient`.
5. **Daemon** — add the routing arm (`crates/daemon/src/dispatch.rs`).
6. **Interfaces** — add a TUI action/key, a Tauri command (plus a React call), and
   an MCP tool with a strict schema, each delegating to the use case.
7. **Tests** — unit tests for the rule, an application test for the use case, an
   MCP/daemon test for the adapter, and a frontend test if the GUI changed.
8. **Docs** — update the relevant guide and, if the architecture changed, an ADR.

Do not put business logic in an interface; do not spawn a process outside the
process manager; do not add a config field without validating it.

## 8. Debugging

- **Daemon** — run it in the foreground
  (`service-orchestrator daemon --config <path>`); errors go to stderr. An
  auto-started daemon logs to `<config_dir>/run/daemon.log`.
- **IPC** — `daemon status` reports connected clients, active tasks, and managed
  services. The protocol version and socket path are shown.
- **TUI** — the Tasks and Logs screens show engine state; `R` refreshes the
  snapshot.
- **MCP** — the adapter writes a one-line JSON audit record per invocation to
  **stderr** (tool name, outcome, duration, error code — never arguments).
- **GUI** — `npm run tauri dev`; the connection banner and the daemon status
  panel report connectivity. The webview talks only to Tauri commands.
- **Logs** — use the unified log view; secrets are redacted best-effort.

## 9. Testing, formatting, linting

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Plus the categorized scripts and the documentation check:

```bash
./scripts/test-unit.sh
./scripts/test-integration.sh
./scripts/test-security.sh
./scripts/test-frontend.sh
./scripts/test-e2e.sh          # RUN_DESKTOP_TESTS=1 / INCLUDE_PERFORMANCE=1 opt-ins
./scripts/test-all.sh
./scripts/check-docs.sh
```

See [`testing.md`](testing.md) for the layers, fixtures, and prerequisites.

## 10. Conventions

- English only, in code and documentation.
- Structured process execution (`Command::new(program).args([...])`), never a
  shell string.
- `Result`-based errors with typed variants and stable codes.
- No abstraction without a concrete reason; no giant enums, no monolithic
  `main.rs`.
- Preserve working behaviour; keep changes scoped; prefer incremental
  improvements.
