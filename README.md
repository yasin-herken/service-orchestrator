# Service Orchestrator

A macOS-native developer environment orchestration platform written in Rust.

Service Orchestrator prepares and manages local multi-repository, multi-service
development environments: synchronizing repositories, checking out branches,
building shared libraries, running Liquibase migrations, building and starting
applications, inspecting logs, running health checks, and executing
dependency-aware workflows.

It provides three interfaces over one shared application core:

- a **Ratatui terminal UI** for human developers,
- an **MCP server** for AI agents, and
- a **Tauri desktop GUI** for human developers (`apps/desktop`, run with
  `npm run tauri dev`).

AI is optional. The core works with no LLM, no MCP client, no internet, and no
remote backend.

## Status

This repository contains the full local orchestration platform: the Cargo
workspace and crate boundaries, the pure domain model, the versioned
configuration system, the task engine, the infrastructure adapters (Git, process,
build, library, service runtime, migration, health, and logging), the shared
application core, and three interface adapters — the Ratatui terminal UI, the MCP
stdio server, and the Tauri desktop GUI.

## Usage

```bash
service-orchestrator tui                  # the terminal interface (default)
service-orchestrator mcp                  # the MCP server on stdin/stdout
service-orchestrator mcp --config <path>  # either interface accepts --config
```

The desktop GUI is a separate application:

```bash
cd apps/desktop
npm install
npm run tauri dev                         # preflight builds the daemon binary, then opens the window
```

All interfaces share one application core, so they observe identical behavior,
policies, and errors. The TUI is fully usable offline; no LLM or MCP client is
required. See [docs/tui.md](docs/tui.md), [docs/mcp.md](docs/mcp.md), and
[docs/desktop-gui.md](docs/desktop-gui.md).

Configuration is external and versioned. See
[docs/configuration.md](docs/configuration.md) for the schema and
[docs/adr/003-configuration-system.md](docs/adr/003-configuration-system.md)
for the rationale. A complete example lives at
[examples/config.toml](examples/config.toml).

## Architecture

The core-first architecture keeps the application core independent of Ratatui
and MCP. Both are interface adapters over the same core.

See [docs/architecture.md](docs/architecture.md) for the full design,
[docs/architecture-simplification.md](docs/architecture-simplification.md) for
the latest simplification audit, and
[docs/adr/001-core-architecture.md](docs/adr/001-core-architecture.md) for the
rationale.

```text
tui  ───────┐
            │
mcp  ───────┼──> application ───> execution ──> domain
gui  ───────┤                └──> policy    ──> domain
future cli ─┘

infrastructure (git, process, build, liquibase, health, logging)
    ──> application + domain
```

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
├── liquibase/     # infrastructure
├── health/        # infrastructure
├── logging/       # infrastructure
├── mcp/           # interface adapter
└── tui/           # interface adapter
apps/
└── desktop/       # interface adapter (Tauri 2 GUI; its own Cargo workspace)
src/               # composition root (service-orchestrator binary)
```

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

See [docs/testing.md](docs/testing.md) and
[docs/test-coverage-report.md](docs/test-coverage-report.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
