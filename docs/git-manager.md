# The Git Manager

The Git manager is the infrastructure adapter that implements the application's
`GitManager` port. It lets every interface synchronize repositories, check out
branches, clone, inspect status, and perform guarded destructive operations
without ever constructing a `git` command itself. It lives in the
`service-orchestrator-git` crate.

It drives the **installed Git CLI** — not `git2`/libgit2 — through the process
manager, so it obeys the single-process-owner rule (`AGENTS.md` §15) while
reusing the developer's own Git configuration, credential helpers, and SSH
agent. It never spawns an operating-system process itself and never depends on
Ratatui or MCP.

```text
Application use case
       │  (mutating op as a TaskDefinition with repository parameters)
       ▼
Task Engine ──> GitTaskExecutor ──┐
       │                          │
       │  (read-only op, direct)  │
       ▼                          ▼
               CliGitManager (GitManager port)
                       │
           ┌───────────┼──────────────┬───────────────┐
           ▼           ▼              ▼               ▼
      command.rs  coordinator.rs   runner.rs      status.rs
    (pure argv)  (per-repo locks)  (process mgr)  (porcelain v2)
                                       │
                                       ▼
                                 Process Manager
                                       │
                                       ▼
                                     git CLI
```

> This task implements the Git manager only. Maven/npm build tooling, Liquibase,
> service startup, the TUI, and MCP are intentionally **not** implemented here.

---

## 1. What the manager is not

- It is **not** a business layer. Validation, safety policy, dependency order,
  and permission gating live in the application and policy layers. The manager
  contains no per-interface behavior and no service-specific branching.
- It is **not** a second concurrency framework. It never creates its own
  thread pool or semaphore; the task engine's `max_parallel_tasks` is the only
  global bound. The manager contributes repository-scoped coordination only.
- It is **not** a configuration loader. Repositories arrive as an already
  validated, workspace-aware `ResolvedRepository`; the manager never parses
  TOML.

---

## 2. The repository value

A repository is never addressed by a bare name: two workspaces may legitimately
contain a service with the same id. Every operation takes a
`ResolvedRepository`:

| Field | Meaning |
| --- | --- |
| `workspace_id` | The owning workspace. |
| `name` | The repository identifier (a service or library id). |
| `url` | The clone URL. Configuration rejects credential-bearing URLs. |
| `path` | The absolute local checkout path. |
| `workspace_root` | The workspace root; the checkout must stay inside it. |
| `default_branch` | The branch used when no other branch is requested. |

`ResolvedRepository::is_inside_workspace` is checked before any command runs;
a checkout that resolves outside its workspace fails with
`GIT_PATH_OUTSIDE_WORKSPACE`. The value round-trips through the task engine via
`to_parameters`/`from_parameters`, so a Git task submitted by the TUI or MCP can
be executed later without re-reading configuration.

---

## 3. Operations

### Read-only (called directly on the port)

| Operation | Git command | Result |
| --- | --- | --- |
| `status` | `git status --porcelain=v2 --branch -z` | `RepositoryStatus` |
| `branches` | `git branch --all --format=...` | `Vec<BranchInfo>` |
| `diff` | `git diff [--cached] --numstat` + redirected patch | `RepoDiff` |

`status` parses porcelain v2 with `-z`, so paths with spaces, newlines, and
unicode are handled correctly. It reports the lifecycle, branch, commit,
detached/unborn state, upstream, ahead/behind counts, and independent
staged/unstaged/untracked/conflicted counts. A missing path is
`RepositoryLifecycle::NotCloned`; a plain directory is `Invalid`.

`diff` returns a per-file `--numstat` summary plus a unified patch. The patch is
redirected to a temporary file (`git diff --output=<file>`) and only a bounded
prefix (`DiffOptions::max_bytes`, default 64 KiB) is read back, so an
arbitrarily large diff never has to travel through the process manager's
bounded line-event channel. When a diff is larger than the limit, `truncated`
is set; the file is removed when the operation ends.

### Mutating (submitted as tasks)

| Operation | Git command | Notes |
| --- | --- | --- |
| `fetch` | `git fetch origin [--prune]` | Updates remote-tracking refs; never touches the working tree. |
| `pull` | `git pull --ff-only` | Fast-forward only; refuses a dirty tree or a diverged branch. |
| `checkout` | `git checkout <branch>` or `--track -b <branch> origin/<branch>` | Sets up tracking for a remote-only branch. |
| `clone` | `git clone --branch <b> -- <url> <dest>` | Refuses a non-empty destination. |
| `sync` | fetch + checkout default branch + `pull --ff-only` | Returns to the configured default branch. |
| `reset_hard` | `git reset --hard HEAD` | Destructive; gated by policy. |
| `clean` | `git clean -fd` | Destructive; gated by policy. |
| `discard_local_changes` | reset + clean | Destructive; gated by policy. |

Each mutating call returns a `GitOperationResult` carrying the operation, the
branch, the before/after commits, and an honest `changed` flag (a fetch or a
checkout of the current branch reports `changed == false`).

---

## 4. Safety

No operation silently discards developer work (`AGENTS.md` §30):

- `pull` and `sync` inspect the working tree first and **refuse** when it has
  local changes, returning `GIT_WORKING_TREE_DIRTY`.
- `pull` and `sync` **refuse** a diverged branch with `GIT_DIVERGED_BRANCH`,
  including the ahead/behind counts, instead of attempting an implicit merge.
- `checkout` lets Git refuse when switching would overwrite local changes and
  maps that to `GIT_WORKING_TREE_DIRTY`; the local modification is preserved.
- `clone` refuses a non-empty destination with `GIT_DESTINATION_NOT_EMPTY`
  rather than overwriting it.
- The destructive operations (`reset_hard`, `clean`, `discard_local_changes`)
  are reachable only through application commands that call
  `authorize(Permission::Destructive)` first. Without explicit confirmation the
  command fails with `ApplicationError::PermissionDenied` and no task is
  scheduled. The check cannot be bypassed by an interface.

---

## 5. Structured errors

Every failure is a `GitError` variant with a stable, machine-readable `code()`
and enough context (repository, operation, branch, exit code) to explain what
happened — without leaking credentials.

| Code | Meaning |
| --- | --- |
| `GIT_NOT_CLONED` | The local path does not exist. |
| `GIT_INVALID_REPOSITORY` | The path exists but is not a Git working copy. |
| `GIT_DESTINATION_NOT_EMPTY` | The clone destination is occupied. |
| `GIT_REPOSITORY_UNAVAILABLE` | The repository could not be inspected. |
| `GIT_WORKING_TREE_DIRTY` | Local changes would be affected. |
| `GIT_BRANCH_NOT_FOUND` | The requested branch does not exist. |
| `GIT_DETACHED_HEAD` | The repository is in a detached-HEAD state. |
| `GIT_UPSTREAM_NOT_CONFIGURED` | The branch has no upstream. |
| `GIT_DIVERGED_BRANCH` | The branch diverged (ahead and behind). |
| `GIT_MERGE_CONFLICT` | A merge or rebase produced conflicts. |
| `GIT_AUTHENTICATION_FAILED` | Authentication with the remote failed. |
| `GIT_REMOTE_UNAVAILABLE` | The remote was unreachable. |
| `GIT_COMMAND_FAILED` | A `git` command exited non-zero. |
| `GIT_TIMEOUT` / `GIT_CANCELLED` | The operation timed out or was cancelled. |
| `GIT_REPOSITORY_BUSY` | The repository is held by another mutation. |
| `GIT_MISSING_PARAMETER` / `GIT_INVALID_PARAMETER` / `GIT_INVALID_REFERENCE` | A task parameter was missing, invalid, or malformed. |
| `GIT_PATH_OUTSIDE_WORKSPACE` | The checkout escapes its workspace. |
| `GIT_UNAVAILABLE` | No Git executable or no manager is wired. |
| `GIT_INTERNAL_ERROR` | An internal manager failure. |

The manager classifies `git` stderr into the specific variants above
(authentication, unreachable remote, not-a-work-tree, dirty tree), bounds the
failure detail, and redacts the configured URL and any URL userinfo before an
error is returned (`AGENTS.md` §31).

---

## 6. Authentication

The manager intentionally uses the developer's own Git setup rather than
carrying credentials:

- It uses the installed `git`, so the developer's credential helpers, SSH agent,
  and configuration apply.
- It never embeds credentials in a URL. Configuration rejects credential-bearing
  URLs, and error output is redacted defensively.
- It forces a **non-interactive** environment (`GIT_TERMINAL_PROMPT=0`,
  `GIT_ASKPASS=""`, `SSH_ASKPASS=""`, `GCM_INTERACTIVE=never`), so a command can
  never block forever waiting for a terminal prompt that no interface can
  answer. A missing credential surfaces as `GIT_AUTHENTICATION_FAILED` instead
  of a hang.

---

## 7. Task-engine integration

Long-running and mutating operations are tasks; nothing blocks an interface.

- `GitTaskExecutor` decodes the `ResolvedRepository` from the task's
  parameters, copies the task timeout and cancellation token into `GitOptions`,
  calls the matching `GitManager` method, and maps the result back to a
  `TaskOutcome` or `TaskExecutorError`. A cancelled `GitError` becomes
  `TaskExecutorError::Cancelled`; other structured failures keep their code.
- `register_git_executors` registers one shared executor against every Git
  `TaskKind` (`GitSync`, `GitFetch`, `GitCheckout`, `GitPull`, `GitClone`,
  `GitResetHard`, `GitClean`, `GitDiscardChanges`). The composition root hands
  the finished registry to the engine via `TaskEngineBuilder::registry`.
- The executor is thin by design: it contains no Git logic, so the TUI and MCP
  reach identical behavior through the same engine (`AGENTS.md` §21).

Cancellation is cooperative. The task's `Cancellation` is passed to the process
manager through `spawn_with_cancellation`, so a cancelled task stops the running
`git` process gracefully (`ExitReason::Cancelled` → `GIT_CANCELLED` →
`TaskExecutorError::Cancelled`). Because a `git` operation can complete part of
its work before cancellation is observed, cancellation is documented as
*best-effort partial completion* rather than an atomic rollback.

Bulk operations submit one independent workflow step per repository, so each
settles on its own and one repository's failure never invalidates another's
result. `Application::summarize_workflow` aggregates the workflow's tasks into a
`BulkOperationSummary` (`total`, `succeeded`, `failed`, `cancelled`, `skipped`,
`pending`).

---

## 8. Concurrency and coordination

- **Repository-scoped coordination.** A `RepositoryCoordinator` holds one
  `RwLock` per repository, keyed by the absolute checkout path. Mutations take
  the write lock; reads take the read lock. Two mutations of the same checkout
  therefore serialize, while independent repositories proceed concurrently.
- **Global bound.** The engine's `max_parallel_tasks` remains the only global
  concurrency limit; the coordinator never introduces a second one.
- **Deadlock avoidance.** A mutating method that needs to inspect the working
  tree while holding its write lock uses internal "unlocked" helpers
  (`status_unlocked`) rather than re-acquiring the non-reentrant lock. `pull`
  and `sync` fetch first, under the same lock, so ahead/behind counts reflect
  the remote before the divergence check.

---

## 9. Testing

Integration tests build a temporary bare "remote" and working clones under a
temporary directory and drive the manager exactly as the application would.
There is no network, GitHub, SSH, or developer repository involved
(`tasks/007.md` §23).

- `crates/git/tests/support/mod.rs` provides the fixtures.
- `crates/git/tests/git_manager.rs` covers status parsing, branches, fetch,
  pull (fast-forward, dirty, diverged, up-to-date), checkout (local,
  remote-track, missing, overwrite), clone (ok, non-empty, invalid URL),
  destructive operations, bounded diff, sync, path safety, and concurrency.
- `crates/git/tests/task_engine.rs` covers the whole adapter flow: a Git task
  succeeding and failing through the engine, a checkout driven by the branch
  parameter, a bulk workflow with per-repository results and a
  `BulkOperationSummary`, the engine's concurrency bound, and cancelling a
  queued task before it runs.

---

## 10. Limits and future work

- `git2`/libgit2 was not used; the CLI is preferred so the developer's existing
  Git configuration and credentials apply.
- Git authentication is delegated to the developer's environment; there is no
  in-application credential store.
- Repository-to-repository relationships (submodules, worktrees) are not
  modelled.
- The build, Liquibase, health, and service-start adapters are separate tasks
  that will reuse the process manager and the task engine in the same way.
