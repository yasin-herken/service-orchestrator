import type { ProfileView } from '../../types/models';
import { EmptyState } from '../StateViews';

/**
 * Profile selection for a workflow (`tasks/022.md` §8).
 *
 * Profiles are read from configuration through the daemon; this component
 * displays them and selects exactly one. It never creates or edits a profile.
 */
export function ProfileSelector({
  profiles,
  selectedId,
  disabled = false,
  onSelect,
}: {
  profiles: readonly ProfileView[];
  selectedId: string | null;
  disabled?: boolean;
  onSelect: (profileId: string) => void;
}) {
  if (profiles.length === 0) {
    return (
      <EmptyState
        title="No profiles configured"
        description="Add profiles under [profiles] in ~/.service-orchestrator/config.toml."
      />
    );
  }

  return (
    <fieldset className="selection-list">
      <legend className="selection-list__legend">Select a development profile</legend>
      <ul className="selection-list__items">
        {profiles.map((profile) => (
          <li key={profile.id} className="selection-list__item">
            <label>
              <input
                type="radio"
                name="workflow-profile"
                checked={selectedId === profile.id}
                disabled={disabled}
                onChange={() => onSelect(profile.id)}
                aria-label={`Select profile ${profile.id}`}
              />
              <span className="selection-list__name">{profile.display_name}</span>
              <span className="muted selection-list__meta">
                {profile.resolved_services.length} service
                {profile.resolved_services.length === 1 ? '' : 's'} ·{' '}
                {profile.groups.length} group{profile.groups.length === 1 ? '' : 's'}
              </span>
            </label>
            {selectedId === profile.id ? (
              <dl className="profile-detail">
                <div>
                  <dt>Workspace</dt>
                  <dd>{profile.workspace_id}</dd>
                </div>
                {profile.description ? (
                  <div>
                    <dt>Description</dt>
                    <dd>{profile.description}</dd>
                  </div>
                ) : null}
                <div>
                  <dt>Services</dt>
                  <dd>{profile.services.join(', ') || '—'}</dd>
                </div>
                <div>
                  <dt>Groups</dt>
                  <dd>{profile.groups.join(', ') || '—'}</dd>
                </div>
                <div>
                  <dt>Resolved services</dt>
                  <dd>{profile.resolved_services.join(', ') || '—'}</dd>
                </div>
              </dl>
            ) : null}
          </li>
        ))}
      </ul>
    </fieldset>
  );
}
