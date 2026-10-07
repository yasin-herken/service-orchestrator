# Architecture

Service Orchestrator is a macOS-native developer environment orchestration
platform written in Rust. It prepares and manages local multi-repository,
multi-service development environments: synchronizing repositories, checking
out branches, building shared libraries, running Liquibase migrations, building
and starting applications, inspecting logs, running health checks, and
executing dependency-aware workflows.

It exposes the same capabilities through two interfaces:

- a Ratatui terminal UI for human developers, and
- an MCP server for AI agents.

Both interfaces are adapters over one shared application core.

---

## 1. Product purpose

Reduce the repetitive work of preparing a developer environment. A developer
should be able to select services, synchronize repositories, build libraries,
migrate databases, build and start applications, inspect logs, and run health
checks without remembering the exact `git`/`mvn`/`npm`/`liquibase` incantations
for each repository. AI agents should be able to do the same work through MCP.

The application is a **developer orchestration platform**, not an AI
application. AI is an optional interface.

---

## 2. The core architectural rule

> The application core must not depend on Ratatui or MCP. Both are interface
> adapters over the same application core.

There is exactly one source of truth for behavior:

```text
TUI -> application use case -> task engine -> infrastructure
MCP -> application use case -> task engine -> infrastructure
```

Business logic is never duplicated per interface.

---

## 3. Layers

From innermost to outermost:

1. **Domain** — pure business concepts and rules. No workspace dependencies.
2. **Core services** — the task/workflow engine and the policy engine. They
   depend only on the domain.
3. **Application** — use cases that coordinate the domain and the execution
   engine, plus the port traits that infrastructure implements.
4. **Infrastructure** — concrete adapters: Git, process management, build
   tooling, Liquibase, health checks, logging.
5. **Interfaces** — TUI and MCP adapters.
6. **Composition root** — the `service-orchestrator` binary. It wires concrete
   infrastructure into the application core and hands the core to an interface.

### Architecture diagram

```text
                         Human developer            AI agent
                                |                      |
                                v                      v
                        +---------------+      +---------------+
   Interfaces           |      tui      |      |      mcp      |
                        +-------+-------+      +-------+-------+
                                |                      |
                                +----------+-----------+
                                           |
                                           v
                        +-------------------------------+       future:
   Application          |          application          |<----- cli, api
   (use cases + ports)  |  use cases / orchestration    |
                        +---+-----------+-----------+---+
                            |           |           |
              +-------------+           |           +--------------+
              |                         v                          |
              |              +---------------------+               |
   Core       |              |   execution engine  |               |
   services   |              | tasks / workflows   |               |
              |              +----------+----------+               |
              |                         |                          |
              |              +----------v----------+               |
              |              |    policy engine    |               |
              |              +----------+----------+               |
              |                         |                          |
              +-------------------------+--------------------------+
                                        |
                                        v
                        +-------------------------------+
   Domain               |            domain             |
   (no dependencies)    | concepts, rules, state types  |
                        +-------------------------------+
                                        ^
                                        | implements ports
                        +-------------------------------+
   Infrastructure       |  git  process  build  health   |
   (adapters)           |  liquibase        logging      |
                        +-------------------------------+
                                        ^
                                        | wired by
                        +-------------------------------+
   Composition root     |     service-orchestrator      |
                        +-------------------------------+
```

The important property of this diagram is that arrows point *inward* toward the
domain. Nothing inner knows about anything outer.

---

## 4. Crate responsibilities

| Crate | Layer | Responsibility |
| --- | --- | --- |
| `domain` | Domain | Pure business concepts and rules. Depends on nothing. |
| `execution` | Core service | Task/workflow engine: state, dependency-aware scheduling, bounded concurrency, cancellation, retry, timeout, progress. |
| `policy` | Core service | Permission model and policy engine (`READ`, `SAFE_WRITE`, `DESTRUCTIVE`). |
| `config` | Infrastructure | Versioned configuration loading, parsing, validation, migration. Produces domain configuration values. |
| `application` | Application | Use cases, orchestration, and infrastructure port traits. |
| `git` | Infrastructure | Git adapter implementing the application Git port. |
| `process` | Infrastructure | Local process lifecycle, state, ports, child processes, logs. |
| `build` | Infrastructure | Build tooling (Maven, npm) via structured process execution. |
| `liquibase` | Infrastructure | Liquibase migration adapter. |
| `health` | Infrastructure | Configurable health checks (process, port, HTTP/actuator). |
| `logging` | Infrastructure | Structured logging and bounded, filterable log retrieval. |
| `mcp` | Interface | MCP stdio adapter over the application core. |
| `tui` | Interface | Ratatui adapter over the application core. |
| `service-orchestrator` (root) | Composition root | Wires infrastructure into the core and dispatches to an interface. |

---

## 5. Dependency direction

The intended direction, expressed as allowed Cargo dependencies:

```text
tui  ─────────┐
              │
mcp  ─────────┼──> application ──> execution ──> domain
              │            │    └──> policy  ──> domain
future cli ───┘            │
                           └──> domain

config ─────────────────────> domain

git, process, build, liquibase, health, logging
    ────────────────────────> application + domain   (implement ports)

service-orchestrator (root) ──> everything         (composition root)
```

### Rules

- `domain` depends on no workspace crate.
- `execution` and `policy` depend only on `domain`.
- `application` depends on `domain`, `execution`, and `policy`.
- Infrastructure crates depend on `application` (to implement its ports) and
  `domain`. They must not depend on each other or on interfaces.
- `tui` and `mcp` depend only on `application`.
- The root binary is the only crate allowed to depend on both infrastructure
  and interfaces; it is the composition root.

### Forbidden edges

```text
domain      -> tui / mcp / any infrastructure
application -> ratatui / mcp / concrete infrastructure
tui         -> infrastructure (git, process, build, ...)
mcp         -> infrastructure (git, process, build, ...)
git         -> tui / mcp
```

These edges are not merely documentation: because every dependency is an
explicit workspace dependency, adding a forbidden edge requires editing a
`Cargo.toml` and is therefore visible in review. Cyclic dependencies are
rejected by Cargo at build time.

---

## 6. Why TUI and MCP are adapters

The TUI and the MCP server answer the same questions ("what services exist?",
"build this service", "start this profile") over different transports. If each
implemented its own version of those operations, behavior would drift and every
new capability would have to be built twice.

Instead:

- The TUI reads application state, renders it, and turns user input into
  application commands. It contains no Git, build, process, or workflow logic.
- The MCP server validates tool input, applies policy, and forwards calls to
  application use cases. It contains no business logic and never shells out
  directly.

Both therefore see identical behavior, identical policies, and identical error
semantics. This is also why the permission model lives in `policy` and is applied
by the application and interfaces rather than inside a specific UI.

---

## 7. Why the core must work without an LLM

Service Orchestrator is a developer orchestration platform. A developer must be
able to run:

```bash
service-orchestrator tui
```

with no LLM, no MCP client, no internet, and no remote backend.

The MCP server is optional:

```bash
service-orchestrator mcp
```

Keeping the core free of any AI dependency is what makes this possible, and it
keeps AI-assisted usage a thin, replaceable layer rather than a load-bearing
part of the system.

---

## 8. Where future interfaces fit

A future CLI or HTTP API is just another adapter:

```text
future cli ──> application
future api ──> application
```

It depends only on `application`, reuses the same use cases, and is wired in by
the composition root exactly like `tui` and `mcp`. No core change is required to
add an interface, and no interface may add business logic.

---

## 9. Where service executors belong

Service-specific behavior (how a Java service builds, how a Node service
builds, how a library is installed) is isolated behind a `ServiceExecutor`
abstraction.

- The `ServiceExecutor` trait is defined by the **application** layer, because
  it is part of the use-case vocabulary and must be replaceable by tests.
- The concrete executors (Java, Node, Library, and future Python/Go/Docker/
  Kubernetes) are **infrastructure** implementations wired in by the
  composition root.

Generic orchestration code must never branch on service technology. It asks an
executor for the tasks to run; it does not know how they are built.

---

## 10. Where the task engine belongs

All meaningful operations are tasks. The task engine lives in the `execution`
crate, one layer above the domain and below the application:

- It owns task states (`queued`, `running`, `succeeded`, `failed`, `cancelled`,
  `skipped`, `blocked`), dependency-aware scheduling, and bounded concurrency.
- The **application** layer submits tasks to the engine and supplies concrete
  handlers; it does not implement scheduling itself.
- Interfaces observe task status through application use cases; neither the TUI
  nor MCP owns task execution.

Workflows are compositions of tasks and live in the same engine, so retry,
timeout, cancellation, and failure propagation are defined once.

---

## 11. Enforcement and validation

- Dependencies are declared exclusively through `[workspace.dependencies]` so
  boundaries are visible in one place.
- `cargo check/build` rejects dependency cycles.
- Shared lints (`unsafe_code = "forbid"`, `missing_docs`, `rust_2018_idioms`,
  `unreachable_pub`) are defined once in the workspace and opted into by every
  crate.
- CI/validation runs:

  ```bash
  cargo fmt --check
  cargo check --workspace
  cargo test --workspace
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  ```

---

## 12. Related documents

- `docs/adr/001-core-architecture.md` — the decision record for the core-first
  architecture.
- `AGENTS.md` — the full engineering guidance and product philosophy.
