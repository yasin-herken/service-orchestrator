import { useMemo, useState } from 'react';

import { ServiceTable, type TableSort } from '../components/ServiceTable';
import { EmptyState, ErrorState, LoadingState } from '../components/StateViews';
import { useConnection } from '../context/ConnectionContext';
import { useWorkspaces } from '../context/WorkspaceContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import type { ServiceSummary } from '../types/models';
import { pluralize } from '../utils/format';
import {
  filterServices,
  sortServices,
  type ServiceSortKey,
} from '../utils/serviceQuery';

/**
 * The read-only services list: search, filtering, sorting, and row selection
 * over real daemon data (`tasks/018.md` §17). Start/stop actions are out of
 * scope for this task.
 */
export function ServicesPage() {
  const { activeWorkspace, loading: workspacesLoading, error: workspacesError, reload } =
    useWorkspaces();
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';

  const services = useAsync<ServiceSummary[]>(
    () => {
      if (!activeWorkspace) {
        return Promise.resolve([]);
      }
      return desktopApi.listServices(activeWorkspace.id);
    },
    [activeWorkspace?.id],
    { enabled: connected && activeWorkspace !== null },
  );

  const [search, setSearch] = useState('');
  const [kind, setKind] = useState('all');
  const [sort, setSort] = useState<TableSort>({ key: 'name', direction: 'asc' });
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const rows = services.data ?? [];
  const kindOptions = useMemo(() => {
    const kinds = new Set(rows.map((row) => row.kind));
    return ['all', ...Array.from(kinds).sort()];
  }, [rows]);

  const filtered = useMemo(
    () => sortServices(filterServices(rows, { search, kind }), sort.key, sort.direction),
    [rows, search, kind, sort],
  );

  const selected = useMemo(
    () => rows.find((row) => row.id === selectedId) ?? null,
    [rows, selectedId],
  );

  const filtersActive = search.trim().length > 0 || kind !== 'all';

  if (workspacesError) {
    return <ErrorState error={workspacesError} onRetry={reload} />;
  }
  if (!connected && !activeWorkspace) {
    return <LoadingState label="Waiting for the daemon connection…" />;
  }
  if (workspacesLoading && !activeWorkspace) {
    return <LoadingState label="Loading workspaces…" />;
  }
  if (!activeWorkspace) {
    return (
      <EmptyState
        title="No workspace selected"
        description="Configure a workspace in ~/.service-orchestrator/config.toml to see its services."
      />
    );
  }

  const toggleSort = (key: ServiceSortKey) => {
    setSort((current) =>
      current.key === key
        ? { key, direction: current.direction === 'asc' ? 'desc' : 'asc' }
        : { key, direction: 'asc' },
    );
  };

  return (
    <div className="page">
      <div className="toolbar" role="search">
        <label className="toolbar__field">
          <span className="visually-hidden">Search services</span>
          <input
            type="search"
            className="input"
            placeholder="Search by name, id, or branch…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            aria-label="Search services"
          />
        </label>
        <label className="toolbar__field">
          <span className="toolbar__field-label">Type</span>
          <select
            className="input input--select"
            value={kind}
            onChange={(event) => setKind(event.target.value)}
            aria-label="Filter by service type"
          >
            {kindOptions.map((option) => (
              <option key={option} value={option}>
                {option === 'all' ? 'All types' : option}
              </option>
            ))}
          </select>
        </label>
        <span className="toolbar__count" aria-live="polite">
          {pluralize(filtered.length, 'service')}
          {filtered.length !== rows.length ? ` of ${rows.length}` : ''}
        </span>
        {filtersActive ? (
          <button
            type="button"
            className="button button--small"
            onClick={() => {
              setSearch('');
              setKind('all');
            }}
          >
            Clear filters
          </button>
        ) : null}
      </div>

      <div className="card card--table">
        {services.loading ? <LoadingState label="Loading services…" /> : null}
        {services.error ? <ErrorState error={services.error} onRetry={services.reload} /> : null}
        {!services.loading && !services.error ? (
          services.data === null ? (
            <LoadingState
              label={connected ? 'Loading services…' : 'Waiting for the daemon connection…'}
            />
          ) : rows.length === 0 ? (
            <EmptyState
              title="No services configured"
              description={`The workspace “${activeWorkspace.name}” has no services.`}
            />
          ) : filtered.length === 0 ? (
            <EmptyState
              title="No services match your filters"
              description="Adjust the search text or type filter."
              action={
                <button
                  type="button"
                  className="button"
                  onClick={() => {
                    setSearch('');
                    setKind('all');
                  }}
                >
                  Clear filters
                </button>
              }
            />
          ) : (
            <ServiceTable
              services={filtered}
              workspaceName={activeWorkspace.name}
              selectedId={selectedId}
              onSelect={setSelectedId}
              sort={sort}
              onSortChange={toggleSort}
            />
          )
        ) : null}
      </div>

      <div className="selection-bar" aria-live="polite">
        {selected ? (
          <span>
            Selected: <strong>{selected.display_name}</strong>
            <span className="muted"> ({selected.id}) — actions arrive in a later task</span>
          </span>
        ) : (
          <span className="muted">Select a row to inspect a service.</span>
        )}
      </div>
    </div>
  );
}
