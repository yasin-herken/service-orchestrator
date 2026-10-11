# ADR 007: The Git manager

- **Status:** Accepted
- **Date:** 2026-10-11
- **Deciders:** Service Orchestrator maintainers

## Context

Repository work is the first thing a developer does when preparing an
environment: synchronize repositories, check out branches, clone what is
missing, inspect status, and occasionally discard local changes. ADR 001 and
ADR 004 placed a `git` adapter in the infrastructure layer and fixed the inward
dependency rule; ADR 006 established the process manager as the only component
allowed to run an operating-system process (`AGENTS.md` §15, §32). ADR 005
established the task engine and its executor registry.

This ADR records how the Git adapter is built: how it runs Git, how its
operations are exposed, how it integrates with the task engine, how it
coordinates concurrent work, and how it stays safe around developer work.

The requirements that shaped the decision:

- the manager must not spawn OS processes directly — all Git execution goes
  through the process manager;
- long-running operations must integrate with the task engine; there must be no
  second concurrency framework and the configured `max_parallel_tasks` remains
  the global bound;
- the manager must not depend on Ratatui or MCP, and must not parse TOML;
- results and errors must be structured and machine-readable;
- destructive operations must require explicit policy authorization and must
  never silently discard local changes;
- the installed Git CLI is preferred over `git2`;
- operations must cover status (porcelain v2), fetch, pull (fast-forward-only by
  default), checkout, branch listing, clone, a bounded diff, and guarded
  destructive reset/clean/discard;
- concurrent mutations of the same repository must not collide, independent
  repositories must run concurrently, and read-only operations may run
  concurrently;
- cancellation must propagate to the process manager; partial completion must be
  documented;
- bulk operations must use the task engine/scheduler with per-repository results
  and a summary;
- tests must use temporary local Git fixtures and bare remotes, never the
  network, GitHub, SSH, or a developer repository.

## Decision

Implement the Git manager as the `service-orchestrator-git` crate. It implements
the application's `GitManager` port by driving the installed Git CLI through the
process manager, and registers a `GitTaskExecutor` for every Git `TaskKind`.

### The CLI, through the process manager

The manager runs `git` as a structured `ProcessSpec` (a program plus a typed
argument vector) through the `ProcessManager`; it never calls
`std::process::Command` and never builds a shell string. This keeps the single
process-owner rule intact and keeps untrusted configuration out of a shell. The
CLI is preferred over `git2` because the developer's existing Git configuration,
credential helpers, and SSH agent then apply unchanged, and because the adapter
does not need to reimplement Git semantics.

### Read-only direct, mutating as tasks

- `status`, `branches`, and `diff` are read-only and are called directly on the
  port by application queries.
- Every mutating operation (`fetch`, `pull`, `checkout`, `clone`, `sync`, and the
  destructive `reset_hard`, `clean`, `discard_local_changes`) is expressed as a
  task. The application attaches the workspace-aware `ResolvedRepository` as
  executor parameters; a `GitTaskExecutor` decodes it at run time, applies the
  task timeout and cancellation token, and maps results back to the engine.
  `register_git_executors` populates the registry, and the composition root
  hands it to the engine through `TaskEngineBuilder::registry`.

This split means an interface can never block on Git, and the same validation,
safety, and coordination code serves both paths.

### Repository-scoped coordination, one global bound

A per-repository `RwLock` (`RepositoryCoordinator`, keyed by the absolute
checkout path) serializes mutations of the same checkout while independent
repositories and concurrent reads proceed in parallel. The engine's
`max_parallel_tasks` remains the only global concurrency limit; the coordinator
adds no pool of its own. Mutating methods that must inspect the working tree
under their own lock use internal unlocked helpers rather than re-acquiring a
non-reentrant lock.

### Safety first

`pull` and `sync` refuse a dirty working tree and a diverged branch; `checkout`
preserves local changes and maps Git's refusal to `GIT_WORKING_TREE_DIRTY`;
`clone` refuses a non-empty destination. Destructive operations are only
reachable through application commands that call
`authorize(Permission::Destructive)`; they are never scheduled without explicit
confirmation (`AGENTS.md` §30).

### Bounded diff

A large patch is redirected to a temporary file (`git diff --output=<file>`) and
only a bounded prefix is read back. The process manager streams output as
line events over a bounded channel; without this, a large diff would either
overflow that channel or be returned as a capture failure. The temporary file
keeps memory bounded and is removed when the operation ends.

### Non-interactive, credential-free authentication

The manager forces `GIT_TERMINAL_PROMPT=0`, empty `GIT_ASKPASS`/`SSH_ASKPASS`,
and `GCM_INTERACTIVE=never`, so a command can never block on a prompt no
interface can answer. Credentials are never embedded in a URL, and error output
redacts the URL and any URL userinfo.

### A documented dependency exception

The Git adapter depends on `execution` (to implement a `TaskExecutor`) and on
`process` (to run Git), in addition to `application` and `domain`. The
architecture rule that infrastructure crates do not depend on each other is
relaxed for exactly this adapter and recorded here. The edge is legitimate
because the Git adapter is the concrete executor for Git tasks and the concrete
client of the process manager; it still points outward from the domain and
neither `process` nor `execution` depends on `git`.

## Alternatives considered

1. **Use `git2`/libgit2 instead of the CLI.**
   Rejected as the primary implementation: it reimplements Git semantics, loses
   the developer's Git configuration and credential helpers, and adds a large
   native dependency. The CLI is preferred; `git2` may be added later only
   behind the same port if a concrete need appears.

2. **Spawn `git` directly from the Git adapter.**
   Rejected: it violates the single-process-owner rule (`AGENTS.md` §15) and
   would leave Git processes outside the manager's supervision, stop, and
   cleanup.

3. **Expose Git operations as a new concurrency framework inside the adapter.**
   Rejected: the task engine already provides bounded concurrency,
   cancellation, retry, and progress. The adapter contributes repository-scoped
   coordination only.

4. **Make all operations synchronous application calls.**
   Rejected: a clone, fetch, or pull can take seconds and would block an
   interface. Mutating operations are tasks; only cheap read-only inspection is
   direct.

5. **Return terminal text instead of structured values and errors.**
   Rejected: the TUI and MCP must render independently. `RepositoryStatus`,
   `BranchInfo`, `RepoDiff`, `GitOperationResult`, and the `GitError` variants
   with stable codes are machine-readable (`AGENTS.md` §28).

6. **Address a repository by name only.**
   Rejected: service names are not globally unique. A workspace-aware
   `ResolvedRepository` pairs the owning workspace with the absolute checkout
   path and round-trips through task parameters.

7. **Allow automatic merge/rebase on a diverged pull.**
   Rejected: it can silently create a merge commit or conflict. `pull` is
   fast-forward-only; a diverged branch is reported with ahead/behind counts so
   the developer decides.

8. **Discard local changes to "make it work."**
   Rejected: destructive operations require explicit authorization and are never
   implicit (`AGENTS.md` §30).

9. **Capture the full diff on standard output and bound it afterwards.**
   Rejected: a large patch overflows the process manager's bounded line-event
   channel. Redirection to a temporary file bounds memory and avoids the
   channel.

10. **Store Git credentials in configuration.**
    Rejected: secrets must not live in configuration (`AGENTS.md` §31).
    Authentication is delegated to the developer's environment, non-interactively.

## Consequences

### Positive

- One adapter implements the Git port for both interfaces, so behavior, safety
  checks, and error semantics cannot drift between the TUI and MCP.
- All Git execution flows through the process manager, so processes are tracked,
  stopped, and cleaned up consistently.
- The task engine provides bounded concurrency, cancellation, timeouts, and
  per-repository bulk summaries without a second framework.
- Safety is enforced in one place: no silent discards, no fast-forward except
  explicitly, no overwrite of a non-empty clone destination.
- Structured, credential-free errors make failures explainable to humans and
  agents alike.
- Tests are deterministic and offline, using temporary fixtures and bare remotes.

### Negative / trade-offs

- The `git -> execution` and `git -> process` edges are an explicit exception to
  the "infrastructure crates do not depend on each other" rule; it must be
  applied deliberately to future adapters, not copied casually.
- Cancellation is cooperative and best-effort: a `git` operation can complete
  part of its work before it observes cancellation. There is no atomic rollback.
- The bounded diff reads a temporary file; the patch is truncated rather than
  streamed, so a caller that needs the full diff must raise the limit or inspect
  the repository directly.
- Git authentication is delegated to the developer's environment; there is no
  in-application credential management.
- Submodules, worktrees, and repository-to-repository relationships are not
  modelled yet.

## Follow-up

- Implement the build (`build`), Liquibase, and health adapters on the same
  pattern: a port, a task executor, and process-manager execution.
- Consider a repository-to-repository dependency edge if submodules or worktrees
  become a requirement.
- Surface Git status, branches, and the bulk-operation summary in the TUI and
  MCP adapters strictly through the application use cases.
- Reuse `BulkOperationSummary` for the other multi-entity operations (build,
  health).
