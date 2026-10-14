# Service Orchestrator

Service Orchestrator is a local macOS developer tool for managing multiple
microservice repositories, builds, database migrations, running services, and
development workflows through a unified desktop GUI, terminal interface, and MCP
integration.

It reduces the repetitive work of preparing a developer environment: instead of
opening every project and remembering the exact `git`/`mvn`/`npm`/`liquibase`
incantations, a developer selects services and lets one tool synchronize
repositories, build shared libraries, apply database migrations, build and start
applications, run health checks, and show logs.

It is written in Rust and is **configuration-driven**: workspaces, services,
libraries, groups, profiles, runtimes, commands, dependencies, health checks, and
ports live in a versioned TOML file, never in source.

## Features

- Git repository management: status, fetch, checkout, pull, sync, and guarded
  destructive operations.
- Maven and npm builds, including Maven Wrapper support.
- Shared-library preparation, local install, artifact verification, and consumer
  version alignment.
- Liquibase migrations with target-aware safety.
- Local service lifecycle: start, stop, restart, port-conflict reporting, and
  process ownership by the daemon.
- Health checks: process, TCP, and HTTP (Spring Boot Actuator).
- A unified, bounded, filterable log store with task and service correlation.
- Dependency-aware environment workflows with a read-only plan.
- Multiple workspaces, service groups, and developer profiles.
- Three interfaces over one shared application core: a Ratatui terminal UI, an
  MCP server for AI agents, and a Tauri 2 desktop GUI.

AI is optional. The core works with no LLM, no MCP client, no internet, and no
remote backend.

## Architecture at a glance

```text
   Ratatui TUI     MCP server     Tauri desktop GUI
        │              │                 │
        └──────────────┼─────────────────┘
                       │  OrchestratorClient (local IPC, Unix domain socket)
                       ▼
                 shared local daemon
                       │
                 application core
                       │
            task / workflow engine + policy
                       │
   git · process · build · runtime · liquibase · health · logging
```

The most important rule in this repository is that the application core never
depends on the TUI, the GUI, or MCP. All three are interface adapters over the
same core, so they observe identical behavior, policies, and errors. One local
daemon owns the single task engine, process registry, and log store; the
interfaces are clients of it, so a task started in one is visible in the others.

- [`docs/architecture.md`](docs/architecture.md) — the full design.
- [`docs/local-daemon.md`](docs/local-daemon.md) — the shared runtime.
- [`docs/architecture-simplification.md`](docs/architecture-simplification.md) —
  the latest simplification audit.

## Requirements

Application development (building this repository):

- macOS (Apple Silicon supported; Intel macOS is feasible).
- Rust stable toolchain — the workspace MSRV is **1.88** (see
  `rust-toolchain.toml` and the root `Cargo.toml`).
- `git`.
- Node.js **20.19.x** (the desktop dependencies require `>=20.19.0`) and `npm`.
- Xcode Command Line Tools (for the Tauri desktop build).

Running the desktop GUI additionally needs the Tauri 2 prerequisites and a
buildable `service-orchestrator` binary (the preflight script builds it).

Managed services may need their own tools — Java, Maven or a Maven Wrapper,
Node/npm — depending on the workspace configuration. Not every developer needs
every tool; only the ones the configured services require. `validate_workspace`
reports which are missing.

## Quick start

```bash
# 1. Build the binary.
cargo build --bin service-orchestrator

# 2. Create a configuration file.
mkdir -p ~/.service-orchestrator
cp examples/config.toml ~/.service-orchestrator/config.toml
#    …then edit it so the workspace root and repositories match your machine.

# 3. Launch an interface. `tui` or `mcp` start the shared daemon if it is not
#    already running, and connect to it.
cargo run --bin service-orchestrator -- tui
```

The default (no-argument) command is also the TUI:

```bash
service-orchestrator              # terminal interface
service-orchestrator mcp          # MCP server on stdin/stdout
service-orchestrator mcp --config /path/to/config.toml
service-orchestrator daemon       # run the shared runtime in the foreground
service-orchestrator daemon status
service-orchestrator daemon stop
```

See [`docs/getting-started.md`](docs/getting-started.md) for a guided first run.

## Interfaces

- **Desktop GUI** — a Tauri 2 + React application under `apps/desktop`:

  ```bash
  cd apps/desktop
  npm install
  npm run tauri dev     # preflight builds the daemon binary, then opens the window
  ```

  See [`docs/desktop-gui.md`](docs/desktop-gui.md).

- **Terminal UI** — `service-orchestrator tui` (the default command). See
  [`docs/tui.md`](docs/tui.md).

- **MCP** — `service-orchestrator mcp`, stdio transport. See
  [`docs/mcp.md`](docs/mcp.md).

All three are clients of the same daemon and observe identical state.

## Install on macOS

A ready-to-install macOS application — `Service Orchestrator.app` and a
`Service Orchestrator_<version>_<arch>.dmg` — can be produced locally:

```bash
./scripts/build-macos.sh              # native Apple Silicon
./scripts/build-macos.sh --arch x86_64     # Intel (needs the Rust target)
./scripts/build-macos.sh --arch universal  # both (needs both Rust targets)
./scripts/verify-macos-bundle.sh      # bundle structure, architecture, signature, DMG
```

Install by opening the DMG and dragging **Service Orchestrator** onto the
**Applications** shortcut. The window then auto-starts the bundled orchestrator
as the shared daemon; no terminal, no PATH setup, and no Rust/Node toolchain are
required to run it.

- **Architectures:** Apple Silicon is the verified primary target. Intel and
  universal builds are supported by the script when the matching Rust target is
  installed; see [`docs/macos-distribution.md`](docs/macos-distribution.md).
- **Signing/notarization:** optional. With no Apple credentials the script
  ad-hoc signs and seals the bundle, so Gatekeeper shows the normal
  "unidentified developer" prompt on first launch. Set
  `APPLE_SIGNING_IDENTITY` (and the Apple notarization variables) to sign and
  notarize.
- Bundling the app never requires signing credentials, and never installs a
  Rust, Cargo, or Node toolchain on the user's machine.

See [`docs/macos-distribution.md`](docs/macos-distribution.md) for the full
guide and [`docs/packaging-report.md`](docs/packaging-report.md) for the
reference build's measured results.

## Configuration

The default configuration file is `~/.service-orchestrator/config.toml`. The
command line `-c/--config` wins, then the `SERVICE_ORCHESTRATOR_CONFIG`
environment variable, then the default.

- [`docs/configuration.md`](docs/configuration.md) — schema, defaults,
  validation, and the configuration-management layer.
- [`examples/config.toml`](examples/config.toml) — a complete, generic example
  that exercises every section.

## Where to find the rest

- [`docs/README.md`](docs/README.md) — the full documentation index.
- [`docs/getting-started.md`](docs/getting-started.md) — first-time setup.
- [`docs/troubleshooting.md`](docs/troubleshooting.md) — common problems.
- [`docs/known-limitations.md`](docs/known-limitations.md) — verified limits.

## Development

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Convenience scripts wrap these into test categories:

```bash
./scripts/test-unit.sh          # fast Rust unit tests
./scripts/test-integration.sh   # Rust integration tests
./scripts/test-security.sh      # security regression suite
./scripts/test-frontend.sh      # desktop type check + Vitest
./scripts/test-e2e.sh           # real binaries and daemon (see the script for opt-ins)
./scripts/test-all.sh           # the full local gate
```

See [`docs/development.md`](docs/development.md) and
[`docs/testing.md`](docs/testing.md).

## Workspace layout

```text
crates/
├── domain/        # pure business concepts and rules
├── execution/     # task and workflow engine
├── policy/        # permission model and policy engine
├── config/        # versioned configuration
├── application/   # use cases, orchestration, and ports
├── git/           # infrastructure
├── process/       # infrastructure
├── build/         # infrastructure
├── runtime/       # infrastructure
├── liquibase/     # infrastructure
├── health/        # infrastructure
├── logging/       # infrastructure
├── client/        # the OrchestratorClient boundary and IPC transport
├── daemon/        # the shared local runtime
├── mcp/           # interface adapter
└── tui/           # interface adapter
apps/
└── desktop/       # interface adapter (Tauri 2 GUI; its own Cargo workspace)
src/               # composition root (service-orchestrator binary)
docs/              # documentation
examples/          # example configuration
scripts/           # test and validation scripts
```

## Documentation validation

A lightweight script checks local documentation links, referenced files, the
example configuration, and referenced scripts:

```bash
./scripts/check-docs.sh
```

## License

Apache-2.0. See [`LICENSE`](LICENSE).
