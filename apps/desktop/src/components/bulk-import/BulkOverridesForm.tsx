import type { MigrationPresetSummaryView } from '../../types/models';
import { migrationEnvironmentLabel } from '../../types/models';
import type { BulkOverridesDraft } from './draft';

/** The environments a destructive preset may be declared against. */
const DATABASE_ENVIRONMENTS = [
  'local',
  'shared_development',
  'staging',
  'production',
  'unknown',
] as const;

/** The build operations the daemon accepts. */
const BUILD_OPERATIONS = [
  'clean',
  'build',
  'test',
  'install_dependencies',
  'publish_local_artifact',
];

/**
 * Shared bulk overrides (`tasks/032.md` §10 step 6, §11).
 *
 * These values apply to every selected repository and are merged by the daemon
 * with the template and the per-repository exceptions. A destructive migration
 * preset is called out explicitly: it still requires the daemon's
 * authorization at execution time and is never enabled implicitly.
 */
export function BulkOverridesForm({
  value,
  presets,
  portBase,
  disabled = false,
  onChange,
  onPortBase,
}: {
  value: BulkOverridesDraft;
  presets: readonly MigrationPresetSummaryView[];
  portBase: string;
  disabled?: boolean;
  onChange: (next: BulkOverridesDraft) => void;
  onPortBase: (value: string) => void;
}) {
  const destructivePreset = presets.find(
    (preset) => preset.id === value.migrationPreset && preset.destructive,
  );

  const patch = (next: Partial<BulkOverridesDraft>) => onChange({ ...value, ...next });

  return (
    <fieldset className="options-form">
      <legend className="selection-list__legend">Shared overrides</legend>
      <p className="muted">
        Leave a field empty to keep the template or detected value. Overrides never bypass
        validation.
      </p>

      <label className="options-form__field">
        <span>Service type</span>
        <select
          className="input input--select"
          value={value.serviceType}
          disabled={disabled}
          onChange={(event) => patch({ serviceType: event.target.value })}
          aria-label="Bulk service type"
        >
          <option value="">From template or detection</option>
          <option value="backend">Backend</option>
          <option value="frontend">Frontend</option>
          <option value="library">Shared library</option>
        </select>
      </label>

      <label className="options-form__field">
        <span>Default branch</span>
        <input
          className="input"
          value={value.defaultBranch}
          disabled={disabled}
          onChange={(event) => patch({ defaultBranch: event.target.value })}
          aria-label="Bulk default branch"
          placeholder="From template or the detected branch"
        />
        <span className="muted">
          No branch defaults to master. Variables such as {'${serviceName}'} are allowed.
        </span>
      </label>

      <label className="options-form__field">
        <span>Build system</span>
        <select
          className="input input--select"
          value={value.buildSystem}
          disabled={disabled}
          onChange={(event) => patch({ buildSystem: event.target.value })}
          aria-label="Bulk build system"
        >
          <option value="">From template or detection</option>
          <option value="maven">Maven</option>
          <option value="npm">npm</option>
        </select>
      </label>

      <label className="options-form__field">
        <span>Build operation</span>
        <select
          className="input input--select"
          value={value.buildOperation}
          disabled={disabled}
          onChange={(event) => patch({ buildOperation: event.target.value })}
          aria-label="Bulk build operation"
        >
          <option value="">From template</option>
          {BUILD_OPERATIONS.map((operation) => (
            <option key={operation} value={operation}>
              {operation.replace(/_/g, ' ')}
            </option>
          ))}
        </select>
      </label>

      <label className="options-form__field">
        <span>Port base</span>
        <input
          className="input"
          value={portBase}
          disabled={disabled}
          onChange={(event) => onPortBase(event.target.value)}
          aria-label="Port base"
          inputMode="numeric"
          placeholder="8080"
        />
        <span className="muted">
          Ports are assigned sequentially in path order, skipping ports already in use.
          A per-repository port always wins.
        </span>
      </label>

      <label className="options-form__field">
        <span>Migrations</span>
        <select
          className="input input--select"
          value={value.migration}
          disabled={disabled}
          onChange={(event) =>
            patch({ migration: event.target.value as BulkOverridesDraft['migration'] })
          }
          aria-label="Bulk migration setting"
        >
          <option value="">From template</option>
          <option value="true">Enabled</option>
          <option value="false">Disabled</option>
        </select>
      </label>

      <label className="options-form__field">
        <span>Migration preset</span>
        <select
          className="input input--select"
          value={value.migrationPreset}
          disabled={disabled}
          onChange={(event) => patch({ migrationPreset: event.target.value })}
          aria-label="Bulk migration preset"
        >
          <option value="">Each service’s preset</option>
          {presets.map((preset) => (
            <option key={preset.id} value={preset.id}>
              {preset.name}
              {preset.read_only ? ' (read-only)' : ''}
              {preset.destructive ? ' (destructive)' : ''}
            </option>
          ))}
        </select>
      </label>

      <label className="options-form__field">
        <span>Declared database environment</span>
        <select
          className="input input--select"
          value={value.databaseEnvironment}
          disabled={disabled}
          onChange={(event) => patch({ databaseEnvironment: event.target.value })}
          aria-label="Bulk database environment"
        >
          <option value="">Not declared</option>
          {DATABASE_ENVIRONMENTS.map((environment) => (
            <option key={environment} value={environment}>
              {migrationEnvironmentLabel(environment)}
            </option>
          ))}
        </select>
        <span className="muted">
          Never inferred. A destructive preset requires a declared environment in its
          allowed list.
        </span>
      </label>

      {destructivePreset ? (
        <div className="config-validation config-validation--error" role="alert">
          <p>
            <span className="badge badge--negative">Destructive</span>{' '}
            “{destructivePreset.name}” drops database objects. It may target only{' '}
            {destructivePreset.allowed_environments
              .map(migrationEnvironmentLabel)
              .join(', ') || 'no declared environment'}
            , and the daemon requires explicit per-run authorization.
          </p>
          <p className="muted">
            Contexts: {destructivePreset.contexts.join(', ') || 'the service’s contexts'}
          </p>
        </div>
      ) : null}

      <label className="options-form__field">
        <span>Health check URL</span>
        <input
          className="input"
          value={value.healthUrl}
          disabled={disabled}
          onChange={(event) => patch({ healthUrl: event.target.value })}
          aria-label="Bulk health check URL"
          placeholder="http://localhost:${port}/actuator/health"
        />
        <span className="muted">
          Replaces every template health check with one HTTP check. {'${port}'} resolves to
          the assigned port.
        </span>
      </label>

      <label className="options-form__field">
        <span>Build command</span>
        <input
          className="input"
          value={value.buildCommand}
          disabled={disabled}
          onChange={(event) => patch({ buildCommand: event.target.value })}
          aria-label="Bulk build command"
          placeholder="Leave empty to use the template"
        />
      </label>

      <label className="options-form__field">
        <span>Start command</span>
        <input
          className="input"
          value={value.startCommand}
          disabled={disabled}
          onChange={(event) => patch({ startCommand: event.target.value })}
          aria-label="Bulk start command"
          placeholder="Leave empty to use the template"
        />
      </label>
    </fieldset>
  );
}
