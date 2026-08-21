//! 外壳：左侧工作流、主题、状态栏、右栏页签。

import { state } from './state.js';
import { $ } from './ui.js';

const ready = () => (state.selected ? null : '先在文件与目录建一个算例');

/** 大步骤只负责分组，真正的前后关系由扁平的子步骤决定。 */
export const WORKFLOW = [
  { n: 1, t: '前处理', d: '原始数据转成模型格式', steps: [
    { id: 'prep', page: 'prep', t: '强迫场与站点属性', d: '准备模型输入', need: () => null },
  ] },
  { n: 2, t: '基本设定', d: '建例与基础输入', steps: [
    { id: 'basic-files', page: 'basic', t: '文件与目录', d: '选站点并建算例', need: () => null },
    { id: 'basic-site', page: 'basic', t: '站点信息', d: '位置、地类与站点文件', need: ready },
    { id: 'basic-timing', page: 'basic', t: '时间与预热', d: '模拟范围与 spin-up', need: ready },
    { id: 'basic-grid', page: 'basic', t: '网格与并行', d: '网格和进程划分', need: ready },
    { id: 'basic-surface', page: 'basic', t: '地表数据', d: '地表输入设置', need: ready },
    { id: 'basic-initial', page: 'basic', t: '初始场', d: '初始状态设置', need: ready },
    { id: 'basic-forcing', page: 'basic', t: '强迫场', d: '强迫场读取设置', need: ready },
    { id: 'basic-other', page: 'basic', t: '其他', d: '按配置显示附加设置', need: ready },
  ] },
  { n: 3, t: '过程参数', d: '按过程逐项配置', steps: [
    { id: 'params-water', page: 'params', t: '水热过程', d: '土壤、积雪与水分', need: ready },
    { id: 'params-eco', page: 'params', t: '生态与生地化', d: '植被、碳氮过程', need: ready },
    { id: 'params-river', page: 'params', t: '河道与水库', d: '汇流与水库过程', need: ready },
    { id: 'params-da', page: 'params', t: '数据同化', d: '同化过程设置', need: ready },
    { id: 'params-tracer', page: 'params', t: '示踪剂', d: '示踪过程设置', need: ready },
    { id: 'params-other', page: 'params', t: '其他', d: '调试与诊断', need: ready },
  ] },
  { n: 4, t: '运行', d: '输出与运行', steps: [
    { id: 'run', page: 'run', t: '运行算例', d: '输出、阶段与日志', need: ready },
  ] },
  { n: 5, t: '结果', d: '曲线与指标', steps: [
    { id: 'result', page: 'result', t: '查看结果', d: '绘图与评估', need: ready },
  ] },
];
export const STEPS = WORKFLOW.flatMap(group => group.steps);

export function nextOf(id) {
  const i = STEPS.findIndex(s => s.id === id);
  if (i < 0) return null;
  return STEPS[i + 1] ?? null;
}

export function prevOf(id) {
  const i = STEPS.findIndex(s => s.id === id);
  if (i <= 0) return null;
  return STEPS[i - 1];
}

/** 每个子步骤都用同一对按钮相连。 */
export function renderNextButtons() {
  for (const page of document.querySelectorAll('.page')) {
    const here = STEPS.find(s => s.id === state.step && s.page === page.dataset.step)
      ?? STEPS.find(s => s.page === page.dataset.step);
    const prev = prevOf(here?.id);
    const next = nextOf(here?.id);
    if (!prev && !next) { page.querySelector('.foot')?.remove(); continue; }
    let foot = page.querySelector('.foot');
    if (!foot) {
      foot = document.createElement('div');
      foot.className = 'foot';
      page.appendChild(foot);
    }
    foot.textContent = '';
    if (prev) {
      const b = document.createElement('button');
      b.className = 'btn-ghost';
      b.textContent = '← 上一步';
      b.onclick = () => go(prev.id);
      foot.appendChild(b);
    }
    if (next) {
      const why = next.need();
      const b = document.createElement('button');
      b.className = 'btn-next';
      b.textContent = why ?? `下一步：${next.t} →`;
      b.disabled = !!why;
      b.onclick = () => go(next.id);
      foot.appendChild(b);
    }
  }
}

export function go(id) {
  const step = STEPS.find(s => s.id === id);
  if (!step) { setStatus(`没有这一步：${id}`); return; }
  const why = step.need();
  if (why) { setStatus(why); return; }
  state.step = id;
  for (const p of document.querySelectorAll('.page')) p.hidden = p.dataset.step !== step.page;
  for (const p of document.querySelectorAll('[data-flow-pane]')) {
    p.hidden = p.dataset.flowPane !== id;
  }
  $('work').scrollTop = 0;
  renderSteps();
}

export function renderSteps() {
  const box = $('steps');
  box.textContent = '';
  for (const group of WORKFLOW) {
    const active = group.steps.some(s => s.id === state.step);
    const why = group.steps[0].need();
    const block = document.createElement('div');
    block.className = 'flow-block';
    const d = document.createElement('div');
    d.className = 'step flow-head' + (active ? ' current' : '');
    if (why) d.setAttribute('aria-disabled', 'true');
    d.innerHTML = `<span class="num">${group.n}</span><span class="step-copy">
      <span class="t">${group.t}</span><span class="d">${why ?? group.d}</span></span>`;
    d.onclick = () => go(group.steps[0].id);
    block.appendChild(d);
    if (group.steps.length > 1) {
      const children = document.createElement('div');
      children.className = 'flow-children';
      for (const s of group.steps) {
        const childWhy = s.need();
        const child = document.createElement('div');
        child.className = 'substep' + (state.step === s.id ? ' active' : '');
        if (childWhy) child.setAttribute('aria-disabled', 'true');
        child.innerHTML = `<span class="t">${s.t}</span><span class="d">${childWhy ?? s.d}</span>`;
        child.onclick = () => go(s.id);
        children.appendChild(child);
      }
      block.appendChild(children);
    }
    box.appendChild(block);
  }
  // 左下角说的是当前作用的站点/算例。
  // **批量时必须说出是几个。** 勾了 20 个站点却只显示一个名字，界面看起来
  // 像在配一个，而改一个字段会写进 20 份 case.nml —— 那是看不出异常的破坏。
  // `params.js` 的 `renderScope()` 已经为此立过一次规矩（不能只在状态栏事后
  // 说），左栏是同一个问题的另一半。
  //
  // **站在站点页时以勾选为准，往后以批次为准。** 这两个数在建过算例之后
  // 会同时有值且不相等：勾了 20 个、而批次里还是上次建的那 1 个。
  // 写成 `batch.length || picked.size` 的话短路会让左栏固执地显示旧数字，
  // 而勾选现在每次都重绘左栏 —— 显示旧数字比干脆不刷新更像在骗人。实测踩过。
  const onBasic = state.step.startsWith('basic-');
  const n = onBasic
    ? (state.picked.size || (state.pickedSite ? 1 : 0))
    : (state.batch.length || state.picked.size);
  // **`one` 必须跟 `n` 同源。** 勾了两个、又点了第三个（没勾）的时候，
  // `n` 数的是勾中的两个而 `pickedSite` 是第三个 —— 左栏会写出
  // 「US-Urb 等 2 个」，而那 2 个里根本没有 US-Urb。实测踩过。
  const one = onBasic
    ? (state.picked.size
       ? state.sites.find(x => state.picked.has(x.site_file))?.name
       : state.pickedSite?.name)
    : (state.selected?.name ?? state.pickedSite?.name);
  $('estSite').textContent = n > 1 ? `${one ?? '—'} 等 ${n} 个` : (one ?? '—');
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
