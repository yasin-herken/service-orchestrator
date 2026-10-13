# Task 029 — Documentation & Developer Onboarding: progress record

This file is a working record for Task 029. It captures the audit, the plan, and
what was verified, so the work can be resumed if context is lost. It is not a
user-facing document.

## Ground truth inspected

- `AGENTS.md`, `Cargo.toml`, `rust-toolchain.toml`, `src/main.rs`,
  `src/probe.rs`, `src/repository.rs`.
- Crates: `config`, `domain`, `client`, `daemon`, `application`, `execution`,
  `policy`, `git`, `process`, `build`, `runtime`, `liquibase`, `health`,
  `logging`, `mcp`, `tui`.
- `apps/desktop` (React/TS) and `apps/desktop/src-tauri` (Tauri adapter).
- `scripts/test-*.sh`, `examples/config.toml`, and the existing `docs/`.

## Key verified facts

- Binary: `service-orchestrator [tui|mcp|daemon|daemon status|daemon stop]`,
  options `-c/--config <path>`, `--config=<path>`, `-h/--help`, `-V/--version`.
- Config precedence: CLI → `SERVICE_ORCHESTRATOR_CONFIG` →
  `~/.service-orchestrator/config.toml`.
- Socket: `$SERVICE_ORCHESTRATOR_SOCKET` or `<config_dir>/run/daemon.sock`.
- Env vars: `SERVICE_ORCHESTRATOR_CONFIG`, `SERVICE_ORCHESTRATOR_SOCKET`,
  `SERVICE_ORCHESTRATOR_NO_AUTOSTART`, `SERVICE_ORCHESTRATOR_DAEMON_BIN`.
- Rust: stable toolchain, MSRV 1.88. Node: 20.19.x (deps require >=20.19.0).
- MCP: 73 tools, stdio transport, read-only resources/templates, 5 prompts;
  MCP is non-interactive (cannot perform destructive operations).
- GUI: sidebar navigation, 12 nav entries, Configuration screen edits
  `config.toml` through the daemon; `bundle.active=false` (no DMG).
- TUI: 10 nav screens + service detail; keybinding map in `crates/tui/src/app.rs`.

## Audit findings (stale/inaccurate docs)

1. `docs/local-daemon.md` §13 said configuration is read-only to clients; the
   daemon now owns the only writer and exposes commit/reload. Removed the stale
   bullet (it contradicted §9).
2. `docs/desktop-gui.md` §1 and §26 said service/workspace creation/editing is
   not delivered; the Configuration screen (Task 023) implements it. Fixed the
   scope paragraph, the delivered/not-delivered table, the file list, and the
   §26 "out of scope" bullet.
3. `docs/tui.md` listed `W` as "cancel workflow" on Tasks (unreachable: the
   global `W` cycles workspace first) and `R` as "restart selection" (not bound).
   Also removed the unreachable `W`-cancel sentence in §5 and noted the
   workspace sync/build actions have no key binding. Fixed.
4. `docs/mcp.md` labelled `check_service_health` / `check_service_readiness` as
   "safe write"; the catalog advertises them read-only. Changed to
   "read (async)" (they return a task id, matching live-probe reads). Verified
   live: `tools/list` reports 73 tools, 5 prompts, 40 read-only hints.
5. Root `README.md` did not mention the `daemon` subcommands, prerequisites, a
   quick start, or the docs index. Improved.
6. `docs/architecture.md` §"Latest evolution" repeated the stale
   "configuration snapshot (read-only to clients)" wording. Corrected.
7. `docs/adr/015-shared-local-runtime.md` repeated the same stale wording.
   Corrected (the daemon is the only writer; clients commit through it).
8. `docs/README.md` referenced a non-existent placeholder progress path
   (`task-0NN-progress.md`); reworded to a real example.

## Documents created

- `docs/README.md` (navigation)
- `docs/getting-started.md`
- `docs/workspaces.md`
- `docs/service-management.md`
- `docs/git-operations.md`
- `docs/builds.md`
- `docs/libraries.md`
- `docs/migrations.md`
- `docs/health.md`
- `docs/logs-and-diagnostics.md`
- `docs/workflows.md`
- `docs/troubleshooting.md`
- `docs/error-reference.md`
- `docs/development.md`
- `docs/known-limitations.md`
- `docs/onboarding-checklist.md`
- `scripts/check-docs.sh` (documentation validation)

## Documents updated

- `README.md` (rewritten: features, prerequisites, quick start, daemon
  subcommands, interfaces, configuration, development, layout, docs validation)
- `docs/local-daemon.md` (removed stale config read-only limitation)
- `docs/desktop-gui.md` (configuration scope, table, file list, §26)
- `docs/tui.md` (keyboard table: `R` refresh vs restart-selection, `W` cycles
  workspace, unreachable workflow cancel, workspace sync/build note)
- `docs/mcp.md` (health tools are read-only)
- `docs/architecture.md` (configuration ownership wording)
- `docs/adr/015-shared-local-runtime.md` (configuration ownership wording)
- `docs/README.md` (replaced a placeholder path)

## Validation

- `cargo fmt --check` — clean.
- `cargo check --workspace` — clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean.
- `cargo test --workspace` — 977 tests passed, 0 failed (60 test targets).
- `./scripts/check-docs.sh --with-config-test` — links, referenced paths, script
  executability, and the example config (including the authoritative Rust parse
  test `loads_the_full_example_configuration`) all pass.
- `apps/desktop`: `npm run typecheck`, `npm run test` (231 tests, 27 files),
  `npm run build` — all pass.

## Onboarding simulation (verified)

Against `examples/config.toml`, with an isolated socket:

1. `service-orchestrator --version` → `service-orchestrator 0.1.0`.
2. `service-orchestrator --help` → documents `tui`, `mcp`, `daemon`,
   `daemon status`, `daemon stop`, `-c/--config`, `-h`, `-V`.
3. `daemon status` (no autostart) → `stopped`, with the derived endpoint.
4. `daemon` (foreground) → started against the example config; `daemon status`
   reported it running with the derived endpoint and protocol version 1.
5. MCP stdio session (`initialize` → `list_workspaces` →
   `get_workspace_status` → `tools/list` → `prompts/list`) → returned the `acme`
   workspace with `auth-service` and `web-ui`, exactly as the config declares;
   73 tools and 5 prompts, matching `docs/mcp.md`.
6. `daemon stop` → clean shutdown; `daemon status` → `stopped`.

Steps not verifiable without a real workspace: cloning/building real
repositories, starting real services, and running real migrations (the example
paths do not exist on this machine).
