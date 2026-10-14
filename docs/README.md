# Service Orchestrator Documentation

This is the documentation index. Start with **Getting Started**, then read the
guide for the task you are doing. The **Architecture** and **Development**
sections are for contributors; the **Security** section describes the guarantees
and the limits of the local trust model.

If you are unsure where to look, the root [`README.md`](../README.md) gives the
project overview and a quick start.

## Getting started

| Document | What it covers |
| --- | --- |
| [`getting-started.md`](getting-started.md) | Install, configure a first workspace, start the daemon, open an interface, run a safe operation. |
| [`onboarding-checklist.md`](onboarding-checklist.md) | A copyable checklist for a new developer's first session. |
| [`configuration.md`](configuration.md) | The complete `config.toml` schema, defaults, and validation, plus the configuration-management layer. |
| [`workspaces.md`](workspaces.md) | Workspaces, paths, multiple workspaces, references, and validation errors. |

## User guides

Day-to-day operation of services and repositories. Each guide follows the same
shape: what the feature is, how to do it in the GUI, TUI, and MCP, and what can
go wrong.

| Document | What it covers |
| --- | --- |
| [`service-management.md`](service-management.md) | Service lifecycle: start, stop, restart, bulk operations, ports, process ownership. |
| [`git-operations.md`](git-operations.md) | Status, fetch, checkout, pull, sync, dirty working trees, destructive operations. |
| [`builds.md`](builds.md) | Maven and npm builds, wrappers, node/java versions, build logs, failures. |
| [`libraries.md`](libraries.md) | Shared libraries, artifact coordinates, local install, verification, version alignment. |
| [`migrations.md`](migrations.md) | Liquibase integration, status, preview, apply, target safety, failure handling. |
| [`health.md`](health.md) | Process, TCP, and HTTP checks, readiness, health status, common failures. |
| [`logs-and-diagnostics.md`](logs-and-diagnostics.md) | Service, task, and workflow logs; filtering, paging, retention, redaction. |
| [`workflows.md`](workflows.md) | Prepare Environment, profiles, groups, dependency resolution, planning, cancellation. |
| [`desktop-gui.md`](desktop-gui.md) | The Tauri 2 desktop GUI: screens, navigation, and operations. |
| [`tui.md`](tui.md) | The Ratatui terminal interface: screens, keys, and operations. |
| [`mcp.md`](mcp.md) | The MCP server: tools, resources, prompts, client setup, and security. |
| [`local-daemon.md`](local-daemon.md) | The shared local daemon every interface talks to, and the IPC protocol. |
| [`macos-distribution.md`](macos-distribution.md) | Building, verifying, installing, and distributing the macOS app and DMG. |
| [`error-reference.md`](error-reference.md) | Stable error codes with meaning, cause, and recommended action. |
| [`troubleshooting.md`](troubleshooting.md) | Diagnosis steps for the most common failures. |
| [`known-limitations.md`](known-limitations.md) | Verified limitations of the current implementation. |

## Architecture

How the platform is built. Read [`architecture.md`](architecture.md) first; the
rest are focused references.

| Document | What it covers |
| --- | --- |
| [`architecture.md`](architecture.md) | Layers, crate responsibilities, dependency rules, and the core principle. |
| [`domain-model.md`](domain-model.md) | Entities, invariants, and the domain vocabulary. |
| [`application-core.md`](application-core.md) | Use cases, orchestration, and application ports. |
| [`task-engine.md`](task-engine.md) | Task states, scheduling, concurrency, cancellation, retry, timeouts. |
| [`process-manager.md`](process-manager.md) | The single owner of OS processes. |
| [`git-manager.md`](git-manager.md) | The Git adapter and its safety rules. |
| [`build-manager.md`](build-manager.md) | The `BuildProvider` architecture (Maven, npm). |
| [`library-manager.md`](library-manager.md) | Shared-library orchestration and artifact inspection. |
| [`service-runtime.md`](service-runtime.md) | The `ServiceRuntime` architecture (Java, Node). |
| [`migration-manager.md`](migration-manager.md) | The `MigrationProvider` architecture (Liquibase) and target safety. |
| [`health-manager.md`](health-manager.md) | Health providers, aggregation, readiness, and polling. |
| [`observability.md`](observability.md) | The unified log store: ingestion, filtering, paging, retention, streaming. |
| [`environment-workflows.md`](environment-workflows.md) | The environment preparation workflow in depth. |
| [`workspace-validation.md`](workspace-validation.md) | The read-only workspace validator. |
| [`architecture-simplification.md`](architecture-simplification.md) | The latest architecture-simplification audit. |
| [`adr/`](adr/) | Architecture decision records (ADR 001–020). |

## Development

| Document | What it covers |
| --- | --- |
| [`development.md`](development.md) | Repository structure, crates, how to run each interface, debugging, and how to add an operation. |
| [`testing.md`](testing.md) | Test layers, categories, test scripts, fixtures, and prerequisites. |
| [`test-coverage-report.md`](test-coverage-report.md) | What is covered and what is not. |
| [`performance-report.md`](performance-report.md) | Scale/measurement harnesses and results. |
| [`reliability-report.md`](reliability-report.md) | Reliability findings locked down by regression tests. |
| [`packaging-report.md`](packaging-report.md) | The macOS packaging reference build: artifacts, architecture, signing status, and limits. |
| [`acceptance-test-matrix.md`](acceptance-test-matrix.md) | The Task 030 end-to-end acceptance scenarios, expected/actual results, and evidence. |
| [`final-acceptance-report.md`](final-acceptance-report.md) | The Task 030 acceptance summary, defects, findings, and packaging-readiness decision. |
| [`release-blockers.md`](release-blockers.md) | Unresolved release-blocking issues (none at Task 030 close). |

## Security

| Document | What it covers |
| --- | --- |
| [`security.md`](security.md) | Threat model, trust boundaries, policy enforcement, and honest limitations. |
| [`security-audit.md`](security-audit.md) | The audit that produced the security document. |

## Task records

Per-task implementation notes live under `docs/` as `task-021-progress.md`
(and its siblings). They are historical records, not user documentation.
