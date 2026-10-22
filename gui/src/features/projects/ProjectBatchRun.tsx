import { useEffect, useState } from 'react';
import { Notice } from '../../components/ui';
import { useApp } from '../../lib/app-state';
import { usePageContract } from '../../lib/guide';
import { invoke } from '../../lib/ipc';
import type { CaseEntry, CaseProfile } from '../../lib/types';
import RunPanel from '../runs/RunPanel';
import { remoteActivity, restoreRunView } from '../runs/run-model';
import { ActionButton } from '../../components/ActionButton';

type Tone = 'running' | 'waiting' | 'succeeded' | 'failed' | 'cancelled' | 'unknown';
// 选站点时还要区分没有运行记录的站点：本地已有结果的算 previous，其余算 idle。
type PickTone = Tone | 'previous' | 'idle';
type PickFilter = 'all' | 'idle' | 'done' | 'problem' | 'active' | 'unknown';

const pickFilters: Record<PickFilter, PickTone[]> = {
  all: [],
  idle: ['idle'],
  done: ['succeeded', 'previous'],
  problem: ['failed', 'cancelled'],
  active: ['running', 'waiting'],
  unknown: ['unknown'],
};

function toneOf(status: string): Tone {
  if (status === 'starting' || status === 'running') return 'running';
  if (['queued', 'preparing', 'submitting', 'submitted'].includes(status)) return 'waiting';
  if (status === 'succeeded' || status === 'failed' || status === 'cancelled') return status;
  return 'unknown';
}

const matches = (name: string, query: string) =>
  !query.trim() || name.toLowerCase().includes(query.trim().toLowerCase());

export default function ProjectBatchRun({
  onBack,
  fromCreation = false,
}: {
  onBack: () => void;
  fromCreation?: boolean;
}) {
  const { state } = useApp();
  // 项目里有格点算例时不叫"站点"（原来格点算例 hb 也被算成"站点"）。
  const anySpatial = state.cases.some((entry) => entry.spatial);
  const en = state.language === 'en',
    t = (zh: string, english: string) =>
      !anySpatial
        ? en
          ? english
          : zh
        : en
          ? english.replace(/\b([Ss])tation(s?)\b/g, (_, s: string, plural: string) =>
              s === 'S' ? `Case${plural}` : `case${plural}`,
            )
          : zh.replaceAll('站点', '算例');
  const handoff =
    state.projectRunSettings?.root === state.root ? state.projectRunSettings : undefined;
  const fixedSelection = fromCreation && !!handoff;
  // 一份编译配置不能同时跑单点和格点：默认只选单点算例（没有单点时选全部格点）。原来默认全选，
  // 混着格点算例时"开始运行"一直是灰的，提示还让人去版本页查（2026-10-22 用户实测）。
  const [selected, setSelected] = useState<string[]>(
    handoff && (fixedSelection || handoff.batch)
      ? handoff.cases
      : (state.cases.some((entry) => !entry.spatial)
          ? state.cases.filter((entry) => !entry.spatial)
          : state.cases
        ).map((entry) => entry.dir),
  );
  const [selecting, setSelecting] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [context, setContext] = useState<{
    key: string;
    profiles?: Record<string, CaseProfile>;
    error?: string;
  } | null>(null);
  const [pickFilter, setPickFilter] = useState<PickFilter>('all');
  const [pickQuery, setPickQuery] = useState('');
  const [monitorFilter, setMonitorFilter] = useState<'all' | Tone>('all');
  const [monitorQuery, setMonitorQuery] = useState('');
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
    // One call reads every case's settings in parallel on the backend and keeps no previews.
    if (first)
      void invoke<Record<string, CaseProfile>>('model_case_profiles', {
        dirs: JSON.parse(contextKey)[0] as string[],
        modelRevision: { id, revision },
      }).then(
        (profiles) => {
          if (!active) return;
          const entries = Object.entries(profiles);
          const firstProfile = profiles[first];
          const mixedKinds = entries.some(
            ([, profile]) => !!profile.spatial !== !!firstProfile.spatial,
          );
          const mixed = entries.some(
            ([, profile]) =>
              (profile.run_host || '') !== (firstProfile.run_host || '') ||
              (profile.engine || 'rust') !== (firstProfile.engine || 'rust'),
          );
          setContext(
            mixedKinds
              ? {
                  key: contextKey,
                  error: en
                    ? 'Single-point and gridded cases need separate batch runs: one build configuration cannot run both.'
                    : '单点算例和格点算例要分开批量运行：一份编译配置不能同时跑两种。',
                }
              : mixed
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
  // A station whose task still runs (here or on a server) keeps its card, status and cancel
  // control: it cannot be deselected until its run ends.
  const pinned = (dir: string) => paths.includes(dir) && !!state.running[dir]?.running;
  function select(dir: string, checked: boolean) {
    if (locked || (!checked && pinned(dir))) return;
    setSelected((previous) =>
      checked ? [...new Set([...previous, dir])] : previous.filter((path) => path !== dir),
    );
  }
  // 批量勾选同样不能丢下仍在运行的站点。
  function choose(dirs: string[], mode: 'add' | 'remove' | 'only') {
    if (locked) return;
    setSelected((previous) => {
      if (mode === 'add') return [...new Set([...previous, ...dirs])];
      if (mode === 'remove') return previous.filter((dir) => pinned(dir) || !dirs.includes(dir));
      return [...new Set([...previous.filter(pinned), ...dirs])];
    });
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
  const toneLabels: Record<Tone, string> = {
    running: t('运行中', 'Running'),
    waiting: t('等待', 'Waiting'),
    succeeded: t('完成', 'Completed'),
    failed: t('失败', 'Failed'),
    cancelled: t('已停止', 'Stopped'),
    unknown: t('待核对', 'Unconfirmed'),
  };
  // 所有站点（不只是已选的）都算一次状态：选站点时要按状态筛选，监控只看已选的。
  const tracked = state.cases.flatMap((entry) => {
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
    return [{ entry, run, view, status, tone: toneOf(status) }];
  });
  const runs = tracked.filter(({ entry }) => paths.includes(entry.dir));
  const ofTone = (tone: Tone) => runs.filter((run) => run.tone === tone);
  const counts = Object.fromEntries(
    (Object.keys(toneLabels) as Tone[]).map((tone) => [tone, ofTone(tone).length]),
  ) as Record<Tone, number>;
  const finished = runs.filter(({ tone }) => ['succeeded', 'failed', 'cancelled'].includes(tone));
  const ended = finished.length;
  const pickTone = (entry: CaseEntry): PickTone =>
    tracked.find((run) => run.entry.dir === entry.dir)?.tone ??
    (entry.has_history ? 'previous' : 'idle');
  const pickLabels: Record<PickFilter, string> = {
    all: t('全部', 'All'),
    idle: t('未运行', 'Not run'),
    done: t('已有结果', 'With results'),
    problem: t('失败或已停止', 'Failed or stopped'),
    active: t('进行中', 'In progress'),
    unknown: t('待核对', 'Unconfirmed'),
  };
  const inFilter = (entry: CaseEntry, filter: PickFilter) =>
    filter === 'all' || pickFilters[filter].includes(pickTone(entry));
  const listed = state.cases.filter(
    (entry) => inFilter(entry, pickFilter) && matches(entry.name, pickQuery),
  );
  const narrowed = pickFilter !== 'all' || !!pickQuery.trim();
  const unfinished = state.cases
    .filter((entry) => !['succeeded', 'previous'].includes(pickTone(entry)))
    .map((entry) => entry.dir);
  const problems = [...ofTone('failed'), ...ofTone('cancelled')].map(({ entry }) => entry.dir);
  // 服务器上的站点再说一句它在做什么：排队的原因、在编译引擎、怎么结束的。阶段已经写明的运行中不重复。
  const remoteNote = (run: (typeof runs)[number]['run']) => {
    if (run.location !== 'remote' || !run.progress?.remote) return '';
    const activity = remoteActivity(run.progress.remote, en);
    return activity.kind === 'running' || activity.kind === 'finished' ? '' : activity.text;
  };
  const renderRun = ({ entry, run, view, status, tone }: (typeof runs)[number]) => (
    <article key={entry.dir} aria-label={entry.name} className={`batch-run is-${tone}`}>
      <div className="batch-run-head">
        <span className="batch-dot" aria-hidden="true" />
        <h4>{entry.name}</h4>
        <span className="batch-run-status">
          {statusLabels[status] ?? t('等待核对', 'Needs verification')}
          {view.stage ? ` · ${stageLabels[view.stage] ?? view.stage}` : ''}
        </span>
        {!!view.total && (
          <span className="batch-run-count">
            {view.step ?? 0} / {view.total}
            <strong>{`${Math.floor(((view.step ?? 0) / view.total) * 100)}%`}</strong>
          </span>
        )}
      </div>
      {remoteNote(run) && <p className="batch-run-note">{remoteNote(run)}</p>}
      {!!view.total && <progress aria-label={entry.name} value={view.step ?? 0} max={view.total} />}
      {run.error && <Notice tone="error">{run.error}</Notice>}
      {!!view.lines?.length && (
        <details className="batch-run-log">
          <summary>{t('查看运行日志', 'View run logs')}</summary>
          <pre className="run-log">{view.lines.join('\n')}</pre>
        </details>
      )}
    </article>
  );
  const shown = (list: typeof runs) =>
    list.filter(({ entry }) => matches(entry.name, monitorQuery));
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
            disabled: !!locked || pinned(entry.dir),
            set: (value) => select(entry.dir, value === 'true'),
          }))
        : [],
    // 按钮自己登记（第 737 轮）。原来清单里的"全选站点"直接选中全部算例，界面上的按钮选的却是
    // 当前筛选列出的站点；"全不选""只选没有结果的"和筛选标签则只有界面上有。
    actions: [],
  });
  return (
    <div className="page-stack batch-page">
      <div className="page-heading">
        <div>
          <h1>{t('批量运行', 'Batch run')}</h1>
          <div className="batch-meta">
            <span className="path">{state.root}</span>
            <span className="badge">
              {t('模型版本 ', 'Model ')}
              {state.version?.name ?? id} · r{revision}
            </span>
          </div>
        </div>
        {!current?.profiles && (
          <ActionButton
            action="project-batch-back"
            label={t('返回算例列表', 'Back to cases')}
            disabled={!!locked}
            run={onBack}
          />
        )}
      </div>
      <section className="card batch-picker" aria-label={t('运行站点', 'Stations')}>
        <div className="batch-picker-head">
          <div>
            <h2>
              {cases.length
                ? t(`运行站点：${cases.length} 个`, `Stations to run: ${cases.length}`)
                : t('请选择要运行的算例。', 'Select cases to run.')}
            </h2>
            <p className="muted">
              {t(
                `项目共 ${state.cases.length} 个站点；每个站点使用各自已保存的设置，统一换用当前模型版本。`,
                `${state.cases.length} stations in the project; each uses its saved settings with the current model revision.`,
              )}
            </p>
          </div>
          {!fixedSelection && (
            <ActionButton
              action="project-batch-selection"
              label={
                selecting
                  ? t('收起站点列表', 'Hide station list')
                  : t('调整运行站点（可选）', 'Adjust stations (optional)')
              }
              disabled={!!locked}
              run={() => setSelecting((value) => !value)}
            />
          )}
        </div>
        {!fixedSelection && selecting && (
          <div className="batch-picker-body">
            <div className="batch-toolbar">
              <input
                type="search"
                className="input batch-search"
                placeholder={t('搜索站点名', 'Search stations')}
                aria-label={t('搜索站点名', 'Search stations')}
                value={pickQuery}
                onChange={(event) => setPickQuery(event.target.value)}
              />
              <div
                className="batch-chips"
                role="group"
                aria-label={t('按状态筛选', 'Filter by status')}
              >
                {(Object.keys(pickFilters) as PickFilter[]).flatMap((filter) => {
                  const count = state.cases.filter((entry) => inFilter(entry, filter)).length;
                  return filter === 'all' || count
                    ? [
                        <ActionButton
                          bare
                          key={filter}
                          action={`project-batch-filter-${filter}`}
                          label={t(
                            `只列出：${pickLabels[filter]}`,
                            `List only: ${pickLabels[filter]}`,
                          )}
                          className="batch-chip"
                          aria-pressed={pickFilter === filter}
                          run={() => setPickFilter(filter)}
                        >
                          {pickLabels[filter]}
                          <span className="batch-chip-count">{count}</span>
                        </ActionButton>,
                      ]
                    : [];
                })}
              </div>
            </div>
            <div className="batch-quick">
              <ActionButton
                action="project-batch-select-all"
                label={
                  narrowed
                    ? t('全选列出的站点', 'Select listed')
                    : t('全选站点', 'Select all stations')
                }
                variant="ghost"
                disabled={!!locked}
                run={() =>
                  choose(
                    listed.map((entry) => entry.dir),
                    'add',
                  )
                }
              />
              <ActionButton
                action="project-batch-select-none"
                label={
                  narrowed ? t('取消列出的站点', 'Deselect listed') : t('全不选', 'Select none')
                }
                variant="ghost"
                disabled={!!locked}
                run={() =>
                  choose(
                    listed.map((entry) => entry.dir),
                    'remove',
                  )
                }
              />
              <ActionButton
                action="project-batch-select-unfinished"
                label={t(
                  `只选还没有结果的站点（${unfinished.length}）`,
                  `Only stations without results (${unfinished.length})`,
                )}
                variant="ghost"
                disabled={!!locked || !unfinished.length}
                run={() => choose(unfinished, 'only')}
              />
              <span className="batch-quick-count">
                {t(
                  `已选 ${cases.length} / ${state.cases.length}`,
                  `${cases.length} of ${state.cases.length} selected`,
                )}
              </span>
            </div>
            <div className="batch-station-grid">
              {listed.map((entry) => {
                const tone = pickTone(entry);
                const checked = paths.includes(entry.dir);
                return (
                  <label
                    key={entry.dir}
                    className={`batch-station is-${tone}${checked ? ' is-selected' : ''}`}
                    title={
                      pinned(entry.dir)
                        ? t('运行结束前不能取消选择', 'Cannot be deselected until its run ends')
                        : tone === 'idle' || tone === 'previous'
                          ? pickLabels[tone === 'idle' ? 'idle' : 'done']
                          : toneLabels[tone]
                    }
                  >
                    <input
                      type="checkbox"
                      checked={checked}
                      disabled={!!locked || pinned(entry.dir)}
                      onChange={(event) => select(entry.dir, event.target.checked)}
                    />
                    <span className="batch-dot" aria-hidden="true" />
                    <span className="batch-station-name">{entry.name}</span>
                    {entry.spatial && anySpatial && state.cases.some((c) => !c.spatial) && (
                      <span className="muted">{t('格点', 'gridded')}</span>
                    )}
                  </label>
                );
              })}
              {!listed.length && (
                <p className="muted">{t('没有符合条件的站点。', 'No stations match.')}</p>
              )}
            </div>
          </div>
        )}
      </section>
      {first && !current && (
        <Notice>
          {t(
            `正在读取 ${paths.length} 个算例的设置…`,
            `Loading the settings of ${paths.length} cases…`,
          )}
        </Notice>
      )}
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
          className="card simulation-monitor batch-monitor"
          aria-label={t('所有站点运行进度', 'All station progress')}
        >
          <div className="batch-monitor-head">
            <h3>{t('所有站点运行进度', 'All station progress')}</h3>
            <span className="batch-monitor-total">
              {t(`已结束 ${ended} / ${runs.length}`, `${ended} of ${runs.length} ended`)}
              <strong>{`${Math.floor((ended / runs.length) * 100)}%`}</strong>
            </span>
          </div>
          <div className="batch-overall" aria-hidden="true">
            {(['succeeded', 'failed', 'cancelled', 'running', 'waiting', 'unknown'] as Tone[])
              .filter((tone) => counts[tone])
              .map((tone) => (
                <span key={tone} className={`is-${tone}`} style={{ flexGrow: counts[tone] }} />
              ))}
          </div>
          <p role="status" aria-live="polite" className="visually-hidden">
            {t(
              `运行中 ${counts.running} · 等待 ${counts.waiting} · 完成 ${counts.succeeded} · 失败 ${counts.failed} · 已停止 ${counts.cancelled} · 待核对 ${counts.unknown}`,
              `Running ${counts.running} · Waiting ${counts.waiting} · Completed ${counts.succeeded} · Failed ${counts.failed} · Stopped ${counts.cancelled} · Unconfirmed ${counts.unknown}`,
            )}
          </p>
          <div className="batch-tiles" role="group" aria-label={t('按状态查看', 'Show by status')}>
            {(['running', 'waiting', 'succeeded', 'failed', 'cancelled', 'unknown'] as Tone[]).map(
              (tone) => (
                <ActionButton
                  bare
                  key={tone}
                  action={`project-batch-monitor-${tone}`}
                  label={t(`只看：${toneLabels[tone]}`, `Show only: ${toneLabels[tone]}`)}
                  className={`batch-tile is-${tone}`}
                  aria-pressed={monitorFilter === tone}
                  disabled={!counts[tone] && monitorFilter !== tone}
                  run={() => setMonitorFilter((value) => (value === tone ? 'all' : tone))}
                >
                  <span className="batch-tile-count">{counts[tone]}</span>
                  <span className="batch-tile-label">{toneLabels[tone]}</span>
                </ActionButton>
              ),
            )}
          </div>
          <div className="batch-toolbar">
            <input
              type="search"
              className="input batch-search"
              placeholder={t('查找站点', 'Find a station')}
              aria-label={t('查找站点', 'Find a station')}
              value={monitorQuery}
              onChange={(event) => setMonitorQuery(event.target.value)}
            />
            {monitorFilter !== 'all' && (
              <ActionButton
                action="project-batch-monitor-all"
                label={t('显示全部站点', 'Show all stations')}
                variant="ghost"
                run={() => setMonitorFilter('all')}
              />
            )}
            {!fixedSelection && !!problems.length && (
              <ActionButton
                action="project-batch-select-problems"
                label={t(
                  `只选失败或已停止的 ${problems.length} 个站点重跑`,
                  `Select the ${problems.length} failed or stopped stations to rerun`,
                )}
                variant="ghost"
                disabled={!!locked}
                run={() => choose(problems, 'only')}
              />
            )}
          </div>
          {monitorFilter === 'all' ? (
            <>
              {shown(ofTone('running')).map(renderRun)}
              {shown(ofTone('unknown')).map(renderRun)}
              {!!shown(ofTone('waiting')).length && (
                <details className="batch-group" open={!!monitorQuery.trim() || undefined}>
                  <summary>
                    {t(
                      `等待中的站点（${shown(ofTone('waiting')).length}）`,
                      `Waiting stations (${shown(ofTone('waiting')).length})`,
                    )}
                  </summary>
                  {shown(ofTone('waiting')).map(renderRun)}
                </details>
              )}
              {!!shown(finished).length && (
                <details className="batch-group" open={!!monitorQuery.trim() || undefined}>
                  <summary>
                    {t(
                      `已结束的站点（${shown(finished).length}）`,
                      `Finished stations (${shown(finished).length})`,
                    )}
                  </summary>
                  {shown(finished).map(renderRun)}
                </details>
              )}
            </>
          ) : (
            shown(ofTone(monitorFilter)).map(renderRun)
          )}
          {!shown(monitorFilter === 'all' ? runs : ofTone(monitorFilter)).length && (
            <p className="muted">{t('没有符合条件的站点。', 'No stations match.')}</p>
          )}
        </section>
      )}
    </div>
  );
}
