import { ConfigurationDiff } from '../configuration/ConfigurationDiff';
import {
  AffectedReferences,
  ConfigurationValidationPanel,
  RunningImpact,
} from '../configuration/ConfigurationValidationPanel';
import { ResolvedEntity } from '../template/ResolvedEntity';
import type { BulkImportPlanView, BulkImportRowView } from '../../types/models';

/** A short label for what the planner will do with a row. */
export function actionLabel(row: BulkImportRowView): string {
  switch (row.action.action) {
    case 'create':
      return 'create';
    case 'update':
      return `replace ${row.action.kind} ${row.action.existing}`;
    case 'skip':
      return `skip (${row.action.kind} ${row.action.existing} exists)`;
    default:
      return 'cannot import';
  }
}

/** A human label for the template source. */
function sourceLabel(row: BulkImportRowView): string {
  switch (row.template_source) {
    case 'explicit':
      return 'explicit';
    case 'matched':
      return 'automatic match';
    default:
      return 'no template';
  }
}

/**
 * The plan review (`tasks/032.md` §10 steps 8–9, §14).
 *
 * Everything shown is the daemon's own plan: per-row actions, template source,
 * assigned ports, structured problems, and the whole-document diff and
 * validation. The GUI never re-decides whether a row is importable.
 */
export function BulkImportReview({
  plan,
  mode = 'preview',
}: {
  plan: BulkImportPlanView;
  /** `validate` lists rows and problems; `preview` adds the diff and proposals. */
  mode?: 'validate' | 'preview';
}) {
  const preview = plan.preview;
  return (
    <section className="import-review" aria-label="Bulk import plan">
      <div className="card__header">
        <div>
          <h3 className="card__title">Validated import plan</h3>
          <p className="muted">
            Revision {plan.revision} · {plan.rows.length} rows · {plan.creates} create ·{' '}
            {plan.updates} replace · {plan.skips} skip · {plan.rejects} rejected
          </p>
        </div>
        <span className={`badge badge--${plan.valid ? 'positive' : 'negative'}`}>
          {plan.valid ? 'valid' : 'not importable'}
        </span>
      </div>

      {(plan.warnings ?? []).length > 0 ? (
        <div className="config-validation" role="status">
          <ul>
            {(plan.warnings ?? []).map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </div>
      ) : null}

      <div className="table-wrap">
        <table className="service-table" aria-label="Import rows">
          <thead>
            <tr>
              <th scope="col">Repository</th>
              <th scope="col">Action</th>
              <th scope="col">Template</th>
              <th scope="col">Entity</th>
              <th scope="col">Ports</th>
              <th scope="col">Problems</th>
            </tr>
          </thead>
          <tbody>
            {plan.rows.map((row) => (
              <tr key={row.relative_path} data-row={row.relative_path}>
                <td>
                  <div className="service-table__name">{row.relative_path}</div>
                  <div className="muted">{row.absolute_path}</div>
                </td>
                <td>
                  <span
                    className={`badge badge--${
                      row.action.action === 'reject'
                        ? 'negative'
                        : row.action.action === 'skip'
                          ? 'neutral'
                          : row.action.action === 'update'
                            ? 'warning'
                            : 'positive'
                    }`}
                  >
                    {actionLabel(row)}
                  </span>
                </td>
                <td>
                  {row.template ?? '—'}
                  <div className="muted">{sourceLabel(row)}</div>
                </td>
                <td>
                  {row.entity_kind ? `${row.entity_kind} ${row.entity_id ?? ''}` : '—'}
                </td>
                <td>{(row.ports ?? []).join(', ') || '—'}</td>
                <td>
                  {(row.errors ?? []).length === 0 && (row.warnings ?? []).length === 0 ? (
                    <span className="muted">none</span>
                  ) : (
                    <ul className="import-problems">
                      {(row.errors ?? []).map((issue) => (
                        <li key={`${issue.code}-${issue.message}`} className="import-problems__error">
                          <strong>{issue.code}</strong>: {issue.message}
                        </li>
                      ))}
                      {(row.warnings ?? []).map((warning) => (
                        <li key={warning}>{warning}</li>
                      ))}
                    </ul>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {preview && mode === 'preview' ? (
        <div className="import-review__preview">
          <h4 className="card__title">Preview of the configuration change</h4>
          <ConfigurationValidationPanel
            revision={preview.proposed_revision}
            valid={preview.valid}
            issues={preview.issues}
          />
          <ConfigurationDiff diff={preview.diff} />
          <AffectedReferences references={preview.affected_references} />
          <RunningImpact impact={preview.running_impact} />
        </div>
      ) : null}

      {mode === 'preview' ? (
        <div className="import-review__proposals">
          <h4 className="card__title">Resolved configurations</h4>
          <p className="muted">
            The exact values the import would write. Expand a row to inspect it.
          </p>
          {plan.rows
            .filter((row) => row.proposal !== undefined)
            .map((row) => (
              <details key={row.relative_path} className="import-row">
                <summary className="import-row__summary">
                  <span className="import-row__path">{row.relative_path}</span>
                  <span className="muted">{actionLabel(row)}</span>
                </summary>
                {row.proposal ? <ResolvedEntity proposal={row.proposal} /> : null}
              </details>
            ))}
        </div>
      ) : null}
    </section>
  );
}
