import { useWorkspaces } from '../context/WorkspaceContext';

/**
 * The workspace selector shown in the header.
 *
 * The selected workspace scopes every service view (`tasks/018.md` §18).
 */
export function WorkspaceSelector() {
  const { workspaces, activeWorkspaceId, selectWorkspace, loading } = useWorkspaces();

  if (workspaces.length === 0 && !loading) {
    return null;
  }

  return (
    <label className="workspace-selector">
      <span className="workspace-selector__label">Workspace</span>
      <select
        className="workspace-selector__select"
        value={activeWorkspaceId ?? ''}
        onChange={(event) => selectWorkspace(event.target.value)}
        disabled={loading || workspaces.length === 0}
        aria-label="Active workspace"
      >
        {workspaces.length === 0 ? (
          <option value="">No workspaces</option>
        ) : (
          workspaces.map((workspace) => (
            <option key={workspace.id} value={workspace.id}>
              {workspace.name}
            </option>
          ))
        )}
      </select>
    </label>
  );
}
