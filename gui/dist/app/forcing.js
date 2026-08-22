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
import { state } from './state.js';
import { $, status, joinPath, baseName, forcingDirectoryForSiteDirectory } from './ui.js';
import { forcingOutputName, missingForcingHeights } from './prep-state.js';
import { scanPreparedSites } from './sites.js';

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
/** 最近一次缺测诊断；映射或修复设置变化就失效。 */
let gapReport = null;
/** 已修复中间文件。没有缺测时保持 null，转换直接读原文件。 */
let repairedSource = null;
let gapSettings = {
  shortGap: 3,
  utcOffset: '',
  latitude: '',
  longitude: '',
  era5: '',
  minOverlap: 24,
};

globalThis.addEventListener?.('colm:prep-site-invalidated', () => {
  lastResult = null;
  if (probe) renderCards();
});

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
    if (!gapSettings.latitude && $('slat')?.value) gapSettings.latitude = $('slat').value;
    if (!gapSettings.longitude && $('slon')?.value) gapSettings.longitude = $('slon').value;
    // 产物目录只在还没填过时用后端建议的那个 —— 用户改过就别再动它，
    // 换一份源文件重新探测不该把他填的路径冲掉。
    if (!dstDir) {
      dstDir = state.prepArtifacts.siteDir
        ? forcingDirectoryForSiteDirectory(state.prepArtifacts.siteDir)
        : (probe.suggest_dst ?? '');
    }
    confirmed = false;
    lastResult = null;
    gapReport = null;
    repairedSource = null;
    Object.assign(state.prepArtifacts, { forcingFile: null, forcingDir: null });
    globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
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
  box.appendChild(gapCard());
  box.appendChild(convertCard());
}

function invalidateGap() {
  gapReport = null;
  repairedSource = null;
  lastResult = null;
}

function selectedSlots() {
  return probe.slots
    .map((s, i) => ({ s, i }))
    .filter(({ i }) => picks[i])
    .map(({ s, i }) => ({
      index: s.index,
      name: picks[i],
      units: unitsInput[i].trim(),
      also_add: extras[i] ?? [],
    }));
}

function optionalNumber(value) {
  if (value === '' || value == null) return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function gapOptions(includeEra5 = true) {
  return {
    short_gap: Math.max(0, Math.trunc(Number(gapSettings.shortGap) || 0)),
    utc_offset: optionalNumber(gapSettings.utcOffset),
    latitude: optionalNumber(gapSettings.latitude),
    longitude: optionalNumber(gapSettings.longitude),
    era5: includeEra5 && gapSettings.era5.trim() ? gapSettings.era5.trim() : null,
    min_overlap: Math.max(1, Math.trunc(Number(gapSettings.minOverlap) || 24)),
  };
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
    invalidateGap();
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
    // **主变量换成了原来那个额外变量时，额外变量要清掉。**
    //
    // 不清的话下面这行赋值会静默失败 —— 上面的循环已经把主变量从选项里
    // 排除了，给 `<select>` 赋一个不存在的选项值，它会变成空串。于是
    // **界面显示「（不加）」，而 `extras[i]` 里还留着那个名字**，
    // 转换时发出去的是 `4=Snowf:kg/m2/s+Snowf`，后端把它加两次，
    // 降水翻倍而模型照样跑完。
    //
    // 校验放在渲染这里而不是 `sel.onchange` 里：这一行守的是
    // 「显示出来的必须等于将要发出去的」，而渲染是唯一能保证覆盖
    // 所有改动路径的地方。
    if (extras[i][0] === picks[i]) extras[i] = [];
    extraSel.value = extras[i][0] ?? '';
    extraSel.onchange = () => {
      extras[i] = extraSel.value ? [extraSel.value] : [];
      confirmed = false;
      invalidateGap();
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
    invalidateGap();
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
      覆盖范围为准，由建例时自动确定，不需要手动填写。</div>
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

// ----------------------------------------------- ④ 缺测诊断、时区与 ERA5-Land

function gapCard() {
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = `
    <h3>缺测诊断与修复</h3>
    <div class="ch">先检查被映射的变量。短缺口按变量类型插值；长缺口在把站点时间换算到 UTC 后，
      读取 ERA5-Land 最近 0.1° 格点，并只用观测重叠期做偏差订正。原始文件不会被覆盖，
      产物逐时记录观测、插值或 ERA5-Land 来源。</div>
    <div class="row" style="margin-top:12px">
      <div class="field"><label>短缺口上限（时间步）</label><input class="input" id="gap-short" type="number" min="0" step="1"></div>
      <div class="field"><label>订正最少重叠样本</label><input class="input" id="gap-overlap" type="number" min="1" step="1"></div>
    </div>
    <div class="row" style="margin-top:10px">
      <div class="field"><label>站点纬度</label><input class="input" id="gap-lat" type="number" min="-90" max="90" step="any" placeholder="优先读取文件"></div>
      <div class="field"><label>站点经度</label><input class="input" id="gap-lon" type="number" min="-180" max="180" step="any" placeholder="优先读取文件"></div>
      <div class="field"><label>人工 UTC 偏移（小时）</label><input class="input" id="gap-offset" type="number" min="-12" max="14" step="0.25" placeholder="自动判断"></div>
    </div>
    <div class="pill-row" style="margin-top:12px"><button class="btn-ghost" id="gap-probe">诊断缺测与时区</button></div>
    <div id="gap-result"></div>`;

  const bind = (selector, key, fallback = '') => {
    const input = card.querySelector(selector);
    input.value = gapSettings[key] ?? fallback;
    input.onchange = () => {
      gapSettings[key] = input.value;
      invalidateGap();
      renderCards();
    };
  };
  bind('#gap-short', 'shortGap', 3);
  bind('#gap-overlap', 'minOverlap', 24);
  bind('#gap-lat', 'latitude');
  bind('#gap-lon', 'longitude');
  bind('#gap-offset', 'utcOffset');
  card.querySelector('#gap-probe').onclick = diagnoseGaps;

  const result = card.querySelector('#gap-result');
  if (gapReport) result.appendChild(gapReportView());
  return card;
}

function gapReportView() {
  const box = document.createElement('div');
  const timezoneLabels = {
    manual_override: '人工覆盖',
    file_metadata: '文件元数据',
    longitude_inferred_offset: '按经度推断（不是行政时区）',
  };
  box.innerHTML = `
    <table style="margin-top:12px">
      <tr><th>UTC 偏移</th><td>UTC${gapReport.timezone_offset_hours >= 0 ? '+' : ''}${gapReport.timezone_offset_hours} · ${timezoneLabels[gapReport.timezone_source] ?? gapReport.timezone_source}</td></tr>
      <tr><th>ERA5-Land 格点定位</th><td>${gapReport.latitude}, ${gapReport.longitude}</td></tr>
      <tr><th>数据范围（UTC 日期）</th><td>${gapReport.start_date} — ${gapReport.end_date}</td></tr>
      <tr><th>缺测总数</th><td class="${gapReport.missing ? 'warn' : ''}">${gapReport.missing}</td></tr>
    </table>
    <table style="margin-top:10px">
      <tr><th>槽位</th><th>变量</th><th>缺测</th><th>短缺口</th><th>需 ERA5</th><th>最长</th><th>已插值</th><th>ERA5-Land</th></tr>
    </table>`;
  const table = box.querySelectorAll('table')[1];
  for (const row of gapReport.variables) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${row.slot}</td><td></td><td>${row.missing}</td><td>${row.short_missing}</td><td class="${row.long_missing ? 'warn' : ''}">${row.long_missing}</td><td>${row.longest_gap}</td><td>${row.interpolated}</td><td>${row.era5_corrected}</td>`;
    tr.children[1].textContent = row.variable;
    table.appendChild(tr);
  }

  if (gapReport.missing === 0) {
    const ready = document.createElement('p');
    ready.className = 'muted mini';
    ready.textContent = '没有缺测，原文件可直接进入标准化转换；时区判定仍会保留在诊断记录中。';
    box.appendChild(ready);
    return box;
  }

  if (gapReport.needs_era5) {
    const field = document.createElement('div');
    field.className = 'field';
    field.style.marginTop = '12px';
    field.innerHTML = `<label>ERA5-Land 缓存目录</label><div class="browse"><input class="input" id="gap-era5" placeholder="…/ERA5-Land"><button class="btn-ghost" id="gap-era5-pick">选择…</button></div>`;
    const input = field.querySelector('#gap-era5');
    if (!gapSettings.era5 && dstDir) gapSettings.era5 = joinPath(dstDir, '.era5land');
    input.value = gapSettings.era5;
    input.onchange = () => { gapSettings.era5 = input.value.trim(); repairedSource = null; renderCards(); };
    field.querySelector('#gap-era5-pick').onclick = async () => {
      try {
        const picked = await invoke('pick_folder', { key: 'gap-era5' });
        if (!picked) return;
        gapSettings.era5 = picked;
        repairedSource = null;
        renderCards();
      } catch (error) { status(error); }
    };
    box.appendChild(field);
    const note = document.createElement('p');
    note.className = 'muted mini';
    note.textContent = '可选择已有 ERA5-Land NetCDF 缓存；也可用本机 CDS API 下载。下载需要先配置 ~/.cdsapirc 并接受 ERA5-Land 数据许可。';
    box.appendChild(note);
  }

  const bar = document.createElement('div');
  bar.className = 'pill-row';
  bar.style.marginTop = '10px';
  if (gapReport.needs_era5) {
    const download = document.createElement('button');
    download.className = 'btn-ghost';
    download.textContent = '下载对应 ERA5-Land 格点';
    download.disabled = !gapSettings.era5.trim();
    download.onclick = downloadEra5;
    bar.appendChild(download);
  }
  const repair = document.createElement('button');
  repair.className = 'btn-next';
  repair.textContent = '生成已修复中间文件';
  repair.disabled = gapReport.needs_era5 && !gapSettings.era5.trim();
  repair.onclick = repairGaps;
  bar.appendChild(repair);
  box.appendChild(bar);
  if (repairedSource) {
    const done = document.createElement('p');
    done.className = 'mini';
    done.append('修复完成：');
    const code = document.createElement('code');
    code.textContent = repairedSource;
    done.appendChild(code);
    box.appendChild(done);
  }
  return box;
}

async function diagnoseGaps() {
  if (!confirmed) { status('先确认槽位映射，再诊断缺测'); return; }
  const missingUnits = missingUnitSlots();
  if (missingUnits.length) { status('先补齐所有已选变量的源单位'); return; }
  try {
    gapReport = await invoke('probe_forcing_gaps', {
      src: srcPath,
      slots: selectedSlots(),
      options: gapOptions(false),
    });
    repairedSource = null;
    status(gapReport.missing ? `发现 ${gapReport.missing} 个缺测值` : '未发现缺测值');
  } catch (error) {
    gapReport = null;
    status(error);
  }
  renderCards();
}

async function downloadEra5() {
  if (!gapReport || !gapSettings.era5.trim()) return;
  try {
    status('正在通过 CDS API 下载 ERA5-Land；长时间序列可能需要几分钟…');
    await invoke('download_era5land', {
      dst: gapSettings.era5.trim(),
      latitude: gapReport.latitude,
      longitude: gapReport.longitude,
      start: gapReport.start_date,
      end: gapReport.end_date,
    });
    status('ERA5-Land 对应格点已缓存，可以生成修复文件');
  } catch (error) { status(error); }
}

async function repairGaps() {
  if (!gapReport || !dstDir.trim()) { status('先诊断缺测并填写产物目录'); return; }
  const stem = state.prepArtifacts.siteStem;
  if (!stem) { status('先生成站点文件'); return; }
  const repaired = joinPath(joinPath(dstDir.trim(), '.colm-gapfill'), `${stem}_Met_repaired.nc`);
  try {
    gapReport = await invoke('repair_forcing', {
      src: srcPath,
      dst: repaired,
      slots: selectedSlots(),
      options: gapOptions(true),
    });
    if (gapReport.unresolved) throw new Error(`仍有 ${gapReport.unresolved} 个缺测值没有解决`);
    repairedSource = repaired;
    status('缺测修复完成，逐时来源已写入 *_gapfill_qc');
  } catch (error) {
    repairedSource = null;
    status(error);
  }
  renderCards();
}

// --------------------------------------------------------------- ⑤ 转换

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
  const missingHeights = missingForcingHeights(heights);
  if (missingHeights.length) {
    reasons.push(`缺少观测高度：${missingHeights.join('、')}`);
  }
  if (!state.prepArtifacts.siteStem) reasons.push('先在“站点数据”子步骤填写站点名并生成站点文件');
  if (!dstDir.trim()) reasons.push('先填写强迫场产物目录');
  if (!gapReport) reasons.push('先完成缺测与时区诊断');
  else if (gapReport.missing > 0 && !repairedSource) reasons.push('先生成已修复中间文件');
  else if (gapReport.unresolved > 0) reasons.push(`仍有 ${gapReport.unresolved} 个缺测值未解决`);

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
  dstInp.onchange = () => { dstDir = dstInp.value.trim(); renderCards(); };
  card.querySelector('#fdst-note').textContent =
    state.prepArtifacts.siteStem
      ? `标准文件名：${forcingOutputName(state.prepArtifacts.siteStem)}，可与站点文件自动配对。`
      : '先生成站点文件，强迫场将沿用同一个站点名。';

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
    p2.textContent = '已自动写入基本设定的强迫场目录，并与刚生成的站点重新配对。';
    resultBox.appendChild(p1);
    resultBox.appendChild(p2);
  }
  return card;
}

async function doConvert() {
  const src = srcPath;
  const dir = $('fdst').value.trim();
  if (!dir) { status('先填产物放哪个目录'); return; }
  const stem = state.prepArtifacts.siteStem;
  if (!stem) { status('先在“站点数据”子步骤生成站点文件'); return; }
  const missingHeights = missingForcingHeights(heights);
  if (missingHeights.length) { status(`先补齐观测高度：${missingHeights.join('、')}`); return; }
  const dst = joinPath(dir, forcingOutputName(stem));
  const btn = $('fconvert');
  if (btn) btn.disabled = true;
  try {
    const slots = selectedSlots();
    const heightsReady = heights.v != null && heights.t != null && heights.q != null;
    const heightsArg = heightsReady ? [heights.v, heights.t, heights.q] : null;
    lastResult = await invoke('convert_forcing', {
      src: repairedSource ?? src,
      dst,
      slots,
      heights: heightsArg,
    });
    Object.assign(state.prepArtifacts, { forcingFile: lastResult, forcingDir: dir });
    $('forcingdir').value = dir;
    if (state.prepArtifacts.siteFile) {
      await scanPreparedSites(state.prepArtifacts.siteFile);
    }
    globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
    status('转换完成：' + lastResult);
  } catch (e) {
    lastResult = null;
    Object.assign(state.prepArtifacts, { forcingFile: null, forcingDir: null });
    globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
    status(e);
  } finally {
    renderCards();
  }
}
