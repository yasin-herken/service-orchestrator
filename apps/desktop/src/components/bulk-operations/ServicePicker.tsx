import type { ServiceSummary } from '../../types/models';

/**
 * Searchable service multi-selection (`tasks/032.md` §23).
 *
 * The picker only changes what the user selected. Dependencies are resolved by
 * the daemon's plan and shown there, so the screen never claims a service was
 * selected that the user did not check, and never affects unselected services.
 */
export function ServicePicker({
  services,
  selectedIds,
  search,
  kind,
  disabled = false,
  onSearch,
  onKind,
  onToggle,
  onSelectAll,
  onSelectNone,
  onSelectVisible,
}: {
  services: readonly ServiceSummary[];
  selectedIds: ReadonlySet<string>;
  search: string;
  kind: string;
  disabled?: boolean;
  onSearch: (value: string) => void;
  onKind: (value: string) => void;
  onToggle: (serviceId: string, selected: boolean) => void;
  onSelectAll: () => void;
  onSelectNone: () => void;
  onSelectVisible: (serviceIds: string[]) => void;
}) {
  const needle = search.trim().toLowerCase();
  const visible = services.filter((service) => {
    if (
      needle !== '' &&
      !service.id.toLowerCase().includes(needle) &&
      !service.display_name.toLowerCase().includes(needle)
    ) {
      return false;
    }
    if (kind === 'all') {
      return true;
    }
    return service.kind === kind;
  });
  const kinds = Array.from(new Set(services.map((service) => service.kind))).sort();

  return (
    <fieldset className="selection-list">
      <legend className="selection-list__legend">Services</legend>
      <div className="selection-list__toolbar">
        <input
          type="search"
          className="input"
          placeholder="Search services"
          value={search}
          onChange={(event) => onSearch(event.target.value)}
          aria-label="Search services"
        />
        <select
          className="input input--select"
          value={kind}
          onChange={(event) => onKind(event.target.value)}
          aria-label="Filter services by type"
        >
          <option value="all">All types</option>
          {kinds.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || services.length === 0}
          onClick={onSelectAll}
        >
          Select all
        </button>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || visible.length === 0}
          onClick={() => onSelectVisible(visible.map((service) => service.id))}
        >
          Select visible
        </button>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || selectedIds.size === 0}
          onClick={onSelectNone}
        >
          Select none
        </button>
        <span className="muted" aria-live="polite">
          {visible.length} visible · {selectedIds.size} selected
        </span>
      </div>

      {visible.length === 0 ? (
        <p className="muted">No services match this filter.</p>
      ) : (
        <ul className="selection-list__items selection-list__items--scroll">
          {visible.map((service) => (
            <li key={service.id} className="selection-list__item">
              <label>
                <input
                  type="checkbox"
                  checked={selectedIds.has(service.id)}
                  disabled={disabled}
                  onChange={(event) => onToggle(service.id, event.target.checked)}
                  aria-label={`Select ${service.id}`}
                />
                <span className="selection-list__name">{service.display_name}</span>
                <span className="muted selection-list__meta">
                  {service.kind}
                  {service.branch ? ` · ${service.branch}` : ''}
                  {service.process_state ? ` · ${service.process_state}` : ''}
                </span>
              </label>
            </li>
          ))}
        </ul>
      )}
    </fieldset>
  );
}
