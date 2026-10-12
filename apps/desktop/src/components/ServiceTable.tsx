import type { ServiceSummary } from '../types/models';
import type { ServiceSortKey, SortDirection } from '../utils/serviceQuery';
import { HealthBadge, ProcessBadge } from './StatusBadge';

/** Current sort of the services table. */
export interface TableSort {
  key: ServiceSortKey;
  direction: SortDirection;
}

const COLUMNS: { key: ServiceSortKey | 'workspace'; label: string; numeric?: boolean }[] = [
  { key: 'name', label: 'Name' },
  { key: 'kind', label: 'Type' },
  { key: 'workspace', label: 'Workspace' },
  { key: 'process_state', label: 'State' },
  { key: 'health_state', label: 'Health' },
  { key: 'branch', label: 'Branch' },
  { key: 'port', label: 'Port', numeric: true },
];

/**
 * The read-only services table: sorting via header buttons, row selection via
 * click or keyboard (Enter/Space). Start/stop actions arrive in Task 019
 * (`tasks/018.md` §17).
 */
export function ServiceTable({
  services,
  workspaceName,
  selectedId,
  onSelect,
  sort,
  onSortChange,
}: {
  services: ServiceSummary[];
  /** The workspace every row belongs to (the table is workspace-scoped). */
  workspaceName: string;
  selectedId: string | null;
  onSelect: (serviceId: string) => void;
  sort: TableSort;
  onSortChange: (key: ServiceSortKey) => void;
}) {
  return (
    <table className="service-table">
      <caption className="visually-hidden">Services in {workspaceName}</caption>
      <thead>
        <tr>
          {COLUMNS.map((column) => {
            const key = column.key;
            if (key === 'workspace') {
              return (
                <th key={key} scope="col">
                  {column.label}
                </th>
              );
            }
            const active = sort.key === key;
            return (
              <th
                key={key}
                scope="col"
                className={column.numeric ? 'service-table__col--numeric' : undefined}
                aria-sort={active ? (sort.direction === 'asc' ? 'ascending' : 'descending') : 'none'}
              >
                <button
                  type="button"
                  className={`service-table__sort${active ? ' service-table__sort--active' : ''}`}
                  onClick={() => onSortChange(key)}
                >
                  {column.label}
                  <span aria-hidden="true" className="service-table__sort-indicator">
                    {active ? (sort.direction === 'asc' ? '▲' : '▼') : ''}
                  </span>
                </button>
              </th>
            );
          })}
        </tr>
      </thead>
      <tbody>
        {services.map((service) => {
          const selected = service.id === selectedId;
          return (
            <tr
              key={service.id}
              tabIndex={0}
              aria-selected={selected}
              className={`service-table__row${selected ? ' service-table__row--selected' : ''}`}
              onClick={() => onSelect(service.id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onSelect(service.id);
                }
              }}
            >
              <td>
                <span className="service-table__name">{service.display_name}</span>
                <span className="service-table__id">{service.id}</span>
              </td>
              <td>{service.kind}</td>
              <td>{workspaceName}</td>
              <td>
                <ProcessBadge state={service.process_state} />
              </td>
              <td>
                <HealthBadge state={service.health_state} />
              </td>
              <td className="service-table__branch">{service.branch ?? '—'}</td>
              <td className="service-table__col--numeric">{service.port ?? '—'}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
