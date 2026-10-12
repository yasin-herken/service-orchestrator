import { PAGES, type PageId } from '../types/nav';

/**
 * The application sidebar navigation.
 *
 * Keyboard-first: entries are native buttons with `aria-current` on the active
 * page (`tasks/018.md` §14, §15).
 */
export function Sidebar({
  activePage,
  onNavigate,
}: {
  activePage: PageId;
  onNavigate: (page: PageId) => void;
}) {
  return (
    <aside className="sidebar">
      <div className="sidebar__brand">
        <span className="sidebar__brand-mark" aria-hidden="true" />
        <span className="sidebar__brand-text">
          Service
          <br />
          Orchestrator
        </span>
      </div>
      <nav className="sidebar__nav" aria-label="Main navigation">
        {PAGES.map((page) => (
          <button
            key={page.id}
            type="button"
            className={`sidebar__item${page.id === activePage ? ' sidebar__item--active' : ''}`}
            aria-current={page.id === activePage ? 'page' : undefined}
            onClick={() => onNavigate(page.id)}
          >
            {page.label}
          </button>
        ))}
      </nav>
    </aside>
  );
}
