import { useMemo, useState, type ReactNode } from 'react';

import { Dialog } from '../Dialog';
import { ErrorState, LoadingState } from '../StateViews';
import type { DesktopError } from '../../types/models';

/** The largest number of preview lines rendered before truncation. */
export const MAX_PREVIEW_LINES = 2_000;

/** Renders `line` with every case-insensitive occurrence of `needle` marked. */
function highlight(line: string, needle: string): ReactNode {
  if (needle.length === 0) {
    return line;
  }
  const lower = line.toLowerCase();
  const target = needle.toLowerCase();
  const parts: ReactNode[] = [];
  let index = 0;
  let key = 0;
  for (;;) {
    const found = lower.indexOf(target, index);
    if (found === -1) {
      break;
    }
    if (found > index) {
      parts.push(line.slice(index, found));
    }
    parts.push(<mark key={key}>{line.slice(found, found + needle.length)}</mark>);
    key += 1;
    index = found + needle.length;
  }
  parts.push(line.slice(index));
  return parts;
}

/**
 * The read-only SQL preview dialog (`tasks/021.md` §7).
 *
 * The SQL comes from the daemon's preview task logs; this dialog only renders
 * it. It never executes a statement, renders no credential, bounds its output
 * with an explicit truncation indicator, and warns that the preview may go
 * stale as soon as the database changes.
 */
export function SqlPreviewDialog({
  title,
  sql,
  loading,
  error,
  onClose,
}: {
  title: string;
  /** The preview SQL exactly as the daemon reported it. */
  sql: string;
  loading: boolean;
  error: DesktopError | null;
  onClose: () => void;
}) {
  const [search, setSearch] = useState('');
  const [wrap, setWrap] = useState(true);
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'unavailable'>('idle');

  const lines = useMemo(() => sql.split('\n'), [sql]);
  const truncated = lines.length > MAX_PREVIEW_LINES;
  const visible = truncated ? lines.slice(0, MAX_PREVIEW_LINES) : lines;
  const needle = search.trim();
  const matches = useMemo(() => {
    if (needle.length === 0) {
      return 0;
    }
    const target = needle.toLowerCase();
    return visible.reduce(
      (total, line) => total + line.toLowerCase().split(target).length - 1,
      0,
    );
  }, [visible, needle]);

  const copy = async () => {
    try {
      if (navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(sql);
        setCopyState('copied');
        return;
      }
      setCopyState('unavailable');
    } catch {
      setCopyState('unavailable');
    }
  };

  return (
    <Dialog
      title={title}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            Close
          </button>
          <button
            type="button"
            className="button button--primary"
            onClick={() => void copy()}
            disabled={loading || sql.length === 0}
          >
            {copyState === 'copied' ? 'Copied' : 'Copy'}
          </button>
        </>
      }
    >
      <div className="sql-preview" role="region" aria-label="SQL preview">
        <div className="sql-preview__toolbar" role="group" aria-label="Preview controls">
          <label className="log-viewer__field">
            <span className="visually-hidden">Search the preview</span>
            <input
              type="search"
              className="input"
              placeholder="Search SQL…"
              aria-label="Search SQL"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </label>
          <span className="log-viewer__count" aria-live="polite">
            {needle.length === 0
              ? `${visible.length} lines`
              : `${matches} match${matches === 1 ? '' : 'es'}`}
          </span>
          <label className="log-viewer__toggle">
            <input
              type="checkbox"
              checked={wrap}
              onChange={(event) => setWrap(event.target.checked)}
            />
            <span>Wrap long lines</span>
          </label>
          {copyState === 'unavailable' ? (
            <span className="muted" role="status">
              Copying is unavailable in this environment; select the text manually.
            </span>
          ) : null}
        </div>

        {loading ? <LoadingState label="Generating SQL preview…" /> : null}
        {error ? <ErrorState error={error} /> : null}

        {!loading && !error ? (
          <>
            <pre
              className={`sql-preview__code${wrap ? ' sql-preview__code--wrap' : ''}`}
              role="log"
              aria-label="Preview SQL"
            >
              {visible.map((line, index) => (
                <span key={index} className="sql-preview__line">
                  {highlight(line, needle)}
                  {'\n'}
                </span>
              ))}
            </pre>
            {truncated ? (
              <p className="dialog__note" role="status">
                Output truncated: showing the first {MAX_PREVIEW_LINES} of{' '}
                {lines.length} lines. The preview is read-only and bounded.
              </p>
            ) : null}
          </>
        ) : null}

        <p className="dialog__note muted">
          This preview is read-only: no statement has been executed. It reflects
          the database state at generation time and may be stale if the target
          database changes before you apply.
        </p>
      </div>
    </Dialog>
  );
}
