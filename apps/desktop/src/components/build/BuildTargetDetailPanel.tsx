import { EmptyState, ErrorState } from '../StateViews';
import { BuildCapabilitiesList, exposedOperations } from './BuildCapabilitiesList';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type { BuildTargetSummary } from '../../types/models';
import { canBuild } from './BuildTargetTable';

/**
 * The detail panel for one build target: the full provider capability list with
 * the Application Core's own explanations, runtime requirements, the real last
 * operation, and a build action (`tasks/020.md` §12–§15).
 */
export function BuildTargetDetailPanel({
  target,
  lastTask,
  busy,
  disabled,
  onClose,
  onBuild,
}: {
  target: BuildTargetSummary | null;
  lastTask: TrackedTask | null;
  busy: boolean;
  disabled: boolean;
  onClose: () => void;
  onBuild: (target: BuildTargetSummary) => void;
}) {
  if (!target) {
    return (
      <section className="card detail-panel" aria-label="Build target detail">
        <div className="card__header">
          <h2 className="card__title">Build target detail</h2>
          <button type="button" className="button button--small" onClick={onClose}>
            Close
          </button>
        </div>
        <EmptyState title="Select a service to see its build capabilities." />
      </section>
    );
  }

  const exposed = exposedOperations(target.capability);
  const buildable = canBuild(target);

  return (
    <section className="card detail-panel" aria-label="Build target detail">
      <div className="card__header">
        <h2 className="card__title">Build target detail · {target.display_name}</h2>
        <button type="button" className="button button--small" onClick={onClose}>
          Close
        </button>
      </div>

      <div className="detail-actions" role="group" aria-label="Build actions">
        <button
          type="button"
          className="button button--small button--primary"
          onClick={() => onBuild(target)}
          disabled={disabled || busy || !buildable}
          title={
            buildable
              ? undefined
              : target.capability_error ??
                'the application core exposes no build operation for this service'
          }
        >
          Build
        </button>
        {busy ? <span className="muted">Working…</span> : null}
      </div>

      <dl className="detail-grid">
        <div>
          <dt>Service id</dt>
          <dd className="detail-grid__mono">{target.service_id}</dd>
        </div>
        <div>
          <dt>Workspace</dt>
          <dd>{target.workspace_id}</dd>
        </div>
        <div>
          <dt>Type</dt>
          <dd>{target.kind}</dd>
        </div>
        <div>
          <dt>Build system</dt>
          <dd className="detail-grid__mono">{target.build_system ?? '—'}</dd>
        </div>
        <div>
          <dt>Last operation</dt>
          <dd>{lastTask ? `${lastTask.kind} · ${lastTask.status}` : '—'}</dd>
        </div>
        <div>
          <dt>Exposed operations</dt>
          <dd>{exposed.length > 0 ? exposed.length : 'None'}</dd>
        </div>
      </dl>

      <section className="detail-panel__block" aria-label="Runtime requirements">
        <h3 className="detail-panel__block-title">Runtimes</h3>
        {target.runtimes.length === 0 ? (
          <p className="muted">None declared.</p>
        ) : (
          <ul className="tag-list">
            {target.runtimes.map((runtime) => (
              <li key={`${runtime.kind}:${runtime.version ?? ''}`} className="tag">
                {runtime.kind}
                {runtime.version ? ` ${runtime.version}` : ''}
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="detail-panel__block" aria-label="Build capabilities">
        <h3 className="detail-panel__block-title">Build capabilities</h3>
        {target.capability_error ? (
          <ErrorState
            error={{ code: 'BUILD_CAPABILITIES_UNAVAILABLE', message: target.capability_error }}
          />
        ) : target.capability ? (
          <BuildCapabilitiesList capability={target.capability} />
        ) : (
          <p className="muted">No build system is configured for this service.</p>
        )}
      </section>

      <section className="detail-panel__block" aria-label="Execution scope">
        <h3 className="detail-panel__block-title">Execution</h3>
        <p className="muted">
          Builds run through the shared daemon: the daemon owns concurrency,
          dependency ordering, timeouts, and the structured result. The GUI
          submits one coordinated request, never an uncoordinated loop.
        </p>
      </section>
    </section>
  );
}
