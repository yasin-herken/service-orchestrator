/**
 * Shared helpers for the configuration editors.
 *
 * The service, library, profile, and group editors all parse the same
 * comma/whitespace separated identifier lists and serialize the same
 * string-or-structured command shape, so those rules live here instead of being
 * copied into each editor.
 */

import type { RawCommandView } from '../types/models';

/** Parses a comma/whitespace separated list into unique, ordered ids. */
export function parseList(text: string): string[] {
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const raw of text.split(/[\s,]+/)) {
    const id = raw.trim();
    if (id !== '' && !seen.has(id)) {
      seen.add(id);
      ids.push(id);
    }
  }
  return ids;
}

/** A non-empty command line, or `undefined`. */
export function toCommand(line: string): RawCommandView | undefined {
  const trimmed = line.trim();
  return trimmed === '' ? undefined : trimmed;
}

/** Renders a structured command as a single line. */
export function structuredCommandLine(command: { program: string; args?: string[] }): string {
  return [command.program, ...(command.args ?? [])].join(' ');
}

/** Renders a command, whether short-string or structured, as one line. */
export function commandText(command: RawCommandView | undefined): string {
  if (command === undefined) {
    return '';
  }
  return typeof command === 'string' ? command : structuredCommandLine(command);
}
