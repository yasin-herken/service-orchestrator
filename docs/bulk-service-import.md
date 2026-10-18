# Bulk Service Import

Bulk service import adds many repositories to a workspace in one reviewed
operation. It is built for the case this project is designed around: dozens of
related repositories that share a build system, runtime, branch, and health
convention.

The flow is deliberately split into two read-only stages and one write:

```text
discover (read-only)
      ↓
plan + preview (read-only)
      ↓
commit (one atomic, revision-checked configuration write)
```

Nothing in the first two stages clones, builds, migrates, starts, or writes
anything. The commit writes configuration only; it does not execute a project
script. Services become usable through the normal Builds / Workflows / Services
screens afterwards.

## 1. Discovery

Discovery walks the workspace root and reports candidate repositories. It is
bounded by configuration and never follows an unbounded tree:

```toml
[discovery]
max_depth = 6            # default 6, maximum 64
exclude = ["vendor"]     # additional names, on top of the built-ins
follow_symlinks = true   # default true; links must stay inside the workspace root
max_repositories = 200   # default 200, 1–10 000
```

The built-in exclusions are `.git`, `node_modules`, `target`, `build`, `dist`,
and `out`, plus any configured `exclude` entries.

For each directory it reports:

- the directory name and workspace-relative path;
- whether it is a Git checkout, its current branch, and its `origin` remote;
- the project files seen;
- detected build systems (Maven, npm, Gradle), runtimes (Java/Node), and
  integrations (Spring Boot, React, Liquibase);
- a confidence (`uncertain`, `low`, `medium`, `high`) and the evidence for each
  detection;
- existing configured services/libraries that already occupy the path.

A Gradle project is reported truthfully as *detected*, with a note that its
execution provider is not installed. It is never presented as importable
tooling and never used to select a template.

Discovery warnings (an unreadable directory, a symlink that escapes the root, a
truncated scan) are surfaced instead of failing the whole scan. A repository cap
that stops the scan early sets `truncated`.

A repository is never imported merely because it was found.

## 2. The ten-step wizard

The desktop wizard implements the required steps:

| Step | Content |
| --- | --- |
| 1 | **Select workspace.** The workspace comes from the header selector; the step shows the discovery bounds and a Scan button. Nothing is scanned implicitly. |
| 2 | **Discover repositories.** Shows the scan result, warnings, truncation, and counts. |
| 3 | **Select repositories.** Every discovered repository is listed with checkboxes, select-all/visible and select-none. The Next button is disabled for an empty selection. |
| 4 | **Review detected technologies.** Per repository: build systems, runtimes, integrations, confidence, evidence, and the suggested template/category. |
| 5 | **Assign templates.** Automatic (`matched`) or explicit. An explicitly chosen template wins; “Automatic” leaves the daemon's matching in place. |
| 6 | **Bulk overrides.** Shared values for all selected repositories: branch, build system, runtimes, build/start command, migration preset, port base, health path, and similar. |
| 7 | **Per-service exceptions.** One expandable row per repository for id, type, branch, URL, build system/operation, runtimes, ports, commands, migration, database environment, and the existing-entity policy. |
| 8 | **Validate all configurations.** Calls the daemon planner; shows per-row actions, created/updated/skipped counts, warnings, and blocking problems. |
| 9 | **Preview changes.** Shows the daemon's diff and validation over the whole proposed document, plus the resolved entity for each repository. |
| 10 | **Commit import.** Confirms explicitly, commits the reviewed revision, and reports created/updated/skipped entities and the new configuration revision. |

The wizard never requires opening an individual form per service; the shared
step fills every row, and the table edits exceptions in place.

## 3. Override precedence

The daemon resolves every field with one deterministic order:

```text
per-repository overrides → shared bulk overrides → template defaults → detection
```

- An explicit override always wins, including an explicitly empty value where
  the field permits it.
- Non-empty runtime and port lists replace the template's lists as a whole
  (they are not merged element-wise).
- A value that remains unknown is reported as a blocking problem with a stable
  code; it is never filled with a hidden guess.

The GUI uses the same request shape for a shared override and for a single row,
so a “bulk” value and a “per-row” value cannot behave differently.

## 4. Port assignment

Ports are assigned deterministically when the import runs:

1. Explicit per-row ports win. Duplicates within a row are rejected.
2. Explicit shared ports are applied to rows that have none.
3. Remaining rows that need a port take the next free port starting at
   `port_base`, in workspace-path order, skipping ports already used in the
   configuration and in this plan.
4. The plan is rejected with `PORT_REQUIRED` if a value (for example a health
   URL using `${port}`) needs a port and neither an override nor a port base is
   available.
5. `PORT_RANGE_EXHAUSTED` is reported if the sequential assignment runs past
   65535.
6. A duplicate port produced by explicit values is `PORT_CONFLICT`.

Port assignment is part of planning. Re-planning on commit therefore reproduces
the same result for the same inputs, and a stale reviewed plan is rejected
rather than overwriting ports chosen by someone else.

## 5. Duplicate detection and existing entities

Existing configuration is matched by **normalized workspace-relative path**,
using both the entity `path` and the repository checkout path. Display names are
never used as identity.

Each discovered repository is annotated with the configured services/libraries
that already occupy its path. The policy is per row:

- **Skip** (default) — the existing entity is left untouched and the row becomes
  a skip.
- **Update** — the existing entity is replaced by the composed one. This is a
  destructive change: it requires explicit per-row selection, a
  `Destructive` permission at commit, and a confirmation in the GUI. It is never
  applied silently.

Renaming an existing entity during a bulk import is not supported
(`RENAME_NOT_SUPPORTED`); edit it on the Configuration screen instead. Two
selected repositories that resolve to the same target are rejected with
`DUPLICATE_EXISTING_TARGET`.

## 6. Plan and commit

`plan_bulk_import` (IPC `plan_bulk_import`) is read-only. It:

1. resolves the workspace and its configuration;
2. runs the bounded discovery scan and indexes existing entities;
3. inspects each selected repository;
4. resolves the template, applies overrides, assigns ports, and composes the
   exact raw service/library entry;
5. previews the whole proposed document through the shared configuration
   manager;
6. returns rows with one action each (`create`, `update`, `skip`, `reject`), the
   summary counts, validation issues, warnings, the proposed entities, and the
   diff.

`commit_bulk_import` (IPC `commit_bulk_import`) takes the same request plus the
**revision of the plan the developer reviewed**. It re-runs the plan against the
current configuration, validates the whole proposed document again, and applies
all changes through one atomic batch transaction:

- all changes are applied in order to a single in-memory document;
- the whole document is validated;
- the on-disk revision is checked again immediately before the write;
- the file is written atomically with the usual backup and permission
  preservation;
- a stale revision fails with `CONFIG_CONFLICT` and writes nothing;
- the configuration is reloaded and broadcast to the TUI, MCP, and GUI.

If every row is a skip, the commit is a no-op that reports the skips and does not
rewrite the file. No write means no revision conflict, and nothing else is
changed.

An invalid plan, a rejected row, or a validation issue blocks the commit. The
GUI disables its import button until the plan is valid and the reviewed request
is unchanged; the daemon enforces the same rules regardless of the GUI.

## 7. Permissions and safety

- Discovery and planning are `Read`/`SafeWrite`-safe inspection and execute
  nothing.
- A commit with only creates/skips requires `SafeWrite`.
- A commit that updates any existing entity requires `Destructive`, because an
  update overwrites hand-written configuration.
- The GUI forwards a destructive flag as an intent only. The daemon authorizes
  the request itself; a frontend flag is never authorization.
- No destructive operation is retried automatically, and no rollback is claimed.
  If the atomic write fails, the previous file is unchanged and the error is
  reported.

## 8. Blocking error codes

Planning reports stable codes. The GUI shows the message and the repository it
belongs to; it does not parse text.

| Code | Meaning |
| --- | --- |
| `INSPECTION_FAILED` | The repository could not be inspected read-only. |
| `REPOSITORY_MISSING` | The selected path does not exist. |
| `TEMPLATE_UNKNOWN` | An explicitly assigned template id does not exist. |
| `TEMPLATE_AMBIGUOUS` | Several templates share the matched category; assign one explicitly. |
| `TEMPLATE_VARIABLE_INVALID` | A placeholder is unknown, empty, or unterminated. |
| `TEMPLATE_RESOLUTION_FAILED` | A template value could not be resolved (for example `${port}` with no port). |
| `SERVICE_TYPE_REQUIRED` | No type from template, override, or detection. |
| `DUPLICATE_EXISTING_TARGET` | Two selections resolve to the same existing target. |
| `RENAME_NOT_SUPPORTED` | An update would change an existing entity id. |
| `BRANCH_REQUIRED` | No branch from override, template, or detected checkout. |
| `REPOSITORY_URL_REQUIRED` | No clone URL from override or `origin`. |
| `ARTIFACT_REQUIRED` | A library has no complete Maven artifact coordinate. |
| `PORT_CONFLICT` | Two explicit ports collide. |
| `PORT_REQUIRED` | A port is needed but neither override nor base is available. |
| `PORT_RANGE_EXHAUSTED` | Sequential assignment ran past 65535. |
| `MIGRATION_PRESET_UNKNOWN` | The assigned migration preset does not exist. |
| `DESTRUCTIVE_TARGET_NOT_AUTHORIZED` | A destructive preset targets an environment it may not target. |
| `INVALID_NAME` / `NAME_REQUIRED` | The composed id is invalid or empty. |
| `CONFIGURATION_INVALID` | The whole proposed document failed validation. |

Ambiguous detections, a missing template category, or an undetected migration
file are **warnings**, not silent decisions. They are shown in the review and
never change the outcome behind the developer's back.

## 9. What the import writes

A service row becomes an ordinary `[[services]]` entry with:

- `id`, `name`, `type`, `workspace`, `path`, `default_branch`;
- `repository.url` (detected or explicit);
- `build_system`, runtimes, commands, ports, health checks, environment;
- `liquibase` settings when configured;
- `build_operation` when set.

A template of type `library` becomes an ordinary `[[libraries]]` entry with its
artifact coordinate, build command, and build operation. A library still needs a
resolvable Maven coordinate; otherwise the row is rejected with
`ARTIFACT_REQUIRED`.

No repository is cloned, no dependency is installed, and no build or migration
runs as part of the import.

## 10. Using it afterwards

After a successful import:

- the new services appear on Services, Git, Builds, Migrations, and Workflows;
- configuration counts and the configuration revision advance;
- the change is visible to every interface because it was committed through the
  shared manager and broadcast;
- use **Bulk Operations** to sync, build, migrate, start, and verify the new
  set in one coordinated workflow (`docs/bulk-operations.md`).

If a reviewed plan is stale, the wizard reports a conflict and asks for a fresh
scan/plan; nothing is partially imported.
