import { useState } from 'react';

import { Dialog } from '../Dialog';
import type {
  MigrationPresetSummaryView,
  RawCommandView,
  RawTemplateView,
} from '../../types/models';
import { parseList } from '../../utils/configuration';

/** The built-in template categories, plus the custom escape hatch. */
const CATEGORIES = [
  { value: 'spring_boot_backend', label: 'Spring Boot backend' },
  { value: 'react_frontend', label: 'React frontend' },
  { value: 'shared_maven_library', label: 'Shared Maven library' },
];

/** The build operations a template may default to. */
const BUILD_OPERATIONS = [
  'clean',
  'build',
  'test',
  'install_dependencies',
  'publish_local_artifact',
];

/** The markup that marks a custom category in the select. */
const CUSTOM = '__custom__';

function isBuiltinCategory(category: string | undefined): boolean {
  return CATEGORIES.some((entry) => entry.value === category);
}

/** A single-line command, or `undefined` when blank. */
function command(line: string): RawCommandView | undefined {
  const trimmed = line.trim();
  return trimmed === '' ? undefined : trimmed;
}

/** Renders `KEY=value` lines as an environment map. */
function parseEnvironment(text: string): Record<string, string> | undefined {
  const entries: Record<string, string> = {};
  for (const line of text.split('\n')) {
    const trimmed = line.trim();
    if (trimmed === '' || trimmed.startsWith('#')) {
      continue;
    }
    const separator = trimmed.indexOf('=');
    if (separator <= 0) {
      continue;
    }
    const key = trimmed.slice(0, separator).trim();
    const value = trimmed.slice(separator + 1).trim();
    if (key !== '') {
      entries[key] = value;
    }
  }
  return Object.keys(entries).length > 0 ? entries : undefined;
}

/** Renders an environment map back to editable lines. */
function environmentText(environment: Record<string, unknown> | undefined): string {
  if (!environment) {
    return '';
  }
  return Object.entries(environment)
    .map(([key, value]) => `${key}=${typeof value === 'string' ? value : ''}`)
    .join('\n');
}

/**
 * The template editor (`tasks/032.md` §5).
 *
 * A template is a set of *defaults* for the existing validated service model:
 * the editor writes only template configuration, never a service, and every
 * `${...}` value is validated by the daemon when the change is previewed. A
 * template id is immutable after creation, so editing never silently renames
 * an entity that other configuration may refer to.
 */
export function TemplateEditor({
  initial,
  mode,
  presets,
  onSave,
  onClose,
}: {
  initial: RawTemplateView | null;
  mode: 'create' | 'edit' | 'duplicate';
  presets: readonly MigrationPresetSummaryView[];
  onSave: (template: RawTemplateView) => void;
  onClose: () => void;
}) {
  const editing = mode === 'edit' && initial !== null;
  const [id, setId] = useState(
    mode === 'duplicate' && initial ? `${initial.id}-copy` : (initial?.id ?? ''),
  );
  const [name, setName] = useState(initial?.name ?? '');
  const [description, setDescription] = useState(initial?.description ?? '');
  const [category, setCategory] = useState(
    initial?.category !== undefined && !isBuiltinCategory(initial.category)
      ? CUSTOM
      : (initial?.category ?? 'spring_boot_backend'),
  );
  const [customCategory, setCustomCategory] = useState(
    initial?.category !== undefined && !isBuiltinCategory(initial.category)
      ? initial.category
      : '',
  );
  const [type, setType] = useState(initial?.type ?? '');
  const [defaultBranch, setDefaultBranch] = useState(initial?.default_branch ?? '');
  const [buildSystem, setBuildSystem] = useState(initial?.build_system ?? '');
  const [buildOperation, setBuildOperation] = useState(initial?.build_operation ?? '');
  const [runtimeType, setRuntimeType] = useState(
    initial?.runtime?.type ?? initial?.runtimes?.[0]?.type ?? '',
  );
  const [runtimeVersion, setRuntimeVersion] = useState(
    initial?.runtime?.version ?? initial?.runtimes?.[0]?.version ?? '',
  );
  const [buildCommand, setBuildCommand] = useState(
    typeof initial?.commands?.build === 'string' ? initial.commands.build : '',
  );
  const [startCommand, setStartCommand] = useState(
    typeof initial?.commands?.start === 'string' ? initial.commands.start : '',
  );
  const [healthUrl, setHealthUrl] = useState(
    initial?.health_checks?.find((check) => check.type === 'http')?.url ?? '',
  );
  const [migration, setMigration] = useState(initial?.liquibase?.enabled ?? true);
  const [migrationPreset, setMigrationPreset] = useState(initial?.liquibase?.preset ?? '');
  const [timeoutSeconds, setTimeoutSeconds] = useState(
    initial?.timeout_seconds !== undefined ? String(initial.timeout_seconds) : '',
  );
  const [environment, setEnvironment] = useState(environmentText(initial?.environment));
  const [capabilities, setCapabilities] = useState((initial?.capabilities ?? []).join(', '));

  const trimmedId = id.trim();
  const enabled = trimmedId !== '';

  const submit = () => {
    if (!enabled) {
      return;
    }
    const template: RawTemplateView = { id: trimmedId };
    if (name.trim() !== '') {
      template.name = name.trim();
    }
    if (description.trim() !== '') {
      template.description = description.trim();
    }
    const resolvedCategory = category === CUSTOM ? customCategory.trim() : category;
    if (resolvedCategory !== '') {
      template.category = resolvedCategory;
    }
    if (type !== '') {
      template.type = type;
    }
    if (defaultBranch.trim() !== '') {
      template.default_branch = defaultBranch.trim();
    }
    if (buildSystem !== '') {
      template.build_system = buildSystem;
    }
    if (buildOperation !== '') {
      template.build_operation = buildOperation;
    }
    if (runtimeType !== '') {
      template.runtime = {
        type: runtimeType,
        ...(runtimeVersion.trim() !== '' ? { version: runtimeVersion.trim() } : {}),
      };
    }
    const build = command(buildCommand);
    const start = command(startCommand);
    if (build !== undefined || start !== undefined) {
      template.commands = {
        ...(build !== undefined ? { build } : {}),
        ...(start !== undefined ? { start } : {}),
      };
    }
    if (migration || migrationPreset !== '') {
      template.liquibase = {
        enabled: migration,
        ...(migrationPreset !== '' ? { preset: migrationPreset } : {}),
      };
    }
    if (healthUrl.trim() !== '') {
      template.health_checks = [{ type: 'http', url: healthUrl.trim() }];
    }
    if (timeoutSeconds.trim() !== '') {
      const parsed = Number(timeoutSeconds);
      if (Number.isFinite(parsed) && parsed > 0) {
        template.timeout_seconds = parsed;
      }
    }
    const parsedEnvironment = parseEnvironment(environment);
    if (parsedEnvironment !== undefined) {
      template.environment = parsedEnvironment;
    }
    const parsedCapabilities = parseList(capabilities);
    if (parsedCapabilities.length > 0) {
      template.capabilities = parsedCapabilities;
    }
    onSave(template);
  };

  return (
    <Dialog
      title={
        mode === 'create'
          ? 'New template'
          : mode === 'duplicate'
            ? 'Duplicate template'
            : 'Edit template'
      }
      onClose={onClose}
      footer={
        <>
          <button type="button" className="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="button button--primary"
            onClick={submit}
            disabled={!enabled}
          >
            Review change…
          </button>
        </>
      }
    >
      <p className="dialog__message">
        A template is a set of defaults, not a second service configuration. The daemon
        validates the whole file and resolves the values only when a repository is imported.
      </p>

      <div className="config-form">
        <label className="config-form__field">
          <span>Identifier</span>
          <input
            className="input"
            value={id}
            disabled={editing}
            onChange={(event) => setId(event.target.value)}
            aria-label="Template identifier"
          />
          <span className="muted">
            {editing
              ? 'A template id cannot be renamed after creation.'
              : 'Stable id used by services and bulk imports.'}
          </span>
        </label>
        <label className="config-form__field">
          <span>Name</span>
          <input
            className="input"
            value={name}
            onChange={(event) => setName(event.target.value)}
            aria-label="Template name"
          />
        </label>
        <label className="config-form__field">
          <span>Description</span>
          <input
            className="input"
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            aria-label="Template description"
          />
        </label>
        <label className="config-form__field">
          <span>Category</span>
          <select
            className="input input--select"
            value={category}
            onChange={(event) => setCategory(event.target.value)}
            aria-label="Template category"
          >
            {CATEGORIES.map((entry) => (
              <option key={entry.value} value={entry.value}>
                {entry.label}
              </option>
            ))}
            <option value={CUSTOM}>Custom…</option>
          </select>
          <span className="muted">
            A known category participates in automatic matching; a custom one never does.
          </span>
        </label>
        {category === CUSTOM ? (
          <label className="config-form__field">
            <span>Custom category</span>
            <input
              className="input"
              value={customCategory}
              onChange={(event) => setCustomCategory(event.target.value)}
              aria-label="Custom category"
            />
          </label>
        ) : null}
        <label className="config-form__field">
          <span>Service type</span>
          <select
            className="input input--select"
            value={type}
            onChange={(event) => setType(event.target.value)}
            aria-label="Template service type"
          >
            <option value="">Inherit from detection</option>
            <option value="backend">Backend</option>
            <option value="frontend">Frontend</option>
            <option value="library">Shared library</option>
          </select>
        </label>
        <label className="config-form__field">
          <span>Default branch</span>
          <input
            className="input"
            value={defaultBranch}
            onChange={(event) => setDefaultBranch(event.target.value)}
            aria-label="Template default branch"
            placeholder="main"
          />
          <span className="muted">
            ${'{'}serviceName{'}'} and the other documented variables are allowed here.
          </span>
        </label>
        <label className="config-form__field">
          <span>Build system</span>
          <select
            className="input input--select"
            value={buildSystem}
            onChange={(event) => setBuildSystem(event.target.value)}
            aria-label="Template build system"
          >
            <option value="">Inherit from detection</option>
            <option value="maven">Maven</option>
            <option value="npm">npm</option>
          </select>
        </label>
        <label className="config-form__field">
          <span>Default build operation</span>
          <select
            className="input input--select"
            value={buildOperation}
            onChange={(event) => setBuildOperation(event.target.value)}
            aria-label="Template build operation"
          >
            <option value="">No default</option>
            {BUILD_OPERATIONS.map((operation) => (
              <option key={operation} value={operation}>
                {operation.replace(/_/g, ' ')}
              </option>
            ))}
          </select>
        </label>
        <label className="config-form__field">
          <span>Runtime</span>
          <select
            className="input input--select"
            value={runtimeType}
            onChange={(event) => setRuntimeType(event.target.value)}
            aria-label="Template runtime"
          >
            <option value="">Inherit from detection</option>
            <option value="java">Java</option>
            <option value="node">Node.js</option>
            <option value="maven">Maven</option>
            <option value="npm">npm</option>
          </select>
        </label>
        <label className="config-form__field">
          <span>Runtime version</span>
          <input
            className="input"
            value={runtimeVersion}
            onChange={(event) => setRuntimeVersion(event.target.value)}
            aria-label="Template runtime version"
            placeholder="21"
          />
        </label>
        <label className="config-form__field">
          <span>Build command</span>
          <input
            className="input"
            value={buildCommand}
            onChange={(event) => setBuildCommand(event.target.value)}
            aria-label="Template build command"
            placeholder="./mvnw clean install"
          />
          <span className="muted">
            A single executable line; the daemon parses it into program and arguments.
          </span>
        </label>
        <label className="config-form__field">
          <span>Start command</span>
          <input
            className="input"
            value={startCommand}
            onChange={(event) => setStartCommand(event.target.value)}
            aria-label="Template start command"
            placeholder="./mvnw spring-boot:run"
          />
        </label>
        <label className="config-form__field">
          <span>Health check URL</span>
          <input
            className="input"
            value={healthUrl}
            onChange={(event) => setHealthUrl(event.target.value)}
            aria-label="Template health check URL"
            placeholder="http://localhost:${port}/actuator/health"
          />
          <span className="muted">
            Templates may declare an HTTP check or a process check, never a fixed port.
          </span>
        </label>
        <label className="config-form__field config-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={migration}
              onChange={(event) => setMigration(event.target.checked)}
              aria-label="Template enables migrations"
            />{' '}
            Migrations enabled by default
          </span>
        </label>
        <label className="config-form__field">
          <span>Migration preset</span>
          <select
            className="input input--select"
            value={migrationPreset}
            onChange={(event) => setMigrationPreset(event.target.value)}
            aria-label="Template migration preset"
          >
            <option value="">Use each service’s preset</option>
            {presets.map((preset) => (
              <option key={preset.id} value={preset.id}>
                {preset.name}
                {preset.destructive ? ' (destructive)' : ''}
              </option>
            ))}
          </select>
          <span className="muted">
            A preset never authorizes anything: execution still re-validates the target.
          </span>
        </label>
        <label className="config-form__field">
          <span>Timeout (seconds)</span>
          <input
            className="input"
            value={timeoutSeconds}
            onChange={(event) => setTimeoutSeconds(event.target.value)}
            aria-label="Template timeout seconds"
            inputMode="numeric"
          />
        </label>
        <label className="config-form__field">
          <span>Environment defaults</span>
          <textarea
            className="input"
            rows={3}
            value={environment}
            onChange={(event) => setEnvironment(event.target.value)}
            aria-label="Template environment defaults"
            placeholder="SPRING_PROFILES_ACTIVE=local"
          />
          <span className="muted">
            One KEY=value per line. Do not store credentials in configuration.
          </span>
        </label>
        <label className="config-form__field">
          <span>Capabilities</span>
          <input
            className="input"
            value={capabilities}
            onChange={(event) => setCapabilities(event.target.value)}
            aria-label="Template capabilities"
            placeholder="build, run, migrate, serve_http"
          />
        </label>
      </div>
    </Dialog>
  );
}
