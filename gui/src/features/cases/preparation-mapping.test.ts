import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { EMPTY_PREP_FORM, type PrepDraft } from './preparation-draft';
import {
  preparedDatasetDestinations,
  preparedSiteDestination,
  preparedSiteNameValid,
} from './case-model';
import ts from 'typescript';
import { ActionButtonStub, treeActions, treeReact } from '../../lib/rendered-tree';

type Contract = {
  fields: { id: string; value: unknown; set: (v: string) => void }[];
  actions: { id: string; run: () => Promise<unknown> }[];
};
type RenderedSelect = {
  value: string;
  options: string[];
  change: (value: string) => void;
};
/** 从假 React 的树里收集真正渲染出来的 `Select`（不是 page contract 里声明的字段）。 */
function treeSelects(node: unknown): RenderedSelect[] {
  if (!node || typeof node !== 'object') return [];
  if (Array.isArray(node)) return node.flatMap(treeSelects);
  const element = node as { type: unknown; props: Record<string, unknown> };
  const own =
    element.type === 'Select'
      ? [
          {
            value: String(element.props.value),
            options: (() => {
              const found: string[] = [];
              const visit = (value: unknown) => {
                if (!value || typeof value !== 'object') return;
                if (Array.isArray(value)) return value.forEach(visit);
                const child = value as { type: unknown; props?: Record<string, unknown> };
                if (child.type === 'option') found.push(String(child.props?.value));
                visit(child.props?.children);
              };
              visit(element.props.children);
              return found;
            })(),
            change: (value: string) =>
              (element.props.onChange as (event: { target: { value: string } }) => void)({
                target: { value },
              }),
          },
        ]
      : [];
  return [...own, ...treeSelects(element.props?.children)];
}
function page(total = false, snow = true, draft: PrepDraft | null = null) {
  let preparation = { classification: 'IGBP', mode: 'igbp', crop: false };
  const values: unknown[] = [],
    calls: { name: string; args: Record<string, unknown> }[] = [];
  let cursor = 0,
    contract: Contract,
    navigationGuard: (() => string | null) | null = null;
  const names = [
    'Tair',
    'Qair',
    'Psurf',
    total ? 'Precip' : 'Rainf',
    'Wind_E',
    'Wind_N',
    'SWdown',
    'LWdown',
  ];
  const context: Record<string, unknown> = {
    React: treeReact,
    ActionButton: ActionButtonStub,
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in values)) values[i] = initial;
      return [
        values[i],
        (next: unknown) => {
          values[i] = typeof next === 'function' ? next(values[i]) : next;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const i = cursor++;
      if (!(i in values)) values[i] = { current: initial };
      return values[i];
    },
    useApp: () => ({
      state: { language: 'en', expert: false },
      notify: () => {},
      navigate: () => {},
      registerCloseGuard: () => () => {},
    }),
    usePageContract: (value: Contract) => {
      contract = value;
    },
    useNavigationGuard: (guard: () => string | null) => {
      navigationGuard = guard;
    },
    invoke: async (name: string, args: Record<string, unknown>) => {
      calls.push({ name, args });
      if (name.startsWith('probe_'))
        return {
          ...(name === 'probe_forcing_table'
            ? { columns: [...names, ...(snow ? ['Snowf'] : [])].map((name) => ({ name })) }
            : { variables: [...names, ...(snow ? ['Snowf'] : [])] }),
          suggest_dst: '/output',
          slots: names.map((sourceName, i) => ({
            index: i + 1,
            meaning: sourceName,
            ...(name === 'probe_forcing_table' ? { column: sourceName } : { guessed: sourceName }),
            units: 'unit',
            wants: 'unit',
          })),
        };
      if (name === 'convert_forcing_table')
        return [{ staged_path: '/staged', final_path: '/final', site: 'site' }];
      return { unresolved: 0 };
    },
    requiredNumber: (v: string) => Number(v),
    preparedDestination: () => '/output',
    casePreparationOptions: () => preparation,
    // 准备页从准备草稿模块导入的纯函数（D-05）；测试里不恢复草稿、不写存储。
    EMPTY_PREP_FORM,
    readPrepDraft: () => draft,
    prepDraftHasInput: () => false,
    writePrepDraft: () => {},
    clearPrepDraft: () => {},
    preparedSiteNameValid,
    preparedSiteDestination,
    preparedDatasetDestinations,
    preparationSiteModes: (crop: boolean) =>
      crop
        ? ['pft', 'pc', 'urban-pft', 'urban-pc']
        : ['igbp', 'usgs', 'pft', 'pc', 'urban-igbp', 'urban-usgs', 'urban-pft', 'urban-pc'],
  };
  for (const name of ['Button', 'Card', 'Input', 'Select', 'Field', 'Advanced', 'Notice'])
    context[name] = name;
  const source = readFileSync(new URL('./PreparationPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const js = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const component = new Function(...Object.keys(context), `${js};return PreparationPage;`)(
    ...Object.values(context),
  );
  let tree: unknown = null;
  const render = () => {
    cursor = 0;
    tree = component();
    return { ...contract, actions: [...contract.actions, ...treeActions(tree)] };
  };
  return {
    render,
    calls,
    enableCrop: () => {
      preparation = { classification: 'IGBP', mode: 'pft', crop: true };
    },
    navigationReason: () => {
      render();
      return navigationGuard ? navigationGuard() : null;
    },
    /** 真正渲染到 DOM 的可选框（区别于 page contract 里只声明、JSX 被过滤掉的字段）。 */
    selects: () => {
      render();
      return treeSelects(tree);
    },
    set: (id: string, value: string) => {
      const f = render().fields.find((f) => f.id === id);
      assert.ok(f, id);
      f.set(value);
    },
    run: async (id: string) => {
      const a = render().actions.find((a) => a.id === id);
      assert.ok(a, id);
      return a.run();
    },
  };
}
for (const mode of ['netcdf', 'table', 'gaps']) {
  test(`${mode} forwards explicitly confirmed snow components exactly once`, async () => {
    const p = page();
    p.set('prep-mode', mode);
    p.set('prep-src', '/input');
    await p.run('prep-inspect');
    p.set('prep-slot-4-also_add', 'Snowf, Snowf, Rainf');
    await p.run('prep-execute');
    const call = p.calls.find(
      (c) =>
        c.name ===
        { netcdf: 'convert_forcing', table: 'convert_forcing_table', gaps: 'repair_forcing' }[mode],
    );
    assert.ok(call);
    const slots = call.args.slots as { index: number; also_add: string[] }[];
    assert.deepEqual(slots.find((s) => s.index === 4)?.also_add, ['Snowf']);
    if (mode === 'table') {
      const finish = p.render().actions.find((a) => a.id.startsWith('prep-finish-'));
      assert.ok(finish);
      await finish.run();
      const repair = p.calls.find((c) => c.name === 'repair_forcing')!;
      assert.deepEqual(
        (repair.args.slots as { index: number; name: string; also_add: string[] }[]).find(
          (s) => s.index === 4,
        ),
        { index: 4, name: 'Precip', units: 'unit', also_add: [] },
      );
    }
  });
}
test('total precipitation is not automatically combined with snow and changing the primary resets additions', async () => {
  const p = page(true);
  p.set('prep-src', '/input');
  await p.run('prep-inspect');
  assert.equal(p.render().fields.find((f) => f.id === 'prep-slot-4-also_add')?.value, '');
  p.set('prep-slot-4-also_add', 'Snowf');
  p.set('prep-slot-4-name', 'PrecipTotal');
  assert.equal(p.render().fields.find((f) => f.id === 'prep-slot-4-also_add')?.value, '');
});

for (const mode of ['netcdf', 'table', 'gaps']) {
  test(`${mode} preselects detected snow when converting rainfall without manual edits`, async () => {
    const p = page();
    p.set('prep-mode', mode);
    p.set('prep-src', '/input');
    await p.run('prep-inspect');
    assert.equal(p.render().fields.find((f) => f.id === 'prep-slot-4-also_add')?.value, 'Snowf');
    await p.run('prep-execute');
    const call = p.calls.find(
      (c) =>
        c.name ===
        { netcdf: 'convert_forcing', table: 'convert_forcing_table', gaps: 'repair_forcing' }[mode],
    );
    assert.ok(call);
    const slots = call.args.slots as { index: number; also_add: string[] }[];
    assert.deepEqual(slots.find((s) => s.index === 4)?.also_add, ['Snowf']);
  });
}

test('rainfall without detected snow stays unchanged', async () => {
  const p = page(false, false);
  p.set('prep-src', '/input');
  await p.run('prep-inspect');
  assert.equal(p.render().fields.find((f) => f.id === 'prep-slot-4-also_add')?.value, '');
});

test('pending preparation input blocks navigation until it is generated', async () => {
  const p = page();
  assert.equal(p.navigationReason(), null);
  p.set('prep-src', '/input');
  assert.match(p.navigationReason() ?? '', /未生成|not been generated/);
  await p.run('prep-inspect');
  // 检查输入只是把探测结果写回 result，不算"已生成"：还没写出产物时离页仍要拦。
  assert.match(
    p.navigationReason() ?? '',
    /未生成|not been generated/,
    'inspecting alone must not release the navigation guard',
  );
  // 被拦下时必须有明确出口：放弃未生成的输入后允许离开（不能只能生成一次或重启应用）。
  await p.run('prep-discard-unfinished');
  assert.equal(p.navigationReason(), null, 'discarding unfinished input releases the guard');
  // 重新改一次并生成：产物已生成，用户的目的已达到，不再拦截“用于新算例”。
  p.set('prep-src', '/input2');
  await p.run('prep-inspect');
  await p.run('prep-execute');
  assert.equal(p.navigationReason(), null);
});

test('flag-like free text and out-of-range coordinates are rejected with field names', async () => {
  const p = page();
  p.set('prep-src', '--oops');
  p.set('prep-dst', '/out');
  await assert.rejects(p.run('prep-execute'), /不能以 -- 开头|must not start with --/);
  const site = page();
  site.set('prep-mode', 'site');
  site.set('prep-dst', '/out/CN-Cng_site.nc');
  site.set('prep-lon', '999');
  await assert.rejects(site.run('prep-execute'), /经度|Longitude/);
});

test('site generation exposes and forwards the land-cover/vegetation scheme', async () => {
  const p = page();
  p.set('prep-mode', 'site');
  // 方案选择必须是**真正渲染出来的**控件，不能只注册在 page contract 里（GUI 复核 D-04）。
  const scheme = p.selects().find((select) => select.options.includes('usgs'));
  assert.ok(scheme, 'the land-cover/vegetation scheme select is rendered for site creation');
  assert.equal(scheme.value, 'igbp', 'defaults to the page profile when no case is open');
  scheme.change('usgs');
  assert.equal(p.render().fields.find((f) => f.id === 'prep-site-mode')?.value, 'usgs');
  p.set('prep-dst', '/out/CN-Cng_site.nc');
  p.set('prep-lon', '10');
  p.set('prep-lat', '20');
  await p.run('prep-execute');
  const call = p.calls.find((c) => c.name === 'make_site');
  assert.ok(call, 'make_site was invoked');
  assert.equal(call.args.mode, 'usgs');
});

for (const [key, value] of [
  ['name', 'Precip'],
  ['units', 'mm/s'],
  ['also_add', ''],
]) {
  test(`changing split-table slot ${key} requires splitting again`, async () => {
    const p = page();
    p.set('prep-mode', 'table');
    p.set('prep-src', '/input');
    await p.run('prep-inspect');
    await p.run('prep-execute');
    assert.ok(p.render().actions.some((a) => a.id === 'prep-finish-0'));
    p.set(`prep-slot-4-${key}`, value!);
    assert.ok(!p.render().actions.some((a) => a.id === 'prep-finish-0'));
    await p.run('prep-execute');
    await p.run('prep-finish-0');
    assert.equal(p.calls.filter((c) => c.name === 'convert_forcing_table').length, 2);
  });
}

test('unchanged mapping and repair-only inputs preserve split-table completion', async () => {
  const p = page();
  p.set('prep-mode', 'table');
  p.set('prep-src', '/input');
  await p.run('prep-inspect');
  await p.run('prep-execute');
  p.set('prep-slot-4-name', 'Rainf');
  p.set('prep-slot-4-units', 'unit');
  p.set('prep-slot-4-also_add', 'Snowf');
  p.set('prep-batch-era5', '/era5');
  await p.run('prep-finish-0');
  assert.equal(p.calls.filter((c) => c.name === 'convert_forcing_table').length, 1);
});

for (const restored of [false, true]) {
  test(`${restored ? 'restored' : 'selected'} site scheme matches submitted mode after enabling CROP`, async () => {
    const p = page(
      false,
      true,
      restored
        ? {
            mode: 'site',
            form: {
              ...EMPTY_PREP_FORM,
              site_mode: 'igbp',
              dst: '/out/test_site.nc',
              lon: '10',
              lat: '20',
            },
          }
        : null,
    );
    if (!restored) {
      p.set('prep-mode', 'site');
      p.set('prep-site-mode', 'igbp');
      p.set('prep-dst', '/out/test_site.nc');
      p.set('prep-lon', '10');
      p.set('prep-lat', '20');
    }
    p.enableCrop();
    const scheme = p.selects().find((select) => select.options.includes('pft'));
    assert.ok(scheme);
    assert.equal(scheme.value, 'pft');
    assert.ok(!scheme.options.includes('igbp'));
    await p.run('prep-execute');
    const call = p.calls.find((c) => c.name === 'make_site');
    assert.ok(call);
    assert.equal(call.args.mode, scheme.value);
    assert.equal(call.args.crop, true);
  });
}
