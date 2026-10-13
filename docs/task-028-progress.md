# Task 028 progress — Architecture Simplification & Technical Debt Reduction

Status: **complete**. See `docs/architecture-simplification.md` for the full
audit, the implemented simplifications, the retained abstractions, and the
remaining technical debt.

## Environment

| Item | Value |
| --- | --- |
| Machine | Apple M2 Pro, 32 GiB RAM |
| OS | macOS (Darwin), Apple Silicon |
| Rust | stable per `rust-toolchain.toml` |
| Node / npm | per `apps/desktop/package.json` |
| Build mode | debug (`cargo test`, `clippy`) |

## Plan

1. Audit the Rust workspace and the React tree for confirmed, measurable
   complexity: single-implementation traits, provider registries, duplicate
   helpers/DTOs, dead code, unused fields, and duplicated frontend state.
2. Implement only the findings that reduce real duplication without weakening a
   documented boundary, changing observable behavior, or contradicting an ADR.
3. Add focused tests for the extracted frontend helpers.
4. Document the audit and the changes in `docs/architecture-simplification.md`.
5. Run the full validation gate.

All steps done. Baseline branch: `027` (task 027 complete).

## Simplifications implemented

Rust:

- removed the dead `HealthState` import and its `#[allow(unused_imports)]`
  suppression in `crates/client/src/client.rs`;
- removed the dead, pure-forwarding `impl HealthIndex for HealthManager`
  (`crates/health/src/manager.rs`);
- removed the dead `let _ = value;` in `crates/config/src/migration.rs`;
- consolidated the two identical daemon `encode` helpers into
  `dispatch::encode` (`crates/daemon`).

React / TypeScript:

- rebuilt `useServiceActions` on the shared `useEntityOperations`;
- extracted `useIndeterminateCheckbox` (five identical ref+effect copies);
- extracted `utils/compare.ts` (duplicated nullable comparators +
  `SortDirection`);
- extracted `utils/configuration.ts` (`parseList`, `toCommand`, command
  rendering, duplicated across four editors);
- removed the `isTerminalStatus` pass-through;
- removed the unused `SERVICE_KINDS` and `TaskStatus` exports.

Retained on purpose (documented in the simplification doc): `ConfigLoader`,
`ProcessRuntimeInspector`, the `Null*` defaults, the provider registries, and the
`config` `RawEntity` helper.

## Tests added

- `apps/desktop/src/utils/compare.test.ts` (2 files/4 assertions across two
  blocks)
- `apps/desktop/src/utils/configuration.test.ts`
- `apps/desktop/src/hooks/useIndeterminateCheckbox.test.tsx`

One obsolete test removed (`isTerminalStatus` in `utils/taskProgress.test.ts`),
already covered by `utils/taskStatus.test.ts`.

No Rust tests were added: the changed Rust code is dead-code removal and an
internal helper consolidation with no new behavior to characterize.

## Final validation

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo check --workspace` | clean |
| `cargo test --workspace` | **977 passed / 0 failed / 17 ignored** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test` (`apps/desktop/src-tauri`) | **131 passed / 0 failed** |
| `npm run typecheck` (`apps/desktop`) | clean |
| `npm run test` (`apps/desktop`) | **231 passed / 0 failed** |
| `npm run build` (`apps/desktop`) | success |

Cross-interface consistency is covered by the desktop IPC suite
(`gui_tui_and_mcp_observe_the_same_authoritative_state`,
`every_client_sees_the_same_authoritative_daemon`) and the daemon/MCP end-to-end
suites included in `cargo test --workspace`.

Task 029 not started (per `tasks/028.md` §38).
