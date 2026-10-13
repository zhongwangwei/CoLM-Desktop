import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  filterModelParameters,
  modelParameterEditable,
  modelSettingGroup,
  modelSettingValue,
  type ModelParameter,
} from './model-settings';
import type { Overlay } from '../../lib/types';
const overlay: Overlay = {
  physics: { DEF_USE_PFT: '.true.' },
  parameters: [
    {
      parameter_id: 'soil',
      raw_key: 'def_soil',
      value: '0.2',
      scope_instance: { kind: 'case-scalar' },
    },
    {
      parameter_id: 'class',
      raw_key: 'CLASS_PARAM',
      value: '99',
      scope_instance: { kind: 'land-cover-class', index: 1 },
    },
  ],
  process_files: [],
};
test('shows only explicit model values, not schema defaults or scoped values as scalar facts', () => {
  assert.equal(modelSettingValue(overlay, 'def_use_pft'), '.true.');
  assert.equal(modelSettingValue(overlay, 'DEF_SOIL'), '0.2');
  assert.equal(modelSettingValue(overlay, 'DEF_USE_BGC'), null);
  assert.equal(modelSettingValue(overlay, 'CLASS_PARAM'), null);
});
test('keeps scientific groups and unknown fields visible', () => {
  assert.equal(modelSettingGroup({ name: 'DEF_USE_PFT', section: '地表数据' }), 'subgrid');
  assert.equal(
    modelSettingGroup({ name: 'DEF_USE_VariablySaturatedFlow', section: '水热过程' }),
    'soil',
  );
  assert.equal(modelSettingGroup({ name: 'DEF_FOO', section: '河道与水库' }), 'river');
  assert.equal(modelSettingGroup({ name: 'DEF_NEW_SCHEME', section: '新增分组' }), 'other');
});

test('version manager returns to the originating simulation route without dropping the restoration marker', async () => {
  const { readFileSync } = await import('node:fs'),
    { default: ts } = await import('typescript');
  const path = new URL('../versions/VersionsPage.tsx', import.meta.url),
    source = ts.createSourceFile(
      path.pathname,
      readFileSync(path, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
  let body = '';
  const visit = (node: import('typescript').Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === 'returnToSimulation')
      body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body);
  const code = ts.transpileModule(`const action=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  for (const route of ['new-case', 'case'] as const) {
    const state = { simulationReturn: { route, step: 3 } },
      calls: unknown[] = [];
    const action = new Function('state', 'navigate', `${code};return action;`)(
      state,
      (...args: unknown[]) => {
        calls.push(args);
        return true;
      },
    );
    assert.equal(action(), true);
    assert.deepEqual(calls, [[route, route === 'case' ? 'settings' : undefined]]);
    assert.equal(state.simulationReturn.step, 3);
  }
  const action = new Function('state', 'navigate', `${code};return action;`)(
    { simulationReturn: null },
    () => assert.fail('no simulation return destination'),
  );
  assert.equal(action(), undefined);
});

test('model group boundaries follow backend sections instead of matching all soil words', () => {
  assert.equal(
    modelSettingGroup({ name: 'DEF_TUNING_CSOILC', section: '生态与生地化' }),
    'ecology',
  );
  for (const name of ['DEF_SOLO_PFT', 'DEF_FAST_PC', 'DEF_USE_LULCC'])
    assert.equal(modelSettingGroup({ name, section: '地表数据' }), 'subgrid');
});
test('catalog remains searchable without advertising case fields or unsupported records as model edits', () => {
  const p = {
    id: 'soil',
    raw_key: 'SOIL',
    label_zh: '土壤',
    label_en: 'Soil',
    section: '水热过程',
    scope: 'case-scalar',
    storage: 'case-nml',
    visibility: 'editable-scientific',
  } as ModelParameter;
  assert.equal(modelParameterEditable(p, []), false);
  assert.equal(modelParameterEditable(p, [{ name: 'soil' }]), true);
  for (const visibility of ['read-only-context', 'blocked-pending-hook', 'excluded-internal'])
    assert.equal(modelParameterEditable({ ...p, visibility }, [{ name: 'SOIL' }]), false);
  assert.equal(
    modelParameterEditable({ ...p, raw_key: 'DEF_SUBGRID_SCHEME' }, [
      { name: 'DEF_SUBGRID_SCHEME' },
    ]),
    false,
  );
  assert.equal(modelParameterEditable({ ...p, storage: 'pft-override' }, []), true);
  const scoped = { ...p, id: 'scoped', scope: 'pft-type', visibility: 'read-only-context' };
  assert.deepEqual(filterModelParameters([p, scoped], '土壤', 'pft-type'), [scoped]);
  assert.deepEqual(filterModelParameters([p, scoped], '', ''), [p, scoped]);
  assert.deepEqual(filterModelParameters([p], 'missing', ''), []);
});

test('model settings guide exposes classification only with a callback and catalog filters only when expanded', async () => {
  const { readFileSync } = await import('node:fs'),
    { default: ts } = await import('typescript'),
    helpers = await import('./model-settings');
  const source = readFileSync(new URL('./ModelSettings.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  let cursor = 0,
    contract: {
      actions: { id: string; enabled?: boolean; run: () => unknown }[];
      fields: { id: string; disabled?: boolean }[];
    };
  const slots: unknown[] = [[], [], '', false, false, '', ''];
  const context = {
    ...helpers,
    setupHandledKeys: [],
    React: { createElement: () => null },
    ScientificSetupCards: () => {},
    ConfigurationOptions: () => {},
    Advanced: () => {},
    Button: () => {},
    Card: () => {},
    Field: () => {},
    Input: () => {},
    Notice: () => {},
    Select: () => {},
    parameterKey: () => '',
    useApp: () => ({ state: { language: 'zh', kernels: [] } }),
    useEffect: () => {},
    useState: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = initial;
      return [
        slots[key],
        (next: unknown) => {
          slots[key] = typeof next === 'function' ? next(slots[key]) : next;
        },
      ];
    },
    usePageContract: (value: typeof contract) => {
      contract = value;
    },
  };
  const Component = new Function(...Object.keys(context), `${code};return ModelSettings;`)(
    ...Object.values(context),
  );
  const version = {
    name: 'official',
    version: { id: 'official', revision: 1 },
    code: { source_sha256: 'abc' },
    overlay: { physics: {}, parameters: [], process_files: [] },
  };
  const choices: string[] = [];
  const render = (callback = true, disabled = false) => {
    cursor = 0;
    Component({
      version,
      onEdit: () => {},
      disabled,
      ...(callback ? { onClassificationChange: (value: string) => choices.push(value) } : {}),
    });
    return contract;
  };
  let result = render();
  assert.deepEqual(result.fields, []);
  result.actions.find((action) => action.id === 'simulation-classification-usgs')!.run();
  assert.deepEqual(choices, ['USGS']);
  assert.deepEqual(version.overlay.physics, {});
  result.actions.find((action) => action.id === 'simulation-parameter-catalog-toggle')!.run();
  result = render();
  assert.deepEqual(
    result.fields.map((field) => field.id),
    ['simulation-parameter-query', 'simulation-parameter-scope'],
  );
  result = render(false, true);
  assert.equal(
    result.actions.some((action) => action.id.startsWith('simulation-classification-')),
    false,
  );
  assert.ok(result.actions.every((action) => action.enabled === false));
  assert.ok(result.fields.every((field) => field.disabled));
});

test('simulation scheme keeps vegetation structure separate from its land-cover classification', async () => {
  const { simulationMode } = await import('./model-settings');
  assert.equal(simulationMode(), undefined);
  assert.equal(simulationMode(undefined, 'USGS', 'PFT'), 'PFT');
  assert.equal(simulationMode(overlay, 'USGS'), 'PFT');
  assert.equal(simulationMode(overlay, 'IGBP', 'PC'), 'PC');
  assert.equal(simulationMode({ ...overlay, physics: { DEF_USE_PC: '.true.' } }, 'USGS'), 'PC');
  assert.equal(simulationMode({ ...overlay, physics: { DEF_USE_PFT: '.false.' } }, 'USGS'), 'USGS');
  assert.equal(
    simulationMode(
      { ...overlay, physics: { DEF_USE_PFT: '.true.', DEF_USE_PC: '.true.' } },
      'IGBP',
    ),
    undefined,
  );
  assert.equal(simulationMode(undefined, undefined, 'USGS'), 'USGS');
});

test('four scheme cards share existing-case modification requests while retaining classification and write locks', async () => {
  const { readFileSync } = await import('node:fs'),
    { default: ts } = await import('typescript'),
    helpers = await import('./model-settings');
  const source = readFileSync(new URL('./ModelSettings.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  type Element = { type: unknown; props: Record<string, unknown>; children: unknown[] };
  let cursor = 0,
    contract: {
      actions: { id: string; enabled?: boolean; run: () => void }[];
      details: { classification: string; simulation_mode: string };
    };
  const slots: unknown[] = [[], [], '', false, false, '', ''];
  const context = {
    ...helpers,
    setupHandledKeys: [],
    React: {
      createElement: (
        type: unknown,
        props: Record<string, unknown>,
        ...children: unknown[]
      ): Element => ({ type, props: props ?? {}, children }),
    },
    ...Object.fromEntries(
      [
        'ScientificSetupCards',
        'ConfigurationOptions',
        'Advanced',
        'Button',
        'Card',
        'Field',
        'Input',
        'Notice',
        'Select',
      ].map((name) => [name, name]),
    ),
    parameterKey: () => '',
    useApp: () => ({ state: { language: 'zh', kernels: [] } }),
    useEffect: () => {},
    useState: (initial: unknown) => [slots[cursor++] ?? initial, () => {}],
    usePageContract: (value: typeof contract) => {
      contract = value;
    },
  };
  const Component = new Function(...Object.keys(context), `${code};return ModelSettings;`)(
    ...Object.values(context),
  );
  const version = {
    name: 'official',
    version: { id: 'official', revision: 1 },
    code: { source_sha256: 'abc' },
    overlay: { physics: { DEF_USE_PFT: '.true.' }, parameters: [], process_files: [] },
  };
  const original = JSON.stringify(version),
    modes: string[] = [],
    classifications: string[] = [],
    requests: { id: string; value: string }[] = [];
  const flatten = (node: unknown): Element[] =>
    Array.isArray(node)
      ? node.flatMap(flatten)
      : node && typeof node === 'object' && 'children' in node
        ? [node as Element, ...flatten((node as Element).children)]
        : [];
  const render = (editable: boolean, request = false, disabled = false) => {
    cursor = 0;
    const tree = Component({
      version,
      disabled,
      ...(request
        ? { onSetupRequest: (v: { id: string; value: string }) => requests.push(v) }
        : {}),
      classification: 'USGS',
      profile: { subgrid: 'PFT' },
      onEdit: () => {},
      ...(editable
        ? {
            onModeChange: (value: string) => modes.push(value),
            onClassificationChange: (value: string) => classifications.push(value),
          }
        : {}),
    });
    return { nodes: flatten(tree), contract };
  };
  let result = render(true);
  const cards = result.nodes.filter((node) => node.props.className === 'simulation-choice-card');
  assert.equal(cards.length, 4);
  assert.deepEqual(
    cards.map((node) => node.props['aria-pressed']),
    [false, false, true, false],
  );
  assert.equal(result.contract.details.classification, 'USGS');
  assert.equal(result.contract.details.simulation_mode, 'PFT');
  for (const id of [
    'simulation-mode-igbp',
    'simulation-mode-usgs',
    'simulation-mode-pft',
    'simulation-mode-pc',
  ])
    result.contract.actions.find((action) => action.id === id)!.run();
  assert.deepEqual(modes, ['IGBP', 'USGS', 'PFT', 'PC']);
  result.contract.actions.find((action) => action.id === 'simulation-classification-igbp')!.run();
  assert.deepEqual(classifications, ['IGBP']);
  (cards[3].props.onClick as () => void)();
  assert.equal(modes.at(-1), 'PC');
  assert.equal(JSON.stringify(version), original);
  result = render(false);
  assert.ok(
    result.nodes
      .filter((node) => node.props.className === 'simulation-choice-card')
      .every((node) => node.props.disabled),
  );
  assert.equal(
    result.contract.actions.some((action) => /simulation-(mode|classification)-/.test(action.id)),
    false,
  );
  result = render(false, true);
  const existing = result.nodes.filter((node) => node.props.className === 'simulation-choice-card');
  assert.equal(existing[0].props.disabled, true, 'existing classification cannot be changed');
  assert.equal(existing[1].props.disabled, false, 'current USGS classification can return to LCT');
  assert.equal(existing[3].props.disabled, false);
  (existing[3].props.onClick as () => void)();
  assert.deepEqual(requests, [{ id: 'subgrid', value: 'PC' }]);
  result.contract.actions.find((a) => a.id === 'simulation-mode-pft')!.run();
  assert.equal(requests.length, 1, 'already selected structure needs no new version');
  result.contract.actions.find((a) => a.id === 'simulation-mode-usgs')!.run();
  assert.deepEqual(requests.at(-1), { id: 'subgrid', value: 'LCT' });
  assert.equal(JSON.stringify(version), original);
  result = render(false, true, true);
  assert.ok(
    result.nodes
      .filter((node) => node.props.className === 'simulation-choice-card')
      .every((node) => node.props.disabled),
  );
  assert.ok(result.contract.actions.every((a) => a.enabled === false));
});
