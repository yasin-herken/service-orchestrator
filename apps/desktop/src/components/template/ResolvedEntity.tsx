import type {
  BulkImportProposalView,
  RawLibraryView,
  RawServiceView,
} from '../../types/models';
import { commandText } from '../../utils/configuration';

/**
 * Renders a daemon-composed entity exactly as it would be written.
 *
 * The component is shared by the template preview and the bulk-import preview
 * so both show the same values from the same planner result; it never derives,
 * defaults, or re-classifies anything (`tasks/032.md` §10 step 9, §14).
 */
export function ResolvedEntity({ proposal }: { proposal: BulkImportProposalView }) {
  const entity: RawServiceView | RawLibraryView =
    proposal.entity === 'service' ? proposal.service : proposal.library;
  const service = proposal.entity === 'service' ? proposal.service : null;
  const runtimes = entity.runtimes ?? (entity.runtime ? [entity.runtime] : []);
  const environment = Object.keys(entity.environment ?? {});

  return (
    <div className="resolved-entity" aria-label="Resolved configuration">
      <dl className="detail-grid">
        <div>
          <dt>Entity</dt>
          <dd>
            {proposal.entity} “{entity.id}”
          </dd>
        </div>
        {service ? (
          <div>
            <dt>Service type</dt>
            <dd>{service.type}</dd>
          </div>
        ) : null}
        <div>
          <dt>Workspace</dt>
          <dd>{entity.workspace}</dd>
        </div>
        <div>
          <dt>Path</dt>
          <dd>{entity.path ?? '—'}</dd>
        </div>
        <div>
          <dt>Repository</dt>
          <dd>{entity.repository?.url ?? '—'}</dd>
        </div>
        <div>
          <dt>Default branch</dt>
          <dd>{entity.default_branch ?? '—'}</dd>
        </div>
        <div>
          <dt>Build system</dt>
          <dd>{entity.build_system ?? '—'}</dd>
        </div>
        <div>
          <dt>Build operation</dt>
          <dd>{entity.build_operation ?? '—'}</dd>
        </div>
        <div>
          <dt>Runtimes</dt>
          <dd>
            {runtimes.length > 0
              ? runtimes
                  .map((runtime) => `${runtime.type} ${runtime.version ?? '(any)'}`)
                  .join(', ')
              : '—'}
          </dd>
        </div>
        <div>
          <dt>Ports</dt>
          <dd>
            {(service?.ports ?? []).map((port) => port.port).join(', ') || '—'}
          </dd>
        </div>
        <div>
          <dt>Build command</dt>
          <dd>{commandText(entity.commands?.build) || '—'}</dd>
        </div>
        <div>
          <dt>Start command</dt>
          <dd>{commandText(entity.commands?.start) || '—'}</dd>
        </div>
        <div>
          <dt>Migration</dt>
          <dd>
            {service?.liquibase
              ? `${service.liquibase.enabled === false ? 'disabled' : 'enabled'}${
                  service.liquibase.preset ? ` · preset ${service.liquibase.preset}` : ''
                }`
              : '—'}
          </dd>
        </div>
        <div>
          <dt>Health checks</dt>
          <dd>
            {(service?.health_checks ?? [])
              .map((check) => (check.type === 'http' ? check.url : check.type))
              .join(', ') || '—'}
          </dd>
        </div>
        <div>
          <dt>Environment</dt>
          <dd>{environment.length > 0 ? environment.join(', ') : '—'}</dd>
        </div>
      </dl>
    </div>
  );
}
