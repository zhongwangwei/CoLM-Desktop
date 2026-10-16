import { choosableVersions, latestOfficial } from '../versions/version-tree';
import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { hasBackend, invoke, invokeJson } from '../../lib/ipc';
import { usePageContract, type UiAction } from '../../lib/guide';
import { Button, Dialog, Notice } from '../../components/ui';
import './project-version.css';
import ProjectBatchRun from './ProjectBatchRun';
import NavIcon from '../../components/NavIcon';
import type { Site } from '../cases/case-model';
import type { CaseEntry, FrozenRevision } from '../../lib/types';
import {
  RECENT_PROJECTS_KEY,
  recentProjectRoots,
  rememberProject,
  projectName,
} from './project-model';

export default function ProjectPage() {
  const { state, update, navigate, openCase } = useApp();
  const en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const [roots, setRoots] = useState<string[]>([]),
    [ready, setReady] = useState(!hasBackend()),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [chooser, setChooser] = useState<'simulation' | 'results' | null>(
      state.projectCasesOpen ? 'simulation' : null,
    ),
    [versionChooser, setVersionChooser] = useState(false),
    [batchPage, setBatchPage] = useState(
      !!state.projectCasesOpen && state.projectRunSettings?.root === state.root,
    );
  const batchFromCreation = useRef(
    !!state.projectCasesOpen && state.projectRunSettings?.root === state.root,
  );
  const lock = useRef(false),
    active = useRef(true);
  useEffect(() => {
    if (state.projectCasesOpen) update({ projectCasesOpen: false });
  }, [state.projectCasesOpen, update]);
  useEffect(() => {
    let current = true;
    active.current = true;
    if (hasBackend())
      void invoke<Record<string, string>>('load_recent')
        .then((recent) => {
          if (current) {
            setRoots(recentProjectRoots(recent[RECENT_PROJECTS_KEY]));
            setReady(true);
          }
        })
        .catch((e) => {
          if (current) {
            setError(String(e));
            setReady(true);
          }
        });
    return () => {
      current = false;
      active.current = false;
    };
  }, []);
  async function work<T>(fn: () => Promise<T>) {
    if (lock.current) throw new Error(t('请等待当前操作完成。', 'Wait for the current operation.'));
    lock.current = true;
    setBusy(true);
    setError('');
    try {
      return await fn();
    } catch (e) {
      if (active.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      if (active.current) setBusy(false);
    }
  }
  const shownRoots = state.root ? rememberProject(roots, state.root) : roots;
  const projectLocation = (() => {
    const first = state.cases[0];
    const parent = (path: string) => {
      const normalized = path.replace(/\/+$/, '');
      return normalized.slice(0, normalized.lastIndexOf('/')) || '/';
    };
    if (first?.run_host && first.remote_directory?.startsWith('/')) {
      const directory = parent(first.remote_directory);
      if (
        state.cases.every(
          (entry) =>
            entry.run_host === first.run_host &&
            entry.remote_directory?.startsWith('/') &&
            parent(entry.remote_directory) === directory,
        )
      )
        return `${first.run_host}: ${directory}`;
    }
    return state.root;
  })();
  const official = latestOfficial(state.versions);
  const versionRows = choosableVersions(state.versions);
  const modelReady =
    !!state.version &&
    state.version.version.id === state.versionRef.id &&
    state.version.version.revision === state.versionRef.revision;
  // The official model is selected by default; choosing a version is optional, not a gate.
  const modelChosen = modelReady;
  const officialSelected =
    modelChosen &&
    state.versionRef.id === 'official' &&
    state.versionRef.revision === official?.version.revision;
  const existingSelected = modelChosen && !officialSelected;
  const versionKey = (version: FrozenRevision) => JSON.stringify(version.version);
  function openBatch() {
    batchFromCreation.current = false;
    setBatchPage(true);
  }
  async function chooseVersion(key: string) {
    return work(async () => {
      const selected = state.versions.find((version) => versionKey(version) === key);
      if (!selected)
        throw new Error(
          t(
            '该模型版本不可用，请重新读取版本列表。',
            'This model revision is unavailable. Reload the model list.',
          ),
        );
      const version = await invokeJson<FrozenRevision>('model_get', selected.version);
      if (!active.current)
        throw new Error(
          t('选择期间页面已变化，请重新选择。', 'The page changed while selecting the model.'),
        );
      update({ version, versionRef: version.version, chosenVersion: version.version });
      setVersionChooser(false);
      return { version: version.version, name: version.name };
    });
  }
  async function loadProject(root: string) {
    const cases = await invoke<CaseEntry[]>('list_cases', { root });
    const next = rememberProject(shownRoots, root);
    await invoke('save_recent', { key: 'root', value: root });
    await invoke('save_recent', { key: RECENT_PROJECTS_KEY, value: JSON.stringify(next) });
    if (!active.current) return { root, cases };
    setRoots(next);
    update({ root, cases, ...(root !== state.root ? { selectedCase: null } : {}) });
    setChooser('simulation');
    return { root, cases };
  }
  const browseProject = () =>
    work(async () => {
      const root = await invoke<string | null>('pick_folder', { key: 'project-open' });
      return root ? await loadProject(root) : { cancelled: true };
    });
  async function chooseCase(dir: string, tab: 'settings' | 'results') {
    const opened = await work(() => openCase(dir, tab));
    setChooser(null);
    return opened;
  }
  const otherResults = () =>
    work(async () => {
      const dir = await invoke<string | null>('pick_folder', { key: 'project-results-case' });
      if (!dir) return { cancelled: true };
      const opened = await openCase(dir, 'results');
      setChooser(null);
      return opened;
    });
  function viewResults() {
    if (state.selectedCase) return navigate('case', 'results');
    setChooser('results');
  }
  const newSimulation = () => {
    update({ newCaseSeed: null });
    return navigate('new-case');
  };
  const example = () =>
    work(async () => {
      const paths = await invoke<{
        sitedir: string;
        forcingdir: string;
        root: string;
        runtimedir: string;
      }>('install_example');
      const found = await invoke<{ sites: Site[]; hint?: string }>('scan_sites', {
        dir: paths.sitedir,
        forcingDir: paths.forcingdir,
        quick: false,
      });
      if (!active.current) throw new Error('The project page changed while preparing the example.');
      const site = found.sites.find((item) => item.name === 'CN-Cng');
      if (!site || site.problem || !site.met_file || !site.start || !site.end)
        throw new Error(
          site?.problem ||
            t('CN-Cng 示例输入不完整。', 'The CN-Cng example inputs are incomplete.'),
        );
      update({
        newCaseSeed: {
          form: {
            root: state.root || paths.root,
            name: site.name,
            sitedir: paths.sitedir,
            forcingdir: paths.forcingdir,
            runtime: paths.runtimedir,
            site: site.site_file,
            met: site.met_file,
            start: site.start,
            end: site.end,
          },
          sites: found.sites,
        },
      });
      lock.current = false;
      if (!navigate('new-case'))
        throw new Error(t('当前页面无法切换。', 'The page could not be changed.'));
      return { site: site.name, inputs: paths, start: site.start, end: site.end };
    });
  const versionEnabled = !busy && !state.busy;
  const enabled = versionEnabled && modelChosen;
  function changeVersionChooser(open: boolean) {
    if (!lock.current && !state.busy) {
      setError('');
      setVersionChooser(open);
    }
  }
  const actions: UiAction[] = [
    {
      id: 'project-new-simulation',
      label: t('新建模拟', 'New simulation'),
      kind: 'click',
      enabled,
      run: newSimulation,
    },
    {
      id: 'project-open',
      label: t('打开项目', 'Open project'),
      kind: 'click',
      enabled: enabled && ready,
      run: browseProject,
    },
    {
      id: 'project-view-results',
      label: t('直接查看结果', 'View results'),
      kind: 'click',
      enabled,
      run: viewResults,
    },
    {
      id: 'project-example',
      label: t('打开 CN-Cng 示例', 'Open CN-Cng example'),
      kind: 'commit',
      enabled: enabled && !!state.version,
      run: example,
    },
    {
      id: 'project-model',
      label: t('新建修改版', 'New modified model'),
      kind: 'click',
      enabled: versionEnabled && modelReady,
      run: () => navigate('versions'),
    },
    {
      id: 'project-official',
      label: t('选择官方版', 'Select official model'),
      kind: 'click',
      enabled: versionEnabled && !!official,
      run: () => chooseVersion(versionKey(official!)),
    },
    {
      id: 'project-existing',
      label: t('已有版本', 'Existing versions'),
      kind: 'click',
      enabled: versionEnabled,
      run: () => changeVersionChooser(true),
    },
    {
      id: 'project-prepare',
      label: t('准备输入数据', 'Prepare input data'),
      kind: 'click',
      enabled,
      run: () => navigate('preparation'),
    },
    ...shownRoots.map((root) => ({
      id: `project-recent:${root}`,
      label: projectName(root),
      kind: 'click' as const,
      enabled: enabled && ready,
      run: () => work(() => loadProject(root)),
    })),
  ];
  const chooserActions: UiAction[] = [
    ...state.cases.map((entry) => ({
      id: `project-case:${entry.dir}`,
      label: entry.name,
      kind: 'click' as const,
      enabled,
      run: () => chooseCase(entry.dir, chooser === 'results' ? 'results' : 'settings'),
    })),
    {
      id: 'project-choose-other',
      label: t('选择其他目录', 'Browse another folder'),
      kind: 'click',
      enabled,
      run: chooser === 'results' ? otherResults : browseProject,
    },
    {
      id: 'project-back',
      label: t('返回项目首页', 'Back to project home'),
      kind: 'click',
      enabled,
      run: () => setChooser(null),
    },
    ...(chooser === 'simulation'
      ? [
          {
            id: 'project-batch-run',
            label: t('批量运行', 'Batch run'),
            kind: 'click' as const,
            enabled: enabled && state.cases.length > 0,
            run: openBatch,
          },
          {
            id: 'project-new-simulation',
            label: t('新建模拟', 'New simulation'),
            kind: 'click' as const,
            enabled,
            run: newSimulation,
          },
        ]
      : []),
  ];
  const versionActions: UiAction[] = [
    ...versionRows.map(({ version }) => ({
      id: `project-version:${versionKey(version)}`,
      label: `${version.name} · r${version.version.revision}`,
      kind: 'click' as const,
      enabled: versionEnabled,
      run: () => chooseVersion(versionKey(version)),
    })),
    {
      id: 'project-version-close',
      label: t('关闭', 'Close'),
      kind: 'click',
      enabled: versionEnabled,
      run: () => changeVersionChooser(false),
    },
  ];
  usePageContract(
    {
      title: versionChooser
        ? t('选择已有版本', 'Choose an existing version')
        : chooser === 'results'
          ? t('选择要分析的算例', 'Choose a case to analyze')
          : chooser
            ? t('项目中的算例', 'Cases in this project')
            : t('项目', 'Project'),
      step: 'project',
      modal: versionChooser,
      status: busy
        ? 'working'
        : error || (modelChosen ? 'ready' : t('正在读取模型版本', 'Loading the model version')),
      fields: [],
      actions: versionChooser ? versionActions : chooser ? chooserActions : actions,
    },
    !batchPage,
  );
  if (batchPage)
    return (
      <ProjectBatchRun
        fromCreation={batchFromCreation.current}
        onBack={() => {
          batchFromCreation.current = false;
          setBatchPage(false);
        }}
      />
    );
  if (chooser)
    return (
      <div className="page-stack">
        <div className="page-heading">
          <h1>
            {chooser === 'results'
              ? t('选择要分析的算例', 'Choose a case to analyze')
              : t('项目中的算例', 'Cases in this project')}
          </h1>
          <Button disabled={!enabled} onClick={() => setChooser(null)}>
            {t('返回项目首页', 'Back to project home')}
          </Button>
        </div>
        <p className="path">{projectLocation}</p>
        {error && <Notice tone="error">{error}</Notice>}
        {busy && <Notice>{t('正在打开，请稍候…', 'Opening, please wait…')}</Notice>}
        {!state.cases.length && (
          <p className="muted">
            {chooser === 'results'
              ? t('可以选择一个已有算例目录。', 'Select an existing case folder.')
              : t(
                  '这个目录中还没有算例，可以新建模拟。',
                  'This folder has no cases yet. Create a new simulation.',
                )}
          </p>
        )}
        <div className="project-case-choices">
          {state.cases.map((entry) => (
            <Button
              key={entry.dir}
              disabled={!enabled}
              onClick={() =>
                void chooseCase(entry.dir, chooser === 'results' ? 'results' : 'settings').catch(
                  () => {},
                )
              }
            >
              <span>{entry.name}</span>
              {entry.has_history && (
                <span className="badge">{t('已有结果', 'Results available')}</span>
              )}
              <span className="project-chevron">›</span>
            </Button>
          ))}
        </div>
        <div className="actions">
          <Button
            disabled={!enabled}
            onClick={() =>
              void (chooser === 'results' ? otherResults() : browseProject()).catch(() => {})
            }
          >
            {t('选择其他目录', 'Browse another folder')}
          </Button>
          {chooser === 'simulation' && (
            <Button disabled={!enabled || !state.cases.length} onClick={openBatch}>
              {t('批量运行', 'Batch run')}
            </Button>
          )}
          {chooser === 'simulation' && (
            <Button disabled={!enabled} variant="primary" onClick={newSimulation}>
              {t('新建模拟', 'New simulation')}
            </Button>
          )}
        </div>
      </div>
    );
  return (
    <div className="project-home">
      <div className="project-intro">
        <h1>{t('开始你的研究', 'Start your research')}</h1>
        <p>
          {t(
            '从模拟开始，探索陆地系统的更多可能。',
            'Start with a simulation and explore how land systems work.',
          )}
        </p>
      </div>
      {!versionChooser && error && <Notice tone="error">{error}</Notice>}
      {!versionChooser && busy && (
        <Notice>{t('正在打开，请稍候…', 'Opening, please wait…')}</Notice>
      )}
      <section
        className="project-version-section"
        aria-label={t('选择模型版本', 'Choose a model revision')}
      >
        <div className="project-version-heading">
          <h2>{t('模型版本', 'Model version')}</h2>
          <p className={modelChosen ? 'project-chosen-model' : 'project-choice-pending'}>
            {modelChosen
              ? `${t('当前', 'Current')}: ${state.version!.version.id === 'official' ? t('官方版', 'Official') : state.version!.name}`
              : t('正在读取…', 'Loading…')}
          </p>
        </div>
        <p className="project-version-hint">
          {t(
            '默认使用官方版。要用自己改过的版本，在这里换。',
            'The official model is used by default. Switch here to use a modified version.',
          )}
        </p>
        <div className="project-actions">
          <Button
            className="project-action project-version-card"
            variant="ghost"
            aria-pressed={officialSelected}
            disabled={!versionEnabled || !official}
            onClick={() => void chooseVersion(versionKey(official!)).catch(() => {})}
          >
            <span className="project-action-icon primary">
              <NavIcon name="project" />
            </span>
            <span>
              <strong>{t('官方版', 'Official model')}</strong>
              <span>{t('使用随软件提供的模型版本', 'Use the model bundled with CoLM')}</span>
            </span>
            <span className="project-chevron">›</span>
          </Button>
          <Button
            className="project-action project-version-card"
            variant="ghost"
            aria-pressed={existingSelected}
            disabled={!versionEnabled}
            onClick={() => changeVersionChooser(true)}
          >
            <span className="project-action-icon">
              <NavIcon name="history" />
            </span>
            <span>
              <strong>{t('已有版本', 'Existing versions')}</strong>
              <span>{t('选择已保存的模型版本', 'Choose a saved model revision')}</span>
            </span>
            <span className="project-chevron">›</span>
          </Button>
          <Button
            className="project-action project-version-card"
            variant="ghost"
            disabled={!versionEnabled || !modelReady}
            onClick={() => navigate('versions')}
          >
            <span className="project-action-icon">＋</span>
            <span>
              <strong>{t('新建修改版', 'New modified model')}</strong>
              <span>
                {t('派生、修改并选择新的版本', 'Derive, modify and select a new revision')}
              </span>
            </span>
            <span className="project-chevron">›</span>
          </Button>
        </div>
      </section>
      <Dialog
        title={t('选择已有版本', 'Choose an existing version')}
        open={versionChooser}
        onOpenChange={changeVersionChooser}
      >
        {error && <Notice tone="error">{error}</Notice>}
        {busy && <Notice>{t('正在读取版本…', 'Loading the revision…')}</Notice>}
        <div className="project-version-list">
          {versionRows.length ? (
            versionRows.map(({ version, depth }) => (
              <Button
                key={versionKey(version)}
                className="project-version-option"
                variant="ghost"
                style={{ paddingLeft: `${1 + depth * 1.5}rem` }}
                disabled={!versionEnabled}
                aria-pressed={versionKey(version) === JSON.stringify(state.versionRef)}
                onClick={() => void chooseVersion(versionKey(version)).catch(() => {})}
              >
                <span>
                  <strong>
                    {version.version.id === 'official' ? t('官方版', 'Official') : version.name}
                  </strong>
                  <span className="muted">
                    {version.version.id === 'official'
                      ? t('随软件提供，不能修改', 'Bundled with the app; read-only')
                      : version.note}
                  </span>
                </span>
                <span>
                  {versionKey(version) === JSON.stringify(state.versionRef)
                    ? t('当前选择', 'Selected')
                    : '›'}
                </span>
              </Button>
            ))
          ) : (
            <p className="muted">{t('暂无可选择的版本。', 'No saved revisions are available.')}</p>
          )}
        </div>
      </Dialog>
      <section
        className={`project-task-section ${modelChosen ? '' : 'awaiting-model'}`}
        aria-label={t('选择操作', 'Choose an action')}
      >
        <div className="project-task-heading">
          <h2>{t('开始', 'Start')}</h2>
          <p>
            {modelChosen
              ? t(
                  '使用已选版本开始或继续研究。',
                  'Start or continue your research with this revision.',
                )
              : t('正在读取模型版本…', 'Loading the model version…')}
          </p>
        </div>
        <div className="project-actions">
          <Button
            className="project-action"
            variant="ghost"
            disabled={!enabled}
            onClick={newSimulation}
          >
            <span className="project-action-icon primary">＋</span>
            <span>
              <strong>{t('新建模拟', 'New simulation')}</strong>
              <span>
                {t(
                  '设置站点、时段和配置，开始一次新的模拟。',
                  'Choose a site, period and data for a new simulation.',
                )}
              </span>
            </span>
            <span className="project-chevron">›</span>
          </Button>
          <Button
            className="project-action"
            variant="ghost"
            disabled={!enabled || !ready}
            onClick={() => void browseProject().catch(() => {})}
          >
            <span className="project-action-icon">
              <NavIcon name="project" />
            </span>
            <span>
              <strong>{t('打开项目', 'Open project')}</strong>
              <span>
                {t(
                  '浏览包含算例的项目目录，继续你的研究。',
                  'Browse a project folder containing cases and continue your work.',
                )}
              </span>
            </span>
            <span className="project-chevron">›</span>
          </Button>
          <Button
            className="project-action"
            variant="ghost"
            disabled={!enabled}
            onClick={viewResults}
          >
            <span className="project-action-icon primary">
              <NavIcon name="results" />
            </span>
            <span>
              <strong>{t('直接查看结果', 'View results')}</strong>
              <span>
                {t(
                  '打开已有的模拟结果，进行分析与评估。',
                  'Open existing simulation results for analysis and evaluation.',
                )}
              </span>
            </span>
            <span className="project-chevron">›</span>
          </Button>
        </div>
      </section>
      <section
        className="project-scenarios-section"
        aria-label={t('科研场景与模板', 'Research scenarios & templates')}
      >
        <div className="project-task-heading">
          <h2>{t('科研场景与推荐模板', 'Research scenarios & templates')}</h2>
          <p>
            {t(
              '根据研究目标选用标准范式与预设配置，快速开始模拟。',
              'Choose a standard scientific workflow and recommended setup for your research.',
            )}
          </p>
        </div>
        <div className="project-scenario-grid">
          <div className="project-scenario-card">
            <div className="project-scenario-header">
              <span className="project-scenario-badge primary">{t('开箱即用', 'Bundled')}</span>
              <span className="project-scenario-tag">{t('站点通量', 'Single-site')}</span>
            </div>
            <h3>{t('单点生态水文通量验证', 'Single-site Ecohydrology')}</h3>
            <p>
              {t(
                'CN-Cng 草地站，自带通量观测与逐半小时强迫场，验证感热、潜热与地表能量平衡。',
                'CN-Cng grassland site with bundled observations and half-hourly forcing for energy balance validation.',
              )}
            </p>
            <div className="project-scenario-footer">
              <Button
                variant="primary"
                disabled={!enabled || !state.version}
                onClick={() => void example().catch(() => {})}
              >
                {t('打开草地示例', 'Open grassland example')}
              </Button>
            </div>
          </div>

          <div className="project-scenario-card">
            <div className="project-scenario-header">
              <span className="project-scenario-badge">{t('物理过程', 'Process')}</span>
              <span className="project-scenario-tag">BGC / CH₄</span>
            </div>
            <h3>{t('碳氮生物地球化学循环', 'Carbon-Nitrogen BGC')}</h3>
            <p>
              {t(
                '植物功能型（PFT）动态分配、光合呼吸、土壤碳氮库演变以及湿地甲烷排放模拟。',
                'Plant functional type dynamics, photosynthesis, soil carbon-nitrogen pools, and wetland methane emissions.',
              )}
            </p>
            <div className="project-scenario-footer">
              <Button variant="ghost" disabled={!enabled} onClick={newSimulation}>
                {t('配置模拟向导', 'Setup simulation')} ›
              </Button>
            </div>
          </div>

          <div className="project-scenario-card">
            <div className="project-scenario-header">
              <span className="project-scenario-badge">{t('空间水文', 'Spatial')}</span>
              <span className="project-scenario-tag">{t('汇流与水库', 'Hydrology')}</span>
            </div>
            <h3>{t('流域河网汇流与水库调度', 'Watershed Routing & Reservoirs')}</h3>
            <p>
              {t(
                '河网水动力学汇流、湖泊水热交换、GRanD 水库调度，产物与陆面网格对齐。',
                'Hydrodynamic routing, lake heat-water exchange, GRanD reservoir operations, and aligned runoff outputs.',
              )}
            </p>
            <div className="project-scenario-footer">
              <Button variant="ghost" disabled={!enabled} onClick={newSimulation}>
                {t('配置模拟向导', 'Setup simulation')} ›
              </Button>
            </div>
          </div>

          <div className="project-scenario-card">
            <div className="project-scenario-header">
              <span className="project-scenario-badge">{t('调优拟合', 'Study')}</span>
              <span className="project-scenario-tag">{t('参数与网络', 'Parameters')}</span>
            </div>
            <h3>{t('参数敏感性与率定实验', 'Sensitivity & Calibration')}</h3>
            <p>
              {t(
                '参数空间敏感性分析、多站点批量调参与混合神经网络拟合，探索最优物理配置。',
                'Parameter sensitivity analysis, multi-site batch tuning, and hybrid neural network fitting.',
              )}
            </p>
            <div className="project-scenario-footer">
              <Button variant="ghost" disabled={!enabled} onClick={() => navigate('research')}>
                {t('进入研究模块', 'Open research')} ›
              </Button>
            </div>
          </div>
        </div>
      </section>

      <section
        className="project-readiness-section"
        aria-label={t('系统与数据就绪状态', 'System & data readiness')}
      >
        <div className="project-task-heading">
          <h2>{t('运行环境与数据就绪', 'Environment & data readiness')}</h2>
        </div>
        <div className="project-readiness-card">
          <div className="readiness-item">
            <span className="status-indicator online" />
            <div className="readiness-info">
              <strong>{t('Rust 逐位一致引擎', 'Rust Bitwise-Exact Engine')}</strong>
              <span>
                {t(
                  '已就绪 · 包含 5 个科学预设内核，支持地表数据、初始场与主循环',
                  'Ready · 5 preset scientific kernels loaded for surface, init, and simulation',
                )}
              </span>
            </div>
            <span className="badge">{t('内核正常', 'Kernels OK')}</span>
          </div>

          <div className="readiness-item">
            <span className="status-indicator active" />
            <div className="readiness-info">
              <strong>{t('当前工作目录', 'Current working folder')}</strong>
              <span className="path">
                {projectLocation ||
                  t(
                    '未选择项目目录（新建模拟时将使用默认路径）',
                    'No project folder selected (default path used for new simulations)',
                  )}
              </span>
            </div>
            <div className="readiness-actions">
              <Button
                variant="ghost"
                disabled={!enabled || !ready}
                onClick={() => void browseProject().catch(() => {})}
              >
                {state.root ? t('切换目录', 'Switch folder') : t('选择目录', 'Select folder')}
              </Button>
            </div>
          </div>

          {shownRoots.length > 0 && (
            <div className="readiness-recents">
              <span className="muted">{t('打开过的目录:', 'Previous folders:')}</span>
              {shownRoots.slice(0, 3).map((root) => (
                <Button
                  key={root}
                  variant="ghost"
                  className="readiness-recent-chip"
                  disabled={!enabled || !ready}
                  title={root}
                  onClick={() => void work(() => loadProject(root)).catch(() => {})}
                >
                  {projectName(root)}
                </Button>
              ))}
            </div>
          )}

          <div className="readiness-footer">
            <Button variant="ghost" disabled={!enabled} onClick={() => navigate('preparation')}>
              {t('准备输入数据与探针', 'Prepare input data & probes')}
            </Button>
            <Button variant="ghost" disabled={!enabled} onClick={() => navigate('history')}>
              {t('查看历史记录流水', 'View full history log')} ›
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}
