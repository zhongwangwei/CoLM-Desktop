import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

import {
  forcingDirectoryForSiteDirectory, matchesBundledExampleMode,
} from '../dist/app/ui.js';

assert.equal(
  forcingDirectoryForSiteDirectory('/data/PLUMBER2s/Sitedata'),
  '/data/PLUMBER2s/Forcing',
);
assert.equal(
  forcingDirectoryForSiteDirectory('C:\\data\\PLUMBER2s\\Sitedata\\'),
  'C:\\data\\PLUMBER2s\\Forcing',
);
assert.equal(forcingDirectoryForSiteDirectory(''), '');

assert.equal(matchesBundledExampleMode('/examples/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc', 'natural'), true);
assert.equal(matchesBundledExampleMode('/examples/Sitedata/AT-Neu_2010-2012_FLUXNET-CH4_site.nc', 'natural'), false);
assert.equal(matchesBundledExampleMode('C:\\examples\\Sitedata\\AT-Neu_2010-2012_FLUXNET-CH4_site.nc', 'methane'), true);
assert.equal(matchesBundledExampleMode('/examples/Sitedata/CN-Cng_2008-2009_FLUXNET2015_site.nc', 'methane'), false);
assert.equal(matchesBundledExampleMode('/examples/Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc', 'crop'), true);
assert.equal(matchesBundledExampleMode('/examples/Sitedata/US-Ne3_2002-2003_FLUXNET2015_CROP_site.nc', 'natural'), false);
assert.equal(matchesBundledExampleMode('/user/Sitedata/custom_site.nc', 'methane'), true);

const sites = await readFile(new URL('../dist/app/sites.js', import.meta.url), 'utf8');
const index = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
assert.match(index, /US-Ne3 作物站/, 'bundled-site help must name the crop example');
assert.match(sites, /mode === 'crop' \? '作物'/, 'empty and summary labels must distinguish crop sites');
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
assert.match(
  sites,
  /urbanEnabled\(\)[\s\S]*\? `urban-\$\{String\(state\.subgrid \?\? 'IGBP'\)\.toLowerCase\(\)\}`/,
  'urban case creation must preserve the selected USGS/IGBP classification',
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
assert.match(sites, /preferredExampleSite[\s\S]*US-Ne3[\s\S]*AT-Neu/, 'crop and methane wizards must prefer their bundled default sites');
assert.match(sites, /state\.pickedSiteAuto = true/, 'auto-selected default sites must be replaceable when the wizard mode changes');
assert.match(sites, /state\.pickedSiteAuto = false;[\s\S]*renderSites\(\)/, 'manual site selection must stop automatic default replacement');
assert.match(html, /AT-Neu 甲烷站/, 'bundled example hint must mention the methane default site');
const css = await readFile(new URL('../dist/app/style.css', import.meta.url), 'utf8');
assert.match(
  css,
  /\.case\.site-row\s*\{[^}]*display:\s*grid;[^}]*grid-template-columns:/s,
  'site rows must use shared grid columns rather than content-dependent flex spacing',
);

console.log('site paths: forcing directory follows Sitedata and availability refreshes automatically');
