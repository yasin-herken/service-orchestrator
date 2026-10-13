# Workspaces

A **workspace** is a directory on disk plus the services, libraries, groups, and
profiles that live inside it. It is the unit of configuration and the unit that
`validate_workspace` inspects. Service Orchestrator supports several workspaces
in one configuration file, and one workspace is "active" at a time in each
interface.

The schema is defined in [`configuration.md`](configuration.md). This guide is
about using workspaces, not about every field.

## 1. Workspace concepts

```text
Workspace (root: ~/Projects/acme)
├── Services      buildable/runnable units, each with an optional repository
├── Libraries     shared buildable artifacts consumed by services
├── Groups        named sets of services
└── Profiles      commonly used developer environments (services + groups)
```

- The workspace **`root`** may be absolute (`/Users/me/Projects/acme`) or
  `~`-relative (`~/Projects/acme`). `~` expands using `HOME`.
- A service or library names its workspace with `workspace = "<id>"`. Its `path`
  may be relative to the root or absolute inside the root.
- Groups and profiles may set `workspace` explicitly or let it be inferred from
  their members; all members must resolve to exactly one workspace.

## 2. Defining a workspace

```toml
config_version = 1

[[workspaces]]
id = "acme"
name = "Acme Platform"
root = "~/Projects/acme"

[workspaces.environment]
LOGGING_LEVEL = "INFO"
```

- `id` is required and unique across workspaces.
- `name` is optional and defaults to the id.
- `root` is required; it must resolve to an absolute path without `..`.
- `workspace.environment` holds variables applied to the workspace's services.

A workspace with no services is valid configuration, but most operations have
nothing to act on.

## 3. Path resolution

- **Workspace root** — absolute or `~`-relative; expanded with `HOME`; must be
  absolute; `..` is rejected.
- **Service/library path** — workspace-relative (`backend/auth-service`) or
  absolute/`~`-relative. An absolute path must live inside the root and is
  rewritten to a relative one. Paths that escape the root, contain `..`, or are
  empty are rejected. A path must point *inside* the root, not at it.
- **Repository checkout path** — the entity's `path`. A repository without a
  `path` is rejected.

Path resolution is purely lexical: nothing is created or deleted, and
repositories are not assumed to exist. Whether they exist is what
`validate_workspace` checks.

## 4. Multiple workspaces

Every workspace is independent:

- Service and library ids must be unique **within** a workspace. The GUI treats
  ids as globally unique for convenience in a single command, and the TUI/GUI
  let you switch the active workspace.
- The daemon's socket path is derived from the *configuration directory*, not the
  workspace, so all workspaces in one `config.toml` share one daemon. Running a
  second configuration file runs a second daemon
  ([`local-daemon.md`](local-daemon.md)).

## 5. References

- A service or library declares the workspace it belongs to.
- A dependency must resolve to a declared entity **in the same workspace**.
- A group must contain at least one service, and each must exist in the
  workspace.
- A profile must reference at least one service or group, with no duplicates.

The loader rejects unknown dependencies, cycles, and undeclared health ports.
`validate_workspace` re-checks them as defence in depth.

## 6. Validating a workspace

`validate_workspace` is a **read-only** report over the configuration, the
filesystem, the installed runtimes, the dependency graph, and the declared
ports. It never fetches, checks out, builds, starts, or rewrites anything.

- **TUI** — the report is not a dedicated screen in the TUI; use the GUI, MCP,
  or the CLI surface.
- **GUI** — the workspace selector's validation view (Configuration screen's
  **Validate** action validates the file; workspace validation is the
  runtime/availability report).
- **MCP** — `validate_workspace { "workspaceId": "acme" }`.

What it checks:

| Area | Checks |
| --- | --- |
| Workspace | The root exists and is a directory. |
| Services & libraries | Each configured path exists and is a directory; a repository directory must contain Git metadata (`.git`). |
| Runtimes | The declared runtime resolves on `PATH`; the configured build tool (`mvn`/`mvnw`, `npm`) is resolvable. |
| Dependencies | Every dependency refers to a declared entity; the graph is acyclic. |
| Ports | Two services declaring the same port is an error; a declared port currently in use is a **warning** only (the occupant is never signalled). |
| Health | A health check declaring a port no service declares is reported. |

The report's stable diagnostic codes include `missing_workspace_root`,
`missing_service_path`, `missing_library_path`, `not_a_directory`,
`missing_repository`, `missing_git_metadata`, `unknown_service_dependency`,
`unknown_library_dependency`, `dependency_cycle`, `undeclared_health_port`,
`duplicate_configured_port`, `port_currently_occupied`, `missing_runtime`,
`missing_build_tool`, and `missing_start_directory`.

A report with errors is still a *successful validation of a valid workspace* that
found problems — the request itself does not fail. An unknown workspace id is a
structured `WORKSPACE_NOT_FOUND`.

See [`workspace-validation.md`](workspace-validation.md) for the full model.

## 7. Common validation errors

| Error | Meaning | Fix |
| --- | --- | --- |
| `unknown workspace '<id>'` | A service/library/group/profile names a workspace that is not declared. | Add the workspace or correct the id. |
| `duplicate workspace id` | Two `[[workspaces]]` share an id. | Make ids unique. |
| `path must point inside the workspace, not at it` | A `path` equals the root. | Point at a subdirectory. |
| `'<p>' is outside the workspace root` | An absolute path escapes the root. | Use a path inside the root. |
| `unknown service '<id>'` (dependency/group) | A reference does not resolve. | Fix the id or declare the entity. |
| `dependency_cycle` / `ServiceDependencyCycle` | Services/libraries form a cycle. | Break the cycle. |
| `missing_git_metadata` | A configured repository path exists but is not a Git checkout. | Clone it or fix `path`. |
| `missing_runtime` / `missing_build_tool` | `java`/`node`/`mvn`/`npm` is not on `PATH`. | Install it or fix the version/runtime. |

## 8. Limitations

- Configuration editing is available in the **desktop GUI** (through the
  daemon). The TUI displays configuration read-only, and MCP has no
  configuration-editing tools. You can always edit `config.toml` by hand and
  reload it.
- The `workspaces/` and `profiles/` subdirectories under
  `~/.service-orchestrator/` are reserved for future per-workspace files and are
  not read today.

See [`known-limitations.md`](known-limitations.md).
