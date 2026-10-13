import { describe, expect, it } from 'vitest';

import { commandText, parseList, structuredCommandLine, toCommand } from './configuration';

describe('parseList', () => {
  it('splits on commas and whitespace, trims, dedupes, and keeps order', () => {
    expect(parseList(' b, a   c ,a\nb ')).toEqual(['b', 'a', 'c']);
  });

  it('returns an empty list for blank input', () => {
    expect(parseList('   ')).toEqual([]);
    expect(parseList('')).toEqual([]);
  });
});

describe('toCommand', () => {
  it('returns undefined for a blank line', () => {
    expect(toCommand('   ')).toBeUndefined();
  });

  it('returns the trimmed line otherwise', () => {
    expect(toCommand('  mvn clean install  ')).toBe('mvn clean install');
  });
});

describe('commandText', () => {
  it('renders a short-string command verbatim', () => {
    expect(commandText('./mvnw clean install')).toBe('./mvnw clean install');
  });

  it('renders a structured command as one line', () => {
    expect(commandText({ program: 'mvn', args: ['clean', 'install'] })).toBe('mvn clean install');
  });

  it('renders an undefined command as an empty string', () => {
    expect(commandText(undefined)).toBe('');
  });
});

describe('structuredCommandLine', () => {
  it('joins the program and its arguments', () => {
    expect(structuredCommandLine({ program: 'node', args: ['server.js'] })).toBe('node server.js');
    expect(structuredCommandLine({ program: 'node' })).toBe('node');
  });
});
