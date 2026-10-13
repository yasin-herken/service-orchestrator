import { useState } from 'react';

import { Dialog } from '../Dialog';
import type { RawProfileView } from '../../types/models';
import { parseList } from '../../utils/configuration';

/**
 * The developer-profile editor (`tasks/023.md` §31).
 *
 * A profile references existing services and groups instead of duplicating
 * their configuration, so the identifiers are resolved by the shared validator
 * — an unknown id is reported before anything is saved.
 */
export function ProfileEditor({
  initial,
  workspaceId,
  onSave,
  onClose,
}: {
  initial: RawProfileView | null;
  workspaceId: string;
  onSave: (entity: RawProfileView) => void;
  onClose: () => void;
}) {
  const editing = initial !== null;
  const [id, setId] = useState(initial?.id ?? '');
  const [name, setName] = useState(initial?.name ?? '');
  const [description, setDescription] = useState(initial?.description ?? '');
  const [servicesText, setServicesText] = useState((initial?.services ?? []).join(', '));
  const [groupsText, setGroupsText] = useState((initial?.groups ?? []).join(', '));

  const idValid = id.trim() !== '';
  const valid = idValid;

  const submit = () => {
    const entity: RawProfileView = {
      id: id.trim(),
      workspace: workspaceId,
    };
    if (name.trim() !== '') {
      entity.name = name.trim();
    }
    if (description.trim() !== '') {
      entity.description = description.trim();
    }
    const services = parseList(servicesText);
    if (services.length > 0) {
      entity.services = services;
    }
    const groups = parseList(groupsText);
    if (groups.length > 0) {
      entity.groups = groups;
    }
    onSave(entity);
  };

  return (
    <Dialog
      title={editing ? `Edit profile “${initial.id}”` : 'Add profile'}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button button--primary"
            onClick={submit}
            disabled={!valid}
          >
            Review change
          </button>
        </>
      }
    >
      <div className="config-form">
        <div className="config-form__grid">
          <label className="config-form__field">
            <span className="toolbar__field-label">Identifier</span>
            <input
              className="input"
              value={id}
              disabled={editing}
              placeholder="full-stack"
              aria-label="Identifier"
              onChange={(event) => setId(event.target.value)}
            />
            {!idValid ? (
              <span className="config-form__error">An identifier is required.</span>
            ) : null}
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Name</span>
            <input
              className="input"
              value={name}
              placeholder="Full stack"
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Workspace</span>
            <input className="input" value={workspaceId} readOnly />
          </label>
        </div>

        <label className="config-form__field">
          <span className="toolbar__field-label">Description</span>
          <input
            className="input"
            value={description}
            placeholder="Backend plus frontend for local development"
            onChange={(event) => setDescription(event.target.value)}
          />
        </label>

        <div className="config-form__grid">
          <label className="config-form__field">
            <span className="toolbar__field-label">Services</span>
            <input
              className="input"
              value={servicesText}
              placeholder="orders-api, admin-ui"
              onChange={(event) => setServicesText(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Groups</span>
            <input
              className="input"
              value={groupsText}
              placeholder="core"
              onChange={(event) => setGroupsText(event.target.value)}
            />
          </label>
        </div>

        <p className="config-form__hint muted">
          Profiles reference existing services and groups; they never duplicate
          service configuration.
        </p>
      </div>
    </Dialog>
  );
}
