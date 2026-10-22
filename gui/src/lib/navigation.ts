import type { AppState, CaseEntry, Route } from './types';

export const navigation = [
  { id: 'project', zh: '项目', en: 'Project', route: 'project' },
  { id: 'simulation', zh: '模拟', en: 'Simulation', route: 'case', tab: 'settings' },
  { id: 'results', zh: '结果', en: 'Results', route: 'case', tab: 'results' },
  { id: 'research', zh: '研究', en: 'Research', route: 'research' },
  { id: 'history', zh: '历史记录', en: 'History', route: 'history' },
  { id: 'resources', zh: '计算资源', en: 'Compute resources', route: 'resources' },
  { id: 'settings', zh: '设置', en: 'Settings', route: 'settings' },
] as const satisfies readonly {
  id: string;
  zh: string;
  en: string;
  route: Route;
  tab?: AppState['caseTab'];
}[];

export function navigationTarget(
  item: (typeof navigation)[number],
  state: Pick<AppState, 'root' | 'projectView'>,
): {
  route: Route;
  tab?: AppState['caseTab'];
  projectView?: AppState['projectView'];
  researchHome?: boolean;
} {
  // 侧栏的"研究"总是回研究首页：已经在研究页时页面不会重新挂载，要靠这个标记让它收起工作台；
  // 从草稿带来的"要写回的草稿"也只属于那次进入，侧栏进来时要清掉。
  if (item.id === 'research') return { route: 'research', researchHome: true };
  if (item.id === 'project' && state.projectView?.root === state.root)
    return { route: 'project', projectView: { ...state.projectView, home: true } };
  if (
    item.id === 'simulation' &&
    state.projectView?.root === state.root &&
    state.projectView?.page === 'batch'
  )
    return { route: 'project', projectView: { ...state.projectView, home: false } };
  return { route: item.route, ...('tab' in item ? { tab: item.tab } : {}) };
}

export function activeNavigation(
  state: Pick<AppState, 'route' | 'caseTab'> & Partial<Pick<AppState, 'root' | 'projectView'>>,
) {
  if (
    state.route === 'project' &&
    state.projectView?.root === state.root &&
    state.projectView?.page === 'batch' &&
    !state.projectView.home
  )
    return 'simulation';
  // 算例里的"运行记录"页内容就是这个算例的历史记录：侧栏点亮"历史记录"，不是"模拟"。
  if (state.route === 'case')
    return state.caseTab === 'results'
      ? 'results'
      : state.caseTab === 'runs'
        ? 'history'
        : 'simulation';
  if (state.route === 'new-case') return 'simulation';
  // 侧栏的"研究"先保留（2026-10-21 用户收回了去掉它的要求）；从"新建修改版"进去的研究也点亮它。
  if (['preparation', 'versions'].includes(state.route)) return 'project';
  if (state.route === 'help') return 'settings';
  return state.route;
}

/** The project being worked on and the page it was left on, kept across navigation and restarts. */
export type SavedProject = {
  root: string;
  page: 'cases' | 'results' | 'batch';
  fromCreation?: boolean;
};
const SAVED_PROJECT_KEY = 'colm-ui-project';

export function readSavedProject(storage: Pick<Storage, 'getItem'>): SavedProject | null {
  try {
    const value = JSON.parse(storage.getItem(SAVED_PROJECT_KEY) ?? 'null') as SavedProject | null;
    return value &&
      typeof value.root === 'string' &&
      value.root &&
      ['cases', 'results', 'batch'].includes(value.page)
      ? value
      : null;
  } catch {
    return null;
  }
}

/** What to remember for the current state, keeping the page saved earlier for the same project. */
export function projectToSave(
  state: Pick<AppState, 'root' | 'projectView'>,
  saved: SavedProject | null,
): SavedProject | null {
  if (!state.root) return null;
  const view = state.projectView?.root === state.root ? state.projectView : undefined;
  if (view)
    return {
      root: state.root,
      page: view.page,
      ...(view.fromCreation ? { fromCreation: true } : {}),
    };
  return saved?.root === state.root ? saved : { root: state.root, page: 'cases' };
}

export function writeSavedProject(
  storage: Pick<Storage, 'setItem' | 'removeItem'>,
  project: SavedProject | null,
) {
  try {
    if (project) storage.setItem(SAVED_PROJECT_KEY, JSON.stringify(project));
    else storage.removeItem(SAVED_PROJECT_KEY);
  } catch {
    /* Remembering the project is a convenience; storage can be unavailable. */
  }
}

/** A short name for the project: the server folder for server projects, else the folder name. */
export function projectName(
  root: string,
  cases: Pick<CaseEntry, 'run_host' | 'remote_directory'>[],
) {
  const base = (path: string) =>
    path
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() || path;
  const parent = (path: string) => path.replace(/\/+$/, '').replace(/\/[^/]*$/, '') || '/';
  const first = cases[0];
  if (first?.run_host && first.remote_directory?.startsWith('/'))
    return `${first.run_host}: ${base(parent(first.remote_directory))}`;
  return base(root);
}

/** The view that returns to the current project's working page. */
export function currentProjectView(
  state: Pick<AppState, 'root' | 'projectView'>,
  saved: SavedProject | null,
): NonNullable<AppState['projectView']> {
  const project = projectToSave(state, saved) ?? { root: state.root, page: 'cases' as const };
  return { ...project, home: false };
}
