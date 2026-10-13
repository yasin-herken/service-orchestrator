import { StatusBadge, type BadgeTone } from '../StatusBadge';
import type { AlignmentStatus, VersionAlignmentView } from '../../types/models';

/** Badge tones for the version-alignment statuses. */
const ALIGNMENT_TONES: Record<AlignmentStatus, BadgeTone> = {
  aligned: 'positive',
  mismatch: 'negative',
  unknown: 'neutral',
  unresolved: 'warning',
};

/** A human-readable label for an alignment status. */
export function alignmentLabel(status: AlignmentStatus): string {
  return status.charAt(0).toUpperCase() + status.slice(1);
}

/**
 * The dependency version-alignment view.
 *
 * It reports, per consumer, the version the consumer resolves to and whether it
 * matches the library's target version — distinguishing aligned, mismatch,
 * unknown, and unresolved. It never rewrites a POM or changes a version; it only
 * displays what the Application Core inspected (`tasks/020.md` §23, §24).
 */
export function VersionAlignmentPanel({
  alignment,
}: {
  alignment: VersionAlignmentView;
}) {
  return (
    <section className="version-alignment" aria-label="Version alignment">
      <p className="version-alignment__headline" aria-live="polite">
        Target <code className="dialog__code">{alignment.target.coordinate}</code> ·{' '}
        {alignment.aligned_count} of {alignment.total} consumer
        {alignment.total === 1 ? '' : 's'} aligned
        {alignment.fully_aligned ? '' : ' — not fully aligned'}
      </p>
      {alignment.consumers.length === 0 ? (
        <p className="muted">No consumers depend on this library.</p>
      ) : (
        <table className="service-table service-table--compact">
          <caption className="visually-hidden">Consumer version alignment</caption>
          <thead>
            <tr>
              <th scope="col">Consumer</th>
              <th scope="col">Resolved version</th>
              <th scope="col">Status</th>
              <th scope="col">Detail</th>
            </tr>
          </thead>
          <tbody>
            {alignment.consumers.map((consumer) => (
              <tr key={consumer.service_id}>
                <td>
                  <span className="service-table__name">{consumer.service_id}</span>
                  {consumer.path ? (
                    <span className="service-table__id">{consumer.path}</span>
                  ) : null}
                </td>
                <td className="service-table__mono">
                  {consumer.current_version ?? 'Unknown'}
                </td>
                <td>
                  <StatusBadge
                    value={consumer.status}
                    tone={ALIGNMENT_TONES[consumer.status]}
                    label={alignmentLabel(consumer.status)}
                  />
                </td>
                <td className="muted">{consumer.detail ?? '—'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <p className="dialog__note muted">
        Alignment is reported, never enforced. The GUI does not rewrite Maven
        POM files or change dependency versions.
      </p>
    </section>
  );
}
