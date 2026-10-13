import { useState } from 'react';

import { EmptyState, ErrorState, LoadingState } from '../StateViews';
import { GitBadge } from '../StatusBadge';
import { ErrorDetailsPanel } from '../tasks/ErrorDetailsPanel';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type {
  BranchView,
  DesktopError,
  RepositoryInspectionView,
  RepositorySummary,
} from '../../types/models';

/** A display label for a checkout lifecycle value. */
function lifecycleLabel(lifecycle: string): string {
  return lifecycle.replace(/_/g, ' ');
}

/**
 * The detail panel for one repository.
 *
 * It shows configured facts plus an on-demand working-tree inspection (branch,
 * commit, upstream, ahead/behind, staged/unstaged/untracked, conflicts), the
 * local and remote branches, and the real last operation. The inspection is
 * fetched only when it is needed, never by scanning every repository on every
 * render (`tasks/020.md` §4, §6, §35).
 */
export function RepositoryDetailPanel({
  repository,
  inspection,
  branches,
  inspecting,
  branchesLoading,
  inspectionError,
  branchesError,
  lastTask,
  busy,
  disabled,
  onClose,
  onInspect,
  onLoadBranches,
  onFetch,
  onPull,
  onSync,
  onCheckout,
}: {
  repository: RepositorySummary;
  inspection: RepositoryInspectionView | null;
  branches: BranchView[];
  inspecting: boolean;
  branchesLoading: boolean;
  inspectionError: DesktopError | null;
  branchesError: DesktopError | null;
  lastTask: TrackedTask | null;
  busy: boolean;
  disabled: boolean;
  onClose: () => void;
  onInspect: () => void;
  onLoadBranches: () => void;
  onFetch: () => void;
  onPull: () => void;
  onSync: () => void;
  onCheckout: (branch: string) => void;
}) {
  const actionsDisabled = disabled || busy;
  const [branch, setBranch] = useState('');
  const status = inspection?.status ?? null;
  const currentBranch = status?.branch ?? repository.branch ?? '';
  const branchOptions = branches
    .filter((candidate) => candidate.kind === 'local')
    .map((candidate) => candidate.name);
  const checkoutDisabled =
    actionsDisabled || branch === '' || branch === currentBranch;

  return (
    <section className="card detail-panel" aria-label="Repository detail">
      <div className="card__header">
        <h2 className="card__title">Repository detail · {repository.name}</h2>
        <button type="button" className="button button--small" onClick={onClose}>
          Close
        </button>
      </div>

      <div className="detail-actions" role="group" aria-label="Repository actions">
        <button
          type="button"
          className="button button--small"
          onClick={onInspect}
          disabled={disabled || inspecting}
        >
          {inspecting ? 'Inspecting…' : 'Inspect status'}
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onLoadBranches}
          disabled={disabled || branchesLoading}
        >
          {branchesLoading ? 'Loading…' : 'List branches'}
        </button>
        <button
          type="button"
          className="button button--small button--primary"
          onClick={onFetch}
          disabled={actionsDisabled}
        >
          Fetch
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onPull}
          disabled={actionsDisabled}
        >
          Pull (fast-forward)
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onSync}
          disabled={actionsDisabled}
        >
          Sync
        </button>
        {busy ? <span className="muted">Working…</span> : null}
      </div>

      <div className="detail-panel__state">
        <span className="detail-panel__state-item">
          Git <GitBadge state={repository.git_state} />
        </span>
      </div>

      <dl className="detail-grid">
        <div>
          <dt>Repository id</dt>
          <dd className="detail-grid__mono">{repository.id}</dd>
        </div>
        <div>
          <dt>Service id</dt>
          <dd className="detail-grid__mono">{repository.service_id}</dd>
        </div>
        <div>
          <dt>Workspace</dt>
          <dd>{repository.workspace_id}</dd>
        </div>
        <div>
          <dt>Path</dt>
          <dd className="detail-grid__mono">{repository.path ?? '—'}</dd>
        </div>
        <div>
          <dt>Provider</dt>
          <dd>{repository.repository?.provider ?? '—'}</dd>
        </div>
        <div>
          <dt>Default branch</dt>
          <dd className="detail-grid__mono">
            {repository.repository?.default_branch ?? '—'}
          </dd>
        </div>
        <div>
          <dt>Last operation</dt>
          <dd>{lastTask ? `${lastTask.kind} · ${lastTask.status}` : '—'}</dd>
        </div>
        <div className="detail-grid__wide">
          <dt>Clone URL</dt>
          <dd className="detail-grid__mono">{repository.repository?.url ?? '—'}</dd>
        </div>
      </dl>

      <section className="detail-panel__block" aria-label="Working tree">
        <h3 className="detail-panel__block-title">Working tree</h3>
        {inspectionError ? (
          <ErrorState error={inspectionError} onRetry={onInspect} />
        ) : inspecting && !inspection ? (
          <LoadingState label="Inspecting working tree…" />
        ) : !inspection ? (
          <p className="muted">Not inspected yet.</p>
        ) : inspection.error ? (
          <ErrorDetailsPanel
            errors={[
              {
                code: inspection.error.code,
                message: inspection.error.message,
              },
            ]}
          />
        ) : status ? (
          <>
            <dl className="detail-grid">
              <div>
                <dt>Lifecycle</dt>
                <dd>{lifecycleLabel(status.lifecycle)}</dd>
              </div>
              <div>
                <dt>Branch</dt>
                <dd className="detail-grid__mono">{status.branch ?? '—'}</dd>
              </div>
              <div>
                <dt>Commit</dt>
                <dd className="detail-grid__mono">{status.commit ?? '—'}</dd>
              </div>
              <div>
                <dt>Upstream</dt>
                <dd className="detail-grid__mono">{status.upstream ?? '—'}</dd>
              </div>
              <div>
                <dt>Ahead / behind</dt>
                <dd>
                  {status.ahead ?? '—'} / {status.behind ?? '—'}
                </dd>
              </div>
              <div>
                <dt>Staged</dt>
                <dd>{status.staged_changes}</dd>
              </div>
              <div>
                <dt>Unstaged</dt>
                <dd>{status.unstaged_changes}</dd>
              </div>
              <div>
                <dt>Untracked</dt>
                <dd>{status.untracked_files}</dd>
              </div>
              <div>
                <dt>Conflicts</dt>
                <dd>{status.conflicts}</dd>
              </div>
              <div>
                <dt>Working tree</dt>
                <dd>
                  {status.clean ? 'Clean' : 'Has local changes'}
                  {status.diverged ? ' · diverged' : ''}
                  {status.detached_head ? ' · detached HEAD' : ''}
                  {status.unborn ? ' · unborn' : ''}
                </dd>
              </div>
            </dl>
            {status.has_local_changes ? (
              <p className="dialog__note muted" role="note">
                This repository has uncommitted local changes. Fetch is safe; a
                checkout or pull that would conflict is blocked or skipped by the
                daemon rather than stashing, resetting, cleaning, or forcing.
              </p>
            ) : null}
          </>
        ) : null}
      </section>

      <section className="detail-panel__block" aria-label="Branches">
        <h3 className="detail-panel__block-title">Branches</h3>
        {branchesError ? (
          <ErrorState error={branchesError} onRetry={onLoadBranches} />
        ) : branchesLoading && branches.length === 0 ? (
          <LoadingState label="Loading branches…" />
        ) : branches.length === 0 ? (
          <EmptyState title="No branches loaded." description="List branches to inspect and check out a branch." />
        ) : (
          <ul className="branch-list">
            {branches.map((entry) => (
              <li key={`${entry.kind}:${entry.name}`} className="branch-list__item">
                <span className="branch-list__name">{entry.name}</span>
                <span className="badge badge--neutral">{entry.kind}</span>
                {entry.current ? <span className="badge badge--info">current</span> : null}
                {entry.upstream ? (
                  <span className="muted">→ {entry.upstream}</span>
                ) : null}
              </li>
            ))}
          </ul>
        )}

        <div className="checkout-form">
          <label className="toolbar__field">
            <span className="toolbar__field-label">Checkout branch</span>
            <select
              className="input input--select"
              aria-label="Checkout branch"
              value={branch}
              disabled={actionsDisabled || branchOptions.length === 0}
              onChange={(event) => setBranch(event.target.value)}
            >
              <option value="">Select a branch…</option>
              {branchOptions.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </select>
          </label>
          <button
            type="button"
            className="button button--small"
            onClick={() => onCheckout(branch)}
            disabled={checkoutDisabled}
          >
            Checkout
          </button>
        </div>
        <p className="dialog__note muted">
          Checking out never discards local changes; the daemon refuses an unsafe
          checkout instead.
        </p>
      </section>
    </section>
  );
}
