import { useState } from 'react';

import { Dialog } from '../Dialog';
import type { PrepareOptions } from '../../services/desktopApi';

/**
 * The Prepare Library options dialog.
 *
 * Git synchronization is optional and off by default: a developer can build the
 * current local branch (including uncommitted changes) without any checkout. No
 * branch is forced, and no POM or version is ever modified
 * (`tasks/020.md` §24, §25, §26).
 */
export function PrepareLibraryDialog({
  title,
  targets,
  onExecute,
  onClose,
}: {
  title: string;
  targets: readonly string[];
  onExecute: (options: PrepareOptions) => void;
  onClose: () => void;
}) {
  const [sync, setSync] = useState(false);
  const [branch, setBranch] = useState('');
  const [build, setBuild] = useState(true);
  const [install, setInstall] = useState(true);
  const [verify, setVerify] = useState(true);

  const anyPhase = sync || build || install || verify;

  return (
    <Dialog
      title={title}
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button button--primary"
            disabled={!anyPhase}
            onClick={() =>
              onExecute({
                sync,
                branch: sync && branch.trim() !== '' ? branch.trim() : null,
                build,
                install,
                verify,
              })
            }
          >
            Prepare
          </button>
        </>
      }
    >
      <dl className="dialog__facts">
        <div>
          <dt>Libraries</dt>
          <dd>{targets.length}</dd>
        </div>
        <div className="dialog__facts-wide">
          <dt>Selected</dt>
          <dd>{targets.join(', ')}</dd>
        </div>
      </dl>

      <fieldset className="options-form">
        <legend className="toolbar__field-label">Phases</legend>
        <label className="options-form__field">
          <input
            type="checkbox"
            checked={build}
            onChange={(event) => setBuild(event.target.checked)}
          />
          <span>Build the library</span>
        </label>
        <label className="options-form__field">
          <input
            type="checkbox"
            checked={install}
            onChange={(event) => setInstall(event.target.checked)}
          />
          <span>Install the artifact locally</span>
        </label>
        <label className="options-form__field">
          <input
            type="checkbox"
            checked={verify}
            onChange={(event) => setVerify(event.target.checked)}
          />
          <span>Verify the installed artifact</span>
        </label>
        <label className="options-form__field">
          <input
            type="checkbox"
            checked={sync}
            onChange={(event) => setSync(event.target.checked)}
          />
          <span>Synchronize the repository first (optional)</span>
        </label>
        <label className="options-form__field options-form__field--indent">
          <span className="toolbar__field-label">Target branch</span>
          <input
            className="input"
            aria-label="Target branch"
            placeholder="Leave empty to keep the current branch"
            value={branch}
            disabled={!sync}
            onChange={(event) => setBranch(event.target.value)}
          />
        </label>
      </fieldset>

      <p className="dialog__note muted">
        Synchronization is optional and never forces <code>master</code>. No
        version is changed and no POM is rewritten — the daemon runs the existing
        prepare-library workflow.
      </p>
    </Dialog>
  );
}
