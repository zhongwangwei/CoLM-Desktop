//! 打开磁盘上已有的算例：从它的 `case.nml`（与跑过时的 `stages.json`）反推出等价的向导选择，
//! 走与向导同一条 `startSession`，再把这个算例放进本次列表并选中。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $ } from './ui.js';
import { go, renderSteps, setStatus } from './shell.js';
import { startSession } from './domain.js';
import { renderCases, selectCase } from './sites.js';
import { fillSpatialForm } from './spatial.js';

/** `open_case` 的 `profile` → `startSession` 的配置。 */
export function sessionFromProfile(p) {
  const [west, east, south, north] = p.domain ?? [];
  const global = p.domain && west <= -180 && east >= 180 && south <= -90 && north >= 90;
  // 建例记录里的范围类型优先（流域 Shapefile 在 case.nml 里看不出来）；没有记录时按范围推断。
  const domainKind = !p.spatial ? 'site'
    : (p.domain_kind ?? (global ? 'global' : 'region'));
  const domain = { kind: domainKind };
  if (p.domain) Object.assign(domain, { west, east, south, north });
  if (domainKind === 'watershed' && p.shapefile) domain.shapefile = p.shapefile;
  const spatial = !p.spatial ? null : {
    domain,
    grid: p.grid === 'latlon'
      ? {
        kind: 'latlon', meshFile: null,
        dlon: p.resolution?.[0], dlat: p.resolution?.[1],
        nlon: p.resolution ? Math.round(360 / p.resolution[0]) : null,
        nlat: p.resolution ? Math.round(180 / p.resolution[1]) : null,
      }
      : p.grid === 'unstructured'
        ? { kind: 'unstructured', meshFile: p.mesh_file ?? null }
        : { kind: p.grid, input: p.catchment_file ?? null },
  };
  return {
    domain: domainKind,
    grid: p.grid,
    spatial,
    subgrid: p.subgrid,
    soil: p.soil,
    physics: {
      urban: p.urban, lulcc: p.lulcc, bgc: p.bgc, crop: p.crop, tracer: p.methane, river: p.river,
    },
    tracer: p.methane ? 'methane' : null,
    methaneMode: p.methane ? (p.methane_mode ?? null) : null,
    debug: { rangecheck: p.rangecheck, colmdebug: p.colmdebug, srfdatadiag: p.srfdatadiag },
  };
}

/** 首页（全屏）还开着时状态栏被它挡住，错误要写在首页上。 */
function report(message) {
  setStatus(message);
  if (!$('domaingate').hidden) $('gateinfo').textContent = message;
}

export async function openExistingCase() {
  const dir = await invoke('pick_folder', { key: 'open-case' });
  if (!dir) return;
  let opened;
  try {
    opened = await invoke('open_case', { dir });
  } catch (e) {
    report(String(e));
    return;
  }
  // 打开算例走的也是 startSession；告诉「文件与目录」页这次不是新向导，表单不要重置成新建状态。
  state.openingCase = true;
  try {
    startSession(sessionFromProfile(opened.profile));
  } finally {
    state.openingCase = false;
  }
  // 跑过的算例记着上次的内核；按宏匹配到的若不是它，换回它。
  const last = state.kernels.find(k => k.preset === opened.profile.kernel_preset);
  if (last) $('kernel').value = last.dir;
  // 建算例表单里的输入原样填回：站点算例是 RawData/Runtime 与根目录，空间算例是整张表单。
  const inputs = opened.profile.inputs;
  for (const [id, value] of [['root', opened.root], ['rawdata', inputs.rawdata], ['runtime', inputs.runtime]]) {
    const el = $(id);
    if (!el || !value) continue;
    el.value = value;
    el.dispatchEvent(new Event('change'));
  }
  if (opened.profile.spatial) await fillSpatialForm(inputs, opened.root);
  try {
    state.cases = await invoke('list_cases', { root: opened.root });
  } catch {
    state.cases = [opened.entry];
  }
  const entry = state.cases.find(c => c.dir === opened.entry.dir) ?? opened.entry;
  state.createdCases.add(entry.dir);
  state.pickedCases.add(entry.dir);
  renderCases();
  await selectCase(entry);
  renderSteps();
  go('basic-files');
  setStatus(`已打开算例 ${entry.name}`);
}

addEventListener('colm:open-case', openExistingCase);
$('open-case')?.addEventListener('click', openExistingCase);
