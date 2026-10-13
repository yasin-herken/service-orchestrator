# Logs and Diagnostics

Service Orchestrator keeps **one** structured, bounded log store shared through
the daemon. It unifies managed-process output, task-engine events, and
application messages, and every interface reads the same entries. The design is
in [`observability.md`](observability.md).

## 1. Log sources and levels

Each entry carries a `source` and a `level`:

| Source | Produced by |
| --- | --- |
| `process` | Managed service stdout/stderr. |
| `task` | Task-engine events. |
| `runtime` | The service runtime adapter. |
| `git` | The Git adapter. |
| `build` | The build adapter. |
| `library` | Library orchestration. |
| `migration` | The migration adapter. |
| `application` | The application core. |

Levels, from most to least severe: `error`, `warn`, `info`, `debug`, `trace`.
Captured process output has no level, so it is recorded as `info`.

Entries correlate to a workspace, a service, a task, and/or a process id when
known — a process line resolves its service through the process manager's
registry, and a task event inherits the service cached when the task was created.
Missing metadata is left absent; it is never invented.

## 2. Viewing logs

| Scope | TUI | GUI | MCP |
| --- | --- | --- | --- |
| One service | Services → `l`, or the Logs screen | Logs → scope: One service | `get_service_logs { serviceId, … }` |
| One task | Tasks → `Enter`/`l` | Tasks → detail; Logs → scope: One task | `get_task_logs { taskId, … }` |
| Whole workspace | Logs screen | Logs → scope: Whole workspace | `get_logs { workspaceId, … }` |

Filters: `level` (a minimum severity), `source`, and a case-insensitive `text`
substring. `after` is a stable cursor for paging.

**TUI Logs screen keys:** `f` toggles follow, `c` clears the buffer, `v` cycles
the minimum level, `↑`/`↓` scroll (which pauses follow). The view holds at most
**2000** entries.

## 3. Realtime vs historical

- **Service logs stream in realtime** in every interface: subscribing to a
  service opens a bounded subscription, and entries arrive as they are produced.
  The subscription is *lossy* under backpressure: a slow subscriber drops only
  its own entries and observes a `Lagged { dropped }` signal; it never blocks the
  managed process's stdout/stderr reader.
- **Historical logs** are queried as **newest-first bounded pages**, or as
  **oldest-first cursor pages** (`after_sequence`) for stable follow.
- The **GUI** streams only service logs; the workspace and task scopes page the
  cursor once a second as an explicit fallback.

## 4. Retention and limits

The log store is **in memory** and bounded:

| Bound | Default |
| --- | --- |
| Entries per service | 2000 |
| Total entries | 20,000 |
| Maximum message length | 4096 characters (truncated with `…`) |
| Query page size | 200 default, 1000 hard maximum |
| Subscriber channel capacity | 1024 |

Filtering is a bounded linear scan, not full-text search. Logs are lost when the
daemon restarts.

The **auto-started daemon's** own stdout/stderr are written to
`<config_dir>/run/daemon.log` (owner-only, `0600`), which is useful when the
daemon fails to start ([`troubleshooting.md`](troubleshooting.md)).

## 5. Secret redaction

Messages are redacted **before storage**. The shared rule masks:

- secret-looking `key=value` pairs (`password=`, `token=`, `api_key=`,
  `authorization=`, …), including `--flag=value` forms;
- credential-bearing URLs (`scheme://user:pass@host`);
- `Bearer`/`Basic` scheme tokens.

Environment values marked `secret = true` render as `***` in the domain and
never reach a log, task result, or error.

**Redaction is best-effort.** It catches obvious shapes; an unrecognized secret
shape in free text may survive. Preventing a secret from entering a log in the
first place is preferred ([`security.md`](security.md) §7).

## 6. Diagnostics

`get_service_diagnostics` returns a structured report for a service, combining:

- the runtime state and the managed process;
- the latest health result;
- recent error logs;
- the declared ports.

Diagnostic codes include `PROCESS_NOT_RUNNING`, `TCP_CONNECTION_REFUSED`,
`HTTP_CHECK_FAILED`, and `LOG_ERROR`. Diagnostics **report observed facts** and
never fabricate a root cause.

Common tooling: the **GUI** service detail panel and the **TUI** service detail
view surface diagnostics; **MCP** exposes `get_service_diagnostics`, and the
`debug-service` prompt guides an agent through status → runtime → health →
diagnostics → logs.

## 7. Limitations

- Logs are in memory only; restarting the daemon loses them.
- Realtime drops are possible under backpressure and are reported, not hidden.
- There is no full-text search or external log sink.
- The GUI cannot report a daemon-side dropped counter for log/task streams
  ([`known-limitations.md`](known-limitations.md)).
- Redaction is best-effort.
