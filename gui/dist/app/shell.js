//! 外壳：步骤条、常规/专家、主题、状态栏、右栏页签。
//!
//! 这一层不碰业务，只管「现在在哪一步、界面长什么样」。
//! 把它单独拎出来是因为它是**唯一回答「这几块什么关系」的地方** ——
//! 原来那版把站点库、新建、算例库并排摆着，谁也看不出它们是一条流水线。

import { state } from './state.js';
import { $ } from './ui.js';

/** 六步。`need` 说明这一步要什么才能进 —— 灰着的步骤要能说出为什么。 */
export const STEPS = [
  // 前处理在前：它产出的正是后面要扫的东西。
  { id: 'prep',   t: '前处理', d: '原始数据转成模型要的格式', need: () => null },
  // **内核排在站点前面，顺序由依赖链定。** 城市站必须走 URBANON 编进去的
  // 内核，还要给全球栅格目录；default 内核跑不了城市站，要的数据和路径也
  // 完全不同。反过来排的话，人挑完二十个城市站才发现手上是 default。
  { id: 'basic',  t: '基本设定', d: '内核与算例目录', need: () => null },
  // 门槛判的是**有没有可用的内核**，不是「用户选了没有」—— 单选 select
  // 只要有 option 就必然选中一项，用户没有「不选」这个动作。
  //
  // **文案必须指向真正的出路。** 说「去第 2 步选一个内核」是死路：
  // 没有内核时那一页也只有一句「没有找到内核」，人照做过去、发现没得选、
  // 于是卡在这里。.gitignore 忽略 /kernels/，新克隆的开发树默认就是这个状态。
  { id: 'sites',  t: '站点',   d: '扫目录、选站、建算例',
    need: () => (state.kernels.length
      ? null
      : '还没有可用的内核 —— 先构建 kernels/（见 README「什么时候要自己编内核」）') },
  { id: 'params', t: '参数',   d: '时间与预热 · namelist 字段表',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
  { id: 'run',    t: '运行',   d: '输出与运行',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
  { id: 'result', t: '结果',   d: '曲线与指标',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
];

/** 下一步是哪一步。**每一页都要有出口** —— 让人自己回左栏找下一步，
 *  等于把「现在该干嘛」这个问题推给用户，而那正是原来那版最大的毛病。 */
export function nextOf(id) {
  // `findIndex` 找不到时返回 -1，而 `STEPS[-1 + 1]` 正好是第一步 ——
  // `?? null` 永远兜不住。表现是：改 step id 时漏改一处，页面不报错，
  // 只是渲染出一个**指回第 1 步**的「下一步」。实测 nextOf('data') === prep。
  const i = STEPS.findIndex(s => s.id === id);
  if (i < 0) return null;
  return STEPS[i + 1] ?? null;
}

/** 给每一页底部装一个「下一步」。进不去时按钮说出原因，而不是只灰着。 */
export function renderNextButtons() {
  for (const page of document.querySelectorAll('.page')) {
    // 有自己出口的页不注入通用按钮：站点页的出口要先建算例再走，
    // 两个长得差不多、行为不同的按钮摆在一起，比没有按钮更糟。
    if (page.hasAttribute('data-own-foot')) continue;
    const next = nextOf(page.dataset.step);
    // 没有下一步的那一页（结果页）不该留一个空的 `.foot` —— 它带
    // border-top 与 padding，实测在页面底部渲染出一条 33px 的、
    // 下面什么都没有的横线。
    if (!next) { page.querySelector('.foot')?.remove(); continue; }
    let foot = page.querySelector('.foot');
    if (!foot) {
      foot = document.createElement('div');
      foot.className = 'foot';
      page.appendChild(foot);
    }
    foot.textContent = '';
    const why = next.need();
    const b = document.createElement('button');
    b.className = 'btn-next';
    b.textContent = why ? why : `下一步：${next.t} →`;
    b.disabled = !!why;
    b.onclick = () => go(next.id);
    foot.appendChild(b);
  }
}

export function go(id) {
  const step = STEPS.find(s => s.id === id);
  // 未知 id 时 `step?.need()` 是 undefined（假值），于是照常往下走，
  // 把**所有**页都 hide 掉 —— 内容区整块空白，而且不报错。
  // 实测 go('nope') 之后可见页数 0。改 id 的任务还有好几个，让它说出来。
  if (!step) { setStatus(`没有这一步：${id}`); return; }
  const why = step.need();
  if (why) { setStatus(why); return; }
  state.step = id;
  for (const p of document.querySelectorAll('.page')) p.hidden = p.dataset.step !== id;
  renderSteps();
}

export function renderSteps() {
  const box = $('steps');
  box.textContent = '';
  for (const [i, s] of STEPS.entries()) {
    const why = s.need();
    const d = document.createElement('div');
    d.className = 'step' + (state.step === s.id ? ' active' : '');
    if (why) d.setAttribute('aria-disabled', 'true');
    d.innerHTML = `<span class="num">${i + 1}</span><span class="step-copy">
      <span class="t">${s.t}</span><span class="d">${why ?? s.d}</span></span>`;
    d.onclick = () => go(s.id);
    box.appendChild(d);
  }
  // 左下角那两行说的是「当前上下文」—— 切到任何一步都还看得见选了哪个站、
  // 用的哪个内核。那两样决定了另外几步的行为。
  //
  // **批量时必须说出是几个。** 勾了 20 个站点却只显示一个名字，界面看起来
  // 像在配一个，而改一个字段会写进 20 份 case.nml —— 那是看不出异常的破坏。
  // `params.js` 的 `renderScope()` 已经为此立过一次规矩（不能只在状态栏事后
  // 说），左栏是同一个问题的另一半。
  //
  // **站在站点页时以勾选为准，往后以批次为准。** 这两个数在建过算例之后
  // 会同时有值且不相等：勾了 20 个、而批次里还是上次建的那 1 个。
  // 写成 `batch.length || picked.size` 的话短路会让左栏固执地显示旧数字，
  // 而勾选现在每次都重绘左栏 —— 显示旧数字比干脆不刷新更像在骗人。实测踩过。
  const onSites = state.step === 'sites';
  const n = onSites
    ? (state.picked.size || (state.pickedSite ? 1 : 0))
    : (state.batch.length || state.picked.size);
  // **`one` 必须跟 `n` 同源。** 勾了两个、又点了第三个（没勾）的时候，
  // `n` 数的是勾中的两个而 `pickedSite` 是第三个 —— 左栏会写出
  // 「US-Urb 等 2 个」，而那 2 个里根本没有 US-Urb。实测踩过。
  const one = onSites
    ? (state.picked.size
       ? state.sites.find(x => state.picked.has(x.site_file))?.name
       : state.pickedSite?.name)
    : (state.selected?.name ?? state.pickedSite?.name);
  $('estSite').textContent = n > 1 ? `${one ?? '—'} 等 ${n} 个` : (one ?? '—');
  const k = $('kernel');
  $('estKernel').textContent = k?.selectedIndex >= 0 ? k.options[k.selectedIndex].textContent : '—';
  $('casename').value = state.batch.length > 1
    ? `${state.batch.length} 个算例`
    : (state.selected ? state.selected.dir : '还没有算例');
  // 步骤条与「下一步」按钮是**同一份状态**推出来的，必须一起刷新。
  // 分开刷新的结果是：左栏已经亮了，页底那个按钮还写着「先去扫描」。实测踩过。
  renderNextButtons();
}

export function setStatus(msg) { $('status').textContent = String(msg); }

/** 状态灯：空闲 / 忙 / 成功 / 失败。**运行是这个程序里唯一会让人等的事**，
 *  切到别的步骤时它仍然要看得见 —— 这正是状态栏存在的理由。 */
export function setRunning(kind, text) {
  const dot = $('rdot');
  dot.className = 'rdot' + (kind === 'busy' ? ' busy' : kind === 'ok' ? ' on' : '');
  $('rtext').textContent = text;
}

export function initShell() {
  // 主题：跟随系统，但用户能自己切，切了就记住。
  const saved = localStorage.getItem('theme');
  const sys = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  setTheme(saved ?? sys);
  $('themeBtn').onclick = () => {
    const next = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
    setTheme(next);
    localStorage.setItem('theme', next);
  };

  for (const b of document.querySelectorAll('#modeSeg button')) {
    b.onclick = () => {
      for (const x of document.querySelectorAll('#modeSeg button')) x.classList.toggle('on', x === b);
      state.expert = b.dataset.mode === 'expert';
      document.body.className = state.expert ? 'expert' : 'normal';
      window.dispatchEvent(new CustomEvent('colm:mode'));
    };
  }

  const tabs = $('livetabs');
  for (const b of tabs.querySelectorAll('button')) {
    b.onclick = () => {
      for (const x of tabs.querySelectorAll('button')) x.classList.toggle('on', x === b);
      for (const p of document.querySelectorAll('.live-pane'))
        p.classList.toggle('on', p.dataset.pane === b.dataset.pane);
    };
  }
  renderSteps();
}

function setTheme(t) {
  document.documentElement.dataset.theme = t;
  $('themeBtn').textContent = t === 'dark' ? '☀️' : '🌙';
}
