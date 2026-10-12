import { ConnectionProvider } from './context/ConnectionContext';
import { TaskEventsProvider } from './context/TaskEventsContext';
import { WorkspaceProvider } from './context/WorkspaceContext';
import { AppShell } from './layouts/AppShell';

/**
 * The application root: wires the daemon-connected providers around the shell.
 *
 * React StrictMode is deliberately not used — its double-invoked effects would
 * issue duplicate IPC requests on every mount (`tasks/018.md` §21).
 */
export default function App() {
  return (
    <ConnectionProvider>
      <WorkspaceProvider>
        <TaskEventsProvider>
          <AppShell />
        </TaskEventsProvider>
      </WorkspaceProvider>
    </ConnectionProvider>
  );
}
