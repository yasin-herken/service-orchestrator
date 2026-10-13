import type { ServiceSummary } from '../../types/models';

/**
 * Explicit service selection for a workflow (`tasks/022.md` §7).
 *
 * Only explicitly checked services are selected here; dependencies are
 * resolved by the daemon's plan and shown separately, so the user always
 * knows what they chose versus what the backend added.
 */
export function ServiceSelection({
  services,
  selectedIds,
  disabled = false,
  onToggle,
  onToggleAll,
}: {
  services: readonly ServiceSummary[];
  selectedIds: ReadonlySet<string>;
  disabled?: boolean;
  onToggle: (serviceId: string, selected: boolean) => void;
  onToggleAll: (selected: boolean) => void;
}) {
  const allSelected =
    services.length > 0 && services.every((service) => selectedIds.has(service.id));

  return (
    <fieldset className="selection-list">
      <legend className="selection-list__legend">Select services</legend>
      <div className="selection-list__toolbar">
        <button
          type="button"
          className="button button--small"
          disabled={disabled || services.length === 0 || allSelected}
          onClick={() => onToggleAll(true)}
        >
          Select all visible
        </button>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || selectedIds.size === 0}
          onClick={() => onToggleAll(false)}
        >
          Clear
        </button>
        <span className="muted" aria-live="polite">
          {selectedIds.size} selected
        </span>
      </div>
      <ul className="selection-list__items">
        {services.map((service) => (
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
              </span>
            </label>
          </li>
        ))}
      </ul>
    </fieldset>
  );
}
