import { useEffect, useRef } from 'react';

import type { TrackedTask } from '../context/TaskEventsContext';
import type { ServiceSummary } from '../types/models';
import { operationLabel } from '../utils/format';
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
 * The services table: stable-id selection via checkboxes, detail navigation by
 * row click or keyboard, sorting via header buttons, and a real "last
 * operation" column derived from daemon task events.
 *
 * Selection and detail navigation are separate: the checkbox toggles the bulk
 * selection, while clicking elsewhere in the row (or Enter) opens the service
 * detail. Space toggles selection from the keyboard (`tasks/019.md` §7, §8).
 */
export function ServiceTable({
  services,
  workspaceName,
  selectedIds,
  onToggleSelect,
  onToggleSelectAll,
  allVisibleSelected,
  someVisibleSelected,
  detailId,
  onOpenDetail,
  sort,
  onSortChange,
  busy,
  lastOperations,
}: {
  services: ServiceSummary[];
  /** The workspace every row belongs to (the table is workspace-scoped). */
  workspaceName: string;
  selectedIds: ReadonlySet<string>;
  onToggleSelect: (serviceId: string, selected: boolean) => void;
  onToggleSelectAll: (selected: boolean) => void;
  allVisibleSelected: boolean;
  someVisibleSelected: boolean;
  detailId: string | null;
  onOpenDetail: (serviceId: string) => void;
  sort: TableSort;
  onSortChange: (key: ServiceSortKey) => void;
  busy: ReadonlySet<string>;
  lastOperations: ReadonlyMap<string, TrackedTask>;
}) {
  const headerCheckbox = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (headerCheckbox.current) {
      headerCheckbox.current.indeterminate = someVisibleSelected;
    }
  }, [someVisibleSelected]);

  return (
    <table className="service-table">
      <caption className="visually-hidden">Services in {workspaceName}</caption>
      <thead>
        <tr>
          <th scope="col" className="service-table__col--select">
            <input
              ref={headerCheckbox}
              type="checkbox"
              aria-label="Select all visible services"
              checked={allVisibleSelected}
              onChange={(event) => onToggleSelectAll(event.target.checked)}
            />
          </th>
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
          <th scope="col">Last operation</th>
        </tr>
      </thead>
      <tbody>
        {services.map((service) => {
          const selected = selectedIds.has(service.id);
          const isBusy = busy.has(service.id);
          const open = service.id === detailId;
          const last = lastOperations.get(service.id);
          return (
            <tr
              key={service.id}
              tabIndex={0}
              aria-selected={selected}
              aria-busy={isBusy || undefined}
              className={`service-table__row${selected ? ' service-table__row--selected' : ''}${
                open ? ' service-table__row--detail' : ''
              }${isBusy ? ' service-table__row--busy' : ''}`}
              onClick={() => onOpenDetail(service.id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter') {
                  event.preventDefault();
                  onOpenDetail(service.id);
                } else if (event.key === ' ') {
                  event.preventDefault();
                  onToggleSelect(service.id, !selected);
                }
              }}
            >
              <td className="service-table__col--select">
                <input
                  type="checkbox"
                  aria-label={`Select ${service.display_name}`}
                  checked={selected}
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => onToggleSelect(service.id, event.target.checked)}
                />
              </td>
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
              <td className="service-table__operation">
                {isBusy ? <span className="service-table__busy">Working…</span> : null}
                {last ? <span>{operationLabel(last)}</span> : <span className="muted">—</span>}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
