import { useEffect, useRef } from 'react';

import type { TrackedTask } from '../../context/TaskEventsContext';
import type { BuildTargetSummary } from '../../types/models';
import { operationLabel } from '../../utils/format';
import { buildOperationLabel, exposedOperations } from './BuildCapabilitiesList';

/** Whether a build target can actually be built by the daemon today. */
export function canBuild(target: BuildTargetSummary): boolean {
  return exposedOperations(target.capability).some(
    (operation) => operation.operation === 'build',
  );
}

/**
 * The build-target table: stable-id selection, per-service provider
 * capabilities, and a real per-service "last operation" column derived from
 * daemon task events.
 *
 * The table is provider-agnostic: it renders whatever the Application Core
 * reports and disables building where the core exposes no build operation
 * (`tasks/020.md` §12, §13, §16).
 */
export function BuildTargetTable({
  targets,
  workspaceName,
  selectedIds,
  onToggleSelect,
  onToggleSelectAll,
  allVisibleSelected,
  someVisibleSelected,
  detailId,
  onOpenDetail,
  onBuild,
  busy,
  lastOperations,
}: {
  targets: BuildTargetSummary[];
  /** The workspace every row belongs to (the table is workspace-scoped). */
  workspaceName: string;
  selectedIds: ReadonlySet<string>;
  onToggleSelect: (serviceId: string, selected: boolean) => void;
  onToggleSelectAll: (selected: boolean) => void;
  allVisibleSelected: boolean;
  someVisibleSelected: boolean;
  detailId: string | null;
  onOpenDetail: (serviceId: string) => void;
  onBuild: (target: BuildTargetSummary) => void;
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
      <caption className="visually-hidden">Build targets in {workspaceName}</caption>
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
          <th scope="col">Service</th>
          <th scope="col">Type</th>
          <th scope="col">Provider</th>
          <th scope="col">Runtime</th>
          <th scope="col">Available operations</th>
          <th scope="col">Last operation</th>
          <th scope="col">Actions</th>
        </tr>
      </thead>
      <tbody>
        {targets.map((target) => {
          const selected = selectedIds.has(target.service_id);
          const isBusy = busy.has(target.service_id);
          const open = target.service_id === detailId;
          const last = lastOperations.get(target.service_id);
          const exposed = exposedOperations(target.capability);
          const buildable = canBuild(target);
          return (
            <tr
              key={target.service_id}
              tabIndex={0}
              aria-selected={selected}
              aria-busy={isBusy || undefined}
              className={`service-table__row${selected ? ' service-table__row--selected' : ''}${
                open ? ' service-table__row--detail' : ''
              }${isBusy ? ' service-table__row--busy' : ''}`}
              onClick={() => onOpenDetail(target.service_id)}
              onKeyDown={(event) => {
                if (event.target !== event.currentTarget) {
                  return;
                }
                if (event.key === 'Enter') {
                  event.preventDefault();
                  onOpenDetail(target.service_id);
                } else if (event.key === ' ') {
                  event.preventDefault();
                  onToggleSelect(target.service_id, !selected);
                }
              }}
            >
              <td className="service-table__col--select">
                <input
                  type="checkbox"
                  aria-label={`Select ${target.display_name}`}
                  checked={selected}
                  onClick={(event) => event.stopPropagation()}
                  onChange={(event) =>
                    onToggleSelect(target.service_id, event.target.checked)
                  }
                />
              </td>
              <td>
                <span className="service-table__name">{target.display_name}</span>
                <span className="service-table__id">{target.service_id}</span>
              </td>
              <td>{target.kind}</td>
              <td>
                {target.build_system ? (
                  <span className="service-table__mono">{target.build_system}</span>
                ) : (
                  <span className="muted">—</span>
                )}
              </td>
              <td className="service-table__branch">
                {target.runtimes.length > 0
                  ? target.runtimes
                      .map(
                        (runtime) =>
                          `${runtime.kind}${runtime.version ? ` ${runtime.version}` : ''}`,
                      )
                      .join(', ')
                  : '—'}
              </td>
              <td className="service-table__mono">
                {target.capability_error ? (
                  <span className="muted" title={target.capability_error}>
                    {target.capability_error}
                  </span>
                ) : exposed.length > 0 ? (
                  exposed.map((operation) => buildOperationLabel(operation.operation)).join(', ')
                ) : (
                  <span className="muted">None</span>
                )}
              </td>
              <td className="service-table__operation">
                {isBusy ? <span className="service-table__busy">Working…</span> : null}
                {last ? <span>{operationLabel(last)}</span> : <span className="muted">—</span>}
              </td>
              <td className="service-table__operation">
                <button
                  type="button"
                  className="button button--small button--primary"
                  disabled={!buildable}
                  title={
                    buildable
                      ? undefined
                      : target.capability_error ??
                        'the application core exposes no build operation for this service'
                  }
                  onClick={(event) => {
                    event.stopPropagation();
                    onBuild(target);
                  }}
                >
                  Build
                </button>{' '}
                <button
                  type="button"
                  className="button button--small"
                  onClick={(event) => {
                    event.stopPropagation();
                    onOpenDetail(target.service_id);
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
