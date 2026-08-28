import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

import {
  forcingOutputName,
  missingForcingHeights,
  normalizeSiteStem,
  siteOutputName,
} from '../dist/app/prep-state.js';
import {
  landCoverClasses,
  landCoverLabel,
} from '../dist/app/land-cover.js';

assert.equal(normalizeSiteStem(' AT-Neu '), 'AT-Neu');
assert.equal(normalizeSiteStem('AT Neu/site'), 'AT-Neu-site');
assert.equal(siteOutputName('AT-Neu'), 'AT-Neu_site.nc');
assert.equal(forcingOutputName('AT-Neu'), 'AT-Neu_Met.nc');
assert.deepEqual(missingForcingHeights({ v: 10, t: null, q: 2 }), ['T']);
assert.deepEqual(missingForcingHeights({ v: 10, t: 2, q: 2 }), []);

const igbp = landCoverClasses('igbp');
const usgs = landCoverClasses('usgs');
assert.equal(igbp.length, 17);
assert.equal(usgs.length, 24);
assert.deepEqual(igbp.map(item => item.value), Array.from({ length: 17 }, (_, i) => i + 1));
assert.deepEqual(usgs.map(item => item.value), Array.from({ length: 24 }, (_, i) => i + 1));
assert.equal(landCoverLabel(igbp[9], 'zh'), '10 · 草地');
assert.equal(landCoverLabel(igbp[9], 'en'), '10 · Grasslands');
assert.equal(landCoverLabel(usgs[0], 'zh'), '1 · 城市与建成区');

const state = await readFile(new URL('../dist/app/state.js', import.meta.url), 'utf8');
assert.match(state, /prepArtifacts:\s*\{/);
assert.match(state, /observationFile: null/);
assert.match(state, /observationDir: null/);

const site = await readFile(new URL('../dist/app/sitedata.js', import.meta.url), 'utf8');
assert.match(site, /adoptPreparedSite/);
assert.match(site, /sitedir/);
assert.match(site, /scanPreparedSites/);
assert.match(site, /invoke\('make_site'[\s\S]*crop: !!state\.wizard\?\.physics\?\.crop/);
assert.match(site, /filter\(item => !crop \|\| item\.value === 12\)/, 'CROP preprocessing must not offer non-cropland land-cover classes');
assert.match(site, /crop \? '12' : ''/, 'CROP preprocessing must default to the required IGBP Croplands class');
assert.match(site, /el\.addEventListener\('input', invalidateSite\);[\s\S]*el\.addEventListener\('change', invalidateSite\);/, 'site generation must react to picker change events as well as typing');
assert.doesNotMatch(site, /innerHTML\s*=\s*`[\s\S]*?\$\{/, 'site reports must not interpolate paths or report fields into HTML');
assert.doesNotMatch(site, /<code>\$\{(?:result\.path|path)\}/, 'site file paths must be rendered through textContent');
assert.match(site, /report\.site_kind === 'urban'[\s\S]*完整 Urban-PLUMBER 站点文件/, 'urban coordinate-only preprocessing must explain rawdata or complete Urban-PLUMBER input requirement');

const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
assert.match(html, /<select class="input" id="slandtype"/);
assert.doesNotMatch(html, /id="slandtype"[^>]*type="number"/);
assert.match(html, /data-file="nc,nc4,csv,txt,tsv"/);
assert.match(html, /单站或多站/);
for (const id of [
  'single-site-example',
  'forcing-table-example',
  'era5land-guide',
  'vsrc',
  'vprobe',
  'validation-table-example',
  'validation-cards',
]) {
  assert.match(html, new RegExp(`id="${id}"`), `preprocessing UI must expose ${id}`);
}
assert.match(html, /验证数据不参与 mksrfdata、mkinidata 或 colm 运行/);
assert.match(html, /缺少验证数据只会限制后续评估与调优/);
assert.match(html, /时间标签保持原样，必须与对应强迫场一致/);
assert.doesNotMatch(html, /时间会统一换算为 UTC/);

const examples = await readFile(new URL('../dist/app/prep-examples.js', import.meta.url), 'utf8');
assert.match(examples, /function forcingTableExample/);
assert.match(examples, /time,site,latitude,longitude,landtype,utc_offset/);
assert.match(examples, /new Blob/);

const validation = await readFile(new URL('../dist/app/validation.js', import.meta.url), 'utf8');
assert.match(validation, /invoke\('probe_observation_table'/);
assert.match(validation, /invoke\('convert_observation_table'/);
assert.match(validation, /dstDir: settings\.dst\.trim\(\)/);
assert.match(validation, /time_column: settings\.time/);
assert.match(validation, /site_column: settings\.site \|\| null/);
assert.match(validation, /qc_column: choice\.qc \|\| null/);
assert.match(validation, /go\('prep-ready'\)/, 'validation data must be skippable');
assert.match(validation, /observationFile/);
assert.match(validation, /dst\.readOnly = true/);
assert.doesNotMatch(validation, /utc_offset|utcOffset/,
  'validation import must preserve the paired forcing clock instead of shifting timestamps');
assert.doesNotMatch(validation, /key: 'validation-output'/,
  'validation output must stay in the sibling Observation directory so scan can auto-pair it');
assert.doesNotMatch(validation, /innerHTML\s*=\s*`[\s\S]*?\$\{/,
  'validation reports must not interpolate paths or source fields into HTML');

const forcing = await readFile(new URL('../dist/app/forcing.js', import.meta.url), 'utf8');
assert.match(forcing, /missingForcingHeights\(heights\)/);
assert.match(forcing, /forcingOutputName/);
assert.match(forcing, /forcingdir/);
assert.match(forcing, /scanPreparedSites\(\)/);
const dynamicForcingHtml = (forcing.match(/innerHTML\s*=\s*`[\s\S]*?`/g) ?? [])
  .filter(block => block.includes('${'));
assert.deepEqual(dynamicForcingHtml, [], 'forcing reports must not interpolate probe/report fields into HTML');

const shell = await readFile(new URL('../dist/app/shell.js', import.meta.url), 'utf8');
for (const id of ['prep-site', 'prep-forcing', 'prep-validation', 'prep-ready']) {
  assert.match(shell, new RegExp(`id: '${id}'`), `workflow must expose ${id}`);
}

console.log('prep workbench: naming, readiness blockers, shared artifacts, and handoff are wired');
