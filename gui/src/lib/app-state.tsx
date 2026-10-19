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
import { invoke, invokeJson, listen, hasBackend, bindRunningTargets } from './ipc';
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
import { projectToSave, readSavedProject, writeSavedProject } from './navigation';
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
  expert: false,
  language: localStorage.getItem('colm-ui-language') === 'en' ? 'en' : 'zh',
  theme: localStorage.getItem('colm-ui-theme') === 'dark' ? 'dark' : 'light',
  assistantOpen: localStorage.getItem('colm-ui-assistant') !== 'false',
  assistantSettingsOpen: false,
  running: {},
  notices: [],
  busy: false,
};
type AppContextValue = {
  state: AppState;
  update: (patch: AppPatch) => void;
  navigate: (route: Route, tab?: AppState['caseTab']) => boolean;
  registerNavigationGuard: (guard: NavigationGuard) => () => void;
  notify: (message: string, error?: boolean) => void;
  refreshCases: () => Promise<void>;
  openCase: (dir: string, tab?: AppState['caseTab']) => Promise<OpenedCase>;
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
  const guards = useRef(new Set<NavigationGuard>());
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
      const epoch = ++openEpoch.current;
      pendingOpen.current = epoch;
      update({ busy: true });
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
          if (!askToRebind(rebindQuestion(missing, target, bilingual)))
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
        if (epoch !== openEpoch.current)
          throw new Error(
            bilingual('打开期间页面已变化，请重新打开', 'The page changed while opening the case.'),
          );
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
      } finally {
        if (pendingOpen.current === epoch) {
          pendingOpen.current = null;
          update({ busy: false });
        }
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
  }, [state.language, state.theme, state.assistantOpen]);
  // Reopen the project worked on last, on the page it was left on; a folder that no longer
  // lists is forgotten.
  const projectRestored = useRef(false);
  useEffect(() => {
    const saved = readSavedProject(localStorage);
    if (!saved || !hasBackend()) {
      projectRestored.current = true;
      return;
    }
    let alive = true;
    void invoke<CaseEntry[]>('list_cases', { root: saved.root })
      .then((cases) => {
        if (alive && !current.current.root)
          update({
            root: saved.root,
            cases,
            projectView: { ...saved, home: current.current.route !== 'project' },
          });
      })
      .catch(() => {})
      .finally(() => {
        projectRestored.current = true;
      });
    return () => {
      alive = false;
    };
  }, [update]);
  useEffect(() => {
    if (projectRestored.current)
      writeSavedProject(localStorage, projectToSave(state, readSavedProject(localStorage)));
    // Only the project and its view are remembered.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.root, state.projectView]);
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
      update({ busy: true });
      try {
        await invoke('backend_ready');
        const [kernels, recent, versions] = await Promise.all([
          invoke<KernelEntry[]>('list_kernels'),
          invoke<Record<string, string>>('load_recent'),
          invokeJson<FrozenRevision[]>('model_list'),
        ]);
        if (!alive) return;
        const official = versions
          .filter((v) => v.version.id === 'official')
          .sort((a, b) => b.version.revision - a.version.revision)[0];
        const root = recent.root ?? '';
        let cases: CaseEntry[] = [];
        if (root) {
          try {
            cases = await invoke<CaseEntry[]>('list_cases', { root });
          } catch (error) {
            if (alive) notify(String(error));
          }
        }
        if (alive)
          update({
            kernels,
            kernelDir: kernels.find((k) => k.preset === 'default')?.dir ?? kernels[0]?.dir ?? null,
            root,
            cases,
            versions,
            version: official ?? null,
            ...(official ? { versionRef: official.version } : {}),
          });
        // 版本列表到手之后，在后台整理没人用的旧官方修订（运行或创建中时后端会拒绝，只记日志）。
        void tidyModelsAfterStartup(invokeJson).then((removed) => {
          if (alive && removed.length)
            update((old) => ({ versions: withoutRevisions(old.versions, removed) }));
        });
      } catch (error) {
        if (alive) notify(String(error));
      } finally {
        if (alive) update({ busy: false });
      }
    }
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
        refreshCases,
        openCase,
        refreshVersions,
      }}
    >
      {children}
    </Context.Provider>
  );
}
