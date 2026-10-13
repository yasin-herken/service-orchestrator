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
build_system = "maven"           # optional; maven | npm; inferred when omitted
```

A service typed `library` is rejected with a pointer to `[[libraries]]`;
libraries are a separate entity. Whether `build_system` is declared or inferred
is described in §5.6.

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

### 5.6 Build system

```toml
build_system = "maven"           # maven | npm
```

A service or library may declare which build system it uses. The value is
optional; when it is omitted the system is inferred, and when nothing is
recognizable the entity simply has no known build system.

Inference order:

- a `maven` or `npm` **runtime** requirement is the most explicit signal, so it
  is consulted first;
- otherwise the **program** of the build, install, and test commands is matched
  by its basename (`mvn`/`mvnw` → Maven; `npm`/`npx`/`yarn`/`pnpm` → npm). Only
  build-related commands are inspected, so `start = "node index.js"` is not
  mistaken for npm;
- for a **library**, its `type` is consulted first (a `maven` library defaults
  to Maven), then the runtime/command inference above.

An explicit `build_system` is **authoritative**: it is validated and, unlike an
inferred value, an unrecognized spelling is a configuration error rather than a
silent fallback. The build system drives provider selection in the Build Manager
(`docs/build-manager.md`).

Only `maven` and `npm` are accepted today. **Gradle is planned but deliberately
not accepted yet**: it has no provider, and offering it would let a configuration
select a system that cannot build it. Adding it later is a new accepted spelling
plus a new provider, with no change to the rest of the schema.

### 5.7 Liquibase

```toml
[services.liquibase]
enabled = true                                  # defaults to true when present
provider = "liquibase"                          # defaults to "liquibase"
execution_mode = "maven"                        # "maven" (default) or "cli"
timeout_seconds = 300                           # optional; falls back to execution.default_timeout_seconds

[services.liquibase.commands]                   # optional; per-operation commands
validate = "./mvnw liquibase:validate"
status   = "./mvnw liquibase:status"
pending  = "./mvnw liquibase:status"
preview  = "./mvnw liquibase:updateSQL"
update   = "./mvnw liquibase:update"

[services.liquibase.database]                   # optional; display-safe target identity
environment   = "local"                         # local | shared_development | staging | production | <other>
database_type = "postgresql"
name          = "acme_auth"
host          = "localhost"
schema        = "public"
contexts      = ["local"]
labels        = []

[services.liquibase.environment]
LIQUIBASE_COMMAND_CONTEXT = "local"
```

The legacy single `command` key is still accepted and maps to the apply
(`update`) operation; an explicit `commands.update` wins. A per-operation command
is authoritative and used verbatim (program plus typed arguments, never a shell
line); only the *default* command needs build-tool knowledge. In `cli` execution
mode a command must be configured for each operation, otherwise the request fails
with `MIGRATION_COMMAND_MISSING`.

`database.environment` identifies the target for safety: only `local` may migrate
without explicit authorization. Any other value — including an unrecognized name
or an absent one — requires confirmation, and locality is never inferred from a
hostname. Credentials are never stored here; secrets belong in the environment,
referenced by name (`AGENTS.md` §31). See `docs/migration-manager.md`.

### 5.8 Health checks

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

### 5.9 Ports

```toml
[[services.ports]]
port = 8081                 # required, 1..=65535
protocol = "tcp"            # tcp | udp, defaults to tcp
```

### 5.10 Dependencies

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

### 5.11 Libraries

```toml
[[libraries]]
id = "common-core"
name = "common-core"
workspace = "acme"
path = "libraries/common-core"
default_branch = "main"
type = "maven"                  # defaults to maven
build_system = "maven"          # optional; inferred from type when omitted

[libraries.repository]
url = "git@github.com:acme/common-core.git"

[libraries.commands]
build = "./mvnw clean install"

[libraries.artifact]
group_id = "com.acme"
artifact_id = "common-core"
version = "1.8.0-SNAPSHOT"
```

A `maven` library must declare artifact metadata and a build command. A library
may declare `build_system` like a service; when it is omitted the library `type`
is consulted first (§5.6).

### 5.12 Service groups

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

### 5.13 Profiles

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

### 5.14 Execution settings

```toml
[execution]
max_parallel_tasks = 4          # default 4
default_timeout_seconds = 1800  # default 1800
```

Scheduling is not implemented here; these values are validated and carried
forward for the execution engine.

### 5.15 Environment variables

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
| service/library `build_system` | inferred | From `maven`/`npm` runtimes, then build-command programs; a library's `type` is tried first. No inference leaves the entity with no known build system. |
| port `protocol` | `tcp` | |
| dependency kind (services) | `runtime` | |
| dependency kind (libraries) | `build` | |
| dependency `required` | `true` | |
| liquibase `enabled` | `true` when the block is present | |
| liquibase `provider` | `liquibase` | |
| liquibase `execution_mode` | `maven` | `maven` or `cli`. |
| liquibase `database.environment` | `unknown` | Only `local` migrates without explicit authorization; `unknown` is non-local. |
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
- An explicit `build_system` other than `maven`/`npm` → validation error listing
  those two. `gradle` is intentionally not accepted yet (it has no provider).
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

## 9. Management, preview, and persistence

The loader in §2 reads configuration. Editing it is a separate concern, owned by
the `config` crate's management layer (`crates/config/src/management.rs`) and
driven through the [`ConfigManager`] trait. **The shared daemon holds the only
writer**; the TUI, MCP, and the desktop GUI never parse or write a file — they
send a typed change to the daemon and review its preview.

### `ConfigManager`

```rust
pub trait ConfigManager: Send + Sync {
    fn current(&self) -> Arc<Config>;
    fn revision(&self) -> String;
    fn snapshot(&self) -> Result<ConfigurationSnapshot, ConfigError>;
    fn preview(&self, change: &ConfigChange, expected_revision: &str)
        -> Result<ConfigurationPreview, ConfigError>;
    fn commit(&self, change: &ConfigChange, expected_revision: &str)
        -> Result<Config, ConfigError>;
    fn reload(&self) -> Result<Arc<Config>, ConfigError>;
    fn validate(&self) -> Result<(), ConfigError>;
}
```

`FileConfigManager` is the file-backed implementation. Its `commit` validates the
*entire* proposed document in memory, re-checks the on-disk revision, and only
then writes atomically; `current()` keeps the validated configuration every
manager reads. `validate` reads and validates **without** replacing the serving
configuration, so an inspection is genuinely read-only.

### The change vocabulary

A change targets exactly one entity and is tagged by kind, so it needs no second
schema on the wire:

```json
{ "kind": "service",
  "change": { "action": "create", "entity": { "id": "billing-api", "type": "backend", "workspace": "acme" } } }
```

```json
{ "kind": "group", "change": { "action": "remove", "id": "legacy" } }
```

`EntityChange<T>` is `create { entity }`, `update { id, entity }`, or
`remove { id }`; `ConfigChange` is `workspace | service | library | profile |
group`. A whole document is never uploaded, so an edit cannot clobber unrelated
sections it did not read.

### Preview

`preview` computes, without writing:

- a **revision** and **proposed_revision** (FNV-1a, hex);
- a structural **diff** (`added`, `updated` with field-level before/after,
  `removed`);
- **validation** issues over the whole proposed document;
- **affected references** — the entities that still point at a removal target;
- the **running impact**, filled in by the application layer, which knows the
  live process state. Configuration changes never restart or stop a running
  service implicitly (`tasks/023.md` §9).

Secret values (`secret = true`) are redacted (`***`) in the diff, and are written
back **verbatim** when they are unchanged, so editing an entity never leaks or
loses a secret.

### Persistence guarantees

- **Atomic.** The proposed file is rendered with `toml_edit` (only the changed
  array of tables is rewritten) and written to a sibling `.<name>.tmp-<pid>`,
  fsynced, then `rename`d over the target. A reader never sees a partial file.
- **Revision-checked (TOCTOU-safe).** The revision is verified again immediately
  before the rename; a mismatch fails with `ConfigError::Conflict` and writes
  nothing. Two editors can never silently overwrite each other.
- **Backed up.** The previous file is copied to `<config>.toml.bak` on every
  successful write.
- **Permissions preserved.** An existing file keeps its mode; a newly created
  file is `0600` on Unix.
- **No partial change.** If the proposal fails validation, the existing file is
  left exactly as it was.

### Repository inspection and technology detection

Adding a service can start from a local directory. Inspection is strictly
**read-only** and never runs a build, a script, or a Git command. It reports the
directory's existence, whether it is a Git repository (with branch and remote),
the project files seen, detected **build systems** (Maven, npm, and Gradle),
**runtimes** (Java/Node version requirements), and integrations (Spring Boot,
React). A detected build system whose execution provider is not installed is
reported as `provider_not_installed` — for example **Gradle is detected but its
provider is not installed**, so it is never presented as operational. The
detection produces a *suggested* service configuration (service type, build
system, runtimes, build and start commands as structured `program`+`args`, never
a shell line) that the wizard fills in; the developer edits it and the daemon
validates it like any other change.

---

## 10. Complete example

See [`examples/config.toml`](../examples/config.toml) for a document that
exercises every section: one workspace, a backend and a frontend service, a
library, dependencies, a group, a profile, runtimes, commands, Liquibase, health
checks, ports, and execution settings. The file is loaded verbatim by the
integration test suite.

---

## 11. Future evolution

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
