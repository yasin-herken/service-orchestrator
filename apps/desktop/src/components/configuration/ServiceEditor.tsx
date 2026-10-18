import { useMemo, useState } from 'react';

import { Dialog } from '../Dialog';
import { desktopApi } from '../../services/desktopApi';
import { toDesktopError } from '../../services/errors';
import type {
  ProjectSuggestionView,
  RawEnvValueView,
  RawHealthCheckView,
  RawRuntimeView,
  RawServiceView,
  RepositoryProbeView,
} from '../../types/models';
import { parseList, structuredCommandLine, toCommand } from '../../utils/configuration';
import { EnvironmentEditor } from './EnvironmentEditor';
import { RepositoryDetectionPanel } from './RepositoryDetectionPanel';

/** The single runtime requirement implied by the build system, if any. */
function runtimeFor(
  buildSystem: string,
  javaVersion: string,
  nodeVersion: string,
): RawRuntimeView | undefined {
  if (buildSystem === 'maven' && javaVersion.trim() !== '') {
    return { type: 'java', version: javaVersion.trim() };
  }
  if (buildSystem === 'npm' && nodeVersion.trim() !== '') {
    return { type: 'node', version: nodeVersion.trim() };
  }
  if (javaVersion.trim() !== '') {
    return { type: 'java', version: javaVersion.trim() };
  }
  if (nodeVersion.trim() !== '') {
    return { type: 'node', version: nodeVersion.trim() };
  }
  return undefined;
}

/** The health checks implied by the health form. */
function healthChecks(
  kind: string,
  url: string,
  port: string,
): RawHealthCheckView[] | undefined {
  if (kind === 'http' && url.trim() !== '') {
    return [{ type: 'http', url: url.trim() }];
  }
  if (kind === 'port' && port.trim() !== '') {
    const parsed = Number.parseInt(port.trim(), 10);
    if (Number.isFinite(parsed)) {
      return [{ type: 'port', port: parsed }];
    }
  }
  return undefined;
}

/**
 * The service wizard/editor (`tasks/023.md` §13–§29, §43).
 *
 * It is one form for creating and editing a service: identity, repository,
 * build, runtime, migration, health, dependencies, and environment. A read-only
 * repository detection panel can prefill the build/runtime fields from a real
 * directory, but the daemon still validates and persists the final entity.
 */
export function ServiceEditor({
  initial,
  workspaceId,
  workspaceRoot,
  onSave,
  onClose,
}: {
  initial: RawServiceView | null;
  workspaceId: string;
  workspaceRoot: string;
  onSave: (entity: RawServiceView) => void;
  onClose: () => void;
}) {
  const editing = initial !== null;
  const [id, setId] = useState(initial?.id ?? '');
  const [name, setName] = useState(initial?.name ?? '');
  const [serviceType, setServiceType] = useState(initial?.type ?? 'backend');
  const [path, setPath] = useState(initial?.path ?? '');
  const [defaultBranch, setDefaultBranch] = useState(
    initial?.default_branch ?? 'main',
  );
  const [repositoryUrl, setRepositoryUrl] = useState(initial?.repository?.url ?? '');
  const [provider, setProvider] = useState(initial?.repository?.provider ?? '');
  const buildSystem = initial?.build_system ?? 'maven';
  const [buildSystemState, setBuildSystemState] = useState(buildSystem);
  const javaRuntime = initial?.runtime?.type === 'java' ? initial.runtime : undefined;
  const nodeRuntime = initial?.runtime?.type === 'node' ? initial.runtime : undefined;
  const [javaVersion, setJavaVersion] = useState(javaRuntime?.version ?? '21');
  const [nodeVersion, setNodeVersion] = useState(nodeRuntime?.version ?? '');
  const [buildCommand, setBuildCommand] = useState(
    typeof initial?.commands?.build === 'string'
      ? initial.commands.build
      : initial?.commands?.build
        ? structuredCommandLine(initial.commands.build)
        : '',
  );
  const [startCommand, setStartCommand] = useState(
    typeof initial?.commands?.start === 'string'
      ? initial.commands.start
      : initial?.commands?.start
        ? structuredCommandLine(initial.commands.start)
        : '',
  );
  const existingHealth = initial?.health_checks?.[0];
  const [healthKind, setHealthKind] = useState(existingHealth?.type ?? 'none');
  const [healthUrl, setHealthUrl] = useState(existingHealth?.url ?? '');
  const [healthPort, setHealthPort] = useState(
    existingHealth?.port !== undefined ? String(existingHealth.port) : '',
  );
  const [portsText, setPortsText] = useState(
    (initial?.ports ?? []).map((port) => port.port).join(', '),
  );
  const dependencyRefs = initial?.dependencies?.services ?? [];
  const [dependenciesText, setDependenciesText] = useState(
    dependencyRefs
      .map((ref) => (typeof ref === 'string' ? ref : ref.id))
      .join(', '),
  );
  const [environment, setEnvironment] = useState<Record<string, RawEnvValueView>>(
    initial?.environment ?? {},
  );

  const [probePath, setProbePath] = useState(
    initial?.path !== undefined && workspaceRoot !== ''
      ? `${workspaceRoot.replace(/\/$/, '')}/${initial.path}`
      : workspaceRoot,
  );
  const [probe, setProbe] = useState<RepositoryProbeView | null>(null);
  const [suggestion, setSuggestion] = useState<ProjectSuggestionView | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [detectionError, setDetectionError] = useState<string | null>(null);

  const idValid = id.trim() !== '';
  const valid = idValid;

  const inspect = async () => {
    setDetecting(true);
    setDetectionError(null);
    try {
      setProbe(await desktopApi.inspectRepository(probePath.trim()));
      setSuggestion(null);
    } catch (cause) {
      setDetectionError(toDesktopError(cause).message);
    } finally {
      setDetecting(false);
    }
  };

  const detect = async () => {
    setDetecting(true);
    setDetectionError(null);
    try {
      const result = await desktopApi.detectProjectConfiguration(probePath.trim());
      setSuggestion(result);
      setProbe(result.inspection);
    } catch (cause) {
      setDetectionError(toDesktopError(cause).message);
    } finally {
      setDetecting(false);
    }
  };

  const apply = (detected: ProjectSuggestionView) => {
    if (detected.service_type) {
      setServiceType(detected.service_type);
    }
    if (detected.build_system) {
      setBuildSystemState(detected.build_system);
    }
    const java = detected.runtimes?.find((runtime) => runtime.kind === 'java');
    const node = detected.runtimes?.find((runtime) => runtime.kind === 'node');
    if (java?.required_version) {
      setJavaVersion(java.required_version);
    }
    if (node?.required_version) {
      setNodeVersion(node.required_version);
    }
    if (detected.build_command) {
      setBuildCommand(structuredCommandLine(detected.build_command));
    }
    if (detected.start_command) {
      setStartCommand(structuredCommandLine(detected.start_command));
    }
  };

  const ports = useMemo(
    () =>
      parseList(portsText)
        .map((value) => Number.parseInt(value, 10))
        .filter((value) => Number.isFinite(value))
        .map((value) => ({ port: value })),
    [portsText],
  );

  const submit = () => {
    const entity: RawServiceView = {
      id: id.trim(),
      type: serviceType,
      workspace: workspaceId,
    };
    if (name.trim() !== '') {
      entity.name = name.trim();
    }
    if (path.trim() !== '') {
      entity.path = path.trim();
    }
    if (defaultBranch.trim() !== '') {
      entity.default_branch = defaultBranch.trim();
    }
    if (repositoryUrl.trim() !== '') {
      entity.repository = { url: repositoryUrl.trim() };
      if (provider.trim() !== '') {
        entity.repository.provider = provider.trim();
      }
    }
    if (buildSystemState !== '') {
      entity.build_system = buildSystemState;
    }
    const runtime = runtimeFor(buildSystemState, javaVersion, nodeVersion);
    if (runtime) {
      entity.runtime = runtime;
    }
    const build = toCommand(buildCommand);
    const start = toCommand(startCommand);
    if (build || start) {
      entity.commands = {};
      if (build) {
        entity.commands.build = build;
      }
      if (start) {
        entity.commands.start = start;
      }
    }
    const checks = healthChecks(healthKind, healthUrl, healthPort);
    if (checks) {
      entity.health_checks = checks;
    }
    if (ports.length > 0) {
      entity.ports = ports;
    }
    const dependencies = parseList(dependenciesText);
    if (dependencies.length > 0) {
      entity.dependencies = { services: dependencies };
    }
    if (Object.keys(environment).length > 0) {
      entity.environment = environment;
    }
    onSave(entity);
  };

  return (
    <Dialog
      title={editing ? `Edit service “${initial.id}”` : 'Add service'}
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
            disabled={!valid}
          >
            Review change
          </button>
        </>
      }
    >
      <div className="config-form">
        <div className="config-form__grid">
          <label className="config-form__field">
            <span className="toolbar__field-label">Identifier</span>
            <input
              className="input"
              value={id}
              disabled={editing}
              placeholder="orders-api"
              aria-label="Identifier"
              onChange={(event) => setId(event.target.value)}
            />
            {!idValid ? (
              <span className="config-form__error">An identifier is required.</span>
            ) : null}
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Name</span>
            <input
              className="input"
              value={name}
              placeholder="orders-api"
              onChange={(event) => setName(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Type</span>
            <select
              className="input input--select"
              value={serviceType}
              onChange={(event) => setServiceType(event.target.value)}
            >
              <option value="backend">backend</option>
              <option value="frontend">frontend</option>
              <option value="library">library</option>
            </select>
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Workspace</span>
            <input className="input" value={workspaceId} readOnly />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Path (relative to root)</span>
            <input
              className="input"
              value={path}
              placeholder="backend/orders-api"
              onChange={(event) => setPath(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Default branch</span>
            <input
              className="input"
              value={defaultBranch}
              placeholder="main"
              onChange={(event) => setDefaultBranch(event.target.value)}
            />
          </label>
        </div>

        <fieldset className="config-form__group">
          <legend className="toolbar__field-label">Repository</legend>
          <label className="config-form__field">
            <span className="toolbar__field-label">Clone URL</span>
            <input
              className="input"
              value={repositoryUrl}
              placeholder="https://example.com/orders-api.git"
              onChange={(event) => setRepositoryUrl(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Provider</span>
            <input
              className="input"
              value={provider}
              placeholder="github"
              onChange={(event) => setProvider(event.target.value)}
            />
          </label>
        </fieldset>

        <fieldset className="config-form__group">
          <legend className="toolbar__field-label">Build and runtime</legend>
          <div className="config-form__grid">
            <label className="config-form__field">
              <span className="toolbar__field-label">Build system</span>
              <select
                className="input input--select"
                value={buildSystemState}
                onChange={(event) => setBuildSystemState(event.target.value)}
              >
                <option value="">none</option>
                <option value="maven">maven</option>
                <option value="npm">npm</option>
              </select>
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Java version</span>
              <input
                className="input"
                value={javaVersion}
                placeholder="21"
                onChange={(event) => setJavaVersion(event.target.value)}
              />
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Node version</span>
              <input
                className="input"
                value={nodeVersion}
                placeholder="20.x"
                onChange={(event) => setNodeVersion(event.target.value)}
              />
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Build command</span>
              <input
                className="input"
                value={buildCommand}
                placeholder="./mvnw clean install"
                onChange={(event) => setBuildCommand(event.target.value)}
              />
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Start command</span>
              <input
                className="input"
                value={startCommand}
                placeholder="./mvnw spring-boot:run"
                onChange={(event) => setStartCommand(event.target.value)}
              />
            </label>
          </div>
        </fieldset>

        <fieldset className="config-form__group">
          <legend className="toolbar__field-label">Migration (Liquibase)</legend>
          <p className="config-form__hint muted">
            Liquibase configuration is preserved from the existing file; this
            editor does not enable or change migrations.
          </p>
        </fieldset>

        <fieldset className="config-form__group">
          <legend className="toolbar__field-label">Health</legend>
          <div className="config-form__grid">
            <label className="config-form__field">
              <span className="toolbar__field-label">Primary check</span>
              <select
                className="input input--select"
                value={healthKind}
                onChange={(event) => setHealthKind(event.target.value)}
              >
                <option value="none">none</option>
                <option value="http">http</option>
                <option value="port">port</option>
              </select>
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Health URL</span>
              <input
                className="input"
                value={healthUrl}
                disabled={healthKind !== 'http'}
                placeholder="http://localhost:8081/actuator/health"
                onChange={(event) => setHealthUrl(event.target.value)}
              />
            </label>
            <label className="config-form__field">
              <span className="toolbar__field-label">Health port</span>
              <input
                className="input"
                value={healthPort}
                disabled={healthKind !== 'port'}
                placeholder="8081"
                onChange={(event) => setHealthPort(event.target.value)}
              />
            </label>
          </div>
        </fieldset>

        <div className="config-form__grid">
          <label className="config-form__field">
            <span className="toolbar__field-label">Ports</span>
            <input
              className="input"
              value={portsText}
              placeholder="8081, 9090"
              onChange={(event) => setPortsText(event.target.value)}
            />
          </label>
          <label className="config-form__field">
            <span className="toolbar__field-label">Service dependencies</span>
            <input
              className="input"
              value={dependenciesText}
              placeholder="admin-ui, shared-kit"
              onChange={(event) => setDependenciesText(event.target.value)}
            />
          </label>
        </div>

        <EnvironmentEditor value={environment} onChange={setEnvironment} />

        <fieldset className="config-form__group">
          <legend className="toolbar__field-label">
            Detect from a repository (read-only)
          </legend>
          <label className="config-form__field">
            <span className="toolbar__field-label">Local path</span>
            <input
              className="input"
              value={probePath}
              placeholder="/Users/dev/Projects/demo/backend/orders-api"
              onChange={(event) => setProbePath(event.target.value)}
            />
          </label>
          <RepositoryDetectionPanel
            probe={probe}
            suggestion={suggestion}
            loading={detecting}
            error={detectionError}
            onApply={apply}
            onInspect={() => void inspect()}
            onDetect={() => void detect()}
          />
        </fieldset>

        <p className="config-form__hint muted">
          Detection never runs a build or a script. The daemon validates the
          whole file before anything is saved.
        </p>
      </div>
    </Dialog>
  );
}
