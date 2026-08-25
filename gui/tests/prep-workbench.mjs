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

const site = await readFile(new URL('../dist/app/sitedata.js', import.meta.url), 'utf8');
assert.match(site, /adoptPreparedSite/);
assert.match(site, /sitedir/);
assert.match(site, /scanPreparedSites/);
assert.doesNotMatch(site, /innerHTML\s*=\s*`[\s\S]*?\$\{/, 'site reports must not interpolate paths or report fields into HTML');
assert.doesNotMatch(site, /<code>\$\{(?:result\.path|path)\}/, 'site file paths must be rendered through textContent');

const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
assert.match(html, /<select class="input" id="slandtype"/);
assert.doesNotMatch(html, /id="slandtype"[^>]*type="number"/);
assert.match(html, /data-file="nc,nc4,csv,txt,tsv"/);
assert.match(html, /单站或多站/);

const forcing = await readFile(new URL('../dist/app/forcing.js', import.meta.url), 'utf8');
assert.match(forcing, /missingForcingHeights\(heights\)/);
assert.match(forcing, /forcingOutputName/);
assert.match(forcing, /forcingdir/);
assert.match(forcing, /scanPreparedSites\(\)/);
const dynamicForcingHtml = (forcing.match(/innerHTML\s*=\s*`[\s\S]*?`/g) ?? [])
  .filter(block => block.includes('${'));
assert.deepEqual(dynamicForcingHtml, [], 'forcing reports must not interpolate probe/report fields into HTML');

const shell = await readFile(new URL('../dist/app/shell.js', import.meta.url), 'utf8');
for (const id of ['prep-site', 'prep-forcing', 'prep-ready']) {
  assert.match(shell, new RegExp(`id: '${id}'`), `workflow must expose ${id}`);
}

console.log('prep workbench: naming, readiness blockers, shared artifacts, and handoff are wired');
