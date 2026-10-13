/** Identifies a top-level screen in the application shell. */
export type PageId =
  | 'dashboard'
  | 'services'
  | 'git'
  | 'builds'
  | 'libraries'
  | 'migrations'
  | 'tasks'
  | 'logs'
  | 'health'
  | 'settings';

/**
 * A cross-screen navigation payload (`tasks/021.md` §30).
 *
 * It carries stable identifiers only — a task id or a service id — so a failed
 * migration can open its task, a failed task can open its logs, and an
 * unhealthy service can open its health detail without parsing anything.
 */
export interface NavIntent {
  /** The task the target screen should open. */
  taskId?: string;
  /** The service the target screen should select. */
  serviceId?: string;
}

/** One navigation entry in the sidebar. */
export interface PageDefinition {
  id: PageId;
  label: string;
  /** Header title for the page. */
  title: string;
}

/**
 * The navigation model, shared by the sidebar and the shell.
 *
 * `settings` still renders a placeholder; every other screen has real content
 * backed by the shared daemon (`tasks/018.md` §14, `tasks/021.md` §30).
 */
export const PAGES: readonly PageDefinition[] = [
  { id: 'dashboard', label: 'Dashboard', title: 'Dashboard' },
  { id: 'services', label: 'Services', title: 'Services' },
  { id: 'git', label: 'Git', title: 'Git' },
  { id: 'builds', label: 'Builds', title: 'Builds' },
  { id: 'libraries', label: 'Libraries', title: 'Libraries' },
  { id: 'migrations', label: 'Migrations', title: 'Migrations' },
  { id: 'tasks', label: 'Tasks', title: 'Tasks' },
  { id: 'logs', label: 'Logs', title: 'Logs' },
  { id: 'health', label: 'Health', title: 'Health' },
  { id: 'settings', label: 'Settings', title: 'Settings' },
] as const;

/** Returns the definition for a page id. */
export function pageDefinition(id: PageId): PageDefinition {
  const found = PAGES.find((page) => page.id === id);
  if (!found) {
    throw new Error(`unknown page: ${id}`);
  }
  return found;
}
