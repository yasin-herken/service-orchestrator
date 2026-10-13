import { useState } from 'react';

import { Header } from '../components/Header';
import { Sidebar } from '../components/Sidebar';
import { BuildsPage } from '../pages/BuildsPage';
import { DashboardPage } from '../pages/DashboardPage';
import { GitPage } from '../pages/GitPage';
import { LibrariesPage } from '../pages/LibrariesPage';
import { PlaceholderPage } from '../pages/PlaceholderPage';
import { ServicesPage } from '../pages/ServicesPage';
import type { PageId } from '../types/nav';

/**
 * The desktop application shell: sidebar navigation, header, and the routed
 * content area. Navigation state lives here so the active page and workspace
 * selection survive screen changes (`tasks/018.md` §13, §14).
 */
export function AppShell() {
  const [page, setPage] = useState<PageId>('dashboard');

  const implemented: readonly PageId[] = [
    'dashboard',
    'services',
    'git',
    'builds',
    'libraries',
  ];

  return (
    <div className="app-shell">
      <Sidebar activePage={page} onNavigate={setPage} />
      <div className="app-shell__main">
        <Header page={page} />
        <main className="app-shell__content">
          {page === 'dashboard' ? <DashboardPage /> : null}
          {page === 'services' ? <ServicesPage onNavigate={setPage} /> : null}
          {page === 'git' ? <GitPage /> : null}
          {page === 'builds' ? <BuildsPage /> : null}
          {page === 'libraries' ? <LibrariesPage /> : null}
          {!implemented.includes(page) ? <PlaceholderPage page={page} /> : null}
        </main>
      </div>
    </div>
  );
}
