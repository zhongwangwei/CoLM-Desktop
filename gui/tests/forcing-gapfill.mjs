import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const frontend = await readFile(new URL('../dist/app/forcing.js', import.meta.url), 'utf8');
const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
for (const command of [
  'probe_forcing_table',
  'convert_forcing_table',
  'probe_forcing_gaps',
  'repair_forcing',
  'download_era5land',
  'convert_forcing',
  'make_site',
  'install_prepared_pair',
]) {
  assert.match(frontend, new RegExp(`invoke\\('${command}'`), `${command} must be wired`);
}
assert.match(frontend, /CSV \/ TXT/);
assert.match(frontend, /多个站点/);
assert.match(frontend, /tableProbe/);
assert.match(frontend, /splitAndDiagnoseTable/);
assert.match(frontend, /repairTableBatch/);
assert.match(frontend, /canonicalTableSlots/);
assert.match(frontend, /name === slot\.guessed \? slot\.units \?\? '' : ''/);
assert.match(frontend, /tableBatch\s*=\s*imported\.map/);
assert.match(frontend, /const allComplete = tableBatch\.every/);
assert.match(frontend, /mustKeepSiteColumn/);
assert.match(frontend, /多个站点必须保留站点名称列，不能合并成一个站点/);
assert.match(frontend, /function invalidateTableBatch\(\)[\s\S]*tableBatch = \[\][\s\S]*resetBatchArtifacts\(\)/);
assert.doesNotMatch(frontend, /gapReport\.missing === 0[\s\S]{0,100}return srcPath/,
  'even gap-free sources must persist the diagnosed timezone before conversion');
assert.match(frontend, /item\.phase = '生成站点文件'[\s\S]*invoke\('make_site'[\s\S]*item\.phase = '修复中'[\s\S]*invoke\('repair_forcing'/);
assert.match(frontend, /invoke\('make_site'[\s\S]*crop: !!state\.wizard\?\.physics\?\.crop/);
assert.match(frontend, /invoke\('install_prepared_pair'[\s\S]*item\.siteReport\.path = item\.siteFinalPath/);
assert.match(frontend, /item\.siteReport\.readiness === 'blocked'[\s\S]*throw new Error/);
assert.match(frontend, /siteOutputName/);
assert.match(frontend, /短缺口上限（时间步）/);
assert.match(frontend, /ERA5-Land 缓存目录/);
assert.match(frontend, /一次下载该站点完整时间段/);
assert.match(frontend, /CDS 服务器可能排队/);
assert.match(frontend, /QC 范围/);
assert.match(frontend, /quality_rejected/);
assert.match(frontend, /message\.includes\('CDS API 配置'\).*globalThis\.alert/);
assert.match(frontend, /\*_gapfill_qc/);
assert.match(frontend, /if \(!gapReport\) reasons\.push\('先完成缺测与时区诊断'\)/);
assert.match(frontend, /gapReport\.missing > 0 && !repairedSource/);
assert.match(frontend, /sourceForConvert = await ensureRepairedSource/);
assert.match(frontend, /src: sourceForConvert/);
const interpolatedHtml = (frontend.match(/innerHTML\s*=\s*`[\s\S]*?`/g) ?? [])
  .filter(block => block.includes('${'));
assert.deepEqual(interpolatedHtml, [], 'forcing UI must render probe/report values with textContent, not template HTML');
assert.match(frontend, /header: true, text: 'UTC 偏移'/);
assert.match(frontend, /\{ text: row\.variable \}/);
assert.match(html, /id="era5land-guide"/);
assert.doesNotMatch(html, /ERA5-Land 没有被删除/);
assert.match(html, /软件会取最近的 0\.1° 格点/);
assert.match(html, /只有发现长缺口时才需要 ERA5-Land/);
assert.match(html, /~\/.cdsapirc/);

const backend = await readFile(new URL('../src-tauri/src/forcing.rs', import.meta.url), 'utf8');
for (const command of [
  'forcing-table-probe',
  'forcing-table-convert',
  'forcing-gap-probe',
  'forcing-repair',
  'era5land-download',
]) {
  assert.match(backend, new RegExp(command));
}

const design = await readFile(new URL('../../docs/plan-forcing-gap-repair.md', import.meta.url), 'utf8');
for (const contract of ['原始文件永不覆盖', 'longitude_inferred_offset', 'ERA5-Land', 'QC']) {
  assert.match(design, new RegExp(contract));
}

const tableDesign = await readFile(new URL('../../docs/plan-tabular-multisite-prep.md', import.meta.url), 'utf8');
for (const contract of ['CSV/TXT', '多个站点', '按站点拆分', '缺失整条时间记录', '<站点>_Met.nc']) {
  assert.match(tableDesign, new RegExp(contract));
}

console.log('forcing preprocessing: NetCDF and multi-site CSV/TXT diagnosis, repair, and conversion gates are wired');
