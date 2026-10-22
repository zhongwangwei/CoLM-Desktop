import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import { ActionButtonStub, treeActions, treeReact } from '../../lib/rendered-tree';

type Contract = {
  fields: { id: string; value: unknown; set: (v: string) => void }[];
  actions: { id: string; run: () => Promise<unknown> }[];
};
function page(total = false, snow = true) {
  const values: unknown[] = [],
    calls: { name: string; args: Record<string, unknown> }[] = [];
  let cursor = 0,
    contract: Contract;
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
    }),
    usePageContract: (value: Contract) => {
      contract = value;
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
    casePreparationOptions: () => ({ classification: 'IGBP' }),
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
  const render = () => {
    cursor = 0;
    const tree = component();
    return { ...contract, actions: [...contract.actions, ...treeActions(tree)] };
  };
  return {
    render,
    calls,
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
