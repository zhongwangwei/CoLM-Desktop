import { useEffect, useState } from 'react';
import { Advanced, Button, Notice } from '../../components/ui';
import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { invoke } from '../../lib/ipc';
import type { CaseProfile } from '../../lib/types';
import RunPanel from '../runs/RunPanel';
import { restoreRunView } from '../runs/run-model';

export default function ProjectBatchRun({
  onBack,
  fromCreation = false,
}: {
  onBack: () => void;
  fromCreation?: boolean;
}) {
  const { state } = useApp();
  const en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const handoff =
    state.projectRunSettings?.root === state.root ? state.projectRunSettings : undefined;
  const fixedSelection = fromCreation && !!handoff;
  const [selected, setSelected] = useState<string[]>(
    handoff && (fixedSelection || handoff.batch)
      ? handoff.cases
      : state.cases.map((entry) => entry.dir),
  );
  const [selecting, setSelecting] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [context, setContext] = useState<{
    key: string;
    profiles?: Record<string, CaseProfile>;
    error?: string;
  } | null>(null);
  const cases = state.cases.filter((entry) => selected.includes(entry.dir));
  const paths = cases.map((entry) => entry.dir);
  const first = paths[0];
  const { id, revision } = state.versionRef;
  const contextKey = JSON.stringify([paths, id, revision]);
  const locked =
    submitting ||
    state.busy ||
    paths.some((dir) => state.running[dir]?.running && state.running[dir]?.location !== 'remote');
  const current = context?.key === contextKey ? context : null;
  useEffect(() => {
    let active = true;
    if (first)
      void Promise.all(
        (JSON.parse(contextKey)[0] as string[]).map(async (dir) => {
          const { profile } = await invoke<{ profile: CaseProfile }>('model_case_context', {
            dir,
            modelRevision: { id, revision },
          });
          return [dir, profile] as const;
        }),
      ).then(
        (entries) => {
          if (!active) return;
          const profiles = Object.fromEntries(entries);
          const firstProfile = profiles[first];
          const mixed = entries.some(
            ([, profile]) =>
              (profile.run_host || '') !== (firstProfile.run_host || '') ||
              (profile.engine || 'rust') !== (firstProfile.engine || 'rust'),
          );
          setContext(
            mixed
              ? {
                  key: contextKey,
                  error: en
                    ? 'Select cases with the same run location and engine.'
                    : '请选择运行位置和引擎相同的算例。',
                }
              : { key: contextKey, profiles },
          );
        },
        (error: unknown) => {
          if (active) setContext({ key: contextKey, error: String(error) });
        },
      );
    return () => {
      active = false;
    };
  }, [first, id, revision, contextKey, en]);
  function select(dir: string, checked: boolean) {
    if (locked) return;
    setSelected((previous) =>
      checked ? [...new Set([...previous, dir])] : previous.filter((path) => path !== dir),
    );
  }
  const statusLabels: Record<string, string> = {
    preparing: t('准备中', 'Preparing'),
    queued: t('等待运行', 'Queued'),
    starting: t('正在启动', 'Starting'),
    running: t('运行中', 'Running'),
    succeeded: t('运行完成', 'Completed'),
    failed: t('运行失败', 'Failed'),
    cancelled: t('已停止', 'Stopped'),
    unknown: t('等待核对', 'Needs verification'),
    submitted: t('已提交，等待服务器状态', 'Submitted; awaiting server status'),
    submitting: t('正在提交', 'Submitting'),
  };
  const stageLabels: Record<string, string> = {
    'preparing the engine': t('准备运行环境', 'Preparing the environment'),
    'installing Rust/Cargo': t('安装 Rust/Cargo', 'Installing Rust/Cargo'),
    'installing CMake': t('安装 CMake', 'Installing CMake'),
    'building the Rust engine': t('编译模型引擎', 'Building the model engine'),
    mksrfdata: t('准备地表数据', 'Surface data'),
    mkinidata: t('准备初始场', 'Initial conditions'),
    colm: t('模型计算', 'Simulation'),
  };
  const runs = cases.flatMap((entry) => {
    const run = state.running[entry.dir];
    const view = restoreRunView(run, entry.dir);
    if (!view) return [];
    const remote = run.progress?.remote as { status?: { state?: string } } | undefined;
    const status =
      run.status === 'submitted'
        ? remote?.status?.state === 'running'
          ? 'running'
          : remote?.status?.state === 'queued'
            ? 'queued'
            : 'submitted'
        : view.status;
    return [{ entry, run, view, status }];
  });
  const activeRuns = runs.filter(({ status }) => ['starting', 'running'].includes(status));
  const waitingRuns = runs.filter(({ status }) =>
    ['queued', 'preparing', 'submitting', 'submitted'].includes(status),
  );
  const terminalRuns = runs.filter(({ status }) =>
    ['succeeded', 'failed', 'cancelled'].includes(status),
  );
  const uncertainRuns = runs.filter(
    (run) => !activeRuns.includes(run) && !waitingRuns.includes(run) && !terminalRuns.includes(run),
  );
  const renderRun = ({ entry, run, view, status }: (typeof runs)[number]) => (
    <article key={entry.dir} aria-label={entry.name}>
      <h4>{entry.name}</h4>
      <p>
        {statusLabels[status] ?? t('等待核对', 'Needs verification')}
        {view.stage ? ` · ${stageLabels[view.stage] ?? view.stage}` : ''}
      </p>
      {!!view.total && (
        <>
          <progress aria-label={entry.name} value={view.step ?? 0} max={view.total} />
          <p>
            {view.step ?? 0} / {view.total}
          </p>
        </>
      )}
      {run.error && <Notice tone="error">{run.error}</Notice>}
      {!!view.lines?.length && (
        <details>
          <summary>{t('查看运行日志', 'View run logs')}</summary>
          <pre className="run-log">{view.lines.join('\n')}</pre>
        </details>
      )}
    </article>
  );
  usePageContract({
    title: t('批量运行', 'Batch run'),
    step: 'project-batch-run',
    fields:
      !fixedSelection && selecting
        ? state.cases.map((entry) => ({
            id: `project-batch-case:${entry.dir}`,
            label: entry.name,
            type: 'switch' as const,
            value: String(paths.includes(entry.dir)),
            disabled: !!locked,
            set: (value) => select(entry.dir, value === 'true'),
          }))
        : [],
    actions: [
      ...(!fixedSelection
        ? [
            {
              id: 'project-batch-selection',
              label: t('调整运行站点（可选）', 'Adjust stations (optional)'),
              kind: 'click' as const,
              enabled: !locked,
              run: () => {
                if (!locked) setSelecting((value) => !value);
              },
            },
            ...(selecting
              ? [
                  {
                    id: 'project-batch-select-all',
                    label: t('全选站点', 'Select all stations'),
                    kind: 'click' as const,
                    enabled: !locked,
                    run: () => {
                      if (!locked) setSelected(state.cases.map((entry) => entry.dir));
                    },
                  },
                ]
              : []),
          ]
        : []),
      ...(!current?.profiles
        ? [
            {
              id: 'project-batch-back',
              label: t('返回算例列表', 'Back to cases'),
              kind: 'click' as const,
              enabled: !locked,
              run: () => {
                if (!locked) onBack();
              },
            },
          ]
        : []),
    ],
  });
  return (
    <div className="page-stack">
      <div className="page-heading">
        <h1>{t('批量运行', 'Batch run')}</h1>
        {!current?.profiles && (
          <Button disabled={!!locked} onClick={onBack}>
            {t('返回算例列表', 'Back to cases')}
          </Button>
        )}
      </div>
      <p className="path">{state.root}</p>
      <Notice>
        {t(
          '所选算例使用各自已保存的设置，并统一应用当前模型版本：',
          'Each selected case uses its saved settings with the current model revision: ',
        )}
        {state.version?.name ?? id} · r{revision}
      </Notice>
      {!fixedSelection && (
        <Advanced
          open={selecting}
          onOpenChange={setSelecting}
          title={t('调整运行站点（可选）', 'Adjust stations (optional)')}
        >
          {selecting && (
            <>
              <div className="actions">
                <Button
                  disabled={!!locked}
                  onClick={() => setSelected(state.cases.map((entry) => entry.dir))}
                >
                  {t('全选站点', 'Select all stations')}
                </Button>
              </div>
              <div className="check-grid">
                {state.cases.map((entry) => (
                  <label key={entry.dir}>
                    <input
                      type="checkbox"
                      checked={paths.includes(entry.dir)}
                      disabled={!!locked}
                      onChange={(event) => select(entry.dir, event.target.checked)}
                    />
                    {entry.name}
                  </label>
                ))}
              </div>
            </>
          )}
        </Advanced>
      )}
      <p>
        {cases.length
          ? t(`运行站点：${cases.length} 个`, `Stations to run: ${cases.length}`)
          : t('请选择要运行的算例。', 'Select cases to run.')}
      </p>
      {first && !current && <Notice>{t('正在读取算例设置…', 'Loading case settings…')}</Notice>}
      {current?.error && <Notice tone="error">{current.error}</Notice>}
      {first && current?.profiles && (
        <RunPanel
          key={contextKey}
          casePath={first}
          batchCases={paths}
          summary={{
            name:
              cases.length > 3
                ? t(`${cases.length} 个站点`, `${cases.length} stations`)
                : cases.map((entry) => entry.name).join('、'),
            period: t('各算例已保存的时段', 'Each case’s saved period'),
          }}
          effectiveProfile={{
            ...current.profiles[first],
            grid: current.profiles[first].grid ?? undefined,
          }}
          batchProfiles={current.profiles}
          initialHost={current.profiles[first].run_host || ''}
          initialSettings={handoff}
          onBusyChange={setSubmitting}
          onBack={() => {
            if (!locked) onBack();
          }}
        />
      )}
      {!!runs.length && (
        <section
          className="simulation-monitor"
          aria-label={t('所有站点运行进度', 'All station progress')}
        >
          <h3>{t('所有站点运行进度', 'All station progress')}</h3>
          <p role="status" aria-live="polite">
            {t(
              `运行中 ${activeRuns.length} · 等待 ${waitingRuns.length} · 完成 ${terminalRuns.filter(({ status }) => status === 'succeeded').length} · 失败 ${terminalRuns.filter(({ status }) => status === 'failed').length} · 已停止 ${terminalRuns.filter(({ status }) => status === 'cancelled').length} · 待核对 ${uncertainRuns.length}`,
              `Running ${activeRuns.length} · Waiting ${waitingRuns.length} · Completed ${terminalRuns.filter(({ status }) => status === 'succeeded').length} · Failed ${terminalRuns.filter(({ status }) => status === 'failed').length} · Stopped ${terminalRuns.filter(({ status }) => status === 'cancelled').length} · Unconfirmed ${uncertainRuns.length}`,
            )}
          </p>
          {activeRuns.map(renderRun)}
          {uncertainRuns.map(renderRun)}
          {!!waitingRuns.length && (
            <details>
              <summary>
                {t(
                  `等待中的站点（${waitingRuns.length}）`,
                  `Waiting stations (${waitingRuns.length})`,
                )}
              </summary>
              {waitingRuns.map(renderRun)}
            </details>
          )}
          {!!terminalRuns.length && (
            <details>
              <summary>
                {t(
                  `已结束的站点（${terminalRuns.length}）`,
                  `Finished stations (${terminalRuns.length})`,
                )}
              </summary>
              {terminalRuns.map(renderRun)}
            </details>
          )}
        </section>
      )}
    </div>
  );
}
