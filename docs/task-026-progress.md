# Task 026 progress — Security Hardening & Permission Management

Status: **complete**. See `docs/security.md` (reference) and
`docs/security-audit.md` (audit: threat model, findings, fixes, tests).

## Environment

| Item | Value |
| --- | --- |
| Machine | Apple M2 Pro, 32 GiB RAM |
| OS | macOS (Darwin) |
| Arch | arm64 |
| rustc / cargo | 1.99.0 |
| Node / npm | v22.18.0 / 11.11.1 |
| Build mode | debug (test/clippy) |

## Plan

1. Audit the platform against the `AGENTS.md` §6 invariants; record confirmed,
   exploitable issues (not hypothetical ones).
2. Implement targeted, low-risk fixes that preserve the architecture.
3. Add regression tests and adapt pre-existing tests broken by the hardening.
4. Write `docs/security-audit.md` and `docs/security.md`; update related docs.
5. Run full validation (`fmt`, `check`, `test`, `clippy`, frontend scripts, and
   available dependency audits).

All steps done.

## Confirmed findings (all fixed unless noted)

| ID | Finding | Severity | Status |
| --- | --- | --- | --- |
| SEC-01 | Git child inherited `GIT_DIR`/`GIT_WORK_TREE`/`GIT_SSH_COMMAND`/… | Medium | Fixed |
| SEC-02 | Config could define a shell command (`sh -c`) | Medium | Fixed |
| SEC-03 | Build/migration output and process descriptions not consistently redacted | Medium | Fixed |
| SEC-04 | HTTP health URL accepted CRLF/whitespace → header injection | Medium | Fixed |
| SEC-05 | IPC had no connection cap / first-request timeout | Medium | Fixed |
| SEC-06 | Auto-started `daemon.log` could be world-readable | Low | Fixed |
| SEC-07 | Config `.toml.bak` could be world-readable | Low | Fixed |
| SEC-08 | Unbounded task listing and diff size | Low | Fixed |
| SEC-09 | Tauri CSP missing `base-uri`/`object-src`/`frame-ancestors`/`form-action` | Low | Fixed |
| SEC-14 | MCP `list_tasks` no-limit path silently changed semantics | Info | Fixed (behavior preserved) |
| SEC-10 | Config may inline a plaintext secret; existing mode preserved | Medium | Accepted / documented |
| SEC-11 | `ClientKind` is not authentication (same-user shutdown/intent) | Low | Accepted / documented |
| SEC-12 | HTTP health target has no allow-list | Low | Deferred |
| SEC-13 | No task-history eviction | Low | Deferred |

## Implemented

- **Redaction as a domain invariant.** Moved the canonical rule to
  `crates/domain/src/redact.rs`; `logging::redact` re-exports it. Applied it to
  build capture (`build/src/manager.rs`), migration output
  (`liquibase/src/manager.rs`, which also drops whole credential-bearing lines),
  and `ProcessSpec::description`.
- **Process environment control.** `ProcessSpec.removed_environment` with
  `remove_env`/`remove_envs`; `build_command` applies `env_remove` before
  explicit `env` entries. Git applies `HARDENED_ENVIRONMENT_REMOVALS` in
  `git/src/manager.rs` (`git/src/command.rs` defines the list), keeping global/
  system Git config for auth.
- **Shell-command rejection in config.** `reject_shell_command` in
  `config/src/validation.rs`, applied to both the string and structured command
  forms.
- **HTTP URL hardening.** `parse_http_url` rejects empty URLs, control
  characters, whitespace, and invalid host characters (`health/src/http.rs`).
- **IPC hardening.** `MAX_CONNECTIONS = 128` reserved before spawn;
  `INITIAL_REQUEST_TIMEOUT = 30 s` bounds only the first request via a testable
  `read_request` helper (`daemon/src/server.rs`).
- **File permissions.** `open_daemon_log` creates/re-tightens `daemon.log` to
  `0600` (`client/src/socket.rs`); `persist` sets `.toml.bak` to `0600`
  (`config/src/management.rs`).
- **Response bounds.** `DEFAULT_TASK_LIST_LIMIT = 100`,
  `MAX_TASK_LIST_LIMIT = 1000`, `clamp_task_limit` (`execution`); daemon
  `LIST_TASKS` truncated to the newest window and `LIST_RECENT_TASKS` clamped
  (`daemon/src/dispatch.rs`); MCP `list_tasks` always bounded while preserving
  active-plus-finished semantics (`mcp/src/server.rs`); diff `max_bytes` clamped
  to `MAX_DIFF_BYTES = 1 MiB` on both the builder
  (`application/src/ports/git.rs`) and the IPC rebuild path
  (`client/src/protocol.rs`).
- **Desktop CSP.** Added `base-uri 'none'; object-src 'none'; frame-ancestors
  'none'; form-action 'none'` to `csp` and `devCsp`
  (`apps/desktop/src-tauri/tauri.conf.json`).

## Tests added / adapted

- New regression tests in: `domain` (redaction), `process` (spec + manager child
  env), `git` (denylist), `config` (2 shell-rejection), `health` (2), `client`
  (2 log-permission + 1 protocol clamp), `application` (diff clamp),
  `execution` (task-limit clamp), `daemon` (3), `mcp` (bounded listing preserved
  by the existing test).
- Pre-existing tests that used `sh -c` were adapted so they no longer configure a
  shell command: `liquibase/src/manager.rs` (classify directly) and
  `liquibase/tests/*` (real executable script fixtures via `support::script`).

## Deferred (documented as remaining risks, not implemented)

- Task-history retention/eviction (needs an explicit retention policy).
- Config file mode tightening when secrets are inlined; prefer secret references.
- HTTP health-check host allow-list.
- Managed-process recovery after a crash (pre-existing, out of scope).

## Final validation

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
| `cargo audit` | no vulnerabilities; 3 transitive `ratatui` warnings (see `docs/security-audit.md` §7) |

Task 027 not started (per `tasks/026.md` §47).
