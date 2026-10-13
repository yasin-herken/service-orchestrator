# Architecture Simplification & Technical Debt Reduction (Task 028)

This document records the Task 028 audit, the simplifications that were
implemented, why each was justified, and the technical debt that was
deliberately left in place. The guiding principle is:

> Prefer the simplest architecture that satisfies the application's actual
> requirements. Do not optimize for hypothetical future use cases. Do not remove
> abstractions that protect real boundaries.

No externally observable behavior changed: the IPC protocol, the MCP tool
surface, the configuration schema, the TUI, and the GUI all behave exactly as
before. This is a maintainability change set, verified against the Task 027
regression suite.

---

## 1. How the audit was performed

The workspace was inspected directly (not from documentation alone):

- Every `trait` in `crates/*/src` and `apps/desktop/src-tauri` was enumerated with
  its production implementations, test doubles, and dispatch style (`dyn` vs
  generic).
- Every provider registry (`BuildProviderRegistry`, `RuntimeRegistry`,
  `MigrationProviderRegistry`, the health `HealthCheckerRegistry`, and the
  execution `ExecutorRegistry`) was inspected for its default registrations.
- The application core, IPC contracts, daemon router, error types, configuration
  model, and the React tree were reviewed for duplicated models, duplicated
  logic, dead code, and unused fields.
- Candidate findings were only accepted once proven by an exhaustive workspace
  search (for example, a config field was only called "unused" after no reader
  could be found anywhere).

The codebase is disciplined: zero `TODO`/`FIXME`/`unimplemented!()` markers in
the Rust sources, a single `#[allow(dead_code)]`-style suppression (now removed),
and a large existing regression suite. The debt was therefore *structural and
duplicative* rather than littered through the tree.

---

## 2. Confirmed findings

### 2.1 Rust

| # | Finding | Evidence | Verdict |
| --- | --- | --- | --- |
| R1 | A dead `use HealthState as _HealthState;` import + suppression kept an otherwise-unused import alive. | `crates/client/src/client.rs` | Removed |
| R2 | `impl HealthIndex for HealthManager` forwarded every call to `self.store` and was never used to build an `Arc<dyn HealthIndex>`; production wiring uses `health.store()`. | `crates/health/src/manager.rs`, `src/main.rs:366` | Removed |
| R3 | A dead `let _ = value;` binding in the identity migration. | `crates/config/src/migration.rs` | Removed |
| R4 | Two byte-identical `encode<T: Serialize>` helpers existed in the daemon. | `crates/daemon/src/server.rs`, `crates/daemon/src/dispatch.rs` | Consolidated |
| R5 | `ConfigLoader` is a single-implementation trait with no `dyn` use and no test double. | `crates/config/src/loader.rs` | **Retained** — see §4 |
| R6 | `ProcessRuntimeInspector` is implemented but never constructed; the default `NullRuntimeInspector` is what runs. | `crates/runtime/src/runtime.rs` | **Retained** — see §4 |
| R7 | The 8 coded `ApplicationError` variants differ only by prefix/namespace. | `crates/application/src/error.rs` | **Retained** — see §5 |
| R8 | `ServiceRuntimeStatus` is a near field-for-field copy of the port's `ServiceProcessInfo`. | `crates/application/src/models.rs`, `ports/service_runtime.rs` | **Retained** — see §5 |
| R9 | `RemoteClient`'s task-list methods use `.unwrap_or_default()` because their trait signatures are infallible. | `crates/client/src/remote.rs` | **Retained** — see §5 |

### 2.2 React / TypeScript

| # | Finding | Evidence | Verdict |
| --- | --- | --- | --- |
| F1 | `useServiceActions` re-implemented the entire `pending`/`operations`/`error` state machine that `useEntityOperations` already provides, and re-declared three equivalent types. | `hooks/useServiceActions.ts` vs `hooks/useEntityOperations.ts` | Consolidated |
| F2 | The `indeterminate` header-checkbox ref+effect was copy-pasted into five tables. | `ServiceTable`, `BuildTargetTable`, `RepositoryTable`, `LibraryTable`, `MigrationTable` | Extracted |
| F3 | `compareNullableString` was duplicated verbatim in two query modules; `SortDirection` lived in one of them as a side effect. | `utils/serviceQuery.ts`, `utils/repositoryQuery.ts` | Extracted |
| F4 | `parseList` was duplicated verbatim in three editors; `toCommand`/command rendering in two. | `ServiceEditor`, `ProfileEditor`, `GroupEditor`, `LibraryEditor` | Extracted |
| F5 | `isTerminalStatus` was a one-line pass-through to `isTerminalTaskStatus`. | `utils/taskProgress.ts` | Removed |
| F6 | `SERVICE_KINDS` and `TaskStatus` were exported but referenced nowhere. | `types/models.ts` | Removed |
| F7 | Four pages hand-roll their own list filters; four pages hand-roll selection bookkeeping. | `pages/*.tsx` | **Retained** — see §5 |
| F8 | Six `OrchestratorDesktopApi` methods are unused by production views. | `services/desktopApi.ts` | **Retained** — see §5 |
| F9 | No npm dependency is unused. | `package.json` | Nothing to remove |

---

## 3. Simplifications implemented

### 3.1 Rust

- **Dead suppression and import removed** (`crates/client/src/client.rs`): the
  `#[allow(unused_imports)] use HealthState as _HealthState;` hack and the
  redundant `HealthState` import are gone. `HealthState` was never referenced by
  the crate and is not re-exported; downstream crates import it from
  `application` directly.
- **Dead forwarding impl removed** (`crates/health/src/manager.rs`): the
  `HealthIndex` impl on `HealthManager` was pure indirection (`self.store`
  forwarding) that nothing constructed. `HealthStore` remains the sole
  `HealthIndex` implementation used at runtime.
- **Dead binding removed** (`crates/config/src/migration.rs`): the `let _ =
  value;` line in the identity migration was unreachable noise.
- **Duplicate daemon helper consolidated** (`crates/daemon`): `dispatch::encode`
  is now the single serialization helper (`pub(crate)`), and `server.rs` imports
  it instead of defining a second, identical copy.

### 3.2 React / TypeScript

- **`useServiceActions` now delegates to `useEntityOperations`**
  (`hooks/useServiceActions.ts`): the hook keeps its public
  `ServiceOperation`/`ActionErrorState`/`ServiceActions` shape (defined as
  aliases of the shared generic types) and only maps a `ServiceTarget` to the
  generic `OperationTarget` plus the single-vs-bulk dispatch. This removes the
  duplicated state machine and the three duplicated type declarations while
  matching the pattern already used by `useBuildOperations`, `useGitOperations`,
  and `useLibraryOperations`.
- **`useIndeterminateCheckbox` extracted** (`hooks/useIndeterminateCheckbox.ts`):
  one hook replaces five identical ref-and-effect pairs. The tables now import
  it and drop their `useEffect`/`useRef` imports.
- **Shared table comparators extracted** (`utils/compare.ts`):
  `compareNullableString`, `compareNullableNumber`, and the shared
  `SortDirection` type now live in one module; `serviceQuery` and
  `repositoryQuery` import them, and the two tables that imported
  `SortDirection` from `serviceQuery` now import it from `compare`.
- **Shared configuration helpers extracted** (`utils/configuration.ts`):
  `parseList`, `toCommand`, `structuredCommandLine`, and `commandText` replace
  the copies in the service, library, profile, and group editors.
- **Pass-through removed** (`utils/taskProgress.ts`): `busyServiceIds` calls
  `isTerminalTaskStatus` directly.
- **Dead exports removed** (`types/models.ts`): `SERVICE_KINDS` and `TaskStatus`.

---

## 4. Abstractions deliberately retained

- **`ConfigLoader`** (`crates/config/src/loader.rs`). Although it currently has a
  single implementation and no `dyn` use, it is the explicitly recorded
  extension seam for additional configuration sources
  (`docs/adr/003-configuration-system.md`,
  `docs/configuration.md`). It is inexpensive and already useful, so removing it
  would trade a documented boundary for a cosmetic line reduction — the exact
  trade-off `tasks/028.md` §23 warns against.
- **`ProcessRuntimeInspector`** (`crates/runtime/src/runtime.rs`). It plugs into a
  real `Arc<dyn RuntimeInspector>` seam and is a documented, planned composition
  step (`docs/adr/010-service-runtime.md` §"Consequences"). It is an extension
  point, not stray code, so it stays.
- **The `Null*` defaults** (`NullGitManager`, `NullArtifactInspector`, …): they are
  what let `Application::new` construct without filesystem, PATH, or network
  access. They are load-bearing, not dead.
- **Provider registries** (`BuildProviderRegistry`, `RuntimeRegistry`,
  `HealthCheckerRegistry`, `ExecutorRegistry`): each has genuinely multiple
  implementations and/or solves a real dispatch problem. The migration registry
  is currently the thinnest (one provider) but mirrors the same documented
  pattern; consolidating only it would make the codebase *less* uniform for no
  concrete benefit.
- **`RawEntity`** (`crates/config/src/management.rs`): a private generic helper
  with five macro-generated implementations, justified by the shared diff logic.

---

## 5. Remaining technical debt (documented, not changed)

These were confirmed during the audit but deliberately left alone. Each is either
a behavior/compatibility change, a protected boundary, or low value relative to
its risk.

1. **`ApplicationError`'s eight coded variants** (`{ code, message }` with only a
   namespace/prefix difference). Collapsing them into one `{ category, code,
   message }` would be cleaner but changes a type that interfaces and many tests
   match on. Deferred.
2. **`ServiceRuntimeStatus` vs `ServiceProcessInfo`.** The application model is a
   mechanical copy of the port snapshot plus a `managed` flag. They sit on
   opposite sides of the application/port boundary, so the duplication is
   intentional layer isolation; a shared constructor would only move ~6 lines.
3. **Error context flattened at boundaries.** `map_submit_error` collapses
   `TaskManagerError` into `TaskManager { message }`; three helpers re-derive an
   error code that the typed error already exposes. Structured codes are
   preserved for interfaces; deeper preservation is a behavior change.
4. **`RemoteClient` best-effort list methods.** `list_tasks`,
   `list_running_tasks`, `list_recent_tasks`, and `config_summary` return
   infallible values and therefore swallow a daemon error with
   `.unwrap_or_default()`. Making them fallible is a cross-interface API change
   (TUI, MCP, GUI), so it is out of scope for a behavior-preserving task.
5. **MCP tool names are independent string literals.** `crates/mcp/src/catalog.rs`
   advertises 73 tool names and `crates/mcp/src/server.rs` dispatches on matching
   string literals, but neither imports `service_orchestrator_client::ops`.
   Binding both to the wire constants would prevent silent drift; it is a
   structural change to the MCP layer and was not attempted here.
6. **Daemon routes six operations outside `dispatch.rs`** (daemon status/shutdown,
   configuration commit/reload, and the two log-subscription methods live in
   `server.rs`). `dispatch.rs` is not quite the single router it is documented to
   be. Folding them in requires threading connection-scoped state through the
   router; deferred.
7. **Parsed-but-not-honored configuration fields.** `RawPort.protocol` is
   validated and then discarded, and `MigrationDatabaseConfig.contexts`/`labels`
   are normalized and then never read. Removing them would reject existing
   configuration files (the TOML model rejects unknown fields) and honoring them
   would be a new feature, so neither is permitted by Task 028 §20/§36. Recorded
   here as known behavior.
8. **Frontend page-level duplication** (per-page list filters and hand-rolled
   selection bookkeeping in `BuildsPage`, `LibrariesPage`, `MigrationsPage`). The
   shared helpers already exist; adopting them across four pages is a broader,
   mechanical change with real regression surface and was left as the next
   low-risk increment.
9. **Six unused `OrchestratorDesktopApi` methods** (`getBuildCapabilities`,
   `getGitStatus`, `getGroup`, `getProfile`, `getServiceStatus`,
   `summarizeWorkflow`). They proxy real Tauri commands and are exercised by the
   test harness; they are "frontend-unused" rather than dead, so they are kept
   until a view needs them or the command is removed on both sides.
10. **`ProcessRuntimeInspector` is not wired.** `docs/adr/010` records this as a
    deliberate future composition step; the code and its seam remain ready.

---

## 6. Compatibility and security impact

- **IPC protocol:** unchanged. No operation name, request/response shape, or
  protocol version changed. The daemon `encode` consolidation is internal.
- **MCP:** unchanged tool names, schemas, resources, and prompts.
- **Configuration:** unchanged schema and semantics. No field was added or
  removed.
- **TUI / GUI behavior:** unchanged. All frontend edits are pure refactors of
  helpers and hooks.
- **Security:** no change to policy enforcement, authorization, destructive-action
  gating, process ownership, migration target restrictions, or secret redaction.
  The removed `HealthIndex` impl was never on an authorization path.

---

## 7. Tests and validation

New focused tests were added for the extracted frontend helpers:

- `apps/desktop/src/utils/compare.test.ts` — nullable string/number ordering.
- `apps/desktop/src/utils/configuration.test.ts` — `parseList`, `toCommand`,
  `commandText`, `structuredCommandLine`.
- `apps/desktop/src/hooks/useIndeterminateCheckbox.test.tsx` — the DOM
  `indeterminate` property is set, distinct from `checked`.

One obsolete test was removed: the `isTerminalStatus` case in
`utils/taskProgress.test.ts` duplicated coverage already in
`utils/taskStatus.test.ts`.

See the Task 028 progress record (`docs/task-028-progress.md`) for the exact
validation commands and results.

The public behavior of every affected area is covered by the pre-existing suite,
which is what makes these refactors safe to make without characterization tests:
the shared hook, comparators, configuration helpers, and table hook are all
exercised through the existing page/component and API tests.

---

## 8. Measurements

- Rust: 4 focused changes across 4 crates (`client`, `config`, `daemon`,
  `health`); no public API removed except one unused non-exported impl and one
  unused import.
- React: 5 duplicated helpers/hooks consolidated; 12 tracked files and 6 new
  files touched; the largest single reduction is `useServiceActions` (≈60 lines
  of duplicated state machine removed).
- Dependencies removed: **none** (the audit found no unused Cargo or npm
  dependency, and no dependency was added).
- Net diff before new tests: ≈71 insertions / ≈289 deletions across tracked
  files, plus small new shared modules.
