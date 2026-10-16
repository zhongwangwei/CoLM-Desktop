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
    runHost: undefined as string | undefined,
    process: undefined as string | undefined,
    engine: undefined as string | undefined,
    hideProcesses: false,
    modalActions: undefined as UiAction[] | undefined,
    removedCaseKeys: [] as string[],
    onReady: undefined as ((ready: boolean) => void) | undefined,
    onCaseChange: undefined as
      undefined | ((patch: helpers.CaseOptionPatch) => void | Promise<void>),
    caseValues: undefined as Record<string, string> | undefined,
    subgrid: undefined as string | undefined,
    caseScope: undefined as 'site' | 'spatial' | 'unknown' | undefined,
    kernelMacros: undefined as readonly string[] | undefined,
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
  const picks: {
    command: string;
    args: Record<string, unknown>;
    resolve: (value: string | null) => void;
    reject: (reason: Error) => void;
  }[] = [];
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
    RemotePathBrowser: 'RemotePathBrowser',
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
    invoke: (command: string, args: Record<string, unknown>) =>
      new Promise<string | null>((resolve, reject) =>
        picks.push({ command, args, resolve, reject }),
      ),
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
  return {
    render,
    settle,
    props,
    requests,
    changes,
    response,
    nodes,
    picks,
    unmount: () => effects.forEach((effect) => effect.cleanup?.()),
  };
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

test('common soil settings expose fitting only for spatial simulations', async () => {
  for (const scope of ['site', 'spatial'] as const) {
    const h = harness(),
      patches: helpers.CaseOptionPatch[] = [];
    h.props.caseScope = scope;
    h.props.modalActions = [];
    h.props.onCaseChange = (patch) => {
      patches.push(patch);
    };
    h.props.caseValues = {};
    for (const [name, kind, initial] of [
      ['DEF_USE_SOILPAR_UPS_FIT', 'logical', '.false.'],
      ['DEF_RSS_SCHEME', 'integer', '1'],
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
    let c = await h.settle();
    c.actions.find((a) => a.id === 'configuration-group-soil')!.run();
    c = h.render();
    assert.ok(c.fields.some((f) => f.id === 'configuration-DEF_RSS_SCHEME'));
    const fitting = c.fields.find((f) => f.id === 'configuration-DEF_USE_SOILPAR_UPS_FIT');
    assert.equal(!!fitting, scope === 'spatial');
    if (fitting) {
      await fitting.set('.true.');
      assert.equal(patches.at(-1)?.DEF_USE_SOILPAR_UPS_FIT, '.true.');
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
  const name = 'DEF_SOIL_REFL_SCHEME';
  h.props.fields.push({
    name,
    section: '水热过程',
    kind: 'integer',
    default: '0',
    values: ['0', '1'],
    derived: false,
  });
  h.props.process = undefined;
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
  h.render()
    .actions.find((a) => a.id === 'configuration-group-soil')!
    .run();
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

test('CN initial file hides missing sentinels and explains the runtime default while preserving paths', async () => {
  for (const value of ['null', "'NONE'", '', "'/runtime/custom.nc'"]) {
    const h = harness();
    const name = 'DEF_file_cn_init';
    h.props.process = 'DEF_USE_BGC';
    h.props.fields.push({
      name,
      kind: 'character',
      section: '生态与生地化',
      default: 'null',
      values: [],
      derived: false,
    });
    h.props.overlay.physics[name] = value;
    h.render();
    const c = await h.settle();
    assert.equal(
      c.fields.find((f) => f.id === `configuration-${name}`)!.value,
      value.includes('custom.nc') ? '/runtime/custom.nc' : '',
    );
    assert.ok(JSON.stringify(h.nodes).includes('cnsteadystate.nc'));
  }
});

test('stomatal restore is visible only for an explicit override', async () => {
  const h = harness();
  h.props.overlay.physics = {};
  h.render();
  await h.settle();
  h.render()
    .actions.find((a) => a.id === 'configuration-group-ecology')!
    .run();
  let action = h.render().actions.find((a) => a.id === 'configuration-stomatal-reset')!;
  assert.equal(action.visible, false);
  assert.equal(action.enabled, false);
  h.props.overlay.physics.DEF_USE_MEDLYNST = '.true.';
  action = h.render().actions.find((a) => a.id === 'configuration-stomatal-reset')!;
  assert.equal(action.visible, true);
  assert.match(action.label, /恢复默认/);
});

test('PFT restore is hidden for inherited values and shown for configured values', async () => {
  const h = harness();
  h.render();
  let c = await h.settle();
  c.actions.find((a) => a.id === 'configuration-pft-toggle')!.run();
  c = h.render();
  c.fields.find((f) => f.id === 'configuration-pft-index')!.set('1');
  h.render();
  const response = h.response();
  const entry: helpers.ConfigurationResult['pft_parameters'][number] = {
    name: 'test_pft',
    parameter_id: 'pft:test',
    label_zh: '测试参数',
    label_en: 'Test parameter',
    unit: null,
    kind: 'real',
    default: '1',
    value: '1',
    configured_value: null,
    scope_kind: 'pft',
    scope_label: 'PFT 1',
    allowed_values: [],
    mode: 'editable',
    pending: false,
  };
  response.pft_parameters = [entry];
  h.requests.at(-1)!.resolve(response);
  await new Promise((resolve) => setTimeout(resolve, 0));
  c = h.render();
  const id = 'configuration-pft-1-test_pft-reset';
  assert.equal(c.actions.find((a) => a.id === id)!.visible, false);
  entry.configured_value = '2';
  c = h.render();
  assert.equal(c.actions.find((a) => a.id === id)!.visible, true);
  assert.match(c.actions.find((a) => a.id === id)!.label, /恢复默认/);
});

function cnFileHarness() {
  const h = harness();
  h.props.process = 'DEF_USE_BGC';
  h.props.fields.push({
    name: 'DEF_file_cn_init',
    section: '生态与生地化',
    kind: 'character',
    default: "'null'",
    values: [],
    derived: false,
  });
  h.props.overlay.physics.DEF_USE_CN_INIT = '.true.';
  return h;
}
const browseId = 'configuration-DEF_file_cn_init-browse';
test('CN file picker stages a quoted path, cancels without edits and reports errors', async () => {
  const h = cnFileHarness();
  const patches: helpers.CaseOptionPatch[] = [];
  h.props.modalActions = [];
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  h.render();
  let c = await h.settle();
  const action = c.actions.find((a) => a.id === browseId)!;
  const pending = action.run();
  assert.deepEqual(h.picks[0].args, { key: 'DEF_file_cn_init', filter: null });
  assert.equal(h.picks[0].command, 'pick_file');
  c = h.render();
  assert.equal(c.actions.find((a) => a.id === browseId)!.enabled, false);
  await assert.rejects(async () => action.run());
  h.picks[0].resolve("/data/O'Brien/cn.nc");
  await pending;
  assert.equal(patches[0].DEF_file_cn_init, "'/data/O''Brien/cn.nc'");
  c = h.render();
  let next = c.actions.find((a) => a.id === browseId)!.run();
  h.picks.at(-1)!.resolve(null);
  await next;
  assert.equal(patches.length, 1);
  c = h.render();
  next = c.actions.find((a) => a.id === browseId)!.run();
  h.picks.at(-1)!.reject(new Error('file dialog unavailable'));
  await next;
  h.render();
  assert.match(JSON.stringify(h.nodes), /file dialog unavailable/);
  assert.equal(patches.length, 1);
});
test('CN picker drops responses after source changes, locking, or dialog unmount', async () => {
  for (const mutation of ['source', 'locked', 'unmount']) {
    const h = cnFileHarness();
    h.render();
    const c = await h.settle();
    const pending = c.actions.find((a) => a.id === browseId)!.run();
    if (mutation === 'source')
      h.props.source = { kind: 'draft', id: 'new-source', generation: 0, overlay: h.props.overlay };
    if (mutation === 'locked') h.props.disabled = true;
    if (mutation === 'unmount') h.unmount();
    else h.render();
    h.picks[0].resolve('/data/cn.nc');
    await pending;
    assert.equal(h.changes.length, 0, mutation);
  }
});
test('scalar rows omit generic applicability disclaimers but retain concrete reasons', async () => {
  const h = harness();
  h.render();
  const result = h.response();
  result.fields.forEach((field) => {
    if (field.mode !== 'disabled')
      field.reason = '适用条件待按所选源码、算例和内核核对；目录默认值仅供参考';
  });
  h.requests.at(-1)!.resolve(result);
  await new Promise((resolve) => setTimeout(resolve, 0));
  h.render();
  const content = JSON.stringify(
    h.nodes.filter((node) => node.props.className === 'model-setting-row'),
  );
  assert.doesNotMatch(content, /适用性待核对|适用条件待按所选源码/);
  assert.match(content, /disabled evidence/);
});
test('ordinary case configuration hides research-only indexed parameters for every subgrid', async () => {
  for (const subgrid of ['IGBP', 'PFT', 'PC']) {
    const h = harness();
    h.props.onCaseChange = () => {};
    h.props.subgrid = subgrid;
    h.render();
    const c = await h.settle();
    assert.ok(!c.actions.some((action) => action.id === 'configuration-pft-toggle'));
  }
});
test('vegetation snow immediately follows the restricted canopy interception selector', async () => {
  const h = harness();
  for (const [name, kind, value] of [
    ['DEF_VEG_SNOW', 'logical', '.false.'],
    ['DEF_Runoff_SCHEME', 'integer', '1'],
    ['DEF_Interception_scheme', 'integer', '1'],
  ])
    h.props.fields.push({
      name,
      kind,
      section: '水热过程',
      default: value,
      values: name === 'DEF_Interception_scheme' ? ['1', '2', '3', '4', '5', '6', '7', '8'] : [],
      derived: false,
    });
  h.render();
  let c = await h.settle();
  c.actions.find((action) => action.id === 'configuration-group-water')!.run();
  c = h.render();
  const index = c.fields.findIndex((field) => field.id === 'configuration-DEF_Interception_scheme');
  assert.ok(index >= 0);
  assert.equal(c.fields[index + 1].id, 'configuration-DEF_VEG_SNOW');
  assert.deepEqual(
    c.fields[index].options?.map((option) => option.value),
    ['1', '8'],
  );
});
test('research plant scope follows PFT or PC and known noncrop kernels exclude crop types', async () => {
  const h = harness();
  h.props.subgrid = 'PFT';
  h.render();
  let c = await h.settle();
  c.actions.find((a) => a.id === 'configuration-pft-toggle')!.run();
  c = h.render();
  assert.ok(!c.fields.some((f) => f.id === 'configuration-pft-kind'));
  const types = c.fields.find((f) => f.id === 'configuration-pft-index')!;
  assert.equal(types.options?.length, 79);
  types.set('78');
  h.render();
  assert.deepEqual(h.requests.at(-1)!.args.pftSelection, { kind: 'pft', index: 78 });
  h.props.subgrid = 'PC';
  h.render();
  assert.deepEqual(h.requests.at(-1)!.args.pftSelection, { kind: 'pc-pft', index: 78 });
  h.props.kernelMacros = [];
  c = h.render();
  assert.equal(h.requests.at(-1)!.args.pftSelection, null);
  const restricted = c.fields.find((f) => f.id === 'configuration-pft-index')!;
  assert.equal(restricted.value, '');
  assert.equal(restricted.options?.length, 16);
  assert.throws(() => restricted.set('16'));
  const select = h.nodes.find((n) => n.type === 'Select' && n.props.value === '');
  assert.ok(select);
  h.props.kernelMacros = ['CROP'];
  c = h.render();
  assert.equal(c.fields.find((f) => f.id === 'configuration-pft-index')!.options?.length, 79);
  for (const scheme of ['IGBP', 'USGS']) {
    h.props.subgrid = scheme;
    c = h.render();
    assert.ok(!c.actions.some((a) => a.id === 'configuration-pft-toggle'));
    assert.equal(h.requests.at(-1)!.args.pftSelection, null);
  }
});

test('CN picker replaces an uncommitted text draft in the research editor', async () => {
  const h = cnFileHarness();
  h.render();
  await h.settle();
  const input = h.nodes.find((node) => node.type === 'Input')!;
  (input.props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: 'unsaved.nc' },
  });
  let c = h.render();
  assert.equal(
    c.fields.find((field) => field.id === 'configuration-DEF_file_cn_init')!.value,
    'unsaved.nc',
  );
  const pending = c.actions.find((action) => action.id === browseId)!.run();
  h.picks[0].resolve('/data/selected.nc');
  await pending;
  h.render();
  c = await h.settle();
  assert.equal(
    c.fields.find((field) => field.id === 'configuration-DEF_file_cn_init')!.value,
    '/data/selected.nc',
  );
});

test('urban hyperspectral albedo has no null text and uses the same guarded file picker', async () => {
  const h = harness();
  const name = 'DEF_HighResUrban_albedo';
  h.props.fields.push({
    name,
    section: '地表数据',
    kind: 'character',
    default: "'null'",
    values: [],
    derived: false,
  });
  h.props.hideProcesses = true;
  h.props.onCaseChange = () => {};
  h.props.caseValues = { DEF_URBAN_RUN: '.false.' };
  h.render();
  let c = await h.settle();
  c.actions.find((a) => a.id === 'configuration-group-other')!.run();
  c = h.render();
  assert.equal(c.fields.find((f) => f.id === `configuration-${name}`)!.value, '');
  const rows = h.nodes.filter((n) => n.props.className === 'model-setting-row');
  assert.match(JSON.stringify(rows), /No default file|未提供默认文件/);
  const patches: helpers.CaseOptionPatch[] = [];
  h.props.modalActions = [];
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  c = h.render();
  const picked = c.actions.find((a) => a.id === `configuration-${name}-browse`)!.run();
  assert.deepEqual(h.picks[0].args, { key: name, filter: null });
  h.picks[0].resolve("/data/O'Brien/urban.nc");
  await picked;
  assert.equal(patches.at(-1)![name], "'/data/O''Brien/urban.nc'");
});

test('catchment lateral flow settings stay accessible without grid river routing', async () => {
  const h = harness();
  h.props.hideProcesses = true;
  h.props.onCaseChange = () => {};
  h.props.kernelMacros = ['CATCHMENT'];
  h.props.caseValues = { DEF_USE_GridRiverLakeFlow: '.false.' };
  const names = [
    'DEF_ElementNeighbour_file',
    'DEF_USE_EstimatedRiverDepth',
    'DEF_Reservoir_Method',
  ];
  for (const name of names)
    h.props.fields.push({
      name,
      section: '河道与水库',
      kind: 'character',
      default: "'null'",
      values: [],
      derived: false,
    });
  h.render();
  let c = await h.settle();
  c.actions.find((a) => a.id === 'configuration-group-water')!.run();
  c = h.render();
  for (const name of names)
    assert.ok(
      c.fields.some((f) => f.id === `configuration-${name}`),
      name,
    );
  h.props.kernelMacros = ['GRIDBASED'];
  c = h.render();
  for (const name of names)
    assert.ok(!c.fields.some((f) => f.id === `configuration-${name}`), name);
});
test('LAI flags and years have one general vegetation entry instead of a duplicate LULCC entry', async () => {
  const h = harness();
  h.props.process = 'DEF_USE_LULCC';
  h.props.onCaseChange = () => {};
  h.props.caseValues = { DEF_USE_LULCC: '.true.', DEF_LAI_CHANGE_YEARLY: '.true.' };
  const names = [
    'DEF_LAI_MONTHLY',
    'DEF_LAI_CHANGE_YEARLY',
    'DEF_LAI_START_YEAR',
    'DEF_LAI_END_YEAR',
  ];
  for (const name of names)
    h.props.fields.push({
      name,
      section: '地表数据',
      kind: 'integer',
      default: '2000',
      values: [],
      derived: false,
    });
  h.render();
  let c = await h.settle();
  for (const name of names)
    assert.ok(!c.fields.some((f) => f.id === `configuration-${name}`), name);
  h.props.process = undefined;
  h.props.hideProcesses = true;
  c = h.render();
  c.actions.find((a) => a.id === 'configuration-group-subgrid')!.run();
  c = h.render();
  for (const name of names)
    assert.equal(c.fields.filter((f) => f.id === `configuration-${name}`).length, 1, name);
});

test('general soil resistance precedes albedo and follows structure defaults without losing custom values', async () => {
  const h = harness();
  h.props.hideProcesses = true;
  h.props.onCaseChange = () => {};
  h.props.caseValues = { DEF_USE_Campbell_SOIL_MODEL: '.false.' };
  for (const name of ['DEF_SOIL_REFL_SCHEME', 'DEF_RSS_SCHEME'])
    h.props.fields.push({
      name,
      section: '水热过程',
      kind: 'integer',
      default: name === 'DEF_RSS_SCHEME' ? '1' : '2',
      values: ['0', '1', '2', '3', '4', '5'],
      derived: false,
    });
  h.render();
  let c = await h.settle();
  c.actions.find((a) => a.id === 'configuration-group-soil')!.run();
  for (const [subgrid, campbell, expected] of [
    ['IGBP', '.false.', '0'],
    ['USGS', '.false.', '0'],
    ['IGBP', '.true.', '1'],
    ['PFT', '.false.', '1'],
    ['PC', '.false.', '1'],
  ]) {
    h.props.subgrid = subgrid;
    h.props.caseValues.DEF_USE_Campbell_SOIL_MODEL = campbell;
    h.render();
    const response = h.response();
    response.fields = response.fields.map((f) => ({
      ...f,
      pending: false,
      mode: f.name === 'DEF_RSS_SCHEME' && expected === '0' ? 'disabled' : 'editable',
    }));
    h.requests.at(-1)!.resolve(response);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    c = h.render();
    const rss = c.fields.findIndex((f) => f.id === 'configuration-DEF_RSS_SCHEME');
    assert.ok(rss >= 0);
    assert.equal(c.fields[rss].value, expected, subgrid + campbell);
    assert.equal(c.fields[rss].disabled, expected === '0');
    if (expected === '0') assert.throws(() => c.fields[rss].set('1'));
    assert.equal(c.fields[rss + 1].id, 'configuration-DEF_SOIL_REFL_SCHEME');
  }
  h.props.caseValues.DEF_RSS_SCHEME = '3';
  for (const [mode, expected] of [
    ['PFT', '3'],
    ['IGBP', '0'],
    ['PC', '3'],
  ]) {
    h.props.subgrid = mode;
    h.render();
    c = await h.settle();
    assert.equal(c.fields.find((f) => f.id === 'configuration-DEF_RSS_SCHEME')!.value, expected);
    assert.equal(h.props.caseValues.DEF_RSS_SCHEME, '3');
  }
  h.props.caseValues.DEF_RSS_SCHEME = '0';
  h.render();
  c = await h.settle();
  assert.equal(c.fields.find((f) => f.id === 'configuration-DEF_RSS_SCHEME')!.value, '0');
  assert.equal(h.changes.length, 0);
});

test('forcing processing stays out of general model controls for case editing', async () => {
  const h = harness();
  h.props.onCaseChange = () => {};
  h.props.hideProcesses = true;
  for (const name of ['DEF_Forcing_Interp_Method', 'DEF_USE_Forcing_Downscaling', 'DEF_DS_topo'])
    h.props.fields.push({
      name,
      section: '强迫场',
      kind: 'integer',
      default: '1',
      values: [],
      derived: false,
    });
  h.render();
  const c = await h.settle();
  assert.ok(!c.actions.some((a) => a.id === 'configuration-group-forcing'));
});

test('VIC files expose preset paths and guarded browsing only for the selected source', async () => {
  for (const grid of [false, true]) {
    const h = harness();
    const name = grid ? 'DEF_file_VIC_OPT' : 'DEF_file_VIC_para';
    const other = grid ? 'DEF_file_VIC_para' : 'DEF_file_VIC_OPT';
    h.props.fields.push(
      ...[name, other].map((name) => ({
        name,
        section: '水热过程',
        kind: 'character',
        default: "'null'",
        values: [],
        derived: false,
      })),
    );
    h.props.overlay.physics = {
      DEF_Runoff_SCHEME: '1',
      DEF_VIC_OPT: grid ? '.true.' : '.false.',
      DEF_dir_runtime: "'/runtime/'",
    };
    h.render();
    await h.settle();
    h.render()
      .actions.find((a) => a.id === 'configuration-group-water')!
      .run();
    let c = h.render();
    assert.equal(
      c.fields.find((f) => f.id === `configuration-${name}`)!.value,
      grid ? '/runtime/vic/vic_para.nc' : '/runtime/vic/vic_para.txt',
    );
    assert.ok(!c.fields.some((f) => f.id === `configuration-${other}`));
    const rows = h.nodes.filter((n) => n.props.className === 'model-setting-row');
    assert.ok(!JSON.stringify(rows).includes('null'));
    const pick = c.actions.find((a) => a.id === `configuration-${name}-browse`)!.run();
    h.picks[0].resolve("/data/O'Brien/vic.txt");
    await pick;
    assert.equal(h.props.overlay.physics[name], "'/data/O''Brien/vic.txt'");
    c = h.render();
    c.actions.find((a) => a.id === `configuration-${name}-preset`)!.run();
    assert.equal(h.props.overlay.physics[name], undefined);
    h.props.overlay.physics.DEF_Runoff_SCHEME = '3';
    c = h.render();
    assert.ok(!c.fields.some((f) => f.id === `configuration-${name}`));
    h.props.overlay.physics.DEF_Runoff_SCHEME = '1';
    h.props.engine = 'fortran';
    c = h.render();
    assert.ok(!c.fields.some((f) => f.id === `configuration-${name}`));
  }
});

for (const outcome of ['select', 'cancel', 'switch', 'lock', 'unmount']) {
  test(`remote configuration file browser preserves ${outcome} guards`, async () => {
    const h = cnFileHarness();
    h.props.runHost = 'compute';
    h.props.modalActions = [];
    const patches: helpers.CaseOptionPatch[] = [];
    h.props.onCaseChange = (patch) => {
      patches.push(patch);
    };
    h.render();
    const c = await h.settle();
    const pending = c.actions.find((a) => a.id === browseId)!.run();
    h.render();
    const picker = h.nodes.find((node) => node.type === 'RemotePathBrowser')!;
    assert.equal(picker.props.host, 'compute');
    assert.equal(picker.props.directory, false);
    assert.equal(h.picks.length, 0);
    if (outcome === 'switch') {
      h.props.runHost = 'other';
      h.render();
    }
    if (outcome === 'lock') {
      h.props.disabled = true;
      h.render();
    }
    if (outcome === 'unmount') h.unmount();
    else if (outcome === 'cancel') (picker.props.onClose as () => void)();
    else (picker.props.onSelect as (path: string) => void)("/server/O'Brien/cn.nc");
    await pending;
    assert.equal(patches.length, outcome === 'select' ? 1 : 0);
    if (outcome === 'select') assert.equal(patches[0].DEF_file_cn_init, "'/server/O''Brien/cn.nc'");
  });
}
