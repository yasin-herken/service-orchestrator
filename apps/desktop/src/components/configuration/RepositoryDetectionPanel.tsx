import type {
  DetectedBuildSystemView,
  ProjectSuggestionView,
  ProviderAvailabilityView,
  RepositoryProbeView,
} from '../../types/models';

/** Maps a provider availability to a badge tone and label. */
function availabilityBadge(availability: ProviderAvailabilityView): {
  tone: string;
  label: string;
} {
  switch (availability) {
    case 'available':
      return { tone: 'positive', label: 'provider available' };
    case 'provider_not_installed':
      return { tone: 'warning', label: 'provider not installed' };
    default:
      return { tone: 'neutral', label: 'not applicable' };
  }
}

/** One detected build system. */
function BuildSystemRow({ system }: { system: DetectedBuildSystemView }) {
  const badge = availabilityBadge(system.availability);
  return (
    <li className="config-detect__row">
      <span className="config-detect__name">{system.label}</span>
      <span className={`badge badge--${badge.tone}`}>{badge.label}</span>
      <span className="muted config-detect__evidence">
        {system.evidence.join(', ')}
      </span>
      {system.note ? (
        <span className="config-detect__note muted">{system.note}</span>
      ) : null}
    </li>
  );
}

/**
 * The read-only detection summary for a repository (`tasks/023.md` §17–§22).
 *
 * It displays exactly what the daemon observed: build systems with provider
 * availability (a detected Gradle project is shown as *provider not installed*,
 * never as operational), runtimes, integrations, and any warnings. Nothing here
 * ever executes a build or a script.
 */
export function RepositoryDetectionPanel({
  probe,
  suggestion,
  loading,
  error,
  onApply,
  onInspect,
  onDetect,
}: {
  probe: RepositoryProbeView | null;
  suggestion: ProjectSuggestionView | null;
  loading: boolean;
  error: string | null;
  onApply: (suggestion: ProjectSuggestionView) => void;
  onInspect: () => void;
  onDetect: () => void;
}) {
  return (
    <section className="config-detect" aria-label="Repository detection">
      <div className="config-detect__actions">
        <button
          type="button"
          className="button button--small"
          onClick={onInspect}
          disabled={loading}
        >
          Inspect repository
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onDetect}
          disabled={loading}
        >
          Detect configuration
        </button>
        {loading ? <span className="muted">Inspecting…</span> : null}
      </div>

      {error ? (
        <p className="config-validation config-validation--error" role="alert">
          {error}
        </p>
      ) : null}

      {probe ? (
        <div className="config-detect__result">
          <p className="config-detect__summary">
            {probe.exists && probe.is_directory ? (
              <span className="badge badge--positive">Directory</span>
            ) : (
              <span className="badge badge--negative">Missing</span>
            )}
            {probe.is_git_repository ? (
              <span className="badge badge--info">Git</span>
            ) : null}
            <code>{probe.path}</code>
            {probe.branch ? <span className="muted"> on {probe.branch}</span> : null}
          </p>

          {probe.project_files && probe.project_files.length > 0 ? (
            <p className="muted">
              Project files: {probe.project_files.join(', ')}
            </p>
          ) : null}

          {probe.build_systems && probe.build_systems.length > 0 ? (
            <ul className="config-detect__list" aria-label="Detected build systems">
              {probe.build_systems.map((system) => (
                <BuildSystemRow key={system.system} system={system} />
              ))}
            </ul>
          ) : (
            <p className="muted">No build system was detected.</p>
          )}

          {probe.runtimes && probe.runtimes.length > 0 ? (
            <p className="config-detect__runtimes">
              Runtimes:{' '}
              {probe.runtimes
                .map(
                  (runtime) =>
                    `${runtime.label}${runtime.required_version ? ` ${runtime.required_version}` : ''}`,
                )
                .join(', ')}
            </p>
          ) : null}

          {probe.integrations && probe.integrations.length > 0 ? (
            <p className="config-detect__integrations">
              Integrations: {probe.integrations.map((i) => i.label).join(', ')}
            </p>
          ) : null}
        </div>
      ) : null}

      {suggestion ? (
        <div className="config-detect__suggestion">
          <p>
            <span className="badge badge--info">Suggested</span>{' '}
            {suggestion.service_type ?? 'service'}
            {suggestion.build_system ? ` · ${suggestion.build_system}` : ''}
            {suggestion.build_command
              ? ` · ${suggestion.build_command.program} ${suggestion.build_command.args.join(' ')}`
              : ''}
          </p>
          {suggestion.warnings && suggestion.warnings.length > 0 ? (
            <ul className="config-detect__warnings" aria-label="Detection warnings">
              {suggestion.warnings.map((warning, index) => (
                <li key={index} className="config-detect__warning">
                  <span className="badge badge--warning">Warning</span> {warning}
                </li>
              ))}
            </ul>
          ) : null}
          <button
            type="button"
            className="button button--small button--primary"
            onClick={() => onApply(suggestion)}
          >
            Apply suggestion
          </button>
        </div>
      ) : null}
    </section>
  );
}
