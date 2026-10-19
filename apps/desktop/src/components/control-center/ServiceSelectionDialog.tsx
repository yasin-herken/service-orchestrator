import { useMemo, useState } from 'react';

import { Dialog } from '../Dialog';
import { HealthBadge, ProcessBadge } from '../StatusBadge';
import type {
  BuildTargetSummary,
  GroupView,
  ProfileView,
  ServiceSummary,
} from '../../types/models';
import {
  filterServices,
  serviceKinds,
  serviceRuntimeLabel,
} from './model';

/**
 * The Workspace Control Center service-selection modal (`tasks/033.md` §4).
 *
 * Selection is local to the modal until the user applies it, so filtering,
 * searching, and switching between profiles and groups never lose checked
 * services. Identifiers are the workspace-scoped service ids the daemon uses;
 * dependencies are resolved later by the read-only plan, never here.
 */
export function ServiceSelectionDialog({
  services,
  buildTargets,
  profiles,
  groups,
  selectedIds,
  loading,
  disabled = false,
  onApply,
  onClose,
}: {
  services: readonly ServiceSummary[];
  buildTargets: readonly BuildTargetSummary[];
  profiles: readonly ProfileView[];
  groups: readonly GroupView[];
  selectedIds: ReadonlySet<string>;
  loading?: boolean;
  disabled?: boolean;
  onApply: (serviceIds: ReadonlySet<string>) => void;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState<ReadonlySet<string>>(
    () => new Set(selectedIds),
  );
  const [search, setSearch] = useState('');
  const [kind, setKind] = useState('all');
  const [profileId, setProfileId] = useState('');
  const [groupId, setGroupId] = useState('');

  const visible = useMemo(
    () => filterServices(services, search, kind),
    [services, search, kind],
  );
  const kinds = useMemo(() => serviceKinds(services), [services]);
  const targetById = useMemo(
    () => new Map(buildTargets.map((target) => [target.service_id, target])),
    [buildTargets],
  );

  const toggle = (serviceId: string, selected: boolean) => {
    setDraft((current) => {
      const next = new Set(current);
      if (selected) {
        next.add(serviceId);
      } else {
        next.delete(serviceId);
      }
      return next;
    });
    // Editing the draft means it no longer equals a profile or group.
    setProfileId('');
    setGroupId('');
  };

  const selectAll = () => {
    setDraft(new Set(services.map((service) => service.id)));
    setProfileId('');
    setGroupId('');
  };

  const selectVisible = () => {
    setDraft((current) => {
      const next = new Set(current);
      for (const service of visible) {
        next.add(service.id);
      }
      return next;
    });
    setProfileId('');
    setGroupId('');
  };

  const clear = () => {
    setDraft(new Set());
    setProfileId('');
    setGroupId('');
  };

  const applyProfile = (id: string) => {
    setProfileId(id);
    setGroupId('');
    if (id === '') {
      return;
    }
    const profile = profiles.find((candidate) => candidate.id === id);
    if (profile) {
      setDraft(new Set(profile.resolved_services));
    }
  };

  const applyGroup = (id: string) => {
    setGroupId(id);
    setProfileId('');
    if (id === '') {
      return;
    }
    const group = groups.find((candidate) => candidate.id === id);
    if (group) {
      setDraft(new Set(group.services));
    }
  };

  return (
    <Dialog
      title="Select services"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button button--primary"
            disabled={disabled}
            onClick={() => onApply(draft)}
          >
            Apply selection
          </button>
        </>
      }
    >
      <div className="selection-list__toolbar control-center__selection-toolbar">
        <input
          type="search"
          className="input"
          placeholder="Search services"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          aria-label="Search services"
        />
        <select
          className="input input--select"
          value={kind}
          onChange={(event) => setKind(event.target.value)}
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
          onClick={selectAll}
        >
          Select all
        </button>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || visible.length === 0}
          onClick={selectVisible}
        >
          Select visible
        </button>
        <button
          type="button"
          className="button button--small"
          disabled={disabled || draft.size === 0}
          onClick={clear}
        >
          Clear selection
        </button>
      </div>

      <div className="control-center__selection-sources">
        <label className="options-form__field">
          <span>Select by profile</span>
          <select
            className="input input--select"
            value={profileId}
            disabled={disabled || profiles.length === 0}
            onChange={(event) => applyProfile(event.target.value)}
            aria-label="Select by profile"
          >
            <option value="">Choose a profile…</option>
            {profiles.map((profile) => (
              <option key={profile.id} value={profile.id}>
                {profile.display_name}
              </option>
            ))}
          </select>
        </label>
        <label className="options-form__field">
          <span>Select by group</span>
          <select
            className="input input--select"
            value={groupId}
            disabled={disabled || groups.length === 0}
            onChange={(event) => applyGroup(event.target.value)}
            aria-label="Select by group"
          >
            <option value="">Choose a group…</option>
            {groups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.display_name}
              </option>
            ))}
          </select>
        </label>
        <p className="muted control-center__selection-count" aria-live="polite">
          {visible.length} visible · {draft.size} selected
        </p>
      </div>

      {loading ? (
        <p className="muted" role="status">
          Loading services…
        </p>
      ) : visible.length === 0 ? (
        <p className="muted">No services match this filter.</p>
      ) : (
        <ul className="selection-list__items control-center__selection-list">
          {visible.map((service) => {
            const runtime = serviceRuntimeLabel(targetById.get(service.id));
            return (
              <li key={service.id} className="selection-list__item">
                <label>
                  <input
                    type="checkbox"
                    checked={draft.has(service.id)}
                    disabled={disabled}
                    onChange={(event) => toggle(service.id, event.target.checked)}
                    aria-label={`Select ${service.id}`}
                  />
                  <span className="selection-list__name">{service.display_name}</span>
                  <span className="muted selection-list__meta">
                    {service.kind}
                    {service.branch ? ` · ${service.branch}` : ''}
                    {runtime ? ` · ${runtime}` : ''}
                  </span>
                  <span className="control-center__selection-badges">
                    <ProcessBadge state={service.process_state} />
                    <HealthBadge state={service.health_state} />
                  </span>
                </label>
              </li>
            );
          })}
        </ul>
      )}

      <p className="dialog__note muted">
        Required dependencies are resolved by the daemon&apos;s read-only plan,
        not by this selection. Unchecked services are never affected.
      </p>
    </Dialog>
  );
}
