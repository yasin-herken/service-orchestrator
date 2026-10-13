import { useEffect, useRef } from 'react';

import { GitBadge } from '../StatusBadge';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type { RepositorySummary } from '../../types/models';
import type { RepositorySortKey } from '../../utils/repositoryQuery';
import type { SortDirection } from '../../utils/serviceQuery';

/** Current sort of the repository table. */
export interface RepositoryTableSort {
  key: RepositorySortKey;
  direction: SortDirection;
}

const COLUMNS: { key: RepositorySortKey; label: string }[] = [
  { key: 'name', label: 'Repository' },
  { key: 'path', label: 'Path' },
  { key: 'branch', label: 'Branch' },
  { key: 'git_state', label: 'State' },
];

/** A concise label for the last-operation cell. */
function operationLabel(task: TrackedTask): string {
  const status = task.status.charAt(0).toUpperCase() + task.status.slice(1);
  return `${task.kind} · ${status}`;
}

/**
 * The Git repository table: stable-id selection, sorting, detail navigation,
 * and a real per-repository "last operation" column derived from daemon task
 * events (`tasks/020.md` §4, §5).
 */
export function RepositoryTable({
  repositories,
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
  repositories: RepositorySummary[];
  /** The workspace every row belongs to (the table is workspace-scoped). */
  workspaceName: string;
  selectedIds: ReadonlySet<string>;
  onToggleSelect: (repositoryId: string, selected: boolean) => void;
  onToggleSelectAll: (selected: boolean) => void;
  allVisibleSelected: boolean;
  someVisibleSelected: boolean;
  detailId: string | null;
  onOpenDetail: (repositoryId: string) => void;
  sort: RepositoryTableSort;
  onSortChange: (key: RepositorySortKey) => void;
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
      <caption className="visually-hidden">Repositories in {workspaceName}</caption>
      <thead>
        <tr>
          <th scope="col" className="service-table__col--select">
            <input
              ref={headerCheckbox}
              type="checkbox"
              aria-label="Select all visible repositories"
              checked={allVisibleSelected}
              onChange={(event) => onToggleSelectAll(event.target.checked)}
            />
          </th>
          {COLUMNS.map((column) => {
            const active = sort.key === column.key;
            return (
              <th
                key={column.key}
                scope="col"
                aria-sort={active ? (sort.direction === 'asc' ? 'ascending' : 'descending') : 'none'}
              >
                <button
                  type="button"
                  className={`service-table__sort${active ? ' service-table__sort--active' : ''}`}
                  onClick={() => onSortChange(column.key)}
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
          <th scope="col">Actions</th>
        </tr>
      </thead>
      <tbody>
        {repositories.map((repository) => {
          const selected = selectedIds.has(repository.id);
          const isBusy = busy.has(repository.id);
          const open = repository.id === detailId;
          const last = lastOperations.get(repository.service_id);
          return (
            <tr
              key={repository.id}
              tabIndex={0}
              aria-selected={selected}
              aria-busy={isBusy || undefined}
              className={`service-table__row${selected ? ' service-table__row--selected' : ''}${
                open ? ' service-table__row--detail' : ''
              }${isBusy ? ' service-table__row--busy' : ''}`}
              onClick={() => onOpenDetail(repository.id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter') {
                  event.preventDefault();
                  onOpenDetail(repository.id);
                } else if (event.key === ' ') {
                  event.preventDefault();
                  onToggleSelect(repository.id, !selected);
                }
              }}
            >
              <td className="service-table__col--select">
                <input
                  type="checkbox"
                  aria-label={`Select ${repository.name}`}
                  checked={selected}
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => onToggleSelect(repository.id, event.target.checked)}
                />
              </td>
              <td>
                <span className="service-table__name">{repository.name}</span>
                <span className="service-table__id">{repository.id}</span>
              </td>
              <td className="service-table__mono">{repository.path ?? '—'}</td>
              <td className="service-table__branch">{repository.branch ?? '—'}</td>
              <td>
                <GitBadge state={repository.git_state} />
              </td>
              <td className="service-table__operation">
                {isBusy ? <span className="service-table__busy">Working…</span> : null}
                {last ? <span>{operationLabel(last)}</span> : <span className="muted">—</span>}
              </td>
              <td className="service-table__operation">
                <button
                  type="button"
                  className="button button--small"
                  onClick={(event) => {
                    event.stopPropagation();
                    onOpenDetail(repository.id);
                  }}
                >
                  Inspect
                </button>
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
