# Error Reference

Structured errors are the contract of every interface. Each error carries a
stable `code` and a human-readable `message`; an MCP tool result wraps it as
`{ "success": false, "error": { "code", "message", "details?" } }`, and the TUI
and GUI render it as an actionable dialog. An agent should read `error.code`
rather than retrying blindly.

This page lists the codes exposed to users, grouped by area. It does not list
internal errors that never cross an interface boundary.

## General and transport

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `WORKSPACE_NOT_FOUND` | No workspace with that id. | Check the id (`list_workspaces`). |
| `SERVICE_NOT_FOUND` | No service with that id. | Check the id (`list_services`). |
| `LIBRARY_NOT_FOUND` | No library with that id. | Check the id (`list_libraries`). |
| `GROUP_NOT_FOUND` | No group with that id. | Check the id (`list_groups`). |
| `PROFILE_NOT_FOUND` | No profile with that id. | Check the id (`list_profiles`). |
| `TASK_NOT_FOUND` | No task with that id (or it aged out). | List tasks; the store is in memory. |
| `WORKFLOW_NOT_FOUND` | No workflow with that id. | List tasks/workflows. |
| `INVALID_OPERATION` | The operation is not valid in this state. | Read the message; adjust the request. |
| `INVALID_ARGUMENT` | An identifier or value failed domain validation. | Use a valid id/value. |
| `INVALID_PATH` | A path is malformed or not allowed. | Use a workspace-relative path inside the root. |
| `PERMISSION_DENIED` | The operation requires authorization that was not granted. | Approve it interactively on a local client. |
| `CONFIRMATION_REQUIRED` | A destructive operation was denied because confirmation is required. | A human must perform it interactively. |
| `DEPENDENCY_NOT_FOUND` | A referenced dependency does not resolve. | Fix the reference. |
| `DEPENDENCY_RESOLUTION_FAILED` | The dependency graph could not be resolved. | Check ids and cycles. |
| `DEPENDENCY_CYCLE` / `SERVICE_DEPENDENCY_CYCLE` | The dependency graph contains a cycle. | Break the cycle. |
| `WORKFLOW_INVALID` | The workflow definition is invalid. | Inspect the plan. |
| `CONFIGURATION_INVALID` / `CONFIG_INVALID` | Configuration failed validation. | Fix the reported fields. |
| `LIBRARY_CONFIGURATION_INVALID` | A library's configuration is invalid. | Add the missing artifact/command. |
| `TASK_ENGINE_ERROR` | The task engine returned an error. | Retry; inspect the task. |
| `LOG_SERVICE_ERROR` / `INVALID_LOG_QUERY` | The log query is invalid. | Fix the filters (`since` > `until` is rejected). |
| `DIAGNOSTICS_UNAVAILABLE` | Diagnostics could not be assembled. | Ensure the service exists and is observable. |
| `VERSION_INSPECTION_UNSUPPORTED` | Version alignment is unsupported for this library. | Inspect manually. |
| `BUILD_PROVIDER_UNSUPPORTED` | The build system cannot perform the operation. | Use Maven/npm. |

### IPC / daemon transport

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `DAEMON_UNAVAILABLE` | The daemon could not be reached, or the connection dropped. | `daemon status`; see [`troubleshooting.md`](troubleshooting.md). |
| `PROTOCOL_INCOMPATIBLE` | The client and daemon speak different protocol versions. | Use matching client and daemon binaries. |
| `INVALID_REQUEST` | The request was malformed or exceeded a transport limit. | Fix the request; frames are capped at 8 MiB. |
| `UNKNOWN_OPERATION` | The daemon does not recognize the method. | The client and daemon are out of sync; rebuild. |
| `UNSUPPORTED_OPERATION` | The chosen `OrchestratorClient` does not implement the call. | Use the daemon-backed client. |

### Configuration management

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `CONFIG_READ_FAILED` | The config file could not be read. | Check the path and permissions. |
| `CONFIG_PARSE_FAILED` | TOML parsing failed (unknown field, bad type). | Fix the reported location. |
| `CONFIG_VERSION_UNSUPPORTED` | `config_version` is not 1. | Set `config_version = 1`. |
| `CONFIG_CONFLICT` | The file changed since the snapshot was read. | Refresh and re-apply the change. |
| `CONFIG_ENTITY_NOT_FOUND` | The entity to update/remove does not exist. | Refresh the snapshot. |
| `CONFIG_ENTITY_EXISTS` | Creating an entity with a duplicate id. | Choose a unique id. |
| `CONFIG_INVALID_CHANGE` | The change is structurally invalid. | Fix the change. |
| `CONFIG_DOCUMENT_INVALID` | The proposed document fails validation. | Fix the reported fields. |
| `CONFIG_WRITE_FAILED` | The atomic write failed. | Check permissions/disk; nothing was written. |
| `INSPECTION_UNAVAILABLE` | Repository inspection could not run. | Check the path exists and is readable. |

## Git

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `GIT_NOT_CLONED` | The repository directory is not a checkout. | Clone it or fix `path`. |
| `GIT_INVALID_REPOSITORY` | The directory is not a valid Git repository. | Re-clone. |
| `GIT_DESTINATION_NOT_EMPTY` | Clone target exists and is not empty. | Choose an empty path. |
| `GIT_REPOSITORY_UNAVAILABLE` | The repository could not be opened. | Check the path/permissions. |
| `GIT_WORKING_TREE_DIRTY` | Local changes block the operation. | Commit or stash them. |
| `GIT_BRANCH_NOT_FOUND` | The branch does not exist. | Fetch first, or use an existing branch. |
| `GIT_DETACHED_HEAD` | The repository is in detached-HEAD state. | Check out a branch. |
| `GIT_UPSTREAM_NOT_CONFIGURED` | The branch has no upstream. | Set an upstream. |
| `GIT_DIVERGED_BRANCH` | Pull cannot fast-forward. | Merge/rebase manually. |
| `GIT_MERGE_CONFLICT` | A merge produced conflicts. | Resolve manually. |
| `GIT_AUTHENTICATION_FAILED` | Git authentication failed. | Fix credentials/SSH. |
| `GIT_REMOTE_UNAVAILABLE` | The remote is unreachable. | Check network/URL. |
| `GIT_COMMAND_FAILED` | Git exited non-zero. | Read the message. |
| `GIT_TIMEOUT` | The operation timed out. | Retry; check the network. |
| `GIT_CANCELLED` | The operation was cancelled. | Rerun when ready. |
| `GIT_REPOSITORY_BUSY` | Another operation holds the repository. | Wait. |
| `GIT_INVALID_PATH` / `GIT_PATH_OUTSIDE_WORKSPACE` | The path is invalid or escapes the root. | Fix the path. |
| `GIT_INVALID_REFERENCE` / `GIT_INVALID_PARAMETER` / `GIT_MISSING_PARAMETER` | Bad Git argument. | Fix the request. |
| `GIT_UNAVAILABLE` / `GIT_INTERNAL_ERROR` | Git tooling unavailable or internal failure. | Ensure `git` is installed. |

## Build and artifacts

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `BUILD_COMMAND_FAILED` | The build tool exited non-zero. | Read the build logs. |
| `BUILD_UNSUPPORTED_SYSTEM` | No provider (Gradle). | Use Maven/npm. |
| `BUILD_UNSUPPORTED_OPERATION` | The provider does not support the operation. | Use a supported operation. |
| `BUILD_INVALID_TARGET` / `BUILD_PATH_OUTSIDE_WORKSPACE` | Invalid/escaping target. | Fix the service `path`. |
| `BUILD_TIMEOUT` | The build exceeded its timeout. | Raise the timeout; check the build. |
| `BUILD_CANCELLED` | The build was cancelled. | Rerun. |
| `BUILD_PROCESS_FAILED` / `BUILD_MISSING_PARAMETER` / `BUILD_INVALID_PARAMETER` | Build setup failure. | Fix the command/parameters. |
| `BUILD_SYSTEM_UNKNOWN` | No known build system. | Declare `build_system`. |
| `BUILD_INTERNAL_ERROR` | Internal build error. | Retry; report if it persists. |
| `ARTIFACT_NOT_FOUND` | The artifact is not in the local repository. | Install/prepare the library. |
| `ARTIFACT_VERSION_MISMATCH` | A consumer references a different version. | Align manually. |
| `ARTIFACT_INSTALLATION_FAILED` | Installing the artifact failed. | Read the build logs. |
| `ARTIFACT_INSPECTION_UNAVAILABLE` | Artifact inspection could not run. | Ensure Maven is installed. |
| `ARTIFACT_LOCAL_REPOSITORY_UNKNOWN` | The local repository could not be discovered. | Configure Maven. |
| `LIBRARY_BUILD_FAILED` | A library build failed. | Read the build logs. |

## Service runtime

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `RUNTIME_UNSUPPORTED` | The runtime type is not implemented. | Use `java`/`node`. |
| `RUNTIME_NOT_FOUND` | The runtime executable is not installed. | Install it. |
| `RUNTIME_VERSION_MISMATCH` | The installed version differs from the pin. | Install the pinned version or adjust it. |
| `RUNTIME_START_COMMAND_MISSING` | `run` capability without a start command. | Add `commands.start`. |
| `RUNTIME_WORKING_DIRECTORY_INVALID` | The working directory is invalid. | Fix `path`. |
| `RUNTIME_PORT_ALREADY_IN_USE` | The configured port is occupied. | Free the port; the occupant is never killed. |
| `RUNTIME_ALREADY_STARTING` | A start is already in progress. | Wait. |
| `RUNTIME_PROCESS_START_FAILED` | The process could not be spawned. | Check the command/directory. |
| `RUNTIME_PROCESS_EXITED` | The process exited during startup. | Read the service logs. |
| `RUNTIME_PROCESS_STOP_FAILED` | The process could not be stopped cleanly. | Retry; check the process. |
| `RUNTIME_OPERATION_CANCELLED` / `RUNTIME_OPERATION_TIMEOUT` | Cancelled / timed out. | Retry when ready. |
| `RUNTIME_INVALID_CONFIGURATION` / `RUNTIME_INVALID_PARAMETER` / `RUNTIME_MISSING_PARAMETER` | Bad configuration or request. | Fix it. |
| `RUNTIME_SERVICE_NOT_FOUND` / `RUNTIME_WORKSPACE_NOT_FOUND` | Unknown service/workspace. | Check the id. |
| `RUNTIME_INTERNAL_ERROR` | Internal runtime error. | Retry; report if it persists. |

## Migrations

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `MIGRATION_PROVIDER_NOT_FOUND` | No provider (Flyway). | Use Liquibase. |
| `MIGRATION_DISABLED` | Migrations are disabled. | Enable them. |
| `MIGRATION_NOT_CONFIGURED` | No Liquibase block. | Add `[services.liquibase]`. |
| `MIGRATION_UNSUPPORTED_OPERATION` | Operation unsupported. | Use a supported one. |
| `MIGRATION_COMMAND_MISSING` | `cli` mode without a command. | Configure the command. |
| `MIGRATION_INVALID_CONFIGURATION` / `MIGRATION_INVALID_TARGET` | Bad configuration/target. | Fix it. |
| `MIGRATION_DATABASE_TARGET_UNKNOWN` / `MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED` | The target is unknown or not authorized. | Declare `environment`; approve non-local interactively. |
| `MIGRATION_STATUS_UNAVAILABLE` | Live status could not be read. | Check DB connectivity. |
| `MIGRATION_PREVIEW_UNSUPPORTED` | Preview is unsupported. | Configure a preview command. |
| `MIGRATION_LOCK_CONFLICT` | The database lock is held. | Wait; it is never released for you. |
| `MIGRATION_FAILED` | A changeset failed. | Fix forward; no rollback is claimed. |
| `MIGRATION_TIMEOUT` | Timed out. | Raise `timeout_seconds`. |
| `MIGRATION_CANCELLED` | Cancelled; **no rollback guarantee**. | Inspect the database state. |
| `MIGRATION_PROCESS_FAILED` / `MIGRATION_MISSING_PARAMETER` / `MIGRATION_INVALID_PARAMETER` | Bad request/tool failure. | Fix it. |
| `MIGRATION_INTERNAL_ERROR` | Internal error. | Retry; report if it persists. |

## Health

See [`health.md`](health.md) §6 for `HEALTH_*` codes and `LOG_ERROR`,
`PROCESS_NOT_RUNNING`, `TCP_CONNECTION_REFUSED`, `HTTP_CHECK_FAILED`.

## Task engine

| Code | Meaning | Recommended action |
| --- | --- | --- |
| `TASK_FAILED` | The task failed. | Read the task's logs. |
| `TASK_TIMEOUT` | The task exceeded its timeout. | Raise the timeout; check the step. |
| `TASK_CANCELLED` | The task was cancelled. | Rerun when ready. |
| `EXECUTOR_NOT_FOUND` | No executor is registered for the task kind. | Rebuild; this is an internal inconsistency. |
| `EXECUTOR_ERROR` / `TASK_EXECUTOR_PANICKED` | The executor errored/panicked. | Retry; report if it persists. |
| `EXECUTION_SPEC_MISSING` | The task carried no specification. | Internal; report it. |
| `INVALID_TASK_TRANSITION` | An illegal task state transition was attempted. | Internal; report it. |

## JSON-RPC protocol errors (MCP)

These are not envelopes; they are reserved for requests that could not be routed:
`METHOD_NOT_FOUND` (unknown tool), `invalid_params` (wrong shape or bad prompt
arguments), `internal_error` (serialization failure), and `resource not found`
(bad resource URI/id). See [`mcp.md`](mcp.md) §2.
