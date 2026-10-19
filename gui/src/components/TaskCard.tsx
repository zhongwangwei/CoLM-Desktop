import type { CSSProperties, ReactNode } from 'react';

/**
 * A long wait the user should understand at a glance: what is happening, which step it is at,
 * how far along (or a moving bar when that is unknown) and for how long. Used for the server
 * engine build and for reading sites, which can each take minutes on a shared login node.
 */
export default function TaskCard({
  title,
  waited,
  steps,
  step = 0,
  percent = null,
  detail,
  hint,
  action,
}: {
  title: string;
  waited: string;
  steps?: string[];
  step?: number;
  /** 0–100 when the share done is known; a moving bar otherwise. */
  percent?: number | null;
  detail: string;
  hint?: string;
  action?: ReactNode;
}) {
  return (
    <section className="task-card" role="status" aria-live="polite">
      <header className="task-card-head">
        <strong>{title}</strong>
        <span className="task-card-time">{waited}</span>
      </header>
      {steps && steps.length > 1 && (
        <ol className="task-card-steps" style={{ '--steps': steps.length } as CSSProperties}>
          {steps.map((label, index) => (
            <li
              key={label}
              className={index < step ? 'is-done' : index === step ? 'is-active' : undefined}
              aria-current={index === step ? 'step' : undefined}
            >
              {label}
            </li>
          ))}
        </ol>
      )}
      <div
        className={`task-card-bar${percent === null ? ' is-indeterminate' : ''}`}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
      >
        <span style={percent === null ? undefined : { width: `${Math.max(percent, 2)}%` }} />
      </div>
      <div className="task-card-meta">
        <span className="task-card-detail">{detail}</span>
        {percent !== null && <span className="task-card-percent">{percent}%</span>}
      </div>
      {(hint || action) && (
        <div className="task-card-foot">
          {hint && <p className="task-card-hint">{hint}</p>}
          {action}
        </div>
      )}
    </section>
  );
}
