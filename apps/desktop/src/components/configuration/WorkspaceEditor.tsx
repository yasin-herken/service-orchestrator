import { useState } from 'react';

import { Dialog } from '../Dialog';
import type { RawEnvValueView, RawWorkspaceView } from '../../types/models';
import { EnvironmentEditor } from './EnvironmentEditor';

/**
 * The workspace editor (`tasks/023.md` §10–§12).
 *
 * The identifier is immutable once a workspace exists; the root and name are
 * fully editable. Nothing is persisted here — the parent previews and commits
 * the change through the daemon.
 */
export function WorkspaceEditor({
  initial,
  onSave,
  onClose,
}: {
  initial: RawWorkspaceView | null;
  onSave: (entity: RawWorkspaceView) => void;
  onClose: () => void;
}) {
  const editing = initial !== null;
  const [id, setId] = useState(initial?.id ?? '');
  const [name, setName] = useState(initial?.name ?? '');
  const [root, setRoot] = useState(initial?.root ?? '');
  const [environment, setEnvironment] = useState<Record<string, RawEnvValueView>>(
    initial?.environment ?? {},
  );

  const idValid = id.trim() !== '';
  const rootValid = root.trim() !== '';
  const valid = idValid && rootValid;

  const submit = () => {
    const entity: RawWorkspaceView = {
      id: id.trim(),
      root: root.trim(),
    };
    if (name.trim() !== '') {
      entity.name = name.trim();
    }
    if (Object.keys(environment).length > 0) {
      entity.environment = environment;
    }
    onSave(entity);
  };

  return (
    <Dialog
      title={editing ? `Edit workspace “${initial.id}”` : 'Add workspace'}
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
        <label className="config-form__field">
          <span className="toolbar__field-label">Identifier</span>
          <input
            className="input"
            value={id}
            disabled={editing}
            placeholder="acme"
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
            placeholder="Acme Platform"
            onChange={(event) => setName(event.target.value)}
          />
        </label>

        <label className="config-form__field">
          <span className="toolbar__field-label">Root</span>
          <input
            className="input"
            value={root}
            placeholder="~/Projects/acme"
            onChange={(event) => setRoot(event.target.value)}
          />
          {!rootValid ? (
            <span className="config-form__error">A workspace root is required.</span>
          ) : null}
        </label>

        <EnvironmentEditor value={environment} onChange={setEnvironment} />

        <p className="config-form__hint muted">
          The daemon validates the whole file before saving and never writes a
          partial change.
        </p>
      </div>
    </Dialog>
  );
}
