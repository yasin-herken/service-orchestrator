import { migrationEnvironmentLabel } from '../../types/models';
import type { MigrationPresetSummaryView } from '../../types/models';
import {
  buildOperationLabel,
  hasGitOperation,
  type ControlCenterOptions,
  type ControlCenterRuntimeAction,
} from './model';

const RUNTIME_ACTIONS: {
  value: ControlCenterRuntimeAction;
  label: string;
  hint: string;
}[] = [
  {
    value: 'none',
    label: 'Do not change runtime state',
    hint: 'Nothing is started, stopped, or restarted.',
  },
  {
    value: 'start',
    label: 'Start services',
    hint: 'Starts the selected services that are not already running.',
  },
  {
    value: 'restart',
    label: 'Restart services',
    hint: 'Stops in reverse dependency order, then starts again.',
  },
  {
    value: 'stop',
    label: 'Stop services',
    hint: 'Stops only the selected services; completed work is preserved.',
  },
];

/**
 * Inline operation configuration for the Workspace Control Center
 * (`tasks/033.md` §5–§10).
 *
 * Every control maps one-to-one onto an existing `PrepareEnvironmentRequest`
 * field. Controls that are irrelevant to the current operation are hidden
 * rather than faked, and build operations come from the daemon's provider
 * capabilities instead of assuming Maven or npm.
 */
export function OperationControls({
  value,
  presets,
  destructivePreset,
  buildOperations,
  defaultBranches,
  disabled = false,
  onChange,
  onOpenAdvanced,
  onOpenLibraries,
}: {
  value: ControlCenterOptions;
  presets: readonly MigrationPresetSummaryView[];
  destructivePreset: MigrationPresetSummaryView | undefined;
  buildOperations: readonly string[];
  defaultBranches: readonly string[];
  disabled?: boolean;
  onChange: (next: ControlCenterOptions) => void;
  onOpenAdvanced: () => void;
  onOpenLibraries: () => void;
}) {
  const patch = (next: Partial<ControlCenterOptions>) =>
    onChange({ ...value, ...next });
  const runtimeActive =
    value.runtime === 'start' || value.runtime === 'restart';
  const gitSelected = hasGitOperation(value);
  const selectedBuildOperation =
    value.buildOperation !== '' && !buildOperations.includes(value.buildOperation);

  return (
    <div className="operation-options control-center__operations">
      <fieldset className="options-form">
        <legend className="selection-list__legend">Git sync</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.sync}
              disabled={disabled}
              onChange={(event) => patch({ sync: event.target.checked })}
              aria-label="Git sync"
            />{' '}
            Synchronize repositories
          </span>
          <span className="muted">
            Fetch, check out the target branch, and fast-forward. Local changes
            are never discarded and no reset or clean runs automatically.
          </span>
        </label>
        {value.sync ? (
          <>
            <div className="control-center__inline-toggles">
              <label>
                <input
                  type="checkbox"
                  checked={value.gitFetch}
                  disabled={disabled}
                  onChange={(event) => patch({ gitFetch: event.target.checked })}
                  aria-label="Git fetch"
                />{' '}
                Fetch
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={value.gitCheckout}
                  disabled={disabled}
                  onChange={(event) =>
                    patch({ gitCheckout: event.target.checked })
                  }
                  aria-label="Git checkout"
                />{' '}
                Checkout
              </label>
              <label>
                <input
                  type="checkbox"
                  checked={value.gitPull}
                  disabled={disabled}
                  onChange={(event) => patch({ gitPull: event.target.checked })}
                  aria-label="Fast-forward pull"
                />{' '}
                Fast-forward pull
              </label>
            </div>
            {!gitSelected ? (
              <p className="control-center__inline-warning" role="status">
                Git sync is enabled but fetch, checkout, and pull are all off, so
                no Git step would be scheduled.
              </p>
            ) : null}
            <label className="options-form__field">
              <span>Target branch (optional)</span>
              <input
                className="input"
                value={value.branch}
                disabled={disabled || !value.gitCheckout}
                onChange={(event) => patch({ branch: event.target.value })}
                aria-label="Target branch"
                placeholder="Use each service's configured default branch"
              />
              <span className="muted">
                {defaultBranches.length === 0
                  ? 'Select services to see their configured default branches.'
                  : defaultBranches.length === 1
                    ? `Configured default: ${defaultBranches[0]}. Leave empty to use it.`
                    : `Configured defaults: ${defaultBranches.join(', ')}. Leave empty to keep each service's own default.`}
              </span>
            </label>
          </>
        ) : null}
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Libraries</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.buildLibraries}
              disabled={disabled}
              onChange={(event) =>
                patch({ buildLibraries: event.target.checked })
              }
              aria-label="Build required libraries"
            />{' '}
            Build &amp; install required libraries
          </span>
          <span className="muted">
            Builds the resolved dependency closure before service builds. Library
            install, verification, and alignment are on-demand tools.
          </span>
        </label>
        <p className="options-form__field">
          <button
            type="button"
            className="button button--small"
            disabled={disabled}
            onClick={onOpenLibraries}
          >
            Library tools…
          </button>
        </p>
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Liquibase</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.runLiquibase}
              disabled={disabled}
              onChange={(event) => {
                const enabled = event.target.checked;
                patch({
                  runLiquibase: enabled,
                  migrationPreset: enabled ? value.migrationPreset : '',
                });
              }}
              aria-label="Run Liquibase migrations"
            />{' '}
            Run Liquibase migrations
          </span>
          <span className="muted">
            Applies only to services with migrations enabled. The daemon verifies
            every target and requires authorization for destructive presets.
          </span>
        </label>
        {value.runLiquibase ? (
          <>
            <label className="options-form__field">
              <span>Migration preset</span>
              <select
                className="input input--select"
                value={value.migrationPreset}
                disabled={disabled}
                onChange={(event) =>
                  patch({ migrationPreset: event.target.value })
                }
                aria-label="Liquibase preset"
              >
                <option value="">Each service&apos;s configured preset</option>
                {presets.map((preset) => (
                  <option key={preset.id} value={preset.id}>
                    {preset.name}
                    {preset.read_only ? ' (read-only)' : ''}
                    {preset.destructive ? ' (destructive)' : ''}
                  </option>
                ))}
              </select>
            </label>
            {value.migrationPreset !== '' ? (
              <p className="muted">
                Contexts:{' '}
                {presets.find((preset) => preset.id === value.migrationPreset)
                  ?.contexts.join(', ') || 'the service’s configured contexts'}
              </p>
            ) : null}
            {destructivePreset ? (
              <div className="config-validation config-validation--error" role="alert">
                <p>
                  <span className="badge badge--negative">Destructive</span>{' '}
                  “{destructivePreset.name}” drops database objects. It is allowed
                  only for{' '}
                  {destructivePreset.allowed_environments
                    .map(migrationEnvironmentLabel)
                    .join(', ') || 'no declared environment'}
                  , and only when each target declares a matching database
                  environment. The daemon re-validates the target and requires
                  explicit interactive authorization. Destructive migrations are
                  never retried automatically.
                </p>
                <p className="muted">
                  Contexts:{' '}
                  {destructivePreset.contexts.join(', ') ||
                    'the service’s configured contexts'}
                </p>
              </div>
            ) : null}
          </>
        ) : null}
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Build services</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.build}
              disabled={disabled}
              onChange={(event) => patch({ build: event.target.checked })}
              aria-label="Build services"
            />{' '}
            Build selected services
          </span>
          <span className="muted">
            The daemon uses each service&apos;s configured build provider and
            rejects an operation it does not support.
          </span>
        </label>
        {value.build ? (
          <label className="options-form__field">
            <span>Build operation</span>
            <select
              className="input input--select"
              value={value.buildOperation}
              disabled={disabled || buildOperations.length === 0}
              onChange={(event) =>
                patch({ buildOperation: event.target.value })
              }
              aria-label="Build operation"
            >
              <option value="">Each service&apos;s configured operation</option>
              {buildOperations.map((operation) => (
                <option key={operation} value={operation}>
                  {buildOperationLabel(operation)}
                </option>
              ))}
              {selectedBuildOperation ? (
                <option value={value.buildOperation}>
                  {buildOperationLabel(value.buildOperation)} (override)
                </option>
              ) : null}
            </select>
            <span className="muted">
              {buildOperations.length === 0
                ? 'Select services to see the operations their build providers expose.'
                : `Provider operations: ${buildOperations
                    .map(buildOperationLabel)
                    .join(', ')}.`}
            </span>
          </label>
        ) : null}
      </fieldset>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Runtime</legend>
        {RUNTIME_ACTIONS.map((action) => (
          <label
            key={action.value}
            className="options-form__field options-form__field--indent"
          >
            <span>
              <input
                type="radio"
                name="control-center-runtime"
                checked={value.runtime === action.value}
                disabled={disabled}
                onChange={() =>
                  patch({
                    runtime: action.value,
                    // A stop and a health check in one workflow would report an
                    // ambiguous state; the core rejects it, so the UI clears it.
                    ...(action.value === 'stop' ? { healthChecks: false } : {}),
                  })
                }
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
              onChange={(event) =>
                patch({ waitForHealth: event.target.checked })
              }
              aria-label="Wait for readiness"
            />{' '}
            Wait for readiness after start
          </span>
          <span className="muted">
            Runs the configured health checks after startup.
          </span>
        </label>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.healthChecks}
              disabled={disabled || value.runtime === 'stop'}
              onChange={(event) =>
                patch({ healthChecks: event.target.checked })
              }
              aria-label="Run health checks"
            />{' '}
            Run health checks without starting
          </span>
          <span className="muted">
            Checks only services that declare health checks. Health checks cannot
            be combined with stopping services.
          </span>
        </label>
      </fieldset>

      <div className="detail-actions control-center__advanced-link">
        <button
          type="button"
          className="button button--small"
          disabled={disabled}
          onClick={onOpenAdvanced}
        >
          Advanced settings…
        </button>
        <span className="muted">
          Temporary overrides for this run; configuration-owned values stay
          read-only.
        </span>
      </div>
    </div>
  );
}
