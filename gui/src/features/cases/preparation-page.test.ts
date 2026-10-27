import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
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
  const path = new URL('./PreparationPage.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path.pathname, 'utf8'),
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
  const path = new URL('./PreparationPage.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path.pathname, 'utf8'),
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
  const form: Record<string, string> = { dst: '/a.nc', src: '/in.nc' };
  const set = pageCallback('set', {
    form,
    setResult: (value: unknown) => shown.push(value),
    setProbe: () => {},
    setSlots: () => {},
    // 像真实 state setter 一样把更新写回 form，这样"同一个值再设一次"才是无变化的。
    setForm: (updater: (current: Record<string, string>) => Record<string, string>) =>
      Object.assign(form, updater(form)),
  });
  void set('dst', '/b.nc');
  assert.deepEqual(shown, [null], 'a changed field invalidates the previous result');
  void set('dst', '/b.nc');
  assert.deepEqual(shown, [null], 're-entering the same value does not add another clear');
});
