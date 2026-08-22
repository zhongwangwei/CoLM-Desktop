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

console.log('site paths: forcing directory follows Sitedata and availability refreshes automatically');
