# GUI 进门向导不显示：建观察通道并定位 实施计划

> **给执行者：** 按任务顺序做，每步都有可验证的预期输出。步骤用 `- [ ]`
> 勾选跟踪。**Task 4 之前不要改任何业务代码** —— 前三个任务是建观察通道，
> 第四个才根据观察结果分支。

**目标：** 让 `gui/dist/app/domain.js` 的进门向导在真机上显示出来；在此之前，
先建一条不依赖 AX 树和截图的诊断通道。

**架构：** 前端把关键节点和所有未捕获错误通过一个新的 Tauri 命令
`probe_log` 打到进程 stderr，启动时重定向到文件。诊断探针用**普通
`<script>`（非 module）**写在 `index.html` 里，排在 `<script type="module">`
之前 —— 这样连 module 自身加载失败都收得到。

**技术栈：** Rust / Tauri v2 / 原生 ES module 前端（无打包器）

---

## 零、先读这一段：这台机器上什么能用、什么不能用

**观察通道只有 stderr 日志。** 另外两条都验证过不可用：

| 通道 | 状态 |
|---|---|
| AX 树（`osascript` + System Events） | **时灵时不灵**。第一次读到内容，之后连读五次都返回 0 条。不可作判据。 |
| `screencapture`（窗口 / 全屏） | 都报 `could not create image`，没有屏幕录制权限。 |
| `strings <二进制>` 找前端字符串 | **无效**。Tauri 压缩存储嵌入资源，连 `gatetitle` 这种确定存在的老元素都搜不到。 |

**两个独立 workspace。** `cargo test --workspace` 在仓库根目录**到不了**
`gui/src-tauri`。GUI 的编译和测试必须 `cd gui/src-tauri` 再跑。

**`node --check` 不要用来验证前端模块。** 它只做语法扫描，不真正编译
ES module —— 本次事故里它两次报「通过」，而模块实际有致命错误。要验证就
用真 `import()`（见 Task 3 步骤 4）。

**每一轮验证的标准动作**（后面反复引用为「标准重启」）：

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
pkill -9 -f colm-desktop-gui 2>/dev/null; sleep 1
(cd gui/src-tauri && cargo build 2>&1 | grep -E "^error|Finished" | tail -3)
(cd gui/src-tauri && nohup ./target/debug/colm-desktop-gui > /tmp/gui.log 2>&1 &)
sleep 8
cat /tmp/gui.log
```

---

## 一、要动的文件

| 文件 | 干什么 |
|---|---|
| `gui/src-tauri/src/config.rs` | 新增 `probe_log` 命令（挨着已有的 `backend_ready`） |
| `gui/src-tauri/src/lib.rs:41` | 把 `probe_log` 加进 `generate_handler!` 列表 |
| `gui/src-tauri/build.rs` | 补 `rerun-if-changed=../dist`（Task 5A，视诊断结果） |
| `gui/dist/index.html:378` | 在 module script **之前**插入探针 `<script>` |
| `gui/dist/app/main.js` | 关键节点打点（诊断完要撤） |

---

## Task 1: 后端开一个说话的口子

**文件：**
- 修改：`gui/src-tauri/src/config.rs`（在 `backend_ready` 之后，第 212 行下方）
- 修改：`gui/src-tauri/src/lib.rs:42`

- [ ] **步骤 1：加命令**

在 `gui/src-tauri/src/config.rs` 中 `backend_ready` 函数结束（`}`，第 212 行）
之后插入：

```rust
/// 前端把话说到 stderr。**这是这台机器上 GUI 唯一可靠的观察通道** ——
/// AX 树读取时灵时不灵、`screencapture` 没有屏幕录制权限，两条都实测不可用。
///
/// 不引 `tauri-plugin-log`：那个插件要在 webview 侧注入 console 钩子，而这里
/// 恰恰要诊断「前端代码到底跑没跑」。诊断工具依赖被诊断的那一层，说明不了问题。
#[tauri::command]
pub fn probe_log(msg: String) {
    eprintln!("colm-desktop[probe]: {msg}");
}
```

- [ ] **步骤 2：注册**

`gui/src-tauri/src/lib.rs` 第 42 行 `backend_ready,` 之后加一行：

```rust
            probe_log,
```

- [ ] **步骤 3：静态检查必须过**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo run -p xtask -- check-gui
```

预期：打印一行统计并退出码 0，形如

```
gui: 30 commands registered, 29 called, 4 events listened for — all resolve
```

（数字随代码变。这个检查是**单向**的 —— `xtask/src/gui.rs:26` 是
`called.difference(&registered)`，只报「前端调了后端没注册的」，
不报「注册了前端没调的」。所以这一步允许 `probe_log` 只注册不调用；
Task 2 加上调用之后 `called` 那个数字会 +1。）

- [ ] **步骤 4：编译**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust/gui/src-tauri && cargo build 2>&1 | tail -3
```

预期：`Finished`，无 error。

- [ ] **步骤 5：提交**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add gui/src-tauri/src/config.rs gui/src-tauri/src/lib.rs
git commit -m "GUI 加一条 stderr 诊断通道 probe_log

这台机器上 AX 树读取时灵时不灵、screencapture 没有屏幕录制权限，
strings 又因为 Tauri 压缩嵌入资源而搜不到任何前端字符串 —— 三条观察
通道全废，只剩进程 stderr。没有它就只能靠猜，本次向导不显示的排查
已经在坏判据上空转了六轮。"
```

---

## Task 2: 前端探针（决定性的一步）

这个探针回答的问题是：**运行中的程序，它的 `index.html` 是不是磁盘上这份。**

**文件：**
- 修改：`gui/dist/index.html`

- [ ] **步骤 1：插入探针**

在 `gui/dist/index.html` 第 379 行 `<script type="module" src="app/main.js"></script>`
**之前**插入下面整段：

```html
<!-- 诊断通道。**必须是普通 script 且排在 module 之前** —— module 是 defer 的，
     它自己加载/解析失败时抛的错，要有人已经在听才收得到。放在 main.js 后面
     就晚了。捕获阶段（第三个参数 true）才收得到 <script> 元素上的失败。 -->
<script>
(function () {
  var q = [];
  function iv() {
    return window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
  }
  // 写成 `invoke('probe_log', …)` 这个形状是**故意的**：`xtask check-gui`
  // 扫的是 `invoke(` 后面那个字符串字面量，写成 `f(...)` 就绕过了检查,
  // 而绕过检查意味着把命令名拼错了也没人告诉你。诊断代码同样要受检。
  function send(m) {
    var invoke = iv();
    if (invoke) { invoke('probe_log', { msg: m }); } else { q.push(m); }
  }
  window.__probe = send;
  window.addEventListener('error', function (e) {
    if (e.target && e.target.tagName === 'SCRIPT') send('SCRIPT-FAILED ' + (e.target.src || 'inline'));
    else send('ERROR ' + e.message + ' @' + e.filename + ':' + e.lineno);
  }, true);
  window.addEventListener('unhandledrejection', function (e) {
    send('REJECT ' + ((e.reason && e.reason.message) || e.reason));
  });
  // __TAURI__ 可能比这段晚就绪，排空积压。
  var t = setInterval(function () {
    var invoke = iv();
    if (invoke) { clearInterval(t); while (q.length) invoke('probe_log', { msg: q.shift() }); }
  }, 50);
  send('HTML-PARSED fp=A1');
})();
</script>
```

- [ ] **步骤 2：静态检查仍要过**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo run -p xtask -- check-gui
```

预期：`called` 的数字比 Task 1 那次大 1，仍然 `all resolve`。若报
`probe_log` 未注册，说明 Task 1 步骤 2 的注册没生效。

- [ ] **步骤 3：标准重启，读日志**

用「零」节的标准重启命令。

- [ ] **步骤 4：判读**

预期日志里有：

```
colm-desktop[probe]: HTML-PARSED fp=A1
```

**如果有** → 运行的 `index.html` 就是磁盘这份，嵌入正常，问题在 JS。**跳到 Task 3。**

**如果没有**（日志只有 `the page reached the backend` 那两行）→ 运行的是**旧的
嵌入快照**，改 dist 根本没进去。**跳到 Task 5A。**

> 判读要点：`the page reached the backend` 那行**不能**用来证明前端是新的 ——
> 旧前端一样会打它。同理，界面上「前处理 / 基本设定 / 站点」这些字是
> `index.html` 里的**静态内容**（实测 5 处），看见它们不证明任何 JS 跑过。

- [ ] **步骤 5：提交**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add gui/dist/index.html
git commit -m "前端接上诊断探针，普通 script 排在 module 之前

排位置是有讲究的：module 是 defer 的，它自身加载或解析失败时抛的错，
要有人已经在监听才收得到；探针放在 main.js 之后就永远看不到 main.js
自己挂掉。error 监听器用捕获阶段，否则收不到 <script> 元素上的失败。"
```

---

## Task 3: 给启动路径打点

**前提：** Task 2 步骤 4 判定为「HTML-PARSED 出现了」。

**文件：**
- 修改：`gui/dist/app/main.js`

- [ ] **步骤 1：在四个位置打点**

`gui/dist/app/main.js` 第 18 行（`import './sitedata.js';` 之后、`initShell();`
之前）插入：

```js
window.__probe && window.__probe('main.js 顶层到达，import 链全部解析成功');
```

第 19 行 `initShell();` 之后插入：

```js
window.__probe && window.__probe('initShell 返回');
```

第 24 行 `showDomainGate();` **改成**下面三行：

```js
window.__probe && window.__probe('即将调用 showDomainGate');
showDomainGate();
window.__probe && window.__probe('showDomainGate 返回，domaingate.hidden=' + document.getElementById('domaingate').hidden);
```

- [ ] **步骤 2：加一个监视器，抓「谁把门又关了」**

同文件末尾追加：

```js
// 门被关掉是 domain.js:261 的 finish() 唯一该做的事。如果日志显示
// hidden 在没人点按钮的情况下变回 true，说明还有第二个地方在动它。
const gateEl = document.getElementById('domaingate');
if (gateEl) {
  new MutationObserver(() => {
    window.__probe && window.__probe('domaingate.hidden 变为 ' + gateEl.hidden);
  }).observe(gateEl, { attributes: true, attributeFilter: ['hidden'] });
}
```

- [ ] **步骤 3：标准重启，读日志**

- [ ] **步骤 4：如果日志在某一点断掉，用真 import 定位**

不要用 `node --check`。写 `/tmp/probe.mjs`：

```js
import { JSDOM } from 'jsdom';   // 没有 jsdom 就用下面的手搓 mock
const mod = await import('/Users/zhongwangwei/Desktop/Github/CoLM-Rust/gui/dist/app/domain.js');
console.log('domain.js 加载 OK', Object.keys(mod));
```

没有 jsdom 时，手搓最小 mock 再 import：

```js
globalThis.window = globalThis;
globalThis.document = {
  getElementById: () => ({ textContent: '', hidden: true, appendChild(){}, setAttribute(){} }),
  createElement: () => ({ classList: { add(){} }, appendChild(){}, setAttribute(){}, style: {} }),
  addEventListener(){},
};
const mod = await import('/Users/zhongwangwei/Desktop/Github/CoLM-Rust/gui/dist/app/domain.js');
console.log('加载 OK');
```

跑：`node /tmp/probe.mjs`。语法/引用错误会在这里精确报行号。

- [ ] **步骤 5：提交打点**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add gui/dist/app/main.js
git commit -m "启动路径打点，另加一个 hidden 属性的 MutationObserver

四个点把启动切成可判读的区间：import 链解析、initShell、调用前、调用后
带 hidden 实际值。观察者是为了回答另一个问题 —— 门开了之后有没有第二个
地方把它关回去，domain.js:261 之外不该有。"
```

---

## Task 4: 按日志分支

对照下表，**只走命中的那一支**：

| 日志现象 | 结论 | 去哪 |
|---|---|---|
| 没有 `HTML-PARSED` | 跑的是旧嵌入快照 | **Task 5A** |
| 有 `HTML-PARSED`，没有 `main.js 顶层到达` | module 图加载失败 | 看同段日志的 `SCRIPT-FAILED` / `ERROR` 行，按行号修；改完回 Task 3 步骤 3 |
| 有 `main.js 顶层到达`，没有 `initShell 返回` | `initShell()` 内部抛错 | 看 `ERROR` 行 |
| 有 `即将调用 showDomainGate`，没有 `showDomainGate 返回` | `showDomainGate()` 内部抛错 | 看 `ERROR` 行 |
| 有 `showDomainGate 返回，domaingate.hidden=false`，界面仍无门 | JS 正确，问题在渲染层 | **Task 5B** |
| 观察者报 `hidden 变为 true` 且无人点按钮 | 有第二处代码关门 | **Task 5C** |

---

## Task 5A: 改 dist 不触发重新嵌入

**症状：** `HTML-PARSED` 不出现，说明 `cargo build` 没把新的 `gui/dist` 嵌进去。

**根因假设：** `gui/src-tauri/build.rs` 现在只有 `tauri_build::build()`，
它对 `tauri.conf.json` 发 `rerun-if-changed`，但**不递归监视 `frontendDist`
指向的目录**。于是改前端不构成重新编译的理由。

- [ ] **步骤 1：先证伪 —— 确认这不是缓存问题**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
rm -rf ~/Library/Caches/edu.sysu.colm.desktop ~/Library/WebKit/edu.sysu.colm.desktop
```
再做标准重启。仍无 `HTML-PARSED` 才继续（WKWebView 缓存已在本次事故中排除过一次，
这步是为了让结论可复现）。

- [ ] **步骤 2：补 rerun-if-changed**

把 `gui/src-tauri/build.rs` 整个改成：

```rust
fn main() {
    // `tauri_build::build()` 只对 tauri.conf.json 发 rerun-if-changed，
    // **不递归监视 frontendDist 指向的目录**。不补这一行，改前端就不构成
    // 重新编译的理由 —— 症状是改了 dist 却一直跑着旧界面，而且 `Finished`
    // 照常打印，看不出任何异常。
    println!("cargo:rerun-if-changed=../dist");
    tauri_build::build()
}
```

- [ ] **步骤 3：验证它真的生效**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
sed -i '' 's/fp=A1/fp=A2/' gui/dist/index.html
cd gui/src-tauri && cargo build 2>&1 | grep -E "Compiling|Finished"
```

预期：出现 `Compiling colm-desktop-gui`。然后标准重启，日志应出现
`HTML-PARSED fp=A2`（**注意是 A2 不是 A1** —— 这证明的是「改动能进去」，
只看到 A1 说明拿的还是上一次的快照）。

- [ ] **步骤 4：如果仍然不行，用绝对判据看嵌入内容**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
rm -rf gui/src-tauri/target/debug/build/colm-desktop-gui-*
cd gui/src-tauri && cargo build 2>&1 | grep -E "Compiling|Finished"
```
`strings` 对这个二进制无效（压缩），所以判据仍然是日志里的 `fp=` 值。

- [ ] **步骤 5：提交**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add gui/src-tauri/build.rs
git commit -m "build.rs 补 rerun-if-changed=../dist

tauri_build 只监视 tauri.conf.json，不递归 frontendDist 指向的目录。
少这一行，改前端不构成重新编译的理由，而 cargo 照常打印 Finished ——
故障表现为「代码改了、界面没变」，且没有任何报错指向构建。

本次事故就是这么烧掉六轮排查的：前端逻辑、CSS、WebKit 缓存、元素 id
全查了一遍，全是对的。"
```

---

## Task 5B: JS 说门开了但看不见

**症状：** 日志有 `domaingate.hidden=false`，界面仍是主界面。

- [ ] **步骤 1：让前端自己报几何**

`gui/dist/app/main.js` 里 `showDomainGate 返回` 那行打点之后追加：

```js
{
  const g = document.getElementById('domaingate');
  const cs = getComputedStyle(g);
  const r = g.getBoundingClientRect();
  window.__probe('门的几何: display=' + cs.display + ' visibility=' + cs.visibility
    + ' opacity=' + cs.opacity + ' zIndex=' + cs.zIndex
    + ' 矩形=' + Math.round(r.width) + 'x' + Math.round(r.height)
    + ' 卡片数=' + document.getElementById('gatecards').childElementCount);
}
```

- [ ] **步骤 2：标准重启并判读**

| 几何 | 含义 |
|---|---|
| `display=none` | 还有别的 CSS 规则赢了 `.gate`，用 `cs` 逐条查 |
| `矩形=0x0` | 门在 DOM 里但没有尺寸，查 `.gate-panel` 的布局 |
| `卡片数=0` | `render()` 没往 `#gatecards` 填东西，回 `domain.js` 的 `render()` 查 |
| 都正常 | 门确实在画，问题在窗口层面（多显示器 / 窗口在屏幕外），此时才有理由请人肉眼确认 |

- [ ] **步骤 3：按判读结果修，提交**

提交信息里写清楚是哪一条几何值指向了根因。

---

## Task 5C: 有第二处代码在关门

**症状：** 观察者报 `hidden 变为 true`，而没人点过按钮。

- [ ] **步骤 1：把栈打出来**

Task 3 步骤 2 的观察者回调改成：

```js
new MutationObserver(() => {
  window.__probe && window.__probe('domaingate.hidden 变为 ' + gateEl.hidden
    + ' 栈=' + new Error().stack.split('\n').slice(1, 5).join(' <- '));
}).observe(gateEl, { attributes: true, attributeFilter: ['hidden'] });
```

- [ ] **步骤 2：标准重启，从栈里读出调用方，去掉那处调用，提交**

---

## Task 6: 撤掉打点，留下通道

**前提：** 门已经能显示（人肉眼确认，或 Task 5B 的几何全部正常）。

- [ ] **步骤 1：撤掉 `main.js` 里的四处打点和几何输出**

保留 `index.html` 里的探针 `<script>` 和后端的 `probe_log` —— 那是
`window.onerror` 通道，以后任何前端错误都能落到日志，值得常驻。
把 `send('HTML-PARSED fp=A2')` 改成 `send('HTML-PARSED')`（指纹是一次性的）。

- [ ] **步骤 2：保留 MutationObserver 吗**

不保留。它是为这一次的疑问装的，疑问回答完就是噪声。

- [ ] **步骤 3：全套检查**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
cargo run -p xtask -- check-gui
cd gui/src-tauri && cargo test 2>&1 | tail -5
```

预期：`check-gui` 静默通过；GUI 测试里 `histvars::the_same_variable_is_writable_under_the_bgc_kernel`
可能仍是红的 —— 那是宏改造第四组的遗留，与本计划无关，**不要在这里修它**。

- [ ] **步骤 4：标准重启，确认门还在，提交**

```bash
cd /Users/zhongwangwei/Desktop/Github/CoLM-Rust
git add -A gui/
git commit -m "撤掉一次性打点，保留 window.onerror 到 stderr 的常驻通道

打点是为这次的疑问装的，答完就是噪声。留下的是错误通道：以后任何
未捕获的前端错误都会落到 /tmp/gui.log，不用再从「界面没反应」这种
症状倒着猜。"
```

---

## 附：这次为什么值得留一条通道

排查向导不显示，前后查了六轮：前端逻辑（`showDomainGate` 无条件调用）、
HTML 元素（`#domaingate` 在）、CSS（`.gate[hidden]` 那条是对的）、WebKit 缓存
（清了）、二进制字符串（`strings` 因压缩而无效）、AX 树（时灵时不灵）。

**每一项单独看都是对的，合起来仍然不显示。** 缺的不是分析，是一条能说
「代码到底跑到哪一行」的通道。三条现成的观察手段在这台机器上全都不可用，
而这个事实本身也是查了几轮才确认的。

`probe_log` 加上去只有四行 Rust。
