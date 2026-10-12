import { pageDefinition, type PageId } from '../types/nav';

/**
 * A clearly labelled placeholder for screens that are not implemented yet.
 *
 * The GUI foundation task implements Dashboard and Services only; every other
 * route says so plainly instead of pretending to work (`tasks/018.md` §14).
 */
export function PlaceholderPage({ page }: { page: PageId }) {
  const definition = pageDefinition(page);
  return (
    <div className="page">
      <div className="state-block state-block--placeholder">
        <span className="badge badge--neutral">Not implemented</span>
        <h2 className="state-block__title">{definition.title}</h2>
        <p className="state-block__message">
          This screen is part of a later task and is intentionally not built yet.
          The GUI foundation implements Dashboard and Services only.
        </p>
      </div>
    </div>
  );
}
