import type { DiscoveredRepositoryView, RawTemplateView } from '../../types/models';
import type { ImportRowDraft } from './draft';
import { categoryLabel } from './RepositorySelection';

/**
 * Template assignment (`tasks/032.md` §10 step 5, §9).
 *
 * "Automatic" is a real option: the daemon matches only when exactly one
 * template of the suggested category exists, and rejects an ambiguous match
 * with `TEMPLATE_AMBIGUOUS`. The suggestion is shown next to each choice so an
 * automatic match is always visible before the user reviews the plan.
 */
export function TemplateAssignment({
  repositories,
  rows,
  templates,
  onTemplate,
}: {
  repositories: readonly DiscoveredRepositoryView[];
  rows: Readonly<Record<string, ImportRowDraft>>;
  templates: readonly RawTemplateView[];
  onTemplate: (relativePath: string, template: string) => void;
}) {
  if (repositories.length === 0) {
    return <p className="muted">Select at least one repository to assign templates.</p>;
  }
  return (
    <section className="import-review" aria-label="Template assignment">
      <h3 className="card__title">Assign templates</h3>
      <p className="muted">
        A shared bulk override never replaces the template; it only fills values the
        template leaves unset. The daemon resolves precedence and reports every problem
        in the validation step.
      </p>
      <div className="table-wrap">
        <table className="service-table" aria-label="Template assignment">
          <thead>
            <tr>
              <th scope="col">Repository</th>
              <th scope="col">Suggestion</th>
              <th scope="col">Template</th>
            </tr>
          </thead>
          <tbody>
            {repositories.map((entry) => {
              const path = entry.repository.relative_path;
              const row = rows[path];
              const ambiguous = (entry.ambiguous_categories ?? []).length > 0;
              return (
                <tr key={path} data-repository={path}>
                  <td>
                    <div className="service-table__name">{entry.repository.name}</div>
                    <div className="muted">{path}</div>
                  </td>
                  <td>
                    {ambiguous ? (
                      <span className="badge badge--warning">
                        ambiguous: {(entry.ambiguous_categories ?? []).map(categoryLabel).join(', ')}
                      </span>
                    ) : entry.suggested_template ? (
                      <span>
                        <span className="badge badge--info">{entry.suggested_template}</span>{' '}
                        <span className="muted">{categoryLabel(entry.suggested_category)}</span>
                      </span>
                    ) : (
                      <span className="muted">
                        {categoryLabel(entry.suggested_category)}
                      </span>
                    )}
                  </td>
                  <td>
                    <select
                      className="input input--select"
                      value={row?.template ?? ''}
                      onChange={(event) => onTemplate(path, event.target.value)}
                      aria-label={`Template for ${path}`}
                    >
                      <option value="">
                        {entry.suggested_template
                          ? `Automatic (${entry.suggested_template})`
                          : 'Automatic matching'}
                      </option>
                      {templates.map((template) => (
                        <option key={template.id} value={template.id}>
                          {template.name ?? template.id} ({template.id})
                          {template.category ? ` · ${categoryLabel(template.category)}` : ''}
                        </option>
                      ))}
                    </select>
                    {(entry.candidate_templates ?? []).length > 1 ? (
                      <div className="muted">
                        Candidates: {(entry.candidate_templates ?? []).join(', ')}
                      </div>
                    ) : null}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
}
