import type { BuildCapabilitiesView, BuildOperationView } from '../../types/models';

/** Human-readable labels for the well-known build operations. */
const OPERATION_LABELS: Record<string, string> = {
  clean: 'Clean',
  build: 'Build',
  test: 'Test',
  install_dependencies: 'Install dependencies',
  publish_local_artifact: 'Publish local artifact',
};

/** A display label for a build operation, without hard-coding a provider. */
export function buildOperationLabel(operation: string): string {
  return OPERATION_LABELS[operation] ?? operation.replace(/_/g, ' ');
}

/** The operations a provider actually exposes for a service. */
export function exposedOperations(
  capability: BuildCapabilitiesView | null,
): BuildOperationView[] {
  if (!capability) {
    return [];
  }
  return capability.operations.filter((operation) => operation.exposed);
}

/**
 * Renders a build system's operations exactly as the Application Core reports
 * them: supported-but-not-exposed operations are shown disabled with the
 * core's own explanation, and unsupported ones say so (`tasks/020.md` §13, §14).
 */
export function BuildCapabilitiesList({
  capability,
}: {
  capability: BuildCapabilitiesView;
}) {
  if (capability.empty || capability.operations.length === 0) {
    return (
      <p className="muted">
        The {capability.build_system} provider reports no supported operations.
      </p>
    );
  }
  return (
    <ul className="capability-list">
      {capability.operations.map((operation) => {
        const state = operation.exposed
          ? { label: 'Available', tone: 'positive' }
          : operation.supported
            ? { label: 'Not exposed', tone: 'neutral' }
            : { label: 'Unsupported', tone: 'warning' };
        return (
          <li
            key={operation.operation}
            className={`capability${operation.exposed ? ' capability--exposed' : ''}`}
          >
            <span className="capability__name">
              {buildOperationLabel(operation.operation)}
            </span>
            <span className={`badge badge--${state.tone}`}>{state.label}</span>
            {operation.reason ? (
              <span className="capability__reason muted">{operation.reason}</span>
            ) : null}
          </li>
        );
      })}
    </ul>
  );
}
