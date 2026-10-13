# Security Audit — Service Orchestrator

This is the audit record for **Task 026 — Security Hardening & Permission
Management**. It documents the threat model, the trust boundaries, the confirmed
findings with their severity and exploitability, the fixes that were implemented
and tested, and the risks that remain or are deliberately deferred.

It describes the state of the code **after** the fixes. `docs/security.md` is
the durable security reference; this file is the audit trail behind it.

The audit reviewed the whole local platform: the shared daemon and its Unix
domain socket IPC, the client, the Application Core and its policy engine, the
MCP adapter, the Tauri desktop adapter, the Git/build/migration/process/health
infrastructure, configuration management, logging/redaction, and the TUI.

---

## 1. Scope and method

- **Source reviewed**: every crate in the workspace (`domain`, `policy`,
  `application`, `execution`, `client`, `daemon`, `config`, `git`, `build`,
  `process`, `runtime`, `liquibase`, `health`, `logging`, `mcp`, `tui`), the
  desktop adapter (`apps/desktop`), and their tests.
- **Method**: read the actual implementations; trace each security invariant of
  `AGENTS.md` §6 to the code that enforces it; look for places where untrusted
  data (MCP arguments, repository content, IPC payloads, configuration) reaches a
  privileged action; then implement and test the justified fixes.
- **Bias**: concrete, exploitable behavior over hypothetical concerns. No
  findings were fabricated; the "positive controls" in §5 were verified and left
  unchanged.

### Trust boundaries

```text
                 untrusted input                untrusted input
        (MCP args, repo content)         (IPC payload, HTTP health URL)
                      │                              │
 React GUI ─────┐     ▼                              ▼
               │   MCP Server ──────────────────────────────┐
 Ratatui TUI ──┼──────────────────────────────────────────►│
               │        Unix domain socket (0600, 0700 dir) │
               └──────────────────────────────────────────►│
                                                           ▼
                                              Shared Daemon
                                           (kernel peer UID check)
                                                           │
                                                   Application Core
                                                           │
                                                     Policy Engine
                                                           │
                                        Infrastructure (Git, build, process,
                                        Liquibase, health, filesystem)
```

- The **kernel peer UID** check at accept time is the only authentication.
- The **policy engine** is the only authorization authority; every interface
  uses it.
- Untrusted data can influence *which* named use case runs, never *whether* it
  is allowed.

---

## 2. Confirmed findings

Severity uses a local scale. "Exploitability" states what an attacker must
already control.

| ID | Finding | Severity | Exploitability | Status |
| --- | --- | --- | --- | --- |
| SEC-01 | Git child inherited repository-redirecting / code-executing env vars | Medium | Control of the daemon's environment (same-user / launch environment) | **Fixed** |
| SEC-02 | Configuration could define a shell command (`sh -c`) | Medium | Ability to edit the user's config file | **Fixed** |
| SEC-03 | Build/migration output and process descriptions were not consistently redacted | Medium | A build/migration printing a secret; or a secret passed as a CLI argument | **Fixed** |
| SEC-04 | HTTP health-check URL accepted CRLF/whitespace/control chars (header injection) | Medium | Ability to set a health-check URL in configuration | **Fixed** |
| SEC-05 | IPC accept loop had no connection cap and no first-request timeout | Medium | Any same-user process opening many/idle socket connections | **Fixed** |
| SEC-06 | Auto-started `daemon.log` could be world-readable | Low | Local user with a permissive umask reading another user's log | **Fixed** |
| SEC-07 | Config `.toml.bak` backup could be world-readable | Low | Local user reading another user's backup | **Fixed** |
| SEC-08 | Unbounded task listing and diff size could inflate IPC/MCP responses | Low | A client requesting an unbounded listing/patch | **Fixed** |
| SEC-09 | Tauri CSP omitted `base-uri`/`object-src`/`frame-ancestors`/`form-action` | Low | Content-injection in the webview (defense in depth) | **Fixed** |
| SEC-10 | Configuration may inline a plaintext secret; existing file mode preserved | Medium | A local user reading a world-readable config file | **Accepted** (§6) |
| SEC-11 | `ClientKind` is not authentication; same-user shutdown / destructive intent | Low | A malicious same-user process | **Accepted** (§6) |
| SEC-12 | HTTP health-check target has no allow-list (SSRF-shaped target selection) | Low | Ability to set a health-check URL in configuration | **Deferred** (§6) |
| SEC-13 | Task history is retained in memory without eviction | Low | Long-running daemon accumulating history | **Deferred** (§6) |
| SEC-14 | Task listing semantics (no-limit path) were inconsistent with the tool contract; bounding must not change what is listed | Informational | — | **Fixed (behavior preserved)** |

### SEC-01 — Git environment inheritance

**What.** The Git manager runs a real `git` process with the daemon's
environment inherited. An inherited `GIT_DIR`, `GIT_WORK_TREE`,
`GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, or
`GIT_COMMON_DIR` redirects Git to a different repository or object store;
`GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF`, or `GIT_CONFIG` makes Git execute a
program the application never chose.

**Why it matters.** A Git operation could silently act on the wrong repository,
or a same-user environment could turn a routine `git fetch`/`status` into
arbitrary code execution. The application promises every operation is scoped to
the repository it resolved.

**Fix.** Added `ProcessSpec::removed_environment` (with `remove_env`/`remove_envs`)
and applied `HARDENED_ENVIRONMENT_REMOVALS` in the Git manager. `build_command`
applies `env_remove` **before** explicit `env` entries, so an explicit value
still wins. Global and system Git configuration is intentionally **not** removed,
so the developer's authentication keeps working.
Files: `crates/process/src/spec.rs`, `crates/process/src/manager.rs`,
`crates/git/src/command.rs`, `crates/git/src/manager.rs`.

### SEC-02 — Shell command via configuration

**What.** Configuration accepts a command as a program plus arguments, but a
user could write `build = "sh -c '…'"` or a structured
`{ program = "bash", args = ["-c", "…"] }`. That re-introduces full shell
interpretation, defeating the structured-command boundary (`AGENTS.md` §32).

**Why it matters.** Any value that later reaches that command becomes shell
interpreted; the guarantee "untrusted input is never interpolated into a shell"
is lost.

**Fix.** `reject_shell_command` refuses a configured command that invokes a
shell with `-c` (for both the string form and the structured form) at
**validation** time, so it can never run. Running a shell *script* without `-c`
is still allowed.
Files: `crates/config/src/validation.rs`, `crates/config/tests/configuration.rs`.

### SEC-03 — Inconsistent secret redaction in output

**What.** The log hub redacted entries, but the build adapter stored raw
captured stdout/stderr, the migration adapter had its own partial rule, and
`ProcessSpec::description()` included raw arguments — so a secret on a command
line (`--password=…`) or printed by a build/migration could reach a task result,
an IPC frame, or an interface.

**Why it matters.** Build logs and task results are user-facing and agent-facing;
a credential leaking there is a direct confidentiality loss (`AGENTS.md` §31).

**Fix.** Moved the canonical redaction rule into the domain
(`crates/domain/src/redact.rs`) so every layer shares one implementation without
a dependency cycle; `logging::redact` now re-exports it. Applied redaction to
build success/failure capture, migration output, and `ProcessSpec::description`
(argument by argument).
Files: `crates/domain/src/redact.rs`, `crates/logging/src/redact.rs`,
`crates/build/src/manager.rs`, `crates/liquibase/src/manager.rs`,
`crates/process/src/spec.rs`.

### SEC-04 — HTTP health-check header injection

**What.** `parse_http_url` built the request line and `Host` header from the
configured URL verbatim. A URL containing a carriage return or line feed — e.g.
`http://localhost:8081/health\r\nX-Evil: 1` — injects additional request
headers, and whitespace or unusual host characters can corrupt the request.

**Fix.** Reject empty URLs, any control character, any whitespace, and any host
that is not composed of hostname characters (`A-Z0-9`, `.`, `-`, `_`).
Files: `crates/health/src/http.rs`.
**Remaining:** the target host is not allow-listed (SEC-12).

### SEC-05 — IPC connection exhaustion / slot squatting

**What.** The accept loop spawned a task per accepted connection with no upper
bound, and a connection that never sent a request held its slot forever. A
same-user process could exhaust the daemon's file descriptors and memory, or
simply squat connections.

**Fix.** `MAX_CONNECTIONS = 128`: a slot is reserved before spawning and the
stream is dropped when the cap is reached. `INITIAL_REQUEST_TIMEOUT = 30 s`
bounds only the **first** request, so an idle-but-subscribed client is not
disturbed; later reads stay unbounded so event streaming still works.
Files: `crates/daemon/src/server.rs`.

### SEC-06 / SEC-07 — File permissions that can hold secrets

**What.** The auto-started daemon's `daemon.log` was created with the process
umask (world-readable under a permissive umask), and the config `.toml.bak`
backup inherited the source file's mode (potentially world-readable).

**Fix.** `open_daemon_log` creates the log `0600` and re-tightens an existing
loose file; `persist` sets the `.toml.bak` backup to `0600`.
Files: `crates/client/src/socket.rs`, `crates/config/src/management.rs`.

### SEC-08 — Unbounded listings and patch size

**What.** `list_tasks` (with no limit) and `LIST_TASKS`/`LIST_RECENT_TASKS` over
IPC could serialize the whole task history into one frame; `DiffOptions`'
`max_bytes` was settable to any value.

**Fix.** `DEFAULT_TASK_LIST_LIMIT = 100`, `MAX_TASK_LIST_LIMIT = 1000`,
`clamp_task_limit`; the daemon truncates `LIST_TASKS` to the newest window and
clamps `LIST_RECENT_TASKS`; the MCP `list_tasks` is always bounded; the diff
builder clamps `max_bytes` to `MAX_DIFF_BYTES = 1 MiB` on both the application
builder and the IPC `DiffParams` rebuild path.
Files: `crates/execution/src/engine.rs`, `crates/execution/src/lib.rs`,
`crates/daemon/src/dispatch.rs`, `crates/mcp/src/server.rs`,
`crates/application/src/ports/git.rs`, `crates/client/src/protocol.rs`.

### SEC-09 — Tauri CSP directives

**What.** The production and dev CSP set `default-src`/`script-src`/`connect-src`
but omitted `base-uri`, `object-src`, `frame-ancestors`, and `form-action`.

**Fix.** Added `base-uri 'none'; object-src 'none'; frame-ancestors 'none';
form-action 'none'` to both `csp` and `devCsp`.
File: `apps/desktop/src-tauri/tauri.conf.json`.

### SEC-14 — Task-listing semantics

**What.** While bounding the MCP `list_tasks`, the no-limit path was changed to
`recent_tasks`, which returns only **finished** tasks; the original behavior
listed all known tasks (active + finished). This silently dropped queued/running
tasks from the response.

**Fix.** The no-limit path now bounds the *existing* listing (newest
`DEFAULT_TASK_LIST_LIMIT` of `tasks()`), preserving the tool's documented
"running tasks + recent history" behavior. Caught by the existing regression
test `a_safe_write_schedules_a_queryable_task`.
File: `crates/mcp/src/server.rs`.

---

## 3. Fixes implemented — summary

| Area | Change | Files |
| --- | --- | --- |
| Redaction | Canonical rule in the domain; applied to build, migration, process description | `domain/src/redact.rs`, `logging/src/redact.rs`, `build/src/manager.rs`, `liquibase/src/manager.rs`, `process/src/spec.rs` |
| Process env | `removed_environment` + `build_command` `env_remove` before explicit env | `process/src/spec.rs`, `process/src/manager.rs` |
| Git | `HARDENED_ENVIRONMENT_REMOVALS` applied to every Git child | `git/src/command.rs`, `git/src/manager.rs` |
| Config | Reject `sh -c`/`bash -c` commands; `0600` backup | `config/src/validation.rs`, `config/src/management.rs` |
| Health | Reject control chars, whitespace, invalid host chars in URL | `health/src/http.rs` |
| IPC | Connection cap + first-request timeout | `daemon/src/server.rs` |
| Bounds | Task-list limits + diff byte clamp | `execution/src/engine.rs`, `execution/src/lib.rs`, `daemon/src/dispatch.rs`, `mcp/src/server.rs`, `application/src/ports/git.rs`, `client/src/protocol.rs` |
| Files | `daemon.log` owner-only | `client/src/socket.rs` |
| Desktop | CSP hardening | `apps/desktop/src-tauri/tauri.conf.json` |

No new product feature, no second policy engine, and no architectural change
were introduced. The existing `Permission` vocabulary, policy engine,
authorization flow, and layered architecture are preserved.

---

## 4. Regression tests added

| Test | Location | Verifies |
| --- | --- | --- |
| `masks_secret_assignments`, `masks_url_userinfo`, `masks_bearer_tokens`, `preserves_whitespace_and_plain_text` | `crates/domain/src/redact.rs` | the shared redaction rule |
| `description_redacts_secret_looking_arguments` | `crates/process/src/spec.rs` | a secret argument is masked in a description |
| `environment_removals_are_recorded` | `crates/process/src/spec.rs` | removals are captured |
| `removed_environment_is_unset_in_the_child_command`, `explicit_environment_wins_over_a_removal` | `crates/process/src/manager.rs` | child env actually drops the var, explicit wins |
| `hardened_removals_cover_repository_redirection_and_code_execution` | `crates/git/src/command.rs` | the denylist covers the dangerous names |
| `rejects_a_shell_command_line_from_a_service_command`, `rejects_a_shell_command_from_a_structured_service_command` | `crates/config/tests/configuration.rs` | shell commands are rejected |
| `rejects_control_characters_and_whitespace`, `rejects_hosts_with_invalid_characters` | `crates/health/src/http.rs` | CRLF/whitespace/bad host rejected |
| `a_prompt_first_request_is_read`, `a_silent_first_connection_is_bounded`, `the_connection_cap_is_positive_and_bounded` | `crates/daemon/src/server.rs` | initial-request timeout + cap |
| `task_limit_is_clamped_to_the_hard_maximum` | `crates/execution/src/engine.rs` | task-limit clamp |
| `diff_max_bytes_is_clamped` | `crates/application/src/ports/git.rs` | diff byte clamp |
| `an_oversized_diff_request_is_clamped_when_rebuilt` | `crates/client/src/protocol.rs` | clamp on the IPC path |
| `the_daemon_log_is_created_owner_only`, `a_pre_existing_world_readable_daemon_log_is_tightened` | `crates/client/src/socket.rs` | log permissions |
| `redaction_scrubs_secret_values_on_surviving_lines` + adapted fixtures | `crates/liquibase/src/manager.rs`, `crates/liquibase/tests/*` | migration output scrubbed |

Pre-existing tests that relied on `sh -c` (now refused) were adapted to real
executable script fixtures or to call classification directly, so they continue
to exercise the same behavior without configuring a shell command.

---

## 5. Positive controls verified (no change needed)

These were inspected and found sound; they are recorded so the audit is honest
about coverage rather than only listing problems.

- **No network listener.** The daemon binds a Unix domain socket only; every
  health check is loopback-only. There is no HTTP server.
- **No `sh -c` in executors.** Git, build, and migration run structured
  program+args; clone passes `--`; branch names are validated.
- **Destructive Git is gated.** `git reset --hard`, `git clean`, and discard are
  `Destructive` and require the three trust signals; pull defaults to
  `--ff-only`; a dirty tree blocks checkout/pull; nothing stashes or resets
  silently.
- **Migration target awareness.** Non-local targets require authorization; a
  failed migration is never retried; a lock is never released automatically; a
  cancelled migration never claims a rollback.
- **Process ownership.** Processes are spawned in their own group, stopped
  gracefully then forcefully, and **never** signaled by PID/port; the daemon
  adopts nothing after a restart.
- **Frame cap.** 8 MiB enforced on both the sync and async read paths.
- **Tauri capability minimum.** `core:default` only; no shell, filesystem, HTTP,
  or process plugin; external navigation blocked.
- **Secret modeling.** Secret environment values render as `***` in the domain;
  config change/field views redact them.
- **Log redaction at ingest.** Recorded messages are redacted and truncated
  before storage regardless of source.
- **Policy engine is the single authority.** MCP and CLI can never self-confirm;
  a caller boolean is never authorization.

---

## 6. Remaining and deferred risks

These are documented rather than fixed, with the reasoning.

- **SEC-10 — config file may inline a plaintext secret.** The local
  configuration can carry a literal secret (a `secret = true` value). New files
  are created `0600` and the backup is `0600`, but an existing file's mode is
  preserved, so a user who inlines a secret into a world-readable file keeps it
  world-readable. The same-user trust boundary means this does not grant another
  user anything they could not already reach, but it is a real data-at-rest
  weakness. **Deferred recommendation:** consider always tightening the config
  file to `0600`, or supporting only secret *references* (environment-variable
  names) rather than inline values.
- **SEC-11 — `ClientKind` is not authentication.** A same-user process can
  declare itself an interactive client and request `allow_destructive`; the
  kernel peer UID check permits it because it *is* the user. A same-user process
  can also request a daemon shutdown. This is accepted: the daemon is not a
  protection against the user's own processes, and inventing a multi-user
  authentication scheme is out of scope (`tasks/026.md` §8). The wire boolean is
  still never trusted on its own.
- **SEC-12 — no health-check allow-list.** The URL is validated for control
  characters and host shape, but it is developer configuration and is not
  constrained to an allow-list; a configured URL could point anywhere (including
  a local tunnel to a remote database). **Deferred recommendation:** an optional
  allow-list of health-check hosts.
- **SEC-13 — no task-history eviction.** History is retained in memory; listing
  is bounded, but a very long-lived daemon accumulates history. Eviction needs
  an explicit retention policy and is deliberately not added here.
- **Best-effort redaction.** Redaction cannot detect every secret shape; the
  primary control is preventing secrets from entering logs (secret-modeled
  values). This limitation is documented, not hidden.
- **Managed-process recovery after a crash** remains unimplemented (pre-existing,
  out of scope): surviving processes become unmanaged rather than being adopted
  by PID/port.

---

## 7. Dependency review

- The Rust dependency surface is pinned by `Cargo.lock`; the MCP adapter uses
  `rmcp` with `default-features = false` (no HTTP/WebSocket/child transport),
  and the Tauri adapter is excluded from the root workspace so its large
  `wry`/`tao`/`objc` tree is not linked into the backend, TUI, MCP, or daemon.
- The frontend dependency set is pinned by `apps/desktop/package-lock.json`.
- **`npm audit` (`apps/desktop`): 0 vulnerabilities.**
- **`cargo audit` (RustSec advisory DB, 1310 advisories, 164 crate
  dependencies): no vulnerabilities.** Three *warnings* were reported, all
  transitive through `ratatui 0.29.0` (the TUI renderer):
  - `paste 1.0.15` — **unmaintained** (RUSTSEC-2024-0436); a compile-time
    proc-macro, not linked into the running binary.
  - `lru 0.12.5` — **unsound** (RUSTSEC-2026-0253: `LruCache::pop()` panic
    safety; RUSTSEC-2026-0002: `IterMut` aliasing). Both concern the crate's
    cache internals, reached only through `ratatui`'s rendering path, not by
    untrusted input.
  - Neither affects the daemon, MCP, or the Git/build/migration/process
    infrastructure, which do not depend on `ratatui`.
- No dependency was upgraded in this task: no advisory-motivated, backward-
  compatible upgrade was identified as necessary, and the task explicitly
  forbids automatic breaking upgrades (`tasks/026.md` §26). Clearing the two
  warnings means bumping `ratatui` (and therefore `lru`/`paste`) when a
  compatible version is adopted; that is tracked, not done here.

---

## 8. Validation

Run on the working tree after all fixes:

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo check --workspace` | clean |
| `cargo test --workspace` | all tests pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `npm run typecheck` (`apps/desktop`) | clean |
| `npm run build` (`apps/desktop`) | success (111 modules transformed) |
| `npm run test` (`apps/desktop`) | 24 files / 219 tests passed |
| `npm audit` (`apps/desktop`) | 0 vulnerabilities |
| `cargo audit` | no vulnerabilities; 3 transitive `ratatui` warnings (§7) |

The exact results, including the advisory-scan output, are also recorded in
`docs/task-026-progress.md`.

---

## 9. Conclusion

The audit found and fixed nine concrete hardening issues (SEC-01…SEC-09) plus
one behavior-preservation correction (SEC-14), covering the highest-value areas:
child-process environment, configuration-injected shell, secret redaction, HTTP
header injection, IPC resource limits, file permissions, response bounds, and
the desktop CSP. It verified the platform's core positive controls and recorded
the remaining risks honestly.

The application is **not** claimed to be perfectly secure. It is a local,
single-user developer tool whose security boundary is the owner-only Unix
socket and the same-user trust model; within that model, authorization is
enforced consistently, destructive actions fail closed, secrets are kept out of
interfaces, and builds are understood to run project code by design.
