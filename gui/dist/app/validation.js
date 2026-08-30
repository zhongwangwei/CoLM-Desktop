//! Optional observation-table workbench. It creates the same sibling
//! Observation/<site>_Flux.nc files already consumed by results and tuning.

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, joinPath, status } from './ui.js';
import { normalizeSiteStem, parentDirectory } from './prep-state.js';
import { adoptPreparedSite } from './sitedata.js';
import { go } from './shell.js';

export const VALIDATION_TABLE_EXAMPLE = `time,site,Qle[W m-2],Qle_qc,Qh[W m-2],Qh_qc,Rnet[W m-2],Rnet_qc,GPP[umol m-2 s-1]
2020-01-01 00:00:00,SITE-A,12.4,0,35.2,0,-18.5,0,0.0
2020-01-01 00:30:00,SITE-A,13.1,0,33.8,0,-16.9,0,0.0
2020-01-01 00:00:00,SITE-B,42.0,0,18.3,0,-5.2,0,0.0
2020-01-01 00:30:00,SITE-B,44.1,0,17.9,0,-2.1,0,0.0`;

let probe = null;
let busy = false;
let outputs = [];
let settings = null;
let fieldId = 0;

function observationDirectory() {
  const dir = state.prepArtifacts.siteDir || state.prepArtifacts.forcingDir;
  const parent = parentDirectory(dir);
  return parent ? joinPath(parent, 'Observation') : '';
}

function optionSelect(values, selected, noneLabel = null) {
  const select = document.createElement('select');
  select.className = 'input';
  if (noneLabel !== null) {
    const option = document.createElement('option');
    option.value = '';
    option.textContent = noneLabel;
    select.appendChild(option);
  }
  for (const value of values) {
    const option = document.createElement('option');
    option.value = value;
    const units = probe?.columns?.find(column => column.name === value)?.units;
    option.textContent = units ? `${value} [${units}]` : value;
    select.appendChild(option);
  }
  select.value = selected ?? '';
  return select;
}

function field(label, control) {
  const wrap = document.createElement('div');
  wrap.className = 'field';
  const heading = document.createElement('label');
  control.id ||= `validation-field-${++fieldId}`;
  heading.htmlFor = control.id;
  heading.textContent = label;
  wrap.append(heading, control);
  return wrap;
}

function tableRow(values, header = false) {
  const row = document.createElement('tr');
  for (const value of values) {
    const cell = document.createElement(header ? 'th' : 'td');
    if (value instanceof Node) cell.appendChild(value);
    else cell.textContent = value ?? '—';
    row.appendChild(cell);
  }
  return row;
}

function resetSettings() {
  const columns = probe.columns.map(column => column.name);
  settings = {
    time: probe.time_column ?? columns.find(name => /^time|timestamp|datetime$/i.test(name)) ?? '',
    site: probe.site_column ?? '',
    siteName: state.prepArtifacts.siteStem
      ?? (!probe.site_column && probe.sites?.length === 1 ? probe.sites[0].id : ''),
    dst: observationDirectory(),
    variables: Object.fromEntries((probe.variables ?? []).map(variable => [
      variable.name,
      { column: variable.column ?? '', qc: variable.qc_column ?? '' },
    ])),
  };
}

function readinessReasons() {
  const reasons = [];
  if (!settings.time) reasons.push('请选择时间列');
  if (!settings.site && !settings.siteName.trim()) reasons.push('没有站点列时必须填写单站名称');
  if (!settings.dst.trim()) reasons.push('先完成站点数据或强迫场，以确定同一数据集的 Observation 目录');
  if (!Object.values(settings.variables).some(choice => choice.column)) reasons.push('至少映射一个可评估变量');
  return reasons;
}

function renderProbeSummary(box) {
  const card = document.createElement('div');
  card.className = 'card';
  const title = document.createElement('h3');
  title.textContent = '表格结构与站点';
  const summary = document.createElement('table');
  summary.append(
    tableRow(['分隔方式', probe.delimiter]),
    tableRow(['数据行', String(probe.rows)]),
    tableRow(['识别到的站点', String(probe.sites?.length ?? 0)]),
  );
  card.append(title, summary);
  if (probe.sites?.length) {
    const sites = document.createElement('table');
    sites.style.marginTop = '10px';
    sites.appendChild(tableRow(['站点', '行数', '时间范围'], true));
    for (const site of probe.sites) {
      sites.appendChild(tableRow([
        site.id,
        String(site.rows),
        site.start && site.end ? `${site.start} — ${site.end}` : '—',
      ]));
    }
    card.appendChild(sites);
  }
  box.appendChild(card);
}

function renderConfiguration(box) {
  const columns = probe.columns.map(column => column.name);
  const card = document.createElement('div');
  card.className = 'card';
  const title = document.createElement('h3');
  title.textContent = '确认时间、站点与变量映射';
  const note = document.createElement('div');
  note.className = 'ch';
  note.textContent = '时间标签保持原样，必须与对应强迫场一致：本地时强迫场就提供站点本地时，UTC 强迫场就提供 UTC。这里不猜单位换算；QC=0 表示可用观测。';
  card.append(title, note);

  const identity = document.createElement('div');
  identity.className = 'table-grid';
  const time = optionSelect(columns, settings.time, '请选择一列');
  time.onchange = () => { settings.time = time.value; render(); };
  const site = optionSelect(columns, settings.site, '（没有 / 单站）');
  site.onchange = () => { settings.site = site.value; render(); };
  const siteName = document.createElement('input');
  siteName.className = 'input';
  siteName.value = settings.siteName;
  siteName.placeholder = '例如 AT-Neu';
  siteName.oninput = () => { settings.siteName = siteName.value; updateConvertState(card); };
  identity.append(
    field('时间列', time),
    field('站点名称列', site),
    field('单站名称（没有站点列时）', siteName),
  );
  card.appendChild(identity);

  const mapping = document.createElement('table');
  mapping.appendChild(tableRow(['目标变量', '目标单位', '数值列', 'QC 列（可选）'], true));
  for (const variable of probe.variables ?? []) {
    const { name, label, units } = variable;
    const choice = settings.variables[name];
    const value = optionSelect(columns, choice.column, '（不使用）');
    value.onchange = () => { choice.column = value.value; updateConvertState(card); };
    const requiresQc = !!variable.requires_qc;
    const qc = optionSelect(columns, choice.qc, requiresQc ? '（自动生成）' : '（该变量无 QC 契约）');
    qc.disabled = !requiresQc;
    qc.onchange = () => { choice.qc = qc.value; };
    mapping.appendChild(tableRow([`${name} · ${label}`, units, value, qc]));
  }
  card.appendChild(mapping);

  const dst = document.createElement('input');
  dst.className = 'input';
  dst.id = 'vdst';
  dst.value = settings.dst;
  dst.placeholder = '…/Observation';
  dst.readOnly = true;
  card.appendChild(field('验证数据产物目录（按数据集自动确定）', dst));

  const actions = document.createElement('div');
  actions.className = 'pill-row';
  const convert = document.createElement('button');
  convert.id = 'validation-convert';
  convert.className = 'btn-next';
  convert.type = 'button';
  convert.textContent = busy ? '正在制作验证数据…' : '制作标准验证数据';
  convert.onclick = convertTable;
  const why = document.createElement('span');
  why.id = 'validation-why';
  why.className = 'muted mini';
  actions.append(convert, why);
  card.appendChild(actions);
  box.appendChild(card);
  updateConvertState(card);
}

function updateConvertState(card) {
  const reasons = readinessReasons();
  const button = card.querySelector('#validation-convert');
  const why = card.querySelector('#validation-why');
  if (button) button.disabled = busy || reasons.length > 0;
  if (why) {
    why.className = (reasons.length ? 'warn' : 'muted') + ' mini';
    why.textContent = reasons.length ? reasons.join('；') : '就绪：原始表格保留不变，每个站点生成一份标准观测文件。';
  }
}

function renderOutputs(box) {
  if (!outputs.length) return;
  const card = document.createElement('div');
  card.className = 'card';
  const title = document.createElement('h3');
  title.textContent = '验证数据制作结果';
  const table = document.createElement('table');
  table.appendChild(tableRow(['站点', '行数', '变量', '产物'], true));
  for (const output of outputs) {
    table.appendChild(tableRow([
      output.site,
      String(output.rows),
      (output.variables ?? []).join('、'),
      output.path,
    ]));
  }
  const note = document.createElement('p');
  note.className = 'muted mini';
  note.textContent = '已按命名约定写入 Observation；进入基本设定重新扫描后会自动用于结果评估与参数调优。';
  card.append(title, table, note);
  const warnings = outputs.flatMap(output => output.warnings ?? []);
  if (warnings.length) {
    const warning = document.createElement('p');
    warning.className = 'warn mini';
    warning.textContent = `质控提示：${warnings.join('；')}`;
    card.appendChild(warning);
  }
  box.appendChild(card);
}

function render() {
  const box = $('validation-cards');
  box.textContent = '';
  if (!probe) return;
  renderProbeSummary(box);
  renderConfiguration(box);
  renderOutputs(box);
}

async function probeTable() {
  const path = $('vsrc').value.trim();
  if (!path || busy) { status('请先选择验证数据表格'); return; }
  busy = true;
  $('vprobe').disabled = true;
  try {
    probe = await invoke('probe_observation_table', { path });
    outputs = [];
    resetSettings();
    status(`已识别 ${probe.rows} 行、${probe.sites?.length ?? 0} 个站点和 ${probe.variables?.filter(item => item.column).length ?? 0} 个变量`);
  } catch (error) {
    probe = null;
    settings = null;
    status(error);
  } finally {
    busy = false;
    $('vprobe').disabled = false;
    render();
  }
}

async function convertTable() {
  const reasons = readinessReasons();
  if (reasons.length || busy) { status(reasons.join('；')); return; }
  busy = true;
  render();
  try {
    const variables = Object.entries(settings.variables)
      .filter(([, choice]) => choice.column)
      .map(([name, choice]) => ({ name, column: choice.column, qc_column: choice.qc || null }));
    const options = {
      time_column: settings.time,
      site_column: settings.site || null,
      site_name: settings.siteName.trim() || null,
      variables,
    };
    outputs = await invoke('convert_observation_table', {
      src: $('vsrc').value.trim(),
      dstDir: settings.dst.trim(),
      options,
    });
    const a = state.prepArtifacts;
    a.observationDir = settings.dst.trim();
    for (const output of outputs) {
      const stem = normalizeSiteStem(output.site);
      const batch = a.batchSites?.find(item => normalizeSiteStem(item.site) === stem);
      if (batch) batch.observationFile = output.path;
      if (!a.siteStem || a.siteStem === stem) a.observationFile = output.path;
    }
    globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
    if (a.siteReport?.path && outputs.length === 1) await adoptPreparedSite();
    status(`已制作 ${outputs.length} 份标准验证数据`);
  } catch (error) {
    outputs = [];
    status(error);
  } finally {
    busy = false;
    render();
  }
}

function downloadText(text, filename) {
  const link = document.createElement('a');
  link.href = URL.createObjectURL(new Blob([text], { type: 'text/csv;charset=utf-8' }));
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 0);
}

$('validation-table-example').textContent = VALIDATION_TABLE_EXAMPLE;
$('validation-example-copy').onclick = async () => {
  try {
    await navigator.clipboard.writeText(VALIDATION_TABLE_EXAMPLE);
    status('验证数据 CSV 样例已复制');
  } catch (error) { status(error); }
};
$('validation-example-download').onclick = () => downloadText(VALIDATION_TABLE_EXAMPLE, 'colm-validation-example.csv');
$('validation-skip').onclick = () => go('prep-ready');
$('vprobe').onclick = probeTable;
$('vsrc').addEventListener('input', () => { probe = null; settings = null; outputs = []; render(); });
globalThis.addEventListener?.('colm:prep-artifacts', () => {
  if (!settings) return;
  settings.dst = observationDirectory();
  render();
});
globalThis.addEventListener?.('colm:prep-site-invalidated', () => {
  if (!settings) return;
  settings.dst = '';
  outputs = [];
  render();
});
