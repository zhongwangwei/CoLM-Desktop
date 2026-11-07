import test from 'node:test';
import { preparedDatasetDestinations } from './case-model';
import { EMPTY_PREP_FORM } from './preparation-draft';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { Button, Card, Notice } from '../../components/ui';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});

type Summary = {
  state: string;
  kind: string;
  output: string | null;
  missing: number | null;
  unresolved: number | null;
  needsExternal: number | null;
  sites: number | null;
  converted: number | null;
  warnings: string[];
  readiness: string | null;
  fromSite?: number;
  fromRaster?: number;
  fromDefault?: number;
  fromLookup?: number;
  siteKind?: string;
  siteMode?: string;
  slots: number | null;
  fields: number | null;
};

/**
 * 页面文件被 preparation-mapping.test.ts 按源码整体编译（只剥 import / export default），所以摘要与
 * 结果视图不能写成 export，否则那个台账 harness 直接 SyntaxError。这里按仓库惯例抽真实函数体执行
 * （任务卡允许的"AST 抽真实渲染/结果处理函数"），顺带证明页面里跑的确实是同一份实现。
 */
const RESULT_PIECES = [
  'PreparationResultSummary',
  'isRecord',
  'finiteNumber',
  'stringField',
  'emptySummary',
  'summarizePreparationResult',
  'kindText',
  'stateText',
  'stateTone',
  'PreparationResultView',
] as const;

function pagePieces() {
  const path = fileURLToPath(new URL('./PreparationPage.tsx', import.meta.url));
  const source = ts.createSourceFile(
    path,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const wanted = new Set<string>(RESULT_PIECES);
  const found = new Set<string>();
  const chunks: string[] = [];
  for (const statement of source.statements) {
    let name: string | null = null;
    if (ts.isFunctionDeclaration(statement) && statement.name) name = statement.name.text;
    else if (ts.isTypeAliasDeclaration(statement)) name = statement.name.text;
    else if (ts.isVariableStatement(statement)) {
      const declaration = statement.declarationList.declarations[0];
      if (declaration && ts.isIdentifier(declaration.name)) name = declaration.name.text;
    }
    if (name && wanted.has(name)) {
      chunks.push(statement.getText(source));
      found.add(name);
    }
  }
  assert.deepEqual(
    RESULT_PIECES.filter((name) => !found.has(name)),
    [],
    'the result summary/render pieces must exist in PreparationPage.tsx',
  );
  const js = ts.transpileModule(chunks.join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const factory = new Function(
    'React',
    'Card',
    'Notice',
    'Button',
    `${js}\nreturn { summarizePreparationResult, PreparationResultView };`,
  ) as (...values: unknown[]) => {
    summarizePreparationResult: (
      result: unknown,
      mode: string,
      gapCheckOnly?: boolean,
    ) => Summary | null;
    PreparationResultView: (props: Record<string, unknown>) => unknown;
  };
  // 新的 Function 里的 JSX 走 React.createElement；组件用界面同一套 Card/Notice/Button 真实渲染。
  return factory({ createElement }, Card, Notice, Button);
}

const { PreparationResultView, summarizePreparationResult } = pagePieces();

const render = (props: Record<string, unknown>) =>
  renderToStaticMarkup(createElement(PreparationResultView as never, props as never));

const SITE_REPORT = {
  path: '/out/site.nc',
  texture: 5,
  texture_name: 'loam',
  bvic: 0.5,
  sand_silt_clay: [40, 40, 20],
  landtype: 9,
  from_site: [],
  from_raster: [],
  from_default: ['soil'],
  from_lookup: [],
  needs_external: ['soil_texture', 'soil_color'],
  site_kind: 'single',
  mode: 'igbp',
  readiness: 'ready_with_rawdata',
  self_contained: false,
};
const GAP_REPORT = {
  timezone_offset_hours: 8,
  timezone_source: 'input',
  timezone_confidence: 'high',
  timezone_conflict: false,
  solar_noon_hour: 12,
  solar_noon_std_hours: 12,
  latitude: 30,
  longitude: 120,
  start_date: '2008-01-01',
  end_date: '2010-12-31',
  missing: 12,
  unresolved: 3,
  needs_era5: true,
  variables: [],
};

test('summary reads only fields the backend actually returns', () => {
  // 字符串结果（转换强迫场/下载 ERA5）只有输出路径。
  assert.deepEqual(summarizePreparationResult('/out/forcing.nc', 'netcdf'), {
    state: 'complete',
    kind: 'forcing',
    output: '/out/forcing.nc',
    missing: null,
    unresolved: null,
    needsExternal: null,
    sites: null,
    converted: null,
    warnings: [],
    readiness: null,
    slots: null,
    fields: null,
  });
  assert.equal(summarizePreparationResult('/out/era5.nc', 'era5')?.kind, 'era5');
  // 站点报告：readiness 与 needs_external 都来自后端字段。
  const site = summarizePreparationResult(SITE_REPORT, 'site');
  assert.equal(site?.state, 'partial');
  assert.equal(site?.kind, 'site');
  assert.equal(site?.readiness, 'needs-rawdata');
  assert.equal(site?.needsExternal, 2);
  assert.equal(site?.output, '/out/site.nc');
  assert.equal(
    summarizePreparationResult({ ...SITE_REPORT, readiness: 'self_contained' }, 'site')?.state,
    'complete',
  );
  // 缺测报告：数字照抄，缺的字段就是 null，不补 0。
  const gaps = summarizePreparationResult(GAP_REPORT, 'gaps');
  assert.equal(gaps?.kind, 'gap-report');
  assert.equal(gaps?.missing, 12);
  assert.equal(gaps?.unresolved, 3);
  assert.equal(gaps?.state, 'partial');
  // 同一份 JSON，点的是「检查缺测」时不能显示成"已修复"。
  assert.equal(summarizePreparationResult(GAP_REPORT, 'gaps', true)?.state, 'checked');
  // 修复报告里后端给出最终路径（表格"修复并完成"把 path 和报告合在一起）。
  assert.equal(
    summarizePreparationResult({ ...GAP_REPORT, unresolved: 0, path: '/final.nc' }, 'table')
      ?.output,
    '/final.nc',
  );
  // 没有后端字段的形态不编造输出/缺测数。
  const unknown = summarizePreparationResult({ hello: 'world' }, 'netcdf');
  assert.equal(unknown?.state, 'unknown');
  assert.equal(unknown?.output, null);
  assert.equal(unknown?.missing, null);
  assert.equal(summarizePreparationResult(null, 'netcdf'), null);
  assert.equal(summarizePreparationResult('', 'netcdf'), null);
});

test('site results surface kind, mode, lookup provenance and rawdata warnings', () => {
  const summary = summarizePreparationResult(
    {
      path: '/out/site.nc',
      readiness: 'self_contained',
      from_site: ['soil_vf_clay'],
      from_raster: [],
      from_default: ['soil_texture'],
      from_lookup: ['canopy_height'],
      warnings: ['rawdata 目录可读，但这份站点文件没有取到任何栅格字段'],
      site_kind: 'single',
      mode: 'igbp',
      needs_external: [],
    },
    'site',
  )!;
  assert.equal(summary.fromLookup, 1);
  assert.equal(summary.siteKind, 'single');
  assert.equal(summary.siteMode, 'igbp');
  assert.deepEqual(summary.warnings, ['rawdata 目录可读，但这份站点文件没有取到任何栅格字段']);
});

test('arrays are distinguished between split tables and converted observations', () => {
  const split = summarizePreparationResult(
    [
      { site: 'a', staged_path: '/stage/a.nc', final_path: '/final/a.nc', rows: 10 },
      { site: 'b', staged_path: '/stage/b.nc', final_path: '/final/b.nc', rows: 20 },
    ],
    'table',
  );
  assert.equal(split?.kind, 'table-split');
  assert.equal(split?.sites, 2);
  assert.equal(split?.state, 'partial');
  assert.equal(split?.output, null, 'staged files are not final output');
  const observation = summarizePreparationResult(
    [
      {
        site: 'a',
        path: '/obs/a.nc',
        rows: 1,
        start: '',
        end: '',
        variables: [],
        warnings: ['qc'],
      },
      { site: 'b', path: '/obs/b.nc', rows: 1, start: '', end: '', variables: [], warnings: [] },
    ],
    'observation',
  );
  assert.equal(observation?.kind, 'observations');
  assert.equal(observation?.converted, 2);
  assert.equal(observation?.state, 'partial', 'backend warnings make it partial');
  assert.deepEqual(observation?.warnings, ['qc']);
  const single = summarizePreparationResult(
    [{ site: 'a', path: '/obs/a.nc', warnings: [] }],
    'observation',
  );
  assert.equal(single?.state, 'complete');
  assert.equal(single?.output, '/obs/a.nc');
});

test('an input probe is reported as a check with backend counts', () => {
  const probe = summarizePreparationResult(
    {
      slots: [{ index: 1 }, { index: 2 }],
      variables: ['time', 'Tair'],
      columns: [{ name: 'SWdown' }],
      time_column: 'time',
    },
    'netcdf',
  );
  assert.equal(probe?.kind, 'probe');
  assert.equal(probe?.state, 'checked');
  assert.equal(probe?.slots, 2);
  assert.equal(probe?.fields, 3);
});

test('the result card shows status, output, gaps and next steps with raw JSON collapsed', () => {
  const success = render({ result: '/out/forcing.nc', mode: 'netcdf', en: true });
  assert.match(success, /Completed · Forcing file/);
  assert.match(success, /Output path/);
  assert.match(success, /\/out\/forcing.nc/);
  assert.match(success, /Next: /);
  assert.match(success, /Raw result \(JSON\)/);
  assert.match(success, /<details/);
  assert.doesNotMatch(success, /<details[^>]*\bopen\b/, 'raw JSON starts collapsed');
  // 中文界面同样完整，不漏英文裸串。
  const chinese = render({ result: '/out/forcing.nc', mode: 'netcdf', en: false });
  assert.match(chinese, /成功 · 强迫场文件/);
  assert.match(chinese, /输出路径/);
  assert.match(chinese, /原始结果（JSON）/);

  const partial = render({
    result: GAP_REPORT,
    mode: 'gaps',
    en: true,
    hasSelectedCase: true,
    onOpenRuns: () => {},
  });
  assert.match(partial, /Partially completed · Gap report/);
  assert.match(partial, /12 missing value/);
  assert.match(partial, /3 still unresolved/);
  assert.match(partial, /Some gaps remain/);
  assert.match(partial, /notice-warning/);
  assert.match(partial, /Go to “Review and run”/);
});

test('a gap check does not claim the output was repaired, and unknown shapes stay honest', () => {
  const checked = render({ result: GAP_REPORT, mode: 'gaps', gapCheckOnly: true, en: true });
  assert.match(checked, /Checked, nothing written · Gap report/);
  assert.match(checked, /Confirm how gaps should be handled/);
  assert.doesNotMatch(checked, /Completed · Gap report/);
  const unknown = render({ result: { anything: 1 }, mode: 'netcdf', en: true });
  assert.match(unknown, /Finished · Processing result/);
  assert.doesNotMatch(unknown, /Output path/);
  assert.doesNotMatch(unknown, /missing value/);
  assert.match(unknown, /raw JSON/i);
});

test('the page-supplied output path is used only as a fallback when the report has none', () => {
  const gap = { ...GAP_REPORT, unresolved: 0, needs_era5: false };
  const withFallback = render({
    result: gap,
    mode: 'gaps',
    en: true,
    fallbackOutput: '/dst/forcing.nc',
  });
  assert.match(withFallback, /Output path/);
  assert.match(withFallback, /\/dst\/forcing\.nc/);
  const without = render({ result: gap, mode: 'gaps', en: true });
  assert.doesNotMatch(without, /Output path/);
  // 后端自己给的路径优先于兜底值。
  const both = render({
    result: { ...gap, path: '/from/backend.nc' },
    mode: 'gaps',
    en: true,
    fallbackOutput: '/dst/forcing.nc',
  });
  assert.match(both, /\/from\/backend\.nc/);
  assert.doesNotMatch(both, /\/dst\/forcing\.nc/);
});

test('finishing one table site and a self-contained site each get their own next step', () => {
  const finalized = render({
    result: { ...GAP_REPORT, unresolved: 0, needs_era5: false, path: '/final/a.nc' },
    mode: 'table',
    en: true,
  });
  assert.match(finalized, /Completed · Gap report/);
  assert.match(finalized, /\/final\/a\.nc/);
  assert.match(finalized, /This site’s final forcing was written/);
  const site = render({
    result: { ...SITE_REPORT, readiness: 'self_contained', needs_external: [] },
    mode: 'site',
    en: true,
  });
  assert.match(site, /Completed · Site file/);
  assert.match(site, /Self-contained; ready to run/);
  assert.match(site, /use “Use for new case” below/);
  // 需要 rawdata 的站点是部分完成，并明确说运行前还缺东西。
  const partial = render({ result: SITE_REPORT, mode: 'site', en: true });
  assert.match(partial, /Partially completed · Site file/);
  assert.match(partial, /2 field\(s\) still need external rawdata/);
  assert.match(partial, /Running still needs the missing fields/);
});

test('a split table points at the per-site finalize step instead of a non-existent output', () => {
  const split = render({
    result: [{ site: 'a', staged_path: '/stage/a.nc', final_path: '/final/a.nc' }],
    mode: 'table',
    en: true,
    // 之前生成过强迫场也不能把那条路径贴到表格拆分结果上：这次只写了暂存文件。
    fallbackOutput: '/previous/forcing.nc',
  });
  assert.match(split, /Partially completed · Split table/);
  assert.match(split, /1 site\(s\) split/);
  assert.match(split, /Finalize each site below/);
  assert.doesNotMatch(split, /Output path/);
  assert.doesNotMatch(split, /\/previous\/forcing\.nc/);
});

/** 从页面源码抽真实函数/箭头函数执行（与 page-results.test.ts 同一套做法）。 */
function pageCallback(name: string, context: Record<string, unknown>) {
  const path = fileURLToPath(new URL('./PreparationPage.tsx', import.meta.url));
  const source = ts.createSourceFile(
    path,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) body = node.getText(source);
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === name &&
      node.initializer
    )
      body = node.initializer.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, `${name} must exist in PreparationPage.tsx`);
  const js = ts.transpileModule(`const run=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return run;`)(...Object.values(context)) as (
    ...args: unknown[]
  ) => Promise<unknown>;
}

test('starting a new operation clears the stale success card and keeps retry possible', async () => {
  const shown: unknown[] = [],
    errors: string[] = [],
    busy: boolean[] = [];
  const lock = { current: false };
  const work = pageCallback('work', {
    registerCloseGuard: () => () => {},
    lock,
    setBusy: (value: boolean) => busy.push(value),
    setResult: (value: unknown) => shown.push(value),
    setError: (value: string) => errors.push(value),
    setDeclined: () => {},
    notify: () => {},
    t: (zh: string) => zh,
  });
  const report = { missing: 0, unresolved: 0 };
  // 成功：先清旧结果，再放新结果。
  assert.equal(await work(async () => report), report);
  assert.deepEqual(shown, [null, report]);
  // 失败：旧的成功结果已经被清掉，只剩错误；锁与 busy 都释放，按钮可以再点一次重试。
  await assert.rejects(
    work(async () => {
      throw new Error('forcing missing');
    }),
    /forcing missing/,
  );
  assert.deepEqual(shown, [null, report, null], 'a failed run cannot leave the old success card');
  assert.match(errors.at(-1) ?? '', /forcing missing/);
  assert.deepEqual(busy, [true, false, true, false]);
  assert.equal(lock.current, false, 'failure leaves the page retryable');
});

test('editing a field clears the previous result so no stale success remains', () => {
  const shown: unknown[] = [];
  const produced: unknown[] = [];
  const imported: unknown[] = [];
  const edited: boolean[] = [];
  const consumed: boolean[] = [];
  const form: Record<string, string> = { dst: '/a.nc', src: '/in.nc' };
  const set = pageCallback('set', {
    form,
    rememberForm: () => {},
    setResult: (value: unknown) => shown.push(value),
    setProduced: (value: unknown) => produced.push(value),
    setImported: (value: unknown) => imported.push(value),
    setDeclined: () => {},
    setEdited: (value: boolean) => edited.push(value),
    setInputsConsumed: (value: boolean) => consumed.push(value),
    setProbe: () => {},
    setSlots: () => {},
    // 像真实 state setter 一样把更新写回 form，这样"同一个值再设一次"才是无变化的。
    setForm: (updater: (current: Record<string, string>) => Record<string, string>) =>
      Object.assign(form, updater(form)),
  });
  void set('dst', '/b.nc');
  assert.deepEqual(shown, [null], 'a changed field invalidates the previous result');
  assert.deepEqual(produced, [{}], 'a changed field invalidates the produced artifacts');
  assert.deepEqual(imported, [[]], 'staged table items also belong to the old inputs');
  assert.deepEqual(edited, [true]);
  assert.deepEqual(consumed, [false]);
  void set('dst', '/b.nc');
  assert.deepEqual(shown, [null], 're-entering the same value does not add another clear');
  assert.deepEqual(produced, [{}], 're-entering the same value keeps the artifacts untouched');
  assert.deepEqual(imported, [[]], 're-entering the same value keeps the staged list untouched');
  // 修复参数（ERA5 辅助文件/缺测阈值）只影响逐站 finish，不影响已经拆分好的暂存文件：
  // 输入它们不能清掉"表格已拆分"待办列表（GUI 审查 F-D-01 回归）。
  for (const [key, value] of [
    ['era5', '/era5.nc'],
    ['short_gap', '3'],
    ['min_overlap', '8'],
  ] as const) {
    void set(key, value);
    assert.deepEqual(
      imported,
      [[]],
      `changing ${key} keeps the split table list so per-site finish stays reachable`,
    );
  }
  assert.deepEqual(produced, [{}, {}, {}, {}], 'stale artifacts are still invalidated');
  // 影响拆分结果的字段仍然清空待办列表。
  void set('time_column', 'TIME');
  assert.deepEqual(imported, [[], []], 'a field that shapes the split still clears the list');
});

test('a cancelled observation overwrite reads as a decision, not a red error', async () => {
  const errors: string[] = [],
    declined: string[] = [],
    notices: [string, boolean | undefined][] = [];
  const work = pageCallback('work', {
    registerCloseGuard: () => () => {},
    lock: { current: false },
    setBusy: () => {},
    setResult: () => {},
    setError: (value: string) => errors.push(value),
    setDeclined: (value: string) => declined.push(value),
    notify: (message: string, error?: boolean) => notices.push([message, error]),
    t: (zh: string) => zh,
  });
  const cancelled = Object.assign(new Error('已取消：保留已有观测文件。'), {
    name: 'CancelledError',
  });
  await assert.rejects(
    work(async () => {
      throw cancelled;
    }),
  );
  assert.deepEqual(notices, [], 'no global red banner for a deliberate cancellation');
  assert.equal(errors.at(-1), '');
  assert.equal(declined.at(-1), '已取消：保留已有观测文件。');
});

test('table finalization uses staged UTC metadata despite source and raw repair offsets', async () => {
  for (const manualOffset of [null, 8, -5]) {
    const requests: { command: string; args: Record<string, unknown> }[] = [];
    const options = { utc_offset: manualOffset, latitude: 1, longitude: 2, era5: '/era5.nc' };
    const finish = pageCallback('finish', {
      slots: [{ index: 1, name: 'temperature', wants: 'K' }],
      gapOptions: () => options,
      work: (fn: () => unknown) => fn(),
      confirmOverwrite: (fn: (overwrite: boolean) => unknown) => fn(false),
      invoke: async (command: string, args: Record<string, unknown>) => {
        requests.push({ command, args });
        return { unresolved: 0 };
      },
      setImported: () => {},
      setProduced: () => {},
      setInputsConsumed: () => {},
      imported: [],
    });
    await finish({
      staged_path: '/stage.nc',
      final_path: '/final.nc',
      latitude: 30,
      longitude: 120,
      timezone_offset_hours: 8,
    });
    assert.equal(requests[0].command, 'repair_forcing');
    assert.equal(requests[0].args.src, '/stage.nc');
    assert.deepEqual(requests[0].args.options, {
      ...options,
      latitude: 30,
      longitude: 120,
      utc_offset: null,
    });
    assert.equal(options.utc_offset, manualOffset, 'raw-file repair options remain intact');
  }
});

test('raw gap repair keeps its explicit manual timezone override', async () => {
  const options = pageCallback('gapOptions', {
    num: () => 1,
    optional: (key: string) => ({ utc_offset: 8, lat: 30, lon: 120 })[key],
    form: { era5: '/era5.nc' },
  });
  assert.deepEqual(await options(), {
    short_gap: 1,
    min_overlap: 1,
    utc_offset: 8,
    latitude: 30,
    longitude: 120,
    era5: '/era5.nc',
  });
});

test('changing preparation task clears its destination and obsolete artifacts', async () => {
  const form: Record<string, string> = {
    dst: '/old/site.nc',
    src: '/input.nc',
    site_mode: 'usgs',
  };
  let produced: unknown = { site: '/old/site.nc', met: '/old/met.nc' };
  const consumed: boolean[] = [];
  const autoDst = { current: '/old/site.nc' };
  const change = pageCallback('changeMode', {
    mode: 'site',
    setMode: () => {},
    clearPrepDraft: () => {},
    dirtyNow: { current: true },
    EMPTY_PREP_FORM,
    setForm: (update: ((old: typeof form) => typeof form) | Record<string, string>) =>
      Object.assign(form, typeof update === 'function' ? update(form) : update),
    autoDst,
    setProduced: (value: unknown) => {
      produced = value;
    },
    setEdited: () => {},
    setImported: () => {},
    setProbe: () => {},
    setSlots: () => {},
    setResult: () => {},
    setInputsConsumed: (value: boolean) => consumed.push(value),
    setError: () => {},
    setDeclined: () => {},
  });
  await change('site');
  assert.equal(form.dst, '/old/site.nc', 'same task retains the user destination');
  await change('netcdf');
  assert.equal(form.dst, '', 'another task must choose its own output');
  // 切任务后残留输入必须清空，否则 edited=false/consumed=true 会把上一条任务的残留
  // 输入误判成"干净的新任务"（D-05 残留）。
  assert.equal(form.src, '');
  assert.equal(form.site_mode, '');
  assert.equal(form.lon, '', 'coordinates from the previous task are cleared too');
  assert.equal(autoDst.current, '');
  assert.deepEqual(produced, {}, 'old artifacts cannot be applied to a new case');
  assert.deepEqual(consumed, [true], 'a new task starts from a clean, non-dirty state');
});

test('site generation retries an existing destination only after overwrite confirmation', async () => {
  for (const accepted of [false, true]) {
    const writes: boolean[] = [];
    const consumed: boolean[] = [];
    let confirmations = 0;
    let produced: unknown = null;
    const form = { dst: '/existing/site.nc', rawdata: '' };
    const confirmOverwrite = pageCallback('confirmOverwrite', {
      form,
      t: (_zh: string, en: string) => en,
      askConfirm: async () => {
        confirmations++;
        return accepted;
      },
    });
    const execute = pageCallback('execute', {
      preparedSiteNameValid: () => true,
      preparedSiteDestination: (_directory: string, current: string) => current,
      rememberForm: () => {},
      form,
      mode: 'site',
      preparation: { mode: 'igbp', crop: false },
      effectiveSiteMode: 'igbp',
      num: () => 1,
      optional: () => null,
      work: (fn: () => unknown) => fn(),
      confirmOverwrite,
      invoke: async (command: string, args: { overwrite?: boolean }) => {
        assert.equal(command, 'make_site');
        writes.push(args.overwrite === true);
        if (args.overwrite === false) throw new Error('产物已存在，未覆盖');
        return SITE_REPORT;
      },
      setProduced: (update: (old: object) => unknown) => {
        produced = update({});
      },
      setInputsConsumed: (value: boolean) => consumed.push(value),
    });
    if (accepted) {
      assert.equal(await execute(), SITE_REPORT);
      assert.deepEqual(writes, [false, true]);
      assert.deepEqual(produced, { site: form.dst });
      assert.deepEqual(consumed, [true], 'a written product consumes the pending input');
    } else {
      await assert.rejects(execute(), /Cancelled/);
      assert.deepEqual(writes, [false], 'no destructive request after cancellation');
      assert.equal(produced, null);
      assert.deepEqual(consumed, [], 'a cancelled write leaves the input pending');
    }
    assert.equal(confirmations, 1);
  }
});

test('use for new case asks before discarding unfinished table sites', async () => {
  const updates: unknown[] = [];
  const navigations: string[] = [];
  const cleared: unknown[] = [];
  const asked: string[] = [];
  const run = pageCallback('useForNewCase', {
    canUse: true,
    produced: { met: '/out/CN-Cng_Met.nc' },
    imported: [{ site: 'a' }, { site: 'b' }],
    update: (patch: unknown) => updates.push(patch),
    dirtyNow: { current: false },
    navigate: (route: string) => (navigations.push(route), true),
    askConfirm: async (message: string) => (asked.push(message), true),
    setImported: (value: unknown) => cleared.push(value),
    setEdited: () => {},
    setInputsConsumed: () => {},
    t: (zh: string) => zh,
  });
  assert.equal(await run(), true);
  assert.equal(asked.length, 1);
  assert.match(asked[0], /2 个站点/);
  assert.deepEqual(cleared, [[]], 'unfinished per-site work is discarded only after confirmation');
  assert.equal(updates.length, 1, 'the new-case seed is written once');
  assert.deepEqual(navigations, ['new-case']);
});

test('discarding unfinished sites lets navigation run in the same tick', async () => {
  // 本次验收 P2：setEdited/setInputsConsumed 是异步的，守卫闭包会读到上一轮 dirty；
  // 确认后必须同步把守卫状态置为可离开，否则 navigate 仍被拦下且 seed 已写入。
  const dirtyNow = { current: true };
  const navigations: string[] = [];
  const run = pageCallback('useForNewCase', {
    canUse: true,
    produced: { met: '/out/CN-Cng_Met.nc' },
    imported: [{ site: 'a' }],
    dirtyNow,
    update: () => {},
    navigate: (route: string) => {
      if (dirtyNow.current) return false;
      navigations.push(route);
      return true;
    },
    askConfirm: async () => true,
    setImported: () => {},
    setEdited: () => {},
    setInputsConsumed: () => {},
    t: (zh: string) => zh,
  });
  assert.equal(await run(), true);
  assert.equal(dirtyNow.current, false);
  assert.deepEqual(navigations, ['new-case']);
});

test('declining to discard unfinished table sites keeps them and stays', async () => {
  const updates: unknown[] = [];
  const navigations: string[] = [];
  const cleared: unknown[] = [];
  const run = pageCallback('useForNewCase', {
    canUse: true,
    produced: { met: '/out/CN-Cng_Met.nc' },
    imported: [{ site: 'a' }],
    update: (patch: unknown) => updates.push(patch),
    dirtyNow: { current: false },
    navigate: (route: string) => (navigations.push(route), true),
    askConfirm: async () => false,
    setImported: (value: unknown) => cleared.push(value),
    setEdited: () => {},
    setInputsConsumed: () => {},
    t: (zh: string) => zh,
  });
  assert.equal(await run(), false);
  assert.deepEqual(cleared, []);
  assert.deepEqual(updates, []);
  assert.deepEqual(navigations, []);
});

test('short-gap and min-overlap must be whole numbers before submit', () => {
  const form: Record<string, string> = { short_gap: '3.5', min_overlap: '8' };
  const num = pageCallback('num', {
    numericLabels: { short_gap: '短缺测最大步数', min_overlap: '最少重叠步数' },
    form,
    t: (zh: string) => zh,
    requiredNumber: (value: string, label: string) => {
      if (!value.trim() || !Number.isFinite(Number(value)))
        throw new Error(`${label}: 请输入有效数字 / enter a number`);
      return Number(value);
    },
  });
  assert.throws(() => num('short_gap'), /必须是整数/);
  assert.equal(num('min_overlap'), 8);
  form.min_overlap = '3.5';
  assert.throws(() => num('min_overlap'), /必须是整数/);
  form.min_overlap = '2000000';
  assert.throws(() => num('min_overlap'), /须在 0 到 1000000 之间/);
});

for (const picked of ['/dataset-B', null]) {
  test(`dataset installation reopens the picker and ${picked ? 'uses the new destination' : 'cancels without writing'}`, async () => {
    const calls: { name: string; args: Record<string, unknown> }[] = [];
    const produced = { site: '/output/test_site.nc', met: '/output/test_Met.nc' };
    let scan: unknown = null;
    const install = pageCallback('installToDataset', {
      datasetRoot: '/dataset-A',
      produced,
      setDatasetRoot: () => {},
      preparedDatasetDestinations,
      notify: () => {},
      t: (_zh: string, en: string) => en,
      setDatasetScan: (value: unknown) => {
        scan = value;
      },
      invoke: async (name: string, args: Record<string, unknown>) => {
        calls.push({ name, args });
        if (name === 'pick_folder') return picked;
        if (name === 'scan_sites') return { sites: [{ name: 'test' }] };
      },
    });
    await install();
    assert.equal(calls[0]?.name, 'pick_folder');
    assert.deepEqual(calls[0]?.args, { key: 'prep-dataset' });
    if (!picked) {
      assert.equal(calls.length, 1);
      assert.equal(scan, null);
    } else {
      assert.deepEqual(
        calls.filter((c) => c.name === 'install_prepared_file').map((c) => c.args.destination),
        ['/dataset-B/Sitedata/test_site.nc', '/dataset-B/Forcing/test_Met.nc'],
      );
      assert.deepEqual(calls.find((c) => c.name === 'scan_sites')?.args, {
        dir: '/dataset-B/Sitedata',
        forcingDir: '/dataset-B/Forcing',
        quick: true,
      });
    }
    assert.deepEqual(produced, { site: '/output/test_site.nc', met: '/output/test_Met.nc' });
  });
}
