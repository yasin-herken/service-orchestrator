import type { RepositorySummary } from '../types/models';
import { compareNullableString, type SortDirection } from './compare';

/** User-facing filters for the Git repository list. */
export interface RepositoryFilter {
  /** Case-insensitive substring match over name, id, path, and branch. */
  search: string;
  /** The hosting provider (repository "type"), or `'all'`. */
  provider: string;
  /** The current branch, or `'all'`. */
  branch: string;
  /** The last observed Git state, or `'all'`. */
  gitState: string;
}

/** Sortable columns of the repository table. */
export type RepositorySortKey = 'name' | 'branch' | 'git_state' | 'path';

/** Applies the toolbar filters to a repository list. */
export function filterRepositories(
  repositories: readonly RepositorySummary[],
  filter: RepositoryFilter,
): RepositorySummary[] {
  const needle = filter.search.trim().toLowerCase();
  return repositories.filter((repository) => {
    if (filter.provider !== 'all' && (repository.repository?.provider ?? '') !== filter.provider) {
      return false;
    }
    if (filter.branch !== 'all' && (repository.branch ?? '') !== filter.branch) {
      return false;
    }
    if (filter.gitState !== 'all' && repository.git_state !== filter.gitState) {
      return false;
    }
    if (needle.length === 0) {
      return true;
    }
    return (
      repository.id.toLowerCase().includes(needle) ||
      repository.service_id.toLowerCase().includes(needle) ||
      repository.name.toLowerCase().includes(needle) ||
      (repository.path ?? '').toLowerCase().includes(needle) ||
      (repository.branch ?? '').toLowerCase().includes(needle)
    );
  });
}

/** Sorts a repository list by one column without mutating the input. */
export function sortRepositories(
  repositories: readonly RepositorySummary[],
  key: RepositorySortKey,
  direction: SortDirection,
): RepositorySummary[] {
  const factor = direction === 'asc' ? 1 : -1;
  return [...repositories].sort((a, b) => {
    let result: number;
    switch (key) {
      case 'name':
        result = a.name.localeCompare(b.name);
        break;
      case 'branch':
        result = compareNullableString(a.branch, b.branch);
        break;
      case 'git_state':
        result = a.git_state.localeCompare(b.git_state);
        break;
      case 'path':
        result = compareNullableString(a.path, b.path);
        break;
    }
    if (result === 0) {
      // Stable, deterministic tie-break on the workspace-aware id.
      result = a.id.localeCompare(b.id);
    }
    return result * factor;
  });
}

/** The distinct branches present across a repository list, sorted. */
export function distinctBranches(repositories: readonly RepositorySummary[]): string[] {
  const branches = new Set<string>();
  for (const repository of repositories) {
    if (repository.branch) {
      branches.add(repository.branch);
    }
  }
  return [...branches].sort();
}

/** The distinct hosting providers across a repository list, sorted. */
export function distinctProviders(repositories: readonly RepositorySummary[]): string[] {
  const providers = new Set<string>();
  for (const repository of repositories) {
    const provider = repository.repository?.provider;
    if (provider) {
      providers.add(provider);
    }
  }
  return [...providers].sort();
}
