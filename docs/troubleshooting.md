# Troubleshooting

This guide covers the failures a developer is most likely to hit, with diagnosis
steps and fixes. Each section names the interface commands or files involved.

A general first step is always the same:

```bash
service-orchestrator daemon status
```

It reports whether the daemon is running, its endpoint, protocol version,
connected clients, active tasks, and managed services — and it never starts a
daemon. When the daemon is up but a specific operation fails, the error `code` is
the fastest path to the fix ([`error-reference.md`](error-reference.md)).

## Daemon fails to start

**Symptoms:** `tui`/`mcp` fail immediately, or the desktop GUI shows a
disconnected banner.

**Diagnose:**

1. Start the daemon in the foreground and read stderr:
   ```bash
   service-orchestrator daemon --config /path/to/config.toml
   ```
   A configuration error is printed and the process exits non-zero.
2. Read the auto-started daemon's log:
   `<config_dir>/run/daemon.log` (owner-only, `0600`).
3. Check `service-orchestrator daemon status`.

**Causes and fixes:**

- **Invalid configuration** — fix the reported field; unknown keys are rejected
  ([`configuration.md`](configuration.md)).
- **A second daemon for the same runtime directory** — it exits with
  `AlreadyRunning`. This is expected; the running daemon is authoritative.
- **Missing or unwritable runtime directory** — `<config_dir>/run` is created
  `0700`; ensure the config directory is writable.
- **A stale socket file** — not a real problem: once the advisory lock is held, a
  leftover socket is removed and rebound safely.

## `DAEMON_UNAVAILABLE` / IPC socket problems

**Symptoms:** commands fail with `DAEMON_UNAVAILABLE`; `daemon status` says
"stopped" or "running but unreachable".

**Diagnose:**

1. `service-orchestrator daemon status`.
2. Confirm the socket path: `$SERVICE_ORCHESTRATOR_SOCKET` if set, else
   `<config_dir>/run/daemon.sock`.
3. Confirm the client and daemon use the **same** configuration path — the socket
   is derived from it, so a different `--config` means a different daemon.

**Fixes:**

- Start the daemon (or let `tui`/`mcp` auto-start it). Set
  `SERVICE_ORCHESTRATOR_NO_AUTOSTART=1` to make a missing daemon a hard error.
- Point both sides at the same `--config` (or unset
  `SERVICE_ORCHESTRATOR_CONFIG`).
- A request that was **written** but got no response is never retried (the
  mutation may have applied); re-issue it deliberately if appropriate.

## Configuration is invalid

**Symptoms:** the daemon exits with a `ConfigError`, or the GUI's Validate
reports problems.

**Diagnose:** the error names a `location` (for example
`services[2].runtime.version`) and a `problem`. The GUI Configuration screen's
**Validate** action checks the file without changing it.

**Fixes:** correct the field. Common cases: an unknown key (typos are errors,
not ignored), an invalid enum spelling, a missing required field, a `path`
outside the root, a missing `default_branch`, a `frontend` service with no
runtime, or a `java`/`node` runtime with no version
([`configuration.md`](configuration.md) §8).

## Repository is missing or invalid

**Symptoms:** validation reports `missing_service_path`,
`missing_repository`, or `missing_git_metadata`; Git operations fail with
`GIT_NOT_CLONED`.

**Fixes:**

- Ensure the configured `path` exists under the workspace `root`.
- A repository directory must contain Git metadata (`.git`); a plain folder is
  reported as invalid rather than assumed to be a checkout.
- Clone it (`git_clone` over MCP, or manually), or fix `path`/`root`.

## Git authentication fails

**Symptoms:** `GIT_AUTHENTICATION_FAILED` or `GIT_REMOTE_UNAVAILABLE`.

**Fixes:**

- Fix SSH keys or credential helper for the user running the daemon. Your normal
  Git configuration is preserved by design.
- Confirm the remote URL is correct. Credential-bearing URLs must **not** be
  embedded in configuration — authentication comes from the environment.
- Fetch over SSH/HTTPS manually once to confirm the credentials work.

## Dirty working tree blocks a checkout or pull

**Symptoms:** `GIT_WORKING_TREE_DIRTY`.

**Fixes:** commit or stash your changes. Service Orchestrator never silently
stashes, resets, or force-checks-out. If you really want to discard changes, use
a destructive operation deliberately (`d`/`D`/`n` in the TUI, or the MCP
`git_reset_hard`/`git_discard_changes`/`git_clean` tools — the latter are denied
for agents and must be performed interactively)
([`git-operations.md`](git-operations.md) §5).

## Maven is unavailable

**Symptoms:** `missing_build_tool`, `BUILD_COMMAND_FAILED` with "mvn: command not
found", or `ARTIFACT_LOCAL_REPOSITORY_UNKNOWN`.

**Fixes:**

- Prefer the project's Maven Wrapper (`./mvnw`), which needs no global Maven.
- Otherwise install Maven and ensure `mvn` is on the daemon's `PATH`.
- `validate_workspace` reports which tool is missing.

## Java version mismatch

**Symptoms:** `RUNTIME_VERSION_MISMATCH`, or a build fails against an unexpected
JDK (for example Lombok reports `cannot find symbol` for generated members, or
the log starts with `WARNING: A restricted method in java.lang.System`).

**Fixes:**

- Install the runtime version pinned in configuration (`runtime.version`), or
  adjust the pin.
- `RuntimeHost` resolves executables across `PATH`, `JAVA_HOME`, and common
  version managers (`nvm`, `fnm`, Volta, `asdf`, `sdkman`, `jenv`).
- Builds inherit the **daemon's** environment. The service `runtimes` pin is
  checked when a service starts, but it does not select a JDK for Maven or npm.
  If the daemon runs on a newer JDK than the project supports, restart it with
  the matching one, for example:

  ```bash
  export JAVA_HOME=$(/usr/libexec/java_home -v 21)
  service-orchestrator daemon stop
  service-orchestrator daemon
  ```

  A desktop-launched daemon inherits the desktop app's environment; start the
  app from the same shell after exporting `JAVA_HOME`, or set it for GUI apps
  with `launchctl setenv JAVA_HOME "$JAVA_HOME"` and relaunch.

  The JVM warnings themselves are not the failure: the build's `[ERROR]` lines
  are. Task details now prefer those lines over stderr warnings.

## Node.js version mismatch

**Symptoms:** `RUNTIME_VERSION_MISMATCH` or an npm build that rejects the Node
version.

**Fixes:** install the pinned Node version (the project's stated managed-service
version is 20.x), or adjust `runtime.version`.

## npm install failure

**Symptoms:** `BUILD_COMMAND_FAILED` during a frontend build.

**Fixes:**

- Read the build task logs (`get_task_logs` / Tasks screen).
- Check the registry/network and any `.npmrc` configuration.
- Delete a corrupt `node_modules` and rebuild if the failure is a partial
  install.

## Library artifact is missing

**Symptoms:** `ARTIFACT_NOT_FOUND` or a consumer build that cannot resolve a
library.

**Fixes:**

- Install or prepare the library (`install_library` / `prepare_library`).
- Verify the artifact (`verify_library_artifact`).
- Remember that installing a library does **not** change the version a consumer
  depends on; check `inspect_library_version_alignment`
  ([`libraries.md`](libraries.md) §7).

## Liquibase migration failure

**Symptoms:** `MIGRATION_FAILED`, `MIGRATION_LOCK_CONFLICT`, or
`MIGRATION_DATABASE_TARGET_NOT_AUTHORIZED`.

**Fixes:**

- Read the migration logs; a failed migration is **not** rolled back or retried.
- A lock conflict means another process holds the database lock; Service
  Orchestrator never releases it for you.
- A non-local target requires interactive confirmation; an agent's request is
  denied with `CONFIRMATION_REQUIRED`.
- Preview first (`preview_migration`) before applying to a non-local target
  ([`migrations.md`](migrations.md)).

## Port already in use

**Symptoms:** validation warns `port_currently_occupied`, or startup fails with
`RUNTIME_PORT_ALREADY_IN_USE`.

**Fixes:**

- Free the port or change the configured port.
- Service Orchestrator **never kills** the occupying process; find it with your
  usual tooling (`lsof -i :8081`) and stop it yourself if appropriate.
- Two services declaring the same port is a configuration error
  (`duplicate_configured_port`).

## Service starts but is unhealthy

**Symptoms:** the process is `running`, but health is `unhealthy`/`unreachable`.

**Diagnose:** `get_service_diagnostics` (GUI detail / MCP / TUI detail), then the
service's error logs.

**Causes:** a dependency that is not up (database, another service), a wrong
port, a still-starting application, or a startup failure. Use
`check_service_readiness` to wait for serving, and start dependency services
first (the workflow does this automatically)
([`health.md`](health.md)).

## Logs are unavailable

**Symptoms:** the log view is empty.

**Fixes:**

- Logs are **in memory** and are lost on a daemon restart. Exercise the service
  to produce new entries.
- For task output, open the **task** logs, not the service logs.
- Check the filter (minimum level / text) is not too narrow.
- The auto-started daemon's own output is in `<config_dir>/run/daemon.log`.

## GUI cannot connect to the daemon

**Symptoms:** a disconnect banner; commands disabled.

**Fixes:**

- `service-orchestrator daemon status`; start it if needed.
- The GUI's Rust adapter locates the daemon binary in this order: the
  `SERVICE_ORCHESTRATOR_DAEMON_BIN` override, then a sibling of the GUI binary,
  then `<bundle>/Contents/Resources/` (packaged macOS), then `PATH`, then a
  `target/{debug,release}` directory found by walking up from the GUI binary
  (the `npm run tauri dev` layout). The bundled candidate deliberately wins over
  `PATH` so a packaged GUI never picks up an unrelated installed binary. If you
  moved binaries, set `SERVICE_ORCHESTRATOR_DAEMON_BIN`.
- If `daemon status` says a daemon **is** running but the GUI still shows the
  disconnect banner, the running daemon may be an **older build** than the GUI.
  The IPC request envelope carries the client's interface kind, and a daemon
  that predates the GUI's kind cannot parse the request: it drops the connection
  without answering, which the GUI reports as "the daemon closed the connection
  without answering". Stop it and reconnect so a current daemon takes over —
  `service-orchestrator daemon stop`, then click **Retry** (or run `tui`/`mcp`,
  which start a fresh daemon). The GUI deliberately never replaces a running
  daemon on its own (`tasks/031.md` §15).
- The GUI reconnects automatically (it probes every 5 seconds; 5 failed probes
  show "Disconnected"). Click **Retry** or wait for reconnection.

## MCP client cannot connect

**Symptoms:** the client reports the server failed to start, or a tool returns
`DAEMON_UNAVAILABLE`.

**Fixes:**

- The command must be the binary with the `mcp` subcommand, for example
  `{ "command": "/usr/local/bin/service-orchestrator", "args": ["mcp", "--config", "/Users/me/.service-orchestrator/config.toml"] }`.
- **stdout is reserved for the protocol.** If the client reports malformed
  JSON, check that nothing else is printing to stdout; diagnostics go to stderr.
- Destructive tools always return `CONFIRMATION_REQUIRED`; that is by design, not
  a connection problem ([`mcp.md`](mcp.md) §6).

## Getting more help

- [`error-reference.md`](error-reference.md) — every stable error code.
- [`local-daemon.md`](local-daemon.md) — the daemon's lifecycle and IPC.
- [`known-limitations.md`](known-limitations.md) — behaviours that are not bugs.
- [`logs-and-diagnostics.md`](logs-and-diagnostics.md) — reading logs and
  diagnostics.
