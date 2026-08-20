//! 前处理页：强迫场探测与转换（`docs/design-prep.md` §2.1 阶段 A）。
//!
//! 四张卡片，顺序不可换：① 选文件（静态，见 index.html）→ ② 槽位映射 →
//! ③ 时间轴与高度 → ④ 转换。后一张依赖前一张探出来的结果，所以②③④
//! 全由同一份内部状态驱动，改任何一处都整体重画 —— 与 `timing.js` 同一个
//! 套路：一张卡片自己渲染自己。
//!
//! **这份状态不进 `state.js`。** 只有这一页用得到它，塞进共享 state
//! 只是让别的模块多一条不需要的依赖。`forcing.js` 只 import `ipc.js`
//! 与 `ui.js` 两个叶子模块，不读、不写 `state`，天然不会跟别的模块成环。
//!
//! **确认映射是必经一步**（`design-prep.md` §2.1）：变量名猜错的后果是
//! 「跑得完、结果全错」——模型照样跑完，曲线照样是曲线，界面上什么都看
//! 不出来。④ 的转换按钮默认禁用，只有在②上点过「这些映射我看过了」才
//! 会启用；改了②的任何一行（变量、源单位、合并变量）都会把确认打回去，
//! 否则「我看过了」说的是改之前那一版。

import { invoke } from './ipc.js';
import { $, status, joinPath, baseName } from './ui.js';

/** 探测结果。没探过是 `null`。 */
let probe = null;
/** 源文件路径，探测成功那一刻记下来 —— 转换时还要用它算产物文件名。 */
let srcPath = '';
/** 每个槽位当前选的变量名，下标对应 `probe.slots`。空字符串是「（不用）」。 */
let picks = [];
/** 每个槽位的源单位（可能是探出来的，也可能是用户补的）。 */
let unitsInput = [];
/** 每个槽位要合并进去的额外变量（目前只有降水槽用得到，至多一个）。 */
let extras = [];
/** 三个观测高度，`null` 表示源文件没有、也还没有人填。 */
let heights = { v: null, t: null, q: null };
/** 用户是否在当前这版映射上点过「确认」。改了任何一行映射就清掉。 */
let confirmed = false;
/** 产物目录，卡片重画时要保留用户已经打的字。 */
let dstDir = '';
/** 上一次转换成功的产物路径。`null` 表示还没转换过，或者刚探了新文件。 */
let lastResult = null;

const MEANING_ZH = {
  'air temperature': '气温',
  'specific humidity': '比湿',
  'surface pressure': '气压',
  precipitation: '降水',
  'eastward wind': '东风',
  'northward or scalar wind': '北风 / 标量风',
  'downward shortwave': '短波辐射',
  'downward longwave': '长波辐射',
};
const zh = m => MEANING_ZH[m] ?? m;

$('fprobe').onclick = async () => {
  const path = $('fsrc').value.trim();
  if (!path) { status('先选一份强迫场文件'); return; }
  $('fprobe').disabled = true;
  try {
    probe = await invoke('probe_forcing', { path });
    srcPath = path;
    picks = probe.slots.map(s => s.guessed ?? '');
    unitsInput = probe.slots.map(s => s.units ?? '');
    extras = probe.slots.map(() => []);
    heights = { v: probe.height_v, t: probe.height_t, q: probe.height_q };
    confirmed = false;
    lastResult = null;
    renderCards();
    status(`已探测 ${baseName(path)}：${probe.variables.length} 个变量，${probe.steps} 步`);
  } catch (e) { status(e); }
  finally { $('fprobe').disabled = false; }
};

function renderCards() {
  const box = $('forcing-cards');
  if (!box) return;
  box.textContent = '';
  if (!probe) return;
  box.appendChild(slotsCard());
  box.appendChild(timingCard());
  box.appendChild(convertCard());
}

// ------------------------------------------------------------ ② 槽位映射

function missingRequiredSlots() {
  return probe.slots
    .map((s, i) => ({ s, i }))
    .filter(({ s, i }) => !s.optional && !picks[i]);
}

function missingUnitSlots() {
  return probe.slots
    .map((s, i) => ({ s, i }))
    .filter(({ i }) => picks[i] && !unitsInput[i].trim());
}

function slotsCard() {
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = `
    <h3>槽位映射</h3>
    <div class="ch">CoLM 认死了这八个槽位（<code>MOD_UserSpecifiedForcing.F90</code>：
      1 气温 2 比湿 3 气压 4 降水 5 东风 6 北风/标量风 7 短波辐射 8 长波辐射）。
      自动猜的对不对、单位要不要换，都要你确认一遍 ——
      <b>变量名猜错的后果是「跑得完、结果全错」</b>，模型照样跑完，曲线照样是曲线，
      界面上什么都看不出来。</div>
    <table>
      <tr><th>槽位</th><th>含义</th><th>变量</th><th>源单位</th><th>目标单位</th></tr>
    </table>`;
  const table = card.querySelector('table');
  for (let i = 0; i < probe.slots.length; i++) table.appendChild(slotRow(i));

  const bar = document.createElement('div');
  bar.className = 'pill-row';
  bar.style.marginTop = '12px';
  const confirmBtn = document.createElement('button');
  confirmBtn.className = 'btn-ghost';
  confirmBtn.textContent = '这些映射我看过了';
  confirmBtn.onclick = () => {
    confirmed = true;
    status('映射已确认，下面「转换」可以按了');
    renderCards();
  };
  bar.appendChild(confirmBtn);
  const st = document.createElement('span');
  st.className = 'mini ' + (confirmed ? 'muted' : 'warn');
  st.textContent = confirmed
    ? '已确认 —— 再改任何一行都会打回未确认'
    : '还没确认，下面「转换」按钮不会亮';
  bar.appendChild(st);
  card.appendChild(bar);

  const missing = missingRequiredSlots();
  if (missing.length) {
    const p = document.createElement('p');
    p.className = 'fail mini';
    p.style.marginTop = '8px';
    p.textContent = '必需槽位还没选变量：' +
      missing.map(({ s }) => `第 ${s.index} 槽（${zh(s.meaning)}）`).join('、');
    card.appendChild(p);
  }
  return card;
}

/** 画第 `i` 个槽位那一行。**恒画 8 行**——不是每个数据集都用得上第 5 槽
 *  （PLUMBER2 只有标量 `Wind`），但那是「这一槽空着」，不是「这一槽不存在」。
 *  写死成 7 行会在 Urban-PLUMBER 的 `Wind_E` 上漏掉一整个变量。 */
function slotRow(i) {
  const s = probe.slots[i];
  const tr = document.createElement('tr');

  const tdIdx = document.createElement('td');
  tdIdx.textContent = String(s.index);
  tr.appendChild(tdIdx);

  const tdMeaning = document.createElement('td');
  tdMeaning.textContent = zh(s.meaning);
  tr.appendChild(tdMeaning);

  const tdVar = document.createElement('td');
  // 没猜到、又是必需槽位：这一行需要人立刻处理。「猜到了」不上色 ——
  // 默认色就是「正常」，只有这两种状态色（warn/fail）才值得标出来。
  if (!s.guessed && !s.optional) tdVar.className = 'fail';
  const sel = document.createElement('select');
  sel.className = 'select';
  const opt0 = document.createElement('option');
  opt0.value = '';
  opt0.textContent = '（不用）';
  sel.appendChild(opt0);
  for (const v of probe.variables) {
    const o = document.createElement('option');
    o.value = v;
    o.textContent = v;
    sel.appendChild(o);
  }
  sel.value = picks[i];
  sel.onchange = () => {
    picks[i] = sel.value;
    // 探测阶段量到的单位只对**猜出来的那个变量**有效。换成别的变量之后
    // 留着旧单位会让人以为它还对 —— 只有选回原来那个猜测时才恢复，
    // 其余一律清空，逼用户自己填。
    unitsInput[i] = sel.value && sel.value === s.guessed ? (s.units ?? '') : '';
    if (s.index !== 4) extras[i] = []; // 只有降水槽用得到 also_add
    confirmed = false;
    renderCards();
  };
  tdVar.appendChild(sel);
  if (s.optional) {
    const note = document.createElement('div');
    note.className = 'muted mini';
    note.textContent = '这一槽可以空着 —— 标量风的数据集没有它，模型照样能跑。';
    tdVar.appendChild(note);
  }
  // 降水槽（第 4 槽）能再加一个同单位的变量合并进去：Urban-PLUMBER 把
  // 降水拆成 Rainf + Snowf，不合并就丢掉全部降雪（实测 FI-Kumpula 少 24.7%）。
  if (s.index === 4 && picks[i]) {
    const note = document.createElement('div');
    note.className = 'muted mini';
    note.style.marginTop = '4px';
    note.textContent = '再加一个同单位的变量合并进这一槽（降水常拆成雨 + 雪两个变量）：';
    tdVar.appendChild(note);
    const extraSel = document.createElement('select');
    extraSel.className = 'select';
    const oNone = document.createElement('option');
    oNone.value = '';
    oNone.textContent = '（不加）';
    extraSel.appendChild(oNone);
    for (const v of probe.variables) {
      if (v === picks[i]) continue;
      const o = document.createElement('option');
      o.value = v;
      o.textContent = v;
      extraSel.appendChild(o);
    }
    extraSel.value = extras[i][0] ?? '';
    extraSel.onchange = () => {
      extras[i] = extraSel.value ? [extraSel.value] : [];
      confirmed = false;
      renderCards();
    };
    tdVar.appendChild(extraSel);
  }
  tr.appendChild(tdVar);

  const tdUnits = document.createElement('td');
  const uInp = document.createElement('input');
  uInp.className = 'input';
  uInp.style.width = '7em';
  uInp.value = unitsInput[i];
  uInp.disabled = !picks[i];
  uInp.placeholder = picks[i] ? '必填' : '—';
  const unitsMissing = !!picks[i] && !unitsInput[i].trim();
  const unitsDiffer = !!picks[i] && !!unitsInput[i].trim() && unitsInput[i].trim() !== s.wants;
  if (unitsMissing) tdUnits.className = 'fail';
  else if (unitsDiffer) tdUnits.className = 'warn';
  uInp.onchange = () => {
    unitsInput[i] = uInp.value.trim();
    confirmed = false;
    renderCards();
  };
  tdUnits.appendChild(uInp);
  tr.appendChild(tdUnits);

  const tdWant = document.createElement('td');
  tdWant.textContent = s.wants;
  tr.appendChild(tdWant);

  return tr;
}

// -------------------------------------------------------- ③ 时间轴与高度

function timingCard() {
  const card = document.createElement('div');
  card.className = 'card';
  const uniformWarn = !probe.step_uniform;
  card.innerHTML = `
    <h3>时间轴与观测高度</h3>
    <div class="ch">步长与观测高度会写进产物；模拟用哪一段时间范围仍以强迫场
      覆盖范围为准，在第 4 步「时间与预热」里看。</div>
    <table>
      <tr><th>步长</th><td>${probe.step_seconds} 秒</td></tr>
      <tr><th>步数</th><td>${probe.steps}</td></tr>
      <tr><th>是否等间隔</th><td class="${uniformWarn ? 'warn' : ''}">${
        uniformWarn ? '不是 —— 重采样不在这一阶段，请先自己处理' : '是'
      }</td></tr>
    </table>`;
  const row = document.createElement('div');
  row.className = 'row';
  row.style.marginTop = '12px';
  for (const [key, label] of [
    ['v', '观测高度 V（风速，米）'],
    ['t', '观测高度 T（气温，米）'],
    ['q', '观测高度 Q（湿度，米）'],
  ]) {
    const f = document.createElement('div');
    f.className = 'field';
    const lab = document.createElement('label');
    lab.textContent = label;
    f.appendChild(lab);
    const inp = document.createElement('input');
    inp.className = 'input';
    inp.type = 'number';
    inp.step = 'any';
    inp.value = heights[key] ?? '';
    inp.onchange = () => {
      const n = parseFloat(inp.value);
      heights[key] = Number.isFinite(n) ? n : null;
      renderCards();
    };
    f.appendChild(inp);
    row.appendChild(f);
  }
  card.appendChild(row);
  if (heights.v == null || heights.t == null || heights.q == null) {
    const note = document.createElement('div');
    note.className = 'expert-note';
    note.innerHTML =
      'CoLM 要观测高度填 <code>DEF_forcing%HEIGHT_V/T/Q</code>。这份文件里没有，' +
      '不填的话模型会拿到 <b>NaN</b> 然后直接崩，而报出来的错看不出是这里的问题。';
    card.appendChild(note);
  }
  return card;
}

// --------------------------------------------------------------- ④ 转换

function convertCard() {
  const card = document.createElement('div');
  card.className = 'card';
  const reasons = [];
  if (!confirmed) reasons.push('先在上面「槽位映射」卡片点一次「这些映射我看过了」');
  const missingReq = missingRequiredSlots();
  if (missingReq.length) {
    reasons.push('必需槽位还没选变量：' + missingReq.map(({ s }) => `第 ${s.index} 槽`).join('、'));
  }
  const missingU = missingUnitSlots();
  if (missingU.length) {
    reasons.push('选了变量但没填源单位：' + missingU.map(({ s }) => `第 ${s.index} 槽`).join('、'));
  }

  card.innerHTML = `
    <h3>转换</h3>
    <div class="ch">按上面确认过的映射写出一份 CoLM 认的标准文件。
      <b>产物目录不能与源文件所在目录相同</b> —— 原始数据要原样留着，
      选了同一个目录后端会直接拒绝。</div>
    <div class="browse"><input class="input" id="fdst" placeholder="…/converted"></div>
    <p class="muted mini" id="fdst-note"></p>
    <div class="pill-row" style="margin-top:12px">
      <button class="btn-ghost" id="fconvert">转换</button>
    </div>
    <p class="mini" id="fconvert-why"></p>
    <div id="fconvert-result"></div>`;

  const dstInp = card.querySelector('#fdst');
  dstInp.value = dstDir;
  dstInp.onchange = () => { dstDir = dstInp.value; };
  card.querySelector('#fdst-note').textContent =
    `产物文件名沿用源文件名（${baseName(srcPath)}），只是换了目录。`;

  const btn = card.querySelector('#fconvert');
  btn.disabled = reasons.length > 0;
  btn.onclick = doConvert;
  const why = card.querySelector('#fconvert-why');
  why.className = (reasons.length ? 'fail' : 'muted') + ' mini';
  why.textContent = reasons.length ? reasons.join('；') : '就绪，可以转换。';

  const resultBox = card.querySelector('#fconvert-result');
  if (lastResult) {
    const p1 = document.createElement('p');
    p1.className = 'mini';
    p1.style.marginTop = '10px';
    const code = document.createElement('code');
    code.textContent = lastResult;
    p1.append('已转换：', code);
    const p2 = document.createElement('p');
    p2.className = 'muted mini';
    p2.textContent =
      '下一步：回到「站点」那一步，把 Sitedata 目录指到产物所在的位置（或它的上级目录）' +
      '重新扫描 —— 这份产物已经是标准约定的强迫场，扫描认得出来。';
    resultBox.appendChild(p1);
    resultBox.appendChild(p2);
  }
  return card;
}

async function doConvert() {
  const src = srcPath;
  const dir = $('fdst').value.trim();
  if (!dir) { status('先填产物放哪个目录'); return; }
  const dst = joinPath(dir, baseName(src));
  const btn = $('fconvert');
  if (btn) btn.disabled = true;
  try {
    const slots = probe.slots
      .map((s, i) => ({ s, i }))
      .filter(({ i }) => picks[i])
      .map(({ s, i }) => ({
        index: s.index,
        name: picks[i],
        units: unitsInput[i].trim(),
        also_add: extras[i] ?? [],
      }));
    const heightsReady = heights.v != null && heights.t != null && heights.q != null;
    const heightsArg = heightsReady ? [heights.v, heights.t, heights.q] : null;
    lastResult = await invoke('convert_forcing', { src, dst, slots, heights: heightsArg });
    status('转换完成：' + lastResult);
  } catch (e) {
    lastResult = null;
    status(e);
  } finally {
    renderCards();
  }
}
