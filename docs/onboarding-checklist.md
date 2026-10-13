# Onboarding Checklist

A practical checklist for a developer's first session with Service Orchestrator.
Copy it, check items off, and follow the linked guides when something needs more
detail.

## Environment

- [ ] macOS with the Xcode Command Line Tools (`xcode-select --install`).
- [ ] Rust stable installed (<https://rustup.rs>); `cargo --version` works.
- [ ] Node.js 20.19.x and npm installed (`node --version`, `npm --version`).
- [ ] `git` installed (`git --version`).

## Build

- [ ] `git clone <clone-url> service-orchestrator && cd service-orchestrator`.
- [ ] `cargo build --bin service-orchestrator` succeeds.
- [ ] `cargo run --bin service-orchestrator -- --help` prints usage.
- [ ] `cargo run --bin service-orchestrator -- --version` prints the version.

## Configuration

- [ ] `mkdir -p ~/.service-orchestrator`.
- [ ] `cp examples/config.toml ~/.service-orchestrator/config.toml`.
- [ ] Edit each workspace `root` to an existing directory.
- [ ] Edit each service/library `repository.url`, `path`, and `default_branch`.
- [ ] (optional) Put the config somewhere else and export
      `SERVICE_ORCHESTRATOR_CONFIG=/path/to/config.toml`, or pass
      `--config /path/to/config.toml`.
- [ ] Validate it: the daemon starts without a `ConfigError`
      (`service-orchestrator daemon`), or use the GUI Configuration screen's
      **Validate**.

## Daemon

- [ ] `service-orchestrator daemon status` reports stopped (before first use).
- [ ] Launch an interface and confirm the daemon auto-starts.
- [ ] `service-orchestrator daemon status` now reports running with a PID,
      endpoint, and protocol version.
- [ ] Read `<config_dir>/run/daemon.log` if the daemon failed to start.

## First interface

- [ ] Open the **desktop GUI** (`cd apps/desktop && npm install && npm run tauri dev`),
      **or** the **TUI** (`service-orchestrator tui`).
- [ ] Switch to your workspace.
- [ ] Confirm the connection indicator / header shows a live daemon.

## First operations

- [ ] List services (`list_services` / Services screen).
- [ ] Inspect a Git status (`git_status` / Git screen, refresh).
- [ ] Validate the workspace (`validate_workspace`) and review the report.
- [ ] Run a safe operation: build a service (`build_service` / `b` / Build).
- [ ] Watch the task reach a terminal state (Tasks screen / `get_task`).
- [ ] View the task or service logs.
- [ ] Inspect health (`get_service_health` / Health screen).

## Optional deeper checks

- [ ] Plan an environment preparation (`plan_prepare_environment`) and review the
      resolved dependencies and phases.
- [ ] Prepare a small subset (`prepare_environment` with sync only).
- [ ] Verify a library artifact and inspect version alignment.
- [ ] Start and stop a service; confirm the port and process state.
- [ ] Run the local gate: `./scripts/test-all.sh`.
- [ ] Run the documentation check: `./scripts/check-docs.sh`.

## If something fails

`service-orchestrator daemon status`, then the error `code`, then
[`troubleshooting.md`](troubleshooting.md) and
[`error-reference.md`](error-reference.md).

## Read next

- [`getting-started.md`](getting-started.md)
- [`configuration.md`](configuration.md)
- [`workflows.md`](workflows.md)
- [`known-limitations.md`](known-limitations.md)
