import { bilingual } from './bilingual';
import { useLayoutEffect, useRef } from 'react';
import { useApp } from './app-state';
import type { AppState, Route } from './types';
import { invokeJson } from './ipc';
import { pageExecutionContext, publicUiResult, waitForContract } from './utils';
export type UiField = {
  id: string;
  label: string;
  value: string;
  type: 'text' | 'number' | 'choice' | 'switch';
  options?: { value: string; label: string }[];
  disabled?: boolean;
  readOnly?: boolean;
  /// 会随时间/轮询自己变化的值（例如候选列表里的默认成员）：审批快照不比较它，避免假阳性。
  volatile?: boolean;
  saves?: boolean;
  visible?: boolean;
  set: (value: string) => void | Promise<void>;
};
export type UiAction = {
  id: string;
  label: string;
  kind: 'click' | 'commit';
  enabled?: boolean;
  visible?: boolean;
  run: () => unknown | Promise<unknown>;
};
export type PageContract = {
  title: string;
  step: string;
  scope?: 'shell' | 'page';
  modal?: boolean;
  status?: string;
  kernelDir?: string | null;
  preprocessors?: string;
  fields: UiField[];
  actions: UiAction[];
  details?: unknown;
};
const pages = new Map<symbol, PageContract>();
let app: ReturnType<typeof useApp> | null = null;
let pending = false;
export function GuideBinding() {
  const value = useApp();
  useLayoutEffect(() => {
    app = value;
    return () => {
      if (app === value) app = null;
    };
  });
  return null;
}
export function usePageContract(contract: PageContract, active = true) {
  const owner = useRef(Symbol('page'));
  useLayoutEffect(() => {
    if (!active) return;
    const key = owner.current;
    pages.set(key, contract);
    return () => {
      pages.delete(key);
    };
  });
}
// 由按钮自己登记的动作（`ActionButton`）。原来每个页面在 PageContract 里另写一份动作清单，
// 助手调用的是清单里的回调，和界面上的按钮是两份：出过"助手能按、界面上没有"的按钮（第 737 轮）。
// 这里只认此刻真的显示着、并且不被打开的对话框挡住的按钮；能不能按看按钮自己的 disabled。
type RenderedAction = {
  action: Omit<UiAction, 'enabled' | 'visible'>;
  element: () => Element | null;
};
const renderedActions = new Map<symbol, RenderedAction>();
/** 登记一个按钮动作，返回注销函数。`useRenderedAction` 在按钮挂载期间调用它。 */
export function registerRenderedAction(
  action: RenderedAction['action'],
  element: RenderedAction['element'],
) {
  const key = Symbol('action');
  renderedActions.set(key, { action, element });
  return () => {
    renderedActions.delete(key);
  };
}
export function useRenderedAction(
  action: RenderedAction['action'] | null,
  ref: { current: Element | null },
) {
  useLayoutEffect(() => (action ? registerRenderedAction(action, () => ref.current) : undefined));
}
const shown = (element: Element) =>
  element.isConnected &&
  element.getClientRects().length > 0 &&
  !element.closest('[hidden], [aria-hidden="true"]');
/** 最上层打开着的对话框：有它时，只有它里面的按钮算数。 */
function topDialog(): Element | null {
  const query = globalThis.document?.querySelectorAll;
  if (typeof query !== 'function') return null;
  const dialogs = [
    ...query.call(globalThis.document, '[role="dialog"], [role="alertdialog"], dialog[open]'),
  ].filter(shown);
  return dialogs.at(-1) ?? null;
}
export function renderedButtons(): UiAction[] {
  if (!renderedActions.size) return [];
  const dialog = topDialog();
  return [...renderedActions.values()].flatMap(({ action, element }) => {
    const el = element();
    if (!el || !shown(el) || (dialog && !dialog.contains(el))) return [];
    return [{ ...action, enabled: !(el as HTMLButtonElement).disabled }];
  });
}
const pageContracts = () => {
  const contracts = [...pages.values()];
  const modal = [...contracts].reverse().find((page) => page.modal);
  return modal ? [modal] : contracts;
};
const fields = () =>
  pageContracts()
    .flatMap((page) => page.fields)
    .filter((field) => field.visible !== false);
const actions = () => [
  ...pageContracts()
    .flatMap((page) => page.actions)
    .filter((action) => action.visible !== false),
  ...renderedButtons(),
];
export function guideState() {
  const state = app?.state;
  const contracts = pageContracts();
  const page = contracts.find((contract) => contract.scope !== 'shell');
  const execution = pageExecutionContext(contracts);
  return {
    ...execution,
    screen: 'workbench',
    step: page?.step ?? state?.route ?? 'loading',
    title: page?.title ?? 'CoLM Desktop',
    busy: pending || state?.busy || false,
    case: state?.selectedCase
      ? { name: state.selectedCase.name, dir: state.selectedCase.dir }
      : null,
    model: state?.version
      ? {
          id: state.version.version.id,
          revision: state.version.version.revision,
          name: state.version.name,
          code: state.version.code,
          overlay_sha256: state.version.overlay_sha256,
        }
      : null,
    steps: contracts.some((contract) => contract.modal)
      ? []
      : [...routes, ...Object.keys(pageAliases)].map((id) => ({ id, label: id })),
    fields: fields().map(
      ({ id, label, value, type, options, disabled, readOnly, volatile, saves }) => ({
        id,
        name: id,
        label,
        value,
        kind: type,
        options,
        disabled: !!disabled,
        readOnly: !!readOnly,
        read_only: !!readOnly,
        volatile: !!volatile,
        saves: !!saves,
      }),
    ),
    buttons: actions().map(({ id, label, kind, enabled }) => ({
      name: id,
      label,
      kind,
      enabled: enabled !== false,
    })),
    status: contracts.map((page) => page.status).filter(Boolean),
    details: budgetUiDetails(publicUiResult(contracts.map((page) => page.details).filter(Boolean))),
  };
}
const routes: Route[] = [
  'project',
  'case',
  'new-case',
  'preparation',
  'versions',
  'resources',
  'history',
  'research',
  'compare',
  'settings',
  'help',
];
const pageAliases: Record<
  string,
  { route: Route; tab?: AppState['caseTab']; control?: string; value?: string }
> = {
  'basic-files': { route: 'case', tab: 'settings' },
  'result-overview': { route: 'case', tab: 'results', control: 'result-tab', value: 'overview' },
  'result-data': { route: 'case', tab: 'results', control: 'result-tab', value: 'catalog' },
  'result-series': { route: 'case', tab: 'results', control: 'result-tab', value: 'series' },
  'result-evaluation': {
    route: 'case',
    tab: 'results',
    control: 'result-tab',
    value: 'evaluation',
  },
  'result-comparison': {
    route: 'case',
    tab: 'results',
    control: 'result-tab',
    value: 'comparison',
  },
  'result-diagnostics': {
    route: 'case',
    tab: 'results',
    control: 'result-tab',
    value: 'diagnostics',
  },
  'result-export': { route: 'case', tab: 'results', control: 'result-tab', value: 'report' },
  'result-tuning': { route: 'research', control: 'research-tab', value: 'tuning' },
  'result-uncertainty': { route: 'research', control: 'research-tab', value: 'uncertainty' },
  'hybrid-learn': { route: 'research', control: 'research-tab', value: 'parameters' },
  'hybrid-process': { route: 'research', control: 'research-tab', value: 'process' },
  code: { route: 'versions' },
};
const rendered = () =>
  new Promise<void>((resolve) =>
    requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
  );
function one<T extends { id: string }>(list: T[], id: string) {
  const matches = list.filter((item) => item.id === id);
  if (matches.length !== 1) throw new Error(`${id} is not a unique visible control; read ui_state`);
  return matches[0];
}
function finishedState() {
  pending = false;
  return guideState();
}
export async function openPage(step: string) {
  if (!app) throw new Error('The interface is not ready.');
  // 页面里的对话框（服务器目录、参数、导入……）多数没有登记成 modal：开着时也不许切页面，免得助手把用户正在
  // 看的对话框连同页面一起丢掉（GUI 审查 A-04）。确认框另外登记成没有任何控件的 modal，见 ConfirmHost。
  const dialogOpen =
    typeof document !== 'undefined' &&
    typeof document.querySelectorAll === 'function' &&
    Array.from(document.querySelectorAll('[role="dialog"], [role="alertdialog"]') ?? []).some(
      (element) => element.getClientRects?.().length > 0,
    );
  if (dialogOpen || pageContracts().some((page) => page.modal))
    throw new Error(
      bilingual('请先关闭当前对话框，再切换页面', 'Close the current dialog before navigating.'),
    );
  const target =
    pageAliases[step] ?? (routes.includes(step as Route) ? { route: step as Route } : null);
  if (!target) throw new Error(`${step} is not an available page; read ui_state`);
  if (!app.navigate(target.route, target.tab))
    throw new Error(
      bilingual(
        '请先保存或放弃当前修改，再切换页面',
        'Save or discard current edits before navigating.',
      ),
    );
  // 批量运行页也在 project 路由下，只注册 project-batch-run：像侧栏"项目"一样回项目首页，
  // 否则一直等不到 project 页的契约，10 秒后超时（GUI 审查 2026-10-22，S05）。
  const view = app.state.projectView;
  if (target.route === 'project' && view && view.root === app.state.root && !view.home)
    app.update({ projectView: { ...view, home: true }, projectCasesOpen: false });
  await rendered();
  await waitForContract(
    () =>
      app?.state.route === target.route &&
      [...pages.values()].some((page) => page.scope !== 'shell' && page.step === target.route),
    rendered,
  );
  if (target.control && target.value) {
    const control = one(fields(), target.control);
    if (control.disabled || control.readOnly)
      throw new Error(`${control.id} cannot be changed here`);
    if (!control.options?.some((option) => option.value === target.value))
      throw new Error(`${step} is unavailable on the current page`);
    await control.set(target.value);
    await rendered();
  }
  const snapshot = guideState();
  return { step: snapshot.step, title: snapshot.title };
}
export async function drive(action: string, args: Record<string, unknown> = {}) {
  if (action === 'state') return guideState();
  if (pending) throw new Error('A UI operation is still in progress; wait for its result.');
  if (!app) throw new Error('The interface is not ready.');
  pending = true;
  try {
    if (action === 'workspace_remote_servers') {
      const config = await invokeJson<{
        servers: {
          host: string;
          root: string;
          scheduler: string;
          cpus?: number;
          memory_gb?: number;
        }[];
      }>('remote_config');
      return {
        servers: config.servers.map(({ host, root, scheduler, cpus, memory_gb }) => ({
          host,
          root,
          scheduler,
          cpus,
          memory_gb,
        })),
      };
    }
    if (action === 'workspace_remote')
      return await invokeJson('workspace_remote', {
        name: args.name,
        operation: args.operation,
        host: args.host ?? null,
        job: args.job ?? null,
        request: args.request ?? null,
      });
    if (action === 'go') {
      await openPage(String(args.step ?? ''));
      return finishedState();
    }
    if (action === 'fill' || action === 'set') {
      const requested = args.fields;
      if (!Array.isArray(requested)) throw new Error('fields must be an array');
      for (const raw of requested) {
        if (!raw || typeof raw !== 'object') throw new Error('invalid field');
        const field = raw as { field?: unknown; value?: unknown };
        const control = one(fields(), String(field.field ?? ''));
        if (control.disabled || control.readOnly)
          throw new Error(`${control.id} cannot be changed here`);
        if (action === 'fill' && control.saves)
          throw new Error(`${control.id} saves immediately; use ui_set`);
        if (action === 'set' && !control.saves)
          throw new Error(`${control.id} is a draft field; use ui_fill then ui_commit`);
        const value = String(field.value ?? '');
        if (control.options && !control.options.some((option) => option.value === value))
          throw new Error(`${control.id}: unsupported option`);
        if (control.type === 'switch' && !['true', 'false'].includes(value))
          throw new Error(`${control.id}: boolean value required`);
        await control.set(value);
        await rendered();
      }
      return finishedState();
    }
    if (action === 'click' || action === 'commit') {
      const control = one(actions(), String(args.target ?? ''));
      if (control.enabled === false) throw new Error(`${control.id} is disabled here`);
      if (control.kind !== action) throw new Error(`${control.id} requires ui_${control.kind}`);
      const result = await control.run();
      await rendered();
      return { result: publicUiResult(result), state: finishedState() };
    }
    throw new Error(`${action} is not supported by this interface`);
  } finally {
    pending = false;
  }
}

/**
 * 页面细节的总预算：`publicUiResult` 只截单个数组和字符串，对象没有总量上限。一个合法上限的研究
 * （1000 候选、1000 任务）实测 469 KiB，每条助手消息都带上（GUI 审查 D-03）。超了就从最大的条目起
 * 换成一句"省略了多少字符"，直到放得下。
 */
// 放在这里而不是 utils.ts：几个测试把 guide.ts 单独转译、手工注入它的导入，放在本文件就不用逐个补。
export function budgetUiDetails(details: unknown, limit = 48_000): unknown {
  const size = (value: unknown) => JSON.stringify(value)?.length ?? 0;
  if (size(details) <= limit || !Array.isArray(details)) return details;
  const pages = details.map((page) =>
    page && typeof page === 'object' && !Array.isArray(page)
      ? { ...(page as Record<string, unknown>) }
      : page,
  );
  const entries = pages
    .flatMap((page, index) =>
      page && typeof page === 'object' && !Array.isArray(page)
        ? Object.entries(page).map(([key, value]) => ({ index, key, length: size(value) }))
        : [],
    )
    .sort((a, b) => b.length - a.length);
  for (const entry of entries) {
    (pages[entry.index] as Record<string, unknown>)[entry.key] = {
      omitted: `${entry.length} characters over the assistant context budget`,
    };
    if (size(pages) <= limit) break;
  }
  return pages;
}
