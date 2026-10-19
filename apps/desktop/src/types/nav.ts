/** Identifies a top-level screen in the application shell. */
export type PageId =
  | 'control-center'
  | 'dashboard'
  | 'services'
  | 'git'
  | 'builds'
  | 'libraries'
  | 'migrations'
  | 'workflows'
  | 'bulk-operations'
  | 'bulk-import'
  | 'templates'
  | 'tasks'
  | 'logs'
  | 'health'
  | 'configuration'
  | 'settings';

/**
 * A cross-screen navigation payload (`tasks/021.md` §30, `tasks/022.md` §6).
 *
 * It carries stable identifiers only — a task id, a service id, or the id of a
 * profile/group to preselect — so a service, a profile, or a group can open
 * the workflow planner with the right selection without any screen parsing
 * anything or duplicating workflow logic.
 */
export interface NavIntent {
  /** The task the target screen should open. */
  taskId?: string;
  /** The service the target screen should select. */
  serviceId?: string;
  /** Services to preselect on the Workflows screen. */
  workflowServiceIds?: string[];
  /** The profile to preselect on the Workflows screen. */
  workflowProfileId?: string;
  /** The group to preselect on the Workflows screen. */
  workflowGroupId?: string;
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
  {
    id: 'control-center',
    label: 'Workspace Control Center',
    title: 'Workspace Control Center',
  },
  { id: 'dashboard', label: 'Dashboard', title: 'Dashboard' },
  { id: 'services', label: 'Services', title: 'Services' },
  { id: 'git', label: 'Git', title: 'Git' },
  { id: 'builds', label: 'Builds', title: 'Builds' },
  { id: 'libraries', label: 'Libraries', title: 'Libraries' },
  { id: 'migrations', label: 'Migrations', title: 'Migrations' },
  { id: 'workflows', label: 'Workflows', title: 'Workflows' },
  { id: 'bulk-operations', label: 'Bulk Operations', title: 'Bulk Operations' },
  { id: 'bulk-import', label: 'Bulk Import', title: 'Bulk Import' },
  { id: 'templates', label: 'Templates', title: 'Templates' },
  { id: 'tasks', label: 'Tasks', title: 'Tasks' },
  { id: 'logs', label: 'Logs', title: 'Logs' },
  { id: 'health', label: 'Health', title: 'Health' },
  { id: 'configuration', label: 'Configuration', title: 'Configuration' },
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
