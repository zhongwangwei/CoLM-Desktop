//! 算例库扫描、选中、新建。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { renderFields } from './params.js';
import { refreshVars } from './results.js';
import { refreshPresets } from './presets.js';
import { go, renderSteps, setStatus } from './shell.js';

$('rescan').onclick = async () => {
  try {
    state.cases = await invoke('list_cases', { root: $('root').value.trim() });
    renderCases();
  } catch (e) { $('status').textContent = String(e); }
};

export function renderCases() {
  const box = $('cases');
  $('runall').disabled = !state.cases.length;
  box.textContent = '';
  if (!state.cases.length) {
    box.innerHTML = '<p class="muted" style="font-size:11px">这个目录下没有算例</p>';
    return;
  }
  for (const c of state.cases) {
    const d = document.createElement('div');
    d.className = 'case';
    d.setAttribute('aria-selected', String(state.selected?.dir === c.dir));
    const s = document.createElement('small');
    // 本次批次里的状态优先 —— 「已跑过」说的是历史，「运行中」说的是现在。
    s.textContent = state.runState[c.dir] ?? (c.has_history ? '已跑过' : '未跑');
    d.textContent = c.name;
    d.appendChild(s);
    d.onclick = () => selectCase(c);
    box.appendChild(d);
  }
}

async function selectCase(c) {
  state.selected = c;
  renderSteps();
  renderCases();
  $('run').disabled = false;
  try {
    state.text = await invoke('read_text', { path: c.dir + '/case.nml' });
    const unknown = await invoke('unknown_fields', { text: state.text });
    const u = $('unknown');
    u.textContent = '';
    if (unknown.length) {
      // 上游自己发布的单点示例就设了两个已删除的字段，CoLM 读到会
      // `Cannot match namelist object name` 然后停。在开跑前说，别让人对着报错发呆。
      const p = document.createElement('p');
      p.className = 'warn';
      p.style.fontSize = '12px';
      p.textContent =
        `这份配置有 ${unknown.length} 个 CoLM 已经不认识的字段，会让运行在读取时就停：` +
        unknown.join('、');
      u.appendChild(p);
    }
    renderFields();
    refreshVars();
    refreshPresets();
  } catch (e) { $('status').textContent = String(e); }
}

$('create').onclick = async () => {
  const site = $('w-site').value.trim();
  const root = $('root').value.trim();
  if (!site || !root) { $('status').textContent = '要先填站点文件与算例目录'; return; }
    // Windows 上的分隔符是反斜杠，两种都认
  const stem = site.split(/[\\/]/).pop();
  const name = $('w-name').value.trim() || stem.split('_')[0];
  $('create').disabled = true;
  try {
    const msg = await invoke('new_case', {
      site, out: root + '/' + name, name,
      start: $('w-start').value.trim() || null,
      end: $('w-end').value.trim() || null,
      rawdata: $('rawdata').value.trim() || null,
      runtime: $('runtime').value.trim() || null,
    });
    $('log').textContent = msg;
    state.cases = await invoke('list_cases', { root });
    renderCases();
    $('status').textContent = '算例已建好';
  } catch (e) { $('status').textContent = String(e); }
  finally { $('create').disabled = false; }
};

// ---------------------------------------------------------------- 站点库

/** 扫一个 Sitedata 目录。两套命名约定都认，判据在 `colm-cli scan`。 */
$('scan').onclick = async () => {
  const dir = $('sitedir').value.trim();
  if (!dir) { status('要先填 Sitedata 目录'); return; }
  $('scan').disabled = true;
  try {
    // quick: 只读站点文件。实测 90 站 0.07 秒，而完整读要 0.35 秒 ——
    // 第一屏只要经纬度与地类，强迫场的时间范围等选中了再补。
    const r = await invoke('scan_sites', { dir, quick: true });
    state.sites = r.sites;
    renderSites(r);
  } catch (e) { status(e); }
  finally { $('scan').disabled = false; }
};

function renderSites(r = {}) {
  const box = $('sites');
  box.textContent = '';
  // **空结果要自己解释。** 指错目录是第一次用最容易发生的事
  // （比如指了 Forcing 而站点文件在 Sitedata），而一个空列表什么都没说。
  if (r.hint) {
    const p = document.createElement('p');
    p.className = 'warn';
    p.style.fontSize = '11px';
    p.textContent = r.hint;
    box.appendChild(p);
    if (r.suggest) {
      const b = document.createElement('button');
      b.textContent = '改用它并重新扫描';
      b.onclick = () => {
        $('sitedir').value = r.suggest;
        $('sitedir').dispatchEvent(new Event('change'));
        $('scan').click();
      };
      box.appendChild(b);
    }
    $('sitesummary').textContent = '\u00a0';
    return;
  }
  const bad = state.sites.filter(s => s.problem).length;
  const noObs = state.sites.filter(s => !s.obs_file).length;
  const urban = state.sites.filter(s => s.urban).length;
  // 把「有多少不能跑 / 不能评估」直接说出来。让人自己数一列图标，
  // 等于把一次可以立刻回答的问题推给用户。
  $('sitesummary').textContent =
    `${state.sites.length} 个站点` +
    (urban ? ` · ${urban} 个城市` : '') +
    (noObs ? ` · ${noObs} 个无观测` : '') +
    (bad ? ` · ${bad} 个读不了` : '');

  for (const s of state.sites) {
    const d = document.createElement('div');
    d.className = 'case';
    d.textContent = s.name;
    const small = document.createElement('small');
    const tags = [];
    if (s.urban) tags.push('城市');
    if (!s.met_file) tags.push('无强迫场');
    if (!s.obs_file) tags.push('无观测');
    if (s.problem) tags.push('读不了');
    small.textContent = tags.length ? tags.join(' · ') : `${s.lat.toFixed(2)}, ${s.lon.toFixed(2)}`;
    if (s.problem) { d.className += ' warn'; d.title = s.problem; }
    d.appendChild(small);
    // 选中就把路径填进新建向导 —— 那是它唯一的去处，不必再让人复制粘贴。
    d.onclick = () => {
      $('w-site').value = s.site_file;
      $('w-site').dispatchEvent(new Event('change'));
      // 城市站点必须给两个栅格目录，那两个框平时藏着 —— 见 plan-gui2.md §1.6。
      $('urbandirs').hidden = !s.urban;
      state.pickedSite = s;
      setStatus(`已选 ${s.name}${s.met_file ? '' : '（没有强迫场，跑不了）'}`);
      // 选完站点就该去第 2 步。让人自己找「下一步在哪」，正是原来那版的毛病。
      go('case');
    };
    box.appendChild(d);
  }
}
