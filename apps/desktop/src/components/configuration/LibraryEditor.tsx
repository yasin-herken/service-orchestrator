import { useState } from 'react';

import { Dialog } from '../Dialog';
import type { RawEnvValueView, RawLibraryView } from '../../types/models';
import { commandText, toCommand } from '../../utils/configuration';
import { EnvironmentEditor } from './EnvironmentEditor';

/**
 * The shared-library editor (`tasks/023.md` §30).
 *
 * A library declares its artifact coordinate so a downstream service can depend
 * on an exact version. This editor never rewrites a POM and never bumps a
 * version automatically; the daemon validates and persists the declaration.
 */
export function LibraryEditor({
  initial,
  workspaceId,
  onSave,
  onClose,
}: {
  initial: RawLibraryView | null;
  workspaceId: string;
  onSave: (entity: RawLibraryView) => void;
  onClose: () => void;
}) {
  const editing = initial !== null;
  const [id, setId] = useState(initial?.id ?? '');
  const [name, setName] = useState(initial?.name ?? '');
  const [path, setPath] = useState(initial?.path ?? '');
  const [defaultBranch, setDefaultBranch] = useState(initial?.default_branch ?? 'main');
  const [type, setType] = useState(initial?.type ?? 'maven');
  const [repositoryUrl, setRepositoryUrl] = useState(initial?.repository?.url ?? '');
  const [groupId, setGroupId] = useState(initial?.artifact?.group_id ?? '');
  const [artifactId, setArtifactId] = useState(initial?.artifact?.artifact_id ?? '');
  const [version, setVersion] = useState(initial?.artifact?.version ?? '');
  const [buildCommand, setBuildCommand] = useState(commandText(initial?.commands?.build));
  const [environment, setEnvironment] = useState<Record<string, RawEnvValueView>>(
    initial?.environment ?? {},
  );

  const idValid = id.trim() !== '';
  const valid = idValid;

  const submit = () => {
    const entity: RawLibraryView = {
      id: id.trim(),
      workspace: workspaceId,
    };
    if (name.trim() !== '') {
      entity.name = name.trim();
    }
    if (path.trim() !== '') {
      entity.path = path.trim();
    }
    if (defaultBranch.trim() !== '') {
      entity.default_branch = defaultBranch.trim();
    }
    if (type.trim() !== '') {
      entity.type = type.trim();
    }
    if (repositoryUrl.trim() !== '') {
      entity.repository = { url: repositoryUrl.trim() };
    }
    if (groupId.trim() !== '' || artifactId.trim() !== '' || version.trim() !== '') {
      entity.artifact = { group_id: groupId.trim(), artifact_id: artifactId.trim(), version: version.trim() };
    }
    const build = toCommand(buildCommand);
    if (build) {
      entity.commands = { build };
    }
    if (Object.keys(environment).length > 0) {
      entity.environment = environment;
    }
    onSave(entity);
  };

  return (
    <Dialog
      title={editing ? `Edit library “${initial.id}”` : 'Add library'}
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
              placeholder="shared-kit"
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
              placeholder="shared-kit"
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Workspace</span>
            <input className="input" value={workspaceId} readOnly />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Type</span>
            <select
              className="input input--select"
              value={type}
              onChange={(event) => setType(event.target.value)}
            >
              <option value="maven">maven</option>
              <option value="npm">npm</option>
            </select>
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Path (relative to root)</span>
            <input
              className="input"
              value={path}
              placeholder="libraries/shared-kit"
              onChange={(event) => setPath(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Default branch</span>
            <input
              className="input"
              value={defaultBranch}
              placeholder="main"
              onChange={(event) => setDefaultBranch(event.target.value)}
            />
          </label>
        </div>

        <fieldset className="config-form__group">
          <legend className="toolbar__field-label">Repository</legend>
          <label className="config-form__field">
            <span className="toolbar__field-label">Clone URL</span>
            <input
              className="input"
              value={repositoryUrl}
              placeholder="https://example.com/shared-kit.git"
              onChange={(event) => setRepositoryUrl(event.target.value)}
            />
          </label>
        </fieldset>

        <fieldset className="config-form__group">
          <legend className="toolbar__field-label">Artifact coordinate</legend>
          <div className="config-form__grid">
            <label className="config-form__field">
              <span className="toolbar__field-label">Group id</span>
              <input
                className="input"
                value={groupId}
                placeholder="com.demo"
                onChange={(event) => setGroupId(event.target.value)}
              />
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Artifact id</span>
              <input
                className="input"
                value={artifactId}
                placeholder="shared-kit"
                onChange={(event) => setArtifactId(event.target.value)}
              />
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Version</span>
              <input
                className="input"
                value={version}
                placeholder="1.8.0-SNAPSHOT"
                onChange={(event) => setVersion(event.target.value)}
              />
            </label>
          </div>
          <p className="config-form__hint muted">
            The declared version is authoritative. The daemon never rewrites a
            POM or bumps a version automatically (`tasks/023.md` §30).
          </p>
        </fieldset>

        <label className="config-form__field">
          <span className="toolbar__field-label">Build command</span>
          <input
            className="input"
            value={buildCommand}
            placeholder="./mvnw clean install"
            onChange={(event) => setBuildCommand(event.target.value)}
          />
        </label>

        <EnvironmentEditor value={environment} onChange={setEnvironment} />
      </div>
    </Dialog>
  );
}
