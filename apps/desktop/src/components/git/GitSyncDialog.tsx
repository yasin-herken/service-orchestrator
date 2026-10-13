import { useState } from 'react';

import { Dialog } from '../Dialog';
import { ErrorDetailsPanel } from '../tasks/ErrorDetailsPanel';
import type {
  DesktopError,
  GitSyncMode,
  GitSyncPreview,
  RepositorySummary,
} from '../../types/models';

/** The modes offered by the bulk synchronization dialog. */
const MODES: { value: GitSyncMode; label: string }[] = [
  { value: 'sync', label: 'Fetch + checkout + fast-forward pull' },
  { value: 'fetch', label: 'Fetch only' },
  { value: 'checkout', label: 'Checkout only' },
];

/** A short, human reason a preview entry is blocked. */
function entryStatus(entry: GitSyncPreview['entries'][number]): string {
  if (entry.blocked) return 'Blocked';
  if (entry.warnings.length > 0) return 'Warning';
  return 'Ready';
}

/** A badge tone for a preview entry. */
function entryTone(entry: GitSyncPreview['entries'][number]): string {
  if (entry.blocked) return 'negative';
  if (entry.warnings.length > 0) return 'warning';
  return 'positive';
}

/**
 * The bulk Git synchronization dialog: a read-only preview, per-repository
 * warnings, and an explicit execute step.
 *
 * The preview aggregates real status and branch data; the daemon still owns and
 * enforces every safety check at execution time. The dialog never stashes,
 * resets, cleans, or force-checks-out anything (`tasks/020.md` §7, §8, §9).
 */
export function GitSyncDialog({
  workspaceName,
  targets,
  preview,
  previewing,
  previewError,
  onPreview,
  onExecute,
  executing,
  onClose,
}: {
  workspaceName: string;
  targets: RepositorySummary[];
  preview: GitSyncPreview | null;
  previewing: boolean;
  previewError: DesktopError | null;
  onPreview: (branch: string, mode: GitSyncMode) => void;
  onExecute: (branch: string, mode: GitSyncMode) => void;
  executing: boolean;
  onClose: () => void;
}) {
  const [branch, setBranch] = useState(targets[0]?.branch ?? 'main');
  const [mode, setMode] = useState<GitSyncMode>('sync');

  const allBlocked =
    preview !== null && preview.total > 0 && preview.entries.every((entry) => entry.blocked);
  const executeDisabled = executing || preview === null || allBlocked;

  return (
    <Dialog
      title={`Synchronize ${targets.length} repositor${targets.length === 1 ? 'y' : 'ies'}`}
      onClose={executing ? () => undefined : onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose} disabled={executing}>
            Close
          </button>
          <button
            type="button"
            className="button button--primary"
            onClick={() => onExecute(branch, mode)}
            disabled={executeDisabled}
          >
            {executing ? 'Executing…' : 'Execute'}
          </button>
        </>
      }
    >
      <dl className="dialog__facts">
        <div>
          <dt>Workspace</dt>
          <dd>{workspaceName}</dd>
        </div>
        <div>
          <dt>Selected</dt>
          <dd>{targets.length}</dd>
        </div>
        <div className="dialog__facts-wide">
          <dt>Repositories</dt>
          <dd>{targets.map((target) => target.name).join(', ')}</dd>
        </div>
      </dl>

      <div className="sync-form">
        <label className="sync-form__field">
          <span className="toolbar__field-label">Target branch</span>
          <input
            className="input"
            aria-label="Target branch"
            value={branch}
            onChange={(event) => setBranch(event.target.value)}
          />
        </label>
        <label className="sync-form__field">
          <span className="toolbar__field-label">Mode</span>
          <select
            className="input input--select"
            aria-label="Sync mode"
            value={mode}
            onChange={(event) => setMode(event.target.value as GitSyncMode)}
          >
            {MODES.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className="button button--small"
          onClick={() => onPreview(branch, mode)}
          disabled={previewing}
        >
          {previewing ? 'Previewing…' : 'Preview'}
        </button>
      </div>

      {previewError ? (
        <ErrorDetailsPanel errors={[{ code: previewError.code, message: previewError.message }]} />
      ) : null}

      {preview ? (
        <div className="sync-preview" aria-live="polite">
          <p className="dialog__message">
            {preview.ready} ready · {preview.dirty} dirty · {preview.missing_branch} missing
            branch · {preview.diverged} diverged · {preview.unavailable} unavailable
          </p>
          <table className="service-table service-table--compact">
            <caption className="visually-hidden">Synchronization preview</caption>
            <thead>
              <tr>
                <th scope="col">Repository</th>
                <th scope="col">Current branch</th>
                <th scope="col">Target exists</th>
                <th scope="col">Operations</th>
                <th scope="col">Status</th>
                <th scope="col">Warnings</th>
              </tr>
            </thead>
            <tbody>
              {preview.entries.map((entry) => (
                <tr key={entry.repository_id}>
                  <td>{entry.name}</td>
                  <td className="service-table__branch">{entry.current_branch ?? '—'}</td>
                  <td>{entry.target_branch_exists ? entry.target_branch_kind ?? 'yes' : 'no'}</td>
                  <td className="service-table__mono">
                    {entry.expected_operations.join(' → ')}
                  </td>
                  <td>
                    <span className={`badge badge--${entryTone(entry)}`}>{entryStatus(entry)}</span>
                  </td>
                  <td>{entry.warnings.length > 0 ? entry.warnings.join('; ') : '—'}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <p className="dialog__note muted">
            Local changes are never discarded. A repository with uncommitted changes
            is reported and blocked rather than stashed, reset, cleaned, or force
            checked out.
          </p>
        </div>
      ) : (
        <p className="muted">
          Preview the operation to see per-repository warnings before executing.
        </p>
      )}
    </Dialog>
  );
}
