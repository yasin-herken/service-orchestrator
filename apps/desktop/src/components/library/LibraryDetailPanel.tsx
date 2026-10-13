import { useState } from 'react';

import { EmptyState, ErrorState, LoadingState } from '../StateViews';
import { GitBadge } from '../StatusBadge';
import { ErrorDetailsPanel } from '../tasks/ErrorDetailsPanel';
import { VersionAlignmentPanel } from './VersionAlignmentPanel';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type {
  ArtifactVerificationView,
  DesktopError,
  LibraryDependentView,
  LibrarySummary,
  VersionAlignmentView,
} from '../../types/models';

/**
 * The detail panel for one library.
 *
 * It shows configuration, artifact coordinates (one or many), local artifact
 * availability, dependents, and consumer version alignment — all from real
 * Application Core data. It never rewrites a POM and never forces a checkout
 * (`tasks/020.md` §20–§26).
 */
export function LibraryDetailPanel({
  library,
  lastTask,
  busy,
  disabled,
  dependents,
  dependentsLoading,
  dependentsError,
  verification,
  verifying,
  verificationError,
  alignment,
  aligning,
  alignmentError,
  onClose,
  onBuild,
  onInstall,
  onSync,
  onPrepare,
  onLoadDependents,
  onVerify,
  onInspectAlignment,
}: {
  library: LibrarySummary | null;
  lastTask: TrackedTask | null;
  busy: boolean;
  disabled: boolean;
  dependents: LibraryDependentView[] | null;
  dependentsLoading: boolean;
  dependentsError: DesktopError | null;
  verification: ArtifactVerificationView | null;
  verifying: boolean;
  verificationError: DesktopError | null;
  alignment: VersionAlignmentView | null;
  aligning: boolean;
  alignmentError: DesktopError | null;
  onClose: () => void;
  onBuild: () => void;
  onInstall: () => void;
  onSync: () => void;
  onPrepare: () => void;
  onLoadDependents: () => void;
  onVerify: () => void;
  onInspectAlignment: () => void;
}) {
  const actionsDisabled = disabled || busy;
  const [showArtifacts, setShowArtifacts] = useState(false);

  if (!library) {
    return (
      <section className="card detail-panel" aria-label="Library detail">
        <div className="card__header">
          <h2 className="card__title">Library detail</h2>
          <button type="button" className="button button--small" onClick={onClose}>
            Close
          </button>
        </div>
        <EmptyState title="Select a library to see its details." />
      </section>
    );
  }

  return (
    <section className="card detail-panel" aria-label="Library detail">
      <div className="card__header">
        <h2 className="card__title">Library detail · {library.display_name}</h2>
        <button type="button" className="button button--small" onClick={onClose}>
          Close
        </button>
      </div>

      <div className="detail-actions" role="group" aria-label="Library actions">
        <button
          type="button"
          className="button button--small button--primary"
          onClick={onBuild}
          disabled={actionsDisabled}
        >
          Build
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onInstall}
          disabled={actionsDisabled}
        >
          Install locally
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onSync}
          disabled={actionsDisabled}
        >
          Sync repository
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onPrepare}
          disabled={actionsDisabled}
        >
          Prepare…
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onVerify}
          disabled={disabled || verifying}
        >
          {verifying ? 'Verifying…' : 'Verify artifact'}
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onInspectAlignment}
          disabled={disabled || aligning}
        >
          {aligning ? 'Inspecting…' : 'Inspect alignment'}
        </button>
        {busy ? <span className="muted">Working…</span> : null}
      </div>

      <div className="detail-panel__state">
        <span className="detail-panel__state-item">
          Git <GitBadge state={library.git_state} />
        </span>
      </div>

      <dl className="detail-grid">
        <div>
          <dt>Library id</dt>
          <dd className="detail-grid__mono">{library.id}</dd>
        </div>
        <div>
          <dt>Workspace</dt>
          <dd>{library.workspace_id}</dd>
        </div>
        <div>
          <dt>Build system</dt>
          <dd>{library.build_system ?? '—'}</dd>
        </div>
        <div>
          <dt>Branch</dt>
          <dd className="detail-grid__mono">{library.branch ?? '—'}</dd>
        </div>
        <div>
          <dt>Commit</dt>
          <dd className="detail-grid__mono">{library.commit ?? 'Not inspected'}</dd>
        </div>
        <div>
          <dt>Configured version</dt>
          <dd className="detail-grid__mono">{library.configured_version ?? '—'}</dd>
        </div>
        <div>
          <dt>Last operation</dt>
          <dd>{lastTask ? `${lastTask.kind} · ${lastTask.status}` : '—'}</dd>
        </div>
        <div className="detail-grid__wide">
          <dt>Repository</dt>
          <dd className="detail-grid__mono">{library.repository_url ?? '—'}</dd>
        </div>
      </dl>

      {library.repository_error ? (
        <section className="detail-panel__block" aria-label="Repository error">
          <h3 className="detail-panel__block-title">Repository</h3>
          <p className="muted">{library.repository_error}</p>
        </section>
      ) : null}

      <section className="detail-panel__block" aria-label="Artifacts">
        <div className="detail-panel__block-header">
          <h3 className="detail-panel__block-title">Artifacts</h3>
          {library.artifacts.length > 1 ? (
            <button
              type="button"
              className="button button--small"
              onClick={() => setShowArtifacts((value) => !value)}
            >
              {showArtifacts ? 'Hide all' : `Show all ${library.artifacts.length}`}
            </button>
          ) : null}
        </div>
        {library.artifacts.length === 0 ? (
          <p className="muted">No artifact coordinates are configured.</p>
        ) : (
          <table className="service-table service-table--compact">
            <caption className="visually-hidden">Library artifacts</caption>
            <thead>
              <tr>
                <th scope="col">Coordinate</th>
                <th scope="col">Packaging</th>
                <th scope="col">Classifier</th>
              </tr>
            </thead>
            <tbody>
              {(showArtifacts ? library.artifacts : library.artifacts.slice(0, 1)).map(
                (artifact) => (
                  <tr key={artifact.coordinate}>
                    <td className="service-table__mono">{artifact.coordinate}</td>
                    <td>{artifact.packaging}</td>
                    <td>{artifact.classifier ?? '—'}</td>
                  </tr>
                ),
              )}
            </tbody>
          </table>
        )}
      </section>

      <section className="detail-panel__block" aria-label="Local artifact availability">
        <h3 className="detail-panel__block-title">Local artifact availability</h3>
        {verificationError ? (
          <ErrorDetailsPanel
            errors={[{ code: verificationError.code, message: verificationError.message }]}
          />
        ) : verification ? (
          <p>
            <span className={`badge badge--${verification.present ? 'positive' : 'warning'}`}>
              {verification.present ? 'Present' : 'Missing'}
            </span>{' '}
            <span className="dialog__code">{verification.artifact.coordinate}</span>
            <span className="muted">
              {' '}
              — {verification.message}
              {verification.path ? ` (${verification.path})` : ''}
            </span>
          </p>
        ) : (
          <p className="muted">Not verified yet.</p>
        )}
      </section>

      <section className="detail-panel__block" aria-label="Dependents">
        <div className="detail-panel__block-header">
          <h3 className="detail-panel__block-title">Dependents</h3>
          <button
            type="button"
            className="button button--small"
            onClick={onLoadDependents}
            disabled={disabled || dependentsLoading}
          >
            {dependentsLoading ? 'Refreshing…' : 'Refresh'}
          </button>
        </div>
        {dependentsError ? (
          <ErrorDetailsPanel
            errors={[{ code: dependentsError.code, message: dependentsError.message }]}
          />
        ) : (
          (dependents ?? library.dependents).length === 0 ? (
            <p className="muted">No services depend on this library.</p>
          ) : (
            <ul className="detail-list">
              {(dependents ?? library.dependents).map((dependent) => (
                <li key={`${dependent.service_id}:${dependent.kind}`}>
                  <span className="detail-grid__mono">{dependent.service_id}</span>
                  <span className="muted">
                    {' '}
                    ({dependent.kind}
                    {dependent.required ? ', required' : ', optional'})
                  </span>
                </li>
              ))}
            </ul>
          )
        )}
      </section>

      <section className="detail-panel__block" aria-label="Alignment inspection">
        <div className="detail-panel__block-header">
          <h3 className="detail-panel__block-title">Version alignment</h3>
          <button
            type="button"
            className="button button--small"
            onClick={onInspectAlignment}
            disabled={disabled || aligning}
          >
            {aligning ? 'Inspecting…' : 'Inspect'}
          </button>
        </div>
        {aligning && !alignment ? (
          <LoadingState label="Inspecting version alignment…" />
        ) : alignmentError ? (
          <ErrorState error={alignmentError} onRetry={onInspectAlignment} />
        ) : alignment ? (
          <VersionAlignmentPanel alignment={alignment} />
        ) : (
          <p className="muted">Not inspected yet.</p>
        )}
      </section>
    </section>
  );
}
