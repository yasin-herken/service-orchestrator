import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { desktopApi, type LogsQuery } from '../../services/desktopApi';
import { toDesktopError } from '../../services/errors';
import type { DesktopError, LogEntryView, LogStreamEvent } from '../../types/models';

/** The largest number of log entries the viewer keeps in memory. */
export const MAX_LOG_BUFFER = 1_000;

/** How many entries are requested per page (the daemon clamps it anyway). */
const PAGE_LIMIT = 500;

/** How often follow mode polls scopes without a push subscription, in ms. */
const FOLLOW_INTERVAL_MS = 1_000;

/** The severity floor options offered by the viewer. */
const LEVELS = ['all', 'trace', 'debug', 'info', 'warn', 'error'] as const;

/** The source options offered by the viewer (matching the daemon's parser). */
const SOURCES = [
  'all',
  'process',
  'task',
  'runtime',
  'git',
  'build',
  'library',
  'migration',
  'application',
] as const;

/** What the viewer is scoped to. */
export type LogScope =
  | { kind: 'workspace'; workspaceId: string }
  | { kind: 'service'; serviceId: string; workspaceId: string | null }
  | { kind: 'task'; taskId: string; workspaceId: string | null };

/** A stable identity for a scope, safe to use in effect dependency lists. */
export function scopeKey(scope: LogScope): string {
  switch (scope.kind) {
    case 'workspace':
      return `workspace:${scope.workspaceId}`;
    case 'service':
      return `service:${scope.serviceId}`;
    case 'task':
      return `task:${scope.taskId}`;
  }
}

/** Keeps only the newest `MAX_LOG_BUFFER` entries. */
function bounded(entries: LogEntryView[]): LogEntryView[] {
  return entries.length <= MAX_LOG_BUFFER
    ? entries
    : entries.slice(entries.length - MAX_LOG_BUFFER);
}

/** Appends `incoming` after `existing`, de-duplicating by sequence. */
function append(existing: LogEntryView[], incoming: LogEntryView[]): LogEntryView[] {
  if (incoming.length === 0) {
    return existing;
  }
  const lastSequence = existing.length > 0 ? existing[existing.length - 1].sequence : -1;
  const fresh = incoming.filter((entry) => entry.sequence > lastSequence);
  if (fresh.length === 0) {
    return existing;
  }
  return bounded([...existing, ...fresh]);
}

/** Serializes the visible buffer for copying. */
function asText(entries: readonly LogEntryView[]): string {
  return entries
    .map((entry) => {
      const time = new Date(entry.timestamp * 1000).toISOString();
      const stream = entry.stream ? ` ${entry.stream}` : '';
      return `${time} ${entry.level}${stream} ${entry.source}: ${entry.message}`;
    })
    .join('\n');
}

/**
 * The unified, bounded log viewer (`tasks/021.md` §23–§28).
 *
 * It reads the existing Observability layer through the adapter — one call per
 * scope, one push subscription per service — keeps a bounded client-side
 * buffer, follows new entries on request, and reports truncation and dropped
 * events honestly instead of pretending the view is complete. Clearing only
 * clears the visible buffer; stored logs are never touched from React.
 */
export function UnifiedLogViewer({
  scope,
  disabled = false,
}: {
  scope: LogScope;
  /** Disables live follow (used while the daemon is unreachable). */
  disabled?: boolean;
}) {
  const [entries, setEntries] = useState<LogEntryView[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [level, setLevel] = useState<(typeof LEVELS)[number]>('all');
  const [source, setSource] = useState<(typeof SOURCES)[number]>('all');
  const [text, setText] = useState('');
  const [follow, setFollow] = useState(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<DesktopError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'unavailable'>('idle');

  const cursorRef = useRef<number | null>(null);
  const scopeRef = useRef(scope);
  scopeRef.current = scope;
  const disposedRef = useRef(false);
  const subscriptionCounter = useRef(0);
  const containerRef = useRef<HTMLDivElement>(null);

  const key = scopeKey(scope);

  /** Runs one bounded query against the adapter. */
  const load = useCallback(
    async (reset: boolean) => {
      const current = scopeRef.current;
      const query: LogsQuery = {
        limit: PAGE_LIMIT,
        ...(current.kind === 'workspace' ? { workspaceId: current.workspaceId } : {}),
        ...(current.kind === 'service' ? { serviceId: current.serviceId } : {}),
        ...(current.kind === 'task' ? { taskId: current.taskId } : {}),
        ...(level !== 'all' ? { minimumLevel: level } : {}),
        ...(source !== 'all' ? { source } : {}),
        ...(text.trim().length > 0 ? { text: text.trim() } : {}),
        ...(reset || cursorRef.current === null ? {} : { after: cursorRef.current }),
      };
      try {
        const page = await desktopApi.getLogs(query);
        if (disposedRef.current) {
          return;
        }
        setError(null);
        setTruncated(page.truncated);
        setEntries((currentEntries) =>
          reset ? bounded(page.entries) : append(currentEntries, page.entries),
        );
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
    [level, source, text],
  );

  // Reset and reload whenever the scope or a filter changes.
  useEffect(() => {
    disposedRef.current = false;
    cursorRef.current = null;
    setEntries([]);
    setNotice(null);
    setCopyState('idle');
    setLoading(true);
    void load(true);
    return () => {
      disposedRef.current = true;
    };
  }, [key, load]);

  // Realtime service logs: one adapter subscription per visible stream.
  useEffect(() => {
    const current = scopeRef.current;
    if (current.kind !== 'service' || !follow || disabled) {
      return undefined;
    }
    let stopped = false;
    let unlisten: (() => void) | undefined;
    subscriptionCounter.current += 1;
    const subscriptionId = `logs-${current.serviceId}-${subscriptionCounter.current}`;
    const serviceId = current.serviceId;

    const handle = (event: LogStreamEvent) => {
      if (stopped || event.subscription_id !== subscriptionId) {
        return;
      }
      if (event.kind === 'entry' && event.entry) {
        setEntries((current) => append(current, [event.entry as LogEntryView]));
      } else if (event.kind === 'lagged' || event.kind === 'closed') {
        setNotice(
          event.message ??
            (event.dropped
              ? `${event.dropped} entries were dropped; this view may be incomplete.`
              : 'The log stream ended.'),
        );
      }
    };

    void (async () => {
      try {
        await desktopApi.subscribeServiceLogs(subscriptionId, serviceId);
        if (stopped) {
          await desktopApi.unsubscribeServiceLogs(subscriptionId);
          return;
        }
        const stop = await desktopApi.onLogEvent(handle);
        if (stopped) {
          stop();
        } else {
          unlisten = stop;
        }
      } catch (cause) {
        if (!stopped) {
          setError(toDesktopError(cause));
        }
      }
    })();

    return () => {
      stopped = true;
      unlisten?.();
      void desktopApi.unsubscribeServiceLogs(subscriptionId);
    };
  }, [key, follow, disabled]);

  // Scopes without a push subscription follow by polling the cursor.
  const shouldPoll = follow && !disabled && scope.kind !== 'service';
  useEffect(() => {
    if (!shouldPoll) {
      return undefined;
    }
    const timer = setInterval(() => {
      void load(false);
    }, FOLLOW_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [shouldPoll, load]);

  // Keep the newest entry in view while following.
  useEffect(() => {
    if (follow && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [entries, follow]);

  const errorCount = useMemo(
    () => entries.filter((entry) => entry.level === 'error').length,
    [entries],
  );
  const bufferFull = entries.length >= MAX_LOG_BUFFER;

  const copy = async () => {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(asText(entries));
        setCopyState('copied');
        return;
      }
      setCopyState('unavailable');
    } catch {
      setCopyState('unavailable');
    }
  };

  return (
    <div className="log-viewer" role="region" aria-label="Log viewer">
      <div className="log-viewer__toolbar" role="group" aria-label="Log controls">
        <label className="log-viewer__field">
          <span className="visually-hidden">Filter logs by text</span>
          <input
            type="search"
            className="input"
            placeholder="Search logs…"
            aria-label="Search logs"
            value={text}
            onChange={(event) => setText(event.target.value)}
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
        <label className="log-viewer__field">
          <span className="log-viewer__field-label">Source</span>
          <select
            className="input input--select"
            aria-label="Filter by log source"
            value={source}
            onChange={(event) => setSource(event.target.value as (typeof SOURCES)[number])}
          >
            {SOURCES.map((option) => (
              <option key={option} value={option}>
                {option === 'all' ? 'All sources' : option}
              </option>
            ))}
          </select>
        </label>
        <label className="log-viewer__toggle">
          <input
            type="checkbox"
            checked={follow}
            disabled={disabled}
            onChange={(event) => setFollow(event.target.checked)}
          />
          <span>Follow</span>
        </label>
        <button
          type="button"
          className="button button--small"
          onClick={() => void load(false)}
          disabled={loading}
        >
          Fetch newer
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={() => setEntries([])}
          disabled={entries.length === 0}
        >
          Clear view
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={() => void copy()}
          disabled={entries.length === 0}
        >
          {copyState === 'copied' ? 'Copied' : 'Copy'}
        </button>
        <span className="log-viewer__count" aria-live="polite">
          {entries.length} entries
          {errorCount > 0 ? ` · ${errorCount} error${errorCount === 1 ? '' : 's'}` : ''}
          {truncated ? ' · older entries exist beyond this page' : ''}
        </span>
      </div>

      {notice ? (
        <div className="page__notice" role="status">
          {notice}
        </div>
      ) : null}
      {bufferFull ? (
        <div className="page__notice" role="status">
          Buffer limit reached ({MAX_LOG_BUFFER} entries): the oldest lines were
          dropped from this view. Stored logs are untouched.
        </div>
      ) : null}
      {copyState === 'unavailable' ? (
        <div className="page__notice" role="status">
          Copying is unavailable in this environment; select the text manually.
        </div>
      ) : null}

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
            <p className="muted">No log output matches this view.</p>
          ) : (
            entries.map((entry) => (
              <div
                key={entry.sequence}
                className={`log-viewer__entry log-viewer__entry--${entry.level}`}
              >
                <span className="log-viewer__stream">
                  {entry.stream ?? entry.source}
                </span>
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
