import type {
  ConfigurationDiffView,
  EntityKindView,
  EntitySummaryView,
  UpdatedEntityView,
} from '../../types/models';

/** Human-readable labels for the entity kinds. */
export const CONFIG_KIND_LABELS: Record<EntityKindView, string> = {
  workspace: 'Workspace',
  service: 'Service',
  library: 'Library',
  profile: 'Profile',
  group: 'Group',
  template: 'Template',
  migration_preset: 'Migration preset',
  operation_preset: 'Operation preset',
};

/** A short `Kind 'id'` label for an entity summary. */
function summaryLabel(summary: EntitySummaryView): string {
  return `${CONFIG_KIND_LABELS[summary.kind]} “${summary.name ?? summary.id}”`;
}

/** One removed or added entity. */
function EntityRow({ summary }: { summary: EntitySummaryView }) {
  return <li className="config-diff__entity">{summaryLabel(summary)}</li>;
}

/** One updated entity and its changed fields. */
function UpdatedRow({ updated }: { updated: UpdatedEntityView }) {
  return (
    <li className="config-diff__entity">
      <span>{summaryLabel(updated)}</span>
      {updated.fields.length > 0 ? (
        <ul className="config-diff__fields">
          {updated.fields.map((field) => (
            <li key={field.field} className="config-diff__field">
              <code className="config-diff__path">{field.field}</code>
              <span className="config-diff__before">
                {field.before ?? <em className="muted">(unset)</em>}
              </span>
              <span aria-hidden="true">→</span>
              <span className="config-diff__after">
                {field.after ?? <em className="muted">(removed)</em>}
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <span className="muted"> (no field-level changes)</span>
      )}
    </li>
  );
}

/**
 * A readable, structured configuration diff (`tasks/023.md` §34).
 *
 * It is rendered entirely from the daemon's [`ConfigurationDiffView`]: added,
 * updated (with field-level before/after), and removed entities. Secret values
 * are already redacted by the daemon, so nothing sensitive is displayed here.
 */
export function ConfigurationDiff({ diff }: { diff: ConfigurationDiffView }) {
  const empty =
    diff.added.length === 0 && diff.updated.length === 0 && diff.removed.length === 0;

  if (empty) {
    return <p className="muted">This change makes no structural difference.</p>;
  }

  return (
    <div className="config-diff">
      {diff.added.length > 0 ? (
        <section className="config-diff__group" aria-label="Added">
          <h4 className="config-diff__title">
            <span className="badge badge--positive">Added</span>
          </h4>
          <ul className="config-diff__list">
            {diff.added.map((summary) => (
              <EntityRow key={`${summary.kind}:${summary.id}`} summary={summary} />
            ))}
          </ul>
        </section>
      ) : null}

      {diff.updated.length > 0 ? (
        <section className="config-diff__group" aria-label="Updated">
          <h4 className="config-diff__title">
            <span className="badge badge--info">Updated</span>
          </h4>
          <ul className="config-diff__list">
            {diff.updated.map((updated) => (
              <UpdatedRow key={`${updated.kind}:${updated.id}`} updated={updated} />
            ))}
          </ul>
        </section>
      ) : null}

      {diff.removed.length > 0 ? (
        <section className="config-diff__group" aria-label="Removed">
          <h4 className="config-diff__title">
            <span className="badge badge--negative">Removed</span>
          </h4>
          <ul className="config-diff__list">
            {diff.removed.map((summary) => (
              <EntityRow key={`${summary.kind}:${summary.id}`} summary={summary} />
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}
