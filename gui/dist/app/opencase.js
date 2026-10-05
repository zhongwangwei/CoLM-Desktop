//! 打开磁盘上已有的算例：从它的 `case.nml`（与跑过时的 `stages.json`）反推出等价的向导选择，
//! 走与向导同一条 `startSession`，再把这个算例放进本次列表并选中。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $ } from './ui.js';
import { go, renderSteps, setStatus } from './shell.js';
import { startSession } from './domain.js';
import { renderCases, selectCase } from './sites.js';

/** `open_case` 的 `profile` → `startSession` 的配置。 */
export function sessionFromProfile(p) {
  const [west, east, south, north] = p.domain ?? [];
  const global = p.domain && west <= -180 && east >= 180 && south <= -90 && north >= 90;
  const domainKind = !p.spatial ? 'site' : (global ? 'global' : 'region');
  const spatial = !p.spatial ? null : {
    domain: p.domain ? { kind: domainKind, west, east, south, north } : { kind: domainKind },
    grid: p.grid === 'latlon'
      ? {
        kind: 'latlon', meshFile: null,
        dlon: p.resolution?.[0], dlat: p.resolution?.[1],
        nlon: p.resolution ? Math.round(360 / p.resolution[0]) : null,
        nlat: p.resolution ? Math.round(180 / p.resolution[1]) : null,
      }
      : { kind: p.grid },
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
    debug: { rangecheck: p.rangecheck, colmdebug: p.colmdebug, srfdatadiag: p.srfdatadiag },
  };
}

export async function openExistingCase() {
  const dir = await invoke('pick_folder', { key: 'open-case' });
  if (!dir) return;
  let opened;
  try {
    opened = await invoke('open_case', { dir });
  } catch (e) {
    setStatus(String(e));
    return;
  }
  startSession(sessionFromProfile(opened.profile));
  // 跑过的算例记着上次的内核；按宏匹配到的若不是它，换回它。
  const last = state.kernels.find(k => k.preset === opened.profile.kernel_preset);
  if (last) $('kernel').value = last.dir;
  for (const id of opened.profile.spatial ? ['root', 'spatial-root'] : ['root']) {
    const el = $(id);
    if (!el) continue;
    el.value = opened.root;
    el.dispatchEvent(new Event('change'));
  }
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
