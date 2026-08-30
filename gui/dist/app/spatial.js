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

function syncSpatialSetup() {
  const spatial = state.domain && state.domain !== 'site';
  $('site-case-setup').hidden = spatial;
  $('spatial-case-setup').hidden = !spatial;
  if (!spatial || !state.spatial) return;
  $('spatial-summary').textContent = `${labels[state.domain]} · ${labels[state.grid]}。`
    + (state.grid === 'catchment'
      ? '将预检已准备的 Catchment/HRU NetCDF。'
      : '将按指定分辨率生成网格，以 mask 剔除海洋和范围外单元，并生成 int64 空间索引合同。');
  if (!$('spatial-rawdata').value) $('spatial-rawdata').value = $('rawdata').value;
  if (!$('spatial-runtime').value) $('spatial-runtime').value = $('runtime').value;
  if (!$('spatial-root').value) $('spatial-root').value = $('root').value;
}

function problem() {
  const required = [
    ['spatial-rawdata', '请选择 rawdata 目录'],
    ['spatial-runtime', '请选择 runtime 目录'],
    ['spatial-forcing', '请选择空间强迫场 namelist'],
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
  if ($('spatial-start').value > $('spatial-end').value) return '开始日期不能晚于结束日期';
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
      nonOceanMask: grid.nonOceanMask ?? null,
      catchmentFile: grid.input ?? null,
      out, name,
      forcing: $('spatial-forcing').value.trim(),
      rawdata: $('spatial-rawdata').value.trim(),
      runtime: $('spatial-runtime').value.trim(),
      start: $('spatial-start').value, end: $('spatial-end').value,
      timestep: Number($('spatial-timestep').value),
      mode: String(state.subgrid ?? 'IGBP').toLowerCase(),
      fields: wizardFields(),
    });
    state.createdCases.add(out);
    state.cases = await invoke('list_cases', { root });
    const made = state.cases.find(c => c.dir === out) ?? state.cases.find(c => c.name === name);
    if (!made) throw new Error('算例已生成，但重新扫描时没有找到它');
    state.batch = [made.dir];
    state.pickedCases.clear();
    state.pickedCases.add(made.dir);
    await selectCase(made);
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

addEventListener('colm:wizard', syncSpatialSetup);
