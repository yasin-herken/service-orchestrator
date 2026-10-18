import type { DiscoveredRepositoryView } from '../../types/models';
import { StatusBadge, type BadgeTone } from '../StatusBadge';
import type { ImportRowDraft } from './draft';

/** Confidence tone for a detection badge. */
export function confidenceTone(confidence: string): BadgeTone {
  switch (confidence) {
    case 'high':
      return 'positive';
    case 'medium':
      return 'info';
    case 'low':
      return 'warning';
    default:
      return 'neutral';
  }
}

/** A human label for a suggested category. */
export function categoryLabel(category: string | undefined): string {
  switch (category) {
    case 'spring_boot_backend':
      return 'Spring Boot backend';
    case 'react_frontend':
      return 'React frontend';
    case 'shared_maven_library':
      return 'Shared Maven library';
    case undefined:
      return 'No suggestion';
    default:
      return category;
  }
}

/** The detection badges shown in the selection table. */
export function DetectionBadges({
  repository,
}: {
  repository: DiscoveredRepositoryView;
}) {
  if (repository.repository.detections.length === 0) {
    return <span className="muted">none detected</span>;
  }
  return (
    <span className="badge-list">
      {repository.repository.detections.map((detection) => (
        <StatusBadge
          key={`${detection.technology}-${detection.confidence}`}
          value={detection.technology}
          tone={confidenceTone(detection.confidence)}
          label={`${detection.label} · ${detection.confidence}`}
        />
      ))}
    </span>
  );
}

/** A small badge naming existing entities at a path. */
function ExistingBadge({ repository }: { repository: DiscoveredRepositoryView }) {
  const existing = repository.existing ?? [];
  if (existing.length === 0) {
    return <span className="badge badge--positive">new</span>;
  }
  return (
    <span className="badge-list">
      {existing.map((entity) => (
        <StatusBadge
          key={`${entity.kind}-${entity.id}`}
          value={entity.kind}
          tone="warning"
          label={`${entity.kind} ${entity.id}`}
        />
      ))}
    </span>
  );
}

/**
 * The repository selection table (`tasks/032.md` §10 step 2–3).
 *
 * Every suggested template is shown as a *suggestion* with its category; an
 * ambiguous or absent match is stated explicitly, so the wizard never implies
 * a classification the daemon did not make (`tasks/032.md` §8, §9).
 */
export function RepositorySelection({
  repositories,
  rows,
  search,
  category,
  disabled = false,
  onSearch,
  onCategory,
  onToggle,
  onSelectAll,
  onSelectVisible,
}: {
  repositories: readonly DiscoveredRepositoryView[];
  rows: Readonly<Record<string, ImportRowDraft>>;
  search: string;
  category: string;
  disabled?: boolean;
  onSearch: (value: string) => void;
  onCategory: (value: string) => void;
  onToggle: (relativePath: string, selected: boolean) => void;
  onSelectAll: (selected: boolean) => void;
  onSelectVisible: (selected: boolean, visiblePaths: string[]) => void;
}) {
  const needle = search.trim().toLowerCase();
  const visible = repositories.filter((entry) => {
    const path = entry.repository.relative_path.toLowerCase();
    if (needle !== '' && !path.includes(needle) && !entry.repository.name.toLowerCase().includes(needle)) {
      return false;
    }
    if (category === 'all') {
      return true;
    }
    if (category === 'none') {
      return entry.suggested_category === undefined;
    }
    return entry.suggested_category === category;
  });
  const selectedCount = Object.values(rows).filter((row) => row.selected).length;

  return (
    <fieldset className="selection-list">
      <legend className="selection-list__legend">Select repositories</legend>
      <div className="selection-list__toolbar">
        <input
          type="search"
          className="input"
          placeholder="Filter by path or name"
          value={search}
          onChange={(event) => onSearch(event.target.value)}
          aria-label="Filter repositories"
        />
        <select
          className="input input--select"
          value={category}
          onChange={(event) => onCategory(event.target.value)}
          aria-label="Filter by suggested category"
        >
          <option value="all">All categories</option>
          <option value="spring_boot_backend">Spring Boot backend</option>
          <option value="react_frontend">React frontend</option>
          <option value="shared_maven_library">Shared Maven library</option>
          <option value="none">No suggestion</option>
        </select>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || repositories.length === 0}
          onClick={() => onSelectAll(true)}
        >
          Select all
        </button>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || visible.length === 0}
          onClick={() =>
            onSelectVisible(
              true,
              visible.map((entry) => entry.repository.relative_path),
            )
          }
        >
          Select visible
        </button>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || selectedCount === 0}
          onClick={() => onSelectAll(false)}
        >
          Select none
        </button>
        <span className="muted" aria-live="polite">
          {visible.length} visible · {selectedCount} selected
        </span>
      </div>

      {visible.length === 0 ? (
        <p className="muted">No repositories match this filter.</p>
      ) : (
        <div className="table-wrap">
          <table className="service-table" aria-label="Discovered repositories">
            <thead>
              <tr>
                <th scope="col">
                  <span className="visually-hidden">Include</span>
                </th>
                <th scope="col">Repository</th>
                <th scope="col">Detected technologies</th>
                <th scope="col">Suggested template</th>
                <th scope="col">Existing configuration</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((entry) => {
                const path = entry.repository.relative_path;
                const row = rows[path];
                const ambiguous = (entry.ambiguous_categories ?? []).length > 0;
                return (
                  <tr key={path} data-repository={path}>
                    <td>
                      <input
                        type="checkbox"
                        checked={row?.selected ?? false}
                        disabled={disabled}
                        onChange={(event) => onToggle(path, event.target.checked)}
                        aria-label={`Include ${path}`}
                      />
                    </td>
                    <td>
                      <div className="service-table__name">{entry.repository.name}</div>
                      <div className="muted">{path}</div>
                      {!entry.repository.inspection.is_git_repository ? (
                        <div className="muted">not a Git checkout</div>
                      ) : null}
                    </td>
                    <td>
                      <DetectionBadges repository={entry} />
                    </td>
                    <td>
                      {ambiguous ? (
                        <span className="badge badge--warning" title={(entry.ambiguous_categories ?? []).join(', ')}>
                          ambiguous — choose
                        </span>
                      ) : entry.suggested_template ? (
                        <span>
                          <StatusBadge
                            value={entry.suggested_template}
                            tone="info"
                            label={entry.suggested_template}
                          />
                          <span className="muted"> {categoryLabel(entry.suggested_category)}</span>
                        </span>
                      ) : (
                        <span className="muted">
                          {categoryLabel(entry.suggested_category)} — choose a template
                        </span>
                      )}
                    </td>
                    <td>
                      <ExistingBadge repository={entry} />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </fieldset>
  );
}
