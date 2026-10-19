import { useState } from 'react';

import { Header } from '../components/Header';
import { Sidebar } from '../components/Sidebar';
import { BulkImportPage } from '../pages/BulkImportPage';
import { BulkOperationsPage } from '../pages/BulkOperationsPage';
import { BuildsPage } from '../pages/BuildsPage';
import { ConfigurationPage } from '../pages/ConfigurationPage';
import { ControlCenterPage } from '../pages/ControlCenterPage';
import { DashboardPage } from '../pages/DashboardPage';
import { GitPage } from '../pages/GitPage';
import { HealthPage } from '../pages/HealthPage';
import { LibrariesPage } from '../pages/LibrariesPage';
import { LogsPage } from '../pages/LogsPage';
import { MigrationsPage } from '../pages/MigrationsPage';
import { PlaceholderPage } from '../pages/PlaceholderPage';
import { ServicesPage } from '../pages/ServicesPage';
import { TasksPage } from '../pages/TasksPage';
import { TemplatesPage } from '../pages/TemplatesPage';
import { WorkflowsPage } from '../pages/WorkflowsPage';
import type { NavIntent, PageId } from '../types/nav';

/**
 * The desktop application shell: sidebar navigation, header, and the routed
 * content area. Navigation state lives here so the active page and workspace
 * selection survive screen changes (`tasks/018.md` §13, §14).
 *
 * Cross-screen navigation carries a small `NavIntent` (a task id or a service
 * id) so one screen can open another on the exact entity the user came from
 * without parsing anything (`tasks/021.md` §30).
 */
export function AppShell() {
  const [page, setPage] = useState<PageId>('control-center');
  const [intent, setIntent] = useState<NavIntent | null>(null);

  const navigate = (next: PageId, nextIntent?: NavIntent) => {
    setPage(next);
    setIntent(nextIntent ?? null);
  };

  const implemented: readonly PageId[] = [
    'control-center',
    'dashboard',
    'services',
    'git',
    'builds',
    'libraries',
    'migrations',
    'workflows',
    'bulk-operations',
    'bulk-import',
    'templates',
    'tasks',
    'logs',
    'health',
    'configuration',
  ];

  return (
    <div className="app-shell">
      <Sidebar activePage={page} onNavigate={(next) => navigate(next)} />
      <div className="app-shell__main">
        <Header page={page} />
        <main className="app-shell__content">
          {page === 'control-center' ? (
            <ControlCenterPage onNavigate={navigate} />
          ) : null}
          {page === 'dashboard' ? <DashboardPage onNavigate={navigate} /> : null}
          {page === 'services' ? <ServicesPage onNavigate={navigate} /> : null}
          {page === 'git' ? <GitPage /> : null}
          {page === 'builds' ? <BuildsPage /> : null}
          {page === 'libraries' ? <LibrariesPage /> : null}
          {page === 'migrations' ? (
            <MigrationsPage intent={intent} onNavigate={navigate} />
          ) : null}
          {page === 'workflows' ? (
            <WorkflowsPage intent={intent} onNavigate={navigate} />
          ) : null}
          {page === 'bulk-operations' ? (
            <BulkOperationsPage onNavigate={navigate} />
          ) : null}
          {page === 'bulk-import' ? <BulkImportPage onNavigate={navigate} /> : null}
          {page === 'templates' ? <TemplatesPage /> : null}
          {page === 'tasks' ? <TasksPage intent={intent} onNavigate={navigate} /> : null}
          {page === 'logs' ? <LogsPage intent={intent} /> : null}
          {page === 'health' ? <HealthPage intent={intent} onNavigate={navigate} /> : null}
          {page === 'configuration' ? <ConfigurationPage onNavigate={navigate} /> : null}
          {!implemented.includes(page) ? <PlaceholderPage page={page} /> : null}
        </main>
      </div>
    </div>
  );
}
