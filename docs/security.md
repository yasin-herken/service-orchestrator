# Security

Service Orchestrator is a **local, single-user** developer orchestration
platform. It runs on macOS, manages the current user's repositories and
processes, and is reachable only from that user's own session. It is not a
server, it has no remote backend, and it does not require an LLM.

This document describes the security model **as implemented**, the controls that
are in force, and — honestly — the guarantees it does *not* make. The companion
`docs/security-audit.md` records the audit that produced this document: the
threat model, the confirmed findings, the fixes, and the risks that remain.

The security invariants of `AGENTS.md` §6 are the contract:

1. All privileged operations pass through the Application Core.
2. All interfaces use the same policy engine.
3. MCP cannot bypass security restrictions.
4. No arbitrary shell execution is exposed by default.
5. Destructive Git operations require authorization.
6. Database migrations require target-aware policy checks.
7. Unrelated OS processes are never terminated.
8. Secrets are not exposed through logs or API responses.
9. Configuration changes are validated.
10. Untrusted input is never interpolated into shell commands.

---

## 1. Threat model in one page

### What is protected

- The developer's working trees: uncommitted changes, branches, remotes.
- Managed service processes and their lifecycle.
- Database migration targets and the data they mutate.
- Credentials: Git authentication, database passwords, tokens, registry
  credentials, Maven/npm settings.
- The configuration file and the daemon's runtime directory.
- The integrity of the one authoritative runtime state (tasks, processes,
  configuration).

### Who is trusted

- **The current local user.** The daemon trusts processes running as the same
  UID. This is the *same-user* trust boundary: anything that can already run as
  the user can already read the user's repositories, credentials, and files.
- **The developer's configuration.** Commands, paths, and environment are
  supplied by the user who owns the machine.

### Who is not trusted

- **AI agents / MCP clients.** Tool arguments are untrusted input.
- **Repository content.** Files, `.git` state, logs, build output, and migration
  output are untrusted data; they may contain hostile instructions aimed at an
  agent whose context includes them.
- **The frontend (React/TUI).** It is a presentation layer, never an
  authorization boundary.
- **Another local process that is not the user.** It cannot reach the daemon
  (§2); the kernel prevents it.
- **A malformed or oversized request.** Every input is validated and bounded.

### Out of scope

- A malicious *same-user* process (it already owns the user's session).
- A compromised operating system or kernel.
- Remote/multi-user operation. The daemon is not exposed over the network.
- Sandboxing builds. Maven plugins and npm scripts execute arbitrary project
  code by design (§6) — running a build is running code the user chose to run.

---

## 2. Trust boundaries and the local IPC

```text
React GUI ─────┐
               │
Ratatui TUI ───┼──► Local IPC (Unix domain socket) ──► Shared Daemon
               │                                          │
MCP Server ────┘                                          ▼
                                                   Application Core
                                                          │
                                                     Policy Engine
                                                          │
                                                    Infrastructure
```

The single boundary that every interface crosses is the **daemon's Unix domain
socket**. It is:

- a Unix domain socket, **never a TCP port**. Nothing binds a network address;
  the daemon is not reachable from another host.
- created in a `0700` runtime directory, with the socket itself at `0600`
  (`crates/client/src/socket.rs`, `crates/daemon/src/server.rs`). The runtime
  directory is derived from the active configuration directory, so two
  configurations get two daemons rather than one shared one.
- bound behind an advisory `flock` lock, so exactly one authoritative daemon
  owns a runtime directory. A stale socket file is removed and rebound safely; a
  crash never leaves the directory permanently locked.

### Peer identity comes from the kernel

The daemon does **not** trust the `client` field in the request envelope. On
accept it reads `peer_cred()` and compares the peer's UID to the daemon's
(`crates/daemon/src/server.rs`). That "same user" fact is a kernel assertion, not
a client claim.

This is the boundary's whole authentication model, and it is deliberately
minimal. The wire-declared `ClientKind` (`tui`/`desktop`/`mcp`/`cli`) is **not**
authentication; it is a hint used only, together with the kernel UID and an
explicit `allow_destructive` intent, to decide whether a destructive operation
may be authorized (§4).

### Protocol validation

- The transport is length-prefixed JSON; a frame is capped at 8 MiB
  (`MAX_FRAME_BYTES`), so a hostile client cannot make the daemon allocate
  unbounded memory.
- A request whose `protocol` version differs is refused **before** it is
  interpreted (`PROTOCOL_INCOMPATIBLE`).
- An unknown method, a malformed frame, or a mistyped payload produces a
  structured error; it never crashes the daemon.
- The daemon caps simultaneous connections (`MAX_CONNECTIONS = 128`) and bounds
  the wait for the **first** request on a connection (30 s). Later reads are
  unbounded so a subscribed client can sit idle waiting for pushed events.

---

## 3. Authorization: one policy engine

Every mutating operation is classified with the domain `Permission`
(`Read`, `SafeWrite`, `Destructive`) and checked by the shared `PolicyEngine`
(`crates/policy`). The engine is small and pure: read and safe-write operations
are always allowed; a destructive operation requires an `Authorization` that
explicitly permits it.

The Application Core runs the check before scheduling work, so **the TUI, the
desktop GUI, the CLI, and MCP cannot differ**. MCP is not a privileged path.

### How a destructive operation is authorized

A destructive request is honored only when **all three** trust signals hold
(`crates/daemon/src/dispatch.rs::destructive_allowed`):

1. the request asks for it (`allow_destructive == true`);
2. the client declares an **interactive** kind (`tui` or `desktop`) — MCP and
   CLI never qualify; and
3. the connection's **kernel-verified peer UID** matches the daemon's.

A caller-supplied `confirmed: true` is treated as *intent*, never as
authorization. There is no MCP tool that sets it, and even if one did, signals 2
and 3 would still have to hold. When a request does not qualify, the operation
fails **closed** with a structured `CONFIRMATION_REQUIRED` / `PERMISSION_DENIED`
error, and the policy engine still runs.

```text
client ── request ─▶ daemon ── kernel peer UID ─┐
                            ── client kind ─────┼──▶ PolicyEngine ──▶ allow / deny
                            ── intent flag ─────┘
```

---

## 4. MCP security

MCP is a thin adapter over the same Application Core (`docs/mcp.md`). It is
**non-interactive**, so it can never self-authorize a destructive operation.

- **Strict schemas.** Every tool's input is a typed struct with
  `deny_unknown_fields`; an unmodelled property is a protocol error, and an
  illegal domain value becomes a structured `INVALID_ARGUMENT` rather than a
  panic or an opaque failure.
- **No shell, no arbitrary filesystem.** There is deliberately no `run_shell`
  tool, no arbitrary database execution, and no raw path input; an operation is
  always a named use case over validated resource ids.
- **Redaction.** Secret environment values serialize as `***` in the domain, so
  they cannot leak through a tool result, a resource, or an error. Tool
  invocation logs record the tool name, outcome, duration, and error code — never
  an argument value.
- **Bounded responses.** Log and task listings always carry a limit; a task list
  is bounded even when no limit is supplied (`DEFAULT_TASK_LIST_LIMIT`, clamped
  to `MAX_TASK_LIST_LIMIT`).
- **Untrusted content is data, not instructions.** Repository files, logs, and
  command output that an agent reads are untrusted. They cannot change an
  authorization decision because authorization is computed by code from the
  kernel UID, the client kind, and the intent flag — never from model output.

---

## 5. Desktop GUI (Tauri) security

The desktop app is a Tauri 2 webview. Its security surface is deliberately tiny
(`docs/desktop-gui.md` §7):

- **Capabilities:** `core:default` only. No shell, filesystem, HTTP, or process
  plugin is granted.
- **CSP (production):** `default-src 'self'`, `connect-src 'self' ipc:`, plus
  `base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action
  'none'`. No remote code can be loaded or framed, and no form can post
  elsewhere.
- **CSP (dev):** adds only the Vite dev origin and HMR WebSocket.
- **Navigation:** the `tauri:` scheme always; the dev origin only in debug
  builds. External navigation is blocked.
- **No secrets in the bundle.** No command returns a secret; the frontend never
  renders one and never relies on client-only validation.

Every mutating command passes through the daemon's policy engine exactly as the
TUI and MCP do.

---

## 6. Execution safety: Git, builds, and migrations

### Structured commands, never a shell

Commands are a structured program plus argument vector (`CommandSpec`), never a
shell string. There is no `sh -c` in the Git, build, or migration executors.
Configuration that tries to configure a shell command (`sh -c "…"`,
`bash -c "…"`) is **rejected at validation time** so it can never re-introduce
shell interpretation (`crates/config/src/validation.rs::reject_shell_command`).
Running a shell *script* without `-c` remains allowed.

### Git

- Structured arguments only; clone uses `--` so a path cannot be read as an
  option; branch names are validated.
- `pull` is fast-forward-only by default; a dirty working tree blocks a checkout
  or pull. Nothing silently stashes, resets, cleans, or force-checks-out.
- `git reset --hard`, `git clean`, and `git discard` are `Destructive` and gated
  by the policy engine.
- The child Git process is stripped of repo-redirecting and code-executing
  environment variables (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`,
  `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_COMMON_DIR`,
  `GIT_SSH`, `GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF`, `GIT_CONFIG`,
  `GIT_CONFIG_COUNT`). An inherited value can no longer redirect a Git operation
  outside the resolved checkout or make Git execute arbitrary code. The
  developer's **global and system Git configuration is intentionally left in
  place**, so existing authentication (credential helpers, SSH config) keeps
  working.
- Credential-bearing URLs are never logged (redaction, §7). SSH host
  verification is never disabled, and no insecure credential helper is
  configured.

### Builds and migrations

- Maven and npm run as configured, structured commands in a validated working
  directory. **Builds are not sandboxed**: a Maven plugin or npm script can
  execute arbitrary code. That is inherent to running a build, and this document
  does not claim otherwise.
- Migration operations are target-aware. A non-local database target requires
  destructive authorization (the three trust signals of §3); a read-only
  operation (validate, status, pending, preview) does not. A failed migration is
  never retried automatically, a Liquibase lock is never released
  automatically, and cancelling a migration never claims to roll back committed
  database changes.
- Captured build and migration output is scrubbed before it is stored or
  returned: whole lines naming a credential (JDBC URLs, password flags) are
  dropped, and the remaining text is passed through the shared redaction rule
  (§7).

---

## 7. Secrets and redaction

Two complementary controls keep secrets out of logs, task results, IPC frames,
and errors:

1. **Do not put secrets in.** Environment values marked `secret` render as `***`
   in the domain, so they cannot be serialized; a task description and a
   redacted config view never include the value.
2. **Scrub obvious shapes from free text.** The canonical redaction rule lives
   in the domain (`crates/domain/src/redact.rs`, re-exported through
   `service_orchestrator_logging::redact`) and is applied by the log hub, the
   build adapter, the migration adapter, and the process description. It masks:

   - secret-looking `key=value` pairs (`password=`, `token=`, `api_key=`,
     `authorization=`, …), including `--flag=value` forms;
   - credential-bearing URLs (`scheme://user:pass@host`);
   - `Bearer`/`Basic` scheme tokens.

   The rule is **best-effort and does not pretend to be complete**. It catches
   obvious shapes; an unrecognized secret shape in free text may survive.
   Preventing a secret from entering a log in the first place is preferred over
   scrubbing it afterwards.

### Permissions on the files that can hold secrets

| File | Mode | Control |
| --- | --- | --- |
| Runtime directory `run/` | `0700` | `ensure_run_dir` |
| `daemon.sock` | `0600` | set at bind time |
| `daemon.log` (auto-started) | `0600` | created `0600`; an existing loose file is re-tightened |
| config `config.toml` (new) | `0600` | `set_owner_only` |
| config `config.toml.bak` | `0600` | set after each backup |
| Process command description | redacted | `ProcessSpec::description` |

---

## 8. Process ownership

The daemon owns every managed service process and is the only thing that signals
them.

- Only processes the daemon itself started are tracked; it **never** signals a
  process by PID or by port. A process that reused a PID, or that merely occupies
  a configured port, is never harmed.
- Managed processes are spawned in their own process group and stopped gracefully
  (SIGTERM) before a forced termination, so children are cleaned up and not
  orphaned.
- Runtime state is **in memory**. After a crash or restart the daemon adopts
  nothing and reports zero managed services; surviving processes become
  unmanaged rather than misattributed.

---

## 9. Resource limits

| Resource | Bound |
| --- | --- |
| IPC frame | 8 MiB (`MAX_FRAME_BYTES`) |
| Simultaneous IPC connections | 128 (`MAX_CONNECTIONS`) |
| First request on a connection | 30 s (`INITIAL_REQUEST_TIMEOUT`) |
| Task listing | default 100, hard max 1000 (`clamp_task_limit`) |
| Log page | hard maximum enforced by the log hub |
| Diff patch | `MAX_DIFF_BYTES = 1 MiB` (clamped) |
| Scheduled concurrency | `[execution] max_parallel_tasks` |

An unbounded task *history* is still retained in memory; retention/eviction is
deferred (§11). Listing is bounded regardless, so a single request cannot
serialize an unbounded history.

---

## 10. Configuration security

- Configuration is schema-validated on load; unknown fields, invalid ids,
  invalid paths, and shell commands are rejected with structured error codes.
- A change is validated and committed atomically (temp file + rename) with an
  optimistic revision check, so a concurrent writer is detected.
- Secret environment values are redacted in the change view and in the entity
  field view returned to clients.
- The daemon owns the live configuration; clients can read a summary but never
  mutate the file directly.

---

## 11. Honest limitations

These are real and are not claimed to be solved:

- **Same-user only.** There is no authentication beyond "runs as the same UID".
  Any process already running as the user can connect. This does not extend the
  user's existing trust boundary, but it means the daemon is not a protection
  against a malicious same-user process.
- **No remote access.** There is no HTTP listener and no network transport. That
  is a feature, not a missing one.
- **Builds run project code.** Maven/npm are not sandboxed.
- **Redaction is best-effort.** See §7.
- **Configuration may inline a secret.** New config files are `0600` and the
  backup is `0600`, but an already-restrictive-or-not mode on an existing file is
  preserved. A user who inlines a secret into a world-readable config file keeps
  it world-readable. Prefer referencing a secret by environment-variable name.
- **Task history is retained in memory** and not evicted; listing is bounded,
  retention is future work.
- **HTTP health-check URLs are not allow-listed.** The URL is validated for
  control characters and host shape, but a configured URL may still point at an
  arbitrary host (including a local tunnel to a remote database). The target is
  developer configuration; a strict allow-list is a deferred hardening step.
- **Managed-process recovery after a crash is not implemented.** See §8.

---

## 12. Related documents

- `docs/security-audit.md` — the audit: threat model, findings, fixes, tests.
- `docs/local-daemon.md` — the daemon, IPC, and authorization boundary.
- `docs/mcp.md` — MCP tools, schemas, and policy behavior.
- `docs/desktop-gui.md` — Tauri capabilities, CSP, and commands.
- `docs/process-manager.md` — process ownership and termination.
- `docs/migration-manager.md` — migration targets and authorization.
- `docs/git-manager.md` — Git operations and safety.
- `docs/configuration.md` — configuration loading and management.
- `docs/domain-model.md` — `Permission`, `Environment`, `CommandSpec`.
- `docs/adr/001-core-architecture.md`, `docs/adr/015-shared-local-runtime.md`.
