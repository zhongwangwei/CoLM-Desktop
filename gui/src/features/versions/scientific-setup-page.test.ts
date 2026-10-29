import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { bilingual } from '../../lib/bilingual';
import * as setup from './scientific-setup';
import { modelSettingValue } from '../cases/model-settings';
import type { ConfigurationSource, ConfigurationResult } from './configuration-options';
import { caseOptionOverlay, caseOptionPatch, type CaseOptionPatch } from './configuration-options';
import type { Overlay, KernelEntry } from '../../lib/types';
import type { ModelField } from './model';
import type { UiAction, UiField } from '../../lib/guide';
type Contract = { fields: UiField[]; actions: UiAction[]; status: string };
function harness() {
  const overlay: Overlay = {
    physics: {
      DEF_USE_LCT: '.false.',
      DEF_USE_PFT: '.true.',
      DEF_USE_PC: '.false.',
      DEF_USE_BGC: '.false.',
    },
    parameters: [],
    process_files: [],
  };
  const fields = setup.setupHandledKeys.map(
    (name) =>
      ({
        name,
        kind: 'logical',
        derived: false,
        values: [],
        default: '.false.',
        section: '',
      }) as ModelField,
  );
  const changes: Overlay[] = [],
    forward: setup.SetupRequest[] = [];
  const props = {
    caseDir: '/case/a',
    source: { kind: 'draft', id: 'd1', generation: 3, overlay } as ConfigurationSource,
    overlay,
    fields,
    environment: { subgrid: 'PFT', domain: 'single', grid: 'single' } as setup.SetupEnvironment,
    hasSettings: () => true,
    onOpenSettings: undefined as undefined | ((id: string) => void),
    includeSubgrid: true,
    disabled: false,
    onChange: (o: Overlay) => changes.push(o),
    onCaseChange: undefined as undefined | ((patch: CaseOptionPatch) => void | Promise<void>),
  };
  const requests: { args: Record<string, unknown>; resolve: (r: ConfigurationResult) => void }[] =
      [],
    slots: unknown[] = [],
    effects: { deps: unknown[]; cleanup?: () => void }[] = [],
    pending: (() => void)[] = [],
    nodes: { type: string; props: Record<string, unknown>; children: unknown[] }[] = [];
  let cursor = 0,
    ei = 0,
    contract: Contract;
  const context = {
    ...setup,
    caseOptionOverlay,
    caseOptionPatch,
    modelSettingValue,
    bilingual,
    React: {
      createElement: (type: string, p: Record<string, unknown>, ...children: unknown[]) => {
        const node = { type, props: p ?? {}, children };
        nodes.push(node);
        return node;
      },
    },
    ...Object.fromEntries(
      ['Button', 'ActionButton', 'Card', 'Field', 'Input', 'Notice'].map((k) => [k, k]),
    ),
    useApp: () => ({ state: { language: 'zh' } }),
    // 动作由 ActionButton 登记（第 737 轮）：取最近一次渲染出的按钮。
    usePageContract: (c: Contract) => {
      contract = {
        ...c,
        get actions() {
          return [
            ...c.actions,
            ...nodes
              .filter((node) => node.type === 'ActionButton')
              .map((node) => ({
                id: String(node.props.action),
                label: String(node.props.label),
                kind: 'click' as const,
                enabled: !node.props.disabled,
                run: node.props.run as () => unknown,
              })),
          ];
        },
      };
    },
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (v: unknown) => {
          slots[i] = v;
        },
      ];
    },
    useRef: (v: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: v };
      return slots[i];
    },
    useEffect: (fn: () => void | (() => void), deps: unknown[]) => {
      const i = ei++;
      if (!effects[i] || deps.some((d, j) => d !== effects[i].deps[j])) {
        effects[i]?.cleanup?.();
        effects[i] = { deps };
        pending.push(() => {
          effects[i].cleanup = fn() || undefined;
        });
      }
    },
    invokeJson: (_cmd: string, args: Record<string, unknown>) =>
      new Promise<ConfigurationResult>((resolve) => requests.push({ args, resolve })),
  };
  const source = readFileSync(new URL('./ScientificSetupCards.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(context), `${code};return ScientificSetupCards;`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    ei = 0;
    nodes.length = 0;
    Component(props);
    pending.splice(0).forEach((fn) => fn());
    return contract;
  }
  const response: ConfigurationResult = {
    crop_defaults: { DEF_TUNING_CROP_PLANTING_DAY: '120' },
    fields: [],
    process_files: [],
    pft_parameters: [],
    warnings: [],
    context_complete: false,
  };
  async function resolve(index = requests.length - 1) {
    requests[index].resolve(response);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    return render();
  }
  return { props, render, resolve, requests, changes, forward, nodes, response };
}
test('cards and guide share visible controls, exact context, loading and disabled locks', async () => {
  const h = harness();
  let c = h.render();
  assert.equal(c.status, 'loading');
  assert.equal(h.requests[0].args.caseDir, '/case/a');
  assert.equal((h.requests[0].args.source as { generation: number }).generation, 3);
  c = await h.resolve();
  assert.equal(c.status, 'pending_source_verification');
  assert.deepEqual(
    c.fields.map((f) => f.id),
    setup
      .setupChoices(h.props.overlay, h.props.fields, h.props.environment)
      .map((o) => `setup-${o.id}`),
  );
  assert.equal(h.nodes.filter((n) => n.type === 'Card').length, c.fields.length);
  assert.ok(!c.fields.some((f) => f.id === 'setup-DEF_UnitCatchment_regional'));
  assert.ok(!JSON.stringify(h.nodes).includes('Regional unit-catchment routing'));
  c.fields.find((f) => f.id === 'setup-DEF_USE_BGC')!.set('.true.');
  assert.equal(h.changes.length, 1);
  const beforeLock = c.fields[0];
  h.props.disabled = true;
  c = h.render();
  assert.throws(() => beforeLock.set('PC'));
  assert.ok(c.fields.every((f) => f.disabled));
  assert.throws(() => c.fields[0].set('PC'));
  h.props.disabled = false;
  h.props.includeSubgrid = false;
  c = h.render();
  assert.ok(!c.fields.some((f) => f.id === 'setup-subgrid'));
});
test('old query and old guide callback cannot write after draft/source changes', async () => {
  const h = harness();
  h.render();
  let c = await h.resolve();
  const old = c.fields[0];
  h.props.source = { kind: 'draft', id: 'd2', generation: 9, overlay: h.props.overlay };
  c = h.render();
  assert.throws(() => old.set('PC'), /stale/);
  assert.equal(c.status, 'loading');
  c = await h.resolve();
  assert.notEqual(c.status, 'loading');
  const early = harness();
  early.render();
  early.props.caseDir = '/case/b';
  early.render();
  c = await early.resolve(0);
  assert.equal(c.status, 'loading');
  c = await early.resolve(1);
  assert.equal(c.status, 'pending_source_verification');
});
test('frozen model cards write case fields directly and leave the selected revision unchanged', async () => {
  const h = harness();
  h.props.source = { kind: 'frozen', version: { id: 'official', revision: 5 } };
  h.render();
  let c = await h.resolve();
  assert.ok(c.fields.every((f) => f.disabled));
  const patches: CaseOptionPatch[] = [];
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  c = h.render();
  const before = structuredClone(h.props.overlay);
  await c.fields.find((f) => f.id === 'setup-soil')!.set('vG');
  assert.deepEqual(patches, [
    { DEF_USE_Campbell_SOIL_MODEL: '.false.', DEF_USE_VariablySaturatedFlow: '.true.' },
  ]);
  assert.deepEqual(h.props.overlay, before);
  assert.equal(h.changes.length, 0);
});

test('tracer choice opens species configuration before saving, including incompatible saved definitions', async () => {
  for (const enabled of [false, true]) {
    const h = harness();
    h.props.source = { kind: 'frozen', version: { id: 'official', revision: 5 } };
    h.props.environment.caseValues = {
      DEF_USE_METHANE: enabled ? '.true.' : '.false.',
      DEF_USE_Campbell_SOIL_MODEL: '.true.',
      DEF_URBAN_RUN: '.true.',
      DEF_METHANE_PARAM_FILE: "'standard_methane_parameter.nml'",
    };
    const patches: CaseOptionPatch[] = [],
      opened: string[] = [];
    h.props.onCaseChange = (patch) => {
      patches.push(patch);
    };
    h.props.onOpenSettings = (id) => {
      opened.push(id);
    };
    h.render();
    const c = await h.resolve();
    const tracer = c.fields.find((field) => field.id === 'setup-tracer')!;
    assert.ok(
      tracer.options!.some((option) => option.value === '.true.' && option.label === '选择示踪剂'),
    );
    await tracer.set('.true.');
    // 界面上示踪剂没有单独的"设置"按钮，再次进入就是再选一次"选择示踪剂"（第 737 轮起清单与界面一致）。
    assert.ok(!c.actions.some((action) => action.id === 'setup-tracer-settings'));
    await tracer.set('.true.');
    assert.deepEqual(opened, ['tracer', 'tracer']);
    assert.deepEqual(patches, []);
    assert.equal(h.props.environment.caseValues.DEF_USE_METHANE, enabled ? '.true.' : '.false.');
    h.props.disabled = true;
    h.render();
    assert.throws(() => tracer.set('.true.'), /locked|stale|不可修改|已切换/i);
  }
});

test('crop card enables immediately using standard case defaults and permits later planting edits', async () => {
  const h = harness();
  h.props.overlay.physics.DEF_USE_BGC = '.true.';
  h.props.fields.find((f) => f.name === 'DEF_TUNING_CROP_PLANTING_DAY')!.kind = 'integer';
  h.props.environment.cropKernel = { dir: '/crop', macros: ['CROP'] } as KernelEntry;
  h.render();
  let c = await h.resolve();
  c.fields.find((f) => f.id === 'setup-crop')!.set('.true.');
  assert.equal(h.changes[0].physics.DEF_TUNING_CROP_PLANTING_DAY, '120');
  h.props.overlay = h.changes[0];
  h.render();
  c = await h.resolve();
  assert.equal(c.fields.find((f) => f.id === 'setup-crop-planting-day')!.value, '120');
  c.fields.find((f) => f.id === 'setup-crop-planting-day')!.set('125');
  c = h.render();
  c.actions.find((a) => a.id === 'setup-crop-apply')!.run();
  assert.equal(h.changes.at(-1)!.physics.DEF_TUNING_CROP_PLANTING_DAY, '125');
});

test('visible river options and guide enforce the same verified methane-mode gate', async () => {
  for (const mode of ['routing', 'hybrid', 'wetwat']) {
    const h = harness();
    Object.assign(h.props.overlay.physics, {
      DEF_USE_METHANE: '.true.',
      DEF_USE_BGC: '.true.',
      DEF_USE_GridRiverLakeFlow: '.true.',
      DEF_METHANE_PARAM_FILE: "'standard_methane_parameter.nml'",
    });
    h.response.process_files = [
      {
        file: 'CH4.nml',
        title: '',
        section: '',
        verified: true,
        active_methane_asset: true,
        entries: [
          {
            path: 'DEF_METHANE%inundation_mode',
            value: mode,
            unset: false,
            group: 'methane',
            kind: 'choice',
            asset_value: mode,
            default: 'wetwat',
            options: ['wetwat', 'routing', 'hybrid'],
            editable: true,
            descriptor_id: null,
            pending: false,
          },
        ],
      },
    ] as ConfigurationResult['process_files'];
    h.render();
    const c = await h.resolve(),
      river = c.fields.find((f) => f.id === 'setup-DEF_USE_GridRiverLakeFlow')!;
    if (mode === 'wetwat') {
      assert.ok(river.options!.some((o) => o.value === '.false.'));
      river.set('.false.');
      assert.equal(h.changes.length, 1);
    } else {
      assert.ok(!river.options!.some((o) => o.value === '.false.'));
      assert.throws(() => river.set('.false.'), /methane/);
      assert.equal(h.changes.length, 0);
    }
  }
});

test('simulation opens settings only after a successful user enable despite the expected caseValues refresh', async () => {
  const h = harness(),
    opened: string[] = [];
  h.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  h.props.environment.caseValues = { DEF_USE_BGC: '.false.' };
  h.props.onOpenSettings = (id) => opened.push(id);
  let finish!: () => void;
  h.props.onCaseChange = (patch) => {
    h.props.environment = {
      ...h.props.environment,
      caseValues: { ...h.props.environment.caseValues, ...patch } as Record<string, string>,
    };
    h.render();
    return new Promise<void>((resolve) => {
      finish = resolve;
    });
  };
  h.render();
  let c = await h.resolve();
  assert.deepEqual(opened, []);
  const pending = c.fields.find((f) => f.id === 'setup-DEF_USE_BGC')!.set('.true.');
  assert.deepEqual(opened, []);
  finish();
  await pending;
  assert.deepEqual(opened, ['DEF_USE_BGC']);
  c = await h.resolve();
  assert.ok(c.actions.some((a) => a.id === 'setup-DEF_USE_BGC-settings'));
  const disabling = c.fields.find((f) => f.id === 'setup-DEF_USE_BGC')!.set('.false.');
  finish();
  await disabling;
  assert.deepEqual(opened, ['DEF_USE_BGC']);
  await h.resolve();
  h.props.onCaseChange = async () => {
    throw new Error('save failed');
  };
  c = h.render();
  await assert.rejects(
    Promise.resolve(c.fields.find((f) => f.id === 'setup-DEF_USE_BGC')!.set('.true.')),
    /save failed/,
  );
  assert.equal(opened.length, 1);
});
test('an enable reply for a previous case never opens settings in the new case', async () => {
  const h = harness(),
    opened: string[] = [];
  h.props.environment.caseValues = { DEF_USE_BGC: '.false.' };
  h.props.onOpenSettings = (id) => opened.push(id);
  let finish!: () => void;
  h.props.onCaseChange = () =>
    new Promise<void>((resolve) => {
      finish = resolve;
    });
  h.render();
  const c = await h.resolve();
  const pending = c.fields.find((f) => f.id === 'setup-DEF_USE_BGC')!.set('.true.');
  h.props.caseDir = '/case/b';
  h.render();
  finish();
  await pending;
  assert.deepEqual(opened, []);
});

test('vG renders the VSF off button disabled and excludes it from guide choices', async () => {
  const h = harness();
  h.props.overlay.physics.DEF_USE_Campbell_SOIL_MODEL = '.false.';
  h.render();
  const c = await h.resolve();
  const vsf = c.fields.find((f) => f.id === 'setup-DEF_USE_VariablySaturatedFlow')!;
  assert.equal(vsf.value, '.true.');
  assert.ok(!vsf.options!.some((o) => o.value === '.false.'));
  assert.throws(() => vsf.set('.false.'), /Campbell/);
  const off = h.nodes.find(
    (n) =>
      n.type === 'Button' &&
      n.children.some((child) => JSON.stringify(child)?.includes('choose Campbell first')),
  );
  assert.ok(off);
  assert.equal(off.props.disabled, true);
});

test('clicking an already enabled process opens settings without saving again', async () => {
  for (const id of ['DEF_USE_BGC', 'DEF_USE_VariablySaturatedFlow', 'crop']) {
    const h = harness(),
      opened: string[] = [],
      patches: CaseOptionPatch[] = [];
    Object.assign(h.props.overlay.physics, {
      DEF_USE_BGC: '.true.',
      DEF_USE_VariablySaturatedFlow: '.true.',
      DEF_TUNING_CROP_PLANTING_DAY: '125',
    });
    h.props.environment.cropKernel = { dir: '/crop', macros: ['CROP'] } as KernelEntry;
    h.props.environment.caseValues = { ...h.props.overlay.physics };
    h.props.onOpenSettings = (value) => opened.push(value);
    h.props.onCaseChange = (patch) => {
      patches.push(patch);
    };
    h.render();
    const c = await h.resolve();
    await c.fields.find((f) => f.id === `setup-${id}`)!.set('.true.');
    assert.deepEqual(opened, [id]);
    assert.deepEqual(patches, []);
    h.props.disabled = true;
    h.render();
    assert.throws(() => c.fields.find((f) => f.id === `setup-${id}`)!.set('.true.'));
    assert.deepEqual(opened, [id]);
  }
});

test('controls without settings never open a dialog when selecting or repeating a value', async () => {
  const h = harness(),
    opened: string[] = [];
  h.props.hasSettings = () => false;
  h.props.onOpenSettings = (id) => opened.push(id);
  h.props.onCaseChange = () => {};
  h.render();
  let c = await h.resolve();
  await c.fields.find((f) => f.id === 'setup-soil')!.set('Campbell');
  h.props.overlay.physics.DEF_USE_Campbell_SOIL_MODEL = '.true.';
  h.props.environment.caseValues = { ...h.props.overlay.physics };
  h.render();
  c = await h.resolve();
  await c.fields.find((f) => f.id === 'setup-soil')!.set('Campbell');
  assert.deepEqual(opened, []);
  assert.ok(!c.actions.some((a) => a.id === 'setup-soil-settings'));
});

test('settings actions from a previous case stay stale after the next case finishes loading', async () => {
  const h = harness(),
    opened: string[] = [];
  h.props.overlay.physics.DEF_USE_BGC = '.true.';
  h.props.environment.caseValues = { ...h.props.overlay.physics };
  h.props.onOpenSettings = (id) => opened.push(id);
  h.props.onCaseChange = () => {};
  h.render();
  const c = await h.resolve();
  const open = c.actions.find((a) => a.id === 'setup-DEF_USE_BGC-settings')!;
  h.props.caseDir = '/case/b';
  h.render();
  await h.resolve();
  assert.throws(() => open.run(), /stale/);
  assert.deepEqual(opened, []);
});

test('an inherited enabled process is persisted before opening either settings entry', async () => {
  for (const useSettingsButton of [false, true]) {
    const h = harness(),
      opened: string[] = [],
      patches: CaseOptionPatch[] = [];
    h.props.fields.find((f) => f.name === 'DEF_USE_GridRiverLakeFlow')!.default = '.true.';
    h.props.environment.domain = 'region';
    h.props.environment.grid = 'latlon';
    h.props.environment.kernel = {
      dir: '/grid',
      macros: ['GRIDBASED', 'GridRiverLakeFlow'],
    } as KernelEntry;
    h.props.onOpenSettings = (id) => {
      assert.equal(patches.at(-1)?.DEF_USE_GridRiverLakeFlow, '.true.');
      opened.push(id);
    };
    h.props.onCaseChange = (patch) => {
      patches.push(patch);
    };
    h.render();
    const c = await h.resolve();
    if (useSettingsButton)
      await c.actions.find((a) => a.id === 'setup-DEF_USE_GridRiverLakeFlow-settings')!.run();
    else await c.fields.find((f) => f.id === 'setup-DEF_USE_GridRiverLakeFlow')!.set('.true.');
    assert.deepEqual(opened, ['DEF_USE_GridRiverLakeFlow']);
    assert.equal(patches.length, 1);
  }
});
test('an unavailable inherited process does not expose an empty settings entry', async () => {
  const h = harness();
  h.props.fields.find((f) => f.name === 'DEF_USE_GridRiverLakeFlow')!.default = '.true.';
  h.props.onOpenSettings = () => {
    throw new Error('must not open');
  };
  h.props.onCaseChange = () => {};
  h.render();
  const c = await h.resolve();
  assert.ok(!c.actions.some((a) => a.id === 'setup-DEF_USE_GridRiverLakeFlow-settings'));
});
