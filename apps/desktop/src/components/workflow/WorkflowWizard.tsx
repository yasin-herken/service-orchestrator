/**
 * The four-step wizard indicator: Configure → Plan → Execute → Results.
 *
 * It is presentation only: the owning page holds the mode, and the steps are
 * labels — no workflow logic lives here (`tasks/022.md` §38).
 */
export type WorkflowMode = 'configure' | 'plan' | 'running' | 'results';

/** The ordered wizard steps. */
const STEPS: { id: WorkflowMode; label: string }[] = [
  { id: 'configure', label: '1. Configure' },
  { id: 'plan', label: '2. Plan' },
  { id: 'running', label: '3. Execute' },
  { id: 'results', label: '4. Results' },
];

/** A read-only stepper showing where the user is in the workflow flow. */
export function WorkflowWizard({ mode }: { mode: WorkflowMode }) {
  const activeIndex = STEPS.findIndex((step) => step.id === mode);
  return (
    <ol className="wizard" aria-label="Workflow steps">
      {STEPS.map((step, index) => {
        const state =
          index < activeIndex ? 'done' : index === activeIndex ? 'current' : 'todo';
        return (
          <li
            key={step.id}
            className={`wizard__step wizard__step--${state}`}
            aria-current={state === 'current' ? 'step' : undefined}
          >
            {step.label}
          </li>
        );
      })}
    </ol>
  );
}
