# Service Orchestrator

A macOS-native developer environment orchestration platform written in Rust.

Service Orchestrator prepares and manages local multi-repository, multi-service
development environments: synchronizing repositories, checking out branches,
building shared libraries, running Liquibase migrations, building and starting
applications, inspecting logs, running health checks, and executing
dependency-aware workflows.

It provides two interfaces over one shared application core:

- a **Ratatui terminal UI** for human developers, and
- an **MCP server** for AI agents.

AI is optional. The core works with no LLM, no MCP client, no internet, and no
remote backend.

## Status

This repository currently contains the **architectural foundation**: the Cargo
workspace, crate boundaries, design documentation, the pure domain model, and
the versioned configuration system. Execution, infrastructure, and interfaces
are not implemented yet.

Configuration is external and versioned. See
[docs/configuration.md](docs/configuration.md) for the schema and
[docs/adr/003-configuration-system.md](docs/adr/003-configuration-system.md)
for the rationale. A complete example lives at
[examples/config.toml](examples/config.toml).

## Architecture

The core-first architecture keeps the application core independent of Ratatui
and MCP. Both are interface adapters over the same core.

See [docs/architecture.md](docs/architecture.md) for the full design and
[docs/adr/001-core-architecture.md](docs/adr/001-core-architecture.md) for the
rationale.

```text
tui  ───────┐
            │
mcp  ───────┼──> application ───> execution ──> domain
            │                └──> policy    ──> domain
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
src/               # composition root (service-orchestrator binary)
```

## Development

```bash
cargo fmt --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## License

Apache-2.0. See [LICENSE](LICENSE).
