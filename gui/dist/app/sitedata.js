//! 前处理页：站点属性子栏（`docs/plan-prep-b.md`，`docs/design-prep.md` §2.2）。
//!
//! 三张卡片：① 位置（经纬度必填、地类可选）→ ② 产物（目录 + 文件名 +
//! 可选 rawdata + 「生成」）→ ③ 结果（12 个必需字段逐个说来自哪里）。
//! ①② 静态放在 `index.html`，③ 探完/生成完才有内容、动态渲染——
//! 与 `forcing.js` 的 `forcing-cards` 同一条约束：`recent.js` 的
//! `wirePickers()` 是一次性绑定，不是事件委托，动态渲染出来的 `pick`
//! 按钮不会被接线，点了没反应也不报错，所以两个「选择…」按钮必须在
//! 页面加载时就已经在 DOM 里。
//!
//! **这份状态不进 `state.js`。** 只有这一页用得到它，与 `forcing.js`
//! 同一个理由：只 import `ipc.js` 与 `ui.js` 两个叶子模块，不读、不写
//! `state`，天然不会跟别的模块成环。
//!
//! **`out` 只在点「生成」那一刻拼。** 目录与文件名分两个框，是因为
//! `site-new` 造的是一份新文件——不像强迫场转换有源文件名可以照抄，
//! 这里没有天然的默认文件名，用户可能要建好几个站点、每次换个名字，
//! 分开两个框比每次都要在一整条路径里找到文件名那一段再改容易。

import { invoke } from './ipc.js';
import { $, status, joinPath } from './ui.js';

/** 上一次「生成」成功的结果。`null` 表示还没生成过，或者刚改过输入。 */
let result = null;

/** 12 个必需字段：地类以外，`site-new` 逐字段说明来自哪里的那些。
 *  只用于「结果」卡片里数一数「一共 12 个」有没有对上——不是给后端用的，
 *  后端已经在 `from_site`/`from_raster`/`from_default` 三个列表里给全了。 */
const REQUIRED_FIELD_COUNT = 12;

/** 解析地类输入框。留空是 `{ value: null }`——合法，意思是「不写，让 CoLM
 *  自己回落」；非空又不是整数是 `{ error }`。**校验与取值走同一个函数**，
 *  不然「生成」按钮判定的合法性与真正发出去的值可能对不上——分开写
 *  两遍校验逻辑，改一处忘改另一处就是这么错的。 */
function parseLandtype() {
  const raw = $('slandtype').value.trim();
  if (!raw) return { value: null };
  const n = Number(raw);
  if (!Number.isInteger(n)) return { error: '地类要是一个整数，留空就不写' };
  return { value: n };
}

/** 「生成」按钮能不能按，以及按不了的原因。**每次输入变化都要重算**——
 *  ④ 的转换按钮（forcing.js）已经立过这个规矩：不能让人点下去才知道
 *  差什么。 */
function readyReasons() {
  const reasons = [];
  const lon = Number($('slon').value.trim());
  const lat = Number($('slat').value.trim());
  if (!$('slon').value.trim() || !Number.isFinite(lon)) reasons.push('经度必填');
  if (!$('slat').value.trim() || !Number.isFinite(lat)) reasons.push('纬度必填');
  const lt = parseLandtype();
  if (lt.error) reasons.push(lt.error);
  if (!$('soutdir').value.trim()) reasons.push('先填产物放哪个目录');
  if (!$('soutname').value.trim()) reasons.push('产物文件名不能为空');
  return reasons;
}

function updateGenerateState() {
  const reasons = readyReasons();
  const btn = $('smake');
  if (btn) btn.disabled = reasons.length > 0;
  const why = $('smake-why');
  if (why) {
    why.className = (reasons.length ? 'fail' : 'muted') + ' mini';
    why.textContent = reasons.length ? reasons.join('；') : '就绪，可以生成。';
  }
}

// 位置、产物两张卡片的每一个输入框改动都要重算「生成」按钮的状态——
// 与 `forcing.js` 的槽位映射同一个道理：不能让人点了才知道差什么。
for (const id of ['slon', 'slat', 'slandtype', 'soutdir', 'soutname', 'srawdata']) {
  const el = $(id);
  if (el) el.addEventListener('input', updateGenerateState);
}
updateGenerateState();

$('smake').onclick = async () => {
  const reasons = readyReasons();
  if (reasons.length) { status(reasons.join('；')); return; }
  const lon = Number($('slon').value.trim());
  const lat = Number($('slat').value.trim());
  const { value: landtype } = parseLandtype();
  const rawdataDir = $('srawdata').value.trim();
  const out = joinPath($('soutdir').value.trim(), $('soutname').value.trim());
  const btn = $('smake');
  btn.disabled = true;
  try {
    result = await invoke('make_site', {
      out,
      lon,
      lat,
      landtype,
      rawdata: rawdataDir || null,
    });
    status(`已生成 ${result.path}：${result.from_default.length} 个字段走标称假设`);
  } catch (e) {
    result = null;
    status(e);
  } finally {
    updateGenerateState();
    renderResult();
  }
};

// --------------------------------------------------------------- ③ 结果

/** 一组来源（站点自身 / rawdata 栅格 / 标称假设）。`cls` 非空时整组
 *  标出状态色——**只有 `warn`/`fail` 两种状态色，没有 `.ok`**：
 *  「来自站点自身」「来自 rawdata 栅格」不上色，默认色就是「正常」。 */
function sourceGroup(title, fields, cls, note) {
  const wrap = document.createElement('div');
  wrap.style.marginTop = '14px';
  const h = document.createElement('div');
  h.className = 'mini' + (cls ? ' ' + cls : '');
  h.style.fontWeight = '650';
  h.textContent = `${title}（${fields.length}）`;
  wrap.appendChild(h);
  if (note) {
    const p = document.createElement('p');
    p.className = (cls ? cls : 'muted') + ' mini';
    p.style.margin = '4px 0 0';
    p.textContent = note;
    wrap.appendChild(p);
  }
  if (!fields.length) {
    const p = document.createElement('p');
    p.className = 'muted mini';
    p.style.margin = '4px 0 0';
    p.textContent = '（无）';
    wrap.appendChild(p);
  } else {
    const ul = document.createElement('ul');
    if (cls) ul.className = cls;
    ul.style.margin = '4px 0 0';
    ul.style.paddingLeft = '18px';
    for (const f of fields) {
      const li = document.createElement('li');
      li.className = 'mini';
      li.textContent = f;
      ul.appendChild(li);
    }
    wrap.appendChild(ul);
  }
  return wrap;
}

function renderResult() {
  const box = $('site-result');
  if (!box) return;
  box.textContent = '';
  if (!result) return;

  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = '<h3>③ 结果</h3>';

  const ch = document.createElement('div');
  ch.className = 'ch';
  const total = result.from_site.length + result.from_raster.length + result.from_default.length;
  ch.textContent = total === REQUIRED_FIELD_COUNT
    ? `CoLM 无条件要读的 ${REQUIRED_FIELD_COUNT} 个必需字段，每个都归到了下面某一类。`
    // 与后端脱钩的信号——12 不该变，见 sitedata_tests.rs 的真 CLI 测试；
    // 界面上不该假装没看见，用户拿这份文件跑之前该知道数对不上。
    : `一共 ${total} 个字段有来源说明（预期 ${REQUIRED_FIELD_COUNT} 个——如果不是，
       说明这份 GUI 与当前的 colm-cli 版本没对齐，先别拿它建算例）。`;
  card.appendChild(ch);

  const summary = document.createElement('table');
  const landtypeText = result.landtype != null
    ? String(result.landtype)
    : '未填 —— 交给 CoLM 按自己的规则回落';
  summary.innerHTML = `
    <tr><th>产物</th><td><code>${result.path}</code></td></tr>
    <tr><th>质地</th><td>${result.texture_name}（第 ${result.texture} 类），BVIC ${result.bvic}</td></tr>
    <tr><th>地类</th><td>${landtypeText}</td></tr>
  `;
  card.appendChild(summary);

  card.appendChild(sourceGroup('来自站点自身', result.from_site, '', ''));
  card.appendChild(sourceGroup('来自 rawdata 栅格', result.from_raster, '', ''));
  card.appendChild(sourceGroup(
    '标称假设',
    result.from_default,
    'warn',
    '这些是标称假设，不是这个站点实测的 —— 拿这份文件跑出来的结果，' +
    '这些字段部分的可信度取决于这一点，不能当成量出来的数。',
  ));

  box.appendChild(card);
}
