# Acceptance Test Matrix — Task 030

Structured, traceable record of the end-to-end acceptance testing performed for
Task 030 (Final Integration & Acceptance Testing). Every row states what was
executed and what actually happened. **No unexecuted scenario is marked as
passed** — scenarios that cannot be exercised in this environment are recorded as
`SKIPPED` or `MANUAL` with the reason.

- The default acceptance suite is [`tests/acceptance.rs`](../tests/acceptance.rs),
  a new root integration target that drives the **real `service-orchestrator`
  binary**: a real shared daemon over the real Unix-domain-socket IPC, a real
  MCP stdio server, real local Git repositories with a real bare remote, a real
  Node.js HTTP fixture service, and real HTTP health probes.
- It runs on `./scripts/test-acceptance.sh` (≈19 s, hermetic).
- Heavier real-toolchain and scale runs are `#[ignore]`d and opt-in
  (`INCLUDE_REAL_TOOLCHAIN=1`, `INCLUDE_PERFORMANCE=1`).

## Environment

| Item | Value |
| --- | --- |
| Machine | Apple Silicon (`arm64`), macOS 26.6.2 (Darwin) |
| rustc / cargo | 1.98.1, stable (`rust-toolchain.toml`) |
| Node / npm | v22.18.0 / 11.11.1 |
| Java / Maven | OpenJDK 25.0.4.1 / Apache Maven 3.9.16 |
| Isolation | Throwaway temp workspaces, per-test private Unix socket pinned via `SERVICE_ORCHESTRATOR_SOCKET`, local bare Git remotes, in-memory H2, isolated Maven local repository. No real repository, database, credential, or developer service is touched. |

## Legend

| Status | Meaning |
| --- | --- |
| **PASS** | Executed and the observed result matched the expected result. |
| **FAIL** | Executed and the observed result did not match; see the defect column. |
| **SKIPPED** | Deliberately not executed in this environment; reason given. Not a pass. |
| **MANUAL** | Requires an interactive session and could not be automated; only the automatable part is claimed. |

## How to reproduce

```bash
# Default hermetic acceptance suite (real binary + real daemon over IPC):
./scripts/test-acceptance.sh

# Add the real Maven/Liquibase `#[ignore]`d acceptance runs:
INCLUDE_REAL_TOOLCHAIN=1 ./scripts/test-acceptance.sh

# Add the scale/measurement harnesses:
INCLUDE_PERFORMANCE=1 ./scripts/test-acceptance.sh

# Everything else the matrix cites:
cargo test --workspace
./scripts/test-security.sh
RUN_DESKTOP_TESTS=1 ./scripts/test-e2e.sh
(cd apps/desktop && npm run typecheck && npm run test && npm run build)
```

---

## 1. Application startup

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-01 | Rust binary and daemon mode start | Built binary | `service-orchestrator daemon --config <fixture>` | Daemon becomes reachable over its socket; serves requests | Reachable in ≈1.5 ms; `list_workspaces`/`list_services` succeed | **PASS** | `tests/acceptance.rs` (`Env::start`), `crates/daemon/tests/daemon.rs` | — |
| AC-02 | TUI mode starts and renders | Built binary, in-memory core | Start `App` over an in-memory `Application`, render frames through Ratatui `TestBackend` | Startup lists services; every screen renders; a too-small terminal shows a notice, never panics | Verified by the TUI integration suite; startup/navigation/rendering all green | **PASS** | `crates/tui/tests/tui.rs` (`startup_lists_the_configured_services`, `every_workspace_screen_renders_without_panicking`, `a_terminal_below_the_minimum_shows_a_notice_instead_of_panicking`) | — |
| AC-03 | MCP mode starts | Built binary | `service-orchestrator mcp --config <fixture>`, then `initialize` over stdio | A clean JSON-RPC session; stdout carries only protocol traffic | `initialize` succeeds; tool calls return the structured envelope; clean exit on stdin EOF | **PASS** | `tests/acceptance.rs::mcp_stdio_drives_the_shared_daemon`, `tests/mcp_stdio.rs` | — |
| AC-04 | Desktop development mode starts | Frontend toolchain | `npm run build`; Tauri adapter tests | Frontend bundles; adapter compiles and runs | `npm run build` succeeds (Vite); Tauri adapter builds and its tests run | **PASS** | `apps/desktop` build; `RUN_DESKTOP_TESTS=1 ./scripts/test-e2e.sh` | — |
| AC-05 | Invalid configuration is rejected with actionable errors | A malformed config | Load config | A structured, field-specific error; no panic | All invalid-config fixtures rejected with specific messages; `reports_multiple_problems_at_once` reports several at once | **PASS** | `crates/config/tests/configuration.rs` (46 cases incl. `rejects_*`) | — |
| AC-06 | Missing configuration is handled | No config path | Start with a missing file | A clear "configuration not found" error, not a panic | Structured error; nothing is started | **PASS** | `crates/config/tests/configuration.rs`, `crates/daemon/tests/daemon.rs` | — |

## 2. Daemon lifecycle

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-07 | Single instance, status, graceful stop | A running daemon | Start a second daemon on the same socket; run `daemon status`; run `daemon stop` | Second daemon refuses (non-zero); status reports running; stop shuts down and releases the socket | Second daemon exited non-zero; status reported `running`; stop exited cleanly and the socket was gone | **PASS** | `tests/acceptance.rs::daemon_lifecycle_status_single_instance_and_stop` | — |
| AC-08 | Stale socket and lock recovery | A stale socket file and lock file in the run directory | Start the daemon | Recovers safely and serves normally | Daemon started over the stale artifacts and served `list_workspaces` | **PASS** | `tests/acceptance.rs::daemon_recovers_from_a_stale_socket_and_lock_file` | — |
| AC-09 | Client connect / disconnect / reconnect | A running daemon | Many clients connect and drop; the daemon keeps serving | No state loss; later clients see the same daemon | Verified alongside shared state (AC-13) and desktop IPC suite | **PASS** | `crates/daemon/tests/daemon.rs`, `apps/desktop/src-tauri/tests/daemon_ipc.rs` | — |
| AC-10 | Closing an interface does not stop the daemon | A running daemon shared by clients | Exit the MCP client; keep operating | The daemon survives an interface exit | After the MCP server exited `0`, the daemon still answered `daemon_status` | **PASS** | `tests/acceptance.rs::mcp_stdio_drives_the_shared_daemon` | — |

## 3. Shared authoritative state

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-11 | Cross-client shared runtime state | A running daemon | Client A starts a service; client B reads the same service | Both observe the identical authoritative state | The second client observed `process_state = "running"` | **PASS** | `tests/acceptance.rs::shared_state_and_queued_task_cancellation_across_clients`, `mcp_stdio_drives_the_shared_daemon` | — |
| AC-12 | Shared queued-task cancellation | `max_parallel_tasks = 1`; one slow build running | Client A submits a second build (queued); client B cancels it | The queued task settles as `cancelled`, observed by the other client | Task settled `cancelled` and was observed terminal by the second client | **PASS** | `tests/acceptance.rs::shared_state_and_queued_task_cancellation_across_clients` | — |
| AC-13 | GUI / TUI / MCP share one daemon | Tauri adapter, real daemon | Drive mutations through the adapter; read through the shared core | One authoritative store; no independent registry | Adapter mutations and reads hit the same real daemon; `every_client_sees_the_same_authoritative_daemon`, `gui_and_other_interfaces_share_one_authoritative_state` pass | **PASS** | `apps/desktop/src-tauri/tests/daemon_ipc.rs` | — |

## 4. Workspace loading and validation

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-14 | Valid workspace loads and lists entities | A fixture config with services, a library, a group, a profile | Start the daemon; list workspaces/services; validate | Correct entities; validation totals accurate | 5 services + 1 library listed; validation totals correct | **PASS** | `tests/acceptance.rs::workspace_loads_and_validation_reports_actionable_diagnostics` | — |
| AC-15 | Missing repository reported as a diagnostic | A configured service never cloned | Validate the workspace | A structured `MissingRepository` diagnostic; `repositories_missing = 1` | Diagnostic present; count correct; no task scheduled | **PASS** | `tests/acceptance.rs::workspace_loads_and_validation_reports_actionable_diagnostics` | — |
| AC-16 | Unknown workspace is a structured error | A running daemon | Validate an unknown workspace id | A structured, non-panicking error naming the resource | Error returned, message mentions the workspace | **PASS** | `tests/acceptance.rs::workspace_loads_and_validation_reports_actionable_diagnostics` | — |
| AC-17 | Config validation: duplicates, paths, deps, profiles, groups, unicode/spaces | Many malformed fixtures | Load each | Each rejected with a specific, actionable message | 46 focused cases cover unknown deps, duplicate ids, path escape, invalid groups/profiles, unsupported versions, etc. | **PASS** | `crates/config/tests/configuration.rs`; `crates/application/tests/workspace_validation.rs` | — |

## 5. Git acceptance

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-18 | Status, branches, fetch, pull, checkout | A fresh clone of a local bare remote | Read status; list branches; add a remote branch+commit; fetch; pull; checkout the new branch | Status clean on `main`; the fetched remote branch is visible; pull fast-forwards and leaves a clean tree; checkout lands on `feature` | All observed exactly as expected | **PASS** | `tests/acceptance.rs::git_status_fetch_pull_branches_and_checkout` | — |
| AC-19 | Bulk sync to the default branch protects local changes | Two repos, one dirty (untracked file) | Sync both | The clean repo syncs; the dirty repo is refused; the untracked file survives; results are aggregated | `succeeded=1`, `failed=1`; the untracked file survived; the failure carried a structured reason | **PASS** | `tests/acceptance.rs::bulk_sync_continues_when_one_repository_is_dirty` | — |
| AC-20 | Clone, diverged branch, missing upstream, timeout, cancellation | Local fixtures | Run each Git edge case | Structured outcomes; destructive actions refused without confirmation | All covered by the Git integration suite (`pull_refuses_a_diverged_branch_*`, `clone_*`, `status_reports_a_branch_with_no_upstream`, `an_operation_that_exceeds_its_timeout_*`, `an_already_cancelled_operation_*`, `operations_reject_a_checkout_outside_the_workspace`) | **PASS** | `crates/git/tests/git_manager.rs` | — |

## 6. Maven / npm / library builds

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-21 | Real Maven library install + artifact verification | Maven + JVM + network; isolated settings and local repository | Build a single-module and a multi-module Maven library; locate artifacts | Exit 0; every artifact present in the isolated repo | 4/4 real-Maven tests passed (single module, reactor, missing artifact, local-repo provider) | **PASS** | `crates/build/tests/maven_library.rs` (`--ignored`) | — |
| AC-22 | npm build / node runtime validation | Node + npm | Build a Node service fixture | The configured build command runs; a bad runtime version is rejected | Real Node fixture starts and serves in acceptance; provider tests cover npm plan/exec and version validation | **PASS** | `crates/build/tests/providers.rs`, `crates/build/tests/manager.rs`, `tests/acceptance.rs` | — |
| AC-23 | Library discovery, ordering, version alignment | Fixture libraries | Plan and run library builds; check consumer alignment | Dependency order respected; mismatch reported | Covered by the application/build/library suites incl. version-alignment mismatch reporting | **PASS** | `crates/application/tests/library_manager.rs`, `crates/build/tests/library.rs` | — |

## 7. Migration acceptance

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-24 | Real Liquibase validate / apply / status / preview | Maven + JVM + network; in-memory H2; isolated Maven settings | Run each migration operation against a fixture Maven project | Exit 0 for each operation | 2/2 real-Liquibase tests passed (apply+status+preview, validate) | **PASS** | `crates/liquibase/tests/liquibase_maven.rs` (`--ignored`, isolated via `scripts/test-acceptance.sh`) | — |
| AC-25 | Migration target authorization and safety | Configuration fixtures | Attempt unauthorized targets; inspect redaction | Unauthorized targets refused; secrets redacted; locks not auto-released | Covered by the Liquibase manager/task-engine suites and the security suite | **PASS** | `crates/liquibase/tests/manager.rs`, `crates/liquibase/tests/task_engine.rs`, `crates/liquibase/src/*` auth tests | — |

## 8. Service runtime and health

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-26 | Real Node service starts, is healthy, stops | Node installed | Start `orders-api`; read runtime status; check HTTP health; duplicate start; stop | Managed process with a pid; HTTP health `Healthy`; duplicate start is a no-op; stop transitions away from active | All observed; duplicate start returned "not scheduled" | **PASS** | `tests/acceptance.rs::service_start_health_duplicate_start_and_stop` | — |
| AC-27 | Unexpected process exit is observed; unrelated processes untouched | A running service and an unrelated `sleep` process | `kill -9` the managed pid | Terminal state recorded; the decoy is never signalled; the daemon keeps serving | Terminal state reached; decoy untouched; daemon still serving | **PASS** | `tests/acceptance.rs::unexpected_service_exit_is_reported` | — |
| AC-28 | Health checks: HTTP, TCP, readiness, transitions, timeout | Fixture endpoints | Run the health suite | Correct classification; running ≠ healthy; timeout honoured | All health-manager cases green (`a_managed_process_exit_changes_health`, `readiness_times_out_when_the_service_never_becomes_ready`, `an_unmanaged_process_is_unhealthy`, …) | **PASS** | `crates/health/tests/health_manager.rs` | — |
| AC-29 | Port-conflict handling and process ownership | A service whose port is taken | Start and inspect | A structured conflict error; ownership rules applied | Covered by the runtime/process suites and the acceptance start/stop path | **PASS** | `crates/runtime/tests/service_manager.rs`, `crates/process/tests/process_manager.rs` | — |

## 9. Logs and observability

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-30 | Live logs, historical paging, filtering, retention, redaction | A running hub | Subscribe; page history; filter; cap retention; emit a secret | Bounded, paginated, filterable; secrets redacted; slow subscribers lag without blocking | All observability cases green (`historical_retrieval_is_bounded_and_filterable`, `per_service_retention_caps_a_chatty_service`, `total_retention_caps_the_whole_hub`, `recorded_messages_are_redacted`, `a_slow_subscriber_lags_without_blocking_the_producer`) | **PASS** | `crates/logging/tests/observability.rs` | — |

## 10. Workflows

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-31 | Prepare-environment workflow and dependency ordering | A fixture workspace | Preview and run a prepare workflow | Tasks run in dependency order; results aggregated | Covered by the application-core/workflow suites and the acceptance bulk-sync workflow | **PASS** | `crates/application/tests/application_core.rs`, `tests/acceptance.rs::bulk_sync_continues_when_one_repository_is_dirty` | — |
| AC-32 | Workflow failure, partial success, blocked dependencies, cancellation | Fixtures with a failing step | Run and cancel workflows | Independent work continues; dependents blocked; no rollback claimed | Covered by the execution/application suites (`cancelling_a_blocked_task_*`, failure-propagation tests) | **PASS** | `crates/execution/tests/task_engine.rs`, `crates/execution/tests/scheduler_stress.rs`, `crates/application/tests/application_core.rs` | — |

## 11. TUI acceptance

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-33 | TUI navigation, listing, multi-selection, filtering | In-memory core | Drive the real `App` state machine with key events over a `TestBackend` | Selection is by typed id and survives filtering; typing narrows the list; bulk actions target the selection | All green | **PASS** | `crates/tui/tests/tui.rs` (`navigation_and_quit_are_keyboard_driven`, `selection_is_by_typed_id_and_survives_filtering`, `bulk_actions_target_the_selection`) | — |
| AC-34 | TUI task monitoring, error dialogs, destructive confirmation, terminal restoration | In-memory core | Dispatch actions; observe task events; quit | Actions become tasks; errors are actionable; destructive actions require confirmation; quitting restores the terminal and does not touch the engine | All green | **PASS** | `crates/tui/tests/tui.rs` (`service_actions_are_dispatched_as_tasks_through_the_engine`, `destructive_actions_require_confirmation`, `application_errors_are_presented_actionably`, `quitting_stops_the_interface_without_touching_the_engine`) | — |
| AC-35 | TUI live visual/interaction on a real terminal | An interactive terminal | Launch the TUI and use it by hand | Correct colours/layout/interaction | Not automated in this environment | **MANUAL** | Automated rendering via `TestBackend` covers frame content; a human visual pass is recommended before release | — |

## 12. MCP acceptance

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-36 | Initialize, tool discovery, structured results, graceful shutdown | The real MCP binary | `initialize`, `tools/call`, close stdin | Clean session; structured envelope; exit code 0 | All observed; stdout contained only JSON-RPC | **PASS** | `tests/acceptance.rs::mcp_stdio_drives_the_shared_daemon`, `tests/mcp_stdio.rs` | — |
| AC-37 | Structured errors and untrusted-input rejection | The real MCP server | Send path traversal, flag injection, oversized requests | Structured rejection; no shell execution | Covered by the MCP suite and the security suite | **PASS** | `crates/mcp/tests/*`, `./scripts/test-security.sh` | — |
| AC-38 | Authorization and secret redaction through MCP | The real MCP server | Attempt a destructive op; request logs containing a secret | Destructive ops denied for MCP; secrets redacted | Covered by the daemon/MCP authorization and redaction tests | **PASS** | `crates/daemon/tests/daemon.rs`, `crates/mcp/tests/*`, `./scripts/test-security.sh` | — |
| AC-39 | Resource and prompt discovery | The real MCP server | List resources and prompts | Resources/prompts are advertised and readable | Covered by the MCP integration suite | **PASS** | `tests/mcp_stdio.rs`, `crates/mcp/tests/*` | — |

## 13. Desktop GUI acceptance

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-40 | Adapter reads/validates/errors over real IPC | Real daemon (real protocol) | Drive adapter queries/mutations | Real data; invalid input rejected; a stopped daemon yields a structured `unavailable` error | All green | **PASS** | `apps/desktop/src-tauri/tests/daemon_ipc.rs` (`the_adapter_reads_workspaces_services_and_status_over_ipc`, `the_adapter_rejects_invalid_mismatched_and_unknown_input`, `a_stopped_daemon_surfaces_a_structured_unavailable_error`) | — |
| AC-41 | GUI service lifecycle, Git, builds, libraries, cancellation | Real daemon | Drive adapter operations | Operations run through the shared core; partial failures reported per item | All green | **PASS** | `apps/desktop/src-tauri/tests/daemon_ipc.rs` (`the_adapter_drives_the_service_lifecycle_through_the_shared_core`, `git_fetch_checkout_and_bulk_sync_run_through_the_shared_core`, `build_targets_carry_provider_capabilities_and_builds_run`, `library_operations_run_through_the_shared_core`, `a_running_operation_can_be_cancelled`, `bulk_builds_report_partial_failure_per_target`) | — |
| AC-42 | Frontend pages: dashboard, services, git, builds, libraries, migrations, tasks, health, logs, workflows, profiles, settings | `paths` | Run the Vitest page suites | Each page renders loading, data, and error states; navigation works | 24 files / 231 tests pass | **PASS** | `apps/desktop/src/**/*.test.tsx` | — |
| AC-43 | Frontend production build and type check | Frontend toolchain | `npm run build`, `npm run typecheck` | Bundle builds; no type errors | Both succeed | **PASS** | `apps/desktop` build output | — |
| AC-44 | Live Tauri window: keyboard accessibility, loading/error states, reconnection, visual layout | An interactive desktop session | Launch the app window and use it by hand | Correct interactive behaviour | Not automated in this environment | **MANUAL** | The adapter data layer and page components are automated (AC-40…AC-43); a human window pass is recommended before release | — |

## 14. Security acceptance

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-45 | Security regression suite (IPC, policy, redaction, injection, limits) | Task 026 suites | `./scripts/test-security.sh` | All invariants hold; no filter matches zero tests | Passed | **PASS** | `./scripts/test-security.sh` | — |
| AC-46 | Destructive authorization cannot be forged | A running daemon | Assert `allow_destructive` for MCP / non-same-UID | Destructive ops allowed only for same-UID interactive clients | Covered by the daemon `destructive_allowed` matrix | **PASS** | `crates/daemon/tests/daemon.rs` | — |
| AC-47 | Path traversal and Git flag injection are rejected | Untrusted input | Send traversal/injection payloads | Rejected, never executed | Covered by the security suite | **PASS** | `./scripts/test-security.sh`, `crates/mcp/tests/*` | — |

## 15. Performance acceptance

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-48 | Daemon startup and IPC latency | Release binary, real socket | Measure startup and 500 `list_services` round trips | Startup < 100 ms; IPC p95 < 5 ms | Startup median 1.50 ms; IPC median ≈330 µs / p95 ≈380 µs (debug re-check) | **PASS** | `crates/daemon/tests/daemon.rs` (`measure_daemon_startup_and_ipc_latency`) | — |
| AC-49 | Read paths at 40 / 100 / 200 services | Synthetic fixtures | Measure `list_services`, `validate_workspace`, config load | `list_services`@200 < 5 ms; `validate`@200 < 5 ms | Debug re-check: `list_services`@200 = 322 µs; `validate`@200 = 2.51 ms; config load@200 = 38.7 ms (startup-only). Release figures are far lower | **PASS** | `crates/application/tests/workspace_validation.rs` (`--ignored`) | — |
| AC-50 | Task engine throughput and scaling | Fake executor | Submit/drain 20 000 tasks; deep workflow 100/500/1000 steps | Submit < 50 µs/task; drain flat; workflow linear | Debug re-check: submit 15.55 µs/task (20 000 in 311 ms); drain 20.6–22.2 µs/task flat to 4 000 history; workflow linear (plan 1.5/6.7/15.1 ms) | **PASS** | `crates/execution/tests/performance.rs` (`--ignored`) | — |

## 16. Reliability and failure recovery

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-51 | Scheduler stress and repeated operations | Fake executor | High-churn scheduler stress | No deadlock, no leak, consistent state | Passed | **PASS** | `crates/execution/tests/scheduler_stress.rs` | — |
| AC-52 | Recovery: daemon stop/crash, service crash, stale socket, invalid config | Fixtures | Exercise each recovery path | Accurate reporting; no adoption/termination of unrelated processes | Covered by AC-08, AC-27, AC-05, and the daemon suite | **PASS** | `tests/acceptance.rs`, `crates/daemon/tests/daemon.rs`, `crates/config/tests/configuration.rs` | — |
| AC-53 | IPC interruption / event-subscription cleanup | Real daemon and clients | Drop clients mid-stream | No leaked subscriptions or tasks; daemon keeps serving | Covered by the daemon/desktop suites and the acceptance Env teardown | **PASS** | `crates/daemon/tests/daemon.rs`, `apps/desktop/src-tauri/tests/daemon_ipc.rs` | — |

## 17. Cross-interface consistency

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-54 | Stable ids and consistent state across interfaces | Real daemon | Start/cancel through one interface; read through another | Same service/task/workflow ids; consistent runtime/health/progress/cancellation | Covered by AC-11, AC-12, AC-13 | **PASS** | `tests/acceptance.rs`, `apps/desktop/src-tauri/tests/daemon_ipc.rs` | — |

## 18. Error quality

| ID | Scenario | Preconditions | Steps | Expected | Actual | Status | Evidence / logs | Defects |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| AC-55 | Failures carry operation, resource, category, safe context, next action | Fixtures that fail | Trigger Git/build/health/config/protocol failures | Structured, non-leaking errors | Covered by the error-reference, protocol-error (`INVALID_REQUEST`), and structured MCP error tests | **PASS** | `docs/error-reference.md`, `crates/daemon/tests/daemon.rs`, `crates/mcp/tests/*` | — |

---

## Summary

| Status | Count |
| --- | --- |
| PASS | 53 |
| FAIL | 0 |
| MANUAL | 2 (AC-35 live TUI visual pass; AC-44 live Tauri window pass) |
| SKIPPED | 0 |

Scenarios that could not be automated (AC-35, AC-44) are recorded as `MANUAL`,
not as passes: their data/command layers are automated (AC-02, AC-33, AC-34 for
the TUI; AC-40…AC-43 for the desktop), but a human visual/interaction pass on a
real session is recommended before packaging and is **not** claimed here.

No acceptance scenario failed and no Critical/High defect was found during Task
030. The only integration defect discovered — a non-hermetic real-Liquibase
acceptance run that read the developer's own `~/.m2/settings.xml` — was in the
**test harness**, not the product, and was fixed by isolating the run in
`scripts/test-acceptance.sh` (see
[`final-acceptance-report.md`](final-acceptance-report.md#confirmed-defects)).
