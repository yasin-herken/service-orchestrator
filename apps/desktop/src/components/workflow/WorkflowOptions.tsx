import type { PrepareEnvironmentInput } from '../../types/models';

/** The editable workflow options; a subset of `PrepareEnvironmentInput`. */
export interface WorkflowOptionsValue {
  /** Optional branch to check out after synchronizing. */
  branch: string;
  sync: boolean;
  include_dependencies: boolean;
  build_libraries: boolean;
  run_liquibase: boolean;
  build: boolean;
  start: boolean;
  wait_for_health: boolean;
}

/** The safe defaults: sync and dependency inclusion on, nothing else. */
export function defaultWorkflowOptions(): WorkflowOptionsValue {
  return {
    branch: '',
    sync: true,
    include_dependencies: true,
    build_libraries: false,
    run_liquibase: false,
    build: false,
    start: false,
    wait_for_health: true,
  };
}

/**
 * Workflow options in display form (`tasks/022.md` §10, §11).
 *
 * Every control maps onto an existing `PrepareEnvironmentRequest` field; no
 * flag exists here that the core would silently ignore, and the defaults are
 * the safe ones — no silent Git discard, no automatic migrations, no forced
 * branch checkout, no auto-start of the workspace.
 */
export function WorkflowOptions({
  value,
  disabled = false,
  onChange,
}: {
  value: WorkflowOptionsValue;
  disabled?: boolean;
  onChange: (next: WorkflowOptionsValue) => void;
}) {
  const toggles: {
    key: keyof Omit<WorkflowOptionsValue, 'branch'>;
    label: string;
    hint: string;
  }[] = [
    {
      key: 'sync',
      label: 'Synchronize repositories (fetch)',
      hint: 'Fetches and fast-forwards; local changes are never discarded.',
    },
    {
      key: 'include_dependencies',
      label: 'Include required dependencies',
      hint: 'Adds required service dependencies resolved by the daemon plan.',
    },
    {
      key: 'build_libraries',
      label: 'Build required libraries',
      hint: 'Runs the library preparation workflow before service builds.',
    },
    {
      key: 'run_liquibase',
      label: 'Run migrations',
      hint: 'Explicit database mutation; requires the daemon’s authorization.',
    },
    { key: 'build', label: 'Build services', hint: 'Builds each selected service.' },
    {
      key: 'start',
      label: 'Start services',
      hint: 'Starts the selected services; never starts the whole workspace.',
    },
    {
      key: 'wait_for_health',
      label: 'Wait for readiness',
      hint: 'Runs the configured health checks after startup.',
    },
  ];

  return (
    <fieldset className="options-form">
      <legend className="selection-list__legend">Workflow options</legend>
      <label className="options-form__field">
        <span>Target branch (optional)</span>
        <input
          type="text"
          className="input"
          placeholder="Leave empty to keep the current branch"
          value={value.branch}
          disabled={disabled}
          onChange={(event) => onChange({ ...value, branch: event.target.value })}
          aria-label="Target branch"
        />
        <span className="muted">
          No branch is checked out unless you type one; nothing defaults to master.
        </span>
      </label>
      {toggles.map((toggle) => (
        <label key={toggle.key} className="options-form__field options-form__field--indent">
          <span>
            <input
              type="checkbox"
              checked={value[toggle.key]}
              disabled={disabled}
              onChange={(event) => onChange({ ...value, [toggle.key]: event.target.checked })}
              aria-label={toggle.label}
            />{' '}
            {toggle.label}
          </span>
          <span className="muted">{toggle.hint}</span>
        </label>
      ))}
    </fieldset>
  );
}

/** Maps the edited options plus a selection onto the daemon request. */
export function toPrepareInput(
  workspaceId: string,
  selection: { serviceIds: string[]; profileId: string | null; groupId: string | null },
  options: WorkflowOptionsValue,
): PrepareEnvironmentInput {
  const branch = options.branch.trim();
  return {
    workspace_id: workspaceId,
    ...(selection.serviceIds.length > 0 ? { services: [...selection.serviceIds].sort() } : {}),
    ...(selection.profileId ? { profile: selection.profileId } : {}),
    ...(selection.groupId ? { group: selection.groupId } : {}),
    ...(branch.length > 0 ? { branch } : {}),
    sync: options.sync,
    include_dependencies: options.include_dependencies,
    build_libraries: options.build_libraries,
    run_liquibase: options.run_liquibase,
    build: options.build,
    start: options.start,
    wait_for_health: options.wait_for_health,
  };
}
