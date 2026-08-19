//! 算例库扫描、选中、新建。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, joinPath } from './ui.js';
import { renderFields } from './params.js';
import { refreshVars } from './results.js';
import { refreshPresets } from './presets.js';
import { renderSteps, setStatus } from './shell.js';
import { updateCaseBatchButtons } from './batch.js';
import { currentKernel, kernelIsUrban } from './kernel.js';

$('rescan').onclick = async () => {
  try {
    state.cases = await invoke('list_cases', { root: $('root').value.trim() });
    renderCases();
    renderSteps();
  } catch (e) { $('status').textContent = String(e); }
};

/** 算例列表渲染进一个容器。
 *
 *  **两页各一个。** 第 3 步问「建出来没有」，第 5 步问「跑哪些」，
 *  两处都要看得见同一份列表，而一个 DOM 元素进不了两页。
 *  勾选状态共享 `state.pickedCases`，两边贯通 —— 在第 3 步勾中的那几个，
 *  翻到第 5 步仍然是勾着的。 */
function renderCasesInto(box) {
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
    const lab = document.createElement('label');
    lab.className = 'tickbox';
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.checked = state.pickedCases.has(c.dir);
    cb.onchange = () => {
      if (cb.checked) state.pickedCases.add(c.dir); else state.pickedCases.delete(c.dir);
      // 两个容器都要跟着重画 —— 勾选状态是共享的，只重画一个的话
      // 另一页会停在旧的勾选态上，而那是看不出异常的。
      renderCases();
    };
    lab.appendChild(cb);
    lab.onclick = e => e.stopPropagation();   // 勾选不等于「切到这一个算例」
    d.appendChild(lab);
    const s = document.createElement('small');
    // 本次批次里的状态优先 —— 「已跑过」说的是历史，「运行中」说的是现在。
    s.textContent = state.runState[c.dir] ?? (c.has_history ? '已跑过' : '未跑');
    d.appendChild(document.createTextNode(c.name));
    d.appendChild(s);
    d.onclick = () => selectCase(c);
    box.appendChild(d);
  }
}

/** 把列表画进它该在的每一个容器。调用点不必知道有几个。 */
export function renderCases() {
  for (const id of ['cases-built', 'cases-run']) {
    const box = $(id);
    if (box) renderCasesInto(box);
  }
  updateCaseBatchButtons();
}

async function selectCase(c) {
  state.selected = c;
  // 从算例列表点进来的是**单个**算例。不重置的话，上一次批量选中的
  // 那 20 个还留在 state.batch 里，改一个字段会连带改掉它们。
  if (!state.batch.includes(c.dir)) state.batch = [c.dir];
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
 *  （名字取站点代号）、要么本来就能后改（时间窗口在运行页）。
 *  一个只有一种填法的表单，就不该是一道关。
 *
 *  建在**第一次真要用它**的时候，不在点中的时候：点站点常常只是看看，
 *  而每建一个都要读站点文件与强迫场、写出补齐的 site.nc。 */
export function pickSite(s) {
  // **只高亮，不动文件。** 原来这里顺手就把算例建了，于是误点一行
  // 就是一次读站点文件、读强迫场、写 site.nc —— 而勾选框就在旁边，
  // 误点是常事而不是意外。建算例挪到「确定」，那是一个人明确按下的地方。
  state.pickedSite = s;
  renderSites();
  setStatus(`已选 ${s.name}${s.met_file ? '' : '（没有强迫场，跑不了）'}`);
}

/** 「确定」：为选中的站点建算例，然后往下走。
 *
 *  勾了就用勾中的那些，一个没勾就用高亮的那一个 —— 与批量按钮同一条规则，
 *  而按钮上的字说出它会做什么，不留隐藏模式。 */
async function confirmSelection() {
  const checked = state.sites.filter(s => state.picked.has(s.site_file));
  const target = checked.length ? checked : (state.pickedSite ? [state.pickedSite] : []);
  if (!target.length) { setStatus('先点一个站点，或勾选几个'); return; }
  const btn = $('makecase').querySelector('button');
  if (btn) btn.disabled = true;
  try {
    const made = await ensureCases(target);
    if (!made.length) return;
    // 整批都交给参数页。代表算例是第一个，但改动落到每一个上。
    state.batch = [...new Set(made.map(c => c.dir))];
    // **走 selectCase，不要只设 state.selected。** 那里还要把 case.nml 读进来、
    // 查出 CoLM 不认识的字段、刷新参数表与预设 —— 只设一个字段的话，
    // 参数页会是空的，而空页面不会报错，只是什么都没有。实测踩过。
    await selectCase(made[0]);
  } finally {
    // 整表重画，不只是重画按钮 —— 站点行上有「已建算例」标记，
    // 而刚刚正是建算例。只刷按钮的话，同一张卡片里下半截列出了新算例、
    // 上半截还说这些站点没建过。
    renderSites();
  }
}

/** 站点卡片里的「建算例」按钮。**字要说出它会做什么** ——
 *  它要读站点文件与强迫场并写出补齐后的 site.nc，那是真动文件。
 *
 *  **它不是页面出口。** 出口是底部通用的「下一步：参数 →」，
 *  由 shell.js 的 renderNextButtons 注入。两个长得差不多、行为不同的
 *  按钮不能摆在一起。 */
export function renderMakeCase() {
  const foot = $('makecase');
  if (!foot) return;
  foot.textContent = '';
  const n = state.picked.size;
  const one = state.pickedSite;
  const b = document.createElement('button');
  b.className = 'btn-next';
  if (n) b.textContent = `建算例：选中的 ${n} 个站点`;
  else if (one) b.textContent = `建算例：${one.name}`;
  else b.textContent = '先点一个站点，或勾选几个';
  b.disabled = !n && !one;
  b.onclick = confirmSelection;
  foot.appendChild(b);
  const info = $('pickinfo');
  if (info) info.textContent = n ? `已勾 ${n} 个` : (one ? `已选 ${one.name}` : '还没选');
}

/** 给每个站点定一个**唯一**的算例名。
 *
 *  站点名不唯一：`AU-Preston` 在 PLUMBER2 与 Urban-PLUMBER 里各有一个
 *  （实测扫同一个目录能同时扫出两个）。两者要跑的东西完全不同 ——
 *  一个是通量站，另一个必须走 URBAN 路径、地类强制 13、还要给栅格目录。
 *
 *  **不去重的后果不是显示错，是两个站点共用一个算例目录**：后建的那个
 *  被 `ensureCase` 按名字认成"已经有了"，于是第二个站点根本没被建，
 *  而界面上两行都显示成就绪。 */
export function assignCaseNames(sites) {
  const seen = new Map();
  for (const s of sites) {
    const n = (seen.get(s.name) ?? 0) + 1;
    seen.set(s.name, n);
    // 第一个用原名（绝大多数站点只有一个，路径不该无端变长）；
    // 重名的那个带上能说明它是什么的后缀，而不是一个 -2。
    s.caseName = n === 1 ? s.name : (s.urban ? `${s.name}-urban` : `${s.name}-${n}`);
  }
  return sites;
}

/** 这个站点的算例，没有就建一个。返回 `null` 表示建不了（并已说明原因）。 */
export async function ensureCase(s) {
  const root = $('root').value.trim();
  if (!root) { setStatus('先指定算例放哪（第 2 步「算例放哪」那张卡片）'); return null; }
  const cname = s.caseName ?? s.name;
  const have = state.cases.find(c => c.name === cname);
  if (have) return have;
  if (!s.met_file) { setStatus(`${s.name} 没有强迫场文件，建不了算例`); return null; }
  setStatus(`正在为 ${s.name} 建算例…`);
  try {
    await invoke('new_case', {
      site: s.site_file, out: joinPath(root, cname), name: cname,
      // 不传时间窗口：`colm-cli new` 用强迫场的完整范围，
      // 而缩短窗口是运行页「时间与预热」里的事。
      start: null, end: null,
      rawdata: $('rawdata').value.trim() || null,
      runtime: $('runtime').value.trim() || null,
    });
    state.cases = await invoke('list_cases', { root });
    renderCases();
    renderSteps();
    setStatus(`已为 ${s.name} 建好算例`);
    return state.cases.find(c => c.name === cname) ?? null;
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
    state.sites = assignCaseNames(r.sites);
    renderSites(r);
    renderSteps();
  } catch (e) { status(e); }
  finally { $('scan').disabled = false; }
};

export function renderSites(r = {}) {
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
  renderMakeCase();
  const urban = state.sites.filter(s => s.urban).length;
  const urbanKernel = kernelIsUrban();
  // 把「有多少不能跑 / 不能评估」直接说出来。让人自己数一列图标，
  // 等于把一次可以立刻回答的问题推给用户。
  //
  // 内核也报在这里：它是第 2 步定的，而到了这一页它决定哪些行能用 ——
  // 让人回上一步去看自己选了什么，等于把上下文丢了。
  const mismatch = state.sites.filter(x => x.urban !== urbanKernel).length;
  const kname = currentKernel()?.preset;
  $('sitesummary').textContent =
    `${state.sites.length} 个站点` +
    (urban ? ` · ${urban} 个城市` : '') +
    (noObs ? ` · ${noObs} 个无观测` : '') +
    (bad ? ` · ${bad} 个读不了` : '') +
    (kname ? ` · 当前内核 ${kname}` : '') +
    (mismatch ? `，其中 ${mismatch} 个跑不了` : '');

  for (const s of state.sites) {
    const d = document.createElement('div');
    d.className = 'case';
    d.setAttribute('aria-selected', String(state.pickedSite?.name === s.name));
    // 多选是批量的入口。**流水线作用于「一组」，选 1 个只是 N=1** ——
    // 批量另开一套界面的话，就有两条流水线要各自维护，而它们迟早会分叉。
    //
    // 勾选框套一个 <label>：**点击区从十几像素变成整个左格**。
    // 一个小靶子紧挨着另一个可点区域，误点是必然而不是意外。
    const lab = document.createElement('label');
    lab.className = 'tickbox';
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.checked = state.picked.has(s.site_file);
    cb.onchange = () => {
      if (cb.checked) state.picked.add(s.site_file); else state.picked.delete(s.site_file);
      renderMakeCase();
      renderSteps();   // 勾选改变了「在配几个」，左栏要立刻跟上
    };
    lab.appendChild(cb);
    lab.onclick = e => e.stopPropagation();   // 勾选不等于「选中这一个」
    d.appendChild(lab);
    d.appendChild(document.createTextNode(s.name));
    const small = document.createElement('small');
    const tags = [];
    // 算例状态排在最前：它是**这一行现在处在流水线哪一段**，
    // 比经纬度重要得多。原来这个信息藏在另一个列表里。
    const c = state.cases.find(x => x.name === s.name);
    if (c) tags.push(c.has_history ? '已跑过' : '已建算例');
    if (s.urban) tags.push('城市');
    // **内核决定这个站点跑不跑得了。** 城市站要 URBANON 编进去的那一套，
    // 非城市站用 urban 内核跑出来的东西也不对。标出来而不是藏起来 ——
    // 过滤掉会让人以为「扫出来就这么多」。
    if (s.urban && !urbanKernel) tags.push('要 urban 内核');
    if (!s.urban && urbanKernel) tags.push('要非 urban 内核');
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
  // 左栏的「已选站点」现在会说出**在配几个**，而勾选正是改变那个数的动作。
  // 不在这里刷一次的话，勾了 20 个左栏还写着上一次的数。
  renderSteps();
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



$('pick-all').onclick = () => { for (const s of state.sites) state.picked.add(s.site_file); renderSites(); };
$('pick-none').onclick = () => { state.picked.clear(); renderSites(); };


/** 用自带的示例站点。
 *
 *  **一个刚装好程序的人手上没有任何数据**，而 PLUMBER2 要注册、几十 GB ——
 *  在拿到数据之前他连"这程序能不能用"都判断不了。这个按钮把自带的那一个
 *  站点放到可写位置，填好两个路径，直接扫出来。
 *
 *  安装目录本身是只读的（macOS 的 .app、Windows 的 Program Files），
 *  所以要先复制出来 —— 否则建算例时会拿到一个权限错误，
 *  而错误信息里看不出问题出在"那是安装目录"。 */
$('use-example').onclick = async () => {
  $('use-example').disabled = true;
  try {
    const e = await invoke('install_example');
    $('sitedir').value = e.sitedir;
    $('root').value = e.root;
    for (const id of ['sitedir', 'root']) $(id).dispatchEvent(new Event('change'));
    setStatus(e.already ? '示例数据已经在了' : '示例数据已放好');
    $('scan').click();
  } catch (err) { setStatus(err); }
  finally { $('use-example').disabled = false; }
};
