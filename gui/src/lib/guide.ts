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
const pageContracts = () => {
  const contracts = [...pages.values()];
  const modal = [...contracts].reverse().find((page) => page.modal);
  return modal ? [modal] : contracts;
};
const fields = () =>
  pageContracts()
    .flatMap((page) => page.fields)
    .filter((field) => field.visible !== false);
const actions = () =>
  pageContracts()
    .flatMap((page) => page.actions)
    .filter((action) => action.visible !== false);
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
    fields: fields().map(({ id, label, value, type, options, disabled, readOnly, saves }) => ({
      id,
      name: id,
      label,
      value,
      kind: type,
      options,
      disabled: !!disabled,
      readOnly: !!readOnly,
      read_only: !!readOnly,
      saves: !!saves,
    })),
    buttons: actions().map(({ id, label, kind, enabled }) => ({
      name: id,
      label,
      kind,
      enabled: enabled !== false,
    })),
    status: contracts.map((page) => page.status).filter(Boolean),
    details: publicUiResult(contracts.map((page) => page.details).filter(Boolean)),
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
  if (pageContracts().some((page) => page.modal))
    throw new Error('请先关闭当前对话框，再切换页面 / Close the current dialog before navigating.');
  const target =
    pageAliases[step] ?? (routes.includes(step as Route) ? { route: step as Route } : null);
  if (!target) throw new Error(`${step} is not an available page; read ui_state`);
  if (!app.navigate(target.route, target.tab))
    throw new Error(
      '请先保存或放弃当前修改，再切换页面 / Save or discard current edits before navigating.',
    );
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
