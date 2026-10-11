# 界面重组：交接与剩余制作流程（给 Codex）

写于 2026-10-12。后端能力（模型版本、出处、服务器、研究快照、比较两个版本、服务器上的研究）都已完成并实测；剩下的主要是**把前端做成示意稿的样子**。本文是可以直接照着做的清单。

## 0. 起点

- 分支：`claude/run-mode-phase1`（基于 main `ffe9ab26`，只在本地，没推）。它正被 worktree `.claude/worktrees/xenodochial-hofstadter-7689bf` 检出，git 不允许同一分支在两处检出，所以在主检出里**另开一个分支接着做**：

  ```bash
  git switch -c codex/ui-redesign claude/run-mode-phase1
  ```

- 已有提交（从旧到新）：

  | 提交 | 内容 |
  |---|---|
  | `f86c0bdd` | 第 1 阶段：运行方式（版本 × 位置 × 引擎）、上下文条、`produced-by.json` |
  | `78e49ad6` | 第 2 阶段：改代码页（模型版本），采纳内核并入版本 |
  | `1e1ce850` | 第 3 阶段：计算资源页，修改版在服务器上跑 |
  | `707c1d67` | 第 4 阶段：研究固定版本快照、比较两个版本 |
  | `d6f4fbf1` | 第 5 阶段：研究在服务器上跑 |
  | `0fafc082` | T7920 真跑修改版的记录 |
  | `42c58c8f` | 重做第 1 步：结果页汇总点（运行列表、白话评估、问题在哪） |
  | `0a830156` | 重做第 2 步（一部分）：左栏扁平五步、总流程图页 |

- 实现记录：`docs/implementation-verification.md` 第 660–664 轮。**新工作从第 665 轮起记**。
- 运行软件看效果：

  ```bash
  cargo run -q --release -p xtask -- stage-sidecar
  cd gui/src-tauri && cargo build --release && ./target/release/colm-desktop-gui
  ```

  前端（`gui/dist`）在编译时打进程序，**改了前端要重新 `cargo build` 才看得到**。只看样式可以用 `python3 -m http.server --directory gui/dist`，在浏览器里用 `await import('/app/shell.js')` 调 `go('步骤')`；要假数据就复制一份 `index.html`，在第一个 `<script>` 前定义 `window.__TAURI__ = { core: { invoke: async (cmd) => … }, event: { listen: async () => () => {} } }`（用完删掉，别提交）。

## 1. 设计依据（不要改的约定）

- **每次运行 = 算例 × 模型版本 × 运行方式（在哪里 · 引擎）。** 研究（调参数与网络）产出新算例；改代码产出新的模型版本；两者平级，产出都回到第 3 步重跑，在结果页对比。
- 左栏：工作流 `1 建算例 · 2 参数 · 3 运行方式 · 4 运行 · 5 结果与评估`；`改进模型`（调参数与网络 / 改代码）；`其他`（计算资源、总流程图）。**已完成。**
- 结果页是枢纽：这个算例的所有运行（每条标出处）→ 评估摘要 → “下一步：问题在哪？”四个选项（参数不对 / 过程或结构不对 / 想知道稳不稳 / 公平比较），推荐项附理由。**已完成**（`gui/dist/app/results-hub.js`）。
- 修改版没有“采纳”“登记”：建好、编好就出现在运行方式里。修改版只在运行方式里选中时才用。
- AI 助手为所有流程服务；写操作每一步都经人确认；导出、撤回、删除是 D 级，只在界面上，助手调不到（`gui/tests/workspace.mjs` 断言）。

## 2. 现有代码地图

**命令行（`crates/colm-cli`）**
- `run --model-version NAME`：检查就绪后交给版本自己的 `colm-cli`；每次 `run` 写 `out/<名>/produced-by.json` 并追加 `<算例>/.colm/runs.jsonl`（`provenance.rs`、`case_runs.rs`）。
- `version-check`、`produced-by`、`case-runs`（结果页用）。
- `ws-create --from ws:NAME --note`、`ws-note`、`ws-status`（含 `dir`）、`ws-list`（含 `note`、`based_on`、`touches`）（`ws_cmd.rs`；版本判定在 `crates/colm-workspace/src/version.rs`）。
- `study-run --model-version`（第一次拍快照到 `<研究>/version/`，写 `model-version.json`）、`study-clone`、`study-compare`（`study_version.rs`）。`study-apply` 在另存的算例里写 `.colm-derived-from.json`。
- `remote-run`/`remote-kernel --model-version`；`remote-study-run`/`-status`/`-fetch`（`remote_cmd.rs`、`remote_study.rs`）。

**Tauri（`gui/src-tauri/src`）**：`version_check`、`case_runs`、`workspace_create`（含 `base`、`note`）、`workspace_note`、`workspace_check`、`workspace_import_patch`、`study_clone`、`study_compare`、`remote_study_run/status/fetch`；`run_case`/`run_batch`/`study_run`/`remote_run`/`remote_preview`/`remote_build_kernel` 都带 `modelVersion`。

**前端（`gui/dist/app`）**
- `run-mode.js`：纯函数（版本名、阻塞原因、上下文条段落、助手快照）。`run-mode-view.js`：运行方式页、上下文条、版本检查。
- `workspace.js`：改代码页（版本卡、本机检查、服务器检查、导出/撤回/删除）。
- `resources.js`：计算资源页。`remote-study.js`：研究交给服务器、轮询、取回。`version-compare.js`：比较两个版本。
- `results-hub.js`：结果页汇总点。`flow.js`：总流程图。
- 事件：`colm:step`、`colm:context`（左栏重画时）、`colm:versions`（版本有增删改）、`colm:use-version`（选中某版本并去运行方式）、`colm:assistant-prompt`（打开助手并填入请求）。

## 3. 剩余任务（按这个顺序做）

每项做完：跑第 4 节的检查，追加一轮记录，单独提交。

### T1 上下文条移到顶栏，改成下拉框

示意稿：顶栏 `CoLM Desktop | 算例 ▾ | 版本 ▾ | 在 ▾ | 引擎 ▾ | AI 助手`，所有页面共用，助手读它。

- `gui/dist/index.html`：把 `<div class="context-bar" id="context-bar">` 从 `<main>` 里移到 `<header class="top">`（放在“返回首页”之后，占剩余宽度）；当前算例的完整路径不再单独占一个输入框（`#casename` 可隐藏，路径放进“算例”段的 `title`）。
- `run-mode-view.js` 的 `renderContextBar()`：
  - “算例”：按钮，点了去 `basic-files`；批量时写“X 等 N 个”（`contextChips` 已有）。
  - “版本 / 在 / 引擎”：各是一个 `<select>`，选项**从运行方式页的 `#model-version`、`#run-target`、`#model-engine` 复制**；改了就设页面上那个 select 的值并 `dispatchEvent(new Event('change'))`——只有一份真实设置，两边都跟着变。
  - 修改版高亮；不能运行时在条尾标“还不能运行”（悬停写原因）。
- `style.css`：去掉 `.context-bar` 的 sticky 与负边距，改成顶栏里的一组紧凑下拉；窄屏时可折成只显示值。
- 验收：新增 `gui/tests/context-bar.mjs`（或扩展 `run-mode.mjs`）——顶栏里有四段；在顶栏改版本后 `#model-version` 的值和 `state.modelVersion` 跟着变；在运行方式页改了顶栏也变。浏览器里看两种主题、1240 与 960 宽度。

### T2 运行方式页改成卡片选择（示意稿 Main）

示意稿：标题“运行方式”，说明“这三项和顶部的上下文条是同一份设置：在这里改，上面跟着变”。三组**卡片**（不是下拉框）：
- 用哪个模型版本：官方版（“CoLM 2024 · 随软件发布，不可修改”，徽章“默认”）；每个修改版（副标题“只改了 Rust 引擎 / Rust 与 Fortran 都改了”，徽章“✓ 检查已通过 / ! 还没测试”）。右上“去改代码 →”。
- 在哪里跑：本机（“MacBook · 16 核”）、每台服务器（“64 核 · 在线 / Slurm 集群 · 在线”）。右上“管理计算资源 →”。
- 用哪个引擎：Rust 引擎（推荐）“单进程多线程；站点、网格、流域都能跑，与 Fortran 逐位一致”；Fortran 内核“需要多进程 MPI 时选；装了 AI 网络的算例不能用它跑”。
- 下面：同时运行的算例数、每个算例的线程数（Rust）/ MPI 进程数（Fortran），按引擎显示（现有逻辑在 `runner.js` 的 `syncParallelMode`）。
- 准备情况清单（现有 `#version-readiness`），底部“下一步：输出与运行 →”，有 ✕ 项时按钮变“先处理上面的 ✕ 项”。

做法：卡片是 `<button aria-pressed>`，点了设隐藏的 `#model-version` / `#run-target` / `#model-engine` 并派发 `change`（保留这三个 select 作为唯一状态，别的模块都读它们）。版本卡的徽章用 `ws-list` 的 `lights`（编译与测试都是 `pass` → “✓ 检查已通过”，有 `fail` → “✕ 检查没通过”，其余 → “! 还没测试”），副标题用 `touches`。服务器卡的“在线”用 `remote_probe` 的缓存结果（没测过就写“没测过”，不要每次进页面都连）。
验收：`gui/tests/runview.mjs` 里“版本、位置、引擎与并行设置只在运行方式里出现一次”的断言改成卡片；点卡片后三个 select 与 `state.modelVersion` 正确；i18n 通过。

### T3 版本详情：四个页签（示意稿 VersionDetail）

示意稿：返回“← 改代码”；标题 = 版本名，副标题 = “目的” · 基于哪一版；右上“用它运行 →”。页签 `修改 · 检查 · 对比 · 用到它的研究`。

1. **修改**：按钮“在右侧让助手改”（已有 `colm:assistant-prompt`）、“用我的编辑器打开”（显示/复制源码路径；有打开文件夹的 Tauri 能力就用）、“导入补丁文件…”（已有）。下面“相对‘父版本或官方版’的改动 · N 个文件”：每个文件一行，写“Rust 引擎 / Fortran 内核 · +6 −2”，两个按钮“看差异”“撤销这处”。底注“每次检查、运行或研究开始前自动拍快照”。
   - 新命令行：`ws-diff --name N --path P`（`git diff <官方基线> HEAD -- P`，输出文本，限长）；`ws-revert-file --name N --path P --message M`（把这个文件恢复成基线的样子并提交一次；路径必须在改动清单里，拒绝 `..`）。对应 Tauri `workspace_diff`、`workspace_revert_file`（撤销这处属于改动，要先 `appConfirm`）。测试放 `crates/colm-workspace` 或 `ws_cmd` 的测试里。
   - 文件归类用 `colm_workspace::version::touches` 的规则（`vendor/CoLM202X/`、`build_kernel.sh` 为 Fortran，其余结果路径为 Rust 引擎）。
2. **检查**：“在哪里检查”选本机或某台服务器（和运行方式同一组卡片）。四项，每项写“为什么要做”和当前状态：
   - 能不能编译（编译灯）——“Rust 引擎与 Fortran 内核都改了，两边都要编译”这类句子按 `touches` 写；
   - 自带测试是否通过（测试灯）；
   - 结果不变检查（两版一致灯）——改了物理时写“— 不需要（改了物理）”，对应 `not_needed`；
   - 和上一版差多少（与原版对比灯）——`stale` 写“! 上次结果对应旧源码，需要重做”。
   按钮调用现有 `workspace_check`（本机）或 `workspace_remote`（服务器，现有远程面板的逻辑）。
3. **对比**：两块——“快速对比”（同一个算例、同一组参数只换版本各跑一次：把当前算例另存一份副本，用父版本/官方版各跑一次，或最简单：说明后跳到运行方式选版本）；“公平比较（研究）”→ 去参数率定页的“比较两个版本”。
4. **用到它的研究**：哪些研究固定了这个版本、在哪里跑、快照时间、快照是不是仍是当前代码（`✓ 快照仍是当前代码` / `! 之后又改过`）。没有时写“还没有研究用过这个版本”，若有父版本就列父版本的。
   - 后端：在 `study_version::pin()` 里给工作区追加一行 `<工作区>/studies.jsonl`（研究目录、研究编号、提交、时间）；`remote_study::cmd_run` 带 `--model-version` 时也追加（位置写服务器名）。`ws-status` 返回 `studies`：读这个文件，去掉已不存在的研究目录，对每条比较记录的提交与当前 HEAD。测试：拍快照后 `ws-status` 能列出研究；改代码后标“之后又改过”。

页面形态：改代码页的版本卡点名字进入详情（`data-step="code"` 里加一个详情视图，或新增 `data-step="version"` 用 `state.versionDetail` 记当前版本），保留列表页。版本卡上现有的折叠块（本机检查、服务器检查、导出、撤回、删除）搬进对应页签；导出、撤回、删除放“修改”页签底部的“更多”里。

### T4 新建修改版：三步向导（示意稿 NewVersion）

示意稿：返回“← 改进模型 · 改代码”；标题“新建修改版”，说明“它会得到一份独立的源码。官方版和别的版本都不受影响。”三步进度条：
1. **起点**：卡片——官方版“从发布的 CoLM 2024 开始”；每个修改版“在已有修改上继续改 · 测试已通过”/“复制一份，分出另一个方向”。
2. **名称与目的**：名称（现有校验 `[A-Za-z0-9][A-Za-z0-9_-]*`，≤40）；“想改什么、为什么”（≤500 字），下注“这句话会写进这个版本跑出的每个结果里，几个月后回头看也知道它是什么。”
3. **怎么改**：卡片——“让 AI 助手改”（打开助手，它知道当前是哪个版本；每次改代码前都会请你确认）、“用我的编辑器打开”、“导入补丁文件”（已有 .diff/.patch）。底注“之后随时可以换方式。改动涉及 Rust 引擎、Fortran 内核还是两者，会自动识别并显示在版本卡上。”最后按钮“建好并开始修改 →”：调用现有 `workspace_create`（`base`、`note`），成功后进入该版本详情，按第 3 步的选择打开助手 / 显示路径 / 弹出选补丁文件。
“其他源码或版本（可选）”保留在第 1 步的折叠区里。
验收：`gui/tests/workspace.mjs` 里 `createWorkspace` 的调用参数断言改成走向导；三步可前进后退，名称不合法时不能到第 3 步。

### T5 研究设置加“运行方式”一步（示意稿 StudySetup）

示意稿（参数率定）：步骤 `1 目标与指标 · 2 参数范围 · 3 站点与时段 · 4 运行方式 · 5 开始`。第 4 步“和工作流第 3 步是同一个组件，默认取顶部上下文条。研究会自动加上自己的约束。”：用哪个模型版本、在哪里跑（每个位置写“预计 约 N 分钟/小时”和“同时 N 个成员”）、引擎（“参数率定可以用 Fortran；AI 两条研究会锁定 Rust 并说明原因”）、准备情况。
- 做法：把 T2 的卡片组件抽成可复用的函数（例如 `renderRunModeCards(container, { lockEngine })`），研究向导里用一份**研究自己的**选择（默认取上下文条），开始时把它传给 `study_run` / `remote_study_run`（`results.js` 的 `runStudy` 现在直接读全局选择，改成读这一步的值）。
- 预计耗时：成员数 =（差分进化）`population × (generations + 1)` 或候选上限；单成员耗时取这个算例最近一次运行的 `produced-by.json` 起止时间（没有就不写预计）；并行数：本机取“同时运行的算例数”，服务器取 `cpus / threads`；预计 = ceil(成员数 / 并行数) × 单成员耗时 × 站点数。只是估计，标“约”。
- 注意 `gui/tests/results.mjs` 断言向导是 7 页（`data-study-step` 0–6），加一步要同步改测试与 `setStudyWizardPage` 的页数。不确定性分析同样加这一步。AI 学习参数（`hybrid.js`）走的也是研究：引擎锁 Rust。

### T6 计算资源页对照示意稿（示意稿 Resources）

示意稿每张资源卡：名称、状态（“✓ 就绪 · 空闲 / ✓ 在线 · 1 分钟前测试 / ! 在线 · 缺 1 项数据对应”）、登录方式与系统（“SSH 配置登录 · Linux · 64 核 · 无调度系统 / 密钥登录 · Slurm · 分区 cpu · 每个作业最长 04:00:00”）、数据对应清单（✓ / ✕ 每项，✕ 项有“告诉我它在服务器上的位置 →”）、“正在用它的”（运行中的研究或检查）、“已就绪的版本”（其余第一次用时自动编译）、按钮“测试连接 / 编辑 / 清理不再使用的版本”。本机卡写“已就绪的版本：官方版、…”。
- 现有 `resources.js` 已有：设为运行位置、检查连接、查看内核、修改、编 Fortran 内核。要补：状态行（缓存上次探测结果与时间，存 localStorage）、数据对应清单（来自服务器设置的 `maps`；本机路径存在与否可以本机判断，服务器侧存在与否用 `remote_probe` 或一次 `test -e`）、正在用它的（扫描本次会话的远程运行记录 `.colm-remote.json` 与 `.colm-remote-study.json`）、已就绪的版本（服务器 `engine/` 下的快照与版本快照号对应，需要一个只读命令列出服务器上的引擎快照，例如 `remote-engines --host H --root R`）。
- “清理不再使用的版本”涉及删除服务器上的目录，属于 D 级：只在界面上、逐个确认、只删 `<根>/engine/<快照>`。

### T7 AI 助手：每页停靠的右栏，写明看得到什么、附依据

示意稿：右栏常驻，顶部“看得到：上下文条 · 本页…（按页面变）”，回答末尾“依据：…”，回答下方给可点的下一步动作（如“预编译到 7920”“用 7920”）。
- `assistant.js`：宽屏（≥1400px）默认展开并记住用户的开关；面板头下加一行“看得到：…”，内容由 `currentView()` 已有的信息生成（上下文条四段 + 当前页名 + 有无指标/研究/版本检查）。
- `crates/colm-agent/src/prompt.md`：要求回答结尾用一行“依据：…”列出用到的工具结果或页面数据；建议的写操作以可点动作给出（仍走现有审批）。
- 验收：`gui/tests/assistant.mjs` 断言“看得到”行随页面变化；助手测试 `cargo test -p colm-agent` 通过。

### T8 收尾

1. T7920 上真跑一次 `remote-study-run`（修改版，参数率定，小预算），取回后 `study-compare` 与本机同设定的研究比较（x86 与 Apple Silicon 不逐位一致，比较在容差内看结论是否一致），记一轮。
2. 检查主检出 `main` 有没有别人的新提交；有就先合进来、解决冲突（`runner.js`、`shell.js`、`results.js`、`assistant.js`、`workspace.js`、`remote.js` 最容易冲突），再合并到 main、推送。

## 4. 每项都要过的检查

```bash
cargo fmt --all -- --check && (cd gui/src-tauri && cargo fmt --check)
cargo clippy --workspace --all-targets -- -D warnings
(cd gui/src-tauri && cargo clippy --all-targets -- -D warnings)
cargo test -p colm-workspace --lib
cargo test -p colm-cli --bin colm-cli -- --test-threads=1
cargo test -p colm-cli --test study_cli_tuning -- --test-threads=1
(cd gui/src-tauri && cargo test)
for t in gui/tests/*.mjs; do node "$t" >/dev/null || echo "FAIL $t"; done
cargo run -q -p xtask -- check-gui
```

合并前跑全套（再加：其余 cargo 测试、`--features colm-numeric/unfused`、`python3 -m pytest -q packaging/python/tests`、`scripts/test-terminal-assistant.py --agent target/debug/colm-agent --cli target/debug/colm-cli`）。

## 5. 容易踩的坑

- **英文**：`gui/tests/i18n.mjs` 要求 `app/*.js` 里每句中文都有英文。固定句子加进 `i18n.js` 末尾的对照表（在 `].sort(` 之前）；带变量的句子在 `translateZh` 的正则段里加 `.replace(/^…$/, …)`；中文前缀与变量拼在一起的，拆成两个文本节点分别翻译。日期用 `toLocaleDateString`，不要手写“月/日”。
- **测试在沙箱里切片源码**：`gui/tests/results.mjs` 会按函数名把 `results.js` 的一段切出来在 `vm` 里跑（例如 `runStudy` 到 `retryStudy` 之间）。往这些函数之间插新的导出函数、或给它们加新依赖，要同步改测试给的上下文。
- **左栏结构有测试**：`results.mjs` 断言五步与各分段；改左栏要同步改。
- **Windows**：只在 Unix 测试里用到的辅助函数要 `#[cfg(unix)]`；假程序用 sh 脚本的测试限定 Unix。`cargo fmt --all` 不管 `gui/src-tauri`，要另跑。
- **CoLM 路径上限 256 字节**：端到端测试的算例与“服务器”目录要短（macOS 的临时目录很长）。
- **验证“确实跑了改过的程序”**要选每一步都生效的改动（例如 `ground_fluxes.rs` 的冯·卡门常数），只在特定条件下触发的改动可能逐位不变。
- **服务器**：T7920 用 `7920ts`（备用 `7920mp`），只在 `/media/zhwei/data02/zhwei/colm-desktop/` 下工作。
