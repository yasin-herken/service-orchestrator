import { Dialog } from '../Dialog';
import type {
  MigrationPresetSummaryView,
  RawCommandView,
  RawEnvValueView,
  RawHealthCheckView,
  RawServiceView,
  ServiceSummary,
} from '../../types/models';
import { buildOperationLabel, type ControlCenterOptions } from './model';

/** Renders a configured command without executing or reinterpreting it. */
function commandText(command: RawCommandView | undefined): string {
  if (command === undefined) {
    return 'not configured';
  }
  if (typeof command === 'string') {
    return command;
  }
  const args = command.args ?? [];
  const directory = command.working_dir
    ? ` (working directory: ${command.working_dir})`
    : '';
  return [command.program, ...args].join(' ') + directory;
}

/** Renders a health check target from configuration. */
function healthText(check: RawHealthCheckView): string {
  const target = check.url ?? (check.port !== undefined ? `port ${check.port}` : '');
  const required = check.required === false ? 'optional' : 'required';
  const timeout =
    check.timeout_seconds !== undefined ? `, ${check.timeout_seconds}s timeout` : '';
  return `${check.type}${target ? ` · ${target}` : ''} (${required}${timeout})`;
}

/** Environment entries with secrets redacted, never rendered as values. */
function environmentEntries(
  environment: Record<string, RawEnvValueView> | undefined,
): { key: string; value: string; secret: boolean }[] {
  return Object.entries(environment ?? {}).map(([key, value]) => {
    if (typeof value === 'string') {
      return { key, value, secret: false };
    }
    return {
      key,
      value: value.secret ? '•••• (secret)' : value.value,
      secret: value.secret,
    };
  });
}

/**
 * The compact Advanced Settings modal (`tasks/033.md` §11, §12).
 *
 * Only real request fields are editable (dependency inclusion, readiness,
 * build operation, migration preset). Values owned by service configuration —
 * working directory, runtime commands, health checks, timeouts, environment
 * references — are shown read-only, with secrets redacted. Nothing here writes
 * service configuration: overrides apply to the next plan/run only.
 */
export function AdvancedSettingsDialog({
  value,
  presets,
  buildOperations,
  selectedServices,
  rawServices,
  configurationError = null,
  disabled = false,
  onChange,
  onOpenConfiguration,
  onReloadConfiguration,
  onClose,
}: {
  value: ControlCenterOptions;
  presets: readonly MigrationPresetSummaryView[];
  buildOperations: readonly string[];
  selectedServices: readonly ServiceSummary[];
  rawServices: readonly RawServiceView[];
  /** Human-safe message when configuration could not be loaded. */
  configurationError?: string | null;
  disabled?: boolean;
  onChange: (next: ControlCenterOptions) => void;
  onOpenConfiguration: () => void;
  onReloadConfiguration?: () => void;
  onClose: () => void;
}) {
  const patch = (next: Partial<ControlCenterOptions>) =>
    onChange({ ...value, ...next });
  const selected = new Set(selectedServices.map((service) => service.id));
  const configured = rawServices.filter((service) => selected.has(service.id));
  const runtimeActive =
    value.runtime === 'start' || value.runtime === 'restart';

  return (
    <Dialog
      title="Advanced settings"
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onOpenConfiguration}>
            Open Configuration…
          </button>
          <button type="button" className="button button--primary" onClick={onClose}>
            Done
          </button>
        </>
      }
    >
      <p className="dialog__note">
        These overrides apply to the next plan and run. They are not written to
        service configuration and never carry authorization.
      </p>

      <fieldset className="options-form">
        <legend className="selection-list__legend">Workflow overrides</legend>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.includeDependencies}
              disabled={disabled}
              onChange={(event) =>
                patch({ includeDependencies: event.target.checked })
              }
              aria-label="Include required dependencies"
            />{' '}
            Include required dependencies
          </span>
          <span className="muted">
            Adds the resolved dependency closure to the workflow. The plan shows
            exactly which services were included.
          </span>
        </label>
        <label className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value.waitForHealth}
              disabled={disabled || !runtimeActive}
              onChange={(event) =>
                patch({ waitForHealth: event.target.checked })
              }
              aria-label="Advanced wait for readiness"
            />{' '}
            Wait for readiness after start
          </span>
          <span className="muted">
            Uses the configured health checks after startup.
          </span>
        </label>
        <label className="options-form__field">
          <span>Build operation</span>
          <select
            className="input input--select"
            value={value.buildOperation}
            disabled={disabled || !value.build || buildOperations.length === 0}
            onChange={(event) =>
              patch({ buildOperation: event.target.value })
            }
            aria-label="Advanced build operation"
          >
            <option value="">Each service&apos;s configured operation</option>
            {buildOperations.map((operation) => (
              <option key={operation} value={operation}>
                {buildOperationLabel(operation)}
              </option>
            ))}
          </select>
        </label>
        <label className="options-form__field">
          <span>Migration preset</span>
          <select
            className="input input--select"
            value={value.migrationPreset}
            disabled={disabled || !value.runLiquibase}
            onChange={(event) =>
              patch({ migrationPreset: event.target.value })
            }
            aria-label="Advanced migration preset"
          >
            <option value="">Each service&apos;s configured preset</option>
            {presets.map((preset) => (
              <option key={preset.id} value={preset.id}>
                {preset.name}
                {preset.destructive ? ' (destructive)' : ''}
              </option>
            ))}
          </select>
        </label>
      </fieldset>

      <section aria-label="Service configuration">
        <h3 className="dependency-view__title">Service configuration (read-only)</h3>
        {configurationError !== null ? (
          <p className="page__notice page__notice--error" role="alert">
            Configuration unavailable: {configurationError}
            {onReloadConfiguration ? (
              <>
                {' '}
                <button
                  type="button"
                  className="button button--small"
                  onClick={onReloadConfiguration}
                >
                  Reload
                </button>
              </>
            ) : null}
          </p>
        ) : null}
        {configured.length === 0 ? (
          <p className="muted">Select services to inspect their configuration.</p>
        ) : (
          <div className="control-center__config-list">
            {configured.map((service) => {
              const environment = environmentEntries(service.environment);
              const health = service.health_checks ?? [];
              return (
                <details key={service.id} className="control-center__config">
                  <summary>
                    {service.name ?? service.id}
                    <span className="muted"> · {service.type}</span>
                  </summary>
                  <dl className="detail-grid">
                    <div>
                      <dt>Working directory</dt>
                      <dd className="detail-grid__mono">
                        {service.path ?? 'workspace root'}
                      </dd>
                    </div>
                    <div>
                      <dt>Default branch</dt>
                      <dd>
                        {service.default_branch ??
                          service.repository?.default_branch ??
                          'not configured'}
                      </dd>
                    </div>
                    <div>
                      <dt>Build system</dt>
                      <dd>{service.build_system ?? 'not configured'}</dd>
                    </div>
                    <div>
                      <dt>Default operation</dt>
                      <dd>
                        {service.build_operation
                          ? buildOperationLabel(service.build_operation)
                          : 'provider default'}
                      </dd>
                    </div>
                    <div className="detail-grid__wide">
                      <dt>Build command</dt>
                      <dd className="detail-grid__mono">
                        {commandText(service.commands?.build)}
                      </dd>
                    </div>
                    <div className="detail-grid__wide">
                      <dt>Start command</dt>
                      <dd className="detail-grid__mono">
                        {commandText(service.commands?.start)}
                      </dd>
                    </div>
                    <div>
                      <dt>Migration preset</dt>
                      <dd>{service.liquibase?.preset ?? 'not configured'}</dd>
                    </div>
                    <div>
                      <dt>Migration timeout</dt>
                      <dd>
                        {service.liquibase?.timeout_seconds !== undefined
                          ? `${service.liquibase.timeout_seconds}s`
                          : 'not configured'}
                      </dd>
                    </div>
                    <div className="detail-grid__wide">
                      <dt>Health checks</dt>
                      <dd>
                        {health.length === 0
                          ? 'none configured'
                          : health.map(healthText).join(' · ')}
                      </dd>
                    </div>
                    <div className="detail-grid__wide">
                      <dt>Environment</dt>
                      <dd>
                        {environment.length === 0 ? (
                          'none configured'
                        ) : (
                          <ul className="control-center__env">
                            {environment.map((entry) => (
                              <li key={entry.key}>
                                <span className="control-center__env-key">
                                  {entry.key}
                                </span>
                                <span
                                  className={
                                    entry.secret ? 'muted' : 'detail-grid__mono'
                                  }
                                >
                                  {entry.value}
                                </span>
                              </li>
                            ))}
                          </ul>
                        )}
                      </dd>
                    </div>
                  </dl>
                </details>
              );
            })}
          </div>
        )}
        <p className="muted">
          Working directory, runtime commands, health checks, timeouts, and
          environment variables belong to service configuration. Edit them in
          Configuration; the Control Center never persists them silently.
        </p>
      </section>
    </Dialog>
  );
}
