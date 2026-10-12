import { ConnectionStatus } from './ConnectionStatus';
import { WorkspaceSelector } from './WorkspaceSelector';
import { pageDefinition, type PageId } from '../types/nav';

/** The application header: page title, workspace selector, connection state. */
export function Header({ page }: { page: PageId }) {
  const definition = pageDefinition(page);
  return (
    <header className="header">
      <div className="header__title-block">
        <h1 className="header__title">{definition.title}</h1>
      </div>
      <div className="header__actions">
        <WorkspaceSelector />
        <ConnectionStatus />
      </div>
    </header>
  );
}
