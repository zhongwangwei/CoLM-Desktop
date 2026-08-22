//! 算例库扫描、选中、新建。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, joinPath, forcingDirectoryForSiteDirectory } from './ui.js';
import { renderFields } from './params.js';
import { refreshVars } from './results.js';
import { renderSteps, setStatus } from './shell.js';
import {
  batchTarget, currentCases, freshCaseName, updateCaseBatchButtons,
} from './batch.js';
import { urbanEnabled } from './kernel.js';
import { wizardFields } from './domain.js';

/** 站点身份必须与向导的 URBAN 选择一致，示例与用户目录走同一条规则。 */
export function sitesForWizard(sites = state.sites) {
  return sites.filter(s => s.urban === urbanEnabled());
}

/** 算例根目录含空格，当场标出来。
 *
 *  CoLM 有 55 处不加引号的 `CALL system('mkdir -p ' // trim(dir))`——路径一有空格
 *  就被拆成两截，建出一棵位置不对的影子目录树，而报出来的是 netCDF 的
 *  `Permission denied`，指向完全错误的方向。`colm-cli new` 会直接拒绝这种
 *  路径（见 crates/colm-cli），但**那是建算例那一刻才报**；这里要在填路径
 *  的当场就说清楚，不能等按下「建算例」才知道，更不能等跑到一半。
 *
 *  判据与 `colm-cli new` 那条一致：路径里有没有空格。 */
export function checkRootSpace() {
  const warn = $('rootspace');
  if (warn) warn.hidden = !$('root').value.includes(' ');
}
$('root').addEventListener('input', checkRootSpace);
$('root').addEventListener('change', checkRootSpace);

$('rescan').onclick = async () => {
  try {
    state.cases = await invoke('list_cases', { root: $('root').value.trim() });
    renderCases();
    renderSteps();
  } catch (e) { $('status').textContent = String(e); }
};

/** 算例列表渲染进一个容器。
 *
 *  **两页各一个。** 基本设定看本次创建的算例，运行页看本次运行批次；
 *  一个 DOM 元素进不了两页。
 *  勾选状态共享 `state.pickedCases`，两边贯通。 */
function renderCasesInto(box) {
  box.textContent = '';
  const cases = box.id === 'cases-run' ? batchTarget() : currentCases();
  if (!cases.length) {
    box.innerHTML = box.id === 'cases-run'
      ? '<p class="muted" style="font-size:11px">本次还没有要运行的算例；先在前面选站点并建算例。</p>'
      : '<p class="muted" style="font-size:11px">本次还没有创建算例；root 里的旧算例不会显示。</p>';
    return;
  }
  for (const c of cases) {
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
      // **只重画另一个容器，自己这个原地不动。** 重建自己所在的容器会把
      // 焦点打到 body 上 —— 键盘操作每勾一个就要重新 Tab 回去。
      // 勾选状态两页共享，所以另一页必须跟着变。
      for (const id of ['cases-built', 'cases-run']) {
        if (id === box.id) continue;
        const other = $(id);
        if (other) renderCasesInto(other);
      }
      updateCaseBatchButtons();
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
  const checked = sitesForWizard().filter(s => state.picked.has(s.site_file));
  const target = checked.length ? checked : (state.pickedSite ? [state.pickedSite] : []);
  if (!target.length) { setStatus('先点一个站点，或勾选几个'); return; }
  const btn = $('makecase').querySelector('button');
  if (btn) btn.disabled = true;
  try {
    const made = await ensureCases(target);
    if (!made.length) return;
    // 整批都交给参数页。代表算例是第一个，但改动落到每一个上。
    state.batch = [...new Set(made.map(c => c.dir))];
    // **刚建的这批就是马上要跑的那批。** 不灌 pickedCases 的话，
    // 过程参数说「改动会写进 2 个算例」而运行页说「运行全部 4 个」会打架。
    // 先清掉上次批次；否则旧算例仍会被误认为本次目标。
    state.pickedCases.clear();
    for (const c of made) state.pickedCases.add(c.dir);
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

let scanTimer = null;

/** 目录选择器连续更新站点、强迫场和算例路径时，只在本轮末尾扫描一次。 */
function scheduleSiteScan() {
  if (!$('sitedir').value.trim()) return;
  if (scanTimer !== null) clearTimeout(scanTimer);
  scanTimer = setTimeout(() => {
    scanTimer = null;
    $('scan').click();
  }, 0);
}

// 强迫场目录与站点目录是一组。换了 Sitedata 还保留上一套 forcingdir，
// 显式旧路径会覆盖 CLI 本来能正确找到的 ../Forcing，整表因此误报“无强迫场”。
$('sitedir').addEventListener('change', () => {
  const forcing = $('forcingdir');
  const expected = forcingDirectoryForSiteDirectory($('sitedir').value);
  if (forcing.value !== expected) {
    forcing.value = expected;
    forcing.dispatchEvent(new Event('change'));
  }
  scheduleSiteScan();
});

// 已经列出站点后再更换强迫场目录，旧的“无强迫场”标签不能留到手动重扫。
$('forcingdir').addEventListener('change', scheduleSiteScan);

/** 这个站点的算例，没有就建一个。返回**算例名**，建不了返回 `null`。
 *
 *  **不自己扫描。** 每建一个就 `list_cases` 一次的话，90 个站点要扫 90 次，
 *  每次都遍历算例根目录并读每份 case.nml，还要重画两个容器（行数
 *  1…90 递增，约 8000 个行节点）。扫描与渲染交给 `ensureCases` 收尾做一次，
 *  中途的进度靠 `setStatus` 报。
 *
 *  返回名字而不是 `Case` 对象，是因为对象要等收尾那次扫描才拿得到。 */
async function ensureCase(s) {
  const root = $('root').value.trim();
  if (!root) { setStatus('先在“基本设定 / 文件与目录”指定算例放哪'); return null; }
  const madeDir = state.createdBySite.get(s.site_file);
  const made = state.cases.find(c => c.dir === madeDir);
  if (made) return made.name;
  const cname = freshCaseName(s.caseName ?? s.name);
  if (!s.met_file) { setStatus(`${s.name} 没有强迫场文件，建不了算例`); return null; }
  setStatus(`正在为 ${s.name} 建算例…`);
  try {
    const out = joinPath(root, cname);
    await invoke('new_case', {
      site: s.site_file, out, name: cname,
      // 不传时间窗口：`colm-cli new` 用强迫场的完整范围，
      // 时间边界与预热在基本设定的专门分页显示。
      start: null, end: null,
      rawdata: $('rawdata').value.trim() || null,
      runtime: $('runtime').value.trim() || null,
      // 扫描已经按“强迫场目录 + 站点名”匹配到具体文件；把确切路径传下去，
      // 避免 new 再按兄弟目录约定推回另一份旧文件。
      met: s.met_file,
      fields: wizardFields(),
    });
    state.createdCases.add(out);
    state.createdBySite.set(s.site_file, out);
    s.caseName = cname;
    setStatus(`已为 ${s.name} 建好算例`);
    return cname;
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
    const r = await invoke('scan_sites', {
      dir, forcingDir: $('forcingdir').value.trim() || null, quick: true,
    });
    // 算例目录也别问 —— 默认放在站点数据旁边。**显示出来且可改**，
    // 不是偷偷决定：产物落在哪儿是用户该看得见的事。
    if (!$('root').value.trim()) {
      const parent = dir.replace(/[\\/][^\\/]*$/, '');
      $('root').value = (parent || dir) + '/colm-cases';
      $('root').dispatchEvent(new Event('change'));
    }
    // **换了目录就清勾选。** 不清的话，#pickinfo 还写着上一批的数，
    // 而按钮上的字与按下去的行为直接对着干（「建算例：选中的 90 个站点」
    // 按下去落一句「先点一个站点」）。
    //
    // 更要紧的是 pickedSite：它是上一个目录里的**站点对象**，
    // 而 confirmSelection 一个没勾时正是拿它去建 —— 会在新目录里建一个
    // 旧目录站点的算例，界面上看不出异常。
    state.picked.clear();
    state.pickedSite = null;
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
  const sites = sitesForWizard();
  const allowed = new Set(sites.map(s => s.site_file));
  for (const path of state.picked) if (!allowed.has(path)) state.picked.delete(path);
  if (state.pickedSite && !allowed.has(state.pickedSite.site_file)) state.pickedSite = null;
  const bad = sites.filter(s => s.problem).length;
  const noObs = sites.filter(s => !s.obs_file).length;
  renderMakeCase();
  const urbanRun = urbanEnabled();
  $('sitesummary').textContent =
    `${sites.length} 个${urbanRun ? '城市' : '自然'}站点` +
    (noObs ? ` · ${noObs} 个无观测` : '') +
    (bad ? ` · ${bad} 个读不了` : '');

  if (!sites.length && state.sites.length) {
    box.innerHTML = `<p class="muted mini">目录里没有${urbanRun ? '城市' : '自然'}站点。</p>`;
    renderSteps();
    return;
  }

  for (const s of sites) {
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
    // 按 caseName 匹配，不是 name —— 重名站点（AU-Preston 在 PLUMBER2 与
    // Urban-PLUMBER 里各有一个）建出来的目录带后缀，按 name 找会一个都
    // 认不出、或者两行都认成同一个。
    const c = currentCases().find(x => x.name === (s.caseName ?? s.name));
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
  const names = [];
  const failed = [];
  const root = $('root').value.trim();
  // 旧算例不展示，但命名时必须看见它们，否则会覆盖同名目录。
  if (root) {
    try { state.cases = await invoke('list_cases', { root }); }
    catch { state.cases = []; } // root 还没创建时自然没有重名
  }
  for (const [i, s] of sites.entries()) {
    setStatus(`准备算例 ${i + 1}/${sites.length}：${s.name}`);
    const n = await ensureCase(s);
    if (n) names.push(n); else failed.push(s.name);
  }
  // **扫一次，不是每建一个扫一次。** 见 ensureCase 的注释。
  if (root) {
    try {
      state.cases = await invoke('list_cases', { root });
      renderCases();
      renderSteps();
    } catch (e) { setStatus(e); }
  }
  if (failed.length) setStatus(`${names.length}/${sites.length} 个就绪；建不了：${failed.join('、')}`);
  // 对外仍然返回 Case 对象 —— confirmSelection 要拿 made[0] 去 selectCase、
  // 拿 .dir 填 batch 与 pickedCases。
  const byName = new Map(state.cases.map(c => [c.name, c]));
  return names.map(n => byName.get(n)).filter(Boolean);
}



$('pick-all').onclick = () => { for (const s of sitesForWizard()) state.picked.add(s.site_file); renderSites(); };
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
    $('forcingdir').value = e.forcingdir;
    $('root').value = e.root;
    for (const id of ['sitedir', 'forcingdir', 'root']) $(id).dispatchEvent(new Event('change'));
    setStatus(e.already ? '示例数据已经在了' : '示例数据已放好');
    scheduleSiteScan();
  } catch (err) { setStatus(err); }
  finally { $('use-example').disabled = false; }
};
