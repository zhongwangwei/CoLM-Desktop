import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

import {
  forcingOutputName,
  missingForcingHeights,
  normalizeSiteStem,
  siteOutputName,
} from '../dist/app/prep-state.js';

assert.equal(normalizeSiteStem(' AT-Neu '), 'AT-Neu');
assert.equal(normalizeSiteStem('AT Neu/site'), 'AT-Neu-site');
assert.equal(siteOutputName('AT-Neu'), 'AT-Neu_site.nc');
assert.equal(forcingOutputName('AT-Neu'), 'AT-Neu_Met.nc');
assert.deepEqual(missingForcingHeights({ v: 10, t: null, q: 2 }), ['T']);
assert.deepEqual(missingForcingHeights({ v: 10, t: 2, q: 2 }), []);

const state = await readFile(new URL('../dist/app/state.js', import.meta.url), 'utf8');
assert.match(state, /prepArtifacts:\s*\{/);

const site = await readFile(new URL('../dist/app/sitedata.js', import.meta.url), 'utf8');
assert.match(site, /adoptPreparedSite/);
assert.match(site, /sitedir/);
assert.match(site, /scanPreparedSites/);

const forcing = await readFile(new URL('../dist/app/forcing.js', import.meta.url), 'utf8');
assert.match(forcing, /missingForcingHeights\(heights\)/);
assert.match(forcing, /forcingOutputName/);
assert.match(forcing, /forcingdir/);

const shell = await readFile(new URL('../dist/app/shell.js', import.meta.url), 'utf8');
for (const id of ['prep-site', 'prep-forcing', 'prep-ready']) {
  assert.match(shell, new RegExp(`id: '${id}'`), `workflow must expose ${id}`);
}

console.log('prep workbench: naming, readiness blockers, shared artifacts, and handoff are wired');
