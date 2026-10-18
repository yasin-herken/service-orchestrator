import type {
  PrepareEnvironmentInput,
  RawOperationPresetView,
} from '../../types/models';

/** How the runtime phase behaves for the operation. */
export type RuntimeAction = 'none' | 'start' | 'restart' | 'stop';

/** The selection the operation targets: exactly one of the three sources. */
export interface OperationSelectionValue {
  serviceIds: string[];
  profileId: string | null;
  groupId: string | null;
}

/** The editable operation options. */
export interface OperationOptionsValue {
  branch: string;
  sync: boolean;
  includeDependencies: boolean;
  buildLibraries: boolean;
  runLiquibase: boolean;
  migrationPreset: string;
  build: boolean;
  buildOperation: string;
  runtime: RuntimeAction;
  waitForHealth: boolean;
}

/** The safe defaults: synchronize and include dependencies; nothing else runs. */
export function defaultOperationOptions(): OperationOptionsValue {
  return {
    branch: '',
    sync: true,
    includeDependencies: true,
    buildLibraries: false,
    runLiquibase: false,
    migrationPreset: '',
    build: false,
    buildOperation: '',
    runtime: 'none',
    waitForHealth: true,
  };
}

/** Maps a preset's composed request back onto the editable options. */
export function optionsFromRequest(
  request: PrepareEnvironmentInput,
): OperationOptionsValue {
  return {
    branch: request.branch ?? '',
    sync: request.sync ?? true,
    includeDependencies: request.include_dependencies ?? true,
    buildLibraries: request.build_libraries ?? false,
    runLiquibase: request.run_liquibase ?? false,
    migrationPreset: request.migration_preset ?? '',
    build: request.build ?? false,
    buildOperation: request.build_operation ?? '',
    runtime: request.restart
      ? 'restart'
      : request.start
        ? 'start'
        : request.stop
          ? 'stop'
          : 'none',
    waitForHealth: request.wait_for_health ?? true,
  };
}

/** Maps a preset's composed request back onto the selection. */
export function selectionFromRequest(
  request: PrepareEnvironmentInput,
): OperationSelectionValue {
  if (request.profile !== undefined) {
    return { serviceIds: [], profileId: request.profile, groupId: null };
  }
  if (request.group !== undefined) {
    return { serviceIds: [], profileId: null, groupId: request.group };
  }
  return { serviceIds: [...(request.services ?? [])], profileId: null, groupId: null };
}

/** Builds the daemon request; the daemon re-validates it at execution time. */
export function toOperationRequest(
  workspaceId: string,
  selection: OperationSelectionValue,
  options: OperationOptionsValue,
): PrepareEnvironmentInput {
  const branch = options.branch.trim();
  const start = options.runtime === 'start';
  const restart = options.runtime === 'restart';
  const stop = options.runtime === 'stop';
  return {
    workspace_id: workspaceId,
    ...(selection.serviceIds.length > 0
      ? { services: [...selection.serviceIds].sort() }
      : {}),
    ...(selection.profileId ? { profile: selection.profileId } : {}),
    ...(selection.groupId ? { group: selection.groupId } : {}),
    ...(branch !== '' ? { branch } : {}),
    sync: options.sync,
    include_dependencies: options.includeDependencies,
    build_libraries: options.buildLibraries,
    run_liquibase: options.runLiquibase,
    ...(options.runLiquibase && options.migrationPreset !== ''
      ? { migration_preset: options.migrationPreset }
      : {}),
    build: options.build,
    ...(options.build && options.buildOperation !== ''
      ? { build_operation: options.buildOperation }
      : {}),
    start,
    restart,
    stop,
    wait_for_health: options.waitForHealth && (start || restart),
  };
}

/**
 * Builds the raw saved preset for the current selection and options.
 *
 * A preset stores the selection and the operation flags only. It never stores
 * credentials and never carries authorization; loading one only pre-fills this
 * screen (`tasks/032.md` §36, §37).
 */
export function toOperationPreset(
  workspaceId: string,
  selection: OperationSelectionValue,
  options: OperationOptionsValue,
  id: string,
  name: string,
  description: string,
  isDefault: boolean,
): RawOperationPresetView {
  const branch = options.branch.trim();
  return {
    id: id.trim(),
    ...(name.trim() !== '' ? { name: name.trim() } : {}),
    ...(description.trim() !== '' ? { description: description.trim() } : {}),
    workspace: workspaceId,
    ...(isDefault ? { default: true } : {}),
    ...(selection.serviceIds.length > 0
      ? { services: [...selection.serviceIds].sort() }
      : {}),
    ...(selection.profileId ? { profiles: [selection.profileId] } : {}),
    ...(selection.groupId ? { groups: [selection.groupId] } : {}),
    git: {
      enabled: options.sync,
      ...(branch !== '' ? { branch } : {}),
    },
    libraries: { enabled: options.buildLibraries },
    migration: {
      enabled: options.runLiquibase,
      ...(options.runLiquibase && options.migrationPreset !== ''
        ? { preset: options.migrationPreset }
        : {}),
    },
    build: {
      enabled: options.build,
      ...(options.build && options.buildOperation !== ''
        ? { operation: options.buildOperation }
        : {}),
    },
    runtime: { action: options.runtime },
    health: { enabled: options.waitForHealth },
  };
}
