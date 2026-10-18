import type { MigrationPresetSummaryView } from '../../types/models';
import { migrationEnvironmentLabel } from '../../types/models';
import type { OperationOptionsValue, RuntimeAction } from './model';

const BUILD_OPERATIONS = [
  'clean',
  'build',
  'test',
  'install_dependencies',
  'publish_local_artifact',
];

const RUNTIME_ACTIONS: { value: RuntimeAction; label: string; hint: string }[] = [
  { value: 'none', label: 'Do not change runtime state', hint: 'Nothing is started or stopped.' },
  { value: 'start', label: 'Start services', hint: 'Starts only the selected services.' },
  {
    value: 'restart',
    label: 'Restart services',
    hint: 'Stops in reverse dependency order, then builds and starts again.',
  },
  { value: 'stop', label: 'Stop services', hint: 'Stops only the selected services.' },
];

/**
 * The operation options (`tasks/032.md` §24–§29).
 *
 * Every control maps onto an existing `PrepareEnvironmentRequest` field; no
 * option exists that the daemon would silently ignore. Library install,
 * verification, and alignment stay on the Libraries screen, which uses the
 * dedicated library manager rather than a second workflow.
 */
export function OperationOptions({
  value,
  presets,
  destructivePreset,
  disabled = false,
  onChange,
  onOpenLibraries,
}: {
  value: OperationOptionsValue;
  presets: readonly MigrationPresetSummaryView[];
  destructivePreset: MigrationPresetSummaryView | undefined;
  disabled?: boolean;
  onChange: (next: OperationOptionsValue) => void;
  onOpenLibraries?: () => void;
}) {
  const patch = (next: Partial<OperationOptionsValue>) => onChange({ ...value, ...next });
  const runtimeActive = value.runtime === 'start' || value.runtime === 'restart';

  return (
    <div className="operation-options">
      <fieldset className="options-form">
        <legend className="selection-list__legend">Git</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.sync}
              disabled={disabled}
              onChange={(event) => patch({ sync: event.target.checked })}
              aria-label="Synchronize repositories"
            />{' '}
            Fetch and fast-forward
          </span>
          <span className="muted">
            Local changes are never discarded; no branch is checked out unless one is set.
          </span>
        </label>
        <label className="options-form__field">
          <span>Target branch (optional)</span>
          <input
            className="input"
            value={value.branch}
            disabled={disabled || !value.sync}
            onChange={(event) => patch({ branch: event.target.value })}
            aria-label="Target branch"
            placeholder="Keep each service’s current branch"
          />
          <span className="muted">
            Leave empty to keep the configured or current branch.
          </span>
        </label>
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Libraries</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.buildLibraries}
              disabled={disabled}
              onChange={(event) => patch({ buildLibraries: event.target.checked })}
              aria-label="Build required libraries"
            />{' '}
            Build required libraries
          </span>
          <span className="muted">
            Builds the dependency closure before service builds.
          </span>
        </label>
        {onOpenLibraries ? (
          <p className="options-form__field">
            <button type="button" className="button button--small" onClick={onOpenLibraries}>
              Install, verify, and align libraries…
            </button>
          </p>
        ) : null}
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Migrations</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.runLiquibase}
              disabled={disabled}
              onChange={(event) => patch({ runLiquibase: event.target.checked })}
              aria-label="Run migrations"
            />{' '}
            Run Liquibase migrations
          </span>
          <span className="muted">
            Applies only to services with migrations enabled. The daemon authorizes every
            destructive target.
          </span>
        </label>
        <label className="options-form__field">
          <span>Migration preset</span>
          <select
            className="input input--select"
            value={value.migrationPreset}
            disabled={disabled || !value.runLiquibase}
            onChange={(event) => patch({ migrationPreset: event.target.value })}
            aria-label="Migration preset"
          >
            <option value="">Each service’s configured preset</option>
            {presets.map((preset) => (
              <option key={preset.id} value={preset.id}>
                {preset.name}
                {preset.read_only ? ' (read-only)' : ''}
                {preset.destructive ? ' (destructive)' : ''}
              </option>
            ))}
          </select>
        </label>
        {destructivePreset ? (
          <div className="config-validation config-validation--error" role="alert">
            <p>
              <span className="badge badge--negative">Destructive</span>{' '}
              “{destructivePreset.name}” drops database objects. It is allowed only for{' '}
              {destructivePreset.allowed_environments
                .map(migrationEnvironmentLabel)
                .join(', ') || 'no declared environment'}
              , and only when each service declares a matching database environment. The
              daemon re-validates the target and requires explicit authorization.
            </p>
            <p className="muted">
              Contexts: {destructivePreset.contexts.join(', ') || 'the service’s contexts'}
            </p>
          </div>
        ) : null}
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Build</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.build}
              disabled={disabled}
              onChange={(event) => patch({ build: event.target.checked })}
              aria-label="Build services"
            />{' '}
            Build services
          </span>
        </label>
        <label className="options-form__field">
          <span>Build operation</span>
          <select
            className="input input--select"
            value={value.buildOperation}
            disabled={disabled || !value.build}
            onChange={(event) => patch({ buildOperation: event.target.value })}
            aria-label="Build operation"
          >
            <option value="">Each service’s configured operation</option>
            {BUILD_OPERATIONS.map((operation) => (
              <option key={operation} value={operation}>
                {operation.replace(/_/g, ' ')}
              </option>
            ))}
          </select>
          <span className="muted">
            The daemon rejects an operation the provider does not support.
          </span>
        </label>
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Runtime</legend>
        {RUNTIME_ACTIONS.map((action) => (
          <label key={action.value} className="options-form__field options-form__field--indent">
            <span>
              <input
                type="radio"
                name="operation-runtime"
                checked={value.runtime === action.value}
                disabled={disabled}
                onChange={() => patch({ runtime: action.value })}
                aria-label={action.label}
              />{' '}
              {action.label}
            </span>
            <span className="muted">{action.hint}</span>
          </label>
        ))}
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Health</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.waitForHealth}
              disabled={disabled || !runtimeActive}
              onChange={(event) => patch({ waitForHealth: event.target.checked })}
              aria-label="Wait for readiness"
            />{' '}
            Wait for readiness
          </span>
          <span className="muted">
            Runs the configured health checks after startup.
          </span>
        </label>
      </fieldset>
    </div>
  );
}
