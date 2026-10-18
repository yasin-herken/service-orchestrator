import type { DiscoveredRepositoryView } from '../../types/models';
import { StatusBadge } from '../StatusBadge';
import { categoryLabel, confidenceTone } from './RepositorySelection';

/**
 * Detection review (`tasks/032.md` §10 step 4, §8).
 *
 * Every technology is listed with its confidence and evidence, and the
 * automatic-match outcome is stated explicitly. A detection is evidence for a
 * suggestion — never a silent classification the user cannot inspect.
 */
export function DetectionReview({
  repositories,
}: {
  repositories: readonly DiscoveredRepositoryView[];
}) {
  if (repositories.length === 0) {
    return <p className="muted">Select at least one repository to review its technologies.</p>;
  }
  return (
    <section className="import-review" aria-label="Detected technologies">
      <h3 className="card__title">Detected technologies</h3>
      <p className="muted">
        Detection reads well-known project files only. Confidence is explicit, and an
        ambiguous detection never chooses a template on its own.
      </p>
      {repositories.map((entry) => {
        const path = entry.repository.relative_path;
        const ambiguous = entry.ambiguous_categories ?? [];
        return (
          <article key={path} className="import-review__repository">
            <header className="import-review__header">
              <h4>{path}</h4>
              <span className="muted">
                {entry.suggested_template
                  ? `Suggested template: ${entry.suggested_template}`
                  : ambiguous.length > 0
                    ? 'No single category matched'
                    : 'No matching template'}
              </span>
            </header>
            {ambiguous.length > 0 ? (
              <p className="config-validation config-validation--error" role="alert">
                The detections tie between {ambiguous.map(categoryLabel).join(', ')}. Choose a
                template explicitly on the next step.
              </p>
            ) : null}
            {entry.repository.detections.length === 0 ? (
              <p className="muted">No technology was detected in this directory.</p>
            ) : (
              <table className="service-table" aria-label={`Technologies in ${path}`}>
                <thead>
                  <tr>
                    <th scope="col">Technology</th>
                    <th scope="col">Confidence</th>
                    <th scope="col">Evidence</th>
                  </tr>
                </thead>
                <tbody>
                  {entry.repository.detections.map((detection) => (
                    <tr key={detection.technology}>
                      <td>{detection.label}</td>
                      <td>
                        <StatusBadge
                          value={detection.confidence}
                          tone={confidenceTone(detection.confidence)}
                          label={detection.confidence}
                        />
                      </td>
                      <td>
                        {detection.evidence.join(', ') || '—'}
                        {detection.note ? (
                          <div className="muted">{detection.note}</div>
                        ) : null}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
            <dl className="detail-grid">
              <div>
                <dt>Suggested category</dt>
                <dd>{categoryLabel(entry.suggested_category)}</dd>
              </div>
              <div>
                <dt>Candidate templates</dt>
                <dd>
                  {(entry.candidate_templates ?? []).join(', ') || 'none'}
                </dd>
              </div>
              <div>
                <dt>Inspection notes</dt>
                <dd>{entry.repository.inspection.notes?.join(' ') || '—'}</dd>
              </div>
            </dl>
          </article>
        );
      })}
    </section>
  );
}
