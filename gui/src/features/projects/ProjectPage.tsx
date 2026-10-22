import { bilingual } from '../../lib/bilingual';
import { choosableVersions, latestOfficial } from '../versions/version-tree';
import { useEffect, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { hasBackend, invoke, invokeJson } from '../../lib/ipc';
import { usePageContract } from '../../lib/guide';
import { ActionButton } from '../../components/ActionButton';
import { Button, Dialog, Notice } from '../../components/ui';
import './project-version.css';
import ProjectBatchRun from './ProjectBatchRun';
import NavIcon from '../../components/NavIcon';
import type { Site } from '../cases/case-model';
import type { CaseEntry, FrozenRevision, VersionRef } from '../../lib/types';
type VersionDeleteReport = {
  deleted: VersionRef[];
  failed: { version: VersionRef; reason: string }[];
  removed: string[];
};
import {
  RECENT_PROJECTS_KEY,
  RECENT_PROJECT_NAMES_KEY,
  recentProjectNames,
  recentProjectRoots,
  rememberProject,
  projectName,
} from './project-model';

export default function ProjectPage() {
  const { state, update, navigate, openCase, refreshVersions, notify } = useApp();
  const en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const [roots, setRoots] = useState<string[]>([]),
    [names, setNames] = useState<Record<string, string>>({}),
    [ready, setReady] = useState(!hasBackend()),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [versionChooser, setVersionChooser] = useState(false),
    [pendingDelete, setPendingDelete] = useState<{
      title: string;
      versions: VersionRef[];
    } | null>(null),
    [deleteReport, setDeleteReport] = useState<VersionDeleteReport | null>(null);
  const createdBatch = !!state.projectCasesOpen && state.projectRunSettings?.root === state.root,
    savedView = state.projectView?.root === state.root ? state.projectView : undefined,
    page = state.projectCasesOpen
      ? createdBatch
        ? 'batch'
        : 'cases'
      : savedView?.home
        ? undefined
        : savedView?.page === 'results'
          ? 'cases'
          : savedView?.page,
    // 首页只有"新建模拟 / 打开项目 / 准备输入数据"三张卡，不再有"直接查看结果"入口；
    // 结果从侧栏"结果"进入已打开算例的结果页。旧会话存下的 results 视图按算例列表处理。
    chooser = page ? 'simulation' : null,
    batchPage = page === 'batch',
    batchFromCreation = createdBatch || (batchPage && !!savedView?.fromCreation);
  const lock = useRef(false),
    active = useRef(true);
  useEffect(() => {
    if (state.projectCasesOpen)
      update({
        projectCasesOpen: false,
        projectView: {
          root: state.root,
          page: createdBatch ? 'batch' : 'cases',
          fromCreation: createdBatch,
        },
      });
  }, [state.projectCasesOpen, state.root, createdBatch, update]);
  useEffect(() => {
    let current = true;
    active.current = true;
    if (hasBackend())
      void invoke<Record<string, string>>('load_recent')
        .then((recent) => {
          if (current) {
            setRoots(recentProjectRoots(recent[RECENT_PROJECTS_KEY]));
            setNames(recentProjectNames(recent[RECENT_PROJECT_NAMES_KEY]));
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
  function setChooser(next: 'simulation' | null) {
    if (!active.current) return;
    update({
      projectView: next ? { root: state.root, page: 'cases' } : undefined,
    });
  }
  function openBatch() {
    update({ projectView: { root: state.root, page: 'batch' } });
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
  const selectedId = state.versionRef.id;
  // Older official revisions are app rebuilds nobody can choose any more; they hold source snapshots.
  const oldOfficial = state.versions
    .filter(
      (version) =>
        version.version.id === 'official' &&
        version.version.revision !== official?.version.revision &&
        versionKey(version) !== JSON.stringify(state.versionRef),
    )
    .map((version) => version.version);
  const revisionsOf = (id: string) =>
    state.versions
      .filter((version) => version.version.id === id)
      .map((version) => version.version)
      .sort((a, b) => b.revision - a.revision);
  function askDelete(version: FrozenRevision) {
    if (version.version.id === 'official' || version.version.id === selectedId) return;
    setDeleteReport(null);
    setPendingDelete({ title: version.name, versions: revisionsOf(version.version.id) });
  }
  function askCleanup() {
    if (!oldOfficial.length) return;
    setDeleteReport(null);
    setPendingDelete({
      title: t('旧的官方版本', 'Older official revisions'),
      versions: oldOfficial,
    });
  }
  async function deleteVersions() {
    if (!pendingDelete) return;
    const target = pendingDelete;
    return work(async () => {
      const report = await invokeJson<VersionDeleteReport>('model_delete', {
        versions: target.versions,
      });
      await refreshVersions();
      if (!active.current) return report;
      setPendingDelete(null);
      setDeleteReport(report);
      return report;
    });
  }
  async function loadProject(root: string) {
    const cases = await invoke<CaseEntry[]>('list_cases', { root });
    const next = rememberProject(shownRoots, root);
    // 记"最近的项目"只是方便：recent.json 坏了也照样打开项目，只提示一句（和 openCase 同一条规矩）。
    // 原来写不进就中止打开，openRecent 再把好好的项目当成"目录已删"从列表里拿掉（GUI 审查 2026-10-22，P1-4）。
    try {
      await invoke('save_recent', { key: 'root', value: root });
      await invoke('save_recent', { key: RECENT_PROJECTS_KEY, value: JSON.stringify(next) });
    } catch (error) {
      notify(
        t(
          `项目已打开，但"最近的项目"没记下来：${String(error)}`,
          `The project opened, but it could not be added to recent projects: ${String(error)}`,
        ),
        true,
      );
    }
    if (!active.current) return { root, cases };
    setRoots(next);
    update({
      root,
      cases,
      projectView: { root, page: 'cases' },
      ...(root !== state.root ? { selectedCase: null } : {}),
    });
    return { root, cases };
  }
  // 最近列表里的目录可能已经删了（临时目录、换过位置的项目）：打不开就从列表里拿掉，别一直留着。
  // 只在读目录失败时拿掉；"等当前操作完成"这类没真去读的失败不算。
  const openRecent = (root: string) =>
    work(async () => {
      try {
        return await loadProject(root);
      } catch (error) {
        // 只有目录真的读不出算例时才从列表里拿掉；其余失败（比如别处的写入错误）不动列表。
        const gone = await invoke<CaseEntry[]>('list_cases', { root }).then(
          () => false,
          () => true,
        );
        if (!gone) throw error;
        const next = roots.filter((item) => item !== root);
        if (active.current) setRoots(next);
        await invoke('save_recent', {
          key: RECENT_PROJECTS_KEY,
          value: JSON.stringify(next),
        }).catch(() => {});
        throw error;
      }
    });
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
  // Choosing a new simulation leaves the project worked on until now: the sidebar's current
  // project becomes the new one once it is created, not the old one meanwhile.
  const newSimulation = () => {
    if (!navigate('new-case')) return false;
    update({
      newCaseSeed: null,
      root: '',
      cases: [],
      selectedCase: null,
      projectView: undefined,
      projectCasesOpen: false,
    });
    return true;
  };
  // Opening a project means choosing from what was already done, so it goes to the history.
  const openProjectFromHistory = () => navigate('history');
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
      if (!active.current)
        throw new Error(
          bilingual(
            '准备示例期间离开了项目页，示例没有打开。',
            'The project page changed while preparing the example.',
          ),
        );
      const site = found.sites.find((item) => item.name === 'CN-Cng');
      if (!site || site.problem || !site.met_file || !site.start || !site.end)
        throw new Error(
          site?.problem ||
            t('CN-Cng 示例输入不完整。', 'The CN-Cng example inputs are incomplete.'),
        );
      // 当前项目里已经有同名算例（再开一次示例）时换个名字，不预填一个建不出来的目录（2026-10-22 用户实测）。
      const root = state.root || paths.root;
      const taken = new Set(
        state.root === root
          ? state.cases.map((entry) =>
              entry.dir
                .replace(/[\\/]+$/, '')
                .split(/[\\/]/)
                .pop(),
            )
          : [],
      );
      let name = site.name;
      for (let n = 2; taken.has(name); n++) name = `${site.name}-${n}`;
      update({
        newCaseSeed: {
          form: {
            root,
            name,
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
      setPendingDelete(null);
      setDeleteReport(null);
      setVersionChooser(open);
    }
  }
  usePageContract(
    {
      title: versionChooser
        ? t('选择已有版本', 'Choose an existing version')
        : chooser
          ? t('项目中的算例', 'Cases in this project')
          : t('项目', 'Project'),
      step: 'project',
      modal: versionChooser,
      status: busy
        ? 'working'
        : error || (modelChosen ? 'ready' : t('正在读取模型版本', 'Loading the model version')),
      fields: [],
      // 按钮自己登记（第 737 轮）。删除版本、清理旧官方版属于 D 级，只在界面上做，原来清单里
      // 却开放给了助手；最近项目原来列出全部、走 loadProject，界面只显示 5 个、走能处理坏记录的
      // openRecent。
      actions: [],
    },
    !batchPage,
  );
  if (batchPage)
    return (
      <ProjectBatchRun
        fromCreation={batchFromCreation}
        onBack={() => {
          // A reopened project keeps its filled-in wizard behind this page.
          if (!(state.newCaseSeed && navigate('new-case'))) setChooser('simulation');
        }}
      />
    );
  if (chooser)
    return (
      <div className="page-stack">
        <div className="page-heading">
          <h1>{t('项目中的算例', 'Cases in this project')}</h1>
          <ActionButton
            action="project-back"
            label={t('返回项目首页', 'Back to project home')}
            disabled={!enabled}
            run={() => setChooser(null)}
          />
        </div>
        <p className="path">{projectLocation}</p>
        {error && <Notice tone="error">{error}</Notice>}
        {busy && <Notice>{t('正在打开，请稍候…', 'Opening, please wait…')}</Notice>}
        {!state.cases.length && (
          <p className="muted">
            {t(
              '这个目录中还没有算例，可以新建模拟。',
              'This folder has no cases yet. Create a new simulation.',
            )}
          </p>
        )}
        <div className="project-case-choices">
          {state.cases.map((entry) => (
            <ActionButton
              key={entry.dir}
              action={`project-case:${entry.dir}`}
              label={entry.name}
              disabled={!enabled}
              run={() => chooseCase(entry.dir, 'settings')}
            >
              <span>{entry.name}</span>
              {entry.has_history && (
                <span className="badge">{t('已有结果', 'Results available')}</span>
              )}
              <span className="project-chevron">›</span>
            </ActionButton>
          ))}
        </div>
        <div className="actions">
          <ActionButton
            action="project-choose-other"
            label={t('选择其他目录', 'Browse another folder')}
            disabled={!enabled}
            run={browseProject}
          />
          <ActionButton
            action="project-batch-run"
            label={t('批量运行', 'Batch run')}
            disabled={!enabled || !state.cases.length}
            run={openBatch}
          />
          <ActionButton
            action="project-new-simulation"
            label={t('新建模拟', 'New simulation')}
            disabled={!enabled}
            variant="primary"
            run={newSimulation}
          />
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
          <ActionButton
            action="project-official"
            label={t('选择官方版', 'Select official model')}
            className="project-action project-version-card"
            variant="ghost"
            aria-pressed={officialSelected}
            disabled={!versionEnabled || !official}
            run={() => chooseVersion(versionKey(official!))}
          >
            <span className="project-action-icon primary">
              <NavIcon name="project" />
            </span>
            <span>
              <strong>{t('官方版', 'Official model')}</strong>
              <span>{t('使用随软件提供的模型版本', 'Use the model bundled with CoLM')}</span>
            </span>
            <span className="project-chevron">›</span>
          </ActionButton>
          <ActionButton
            action="project-existing"
            label={t('已有版本', 'Existing versions')}
            className="project-action project-version-card"
            variant="ghost"
            aria-pressed={existingSelected}
            disabled={!versionEnabled}
            run={() => changeVersionChooser(true)}
          >
            <span className="project-action-icon">
              <NavIcon name="history" />
            </span>
            <span>
              <strong>{t('已有版本', 'Existing versions')}</strong>
              <span>{t('选择已保存的模型版本', 'Choose a saved model revision')}</span>
            </span>
            <span className="project-chevron">›</span>
          </ActionButton>
          <ActionButton
            action="project-model"
            label={t('新建修改版', 'New modified model')}
            className="project-action project-version-card"
            variant="ghost"
            disabled={!versionEnabled || !modelReady}
            run={() => {
              update({ versionsView: 'create' });
              return navigate('versions');
            }}
          >
            <span className="project-action-icon">＋</span>
            <span>
              <strong>{t('新建修改版', 'New modified model')}</strong>
              <span>
                {t(
                  '手动改参数、率定、不确定性分析、AI、改源码',
                  'Edit parameters, calibrate, analyse uncertainty, AI, change code',
                )}
              </span>
            </span>
            <span className="project-chevron">›</span>
          </ActionButton>
        </div>
      </section>
      <Dialog
        title={t('选择已有版本', 'Choose an existing version')}
        open={versionChooser}
        onOpenChange={changeVersionChooser}
        closeAction="project-version-close"
      >
        {error && <Notice tone="error">{error}</Notice>}
        {busy && <Notice>{t('正在读取版本…', 'Loading the revision…')}</Notice>}
        {pendingDelete ? (
          <div className="project-version-delete">
            <Notice tone="warning">
              {t(
                `将永久删除“${pendingDelete.title}”的 ${pendingDelete.versions.length} 个版本修订，以及只被它们使用的源码快照。使用这些版本的历史记录仍在时不会删除，算例下次打开会改用最新官方版。`,
                `Permanently delete ${pendingDelete.versions.length} revisions of “${pendingDelete.title}” and source snapshots only they use. Revisions still named by history records are kept; cases using them open with the latest official model.`,
              )}
            </Notice>
            <div className="button-row">
              <Button disabled={!versionEnabled} onClick={() => setPendingDelete(null)}>
                {t('取消', 'Cancel')}
              </Button>
              <Button
                variant="danger"
                disabled={!versionEnabled}
                onClick={() => void deleteVersions().catch(() => {})}
              >
                {busy ? t('正在删除…', 'Deleting…') : t('确认删除', 'Confirm deletion')}
              </Button>
            </div>
          </div>
        ) : (
          <>
            {deleteReport && (
              <Notice tone={deleteReport.failed.length ? 'warning' : 'success'}>
                {t(
                  `已删除 ${deleteReport.deleted.length} 个版本修订，清理 ${deleteReport.removed.length - deleteReport.deleted.length} 个源码快照；保留 ${deleteReport.failed.length} 个。`,
                  `Deleted ${deleteReport.deleted.length} revisions and ${deleteReport.removed.length - deleteReport.deleted.length} source snapshots; kept ${deleteReport.failed.length}.`,
                )}
                {[...new Set(deleteReport.failed.map((failure) => failure.reason))].map(
                  (reason) => (
                    <span key={reason} className="project-version-delete-reason">
                      {reason}
                    </span>
                  ),
                )}
              </Notice>
            )}
            <div className="project-version-list">
              {versionRows.length ? (
                versionRows.map(({ version, depth }) => (
                  <div
                    key={versionKey(version)}
                    className="project-version-row"
                    style={{ paddingLeft: `${depth * 1.5}rem` }}
                  >
                    <ActionButton
                      action={`project-version:${versionKey(version)}`}
                      label={`${version.name} · r${version.version.revision}`}
                      className="project-version-option"
                      variant="ghost"
                      disabled={!versionEnabled}
                      aria-pressed={versionKey(version) === JSON.stringify(state.versionRef)}
                      run={() => chooseVersion(versionKey(version))}
                    >
                      <span>
                        <strong>
                          {version.version.id === 'official'
                            ? t('官方版', 'Official')
                            : version.name}
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
                    </ActionButton>
                    {version.version.id !== 'official' && (
                      <Button
                        variant="ghost"
                        className="project-version-delete-button"
                        disabled={!versionEnabled || version.version.id === selectedId}
                        title={
                          version.version.id === selectedId
                            ? t('先选择其他版本再删除', 'Choose another version before deleting')
                            : undefined
                        }
                        aria-label={`${t('删除版本', 'Delete version')} ${version.name}`}
                        onClick={() => askDelete(version)}
                      >
                        {t('删除', 'Delete')}
                      </Button>
                    )}
                  </div>
                ))
              ) : (
                <p className="muted">
                  {t('暂无可选择的版本。', 'No saved revisions are available.')}
                </p>
              )}
            </div>
            {oldOfficial.length > 0 && (
              <Button disabled={!versionEnabled} onClick={askCleanup}>
                {t(
                  `清理旧的官方版本（${oldOfficial.length} 个）`,
                  `Clean up older official revisions (${oldOfficial.length})`,
                )}
              </Button>
            )}
          </>
        )}
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
          <ActionButton
            action="project-new-simulation"
            label={t('新建模拟', 'New simulation')}
            className="project-action"
            variant="ghost"
            disabled={!enabled}
            run={newSimulation}
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
          </ActionButton>
          <ActionButton
            action="project-open"
            label={t('打开项目', 'Open project')}
            className="project-action"
            variant="ghost"
            disabled={!enabled || !ready}
            run={openProjectFromHistory}
          >
            <span className="project-action-icon">
              <NavIcon name="project" />
            </span>
            <span>
              <strong>{t('打开项目', 'Open project')}</strong>
              <span>
                {/* 点了进历史记录（第 716 轮用户定的），文案原来写"浏览目录"，和行为对不上。 */}
                {t(
                  '从历史记录里找回做过的项目；其他目录用下方的"选择目录"（或"切换目录"）。',
                  'Pick a project you worked on from the history; for other folders use “Select folder” (or “Switch folder”) below.',
                )}
              </span>
            </span>
            <span className="project-chevron">›</span>
          </ActionButton>
          <ActionButton
            action="project-prepare"
            label={t('准备输入数据', 'Prepare input data')}
            className="project-action"
            variant="ghost"
            disabled={!enabled}
            run={() => navigate('preparation')}
          >
            <span className="project-action-icon">
              <NavIcon name="data" />
            </span>
            <span>
              <strong>{t('准备输入数据', 'Prepare input data')}</strong>
              <span>
                {t(
                  '制作站点文件，或转换、修复强迫场，供新建算例使用。',
                  'Make site files, or convert and repair forcing for a new case.',
                )}
              </span>
            </span>
            <span className="project-chevron">›</span>
          </ActionButton>
        </div>
      </section>
      {/* 设计里 CN-Cng 示例与数据准备是次要可见入口；只留在助手契约里等于界面上不存在。 */}
      <section className="project-example" aria-label={t('内置示例', 'Bundled example')}>
        <img src="assets/grassland-example.svg" alt="" />
        <div>
          <h2>{t('CN-Cng 草地示例', 'CN-Cng grassland example')}</h2>
          <p>
            {t(
              '内置草地站点的强迫场与观测，点开就能建一个示例算例。',
              'Bundled forcing and observations for a grassland site; open it to start an example case.',
            )}
          </p>
        </div>
        <ActionButton
          action="project-example"
          kind="commit"
          label={t('打开 CN-Cng 示例', 'Open CN-Cng example')}
          variant="secondary"
          disabled={!enabled || !state.version}
          run={example}
        >
          {t('打开示例', 'Open example')}
        </ActionButton>
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
                  '已就绪 · 支持地表数据、初始场与主循环',
                  'Ready · Surface data, initialization, and simulation engines available',
                )}
              </span>
            </div>
            <span className="badge">{t('模型内核就绪', 'Model kernel ready')}</span>
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
              <ActionButton
                action="project-browse"
                label={state.root ? t('切换目录', 'Switch folder') : t('选择目录', 'Select folder')}
                variant="ghost"
                disabled={!enabled || !ready}
                run={browseProject}
              />
            </div>
          </div>

          {shownRoots.length > 0 && (
            <div className="readiness-recents">
              <span className="muted">{t('最近的项目:', 'Recent projects:')}</span>
              {/* 启动时不再自动打开上次的项目，这里是重新打开的主要入口，所以多列几个。 */}
              {shownRoots.slice(0, 5).map((root) => (
                <ActionButton
                  key={root}
                  action={`project-recent:${root}`}
                  label={names[root] ?? projectName(root)}
                  variant="ghost"
                  className="readiness-recent-chip"
                  disabled={!enabled || !ready}
                  title={root}
                  run={() => openRecent(root)}
                />
              ))}
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
