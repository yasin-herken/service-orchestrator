# Getting Started

This guide takes a new developer from a clean checkout to a running
environment: build the project, write a configuration file, start the shared
daemon, open an interface, list services, and run a safe operation.

Everything here is verified against the implementation. Where a step cannot be
verified automatically (for example starting a real backend service), it is
marked as such.

## 1. Prerequisites

### Building the application

| Tool | Why | Notes |
| --- | --- | --- |
| macOS | the target platform | Apple Silicon and Intel. |
| Rust stable | all crates | MSRV **1.88** (`rust-toolchain.toml`, root `Cargo.toml`). Install from <https://rustup.rs>. |
| `git` | Git operations and the workspace's repositories | On macOS it ships with the Xcode Command Line Tools. |
| Node.js 20.19.x + npm | the desktop GUI and its tests | The desktop dependencies require `>=20.19.0`. |
| Xcode Command Line Tools | native build for the Tauri GUI and `git` | `xcode-select --install`. |
| Tauri 2 prerequisites | only for `npm run tauri dev` | The Tauri CLI is installed by `npm install`; the system prerequisites are the webview and build tools. |

### Running managed services

The services in *your* workspace may need Java, Maven (or a Maven Wrapper),
Node.js, and npm. Install only what your configured services use. You do not
need these to build Service Orchestrator itself. Use `validate_workspace` (GUI,
TUI, or MCP) to check which tools are missing on this machine
([`workspaces.md`](workspaces.md)).

## 2. Build

```bash
git clone <your-clone-url> service-orchestrator
cd service-orchestrator
cargo build --bin service-orchestrator
```

The binary is `target/debug/service-orchestrator`. All versions of this document
assume you run it via `cargo run --bin service-orchestrator -- <args>` or after
adding `cargo build` output to `PATH`.

Confirm it works:

```bash
cargo run --bin service-orchestrator -- --help
cargo run --bin service-orchestrator -- --version
```

## 3. Create a configuration file

Service Orchestrator is configuration-driven. Start from the example:

```bash
mkdir -p ~/.service-orchestrator
cp examples/config.toml ~/.service-orchestrator/config.toml
```

Then edit `~/.service-orchestrator/config.toml`:

1. Set each workspace `root` to a directory that exists on your machine.
2. Point each service/library `repository.url` and `path` at your real
   repositories (or remove the entries you do not have yet).
3. Keep or adjust `default_branch`; there is **no default branch** assumed, so
   every repository must declare one.

The full schema is in [`configuration.md`](configuration.md). The example is a
valid document; the integration tests load it verbatim.

### Config location and override

The configuration file is resolved in this order:

1. `--config <path>` / `-c <path>` / `--config=<path>` on the command line;
2. the `SERVICE_ORCHESTRATOR_CONFIG` environment variable;
3. `~/.service-orchestrator/config.toml` (default).

The **daemon** loads and validates the file. The TUI and MCP only need the path
to derive the socket location.

## 4. Validate the configuration

Before starting anything, load the file to catch mistakes early. The simplest
check is to start the daemon; a validation error is printed and it exits
non-zero:

```bash
cargo run --bin service-orchestrator -- daemon
```

A valid configuration prints nothing and the daemon blocks, serving requests.
Stop it with Ctrl+C (foreground) or, from another terminal,
`service-orchestrator daemon stop`.

For a richer, read-only check that also inspects repositories, runtimes, and
ports, use `validate_workspace` through an interface
([`workspaces.md`](workspaces.md)).

## 5. Start the daemon and open an interface

You normally do not start the daemon by hand: `tui`, `mcp`, and the desktop GUI
start it if it is not already running, wait for it to become reachable, and
connect.

```bash
# Terminal interface (also the default no-argument command).
cargo run --bin service-orchestrator -- tui
```

Or open the desktop GUI:

```bash
cd apps/desktop
npm install
npm run tauri dev
```

`npm run tauri dev` runs a preflight that builds the `service-orchestrator`
binary (the daemon auto-start prerequisite), then starts Vite and opens the
window. The GUI is a client of the same daemon the TUI uses.

### Inspect the daemon

```bash
service-orchestrator daemon status   # not an error when stopped
```

This prints the PID, endpoint, protocol version, connected-client count, active
tasks, managed services, and start time — never a secret.

## 6. First operations

### List services

- **TUI** — press `<Tab>`/`<Left>` to reach the navigation panel, highlight
  **Services**, press `Enter`.
- **GUI** — the **Services** sidebar entry.
- **MCP** — `list_services { "workspaceId": "<id>" }`.

### Inspect Git status (read-only)

- **TUI** — **Git** screen, then `r` to refresh the status of the current row.
- **GUI** — **Git** screen, then inspect a repository.
- **MCP** — `git_status { "serviceId": "<id>" }`.

### Run a safe operation

Build a service — a `SAFE_WRITE` operation:

- **TUI** — Services screen, `b` on the current service.
- **GUI** — **Builds** screen, build a target.
- **MCP** — `build_service { "serviceId": "<id>" }`; it returns a task id.

Then watch the task:

- **TUI** — **Tasks** screen, `Enter` for task logs.
- **GUI** — **Tasks** screen.
- **MCP** — `get_task { "taskId": "<id>" }` or `task://<id>`.

A task is asynchronous: the call returns immediately and the work continues in
the daemon.

### Prepare a whole environment

Press `P` in the TUI (Dashboard or Services), or use the **Workflows** screen in
the GUI, or `plan_prepare_environment` then `prepare_environment` over MCP. The
defaults are conservative: **synchronize Git only**. See
[`workflows.md`](workflows.md).

## 7. Common first-run errors

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `ConfigError::Read` / `Parse` / `UnsupportedVersion` | Missing file, a typo, or a `config_version` other than 1 | Check the path; unknown keys are rejected — see [`configuration.md`](configuration.md) §8. |
| `unknown workspace` / `unknown service` | A reference does not resolve | Fix the id; see [`workspaces.md`](workspaces.md). |
| Daemon prints nothing but exits | The config is invalid; the error is on stderr | Run with the config explicitly and read stderr. |
| TUI/GUI cannot connect | The daemon failed to start (bad config or a socket issue) | `service-orchestrator daemon status`; see [`troubleshooting.md`](troubleshooting.md). |
| Repository reported missing | The configured `path` does not exist under the workspace root | Clone it, or fix `path`/`root`. |
| Runtime reported missing | Java/Node/Maven/npm not on `PATH` | Install it, or fix the configured version. |

The full list is in [`troubleshooting.md`](troubleshooting.md).

## What next

- [`configuration.md`](configuration.md) — model your real workspace.
- [`workspaces.md`](workspaces.md) — validation and multiple workspaces.
- [`onboarding-checklist.md`](onboarding-checklist.md) — a copyable checklist.
- The user guides, starting with
  [`service-management.md`](service-management.md).
