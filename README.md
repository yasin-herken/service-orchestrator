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

This repository currently contains the **architectural foundation only**:
the Cargo workspace, crate boundaries, and design documentation. Business logic
and interfaces are not implemented yet.

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
