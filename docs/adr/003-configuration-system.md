# ADR 003: Versioned, configuration-driven settings

- **Status:** Accepted
- **Date:** 2026-10-07
- **Deciders:** Service Orchestrator maintainers

## Context

Service Orchestrator prepares and manages local development environments for
many repositories and microservices. The set of workspaces, services, libraries,
groups, profiles, runtimes, commands, dependencies, health checks, ports, and
execution settings differs for every developer and every project, and it changes
far more often than the code that orchestrates it.

Two failure modes are easy to fall into:

1. **Hard-coded settings.** Embedding service names, repository URLs, Maven/npm
   commands, or the default branch in Rust source means every new service is a
   code change and a rebuild, and the tool can never be reused by another team.
2. **A leaky parse.** Reading TOML directly wherever a value is needed spreads
   TOML syntax, file paths, and validation across the whole application. The
   domain and application layers then depend on a file format, and the same
   document is interpreted inconsistently by the TUI, the MCP server, and any
   future interface.

There is also a correctness and safety dimension. Misconfiguration — a typo in
an execution limit, a port out of range, a dependency on a service that does not
exist, an unknown schema version — must fail loudly and explain how to fix it,
not be silently ignored or interpreted as "empty".

The architecture (ADR 001) places infrastructure such as configuration behind an
application-facing boundary and keeps the domain independent of it. ADR 002
established that services are configuration data, not code. This ADR records how
configuration is loaded, validated, and handed to the core.

## Decision

Configuration is **external, versioned, validated before use, and contained to a
single crate**. The `service-orchestrator-config` crate owns parsing and
validation; every other layer receives one validated value, `Config`.

### One direction of dependency, one entry point

```text
config.toml
    │  read
    ▼
toml::Value ── version gate ── migrations (future)
    │  deserialize (strict)
    ▼
raw model             crates/config/src/raw.rs
    │  validate + normalize
    ▼
Config                service_orchestrator_config::Config
    │  build
    ▼
domain Workspace      service-orchestrator-domain
```

The `config` crate depends only on the domain and on `serde`/`toml`/
`thiserror`. TOML never leaks past it: no other crate reads a configuration
file or sees a `toml::Value`. The application depends on the `ConfigLoader`
trait, not on `toml`:

```rust
pub trait ConfigLoader {
    fn load(&self, path: &Path) -> Result<Config, ConfigError>;
}
```

`TomlFileLoader` is the only implementation today, plus `load_default`,
`load_from_path`, and `load_from_str` conveniences.

### Why TOML

TOML is used for the human-authored configuration file.

- It is a **readable, comment-friendly** format for the kind of document a
  developer keeps in `~/.service-orchestrator/config.toml`.
- It has **native tables and arrays of tables**, which map cleanly onto
  workspaces, services, commands, dependencies, and health checks without the
  nesting and quoting pain of flat formats.
- It has **typed scalars** (integers, booleans, strings) and a mature,
  well-specified Rust crate (`toml`, built on `serde`), so no custom parser is
  needed.
- It is **strict-friendly**: combined with `serde(deny_unknown_fields)` it turns
  typos into errors.
- It is already the ecosystem default for Rust tooling (`Cargo.toml`), so it is
  familiar to the target audience.

The format is an implementation detail of this crate. If another source
(JSON from a tool, YAML from a team repo) is ever needed, it becomes another
`ConfigLoader`, not a change to the domain.

### Why configuration is versioned

Every document declares `config_version`:

```toml
config_version = 1
```

- The loader **gates on the version before structural deserialization**. An
  unsupported or malformed version is rejected with a dedicated error; a schema
  is never guessed.
- A **migration hook** (`migration.rs`) upgrades older documents step by step to
  the supported version. Only version 1 exists today, so the hook is the
  identity function, but its shape (`upgrade_to_n`) is in place so future
  versions do not require reshaping the loader.
- Omitting the field defaults to version 1, so the simplest files still work,
  while an explicit version lets the tool detect files written for a newer
  build rather than misreading them.

Versioning is what lets the schema evolve — rename a field, split a section,
change a default — without silently corrupting an existing developer's setup.

### Why raw config and validated config are separated

There are three distinct representations, each with a job:

1. **Raw model** (`raw.rs`) — a faithful, unvalidated mirror of the TOML.
   Deliberately stringly typed (`String`, `Option`, `Vec`) so that every value
   is visible during validation and can be reported with a precise location.
   Every struct uses `deny_unknown_fields`.
2. **Validation + normalization** (`validation.rs`, `normalize.rs`) — converts
   strings into domain value objects, resolves cross-references, checks
   uniqueness and consistency, and normalizes paths. Errors are accumulated so
   several problems are reported in one pass.
3. **Validated configuration** (`model.rs`) — `Config`, built from domain
   `Workspace` aggregates plus the few configuration-only values the domain does
   not model yet (global `ExecutionSettings`, workspace `Environment`, and
   per-service `LiquibaseConfig`).

Separating the raw model from the validated model is what makes actionable
errors possible. If the TOML were deserialized straight into domain types, the
first invalid value would abort with an opaque serde message and no location.
Keeping a stringly-typed stage lets the crate say *which* field, *why*, what was
expected, and how to fix it — for example:

```text
Location:
services[2].runtime.version

Problem:
node runtime version is missing

Expected:
a non-empty semantic/version-like string

Suggested fix:
Add `version = "20.19.4"` (or the required version) to the runtime.
```

The separation also keeps the domain pure (ADR 002): the domain never learns
about TOML, `deny_unknown_fields`, or `other:<name>` spellings.

### Strict handling of unknown fields

All raw structures use `#[serde(deny_unknown_fields)]`. A misspelled key such as
`max_paralel_tasks` is an error, never silently ignored. The trade-off is that a
file written for a *newer* schema is rejected; that is intentional — new fields
arrive under a new `config_version` with a migration. Genuinely open-ended
concepts (a future runtime or service type) use explicit `other:<name>` escape
hatches rather than an unchecked map.

### Practical, layered validation

Validation is deliberately practical rather than exhaustive:

- **Structural:** missing fields, unknown fields, invalid enum spellings,
  non-integer/unsupported version.
- **References:** unknown workspace, service, library, group, profile;
  duplicate ids within a workspace.
- **Paths and ports:** empty/escaping/`..` paths, out-of-range ports, unknown
  protocol, duplicate ports, a port health check for an undeclared port.
- **Dependencies:** unknown target, self-dependency, duplicate target, and
  cycles — the last two via the domain's `Workspace::validate`.
- **Consistency:** capabilities imply commands (`build` → build command,
  `run` → start command); a `frontend` service must declare a runtime; a
  `maven` library must declare a build command and artifact metadata; health
  checks must supply the fields their type requires.

`Workspace::validate` is called during finalization, so the config crate reuses
the domain's uniqueness, reference, and acyclicity rules instead of duplicating
them.

### No filesystem mutation

Path handling is purely lexical. Workspace roots expand a leading `~` using
`HOME`; service and library paths are normalized relative to the workspace root;
an absolute path inside the root is rewritten as workspace-relative. `..`,
empty paths, and paths that escape the root are rejected. Nothing is created,
deleted, or resolved against the real filesystem, and repositories are not
assumed to exist.

### No merge strategy

No configuration merging is implemented. There is one file and one loader. If
overrides become necessary, the natural extension point is a loader decorator
that overlays another source before normalization; precedence is intentionally
left undefined until a real requirement exists.

## Alternatives considered

1. **Hard-coded configuration (or per-service Rust files).**
   Rejected: every new service becomes a rebuild, the tool is not reusable, and
   the domain would absorb project-specific data — contrary to AGENTS.md §12 and
   ADR 002.

2. **JSON or YAML instead of TOML.**
   JSON lacks comments and is unpleasant to hand-edit; YAML's implicit typing and
   indentation pitfalls work against strict, predictable validation. TOML is
   comment-friendly, typed, and already familiar to Rust developers. The format
   is contained to this crate, so this is reversible.

3. **Deserialize TOML directly into domain types.**
   Rejected: serde errors are opaque and unlocated, there is nowhere to resolve
   references or accumulate multiple problems, and the domain would become
   format-aware. The raw/validated split is what makes the error model possible.

4. **A single universal map / `Value` bag.**
   Rejected: it trades type safety for speculative flexibility and defeats the
   strict, documented schema.

5. **Lenient unknown-field handling.**
   Rejected: silently ignoring `max_paralel_tasks` hides user error. `deny_unknown_fields`
   plus versioned schema changes is the safer default.

6. **An unversioned schema.**
   Rejected: there would be no way to evolve the format without breaking
   existing files or guessing at their shape.

7. **Build a general merge/override engine now.**
   Rejected as speculative: no source besides the single TOML file exists. The
   loader trait is the seam where such a source would be added, and the
   documented merge point is a decorator.

8. **A `config` module inside an existing crate.**
   Rejected: the crate boundary is what keeps `toml` out of the domain and makes
   "who parses configuration" a compile-time, reviewable fact.

## Reasoning

- **Configuration-first.** Services, libraries, groups, and profiles are data.
  Adding one is editing a file, not shipping code.
- **One source of truth.** Every interface loads the same `Config` produced by
  the same validation, so the TUI and MCP cannot interpret a document
  differently.
- **Actionable failures.** A wrong field names itself and suggests a fix, which
  matters most for the developer editing the file.
- **Domain purity.** The domain stays free of TOML and of infrastructure
  concerns (ADR 001, ADR 002).
- **Safe evolution.** A version gate plus a migration seam lets the schema change
  without corrupting existing setups.
- **Honest defaults.** Defaults are only provided where they are non-destructive;
  notably there is **no default branch**, so `master` is never silently assumed
  (AGENTS.md §13).

## Consequences

### Positive

- Adding or reconfiguring a service needs no code change or rebuild.
- A single validated object crosses into the application; TOML is fully
  contained.
- Typos and reference errors fail loudly, with location and suggested fix.
- The version gate and migration hook make schema evolution tractable.
- No filesystem mutation, so loading configuration is side-effect free and easy
  to test.

### Negative / trade-offs

- Three representations (raw, validation, validated) and more types than a
  direct deserialization.
- `deny_unknown_fields` rejects files written for a newer schema; this is
  accepted because new fields are versioned and migrated.
- Configuration-only values (execution settings, workspace environment,
  Liquibase settings) currently live in the `config` crate rather than the
  domain; if they become first-class they should move to the domain with an ADR.
- Environment variables model literal values only in this task. Storing real
  secrets on disk is discouraged; secret *references* are future work.
- No merge strategy exists; overrides will require a deliberately designed
  precedence rule rather than a general engine.

## Follow-up

- The `application` layer consumes `Config`/`ConfigLoader` and wires the loader
  in the composition root.
- The `execution` crate consumes `ExecutionSettings`.
- The Liquibase, health, and process adapters consume the corresponding
  configuration without parsing TOML themselves.
- Add secret *references* (`from_env`, secret manager) so values need not be
  stored on disk.
- When a second schema version is needed, add `upgrade_to_n` migrations and a
  new `SUPPORTED_CONFIG_VERSION`.
