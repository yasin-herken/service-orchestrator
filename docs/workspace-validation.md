# Workspace Validation

`ValidateWorkspace` answers one question about a configured workspace:
**is this environment actually runnable on this machine?** It inspects the
configuration, the filesystem, the installed runtimes, the dependency graph,
and the declared ports, and returns a structured report. It is available
through every interface — the MCP `validate_workspace` tool,
`OrchestratorClient::validate_workspace`, and the CLI/daemon IPC surface — and
all of them observe identical results because the logic lives once, in the
application core (`docs/adr/017-workspace-validation.md`,
`docs/architecture.md` §18).

## Read-only guarantees

Validation **never mutates anything**:

- no repository is created, fetched, checked out, or cleaned;
- no process is started, stopped, or signalled;
- no port is bound beyond a short-lived loopback *connect* probe that is
  closed immediately;
- the configuration file is only read.

A configured port that is currently occupied by an unrelated process is
reported as a **warning** (`port_currently_occupied`), never as an error, and
the occupying process is left alone — the same "never kill an occupant" rule
the Service Runtime follows (`AGENTS.md` §30). A stopped service is a valid
service; validation concerns structure and availability, not runtime state.

## What is checked

| Area | Checks |
| --- | --- |
| Workspace | The configured root directory exists and is a directory. |
| Services & libraries | Each configured path exists under the root and is a directory; each repository directory contains Git metadata (`.git`), so a plain folder is reported as invalid rather than assumed to be a checkout. |
| Runtimes | The runtime declared by a service (`java`, `node`, …) resolves to an executable on `PATH`; the configured build tool (`mvn`/`mvnw`, `npm`) is likewise resolvable. |
| Dependencies | Every service/library dependency refers to a declared entity; the dependency graph is acyclic (Kahn's algorithm) and every dependency edge is resolvable. |
| Ports | Two services declaring the same port is a **configuration conflict** (`duplicate_configured_port`, error); a declared port that something else is listening on right now is **runtime occupancy** (`port_currently_occupied`, warning). |
| Health | A service whose health check declares a port that no service declares is reported (`undeclared_health_port`). |

The dependency and health checks are **defence in depth**: the configuration
loader already rejects unknown dependencies, cycles, and undeclared health
ports at parse time, so a normally loaded `Config` can never reach the
validator with those defects. They are re-checked so that a `Config` assembled
by any other means (tests, future programmatic construction) is still
diagnosed, and the validator is tested by mutating a loaded `Config` directly.

## The report

`WorkspaceValidationReport` (serde: snake_case) contains:

- `workspace_id`, `name`, `root`;
- `totals` — `services`, `backend_services`, `frontend_services`, `libraries`,
  `repositories_valid`, `repositories_missing`, `repositories_invalid`,
  `invalid_configurations`, `missing_runtimes`, `dependency_errors`,
  `port_conflicts`, `errors`, `warnings`;
- `repositories` — one entry per service/library with its `owner`
  (`{"kind":"service","id":…}` / `{"kind":"library","id":…}`), `path`, and
  `status` (`valid`, `missing`, `not_a_directory`, `missing_git_metadata`);
- `diagnostics` — `{severity, code, scope, message}` with stable codes:
  - `missing_workspace_root`, `missing_service_path`, `missing_library_path`,
    `not_a_directory`, `missing_repository`, `missing_git_metadata`;
  - `unknown_service_dependency`, `unknown_library_dependency`,
    `dependency_cycle`, `undeclared_health_port`;
  - `duplicate_configured_port` (error, configuration),
    `port_currently_occupied` (warning, runtime);
  - `missing_runtime`, `missing_build_tool`, `missing_start_directory`;
- `port_conflicts` — `{port, services}` for configuration-level collisions;
- `occupied_ports` — `{port, services}` for currently occupied ports.

An unknown workspace id fails with the standard structured
`WORKSPACE_NOT_FOUND` error; validation of a valid workspace always succeeds
at the transport level and carries its findings in `diagnostics` — a report
with errors is a *successful validation that found problems*, not a failed
request.

## The probe boundary

The application core may not read the filesystem, `PATH`, or sockets
(`docs/architecture.md` §5). All system facts arrive through the read-only
`WorkspaceProbe` port (`crates/application/src/ports/probe.rs`):

- `exists` — path presence (via `symlink_metadata`, so even a dangling
  symlink counts as present);
- `is_dir` — path presence and directory-ness;
- `contains_entry` — used, for example, to require a `.git` entry in a
  repository directory; no Git command is involved in validation;
- `has_program` — executable resolution: an explicit path (containing `/`) is
  checked directly, otherwise `PATH` is scanned and the execute bit is
  required;
- `is_port_in_use` — a loopback TCP connect with a 150 ms timeout; a timeout
  means the port is treated as free.

`NullWorkspaceProbe` (permissive defaults) keeps the application constructible
without infrastructure, exactly like `NullHealthIndex`. The real
`SystemWorkspaceProbe` lives in the binary's composition root (`src/probe.rs`)
and is wired with `.with_workspace_probe(...)` in `src/main.rs`; no
infrastructure crate leaks into the core.

## Safe execution

Via the MCP server (read-only, `READ` policy):

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": { "name": "validate_workspace", "arguments": { "workspace": "acme" } }
}
```

Via a running daemon's IPC socket (same envelope, `VALIDATE_WORKSPACE`
operation), or programmatically through `OrchestratorClient::validate_workspace`.

Validation is classified `READ` in the policy engine: it is always permitted
and never asks for confirmation, because it cannot change anything.

## Testing

- `crates/application/tests/workspace_validation.rs` — the use case end to end
  against a scripted `WorkspaceProbe`: every diagnostic class, the totals, the
  conflict/occupancy distinction, cycle detection, and two `#[ignore]`d scale
  tests (`validation_latency_at_scale_is_bounded`,
  `read_path_latency_across_documented_scales`,
  `configuration_loading_latency_across_documented_scales`) that double as the
  measurement harness for `docs/performance-report.md`.
- `crates/mcp/src/tests.rs` — the MCP adapter's report shape and the
  `WORKSPACE_NOT_FOUND` path.
- `crates/daemon/tests/daemon.rs` — the IPC round trip, including that
  validation submits no tasks and starts no processes.
- `tests/mcp_stdio.rs` — the real binary end to end: MCP server + daemon +
  `validate_workspace` against a generated fixture, asserting the missing
  library path, `repositories_invalid = 2`, and `repositories_missing = 1`.
