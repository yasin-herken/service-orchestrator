import { useState } from 'react';

import type { RawEnvValueView } from '../../types/models';

/** A single editable environment entry. */
interface EnvRow {
  /**
   * A stable identity independent of position.
   *
   * The reveal state and React keys are keyed by this id, not by the array
   * index: removing a row must not shift the "revealed" flag onto a different
   * secret, and reordering must not remount unrelated inputs (`tasks/024.md`
   * §5, §12).
   */
  id: string;
  key: string;
  value: string;
  secret: boolean;
}

/** Monotonic source of stable row ids (unique per mounted editor history). */
let rowIdCounter = 0;

function nextRowId(): string {
  rowIdCounter += 1;
  return `env-${rowIdCounter}`;
}

/** Normalizes the wire value into an editable row. */
function toRow(key: string, value: RawEnvValueView): EnvRow {
  if (typeof value === 'string') {
    return { id: nextRowId(), key, value, secret: false };
  }
  return { id: nextRowId(), key, value: value.value, secret: value.secret };
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
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(new Set());

  const commit = (next: EnvRow[]) => {
    setRows(next);
    onChange(toEnvironment(next));
  };

  const update = (id: string, patch: Partial<EnvRow>) => {
    commit(rows.map((row) => (row.id === id ? { ...row, ...patch } : row)));
  };

  const remove = (id: string) => {
    commit(rows.filter((row) => row.id !== id));
    setRevealed((current) => {
      if (!current.has(id)) {
        return current;
      }
      const next = new Set(current);
      next.delete(id);
      return next;
    });
  };

  const toggleReveal = (id: string) => {
    setRevealed((current) => {
      const next = new Set(current);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  return (
    <fieldset className="config-form__group">
      <legend className="toolbar__field-label">{label}</legend>
      {rows.length === 0 ? (
        <p className="muted config-form__hint">No environment variables.</p>
      ) : null}
      <ul className="config-env">
        {rows.map((row, index) => (
          <li key={row.id} className="config-env__row">
            <input
              className="input config-env__key"
              aria-label={`Environment name ${index + 1}`}
              placeholder="NAME"
              value={row.key}
              onChange={(event) => update(row.id, { key: event.target.value })}
            />
            <input
              className="input config-env__value"
              aria-label={`Environment value ${index + 1}`}
              type={row.secret && !revealed.has(row.id) ? 'password' : 'text'}
              placeholder="value"
              value={row.value}
              onChange={(event) => update(row.id, { value: event.target.value })}
            />
            <label className="config-env__secret">
              <input
                type="checkbox"
                checked={row.secret}
                onChange={(event) =>
                  update(row.id, { secret: event.target.checked })
                }
              />
              <span>Secret</span>
            </label>
            {row.secret ? (
              <button
                type="button"
                className="button button--small"
                aria-label={`${
                  revealed.has(row.id) ? 'Hide' : 'Reveal'
                } environment value ${index + 1}`}
                onClick={() => toggleReveal(row.id)}
              >
                {revealed.has(row.id) ? 'Hide' : 'Reveal'}
              </button>
            ) : null}
            <button
              type="button"
              className="button button--small"
              aria-label={`Remove environment variable ${index + 1}`}
              onClick={() => remove(row.id)}
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
      <button
        type="button"
        className="button button--small"
        onClick={() =>
          commit([...rows, { id: nextRowId(), key: '', value: '', secret: false }])
        }
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
