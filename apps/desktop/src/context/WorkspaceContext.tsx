import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from 'react';

import { useConnection } from './ConnectionContext';
import { useAsync } from '../hooks/useAsync';
import { desktopApi } from '../services/desktopApi';
import type { DesktopError, WorkspaceSummary } from '../types/models';

/** The value provided by [`WorkspaceProvider`]. */
export interface WorkspaceContextValue {
  /** Every configured workspace. */
  workspaces: WorkspaceSummary[];
  loading: boolean;
  error: DesktopError | null;
  /** The workspace whose services are shown, or `null` before the first load. */
  activeWorkspaceId: string | null;
  /** The active workspace's summary, or `null`. */
  activeWorkspace: WorkspaceSummary | null;
  /** Selects the workspace that scopes service views. */
  selectWorkspace: (workspaceId: string) => void;
  /** Reloads the workspace list (and the daemon connection first, if lost). */
  reload: () => void;
}

const WorkspaceContext = createContext<WorkspaceContextValue | null>(null);

/**
 * Loads the configured workspaces and owns the active selection.
 *
 * The selection defaults to the first workspace, survives navigation because
 * the provider wraps the whole shell, and is re-validated after a reconnect
 * (`tasks/018.md` §18).
 */
export function WorkspaceProvider({ children }: { children: ReactNode }) {
  const { snapshot } = useConnection();
  const connected = snapshot.state === 'connected';
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Workspaces are loaded only while connected; `useAsync` refetches exactly
  // once on every (re)connect transition and keeps the last list meanwhile.
  const { data, loading, error, reload } = useAsync<WorkspaceSummary[]>(
    () => desktopApi.listWorkspaces(),
    [],
    { enabled: connected },
  );
  const workspaces = data ?? [];

  // Default to the first workspace; drop a selection that no longer exists.
  useEffect(() => {
    if (workspaces.length === 0) {
      return;
    }
    const stillExists = selectedId !== null && workspaces.some((w) => w.id === selectedId);
    if (!stillExists) {
      setSelectedId(workspaces[0].id);
    }
  }, [workspaces, selectedId]);

  const selectWorkspace = useCallback((workspaceId: string) => {
    setSelectedId(workspaceId);
  }, []);

  const activeWorkspace = useMemo(
    () => workspaces.find((w) => w.id === selectedId) ?? null,
    [workspaces, selectedId],
  );

  const value = useMemo<WorkspaceContextValue>(
    () => ({
      workspaces,
      loading,
      error,
      activeWorkspaceId: activeWorkspace ? activeWorkspace.id : null,
      activeWorkspace,
      selectWorkspace,
      reload,
    }),
    [workspaces, loading, error, activeWorkspace, selectWorkspace, reload],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

/** Reads the workspace list and active selection. Must be under the provider. */
export function useWorkspaces(): WorkspaceContextValue {
  const value = useContext(WorkspaceContext);
  if (!value) {
    throw new Error('useWorkspaces must be used within a WorkspaceProvider');
  }
  return value;
}
