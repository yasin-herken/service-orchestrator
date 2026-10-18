# Service Templates

Service templates are reusable defaults for services and libraries created by
the **Bulk Service Import** wizard. A template is not a second service model and
not a script: it is a set of validated defaults that is resolved for one concrete
repository and then written as an ordinary

```toml
[[services]]
```

or

```toml
[[libraries]]
```

entry. The daemon validates the resolved entry exactly like a hand-written one
(`docs/configuration.md`).

Templates live in the same `config.toml` as everything else:

```toml
[[templates]]
id = "spring-boot-backend"
name = "Spring Boot Backend"
description = "Spring Boot 3 service on Java 21, built with the Maven wrapper."
category = "spring_boot_backend"
type = "backend"
default_branch = "main"
build_system = "maven"
build_operation = "build"

[templates.runtime]
type = "java"
version = "21"

[templates.commands]
install = "./mvnw dependency:resolve"
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[templates.liquibase]
enabled = true
execution_mode = "maven"
preset = "status"

[[templates.health_checks]]
type = "http"
url = "http://localhost:${port}/actuator/health"
timeout_seconds = 10
```

There is no separate template store, no user-level preference file, and no
second configuration engine. The template list is part of the configuration
snapshot, is written by the same `ConfigManager`, and is broadcast to the TUI,
MCP, and the desktop GUI like any other configuration change.

## 1. What a template may contain

| Field | Meaning |
| --- | --- |
| `id` | Stable identifier, unique among templates. |
| `name` | Display name; defaults to `id`. |
| `description` | Optional free text. |
| `category` | `spring_boot_backend`, `react_frontend`, `shared_maven_library`, or a project-specific label. Drives automatic matching. |
| `type` | `backend`, `frontend`, `library`, or `other:<name>`. `library` creates a `[[libraries]]` entry. |
| `default_branch` | Branch checked out for a matching repository. No branch is ever assumed. |
| `build_system` | `maven` or `npm`. Gradle is detected during discovery but has no execution provider, so it is not a template build system. |
| `build_operation` | Operation used by the build phase: `clean`, `build`, `test`, `install_dependencies`, or `publish_local_artifact`. |
| `runtime` / `runtimes` | One or more runtime requirements (`java`, `node`, `maven`, `npm`). Declared runtimes win over detected ones. |
| `commands` | `install`, `build`, `start`, `test` as structured commands. Command **values** may use variables; command **programs** may not. |
| `liquibase` | Whether migration runs, the provider/mode, and the default named preset. Never a database target or credentials. |
| `health_checks` | `process` or `http` checks. A URL may use `${port}`. |
| `timeout_seconds` | Operation timeout. |
| `environment` | Environment variables. The explicit `secret` flag still applies and secret values stay redacted. |
| `capabilities` | Extra capabilities beyond those implied by `type`. |

A template deliberately has **no**:

- ports (they are assigned during import or set by an override);
- repository URL/clone URL (it comes from the discovered repository or an
  explicit override);
- database target, database environment, or credentials;
- branch expression, shell line, or arbitrary command.

## 2. Template variables

Every string value may contain a closed set of placeholders. This is a single
textual substitution pass, not an expression language:

| Placeholder | Resolves to |
| --- | --- |
| `${serviceName}` | The id of the entity being created. |
| `${repositoryName}` | The repository directory name. |
| `${repositoryPath}` | The repository's workspace-relative path. |
| `${workspaceRoot}` | The absolute workspace root. |
| `${relativePath}` | The workspace-relative path being resolved (equal to the repository path for repository-level values). |
| `${port}` | The port assigned during the import. |

Rules:

- Matching is exact and case-sensitive. `servicename` is not a variable.
- Unknown, empty (`${}`), or unterminated (`${serviceName`) placeholders are
  hard errors. They are never left in the configuration and never silently
  skipped.
- A bare `$` that is not followed by `{` is ordinary text.
- `${port}` fails to resolve when no port has been assigned. A template that
  uses it therefore requires either a port override or a usable port base.
- A resolved value still passes ordinary configuration validation, including
  path validation. A variable cannot escape the workspace root or bypass any
  check.

Variables are allowed in:

- service/library names, command values and arguments, environment values,
  health-check URLs, and similar text values.

Variables are **not** allowed in:

- branches (`default_branch`, an operation branch, or a Git override);
- command *programs* (the executable name);
- ids that must already be stable.

Validation reports the location as `templates[i].field`, so the GUI can show the
problem on the exact template.

## 3. How a template is resolved

The precedence is deterministic and deliberately short:

```text
per-repository overrides
        ↓
shared bulk overrides
        ↓
template defaults
        ↓
detected repository metadata
```

- The **per-repository override** always wins.
- The **shared override** fills a value for every selected repository.
- The **template default** fills what the overrides leave empty.
- **Detection** fills what remains: the clone URL, a plausible build system,
  runtimes, the Maven artifact coordinate for a library, and detected build or
  start commands when the template has none.
- A value that still cannot be determined is a plan error (for example
  `BRANCH_REQUIRED`, `REPOSITORY_URL_REQUIRED`, or `ARTIFACT_REQUIRED`), not a
  guess.

A template is only a source of *defaults*. If a template declares a build
command and the developer writes another one for one repository, the explicit
value wins and the template is not re-applied later.

## 4. Automatic matching is a suggestion

Discovery reports detected technologies with a confidence
(`uncertain`/`low`/`medium`/`high`). Matching maps the strongest signal to a
template category:

| Detection | Suggested category |
| --- | --- |
| Spring Boot integration | `spring_boot_backend` |
| React integration | `react_frontend` |
| Maven, with neither Spring Boot nor React | `shared_maven_library` |
| Equally strong conflicting signals | none — reported as ambiguous |
| Nothing recognized | none |

Gradle is detected and reported but ignored for matching, because it has no
execution provider.

The result is only a **suggestion**:

- Exactly one template in the suggested category → it is proposed and shown as
  `matched`.
- More than one template in that category → planning fails with
  `TEMPLATE_AMBIGUOUS`; the developer must choose one.
- No template in that category → the import still proceeds with a warning and
  the detected type/commands; the developer may assign a template explicitly.
- A **custom** category is never matched automatically; it is only usable by
  explicit assignment.

The wizard always displays the resolved entity before anything is written. It
never silently classifies a repository, and a repository is never imported just
because it was detected.

## 5. Template management GUI

The **Templates** screen lists every configured template with its category,
type, branch, build system, and whether it is built in. It supports:

- **Create** and **Edit** through the shared configuration review. The daemon
  previews the whole proposed file and commits atomically with a revision check;
  the GUI never edits TOML.
- **Duplicate**, which opens the editor with a copy and a new id.
- **Delete**, as a destructive change that still goes through the same review.
- **Validate**, which runs the daemon's full validation and shows only the
  issues located under that template.
- **Preview resolved configuration**, which asks the real daemon planner to
  resolve the template for a workspace, relative path, optional service name,
  port base, and repository URL. The preview runs
  `plan_bulk_import` — the same code path as the wizard — so it cannot drift from
  what an import would produce. The preview never writes and never executes
  anything.

Template edits are ordinary `template` configuration changes. They are not
special-cased in the daemon, and they do not require a restart.

## 6. Validation and safety

A template is validated when the configuration is loaded and whenever it is
previewed or committed:

- `id` must be a valid entity id and unique.
- `category` is parsed; an unknown label becomes a custom category (never
  auto-matched).
- `type`, `build_system`, `build_operation`, runtimes, health checks, commands,
  and Liquibase settings use the same domain parsers as services.
- Every placeholder is validated.
- Branches and command programs may not contain placeholders.
- A template's default migration preset must exist.
- A template cannot carry credentials, ports, or a repository URL; unknown
  fields are rejected by the strict parser.

A template is **not authorization**. Importing with a template still requires a
`SafeWrite` permission, or `Destructive` when an existing entity is replaced.
The daemon re-plans and re-validates on commit; a stale reviewed plan is
reported as a configuration conflict instead of being applied.

## 7. Example: explicit versus matched

Given the template above, importing `backend/orders-api` with no override
resolves to a service like:

```toml
[[services]]
id = "orders-api"
name = "orders-api"
type = "backend"
workspace = "acme"
path = "backend/orders-api"
default_branch = "main"
build_system = "maven"
build_operation = "build"

[services.repository]
url = "https://example.com/orders-api.git"

[services.runtime]
type = "java"
version = "21"

[services.commands]
build = "./mvnw clean install"
start = "./mvnw spring-boot:run"

[[services.ports]]
port = 8081

[[services.health_checks]]
type = "http"
url = "http://localhost:8081/actuator/health"
```

The port, URL, name, and detected fallbacks came from the import context, not
from the template. See [`bulk-service-import.md`](bulk-service-import.md) for the
full pipeline.
