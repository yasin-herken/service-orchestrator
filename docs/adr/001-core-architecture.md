# ADR 001: Core-first architecture with TUI and MCP as adapters

- **Status:** Accepted
- **Date:** 2026-10-07
- **Deciders:** Service Orchestrator maintainers

## Context

Service Orchestrator prepares and manages local development environments for
multiple repositories and microservices. It must be usable in two very
different ways:

- interactively, by a human developer through a terminal UI, and
- programmatically, by an AI agent through MCP.

The product is primarily a developer orchestration platform. AI assistance is
optional, and a developer must be able to use the terminal UI with no LLM, no
MCP client, no internet, and no remote backend.

The operations involved are non-trivial and stateful: Git synchronization,
branch checkout, library and service builds, Liquibase migrations, process
start/stop, health checks, log retrieval, and dependency-aware workflows. Many
of these operations are destructive if misused (for example discarding local
changes), so both interfaces must apply identical safety rules.

The naive approach is to build the terminal UI first and later bolt on an MCP
server, or vice versa. That leads to two implementations of the same behavior
that drift over time, inconsistent safety semantics between human and AI access,
and a growing inability to add further interfaces.

## Decision

We adopt a **core-first architecture** in which a single application core is the
only source of truth for behavior, and both interfaces are thin adapters over
that core.

- The **domain** holds pure business concepts and rules.
- The **application** layer owns use cases, orchestration, and the port traits
  that infrastructure implements.
- The **task/workflow engine** (`execution`) and the **policy engine** (`policy`)
  are core services that depend only on the domain.
- **Infrastructure** (Git, process, build, Liquibase, health, logging,
  configuration) implements application ports.
- The **TUI** and **MCP** are adapters that call application use cases and
  contain no business logic.
- A single **composition root** wires concrete infrastructure into the core and
  dispatches to an interface.

Concretely:

```text
tui  ───────┐
            │
mcp  ───────┼──> application ───> execution ──> domain
            │                └──> policy    ──> domain
future cli ─┘

infrastructure (git, process, build, liquibase, health, logging)
    ──> application + domain   (implement ports)
```

The dependency direction points inward toward the domain. The application core
must never depend on Ratatui or MCP, and no interface may depend on concrete
infrastructure.

The workspace structure mirrors this decision:

```text
crates/
├── domain/        # pure concepts, no workspace dependencies
├── execution/     # task/workflow engine
├── policy/        # permission model and policy engine
├── config/        # versioned configuration
├── application/   # use cases + ports
├── git/  process/  build/  liquibase/  health/  logging/   # infrastructure
├── mcp/  tui/     # interface adapters
└── src/ (root)    # composition root: service-orchestrator binary
```

## Alternatives considered

1. **TUI-first with an MCP server added later.**
   Fastest to a visible result, but the MCP server would either reimplement
   behavior or reach directly into UI logic. Rejected because it guarantees
   duplication and behavioral drift.

2. **MCP-first / server-first.** Treat the application as a backend and make the
   TUI a client. Rejected because it violates the product philosophy: the tool
   must work locally with no remote backend and no LLM, and it adds operational
   weight (a daemon, transport, lifecycle) that is unnecessary for a local tool.

3. **A single crate with strict module conventions.**
   Simpler initially, but nothing stops a module from reaching across layers.
   Rejected because the architecture rule would be enforced only by discipline,
   not by the compiler.

4. **Shared business logic duplicated per interface.**
   Rejected outright: two sources of truth for safety-critical, destructive
   operations is unacceptable.

## Reasoning

- **One source of truth.** Every capability is implemented once, in the
  application core. Interfaces cannot disagree about behavior.
- **Uniform safety.** The policy engine classifies operations as `READ`,
  `SAFE_WRITE`, or `DESTRUCTIVE` and is applied to both TUI and MCP. AI agents
  cannot bypass the rules a human is subject to.
- **Extensibility without core change.** A future CLI or HTTP API is another
  adapter over the same core.
- **Testability.** The domain and application layers can be tested without a
  terminal, a running MCP client, or real Git/Maven/Node processes.
- **Compile-time enforcement.** Crate boundaries and the workspace dependency
  table make a forbidden dependency a visible, reviewable `Cargo.toml` change,
  and cycles are rejected by Cargo.
- **LLM-optional.** Because the core has no AI dependency, the tool remains fully
  usable offline and without any AI infrastructure.

## Consequences

### Positive

- Consistent behavior and safety across all interfaces.
- New interfaces are cheap to add and cannot silently diverge.
- Core logic is unit- and integration-testable in isolation.
- The architecture rule is enforced by the build, not only by convention.
- AI remains an optional, replaceable adapter.

### Negative / trade-offs

- More crates and more boilerplate than a single-crate project; small changes may
  touch several `Cargo.toml` files.
- The composition root must be kept disciplined as the only place that knows
  both infrastructure and interfaces.
- Port traits must be designed before infrastructure can be plugged in, which
  front-loads some interface design.

### Follow-up

- Continue with the task/workflow engine design (`execution`) and the service
  executor abstraction (`application` + infrastructure).
- Add ADRs for the task engine, process management, Git integration,
  configuration, and MCP security as those decisions are made.
