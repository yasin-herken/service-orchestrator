import type {
  AffectedReferenceView,
  RunningServiceImpactView,
  ValidationIssueView,
} from '../../types/models';
import { CONFIG_KIND_LABELS } from './ConfigurationDiff';

/**
 * The validation problems of a proposed change (`tasks/023.md` §35).
 *
 * Every issue already carries a location, the problem, the expectation, and a
 * suggested fix, straight from the shared configuration validator, so the panel
 * renders them verbatim instead of re-deriving meaning.
 */
export function ConfigurationValidationPanel({
  revision,
  valid,
  issues = [],
}: {
  revision: string;
  valid: boolean;
  /** The daemon omits the list when there is nothing to report. */
  issues?: ValidationIssueView[];
}) {
  if (valid && issues.length === 0) {
    return (
      <p className="config-validation config-validation--ok" role="status">
        <span className="badge badge--positive">Valid</span> The proposed
        configuration passes validation at revision <code>{revision}</code>.
      </p>
    );
  }

  return (
    <div className="config-validation config-validation--error" role="alert">
      <p>
        <span className="badge badge--negative">Invalid</span>{' '}
        {issues.length} problem{issues.length === 1 ? '' : 's'} must be resolved
        before saving.
      </p>
      <ul className="config-validation__list">
        {issues.map((issue, index) => (
          <li
            key={`${issue.location}:${index}`}
            className="config-validation__issue"
          >
            <code className="config-validation__location">{issue.location}</code>
            <p className="config-validation__problem">{issue.problem}</p>
            {issue.expected ? (
              <p className="config-validation__detail muted">
                Expected: {issue.expected}
              </p>
            ) : null}
            {issue.suggested_fix ? (
              <p className="config-validation__detail muted">
                Fix: {issue.suggested_fix}
              </p>
            ) : null}
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * References from other entities that point at a change target.
 *
 * A removal that would orphan a reference is surfaced here rather than
 * cascaded: the daemon never silently deletes another entity
 * (`tasks/023.md` §15, §16).
 */
export function AffectedReferences({
  references,
}: {
  references: AffectedReferenceView[];
}) {
  if (references.length === 0) {
    return null;
  }
  return (
    <section className="config-impact" aria-label="Affected references">
      <h4 className="config-diff__title">
        <span className="badge badge--warning">Affected references</span>
      </h4>
      <ul className="config-validation__list">
        {references.map((reference, index) => (
          <li key={`${reference.source_id}:${index}`} className="config-impact__row">
            {CONFIG_KIND_LABELS[reference.source_kind]} “{reference.source_id}”
            references “{reference.target}” ({reference.relation}).
          </li>
        ))}
      </ul>
    </section>
  );
}

/**
 * The impact of a change on running processes (`tasks/023.md` §9, §29).
 *
 * The daemon never restarts or stops a service as a side effect of a
 * configuration change; it reports the required action instead.
 */
export function RunningImpact({
  impact,
}: {
  impact: RunningServiceImpactView[];
}) {
  if (impact.length === 0) {
    return null;
  }
  return (
    <section className="config-impact" aria-label="Running services affected">
      <h4 className="config-diff__title">
        <span className="badge badge--warning">Running services affected</span>
      </h4>
      <ul className="config-validation__list">
        {impact.map((entry) => (
          <li key={entry.service_id} className="config-impact__row">
            <strong>{entry.service_id}</strong>
            {entry.running ? ' is running' : ' is not running'} —{' '}
            {entry.action_required}
          </li>
        ))}
      </ul>
    </section>
  );
}
