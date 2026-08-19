# GUI 工作流重划实施计划（里程碑 14）

> **给执行者：** 用 `superpowers:subagent-driven-development`（推荐）或
> `superpowers:executing-plans` 按任务逐条实施。步骤用 `- [ ]` 复选框标记。

**目标：** 进门先选站点/区域/全球，站点之后走六步；算例、内核与时间预热
合成「基本设定」；6 个只读派生项并入各分节，常规/专家开关腾空。

**架构：** 全部改动在 `gui/dist/`（纯静态前端）与 `gui/dist/index.html`。
**后端 Rust 一行不动** —— 不新增、不删改任何 Tauri 命令，429 个 Tauri 依赖不用重编。
设计见 `docs/design-gui3.md`。

**技术栈：** Tauri v2、无 npm 的静态 ES module 前端、`xtask check-gui` 做静态守门。
**不引入任何前端框架、构建工具或 npm 依赖** —— 既有约束，见 `design.md` §4.2。

---

## 0. 先读这一节：这个项目怎么验前端

**前端没有测试框架，也不会为这次改动引入一个。** 红-绿循环落在三件事上：

| 手段 | 抓什么 | 抓不到什么 |
|---|---|---|
| `cargo run -p xtask -- check-gui` | invoke 命令名/参数名对不上后端、listen 没有 emit、import 解析不了、**模块成环** | 「这一页上真的有东西」 |
| `node --check <file>.js` | 语法错 | 语义 |
| 跑起来用辅助功能树读 DOM | 页面真的渲染成什么样 | 需要人跑一次 |

**第三条不能省。** 这个项目吃过一次亏：进度条建在一个永远不会到达的输入上，
静态检查全绿，而它从来不动（README「进度条曾经建在一个永远不会到达的输入上」）。
DOM 结构改动尤其如此。

改动前先量一次基线，量到的数写在这里，执行时直接用：

| 事实 | 数 |
|---|---|
| schema 字段总数 | 737 |
| 其中 `group: None`（derived） | 6 |
| 现有步骤 | 5 |
| 前端模块 | 14 个 `.js` |
| `index.html` | 261 行 |

那 6 个 derived 与它们的分节：

| 分节 | 字段 |
|---|---|
| 文件与目录 | `DEF_dir_landdata` `DEF_dir_restart` `DEF_dir_history` |
| 地表数据 | `DEF_USE_USGS` `DEF_USE_IGBP` |
| 示踪剂 | `DEF_wetland_finundation_scheme` |

---

## Task 1: 准备验收手段（不改代码）

**Files:** 无（只写一个临时脚本到 `/tmp`，不入库）

- [ ] **Step 1: 把辅助功能树 dump 脚本存到 `/tmp/ax.sh`**

macOS 上读 WKWebView 渲染结果的唯一免权限途径。`screencapture` 需要「屏幕录制」
授权，终端通常没有；辅助功能树只要「辅助功能」授权，而 System Events 已经有。

递归遍历必须用 `try` 包住每个元素：DOM 在重绘时索引会失效，不包会报
`Invalid index. (-1719)` 然后整个 dump 失败。

```bash
cat > /tmp/ax.sh <<'SH'
#!/bin/bash
# dump CoLM Desktop 的辅助功能树。用法：bash /tmp/ax.sh
osascript <<'EOF'
on walk(e, depth, maxd)
  set out to ""
  if depth > maxd then return out
  tell application "System Events"
    set kids to {}
    try
      set kids to UI elements of e
    end try
    repeat with c in kids
      try
        set v to ""
        try
          set v to value of c as text
        end try
        if v is "missing value" then set v to ""
        set t to ""
        try
          set t to title of c as text
        end try
        set out to out & depth & ">" & (role of c) & " |" & t & "| " & v & linefeed
        set out to out & my walk(c, depth + 1, maxd)
      end try
    end repeat
  end tell
  return out
end walk
tell application "System Events" to tell process "colm-desktop-gui"
  return my walk(UI element 1 of window 1, 0, 7)
end tell
EOF
SH
chmod +x /tmp/ax.sh
```

- [ ] **Step 2: 把点按钮的脚本存到 `/tmp/click.sh`**

后面每个任务都要点几下界面。WKWebView 里的 `<button>` 在辅助功能树里是
`AXButton`，标题就是按钮上的字，可以按标题找到并 `click`。
递归同样要用 `try` 包住 —— 理由和 dump 一样。

```bash
cat > /tmp/click.sh <<'SH'
#!/bin/bash
# 按标题点 CoLM Desktop 里的一个按钮。用法：bash /tmp/click.sh "扫描"
osascript <<EOF
on findbtn(e, t, depth)
  if depth > 8 then return missing value
  tell application "System Events"
    set kids to {}
    try
      set kids to UI elements of e
    end try
    repeat with c in kids
      try
        if role of c is "AXButton" then
          if (title of c as text) contains t then return c
        end if
      end try
      set r to my findbtn(c, t, depth + 1)
      if r is not missing value then return r
    end repeat
  end tell
  return missing value
end findbtn
tell application "System Events" to tell process "colm-desktop-gui"
  set b to my findbtn(UI element 1 of window 1, "$1", 0)
  if b is missing value then return "not found: $1"
  click b
  return "clicked: $1"
end tell
EOF
SH
chmod +x /tmp/click.sh
```

站点列表那种 `<div>` 行不是 `AXButton`，点它要按坐标。存第三个脚本：

```bash
cat > /tmp/clicktext.sh <<'SH'
#!/bin/bash
# 点一个静态文本所在的位置（用于站点列表那种 div 行）。
# 用法：bash /tmp/clicktext.sh "CN-Cng"
osascript <<EOF
on findtext(e, t, depth)
  if depth > 8 then return missing value
  tell application "System Events"
    set kids to {}
    try
      set kids to UI elements of e
    end try
    repeat with c in kids
      try
        if role of c is "AXStaticText" then
          if (value of c as text) is t then return c
        end if
      end try
      set r to my findtext(c, t, depth + 1)
      if r is not missing value then return r
    end repeat
  end tell
  return missing value
end findtext
tell application "System Events" to tell process "colm-desktop-gui"
  set el to my findtext(UI element 1 of window 1, "$1", 0)
  if el is missing value then return "not found: $1"
  set p to position of el
  set sz to size of el
  click at {(item 1 of p) + (item 1 of sz) / 2, (item 2 of p) + (item 2 of sz) / 2}
  return "clicked: $1"
end tell
EOF
SH
chmod +x /tmp/clicktext.sh
```

**窗口可能在副屏，坐标是负的** —— `clicktext.sh` 用的是全局坐标，
副屏在主屏左边时 x 会是负数，那是正常的，不是脚本坏了。

- [ ] **Step 3: 确认 sidecar 与内核都在，否则界面走不到第 3 步以后**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
ls target/debug/colm-cli kernels/waterheat/manifest.json
```

期望：两个路径都存在。缺 `colm-cli` 就 `cargo build -p colm-cli`，
缺内核就 `./oracle/scripts/build_kernel.sh waterheat`。

- [ ] **Step 4: 跑起来量基线**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
bash /tmp/ax.sh | grep -c "^"
```

期望：进程起来，dump 有输出。记下当前左栏是 5 步。

- [ ] **Step 5: 关掉它**

```bash
pkill -f "target/debug/colm-desktop-gui"
```

本任务不提交。三个脚本留在 `/tmp`，后面每个任务都用它们。

---

## Task 2: 六步骨架

把 `STEPS` 从 5 项改成 6 项，页面 id 跟着改名，新增基本设定的空壳页。
**这四处必须一次改完** —— 只改其中一处界面是坏的（`go()` 找不到页面，
或者页面找不到步骤）。

**Files:**
- Modify: `gui/dist/app/state.js`
- Modify: `gui/dist/app/shell.js:10-24`
- Modify: `gui/dist/index.html`

- [ ] **Step 1: `state.js` 补两个字段**

`pickedSite` 现在被 `sites.js` 直接赋值却从未声明，初始是 `undefined` ——
新的 `need()` 要读它，声明出来免得下一个人以为它不存在。

把 `step: 'data',` 那一行改成：

```js
  /** 当前在第几步，见 shell.js 的 STEPS。 */
  step: 'prep',
  /** 这次要跑什么。'site' | 'region' | 'global'，进门那道门设的。
   *  区域与全球还没有步骤链，现在只可能是 'site'。 */
  domain: null,
```

再在 `picked: new Set(),` 那一行之后插入：

```js
  /** 高亮（而不是勾选）的那一个站点。**只高亮不动文件** ——
   *  建算例是第 3 步「确定」按下去的事。 */
  pickedSite: null,
```

- [ ] **Step 2: `shell.js` 的 `STEPS` 换成六项**

把 `export const STEPS = [ ... ];` 整块替换成：

```js
export const STEPS = [
  // 前处理在前：它产出的正是下一步要扫的东西。
  { id: 'prep',   t: '前处理', d: '原始数据转成模型要的格式', need: () => null },
  // 第二步叫「站点」而不是「数据」—— 两步都关于数据，而它实际展示的是站点。
  { id: 'sites',  t: '站点',   d: '扫目录、选站', need: () => null },
  // 基本设定回答「在哪跑、用什么物理、跑多久」。三张卡片顺序不可换 ——
  // 建算例必须在最前，因为内核与时间要写进它产出的 case.nml。
  //
  // 门槛认「选了站点**或者**已经有算例」：重启程序后 recent.json 恢复了
  // 算例目录，那时没有 pickedSite 但算例是现成的，不该被拦在门外。
  { id: 'basic',  t: '基本设定', d: '算例、内核、时间与预热',
    need: () => (state.pickedSite || state.picked.size || state.cases.length
      ? null : '先在第 2 步选一个站点') },
  { id: 'params', t: '参数',   d: 'namelist 字段表',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
  { id: 'run',    t: '运行',   d: '输出与运行',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
  { id: 'result', t: '结果',   d: '曲线与指标',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
];
```

- [ ] **Step 3: `index.html` 把站点页改名，并把 kicker 编号推后**

`data-step="data"` 在整个仓库只出现一次。改成 `sites`：

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
grep -rn 'data-step="data"' gui/dist/
```

期望：只有 `gui/dist/index.html` 一行。把那一行的 `data-step="data"` 改成
`data-step="sites"`，**同时先不要动 `data-own-foot`**（Task 4 才处理）。

然后改四处 kicker 文案：

| 原文 | 改成 |
|---|---|
| `<span class="kicker">第 3 步</span>`（params 页） | `<span class="kicker">第 4 步</span>` |
| `<span class="kicker">第 4 步</span>`（run 页） | `<span class="kicker">第 5 步</span>` |
| `<span class="kicker">第 5 步</span>`（result 页） | `<span class="kicker">第 6 步</span>` |

**从后往前改**，否则改完 params 的「第 3 步→第 4 步」会撞上 run 页原有的
「第 4 步」，下一次替换分不清哪个是哪个。

- [ ] **Step 4: `index.html` 插入基本设定的空壳页**

插在站点页 `</section>` 之后、参数页 `<!-- ③ 参数 -->` 之前。
这一步只放骨架，三张卡片由 Task 4/5/6 分别填进去：

```html
    <!-- ③ 基本设定：跑一次模拟最少要定的三件事 —— 在哪跑、用什么物理、跑多久。
         三张卡片顺序不可换：建算例产出 case.nml，后两张要写进它。 -->
    <section class="page" data-step="basic" hidden>
      <div class="work-head"><span class="kicker">第 3 步</span></div>
      <h1>基本设定</h1>
      <p class="sub">先为选中的站点建算例，再选一套编译内核与时间范围。
        <b>这三样决定了这次模拟是什么</b>；细调留给下一步的参数表。</p>
    </section>
```

同时把参数页与结果页的注释编号改掉：`<!-- ③ 参数 -->` → `<!-- ④ 参数 -->`，
`<!-- ④ 运行 -->` → `<!-- ⑤ 运行 -->`，`<!-- ⑤ 结果 -->` → `<!-- ⑥ 结果 -->`。

- [ ] **Step 5: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/state.js && node --check gui/dist/app/shell.js
cargo run -p xtask -- check-gui
```

期望：`node --check` 无输出；check-gui 打印
`gui: N commands registered, M called, K events listened for — all resolve`。

- [ ] **Step 6: 跑起来验左栏变成六步**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
bash /tmp/ax.sh | grep -A1 "前处理\|基本设定\|站点$"
```

期望：左栏出现「前处理 / 站点 / 基本设定 / 参数 / 运行 / 结果」六项，
后四项灰着并写出原因。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 7: 提交**

```bash
git add gui/dist/app/state.js gui/dist/app/shell.js gui/dist/index.html
git commit -m "把五步骨架换成六步

站点与参数之间插入「基本设定」，站点页 id 从 data 改成 sites。
这一步只搭骨架，三张卡片分三次搬进去。

Constraint: STEPS、页面 id 与 kicker 编号必须一次改齐
Confidence: high
Scope-risk: narrow
Tested: node --check; xtask check-gui; 辅助功能树读出六步"
```

---

## Task 3: 启动弹框

**Files:**
- Create: `gui/dist/app/domain.js`
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/style.css`
- Modify: `gui/dist/app/main.js`

- [ ] **Step 1: 新建 `gui/dist/app/domain.js`**

```js
//! 进门第一道：这次要跑什么。
//!
//! **不是欢迎页，是分流点。** 站点、区域、全球三种域各自要的前处理、
//! 地表数据与并行设置都不一样，将来它们会各自展开自己的步骤链。
//! 三档现在就摆出来，任何一档落地时不用再改这一层，且用户一眼看得到路线图。
//!
//! **没实现的那两档是 disabled，不是「点了报错」** —— 一个能点但必然失败的
//! 入口比一个灰着的更糟。
//!
//! **每次启动都弹，不记忆。** 它在区域与全球落地后是真正的分流点，
//! 不是一次性的欢迎页。
//!
//! 依赖方向只出不进：`main.js` import 它，它不被任何业务模块 import ——
//! `check-gui` 禁止模块成环，而 `sites ↔ results` 有前科。

import { state } from './state.js';
import { $ } from './ui.js';
import { go } from './shell.js';

const DOMAINS = [
  { id: 'site',   t: '站点', d: 'PLUMBER2 / Urban-PLUMBER 单点模拟', ready: true },
  { id: 'region', t: '区域', d: '有限范围网格', ready: false },
  { id: 'global', t: '全球', d: '全球网格', ready: false },
];

/** 立起门。后台初始化在它后面照常跑 —— 门只是视觉遮挡。 */
export function showDomainGate() {
  const box = $('domaincards');
  box.textContent = '';
  for (const d of DOMAINS) {
    const b = document.createElement('button');
    b.className = 'domain-card';
    b.disabled = !d.ready;
    const t = document.createElement('span');
    t.className = 'dt';
    t.textContent = d.t;
    b.appendChild(t);
    const sub = document.createElement('span');
    sub.className = 'dd';
    sub.textContent = d.d;
    b.appendChild(sub);
    if (d.ready) {
      b.onclick = () => pick(d.id);
    } else {
      const soon = document.createElement('span');
      soon.className = 'dsoon';
      soon.textContent = '暂不支持';
      b.appendChild(soon);
    }
    box.appendChild(b);
  }
  $('domaingate').hidden = false;
}

function pick(id) {
  state.domain = id;
  $('domaingate').hidden = true;
  go('prep');
}
```

- [ ] **Step 2: `index.html` 加门的 DOM**

插在 `</div>` （`<div class="app">` 的收尾）之后、`<script>` 之前。
**门是 body 的直接子元素**，因为它要覆盖整个窗口而不是某一栏。

```html
<!-- 进门那道分流。默认 hidden，由 domain.js 立起来 —— 直接写成可见的话，
     JS 加载失败时它会永远挡在那里，而那种故障从外面看就是「程序打不开」。 -->
<div id="domaingate" class="gate" hidden>
  <div class="gate-panel">
    <h2>这次要跑什么？</h2>
    <p class="muted mini">现在只有站点能跑。区域与全球的步骤链还没有实现。</p>
    <div class="domain-cards" id="domaincards"></div>
  </div>
</div>
```

- [ ] **Step 3: `style.css` 加样式**

追加到文件末尾：

```css
/* 进门那道分流。fixed 覆盖整个窗口 —— 它要挡住的是整个工作台，不是某一栏。 */
.gate { position: fixed; inset: 0; z-index: 50; background: var(--bg);
  display: flex; align-items: center; justify-content: center; }
/* **这一条不能省。** `display:flex` 会盖掉 `[hidden]` 的默认
   `display:none`，不补的话门隐藏不了 —— 点完站点界面仍然被挡着。 */
.gate[hidden] { display: none; }
.gate-panel { text-align: center; max-width: 720px; padding: var(--s-xl); }
.gate-panel h2 { margin: 0 0 6px; font-size: 20px; }
.domain-cards { display: flex; gap: var(--s-lg); margin-top: var(--s-xl);
  justify-content: center; }
.domain-card { display: flex; flex-direction: column; gap: 6px; width: 180px;
  padding: var(--s-lg); background: var(--elevated); border: 1px solid var(--border);
  border-radius: var(--r-md); box-shadow: var(--shadow); cursor: pointer;
  font: inherit; color: var(--text); text-align: left; }
.domain-card:hover:not(:disabled) { border-color: var(--accent); }
.domain-card:disabled { opacity: .5; cursor: not-allowed; }
.domain-card .dt { font-size: 16px; font-weight: 600; }
.domain-card .dd { font-size: 12px; color: var(--muted); }
.domain-card .dsoon { font-size: 11px; color: var(--warn); margin-top: 4px; }
```

- [ ] **Step 4: `main.js` 接线**

在 `import { restoreRecent, wirePickers } from './recent.js';` 之后加一行：

```js
import { showDomainGate } from './domain.js';
```

把 `initShell();` 之后的那段改成：

```js
initShell();
// 门先立起来，后台初始化在它后面照常跑 —— 用户点完站点时界面已经就绪。
// **门不拦后台的错误**：list_kernels 失败、示例数据装不上，照常落状态栏，
// 选完站点就看得见。把错误藏在门后面等于延迟暴露。
showDomainGate();
```

再把 `boot()` 末尾的 `go('prep');` 删掉 —— 翻到第一页现在是 `pick()` 的事。
`boot()` 最后两行改成：

```js
  await watchRun();
  addEventListener('colm:mode', () => { if (state.selected) renderFields(); });
}
```

- [ ] **Step 5: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/domain.js && node --check gui/dist/app/main.js
cargo run -p xtask -- check-gui
```

期望：无语法错；check-gui 全绿（尤其**不报 import cycle**）。

- [ ] **Step 6: 跑起来验门**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
bash /tmp/ax.sh | grep "这次要跑什么\|站点\|区域\|全球\|暂不支持"
```

期望：读出「这次要跑什么？」、三张卡片、两个「暂不支持」。

点掉门再 dump 一次，验它真的消失且落在第 1 步：

```bash
bash /tmp/click.sh "站点"
sleep 1
bash /tmp/ax.sh | grep -q "这次要跑什么" && echo "门没关掉！" || echo "门关掉了"
bash /tmp/ax.sh | grep "前处理"
```

期望：打印 `clicked: 站点`、`门关掉了`，且左栏第 1 步「前处理」是当前页。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 7: 提交**

```bash
git add gui/dist/app/domain.js gui/dist/app/main.js gui/dist/index.html gui/dist/app/style.css
git commit -m "进门先分流域类型

站点、区域、全球三档，后两档灰着标暂不支持。每次启动都弹，不记忆 ——
它在区域与全球落地后是真正的分流点，不是欢迎页。

Constraint: 门不拦后台初始化的错误，那些照常落状态栏
Rejected: 只弹一次并记住 | 区域与全球落地后这是真正的分流点
Confidence: high
Scope-risk: narrow
Directive: .gate[hidden] 必须显式 display:none，flex 会盖掉 hidden
Tested: node --check; xtask check-gui 无成环; 辅助功能树验门开与关"
```

---

## Task 4: 算例卡片搬进基本设定

**Files:**
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/sites.js`

- [ ] **Step 1: `index.html` 把「算例放哪」整张卡片搬进 basic 页**

从站点页剪掉 `<h3>算例放哪</h3>` 那张 `<div class="card">`（含 `#root`、
`#urbandirs`、`#rawdata`、`#runtime`），连同它后面那行
`<div class="foot" id="datafoot"></div>`，一起粘到 basic 页 `<p class="sub">`
之后。粘进去时把卡片标题与说明改成这样（它现在既是「放哪」也是「建」）：

```html
      <div class="card">
        <h3>算例</h3>
        <div class="ch">每个站点占一个子目录。默认放在站点数据旁边，可以改。
          <b>按下面的「确定」才会真的建</b> —— 那一步要读站点文件与强迫场并写出
          补齐后的 <code>site.nc</code>。</div>
        <div class="browse">
          <input class="input" id="root" placeholder="…/colm-cases">
          <button class="btn-ghost pick" data-for="root">选择…</button>
          <button class="btn-ghost" id="rescan">重新扫描已建的</button>
        </div>
        <div id="urbandirs" hidden>
          <div class="expert-note">
            选中的站点里有<b>城市站</b>。城市算例必须给全球栅格目录 ——
            土壤剖面、湖深、土壤反照率与 LCZ 分类都只能从那里取，站点文件里没有。
          </div>
          <div class="row">
            <div class="field"><label>rawdata 目录</label>
              <div class="browse"><input class="input" id="rawdata">
                <button class="btn-ghost pick" data-for="rawdata">选择…</button></div></div>
            <div class="field"><label>runtime 目录</label>
              <div class="browse"><input class="input" id="runtime">
                <button class="btn-ghost pick" data-for="runtime">选择…</button></div></div>
          </div>
        </div>
        <!-- 建算例的按钮。**它是卡片内的动作，不是页面出口** ——
             页面出口是底部通用的「下一步：参数 →」。两个长得差不多、
             行为不同的按钮不能摆在一起。 -->
        <div id="datafoot" style="margin-top:12px"></div>
      </div>
```

已建算例的列表**这一步先不加** —— `renderCases()` 还只认运行页那一个容器，
现在加进来会是个永远空着的 div，而空 div 在界面上和「一个算例都没有」
长得一样。它跟着 Task 7 的参数化一起进来。

- [ ] **Step 2: 站点页去掉 `data-own-foot`**

站点页的出口交回通用的「下一步：基本设定 →」。找到这一行：

```html
    <section class="page" data-step="sites" data-own-foot hidden>
```

改成：

```html
    <section class="page" data-step="sites" hidden>
```

同时把站点列表下面那句提示改掉 —— 它现在指的是下一页的按钮：

```html
        <p class="muted mini" style="margin-top:8px">
          点行只是选中，<b>不会动任何文件</b>。建算例在下一步，
          那一步要读站点文件与强迫场并写出补齐后的 <code>site.nc</code>。</p>
```

- [ ] **Step 3: `sites.js` 改 `renderDataFoot` 的职责**

它现在渲染的是站点页的出口（按下去建算例再翻页）。改成渲染算例卡片里的
动作按钮：翻页交给通用出口，它只管建。

把 `renderDataFoot` 整个函数替换成：

```js
/** 算例卡片里的「建算例」按钮。**字要说出它会做什么** ——
 *  它要读站点文件与强迫场并写出补齐后的 site.nc，那是真动文件。
 *
 *  **它不是页面出口。** 出口是底部通用的「下一步：参数 →」，
 *  由 shell.js 的 renderNextButtons 注入。两个长得差不多、行为不同的
 *  按钮不能摆在一起。 */
export function renderDataFoot() {
  const foot = $('datafoot');
  if (!foot) return;
  foot.textContent = '';
  const n = state.picked.size;
  const one = state.pickedSite;
  const b = document.createElement('button');
  b.className = 'btn-next';
  if (n) b.textContent = `确定：为选中的 ${n} 个站点建算例`;
  else if (one) b.textContent = `确定：为 ${one.name} 建算例`;
  else b.textContent = '先回第 2 步点一个站点，或勾选几个';
  b.disabled = !n && !one;
  b.onclick = confirmSelection;
  foot.appendChild(b);
  const info = $('pickinfo');
  if (info) info.textContent = n ? `已勾 ${n} 个` : (one ? `已选 ${one.name}` : '还没选');
}
```

- [ ] **Step 4: `sites.js` 的 `confirmSelection` 不再翻页**

建完算例就停在这一页，让用户看见列表里多出来的那几行。翻页是底部出口的事。
把 `confirmSelection` 里这两行：

```js
    await selectCase(made[0]);
    go('params');
```

改成：

```js
    // 走 selectCase 而不是只设 state.selected：那里还要把 case.nml 读进来、
    // 查出 CoLM 不认识的字段、刷新参数表与预设 —— 只设一个字段的话，
    // 参数页会是空的，而空页面不会报错，只是什么都没有。实测踩过。
    await selectCase(made[0]);
```

`go` 的 import 现在没有消费者了。把 `sites.js` 顶部这一行：

```js
import { go, renderSteps, setStatus } from './shell.js';
```

改成：

```js
import { renderSteps, setStatus } from './shell.js';
```

**这一步不能忘** —— `check-gui` 会验 import 解析得了，但一个没人用的 import
它不报；留着只是给下一个人添乱。

- [ ] **Step 5: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/sites.js
cargo run -p xtask -- check-gui
```

期望：无语法错；check-gui 全绿。

- [ ] **Step 6: 跑起来走一遍**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
```

```bash
bash /tmp/click.sh "站点"                    # 进门
sleep 1
bash /tmp/click.sh "用自带的示例站点"          # 第 2 步：装示例并扫描
sleep 3
bash /tmp/clicktext.sh "CN-Cng"              # 选中那一行
sleep 1
bash /tmp/ax.sh | grep "下一步：基本设定"      # 站点页的出口
bash /tmp/click.sh "下一步：基本设定"
sleep 1
bash /tmp/click.sh "确定：为 CN-Cng 建算例"
sleep 5
bash /tmp/ax.sh | grep "已为 CN-Cng 建好算例\|确定：为\|算例$"
```

期望：站点页底部是「下一步：基本设定 →」；第 3 步的算例卡片里是
「确定：为 CN-Cng 建算例」；按下去之后状态栏读出「已为 CN-Cng 建好算例」。

**这一步还看不到已建算例的列表** —— 它在 Task 7 才进第 3 步。
现在验的是「建这个动作搬对了地方」，不是「列表画出来了」。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 7: 提交**

```bash
git add gui/dist/index.html gui/dist/app/sites.js
git commit -m "把建算例挪到设定算例目录的同一页

站点页出口交回通用的「下一步」，建算例成为算例卡片里的动作按钮。
原来出口摆在设定算例目录之前，等于让人先按确定再想产物放哪。

Constraint: 建算例按钮不是页面出口，两个长得像的按钮不能摆在一起
Confidence: high
Scope-risk: moderate
Tested: node --check; xtask check-gui; 走通选站→建算例整条链"
```

---

## Task 5: 内核卡片搬进基本设定

**Files:**
- Modify: `gui/dist/index.html`

- [ ] **Step 1: 把内核卡片从参数页剪到基本设定**

从参数页剪掉这张卡片：

```html
      <div class="card">
        <h3>内核</h3>
        <div class="ch">一个内核 = 一组编译期宏。选哪个决定了哪些参数有意义、哪些输出变量写得出来。</div>
        <select class="select" id="kernel"></select>
        <p class="muted mini" id="kernelmeta">&nbsp;</p>
      </div>
```

原样粘到 basic 页的算例卡片之后。**内容一个字不改** —— `#kernel` 与
`#kernelmeta` 两个 id 被 `runner.js` 与 `shell.js` 按 id 取，搬家不影响。

- [ ] **Step 2: 参数页的引导语改掉**

它现在还在说「选择编译内核」，而内核已经不在这一页了。把参数页的
`<p class="sub">` 换成：

```html
      <p class="sub">按 CoLM 源码 namelist 的用途配置参数。
        <b>列表随第 3 步选的内核变</b> —— 换 waterheat、urban、bgc 时可配置项会一起换；
        未设置项显示源码默认值，只读派生项排在各分节末尾。</p>
```

- [ ] **Step 3: 静态检查与跑一遍**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
bash /tmp/ax.sh | grep -B2 -A2 "waterheat"
```

期望：`waterheat` 出现在第 3 步的内核卡片里，`kernelmeta` 那行显示
`SinglePoint LULC_IGBP URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF`。
左栏「内核」仍显示 `waterheat`（`renderSteps` 按 id 读 `#kernel`，不受搬家影响）。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 4: 提交**

```bash
git add gui/dist/index.html
git commit -m "内核卡片跟着算例走进基本设定

选哪套物理属于「这次模拟是什么」，不是参数细调。

Confidence: high
Scope-risk: narrow
Tested: xtask check-gui; 辅助功能树读出内核下拉与宏组合"
```

---

## Task 6: 时间与预热搬进基本设定

**Files:**
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/timing.js:1-9`

- [ ] **Step 1: 把 `#timing` 容器从运行页剪到基本设定**

从运行页剪掉这一行：

```html
      <div id="timing"></div>
```

粘到 basic 页的内核卡片之后（也就是 basic 页的最后一个元素）。

- [ ] **Step 2: 运行页的引导语改掉**

它现在还在说「设置时间、预热与输出」。换成：

```html
      <p class="sub">设置输出，然后依次运行 <code>mksrfdata</code>、<code>mkinidata</code>、
        <code>colm</code>。<b>输入没变的阶段会按输入指纹跳过</b>。
        时间与预热在第 3 步。</p>
```

- [ ] **Step 3: `timing.js` 的模块注释改掉**

它现在说「所以它现在放在运行页，紧挨着输出设置」，而那已经不成立。
把开头的注释块换成：

```js
//! 「时间与预热」卡片。
//!
//! 这两样都在 737 个字段的表里躺着（`DEF_simulation_time%*`），但**躺在
//! 表里等于不存在** —— 实测：用户翻完参数页，说"我没有看到 spin-up 的选项"。
//! 一个决定输出从哪天开始的开关，不该和 `DEF_USE_SNICAR` 长得一样。
//! 所以它有自己的卡片，放在第 3 步「基本设定」里 —— 跑多久是「这次模拟是
//! 什么」的一部分，不是「怎么跑」。
//!
//! 值仍然写进同一份 case.nml，改完在参数表里也看得见 —— 这里只是给它一个
//! 说得出后果的入口，不是另一套配置。
//!
//! **刷新时机挂在 `renderFields()` 上**（`params.js` 里 `await renderTiming()`）。
//! 那是选中算例之后必经的一次渲染，而 `#timing` 在哪一页不影响它能不能渲染。
```

- [ ] **Step 4: 静态检查与跑一遍**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/timing.js
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
```

走到第 3 步建完算例，然后：

```bash
bash /tmp/ax.sh | grep "spin-up\|时间与预热\|模拟"
```

期望：「时间与预热（spin-up）」卡片出现在第 3 步，且表格里有真实的时间范围
（不是空的）—— 空的说明 `renderTiming` 没被调到。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 5: 提交**

```bash
git add gui/dist/index.html gui/dist/app/timing.js
git commit -m "时间与预热归入基本设定

跑多久是「这次模拟是什么」的一部分，不是「怎么跑」。

Confidence: high
Scope-risk: narrow
Tested: node --check; xtask check-gui; 第 3 步读出真实时间范围"
```

---

## Task 7: 算例列表两页各一个

**Files:**
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/sites.js:18-56`

第 3 步要「已建算例列表」，第 5 步要「跑哪些」，现在是同一个 `#cases`，
一个 DOM 元素进不了两页。

- [ ] **Step 1: `index.html` 两个容器各就各位**

先在第 3 步的算例卡片里，`<div id="datafoot" style="margin-top:12px"></div>`
那一行之后插入：

```html
        <div id="cases-built" class="listbox" style="margin-top:12px"></div>
```

再把运行页那一行：

```html
        <div id="cases" class="listbox"></div>
```

改成：

```html
        <div id="cases-run" class="listbox"></div>
```

- [ ] **Step 2: `sites.js` 的 `renderCases` 参数化**

把 `export function renderCases() { ... }` 整个函数替换成下面两个。
**对外的签名不变**（仍是无参 `renderCases()`），所以九处调用点一个都不用改：

```js
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
```

- [ ] **Step 3: 确认没有别处还在按老 id 取它**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
grep -rn "'cases'\|\"cases\"" gui/dist/app/ gui/dist/index.html
```

期望：**没有任何输出**。有的话说明还有地方按老 id 取，一并改掉。

- [ ] **Step 4: 静态检查与跑一遍**

```bash
node --check gui/dist/app/sites.js
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
```

走到第 3 步建两个算例（用自带示例只有一个站点，可以先建 CN-Cng，
再改算例目录建第二份），在第 3 步勾中一个，翻到第 5 步：

```bash
bash /tmp/ax.sh | grep -A3 "跑哪些\|运行选中的\|运行全部"
```

期望：第 5 步的列表里那一个是勾着的，`#runall` 按钮的文字是
「运行选中的 1 个」而不是「运行全部 N 个」—— 勾选贯通了。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 5: 提交**

```bash
git add gui/dist/index.html gui/dist/app/sites.js
git commit -m "算例列表在基本设定与运行页各画一份

一个 DOM 元素进不了两页，而两处都要回答「哪些算例存在」。
勾选状态共享 state.pickedCases，两边贯通。

Constraint: renderCases 对外签名不变，九处调用点不动
Rejected: 只留一页、另一页显示摘要 | 摘要行答不了「AU-DaS 到底建出来没有」
Confidence: high
Scope-risk: moderate
Tested: node --check; xtask check-gui; 第 3 步勾选在第 5 步仍然勾着"
```

---

## Task 8: 只读派生项并入各分节

**Files:**
- Modify: `gui/dist/app/params.js:150-175`

- [ ] **Step 1: 去掉专家模式那层过滤**

`params.js` 里这两行：

```js
  // 常规与专家都严格跟随所选内核；专家模式只额外显示源码派生的只读项。
  const shown = inGroup
    .filter(e => !state.irrelevant.has(e.path))
    .filter(e => state.expert || !e.derived);
```

替换成：

```js
  // 严格跟随所选内核。**只读派生项不再藏在专家模式后面** ——
  // 全仓库只有 6 个（DEF_dir_landdata/restart/history、DEF_USE_USGS/IGBP、
  // DEF_wetland_finundation_scheme），它们是「这个值现在是多少」的答案，
  // 而那是个常规问题。
  const shown = inGroup.filter(e => !state.irrelevant.has(e.path));
```

- [ ] **Step 2: 每个分节内部排序，只读排到末尾**

找到分节循环里这一行：

```js
    const rows = visible.filter(e => sectionOf(e) === section);
```

替换成：

```js
    // 可编辑的在前，只读派生项排到本节末尾 —— 只读行混在中间会打断编辑节奏。
    // 按 field_section() 实际推导，最多的一节（文件与目录）也只有 3 个。
    const rows = visible.filter(e => sectionOf(e) === section)
      .sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0));
```

`Array.prototype.sort` 在现代引擎里是稳定的，所以同类之间的原有顺序不变。

- [ ] **Step 3: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/params.js
cargo run -p xtask -- check-gui
```

- [ ] **Step 4: 跑起来验那 6 行在常规模式下就看得见**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
```

走到第 4 步（参数），保持**常规模式**（不点「专家」），然后：

```bash
bash /tmp/ax.sh | grep "派生值，改不了"
```

期望：读出若干行「（派生值，改不了）」。当前内核是 waterheat（TRACEROFF），
所以示踪剂那一个可能被 `irrelevant` 挡掉，看到 3–5 行都算正常；
**一行都没有就是这一步没生效**。

再确认排序：

```bash
bash /tmp/ax.sh | grep -A12 "文件与目录"
```

期望：`DEF_dir_landdata`、`DEF_dir_restart`、`DEF_dir_history` 三行
排在该分节其余字段之后。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 5: 提交**

```bash
git add gui/dist/app/params.js
git commit -m "只读派生项并入各分节，不再藏在专家模式后面

全仓库只有 6 个，它们回答「这个值现在是多少」，而那是个常规问题。
每节内可编辑在前、只读在后，只读行混在中间会打断编辑节奏。

Constraint: state.expert 与 body.expert 保留，那是后续挂选项的钩子
Confidence: high
Scope-risk: narrow
Tested: node --check; xtask check-gui; 常规模式下读出派生行且排在节末"
```

---

## Task 9: 专家模式腾空后要说话

**Files:**
- Modify: `gui/dist/app/params.js`

Task 8 之后 `state.expert` 没有任何消费者了，切过去界面纹丝不动。
**一个点了没反应的按钮比没有按钮更糟** —— 这个项目自己的话：
静默跳过与静默失败在界面上长得一样。

- [ ] **Step 1: 在参数表顶上加一句占位**

`params.js` 的 `renderFields()` 里，找到这一行：

```js
  renderScope(box);
```

在它**之前**插入：

```js
  // 专家模式这轮腾空了 —— 那 6 个只读派生项已经并入各分节。开关与
  // body.expert 都留着等后续挂选项，但空着的时候要明说：一个点了没反应的
  // 按钮比没有按钮更糟。
  if (state.expert) {
    const note = document.createElement('div');
    note.className = 'expert-note';
    note.style.marginBottom = '10px';
    note.textContent =
      '专家选项还在规划中。只读派生项已经并入下面各分节，不再单列 —— '
      + '现在常规模式看到的就是全部。';
    box.appendChild(note);
  }
```

- [ ] **Step 2: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/params.js
cargo run -p xtask -- check-gui
```

- [ ] **Step 3: 跑起来验切换有反应**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
```

走到第 4 步，点顶栏的「专家」，然后：

```bash
bash /tmp/ax.sh | grep "专家选项还在规划中"
```

期望：读得到那句话。再点回「常规」，同一条 grep 应该没有输出。

`main.js` 里 `addEventListener('colm:mode', ...)` 已经接好了重渲染，
不用另外接线 —— 读不到就是那条事件没触发，回去查 `shell.js` 的 `modeSeg` 接线。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 4: 提交**

```bash
git add gui/dist/app/params.js
git commit -m "专家模式空着的时候要说话

派生项并入各分节后这个开关没有消费者了，切过去界面纹丝不动。
一个点了没反应的按钮比没有按钮更糟。

Confidence: high
Scope-risk: narrow
Tested: node --check; xtask check-gui; 切专家读出占位、切回常规消失"
```

---

## Task 10: 全量回归与文档

**Files:**
- Modify: `README.md`

- [ ] **Step 1: 全量静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
for f in gui/dist/app/*.js; do node --check "$f" || echo "FAIL $f"; done
cargo run -p xtask -- check-gui
cargo clippy --all-targets -- -D warnings
cd gui/src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
```

期望：全绿。后端这轮没改，`cargo test` 是回归保险，43 个测试应该全过。

- [ ] **Step 2: 端到端走一遍六步**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 3
```

按顺序走：门选「站点」→ 第 2 步「用自带的示例站点」→ 点 CN-Cng →
「下一步：基本设定」→「确定：为 CN-Cng 建算例」→ 确认内核是 waterheat、
时间卡片有真实范围 →「下一步：参数」→ 确认字段表有内容且有派生行 →
「下一步：运行」→ 点「▶ 运行」→ 等三个阶段跑完 →「下一步：结果」→ 画图。

```bash
bash /tmp/ax.sh > /tmp/final.txt
grep -c "^" /tmp/final.txt
grep "第 6 步\|净辐射\|已完成" /tmp/final.txt
```

期望：六步全部走得通，结果页画得出图。

- [ ] **Step 3: 更新 README 的 GUI 一节**

README 「GUI（部分验收）」那一节描述的是五步与「三栏骨架 + 三个页签」。
在那一节的验收表格之前插入一段：

```markdown
### 六步与进门那道分流

进门先问「这次要跑什么」：站点、区域、全球三档，后两档灰着标「暂不支持」。
**三档现在就摆出来**，将来任何一档落地时不用再改这一层。

选了站点之后是六步：前处理 → 站点 → 基本设定 → 参数 → 运行 → 结果。
「基本设定」回答「在哪跑、用什么物理、跑多久」，装着算例、内核与时间预热
三张卡片；参数是细调，运行是执行与产出。

**只读派生项不再藏在专家模式后面。** 全仓库只有 6 个（`DEF_dir_landdata`
`DEF_dir_restart` `DEF_dir_history` `DEF_USE_USGS` `DEF_USE_IGBP`
`DEF_wetland_finundation_scheme`），它们并入各自的分节并排在节末。
常规/专家开关保留着等后续安排，空着的时候界面会明说。
```

- [ ] **Step 4: 提交**

```bash
git add README.md
git commit -m "README 跟上六步与进门分流

Confidence: high
Scope-risk: narrow
Tested: 端到端走完六步，结果页画得出图"
```

- [ ] **Step 5: 收尾**

```bash
pkill -f "target/debug/colm-desktop-gui"
rm -f /tmp/ax.sh /tmp/final.txt
git log --oneline -9
```

期望：九个提交，从「把五步骨架换成六步」到「README 跟上六步与进门分流」。

---

## 附：这份计划**不做**什么

- 不实现区域与全球的步骤链 —— 只把入口摆出来
- 不给专家模式编内容 —— 腾空等后续安排
- 不动前处理页 —— 它仍是占位页
- 不动结果页 —— 它现在的形状是对的
- 不动后端任何一个 Tauri 命令
- 不引入前端框架、构建工具或 npm 依赖
