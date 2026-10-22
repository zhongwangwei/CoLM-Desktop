import { bilingual } from './bilingual';
import {
  createContext,
  useContext,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { invoke, invokeJson, listen, hasBackend, bindRunningTargets, withTimeout } from './ipc';
import { applyRunEvent } from './run-events';
import {
  askToRebind,
  missingRevision,
  rebindQuestion,
  tidyModelsAfterStartup,
  versionLabel,
  withoutRevisions,
} from './model-tidy';
import { navigationBlock, type NavigationGuard, type NavigationTarget } from './navigation-guard';
import { projectName, projectToSave, readSavedProject, writeSavedProject } from './navigation';
import {
  RECENT_PROJECTS_KEY,
  RECENT_PROJECT_NAMES_KEY,
  namesFor,
  recentProjectNames,
  recentProjectRoots,
  rememberProject,
} from '../features/projects/project-model';
import type {
  AppState,
  AppPatch,
  CaseEntry,
  FrozenRevision,
  OpenedCase,
  KernelEntry,
  Route,
  VersionRef,
} from './types';
const initial: AppState = {
  route: 'project',
  caseTab: 'settings',
  selectedCase: null,
  cases: [],
  kernels: [],
  kernelDir: null,
  version: null,
  versionRef: { id: 'official', revision: 1 },
  chosenVersion: null,
  versions: [],
  root: '',
  newCaseSeed: null,
  simulationReturn: null,
  // 设置页的"默认展开专家设置"：和语言、外观一样记住，原来每次重开都回到不展开。
  expert: localStorage.getItem('colm-ui-expert') === 'true',
  language: localStorage.getItem('colm-ui-language') === 'en' ? 'en' : 'zh',
  theme: localStorage.getItem('colm-ui-theme') === 'dark' ? 'dark' : 'light',
  assistantOpen: localStorage.getItem('colm-ui-assistant') !== 'false',
  assistantSettingsOpen: false,
  running: {},
  notices: [],
  busy: false,
  starting: true,
  // 封面显示到首次加载完成；失败时保留封面并给"重试启动"，不再 20 秒后无声消失（GUI 审查 P1-3）。
  startup: { phase: 'booting', stage: 'backend', startedAt: Date.now() },
};
type AppContextValue = {
  state: AppState;
  update: (patch: AppPatch) => void;
  navigate: (route: Route, tab?: AppState['caseTab']) => boolean;
  registerNavigationGuard: (guard: NavigationGuard) => () => void;
  notify: (message: string, error?: boolean) => void;
  /** 重新尝试启动期失败的阶段；封面上的"重试启动"用它。 */
  retryStartup: () => void;
  refreshCases: () => Promise<void>;
  /** `null`：打开还没完成就被后来的页面切换或另一次打开取代了，界面以后来的为准，不算出错。 */
  openCase: (dir: string, tab?: AppState['caseTab']) => Promise<OpenedCase | null>;
  refreshVersions: () => Promise<void>;
};
const Context = createContext<AppContextValue | null>(null);
export const useApp = () => {
  const value = useContext(Context);
  if (!value) throw new Error('AppProvider is missing');
  return value;
};
export function useNavigationGuard(guard: NavigationGuard) {
  const { registerNavigationGuard } = useApp();
  const latest = useRef(guard);
  latest.current = guard;
  useLayoutEffect(() => registerNavigationGuard(() => latest.current()), [registerNavigationGuard]);
}
export function AppProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState(initial);
  const current = useRef(state);
  current.current = state;
  const openEpoch = useRef(0);
  const pendingOpen = useRef<number | null>(null);
  // 正在进行的那次打开：同一算例、同一页签再点一次时接着等它，不重新开始。
  const inflightOpen = useRef<{
    key: string;
    epoch: number;
    promise: Promise<OpenedCase | null>;
  } | null>(null);
  const guards = useRef(new Set<NavigationGuard>());
  // 启动流程放在 effect 里（事件监听也一起），重试按钮通过这个 ref 重新调用同一次 boot。
  const bootRef = useRef<(() => Promise<void>) | null>(null);
  const update = useCallback(
    (patch: AppPatch) =>
      setState((old) => {
        const next = { ...old, ...(typeof patch === 'function' ? patch(old) : patch) };
        current.current = next;
        return next;
      }),
    [],
  );
  const notify = useCallback(
    (message: string, error = false) =>
      update((old) => {
        const notices =
          old.notices.at(-1) === message ? old.notices : [...old.notices.slice(-4), message];
        const errors = (old.errorNotices ?? []).filter((item) => notices.includes(item));
        return {
          notices,
          errorNotices: error && !errors.includes(message) ? [...errors, message] : errors,
        };
      }),
    [update],
  );
  const registerNavigationGuard = useCallback((guard: NavigationGuard) => {
    guards.current.add(guard);
    return () => {
      guards.current.delete(guard);
    };
  }, []);
  const retryStartup = useCallback(() => {
    // 已经在启动中就不要叠第二次。
    if (current.current.startup?.phase === 'booting') return;
    void bootRef.current?.();
  }, []);
  const blocked = useCallback(
    (target: NavigationTarget) =>
      navigationBlock(
        { route: current.current.route, caseDir: current.current.selectedCase?.entry.dir },
        target,
        guards.current,
      ),
    [],
  );
  const navigate = useCallback(
    (route: Route, tab?: AppState['caseTab']) => {
      const reason = blocked({ route });
      if (reason) {
        notify(reason);
        return false;
      }
      openEpoch.current++;
      const cancelled = pendingOpen.current !== null;
      pendingOpen.current = null;
      update({ route, ...(tab ? { caseTab: tab } : {}), ...(cancelled ? { busy: false } : {}) });
      return true;
    },
    [blocked, notify, update],
  );
  const refreshCases = useCallback(async () => {
    const root = current.current.root;
    if (!root) {
      update({ cases: [] });
      return;
    }
    const cases = await invoke<CaseEntry[]>('list_cases', { root });
    if (current.current.root === root) update({ cases });
  }, [update]);
  const refreshVersions = useCallback(async () => {
    const versions = await invokeJson<FrozenRevision[]>('model_list');
    const ref = current.current.versionRef;
    const version =
      versions.find((v) => v.version.id === ref.id && v.version.revision === ref.revision) ??
      versions
        .filter((v) => v.version.id === 'official')
        .sort((a, b) => b.version.revision - a.version.revision)[0] ??
      null;
    update({ versions, version, ...(version ? { versionRef: version.version } : {}) });
  }, [update]);
  const openCase = useCallback(
    async (dir: string, tab: AppState['caseTab'] = 'settings') => {
      if (!dir) throw new Error(bilingual('请选择算例目录', 'Select a case directory.'));
      const reason = blocked({ route: 'case', caseDir: dir, reload: true });
      if (reason) throw new Error(reason);
      // 等得久了又点同一个算例：接着等前一次，两次不会互相作废（2026-10-20 用户看到"打开期间页面已变化"）。
      const key = `${dir}\n${tab}`;
      const running = inflightOpen.current;
      if (running && running.key === key && pendingOpen.current === running.epoch)
        return running.promise;
      const epoch = ++openEpoch.current;
      pendingOpen.current = epoch;
      update({ busy: true });
      const opening = (async (): Promise<OpenedCase | null> => {
        try {
          const opened = await invoke<OpenedCase>('open_case', { dir });
          const latestOfficial = current.current.versions
            .filter((v) => v.version.id === 'official')
            .sort((a, b) => b.version.revision - a.version.revision)[0]?.version;
          const binding = await invokeJson<{ version: VersionRef }>('model_migrate_case', {
            dir,
          }).catch(async (error: unknown) => {
            // 绑定的修订已被整理（或删除）：问一句改绑到哪个版本，而不是抛出读不到 revision.json。
            const missing = missingRevision(error);
            const target = current.current.chosenVersion ?? latestOfficial;
            if (!missing || !target) throw error;
            if (!(await askToRebind(rebindQuestion(missing, target, bilingual))))
              throw new Error(
                bilingual(
                  `算例绑定的模型版本 ${versionLabel(missing)} 已不在模型库中；未改绑，算例未打开。`,
                  `The case's model revision ${versionLabel(missing)} is no longer in the model registry; it was not rebound or opened.`,
                ),
              );
            const rebound = await invokeJson<{ version: VersionRef }>('model_migrate_case', {
              dir,
              target,
            });
            notify(
              bilingual(
                `已把算例改绑到 ${versionLabel(target)}（原绑定的 ${versionLabel(missing)} 已整理）`,
                `Rebound the case to ${versionLabel(target)} (${versionLabel(missing)} was tidied away).`,
              ),
            );
            return rebound;
          });
          // 优先使用用户明确选择的版本；没有明确选择时恢复算例的修改版绑定，否则用最新官方版。
          // 运行时再把实际所用版本写回算例。
          const bound =
            binding.version.id !== 'official' &&
            current.current.versions.some(
              (v) =>
                v.version.id === binding.version.id &&
                v.version.revision === binding.version.revision,
            );
          // 只认用户明确选的版本（`chosenVersion`）；打开上一个算例时带出的版本不能串到别的算例上。
          const selectedVersion =
            current.current.chosenVersion ??
            (bound ? binding.version : (latestOfficial ?? current.current.versionRef));
          const version = await invokeJson<FrozenRevision>('model_get', { ...selectedVersion });
          const cases = await invoke<CaseEntry[]>('list_cases', { root: opened.root });
          // 打开期间页面切走了、或者又开始打开别的算例：这次作废，以后来的为准。不报错——后来的那次
          // 已经正常打开，或者用户本来就离开了，弹一个 Error 只会让人以为出了问题。
          if (epoch !== openEpoch.current) return null;
          const normalized = { ...opened, dir: opened.entry.dir, name: opened.entry.name, binding };
          update({
            selectedCase: normalized,
            root: opened.root,
            cases,
            version,
            versionRef: selectedVersion,
            route: 'case',
            caseTab: tab,
          });
          // 算例已经打开；记最近目录失败只提示，不能把已成功的打开报成失败。
          await invoke('save_recent', { key: 'root', value: opened.root }).catch((error) =>
            notify(String(error)),
          );
          return normalized;
        } catch (error) {
          // 已经被取代的那次打开出了错，也不是用户眼下在做的事。
          if (epoch !== openEpoch.current) return null;
          throw error;
        } finally {
          if (pendingOpen.current === epoch) {
            pendingOpen.current = null;
            update({ busy: false });
          }
        }
      })();
      inflightOpen.current = { key, epoch, promise: opening };
      try {
        return await opening;
      } finally {
        if (inflightOpen.current?.epoch === epoch) inflightOpen.current = null;
      }
    },
    [blocked, notify, update],
  );
  useEffect(() => {
    document.documentElement.lang = state.language === 'en' ? 'en' : 'zh-CN';
    document.documentElement.dataset.theme = state.theme;
    localStorage.setItem('colm-ui-language', state.language);
    localStorage.setItem('colm-ui-theme', state.theme);
    localStorage.setItem('colm-ui-assistant', String(state.assistantOpen));
    localStorage.setItem('colm-ui-expert', String(state.expert));
  }, [state.language, state.theme, state.assistantOpen, state.expert]);
  // 当前项目只在这次打开的应用里有效：启动时不再自动打开上次的项目（2026-10-21 用户：重新打开总显示
  // "当前项目 tianhe:uuu"，不合理）。首页列出最近的项目，点一下再打开；本次会话里各页之间仍能回到它。
  useEffect(() => {
    writeSavedProject(localStorage, projectToSave(state, readSavedProject(localStorage)));
    // Only the project and its view are remembered.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.root, state.projectView]);
  // 成为当前项目的目录都记进"最近的项目"，连同显示名：首页靠这份列表一点就开。原来只有项目页"选择目录"
  // 会记，从历史、新建模拟打开的项目（比如服务器项目）都不在里面。`root` 也照旧记，后台整理模型版本时
  // 靠它和这份列表判断哪些版本还有项目在用。
  useEffect(() => {
    const root = state.root;
    if (!root || !hasBackend()) return;
    // 算例还没读到时取不出服务器项目的名字，不拿哈希目录名去覆盖已有的名字。
    const name = state.cases.length ? projectName(root, state.cases) : '';
    void (async () => {
      const recent = await invoke<Record<string, string>>('load_recent');
      const roots = rememberProject(recentProjectRoots(recent[RECENT_PROJECTS_KEY]), root);
      const names = recentProjectNames(recent[RECENT_PROJECT_NAMES_KEY]);
      if (name) names[root] = name;
      await invoke('save_recent', { key: RECENT_PROJECTS_KEY, value: JSON.stringify(roots) });
      await invoke('save_recent', {
        key: RECENT_PROJECT_NAMES_KEY,
        value: JSON.stringify(namesFor(roots, names)),
      });
      await invoke('save_recent', { key: 'root', value: root });
    })().catch(() => {});
  }, [state.root, state.cases]);
  useEffect(() => {
    let alive = true;
    const identity = openEpoch;
    const unlisten: (() => void)[] = [];
    bindRunningTargets(
      () =>
        new Set(
          Object.entries(current.current.running)
            .filter(([, run]) => run.running)
            .map(([dir]) => dir),
        ),
    );
    if (!hasBackend()) {
      update({ starting: false, startup: undefined });
      notify(
        bilingual(
          '请使用桌面软件执行模型操作；浏览器仅能查看界面。',
          'Model operations require the desktop application.',
        ),
      );
      return () => {
        alive = false;
        bindRunningTargets(() => new Set());
      };
    }
    async function boot() {
      if (!alive) return;
      const startedAt = Date.now();
      let stage: NonNullable<AppState['startup']>['stage'] = 'backend';
      update({
        busy: true,
        starting: true,
        startup: { phase: 'booting', stage, startedAt },
      });
      try {
        // 每个阶段都有上限：命令永不返回时也要落到"失败 + 重试"，而不是一直转圈。
        await withTimeout(invoke('backend_ready'), 15_000, 'backend_ready');
        if (!alive) return;
        stage = 'kernels';
        update({ startup: { phase: 'booting', stage, startedAt } });
        // 内核与版本互不依赖：一个失败仍保留另一个的结果，避免一次失败把整屏都清空。
        // 超时只是不再干等，命令本身还在跑：晚到的结果照样接上，两样都到了就收起失败封面。原来超时后
        // 结果被丢掉，慢盘上首次哈希内核超过 30 秒时，"重试启动"每次都一样超时（2026-10-22 复查）。
        let gotKernels = false,
          gotVersions = false;
        const settle = () => {
          if (alive && gotKernels && gotVersions)
            update((old) => (old.startup?.phase === 'failed' ? { startup: undefined } : {}));
        };
        const kernelsCall = invoke<KernelEntry[]>('list_kernels');
        const versionsCall = invokeJson<FrozenRevision[]>('model_list');
        kernelsCall.then(
          (kernels) => {
            if (!alive) return;
            gotKernels = true;
            update({
              kernels,
              kernelDir:
                kernels.find((k) => k.preset === 'default')?.dir ?? kernels[0]?.dir ?? null,
            });
            settle();
          },
          () => {},
        );
        versionsCall.then(
          (versions) => {
            if (!alive) return;
            gotVersions = true;
            const official = versions
              .filter((v) => v.version.id === 'official')
              .sort((a, b) => b.version.revision - a.version.revision)[0];
            update({
              versions,
              version: official ?? null,
              ...(official ? { versionRef: official.version } : {}),
            });
            settle();
          },
          () => {},
        );
        const [kernelResult, versionResult] = await Promise.allSettled([
          withTimeout(kernelsCall, 30_000, 'list_kernels'),
          withTimeout(versionsCall, 30_000, 'model_list'),
        ]);
        if (!alive) return;
        const problems: string[] = [];
        if (kernelResult.status === 'rejected' && !gotKernels)
          problems.push(
            bilingual(
              `模型内核读取失败：${String(kernelResult.reason)}`,
              `Reading model kernels failed: ${String(kernelResult.reason)}`,
            ),
          );
        if (versionResult.status === 'rejected' && !gotVersions)
          problems.push(
            bilingual(
              `模型版本读取失败：${String(versionResult.reason)}`,
              `Reading model versions failed: ${String(versionResult.reason)}`,
            ),
          );
        if (problems.length) throw new Error(problems.join('；'));
        update({ startup: undefined });
        // 版本列表到手之后，在后台整理没人用的旧官方修订（运行或创建中时后端会拒绝，只记日志）。
        void tidyModelsAfterStartup(invokeJson).then((removed) => {
          if (alive && removed.length)
            update((old) => ({ versions: withoutRevisions(old.versions, removed) }));
        });
      } catch (error) {
        // 失败留在封面上（含阶段和原因），并给"重试启动"；不再只发一条可被清掉的通知。
        if (alive)
          update({
            startup: { phase: 'failed', stage, startedAt, error: String(error) },
          });
      } finally {
        if (alive) update({ busy: false, starting: false });
      }
    }
    bootRef.current = boot;
    void boot();
    const events = [
      'run://started',
      'run://stage',
      'run://progress',
      'run://lines',
      'run://done',
    ] as const;
    for (const event of events)
      void listen<Record<string, unknown>>(event, (payload) => {
        if (!alive) return;
        const dir =
          typeof payload.source_case === 'string'
            ? payload.source_case
            : typeof payload.case === 'string'
              ? payload.case
              : '';
        if (!dir) return;
        update((old) => {
          const next = applyRunEvent(old.running[dir], event, payload);
          return next ? { running: { ...old.running, [dir]: next } } : {};
        });
      }).then((stop) => {
        if (alive) unlisten.push(stop);
        else stop();
      });
    return () => {
      alive = false;
      bootRef.current = null;
      identity.current++;
      for (const stop of unlisten) stop();
      bindRunningTargets(() => new Set());
    };
  }, [notify, update]);
  return (
    <Context.Provider
      value={{
        state,
        update,
        navigate,
        registerNavigationGuard,
        notify,
        retryStartup,
        refreshCases,
        openCase,
        refreshVersions,
      }}
    >
      {children}
    </Context.Provider>
  );
}
