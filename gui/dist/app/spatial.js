//! 空间向导到算例目录的最短闭环：生成/采用网格、预检、写 case.nml。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, joinPath } from './ui.js';
import { wizardFields } from './domain.js';
import { renderCases, selectCase } from './sites.js';
import { renderSteps, setStatus } from './shell.js';

const labels = {
  watershed: '流域', region: '区域', global: '全球',
  latlon: '经纬度网格', unstructured: '非结构网格', catchment: '流域网格',
};
const spatialStudyText = '空间算例暂不支持参数调优和不确定性分析。';
const spatialContext = () => JSON.stringify({
  root: $('spatial-root')?.value.trim(), name: $('spatial-name')?.value.trim(),
  domain: state.domain, grid: state.grid, spatial: state.spatial, subgrid: state.subgrid, wizard: state.wizard,
});

/** 下拉框最后一项（改用用户自己的 forcing namelist）的值。 */
const CUSTOM_FORCING = '__custom__';
let forcingDatasetsLoaded = false;

async function loadForcingDatasets() {
  if (forcingDatasetsLoaded) return;
  const select = $('spatial-forcing-dataset');
  let names = [];
  try { names = await invoke('forcing_datasets'); } catch { /* 仍可用已有 namelist */ }
  select.textContent = '';
  for (const name of names) {
    const option = document.createElement('option');
    option.value = name;
    option.textContent = name;
    select.appendChild(option);
  }
  const custom = document.createElement('option');
  custom.value = CUSTOM_FORCING;
  custom.textContent = '使用已有 forcing namelist…';
  select.appendChild(custom);
  select.value = names.includes('JRA3Q') ? 'JRA3Q' : (names[0] ?? CUSTOM_FORCING);
  forcingDatasetsLoaded = true;
  syncForcingMode();
}

function syncForcingMode() {
  const custom = $('spatial-forcing-dataset').value === CUSTOM_FORCING;
  $('spatial-forcing-dir-field').hidden = custom;
  $('spatial-forcing-field').hidden = !custom;
  $('spatial-forcing-note').textContent = custom
    ? '使用你自己准备的 forcing namelist（需含 DEF_dir_forcing 与 DEF_forcing%* 设置）。'
    : '选择数据集后，按 CoLM 的标准模板在算例目录生成 forcing.nml，只把数据目录换成你选的位置。';
}

$('spatial-forcing-dataset').onchange = syncForcingMode;

/** 打开已有空间算例：把它的输入填回建算例表单（强迫场用它自己的 forcing namelist）。 */
export async function fillSpatialForm(inputs, root) {
  await loadForcingDatasets();
  const set = (id, value) => {
    const el = $(id);
    if (!el || value == null || value === '') return;
    el.value = String(value);
    el.dispatchEvent(new Event('change'));
  };
  set('spatial-rawdata', inputs.rawdata);
  set('spatial-runtime', inputs.runtime);
  if (inputs.forcing_namelist) {
    $('spatial-forcing-dataset').value = CUSTOM_FORCING;
    syncForcingMode();
    set('spatial-forcing', inputs.forcing_namelist);
  }
  set('spatial-start', inputs.start);
  set('spatial-end', inputs.end);
  if (Number.isFinite(inputs.timestep) && inputs.timestep > 0) set('spatial-timestep', inputs.timestep);
  set('spatial-root', root);
  set('spatial-name', inputs.name);
  $('spatial-summary').textContent =
    `已打开算例 ${inputs.name}：下面是它现有的设置。改参数、运行直接到后面各页；只有要另建一个新算例时才需要按下面的建算例按钮。`;
}

function syncSpatialSetup() {
  const spatial = state.domain && state.domain !== 'site';
  $('site-case-setup').hidden = spatial;
  $('spatial-case-setup').hidden = !spatial;
  if (!spatial || !state.spatial) return;
  $('spatial-summary').textContent = `${labels[state.domain]} · ${labels[state.grid]} 。`
    + (state.grid === 'unstructured'
      ? '将读取并预检已有 mesh NetCDF，并复用其中 elmindex。'
      : state.grid === 'catchment'
      ? '将预检已准备的 Catchment/HRU NetCDF。'
      : '将按指定范围与分辨率生成网格（海洋按 RawData 的地表覆盖剔除），并生成 int64 空间索引合同。');
  $('make-spatial-case').textContent = state.grid === 'unstructured'
    ? '读取网格、预检并建算例'
    : '生成网格、预检并建算例';
  const warning = $('spatial-early-warning');
  if (warning) warning.textContent = spatialStudyText;
  if (!$('spatial-rawdata').value) $('spatial-rawdata').value = $('rawdata').value;
  if (!$('spatial-runtime').value) $('spatial-runtime').value = $('runtime').value;
  if (!$('spatial-root').value) $('spatial-root').value = $('root').value;
  loadForcingDatasets();
}

/**
 * 日期文本 → `YYYY-MM-DD`；不是真实日期时返回 null。接受 `2010-1-1`、`2010/01/01`、`2010.1.1`、`20100101`。
 * 用文本框而不是 `type="date"`：macOS 的 WKWebView 里日期控件选中年/月/日后键盘输入不可靠。
 */
export function normalizeDate(text) {
  const raw = String(text ?? '').trim();
  const match = /^(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})$/.exec(raw) ?? /^(\d{4})(\d{2})(\d{2})$/.exec(raw);
  if (!match) return null;
  const [year, month, day] = match.slice(1).map(Number);
  const date = new Date(Date.UTC(year, month - 1, day));
  if (date.getUTCFullYear() !== year || date.getUTCMonth() !== month - 1 || date.getUTCDate() !== day) {
    return null;
  }
  return `${String(year).padStart(4, '0')}-${String(month).padStart(2, '0')}-${String(day).padStart(2, '0')}`;
}

for (const id of ['spatial-start', 'spatial-end']) {
  $(id).onchange = () => {
    const value = normalizeDate($(id).value);
    if (value) $(id).value = value;
  };
}

function problem() {
  const required = [
    ['spatial-rawdata', '请选择 RawData 目录'],
    ['spatial-runtime', '请选择 Runtime 目录'],
    $('spatial-forcing-dataset').value === CUSTOM_FORCING
      ? ['spatial-forcing', '请选择已有 forcing namelist']
      : ['spatial-forcing-dir', '请选择强迫数据目录'],
    ['spatial-start', '请选择开始日期'],
    ['spatial-end', '请选择结束日期'],
    ['spatial-root', '请选择算例根目录'],
    ['spatial-name', '请输入算例名称'],
  ];
  const missing = required.find(([id]) => !$(id).value.trim());
  if (missing) return missing[1];
  if ($('spatial-root').value.includes(' ') || $('spatial-name').value.includes(' ')) {
    return '算例路径不能含空格';
  }
  const start = normalizeDate($('spatial-start').value);
  const end = normalizeDate($('spatial-end').value);
  if (!start) return '开始日期格式应为 YYYY-MM-DD，且是真实日期';
  if (!end) return '结束日期格式应为 YYYY-MM-DD，且是真实日期';
  $('spatial-start').value = start;
  $('spatial-end').value = end;
  if (start > end) return '开始日期不能晚于结束日期';
  const timestep = Number($('spatial-timestep').value);
  if (!(timestep > 0)) return '时间步长必须大于 0';
  return null;
}

$('make-spatial-case').onclick = async () => {
  const error = problem();
  $('spatial-error').hidden = !error;
  $('spatial-error').textContent = error ?? '';
  if (error) return;
  const button = $('make-spatial-case');
  button.disabled = true;
  const root = $('spatial-root').value.trim();
  const name = $('spatial-name').value.trim();
  const context = spatialContext();
  const wizardRef = state.wizard;
  const stillCurrent = () => context === spatialContext() && wizardRef === state.wizard;
  const out = joinPath(root, name);
  const domain = state.spatial.domain;
  const grid = state.spatial.grid;
  setStatus('正在生成并预检空间算例…');
  try {
    await invoke('new_spatial_case', {
      domain: domain.kind, gridKind: grid.kind,
      shapefile: domain.shapefile ?? null,
      west: domain.west ?? null, east: domain.east ?? null,
      south: domain.south ?? null, north: domain.north ?? null,
      dlon: grid.dlon ?? null, dlat: grid.dlat ?? null,
      meshFile: grid.meshFile ?? null,
      catchmentFile: grid.input ?? null,
      out, name,
      ...($('spatial-forcing-dataset').value === CUSTOM_FORCING
        ? { forcing: $('spatial-forcing').value.trim(), forcingDataset: null, forcingDir: null }
        : {
          forcing: null,
          forcingDataset: $('spatial-forcing-dataset').value,
          forcingDir: $('spatial-forcing-dir').value.trim(),
        }),
      rawdata: $('spatial-rawdata').value.trim(),
      runtime: $('spatial-runtime').value.trim(),
      start: $('spatial-start').value, end: $('spatial-end').value,
      timestep: Number($('spatial-timestep').value),
      mode: String(state.subgrid ?? 'IGBP').toLowerCase(),
      fields: wizardFields(),
    });
    if (!stillCurrent()) {
      setStatus(`空间算例 ${name} 已生成；当前已切换工作流，未加入本次算例列表`);
      return;
    }
    const cases = await invoke('list_cases', { root });
    if (!stillCurrent()) {
      setStatus(`空间算例 ${name} 已生成；当前已切换工作流，未加入本次算例列表`);
      return;
    }
    state.createdCases.add(out);
    state.cases = cases;
    const made = state.cases.find(c => c.dir === out) ?? state.cases.find(c => c.name === name);
    if (!made) throw new Error('算例已生成，但重新扫描时没有找到它');
    state.batch = [made.dir];
    state.pickedCases.clear();
    state.pickedCases.add(made.dir);
    await selectCase(made);
    if (!stillCurrent()) {
      setStatus(`空间算例 ${name} 已生成；当前已切换工作流，未加入本次算例列表`);
      return;
    }
    renderCases();
    renderSteps();
    setStatus(`空间算例 ${name} 已通过预检`);
  } catch (e) {
    $('spatial-error').hidden = false;
    $('spatial-error').textContent = String(e);
    setStatus(e);
  } finally {
    button.disabled = false;
  }
};

// 新向导是一次新任务：上一个（可能是打开的）算例的名称与强迫场不能带进来；日期保留，常常沿用。
function resetSpatialForm() {
  if (state.openingCase) return;
  $('spatial-name').value = 'spatial-case';
  $('spatial-forcing').value = '';
  if (forcingDatasetsLoaded) {
    const select = $('spatial-forcing-dataset');
    const names = [...select.options].map(o => o.value).filter(v => v !== CUSTOM_FORCING);
    select.value = names.includes('JRA3Q') ? 'JRA3Q' : (names[0] ?? CUSTOM_FORCING);
    syncForcingMode();
  }
}

addEventListener('colm:wizard', () => { resetSpatialForm(); syncSpatialSetup(); });
