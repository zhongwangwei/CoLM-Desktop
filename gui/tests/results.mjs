import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-results-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;

const {
  LruCache, boundedMap, downsampleSeries, metricKey, resultCases, rowsToCsv,
} = await import(moduleUrl('result-model.js'));

const cases = [
  { dir: '/cases/old', name: 'old', has_history: true },
  { dir: '/cases/a', name: 'A', has_history: true },
  { dir: '/cases/b', name: 'B', has_history: false },
  { dir: '/cases/c', name: 'C', has_history: true },
];
const scope = resultCases(cases, new Set(['/cases/a', '/cases/b', '/cases/c']));
if (scope.map(c => c.name).join('|') !== 'A|B|C') {
  throw new Error('result scope leaked old root cases or lost current cases');
}
if (resultCases(cases, new Set(['/cases/a', '/cases/c']), true).map(c => c.name).join('|') !== 'A|C') {
  throw new Error('completed result scope is not derived from history availability');
}

let active = 0;
let peak = 0;
const mapped = await boundedMap([1, 2, 3, 4, 5], 2, async value => {
  active += 1;
  peak = Math.max(peak, active);
  await new Promise(resolve => setTimeout(resolve, 5));
  active -= 1;
  if (value === 3) throw new Error('site failed');
  return value * 2;
});
if (peak !== 2 || mapped.filter(x => x.ok).length !== 4 || mapped[2].ok) {
  throw new Error('bounded result pool lost its concurrency or partial-failure contract');
}

const cache = new LruCache(2);
cache.set('a', 1); cache.set('b', 2); cache.get('a'); cache.set('c', 3);
if (cache.has('b') || !cache.has('a') || !cache.has('c')) {
  throw new Error('result LRU does not evict the least recently used item');
}
if (metricKey({ caseDir: '/a', obs: '/o', summaryOnly: false })
    === metricKey({ caseDir: '/a', obs: '/o', summaryOnly: true })) {
  throw new Error('summary-only metrics must not collide with chart-pair metrics in cache');
}

const time = Array.from({ length: 1000 }, (_, i) => i);
const values = time.map(i => i === 501 ? 9999 : Math.sin(i / 10));
const sampled = downsampleSeries(time, values, 80);
if (sampled.time.length > 80 || !sampled.values.includes(9999)
    || sampled.time[0] !== 0 || sampled.time.at(-1) !== 999) {
  throw new Error('result downsampling lost its point limit, extrema, or endpoints');
}

const csv = rowsToCsv([{ site: 'A,1', note: 'line\n"two"' }], ['site', 'note']);
if (!csv.includes('"A,1"') || !csv.includes('"line\n""two"""')) {
  throw new Error('result CSV export does not quote delimiters and newlines safely');
}

const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const pane of [
  'result-overview', 'result-data', 'result-series', 'result-evaluation',
  'result-comparison', 'result-diagnostics', 'result-export',
]) {
  if (!html.includes(`data-flow-pane="${pane}"`)) {
    throw new Error(`results workbench is missing ${pane}`);
  }
}
if (!html.includes('id="series-csv"')) {
  throw new Error('time-series workbench is missing the full-resolution CSV export');
}
const resultUi = await readFile(join(root, 'dist', 'app', 'results.js'), 'utf8');
if (!resultUi.includes('summaryOnly') || !resultUi.includes('pairVar')
    || !resultUi.includes("false, summaryRow.name, 2400")) {
  throw new Error('multi-site summaries and selected-variable chart pairs are not loaded independently');
}

console.log('results: independent scope, bounded concurrency, LRU, downsampling, export, and seven panes are present');
