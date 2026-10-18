/** The ten ordered bulk-import steps (`tasks/032.md` §10). */
export const BULK_IMPORT_STEPS = [
  'Scan workspace',
  'Review discovery',
  'Select repositories',
  'Review technologies',
  'Assign templates',
  'Bulk overrides',
  'Per-repository exceptions',
  'Validate',
  'Preview changes',
  'Import',
] as const;

/**
 * The bulk-import step rail.
 *
 * Presentation only: the page owns the state and decides what each step does.
 * The rail makes the ten-step flow explicit so the user always knows whether
 * they are reviewing, validating, or committing (`tasks/032.md` §10).
 */
export function BulkImportWizard({ step }: { step: number }) {
  return (
    <ol className="wizard wizard--compact" aria-label="Bulk import steps">
      {BULK_IMPORT_STEPS.map((label, index) => {
        const position = index + 1;
        const state =
          position < step ? 'done' : position === step ? 'current' : 'todo';
        return (
          <li
            key={label}
            className={`wizard__step wizard__step--${state}`}
            aria-current={position === step ? 'step' : undefined}
          >
            {position}. {label}
          </li>
        );
      })}
    </ol>
  );
}
