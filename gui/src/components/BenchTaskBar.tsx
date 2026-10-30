import { useEffect, useState } from 'react';
import { visibleBenchTasks, type BenchTask } from '../lib/bench-task';
import { Button } from './ui';

const formatWaited = (ms: number) => {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
};

/**
 * 壳上的常驻比较任务条（UX ③）：离开比较页以后仍能看到"在跑什么、到哪一步、等了多久"，
 * 并能回到比较页。任务本身仍由比较页提交/等待，这里不发起新的运行。
 */
export default function BenchTaskBar({
  tasks,
  t,
  onReturn,
  onStop,
  onDismiss,
}: {
  tasks: BenchTask[];
  t: (zh: string, en: string) => string;
  onReturn: (task: BenchTask) => void;
  onStop: (task: BenchTask) => void;
  onDismiss: (task: BenchTask) => void;
}) {
  const [now, setNow] = useState(() => Date.now());
  const running = tasks.some((task) => task.state === 'running');
  useEffect(() => {
    // 任务列表没变但时间在走（running）；全部结束后也要刷一次，完成项的"保留 5 分钟"才会按期收起。
    setNow(Date.now());
    if (!running) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running, tasks]);
  const shown = visibleBenchTasks(tasks, now);
  if (!shown.length) return null;
  return (
    <section
      className="bench-task-bar"
      data-bench-task-bar
      aria-label={t('评估任务', 'Evaluation tasks')}
    >
      {shown.map((task) => (
        <div key={task.id} className="bench-task-row" data-bench-task={task.state}>
          <span className="bench-task-label">
            {t('评估', 'Evaluation')} · {task.label}
            {task.host ? ` @ ${task.host}` : ''}
          </span>
          <span className="bench-task-phase" role="status" aria-live="polite">
            {task.state === 'running'
              ? t(task.phase[0], task.phase[1])
              : task.state === 'done'
                ? t('已完成', 'Finished')
                : task.state === 'failed'
                  ? t('失败', 'Failed')
                  : t(
                      '已中断（应用重启或任务被替换）',
                      'Interrupted (app restarted or task replaced)',
                    )}
          </span>
          {task.state === 'running' && (
            <span className="bench-task-waited" aria-hidden="true">
              {formatWaited(now - task.startedAt)}
            </span>
          )}
          {task.state === 'failed' && task.error && (
            <span className="bench-task-error">{task.error}</span>
          )}
          <span className="bench-task-actions">
            {task.state === 'running' && (
              <>
                <Button variant="ghost" onClick={() => onReturn(task)}>
                  {t('回到比较页', 'Back to Compare')}
                </Button>
                <Button variant="ghost" onClick={() => onStop(task)}>
                  {t('停止', 'Stop')}
                </Button>
              </>
            )}
            {task.state !== 'running' && (
              <>
                <Button variant="ghost" onClick={() => onReturn(task)}>
                  {task.state === 'done'
                    ? t('查看结果', 'View result')
                    : t('回到比较页', 'Back to Compare')}
                </Button>
                <Button variant="ghost" onClick={() => onDismiss(task)}>
                  {t('关闭', 'Dismiss')}
                </Button>
              </>
            )}
          </span>
        </div>
      ))}
    </section>
  );
}
