import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { cnInitialStatePatch, type CaseOptionPatch } from '../versions/configuration-options';
import type { UiAction } from '../../lib/guide';

function dialog(process = 'DEF_USE_BGC') {
  const nodes: { type: string; props: Record<string, unknown>; children: unknown[] }[] = [],
    slots: unknown[] = [];
  let cursor = 0,
    setup: (() => () => void) | undefined,
    cleanup: (() => void) | undefined;
  const saved: CaseOptionPatch[] = [];
  let closed = 0,
    fail = false;
  const props = {
    process,
    fields: [],
    catalog: [],
    version: {
      version: { id: 'official', revision: 1 },
      overlay: { physics: {}, parameters: [], process_files: [] },
    },
    caseDir: '/case',
    subgrid: 'PFT',
    environment: {
      domain: 'site',
      caseValues: { DEF_USE_BGC: '.true.', DEF_file_cn_init: "'original.nc'" } as Record<
        string,
        string
      >,
    },
    disabled: false,
    onSave: async (patch: CaseOptionPatch) => {
      if (fail) throw new Error('save failed');
      saved.push(patch);
    },
    onClose: () => {
      closed++;
    },
  };
  const context = {
    cnInitialStatePatch,
    React: {
      createElement: (type: string, p: Record<string, unknown>, ...children: unknown[]) => {
        nodes.push({ type, props: p, children });
        return null;
      },
    },
    Dialog: 'Dialog',
    ConfigurationOptions: 'ConfigurationOptions',
    Button: 'Button',
    Notice: 'Notice',
    setupHandledKeys: ['DEF_USE_BGC', 'DEF_TUNING_CROP_PLANTING_DAY'],
    spatialOnlySchemeKeys: ['DEF_SOLO_PFT', 'DEF_FAST_PC'],
    useApp: () => ({ state: { language: 'zh' } }),
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (next: unknown) => {
          slots[i] = typeof next === 'function' ? next(slots[i]) : next;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: initial };
      return slots[i];
    },
    useEffect: (fn: () => () => void) => {
      if (!setup) {
        setup = fn;
        cleanup = fn();
      }
    },
  };
  const source = readFileSync(new URL('./ModelSettings.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(context), `${code};return ProcessSettingsDialog;`)(
    ...Object.values(context),
  );
  const render = () => {
    cursor = 0;
    nodes.length = 0;
    Component(props);
    return nodes.find((n) => n.type === 'ConfigurationOptions')!.props as {
      onCaseChange: (patch: CaseOptionPatch) => void;
      onReady: (ready: boolean) => void;
      modalActions: UiAction[];
      caseValues: Record<string, string>;
      removedCaseKeys: string[];
      excludeKeys: string[];
    };
  };
  const ready = () => {
    render().onReady(true);
    return render();
  };
  const action = (id: string) =>
    render().modalActions.find((a) => a.id === `process-settings-${id}`)!;
  return {
    props,
    render,
    ready,
    action,
    saved,
    nodes,
    get closed() {
      return closed;
    },
    set fail(value: boolean) {
      fail = value;
    },
    strictReplay: () => {
      cleanup?.();
      cleanup = setup!();
    },
    unmount: () => cleanup?.(),
  };
}
test('process dialog stages edits, discards on cancel or escape, and never disables the parent', async () => {
  for (const escape of [false, true]) {
    const h = dialog();
    h.ready().onCaseChange({ DEF_file_cn_init: "'changed.nc'" });
    assert.deepEqual(h.saved, []);
    if (escape)
      (h.nodes.find((n) => n.type === 'Dialog')!.props.onOpenChange as (open: boolean) => void)(
        false,
      );
    else await h.action('cancel').run();
    assert.equal(h.closed, 1);
    assert.deepEqual(h.saved, []);
  }
});

test('tracer chooser stages enable intent without changing the saved switch and cancels atomically', async () => {
  for (const escape of [false, true]) {
    const h = dialog('DEF_USE_TRACER');
    h.props.environment.caseValues.DEF_USE_TRACER = '.false.';
    const child = h.render();
    assert.equal(child.caseValues.DEF_USE_TRACER, '.true.');
    assert.equal(h.action('save').enabled, false);
    child.onCaseChange({ DEF_TRACER_NAMES: "'CH4'", DEF_TRACER_TYPES: "'gas'" });
    assert.deepEqual(h.saved, []);
    if (escape)
      (h.nodes.find((n) => n.type === 'Dialog')!.props.onOpenChange as (open: boolean) => void)(
        false,
      );
    else await h.action('cancel').run();
    assert.equal(h.closed, 1);
    assert.deepEqual(h.saved, []);
    assert.equal(h.props.environment.caseValues.DEF_USE_TRACER, '.false.');
  }
});

test('tracer save commits enable and validated definitions together, preserving the draft on failure', async () => {
  const h = dialog('DEF_USE_TRACER');
  h.props.environment.caseValues.DEF_USE_TRACER = '.false.';
  const changes = {
    DEF_TRACER_NUM: '1',
    DEF_TRACER_NAMES: "'CH4'",
    DEF_TRACER_TYPES: "'gas'",
    DEF_TRACER_PARAM_FILES: "'CH4:standard_ch4_parameter.nml'",
  };
  const child = h.ready();
  child.onCaseChange(changes);
  h.fail = true;
  await h.action('save').run();
  assert.equal(h.closed, 0);
  assert.deepEqual(h.saved, []);
  assert.equal(h.render().caseValues.DEF_TRACER_NAMES, "'CH4'");
  h.fail = false;
  await h.action('save').run();
  assert.deepEqual(h.saved, [{ DEF_USE_TRACER: '.true.', ...changes }]);
  assert.equal(h.closed, 1);
});
test('process dialog saves the latest patch without another render, keeps removals explicit, and supports StrictMode replay', async () => {
  const h = dialog();
  h.ready();
  h.strictReplay();
  const child = h.render();
  child.onCaseChange({ DEF_file_cn_init: "'last-character.nc'" });
  await child.modalActions.find((a) => a.id === 'process-settings-save')!.run();
  assert.deepEqual(h.saved, [{ DEF_file_cn_init: "'last-character.nc'" }]);
  const reset = dialog();
  reset.ready().onCaseChange({ DEF_file_cn_init: null });
  assert.deepEqual(reset.render().removedCaseKeys, ['DEF_file_cn_init']);
  assert.equal(reset.render().caseValues.DEF_file_cn_init, undefined);
  await reset.action('save').run();
  assert.deepEqual(reset.saved, [{ DEF_file_cn_init: null }]);
});
test('process dialog completes unchanged without empty save and stays open on failure or stale locks', async () => {
  const unchanged = dialog();
  unchanged.ready();
  await unchanged.action('save').run();
  assert.equal(unchanged.closed, 1);
  assert.deepEqual(unchanged.saved, []);
  const h = dialog();
  h.ready().onCaseChange({ DEF_file_cn_init: "'changed.nc'" });
  h.fail = true;
  await h.action('save').run();
  assert.equal(h.closed, 0);
  assert.ok(h.nodes.length);
  h.props.disabled = true;
  h.render();
  await assert.rejects(Promise.resolve(h.action('save').run()), /locked/);
  h.props.disabled = false;
  h.unmount();
  await assert.rejects(Promise.resolve(h.action('save').run()), /locked/);
});

test('all process settings use save and cancel without the redundant close button', () => {
  for (const process of ['PC', 'PFT', 'DEF_USE_BGC', 'DEF_USE_TRACER', 'crop']) {
    const h = dialog();
    h.props.process = process;
    h.render();
    assert.equal(h.nodes.find((n) => n.type === 'Dialog')!.props.showClose, false);
    assert.ok(h.action('save'));
    assert.ok(h.action('cancel'));
  }
});

test('CN initial state resolves runtime default for display and persists it even without another edit', async () => {
  const h = dialog();
  Object.assign(h.props.environment.caseValues, {
    DEF_USE_CN_INIT: '.true.',
    DEF_file_cn_init: "'null'",
    DEF_dir_runtime: "'/data/runtime/'",
  });
  assert.equal(h.ready().caseValues.DEF_file_cn_init, "'/data/runtime/cnsteadystate.nc'");
  await h.action('save').run();
  assert.deepEqual(h.saved, [{ DEF_file_cn_init: "'/data/runtime/cnsteadystate.nc'" }]);
});
test('CN default uses the latest enable and runtime edits and preserves explicit paths', async () => {
  const h = dialog();
  h.props.environment.caseValues.DEF_file_cn_init = "'null'";
  const child = h.ready();
  child.onCaseChange({ DEF_USE_CN_INIT: '.true.', DEF_dir_runtime: "'/new runtime'" });
  await child.modalActions.find((a) => a.id === 'process-settings-save')!.run();
  assert.equal(h.saved[0].DEF_file_cn_init, "'/new runtime/cnsteadystate.nc'");
  const custom = dialog();
  custom.props.environment.caseValues.DEF_USE_CN_INIT = '.true.';
  custom.props.environment.caseValues.DEF_dir_runtime = "'/runtime'";
  custom.ready();
  await custom.action('save').run();
  assert.deepEqual(custom.saved, []);
});
