import type { DiscoveredRepositoryView, MigrationPresetSummaryView } from '../../types/models';
import { migrationEnvironmentLabel } from '../../types/models';
import type { ImportRowDraft } from './draft';

const BUILD_OPERATIONS = [
  'clean',
  'build',
  'test',
  'install_dependencies',
  'publish_local_artifact',
];

const DATABASE_ENVIRONMENTS = [
  'local',
  'shared_development',
  'staging',
  'production',
  'unknown',
] as const;

/** The suggestion shown for a row. */
function suggestionFor(
  repository: DiscoveredRepositoryView | undefined,
  row: ImportRowDraft,
): string {
  if (row.template !== '') {
    return `template ${row.template}`;
  }
  if (!repository) {
    return 'automatic matching';
  }
  if ((repository.ambiguous_categories ?? []).length > 0) {
    return 'ambiguous — choose on step 5';
  }
  return repository.suggested_template
    ? `suggested ${repository.suggested_template}`
    : 'no suggestion';
}

/**
 * Per-repository exceptions (`tasks/032.md` §10 step 7, §12).
 *
 * Each repository can override the entity id, ports, branch, build operation,
 * migration settings, and commands. Only the fields the user fills in are sent;
 * everything else keeps the deterministic bulk → template → detection
 * precedence resolved by the daemon.
 */
export function RepositoryOverridesTable({
  rows,
  repositories,
  presets,
  onChange,
}: {
  rows: readonly ImportRowDraft[];
  repositories: readonly DiscoveredRepositoryView[];
  presets: readonly MigrationPresetSummaryView[];
  onChange: (relativePath: string, patch: Partial<ImportRowDraft>) => void;
}) {
  if (rows.length === 0) {
    return <p className="muted">No repositories are selected.</p>;
  }
  const byPath = new Map(
    repositories.map((entry) => [entry.repository.relative_path, entry]),
  );

  return (
    <section className="import-review" aria-label="Per-repository overrides">
      <h3 className="card__title">Per-repository exceptions</h3>
      <p className="muted">
        Empty fields inherit from the shared overrides, the template, or the detected
        value. Nothing is written until the final import is confirmed.
      </p>
      {rows.map((row) => {
        const repository = byPath.get(row.relativePath);
        return (
          <details key={row.relativePath} className="import-row" open={rows.length <= 3}>
            <summary className="import-row__summary">
              <span className="import-row__path">{row.relativePath}</span>
              <span className="muted">{suggestionFor(repository, row)}</span>
              <span className="muted">
                ports: {row.ports.trim() || 'assigned'} · exists: {row.existing}
              </span>
            </summary>
            <div className="config-form import-row__form">
              <label className="config-form__field">
                <span>Entity id</span>
                <input
                  className="input"
                  value={row.name}
                  onChange={(event) => onChange(row.relativePath, { name: event.target.value })}
                  aria-label={`Entity id for ${row.relativePath}`}
                  placeholder="Derived from the directory name"
                />
              </label>
              <label className="config-form__field">
                <span>Exposed ports</span>
                <input
                  className="input"
                  value={row.ports}
                  onChange={(event) => onChange(row.relativePath, { ports: event.target.value })}
                  aria-label={`Ports for ${row.relativePath}`}
                  placeholder="8081, 8443"
                />
              </label>
              <label className="config-form__field">
                <span>Branch</span>
                <input
                  className="input"
                  value={row.branch}
                  onChange={(event) => onChange(row.relativePath, { branch: event.target.value })}
                  aria-label={`Branch for ${row.relativePath}`}
                  placeholder="From the shared override or template"
                />
              </label>
              <label className="config-form__field">
                <span>Existing entity</span>
                <select
                  className="input input--select"
                  value={row.existing}
                  onChange={(event) =>
                    onChange(row.relativePath, {
                      existing: event.target.value as ImportRowDraft['existing'],
                    })
                  }
                  aria-label={`Existing entity policy for ${row.relativePath}`}
                >
                  <option value="skip">Leave it untouched (skip)</option>
                  <option value="update">Replace it (destructive)</option>
                </select>
              </label>
              <label className="config-form__field">
                <span>Build operation</span>
                <select
                  className="input input--select"
                  value={row.buildOperation}
                  onChange={(event) =>
                    onChange(row.relativePath, { buildOperation: event.target.value })
                  }
                  aria-label={`Build operation for ${row.relativePath}`}
                >
                  <option value="">Inherit</option>
                  {BUILD_OPERATIONS.map((operation) => (
                    <option key={operation} value={operation}>
                      {operation.replace(/_/g, ' ')}
                    </option>
                  ))}
                </select>
              </label>
              <label className="config-form__field">
                <span>Migrations</span>
                <select
                  className="input input--select"
                  value={row.migration}
                  onChange={(event) =>
                    onChange(row.relativePath, {
                      migration: event.target.value as ImportRowDraft['migration'],
                    })
                  }
                  aria-label={`Migration setting for ${row.relativePath}`}
                >
                  <option value="">Inherit</option>
                  <option value="true">Enabled</option>
                  <option value="false">Disabled</option>
                </select>
              </label>
              <label className="config-form__field">
                <span>Migration preset</span>
                <select
                  className="input input--select"
                  value={row.migrationPreset}
                  onChange={(event) =>
                    onChange(row.relativePath, { migrationPreset: event.target.value })
                  }
                  aria-label={`Migration preset for ${row.relativePath}`}
                >
                  <option value="">Inherit</option>
                  {presets.map((preset) => (
                    <option key={preset.id} value={preset.id}>
                      {preset.name}
                      {preset.destructive ? ' (destructive)' : ''}
                    </option>
                  ))}
                </select>
              </label>
              <label className="config-form__field">
                <span>Database environment</span>
                <select
                  className="input input--select"
                  value={row.databaseEnvironment}
                  onChange={(event) =>
                    onChange(row.relativePath, { databaseEnvironment: event.target.value })
                  }
                  aria-label={`Database environment for ${row.relativePath}`}
                >
                  <option value="">Inherit</option>
                  {DATABASE_ENVIRONMENTS.map((environment) => (
                    <option key={environment} value={environment}>
                      {migrationEnvironmentLabel(environment)}
                    </option>
                  ))}
                </select>
              </label>
              <label className="config-form__field">
                <span>Health check URL</span>
                <input
                  className="input"
                  value={row.healthUrl}
                  onChange={(event) =>
                    onChange(row.relativePath, { healthUrl: event.target.value })
                  }
                  aria-label={`Health check URL for ${row.relativePath}`}
                  placeholder="http://localhost:${port}/actuator/health"
                />
              </label>
              <label className="config-form__field">
                <span>Build command</span>
                <input
                  className="input"
                  value={row.buildCommand}
                  onChange={(event) =>
                    onChange(row.relativePath, { buildCommand: event.target.value })
                  }
                  aria-label={`Build command for ${row.relativePath}`}
                  placeholder="Inherit"
                />
              </label>
              <label className="config-form__field">
                <span>Start command</span>
                <input
                  className="input"
                  value={row.startCommand}
                  onChange={(event) =>
                    onChange(row.relativePath, { startCommand: event.target.value })
                  }
                  aria-label={`Start command for ${row.relativePath}`}
                  placeholder="Inherit"
                />
              </label>
            </div>
          </details>
        );
      })}
    </section>
  );
}
