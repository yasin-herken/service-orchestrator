import type { GroupView } from '../../types/models';
import { EmptyState } from '../StateViews';

/**
 * Service-group selection for a workflow (`tasks/022.md` §9).
 *
 * Members come from configuration; the dependencies those members pull in are
 * resolved by the daemon's plan, never duplicated here.
 */
export function GroupSelector({
  groups,
  selectedId,
  disabled = false,
  onSelect,
}: {
  groups: readonly GroupView[];
  selectedId: string | null;
  disabled?: boolean;
  onSelect: (groupId: string) => void;
}) {
  if (groups.length === 0) {
    return (
      <EmptyState
        title="No groups configured"
        description="Add groups under [groups] in ~/.service-orchestrator/config.toml."
      />
    );
  }

  return (
    <fieldset className="selection-list">
      <legend className="selection-list__legend">Select a service group</legend>
      <ul className="selection-list__items">
        {groups.map((group) => (
          <li key={group.id} className="selection-list__item">
            <label>
              <input
                type="radio"
                name="workflow-group"
                checked={selectedId === group.id}
                disabled={disabled}
                onChange={() => onSelect(group.id)}
                aria-label={`Select group ${group.id}`}
              />
              <span className="selection-list__name">{group.display_name}</span>
              <span className="muted selection-list__meta">
                {group.services.length} member{group.services.length === 1 ? '' : 's'}
              </span>
            </label>
            {selectedId === group.id ? (
              <dl className="profile-detail">
                <div>
                  <dt>Workspace</dt>
                  <dd>{group.workspace_id}</dd>
                </div>
                {group.description ? (
                  <div>
                    <dt>Description</dt>
                    <dd>{group.description}</dd>
                  </div>
                ) : null}
                <div>
                  <dt>Member services</dt>
                  <dd>{group.services.join(', ') || '—'}</dd>
                </div>
                <div className="dialog__facts-wide">
                  <dt>Workflow actions</dt>
                  <dd>Preview a read-only plan, then execute it through the daemon.</dd>
                </div>
              </dl>
            ) : null}
          </li>
        ))}
      </ul>
    </fieldset>
  );
}
