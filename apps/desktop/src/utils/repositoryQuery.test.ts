import { describe, expect, it } from 'vitest';

import { makeRepository } from '../test/fixtures';
import type { RepositorySummary } from '../types/models';
import {
  distinctBranches,
  distinctProviders,
  filterRepositories,
  sortRepositories,
} from './repositoryQuery';

/** Two repositories with different branches, providers, and states. */
function sample(): RepositorySummary[] {
  return [
    makeRepository({ id: 'demo:a', name: 'alpha', branch: 'main', git_state: 'clean' }),
    makeRepository({
      id: 'demo:b',
      name: 'beta',
      branch: 'develop',
      git_state: 'modified',
      repository: {
        url: 'https://gitlab.example.com/beta.git',
        local_path: 'beta',
        default_branch: 'develop',
        provider: 'gitlab',
      },
    }),
  ];
}

describe('repositoryQuery', () => {
  it('filters by provider, branch, state, and search text', () => {
    const repositories = sample();

    expect(
      filterRepositories(repositories, {
        search: '',
        provider: 'gitlab',
        branch: 'all',
        gitState: 'all',
      }).map((repository) => repository.id),
    ).toEqual(['demo:b']);

    expect(
      filterRepositories(repositories, {
        search: 'beta',
        provider: 'all',
        branch: 'develop',
        gitState: 'modified',
      }).map((repository) => repository.id),
    ).toEqual(['demo:b']);

    expect(
      filterRepositories(repositories, {
        search: 'zzz',
        provider: 'all',
        branch: 'all',
        gitState: 'all',
      }),
    ).toHaveLength(0);
  });

  it('sorts deterministically with a stable id tie-break', () => {
    const sorted = sortRepositories(sample(), 'git_state', 'asc');
    expect(sorted.map((repository) => repository.git_state)).toEqual(['clean', 'modified']);
  });

  it('collects distinct branches and providers', () => {
    const repositories = sample();
    expect(distinctBranches(repositories)).toEqual(['develop', 'main']);
    expect(distinctProviders(repositories)).toEqual(['github', 'gitlab']);
  });
});
