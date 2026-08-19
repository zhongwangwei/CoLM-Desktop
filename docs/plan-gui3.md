# GUI 工作流重划实施计划（里程碑 14）

> **给执行者：** 用 `superpowers:subagent-driven-development`（推荐）或
> `superpowers:executing-plans` 按任务逐条实施。步骤用 `- [ ]` 复选框标记。

**目标：** 进门先选站点/区域/全球；站点之后走六步，**内核排在站点前面**
（内核决定站点跑不跑得了）；时间与预热跟着 `case.nml` 走到参数页；
6 个只读派生项并入各分节，常规/专家开关腾空；`waterheat` 更名 `default`。

**架构：** 界面改动全在 `gui/dist/`（纯静态前端）与 `gui/dist/index.html`，
**不新增、不删改任何 Tauri 命令**。唯一越出前端的是 Task 12 的 `waterheat` →
`default` 改名，它动脚本、Rust 测试与黄金基准。设计见 `docs/design-gui3.md`。

**修订记录：** Task 2 / 2b 落地时的顺序是「站点 → 基本设定」，
后来确认依赖链是反的（内核无依赖 → 站点要先知道内核 → 时间要先有
`case.nml`），Task 2c 负责把顺序转过来。已落地的两个提交不回滚 ——
它们的骨架、多站点上下文与门槛机制都还在用。

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

**改 Fortran 之后必须先提交再编译。** `oracle/scripts/build_kernel.sh:41` 用

```
git -C "$SRC" -c core.symlinks=false worktree add --detach --force "$BUILD" HEAD
```

从一个**钉在 `HEAD` 的 worktree** 构建 —— 工作树里未提交的改动对编译**不可见**。
实测踩过：改完 Fortran 直接 `build_kernel.sh`，编出来的是没有修复的二进制，
探针忠实复现了旧 bug，一度看起来像修复失败。判据是 `manifest.json` 的
`colm_git_sha`：它等于 submodule 的 `HEAD`，不是你手上那份代码。

那 6 个 derived 与它们的分节：

| 分节 | 字段 |
|---|---|
| 文件与目录 | `DEF_dir_landdata` `DEF_dir_restart` `DEF_dir_history` |
| 地表数据 | `DEF_USE_USGS` `DEF_USE_IGBP` |
| 示踪剂 | `DEF_wetland_finundation_scheme` |

---

## Task 1: 准备验收手段（不改代码）✅ 已完成

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

## Task 2: 六步骨架 ✅ 已完成（`2ecdfc9`，顺序待 Task 2c 反转）

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

## Task 2b: 多站点是一等公民 ✅ 已完成（`3d49d8b`，左栏刷新待 Task 2c 打通）

第 2 步可以勾任意多个站点，第 3 步的三张卡片全部对整批生效。
**后端与状态层本来就是批量的**（`state.picked` 是 Set、`state.batch` 是目录数组、
`read_timing` 收 `dirs`、`varying_fields` 专答「这批里哪些字段取值不同」）——
要补的是界面：它现在把批量说成了单数。

**Files:**
- Modify: `gui/dist/app/shell.js`
- Modify: `gui/dist/index.html`

- [ ] **Step 1: `shell.js` 顶上那句注释改成六步**

Task 2 只换了 `STEPS` 数组本身，这行注释还写着「五步」：

```js
/** 五步。`need` 说明这一步要什么才能进 —— 灰着的步骤要能说出为什么。 */
```

改成：

```js
/** 六步。`need` 说明这一步要什么才能进 —— 灰着的步骤要能说出为什么。 */
```

- [ ] **Step 2: `shell.js` 的门槛文案说清楚可以多选**

`STEPS` 里 basic 那一项的 `need()`，把 `'先在第 2 步选一个站点'` 改成
`'先在第 2 步选站点（可以多选）'`：

```js
  { id: 'basic',  t: '基本设定', d: '算例、内核、时间与预热',
    need: () => (state.pickedSite || state.picked.size || state.cases.length
      ? null : '先在第 2 步选站点（可以多选）') },
```

- [ ] **Step 3: `shell.js` 的左栏上下文说出是几个**

`renderSteps()` 末尾这三行：

```js
  $('estSite').textContent = state.selected?.name ?? '—';
  const k = $('kernel');
  $('estKernel').textContent = k?.selectedIndex >= 0 ? k.options[k.selectedIndex].textContent : '—';
  $('casename').value = state.selected ? state.selected.dir : '还没有算例';
```

替换成：

```js
  // **批量时必须说出是几个。** 勾了 20 个站点却只显示一个名字，界面看起来
  // 像在配一个，而改一个字段会写进 20 份 case.nml —— 那是看不出异常的破坏。
  // `params.js` 的 `renderScope()` 已经为此立过一次规矩（不能只在状态栏事后
  // 说），左栏是同一个问题的另一半。
  //
  // 数取 batch 优先：建完算例之后它才是参数改动的**实际**作用对象；
  // 还没建时退回勾中的站点数。
  const n = state.batch.length || state.picked.size;
  const one = state.selected?.name ?? state.pickedSite?.name
    ?? state.sites.find(x => state.picked.has(x.site_file))?.name;
  $('estSite').textContent = n > 1 ? `${one ?? '—'} 等 ${n} 个` : (one ?? '—');
  const k = $('kernel');
  $('estKernel').textContent = k?.selectedIndex >= 0 ? k.options[k.selectedIndex].textContent : '—';
  $('casename').value = state.batch.length > 1
    ? `${state.batch.length} 个算例`
    : (state.selected ? state.selected.dir : '还没有算例');
```

循环里的 `s` 是步骤，这里的 `x` 是站点 —— **别把 `x` 写成 `s`**，
虽然作用域不同不会报错，但读的人要停下来想一次。

- [ ] **Step 4: `index.html` 基本设定页的引导语说清楚对整批生效**

把 basic 页的 `<p class="sub">` 换成：

```html
      <p class="sub">为第 2 步选中的<b>每一个</b>站点建算例，再选一套编译内核与时间范围。
        <b>这三样对整批生效</b> —— 勾了几个就配几个；取值不一致的字段会被标出来，
        细调留给下一步的参数表。</p>
```

- [ ] **Step 5: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/shell.js
cargo run -p xtask -- check-gui
```

- [ ] **Step 6: 跑起来验单站与多站两种显示**

```bash
cd gui/src-tauri && cargo build && (./target/debug/colm-desktop-gui &) && sleep 4
bash /tmp/ax.sh | grep "先在第 2 步选站点\|已选站点" -A2
```

期望：第 3 步灰着时写的是「先在第 2 步选站点（可以多选）」。

再验多站显示：进门 → 第 2 步扫出站点 → 点「全选」→ 看左栏。

```bash
bash /tmp/click.sh "站点"; sleep 1
bash /tmp/click.sh "用自带的示例站点"; sleep 3
bash /tmp/click.sh "全选"; sleep 1
bash /tmp/ax.sh | grep -A2 "已选站点"
```

期望：自带示例只有一个站点，所以显示的是 `CN-Cng` 而不是「等 1 个」——
`n > 1` 才加后缀，**1 个的时候不该出现「等 1 个」**。

若手边有 PLUMBER2 的 Sitedata 目录（90 个站点），扫它再全选，
左栏应显示 `AT-Neu 等 90 个`。没有那份数据就跳过这一条，在报告里写明跳过了。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 7: 提交**

```bash
git add gui/dist/app/shell.js gui/dist/index.html
git commit -m "左栏说出在配几个站点

勾了 20 个却只显示一个名字，界面看起来像在配一个，而改一个字段会写进
20 份 case.nml —— 那是看不出异常的破坏。顺手把 STEPS 上面那句「五步」
注释改成六步。

Constraint: 1 个的时候不加「等 N 个」后缀
Confidence: high
Scope-risk: narrow
Directive: 批量是常态不是特例，新增的上下文显示都要先回答「几个」
Tested: node --check; xtask check-gui; 单站显示不带后缀"
```

---

## Task 2c: 把顺序转过来，并让骨架自洽

Task 2 的六步顺序是「站点 → 基本设定」，而依赖链要求反过来。同时把两轮审查
发现的四个骨架问题一并修掉 —— 它们都是「骨架换了、周边没跟上」的同一类。

**Files:**
- Modify: `gui/dist/app/shell.js`
- Modify: `gui/dist/app/sites.js`
- Modify: `gui/dist/index.html`

- [ ] **Step 1: `shell.js` 的 `STEPS` 反转顺序并重设门槛**

把整个 `STEPS` 数组替换成：

```js
export const STEPS = [
  // 前处理在前：它产出的正是后面要扫的东西。
  { id: 'prep',   t: '前处理', d: '原始数据转成模型要的格式', need: () => null },
  // **内核排在站点前面，顺序由依赖链定。** 城市站必须走 URBANON 编进去的
  // 内核，还要给全球栅格目录；default 内核跑不了城市站，要的数据和路径也
  // 完全不同。反过来排的话，人挑完二十个城市站才发现手上是 default。
  { id: 'basic',  t: '基本设定', d: '内核与算例目录', need: () => null },
  // 门槛认「选了内核」而不是「有内核」：下拉框里没选中任何一项时
  // $('kernel').value 是空串，那时建出来的算例没有物理可跑。
  { id: 'sites',  t: '站点',   d: '扫目录、选站、建算例',
    need: () => (document.getElementById('kernel')?.value
      ? null : '先在第 2 步选一个内核') },
  { id: 'params', t: '参数',   d: '时间与预热 · namelist 字段表',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
  { id: 'run',    t: '运行',   d: '输出与运行',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
  { id: 'result', t: '结果',   d: '曲线与指标',
    need: () => (state.selected ? null : '先在第 3 步建一个算例') },
];
```

`need()` 里直接用 `document.getElementById` 而不是 `$` —— `$` 来自 `ui.js`，
而 `STEPS` 是模块顶层的常量，`need()` 在 `renderSteps()` 里每次都调，
用哪个都行；这里写全名是为了让「它读的是 DOM 而不是 state」一眼看得见。

- [ ] **Step 2: `shell.js` 的 `nextOf()` 兜住未知 id**

现在是：

```js
export function nextOf(id) {
  const i = STEPS.findIndex(s => s.id === id);
  return STEPS[i + 1] ?? null;
}
```

改成：

```js
export function nextOf(id) {
  // `findIndex` 找不到时返回 -1，而 `STEPS[-1 + 1]` 正好是第一步 ——
  // `?? null` 永远兜不住。表现是：改 step id 时漏改一处，页面不报错，
  // 只是渲染出一个**指回第 1 步**的「下一步」。实测 nextOf('data') === prep。
  const i = STEPS.findIndex(s => s.id === id);
  if (i < 0) return null;
  return STEPS[i + 1] ?? null;
}
```

- [ ] **Step 3: `shell.js` 的 `go()` 兜住未知 id**

现在是：

```js
export function go(id) {
  const step = STEPS.find(s => s.id === id);
  const why = step?.need();
  if (why) { setStatus(why); return; }
```

改成：

```js
export function go(id) {
  const step = STEPS.find(s => s.id === id);
  // 未知 id 时 `step?.need()` 是 undefined（假值），于是照常往下走，
  // 把**所有**页都 hide 掉 —— 内容区整块空白，而且不报错。
  // 实测 go('nope') 之后可见页数 0。改 id 的任务还有好几个，让它说出来。
  if (!step) { setStatus(`没有这一步：${id}`); return; }
  const why = step.need();
  if (why) { setStatus(why); return; }
```

- [ ] **Step 4: `shell.js` 的 `renderNextButtons()` 不给末步留空 `.foot`**

找到这一段：

```js
    const next = nextOf(page.dataset.step);
    let foot = page.querySelector('.foot');
    if (!foot) {
      foot = document.createElement('div');
      foot.className = 'foot';
      page.appendChild(foot);
    }
    foot.textContent = '';
    if (!next) continue;
```

改成：

```js
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
```

- [ ] **Step 5: `sites.js` 让勾选立刻刷新左栏**

Task 2b 加的「已选站点：AT-Neu 等 20 个」只在 `renderSteps()` 里刷新，
而勾选走的是 `renderSites()` 与勾选框的 `onchange`，两条路都不调它 ——
实测勾完左栏还写着上一次的数，要切一次步骤才对得上。

在 `renderSites()` 函数**末尾**（`for` 循环之后、函数收尾的 `}` 之前）加：

```js
  // 左栏的「已选站点」现在会说出**在配几个**，而勾选正是改变那个数的动作。
  // 不在这里刷一次的话，勾了 20 个左栏还写着上一次的数。
  renderSteps();
```

再在站点行勾选框的 `onchange` 里补一行 —— 它不重绘整个列表（会丢焦点），
所以要单独刷左栏：

```js
    cb.onchange = () => {
      if (cb.checked) state.picked.add(s.site_file); else state.picked.delete(s.site_file);
      renderDataFoot();
      renderSteps();   // 勾选改变了「在配几个」，左栏要立刻跟上
      $('urbandirs').hidden = !state.sites.some(x => x.urban && state.picked.has(x.site_file));
    };
```

- [ ] **Step 6: `sites.js` 的出口指向新的下一步**

`confirmSelection()` 末尾现在是 `go('params')`。站点页现在是第 3 步，
它的下一步正是参数页，**这一行不用改**。确认一下就行：

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
grep -n "go('" gui/dist/app/sites.js
```

期望：只有一处 `go('params')`。**不要改它。**

- [ ] **Step 7: `index.html` 交换站点页与基本设定页的位置**

现在 DOM 顺序是 `prep → sites → basic → params → run → result`。
把 `data-step="basic"` 那个 `<section>` 整块剪下来，粘到 `data-step="sites"`
那个 `<section>` **之前**。

DOM 顺序本身不决定显示顺序（`go()` 按 id 显示/隐藏，左栏按 `STEPS` 排），
但**读文件的人按 DOM 顺序理解流程**，让它和 `STEPS` 一致。

- [ ] **Step 8: `index.html` 重排 kicker 编号与页面文案**

| 页面 | kicker 改成 | 另外要改的 |
|---|---|---|
| `basic` | `第 2 步` | `<h1>` 保持「基本设定」；`<p class="sub">` 换成下面那段 |
| `sites` | `第 3 步` | `<p class="sub">` 末尾补一句建算例 |
| `params` | `第 4 步` | 不变 |
| `run` | `第 5 步` | 不变 |
| `result` | `第 6 步` | 不变 |

basic 页的 `<p class="sub">` 换成：

```html
      <p class="sub">先定<b>用哪套物理</b>和<b>算例放哪</b>。
        内核是编译期决定的，它决定了下一步哪些站点跑得了 ——
        城市站必须走 <code>urban</code> 内核，还要额外给全球栅格目录。</p>
```

sites 页的 `<p class="sub">` 换成：

```html
      <p class="sub">指向 PLUMBER2 或 Urban-PLUMBER 的 <code>Sitedata</code> 目录。
        程序顺着命名约定把每个站点的强迫场与观测文件一并找出来 ——
        <b>有没有观测，决定了跑完能不能自动评估</b>。选好之后在这一页建算例。</p>
```

- [ ] **Step 9: `index.html` 三处过期的步骤交叉引用**

这三处指的都是运行页（现在是第 5 步）与预热（Task 7 之后落在第 4 步）：

| 行 | 现在 | 改成 |
|---|---|---|
| 42 | `判据与第 4 步的阶段跳过一样` | `判据与第 5 步的阶段跳过一样` |
| 206 | `与第 4 步里的模型预热（spin-up）不是一回事。` | `与第 4 步里的模型预热（spin-up）不是一回事。`（**不用改** —— 预热 Task 7 会搬到参数页，正好是第 4 步） |
| 225 | `与第 4 步的预热（spin-up）不是一回事` | 同上，**不用改** |

**只改第 42 行那一处。** 另外两处凑巧因为预热搬到参数页而重新成立了 ——
在 Task 7 落地之前它们是错的，落地之后是对的，而 Task 7 就在后面。
在提交信息里写明这一点，免得下一个人以为漏了。

- [ ] **Step 10: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/shell.js && node --check gui/dist/app/sites.js
cargo run -p xtask -- check-gui
```

期望：无语法错；check-gui 打印 `gui: 29 commands registered, 28 called, 4 events listened for — all resolve`。

- [ ] **Step 11: 跑起来验顺序与门槛**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust/gui/src-tauri
cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
bash $S/ax.sh | grep "前处理\|基本设定\|站点\|参数\|运行\|结果\|先在第"
```

期望左栏依次是：

```
1 前处理    原始数据转成模型要的格式
2 基本设定  内核与算例目录
3 站点      扫目录、选站、建算例        ← 这台机器编了内核，所以不灰
4 参数      先在第 3 步建一个算例
5 运行      先在第 3 步建一个算例
6 结果      先在第 3 步建一个算例
```

**第 3 步灰不灰取决于内核下拉框选中了没有。** 这台机器 `kernels/` 下有内核，
启动时 `refreshKernels()` 会选中第一个，所以第 3 步应该是**不灰**的。
若它灰着并写「先在第 2 步选一个内核」，说明 `$('kernel').value` 是空 ——
去看启动日志里那行 `N preset(s) from ...`。

再验未知 id 的兜底真的会说话：

```bash
bash $S/ax.sh | grep -c "^"
```

记下行数。这一步不方便从外面调 `go('nope')`，兜底逻辑靠代码审查确认即可。

```bash
pkill -f "target/debug/colm-desktop-gui"
```

- [ ] **Step 12: 提交**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add gui/dist/app/shell.js gui/dist/app/sites.js gui/dist/index.html
git commit -m "内核排到站点前面，并让骨架自洽

顺序由依赖链定：内核无依赖，站点要先知道内核（城市站必须 URBANON 且要
全球栅格目录）。反过来排的话，人挑完二十个城市站才发现手上是 default。

一并修掉四处骨架问题：nextOf 对未知 id 返回第一步而不是 null（
STEPS[-1+1] 正好是第一步，?? null 永远兜不住）；go 对未知 id 把所有页
hide 掉且不报错；结果页留一条 33px 的空 .foot 横线；勾选站点不刷新左栏，
Task 2b 加的「在配几个」要切步骤才对得上。

index.html 第 206、225 行的「第 4 步预热」没改 —— 预热 Task 7 会搬到
参数页，正好是第 4 步，那两处届时自动成立。

Constraint: 站点页出口仍是 go('params')，站点现在就是参数的上一步
Confidence: high
Scope-risk: moderate
Directive: 改 step id 时先看 nextOf/go 的兜底还在不在
Tested: node --check; xtask check-gui; 辅助功能树读出六步新顺序"
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

插在 `<div class="app">` 的收尾 `</div>` 之后、`<script>` 之前。
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

- [ ] **Step 3: `style.css` 加样式（追加到文件末尾）**

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

在 `import { restoreRecent, wirePickers } from './recent.js';` 之后加：

```js
import { showDomainGate } from './domain.js';
```

在 `initShell();` 之后加：

```js
// 门先立起来，后台初始化在它后面照常跑 —— 用户点完站点时界面已经就绪。
// **门不拦后台的错误**：list_kernels 失败、示例数据装不上，照常落状态栏，
// 选完站点就看得见。把错误藏在门后面等于延迟暴露。
showDomainGate();
```

把 `boot()` 末尾的 `go('prep');` 删掉 —— 翻到第一页现在是 `pick()` 的事。
`boot()` 最后两行变成：

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

期望：无语法错；check-gui 全绿，**尤其不报 import cycle**。

- [ ] **Step 6: 用 playwright 验（这一步不需要真窗口）**

门是纯前端的，浏览器里就能验，比编译 Tauri 快得多。
**注意**：浏览器里没有 IPC 后端，`boot()` 整个不执行，只有 `initShell()`
和门会跑 —— 这一步只验门本身，别指望别的。

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust/gui/dist
python3 -m http.server 8749 > /dev/null 2>&1 &
sleep 1
```

用 playwright 打开 `http://127.0.0.1:8749/`，读页面快照，确认：
- 「这次要跑什么？」标题在
- 三张卡片：站点（可点）、区域（disabled）、全球（disabled）
- 后两张各带「暂不支持」

再点「站点」，确认门消失、左栏第 1 步「前处理」是当前页。

```bash
pkill -f "http.server 8749"
```

playwright 的运行产物落在 `.playwright-mcp/`，**收尾时删掉**，别让它进仓库。

- [ ] **Step 7: 提交**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add gui/dist/app/domain.js gui/dist/app/main.js gui/dist/index.html gui/dist/app/style.css
git commit -m "进门先分流域类型

站点、区域、全球三档，后两档灰着标暂不支持。每次启动都弹，不记忆 ——
它在区域与全球落地后是真正的分流点，不是欢迎页。

Constraint: 门不拦后台初始化的错误，那些照常落状态栏
Rejected: 只弹一次并记住 | 区域与全球落地后这是真正的分流点
Confidence: high
Scope-risk: narrow
Directive: .gate[hidden] 必须显式 display:none，flex 会盖掉 hidden
Tested: node --check; xtask check-gui 无成环; playwright 验门开与关"
```

---

## Task 4: 内核卡片搬到第 2 步

内核现在在参数页，而它是**第 2 步的第一张卡片** —— 它决定第 3 步哪些站点跑得了。

**Files:**
- Modify: `gui/dist/index.html`

- [ ] **Step 1: 把内核卡片从参数页剪到基本设定页**

从参数页剪掉这整张卡片：

```html
      <div class="card">
        <h3>内核</h3>
        <div class="ch">一个内核 = 一组编译期宏。选哪个决定了哪些参数有意义、哪些输出变量写得出来。</div>
        <select class="select" id="kernel"></select>
        <p class="muted mini" id="kernelmeta">&nbsp;</p>
      </div>
```

粘到 basic 页的 `<p class="sub">` 之后，并把说明补上「它决定站点」这一层：

```html
      <div class="card">
        <h3>内核</h3>
        <div class="ch">一个内核 = 一组编译期宏。选哪个决定了哪些参数有意义、
          哪些输出变量写得出来，<b>以及下一步哪些站点跑得了</b> ——
          城市站必须走编进 <code>URBANON</code> 的那一套。</div>
        <select class="select" id="kernel"></select>
        <p class="muted mini" id="kernelmeta">&nbsp;</p>
      </div>
```

`#kernel` 与 `#kernelmeta` 两个 id 被 `runner.js`、`shell.js` 按 id 取，搬家不影响。

- [ ] **Step 2: 参数页的引导语改掉**

它还在说「选择编译内核」，而内核已经不在这一页。换成：

```html
      <p class="sub">按 CoLM 源码 namelist 的用途配置参数。
        <b>列表随第 2 步选的内核变</b> —— 换 default、urban、bgc 时可配置项会一起换；
        未设置项显示源码默认值，只读派生项排在各分节末尾。</p>
```

- [ ] **Step 3: 静态检查与跑一遍**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
bash $S/click.sh "站点"      # 进门
sleep 1
bash $S/ax.sh | grep -B3 -A3 "waterheat"
pkill -f "target/debug/colm-desktop-gui"
```

期望：内核下拉与 `generator_args` 那行出现在**第 2 步**（基本设定）页上，
不在参数页。左栏「内核」仍显示预设名（`renderSteps()` 按 id 读 `#kernel`，
不受搬家影响）。

- [ ] **Step 4: 提交**

```bash
git add gui/dist/index.html
git commit -m "内核卡片搬到第 2 步

它决定第 3 步哪些站点跑得了，是整条链最前面的那个决定，不是参数细调。

Confidence: high
Scope-risk: narrow
Tested: xtask check-gui; 辅助功能树读出内核下拉在第 2 步"
```

---

## Task 5: 算例目录与城市栅格搬到第 2 步

「算例放哪」现在在站点页，而它不依赖站点 —— 它是第 2 步该定的事。
城市栅格目录（`rawdata` / `runtime`）的显示条件也要改：从「勾中的站点里有
城市站」改成「**当前内核是 urban**」，因为内核现在排在站点前面。

**Files:**
- Create: `gui/dist/app/kernel.js`
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/runner.js`
- Modify: `gui/dist/app/sites.js`

- [ ] **Step 1: 新建 `gui/dist/app/kernel.js`**

判断「当前内核是不是 urban」这件事，`runner.js`（切内核时）与 `sites.js`
（标站点）都要问。**不能放进这两个中的任何一个** —— `runner.js` 已经
`import { renderCases, ensureCases } from './sites.js'`，反过来 import 就是
一个环，而 `check-gui` 会当场拦下（`sites ↔ results` 有前科）。

```js
//! 「当前选的是哪个内核、它编进了什么」。
//!
//! **单独一个模块，不是为了整齐。** `runner.js`（切内核时更新界面）与
//! `sites.js`（标出站点配不配得上内核）都要问同一个问题，而
//! `runner.js` 已经 import 了 `sites.js` —— 反过来 import 就是一个环。
//! ES module 的环**不报错**，只让某个 import 在运行时变成 `undefined`，
//! 那种故障比编译错误难查得多。`batch.js` 当初正是为同样的理由立的。
//!
//! 判据取 `generator_args` 而不是目录名或 preset 名：**目录名不是身份**，
//! 「这个内核到底编没编 URBAN」只有那一行宏组合说了算。

import { state } from './state.js';
import { $ } from './ui.js';

/** 下拉框现在选中的那个内核条目，没选中就是 `null`。 */
export function currentKernel() {
  return state.kernels.find(k => k.dir === $('kernel')?.value) ?? null;
}

/** 当前内核编进了 URBAN 吗？城市站只有它跑得了。 */
export function kernelIsUrban() {
  return !!currentKernel()?.generator_args?.includes('URBANON');
}
```

- [ ] **Step 2: `index.html` 把「算例放哪」卡片搬到基本设定**

从站点页剪掉 `<h3>算例放哪</h3>` 那整张 `<div class="card">`（含 `#root`、
`#rescan`、`#urbandirs`、`#rawdata`、`#runtime`），粘到 basic 页的内核卡片
**之后**，并把 `#urbandirs` 里的说明改成跟着内核走：

```html
      <div class="card">
        <h3>算例放哪</h3>
        <div class="ch">每个站点占一个子目录。默认放在站点数据旁边，可以改。
          下一步选中的每个站点都会在这里建一个。</div>
        <div class="browse">
          <input class="input" id="root" placeholder="…/colm-cases">
          <button class="btn-ghost pick" data-for="root">选择…</button>
          <button class="btn-ghost" id="rescan">重新扫描已建的</button>
        </div>
        <div id="urbandirs" hidden>
          <div class="expert-note">
            当前内核编进了 <b>URBAN</b>。城市算例必须给全球栅格目录 ——
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
      </div>
```

- [ ] **Step 3: `runner.js` 切内核时更新城市栅格目录的显示**

顶部 import 加：

```js
import { kernelIsUrban } from './kernel.js';
```

在 `showKernelMeta()` 函数末尾加一行 —— 它在初次渲染与每次切内核时都会被调，
正是这个开关该跟着变的时机：

```js
  // 城市栅格目录跟着内核走，不跟着站点走 —— 内核现在排在站点前面，
  // 到选站点时这两个目录必须已经填好。
  const ud = $('urbandirs');
  if (ud) ud.hidden = !kernelIsUrban();
```

- [ ] **Step 4: `sites.js` 去掉按站点切换 `#urbandirs` 的旧逻辑**

站点行勾选框的 `onchange` 里这一行删掉：

```js
      $('urbandirs').hidden = !state.sites.some(x => x.urban && state.picked.has(x.site_file));
```

删掉之后那个 `onchange` 是：

```js
    cb.onchange = () => {
      if (cb.checked) state.picked.add(s.site_file); else state.picked.delete(s.site_file);
      renderDataFoot();
      renderSteps();   // 勾选改变了「在配几个」，左栏要立刻跟上
    };
```

- [ ] **Step 5: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/kernel.js && node --check gui/dist/app/runner.js && node --check gui/dist/app/sites.js
cargo run -p xtask -- check-gui
```

期望：无语法错；check-gui 全绿，**尤其不报 import cycle** —— 新模块
`kernel.js` 只依赖 `state.js` 与 `ui.js`，谁都可以 import 它。

- [ ] **Step 6: 跑起来验**

```bash
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
bash $S/click.sh "站点"; sleep 1
bash $S/ax.sh | grep "算例放哪\|rawdata\|URBAN"
pkill -f "target/debug/colm-desktop-gui"
```

期望：「算例放哪」出现在**第 2 步**。这台机器只编了一个非 urban 内核，
所以 `#urbandirs` 应该是**隐藏的** —— grep 不到 `rawdata` 才是对的。

若手边编了 urban 内核（`./oracle/scripts/build_kernel.sh urban`），切到它
应该让那两个目录框出现。没编就跳过这一条，在报告里写明。

- [ ] **Step 7: 提交**

```bash
git add gui/dist/app/kernel.js gui/dist/app/runner.js gui/dist/app/sites.js gui/dist/index.html
git commit -m "算例目录与城市栅格跟着内核走到第 2 步

算例放哪不依赖站点；城市栅格目录的条件从「勾中的站点里有城市站」改成
「当前内核编进了 URBANON」—— 内核现在排在站点前面，到选站时这两个目录
必须已经填好。

判据取 generator_args 而不是目录名：目录名不是身份。
kernel.js 单独立一个模块是因为 runner 已经 import 了 sites，
反过来 import 就是一个环，check-gui 会当场拦下。

Confidence: high
Scope-risk: moderate
Tested: node --check; xtask check-gui 无成环; 第 2 步读出算例目录卡片"
```

---

## Task 6: 建算例留在站点页，出口交回通用按钮

**Files:**
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/sites.js`

- [ ] **Step 1: 站点页去掉 `data-own-foot`**

```html
    <section class="page" data-step="sites" data-own-foot hidden>
```

改成：

```html
    <section class="page" data-step="sites" hidden>
```

- [ ] **Step 2: `index.html` 把 `#datafoot` 改名并搬进站点卡片**

站点页末尾那一行连同它上面的注释：

```html
      <!-- 出口在这里，不在站点列表下面：按下去会**在上面那个目录里**建算例，
           摆在设定那个目录之前，等于让人先按确定再想产物放哪。 -->
      <div class="foot" id="datafoot"></div>
```

删掉。改为在站点列表那张卡片里、`#pickinfo` 那一行之后插入：

```html
        <!-- 建算例的按钮。**它是卡片内的动作，不是页面出口** ——
             页面出口是底部通用的「下一步：参数 →」。算例目录在第 2 步
             已经定好了，所以这里按下去不会再有「产物放哪」的疑问。 -->
        <div id="makecase" style="margin-top:12px"></div>
```

顺带把那段提示改掉（它还说「按下面的确定」，而按钮现在在上面）：

```html
        <p class="muted mini" style="margin-top:8px">
          点行只是选中，<b>不会动任何文件</b>。按「建算例」才会真的建 ——
          那一步要读站点文件与强迫场并写出补齐后的 <code>site.nc</code>。</p>
```

- [ ] **Step 3: `sites.js` 把 `renderDataFoot` 更名 `renderMakeCase`**

step id 已经从 `data` 改成 `sites`，留着 `data` 前缀会让人去找一个不存在的步骤。

整个函数替换成：

```js
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
```

**全仓库替换所有调用点**：

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
grep -rn "renderDataFoot\|datafoot" gui/dist/
```

把每一处 `renderDataFoot` 改成 `renderMakeCase`、`$('datafoot')` 改成
`$('makecase')`。改完再 grep 一次，期望**零结果**。

- [ ] **Step 4: `sites.js` 的 `confirmSelection` 不再翻页**

建完算例停在这一页，让用户看见列表里多出来的那几行。翻页是底部出口的事。

删掉 `go('params');` 这一行。`confirmSelection` 的 `try` 块末尾变成：

```js
    // **走 selectCase，不要只设 state.selected。** 那里还要把 case.nml 读进来、
    // 查出 CoLM 不认识的字段、刷新参数表与预设 —— 只设一个字段的话，
    // 参数页会是空的，而空页面不会报错，只是什么都没有。实测踩过。
    await selectCase(made[0]);
```

`go` 的 import 现在没有消费者了。把顶部：

```js
import { go, renderSteps, setStatus } from './shell.js';
```

改成：

```js
import { renderSteps, setStatus } from './shell.js';
```

**别忘这一步** —— `check-gui` 验的是 import 解析得了，一个没人用的 import
它不报，留着只是给下一个人添乱。

- [ ] **Step 5: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/sites.js
grep -rn "renderDataFoot\|datafoot" gui/dist/ || echo "改名干净"
cargo run -p xtask -- check-gui
```

期望：无语法错；grep 打印「改名干净」；check-gui 全绿。

- [ ] **Step 6: 跑起来走一遍**

```bash
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
bash $S/click.sh "站点"; sleep 1          # 进门
bash $S/click.sh "下一步"; sleep 1        # 第 1 步 → 第 2 步
bash $S/click.sh "下一步"; sleep 1        # 第 2 步 → 第 3 步
bash $S/click.sh "用自带的示例站点"; sleep 4
bash $S/clicktext.sh "CN-Cng"; sleep 1
bash $S/ax.sh | grep "建算例\|下一步"
bash $S/click.sh "建算例"; sleep 6
bash $S/ax.sh | grep "已为 CN-Cng 建好算例\|下一步：参数"
pkill -f "target/debug/colm-desktop-gui"
```

期望：站点页有两个按钮 —— 卡片内的「建算例：CN-Cng」和页底的
「下一步：参数 →」（建算例之前它灰着写「先在第 3 步建一个算例」）。
按下建算例之后状态栏读出「已为 CN-Cng 建好算例」，页底按钮变成可点的
「下一步：参数 →」。

- [ ] **Step 7: 提交**

```bash
git add gui/dist/index.html gui/dist/app/sites.js
git commit -m "建算例成为站点页里的动作，出口交回通用按钮

算例目录在第 2 步已经定好，所以这里按下去不会再有「产物放哪」的疑问 ——
原来出口摆在设定目录之前，等于让人先按确定再想产物放哪。

renderDataFoot 更名 renderMakeCase、#datafoot 更名 #makecase：step id 已经
从 data 改成 sites，留着 data 前缀会让人去找一个不存在的步骤。

Constraint: 建算例按钮不是页面出口，两个长得像的按钮不能摆在一起
Confidence: high
Scope-risk: moderate
Tested: node --check; grep 确认改名干净; xtask check-gui; 走通选站→建算例"
```

---

## Task 6b: 修掉审查抓到的两个真故障

`0b97703` 的审查在隔离 worktree 里配假后端实跑，抓到一个 Critical、一个
Important、一个 Minor。**都是真的**，且第一个会让新克隆的开发树整条流水线锁死。

**Files:**
- Modify: `gui/dist/app/shell.js`
- Modify: `gui/dist/app/sites.js`

- [ ] **Step 1（Critical）: 没有内核时不要把第 3 步锁死**

`STEPS` 里 `sites` 的门槛现在读 `document.getElementById('kernel')?.value`。
没有内核时 `runner.js` 往下拉框里放的占位 option **`value` 就是空串**，
于是门槛关着；而第 2 步那一页此时也没得选 —— 文案叫人「去第 2 步选一个
内核」，去了发现只有一句「没有找到内核」。**从此扫站点都进不去。**

触发条件不是假想：`.gitignore` 第 7 行忽略 `/kernels/`，**新克隆的开发树
默认没有这个目录**；打包版也可能空 —— `Kernel::open` 校验三个二进制的
sha256，不过的会被静默丢掉。

另外原注释里「下拉框里没选中任何一项时 `.value` 是空串」这个理由与实际
行为对不上：单选 `<select>` 只要有 option 就必然选中一项，用户没有「不选」
这个动作。门槛实际判的是**有没有可用的内核**。

把那一项换成：

```js
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
```

读 `state.kernels.length` 而不是查 DOM：`shell.js` 不该反查一个住在别的页
上的 DOM id，而且那个数组正是 `list_kernels` 的直接结果。

- [ ] **Step 2（Important）: 左栏的数字别被短路成旧批次**

`renderSteps()` 里 `const n = state.batch.length || state.picked.size;`
在建过算例之后会短路成旧数字。实测复现：勾 ST-000 → 建算例（`batch` 长度 1）
→ 回站点页 → 全选 20 个，此刻同一屏上三个数字打架：

| 位置 | 显示 |
|---|---|
| 页内 `#pickinfo` | 已勾 20 个 |
| 建算例按钮 | 选中的 **20** 个站点 |
| 左栏「已选站点」 | `ST-000`（一个数都没有） |

这正是 `3d49d8b` 要修的那个毛病的翻版。而且勾选现在每次都真的重绘左栏，
**固执地显示旧数字比干脆不刷新更像在骗人**。

把 `renderSteps()` 里取 `n` 与 `one` 的那几行换成：

```js
  // **站在站点页时以勾选为准，往后以批次为准。** 这两个数在建过算例之后
  // 会同时有值且不相等：勾了 20 个、而批次里还是上次建的那 1 个。
  // 写成 `batch.length || picked.size` 的话短路会让左栏固执地显示旧数字,
  // 而勾选现在每次都重绘左栏 —— 显示旧数字比干脆不刷新更像在骗人。实测踩过。
  const onSites = state.step === 'sites';
  const n = onSites
    ? (state.picked.size || (state.pickedSite ? 1 : 0))
    : (state.batch.length || state.picked.size);
  const one = onSites
    ? (state.pickedSite?.name
       ?? state.sites.find(x => state.picked.has(x.site_file))?.name)
    : (state.selected?.name ?? state.pickedSite?.name);
  $('estSite').textContent = n > 1 ? `${one ?? '—'} 等 ${n} 个` : (one ?? '—');
```

- [ ] **Step 3（Minor）: 指错步骤的那句提示**

`sites.js` 的 `ensureCase()` 里：

```js
  if (!root) { setStatus('先指定算例放哪（第 1 步下面那张卡片）'); return null; }
```

`#root` 卡片已经搬到基本设定页，那是**第 2 步**。改成：

```js
  if (!root) { setStatus('先指定算例放哪（第 2 步「算例放哪」那张卡片）'); return null; }
```

- [ ] **Step 4: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/shell.js && node --check gui/dist/app/sites.js
cargo run -p xtask -- check-gui
```

- [ ] **Step 5: 跑起来验两个场景**

```bash
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
bash $S/click.sh "站点"; sleep 1
bash $S/ax.sh | grep "站点\|先在第\|还没有可用的内核"
pkill -f "target/debug/colm-desktop-gui"
```

期望（这台机器有内核）：第 3 步**不灰**，读不到「还没有可用的内核」。

再验没有内核的那条路 —— **这是本任务的重点**，把内核目录临时藏起来：

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
mv kernels kernels-hidden
cd gui/src-tauri
./target/debug/colm-desktop-gui > /tmp/nokernel.log 2>&1 &
sleep 4
bash $S/click.sh "站点"; sleep 1
bash $S/ax.sh | grep "还没有可用的内核\|没有找到内核"
pkill -f "target/debug/colm-desktop-gui"
mv kernels-hidden kernels
```

期望：第 3 步灰着并写「还没有可用的内核 —— 先构建 kernels/…」，
**而不是**「先在第 2 步选一个内核」。

**别忘了把 `kernels` 目录改回来。** 它是 gitignore 的构建产物，
丢了要重编一次 Fortran。

- [ ] **Step 6: 提交**

```bash
git add gui/dist/app/shell.js gui/dist/app/sites.js
git commit -m "没有内核时别把第 3 步锁死

门槛原来读 #kernel 的 value，而没有内核时那个占位 option 的 value 是空串,
于是第 3 步灰着叫人「去第 2 步选一个内核」—— 去了只有一句「没有找到内核」。
.gitignore 忽略 /kernels/，新克隆的开发树默认就是这个状态，整条流水线锁死。
改判 state.kernels.length，文案指向真正的出路。

左栏的数字也修了：batch.length || picked.size 在建过算例之后会短路成旧
批次，勾 20 个而左栏显示上次建的那 1 个 —— 正是 3d49d8b 要修的那个毛病的
翻版，且勾选现在每次都重绘左栏，显示旧数字比不刷新更像在骗人。

Constraint: shell.js 不反查住在别的页上的 DOM id
Confidence: high
Scope-risk: narrow
Directive: 门槛文案必须指向真正的出路，不能指向一个办不到这件事的页面
Tested: node --check; xtask check-gui; 藏掉 kernels/ 实测新文案"
```

---

## Task 7: 时间与预热搬到参数页

它读 `DEF_simulation_time%*`，而那份 `case.nml` 是建算例产出的 ——
所以它必须排在建算例之后，且跟它实际写进去的那份 namelist 在同一页。

**Files:**
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/timing.js`

- [ ] **Step 1: `#timing` 容器从运行页剪到参数页**

从运行页剪掉这一行：

```html
      <div id="timing"></div>
```

粘到参数页 `<p class="sub">` 之后、`<h3>namelist 参数</h3>` 那张卡片之前。

- [ ] **Step 2: 运行页的引导语改掉**

```html
      <p class="sub">设置输出，然后依次运行 <code>mksrfdata</code>、<code>mkinidata</code>、
        <code>colm</code>。<b>输入没变的阶段会按输入指纹跳过</b>。
        时间与预热在第 4 步。</p>
```

- [ ] **Step 3: `timing.js` 的模块注释改掉**

它现在说「所以它现在放在运行页，紧挨着输出设置」，那已经不成立。
把开头整个注释块换成：

```js
//! 「时间与预热」卡片。
//!
//! 这两样都在 737 个字段的表里躺着（`DEF_simulation_time%*`），但**躺在
//! 表里等于不存在** —— 实测：用户翻完参数页，说"我没有看到 spin-up 的选项"。
//! 一个决定输出从哪天开始的开关，不该和 `DEF_USE_SNICAR` 长得一样。
//! 所以它有自己的卡片，摆在参数页最上面。
//!
//! **为什么是参数页而不是更靠前的一步**：它读的是 case.nml，而那份文件是
//! 建算例产出的 —— 建算例又要先选站点。依赖链把它顶到了这里，
//! 而这里正好也是它实际写进去的那份 namelist 所在的页。
//!
//! **刷新时机挂在 `renderFields()` 上**（`params.js` 里 `await renderTiming()`）。
//! 那是选中算例之后必经的一次渲染。
```

- [ ] **Step 4: 静态检查与跑一遍**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/timing.js
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
```

走到第 4 步（要先建好算例），然后：

```bash
bash $S/ax.sh | grep "spin-up\|时间与预热\|模拟"
pkill -f "target/debug/colm-desktop-gui"
```

期望：「时间与预热（spin-up）」卡片出现在**第 4 步**，且表格里有真实的
时间范围（不是空的）—— 空的说明 `renderTiming()` 没被调到。

顺带确认结果页那两句「与第 4 步里的模型预热」现在**说对了** ——
Task 2c 特意没改它们，等的就是这一步。

- [ ] **Step 5: 提交**

```bash
git add gui/dist/index.html gui/dist/app/timing.js
git commit -m "时间与预热归入参数页

它读 case.nml，而那是建算例产出的 —— 依赖链把它顶到建算例之后，
而参数页正好也是它实际写进去的那份 namelist 所在的页。

结果页那两句「与第 4 步里的模型预热」到这一步才真正成立。

Confidence: high
Scope-risk: narrow
Tested: node --check; xtask check-gui; 第 4 步读出真实时间范围"
```

---

## Task 8: 站点行说出配不配得上当前内核

**标出来，不要过滤掉。** 内核是 `urban` 时把非城市站藏起来（或反过来）
会让人以为「扫出来就这么多」—— 而这个程序自己立过规矩：静默跳过与静默
失败在界面上长得一样。

**Files:**
- Modify: `gui/dist/app/sites.js`

- [ ] **Step 1: import 内核判据**

`sites.js` 顶部 import 区加：

```js
import { kernelIsUrban } from './kernel.js';
```

`kernel.js` 只依赖 `state.js` 与 `ui.js`，不会成环。

- [ ] **Step 2: `renderSites()` 里给站点行加匹配标签**

找到组装 `tags` 的那一段：

```js
    const tags = [];
    // 算例状态排在最前：它是**这一行现在处在流水线哪一段**，
    // 比经纬度重要得多。原来这个信息藏在另一个列表里。
    const c = state.cases.find(x => x.name === s.name);
    if (c) tags.push(c.has_history ? '已跑过' : '已建算例');
    if (s.urban) tags.push('城市');
    if (!s.met_file) tags.push('无强迫场');
    if (!s.obs_file) tags.push('无观测');
    if (s.problem) tags.push('读不了');
```

在 `if (s.urban) tags.push('城市');` 之后插入：

```js
    // **内核决定这个站点跑不跑得了。** 城市站要 URBANON 编进去的那一套，
    // 非城市站用 urban 内核跑出来的东西也不对。标出来而不是藏起来 ——
    // 过滤掉会让人以为「扫出来就这么多」。
    if (s.urban && !urbanKernel) tags.push('要 urban 内核');
    if (!s.urban && urbanKernel) tags.push('要非 urban 内核');
```

在 `for (const s of state.sites) {` 这一行**之前**取一次内核判据
（循环里每行都调一次是白费）：

```js
  const urbanKernel = kernelIsUrban();
```

- [ ] **Step 3: 页顶摘要说出当前内核**

找到 `$('sitesummary').textContent = ...` 那一段，把它换成：

```js
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
```

顶部 import 相应改成：

```js
import { currentKernel, kernelIsUrban } from './kernel.js';
```

- [ ] **Step 4: 切内核时站点列表要跟着重画**

`runner.js` 的 `showKernelMeta()` 末尾（Task 5 加的那两行之后）再加：

```js
  // 站点行上的「要 urban 内核」标记跟着内核变。不重画的话，切了内核
  // 站点列表还标着上一个内核的判断，而那正是最容易看错的一处。
  if (state.sites.length) renderSites();
```

`runner.js` 已经 `import { renderCases, ensureCases } from './sites.js'`，
把 `renderSites` 加进那一行即可 —— 但 `renderSites` 现在不是 export 的，
先在 `sites.js` 里给它加 `export`：

```js
export function renderSites(r = {}) {
```

- [ ] **Step 5: 静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/sites.js && node --check gui/dist/app/runner.js
cargo run -p xtask -- check-gui
```

期望：无语法错；check-gui 全绿且**不报 import cycle**。

- [ ] **Step 6: 跑起来验**

```bash
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
bash $S/click.sh "站点"; sleep 1
```

走到第 3 步、扫出自带示例，然后：

```bash
bash $S/ax.sh | grep "个站点\|当前内核\|要 urban"
pkill -f "target/debug/colm-desktop-gui"
```

期望：摘要行读出 `1 个站点 · 当前内核 waterheat`（Task 12 改名后是
`default`）。CN-Cng 不是城市站、当前内核也不是 urban，所以**不该**出现
「要 urban 内核」标记。

若编了 urban 内核，切到它之后 CN-Cng 那一行应该出现「要非 urban 内核」，
摘要行出现「其中 1 个跑不了」。没编就跳过，在报告里写明。

- [ ] **Step 7: 提交**

```bash
git add gui/dist/app/sites.js gui/dist/app/runner.js
git commit -m "站点行说出配不配得上当前内核

标出来而不是过滤掉 —— 藏起来会让人以为扫出来就这么多，而这个程序
自己立过规矩：静默跳过与静默失败在界面上长得一样。

摘要行也报当前内核：它是第 2 步定的，到这一页决定哪些行能用，
让人回上一步去看自己选了什么等于把上下文丢了。

Constraint: 判据取 generator_args 含 URBANON，不看目录名
Rejected: 按内核过滤站点列表 | 静默跳过与静默失败长得一样
Confidence: high
Scope-risk: narrow
Tested: node --check; xtask check-gui 无成环; 摘要行读出当前内核"
```

---

## Task 8c: 城市算例脱离全球栅格

**这是整个计划里唯一改引擎行为的任务。** 前面所有任务都只动界面。

城市算例现在必须给 `--rawdata` / `--runtime`，而那套栅格实测 698 GB。
但**路径是现成的** —— CoLM 本来就先看 `site.nc` 再回落栅格：

```fortran
! MOD_SingleSrfdata.F90:1584
u_site_utype = ncio_var_exist(fsrfdata,'LCZ_DOM')
   CALL ncio_read_serial (fsrfdata, 'LCZ_DOM', SITE_urbtyp)      ! 先看 site.nc
   ...
   CALL read_point_5x5_var_2d_int32 (gridupatch, dir_5x5, ...)   ! 没有才去栅格
```

`LUCY_ID` 同样（1856–1864 行）。而 `site::fill` 对自然站点做的正是这件事：
把 12 个字段按**站点文件 → 栅格 → 模块默认值**三级回落写进 `site.nc`，
每个带一条 `source` 属性注明出处。

**城市站没走这条路** —— `prepare_urban` 只把 `ground_height` 抄成
`elevation` 就结束了，然后 `colm-case` 把三个 `USE_SITE_*` 设成 `.false.`
把 CoLM 推去读栅格。

**Files:**
- Modify: `crates/colm-srfdata/src/site.rs`
- Modify: `crates/colm-case/src/build.rs`
- Modify: `crates/colm-cli/src/main.rs`

- [ ] **Step 1: 先实测「不给栅格会缺什么」**

**别照着推理改。** 先把三个 `USE_SITE_*` 手工改成 `.true.`、
`DEF_dir_rawdata` 指向不存在的目录，跑一次 AU-Preston，看 CoLM 到底
在哪一步停、报什么。

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
U=/Users/zhongwangwei/Desktop/colm-rust/Urban-PLUMBER
T=/tmp/urban-probe && rm -rf $T
./target/debug/colm-cli new --site "$U/Sitedata/AU-Preston_site_v1.nc" \
  --out $T --name AU-Preston --rawdata /nonexistent --runtime /nonexistent
sed -i '' 's/USE_SITE_lakedepth *= *\.false\./USE_SITE_lakedepth = .true./;
           s/USE_SITE_soilreflectance *= *\.false\./USE_SITE_soilreflectance = .true./;
           s/USE_SITE_soilparameters *= *\.false\./USE_SITE_soilparameters = .true./' $T/case.nml
./target/debug/colm-cli run $T --kernel kernels/waterheat 2>&1 | tail -30
```

（用 `waterheat` 只是为了看 `mksrfdata` 在读栅格前后停在哪；真正跑城市
要 urban 内核。若报「内核不匹配」之类，改用
`./oracle/scripts/build_kernel.sh urban` 先编一个。）

**把实际报错抄进报告。** 后面几步补哪些字段，以这次实测为准，
不以下面的清单为准 —— 清单是按源码推的，可能不全。

- [ ] **Step 2: `prepare_urban` 补齐缺的字段**

`crates/colm-srfdata/src/site.rs` 里 `prepare_urban` 现在只补 `elevation`。
让它复用 `fill` 已有的三级回落，补上 Step 1 实测缺的那些。至少包括：

| 变量 | 回落来源 |
|---|---|
| `lakedepth` | `MOD_SingleSrfdata.F90:47` 模块默认值 |
| `soil_s_v_alb` `soil_d_v_alb` `soil_s_n_alb` `soil_d_n_alb` | 同 `fill` 的土壤反照率回落 |
| `soil_texture` `soil_vf_clay` `soil_wf_clay` `soil_wf_om` | 同 `fill` 的土壤参数回落 |
| `elvstd` `sloperatio` | 只服务已关闭的降尺度，模块默认值即可 |
| **`LCZ_DOM`** | **默认 6**，见下 |
| `LUCY_ID` | Step 1 实测确认要不要；要的话默认值也在那一步定 |

**`LCZ_DOM` 默认取 6（开阔低层建筑）的依据**，AU-Preston 实测形态学量：

```
building_mean_height      6.4 m     LCZ 6 定义 3–10 m        ✓
canyon_height_width_ratio 0.42      LCZ 6 定义 0.3–0.75      ✓
tree + grass              0.375     LCZ 6 透水面 30–60%      ✓
roof_area_fraction        0.445
```

按 Stewart & Oke (2012)，这是墨尔本郊区住宅区的典型形态。
**每个补进去的值都要带 `source` 属性说明它是量出来的还是假设的** ——
这是 `site.rs` 模块注释里立的规矩。`LCZ_DOM` 的 source 写
`"assumed: LCZ 6 open low-rise (Stewart & Oke 2012), from site morphology"`。

- [ ] **Step 3: `colm-case` 不再把三项设成 `.false.`**

`crates/colm-case/src/build.rs` 第 230–236 行那个循环删掉，
并把上面那段注释改成说明新的事实：

```rust
        // 这三项保持默认的 .true.（「站点文件里有，用它」）—— `prepare_urban`
        // 现在会把它们按**站点文件 → 栅格 → 模块默认值**三级回落写进
        // site.nc，和自然站点走的是同一条路。
        //
        // 改成 .false. 会把 CoLM 推去读全球栅格，而那套数据实测 698 GB。
        // 城市站点文件里确实没有这三样（25 个变量全是形态学量），
        // 但「站点文件里没有」与「必须去栅格取」之间隔着 site::fill。
```

- [ ] **Step 4: `colm-cli new` 的两个目录改回可选**

`crates/colm-cli/src/main.rs` 第 510–521 行那个 `if urban { ... }` 分支：
给了 `--rawdata` 就用（栅格优先，三级回落的第二级），没给就和自然站点
一样指向不存在的目录。把那段注释改成：

```rust
    // 全球栅格目录。**给了就用，没给就回落** —— `site::fill` /
    // `prepare_urban` 已经把该有的都写进 site.nc 了，跑通了就证明没读栅格。
    //
    // 城市算例曾经必填这两个：站点文件里只有形态学量，土壤剖面、湖深、
    // 反照率、LCZ 分类都得从别处来。但 CoLM 本来就先看 site.nc 再回落栅格
    // （MOD_SingleSrfdata.F90:1584 的 LCZ_DOM、1856 的 LUCY_ID），
    // 所以那些值写进 site.nc 就够了，698 GB 的栅格不再是门槛。
```

- [ ] **Step 5: 实测 AU-Preston 不给栅格跑完三段**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
./oracle/scripts/build_kernel.sh urban        # 还没编的话
U=/Users/zhongwangwei/Desktop/colm-rust/Urban-PLUMBER
T=/tmp/urban-run && rm -rf $T
cargo build -p colm-cli
./target/debug/colm-cli new --site "$U/Sitedata/AU-Preston_site_v1.nc" \
  --out $T --name AU-Preston --start 1993-01-01 --end 1993-01-11
grep -E "USE_SITE_(lakedepth|soilreflectance|soilparameters)|DEF_dir_rawdata|DEF_URBAN" $T/case.nml
./target/debug/colm-cli run $T --kernel kernels/urban 2>&1 | tail -20
```

期望：

- `case.nml` 里三个 `USE_SITE_*` **不出现**（保持默认 `.true.`），
  `DEF_dir_rawdata` 指向 `$T/rawdata_unused/`
- 三段全 `ok`
- 产出 history 文件

README 里 URBAN 那一节的验收基准是「AU-Preston，1993-01-01 至 01-11，
1800 s 步长，三段全 ok，264 条小时记录，`f_tref` 峰值 312 K」——
**对照它**。跑出来的记录条数、量级对不上就报出来，不要含糊过去。

- [ ] **Step 6: 自然站点不能被弄坏**

改的是 `site.rs` 与 `build.rs` 的共享路径，必须确认 CN-Cng 没受影响：

```bash
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo test --workspace 2>&1 | tail -10
cargo run -p oracle --bin golden-run -- CN-Cng 2>&1 | tail -5
cargo run -q -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

期望**逐字**：

```
identical: 129 variables, 10 dimensions (ignoring ["create_time"])
```

对不上就 **BLOCKED**。城市站能跑了但自然站点变了，是净亏损。

- [ ] **Step 7: 提交**

```bash
git add crates/colm-srfdata/src/site.rs crates/colm-case/src/build.rs crates/colm-cli/src/main.rs
git commit -m "城市算例脱离全球栅格

CoLM 本来就先看 site.nc 再回落栅格（MOD_SingleSrfdata.F90:1584 的
LCZ_DOM、1856 的 LUCY_ID），而 site::fill 对自然站点做的正是把值写进
site.nc。城市站没走这条路 —— prepare_urban 只抄了 elevation，然后
colm-case 把三个 USE_SITE_* 设成 .false. 把 CoLM 推去读那 698 GB 栅格。

现在 prepare_urban 也走三级回落（站点文件 → 栅格 → 模块默认值），
LCZ_DOM 默认 6（开阔低层建筑）—— AU-Preston 实测建筑高 6.4 m、
H/W 0.42、透水面 37.5%，按 Stewart & Oke (2012) 正是这一类。

Constraint: 每个补进去的值都带 source 属性说明是量出来的还是假设的
Constraint: 自然站点的黄金比对必须仍然 identical
Confidence: medium
Scope-risk: broad
Directive: 改引擎输入前先实测「不给栅格到底缺什么」，别照着源码推
Tested: AU-Preston 不给栅格三段全 ok; CN-Cng 黄金比对 identical"
```

---

## Task 8b: 自带一个城市示例站点

现在自带的示例只有 CN-Cng（自然站点）。选了 `urban` 内核的人手上没有任何
能试的东西 —— 而 `examples/README.md` 当初的理由是「城市算例不能自带：
栅格实测 698 GB」。

**站点文件能自带，栅格不能** —— 但 Task 8c 之后城市算例已经不需要栅格了
（值写进 `site.nc`，CoLM 先看那里）。所以这一步只管把站点文件发出去；
下面那些「栅格门槛」的界面提示**以 Task 8c 的实测结果为准**：
真的不需要栅格了就不要加那些提示，加了就是骗人。

**实测的门槛**（本机跑过）：

```
$ colm-cli new --site .../AU-Preston_site_v1.nc --out ... --name AU-Preston
site: urban-shaped; elevation 93 m taken from ground_height so CoLM never needs the 7 GB elevation.nc
Error: an urban case needs --rawdata: the site file carries only morphology,
       so soil, lake depth, albedo and the LCZ class all come from the global grid
```

后端已经拦得很清楚。要做的是**在按下去之前就说**——等报错才知道，人已经
以为自己走通了。

**站点选 AU-Preston**：README「URBAN 是唯一必须带全球栅格跑的预设」那一节
的验收就是拿它做的（1993-01-01 至 01-11，三段全 ok，264 条小时记录，
`f_tref` 峰值 312 K 对得上墨尔本一月）。三件套 4.5 MB。

**Files:**
- Create: `examples/Sitedata/AU-Preston_site_v1.nc` 等三个文件
- Modify: `examples/README.md`
- Modify: `gui/src-tauri/src/example.rs`（模块注释）
- Modify: `gui/dist/app/sites.js`
- Modify: `gui/dist/app/runner.js`

- [ ] **Step 1: 把三个文件复制进 `examples/`**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
U=/Users/zhongwangwei/Desktop/colm-rust/Urban-PLUMBER
cp "$U/Sitedata/AU-Preston_site_v1.nc"   examples/Sitedata/
cp "$U/Forcing/AU-Preston"*.nc           examples/Forcing/
cp "$U/Observation/AU-Preston"*.nc       examples/Observation/
ls -la examples/*/ | grep -i preston
du -sh examples/
```

期望：三个目录各多一个 AU-Preston 文件，`examples/` 总计约 7.6 MB。

`.gitattributes` 里 `*.nc  binary`，不走 LFS，直接入库。

- [ ] **Step 2: 确认扫描能同时认出两个**

```bash
./target/debug/colm-cli scan --dir examples/Sitedata --quick 1 | python3 -c "
import json,sys
for s in json.load(sys.stdin):
    print(s['name'], 'urban=' + str(s['urban']), 'met=' + str(bool(s['met_file'])), 'landtype=' + str(s['landtype']))
"
```

**已在 `/tmp` 里预演过**（把两套文件拷进同一组目录再扫），实测输出：

```
AU-Preston   urban=True   met=有  obs=有  landtype=None
CN-Cng       urban=False  met=有  obs=有  landtype=10
总体积 7.6 MB
```

`AU-Preston` 的 `landtype` 是 `None` —— 城市站点文件没有 `IGBP_classification`，
而「没有它就是城市站」正是 `scan` 认城市站的判据（README「站点库」一节）。
命名约定也照常生效，两个站点各自的强迫场与观测都配对上了。

**两套都装、都显示，不按内核过滤。** 这与 Task 8 立的规矩一致：标出来而
不是藏起来。`install_example` 是整目录复制，不用改后端逻辑。

- [ ] **Step 3: 城市站行标出「需全球栅格」**

`gui/dist/app/sites.js` 的 `renderSites()` 里，Task 8 加的那两行标记之后再加：

```js
    // 城市站还要全球栅格，而站点文件里只有形态学量 —— 土壤剖面、湖深、
    // 反照率、LCZ 分类都得从栅格取。**在按下建算例之前说**：
    // colm-cli new 会直接报错，但那时人已经以为自己走通了。
    if (s.urban && !hasRaster()) tags.push('需全球栅格');
```

在文件里加一个小助手（放在 `renderSites` 之前）：

```js
/** 城市栅格的两个目录都填了吗。两个都要 —— `<rawdata>/urban/` 给
 *  mksrfdata，`<runtime>/urban/` 给 mkinidata，路径由两处不同的代码各拼各的。 */
function hasRaster() {
  return !!($('rawdata').value.trim() && $('runtime').value.trim());
}
```

- [ ] **Step 4: 建算例按钮在缺栅格时说清楚**

`renderMakeCase()` 里，在 `b.disabled = !n && !one;` **之后**插入：

```js
  // 选中的里面有城市站而栅格目录没填 —— 按下去必然是
  // 「an urban case needs --rawdata」。**在按下去之前说**，
  // 并且指明去哪儿填（第 2 步），而不是让人对着一句英文报错发呆。
  const target = n
    ? state.sites.filter(x => state.picked.has(x.site_file))
    : (one ? [one] : []);
  if (target.some(x => x.urban) && !hasRaster()) {
    b.textContent = '城市站还需要全球栅格目录 —— 去第 2 步填 rawdata 与 runtime';
    b.disabled = true;
  }
```

- [ ] **Step 5: 示例按钮的文案随内核变**

`gui/dist/app/runner.js` 的 `showKernelMeta()` 末尾（Task 8 加的 `renderSites()`
那一行之后）再加：

```js
  // 自带示例现在有两个：CN-Cng（自然）与 AU-Preston（城市）。两个都会装，
  // 但按钮说出**当前内核下哪个用得上** —— 选了 urban 却看到「CN-Cng」，
  // 人会以为自带的这份跟自己没关系。
  const ex = $('use-example');
  if (ex) {
    ex.textContent = kernelIsUrban()
      ? '用自带的示例站点（城市站 AU-Preston）'
      : '用自带的示例站点（CN-Cng）';
  }
```

`index.html` 里那句静态说明也要跟着改成两个都提：

```html
          <span class="muted mini">CN-Cng 内蒙古草地、AU-Preston 墨尔本城市站，
            装在程序里不用另外下数据；<b>城市站还需要自备全球栅格</b></span>
```

- [ ] **Step 6: 更新 `examples/README.md`**

那一节「为什么只有一个，而且是自然站点」的立场变了。整节换成：

```markdown
## 为什么是这两个站点

CN-Cng 就是黄金回归用的那个站点（`oracle/cases/CN-Cng/`）。示例与测试覆盖
的是同一份数据 —— 示例跑不通，回归测试会先一步红。

AU-Preston 是 URBAN 预设验收用的那个（README「URBAN 是唯一必须带全球栅格
跑的预设」一节）。选了 `urban` 内核的人手上得有个能试的东西。

## 城市站自带的是站点文件，不是栅格

**AU-Preston 装完不能直接跑。** 城市算例的土壤剖面、湖深、土壤反照率、
LCZ 分类都只能从全球栅格取，而那套数据实测 698 GB —— 装不进任何安装包。
站点文件里只有 25 个形态学量。

所以 `colm-cli new` 认出城市站点文件之后，`--rawdata` 与 `--runtime`
变成必填，缺了直接报错：

```
an urban case needs --rawdata: the site file carries only morphology,
so soil, lake depth, albedo and the LCZ class all come from the global grid
```

界面在**按下建算例之前**就把这条说出来：站点行标「需全球栅格」，
按钮变成「城市站还需要全球栅格目录 —— 去第 2 步填 rawdata 与 runtime」
并置灰。**一个能点但必然失败的入口比一个灰着的更糟。**
```

- [ ] **Step 7: 更新 `example.rs` 的模块注释**

开头那句「**只有一个，而且是自然站点。**」已经不成立。整个 `//!` 块换成：

```rust
//! 自带的示例站点。
//!
//! 一个刚装好程序的人手上**没有任何数据**。PLUMBER2 要注册才能下载、
//! 几十 GB，而在拿到数据之前他连"这程序能不能用"都判断不了。
//! 所以装两个站点进去：CN-Cng（内蒙古草地，2008–2009）与 AU-Preston
//! （墨尔本城市站）—— 后者是给选了 `urban` 内核的人准备的。
//!
//! **城市那个自带的是站点文件，不是栅格。** 土壤剖面、湖深、土壤反照率、
//! LCZ 分类都只能从全球栅格取，实测那套数据 698 GB，装不进任何安装包。
//! 所以 AU-Preston 装完**不能直接跑**，界面会在按下建算例之前说明这一点。
//! CN-Cng 则装完就能跑通建算例 → 三段运行 → 与观测比对的完整流程。
```

- [ ] **Step 8: 静态检查与跑一遍**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/sites.js && node --check gui/dist/app/runner.js
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo test 2>&1 | tail -5
```

期望：无语法错；check-gui 全绿；GUI 后端测试全过（`example_tests.rs` 用的是
自造的临时文件，不依赖 `examples/` 实际内容，加文件不会撞它）。

跑起来（内核是非 urban 的 default）：

```bash
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
bash $S/click.sh "站点"; sleep 1
```

走到第 3 步，点「用自带的示例站点」。**注意**：应用数据目录里可能已经有
上次装的那份（只有 CN-Cng），`install_example` 见到 `Sitedata` 存在就不再
复制。要验新文件得先清掉：

```bash
rm -rf ~/Library/Application\ Support/edu.sysu.colm.desktop/examples
```

然后重新点。期望：

- 按钮文字是「用自带的示例站点（CN-Cng）」（当前内核非 urban）
- 扫出 **2 个站点**
- AU-Preston 那一行带「城市」「要 urban 内核」「需全球栅格」三个标记
- 选中 AU-Preston 之后，建算例按钮变成
  「城市站还需要全球栅格目录 —— 去第 2 步填 rawdata 与 runtime」且**灰着**
- 选中 CN-Cng 则按钮正常，是「建算例：CN-Cng」

```bash
bash $S/ax.sh | grep "AU-Preston\|CN-Cng\|需全球栅格\|城市站还需要"
pkill -f "target/debug/colm-desktop-gui"
```

urban 内核这台机器没编，按钮文案随内核变那条**跳过**，在报告里写明。
（想验的话：`./oracle/scripts/build_kernel.sh urban`，几十秒。若编了，
切到 urban 内核后按钮应变成「用自带的示例站点（城市站 AU-Preston）」。）

- [ ] **Step 9: 提交**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add examples/ gui/dist/app/sites.js gui/dist/app/runner.js gui/dist/index.html gui/src-tauri/src/example.rs
git commit -m "自带一个城市示例站点，并把栅格门槛说在前面

选了 urban 内核的人原来手上没有任何能试的东西。AU-Preston 是 URBAN 预设
验收用的那个站点，三件套 4.5 MB。

**自带的是站点文件，不是栅格。** 城市算例的土壤剖面、湖深、反照率与 LCZ
分类只能从全球栅格取，那套数据 698 GB。所以 AU-Preston 装完不能直接跑 ——
界面在按下建算例之前就说：站点行标「需全球栅格」，按钮置灰并指明去第 2 步
填 rawdata 与 runtime。一个能点但必然失败的入口比一个灰着的更糟。

两套都装、都显示、标出匹配，不按内核过滤 —— 藏起来会让人以为自带的就那
一个。

Constraint: examples/README.md 与 example.rs 里「只有一个自然站点」的立场
已改，两处注释同步更新
Confidence: high
Scope-risk: moderate
Tested: colm-cli new 实测缺栅格的报错原文; scan 扫出 2 个站点; check-gui"
```

---

## Task 9: 算例列表两页各一份

第 3 步（站点）建完算例要看见「建出来没有」，第 5 步（运行）要「跑哪些」，
现在是同一个 `#cases`，一个 DOM 元素进不了两页。

**Files:**
- Modify: `gui/dist/index.html`
- Modify: `gui/dist/app/sites.js`

- [ ] **Step 1: `index.html` 两个容器各就各位**

在站点页 `<div id="makecase" style="margin-top:12px"></div>` 那一行之后插入：

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
**对外签名不变**（仍是无参 `renderCases()`），所以九处调用点一个都不用改：

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

- [ ] **Step 3: 确认没有别处还按老 id 取它**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
grep -rn "'cases'\|\"cases\"" gui/dist/app/ gui/dist/index.html || echo "干净"
```

期望：打印「干净」。有输出就说明还有地方按老 id 取，一并改掉。

- [ ] **Step 4: 静态检查与跑一遍**

```bash
node --check gui/dist/app/sites.js
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
```

走到第 3 步建好算例，确认列表出现在站点页；翻到第 5 步，确认同一份列表也在，
且在第 3 步勾中的那个在第 5 步仍然勾着：

```bash
bash $S/ax.sh | grep -A3 "跑哪些\|运行选中的\|运行全部"
pkill -f "target/debug/colm-desktop-gui"
```

期望：`#runall` 的文字随勾选变（「运行选中的 1 个」/「运行全部 N 个」）。

- [ ] **Step 5: 提交**

```bash
git add gui/dist/index.html gui/dist/app/sites.js
git commit -m "算例列表在站点页与运行页各画一份

一个 DOM 元素进不了两页，而两处都要回答「哪些算例存在」。
勾选状态共享 state.pickedCases，两边贯通。

Constraint: renderCases 对外签名不变，九处调用点不动
Rejected: 只留一页、另一页显示摘要 | 摘要行答不了「AU-DaS 到底建出来没有」
Confidence: high
Scope-risk: moderate
Tested: node --check; xtask check-gui; 第 3 步勾选在第 5 步仍然勾着"
```

---

## Task 10: 只读派生项并入各分节

**Files:**
- Modify: `gui/dist/app/params.js`

- [ ] **Step 1: 去掉专家模式那层过滤**

`params.js` 里这三行：

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

`Array.prototype.sort` 在现代引擎里是稳定的，同类之间的原有顺序不变。

- [ ] **Step 3: 静态检查与跑一遍**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/params.js
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
```

走到第 4 步（参数），**保持常规模式**（不点「专家」）：

```bash
bash $S/ax.sh | grep "派生值，改不了"
bash $S/ax.sh | grep -A12 "文件与目录"
pkill -f "target/debug/colm-desktop-gui"
```

期望：读出若干行「（派生值，改不了）」；当前内核是非 urban 且 TRACEROFF，
示踪剂那一个会被 `irrelevant` 挡掉，看到 3–5 行都算正常，**一行都没有就是
这一步没生效**。「文件与目录」分节里 `DEF_dir_landdata`、`DEF_dir_restart`、
`DEF_dir_history` 三行排在该分节其余字段之后。

- [ ] **Step 4: 提交**

```bash
git add gui/dist/app/params.js
git commit -m "只读派生项并入各分节，不再藏在专家模式后面

全仓库只有 6 个，它们回答「这个值现在是多少」，而那是个常规问题。
每节内可编辑在前、只读在后，只读行混在中间会打断编辑节奏。

Constraint: state.expert 与 body.expert 保留，那是后续挂选项的钩子
Confidence: high
Scope-risk: narrow
Tested: node --check; xtask check-gui; 常规模式读出派生行且排在节末"
```

---

## Task 11: 专家模式腾空后要说话

Task 10 之后 `state.expert` 没有任何消费者了，切过去界面纹丝不动。
**一个点了没反应的按钮比没有按钮更糟** —— 这个项目自己的话：
静默跳过与静默失败在界面上长得一样。

**Files:**
- Modify: `gui/dist/app/params.js`

- [ ] **Step 1: 在参数表顶上加一句占位**

`renderFields()` 里找到 `renderScope(box);` 这一行，在它**之前**插入：

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

- [ ] **Step 2: 静态检查与跑一遍**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
node --check gui/dist/app/params.js
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /dev/null 2>&1 &
sleep 4
```

走到第 4 步，点顶栏的「专家」：

```bash
bash $S/click.sh "专家"; sleep 1
bash $S/ax.sh | grep "专家选项还在规划中"
bash $S/click.sh "常规"; sleep 1
bash $S/ax.sh | grep "专家选项还在规划中" || echo "切回常规后消失，正确"
pkill -f "target/debug/colm-desktop-gui"
```

期望：切专家读得到那句话，切回常规打印「切回常规后消失，正确」。
`main.js` 里 `addEventListener('colm:mode', ...)` 已经接好重渲染，
读不到就是那条事件没触发，回去查 `shell.js` 的 `modeSeg` 接线。

- [ ] **Step 3: 提交**

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

## Task 11b: 修掉一个早就红了的测试

**这不是本次改动引入的。** `xtask/tests/params_groups.rs` 的两个测试在 `main`
上就是红的：

```
every_group_is_present_and_the_catch_all_is_last  → 「分类 site 不见了」
the_always_shown_whitelist_names_real_fields      → 「白名单」
```

它验的是 `params.js` 里的九分类表（`id: 'site'` …）与 `const ALWAYS_SHOWN`
白名单，而 `dfa0d1b`「让参数清单严格跟随所选内核」把这两样都换掉了 ——
现在是 `PARAM_SECTIONS` 加后端 `field_section()` 推导。那个提交的
`Tested:` 行写的是 `config tests; clippy; xtask check-gui; node --check`，
**没跑 `cargo test --workspace`**，所以漏了。

**它守的东西仍然有价值，只是判据变了。** 后端 `field_section()` 返回一个
分类名，前端 `params.filter(e => PARAM_SECTIONS.includes(sectionOf(e)))`
**把没列进来的分类静默丢掉** —— 上游新增一个分类而前端忘了加，那一整组
字段就在界面上消失，而且不报错。这正是要守住的。

实测的两侧集合：

| 侧 | 集合 |
|---|---|
| 后端 `field_section()` | 16 个：算例 站点 文件与目录 网格与并行 地表数据 初始场 城市 水热过程 生态与生地化 河道与水库 强迫场 数据同化 示踪剂 **时间与预热 输出与重启 输出变量** |
| 前端 `PARAM_SECTIONS` | 前 13 个 |
| 差集 | 时间与预热 · 输出与重启 · 输出变量 —— 各有专门的卡片，不进字段表 |

**Files:**
- Modify: `xtask/tests/params_groups.rs`

- [ ] **Step 1: 把整个测试文件换成验新的不变式**

```rust
//! 后端推导的分类，前端必须每一个都处理到。
//!
//! 分类在 Rust 侧由 `field_section()` 推导，显示顺序在 JS 侧的
//! `PARAM_SECTIONS` 里，两边各自的测试都不会发现对方变了。
//!
//! **漏一个的后果是静默消失**：`params.js` 用
//! `PARAM_SECTIONS.includes(sectionOf(e))` 过滤，没列进来的分类
//! 那一整组字段在界面上不出现，且不报错。
//!
//! 这个文件原来验的是九分类表与 `ALWAYS_SHOWN` 白名单，那两样已经被
//! 「让参数清单严格跟随所选内核」换掉了，于是测试红了很久没人发现 ——
//! 那个提交没跑 `cargo test --workspace`。

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("repo root")
}

/// `field_section()` 可能返回的全部分类。
///
/// 扫源码而不是调函数：`field_section` 住在 `gui/src-tauri`，那是**另一个
/// workspace**（把 429 个 Tauri 依赖挡在引擎外面，见 design.md §4.1），
/// xtask 依赖不到它。`xtask/src/gui.rs` 的静态检查用的是同一手法。
///
/// **不要用 shell 的 `grep -o` 做这件事** —— 实测 macOS 的 grep 对这些
/// 多字节字面量只抓得到一部分（17 个里只报 5 个）。
fn backend_sections() -> BTreeSet<String> {
    let src = std::fs::read_to_string(repo().join("gui/src-tauri/src/config.rs"))
        .expect("config.rs");
    let start = src
        .find("pub(crate) fn field_section")
        .expect("field_section 不见了");
    let end = src[start..].find("\npub ").map(|i| start + i).unwrap_or(src.len());
    let body = &src[start..end];

    let mut out = BTreeSet::new();
    let mut rest = body;
    while let Some(i) = rest.find("Some(\"") {
        rest = &rest[i + 6..];
        let Some(j) = rest.find('"') else { break };
        let name = &rest[..j];
        // `field_section` 里也会拿 namelist 组名做判断，那不是分类。
        if !name.starts_with("nl_") {
            out.insert(name.to_string());
        }
        rest = &rest[j..];
    }
    assert!(
        out.len() > 10,
        "只扫出 {} 个分类，扫法多半坏了而不是代码变了",
        out.len()
    );
    out
}

/// 前端字段表按这个顺序分节显示。
fn param_sections() -> BTreeSet<String> {
    let js = std::fs::read_to_string(repo().join("gui/dist/app/params.js")).expect("params.js");
    let start = js.find("const PARAM_SECTIONS").expect("PARAM_SECTIONS 不见了");
    let end = js[start..].find("];").expect("PARAM_SECTIONS 结尾") + start;
    js[start..end]
        .split('\'')
        .skip(1)
        .step_by(2)
        .map(|s| s.to_string())
        .collect()
}

/// 这三个分类**有意**不进字段表 —— 各自有专门的卡片。
///
/// 写死在这里而不是「凡是不认识的都放过」：新增一个分类时这个测试要红，
/// 逼人做一次决定 —— 是进字段表，还是也给它一张卡片。
const HANDLED_ELSEWHERE: &[&str] = &["时间与预热", "输出与重启", "输出变量"];

#[test]
fn every_backend_section_is_handled_by_the_frontend() {
    let backend = backend_sections();
    let front = param_sections();
    let elsewhere: BTreeSet<String> =
        HANDLED_ELSEWHERE.iter().map(|s| s.to_string()).collect();

    let unhandled: Vec<&String> = backend
        .iter()
        .filter(|s| !front.contains(*s) && !elsewhere.contains(*s))
        .collect();

    assert!(
        unhandled.is_empty(),
        "后端会把字段分到这些类里，而前端一个都没处理 —— 它们会在界面上\n\
         静默消失（params.js 按 PARAM_SECTIONS 过滤）：{unhandled:?}"
    );
}

#[test]
fn param_sections_names_no_section_the_backend_never_returns() {
    let backend = backend_sections();
    let front = param_sections();

    let dead: Vec<&String> = front.iter().filter(|s| !backend.contains(*s)).collect();

    assert!(
        dead.is_empty(),
        "PARAM_SECTIONS 里这几个分类后端从来不返回，是写错了名字还是留下的死条目：{dead:?}"
    );
}

#[test]
fn the_three_special_sections_really_exist() {
    // 白名单写错一个字，对应那组字段就悄悄掉进「没人处理」里，
    // 而上面那条测试**不会**报 —— 它只看有没有漏，不看白名单本身对不对。
    let backend = backend_sections();
    for s in HANDLED_ELSEWHERE {
        assert!(
            backend.contains(*s),
            "HANDLED_ELSEWHERE 里的 {s:?} 后端根本不会返回，白名单写错了"
        );
    }
}
```

- [ ] **Step 2: 跑**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo test -p xtask --test params_groups 2>&1 | tail -12
```

期望：3 个测试全过。

- [ ] **Step 3: 确认它真的抓得住回归**

**测试自己也要被测。** 临时把 `PARAM_SECTIONS` 里的 `'算例'` 删掉，
再跑一次，必须**红**：

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cp gui/dist/app/params.js /tmp/params.js.bak
sed -i '' "s/'算例', '站点'/'站点'/" gui/dist/app/params.js
cargo test -p xtask --test params_groups 2>&1 | grep -E "算例|test result"
cp /tmp/params.js.bak gui/dist/app/params.js
git diff --stat gui/dist/app/params.js
```

期望：报出「后端会把字段分到这些类里，而前端一个都没处理…：["算例"]」，
`test result: FAILED`。最后 `git diff --stat` 必须是**空的**（文件已还原）。

**这一步不能省。** 一个永远绿的测试和没有测试是一回事。

- [ ] **Step 4: 全量测试**

```bash
cargo test --workspace 2>&1 | tail -12
```

期望：全绿。需要 `PLUMBER2_ROOT` 的那几个会自己打印
`PLUMBER2_ROOT not set — skipping`，那是跳过不是失败。

- [ ] **Step 5: 提交**

```bash
git add xtask/tests/params_groups.rs
git commit -m "修掉一个早就红了的分类测试

它验的九分类表与 ALWAYS_SHOWN 白名单在「让参数清单严格跟随所选内核」
那次就被换掉了，而那个提交没跑 cargo test --workspace，于是红到现在。

守的东西仍然有价值，只是判据变了：后端 field_section 推导分类，前端
按 PARAM_SECTIONS 过滤，**没列进来的分类会静默消失**。三个测试分别守
「后端有前端没有」「前端有后端没有」「白名单本身没写错」。

Constraint: 扫源码不调函数 —— gui/src-tauri 是另一个 workspace
Directive: 别用 shell grep 提取这些中文字面量，macOS 的 grep 17 个只报 5 个
Confidence: high
Scope-risk: narrow
Tested: 3 个测试全过；删掉一个分类后确认它真的会红；cargo test --workspace"
```

---

## Task 11c: 修审查抓到的状态一致性问题（已派发）

一轮审查在隔离副本里配假后端跑完完整六步，抓到 1 个 Critical + 7 个
Important + 4 个 Minor，**每条都实测复现**。这一批修前五条：

| # | 问题 | 表现 |
|---|---|---|
| Critical | `restoreRecent` 只写 `el.value` 不发 `change` | 第二次启动起城市栅格目录永远出不来，建出 `rawdata: null` 的城市算例而界面说它能跑 |
| 2 | 建完算例站点列表不刷新 | 同一张卡片下半截列出新算例、上半截还说这些站点没建过 |
| 3 | 左栏 `one` 与 `n` 不同源 | 写「US-Urb 等 2 个」，而那 2 个里没有 US-Urb |
| 4 | 重扫不清 `picked`/`pickedSite` | 按钮写「建算例：选中的 90 个站点」，按下去落一句「先点一个站点」 |
| 5 | `batch` 与 `pickedCases` 没有桥 | 第 4 步说改 2 个，第 5 步说跑 4 个 |

第 4 条更阴的那一半：重扫后 `pickedSite` 仍指向**上一个目录**的站点对象，
而一个没勾时 `confirmSelection` 正是拿它去建 —— 在新目录里建一个旧目录
站点的算例，**界面上看不出异常**。

**Critical 那条的教训值得单独记**：`#kernel` 的 `onchange` 是「内核变了」
的唯一通路，而它管着三样东西（kernelmeta 文字、`#urbandirs` 显隐、站点行
的内核匹配标记）。用代码设 DOM 值时**凡是有人监听 change 的控件都要补一次
派发** —— 否则「内核」这个概念在界面上会分裂成两个：下拉框知道的那个，
和其余三处以为的那个。

---

## Task 8c-2: 预抽 21 个城市站的土壤剖面点值

城市算例最后一道门槛。8c-1 修掉两个 Fortran bug 之后省了 130 MB，
剩下的 **122 GB `soil/`** 要靠这一步。

**关键事实**（已查证）：城市段**已经有**从 `site.nc` 读土壤的分支，
而且那里的 `readflag` 写法是**对的**（不像 lakedepth 那处）：

```fortran
readflag = USE_SITE_soilparameters
u_site_vf_sand = readflag .and. ncio_var_exist(fsrfdata,'soil_vf_sand',readflag)
IF (u_site_vf_sand) THEN
   CALL ncio_read_serial (fsrfdata, 'soil_vf_sand', SITE_soil_vf_sand)
ELSE
   ... 从 soil/vf_sand_s.nc 读 8 层
```

所以值写进 `site.nc` 就能喂进去，**不用再改 Fortran**。

**为什么自然站点不需要这一步**：PLUMBER2 站点文件自带 23 个 `soil_*`
变量，`fill()` 再推导补 4 个。Urban-PLUMBER 站点文件 23 个变量**全是
形态学量，一个 `soil_*` 都没有**。

**Files:**
- Create: `oracle/src/bin/extract_urban_soil.rs`
- Create: `crates/colm-srfdata/src/urban_soil.rs`（生成的产物，入库）
- Modify: `crates/colm-srfdata/src/lib.rs`（挂上新模块）

- [ ] **Step 1: 先实测「写进哪些就够了」——别照源码推清单**

上一轮 8c 正是因为照源码推的清单不准而 BLOCKED。这次先量。

城市段 `allocate` 的是这 8 个（各 8 层）：

```
soil_vf_clay  soil_vf_gravels  soil_vf_om  soil_vf_quartz_mineral
soil_vf_sand  soil_wf_clay     soil_wf_gravels  soil_wf_sand
```

但 CoLM 从 `site.nc` 读的 `soil_*` 全集有 29 个
（`grep -o "ncio_var_exist(fsrfdata,'soil_[^']*'"`）。**先写这 8 个，
跑一次，看还去开哪些栅格文件**，再按报错逐轮补，直到不再碰 `soil/`。

用 8c-1 那套办法：造一个真实 rawdata 的软链树，**减去 `soil/`**，
跑通就直接证明依赖没了。

```bash
R=/Users/zhongwangwei/Desktop/colm-rust/rawdata
M=/tmp/raw-no-soil && rm -rf $M && mkdir -p $M
for f in $R/*; do [ "$(basename $f)" = soil ] || ln -s "$f" $M/; done
ls $M
```

站点用 AU-Preston，内核 `kernels/urban`。写变量进 `site.nc` 用本机
`/Users/zhongwangwei/miniforge3/bin/python`（有 netCDF4）。

**每一轮的报错都记下来**，那是这个任务最有价值的产出。

- [ ] **Step 2: 写抽取工具 `oracle/src/bin/extract_urban_soil.rs`**

对每个城市站点，按经纬度从栅格读点值。

栅格里每层是一个独立变量：`vf_sand_s.nc` 里是 `vf_sand_s_l1` …
`vf_sand_s_l8`（已用 `ncdump` 确认）。**文件名到 `site.nc` 变量名的映射
不规则**（`k_s.nc` → `soil_k_s` 而不是 `soil_k`；`psi_s.nc` → `soil_psi_s`），
必须显式列表，不能用规则推。

用法：

```
extract-urban-soil <Urban-PLUMBER/Sitedata> <rawdata> > crates/colm-srfdata/src/urban_soil.rs
```

**读点值的办法照抄 `crates/colm-srfdata/src/raster.rs` 的
`point_f64` / `point_i32`** —— 它已经处理了经纬度定位与缺测。别自己写一份。

- [ ] **Step 3: 产出 `crates/colm-srfdata/src/urban_soil.rs`**

**生成 Rust 源码，不是 JSON** —— `colm-srfdata` 只依赖 `anyhow` 与
`netcdf`，加一个 `serde_json` 只为读一张静态表不划算；而
`colm-schema/src/generated.rs` 已经立了「生成的产物入库」这个先例。

形状（具体字段按 Step 1 的实测清单定）：

```rust
//! 21 个 Urban-PLUMBER 站点的土壤剖面点值，从 CoLM 2024 rawdata 抽出。
//!
//! **生成的产物，不要手改。** 重生成：
//! `cargo run -p oracle --bin extract-urban-soil -- <Sitedata> <rawdata> > 本文件`
//!
//! **为什么要它**：城市站点文件 23 个变量全是形态学量，一个土壤剖面量
//! 都没有；而 CoLM 的城市路径缺了它们就只能去开 122 GB 的 `soil/`。
//! 每个站点一次只读一格 —— 把那一格预先抽出来，门槛就从 122 GB 落到几十 KB。
//!
//! **这些值是量出来的，不是假设的**：来源是 CoLM 2024 rawdata 在该站点
//! 经纬度上的格点值。写进 site.nc 时的 `source` 属性要说出这一点。

/// 一个城市站点的土壤剖面。层数与 CoLM 的 `nl_soil` 对齐。
pub struct UrbanSoil {
    pub site: &'static str,
    pub lon: f64,
    pub lat: f64,
    pub vf_sand: [f64; 8],
    // … 其余按 Step 1 实测出来的清单
}

pub static SITES: &[UrbanSoil] = &[ /* 生成 */ ];

/// 按经纬度找这个站点的剖面。**按经纬度不按名字** —— 名字在两套数据集里
/// 会重（`AU-Preston` 在 PLUMBER2 与 Urban-PLUMBER 里各有一个），
/// 而经纬度是抽取时用的键。
pub fn lookup(lon: f64, lat: f64) -> Option<&'static UrbanSoil> {
    SITES.iter().find(|s| (s.lon - lon).abs() < 1e-3 && (s.lat - lat).abs() < 1e-3)
}
```

- [ ] **Step 4: 只做到这里，先停**

`prepare_urban` 的接通是下一个任务（8c-3）。这一步的产出是
**一张查得到的表** + **一份实测清单**，验收标准是：

```bash
cargo build -p colm-srfdata
cargo test --workspace 2>&1 | tail -5
```

265 passed 不能变少。加一个小测试确认表能查到 AU-Preston 且层数对。

- [ ] **Step 5: 提交**

分两个提交：工具一个，生成的数据一个（数据那个的信息里写清楚是怎么生成的、
用的哪份 rawdata）。

---

## Task 8c-3: 接通 —— 城市算例彻底不碰栅格

8c-1 修掉两个 Fortran bug（省 130 MB），8c-2 把 21 个站的土壤剖面点值抽成
一张入库的表（省 122 GB）。这一步把它接进 `prepare_urban`，并把
`--rawdata` 从必填改回可选。

**字段清单以 8c-2 的实测结果为准**，不以这里写的为准。

**Files:**
- Modify: `crates/colm-srfdata/src/site.rs`
- Modify: `crates/colm-case/src/build.rs`
- Modify: `crates/colm-cli/src/main.rs`

- [ ] **Step 1: `prepare_urban` 写入土壤剖面**

它现在只把 `ground_height` 抄成 `elevation`。让它同时：

1. 按经纬度查 `urban_soil::lookup(lon, lat)`
2. 查到就把各变量写进 `site.nc`，每个带 `source` 属性：
   `"extracted from CoLM 2024 rawdata soil/*.nc at this site"`
3. **查不到就什么都不写** —— 让 CoLM 回落栅格，并在返回的 `UrbanReport`
   里说明「这个站点不在预抽表里，需要 rawdata」

第 3 条很重要：表只覆盖 Urban-PLUMBER 那 21 个站。**别对表外的站点编默认值**
—— 土壤剖面不像 `elvstd`/`sloperatio` 那样「模块默认值恰好没代价」，
编一个会让结果错得看不出来。

`LCZ_DOM` 与 `LUCY_ID` 也在这一步写（8c-2 已核对真值：AU-Preston 是
LCZ 6、LUCY 12）。`LCZ_DOM` 若表里没有就用 6 并把 `source` 写成
`"assumed: LCZ 6 open low-rise (Stewart & Oke 2012)"` —— 那是**假设**，
措辞要与「量出来的」区分开。

- [ ] **Step 2: `colm-case` 不再把三项设成 `.false.`**

`crates/colm-case/src/build.rs` 里 `for n in ["lakedepth", "soilreflectance",
"soilparameters"]` 那个循环删掉，注释换成：

```rust
        // 这三项保持默认的 .true.（「站点文件里有，用它」）—— `prepare_urban`
        // 现在把土壤剖面、湖深、反照率都写进了 site.nc：剖面来自随仓库发的
        // 预抽表（21 个 Urban-PLUMBER 站点，值是从 CoLM 2024 rawdata 该站
        // 格点上量出来的），其余走模块默认值。
        //
        // 改成 .false. 会把 CoLM 推去读全球栅格，而那套数据实测 240 GB。
        // 城市站点文件里确实没有这三样（23 个变量全是形态学量），
        // 但「站点文件里没有」与「必须去栅格取」之间隔着 prepare_urban。
```

`DEF_URBAN_type_scheme = 2`（LCZ）那一句**保留**。

- [ ] **Step 3: `colm-cli new` 的两个目录改回可选**

给了就用（栅格优先），没给就和自然站点一样指向不存在的目录。注释：

```rust
    // 全球栅格目录。**给了就用，没给就回落** —— `site::fill` /
    // `prepare_urban` 已经把该有的都写进 site.nc 了，跑通了就证明没读栅格。
    //
    // 城市算例曾经必填这两个。解开它花了三步：修掉两个让 site.nc 分支
    // 不可达的 Fortran bug（lakedepth 的 readflag 取自未赋值的结果变量、
    // TREE_LAI 命中分支不分配 SITE_LAI_year），再把 21 个站的土壤剖面
    // 从栅格预抽成一张入库的表。城市算例一次只读一格，而那 21 格加起来
    // 只有几十 KB。
```

**表外的城市站点仍然需要 rawdata** —— 那时 `prepare_urban` 不写土壤，
CoLM 回落栅格，若目录不存在就报错。这是对的：不能对没量过的站点编数。
错误信息要说清楚「这个站点不在预抽表里」。

- [ ] **Step 4: 实测 AU-Preston 完全不给栅格跑完三段**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo build -p colm-cli
U=/Users/zhongwangwei/Desktop/colm-rust/Urban-PLUMBER
T=/tmp/urban-final && rm -rf $T
./target/debug/colm-cli new --site "$U/Sitedata/AU-Preston_site_v1.nc" \
  --out $T --name AU-Preston --start 1993-01-01 --end 1993-01-11
grep -E "USE_SITE_(lakedepth|soilreflectance|soilparameters)|DEF_dir_rawdata" $T/case.nml
./target/debug/colm-cli run $T --kernel kernels/urban 2>&1 | tail -20
```

期望：三个 `USE_SITE_*` **不出现**（保持默认 `.true.`），
`DEF_dir_rawdata` 指向 `$T/rawdata_unused/`，三段全 `ok`。

**对照 README 的验收基准**：264 条小时记录、`f_tref` 峰值 312 K。
8c-1 已用真实栅格复现过 264 条 / 311.96 K —— 这次不给栅格，
**结果应当与那次一致**。对不上就如实报出来，那意味着预抽的值与栅格真值有出入。

- [ ] **Step 5: 21 个站全跑一遍**

一个站点跑通不代表表是对的。把 21 个都建出来（不必都跑完三段，
`mksrfdata` 过了就说明土壤读到了）：

```bash
for f in "$U"/Sitedata/*_site_v1.nc; do
  n=$(basename "$f" _site_v1.nc)
  ./target/debug/colm-cli new --site "$f" --out /tmp/u21/$n --name $n \
    --start 1993-01-01 --end 1993-01-03 2>&1 | tail -1
done
```

然后挑 3–4 个跑 `mksrfdata`。**哪个站点失败就报出来**，那说明预抽表漏了它
或者那个站点有别的特殊之处。

- [ ] **Step 6: 自然站点不能被弄坏（硬门槛）**

```bash
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo test --workspace 2>&1 | tail -6
cargo run -p oracle --bin golden-run -- CN-Cng 2>&1 | tail -5
cargo run -q -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

期望逐字 `identical: 129 variables, 10 dimensions (ignoring ["create_time"])`，
`cargo test --workspace` 不少于 265 passed。对不上就 **BLOCKED**。

- [ ] **Step 7: 提交，并更新 README**

README「URBAN 是唯一必须带全球栅格跑的预设」那一节的结论变了。改成说明
现在的状态：站点在预抽表里就不需要栅格，表外的仍然需要。

---

## Task 11d: 性能与重构（审查剩下的那批）

**Files:**
- Modify: `gui/dist/app/sites.js`
- Modify: `gui/dist/app/runner.js`
- Modify: `gui/dist/app/main.js`
- Modify: `gui/dist/app/domain.js`

- [ ] **Step 1（Important）: 批量建算例别每次都重扫**

`ensureCase()` 每建一个就 `list_cases` 一次并 `renderCases()` 一次，而
`renderCases()` 现在要画**两个**容器。实测 90 站「全选 + 建算例」：
`new_case` 90 次、`list_cases` **也 90 次**、约 8000 个行节点（行数
1…90 递增 × 2 个容器）。假后端 355 ms；真后端每次 `list_cases` 要遍历
算例根目录并读每份 `case.nml`。

**两容器是这轮加的，DOM 常数直接乘 2** —— 原来的 O(n²) 被放大了一倍。

把 `ensureCase()` 里的 `list_cases` + `renderCases()` + `renderSteps()`
拿掉（保留 `setStatus` 的进度报告），改由 `ensureCases()` 在循环结束后
统一扫一次、渲染一次：

```js
/** 为一批站点确保算例存在。**扫一次，不是每建一个扫一次。**
 *
 *  实测 90 站：原来 new_case 90 次、list_cases 也 90 次，每次之后重画
 *  两个容器（行数 1…90 递增），约 8000 个行节点。中途的进度靠
 *  `setStatus` 报，不靠列表刷新。 */
export async function ensureCases(sites) {
  const made = [];
  const failed = [];
  for (const [i, s] of sites.entries()) {
    setStatus(`准备算例 ${i + 1}/${sites.length}：${s.name}`);
    const c = await ensureCase(s);
    if (c) made.push(c); else failed.push(s.name);
  }
  // 循环结束后统一扫一次并渲染一次。
  const root = $('root').value.trim();
  if (root) {
    try {
      state.cases = await invoke('list_cases', { root });
      renderCases();
      renderSteps();
    } catch (e) { setStatus(e); }
  }
  if (failed.length) setStatus(`${made.length}/${sites.length} 个就绪；建不了：${failed.join('、')}`);
  return made;
}
```

`ensureCase()` 里改成建完直接返回，不自己扫。**注意它的返回值现在要靠
`list_cases` 才拿得到 `Case` 对象** —— 改成返回一个由站点名与目录拼出的
最小对象，或者让 `ensureCases` 在统一扫描后按名字回填。**以实际代码为准，
别把返回值弄丢了**，`confirmSelection` 要拿 `made[0]` 去 `selectCase`。

- [ ] **Step 2（Important）: 勾算例不要重建自己所在的容器**

`renderCasesInto()` 里勾选框的 `onchange` 直接调 `renderCases()`，
把自己所在的容器整个重建了。实测：按一下复选框，`document.activeElement`
变成 `<body>` —— 键盘操作每勾一个就要重新 Tab 回去。

（站点列表没这个问题，它只 `renderMakeCase` + `renderSteps`，不重建自己。）

改成只重画**另一个**容器：

```js
    cb.onchange = () => {
      if (cb.checked) state.pickedCases.add(c.dir); else state.pickedCases.delete(c.dir);
      // **只重画另一个容器，自己这个原地不动。** 重建自己所在的容器会
      // 把焦点打到 body 上 —— 键盘操作每勾一个就要重新 Tab 回去。
      // 勾选状态两页共享，另一页必须跟着变。
      for (const id of ['cases-built', 'cases-run']) {
        if (id === box.id) continue;
        const other = $(id);
        if (other) renderCasesInto(other);
      }
      updateCaseBatchButtons();
    };
```

`renderCasesInto(box)` 要能知道自己画在哪个容器里 —— `box.id` 就是。

- [ ] **Step 3（Important）: `showKernelMeta` 改名，并成为「内核变了」的唯一入口**

它现在做三件事（写 meta 文字、切 `#urbandirs`、重画站点列表），三个调用点
确实三件都要，所以不是多做了 —— **问题在名字只说了第一件**。也正因为它
读起来像个纯显示函数，Task 11c 那个 Critical 才没人想到「恢复内核之后
也得调它」。

改名 `applyKernel()`，并把 `onchange` 里另外两个调用收进去：

```js
/** 内核变了：把随之而变的东西全部更新。
 *
 *  **这是「内核变了」的唯一入口。** 它管着四样：meta 文字、城市栅格目录
 *  的显隐、站点行的内核匹配标记、当前内核编不进去的字段名单。
 *  分散在调用点的话，总有一条路径会漏掉其中一样 —— 实测漏过：
 *  `restoreRecent` 只写 `el.value` 不发 change，于是恢复出来的内核
 *  只有下拉框自己知道，城市栅格目录永远出不来。
 *
 *  原名叫 `showKernelMeta`，只说了第一件事，而它读起来像个纯显示函数。 */
async function applyKernel() { ... }
```

`onchange` 相应简化成 `s.onchange = applyKernel;`。

**顺带消掉一次重复**：原来的 `onchange` 是
`showKernelMeta(); refreshRelevance(); renderSteps();`，而
`showKernelMeta → renderSites → renderSteps` 已经刷过一次 ——
`renderSteps` 每次切内核跑两遍。

- [ ] **Step 4（Minor）: 站点行的「已建算例」标记按 `caseName` 匹配**

`renderSites()` 里 `state.cases.find(x => x.name === s.name)`，
而 `ensureCase` 建的时候用的是 `s.caseName ?? s.name`。

实测：为 urban 那个 `AU-Preston` 建好算例（目录 `AU-Preston-urban`）后，
那一行**整表重画也永远**不显示「已建算例」；反过来只建了非 urban 那个时，
两行都显示「已建算例」。

```js
    // 按 caseName 匹配，不是 name —— 重名站点（AU-Preston 在 PLUMBER2 与
    // Urban-PLUMBER 里各有一个）建出来的目录带后缀，按 name 找会一个都
    // 认不出、或者两行都认成同一个。
    const c = state.cases.find(x => x.name === (s.caseName ?? s.name));
```

- [ ] **Step 5（Minor）: 启动时把已建的算例列出来**

`boot()` 里没人调 `renderCases()`。实测：`root` 由 `load_recent` 恢复、
磁盘上有算例时，第 3 步的 `#cases-built` 是个高 2px 的空盒子（连
「这个目录下没有算例」都没有），`#runall` 显示 HTML 里的死字「运行全部」。

在 `restoreRecent()` 之后补：

```js
    // 恢复出来的算例根目录里可能已经有算例 —— 不扫的话第 3 步是个空盒子，
    // 而用户上次的工作就在那里。
    const root = $('root').value.trim();
    if (root) {
      try {
        state.cases = await invoke('list_cases', { root });
        renderCases();
      } catch (e) { /* 目录没了就算了，扫描按钮还在 */ }
    }
```

- [ ] **Step 6（Minor）: 在 `domain.js` 里点明「落地时要改什么」**

`state.domain` 目前零读取点，形状只留了一半：`pick()` 对三档都硬编码
`go('prep')`，而 `STEPS` 是 `shell.js` 的模块级 const，被 `nextOf` /
`go` / `renderSteps` 直接闭包引用。区域/全球落地时必须把它变成
`STEPS[state.domain]` 或一个函数，`nextOf` 的签名和三个调用点都要跟着动。

接得上的那半是 `renderNextButtons` 遍历 `.page`：未知 `data-step` 的页
`nextOf` 返回 null、`.foot` 会被移掉，新域的页面加进来不会炸。

在 `domain.js` 的模块注释末尾补一段：

```js
//! **区域/全球落地时要改的是 `shell.js` 的 `STEPS`。** 它现在是模块级
//! 的 const，被 `nextOf` / `go` / `renderSteps` 直接闭包引用 —— 三档
//! 各自一套步骤链的话，得把它变成 `STEPS[state.domain]` 或一个函数，
//! 那三个调用点都要跟着动。`state.domain` 现在零读取点，别以为它已经接好了。
//!
//! 已经接好的那半：`renderNextButtons` 遍历 `.page`，未知 `data-step`
//! 的页 `nextOf` 返回 null、`.foot` 会被移掉，新域的页面加进来不会炸。
```

- [ ] **Step 7: 验证与提交**

每步之后 `node --check`；全部改完跑 `cargo run -p xtask -- check-gui`
与真界面（重点验 Step 1 的 90 站批量建：`list_cases` 只应发一次）。

分六个提交，一步一个。

---

## Task 12: `waterheat` 更名 `default`

这是整个计划里**唯一越出前端**的任务。它动脚本、Rust 测试与回归黄金基准。

`waterheat` 说的是它编进了什么（水热过程），但它实际的角色是**默认预设** ——
三个里最常用、文档里到处拿它举例的那个。

**要改的**（`grep -rl waterheat` 去掉历史文档后的全集）：

| 文件 | 处数 | 性质 |
|---|---|---|
| `oracle/scripts/build_kernel.sh` | 2 | usage 串 + `case` 分支 |
| `oracle/golden/kernel-manifest.json` | 1 | **回归黄金基准** |
| `oracle/src/bin/golden_run.rs` | 1 | 默认内核目录 |
| `oracle/tests/generated_case.rs` | 2 | 测试 |
| `oracle/tests/histmap.rs` | 1 | 测试 |
| `crates/colm-kernel/src/manifest.rs` | 1 | 注释 |
| `crates/colm-kernel/src/manifest_tests.rs` | 4 | 测试 |
| `crates/colm-hist/src/lib.rs` | 3 | 注释 |
| `crates/colm-hist/src/lib_tests.rs` | 11 | 测试 |
| `crates/colm-cli/src/fingerprint_tests.rs` | 5 | 测试 |
| `.github/workflows/ci.yml` | 1 | |
| `.github/workflows/release.yml` | 3 | |
| `.github/workflows/windows-kernel.yml` | 14 | 含 PowerShell 反斜杠路径 |
| `README.md` | 11 | 预设表与正文 |
| `docs/design.md` | 4 | 当前设计文档 |
| `kernels/waterheat/` | 目录 | 本机构建产物（gitignore） |

**不改**：`docs/plan-m*.md`、`docs/plan-gui1.md`、`docs/plan-gui2.md` ——
它们是历史记录，里面的 `waterheat` 连着当时的实测输出，改了就对不上了。
`docs/plan-gui3.md`（本文件）里的 `waterheat` 出现在验收命令与期望输出里，
**改**。

- [ ] **Step 1: 先量基线 —— 改名前跑一次黄金比对**

**这一步不能省。** 黄金文件是回归判据，改判据之前必须知道改之前它是过的。

PLUMBER2 数据在本机 `/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s`，
90 个站点，三个黄金输入的 sha256 与 `oracle/fixtures/inputs.sha256`
**逐一核对过、完全一致**。

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo run -p oracle --bin golden-run -- CN-Cng 2>&1 | tail -8
```

`golden-run` 自己**不比对**，它只跑三个阶段并提示下一条命令。判据要再跑一次：

```bash
cargo run -q -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

**基线已经实测过了**（改名前，`kernels/waterheat`），输出是：

```
identical: 129 variables, 10 dimensions (ignoring ["create_time"])
```

三个阶段 `mksrfdata ok / mkinidata ok / colm ok`。**这一行就是改名后要
逐字复现的目标。**

若这一步在你手上跑出别的结果，**停下来报 BLOCKED** ——
在一个本来就红的基准上做改名，改完分不清是改名弄坏的还是本来就坏的。

- [ ] **Step 2: 脚本与内核目录**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
sed -i '' 's/waterheat/default/g' oracle/scripts/build_kernel.sh
grep -n "default" oracle/scripts/build_kernel.sh
```

期望：usage 串变成 `<default|bgc|urban>`，`case` 分支变成 `default)`。

内核目录重新生成（而不是 `mv` + 手改 manifest —— 让脚本自己写出正确的
`preset` 字段，少一处手误）：

```bash
rm -rf kernels/waterheat
./oracle/scripts/build_kernel.sh default
cat kernels/default/manifest.json | head -8
```

期望：`"preset": "default"`，三个 `.x` 都在。

- [ ] **Step 3: Rust 源码、测试与黄金文件**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
for f in \
  oracle/golden/kernel-manifest.json \
  oracle/src/bin/golden_run.rs \
  oracle/tests/generated_case.rs \
  oracle/tests/histmap.rs \
  crates/colm-kernel/src/manifest.rs \
  crates/colm-kernel/src/manifest_tests.rs \
  crates/colm-hist/src/lib.rs \
  crates/colm-hist/src/lib_tests.rs \
  crates/colm-cli/src/fingerprint_tests.rs ; do
  sed -i '' 's/waterheat/default/g' "$f"
done
grep -rn "waterheat" crates/ oracle/ || echo "Rust 侧干净"
```

期望：打印「Rust 侧干净」。

- [ ] **Step 4: CI workflow**

```bash
for f in .github/workflows/ci.yml .github/workflows/release.yml .github/workflows/windows-kernel.yml; do
  sed -i '' 's/waterheat/default/g' "$f"
done
grep -rn "waterheat" .github/ || echo "CI 干净"
```

期望：打印「CI 干净」。注意 `windows-kernel.yml` 里有 PowerShell 的反斜杠
路径（`kernels\waterheat\colm.exe`），`sed` 一样能替换，替换后手工看一眼
那几行没被弄坏。

- [ ] **Step 5: 文档**

```bash
sed -i '' 's/waterheat/default/g' README.md docs/design.md docs/design-gui3.md docs/plan-gui3.md
grep -rn "waterheat" README.md docs/design.md || echo "文档干净"
grep -rln "waterheat" docs/
```

期望：前一条打印「文档干净」；后一条**只**列出 `docs/plan-m*.md` 与
`docs/plan-gui1.md` / `plan-gui2.md`（历史记录，不动）。

- [ ] **Step 6: 全量测试**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo test --workspace 2>&1 | tail -20
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
cargo run -p xtask -- check-gui
```

期望：全绿。

- [ ] **Step 7: 黄金比对 —— 改名后必须仍然逐位相同**

**这是这次改名唯一的风险点。** 只改名字不该动到任何一个字节的输出。

```bash
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo run -p oracle --bin golden-run -- CN-Cng 2>&1 | tail -8
```

再跑一次比对：

```bash
cargo run -q -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

期望**逐字**是：

```
identical: 129 variables, 10 dimensions (ignoring ["create_time"])
```

只改名字不该动到任何一个字节的输出。不一致就 **停下来报 BLOCKED**，不要试图「顺手修一下」——
改名改坏了黄金基准是必须当场查清的事。

注意内核目录也改名了，`golden_run.rs` 的默认值 `kernels/waterheat` 已经
在 Step 3 里跟着变成 `kernels/default`；若这一步报「找不到内核」，
说明那一处漏改了。

- [ ] **Step 8: 提交**

```bash
git add -A
git commit -m "waterheat 更名 default

它说的是编进了什么（水热过程），但实际角色是默认预设 —— 三个里最常用、
文档里到处拿它举例的那个。

docs/plan-m*.md 与 plan-gui[12].md 没动：那是历史记录，里面的 waterheat
连着当时的实测输出，改了就对不上。

Constraint: 黄金基准改了名，改后必须仍然 identical
Confidence: high
Scope-risk: broad
Directive: 改回归判据之前先量一次基线，否则分不清是改坏的还是本来就坏的
Tested: cargo test --workspace; clippy -D warnings; xtask check-gui; 黄金比对改名前后都过"
```

---

## Task 13: 全量回归与文档

**Files:**
- Modify: `README.md`

- [ ] **Step 1: 全量静态检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
for f in gui/dist/app/*.js; do node --check "$f" || echo "FAIL $f"; done
cargo run -p xtask -- check-gui
cargo test --workspace 2>&1 | tail -10
cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
cd gui/src-tauri && cargo test 2>&1 | tail -5 && cargo clippy --all-targets -- -D warnings 2>&1 | tail -3
```

期望：全绿。GUI 后端 43 个测试全过。

- [ ] **Step 2: 端到端走一遍六步**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust/gui/src-tauri && cargo build
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
./target/debug/colm-desktop-gui > /tmp/gui-final.log 2>&1 &
sleep 4
```

按顺序走：门选「站点」→ 第 2 步确认内核是 `default`、设算例目录 →
第 3 步「用自带的示例站点」→ 点 CN-Cng → 「建算例」→ 第 4 步确认时间卡片
与字段表（含派生行）→ 第 5 步「▶ 运行」→ 等三阶段跑完 → 第 6 步画图。

```bash
bash $S/ax.sh > $S/final.txt
grep -c "^" $S/final.txt
grep "第 6 步\|净辐射\|已完成\|default" $S/final.txt
cat /tmp/gui-final.log
pkill -f "target/debug/colm-desktop-gui"
```

期望：六步全走得通，结果页画得出图；启动日志两行面包屑分别报出
`colm-cli resolved to .../target/debug/colm-cli` 与 `1 preset(s) from .../kernels`。

- [ ] **Step 2b: 用 90 个站点验多站点上下文**

前面几个任务都因为「自带示例只有一个站点」跳过了这条。数据在
`/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s/Sitedata`，90 个站点。

走到第 3 步，把站点目录填成上面那个路径、点「扫描」，然后点「全选」：

```bash
S=/private/tmp/claude-501/-Users-zhongwangwei-Desktop-Github-CoLM-Rust/bb10e196-9af7-4677-8652-790e39e5da15/scratchpad
bash $S/ax.sh | grep -A2 "已选站点"
bash $S/ax.sh | grep "个站点\|已勾"
```

期望：

| 位置 | 应该显示 |
|---|---|
| 摘要行 | `90 个站点 · N 个无观测 · 当前内核 default` |
| 左栏「已选站点」 | `AT-Neu 等 90 个`（**带**「等 N 个」后缀） |
| `#pickinfo` | `已勾 90 个` |
| 建算例按钮 | `建算例：选中的 90 个站点` |

**四处的数必须一致。** 这正是「左栏说出在配几个站点」与「没有内核时别把
第 3 步锁死」两个提交合起来要保证的事，此前一直没有数据能验。

**不要真按下建算例** —— 那会串行建 90 个算例，每个都要读站点文件与强迫场
并写出 `site.nc`。验完显示就够了。

- [ ] **Step 2c: 重录黄金基准，消掉 provenance 噪音**

修了两个 Fortran bug 之后 `kernels/` 是 `ad77af53`，而
`oracle/golden/kernel-manifest.json` 仍记录 `72dd76b9`，于是**每次**
`golden-run` 都打：

```
  WARNING: kernel differs from the one that produced the golden files:
    colm_git_sha: recorded "72dd76b9", current "ad77af53"
```

它不阻断，但**持久的警告会让人对真正的 drift 麻木** —— 下次真出问题时
这行字已经没人看了。

前提是输出确实没变（已实测：`identical: 129 variables`）。**先比对，
再重录，顺序不能反**：

```bash
export PLUMBER2_ROOT=/Users/zhongwangwei/Desktop/colm-rust/PLUMBER2s
cargo run -p oracle --bin golden-run -- CN-Cng 2>&1 | tail -5
cargo run -q -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

**必须先看到 `identical: 129 variables` 再往下。** 不 identical 就说明
那两个 Fortran 改动动了自然站点的结果，那时重录等于把回归基准改坏 ——
停下来报 BLOCKED。

确认之后重录：

```bash
cargo run -p oracle --bin golden-run -- CN-Cng --write-golden
git diff --stat oracle/golden/
```

期望：`kernel-manifest.json` 的 `colm_git_sha` 变成 `ad77af53`、三个
sha256 跟着变；`CN-Cng_hist_2008-01.nc` **应该也会变**（`create_time`
不同），那是正常的 —— 比对本来就 `ignoring ["create_time"]`。

再跑一次 `golden-run`，确认 WARNING 消失、`provenance matches the
recorded kernel` 出现。

- [ ] **Step 3: 清掉 playwright 的运行垃圾并加进 `.gitignore`**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
rm -rf .playwright-mcp
grep -q '^\.playwright-mcp/' .gitignore || echo '/.playwright-mcp/' >> .gitignore
tail -3 .gitignore
```

- [ ] **Step 4: 更新 README 的 GUI 一节**

README「GUI（部分验收）」那一节描述的是五步与旧的三栏骨架。
在那一节的验收表格之前插入：

```markdown
### 进门那道分流，与六步

进门先问「这次要跑什么」：站点、区域、全球三档，后两档灰着标「暂不支持」。
**三档现在就摆出来**，将来任何一档落地时不用再改这一层。

选了站点之后是六步，**顺序由依赖链定**：

```
内核（无依赖） → 站点（要先知道内核） → 时间与预热（要先有 case.nml）
```

| 步 | 装着什么 |
|---|---|
| ① 前处理 | 占位页，形状先定下来 |
| ② 基本设定 | 内核 · 算例放哪 · （urban 内核时）全球栅格目录 |
| ③ 站点 | 扫目录 · 选站 · 建算例 |
| ④ 参数 | 时间与预热 · namelist 字段表 |
| ⑤ 运行 | 输出 · 跑哪些 · 开始 |
| ⑥ 结果 | 曲线与指标 |

**内核必须排在站点前面**：城市站要 `URBANON` 编进去的内核，还要额外给全球
栅格目录；`default` 内核跑不了城市站。反过来排的话，人挑完二十个城市站
才发现手上是 `default`。站点行会标出「要 urban 内核」而不是把它藏起来 ——
静默跳过与静默失败在界面上长得一样。

**只读派生项不再藏在专家模式后面。** 全仓库只有 6 个（`DEF_dir_landdata`
`DEF_dir_restart` `DEF_dir_history` `DEF_USE_USGS` `DEF_USE_IGBP`
`DEF_wetland_finundation_scheme`），它们并入各自的分节并排在节末。
常规/专家开关保留着等后续安排，空着的时候界面会明说。
```

- [ ] **Step 5: 提交并收尾**

```bash
git add README.md .gitignore
git commit -m "README 跟上六步与进门分流

Confidence: high
Scope-risk: narrow
Tested: 端到端走完六步，结果页画得出图"
git log --oneline -12
```

期望：十二个提交，从「把五步骨架换成六步」到「README 跟上六步与进门分流」。

---

## 附：这份计划**不做**什么

- 不实现区域与全球的步骤链 —— 只把入口摆出来
- 不给专家模式编内容 —— 腾空等后续安排
- 不动前处理页 —— 它仍是占位页
- 不动结果页的功能 —— 它现在的形状是对的
- 不按内核过滤站点列表 —— 只标出来
- 不改 `docs/plan-m*.md` 与 `plan-gui[12].md` 里的 `waterheat` —— 那是历史记录
- 不引入前端框架、构建工具或 npm 依赖
