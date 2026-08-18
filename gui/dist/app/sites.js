//! 算例库扫描、选中、新建。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { renderFields } from './params.js';
import { refreshVars } from './results.js';
import { refreshPresets } from './presets.js';
import { renderSteps, setStatus } from './shell.js';
import { updateCaseBatchButtons } from './batch.js';

$('rescan').onclick = async () => {
  try {
    state.cases = await invoke('list_cases', { root: $('root').value.trim() });
    renderCases();
    renderSteps();
  } catch (e) { $('status').textContent = String(e); }
};

export function renderCases() {
  const box = $('cases');
  box.textContent = '';
  if (!state.cases.length) {
    box.innerHTML = '<p class="muted" style="font-size:11px">这个目录下没有算例</p>';
    return;
  }
  for (const c of state.cases) {
    const d = document.createElement('div');
    d.className = 'case';
    d.setAttribute('aria-selected', String(state.selected?.dir === c.dir));
    // 与站点列表同一套：**这个列表的勾选，驱动这个列表上的批量操作**。
    // 站点那边的勾选驱动批量建，这边的驱动批量运行与批量评估。
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.checked = state.pickedCases.has(c.dir);
    cb.style.cssText = 'width:auto;margin-right:8px';
    cb.onclick = e => {
      e.stopPropagation();   // 勾选不等于「切到这一个算例」
      if (cb.checked) state.pickedCases.add(c.dir); else state.pickedCases.delete(c.dir);
      updateCaseBatchButtons();
    };
    d.appendChild(cb);
    const s = document.createElement('small');
    // 本次批次里的状态优先 —— 「已跑过」说的是历史，「运行中」说的是现在。
    s.textContent = state.runState[c.dir] ?? (c.has_history ? '已跑过' : '未跑');
    d.appendChild(document.createTextNode(c.name));
    d.appendChild(s);
    d.onclick = () => selectCase(c);
    box.appendChild(d);
  }
  updateCaseBatchButtons();
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

/** 选一个站点：**算例按需建**。
 *
 *  原来这里要人先填表、按「新建算例」，而那张表问的东西要么推得出来
 *  （名字取站点代号）、要么本来就能后改（时间窗口在参数页）。
 *  一个只有一种填法的表单，就不该是一道关。
 *
 *  建在**第一次真要用它**的时候，不在点中的时候：点站点常常只是看看，
 *  而每建一个都要读站点文件与强迫场、写出补齐的 site.nc。 */
export async function pickSite(s) {
  state.pickedSite = s;
  $('urbandirs').hidden = !state.sites.some(x => x.urban && state.picked.has(x.name)) && !s.urban;
  renderSites();
  setStatus(`已选 ${s.name}${s.met_file ? '' : '（没有强迫场，跑不了）'}`);
  const c = await ensureCase(s);
  if (c) { state.selected = c; renderSteps(); }
}

/** 这个站点的算例，没有就建一个。返回 `null` 表示建不了（并已说明原因）。 */
export async function ensureCase(s) {
  const root = $('root').value.trim();
  if (!root) { setStatus('先指定算例放哪（第 1 步下面那张卡片）'); return null; }
  const have = state.cases.find(c => c.name === s.name);
  if (have) return have;
  if (!s.met_file) { setStatus(`${s.name} 没有强迫场文件，建不了算例`); return null; }
  setStatus(`正在为 ${s.name} 建算例…`);
  try {
    await invoke('new_case', {
      site: s.site_file, out: root + '/' + s.name, name: s.name,
      // 不传时间窗口：`colm-cli new` 用强迫场的完整范围，
      // 而缩短窗口是参数页「时间」分类里同一组字段的事。
      start: null, end: null,
      rawdata: $('rawdata').value.trim() || null,
      runtime: $('runtime').value.trim() || null,
    });
    state.cases = await invoke('list_cases', { root });
    renderCases();
    renderSteps();
    setStatus(`已为 ${s.name} 建好算例`);
    return state.cases.find(c => c.name === s.name) ?? null;
  } catch (e) { setStatus(`${s.name}：${e}`); return null; }
}


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
    // 算例目录也别问 —— 默认放在站点数据旁边。**显示出来且可改**，
    // 不是偷偷决定：产物落在哪儿是用户该看得见的事。
    if (!$('root').value.trim()) {
      const parent = dir.replace(/[\\/][^\\/]*$/, '');
      $('root').value = (parent || dir) + '/colm-cases';
      $('root').dispatchEvent(new Event('change'));
    }
    state.sites = r.sites;
    renderSites(r);
    renderSteps();
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
    d.setAttribute('aria-selected', String(state.pickedSite?.name === s.name));
    // 多选是批量的入口。**流水线作用于「一组」，选 1 个只是 N=1** ——
    // 批量另开一套界面的话，就有两条流水线要各自维护，而它们迟早会分叉。
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.checked = state.picked.has(s.name);
    cb.onclick = e => {
      e.stopPropagation();   // 勾选不等于「选中这一个」
      if (cb.checked) state.picked.add(s.name); else state.picked.delete(s.name);
        };
    cb.style.cssText = 'width:auto;margin-right:8px';
    d.appendChild(cb);
    d.appendChild(document.createTextNode(s.name));
    const small = document.createElement('small');
    const tags = [];
    // 算例状态排在最前：它是**这一行现在处在流水线哪一段**，
    // 比经纬度重要得多。原来这个信息藏在另一个列表里。
    const c = state.cases.find(x => x.name === s.name);
    if (c) tags.push(c.has_history ? '已跑过' : '已建算例');
    if (s.urban) tags.push('城市');
    if (!s.met_file) tags.push('无强迫场');
    if (!s.obs_file) tags.push('无观测');
    if (s.problem) tags.push('读不了');
    small.textContent = tags.length ? tags.join(' · ') : `${s.lat.toFixed(2)}, ${s.lon.toFixed(2)}`;
    if (s.problem) { d.className += ' warn'; d.title = s.problem; }
    d.appendChild(small);
    // 选中就把路径填进新建向导 —— 那是它唯一的去处，不必再让人复制粘贴。
    d.onclick = () => pickSite(s);
    box.appendChild(d);
  }
}



/** 勾了几个、能不能批量建。 */

/** 批量建算例：流水线最前面那一段，原来只能一个一个点。
 *
 *  **串行，不并发。** 每次 `new_case` 都要读站点文件与强迫场文件并写出
 *  补齐后的 site.nc，瓶颈在磁盘；并发几个只是让它们互相抢。
 *  一个失败不中止整批 —— 90 个里有一个站点文件坏了，其余 89 个仍要建出来。 */
/** 为一批站点确保算例存在。运行前调用 —— **建算例不再是一道要人按的关**。
 *
 *  串行：每次都要读站点文件与强迫场并写出 site.nc，瓶颈在磁盘，
 *  并发只是让它们互相抢。一个失败不中止整批，失败的会点名 ——
 *  一批悄悄少建几个，要到运行时才发现，那时已经隔了一层。 */
export async function ensureCases(sites) {
  const made = [];
  const failed = [];
  for (const [i, s] of sites.entries()) {
    setStatus(`准备算例 ${i + 1}/${sites.length}：${s.name}`);
    const c = await ensureCase(s);
    if (c) made.push(c); else failed.push(s.name);
  }
  if (failed.length) setStatus(`${made.length}/${sites.length} 个就绪；建不了：${failed.join('、')}`);
  return made;
}

