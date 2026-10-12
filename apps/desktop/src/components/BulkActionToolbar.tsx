/**
 * The selection summary and bulk action bar for the services screen.
 *
 * It only ever acts on the explicit selection — never on the filtered or full
 * list — and disables its actions while disconnected or while any selected
 * service is busy with another operation (`tasks/019.md` §8, §14–§16, §28).
 */
export function BulkActionToolbar({
  selectedCount,
  visibleCount,
  totalCount,
  busySelectedCount,
  allVisibleSelected,
  disabled,
  kinds,
  states,
  onSelectAllVisible,
  onClearSelection,
  onSelectKind,
  onSelectState,
  onStart,
  onStop,
  onRestart,
}: {
  selectedCount: number;
  visibleCount: number;
  totalCount: number;
  busySelectedCount: number;
  allVisibleSelected: boolean;
  disabled: boolean;
  kinds: readonly string[];
  states: readonly string[];
  onSelectAllVisible: () => void;
  onClearSelection: () => void;
  onSelectKind: (kind: string) => void;
  onSelectState: (state: string) => void;
  onStart: () => void;
  onStop: () => void;
  onRestart: () => void;
}) {
  const hasSelection = selectedCount > 0;
  const blocked = disabled || busySelectedCount > 0;

  return (
    <div className="bulk-bar" role="toolbar" aria-label="Service selection actions">
      <span className="bulk-bar__count" aria-live="polite">
        <strong>{selectedCount}</strong> selected
        <span className="muted">
          {' '}
          · {visibleCount} visible · {totalCount} total
        </span>
      </span>

      <div className="bulk-bar__select">
        <button
          type="button"
          className="button button--small"
          onClick={onSelectAllVisible}
          disabled={visibleCount === 0 || allVisibleSelected}
        >
          Select all visible
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onClearSelection}
          disabled={!hasSelection}
        >
          Clear
        </button>
        <label className="bulk-bar__field">
          <span className="visually-hidden">Select services by type</span>
          <select
            className="input input--select"
            value=""
            aria-label="Select services by type"
            disabled={totalCount === 0}
            onChange={(event) => {
              if (event.target.value) {
                onSelectKind(event.target.value);
              }
            }}
          >
            <option value="">Select by type…</option>
            {kinds.map((kind) => (
              <option key={kind} value={kind}>
                All {kind}
              </option>
            ))}
          </select>
        </label>
        <label className="bulk-bar__field">
          <span className="visually-hidden">Select services by runtime state</span>
          <select
            className="input input--select"
            value=""
            aria-label="Select services by runtime state"
            disabled={totalCount === 0}
            onChange={(event) => {
              if (event.target.value) {
                onSelectState(event.target.value);
              }
            }}
          >
            <option value="">Select by state…</option>
            {states.map((state) => (
              <option key={state} value={state}>
                All {state.replace(/_/g, ' ')}
              </option>
            ))}
          </select>
        </label>
      </div>

      <div className="bulk-bar__actions">
        <button
          type="button"
          className="button button--small button--primary"
          onClick={onStart}
          disabled={!hasSelection || blocked}
        >
          Start selected
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onStop}
          disabled={!hasSelection || blocked}
        >
          Stop selected
        </button>
        <button
          type="button"
          className="button button--small"
          onClick={onRestart}
          disabled={!hasSelection || blocked}
        >
          Restart selected
        </button>
      </div>

      {busySelectedCount > 0 ? (
        <span className="bulk-bar__hint muted" role="note">
          {busySelectedCount} selected service{busySelectedCount === 1 ? '' : 's'} busy —
          wait for the current operation to settle.
        </span>
      ) : null}
    </div>
  );
}
