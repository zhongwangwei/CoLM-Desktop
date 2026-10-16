import { useEffect, useState } from 'react';
import { Button, Notice } from '../../components/ui';
import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { invoke } from '../../lib/ipc';
import type { CaseProfile } from '../../lib/types';
import RunPanel from '../runs/RunPanel';

export default function ProjectBatchRun({ onBack }: { onBack: () => void }) {
  const { state } = useApp();
  const en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const handoff =
    state.projectRunSettings?.root === state.root ? state.projectRunSettings : undefined;
  const [selected, setSelected] = useState<string[]>(handoff?.cases ?? []);
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
  usePageContract({
    title: t('批量运行', 'Batch run'),
    step: 'project-batch-run',
    fields: state.cases.map((entry) => ({
      id: `project-batch-case:${entry.dir}`,
      label: entry.name,
      type: 'switch' as const,
      value: String(paths.includes(entry.dir)),
      disabled: !!locked,
      set: (value) => select(entry.dir, value === 'true'),
    })),
    actions: [
      {
        id: 'project-batch-select-all',
        label: t('全选站点', 'Select all stations'),
        kind: 'click',
        enabled: !locked,
        run: () => {
          if (!locked) setSelected(state.cases.map((entry) => entry.dir));
        },
      },
      {
        id: 'project-batch-back',
        label: t('返回算例列表', 'Back to cases'),
        kind: 'click',
        enabled: !locked,
        run: () => {
          if (!locked) onBack();
        },
      },
    ],
  });
  return (
    <div className="page-stack">
      <div className="page-heading">
        <h1>{t('批量运行', 'Batch run')}</h1>
        <Button disabled={!!locked} onClick={onBack}>
          {t('返回算例列表', 'Back to cases')}
        </Button>
      </div>
      <p className="path">{state.root}</p>
      <Notice>
        {t(
          '所选算例使用各自已保存的设置，并统一应用当前模型版本：',
          'Each selected case uses its saved settings with the current model revision: ',
        )}
        {state.version?.name ?? id} · r{revision}
      </Notice>
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
      <p>
        {cases.length
          ? t(`已选择 ${cases.length} 个站点`, `${cases.length} stations selected`)
          : t('请选择要运行的算例。', 'Select cases to run.')}
      </p>
      {cases
        .filter((entry) => state.running[entry.dir])
        .map((entry) => {
          const run = state.running[entry.dir];
          const status = !run
            ? t('尚未运行', 'Not started')
            : run.running
              ? t('运行中', 'Running')
              : run.status === 'succeeded'
                ? t('运行完成', 'Completed')
                : run.status === 'failed'
                  ? t('运行失败', 'Failed')
                  : run.status === 'cancelled'
                    ? t('已停止', 'Stopped')
                    : t('等待核对', 'Needs verification');
          return (
            <div key={entry.dir}>
              <p>
                {entry.name} · {status}
              </p>
              {!!run?.logs?.length && (
                <details>
                  <summary>{t('查看运行日志', 'View run logs')}</summary>
                  <pre className="log">{run.logs.join('\n')}</pre>
                </details>
              )}
              {run?.error && (
                <Notice tone="error">
                  {entry.name}: {run.error}
                </Notice>
              )}
            </div>
          );
        })}
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
        />
      )}
    </div>
  );
}
