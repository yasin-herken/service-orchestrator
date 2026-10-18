import type {
  BuildOperationNameView,
  BuildSystemView,
  BulkImportInputView,
  BulkImportOverridesView,
  ExistingEntityPolicyView,
  MigrationTargetEnvironmentView,
  ServiceTypeView,
} from '../../types/models';

/** The editable state of one repository in the import wizard. */
export interface ImportRowDraft {
  relativePath: string;
  selected: boolean;
  /** The explicitly assigned template; empty means automatic matching. */
  template: string;
  /** The entity id override; empty derives it from the directory name. */
  name: string;
  /** A comma-separated port list. */
  ports: string;
  branch: string;
  buildOperation: string;
  /** `''` inherits, `'true'` enables, `'false'` disables migrations. */
  migration: '' | 'true' | 'false';
  migrationPreset: string;
  databaseEnvironment: string;
  healthUrl: string;
  buildCommand: string;
  startCommand: string;
  /** What to do when an entity already occupies the path. */
  existing: ExistingEntityPolicyView;
}

/** The editable shared overrides applied to every selected repository. */
export interface BulkOverridesDraft {
  serviceType: string;
  defaultBranch: string;
  buildSystem: string;
  buildOperation: string;
  migration: '' | 'true' | 'false';
  migrationPreset: string;
  databaseEnvironment: string;
  healthUrl: string;
  buildCommand: string;
  startCommand: string;
}

/** A new, unselected row draft for a discovered repository. */
export function newRowDraft(relativePath: string): ImportRowDraft {
  return {
    relativePath,
    selected: true,
    template: '',
    name: '',
    ports: '',
    branch: '',
    buildOperation: '',
    migration: '',
    migrationPreset: '',
    databaseEnvironment: '',
    healthUrl: '',
    buildCommand: '',
    startCommand: '',
    existing: 'skip',
  };
}

/** Empty shared overrides: nothing is overridden until the user says so. */
export function emptyBulkOverrides(): BulkOverridesDraft {
  return {
    serviceType: '',
    defaultBranch: '',
    buildSystem: '',
    buildOperation: '',
    migration: '',
    migrationPreset: '',
    databaseEnvironment: '',
    healthUrl: '',
    buildCommand: '',
    startCommand: '',
  };
}

/** Parses a comma-separated port list, reporting invalid tokens. */
export function parsePorts(text: string): { ports: number[]; invalid: string[] } {
  const ports: number[] = [];
  const invalid: string[] = [];
  for (const token of text.split(/[\s,]+/)) {
    const trimmed = token.trim();
    if (trimmed === '') {
      continue;
    }
    const value = Number(trimmed);
    if (!Number.isInteger(value) || value < 1 || value > 65535) {
      invalid.push(trimmed);
      continue;
    }
    if (!ports.includes(value)) {
      ports.push(value);
    }
  }
  return { ports, invalid };
}

/** Parses the optional port base. */
export function parsePortBase(text: string): {
  portBase: number | undefined;
  invalid: boolean;
} {
  if (text.trim() === '') {
    return { portBase: undefined, invalid: false };
  }
  const value = Number(text);
  if (!Number.isInteger(value) || value < 1 || value > 65535) {
    return { portBase: undefined, invalid: true };
  }
  return { portBase: value, invalid: false };
}

/** Maps a draft migration flag to the wire value. */
function migrationEnabled(value: '' | 'true' | 'false'): boolean | undefined {
  if (value === '') {
    return undefined;
  }
  return value === 'true';
}

/** Builds the override object for one row. */
export function rowOverrides(row: ImportRowDraft): BulkImportOverridesView {
  const overrides: BulkImportOverridesView = {};
  if (row.name.trim() !== '') {
    overrides.name = row.name.trim();
  }
  const { ports } = parsePorts(row.ports);
  if (ports.length > 0) {
    overrides.ports = ports;
  }
  if (row.branch.trim() !== '') {
    overrides.default_branch = row.branch.trim();
  }
  if (row.buildOperation !== '') {
    overrides.build_operation = row.buildOperation as BuildOperationNameView;
  }
  const migration = migrationEnabled(row.migration);
  if (migration !== undefined) {
    overrides.migration_enabled = migration;
  }
  if (row.migrationPreset !== '') {
    overrides.migration_preset = row.migrationPreset;
  }
  if (row.databaseEnvironment !== '') {
    overrides.database_environment =
      row.databaseEnvironment as MigrationTargetEnvironmentView;
  }
  if (row.healthUrl.trim() !== '') {
    overrides.health_url = row.healthUrl.trim();
  }
  if (row.buildCommand.trim() !== '') {
    overrides.build_command = row.buildCommand.trim();
  }
  if (row.startCommand.trim() !== '') {
    overrides.start_command = row.startCommand.trim();
  }
  return overrides;
}

/** Builds the shared override object. */
export function bulkOverrides(draft: BulkOverridesDraft): BulkImportOverridesView {
  const overrides: BulkImportOverridesView = {};
  if (draft.serviceType !== '') {
    overrides.service_type = draft.serviceType as ServiceTypeView;
  }
  if (draft.defaultBranch.trim() !== '') {
    overrides.default_branch = draft.defaultBranch.trim();
  }
  if (draft.buildSystem !== '') {
    overrides.build_system = draft.buildSystem as BuildSystemView;
  }
  if (draft.buildOperation !== '') {
    overrides.build_operation = draft.buildOperation as BuildOperationNameView;
  }
  const migration = migrationEnabled(draft.migration);
  if (migration !== undefined) {
    overrides.migration_enabled = migration;
  }
  if (draft.migrationPreset !== '') {
    overrides.migration_preset = draft.migrationPreset;
  }
  if (draft.databaseEnvironment !== '') {
    overrides.database_environment =
      draft.databaseEnvironment as MigrationTargetEnvironmentView;
  }
  if (draft.healthUrl.trim() !== '') {
    overrides.health_url = draft.healthUrl.trim();
  }
  if (draft.buildCommand.trim() !== '') {
    overrides.build_command = draft.buildCommand.trim();
  }
  if (draft.startCommand.trim() !== '') {
    overrides.start_command = draft.startCommand.trim();
  }
  return overrides;
}

/**
 * Builds the daemon request.
 *
 * Only selected repositories are sent. The shared overrides are included as
 * given; precedence between shared, per-repository, and template values is
 * resolved by the daemon (`tasks/032.md` §6, §11, §12).
 */
export function buildImportRequest(
  workspaceId: string,
  rows: readonly ImportRowDraft[],
  draft: BulkOverridesDraft,
  portBase: number | undefined,
): BulkImportInputView {
  return {
    workspace_id: workspaceId,
    repositories: rows
      .filter((row) => row.selected)
      .map((row) => ({
        relative_path: row.relativePath,
        ...(row.template !== '' ? { template: row.template } : {}),
        existing: row.existing,
        overrides: rowOverrides(row),
      })),
    bulk: bulkOverrides(draft),
    ...(portBase !== undefined ? { port_base: portBase } : {}),
  };
}

/** A stable key for detecting a stale plan after any edit. */
export function requestKey(input: BulkImportInputView): string {
  return JSON.stringify(input);
}

/**
 * Returns the locally detectable input problems.
 *
 * The daemon is still authoritative; these checks only prevent sending input
 * that is known to be malformed (an invalid port token or port base) and
 * silently changing its meaning.
 */
export function draftProblems(
  rows: readonly ImportRowDraft[],
  portBaseText: string,
): string[] {
  const problems: string[] = [];
  if (parsePortBase(portBaseText).invalid) {
    problems.push('The port base must be a whole number between 1 and 65535.');
  }
  for (const row of rows) {
    if (!row.selected) {
      continue;
    }
    const { invalid } = parsePorts(row.ports);
    if (invalid.length > 0) {
      problems.push(
        `${row.relativePath}: invalid port ${invalid.map((token) => `“${token}”`).join(', ')}.`,
      );
    }
  }
  return problems;
}
