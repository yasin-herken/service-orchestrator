import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { desktopApi } from '../../services/desktopApi';
import { toDesktopError } from '../../services/errors';
import type { DesktopError, LogEntryView } from '../../types/models';

/** The largest number of log entries the viewer retains in memory. */
export const MAX_LOG_BUFFER = 2_000;

/** How many entries are requested per page. */
const PAGE_LIMIT = 500;

/** How often follow mode polls for new entries, in milliseconds. */
const FOLLOW_INTERVAL_MS = 1_000;

/** The severity floor options offered by the viewer. */
const LEVELS = ['all', 'info', 'warn', 'error'] as const;

/** Keeps only the newest `MAX_LOG_BUFFER` entries. */
function bounded(entries: LogEntryView[]): LogEntryView[] {
  return entries.length <= MAX_LOG_BUFFER
    ? entries
    : entries.slice(entries.length - MAX_LOG_BUFFER);
}

/** Appends `incoming` after `existing`, de-duplicating by sequence. */
function append(existing: LogEntryView[], incoming: LogEntryView[]): LogEntryView[] {
  const lastSequence = existing.length > 0 ? existing[existing.length - 1].sequence : -1;
  const fresh = incoming.filter((entry) => entry.sequence > lastSequence);
  return bounded([...existing, ...fresh]);
}

/**
 * A bounded, followable viewer for a task's structured logs.
 *
 * It reads the existing Observability API through the adapter, keeps a bounded
 * buffer, and distinguishes stdout from stderr by text and style — never by
 * color alone (`tasks/020.md` §18, §35, §36). Follow mode polls for new entries
 * with a stable sequence cursor; it never buffers unlimited output in React.
 */
export function LogViewer({
  taskId,
  live = false,
}: {
  taskId: string;
  /** Whether the task is still active (enables follow polling). */
  live?: boolean;
}) {
  const [entries, setEntries] = useState<LogEntryView[]>([]);
  const [following, setFollowing] = useState(live);
  const [level, setLevel] = useState<(typeof LEVELS)[number]>('all');
  const [search, setSearch] = useState('');
  const [error, setError] = useState<DesktopError | null>(null);
  const [loading, setLoading] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);
  // The pagination cursor lives in a ref, so `load` is stable across polls and
  // the follow interval is never torn down and recreated on every fetch.
  const cursorRef = useRef<number | null>(null);
  const disposedRef = useRef(false);

  const load = useCallback(
    async (reset: boolean) => {
      try {
        const page = await desktopApi.getTaskLogs(taskId, {
          ...(reset || cursorRef.current === null ? {} : { after: cursorRef.current }),
          limit: PAGE_LIMIT,
          minimumLevel: level === 'all' ? undefined : level,
          text: search.trim() || undefined,
        });
        if (disposedRef.current) {
          return;
        }
        setError(null);
        setEntries((current) => (reset ? bounded(page.entries) : append(current, page.entries)));
        cursorRef.current = page.next_cursor;
      } catch (cause) {
        if (!disposedRef.current) {
          setError(toDesktopError(cause));
        }
      } finally {
        if (!disposedRef.current) {
          setLoading(false);
        }
      }
    },
    [taskId, level, search],
  );

  // Reload from the start whenever the filters change.
  useEffect(() => {
    disposedRef.current = false;
    cursorRef.current = null;
    setEntries([]);
    setLoading(true);
    void load(true);
    return () => {
      disposedRef.current = true;
    };
  }, [load]);

  // Follow mode: poll for new entries while enabled.
  useEffect(() => {
    if (!following) {
      return undefined;
    }
    const timer = setInterval(() => {
      void load(false);
    }, FOLLOW_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [following, load]);

  // Keep the newest entry in view while following.
  useEffect(() => {
    if (following && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [entries, following]);

  const errorCount = useMemo(
    () => entries.filter((entry) => entry.level === 'error').length,
    [entries],
  );

  return (
    <div className="log-viewer">
      <div className="log-viewer__toolbar" role="group" aria-label="Log controls">
        <label className="log-viewer__field">
          <span className="visually-hidden">Filter logs by text</span>
          <input
            type="search"
            className="input"
            placeholder="Search logs…"
            aria-label="Search logs"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
        <label className="log-viewer__field">
          <span className="log-viewer__field-label">Level</span>
          <select
            className="input input--select"
            aria-label="Filter by log level"
            value={level}
            onChange={(event) => setLevel(event.target.value as (typeof LEVELS)[number])}
          >
            {LEVELS.map((option) => (
              <option key={option} value={option}>
                {option === 'all' ? 'All levels' : option}
              </option>
            ))}
          </select>
        </label>
        <label className="log-viewer__toggle">
          <input
            type="checkbox"
            checked={following}
            onChange={(event) => setFollowing(event.target.checked)}
          />
          <span>Follow</span>
        </label>
        <span className="log-viewer__count" aria-live="polite">
          {entries.length} entries
          {errorCount > 0 ? ` · ${errorCount} error${errorCount === 1 ? '' : 's'}` : ''}
        </span>
      </div>

      {error ? (
        <div className="state-block state-block--error" role="alert">
          <p className="state-block__message">{error.message}</p>
          <p className="state-block__code">{error.code}</p>
          <button type="button" className="button button--small" onClick={() => void load(true)}>
            Retry
          </button>
        </div>
      ) : (
        <div className="log-viewer__output" role="log" ref={containerRef}>
          {loading && entries.length === 0 ? (
            <p className="muted">Loading logs…</p>
          ) : entries.length === 0 ? (
            <p className="muted">No log output yet.</p>
          ) : (
            entries.map((entry) => (
              <div
                key={entry.sequence}
                className={`log-viewer__entry log-viewer__entry--${entry.level}`}
              >
                <span className="log-viewer__stream">{entry.stream ?? entry.source}</span>
                <span className="log-viewer__level">{entry.level}</span>
                <span className="log-viewer__message">{entry.message}</span>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
