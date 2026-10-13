import { useState } from 'react';

import { Dialog } from '../Dialog';
import type { RawGroupView } from '../../types/models';

/** Parses a comma/whitespace separated list into unique, ordered ids. */
function parseList(text: string): string[] {
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const raw of text.split(/[\s,]+/)) {
    const id = raw.trim();
    if (id !== '' && !seen.has(id)) {
      seen.add(id);
      ids.push(id);
    }
  }
  return ids;
}

/**
 * The service-group editor (`tasks/023.md` §32).
 *
 * A group is a logical set of services; it reuses the same workflows as a
 * multi-service selection and never duplicates service configuration. Removing
 * a service from a group never removes the service itself.
 */
export function GroupEditor({
  initial,
  workspaceId,
  onSave,
  onClose,
}: {
  initial: RawGroupView | null;
  workspaceId: string;
  onSave: (entity: RawGroupView) => void;
  onClose: () => void;
}) {
  const editing = initial !== null;
  const [id, setId] = useState(initial?.id ?? '');
  const [name, setName] = useState(initial?.name ?? '');
  const [description, setDescription] = useState(initial?.description ?? '');
  const [servicesText, setServicesText] = useState((initial?.services ?? []).join(', '));

  const idValid = id.trim() !== '';
  const valid = idValid;

  const submit = () => {
    const entity: RawGroupView = {
      id: id.trim(),
    };
    if (workspaceId !== '') {
      entity.workspace = workspaceId;
    }
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
    onSave(entity);
  };

  return (
    <Dialog
      title={editing ? `Edit group “${initial.id}”` : 'Add group'}
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
              placeholder="core"
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
              placeholder="Core services"
              onChange={(event) => setName(event.target.value)}
            />
          </label>
        </div>

        <label className="config-form__field">
          <span className="toolbar__field-label">Description</span>
          <input
            className="input"
            value={description}
            placeholder="Services required for basic development"
            onChange={(event) => setDescription(event.target.value)}
          />
        </label>

        <label className="config-form__field">
          <span className="toolbar__field-label">Services</span>
          <input
            className="input"
            value={servicesText}
            placeholder="orders-api, admin-ui"
            onChange={(event) => setServicesText(event.target.value)}
          />
        </label>

        <p className="config-form__hint muted">
          Group actions reuse the same application-level workflows as a manual
          multi-selection.
        </p>
      </div>
    </Dialog>
  );
}
