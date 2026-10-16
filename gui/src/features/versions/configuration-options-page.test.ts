import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as helpers from './configuration-options';
import * as settings from '../cases/model-settings';
import * as presentation from '../../lib/param-presentation';
import { PFT } from './scope-labels';
import type { Overlay } from '../../lib/types';
import type { UiAction, UiField } from '../../lib/guide';
import type { ModelField, Descriptor } from './model';
type Contract = { modal?: boolean; fields: UiField[]; actions: UiAction[]; status: string };
function harness() {
  const overlay: Overlay = {
    physics: {
      DEF_USE_BGC: '.true.',
      DEF_USE_TRACER: '.true.',
      DEF_USE_GridRiverLakeFlow: '.true.',
      DEF_USE_WUEST: '.true.',
    },
    parameters: [],
    process_files: [],
  };
  const fields: ModelField[] = [
    ['DEF_USE_BGC', '生态与生地化'],
    ['DEF_USE_MEDLYNST', '生态与生地化'],
    ['DEF_USE_WUEST', '生态与生地化'],
    ['DEF_USE_NITRIF_DENITRIF', '生态与生地化'],
    ['DEF_USE_GridRiverLakeFlow', '河道与水库'],
    ['DEF_USE_RESERVOIR', '河道与水库'],
    ['DEF_USE_TRACER', '示踪剂'],
    ['DEF_URBAN_RUN', '城市'],
    ['DEF_URBAN_type_scheme', '城市'],
    ...helpers.tracerKeys.map((k) => [k, '示踪剂']),
  ].map(([name, section]) => ({
    name,
    section,
    kind: helpers.tracerKeys.includes(name)
      ? name === 'DEF_TRACER_NUM'
        ? 'integer'
        : 'character'
      : 'logical',
    default: '.false.',
    values: [],
    derived: false,
  }));
  const props = {
    process: undefined as string | undefined,
    hideProcesses: false,
    modalActions: undefined as UiAction[] | undefined,
    removedCaseKeys: [] as string[],
    onReady: undefined as ((ready: boolean) => void) | undefined,
    onCaseChange: undefined as
      undefined | ((patch: helpers.CaseOptionPatch) => void | Promise<void>),
    caseValues: undefined as Record<string, string> | undefined,
    subgrid: undefined as string | undefined,
    source: { kind: 'draft', id: 'd1', generation: 0, overlay } as helpers.ConfigurationSource,
    overlay,
    fields,
    catalog: [] as Descriptor[],
    disabled: false,
    onChange: (next: Overlay) => {
      props.overlay = next;
      changes.push(next);
    },
  };
  const changes: Overlay[] = [],
    requests: {
      command: string;
      args: Record<string, unknown>;
      resolve: (value: helpers.ConfigurationResult) => void;
    }[] = [],
    slots: unknown[] = [],
    effects: { deps: unknown[]; cleanup?: () => void }[] = [],
    pending: (() => void)[] = [];
  let cursor = 0,
    effectCursor = 0,
    contract: Contract;
  const nodes: { type: string; props: Record<string, unknown>; children?: unknown[] }[] = [];
  const context = {
    PFT,
    ...helpers,
    ...settings,
    ...presentation,
    React: {
      createElement: (type: string, p: Record<string, unknown>, ...children: unknown[]) => {
        const result = { type, props: p ?? {}, children };
        nodes.push(result);
        return result;
      },
    },
    ...Object.fromEntries(
      ['Advanced', 'Button', 'Card', 'Field', 'Input', 'Notice', 'Select'].map((k) => [k, k]),
    ),
    useApp: () => ({ state: { language: 'zh' } }),
    usePageContract: (value: Contract) => {
      contract = value;
    },
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (value: unknown) => {
          slots[i] = typeof value === 'function' ? value(slots[i]) : value;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: initial };
      return slots[i];
    },
    useEffect: (fn: () => void | (() => void), deps: unknown[]) => {
      const i = effectCursor++;
      if (!effects[i] || deps.some((d, j) => d !== effects[i].deps[j])) {
        effects[i]?.cleanup?.();
        effects[i] = { deps };
        pending.push(() => {
          const cleanup = fn();
          effects[i].cleanup = cleanup || undefined;
        });
      }
    },
    invokeJson: (command: string, args: Record<string, unknown>) =>
      new Promise<helpers.ConfigurationResult>((resolve) =>
        requests.push({ command, args, resolve }),
      ),
  };
  const source = readFileSync(new URL('./ConfigurationOptions.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(context), `${code};return ConfigurationOptions;`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    effectCursor = 0;
    nodes.length = 0;
    Component(props);
    pending.splice(0).forEach((f) => f());
    return contract;
  }
  const response = (): helpers.ConfigurationResult => ({
    fields: fields.map((f) => ({
      name: f.name,
      mode: f.name === 'DEF_USE_NITRIF_DENITRIF' ? 'disabled' : 'editable',
      reason: f.name === 'DEF_USE_NITRIF_DENITRIF' ? 'disabled evidence' : 'pending',
      configured_value: props.overlay.physics[f.name] ?? null,
      allowed_values: [],
      pending: true,
    })),
    process_files: [],
    pft_parameters: [],
    warnings: ['pending frozen source'],
    context_complete: false,
  });
  async function settle() {
    requests.at(-1)!.resolve(response());
    for (let i = 0; i < 5; i++) await Promise.resolve();
    return render();
  }
  return { render, settle, props, requests, changes, response, nodes };
}
test('actual editor and guide expose conditional groups, retain inactive fields, and lock frozen/busy writes', async () => {
  const p = harness();
  let c = p.render();
  assert.equal(c.status, 'loading');
  assert.equal(p.requests[0].command, 'model_configuration');
  assert.equal(p.requests[0].args.caseValues, null);
  c = await p.settle();
  const medlyn = () => c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!;
  assert.ok(medlyn());
  assert.equal(medlyn().disabled, false);
  const inactive = c.fields.find((f) => f.id === 'configuration-DEF_USE_NITRIF_DENITRIF')!;
  assert.equal(inactive.disabled, true);
  assert.throws(() => inactive.set('.true.'));
  assert.equal(p.changes.length, 0);
  medlyn().set('MEDLYN');
  assert.equal(p.props.overlay.physics.DEF_USE_WUEST, '.false.');
  assert.equal(p.props.overlay.physics.DEF_USE_BGC, '.true.');
  c = p.render();
  assert.equal(c.status, 'loading');
  c = await p.settle();
  assert.equal(medlyn().value, 'MEDLYN');
  p.props.disabled = true;
  c = p.render();
  assert.equal(medlyn().disabled, true);
  assert.throws(() => medlyn().set('BALL_BERRY'));
  p.props.disabled = false;
  p.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  c = p.render();
  c = await p.settle();
  assert.equal(medlyn().disabled, true);
  assert.throws(() => medlyn().set('BALL_BERRY'));
  assert.equal(p.changes.length, 1);
});
test('late configuration responses cannot replace current version and guide only contains expanded controls', async () => {
  const p = harness();
  p.render();
  const old = p.requests[0];
  p.props.source = { kind: 'draft', id: 'd2', generation: 0, overlay: p.props.overlay };
  p.render();
  let c: Contract;
  const latest = p.requests[1];
  old.resolve(p.response());
  for (let i = 0; i < 5; i++) await Promise.resolve();
  c = p.render();
  assert.equal(c.status, 'loading');
  latest.resolve(p.response());
  for (let i = 0; i < 5; i++) await Promise.resolve();
  c = p.render();
  assert.notEqual(c.status, 'loading');
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  c = p.render();
  assert.ok(!c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
  assert.ok(!c.actions.some((a) => a.id === 'configuration-stomatal-reset'));
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  c = p.render();
  assert.ok(c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
  assert.ok(c.actions.some((a) => a.id === 'configuration-stomatal-reset'));
  assert.equal(p.requests.at(-1)!.args.pftSelection, null);
  assert.ok(!c.fields.some((f) => f.id === 'configuration-pft-index'));
  c.actions.find((a) => a.id === 'configuration-pft-toggle')!.run();
  c = p.render();
  const index = c.fields.find((f) => f.id === 'configuration-pft-index')!;
  index.set('');
  p.render();
  assert.equal(p.requests.at(-1)!.args.pftSelection, null);
});
test('tracer table accumulates incomplete rows locally, then applies valid aligned definitions atomically', async () => {
  const p = harness();
  p.render();
  let c = await p.settle();
  c.actions.find((a) => a.id === 'configuration-tracers-add')!.run();
  c = p.render();
  assert.equal(p.changes.length, 0);
  assert.throws(() => c.actions.find((a) => a.id === 'configuration-tracers-apply')!.run());
  assert.equal(p.changes.length, 0);
  c.fields.find((f) => f.id === 'configuration-tracer-0-NAMES')!.set('CH4');
  c = p.render();
  c.fields.find((f) => f.id === 'configuration-tracer-0-TYPES')!.set('reactive');
  c = p.render();
  c.actions.find((a) => a.id === 'configuration-tracers-apply')!.run();
  assert.equal(p.changes.length, 1);
  assert.equal(p.props.overlay.physics.DEF_TRACER_NUM, '1');
  assert.equal(p.props.overlay.physics.DEF_TRACER_NAMES, "'CH4'");
});
test('enabling BGC, rivers or tracer reopens previously collapsed children only on a new activation', async () => {
  for (const [group, key] of [
    ['ecology', 'DEF_USE_BGC'],
    ['river', 'DEF_USE_GridRiverLakeFlow'],
    ['tracer', 'DEF_USE_TRACER'],
  ]) {
    const p = harness();
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.false.' },
    };
    p.render();
    let c = await p.settle();
    const groupNode = () =>
      p.nodes.find(
        (n) =>
          n.type === 'Advanced' &&
          String(n.props.title).startsWith(
            settings.modelSettingGroups.find((g) => g[0] === group)![1],
          ),
      )!;
    assert.equal(groupNode().props.open, false);
    c.actions.find((a) => a.id === `configuration-group-${group}`)!.run();
    c = p.render();
    assert.equal(groupNode().props.open, true);
    c.actions.find((a) => a.id === `configuration-group-${group}`)!.run();
    p.render();
    assert.equal(groupNode().props.open, false);
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.true.' },
    };
    p.render();
    assert.equal(groupNode().props.open, true);
    c = await p.settle();
    assert.equal(groupNode().props.open, true);
    c.actions.find((a) => a.id === `configuration-group-${group}`)!.run();
    p.render();
    assert.equal(groupNode().props.open, false);
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, DEF_USE_WUEST: '.false.' },
    };
    p.render();
    assert.equal(groupNode().props.open, false);
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.false.' },
    };
    p.render();
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.true.' },
    };
    p.render();
    assert.equal(groupNode().props.open, true);
  }
});

test('unknown parent activation opens its previously collapsed group without waiting for metadata', async () => {
  const p = harness();
  delete p.props.overlay.physics.DEF_USE_BGC;
  p.render();
  let c = await p.settle();
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  c = p.render();
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  p.render();
  p.props.overlay = {
    ...p.props.overlay,
    physics: { ...p.props.overlay.physics, DEF_USE_BGC: '.true.' },
  };
  c = p.render();
  assert.equal(c.status, 'loading');
  assert.ok(c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
});
test('actual process-list editor preserves quoted list display and submits per-element typed values', async () => {
  const p = harness();
  p.props.overlay.process_files = [{ relative_path: 'forcing.nml', sha256: 'a' }];
  p.props.catalog = [
    {
      id: 'forcing:role',
      raw_key: 'DEF_forcing%forcing_role',
      label_zh: '强迫场角色',
      label_en: 'Forcing roles',
      scope: 'process-file',
      storage: 'process-parameter-file',
      section: '',
      value_kind: 'character',
      unit: null,
      visibility: 'editable',
      activation: [],
      default: null,
      doc_zh: '',
      doc_en: '',
      validation: { min: null, max: null, allowed_values: [], finite: false },
    },
  ];
  p.render();
  const response = p.response();
  response.process_files = [
    {
      file: 'forcing.nml',
      title: 'forcing',
      section: '',
      verified: true,
      active_methane_asset: false,
      entries: [
        {
          path: 'DEF_forcing%forcing_role',
          group: 'DEF_forcing',
          kind: 'list',
          value: "'TAIR', 'WIND'",
          asset_value: "'TAIR', 'WIND'",
          default: '',
          unset: false,
          options: [],
          editable: true,
          descriptor_id: 'forcing:role',
          pending: true,
        },
      ],
    },
  ];
  p.requests.at(-1)!.resolve(response);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  const c = p.render(),
    field = c.fields.find(
      (f) => f.id === 'configuration-process-forcing.nml-DEF_forcing%forcing_role',
    )!;
  assert.equal(field.value, "'TAIR', 'WIND'");
  field.set("'TAIR', 'RAIN'");
  assert.equal(p.changes[0].parameters[0].value, "'TAIR', 'RAIN'");
});

test('actual generic groups and guide omit land-class fields that require indexed parameter overrides', async () => {
  const p = harness();
  p.props.fields.push({
    name: 'DEF_LC_VMAX25',
    section: '生态与生地化',
    kind: 'real',
    default: '50.',
    values: [],
    derived: false,
  });
  p.props.catalog.push({
    id: 'land:IGBP:vmax',
    raw_key: 'def_lc_vmax25',
    label_zh: '光合能力',
    label_en: 'Photosynthesis capacity',
    scope: 'land-cover-class',
    storage: 'case-nml',
    section: '',
    value_kind: 'real',
    unit: null,
    visibility: 'editable',
    activation: [],
    default: null,
    doc_zh: '',
    doc_en: '',
    validation: { min: null, max: null, allowed_values: [], finite: true },
  });
  p.render();
  const c = await p.settle();
  assert.ok(c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
  assert.ok(!c.fields.some((f) => f.id.includes('DEF_LC_VMAX25')));
  assert.ok(!c.actions.some((a) => a.id.includes('DEF_LC_VMAX25')));
  assert.ok(
    !p.nodes.some(
      (n) => n.type === 'Field' && n.props.label === presentation.fieldLabel('DEF_LC_VMAX25', 'zh'),
    ),
  );
  assert.equal(p.changes.length, 0);
});

test('family cards stage only missing names and types, preserve custom rows and never save on click', async () => {
  const p = harness();
  p.props.overlay.physics = {
    ...p.props.overlay.physics,
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_URBAN_RUN: '.false.',
    DEF_USE_PFT: '.true.',
    DEF_USE_LCT: '.false.',
    DEF_USE_PC: '.false.',
    DEF_USE_LULCC: '.false.',
  };
  p.render();
  let c = await p.settle();
  const isotope = c.actions.find((a) => a.id === 'configuration-family-isotope')!;
  assert.equal(isotope.enabled, true);
  isotope.run();
  c = p.render();
  assert.equal(p.changes.length, 0);
  assert.equal(c.fields.find((f) => f.id === 'configuration-tracer-0-NAMES')?.value, 'H2_18O');
  assert.equal(c.fields.find((f) => f.id === 'configuration-tracer-0-MRAT')?.value, '');
  assert.equal(c.fields.find((f) => f.id === 'configuration-tracer-1-NAMES')?.value, 'HDO');
  assert.equal(c.actions.find((a) => a.id === 'configuration-family-isotope')?.enabled, false);
  c.actions.find((a) => a.id === 'configuration-tracers-remove')!.run();
  c = p.render();
  assert.equal(
    c.fields.some((f) => f.id === 'configuration-tracer-1-NAMES'),
    false,
  );
  assert.equal(p.changes.length, 0);
  p.props.overlay = {
    ...p.props.overlay,
    physics: { ...p.props.overlay.physics, DEF_USE_LULCC: '.true.' },
  };
  p.render();
  c = await p.settle();
  assert.equal(c.actions.find((a) => a.id === 'configuration-family-isotope')?.enabled, false);
  p.props.disabled = true;
  c = p.render();
  assert.ok(
    c.actions
      .filter((a) => a.id.startsWith('configuration-family-'))
      .every((a) => a.enabled === false),
  );
});
test('core runoff enum appears as choice cards while the visible guide shares the same atomic setter', async () => {
  const p = harness();
  p.props.fields.push({
    name: 'DEF_Runoff_SCHEME',
    section: '水热过程',
    kind: 'integer',
    default: '0',
    values: ['0', '1', '2', '3'],
    derived: false,
  });
  p.render();
  let c = await p.settle();
  c.actions.find((a) => a.id === 'configuration-group-water')!.run();
  c = p.render();
  const field = c.fields.find((f) => f.id === 'configuration-DEF_Runoff_SCHEME')!;
  assert.equal(field.value, '0');
  assert.deepEqual(
    field.options?.map((o) => o.value),
    ['0', '1', '2', '3'],
  );
  const choice = p.nodes.find(
    (n) =>
      n.type === 'Button' &&
      n.props['aria-pressed'] === false &&
      n.props.className === 'simulation-choice-card' &&
      n.children?.flat().includes(presentation.optionLabel('DEF_Runoff_SCHEME', '2', 'zh')),
  )!;
  assert.ok(choice);
  field.set('2');
  assert.equal(p.changes[0].physics.DEF_Runoff_SCHEME, '2');
  assert.throws(() => field.set('99'));
});

test('same-kernel scheme changes reload BGC tracer and PFT context and invalidate prior replies', async () => {
  const h = harness();
  h.props.subgrid = 'IGBP';
  h.render();
  await h.settle();
  const old = h.requests.at(-1)!;
  h.props.subgrid = 'PFT';
  h.render();
  const pft = h.requests.at(-1)!;
  assert.notEqual(pft, old);
  assert.equal((pft.args.caseValues as Record<string, string>).DEF_USE_PFT, '.true.');
  h.props.subgrid = 'PC';
  h.render();
  const pc = h.requests.at(-1)!;
  assert.notEqual(pc, pft);
  assert.equal((pc.args.caseValues as Record<string, string>).DEF_USE_PC, '.true.');
  assert.deepEqual(pc.args.source, pft.args.source);
  assert.equal(pc.args.kernelDir, pft.args.kernelDir);
  pft.resolve({ ...h.response(), warnings: ['STALE PFT RESULT'] });
  for (let i = 0; i < 5; i++) await Promise.resolve();
  h.render();
  assert.ok(!JSON.stringify(h.nodes).includes('STALE PFT RESULT'));
  await h.settle();
  assert.ok(!JSON.stringify(h.nodes).includes('STALE PFT RESULT'));
});

test('frozen model process options persist an atomic case patch without editing the revision', async () => {
  const h = harness(),
    patches: helpers.CaseOptionPatch[] = [];
  h.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  h.props.caseValues = {
    DEF_USE_BGC: '.true.',
    DEF_USE_MEDLYNST: '.false.',
    DEF_USE_WUEST: '.true.',
  };
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  const original = structuredClone(h.props.overlay);
  h.render();
  const c = await h.settle();
  c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!.set('MEDLYN');
  for (let i = 0; i < 8; i++) await Promise.resolve();
  assert.deepEqual(patches, [{ DEF_USE_MEDLYNST: '.true.', DEF_USE_WUEST: '.false.' }]);
  assert.equal(h.changes.length, 0);
  assert.deepEqual(h.props.overlay, original);
});
test('case option failure and readonly source gates preserve the selected revision', async () => {
  const h = harness();
  h.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  h.props.caseValues = { DEF_USE_BGC: '.true.' };
  h.props.onCaseChange = async () => {
    throw new Error('case save rejected');
  };
  const original = structuredClone(h.props.overlay);
  h.render();
  let c = await h.settle();
  c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!.set('MEDLYN');
  for (let i = 0; i < 12; i++) await Promise.resolve();
  c = h.render();
  assert.match(c.status, /case save rejected/);
  assert.deepEqual(h.props.overlay, original);
  assert.equal(h.changes.length, 0);
  h.props.subgrid = 'PC';
  h.render();
  const response = h.response();
  response.fields.find((f) => f.name === 'DEF_USE_MEDLYNST')!.mode = 'readonly';
  h.requests.at(-1)!.resolve(response);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  c = h.render();
  assert.equal(c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!.disabled, true);
});

test('simulation hides inactive process details and files without erasing retained settings', async () => {
  const p = harness();
  p.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  p.props.onCaseChange = () => {};
  p.props.subgrid = 'IGBP';
  const original = JSON.stringify(p.props.overlay);
  for (const value of [undefined, '.false.', '.true.', '.false.']) {
    p.props.caseValues = Object.fromEntries(
      ['DEF_USE_BGC', 'DEF_USE_TRACER', 'DEF_USE_GridRiverLakeFlow', 'DEF_URBAN_RUN'].flatMap(
        (name) => (value ? [[name, value]] : []),
      ),
    );
    p.render();
    const response = p.response();
    response.fields = response.fields.map((field) => ({ ...field, configured_value: null }));
    p.requests.at(-1)!.resolve(response);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    const c = p.render();
    for (const group of ['river', 'urban', 'tracer']) {
      const available = value === '.true.';
      assert.equal(
        c.actions.some((a) => a.id === `configuration-group-${group}`),
        available,
      );
    }
    assert.equal(
      p.nodes.some((n) => n.type === 'Card' && n.props.title === '示踪过程文件'),
      value === '.true.',
    );
    assert.equal(
      c.fields.some((f) => f.id === 'configuration-DEF_USE_NITRIF_DENITRIF'),
      value === '.true.',
    );
    assert.ok(!c.actions.some((a) => a.id === 'configuration-pft-toggle'));
  }
  assert.equal(JSON.stringify(p.props.overlay), original);
  assert.equal(p.changes.length, 0);
});

test('simulation hides confirmed disabled child fields but retains readonly options', async () => {
  const p = harness();
  p.props.onCaseChange = () => {};
  p.props.caseValues = {
    DEF_USE_BGC: '.true.',
    DEF_USE_TRACER: '.true.',
    DEF_USE_GridRiverLakeFlow: '.true.',
  };
  p.render();
  const response = p.response();
  response.fields = response.fields.map((field) => ({
    ...field,
    mode:
      field.name === 'DEF_USE_NITRIF_DENITRIF'
        ? 'disabled'
        : field.name === 'DEF_USE_RESERVOIR'
          ? 'readonly'
          : field.mode,
    pending: false,
  }));
  p.requests.at(-1)!.resolve(response);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  const c = p.render();
  assert.ok(!c.fields.some((f) => f.id === 'configuration-DEF_USE_NITRIF_DENITRIF'));
  assert.ok(c.fields.some((f) => f.id === 'configuration-DEF_USE_RESERVOIR'));
  assert.equal(p.changes.length, 0);
});

test('simulation details honor backend parent gate even when applicability remains pending', async () => {
  for (const enabled of [undefined, false, true]) {
    const p = harness();
    p.props.onCaseChange = () => {};
    p.props.caseValues = { DEF_USE_BGC: '.true.' };
    p.render();
    const response = p.response();
    response.fields = response.fields.map((f) => ({
      ...f,
      mode: 'editable',
      pending: true,
      ...(f.name === 'DEF_USE_NITRIF_DENITRIF'
        ? { parent_gate: 'DEF_USE_BGC', parent_gate_enabled: enabled }
        : {}),
    }));
    p.requests.at(-1)!.resolve(response);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    assert.equal(
      p.render().fields.some((f) => f.id === 'configuration-DEF_USE_NITRIF_DENITRIF'),
      enabled === true,
    );
  }
});

test('focused simulation editor stages the final typed value and explicit reset while preserving modal guide scope', async () => {
  const h = harness(),
    patches: helpers.CaseOptionPatch[] = [];
  h.props.process = 'DEF_URBAN_RUN';
  h.props.modalActions = [{ id: 'save-dialog', label: 'Save', kind: 'click', run: () => {} }];
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  h.props.caseValues = { DEF_URBAN_RUN: '.true.', DEF_URBAN_type_scheme: '1' };
  const field = h.props.fields.find((f) => f.name === 'DEF_URBAN_type_scheme')!;
  field.kind = 'integer';
  field.values = [];
  h.render();
  h.requests.at(-1)!.resolve(h.response());
  await new Promise((resolve) => setTimeout(resolve, 0));
  let c = h.render();
  assert.equal(c.modal, true);
  assert.ok(c.actions.some((a) => a.id === 'save-dialog'));
  assert.ok(!c.fields.some((f) => f.id.includes('tracer') || f.id.includes('stomatal')));
  const input = h.nodes.find((n) => n.type === 'Input' && n.props.value === '1')!;
  (input.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: '2' } });
  assert.deepEqual(patches.at(-1), { DEF_URBAN_type_scheme: '2' });
  c = h.render();
  await c.actions.find((a) => a.id === 'configuration-DEF_URBAN_type_scheme-reset')!.run();
  assert.deepEqual(patches.at(-1), { DEF_URBAN_type_scheme: null });
  h.props.removedCaseKeys = ['DEF_URBAN_type_scheme'];
  assert.equal(
    h.render().fields.find((f) => f.id === 'configuration-DEF_URBAN_type_scheme')?.value,
    '.false.',
  );
});
test('modal save readiness rejects invalid text and unapplied tracer definitions', async () => {
  const h = harness();
  let ready = false;
  h.props.process = 'DEF_USE_TRACER';
  h.props.modalActions = [];
  h.props.onReady = (value) => {
    ready = value;
  };
  h.props.onCaseChange = () => {};
  h.props.caseValues = { DEF_USE_TRACER: '.true.' };
  h.render();
  h.requests.at(-1)!.resolve(h.response());
  await new Promise((resolve) => setTimeout(resolve, 0));
  let c = h.render();
  assert.equal(ready, true);
  await c.actions.find((a) => a.id === 'configuration-tracers-add')!.run();
  c = h.render();
  assert.equal(ready, false);
  assert.ok(
    h.nodes.some((n) => n.type === 'Notice' && JSON.stringify(n.children).includes('先应用')),
  );
  await c.actions.find((a) => a.id === 'configuration-tracers-remove')!.run();
  h.render();
  // Even an empty pending table must be applied explicitly before Save.
  assert.equal(ready, false);
  const urban = harness();
  urban.props.process = 'DEF_URBAN_RUN';
  urban.props.modalActions = [];
  urban.props.onReady = (value) => {
    ready = value;
  };
  urban.props.onCaseChange = () => {};
  urban.props.caseValues = { DEF_URBAN_RUN: '.true.', DEF_URBAN_type_scheme: '1' };
  urban.props.fields.find((f) => f.name === 'DEF_URBAN_type_scheme')!.kind = 'integer';
  urban.render();
  urban.requests.at(-1)!.resolve(urban.response());
  await new Promise((resolve) => setTimeout(resolve, 0));
  urban.render();
  assert.equal(ready, true);
  const input = urban.nodes.find((n) => n.type === 'Input' && n.props.value === '1')!;
  (input.props.onChange as (e: { target: { value: string } }) => void)({
    target: { value: 'invalid' },
  });
  urban.render();
  assert.equal(ready, false);
});

test('ordinary simulation options keep independent ecology and stomatal choices accessible with BGC off', async () => {
  const h = harness();
  h.props.hideProcesses = true;
  h.props.onCaseChange = () => {};
  h.props.caseValues = { DEF_USE_BGC: '.false.' };
  for (const name of ['DEF_USE_SNICAR', 'DEF_USE_PROSPECT', 'DEF_LC_YEAR'])
    h.props.fields.push({
      name,
      section: '生态与生地化',
      kind: 'logical',
      default: '.false.',
      values: [],
      derived: false,
    });
  h.render();
  h.requests.at(-1)!.resolve(h.response());
  await new Promise((resolve) => setTimeout(resolve, 0));
  let c = h.render();
  await c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  c = h.render();
  for (const name of ['DEF_USE_SNICAR', 'DEF_USE_PROSPECT'])
    assert.ok(c.fields.some((f) => f.id === `configuration-${name}`));
  assert.ok(c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
  for (const name of ['DEF_LC_YEAR', 'DEF_USE_NITRIF_DENITRIF'])
    assert.ok(!c.fields.some((f) => f.id === `configuration-${name}`));
});

test('simulation fractionation children stay hidden while queries are pending and preserve saved values', async () => {
  const h = harness();
  const parent = 'DEF_TRACER_USE_FRACTIONATION';
  const children = [
    'DEF_TRACER_KINETIC_SCHEME',
    'DEF_TRACER_OPEN_WATER_KINETIC',
    'DEF_TRACER_SOIL_KINETIC',
  ];
  const schemes: Record<string, string[]> = {
    DEF_TRACER_KINETIC_SCHEME: ['MERLIVAT1978', 'CAPPA2003'],
    DEF_TRACER_OPEN_WATER_KINETIC: ['MJ79', 'EXPONENT'],
    DEF_TRACER_SOIL_KINETIC: ['RESISTANCE', 'EXPONENT'],
  };
  for (const name of [parent, ...children])
    h.props.fields.push({
      name,
      section: '示踪剂',
      kind: name === parent ? 'logical' : 'character',
      default: name === parent ? '.false.' : schemes[name][0],
      values: schemes[name] ?? [],
      derived: false,
    });
  h.props.process = 'DEF_USE_TRACER';
  h.props.onCaseChange = () => assert.fail('visibility changes must not save');
  h.props.caseValues = {
    DEF_USE_TRACER: '.true.',
    [parent]: '.false.',
    ...Object.fromEntries(children.map((name) => [name, `'${schemes[name][0]}'`])),
  };
  const retained = JSON.stringify(h.props.caseValues);
  let c = h.render();
  for (const name of children)
    assert.ok(
      !c.fields.some((f) => f.id === `configuration-${name}`),
      `${name} must stay hidden before the query returns`,
    );
  assert.ok(
    !h.nodes.some(
      (n) => n.type === 'small' && JSON.stringify(n.children).includes('DEF_TRACER_KINETIC_SCHEME'),
    ),
  );
  h.props.caseValues = { ...h.props.caseValues, [parent]: '.true.' };
  h.render();
  const response = h.response();
  response.fields = response.fields.map((f) =>
    children.includes(f.name) ? { ...f, parent_gate: parent, parent_gate_enabled: true } : f,
  );
  h.requests.at(-1)!.resolve(response);
  await new Promise((resolve) => setTimeout(resolve, 0));
  c = h.render();
  for (const name of children)
    assert.equal(c.fields.find((f) => f.id === `configuration-${name}`)?.value, schemes[name][0]);
  h.props.caseValues = { ...h.props.caseValues, [parent]: '.false.' };
  c = h.render();
  for (const name of children)
    assert.ok(
      !c.fields.some((f) => f.id === `configuration-${name}`),
      `${name} must immediately hide before the new query returns`,
    );
  assert.equal(JSON.stringify(h.props.caseValues), retained);
  assert.ok(
    c.fields.some((f) => f.id === `configuration-${parent}`),
    'the parent switch stays available',
  );
  h.requests.at(-1)!.resolve(response); // contradictory metadata cannot override the current explicit off choice
  await new Promise((resolve) => setTimeout(resolve, 0));
  c = h.render();
  for (const name of children) {
    assert.ok(!c.fields.some((f) => f.id === `configuration-${name}`));
    assert.ok(
      !h.nodes.some(
        (node) => node.type === 'small' && JSON.stringify(node.children).includes(name),
      ),
    );
  }
  h.props.onCaseChange = undefined;
  c = h.render();
  for (const name of children)
    assert.ok(
      c.fields.some((f) => f.id === `configuration-${name}`),
      'research editor keeps its existing visibility',
    );
});

test('river file placeholders are free text in both the actual editor and guide and persist a quoted path', async () => {
  const h = harness(),
    patches: helpers.CaseOptionPatch[] = [];
  const name = 'DEF_UnitCatchment_file';
  h.props.fields.push({
    name,
    kind: 'Character { len: 256 }',
    section: '河道与水库',
    default: 'null',
    values: ['', 'NONE', 'NULL'],
    derived: false,
  });
  h.props.process = 'DEF_USE_GridRiverLakeFlow';
  h.props.modalActions = [];
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  h.props.caseValues = { DEF_USE_GridRiverLakeFlow: '.true.', [name]: "'null'" };
  h.render();
  h.requests.at(-1)!.resolve(h.response());
  await new Promise((resolve) => setTimeout(resolve, 0));
  const c = h.render(),
    control = c.fields.find((f) => f.id === `configuration-${name}`)!;
  assert.equal(control.type, 'text');
  assert.ok(h.nodes.some((n) => n.type === 'Input' && n.props.value === 'null'));
  assert.ok(!h.nodes.some((n) => n.type === 'Select' && n.props.value === 'null'));
  const path = "/Volumes/Data/CoLMruntime/river/O'Brien network.nc";
  await control.set(path);
  assert.deepEqual(patches, [{ [name]: "'/Volumes/Data/CoLMruntime/river/O''Brien network.nc'" }]);
});

for (const [process, input, output, kind, initial] of [
  ['crop', 'DEF_TUNING_CROP_PLANTING_DAY', 'DEF_hist_vars%cropprod1c', 'integer', '120'],
  ['crop', 'DEF_USE_FERT', 'DEF_hist_vars%fertnitro_corn', 'logical', '.false.'],
  ['DEF_USE_BGC', 'DEF_file_cn_init', 'DEF_hist_vars%ndep_to_sminn', 'character', 'null'],
] as const)
  test(`${process} settings retain ${input} but leave ${output} in output settings`, async () => {
    const h = harness();
    h.props.process = process;
    h.props.modalActions = [];
    h.props.onCaseChange = () => {};
    h.props.caseValues = { DEF_USE_BGC: '.true.', DEF_USE_FERT: '.false.' };
    h.props.fields.push(
      { name: input, section: '生态与生地化', kind, default: initial, values: [], derived: false },
      {
        name: output,
        section: '输出变量',
        kind: 'logical',
        default: '.true.',
        values: [],
        derived: false,
      },
    );
    h.render();
    const c = await h.settle();
    assert.ok(c.fields.some((f) => f.id === `configuration-${input}`));
    assert.ok(!c.fields.some((f) => f.id === `configuration-${output}`));
    assert.ok(!h.nodes.some((n) => n.type === 'Field' && JSON.stringify(n.props).includes(output)));
  });

test('soil settings render and save fitting and resistance while respecting applicability', async () => {
  for (const hidden of [undefined, 'DEF_USE_SOILPAR_UPS_FIT', 'DEF_RSS_SCHEME']) {
    const h = harness(),
      patches: helpers.CaseOptionPatch[] = [];
    h.props.process = 'soil';
    h.props.modalActions = [];
    h.props.onCaseChange = (patch) => {
      patches.push(patch);
    };
    h.props.caseValues = {};
    for (const [name, kind, initial] of [
      ['DEF_USE_SOILPAR_UPS_FIT', 'logical', '.false.'],
      ['DEF_RSS_SCHEME', 'integer', '0'],
    ])
      h.props.fields.push({
        name,
        kind,
        section: '水热过程',
        default: initial,
        values: [],
        derived: false,
      });
    h.render();
    const response = h.response();
    response.fields = response.fields.map((f) => ({
      ...f,
      pending: false,
      mode: f.name === hidden ? 'disabled' : 'editable',
    }));
    h.requests.at(-1)!.resolve(response);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    const c = h.render();
    for (const [name, value] of [
      ['DEF_USE_SOILPAR_UPS_FIT', '.true.'],
      ['DEF_RSS_SCHEME', '1'],
    ]) {
      const field = c.fields.find((f) => f.id === `configuration-${name}`);
      if (name === hidden) assert.equal(field, undefined);
      else {
        assert.ok(field, name);
        await field.set(value);
        assert.equal(patches.at(-1)?.[name], value);
      }
    }
  }
});

test('process settings display inherited defaults without creating overrides, and reset returns to default', async () => {
  const h = harness(),
    patches: helpers.CaseOptionPatch[] = [];
  const name = 'DEF_PC_CROP_SPLIT';
  h.props.fields.push({
    name,
    section: '地表数据',
    kind: 'logical',
    default: '.true.',
    values: [],
    derived: false,
  });
  h.props.process = 'PC';
  h.props.subgrid = 'PC';
  h.props.modalActions = [];
  h.props.onCaseChange = (p) => {
    patches.push(p);
  };
  h.props.caseValues = {};
  h.render();
  let c = await h.settle();
  assert.equal(c.fields.find((f) => f.id === `configuration-${name}`)!.value, '.true.');
  assert.ok(
    !h.nodes.some((n) => n.type === 'option' && n.props.value === ''),
    'known default has no unspecified option',
  );
  assert.equal(c.actions.find((a) => a.id === `configuration-${name}-reset`)!.visible, false);
  assert.deepEqual(patches, []);
  await c.fields.find((f) => f.id === `configuration-${name}`)!.set('.false.');
  assert.equal(patches.at(-1)?.[name], '.false.');
  h.props.overlay.physics[name] = '.false.';
  h.props.caseValues[name] = '.false.';
  c = h.render();
  assert.equal(c.fields.find((f) => f.id === `configuration-${name}`)!.value, '.false.');
  await c.actions.find((a) => a.id === `configuration-${name}-reset`)!.run();
  assert.equal(patches.at(-1)?.[name], null);
  delete h.props.overlay.physics[name];
  delete h.props.caseValues[name];
  h.props.removedCaseKeys = [name];
  c = h.render();
  assert.equal(c.fields.find((f) => f.id === `configuration-${name}`)!.value, '.true.');
});

test('context defaults take precedence over catalog defaults without guessing a choice', async () => {
  const h = harness();
  const name = 'DEF_RSS_SCHEME';
  h.props.fields.push({
    name,
    section: '水热过程',
    kind: 'integer',
    default: '0',
    values: ['0', '1'],
    derived: false,
  });
  h.props.process = 'soil';
  h.props.onCaseChange = () => {};
  h.props.caseValues = {};
  h.render();
  const response = h.response();
  response.fields = response.fields.map((f) => ({
    ...f,
    built_in_default: f.name === name ? '1' : null,
  }));
  h.requests.at(-1)!.resolve(response);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  assert.equal(h.render().fields.find((f) => f.id === `configuration-${name}`)!.value, '1');
});

test('stomatal defaults select Ball–Berry but conflicting overrides remain unresolved', async () => {
  const h = harness();
  h.props.overlay.physics = {};
  h.render();
  await h.settle();
  h.render()
    .actions.find((a) => a.id === 'configuration-group-ecology')!
    .run();
  assert.equal(
    h.render().fields.find((f) => f.id === 'configuration-stomatal-scheme')!.value,
    'BALL_BERRY',
  );
  assert.deepEqual(h.changes, []);
  h.props.overlay.physics = { DEF_USE_MEDLYNST: '.true.', DEF_USE_WUEST: '.true.' };
  assert.equal(h.render().fields.find((f) => f.id === 'configuration-stomatal-scheme')!.value, '');
});
