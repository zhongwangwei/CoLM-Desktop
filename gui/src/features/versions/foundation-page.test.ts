import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as model from './model';
import * as setup from './scientific-setup';
import * as presentation from '../../lib/param-presentation';
import { bilingual } from '../../lib/bilingual';
import { researchVersion } from './research-version';
import { homeResearchEntry, hubMethods, hubResearchEntry, studyResearchEntry } from './hub-methods';
import { appLocale, historyKindLabel, historyStatusLabel } from '../history/history';
import { schemeVersionName } from './version-tree';
import { isContentConflict, workspaceErrorText } from './workspace-errors';
import { sourceGuardReason } from './source-edits';
import type { ParameterOverride } from '../../lib/types';
import { compatibleKernel } from '../runs/run-model';
import { effectiveModelFields } from '../cases/case-model';
import type { UiAction, UiField } from '../../lib/guide';
import { SaveBadge } from '../../components/SaveBadge';
import { saveStateText } from '../../lib/save-status';

// StudyPicks 用 localStorage 记住选过的研究参数；node 里没有，给个空的。
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null, setItem: () => {} },
});

type Element = { type: unknown; props: Record<string, unknown>; children: unknown[] };
type Contract = {
  title?: string;
  step?: string;
  fields: UiField[];
  actions: UiAction[];
  status?: string;
};
type Store = {
  slots: unknown[];
  cursor: number;
  effectCursor: number;
  registered: Set<number>;
  deps: Map<number, unknown[] | undefined>;
  cleanups: (void | (() => void))[];
};

/**
 * 版本页的测试框架。第 737 轮起动作由按钮自己登记（`ActionButton`、`Dialog` 的 closeAction），
 * 所以这里把页面源码和它渲染的子组件源码都拿出来执行，得到一个元素树，再从树里收集按钮：
 * 只算此刻显示着的（隐藏页签、收起的折叠项不算），有打开的对话框时只算对话框里的。
 * 子组件按函数名编译成一个微型渲染器（每个组件实例有自己的 hook 槽位），这样版本库、参数对话框、
 * 代码文件面板里的按钮也都是它们自己登记的，测试不用再重写一遍动作清单。
 */
function page(
  subgrid?: 'LCT' | 'PFT' | 'PC',
  physics: Record<string, string> = {},
  parameters: ParameterOverride[] = [],
  codeRef: Record<string, unknown> = { kind: 'official' },
  // 研究导入的产物类型与资产目录：网络/联合导入会把网络文件暂存到新目录（审计 P1-5）。
  studyImport: {
    kind?: 'parameters' | 'network' | 'joint';
    network?: Record<string, unknown> | null;
    assetRoot?: string;
  } = {},
) {
  const version = {
    name: 'Official',
    note: '',
    version: { id: 'official', revision: 1 },
    code: codeRef,
    overlay: { physics, parameters, process_files: [], hybrid: {} },
    applicability: { sites: [], land_classes: [] },
  };
  let failedCommand = '';
  // 被 hold 的命令与它的放行者（U-05：验证"正在保存"三态）。
  let heldCommand: string | null = null;
  let heldResolver: (() => void) | null = null;
  let draft = { ...version, id: 'custom', generation: 1 };
  let folderPick: string | null = null;
  async function respondJson(command: string, args: Record<string, unknown>): Promise<unknown> {
    switch (command) {
      case 'model_create':
        return draft;
      case 'model_save_draft':
        draft = args.draft as typeof draft;
        return { ...draft, generation: draft.generation + 1 };
      case 'model_freeze':
        return { ...draft, version: { id: draft.id, revision: 2 } };
      case 'model_configuration':
        return {
          fields: [],
          process_files: [],
          pft_parameters: [],
          warnings: [],
          context_complete: false,
        };
      case 'parameter_catalog':
        return [];
      case 'model_fields':
        return setup.setupHandledKeys.map((name) => ({
          name,
          kind: name === 'DEF_TUNING_CROP_PLANTING_DAY' ? 'integer' : 'logical',
          values: [],
          section: '',
          derived: false,
          default: '.false.',
        }));
      case 'model_drafts':
        return [draft];
      case 'history_list':
        return { items: [] };
      case 'model_draft':
        return draft;
      case 'model_freeze_check':
        return {
          ready: false,
          items: [
            { code: 'clean', ok: true, blocking: false, message: '源码干净' },
            {
              code: 'engine_built',
              ok: false,
              blocking: true,
              message: '先在当前提交上编译 Rust 引擎',
            },
          ],
        };
      case 'model_usage':
        return state.usage;
      case 'model_code_files':
      case 'workspace_files':
        return { entries: [] };
      case 'model_code_file':
      case 'workspace_file':
        return { path: '', size: 0, text: null };
      case 'parameter_type_defaults':
        return [];
      case 'standard_process_parameters':
        return [];
      case 'workspace_status':
        return { changed_files: [] };
      case 'workspace_check':
        return {};
      case 'model_delete_draft':
      case 'model_branch_code':
      case 'model_refresh_code':
        return {};
      case 'pick_folder':
        return folderPick;
      case 'model_import_study':
        if (args.preview)
          return {
            study: '/study',
            member: 'm000001',
            objective: 1.25,
            records: parameters,
            kind: studyImport.kind,
            network: studyImport.network ?? null,
          };
        return {
          draft,
          member: 'm000001',
          parameters: (args.only as unknown[]).length,
          kind: studyImport.kind,
          asset_root: studyImport.assetRoot ?? null,
          network: studyImport.network ?? null,
        };
      default:
        throw Error(command);
    }
  }
  const notices: string[] = [];
  const calls: { command: string; args: Record<string, unknown> }[] = [],
    scrolls: unknown[] = [],
    routes: string[] = [];
  const state = {
    language: 'zh',
    version,
    versionRef: version.version,
    versions: [version],
    // 模拟明确选过的版本（U-01：研究基线不得写它）。
    chosenVersion: null as null | { id: string; revision: number },
    // 研究入口状态（U-01/U-02）：从"新建修改版"带进研究页的基线等。
    researchBaseline: null as null | { id: string; revision: number },
    researchMethod: null as string | null,
    researchSeed: null as null | { parameters: unknown[] },
    researchTarget: null as null | { draftId: string; name: string },
    simulationReturn: (subgrid ? { route: 'new-case', step: 2, subgrid } : null) as {
      route: 'new-case' | 'case';
      step: 2;
      subgrid?: 'LCT' | 'PFT' | 'PC';
      setup?: setup.SetupRequest;
    } | null,
    newCaseSeed: undefined as { form: Record<string, string> } | undefined,
    selectedCase: undefined as { entry: { dir: string }; profile: { engine: string } } | undefined,
    expert: false,
    kernels: [],
    versionsView: undefined as 'library' | 'create' | undefined,
    usage: [] as {
      version: { id: string; revision: number };
      cases: number;
      history: number;
      blocker: string | null;
    }[],
  };
  let guard: (() => string | null) | undefined;
  const stores = new Map<string, Store>();
  let active: Store | null = null;
  const pending: (() => void)[] = [];
  const contracts: Contract[] = [];
  const elements: Element[] = [];
  const components = new Map<string, (props: Record<string, unknown>) => unknown>();
  let tree: unknown = null;
  function storeFor(key: string) {
    let value = stores.get(key);
    if (!value) {
      value = {
        slots: [],
        cursor: 0,
        effectCursor: 0,
        registered: new Set(),
        deps: new Map(),
        cleanups: [],
      };
      stores.set(key, value);
    }
    return value;
  }
  const useState = (initial: unknown) => {
    const store = active!;
    const key = store.cursor++;
    if (!(key in store.slots))
      store.slots[key] = typeof initial === 'function' ? (initial as () => unknown)() : initial;
    return [
      store.slots[key],
      (value: unknown) => {
        store.slots[key] =
          typeof value === 'function'
            ? (value as (current: unknown) => unknown)(store.slots[key])
            : value;
      },
    ];
  };
  const useRef = (initial: unknown) => {
    const store = active!;
    const key = store.cursor++;
    if (!(key in store.slots)) store.slots[key] = { current: initial };
    return store.slots[key];
  };
  const useEffect = (fn: () => void | (() => void), deps?: unknown[]) => {
    const store = active!;
    const key = store.effectCursor++;
    let changed = !store.registered.has(key);
    if (!changed && deps) {
      const previous = store.deps.get(key);
      changed = !previous || deps.some((value, index) => !Object.is(value, previous[index]));
    }
    if (!changed) return;
    store.registered.add(key);
    store.deps.set(key, deps);
    pending.push(() => {
      store.cleanups[key]?.();
      store.cleanups[key] = fn() ?? undefined;
    });
  };
  const useMemo = (fn: () => unknown) => fn();
  function update(patch: Record<string, unknown>) {
    Object.assign(state, patch);
  }
  const context: Record<string, unknown> = {
    ...model,
    ...setup,
    ...presentation,
    bilingual,
    historyKindLabel,
    historyStatusLabel,
    researchVersion,
    hubMethods,
    hubResearchEntry,
    homeResearchEntry,
    studyResearchEntry,
    appLocale,
    schemeVersionName,
    compatibleKernel,
    effectiveModelFields,
    IGBP: [],
    USGS: [],
    PFT: [],
    useState,
    useRef,
    useEffect,
    useMemo,
    React: {
      // 动作按钮按它渲染出的样子收：普通按钮，onClick 就是 run，没写子节点时显示 label。
      createElement: (
        type: unknown,
        props: Record<string, unknown> | null,
        ...children: unknown[]
      ) => {
        const merged = props ?? {};
        if (type === 'ActionButton')
          return {
            type: 'Button',
            props: { ...merged, onClick: merged.run },
            children: children.length ? children : [String(merged.label)],
          };
        return { type, props: merged, children };
      },
      Fragment: 'Fragment',
    },
    ...Object.fromEntries(
      [
        'ActionButton',
        'Advanced',
        'Button',
        'Card',
        'ConfigurationOptions',
        'Dialog',
        'Field',
        'Input',
        'Notice',
        'ScientificSetupCards',
        'Select',
        'VersionChanges',
      ].map((name) => [name, name]),
    ),
    document: {
      querySelector: (selector: string) => {
        assert.equal(selector, '.main-pane');
        return { scrollTo: (position: unknown) => scrolls.push(position) };
      },
    },
    useApp: () => ({
      state,
      notify: (message: string) => notices.push(message),
      update,
      // Like the app: a page's leave guard can refuse the navigation.
      navigate: (route: string) => {
        if (guard?.()) return false;
        routes.push(route);
        return true;
      },
      refreshVersions: async () => {},
    }),
    useNavigationGuard: (next: () => string | null) => {
      guard = next;
    },
    // 组件在真实界面里用这个 hook 登记动作；这个框架直接从元素树里收集，所以空实现。
    useRenderedAction: () => {},
    usePageContract: (value: Contract) => {
      contracts.push({
        ...value,
        get actions() {
          return [...value.actions, ...visibleActions()];
        },
      });
    },
    invokeJson: async (command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args });
      if (command === failedCommand) throw new Error(command + ' failed');
      // U-05 要验证"正在保存"这一态：hold 的命令先把结果算出来（含草稿的副作用），
      // 再挂住直到用例 releaseHeld()，页面就能在请求回来的这段时间里显示 saving。
      if (command === heldCommand) {
        const result = await respondJson(command, args);
        return await new Promise((resolve) => {
          heldResolver = () => resolve(result);
        });
      }
      return respondJson(command, args);
    },
    invoke: async (command: string) => {
      calls.push({ command, args: {} });
      if (command === 'pick_file') return null;
      if (command === 'pick_folder') return folderPick;
      throw new Error(command);
    },
    askConfirm: async () => false,
    workspaceErrorText,
    isContentConflict,
    sourceGuardReason,
    // U-05：页面用 Lead 提供的共享三态徽标与措辞；这里用真实实现，测试检查它收到的 state。
    SaveBadge,
    saveStateText,
    // 真实实现是 Radix 对话框（只在打开时挂载、按钮由真实 React 渲染）。这个假框架只验证页面流程，
    // 未保存源码时的三选一在 code-files.test / source-edits.test 里单独证明；这里让 ask 直接放行，
    // 保持已有页面行为用例不变。
    useSourceChoicePrompt: () => ({ ask: async () => true, dialog: null }),
    download: async () => {},
  };
  function compile(name: string, exported: string[] = []) {
    const source = readFileSync(new URL(`./${name}.tsx`, import.meta.url), 'utf8')
      .replace(/^import\b[\s\S]*?;$/gm, '')
      .replace('export default function', 'function')
      .replace(/^export /gm, '');
    const code = ts.transpileModule(source, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
    }).outputText;
    const body = `${code}\nreturn { default: ${name}${exported
      .map((key) => `, ${key}`)
      .join('')} };`;
    const factory = new Function(...Object.keys(context), body) as (
      ...args: unknown[]
    ) => Record<string, unknown>;
    const module = factory(...Object.values(context));
    components.set(name, module.default as (props: Record<string, unknown>) => unknown);
    for (const key of exported) context[key] = module[key];
  }
  // 依赖顺序：被引用的组件先编译，编译后把它的存在回填给后面的组件源码。
  compile('ParameterDialog', ['literalNumber', 'valueProblem', 'docOf', 'rangeText', 'typesOf']);
  context.ParameterDialog = 'ParameterDialog';
  compile('CodeFiles');
  context.CodeFiles = 'CodeFiles';
  compile('StudyImportDialog', ['chosenKey']);
  context.StudyImportDialog = 'StudyImportDialog';
  compile('VersionLibrary', ['changeSummary']);
  context.VersionLibrary = 'VersionLibrary';
  compile('VersionHub');
  context.VersionHub = 'VersionHub';
  compile('StudyPicks');
  context.StudyPicks = 'StudyPicks';
  compile('CodeWorkspace');
  context.CodeWorkspace = 'CodeWorkspace';
  compile('VersionsPage');
  const Component = components.get('VersionsPage')!;

  function renderComponent(
    component: (props: Record<string, unknown>) => unknown,
    props: Record<string, unknown>,
    key: string,
  ) {
    const store = storeFor(key);
    store.cursor = 0;
    store.effectCursor = 0;
    const previous = active;
    active = store;
    try {
      return component(props);
    } finally {
      active = previous;
    }
  }
  function materialize(node: unknown, key: string): unknown {
    if (Array.isArray(node))
      return node.map((child, index) => materialize(child, `${key}.${index}`));
    if (!node || typeof node !== 'object') return node;
    const element = node as Element;
    if (typeof element.type === 'string' && components.has(element.type)) {
      const rendered = renderComponent(
        components.get(element.type)!,
        element.props,
        `${key}:${element.type}`,
      );
      return {
        type: element.type,
        props: element.props,
        children: [materialize(rendered, `${key}:${element.type}.out`)],
      };
    }
    // 关着的对话框在真实界面里不渲染里面的东西（Radix 只在打开时挂载内容）。
    if (element.type === 'Dialog' && !element.props.open)
      return { type: 'Dialog', props: element.props, children: [] };
    return {
      type: element.type,
      props: element.props,
      children: element.children.map((child, index) => materialize(child, `${key}.${index}`)),
    };
  }
  function flatten(node: unknown, out: Element[] = []): Element[] {
    if (Array.isArray(node)) {
      node.forEach((child) => flatten(child, out));
      return out;
    }
    if (node && typeof node === 'object') {
      const element = node as Element;
      out.push(element);
      flatten(element.children, out);
    }
    return out;
  }
  function eachElement(node: unknown, visit: (element: Element) => void) {
    if (Array.isArray(node)) {
      node.forEach((child) => eachElement(child, visit));
      return;
    }
    if (!node || typeof node !== 'object') return;
    const element = node as Element;
    visit(element);
    eachElement(element.children, visit);
  }
  function topDialog() {
    const dialogs: Element[] = [];
    eachElement(tree, (element) => {
      if (element.type === 'Dialog' && element.props.open) dialogs.push(element);
    });
    return dialogs.at(-1) ?? null;
  }
  function collectActions(node: unknown, out: UiAction[]) {
    if (Array.isArray(node)) {
      node.forEach((child) => collectActions(child, out));
      return;
    }
    if (!node || typeof node !== 'object') return;
    const element = node as Element;
    const props = element.props;
    // 隐藏的页签、收起的折叠项，界面上看不到里面的按钮。
    if (element.type === 'div' && props.hidden) return;
    if (element.type === 'Advanced') {
      if (typeof props.action === 'string' && props.action)
        out.push({
          id: props.action,
          label: String(props.actionLabel ?? props.title ?? props.action),
          kind: 'click',
          run: () => {},
        });
      if (props.open !== true) return;
    }
    if (element.type === 'details') {
      if (props.open !== true) return;
    }
    if (element.type === 'Dialog' && props.open && props.closeAction && props.showClose !== false)
      out.push({
        id: String(props.closeAction),
        label: 'close',
        kind: 'click',
        run: () => (props.onOpenChange as (open: boolean) => void)(false),
      });
    if (typeof props.action === 'string' && props.action)
      out.push({
        id: props.action,
        label: String(props.label ?? props.action),
        kind: (props.kind as UiAction['kind']) ?? 'click',
        enabled: !props.disabled,
        run: props.run as UiAction['run'],
      });
    collectActions(element.children, out);
  }
  function visibleActions() {
    const out: UiAction[] = [];
    collectActions(topDialog() ?? tree, out);
    return out;
  }
  function render() {
    contracts.length = 0;
    const root = renderComponent(Component, {}, 'page');
    tree = materialize(root, 'tree');
    elements.length = 0;
    elements.push(...flatten(tree));
    return contracts[0]!;
  }
  function flush() {
    while (pending.length) pending.splice(0).forEach((effect) => effect());
  }
  async function settle() {
    for (let round = 0; round < 8; round++) {
      flush();
      for (let index = 0; index < 24; index++) await Promise.resolve();
      render();
      if (!pending.length) break;
    }
    return contracts[0]!;
  }
  async function click(id: string) {
    const action = contracts[0]?.actions.find((item) => item.id === id);
    assert.ok(action, `缺少可见动作 ${id}`);
    await action.run();
  }
  // 版本页先显示"新建修改版"；这些测试要的是某个版本的工作台，所以像用户一样点进版本库再打开它。
  let opened = false;
  async function mount(open = true) {
    render();
    await settle();
    if (open && !opened) {
      opened = true;
      await click('version-library');
      await settle();
      await click(`version-select-${version.version.id}-${version.version.revision}`);
      await settle();
    }
    await settle();
    return contracts[0]!;
  }
  async function mountHub() {
    render();
    return settle();
  }
  async function openDraft(id: string) {
    const find = () => contracts[0]?.actions.find((item) => item.id === `version-draft-${id}`);
    if (!find()) {
      const back = contracts[0]!.actions.find((item) => item.id === 'version-back');
      if (back) {
        await back.run();
        await settle();
      }
    }
    if (!find()) {
      const library = contracts[0]!.actions.find((item) => item.id === 'version-library');
      if (library) {
        await library.run();
        await settle();
      }
    }
    const action = find();
    assert.ok(action, `缺少草稿按钮 version-draft-${id}`);
    await action.run();
    await settle();
  }
  return {
    render,
    mount,
    mountHub,
    openDraft,
    click,
    settle,
    elements,
    state,
    calls,
    notices,
    routes,
    version,
    scrolls,
    pickFolder: (value: string | null) => {
      folderPick = value;
    },
    fail: (command: string) => {
      failedCommand = command;
    },
    // 让一条命令挂住不返回，直到 releaseHeld：页面在这段时间里显示"正在保存"（U-05）。
    hold: (command: string) => {
      heldCommand = command;
      heldResolver = null;
    },
    releaseHeld: () => {
      const resolve = heldResolver;
      heldCommand = null;
      heldResolver = null;
      resolve?.();
    },
    actions: () => visibleActions(),
  };
}

/** 页面上"成功"类的提示（研究导入结果写在这里，不是转瞬即逝的通知）。 */
function successText(p: ReturnType<typeof page>): string {
  return p.elements
    .filter((element) => element.type === 'Notice' && element.props.tone === 'success')
    .map((element) => {
      const out: string[] = [];
      const walk = (node: unknown) => {
        if (typeof node === 'string') out.push(node);
        else if (Array.isArray(node)) node.forEach(walk);
        else if (node && typeof node === 'object') walk((node as Element).children);
      };
      walk(element.children);
      return out.join('');
    })
    .join('\n');
}

test('legacy return context cannot create a vegetation revision', async () => {
  for (const mode of ['PFT', 'PC', 'LCT'] as const) {
    const p = page(mode);
    const c = await p.mount();
    assert.equal(
      c.actions.some((action) => action.id === 'version-apply-simulation-structure'),
      false,
    );
    assert.equal(
      p.elements.some((e) => e.type === 'ScientificSetupCards'),
      false,
    );
    assert.ok(c.actions.find((action) => action.id === 'version-return-simulation')?.enabled);
    assert.ok(!p.calls.some((call) => call.command === 'model_create'));
  }
});
test('saved drafts can return while unsaved model edits keep the return button locked', async () => {
  const p = page('PFT');
  await p.mount();
  await p.openDraft('custom');
  let c = p.render();
  assert.equal(
    c.actions.find((action) => action.id === 'version-return-simulation')?.enabled,
    true,
  );
  c.fields.find((field) => field.id === 'draft-name')!.set('changed draft');
  c = p.render();
  assert.equal(
    c.actions.find((action) => action.id === 'version-return-simulation')?.enabled,
    false,
  );
  await c.actions.find((action) => action.id === 'version-save')!.run();
  c = await p.settle();
  assert.equal(
    c.actions.find((action) => action.id === 'version-return-simulation')?.enabled,
    true,
  );
  await c.actions.find((action) => action.id === 'version-return-simulation')!.run();
  assert.deepEqual(p.routes, ['new-case']);
  assert.equal(p.state.versionRef.id, 'official');
  assert.ok(!p.calls.some((call) => call.command === 'model_freeze'));
});

test('tracer and sediment parameters live in the parameter dialog, not a separate block', async () => {
  const p = page('PFT');
  await p.mount();
  assert.ok(!p.elements.some((e) => e.type === 'ConfigurationOptions'));
  // A frozen version opens read-only: no importing files into it.
  const dialog = () => p.elements.find((e) => e.type === 'ParameterDialog')!.props;
  assert.equal(dialog().onImportProcessFile, undefined);
  await p.openDraft('custom');
  assert.equal(typeof dialog().onImportProcessFile, 'function');
});

test('the source-code method starts a draft with its own copy of the parent code', async () => {
  const p = page('PFT');
  const c = await p.mountHub();
  assert.ok(c.actions.find((action) => action.id === 'version-method-source'));
  await c.actions.find((action) => action.id === 'version-method-source')!.run();
  await p.settle();
  const call = p.calls.find((entry) => entry.command === 'model_create')!;
  assert.equal(call.args.branchCode, true);
  assert.equal(call.args.workspace, null);
  assert.match(String(call.args.name), /^修改源码 /);
  // 手动改参数同样是写盘动作：直接建草稿并打开参数表。
  const q = page('PFT');
  const d = await q.mountHub();
  await d.actions.find((action) => action.id === 'version-method-manual')!.run();
  await q.settle();
  assert.equal(q.calls.filter((entry) => entry.command === 'model_create').length, 1);
  assert.equal(q.elements.find((e) => e.type === 'ParameterDialog')!.props.open, true);
});

test('a draft on its own code shows what blocks freezing and cannot be frozen until it is fixed', async () => {
  const p = page(undefined, {}, [], { kind: 'workspace', workspace: 'ver-1', commit: 'abc' });
  await p.mount();
  await p.openDraft('custom');
  await p.mount();
  assert.ok(p.calls.some((call) => call.command === 'model_freeze_check'));
  assert.equal(
    (await p.settle()).actions.find((action) => action.id === 'version-freeze')?.enabled,
    false,
  );
  const texts = p.elements.flatMap((e) => e.children.filter((x) => typeof x === 'string'));
  assert.ok(texts.includes('先在当前提交上编译 Rust 引擎'));
});

test('the version page shows one version in tabs that hide panels without unmounting them', async () => {
  const p = page('PFT');
  await p.mount();
  const tabs = p.elements.filter((e) => e.props.role === 'tab' && e.type === 'Button');
  // A frozen version: "what changed" instead of an editing page; no usage tab while nothing used it.
  assert.deepEqual(
    tabs.map((e) => e.children.flat().join('')),
    ['改了什么', '代码', '研究这个版本'],
  );
  assert.equal(tabs.filter((e) => e.props['aria-selected']).length, 1, 'one tab is current');
  const panels = p.elements.filter(
    (e) => e.type === 'div' && e.props.className === 'versions-panel',
  );
  assert.equal(panels.length, 4);
  assert.equal(panels.filter((e) => !e.props.hidden).length, 1, 'only the current panel shows');
  // Hidden panels still render: their page contracts and unsaved edits stay.
  assert.ok(p.elements.some((e) => e.type === 'CodeWorkspace'));
  assert.ok(p.elements.some((e) => e.type === 'ParameterDialog'));
  assert.ok(!p.elements.some((e) => e.type === 'VersionLibrary'), 'the library is its own page');
});

test('新建修改版 offers six ways and starts each without a confirmation form', async () => {
  const p = page();
  const c = await p.mountHub();
  const ids = c.actions
    .filter((action) => action.id.startsWith('version-method-'))
    .map((a) => a.id);
  assert.deepEqual(ids, [
    'version-method-manual',
    'version-method-tuning',
    'version-method-uncertainty',
    'version-method-parameters',
    'version-method-process',
    'version-method-source',
  ]);
  // By hand: a draft is created straight away, named automatically, and the parameter dialog opens.
  await c.actions.find((action) => action.id === 'version-method-manual')!.run();
  await p.settle();
  const create = p.calls.find((call) => call.command === 'model_create')!;
  assert.equal(create.args.branchCode, false);
  assert.match(String(create.args.name), /^手动修改参数 /);
  assert.ok(String(create.args.note).includes('基于 Official r1'));
  const dialog = p.elements.find((e) => e.type === 'ParameterDialog')!;
  assert.equal(dialog.props.open, true);
  // A study method goes to the research page with that method chosen.
  const q = page();
  const d = await q.mountHub();
  await d.actions.find((action) => action.id === 'version-method-tuning')!.run();
  assert.equal(q.routes.at(-1), 'research');
  assert.equal((q.state as { researchMethod?: string }).researchMethod, 'tuning');
});

test('the research home starts 修改参数（手动） and 修改源码 here, once per click', async () => {
  const p = page();
  await p.mount();
  const creates = () => p.calls.filter((call) => call.command === 'model_create');
  const manual = { method: 'manual' as const };
  Object.assign(p.state, { versionMethod: manual });
  await p.mount();
  assert.equal(creates().length, 1);
  assert.equal(creates()[0]!.args.branchCode, false);
  assert.equal((p.state as { versionMethod?: unknown }).versionMethod, null);
  assert.equal(p.elements.find((e) => e.type === 'ParameterDialog')!.props.open, true);
  // Development mode runs the effect twice with the same request: no second, empty draft.
  Object.assign(p.state, { versionMethod: manual });
  await p.mount();
  assert.equal(creates().length, 1);
  // Back goes where the user came from: research home, not 新建修改版.
  const back = p.elements.find(
    (e) => e.type === 'Button' && e.children.flat().includes('研究首页'),
  )!;
  (back.props.onClick as () => void)();
  await p.settle();
  assert.equal(p.routes.at(-1), 'research');
  Object.assign(p.state, { versionMethod: { method: 'source' } });
  await p.mount();
  assert.equal(creates().length, 2);
  assert.equal(creates()[1]!.args.branchCode, true);
});

test('closing the parameter table of a fresh hand-edit draft without changes cancels it', async () => {
  const dialog = (p: ReturnType<typeof page>) =>
    p.elements.find((e) => e.type === 'ParameterDialog')!.props as {
      open: boolean;
      onOpenChange: (open: boolean) => void;
      onChange: (next: unknown) => void;
      overlay: Record<string, unknown>;
    };
  const deletes = (p: ReturnType<typeof page>) =>
    p.calls.filter((call) => call.command === 'model_delete_draft').length;
  // From research home: back to research home, and the empty draft is gone.
  const r = page();
  await r.mount();
  Object.assign(r.state, { versionMethod: { method: 'manual' } });
  await r.mount();
  dialog(r).onOpenChange(false);
  await r.settle();
  assert.equal(deletes(r), 1);
  assert.equal(r.routes.at(-1), 'research');
  // From 新建修改版: back to the six ways on the same page.
  const h = page();
  const c = await h.mountHub();
  await c.actions.find((action) => action.id === 'version-method-manual')!.run();
  await h.settle();
  dialog(h).onOpenChange(false);
  await h.settle();
  assert.equal(deletes(h), 1);
  assert.deepEqual(h.routes, []);
  assert.ok(h.elements.some((el) => el.type === 'VersionHub'));
  // Something was changed: closing keeps the draft open for saving and freezing.
  const e = page();
  const d = await e.mountHub();
  await d.actions.find((action) => action.id === 'version-method-manual')!.run();
  await e.settle();
  dialog(e).onChange({ ...dialog(e).overlay, physics: { DEF_USE_PLANTHYDRAULICS: '.true.' } });
  e.render();
  dialog(e).onOpenChange(false);
  await e.settle();
  assert.equal(deletes(e), 0);
  assert.ok(!e.elements.some((el) => el.type === 'VersionHub'));
  // Opening the table again from the draft and closing it leaves the draft alone as well.
  dialog(e).onOpenChange(true);
  dialog(e).onOpenChange(false);
  await e.settle();
  assert.equal(deletes(e), 0);
});

test('a draft offers importing a study only on its research tab', async () => {
  const p = page();
  await p.mount();
  await p.openDraft('custom');
  const imports = p.elements.filter(
    (e) => e.type === 'Button' && e.children.flat().includes('从研究导入最优参数…'),
  );
  assert.equal(imports.length, 1);
});

test('frozen versions keep their next steps in the bottom bar, and freezing says what comes next', async () => {
  const p = page();
  const c = await p.mount();
  const bar = () =>
    p.elements.some((e) => e.type === 'div' && e.props.className === 'versions-actionbar');
  const button = (text: string) =>
    p.elements.find((e) => e.type === 'Button' && e.children.flat().includes(text));
  // The official version in use: derive from it at the bottom, nothing to freeze.
  assert.ok(bar());
  assert.ok(button('基于它新建修改版'));
  assert.ok(!button('创建模型修订（冻结）'));
  await c.actions.find((action) => action.id === 'version-derive')!.run();
  await p.settle();
  assert.ok(p.state.versionsView === 'create', 'deriving opens the six ways');
  const q = page();
  await q.mount();
  await q.openDraft('custom');
  await q.mount();
  const freeze = q.elements.find(
    (e) => e.type === 'Button' && e.children.flat().includes('创建模型修订（冻结）'),
  )!;
  (freeze.props.onClick as () => void)();
  await q.settle();
  assert.match(
    q.notices.at(-1) ?? '',
    /已创建模型修订.*模拟已改用这个修订/,
    '普通草稿冻结沿用既有语义：新修订就是模拟接着用的版本',
  );
});

test('the version library registers open, use and draft buttons but never deletion', async () => {
  const p = page();
  const modified = {
    ...p.version,
    name: 'Modified',
    version: { id: 'modified', revision: 1 },
    parent: p.version.version,
    created_unix: 0,
  };
  p.state.versions.push(modified);
  // 修改版没有算例/研究绑定，界面上会显示"删除"；助手不应该看到这个动作。
  p.state.usage = [
    { version: { id: 'modified', revision: 1 }, cases: 0, history: 0, blocker: null },
  ];
  await p.mount();
  await p.click('version-back');
  const c = await p.settle();
  const ids = c.actions.map((action) => action.id);
  assert.ok(ids.includes('version-new'), '新建修改版');
  assert.ok(ids.includes('version-select-modified-1'), '点版本名打开');
  assert.ok(ids.includes('version-open-modified-1'), '行里的"打开"');
  assert.ok(ids.includes('version-use-modified-1'), '设为运行模型');
  const use = c.actions.find((action) => action.id === 'version-use-modified-1')!;
  assert.equal(use.label, '设为运行模型', 'U-05：动作名明确说明它改的是模拟运行模型');
  assert.ok(ids.includes('version-draft-custom'), '草稿行');
  assert.ok(ids.includes('version-resume-custom'), '继续编辑');
  // 界面上确实有删除（修改版可删），但它是 D 级动作，不登记。
  assert.ok(
    p.elements.some((e) => e.type === 'button' && e.children.flat().includes('删除')),
    '删除按钮在界面上',
  );
  assert.equal(
    ids.some((id) => /delete|remove|cleanup/i.test(id)),
    false,
    '删除版本、草稿与清理旧官方版本属于 D 级，助手看不到',
  );
  assert.equal(new Set(ids).size, ids.length, '同时显示的动作 id 不能重复');
});

test('an open parameter dialog hides the page behind it and registers its own controls', async () => {
  const p = page();
  await p.mount();
  await p.openDraft('custom');
  const c = await p.settle();
  await c.actions.find((action) => action.id === 'version-parameters')!.run();
  await p.settle();
  const ids = p.actions().map((action) => action.id);
  assert.ok(ids.includes('version-parameters-close'));
  assert.ok(ids.includes('version-parameters-tab-global'));
  assert.ok(ids.includes('version-parameters-tab-process'));
  assert.equal(ids.includes('version-save'), false, '对话框打开时页面底部的保存按钮被挡住');
  assert.equal(new Set(ids).size, ids.length);
  // 关着的时候这些都不存在；底部动作重新可见。
  await p.settle();
  const dialog = p.elements.find((e) => e.type === 'ParameterDialog')!;
  (dialog.props.onOpenChange as (open: boolean) => void)(false);
  const closed = await p.settle();
  const visible = closed.actions.map((action) => action.id);
  assert.equal(
    visible.some((id) => id.startsWith('version-parameters-tab-')),
    false,
  );
  assert.ok(visible.includes('version-save'));
});

test('importing study parameters first previews them in a dialog the assistant cannot skip', async () => {
  const record = {
    parameter_id: 'case:DEF_X',
    raw_key: 'DEF_X',
    value: '0.5',
    unit: null,
    scope_instance: { kind: 'case-scalar', scheme: null, index: null },
  } as unknown as ParameterOverride;
  const p = page(undefined, {}, [record]);
  await p.mount();
  await p.openDraft('custom');
  // 列表里的"从研究导入"以前直接全写进草稿；界面上的按钮先打开勾选对话框，现在两者一致。
  p.pickFolder('/study');
  await p.click('version-tab-research');
  const c = await p.settle();
  await c.actions.find((action) => action.id === 'version-import-study')!.run();
  await p.settle();
  const ids = p.actions().map((action) => action.id);
  assert.deepEqual(ids, ['version-import-study-cancel', 'version-import-study-apply']);
  assert.equal(
    p.calls.some((call) => call.command === 'model_import_study' && !call.args.preview),
    false,
    '预览之前不写草稿',
  );
  await p.click('version-import-study-apply');
  await p.settle();
  const write = p.calls.find(
    (call) => call.command === 'model_import_study' && call.args.preview === null,
  )!;
  assert.deepEqual(write.args.only, [['case:DEF_X', null, null]]);
});

test('the code tab registers its own actions and keeps export and revert out of reach', async () => {
  const p = page();
  await p.mount();
  await p.click('version-tab-code');
  let c = await p.settle();
  // A frozen official version can start a code-changing draft; the changes tab's buttons are hidden.
  assert.ok(c.actions.some((action) => action.id === 'version-method-source'));
  // 冻结版本代码页的两个折叠区（用已有工作区 / 从其他仓库建）也登记展开动作。
  assert.ok(c.actions.some((action) => action.id === 'version-new-workspace-advanced'));
  // 草稿还没有自己的代码时：开始修改代码 + 两个折叠入口都可见。
  const d = page();
  await d.mount();
  await d.openDraft('custom');
  await d.click('version-tab-code');
  const draftCode = await d.settle();
  assert.ok(draftCode.actions.some((action) => action.id === 'version-branch-code'));
  assert.ok(draftCode.actions.some((action) => action.id === 'version-reuse-workspace-advanced'));
  // 参数页签里挂着一个代码工作区：创建、检查能按，导出补丁和删除工作区不登记。
  const q = page(undefined, {}, [], { kind: 'workspace', workspace: 'ver-1', commit: 'abc' });
  await q.mount();
  await q.openDraft('custom');
  await q.click('version-tab-code');
  c = await q.settle();
  assert.ok(c.actions.some((action) => action.id === 'workspace-check'));
  assert.ok(c.actions.some((action) => action.id === 'workspace-patch'));
  assert.equal(
    c.actions.some((action) => /export|revert|delete/.test(action.id)),
    false,
    '导出补丁、恢复与删除工作区属于 D 级',
  );
});

test('a study button is visible on the research tab but adding a pick needs a chosen parameter', async () => {
  const p = page();
  await p.mount();
  await p.openDraft('custom');
  await p.click('version-tab-research');
  const c = await p.settle();
  const add = c.actions.find((action) => action.id === 'version-study-pick-add');
  assert.ok(add);
  assert.equal(add.enabled, false, '没选参数时"加入"是灰的，助手看到的也是不可用');
  assert.equal(
    c.actions.some((action) => action.id.includes('study-pick-remove')),
    false,
  );
});

// U-01：从"新建修改版"选研究基线不得换掉模拟正在用的版本。base 通过界面上"基于哪个版本"的
// Select（VersionHub 的 onBase）选择，和用户点选一样。
test('a research method from 新建修改版 records its own baseline and keeps the run version', async () => {
  const p = page();
  p.state.versions.push({
    ...p.version,
    name: 'Soil scheme',
    version: { id: 'soil', revision: 2 },
  });
  await p.mountHub();
  const hub = p.elements.find((element) => element.type === 'VersionHub')!;
  (hub.props.onBase as (key: string) => void)('soil#2');
  p.render();
  const c = await p.settle();
  await c.actions.find((action) => action.id === 'version-method-tuning')!.run();
  await p.settle();
  assert.deepEqual(p.state.researchBaseline, { id: 'soil', revision: 2 });
  assert.equal(p.state.researchMethod, 'tuning');
  assert.deepEqual(
    p.state.versionRef,
    { id: 'official', revision: 1 },
    '选择研究基线不改模拟运行版本',
  );
  assert.deepEqual(p.state.version.version, { id: 'official', revision: 1 });
  assert.equal(p.state.chosenVersion, null, '研究基线不是用户明确选择的运行版本');
  assert.equal(p.routes.at(-1), 'research');
});

// U-01：从草稿的"研究"页签进去会先把草稿冻结成研究修订；这个修订只是研究基线，不是运行版本。
test('the research tab on a draft freezes a study baseline without switching the run version', async () => {
  const p = page();
  await p.mount();
  await p.openDraft('custom');
  await p.click('version-tab-research');
  const c = await p.settle();
  await c.actions.find((action) => action.id === 'version-method-tuning')!.run();
  await p.settle();
  const freeze = p.calls.find((call) => call.command === 'model_freeze')!;
  assert.equal(freeze.args.finish, false, '为研究冻结时草稿保留，研究结果还要写回它');
  assert.deepEqual(p.state.researchBaseline, { id: 'custom', revision: 2 });
  assert.deepEqual(p.state.versionRef, { id: 'official', revision: 1 });
  assert.equal(p.state.chosenVersion, null);
  assert.deepEqual(p.state.researchTarget, { draftId: 'custom', name: 'Official' });
  assert.equal(p.routes.at(-1), 'research');
});

test('the research tab on a frozen revision records it as the baseline only', async () => {
  const p = page();
  await p.mount();
  await p.click('version-tab-research');
  const c = await p.settle();
  await c.actions.find((action) => action.id === 'version-method-uncertainty')!.run();
  await p.settle();
  assert.deepEqual(p.state.researchBaseline, { id: 'official', revision: 1 });
  assert.deepEqual(p.state.versionRef, { id: 'official', revision: 1 });
  assert.equal(p.state.chosenVersion, null, '用这个版本做研究，不等于把它设为运行模型');
  assert.ok(!p.calls.some((call) => call.command === 'model_freeze'));
});

// U-01/U-05：研究结果形成的修订冻结后不改运行版本，必须点「设为运行模型」才生效。
test('a revision frozen from study results becomes the run model only after 设为运行模型', async () => {
  const p = page();
  // 研究写回后版本页直接打开那个草稿：confirmWriteBack 会设 openDraft + researchTarget。
  p.state.versions.push({ ...p.version, name: 'Official', version: { id: 'custom', revision: 2 } });
  await p.mount();
  await p.openDraft('custom');
  Object.assign(p.state, { researchTarget: { draftId: 'custom', name: 'PFT study' } });
  let c = await p.settle();
  await c.actions.find((action) => action.id === 'version-freeze')!.run();
  c = await p.settle();
  assert.deepEqual(
    p.state.versionRef,
    { id: 'official', revision: 1 },
    '研究修订冻结后模拟仍用原来的版本',
  );
  assert.equal(p.state.chosenVersion, null);
  assert.match(p.notices.at(-1) ?? '', /研究基线/, '提示里说清楚运行版本没有被改动');
  const use = c.actions.find((action) => action.id === 'version-use-custom-2');
  assert.ok(use, '冻结出研究修订后必须给显式的「设为运行模型」动作');
  assert.equal(use.label, '设为运行模型');
  await use.run();
  await p.settle();
  assert.deepEqual(p.state.versionRef, { id: 'custom', revision: 2 });
  assert.deepEqual(p.state.chosenVersion, { id: 'custom', revision: 2 });
});

// 审计 P1-5：研究页把 DE 训练出的网络写回草稿时，网络文件在草稿暂存目录里；版本页冻结必须
// 把那个 asset_root 一起交给 model_freeze，否则冻结出来的版本没有网络文件。
test('a network written back from a study is frozen with the asset root it carries', async () => {
  const p = page();
  // harness 的 state 类型只列了页面用到的键；这里模拟研究页导航前写进共享状态的两个字段。
  const app = p.state as Record<string, unknown>;
  app.openDraft = 'custom';
  app.openDraftAssets = '/assets/model-custom/7';
  await p.mount(false);
  // 研究页带过来的资产目录被消费一次，不同草稿之间不串用。
  assert.equal(app.openDraft, null);
  assert.equal(app.openDraftAssets, null);
  const c = p.render();
  const freeze = c.actions.find((action) => action.id === 'version-freeze');
  assert.ok(freeze, '打开草稿后应能冻结');
  await freeze!.run();
  await p.settle();
  const call = p.calls.find((call) => call.command === 'model_freeze');
  assert.ok(call, '冻结应调用 model_freeze');
  assert.equal(call!.args.assetRoot, '/assets/model-custom/7');
});

// P2：模型导入命令也返回产物类型与网络暂存目录（`kind` / `asset_root` / `network`）。原来
// importChosen 只读 draft/member/parameters，网络导入返回的新 asset_root 被丢掉，冻结仍用上一次
// 的暂存目录，冻结出来的版本没有这次导入的网络文件。
const studyNetwork = {
  slot: 'pft',
  features: ['clim_tair'],
  outputs: ['DEF_PFT_VMAX25'],
  hidden: [],
  normalize: false,
  weights: 3,
};

/** 从研究导入：打开草稿（带旧资产目录）→ 研究页签 → 预览 → 写进草稿。 */
async function importStudyIntoDraft(p: ReturnType<typeof page>, priorAssets: string) {
  const app = p.state as Record<string, unknown>;
  app.openDraft = 'custom';
  app.openDraftAssets = priorAssets;
  await p.mount(false);
  await p.settle();
  await p.click('version-tab-research');
  await p.settle();
  p.pickFolder('/study');
  let c = await p.settle();
  await c.actions.find((action) => action.id === 'version-import-study')!.run();
  await p.settle();
  c = await p.settle();
  await c.actions.find((action) => action.id === 'version-import-study-apply')!.run();
  await p.settle();
  return p.render();
}

test('a network imported from a study replaces the asset root used for freezing', async () => {
  const p = page(
    undefined,
    {},
    [],
    { kind: 'official' },
    {
      kind: 'network',
      network: studyNetwork,
      assetRoot: '/new-network-assets',
    },
  );
  const c = await importStudyIntoDraft(p, '/prior-process-assets');
  // 成功提示按产物类型说明这次写进的是 AI 网络，不是参数。
  assert.match(successText(p), /AI 网络|网络/);
  const freeze = c.actions.find((action) => action.id === 'version-freeze');
  assert.ok(freeze, '网络导入后草稿仍可冻结');
  await freeze!.run();
  await p.settle();
  const call = p.calls.find((call) => call.command === 'model_freeze');
  assert.ok(call, '冻结应调用 model_freeze');
  assert.equal(call!.args.assetRoot, '/new-network-assets', '冻结要用这次网络导入返回的新资产目录');
});

test('a parameter-only study import keeps the asset root the page already had', async () => {
  const record = {
    parameter_id: 'case:DEF_X',
    raw_key: 'DEF_X',
    value: '0.5',
    unit: null,
    scope_instance: { kind: 'case-scalar', scheme: null, index: null },
  } as unknown as ParameterOverride;
  const p = page(undefined, {}, [record]);
  const c = await importStudyIntoDraft(p, '/prior-process-assets');
  assert.match(successText(p), /个参数/);
  const freeze = c.actions.find((action) => action.id === 'version-freeze');
  assert.ok(freeze, '参数导入后草稿仍可冻结');
  await freeze!.run();
  await p.settle();
  const call = p.calls.find((call) => call.command === 'model_freeze');
  assert.ok(call, '冻结应调用 model_freeze');
  assert.equal(call!.args.assetRoot, '/prior-process-assets', '纯参数导入不改资产目录');
});

// U-05：草稿保存的三态 + 失败保留 dirty + 字段改动清掉过时的成功提示。
test('draft saving reports dirty, saving and saved, and keeps edits when the save fails', async () => {
  const p = page();
  await p.mount();
  await p.openDraft('custom');
  await p.settle();
  const badge = () =>
    p.elements.find((element) => element.type === SaveBadge) as Element | undefined;
  // 取真实字符串节点（JSON.stringify 会把提示里的引号转义，正则对不上）。
  const texts = () => {
    const out: string[] = [];
    const walk = (node: unknown) => {
      if (typeof node === 'string') out.push(node);
      else if (Array.isArray(node)) node.forEach(walk);
      else if (node && typeof node === 'object') walk((node as Element).children);
    };
    walk(p.elements);
    return out.join('\n');
  };
  assert.equal(badge()?.props.state, 'saved', '刚打开的草稿没有未保存修改');
  let c = await p.settle();
  c.fields.find((field) => field.id === 'draft-name')!.set('renamed');
  c = await p.settle();
  assert.equal(badge()?.props.state, 'dirty');
  // 写盘请求还挂着：徽标是"正在保存"，旁边也说正在保存。
  p.hold('model_save_draft');
  void c.actions.find((action) => action.id === 'version-save')!.run();
  p.render();
  assert.equal(badge()?.props.state, 'saving');
  assert.match(texts(), /正在把草稿写进版本库/);
  // 让挂住的请求把放行者装好，再放行。
  await p.settle();
  p.releaseHeld();
  c = await p.settle();
  assert.equal(badge()?.props.state, 'saved');
  assert.match(texts(), /已保存草稿"renamed"/);
  // 再改字段：上一条"已保存草稿…"必须清掉，不能停在已经改过的内容旁边。
  c.fields.find((field) => field.id === 'draft-name')!.set('renamed again');
  c = await p.settle();
  assert.equal(badge()?.props.state, 'dirty');
  assert.doesNotMatch(texts(), /已保存草稿/);
  // 保存失败：错误显示出来、dirty 保留，徽标绝不回到"已保存"。
  p.fail('model_save_draft');
  void c.actions.find((action) => action.id === 'version-save')!.run();
  await p.settle();
  assert.equal(badge()?.props.state, 'dirty', '保存失败保留未保存状态');
  assert.doesNotMatch(texts(), /已保存草稿/);
  assert.match(texts(), /model_save_draft failed/);
});
