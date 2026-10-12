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
 * Only `dashboard` and `services` have functional content in this task; the
 * rest render clearly labelled placeholders (`tasks/018.md` §14).
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
