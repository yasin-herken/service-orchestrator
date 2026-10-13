import type { BulkSummaryView } from '../../types/models';
import type { WorkflowProgress } from '../../utils/taskProgress';

/** The counts a bulk summary renders, from live tasks or a server aggregate. */
export function summaryCounts(
  progress: WorkflowProgress,
  server?: BulkSummaryView | null,
): WorkflowProgress {
  if (!server) {
    return progress;
  }
  // The server aggregate is authoritative once available; map it onto the
  // same shape the live-tracked counts use.
  const total = server.total;
  const settled = server.succeeded + server.failed + server.cancelled + server.skipped;
  return {
    total,
    running: Math.max(total - settled - server.pending, 0),
    pending: server.pending,
    succeeded: server.succeeded,
    failed: server.failed,
    cancelled: server.cancelled,
    skipped: server.skipped,
    complete: server.complete,
  };
}

/**
 * Aggregated per-target outcomes for a bulk operation.
 *
 * Every count comes from real task-engine data — either the live task stream or
 * the daemon's own workflow aggregate — never a fabricated percentage
 * (`tasks/020.md` §10, §27).
 */
export function BulkOperationSummary({
  progress,
  server,
  targetLabel = 'repositories',
}: {
  progress: WorkflowProgress;
  server?: BulkSummaryView | null;
  targetLabel?: string;
}) {
  const counts = summaryCounts(progress, server);
  const complete = counts.complete;
  return (
    <div className="bulk-summary" role="group" aria-label="Bulk operation summary">
      <p className="bulk-summary__headline" aria-live="polite">
        {complete ? 'Completed' : 'In progress'} — {counts.total}{' '}
        {targetLabel}
      </p>
      <ul className="progress__counts">
        <li>
          Succeeded <strong>{counts.succeeded}</strong>
        </li>
        <li>
          Running <strong>{counts.running}</strong>
        </li>
        <li>
          Pending <strong>{counts.pending}</strong>
        </li>
        <li>
          Failed <strong>{counts.failed}</strong>
        </li>
        {counts.cancelled > 0 ? (
          <li>
            Cancelled <strong>{counts.cancelled}</strong>
          </li>
        ) : null}
        {counts.skipped > 0 ? (
          <li>
            Skipped <strong>{counts.skipped}</strong>
          </li>
        ) : null}
      </ul>
      {!complete && counts.failed === 0 && counts.cancelled === 0 ? (
        <p className="muted">Waiting for the daemon to report every target…</p>
      ) : null}
      {counts.failed > 0 ? (
        <p className="bulk-summary__partial" role="note">
          {counts.failed} {targetLabel} failed. This is a partial result — not
          every target succeeded.
        </p>
      ) : null}
    </div>
  );
}
