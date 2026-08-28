//! Small, copyable preprocessing examples. They are reference data, not hidden
//! demo imports: filling a single-site example never touches local paths.

import { state } from './state.js';
import { $, status } from './ui.js';
import { prepMode } from './prep-state.js';

const EXAMPLES = {
  natural: { site: 'AT-Neu', longitude: 11.3175, latitude: 47.1167, landtype: 10 },
  usgs: { site: 'AT-Neu', longitude: 11.3175, latitude: 47.1167, landtype: 7 },
  crop: { site: 'US-Ne3', longitude: -96.44, latitude: 41.18, landtype: 12 },
  urban: { site: 'AU-Preston', longitude: 145.01, latitude: -37.73, landtype: 6 },
};

function currentExample() {
  if (state.wizard?.physics?.crop) return EXAMPLES.crop;
  const mode = prepMode(state);
  return EXAMPLES[mode] ?? EXAMPLES.natural;
}

function exampleText(example = currentExample()) {
  return `site: ${example.site}\nlongitude: ${example.longitude}\nlatitude: ${example.latitude}\nlandtype: ${example.landtype}\noutput: ${example.site}_site.nc`;
}

export function forcingTableExample(example = currentExample()) {
  const second = `${example.site}-B`;
  const lon2 = Number((example.longitude + 0.01).toFixed(4));
  const lat2 = Number((example.latitude + 0.01).toFixed(4));
  return `time,site,latitude,longitude,landtype,utc_offset,Tair[K],Qair[kg kg-1],Psurf[Pa],Precip[kg m-2 s-1],Wind_E[m s-1],Wind_N[m s-1],SWdown[W m-2],LWdown[W m-2]
2020-01-01 00:00,${example.site},${example.latitude},${example.longitude},${example.landtype},0,273.15,0.0032,85000,0.000000,1.2,0.4,0,280
2020-01-01 00:30,${example.site},${example.latitude},${example.longitude},${example.landtype},0,273.45,0.0031,85010,0.000001,1.4,0.5,5,282
2020-01-01 00:00,${second},${lat2},${lon2},${example.landtype},0,274.15,0.0034,85100,0.000000,0.8,0.2,0,285
2020-01-01 00:30,${second},${lat2},${lon2},${example.landtype},0,274.50,0.0035,85090,0.000000,0.7,0.1,12,287`;
}

function renderSingleExample() {
  if ($('single-site-example')) $('single-site-example').textContent = exampleText();
  if ($('forcing-table-example')) $('forcing-table-example').textContent = forcingTableExample();
}

async function copyText(text, success) {
  await navigator.clipboard.writeText(text);
  status(success);
}

function downloadText(text, filename) {
  const link = document.createElement('a');
  link.href = URL.createObjectURL(new Blob([text], { type: 'text/csv;charset=utf-8' }));
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 0);
}

renderSingleExample();

$('site-example-fill').onclick = () => {
  const example = currentExample();
  $('sname').value = example.site;
  $('slon').value = example.longitude;
  $('slat').value = example.latitude;
  $('slandtype').value = example.landtype;
  $('sname').dispatchEvent(new Event('input', { bubbles: true }));
  status('示例已填入；本机目录仍由你选择');
};
$('site-example-copy').onclick = () => copyText(exampleText(), '单站样例已复制').catch(status);
$('forcing-example-copy').onclick = () => copyText(forcingTableExample(), '多站点 CSV 样例已复制').catch(status);
$('forcing-example-download').onclick = () => downloadText(forcingTableExample(), 'colm-multisite-example.csv');

globalThis.addEventListener?.('colm:wizard', renderSingleExample);
