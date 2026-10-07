# Configuration

Service Orchestrator is configuration-driven. Workspaces, services, libraries,
groups, profiles, runtimes, commands, dependencies, health checks, ports, and
execution settings are all described in a versioned TOML file and are never
hard-coded in source.

This document describes the configuration schema, the loader API, path
resolution, defaults, and validation behavior. The rationale is recorded in
[`docs/adr/003-configuration-system.md`](adr/003-configuration-system.md).

---

## 1. Pipeline

```text
config.toml
    │  read
    ▼
toml::Value  ──  version gate  ──  migrations (future)
    │  deserialize (strict)
    ▼
raw configuration model        crates/config/src/raw.rs
    │  validate + normalize
    ▼
validated configuration        service_orchestrator_config::Config
    │  build
    ▼
domain Workspace               service-orchestrator-domain
```

The three representations are deliberately separate:

1. **Raw model** (`raw.rs`) — a faithful, unvalidated mirror of the TOML.
   Stringly typed so failures can be reported with a precise location.
2. **Normalization + validation** (`validation.rs`, `normalize.rs`) — converts
   strings into domain value objects, resolves references, checks uniqueness,
   and normalizes paths.
3. **Validated configuration** (`model.rs`) — [`Config`], the only type the rest
   of the application sees. It is built from domain `Workspace` aggregates plus
   a few configuration-only values (execution settings, workspace environment,
   Liquibase settings).

TOML never leaks past the `config` crate. No other crate parses configuration
files.

---

## 2. File location and loading

The default location is:

```text
~/.service-orchestrator/config.toml
```

Suggested layout:

```text
~/.service-orchestrator/
├── config.toml
├── workspaces/
├── profiles/
└── logs/
```

Only `config.toml` is read today; the other directories are reserved for
future per-workspace/profile files.

### API

```rust
use std::path::Path;
use service_orchestrator_config::{load_default, load_from_path, load_from_str, ConfigLoader, TomlFileLoader};

// The trait the application depends on:
pub trait ConfigLoader {
    fn load(&self, path: &Path) -> Result<Config, ConfigError>;
}

// Convenience functions:
let config = load_default()?;                       // ~/.service-orchestrator/config.toml
let config = load_from_path(Path::new("c.toml"))?;  // arbitrary file
let config = load_from_str(text, "<memory>")?;      // in-memory (tests, adapters)
```

`load_from_str` names the source in error messages and is intended for
adapters and tests that validate a document without touching the filesystem.

The loading boundary is a trait so future sources — environment variables, CLI
overrides, team-shared configuration, or remote configuration — can implement
the same interface. None are implemented yet.

### `Config`

```rust
pub struct Config {
    pub version: u32,
    pub execution: ExecutionSettings,
    pub workspaces: Vec<ConfiguredWorkspace>,
}

pub struct ConfiguredWorkspace {
    pub workspace: Workspace,                  // domain aggregate
    pub environment: Environment,              // workspace-level variables
    pub liquibase: BTreeMap<ServiceId, LiquibaseConfig>,
}
```

Helper lookups (`config.service(id)`, `config.workspace(id)`, …) search across
all workspaces.

---

## 3. Versioning

Every file should declare its schema version:

```toml
config_version = 1
```

- If `config_version` is omitted it defaults to `1`.
- Any other value is rejected with `ConfigError::UnsupportedVersion`. The schema
  is never guessed.
- A migration hook (`crates/config/src/migration.rs`) upgrades older documents
  step by step before deserialization. Only version 1 exists today, so the hook
  is the identity function; it exists so future versions can be introduced
  without reshaping the loader.

---

## 4. Unknown fields

All raw structures use `#[serde(deny_unknown_fields)]`. A misspelled key is an
error, never silently ignored:

```toml
[execution]
max_paralel_tasks = 4   # error: unknown field `max_paralel_tasks`
```

This is an intentional trade-off: it makes typos loud at the cost of rejecting
configuration written for a newer schema. Unknown *values* such as a future
runtime are still accommodated through explicit `other:<name>` escape hatches,
and new fields are added to the schema (and its version) deliberately.

---

## 5. Schema

### 5.1 Workspaces

```toml
[[workspaces]]
id = "acme"                      # required, unique
name = "Acme Platform"           # optional, defaults to id
root = "~/Projects/acme"         # required, absolute or ~-relative
[workspaces.environment]         # optional
LOGGING_LEVEL = "INFO"
```

A workspace owns its services, libraries, groups, and profiles. Services and
libraries name their workspace with a `workspace` field. Groups and profiles may
name it explicitly or let it be inferred from their members (all members must
resolve to exactly one workspace).

### 5.2 Services

```toml
[[services]]
id = "auth-service"              # required, unique within the workspace
name = "auth-service"            # optional, defaults to id
type = "backend"                 # backend | frontend | library | other:<name>
workspace = "acme"               # required
path = "backend/auth-service"    # optional; required if a repository is given
default_branch = "main"          # optional; fallback for the repository
```

A service typed `library` is rejected with a pointer to `[[libraries]]`;
libraries are a separate entity.

### 5.3 Repository

```toml
[services.repository]
url = "git@github.com:acme/auth-service.git"   # required
default_branch = "main"                         # optional; falls back to the service
provider = "git"                                # optional
```

URLs must not embed credentials (`scheme://user:token@host`); authentication
comes from the environment. The repository's checkout path is the service's
`path`.

### 5.4 Runtime

```toml
[services.runtime]
type = "java"        # java | node | maven | npm | other:<name>
version = "21"       # required for java and node
```

Additional runtimes may be listed with `[[services.runtimes]]`. Runtimes are
requirements only; discovery is out of scope for this task.

### 5.5 Commands

```toml
[services.commands]
install = "./mvnw dependency:resolve"
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"
stop = "kill $(cat target/pid)"
restart = "./mvnw spring-boot:run"
test = "./mvnw test"
liquibase = "./mvnw liquibase:update"

[services.commands.custom]
lint = "npm run lint"
```

Commands may be written as strings or in structured form:

```toml
build = { program = "./mvnw", args = ["clean", "install"], working_dir = "backend/auth-service" }
```

String commands are tokenized without a shell: single/double quotes and
backslash escapes are honored, but there is **no** expansion, globbing, piping,
or command substitution. Programs containing whitespace or unsafe characters are
rejected. Structured commands are preferred for anything non-trivial
(`AGENTS.md` §32).

Commands are only modeled and validated here; this task executes nothing.

### 5.6 Liquibase

```toml
[services.liquibase]
enabled = true                                  # defaults to true when present
command = "./mvnw liquibase:update"
timeout_seconds = 300
[services.liquibase.environment]
LIQUIBASE_COMMAND_CONTEXT = "local"
```

### 5.7 Health checks

```toml
[[services.health_checks]]
type = "http"                                   # process | port | http
url = "http://localhost:8081/actuator/health"   # required for http
expected_status = 200                           # optional for http
timeout_seconds = 10                            # optional
interval_seconds = 5                            # optional

[[services.health_checks]]
type = "port"
port = 8081                                     # required for port
```

A port health check must target a port the service declares.

### 5.8 Ports

```toml
[[services.ports]]
port = 8081                 # required, 1..=65535
protocol = "tcp"            # tcp | udp, defaults to tcp
```

### 5.9 Dependencies

```toml
[services.dependencies]
services = ["user-service"]     # default kind: runtime
libraries = ["common-core"]     # default kind: build
```

Entries may be expanded:

```toml
[services.dependencies]
services = [
  { id = "user-service", kind = "runtime", required = true },
  { id = "metrics-service", kind = "build", required = false },
]
```

Defaults: service dependencies are `runtime`, library dependencies are `build`,
and `required` is `true`. A dependency must resolve to an entity in the same
workspace, must not target itself, and must not be listed twice. Circular
dependency detection is performed by the domain's `Workspace::validate` at
finalization, using the combined service + library graph.

Libraries may only depend on libraries; a library dependency on a service is
rejected with a suggestion to move it under `dependencies.libraries`.

### 5.10 Libraries

```toml
[[libraries]]
id = "common-core"
name = "common-core"
workspace = "acme"
path = "libraries/common-core"
default_branch = "main"
type = "maven"                  # defaults to maven

[libraries.repository]
url = "git@github.com:acme/common-core.git"

[libraries.commands]
build = "./mvnw clean install"

[libraries.artifact]
group_id = "com.acme"
artifact_id = "common-core"
version = "1.8.0-SNAPSHOT"
```

A `maven` library must declare artifact metadata and a build command.

### 5.11 Service groups

```toml
[[groups]]
id = "customer"
name = "Customer Management"
description = "Customer-facing services"
workspace = "acme"              # optional; inferred from services when omitted
services = ["auth-service", "user-service"]
```

Groups reference services; they never duplicate their definitions. A group must
contain at least one service, and every reference must resolve in the same
workspace.

### 5.12 Profiles

```toml
[[profiles]]
id = "full-stack"
name = "Full Stack"
description = "Everything needed for end-to-end work"
workspace = "acme"              # optional; inferred when omitted
services = ["auth-service"]
groups = ["customer"]
```

Profiles reference services and groups. They must reference at least one entity
and must not contain duplicate references.

### 5.13 Execution settings

```toml
[execution]
max_parallel_tasks = 4          # default 4
default_timeout_seconds = 1800  # default 1800
```

Scheduling is not implemented here; these values are validated and carried
forward for the execution engine.

### 5.14 Environment variables

```toml
[services.environment]
SPRING_PROFILES_ACTIVE = "local"
SERVER_PORT = "8081"
API_TOKEN = { value = "placeholder", secret = true }
```

A bare string is a literal, non-secret value. The structured form adds an
explicit `secret` flag; secret values are redacted (`***`) whenever a domain
value is displayed or serialized (`AGENTS.md` §31).

**Do not store real secrets in `config.toml`.** The `secret` flag controls
redaction, not storage. Future versions will support *references* — `from_env`
and a secret-manager reference — so the value is never written to disk. The
domain's `EnvironmentVariable` currently models literal values only, and this
task does not implement external sources.

---

## 6. Path resolution

- **Workspace `root`** — may be absolute (`/Users/dev/projects/acme`) or
  `~`-relative (`~/Projects/acme`). `~` is expanded using `HOME`. The result
  must be absolute. `..` segments are rejected.
- **Service/library `path`** — may be workspace-relative
  (`backend/auth-service`) or absolute/`~`-relative. An absolute path must live
  inside the workspace root; it is rewritten as a workspace-relative path.
  Paths that escape the root, contain `..`, or are empty are rejected.
- **Repository checkout path** — the entity's `path`. A repository without a
  `path` is rejected.

Path resolution is purely lexical. Nothing is created, deleted, or resolved
against the real filesystem, and repositories are not assumed to exist.

---

## 7. Defaults

| Setting | Default | Notes |
| --- | --- | --- |
| `config_version` | `1` | Unknown versions are rejected. |
| workspace `name` | the workspace `id` | |
| service/library `name` | the entity `id` | |
| library `type` | `maven` | |
| port `protocol` | `tcp` | |
| dependency kind (services) | `runtime` | |
| dependency kind (libraries) | `build` | |
| dependency `required` | `true` | |
| liquibase `enabled` | `true` when the block is present | |
| `execution.max_parallel_tasks` | `4` | |
| `execution.default_timeout_seconds` | `1800` | |

No destructive or surprising defaults are provided. In particular, there is
**no default branch**: if neither the repository nor the entity declares one,
configuration is rejected. `master` is never assumed (`AGENTS.md` §13).

---

## 8. Validation

Validation happens in two stages: structural/reference checks in the `config`
crate, then a final `Workspace::validate` in the domain.

### Structural

- Missing required fields → parse error with the field name.
- Unknown fields → parse error naming the field.
- Invalid enum spellings (`type`, runtime, health check, capability, dependency
  kind, protocol) → validation error listing the accepted values.
- `config_version` not an integer, or unsupported → dedicated error.

### References

- Unknown workspace (`services[].workspace`, `libraries[].workspace`,
  `groups[].workspace`, `profiles[].workspace`).
- Unknown service or library in a dependency.
- Unknown service in a group.
- Unknown service or group in a profile.
- Duplicate ids within a workspace (services, libraries, groups, profiles,
  workspaces).

### Paths and ports

- Empty, absolute, `~`, `..` or escaping paths.
- Port `0` or out of range; unknown protocol.
- Duplicate ports within a service.
- A port health check for an undeclared port.

### Dependencies

- Unknown target, self-dependency, duplicate target (via the domain).
- Cycles across services and libraries (via the domain).

### Consistency

- A service whose declared capabilities include `build` must have a build
  command; `run` must have a start command.
- A `frontend` service must declare a runtime; declared `java`/`node` runtimes
  must pin a version.
- A `maven` library must have a build command and artifact metadata.
- Health checks must supply the fields their type requires.

Checks are intentionally practical rather than exhaustive; assumptions that are
not required are not enforced.

### Error shape

Errors are actionable and may be reported several at a time:

```text
Configuration validation failed.

File:
~/.service-orchestrator/config.toml

Location:
services[2].runtime.version

Problem:
node runtime version is missing

Expected:
a non-empty semantic/version-like string

Suggested fix:
Add `version = "20.19.4"` (or the required version) to the runtime.
```

The Rust types are:

- `ConfigError::Read { path, source }`
- `ConfigError::Parse { path, message }`
- `ConfigError::UnsupportedVersion { path, found, supported }`
- `ConfigError::Validation(ValidationErrors)`, where each `ValidationError`
  carries `file`, `location`, `problem`, `expected`, and `suggested_fix`.

---

## 9. Complete example

See [`examples/config.toml`](../examples/config.toml) for a document that
exercises every section: one workspace, a backend and a frontend service, a
library, dependencies, a group, a profile, runtimes, commands, Liquibase, health
checks, ports, and execution settings. The file is loaded verbatim by the
integration test suite.

---

## 10. Future evolution

- **Additional sources.** `ConfigLoader` is the extension point; environment,
  CLI, team-shared, and remote sources would implement it and be composed in the
  composition root.
- **Merging.** No merge strategy is implemented. If overrides are needed, the
  natural place is a loader decorator that overlays a second source before
  normalization; precedence is not defined until then.
- **Secret references.** `from_env` / secret-manager references would resolve at
  load time so no secret is stored on disk.
- **Schema versions.** New fields and shapes are added under a new
  `config_version` with a migration step.
