import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

import { forcingDirectoryForSiteDirectory } from '../dist/app/ui.js';

assert.equal(
  forcingDirectoryForSiteDirectory('/data/PLUMBER2s/Sitedata'),
  '/data/PLUMBER2s/Forcing',
);
assert.equal(
  forcingDirectoryForSiteDirectory('C:\\data\\PLUMBER2s\\Sitedata\\'),
  'C:\\data\\PLUMBER2s\\Forcing',
);
assert.equal(forcingDirectoryForSiteDirectory(''), '');

const sites = await readFile(new URL('../dist/app/sites.js', import.meta.url), 'utf8');
assert.match(
  sites,
  /\$\('sitedir'\)\.addEventListener\('change',[\s\S]*forcingDirectoryForSiteDirectory/,
  'changing the site directory must replace a stale forcing directory',
);
assert.match(
  sites,
  /\$\('forcingdir'\)\.addEventListener\('change', scheduleSiteScan\)/,
  'changing the forcing directory must refresh stale site availability',
);

const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
assert.match(
  html,
  /id="makecase"[^>]*justify-content:flex-end/,
  'the create-case action must sit on the right of the site-selection card',
);

assert.match(sites, /d\.className = 'case site-row'/);
assert.match(sites, /name\.className = 'site-name'/);
assert.match(sites, /small\.className = 'site-meta'/);
const css = await readFile(new URL('../dist/app/style.css', import.meta.url), 'utf8');
assert.match(
  css,
  /\.case\.site-row\s*\{[^}]*display:\s*grid;[^}]*grid-template-columns:/s,
  'site rows must use shared grid columns rather than content-dependent flex spacing',
);

console.log('site paths: forcing directory follows Sitedata and availability refreshes automatically');
