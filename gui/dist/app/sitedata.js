//! Preprocessing workbench: create a standard site file, expose the complete
//! mksrfdata contract, and hand the generated dataset to Basic Settings.

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, joinPath, forcingDirectoryForSiteDirectory } from './ui.js';
import {
  normalizeSiteStem, parentDirectory, prepMode, siteOutputName,
} from './prep-state.js';
import { scanPreparedSites } from './sites.js';
import { go } from './shell.js';

let result = null;
const REQUIRED_FIELD_COUNT = 12;

const MODE_LABELS = {
  igbp: 'IGBP 自然站点',
  usgs: 'USGS 自然站点',
  pft: 'PFT 自然站点',
  pc: 'PC 自然站点',
  urban: 'URBAN 城市站点',
};

function parseLandtype() {
  if (prepMode(state) === 'urban') return { value: null };
  const raw = $('slandtype').value.trim();
  if (!raw) return { value: null };
  const n = Number(raw);
  if (!Number.isInteger(n)) return { error: '地表覆盖类型必须是整数；留空则由 rawdata 提供' };
  return { value: n };
}

function syncIdentity() {
  const stem = normalizeSiteStem($('sname').value);
  $('soutname').value = stem ? siteOutputName(stem) : '_site.nc';
  const mode = prepMode(state);
  $('smode').value = MODE_LABELS[mode] ?? mode.toUpperCase();
  $('slandtype').disabled = mode === 'urban';
  $('slandtype').placeholder = mode === 'urban' ? '城市模式由向导固定' : '留空 = 由 rawdata 提供';
  $('slandtype-label').firstChild.textContent = mode === 'usgs'
    ? 'USGS 地表覆盖类型 '
    : '地表覆盖类型 ';
}

function readyReasons() {
  const reasons = [];
  const stem = normalizeSiteStem($('sname').value);
  const lon = Number($('slon').value.trim());
  const lat = Number($('slat').value.trim());
  if (!stem) reasons.push('站点名必填');
  if (!$('slon').value.trim() || !Number.isFinite(lon) || lon < -180 || lon > 180) {
    reasons.push('经度必须在 -180 到 180 之间');
  }
  if (!$('slat').value.trim() || !Number.isFinite(lat) || lat < -90 || lat > 90) {
    reasons.push('纬度必须在 -90 到 90 之间');
  }
  const lt = parseLandtype();
  if (lt.error) reasons.push(lt.error);
  if (!$('soutdir').value.trim()) reasons.push('先选择站点数据目录');
  return reasons;
}

function updateGenerateState() {
  syncIdentity();
  const reasons = readyReasons();
  $('smake').disabled = reasons.length > 0;
  $('smake-why').className = (reasons.length ? 'fail' : 'muted') + ' mini';
  $('smake-why').textContent = reasons.length
    ? reasons.join('；')
    : '可以生成结构文件；生成后会继续检查当前模式的完整运行契约。';
}

function invalidateSite() {
  result = null;
  Object.assign(state.prepArtifacts, {
    siteStem: null,
    siteFile: null,
    siteDir: null,
    siteReport: null,
    forcingFile: null,
    forcingDir: null,
  });
  globalThis.dispatchEvent?.(new Event('colm:prep-site-invalidated'));
  renderResult();
  renderPrepReady();
  updateGenerateState();
}

for (const id of ['sname', 'slon', 'slat', 'slandtype', 'soutdir', 'srawdata']) {
  const el = $(id);
  if (el) el.addEventListener('input', invalidateSite);
}
globalThis.addEventListener?.('colm:wizard', () => {
  invalidateSite();
  syncIdentity();
});
updateGenerateState();

$('smake').onclick = async () => {
  const reasons = readyReasons();
  if (reasons.length) { status(reasons.join('；')); return; }
  const stem = normalizeSiteStem($('sname').value);
  const outDir = $('soutdir').value.trim();
  const rawdataDir = $('srawdata').value.trim();
  const out = joinPath(outDir, siteOutputName(stem));
  const { value: landtype } = parseLandtype();
  $('smake').disabled = true;
  try {
    result = await invoke('make_site', {
      out,
      lon: Number($('slon').value.trim()),
      lat: Number($('slat').value.trim()),
      landtype,
      rawdata: rawdataDir || null,
      mode: prepMode(state),
    });
    Object.assign(state.prepArtifacts, {
      siteStem: stem,
      siteFile: result.path,
      siteDir: parentDirectory(result.path),
      siteReport: result,
      rawdataDir: rawdataDir || null,
    });
    await adoptPreparedSite(result);
    status(result.readiness === 'blocked'
      ? `已生成 ${result.path}，但当前模式还缺 ${result.needs_external.length} 项外部数据`
      : `站点数据已生成并交给基本设定：${result.path}`);
  } catch (error) {
    result = null;
    Object.assign(state.prepArtifacts, {
      siteStem: null, siteFile: null, siteDir: null, siteReport: null,
    });
    status(error);
  } finally {
    renderResult();
    renderPrepReady();
    updateGenerateState();
  }
};

export async function adoptPreparedSite(report = state.prepArtifacts.siteReport) {
  if (!report?.path) return null;
  const dir = parentDirectory(report.path);
  $('sitedir').value = dir;
  $('forcingdir').value = state.prepArtifacts.forcingDir
    || forcingDirectoryForSiteDirectory(dir);
  if ($('rawdata')) $('rawdata').value = state.prepArtifacts.rawdataDir ?? '';
  const selected = await scanPreparedSites(report.path);
  if (selected?.met_file && !state.prepArtifacts.forcingFile) {
    state.prepArtifacts.forcingFile = selected.met_file;
    state.prepArtifacts.forcingDir = parentDirectory(selected.met_file);
    $('forcingdir').value = state.prepArtifacts.forcingDir;
  }
  renderPrepReady();
  return selected;
}

function sourceGroup(title, fields, cls, note) {
  const wrap = document.createElement('div');
  wrap.style.marginTop = '14px';
  const heading = document.createElement('div');
  heading.className = 'mini' + (cls ? ` ${cls}` : '');
  heading.style.fontWeight = '650';
  heading.textContent = `${title}（${fields.length}）`;
  wrap.appendChild(heading);
  if (note) {
    const p = document.createElement('p');
    p.className = (cls || 'muted') + ' mini';
    p.style.margin = '4px 0 0';
    p.textContent = note;
    wrap.appendChild(p);
  }
  const list = document.createElement('div');
  list.className = 'mini';
  list.style.marginTop = '4px';
  list.textContent = fields.length ? fields.join(' · ') : '（无）';
  wrap.appendChild(list);
  return wrap;
}

function readinessCopy(report) {
  if (report.readiness === 'self_contained') {
    return ['可独立运行', '站点文件自身满足当前模式，不需要 CoLM 全球 rawdata。', ''];
  }
  if (report.readiness === 'ready_with_rawdata') {
    return ['可随 rawdata 运行', `站点文件还缺 ${report.needs_external.length} 项；已选择的 rawdata 将在 mksrfdata 阶段提供。`, 'warn'];
  }
  return ['尚不可运行', `缺少 ${report.needs_external.length} 项且没有可用 rawdata。文件可以保存，但建例会被阻止。`, 'fail'];
}

function renderResult() {
  const box = $('site-result');
  box.textContent = '';
  if (!result) return;
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = '<h3>生成结果与运行契约</h3>';
  const [title, detail, cls] = readinessCopy(result);
  const banner = document.createElement('div');
  banner.className = `ch${cls ? ` ${cls}` : ''}`;
  banner.textContent = `${title}：${detail}`;
  card.appendChild(banner);

  const total = result.from_site.length + result.from_raster.length + result.from_default.length;
  const table = document.createElement('table');
  table.innerHTML = `
    <tr><th>产物</th><td><code>${result.path}</code></td></tr>
    <tr><th>模式</th><td>${String(result.mode).toUpperCase()} · ${result.site_kind === 'urban' ? '城市' : '自然'}站点</td></tr>
    <tr><th>结构字段</th><td>${total}/${REQUIRED_FIELD_COUNT}</td></tr>
    <tr><th>质地</th><td>${result.texture_name}（第 ${result.texture} 类），BVIC ${result.bvic}</td></tr>`;
  card.appendChild(table);
  card.appendChild(sourceGroup('来自站点自身', result.from_site, '', ''));
  card.appendChild(sourceGroup('来自 rawdata 栅格', result.from_raster, '', ''));
  card.appendChild(sourceGroup(
    '有依据的查表值', result.from_lookup, 'warn',
    '查表值不是站点实测；只有对应方案确有模型查表依据时才写入。',
  ));
  card.appendChild(sourceGroup(
    '标称或模块默认值', result.from_default, 'warn',
    '这些值不是本站实测值；结果解释时必须保留这一数据来源限制。',
  ));
  card.appendChild(sourceGroup(
    '仍需外部数据', result.needs_external, result.needs_external.length ? (result.readiness === 'blocked' ? 'fail' : 'warn') : '',
    result.needs_external.length
      ? '清单来自 mksrfdata 的模式契约，不再只检查 12 个结构字段。'
      : '当前站点文件已经自包含。',
  ));
  box.appendChild(card);
}

export function renderPrepReady() {
  const box = $('prep-ready-summary');
  if (!box) return;
  box.textContent = '';
  const a = state.prepArtifacts;
  const rows = [
    ['站点文件', a.siteFile, a.siteReport?.readiness ?? '未生成'],
    ['强迫场', a.forcingFile, a.forcingFile ? '已匹配标准文件' : '未准备'],
    ['rawdata', a.rawdataDir, a.rawdataDir ? '已选择' : '未选择'],
  ];
  const table = document.createElement('table');
  for (const [label, path, stateText] of rows) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<th>${label}</th><td>${path ? `<code>${path}</code>` : '—'}<div class="muted mini">${stateText}</div></td>`;
    table.appendChild(tr);
  }
  box.appendChild(table);
  const runnableSite = a.siteReport && a.siteReport.readiness !== 'blocked';
  const matched = runnableSite && !!a.forcingFile;
  $('prep-use').disabled = !matched;
  if (!matched) {
    const p = document.createElement('p');
    p.className = 'warn mini';
    p.textContent = !runnableSite
      ? '先生成一份可独立运行或可随 rawdata 运行的站点文件。'
      : '站点数据已就绪；还需要在“强迫场”子步骤完成转换。';
    box.appendChild(p);
  }
}

$('prep-use').onclick = async () => {
  if ($('prep-use').disabled) return;
  await adoptPreparedSite();
  go('basic-files');
};

globalThis.addEventListener?.('colm:prep-artifacts', renderPrepReady);
renderPrepReady();
