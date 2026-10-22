import { ActionButton } from '../../components/ActionButton';
import { simulationSteps, type SimulationStep } from './simulation-model';
export default function SimulationSteps({
  step,
  en,
  disabled = false,
  actionPrefix,
  onChange,
}: {
  step: SimulationStep;
  en: boolean;
  disabled?: boolean;
  /** 助手动作名前缀（`case` / `new`），步骤按钮登记为 `<前缀>-step-<n>`。 */
  actionPrefix: string;
  onChange: (step: SimulationStep) => void;
}) {
  return (
    <nav className="simulation-steps" aria-label={en ? 'Simulation steps' : '模拟步骤'}>
      {simulationSteps.map((label, i) => (
        <ActionButton
          key={label[0]}
          action={`${actionPrefix}-step-${i + 1}`}
          label={label[en ? 1 : 0]}
          variant="ghost"
          className={`${step === i + 1 ? 'current' : ''} ${step > i + 1 ? 'visited' : ''}`}
          aria-current={step === i + 1 ? 'step' : undefined}
          disabled={disabled || i + 1 > step}
          run={() => onChange((i + 1) as SimulationStep)}
        >
          <span className="step-marker" aria-hidden="true">
            {step > i + 1 ? '✓' : i + 1}
          </span>
          <span>{label[en ? 1 : 0]}</span>
        </ActionButton>
      ))}
    </nav>
  );
}
