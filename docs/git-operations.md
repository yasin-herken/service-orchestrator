# Git Operations

Service Orchestrator manages the Git repositories declared in your workspace
configuration. It drives the installed `git` CLI through the process manager (the
single process owner) and returns structured results, never pre-formatted
terminal text. The design and safety rules are in
[`git-manager.md`](git-manager.md).

## 1. Repository status

Read-only inspection is always available and never mutates a checkout.

- **TUI** — the **Git** screen lists configured repositories. Status is
  inspected **on demand** (press `r`); the screen never scans every repository on
  each frame.
- **GUI** — the **Git** screen; the repository list is configuration-derived and
  opening the screen performs no Git operation. Status/branches are inspected on
  demand.
- **MCP** — `git_status { serviceId }`, `git_list_branches { serviceId }`,
  `git_diff { serviceId, staged?, maxBytes? }`.

The status includes the current branch, upstream, ahead/behind counts, and
whether the working tree is dirty.

## 2. Fetch, pull, checkout, sync

| Operation | What it does | TUI | GUI | MCP |
| --- | --- | --- | --- | --- |
| Fetch | `git fetch` the remote; changes nothing locally | `f` / `F` | Fetch | `git_fetch` |
| Pull | Fast-forward the current branch; **fails if not fast-forwardable** | `p` | Pull (single only) | `git_pull` |
| Checkout | Switch to a branch | `c` (choose branch) | Checkout | `git_checkout { branch }` |
| Sync | Fetch, check out the configured branch, and fast-forward | `y` / `Y` | Sync | `git_sync` |
| Clone | Clone a repository that is not present | — | — | `git_clone` |

Bulk operations:

- **TUI** — uppercase `F`/`Y` apply fetch/sync to the selection.
- **GUI** — bulk fetch, checkout, and sync, with a read-only preview.
- **MCP** — `git_sync_repositories`, `git_fetch_repositories`,
  `git_checkout_repositories` (each returns a workflow id).

> There is **no bulk pull**: pull is single-repository only. Bulk
> synchronization is fetch + checkout + fast-forward pull, expressed as one
> coordinated workflow — not a per-repository loop in the interface.

## 3. Dirty working trees

Local changes are protected:

- A **dirty working tree blocks a checkout or a pull** — the operation fails with
  `GIT_WORKING_TREE_DIRTY` rather than silently stashing, resetting, or
  force-checking-out.
- Nothing in the normal workflow (`Sync`, environment preparation) ever runs
  `reset`, `clean`, or a forced checkout.
- To proceed, either commit/stash your changes yourself, or explicitly use a
  destructive operation (§5).

## 4. Fast-forward-only pull

`pull` is **fast-forward-only by default**:

- If the local branch has diverged from its upstream, the pull fails with
  `GIT_DIVERGED_BRANCH` instead of creating a merge commit.
- If the branch has no configured upstream, it fails with
  `GIT_UPSTREAM_NOT_CONFIGURED`.
- A detached HEAD is reported with `GIT_DETACHED_HEAD`.

This keeps a developer's history predictable; merging is left to the developer.

## 5. Destructive operations

Three operations are classified **DESTRUCTIVE** and require explicit
authorization:

| Operation | Effect | TUI | MCP |
| --- | --- | --- | --- |
| Hard reset | `git reset --hard`, discarding local changes | `d` | `git_reset_hard` |
| Discard local changes | Revert tracked files to HEAD | `D` | `git_discard_changes` |
| Clean | Remove untracked files | `n` | `git_clean` |

- In the **TUI**, the application denies the operation under standard
  authorization and the interface opens a confirmation dialog; only an explicit
  confirmation re-runs it with destructive authorization.
- In **MCP**, the operation is **denied** with `CONFIRMATION_REQUIRED`. There is
  no `confirmed` flag for an agent to set; a human must perform it interactively.
- The **desktop GUI does not expose destructive Git operations** at all.

The policy check runs inside the application (and again at the daemon boundary),
so a UI confirmation is never the authorization itself
([`security.md`](security.md) §3).

## 6. Cancellation and partial failures

- Mutating Git operations are tasks. A task that has not started can be
  cancelled; a running task is asked to cancel cooperatively. A cancelled
  operation reports `GIT_CANCELLED`.
- A bulk Git operation is a **workflow**. Some repositories can succeed while
  others fail. A partially failed workflow is reported as such — never as a
  success — with the failed steps named.
- A request that was written to the daemon but received no response is **never
  retried**, because the mutation may have been applied. The caller receives
  `DAEMON_UNAVAILABLE` ([`local-daemon.md`](local-daemon.md) §10).

## 7. Authentication

- Git authentication comes from your environment and your normal Git
  configuration. Credential-bearing URLs are never stored in configuration and
  are never logged.
- The child Git process has repo-redirecting and code-executing environment
  variables stripped (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_SSH`, `GIT_EXTERNAL_DIFF`,
  …), while your **global and system Git configuration is intentionally left in
  place**, so existing credential helpers and SSH setup keep working
  ([`security.md`](security.md) §6).
- A failure to authenticate is reported as `GIT_AUTHENTICATION_FAILED`.

## 8. Workflow integration

Environment preparation (`Sync`) uses exactly these operations: fetch, check out
the configured target branch (never a hard-coded `master`), and fast-forward.
It schedules no reset, clean, or forced checkout. See
[`workflows.md`](workflows.md).

## 9. Common Git errors

| Code | Meaning | Action |
| --- | --- | --- |
| `GIT_NOT_CLONED` | The repository directory is not a checkout. | Clone it (`git_clone`) or fix `path`. |
| `GIT_WORKING_TREE_DIRTY` | Local changes block a checkout/pull. | Commit or stash them. |
| `GIT_BRANCH_NOT_FOUND` | The requested branch does not exist. | Fetch first, or use an existing branch. |
| `GIT_DIVERGED_BRANCH` | Pull cannot fast-forward. | Merge/rebase manually. |
| `GIT_UPSTREAM_NOT_CONFIGURED` | No upstream for the branch. | Set an upstream. |
| `GIT_AUTHENTICATION_FAILED` | Credentials rejected. | Fix your Git credentials/SSH agent. |
| `GIT_REMOTE_UNAVAILABLE` | The remote is unreachable. | Check the network and the URL. |
| `GIT_REPOSITORY_BUSY` | Another operation holds the repository. | Wait; mutations of one checkout are serialized. |
| `GIT_TIMEOUT` | The operation exceeded its timeout. | Retry; check the network. |

The full table is in [`error-reference.md`](error-reference.md).

## 10. Limitations

- The GUI has no destructive Git operations and no bulk pull.
- Advanced Git UX (interactive rebase, history rewriting, diff review) is not
  provided; Service Orchestrator does not replace a Git client.
- `git clone` has no TUI/GUI affordance today; use MCP or clone manually.
