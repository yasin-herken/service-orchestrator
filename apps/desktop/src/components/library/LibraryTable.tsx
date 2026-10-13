import { useEffect, useRef } from 'react';

import { GitBadge } from '../StatusBadge';
import type { TrackedTask } from '../../context/TaskEventsContext';
import type { LibrarySummary } from '../../types/models';
import { operationLabel } from '../../utils/format';

/**
 * The library table: stable-id selection, artifact coordinates, configured
 * version, git state, dependents, and a real per-library "last operation"
 * column derived from daemon task events (`tasks/020.md` §20, §22).
 */
export function LibraryTable({
  libraries,
  workspaceName,
  selectedIds,
  onToggleSelect,
  onToggleSelectAll,
  allVisibleSelected,
  someVisibleSelected,
  detailId,
  onOpenDetail,
  busy,
  lastOperations,
}: {
  libraries: LibrarySummary[];
  /** The workspace every row belongs to (the table is workspace-scoped). */
  workspaceName: string;
  selectedIds: ReadonlySet<string>;
  onToggleSelect: (libraryId: string, selected: boolean) => void;
  onToggleSelectAll: (selected: boolean) => void;
  allVisibleSelected: boolean;
  someVisibleSelected: boolean;
  detailId: string | null;
  onOpenDetail: (libraryId: string) => void;
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
      <caption className="visually-hidden">Libraries in {workspaceName}</caption>
      <thead>
        <tr>
          <th scope="col" className="service-table__col--select">
            <input
              ref={headerCheckbox}
              type="checkbox"
              aria-label="Select all visible libraries"
              checked={allVisibleSelected}
              onChange={(event) => onToggleSelectAll(event.target.checked)}
            />
          </th>
          <th scope="col">Library</th>
          <th scope="col">Branch</th>
          <th scope="col">Artifact</th>
          <th scope="col">Configured version</th>
          <th scope="col">Git state</th>
          <th scope="col">Dependents</th>
          <th scope="col">Last operation</th>
          <th scope="col">Actions</th>
        </tr>
      </thead>
      <tbody>
        {libraries.map((library) => {
          const selected = selectedIds.has(library.id);
          const isBusy = busy.has(library.id);
          const open = library.id === detailId;
          const last = lastOperations.get(library.id);
          return (
            <tr
              key={library.id}
              tabIndex={0}
              aria-selected={selected}
              aria-busy={isBusy || undefined}
              className={`service-table__row${selected ? ' service-table__row--selected' : ''}${
                open ? ' service-table__row--detail' : ''
              }${isBusy ? ' service-table__row--busy' : ''}`}
              onClick={() => onOpenDetail(library.id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter') {
                  event.preventDefault();
                  onOpenDetail(library.id);
                } else if (event.key === ' ') {
                  event.preventDefault();
                  onToggleSelect(library.id, !selected);
                }
              }}
            >
              <td className="service-table__col--select">
                <input
                  type="checkbox"
                  aria-label={`Select ${library.display_name}`}
                  checked={selected}
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) => onToggleSelect(library.id, event.target.checked)}
                />
              </td>
              <td>
                <span className="service-table__name">{library.display_name}</span>
                <span className="service-table__id">{library.id}</span>
              </td>
              <td className="service-table__branch">{library.branch ?? '—'}</td>
              <td className="service-table__mono">
                {library.artifact ? (
                  library.artifact.coordinate
                ) : (
                  <span className="muted">Unknown</span>
                )}
              </td>
              <td className="service-table__mono">
                {library.configured_version ?? '—'}
              </td>
              <td>
                <GitBadge state={library.git_state} />
              </td>
              <td className="service-table__col--numeric">
                {library.dependents.length}
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
                    onOpenDetail(library.id);
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
