# ADR 017: Read-Only Workspace Validation Behind a Probe Port

- **Status:** Accepted
- **Date:** 2026-10-12
- **Deciders:** Service Orchestrator maintainers

## Context

`tasks/017.md` asked for a real, end-to-end validation of a configured
workspace: does the declared root exist, are the repositories present and
valid, are the runtimes available, do the dependencies resolve, and do the
declared ports collide? The same capability had to be reachable from the TUI,
the MCP server, and the CLI — three adapters over one application core
(ADR 001) — and it had to be **strictly read-only**.

Two constraints shaped the design:

- **No direct system access in the application core.** The core may not touch
  the filesystem, `PATH`, or TCP sockets directly (`docs/architecture.md` §5,
  rules on dependency direction). Every check the validator performs needs
  system knowledge the core is not allowed to acquire itself.
- **One source of truth.** The checks belong to the application core so that
  every interface observes identical results; reimplementing them in an
  adapter would violate ADR 001.

The configuration loader already rejects unknown dependencies, dependency
cycles, and undeclared health ports at parse time, so a naive reading is that
a runtime validator is redundant. It is not: the loader validates the
*document*, while the validator inspects the *machine* — paths, executables,
occupied ports — which no parse step can know about.

## Decision

Add a `ValidateWorkspace` use case in the application core, fronted by a new
read-only port, `WorkspaceProbe`, that the composition root implements
against the real system.

- **`WorkspaceProbe`** (`crates/application/src/ports/probe.rs`) declares the
  four system facts the validator needs: a path exists, an executable is
  resolvable on `PATH`, a directory is a Git working tree, and a TCP port is
  occupied on loopback. `NullWorkspaceProbe` is the permissive default that
  keeps the application constructible without infrastructure, mirroring
  `NullHealthIndex` and the other null ports.
- **`SystemWorkspaceProbe`** (`src/probe.rs`, the binary's composition root)
  implements the port with `std::fs`, `PATH` scanning with an execute-bit
  check, `git -C <dir> rev-parse --is-inside-work-tree`, and a 150 ms
  loopback TCP connect. It is wired once, next to the other real adapters.
- **The use case** (`crates/application/src/queries/validation.rs`) walks the
  configured workspaces and emits a structured `WorkspaceValidationReport`:
  per-check `ValidationDiagnostic`s with a stable `ValidationCode` and
  `ValidationSeverity`, plus `ValidationTotals`. Configuration-level defects
  (missing dependency, cycle, undeclared health port, duplicate port) are
  re-checked as defence-in-depth even though the loader rejects them.
- **Port conflicts are distinguished from runtime occupancy.** Two services
  declaring the same port is a *configuration* error; a configured port that
  is *currently occupied* by an unrelated process is a *warning* — the
  occupying process is never signalled or killed, consistent with the Service
  Runtime's conflict reporting (`docs/adr/010-service-runtime.md`).
- **One client boundary, three adapters.** `OrchestratorClient::validate_workspace`
  reaches it over the daemon's IPC (`VALIDATE_WORKSPACE`); the MCP server
  exposes it as the `validate_workspace` tool with a strict schema; the same
  path is exercised by the TUI's client.

## Alternatives considered

- **Validate in each adapter.** Rejected: three implementations, three
  behaviours, and the core would no longer be the single source of truth
  (ADR 001).
- **Let the use case read the filesystem directly.** Rejected: the
  application core must not depend on the filesystem or OS APIs
  (`docs/architecture.md` §5); the checks would also become untestable
  without touching a real disk.
- **Rely on configuration parse-time validation alone.** Rejected: the loader
  sees the document, not the machine; missing repositories, absent runtimes,
  and occupied ports are runtime facts.
- **A write-capable "repair" validator.** Rejected: `tasks/017.md` requires
  read-only validation with no silent mutation; a repair mode would need the
  destructive-authorization machinery (ADR 007/014 policy) and is out of
  scope.

## Consequences

- The validator is unit-testable end to end with a scripted probe —
  `crates/application/tests/workspace_validation.rs` covers every diagnostic
  class, including scale fixtures, without touching a real disk.
- The real probe is exercised end to end by the MCP stdio integration test
  (`tests/mcp_stdio.rs`) and by a read-only run against a developer's live
  configuration.
- The port list grows by one read-only member, following the established
  pattern (`HealthIndex`, `ServiceRuntimeIndex`); no new dependency edges are
  introduced — `git`/`process`-adjacent behaviour stays in the adapters.
- Defence-in-depth validator checks duplicate a small amount of loader
  validation on purpose; the duplication is tested by mutating the `Config`
  after loading.
