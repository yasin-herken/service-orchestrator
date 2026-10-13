import { useState } from 'react';

import type { RawEnvValueView } from '../../types/models';

/** A single editable environment entry. */
interface EnvRow {
  key: string;
  value: string;
  secret: boolean;
}

/** Normalizes the wire value into an editable row. */
function toRow(key: string, value: RawEnvValueView): EnvRow {
  if (typeof value === 'string') {
    return { key, value, secret: false };
  }
  return { key, value: value.value, secret: value.secret };
}

/** Converts editable rows back into the wire shape, dropping empty keys. */
function toEnvironment(rows: readonly EnvRow[]): Record<string, RawEnvValueView> {
  const environment: Record<string, RawEnvValueView> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (key === '') {
      continue;
    }
    environment[key] = row.secret ? { value: row.value, secret: true } : row.value;
  }
  return environment;
}

/**
 * An editor for a map of environment variables.
 *
 * A value marked `secret` is masked by default and can be revealed per row: the
 * literal is never exposed in a diff or log by the daemon, and the GUI keeps it
 * masked until a developer explicitly asks to see it (`tasks/023.md` §31, §46).
 */
export function EnvironmentEditor({
  value,
  onChange,
  label = 'Environment',
}: {
  value: Record<string, RawEnvValueView>;
  onChange: (next: Record<string, RawEnvValueView>) => void;
  label?: string;
}) {
  const [rows, setRows] = useState<EnvRow[]>(() =>
    Object.entries(value).map(([key, entry]) => toRow(key, entry)),
  );
  const [revealed, setRevealed] = useState<ReadonlySet<number>>(new Set());

  const commit = (next: EnvRow[]) => {
    setRows(next);
    onChange(toEnvironment(next));
  };

  const update = (index: number, patch: Partial<EnvRow>) => {
    commit(rows.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  };

  return (
    <fieldset className="config-form__group">
      <legend className="toolbar__field-label">{label}</legend>
      {rows.length === 0 ? (
        <p className="muted config-form__hint">No environment variables.</p>
      ) : null}
      <ul className="config-env">
        {rows.map((row, index) => (
          <li key={index} className="config-env__row">
            <input
              className="input config-env__key"
              aria-label={`Environment name ${index + 1}`}
              placeholder="NAME"
              value={row.key}
              onChange={(event) => update(index, { key: event.target.value })}
            />
            <input
              className="input config-env__value"
              aria-label={`Environment value ${index + 1}`}
              type={row.secret && !revealed.has(index) ? 'password' : 'text'}
              placeholder="value"
              value={row.value}
              onChange={(event) => update(index, { value: event.target.value })}
            />
            <label className="config-env__secret">
              <input
                type="checkbox"
                checked={row.secret}
                onChange={(event) =>
                  update(index, { secret: event.target.checked })
                }
              />
              <span>Secret</span>
            </label>
            {row.secret ? (
              <button
                type="button"
                className="button button--small"
                aria-label={`Reveal environment value ${index + 1}`}
                onClick={() =>
                  setRevealed((current) => {
                    const next = new Set(current);
                    if (next.has(index)) {
                      next.delete(index);
                    } else {
                      next.add(index);
                    }
                    return next;
                  })
                }
              >
                {revealed.has(index) ? 'Hide' : 'Reveal'}
              </button>
            ) : null}
            <button
              type="button"
              className="button button--small"
              aria-label={`Remove environment variable ${index + 1}`}
              onClick={() => commit(rows.filter((_, i) => i !== index))}
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
      <button
        type="button"
        className="button button--small"
        onClick={() => commit([...rows, { key: '', value: '', secret: false }])}
      >
        Add variable
      </button>
      <p className="config-form__hint muted">
        A secret value is masked here and redacted by the daemon in diffs and
        logs; it is written back verbatim when unchanged.
      </p>
    </fieldset>
  );
}
