import type { EnvironmentPlanView } from '../../types/models';

/**
 * A readable dependency view over the daemon's plan (`tasks/022.md` §14).
 *
 * The order and membership come from `plan.resolved_services` and
 * `plan.included_dependencies`; React never computes a dependency graph. Each
 * automatically included service is marked so the user understands why it is
 * there.
 */
export function DependencyView({ plan }: { plan: EnvironmentPlanView }) {
  const included = new Set(plan.included_dependencies);
  const selected = new Set(plan.selected_services);

  return (
    <section className="dependency-view" aria-label="Dependency resolution">
      <h3 className="dependency-view__title">Startup order and dependencies</h3>
      <ol className="dependency-view__list">
        {plan.resolved_services.map((serviceId, index) => (
          <li key={serviceId} className="dependency-view__item">
            <span className="dependency-view__order">{index + 1}.</span>
            <span className="dependency-view__name">{serviceId}</span>
            <span className="muted">
              {included.has(serviceId) && !selected.has(serviceId)
                ? 'required dependency (added automatically)'
                : 'explicitly selected'}
            </span>
          </li>
        ))}
      </ol>
      {plan.required_libraries.length > 0 ? (
        <>
          <h3 className="dependency-view__title">Required libraries</h3>
          <ol className="dependency-view__list">
            {plan.required_libraries.map((libraryId) => (
              <li key={libraryId} className="dependency-view__item">
                <span className="dependency-view__name">{libraryId}</span>
                <span className="muted">prepared before dependent builds</span>
              </li>
            ))}
          </ol>
        </>
      ) : null}
    </section>
  );
}
