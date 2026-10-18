import { useState } from 'react';

import { Dialog } from '../Dialog';
import type { RawOperationPresetView } from '../../types/models';

/**
 * The saved-operation-preset editor (`tasks/032.md` §38).
 *
 * A preset stores the selection and flags only — no credentials, no stored
 * authorization — and saving one writes through the existing configuration
 * manager, so the daemon validates and revisions the whole document.
 */
export function PresetEditor({
  mode,
  initial,
  selectionSummary,
  defaultExists,
  onSave,
  onClose,
}: {
  mode: 'create' | 'rename';
  initial: RawOperationPresetView | null;
  /** A short description of the current selection, for the create flow. */
  selectionSummary: string;
  /** Whether another preset is already the workspace default. */
  defaultExists: boolean;
  onSave: (values: {
    id: string;
    name: string;
    description: string;
    isDefault: boolean;
  }) => void;
  onClose: () => void;
}) {
  const editing = mode === 'rename';
  const [id, setId] = useState(initial?.id ?? '');
  const [name, setName] = useState(initial?.name ?? '');
  const [description, setDescription] = useState(initial?.description ?? '');
  const [isDefault, setIsDefault] = useState(false);

  return (
    <Dialog
      title={editing ? 'Rename operation preset' : 'Save current selection as a preset'}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button button--primary"
            onClick={() =>
              onSave({
                id: id.trim(),
                name: name.trim(),
                description: description.trim(),
                isDefault,
              })
            }
            disabled={id.trim() === '' || name.trim() === ''}
          >
            Review change…
          </button>
        </>
      }
    >
      <p className="dialog__message">
        {editing
          ? 'Renaming changes the display name only; the id stays stable.'
          : `Saves the current selection (${selectionSummary}) and operation flags. Loading the preset later only pre-fills this screen; it never executes anything.`}
      </p>
      <div className="config-form">
        <label className="config-form__field">
          <span>Identifier</span>
          <input
            className="input"
            value={id}
            disabled={editing}
            onChange={(event) => setId(event.target.value)}
            aria-label="Preset identifier"
          />
        </label>
        <label className="config-form__field">
          <span>Name</span>
          <input
            className="input"
            value={name}
            onChange={(event) => setName(event.target.value)}
            aria-label="Preset name"
          />
        </label>
        <label className="config-form__field">
          <span>Description</span>
          <input
            className="input"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            aria-label="Preset description"
          />
        </label>
        {!editing ? (
          <label className="config-form__field options-form__field--indent">
            <span>
              <input
                type="checkbox"
                checked={isDefault}
                disabled={defaultExists}
                onChange={(event) => setIsDefault(event.target.checked)}
                aria-label="Make this the default preset"
              />{' '}
              Make this the workspace default
            </span>
            <span className="muted">
              {defaultExists
                ? 'Another preset is already the default. Clear it first on the preset bar.'
                : 'At most one preset per workspace may be the default.'}
            </span>
          </label>
        ) : null}
        <p className="muted">
          Presets never store credentials and never carry authorization.
        </p>
      </div>
    </Dialog>
  );
}
