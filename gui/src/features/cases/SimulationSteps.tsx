import { ActionButton } from '../../components/ActionButton';
import { simulationSteps, type SimulationStep } from './simulation-model';
export default function SimulationSteps({
  step,
  en,
  disabled = false,
  actionPrefix,
  onChange,
  recheck,
}: {
  step: SimulationStep;
  en: boolean;
  disabled?: boolean;
  /** 助手动作名前缀（`case` / `new`），步骤按钮登记为 `<前缀>-step-<n>`。 */
  actionPrefix: string;
  onChange: (step: SimulationStep) => void;
  /** 因模型结构/方案改变而需要重新检查的步骤与原因（U-03）：导航上标记，悬停说明原因。 */
  recheck?: Partial<Record<SimulationStep, string>>;
}) {
  return (
    <nav className="simulation-steps" aria-label={en ? 'Simulation steps' : '模拟步骤'}>
      {simulationSteps.map((label, i) => {
        const issue = recheck?.[(i + 1) as SimulationStep];
        return (
          <ActionButton
            key={label[0]}
            action={`${actionPrefix}-step-${i + 1}`}
            label={label[en ? 1 : 0]}
            variant="ghost"
            className={`${step === i + 1 ? 'current' : ''} ${step > i + 1 ? 'visited' : ''}`}
            aria-current={step === i + 1 ? 'step' : undefined}
            aria-label={issue ? `${label[en ? 1 : 0]} — ${issue}` : undefined}
            title={issue}
            disabled={disabled || i + 1 > step}
            run={() => onChange((i + 1) as SimulationStep)}
          >
            <span className="step-marker" aria-hidden="true">
              {issue ? '!' : step > i + 1 ? '✓' : i + 1}
            </span>
            <span>{label[en ? 1 : 0]}</span>
            {issue && (
              <span className="muted" style={{ fontSize: 10 }}>
                {en ? 'recheck' : '需重检'}
              </span>
            )}
          </ActionButton>
        );
      })}
    </nav>
  );
}
