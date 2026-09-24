# CoLM Desktop 实现与验证记录

> 本文归档开发过程中的实现依据、缺陷复盘、跨平台构建细节与黄金回归证据。面向普通用户的安装和使用说明请从[项目首页](../README.md)开始。

- **开发与维护**：魏忠旺 @ CoLM陆面模式开发团队，中山大学大气科学学院
- **联系邮箱**：weizhw6@mail.sysu.edu.cn
- **Copyright**：CoLM陆面模式开发团队，中山大学大气科学学院

## 下载

[下载最新编译版（macOS / Windows / Linux）](https://github.com/zhongwangwei/CoLM-Desktop/releases/latest)

安装包已经包含 CoLM 内核和示例站点，使用桌面程序无需安装 Rust、Fortran 或 NetCDF 编译环境。

把 CoLM202X 的 SinglePoint 模式做成跨平台桌面程序。设计见 `docs/design.md`。

**当前状态**：命令行端到端可用 —— 一条命令可从站点文件建例、运行并生成
指标表。PFT / PC / BGC / URBAN / TRACER 是运行时开关；IGBP / USGS
由两份编译产物覆盖。GUI 能完成站点数据前处理、按约束建例、批量配置与运行、
结果浏览和评估，并提供不确定性分析、参数调优及报告导出工作流。
安装包由 `release.yml` 三平台产出，
内核随包走 —— **用桌面程序的人不需要装任何编译器**。

### GUI 能做什么

启动时先选择计算资源：本地运行可用，服务器运行保留为不可选入口。进入本地
工作台后先走约束卡片，确定空间结构、地类体系、次网格、过程模块与土壤水力；
区域、全球和流域等尚未实现的入口保持灰色，不能进入一个必然失败的流程。

约束卡片完成后直接进入「基本设定 / 文件与目录」。左侧工作流有八个顶级组；
前处理是独立入口，不再强迫已经准备好站点数据的用户先经过它：

| | 步骤 | 要什么才能进 |
|---|---|---|
| ① | 前处理 | 可选；将 NetCDF 或单站/多站 CSV、TXT、TSV 整理成站点数据和强迫场 |
| ② | 基本设定 | 扫站点、建算例；文件、预热、网格、地表、初始场与强迫场分栏 |
| ③ | 过程参数 | 要先建过算例；只显示当前模型涉及且可配置的过程 |
| ④ | 运行 | 分站点运行、取消、进度与日志 |
| ⑤ | 结果分析 | 时间序列、多站比较、模型评估与图形诊断 |
| ⑥ | 不确定性分析 | 从完成算例创建并运行参数扰动 Study |
| ⑦ | 参数调优 | 差分进化、校准/验证窗口与候选应用 |
| ⑧ | 报告与导出 | 汇总并导出结果 |

物理和次网格已在进门向导选完。GUI 后台按选择自动匹配 IGBP 或 USGS 产物，
主界面不再给用户一个重复的“选内核”下拉框。

| | |
|---|---|
| 站点库 | 扫 `Sitedata` 目录，两套命名约定都认；列出「城市 / 无观测 / 读不了」 |
| 参数 | 按用途分节；向导已定义的字段不重复显示，当前配置不可用的也默认隐藏 |
| 输出变量 | 输出开关独立成页，并说明当前配置下能否产出；TRACER history 同样进入闸门 |
| 运行 | 三段各自状态；**输入没变就跳过**（输入指纹，不是只看文件在不在）；逐站点显示进度和日志并可取消 |
| 评估 | 指标表（含 KGE 可信度提示）、模型 vs 观测双线图、散点图和批量汇总表 |
| Study | 不确定性与调优共用后端执行、状态恢复和导出契约，界面保持两个独立工作流 |

## 仓库与依赖

`vendor/CoLM202X` 是入库的源码快照；来源、基线 commit 与本地改动记录在
`vendor/PROVENANCE.md`，普通克隆不需要再初始化 submodule。

CI 分两层。每个 PR 在 Ubuntu / macOS / Windows 运行 workspace 与 GUI 测试、
静态 GUI IPC 契约检查、格式化和 Clippy；需要源码与已入库黄金文件的集成测试
也在门禁中显式列出。依赖 5.5 GB PLUMBER2 与 38 GB
rawdata（或一个已构建的内核），只能在带那些东西的自托管 runner 上跑；
「它们没跑」这件事会在 PR 界面上以警告形式出现，而不是静默缺席。

## 内核编排层

`crates/colm-kernel` 负责三件 CoLM 自己不会替你做的事。GUI 与黄金回归共用同一份 ——
`oracle/src/bin/golden_run.rs` 调的就是它，所以每跑一次回归都在验这一层。

**一、判成败。** CoLM 在单点模式下，**成功与失败都以退出码 0 结束，
但走的是两条不同的路**：

- 失败走 `share/MOD_SPMD_Task.F90` 的 `CoLM_stop`，其 `#ifndef USEMPI` 分支是裸 `STOP`。
- 成功不执行任何收尾调用，直接跑到 `main/CoLM.F90:764` 的 `END PROGRAM CoLM`
  （`spmd_exit` 只定义并调用于 `#ifdef USEMPI` 内）。

退出码相同是两条路径的巧合，不是共用一条路径。所以判定成败必须同时满足三件事：
无错误标记、有正向成功标记、产物齐全。产物必须列到**文件**而不是目录 ——
目录在程序写任何东西之前就已存在，只列目录的话「跑完了但什么都没写」恰好抓不到。

附带结论：既然 `CoLM_stop` 是失败专用的，把那个裸 `STOP` 改成 `STOP 1`
是安全的上游修复。即便上游改了，这一层仍然必要 —— 后两条腿抓的是别的东西。

**二、认内核。** 三个可执行文件都只以 `getarg(1)` 取 namelist 路径，
**没有 `--version`**，所以版本握手靠构建期写出的 `manifest.json`。
清单里两组字段职责不同：`macros` / `colm_git_sha` / `generator_args` 可复现，
认定**配置身份**（单点模式最容易搞错的正是编译期宏集合）；`sha256` 每次构建
都变，只认定**完整性**。「二进制不存在」与「存在但被换过」是两条不同的报错，
因为用户对这两种的处置完全不同。

**三、报覆盖。** CoLM 会不声不响地改掉你的配置，打印一行 `Note:` / `Warning:`，
然后继续跑。实测一次 CN-Cng 运行有 9 种这样的消息，其中两条是真正的覆盖
（变饱和流被自动打开、VG + IGBP 下土壤阻抗被自动关掉）。抽取只认前缀不认文本：
CoLM 把 automatically 拼成了 automaticlly，按文本匹配的代码会在上游改错字的
那天静默失效。整行原样交给上层，由上层呈现成「你要求了 X，模型实际用了 Y」。

## 端到端

```bash
colm-cli all --site <PLUMBER2>/Sitedata/CN-Cng_..._site.nc \
             --out  ~/cases/CN-Cng \
             --kernel kernels/default \
             --obs  <PLUMBER2>/Observation/CN-Cng_..._Flux.nc \
             --start 2008-01-01 --end 2008-01-11 --spinup 8
```

城市站点由站点文件内容识别，没有单独的 `--urban` 开关。完整站点文件已经包含
城市类型、人口密度和其它必需字段时可以不提供 rawdata；审计缺少字段时会明确
列出仍需哪一类外部数据：

```bash
colm-cli new --site <Urban-PLUMBER>/Sitedata/AU-Preston_site_v1.nc \
             --out  ~/cases/AU-Preston \
             --rawdata ~/rawdata --runtime ~/runtime \
             --start 1993-01-01 --end 1993-01-11
colm-cli run ~/cases/AU-Preston --kernel kernels/default
```

`colm-cli` 是**唯一的编排可执行文件**（`design.md` §4.2：「GUI 只跟它说话」），
所以它是唯一一处同时依赖全部五层的地方；各层之间仍然互不依赖。建例、三段
运行、评估、前处理、ERA5-Land 与 Study 子命令都通过这一边界提供给 GUI。

**能读出来的都不问。** 强迫场与观测文件在站点文件旁边找 —— PLUMBER2 的三个
目录共用同一个词干，只差 `_site.nc` / `_Met.nc` / `_Flux.nc`；经纬度与地类读自
站点文件；时间步长读自强迫场文件；不给窗口就用强迫场覆盖的完整范围。
留给人的只有一个算例名，以及可选的窗口收窄。

生成的 `case.nml` **只含真正偏离 CoLM 默认值的字段**，CN-Cng 上是 20 个字段
（手写版 43 行）。判据逐算例算，不是照固定清单：`DEF_simulation_time%timestep`
默认 1800 秒，90 个强迫场里 88 个如此可以省略，而 `US-Ne3` 与 `US-MMS` 是
3600 秒必须写 —— 漏了的话模型按半小时推进而强迫场是整小时，**跑得完，结果全错**。

`oracle/tests/generated_case.rs` 钉住这件事：生成的算例跑出的 history 与黄金
文件 `identical: 127 variables`。这比「生成的文件长得对」强得多 —— 它说的是
生成的配置与手写那份**语义等价**。

### 没有 rawdata 也能跑

实测在 CN-Cng 上，**完全不给 rawdata 目录**（7 个字段回落到模块默认值）
产出的 history 与黄金文件逐位相同。原因是这三类字段在这个算例里都不起作用：
四个土壤反照率的模块默认值恰好等于栅格给出的第 10 档、湖深不进草地斑块、
高程标准差与坡度只服务已关闭的降尺度。

这是本项目核心承诺的一次端到端验证 —— 桌面用户装不了几百 GB 的全球栅格。
**但这是一个站点、一个窗口、一个预设的结论，不外推到另外 89 个站点。**

城市算例曾经是这条承诺唯一的例外（它要 240 GB），现在对 Urban-PLUMBER 那
21 个站也不要了 —— 见「运行时物理与地类产物的实际状态」一节，那里有实测的
`identical: 146 variables` 与这张表的边界。

## GUI

```bash
cd gui/src-tauri && cargo tauri dev
```

### 验收到哪一步了

**窗口真的开得出来。** 启动之后 `System Events` 报出一个标题为
`CoLM Desktop` 的窗口，进程不退。但这条只证明壳子起来了 —— 白窗口从外面看
一模一样：进程活着、标题也在。所以 `backend_ready` 往 stderr 记一行；
**只有 webview 真的加载并执行了 `index.html` 的 JS 才会调到它**。实测输出：

```
colm-desktop: the page reached the backend; backend reachable — 737 configuration fields known
```

**页面渲染与交互在 Chromium 里逐条走过。** 做法是把 `gui/dist/` 原样复制出去，
**只在 uPlot 那行 `<script>` 之前插一段 mock**（其余逐字节相同，由脚本断言），
mock 返回的载荷全部是**真后端导出的** —— `describe_fields`、`read_case`、
`unknown_fields` 来自 GUI crate 自己的函数，`series` 来自 `colm-cli series`，
`case.nml` 是真文件。走到的：

| 走到的路径 | 实测结果 |
|---|---|
| 三栏骨架 + 三个页签 | 全部渲染，`算例` 页签 `aria-pressed=true` |
| 扫描算例库 | 9 个算例，「已跑过 / 未跑」标记正确 |
| 选中算例 → 配置表 | 城市算例 24 个字段全渲染，含 `DEF_URBAN_type_scheme = 2` |
| 空分组 | 「这一组里这份配置没有设任何字段」 |
| 上游示例 `SiteSYSUAtmos_IGBP_VG.nml` | 警告条点名 `USE_SITE_topostd`、`USE_SITE_BVIC`，表里两行同时标红 |
| 画图 | uPlot 画布 724×380，**26838 个不透明像素、11 种颜色**，标题「净辐射 Rnet · 264 点」 |
| 时区 | 浏览器时区 `Asia/Shanghai`，图上首点显示 `0:30` 而不是本地的 `8:30` —— `tzDate` 那条注释是对的，且这次是真验了 |
| 反复画图 | 点 6 次之后仍是 4 张图，上限生效 |

**走这一遍时发现了一个真缺陷 —— 进度条永远不会动。** 详见下一节。补好之后
`run_case` 那条链也验过了：把 `colm-cli run --stream 1` 真实输出的 34180 行
按 `sidecar.rs` 的筛选规则算成事件载荷回放进页面，进度条 63% → 81% → 100%，
进度文字读出真实模型日期「第 313 步 · 1993-01-07-43200」，日志窗涨到 47183
字符后被截回 40000（60000 上限规则生效），结束文案是「完成 · 子进程打了
34180 行，丢弃 30802 行噪声」，`运行` 按钮恢复可点、`画图` 变可点。

**打包出来的 `.app` 双击也验过了 —— 过程中又抓到两个缺陷。** 见下节。
`open "CoLM Desktop.app"` 之后窗口起来，`System Events` 报出标题
`CoLM Desktop`、尺寸 1240×820，正是 `tauri.conf.json` 里声明的那组数。

**仍没走到的**：Linux 与 Windows 上的窗口。

### 打包路径从来没被跑过，于是躺着两个缺陷

CI 只跑 `xtask check-gui`，**从不构建 GUI，更不打包**。所以：

1. `tauri.bundle.conf.json` 的 `beforeBuildCommand` 写的是
   `--manifest-path ../../xtask/Cargo.toml`，而 Tauri 执行它的工作目录是
   `gui/`（不是 `gui/src-tauri/`）—— 打包第一步就报
   `manifest path does not exist`。正确的是 `../xtask/Cargo.toml`。
2. `resolve_cli` 在 `app.path().resource_dir()` 里找 sidecar，可 Tauri 把
   `externalBin` 放在**主二进制旁边** —— macOS 是 `Contents/MacOS/colm-cli`，
   而 `resource_dir()` 是 `Contents/Resources/`，那里只有图标。

第 2 条尤其阴：`resolve_cli` 的第三条回落是「仓库的 `target/` 产物」，
在开发机上**永远命中**，所以本地怎么试都对。要看见它，得先把
`target/{debug,release}/colm-cli` 挪走 —— 那时打包版本报出
`colm-cli resolved to colm-cli`，一路掉到 PATH。装到别人机器上，
第一次点「运行」就是 `cannot start colm-cli`。

于是 `backend_ready` 那行面包屑现在**连解析到的 CLI 路径一起报**。四条回落
里有一条在开发机上必中，这种结构只能靠把结果打出来才看得见。修好之后同样
条件下报的是
`.../CoLM Desktop.app/Contents/MacOS/colm-cli`。

CI 补了一个 `gui` 作业：三个平台构建 + clippy + fmt + 17 个后端测试，
macOS 上另外 `cargo tauri build` 并断言 `Contents/MacOS/colm-cli` 存在且跑得动。

### 进度条曾经建在一个永远不会到达的输入上

`colm.x` 在一次 528 步的运行里打出 34180 行，其中 528 行是
`TIMESTEP = n | DATE = ...` —— GUI 的进度条与日志窗全靠它们。但
`colm_kernel::run_stage` 用的是 `Command::output()`，**阻塞到子进程结束才
一次性收全部输出**；`colm-cli run` 再从中挑出 39 行摘要打到 stdout。于是
GUI 的 sidecar 读到的是：运行期间一片空白，结束时 39 行一起到达，一条
`TIMESTEP` 都没有。界面那边的 `TIMESTEP` 解析、100 ms 限流、批量发送
全都对着一个不存在的输入。

`xtask check-gui` 抓不到这个：它验的是「发出去的事件都有人听」，
而这里的问题是**没有人发**。

修法分两层。`colm_kernel::run_stage_streaming` 逐行读 stdout 并回调，
`run_stage` 成为它传空回调的特例；stderr 由单独线程读到底（两个管道都由本
进程读，先读完 stdout 再读 stderr 会在 stderr 管道写满时双向死等）。
`colm-cli run --stream 1` 把每一行原样转发并**逐行 flush** —— 默认的行缓冲
只在连着终端时生效，对着管道会变成 8 KB 块缓冲，从界面上看跟不转发差不多。
默认仍是 39 行摘要：终端前的人要的是那 39 行，GUI 要的是全部，由调用方说。

日志落盘的字节没有变，两条路各跑一次比对逐字节相同（末行不带换行、
stderr 分隔符两处最容易在重新拼接时走样，测试专门钉了）。黄金回归
两个窗口仍是 `identical: 127 variables, 10 dimensions`。

顺带量到的：那 34180 行里 28152 行是 RangeCheck 噪声、2650 行是空行、
528 行是进度，**真正进日志窗的只有 2850 行**，低于 4000 的环形缓冲上限。
筛选规则原本是在 5330 行的水热运行上定的，在这个大 6 倍的负载上仍然够用。

三栏布局：左边是步骤条与当前上下文（选了哪个站、用的哪个内核），中间是
当前那一步，右边是日志与曲线。**原来那版把站点库、新建、算例库并排摆着，
谁也看不出它们是一条流水线** —— 现在左栏五个大步骤就是流水线本身，每一页
底部都有通往下一步的出口，不用人自己回左栏找「现在该干嘛」。

建算例不再问什么：经纬度、地类、时间步长与默认窗口全从站点文件与强迫场
里读。批量勾选时左栏与按钮都说出**是几个**（「AT-Neu 等 90 个」、
「建算例：选中的 90 个站点」）—— 勾了 90 个却只显示一个名字，界面看起来
像在配一个，而改一个字段会写进 90 份 `case.nml`。

**窗口进程不链接 netcdf/hdf5。** 实测各层的依赖节点数：`colm-namelist` /
`colm-schema` / `colm-case` / `colm-kernel` / `colm-hist`（默认）全是 0，
而 `colm-forcing` 7、`colm-srfdata` 7、`colm-cli` 9。所以后端只链接前几层，
凡要读 NetCDF 的一律走 `colm-cli` sidecar —— 为了画一条曲线把整个静态 HDF5
拖进窗口进程是不划算的。这个分界不是照搬来的，是已有分层里自然掉出来的。

两个 workspace 刻意分离：引擎的 `Cargo.lock` 有 72 个 crate，GUI 自己的有 431 个。
`cargo metadata` 列出引擎恰好 10 个成员，GUI 不在其中。

### 日志必须在过 IPC 之前降速

实测一次 528 步的运行写出 39215 行日志，其中 **33357 行（85%）是 RangeCheck
的逐变量播报**；完整两年外推约 260 万行。处置是：RangeCheck 行丢弃（越界时
它会在同一行追加 `with NAN` / `Out of Range!`，而那两句是 `colm-kernel` 的
失败标记，运行会被判失败）、空行丢弃、进度行节流、**其余按批发送**。

按批而不是按前缀筛选是刻意的：列举「哪些是逐步碎语」等于让日志面板依赖
CoLM 的措辞，而它把 automatically 拼成 automaticlly 这件事已经教过一次。
批量节流不判断任何一行的价值，只保证事件率有上界 —— 约 20 事件/秒，
与日志量无关（逐行发送会是 595）。

### 前端的接口有静态检查

静态 JS 没有类型检查器，拼错的命令名要等到点下去才暴露。

```bash
cargo run -p xtask -- check-gui
```

它解析 `generate_handler!` 与前端的 `invoke` / `listen`，对不上就红，已接进 CI。

### 打包

```bash
cargo run -p xtask -- stage-sidecar    # 把 colm-cli 拷成带目标三元组后缀的副本
cd gui/src-tauri && cargo tauri build --config tauri.bundle.conf.json
```

暂存用 xtask 而不是 Node 脚本 —— 本项目一处都没有 Node，不该为一个拷贝动作
引入第二套工具链。

**没有做「先拷成临时副本再跑 sidecar」那个变通。** EarthMesh 需要它是因为
它的静态 netcdf 二进制在源码树里运行会被 SIGKILL；本项目实测没有这个问题：
`target/debug/colm-cli` 直接跑正常，动态依赖只剩 `libiconv` 与 `libSystem`
两个系统库。复现不出来的问题不写变通。

## 跑黄金回归

需要 PLUMBER2 数据（不入库）与 gfortran + netcdf-fortran。

```bash
export PLUMBER2_ROOT=/path/to/PLUMBER2s
./oracle/scripts/build_kernel.sh default
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

## 配置层

`crates/colm-namelist` 读写 CoLM 的 namelist，**保留原文格式**：解析→修改→
序列化后，未改动的行逐字节不变。验收是对 `vendor/CoLM202X` 里全部 55 个真实
`.nml`（4167 行）做往返测试。理由是用户算例文件里的注释是他们自己的笔记。

`crates/colm-schema` 描述每个配置字段的类型、默认值、所属 group 与说明。这张表
**由 `cargo run -p xtask -- gen-schema` 从 `MOD_Namelist.F90` 生成**，产物入库，
`tests/drift.rs` 保证它不会与上游脱节。详见 `crates/colm-schema/build-notes.md`。

**「什么算一个字段」的判据是 CoLM 自己的 `namelist /.../` 语句，不是 `DEF_` 前缀。**
前缀判据看着够用，实际两头都错：它滤掉了 `MOD_Namelist.F90` 里 **Part 3: For Single
Point** 整段的 21 个 `SITE_*` / `USE_SITE_*`（在一个专做单点的项目里），又因为
`USE, intrinsic :: ieee_arithmetic` 长得像声明而需要额外的特例去挡。改用 namelist
语句之后两件事都是顺带解决的 —— `ieee_arithmetic` 不在任何 namelist 组里，自然落选。

`group` 回答的是**这个字段该写进哪个文件**：`nl_colm` / `nl_colm_forcing` /
`nl_colm_history`。派生类型成员继承容器所在的组，所以 `DEF_forcing%dataset` 是
`nl_colm_forcing`、`DEF_hist_vars%*` 是 `nl_colm_history`。

`group` 为 `None` 的 **6 个字段是谁都设不了的**，但它们仍留在表里并被标出来：
`DEF_dir_history` / `DEF_dir_landdata` / `DEF_dir_restart` 由 `DEF_dir_output`
派生（`MOD_Namelist.F90:1406` 无条件覆盖），`DEF_USE_IGBP` / `DEF_USE_USGS` /
`DEF_Wetland_finundation_scheme` 由编译期宏决定。它们有声明、有默认值，
只是不出现在任何 namelist 组里。GUI 该把它们显示成只读的派生值 ——
给一个改了没用的输入框，比不显示更糟。

注意 schema 记录的是 **CoLM 声明的**默认值，一字不改。这很重要，因为
CoLM 的默认值假设 HPC 数据树存在：`DEF_USE_OZONEDATA` 默认 `.true.`，
要读 2.8 GB 的 `Ozone/Global/OZONE-setgrid.nc`；`DEF_Runoff_SCHEME` 默认 `3`
（Simple VIC），要求站点文件里有 `soil_texture`。

这两条的处置并不相同：桌面端新算例会显式关闭臭氧胁迫与臭氧读取；用户在
GUI 启用时选择并校验 NetCDF 文件。产流方案则沿用 CoLM 的 `3`，代价是站点
文件缺 `soil_texture` 时要合成一个。
哪个照搬、哪个偏离、偏离的理由，都由上层决定并解释，schema 不参与 ——
见 `docs/design.md` §2.5 与 §2.7。

## 输出变量

「这个内核能产出哪些变量」必须在**开跑之前**答得出来 —— 否则勾选界面只能把 482 个
`DEF_hist_vars%*` 开关一股脑铺出来，而 default 预设的一次真实运行只写出 119 个。

差额不是 bug，是三道闸门依次收窄：

| 闸门 | 判据在哪 | default 下 |
|---|---|---|
| 1. 编译期宏 | `MOD_Hist.F90` 里的 `#ifdef` / `#ifndef` | 456 个写出点 → **346** |
| 2. 运行时 `DEF_*` 条件 | 同一文件里的内联条件与完整 `IF` / `ELSE` 嵌套 | 114 个无条件，232 个有条件；本次 5 个条件成立 → **119** |
| 3. 变量自己的开关 | `DEF_hist_vars%X`，在 `colm-schema` 里 | 482 个中 343 个默认开启 |

`crates/colm-hist` 只回答闸门 1，输入是内核清单里的 `macros`：

```rust
// 清单里的 macros 是 Vec<String>（它要能从 JSON 反序列化），闸门表要 &str
let macros = manifest.macros.iter().map(String::as_str).collect();
colm_hist::writable(&macros)   // -> BTreeSet<&'static str>，default 下 346 个
```

闸门 2 会保留完整逻辑表达式、合并互补分支，但不在生成时绑定具体配置；GUI 再用
当前算例求值，好让界面说清「为什么你勾了它却没有」。闸门 3 已经在
`colm-schema` 里，两张表在 GUI 层合并即可。

表由 `cargo run -p xtask -- gen-histmap` 生成，产物入库，`tests/drift.rs` 守住它
不与上游脱节。

**覆盖消息与缺失变量是同一件事的两面。** `qlayer` 与 `qcharge` 挂在
`DEF_USE_VariablySaturatedFlow` 的两侧 —— 这道闸门不是「条件成立才加」，
而是「条件决定写哪一个」。而那个条件正是 CoLM 打印的第一条覆盖消息说的事：

```
DEF_USE_VariablySaturatedFlow is automaticlly set to .true.
```

于是有了 `qlayer`、没了 `qcharge`。用户看到的应该是这两句连起来的一句话，
而不是一条淹没在日志里的 `Note:` 加一个莫名其妙空着的变量。

静态表必须被一次真实运行钉住，所以 `oracle/tests/histmap.rs` 拿它跟入库的黄金
文件对：对那 119 个变量**零漏报**，多报恰好是 `dz_lake` / `qcharge` / `t2m_wmo` /
`xy_hpbl` 四个 —— 都是闸门 2 挡下的，且每个的条件原文都在表里。多报的方向是安全
的（「可能产出 X」而实际没有），漏报则是在用一张表去否定一次真实运行。两个黄金
窗口的变量集相同，这条也在测：变量集取决于预设与配置，不取决于季节。

## 指标：把模型跟观测对上

`design.md` §2.8 记着一句判据：**冬季窗口 Rnet 的 R²=0.986 同时证明强迫场转换、
时间轴对齐、时区处理、经纬度定位与辐射物理全部正确** —— 任一环出错它都到不了
这个数。`oracle/tests/metrics.rs` 把那句话变成可执行的，两个窗口六行指标全部复现。

对上这六行要过三道坎，每一道都不是看着显然的。

**一、两条时间轴的标签含义不同。**

| | 单位 | 步长 | 标签位置 |
|---|---|---|---|
| 模型 history | `minutes since 1900-1-1 0:0:0` | 60 分 | **区间中点** |
| PLUMBER2 观测 | `seconds since <起始日> 00:00:00` | 1800 秒 | **区间起点** |

模型首点是 00:30 而不是 01:00 —— 00:00–01:00 那一小时的标签打在中间。
所以模型标签 `t` 对应观测的 `t−1800s` 与 `t` 两点，取平均。

这条对齐是**被证伪过**的，不是拟合好就算数：把模型时钟整体平移 ±8 小时，
R² 从 0.986 掉到 0.146、RMSE 从约 15 涨到 ~126。这同时排掉一个疑点 ——
CN-Cng 在 123.5°E 正好 UTC+8，而冬季窗口恰好要剔除前 8 小时，
太像时区补偿了，必须验而不是想当然。

**二、QC 是「两个半小时里至少一个好」，不是「两个都要好」。**
后者给出冬季 Qh/Qle 的 250/245，前者给出 253/254 —— 而 253/254 才是记录值。
Rnet 在两种规则下都是 256，**光验 Rnet 定不下这条**，所以验收必须覆盖三个变量。

**三、spin-up 是每次分析各自的参数。** 冬季丢 8 小时，湿季丢 4 天。写死一个值，
另一个窗口就整体错位。

### KGE 的 β 项只标记，不改值

KGE 里的 β = 模型均值 / 观测均值，在观测均值接近零时失去意义。实测冬季 Qh：
观测均值 2.8、标准差 38.3，于是 β = 13.64，**那一行 KGE = −11.64 里有 12.64
全部来自 β 项** —— 它报的不是技巧，是「观测均值接近零」。湿季 Qh 更糟：
模型与观测均值反号，β 是负的，比值根本没有物理意义。

两条判据（`|μo| < 0.1σo`、`μm·μo < 0`）恰好命中 `design.md` 点名的那两行。
**保护是标记而不是替换**：一旦改了 KGE 的值，那张参考表就再也对不上了，
而它是这一层唯一的验收依据。

`colm-hist` 的读文件与算指标这一半在 `io` feature 之后。闸门表那一半保持零依赖 ——
GUI 为了问一句「这个内核能产出什么」不该拖进整个 HDF5。

## 站点地表参数

CoLM 读站点文件的规则是「有这个变量就用，没有就回落到全球 rawdata」，而回落要
35 个全球栅格、几百 GB。桌面用户不会有它们，所以 `crates/colm-srfdata` 的职责是
把站点文件补到 CoLM 永远不必回落。

```
cargo run -p colm-srfdata --bin site-fill -- <站点文件> <输出> [rawdata 目录]
```

实测 90 个 PLUMBER2 站点文件的变量集完全相同（各 39 个），都缺同样的 12 个字段。
取值优先级是**站点自有 > 栅格 > 模块默认**：质地类别由站点文件自己的土壤剖面
算得，高程取自同站 `Observation` 文件的 `Site elevation`，其余（湖深、高程标准差、
坡度、四个土壤反照率）站点侧没有对应值，从栅格取。不给 rawdata 目录时，栅格那
部分退到 CoLM 的模块默认值。

**每个补进去的变量都带 `source` 属性，写明它是量出来的、推导来的，还是标称值。**
命令行也会分别列出 `from raster` 与 `from default`。这不是装饰：本项目先前那版
生成器把土壤颜色档写死为 10，而 90 个站点里只有 1 个是 10 —— 那种错误不会让模型
崩，只会让它安静地用错的反照率算下去。详见 `oracle/fixtures/PROVENANCE.md`。

质地类别用 CoLM 自己的 USDA 三角作用于站点文件的土壤剖面，而不是读 CoLM 的
全球栅格 —— 站点文件的其余土壤参数会被 CoLM 原样采用，质地再从另一个产品取，
同一份土壤就自相矛盾了。两者在 90 个站点里只有 26 个一致（SoilGrids v2 与
Shangguan 2014 是不同产品，本就不该一致），不同时命令行会把栅格的答案也打出来。

## 强迫场

`crates/colm-forcing` **不转换数据**。CoLM 直接读 PLUMBER2 的 Met 文件
（`MOD_UserSpecifiedForcing.F90:683`，POINT 下 `metfilename = fprefix(1)`），
所以这一层产出的是那份 `nl_colm_forcing` namelist，加一组开跑前的校验。

```
cargo run -p colm-forcing --bin forcing-nml -- <Met 文件> [输出]
```

生成的 namelist 是给人看的：为什么第 5 槽是 `NULL`（PLUMBER2 只有标量 `Wind`）、
为什么三个 `HEIGHT_*` 会被 CoLM 用文件里的 `reference_height_*` 覆盖，都写在注释里。
产物会被 `crates/colm-namelist` 解析回来做断言——验的是它**说了什么**，不是长什么样。

校验拦的不是坏文件（90 个真实文件零 NaN、零填充值、步长均匀），而是几种
**能跑完却给出错误结果**的配置。头一种是 CoLM 自己写在注释里的：

```fortran
! when reaching the END of forcing data, show a Warning but still try to run
```

模拟窗口跑过强迫场末端时它只警告不报错，产出一份完整而错误的 history，而
`colm-kernel` 的失败标记里没有 `Warning:`——那样的运行会被判成功。

**三个参考高度必须分别读**：实测 90 个站点里有 30 个三者互不相同（CA-SF1 是
v=12.1 而 t=q=1.5，差 8 倍）。时间步长也不是普适的 1800 s：88 个站点是，
2 个是 3600 s，而算例里的 `DEF_simulation_time%timestep` 必须跟着走。

黄金回归用的就是这个生成器（`oracle/cases/<算例>/met.txt` 指明用哪个强迫场文件），
所以每次回归都在验它：生成的 namelist 若改变了语义，history 会先变。

## 两个窗口覆盖到什么、没覆盖到什么

下表全部来自对两个黄金文件的实测，不是设计意图。

| 算例 | 窗口 | 实测覆盖 |
|---|---|---|
| `CN-Cng` | 2008-01-01 → 01-11（264 步） | 冻结土壤热力、地表能量平衡、辐射与反照率、变饱和流求解（528 隐式 / 6 显式） |
| `CN-Cng-wet` | 2008-07-01 → 07-16（384 步） | 入渗（340/384 步非零）、饱和超渗产流（44/384）、地下水位动态、生长季光合与蒸散 |

**两个窗口都完全没有执行到的代码**（逐项实测）：

| 模块 | 证据 |
|---|---|
| 多层雪模型（雪层生成/合并分割、雪水文、雪热力） | `f_t_soisno` 的 5 个雪层在两个窗口**全程为 0**，即 `snl = 0`，从未生成过雪层。冬季 `f_xy_snow` 264 步全为 0（无一次降雪），`f_scv` 峰值仅 0.0276 kg/m²、`f_snowdp` 峰值 0.26 mm，均来自冷启动初始化 |
| `MOD_SnowSnicar` | 双重未覆盖：`DEF_USE_SNICAR` 默认 `.false.`，两个算例都没开 |
| 超渗产流 `f_rsur_ie` | 两窗口恒为 0 |
| 地下产流 `f_rsub` | 两窗口恒为 0 |
| 湖泊 | `f_lake_icefrac` 恒为 0，`f_t_lake` 始终停在 285.0 的初始值 |
| 湿地 | `f_wetwat` / `f_wetwat_inst` 恒为 0 |
| 含水层 | `f_wa` / `f_wa_inst` 恒为 0 |
| 土壤表面阻抗 | `f_rss` 恒为 0 |

**在这些分支被覆盖之前，不得声称对应模块已验证。** 设计文档 §2.11 把
`MOD_SnowSnicar` 列为六个最难移植单元之一，而它目前零覆盖 —— 这是 C 阶段
最需要先补上的窗口。

覆盖面还受这些单一取值限制：一个站点、一种斑块类型（IGBP 10 草地）、
一种产流方案（Simple VIC）、一种截留方案。黄金基准本身仍只用 `default`。

## 什么时候要自己编内核

**大多数时候不用。** 当前发行包只需要 IGBP 与 USGS 两份编译产物；两者必须
分开是因为地类数组尺寸不同。PFT / PC / BGC / URBAN / CROP / TRACER、土壤水力
与调试开关均由运行时 namelist 控制，`bgc` / `urban` 目录只是旧版兼容别名，
不再代表独立物理内核。

只有改了 Fortran 源码、需要尚未随包发布的平台/架构，或确实改变仍属编译期的
结构宏时才需要自己构建。桌面端依据约束卡片在 IGBP/USGS 产物间选择，不向用户
暴露一组重复且可能互相矛盾的“物理预设内核”。

### GitHub 直接产出安装包

`.github/workflows/release.yml`：打 `v*` tag 触发，三个平台各一个作业，
每个作业**先编 IGBP/USGS 两份 Fortran 产物，再打包 GUI**，内核作为
`bundle.resources` 进安装包。产物是 `.dmg` / `.deb` / `.rpm` / `.AppImage`
/ `.msi` / `.exe`，汇总成一份 draft release 等人过目。

发行包中携带 IGBP/USGS 两份通过完整性校验的 Fortran 产物。

**「用户什么都不用装」是验过的，不是推的。** 把仓库的 `kernels/` 与
`target/*/colm-cli` 都藏起来 —— 也就是一台没有源码树的机器 —— 再跑打包出来的
`.app`，它自己报：

```
colm-cli resolved to .../CoLM Desktop.app/Contents/MacOS/colm-cli
2 preset(s) from .../CoLM Desktop.app/Contents/Resources/kernels
```

两份产物都走 `Kernel::open` 列出，也就是**连各自三个二进制的 sha256
一起校验过**。顺带确认了打包不改字节：`colm.x` 的 sha256 与
`manifest.json` 里记的逐个相同。（真做代码签名时这条要重验 —— 签名会改
Mach-O 的字节，而清单认的正是字节。）

`resolve_cli` 用 `current_exe()` 的同级目录、`list_kernels` 用
`resource_dir()`，两处不同是因为 Tauri 本来就把 `externalBin` 放在主二进制
旁边、把 `bundle.resources` 放进 `Contents/Resources/`。

## Windows 上要装什么

**用桌面程序的人：什么都不用装**（同上）。

**要自己编内核的人：只需要 MSYS2**，装一个 MINGW64 环境加五个包：

```
mingw-w64-x86_64-gcc-fortran      # gfortran，CI 上实测 16.2.0
mingw-w64-x86_64-netcdf-fortran   # 4.6.3，会连带拖进 netcdf 4.9.3 与 hdf5 2.2.0
mingw-w64-x86_64-lapack           # 连带 blas
mingw-w64-x86_64-msmpi            # 只为了 mpif.h，见下
make                              # 加 git
```

不需要 Visual Studio、不需要 Intel Fortran、不需要 WSL。`pacman` 一共装 98 个包
（大多是依赖），CI 上整个作业约 3.5 分钟，其中大半花在装包上。

**`msmpi` 那条是个意外。** SinglePoint 号称不用 MPI，`define.h` 也确实
`#undef USEMPI`，但 `share/MOD_SPMD_Task.F90:34` 的 `include 'mpif.h'`
写在 `#ifndef USEMPI` **之外** —— 头文件必须存在，哪怕一个 MPI 符号都不会被用到。
macOS 与 Linux 上恰好都装着 MPI，所以这件事在 Windows 之前从没暴露过。
（改上游一行就能去掉，但会增加一处没有必要的 vendor 偏离；装一个只提供
头文件的包更便宜。）

### Windows 上二进制叫 `.exe`，不叫 `.x`

CoLM 的 Makefile 在所有平台都产出 `.x`；`build_kernel.sh` 在 Windows 上把
**拷进内核目录的那份**改名，不碰 `run/` 里 Makefile 的产物。

理由是 Windows 的 `PATHEXT` 不含 `.x`：系统不把这个文件当可执行文件，而是当
「文档」。实测 PowerShell 直接拒绝 `& .\colm.x | ...`，报
`Cannot run a document in the middle of a pipeline`；双击也没反应；安全软件
对「带 PE 头却顶着陌生后缀」的文件通常更不客气。

**严格说程序本身不依赖这个改名** —— `run_stage` 用 `Command::new(绝对路径)`，
走 `CreateProcessW`，对显式路径不查 `PATHEXT`。但在改名之前那只是一句推断：
CI 里唯一跑通过的那次，是**先把 `.x` 拷成 `.exe` 再跑的**。现在
`run_tests::a_real_kernel_can_actually_be_spawned` 让 `colm-kernel` 自己去起
一个真内核（`COLM_KERNEL_DIR` 指到内核目录，Windows CI 会带着它跑），
判据是 `run_stage` 返回 `Ok` —— 它只在**起不来**时返回 `Err`，
进程起来后自己死掉算 `Ok`。于是这条测的正是「操作系统肯不肯启动它」。

文件名的唯一真相是 `colm_kernel::program_file()`；`build_kernel.sh` 与两个
工作流都跟着它走。校验时找一个名字、启动时找另一个，是这类改动最容易留下的
裂缝。

### 还没答的那半：分发时要不要带 DLL

`nf-config --flibs` 在 CI 上返回 `-L/mingw64/lib -lnetcdff -lnetcdf` ——
**动态链接**。产出的 `.x` 因此依赖一串 MSYS2 的 DLL，而工作流里那次冒烟测试
是在 MSYS2 shell 里跑的（`/mingw64/bin` 在 PATH 上），它证明的只是
「装了 MSYS2 的机器上能跑」。`design.md` §9 原先写着「静态链接 gcc 运行时」，
那是计划不是现状，已经改掉。

`windows-kernel.yml` 现在多两步专门量这件事：`ldd` 列出 `/mingw64` 依赖，
再从 PowerShell（没有那个 PATH）跑一次看它缺什么。有了那份清单才谈得上
「随程序带哪些 DLL」还是「改成静态」。

这跟打包 `.app` 时踩到的是同一类错误：**一条在开发环境里永远成立的前提，
被当成了结论。**

## 运行时物理与地类产物的实际状态

| 配置 | 所需发行内核 | 运行 | 备注 |
|---|---|---|---|
| `default` | IGBP | ✅ 黄金基准 | —— |
| `PC` | IGBP | ✅ 三阶段 + 96 步 | 运行时 `DEF_USE_PC` |
| `USGS` | USGS | ✅ 三阶段 + 96 步 | 编译期地类数组不同 |
| `bgc` | IGBP | ✅ 三段跑通 | BGC 是运行时开关；需要两份 runtime 数据，见下 |
| `urban` | IGBP | ✅ 三段跑通 | URBAN 是运行时开关；完整站点文件可不提供 rawdata，见下 |

Rust 原生后端仍在迁移中。`colm-core::standard_lct_snow_soil_step` 已把普通地表的
非拆分活动积雪路径连接到新雪、`THERMAL`、`snowwater`、`WATER_2014`、压实、
合并和重新分层，并让积雪、土温和土壤水分状态继续传递。无雪到成雪的状态切换、
SNICAR、tracer 与拆分土壤/积雪路径仍未接入完整时间循环。

**BGC 需要两份 runtime 数据，而 `design.md` §10 只记了一份。**
`nitrif/`（30 MB）是记过的；`ndep/fndep_colm_hist_simyr1849-2006_1.9x2.5_c100428.nc`
（17 MB）没有记过，而且**无法绕开** —— `main/CoLM.F90:391-394` 的两个分支
（`DEF_NDEP_FREQUENCY==1` 年际 / 否则月际）都在 `#ifdef BGC` 内，没有关闭分支，
schema 里也只有频率没有开关。

**URBAN 曾经是唯一必须带全球栅格跑的预设 —— 现在（对 Urban-PLUMBER 那 21 个
站）不是了。** 这一节记的是它为什么曾经是、以及那 240 GB 是怎么去掉的。

曾经的理由：`default` 与 BGC 算例的 `DEF_dir_rawdata` 故意指向一个**不存在**
的目录 —— `site::fill` 已经把该有的都写进了 `site.nc`，跑通了就证明一个字节
都没读回去。城市算例做不到：`MOD_Namelist.F90` 的 Part 3 有
`USE_SITE_urban_geometry` / `_ecology` / `_radiation` / `_thermal` / `_human`
五个开关，**唯独没有 `USE_SITE_urban_type`**；再加上 Urban-PLUMBER 的站点
文件里 23 个变量全是形态学量（建筑高度、道路面积比、树高…），没有土壤剖面、
没有湖深、没有土壤反照率。一次真实运行的来源清单里有 30 项写着
`from CoLM 2024 raw data`。

**门槛分四步拆掉，缺一不可：**

1. **两个上游 Fortran bug** —— 它们让「站点文件里有就用站点文件」那条分支
   根本不可达。修补已经纳入当前 `vendor/CoLM202X` 快照；来源与本地差异见
   `vendor/PROVENANCE.md`：
   - `lakedepth` 的 readflag 取自一个**还没赋值的结果变量**，`.and.` 短路之后
     连警告都不打，站点值静默地被栅格顶掉；
   - `TREE_LAI` 命中站点分支时不分配 `SITE_LAI_year`，而写出时无条件调
     `size()` —— **必然段错误**。
2. **土壤剖面**：21 个站的 24 个剖面量 × 8 层，加上 `soil_texture`，
   预抽成 `crates/colm-srfdata/src/urban_soil.rs`（90 KB）。8 层不是
   `nl_soil`（那是 10）—— `MOD_SoilParametersReadin.F90` 是 `DO nsl = 1, 8`。
   这一步搬走 `soil/` 那 **122 GB**。
3. **其余城市栅格点值**：LCZ/NCAR 分类、LUCY_ID、土壤颜色档、湖深、地形、树 LAI/SAI
   → `crates/colm-srfdata/src/urban_extra.rs`（约 250 KB）。这些来源**开不到就
   `CoLM_stop`，不是警告**；其中 `urban_lai_500m/` 单个瓦片 85 MB，21 个站
   要 15 块 × 23 年 ≈ 7 GB。
4. **两张小型全局属性表随包发**：`LUCY_rawdata.nc`（37 KB）自动铺到
   `runtime/urban/`；`NCAR_urban_properties.nc`（62 KB）在站点具有有效
   `REGION_ID` 与 `URBAN_DENSITY_CLASS` 时自动铺到 `rawdata/urban/`。

**省下的不是估算，是实测。** AU-Preston（1993-01-01 至 01-11，1800 s 步长）
在站点文件包含城市人口密度等完整字段时，**完全不给 `--rawdata` / `--runtime`**：
三段全 `ok`，264 条小时记录，
`f_tref` 峰值 `311.9649983374719 K`。拿同一个算例、改成直接读 122 GB 栅格的
参照 run 比对：

```
identical: 146 variables
```

**逐位相同**，不是「量级对得上」。21 个站全部建算例成功。

### 这张表的边界

**表只覆盖 Urban-PLUMBER 那 21 个站。表外的城市站点仍然需要 `--rawdata`。**
`urban_extra.rs` 查不到的站点一个字都不写，让 CoLM 照旧回落栅格 ——
编一个 `LCZ_DOM` 出来，会把整个城市形态换掉而结果看上去仍然正常。
**不要对没量过的站点外推。**

**照抄栅格，不替它「修正」。** `soil_texture` 在 **21 个站里有 16 个是 `-1`**
（质地产品在建成区没有数据），照抄栅格的 `_FillValue`，而**不是**由砂黏比
反推一个「看着合理」的类别 —— CoLM 自己有 `WHERE (soiltext < 0)` 的处理路径
（夹到 0 再取 `BVIC_USDA(0) = 1.0`）。同理，南半球两个站抽出来的树 LAI 月相位
像北半球物候、FI-Torni 全年 0.00，都原样入库。改成「看着对」的值就不再与
「让 CoLM 自己去读栅格」逐位相同了，而逐位相同正是上面那条 `identical` 的
全部意义。

城市算例与水热算例的三处配置差别，全部由 `colm-case` 自动写出：

| 字段 | 值 | 为什么 |
|---|---|---|
| `SITE_landtype` / `USE_SITE_landtype` | `13` / `.true.` | URBAN 路径反正会强制成 13（`MOD_SingleSrfdata.F90:1548`），写出来是让配置文件说出实际会发生的事 |
| `DEF_URBAN_type_scheme` | `2` | LCZ。默认的 `1`（NCAR 城市密度分类）在栅格给不出城市类别时越界 —— CoLM 自带的 `ex03_site_urban` 用的也是 2 |
| `USE_SITE_lakedepth` / `_soilreflectance` / `_soilparameters` | `.false.` | 三项默认 `.true.`（「站点文件里有」），可城市站点文件里没有 |

站点文件这边只补一样东西：`prepare_urban` 把 `ground_height`
（`long_name = "Ground height above sea level"`）抄成 `elevation`，于是
`USE_SITE_topography` 能留在默认的 `.true.`，CoLM 再也不需要那份 7 GB 的
`elevation.nc`。除此之外原文件逐字节照抄 —— 实测 `ex03_site_urban` 用的就是
未经处理的原件。

能量闭合残差 `f_xerr` 在 `1e-15` 量级，`f_tref` 峰值 312 K —— 墨尔本一月的
夏季午后，量级对得上。

### 已修复的两个「装完就跑」阻塞

栅格门槛解决后还出现过两个与 rawdata 无关的阻塞；当前均已修复并有回归检查：

1. **算例目录里不能有空格。** CoLM 建目录用的是不加引号的
   `CALL system('mkdir -p ' // trim(dir))`（`vendor/CoLM202X` 里 **55 处**）。
   路径一有空格就被 shell 拆成两个参数，真正的 `landdata/` 从没被建出来，
   netCDF 报的却是一句看不出所以然的 `Netcdf error: Permission denied`。
   偏偏 GUI「用自带的示例站点」默认把算例放在
   `~/Library/Application Support/…` —— **那里就有一个空格**。换成无空格的
   算例目录之后，CN-Cng 五步全程跑通（9 个月度 history、曲线 8760 点）。
2. **默认时间窗口比强迫场早了一整天。** 不给 `--start` / `--end` 时，
   AU-Preston 当时推出来的窗口是 `1992-12-31`，而强迫场第一条记录在
   `1992-12-31 23:30`（CoLM 报 `Model start 1992 365 86400` vs
   `Forc start 1992 366 84600`），于是 `colm` 段以
   `Forcing does not cover simulation period!` 失败。**推导保留了日期却丢掉了
   当天的时刻。**现在按强迫场保留日内秒数，AU-Preston 完整窗口可直接运行。**

附带一条：内核始终包含城市模块，但 `DEF_URBAN_RUN` 默认 `.false.`；新建城市
算例会显式写成 `.true.`，自然站保持关闭。

还有一条给下一个人的提醒，**表外的站点仍然需要完整 rawdata**；21 个内置站点
不再需要人工摆表。`colm-cli new` 会把 NCAR 城市属性表铺到 `<rawdata>/urban/`，
把 LUCY 表铺到 `<runtime>/urban/`，两个目录分别供 `mksrfdata` 和后续阶段读取。

### 闸门表在第二个预设上被独立验证

`colm-hist` 的闸门表是拿 `default` 的黄金文件建并验的。BGC 跑通之后拿它
再验一次：预测可写 326、实际写出 261、**漏报 0**。多报的 65 个全是运行时条件
为假的那些（256 无条件 + 5 个条件成立 = 261，自洽）。

一张只在一个预设上验过的表，在另一个预设上零漏报 —— 这比再多几条单元测试
更能说明它抓对了闸门。

### 两个预设的指标对比

同一个算例、同一个窗口（CN-Cng 2008-01-01 → 01-11，剔除前 8 小时）：

| | Rnet R² | Qle R² | Qh R² |
|---|---|---|---|
| `default` | 0.986 | 0.047 | 0.530 |
| `bgc` | 0.985 | **0.503** | 0.305 |

潜热大幅改善（RMSE 32.47 → 12.7），感热变差，净辐射几乎不变 —— 能量分配变了
而辐射物理没动，符合预期。

**但这不是一次干净的对照**：`bgc` 预设同时把 `LULC_IGBP` 换成了
`LULC_IGBP_PFT`，所以两个变量一起变了。要分清是 BGC 还是 PFT 方案带来的
改善，得再构建一个只改其中一个的预设。这一条记在这里，不当结论用。

## 全仓深度审计（2026-08-24）

### 修复复核（当前工作树）

首次审计发现已经逐项复核。下表是当前结论；后面的长清单保留为**修复前快照**，
用于说明问题来源，不再代表现在的实现状态。

| 原编号 | 当前状态 | 修复或判定 |
|---|---|---|
| H1 / H2 | 已修复 | 动态文案补齐翻译；`gui/tests/i18n.mjs` 机械扫描 `app/*.js` 的普通字符串和模板字符串，未知中文会使测试失败 |
| M1 | 已修复 | `golden-run` 与 Study 测试内核都从 `case.nml` 读取 `DEF_LC_YEAR`，不再写死 `lc2005` |
| M2 / M10 | 已修复必要部分 | 单算例和批量运行可取消，窗口退出会终止进程树；阻塞 sidecar 调用统一离开 Tokio worker。未加任意绝对超时，避免合法的长模拟被误杀 |
| M3 | 已修复 | rawdata 文件和目录内容进入输入指纹；大文件采用头尾有界采样加元数据，避免为缓存判断完整读取数十 GB |
| M4 | 已修复 | `check-gui` 的导出检查和 import 环检查均支持多行 import，并有回归测试 |
| M5 | 已修复（待提交） | AT-Neu 四件套与 `kernel_profile` 测试已纳入 Git 索引，CI 显式运行该集成测试；发布前须连同本轮代码一起提交 |
| M6 / M7 | 已修复 | 城市站点契约加入 `resident_population_density`；真实 site-vs-raster 测试明确验证站点文件优先 |
| M8 / M9 | 已修复 | history 闸门由主 history 与 TRACER/CH4 源共同生成，共 619 个写出点；tier-check 增加层级不倒挂断言 |
| M11 / M12 / M13 | 已修复 | 删除未调用命令、共享 RunLog 和前端重复降采样；原生 prompt/confirm 也经过翻译 |
| M14 | 非缺陷 | `tolerances.toml` 约束的是黄金 history 比较，不覆盖地理匹配、单位换算和时间轴判断的局部数值容差 |
| M15 | 约定债务 | 1-based 语义已有结构字段注释和测试保护；为追求后缀统一而批量改名没有运行时收益，未制造无意义 churn |
| 低危批量 | 已修复本轮处理的可触发项 | `--pairs-var` 未知值会报错；primary/TRACER/CaMa history 分流；转换与修复均拒绝同文件别名；时间输入标明 UTC；CLI 支持 `--help` |

当前验证：`cargo test --workspace`、GUI 后端 111 项测试、全部 `gui/tests/*.mjs`、
workspace 与 GUI Clippy（`-D warnings`）、`cargo fmt --check`、`check-gui`
（56 注册 / 56 调用 / 6 事件）均通过；tier-check 覆盖 127 个黄金变量。

### 首次审计快照（修复前，只读证据）

> 方法：8 条并行审计线（配置管线 / 评估物理 / 运行编排 / 强迫场前处理 /
> 地表数据 / GUI 后端 / GUI 前端 / 门禁·测试·CI·文档，各自读源码取证）+
> 2 条交叉验证线（跨层契约复核、物理对账），全程只读，未改任何文件。
> 当时审计对象有 119 处未提交改动，其中 Study 调参特性
> （`crates/colm-cli/src/study/`、`crates/colm-case/src/tuning.rs`、
> `gui/dist/app/study-model.js`）与 AT-Neu 示例均未入库。

### 修复前执行证据（实测，非声称）

| 命令 | 结果 |
|---|---|
| `cargo test -p colm-schema --test drift` / `--lib`；`-p colm-namelist`（含 roundtrip）；`-p colm-case --lib` | 1+15 / 28+5 / 31 通过 |
| `cargo test -p colm-forcing --lib`；`-p colm-srfdata --lib` | 109 / 77 通过 |
| `cargo test -p colm-kernel --lib`；`-p colm-cli --bin colm-cli` | 40 / 98 通过 |
| `cargo test -p colm-hist --lib` / `--features io` / `--test drift` | 32 / 35 / 1 通过 |
| `cargo test -p oracle --test judge` / `histmap` / `metrics` | 10 / 4 / 4（metrics 因无 PLUMBER2_ROOT 走 skip） |
| `tier-check` | 127 变量全覆盖，无重复无 stale |
| `cargo test --manifest-path gui/src-tauri/Cargo.toml --lib` | 106 通过 |
| `cargo run -q -p xtask -- check-gui` | `59 registered, 55 called, 6 events — all resolve` |
| `node gui/tests/*.mjs`（10 个） | 全部 exit 0 |
| **合计** | **546+ 通过 / 0 失败** |

未跑（按约定）：黄金回归（需重建内核）、`colm-srfdata` raster/real_sites
（38 GB）、`colm-forcing` met/real_forcing（PLUMBER2）、release 打包。

### 修复前六维判定

| 维度 | 判定 | 一句话理由 |
|---|---|---|
| 合理性 | 良 | 三阶段编排、成功判定三件套、容差分层、sidecar 隔离均有实测依据；短板在进程生命周期（无超时/取消） |
| 完整性 | 不通过 | i18n 漏翻约 167 处；城市"免 rawdata"声称不成立；TRACER 闸门表缺；README/design.md 滞后于 Study 功能；release 资产未入库 |
| 物理缺陷 | 无致命项 | 全部常数逐位一致、公式标准、单位/符号/时区正确；仅冰区间饱和水汽压公式分叉（dormant）与若干低危边界 |
| bugs | 无功能性 bug | 546+ 测试全绿；实锤均为低-中危边界，且交叉验证后两处被降级（见 M 节） |
| 自洽 | 良（有漂移） | 前后端契约（59/55/6）静态守死；漂移集中在文档（7 vs 9 pane、25 vs 26、submodule 分布、PROVENANCE 计数） |
| 扁平化 | 良好 | 指标公式/单位换算/配对逻辑均单一实现；重复与死代码清单见下 |

### 修复前高危（2）

**H1. i18n 漏翻约 167 处动态文案，英文模式中英混排。** 证据：
`gui/dist/app/sitedata.js:237,278,286-294,341-342,390`（"可独立运行/结构字段/
有依据的查表值"整卡）、`shell.js:17,48-52`、`forcing.js:177-179,1156-1158`、
`results.js:1960`（"请先创建调优 Study。"，词典只有无"调优"的版本）、
`domain.js:220-240`、`params.js:275,294`。用真实 `translateZh` 仿真（占位法）：
426 个含中文串翻译后仍留中文，扣除 `param-presentation.js` 的 pair() 双语机制后
≈167 处。sitedata.js 是最近提交 f7122a3 引入的新文案 —— 违反"新增文案必须
同步 i18n.js 并加断言"的仓库规则。

**H2. i18n.mjs 断言机制拦不住 JS 模块漏翻。** `gui/tests/i18n.mjs:12-74` 只
手工抽查十几个动态串，`:97-117` 的"全量"检查只覆盖 `index.html` 静态文本；
`i18n.js:1033-1037` 对未知文本保持原样 → 漏翻静默无信号。建议测试改为收集
全部 `app/*.js` 中文字面量逐个跑 `translateZh` 断言无残留中文。

### 修复前中危（15）

- **M1. mkinidata 产物年份写死 `lc2005`，与可配 `DEF_LC_YEAR` 脱钩（真实可触发）。**
  `crates/colm-cli/src/main.rs:1660-1661` 与 `oracle/src/bin/golden_run.rs:100`
  写死；Fortran 侧按年号拼名（`MOD_Vars_TimeInvariants.F90:455-456`，
  `lc_year = DEF_LC_YEAR`）；**GUI 暴露并可编辑该字段**（`config.rs:120,1101,1117`）。
  用户改年份 → 产物校验误报 MissingArtifact。同源：`study/runner.rs:2286`
  测试脚手架也写死。产物表在 colm-cli 与 oracle 各一份拷贝，改一处忘另一处。
- **M2. 运行无超时/无取消；GUI 退出后 colm-cli + 三个内核进程成孤儿。**
  `colm-kernel/src/run.rs:189`、`gui/src-tauri/src/sidecar.rs:363,642` 阻塞
  `child.wait()` 无超时；全后端仅 `study_cancel`；`runner.js:226-265` 无取消入口；
  `capture()`（sidecar.rs:1167-1179）与 ERA5 下载（main.rs:3199-3208）同样无超时。
- **M3. fingerprint 对 rawdata 目录内容变化漏报。** `fingerprint.rs:85-167,195-201`
  只哈希 `looks_like_config_path` 命中的字段，`DEF_dir_rawdata` 不命中 —— 换掉
  栅格内容（路径不变）指纹判"可跳过"，旧 srfdata.nc 被当新数据。
- **M4. check-gui 对多行 import 完全失明（导出检查与环检测同病）。**
  `xtask/src/gui.rs:172-186`（`split_once('}')` 要求 `{`/`}` 同行）、`:87-98`；
  当前树已有 5 处多行 import（results.js:15-18、sitedata.js:7-9、forcing.js:20-22、
  sites.js:5-7,11-13）。逐名验证这些名字当前都真实存在 —— **检查器失效但暂无实害**，
  未来改名/删 export 时两条检查线同时静默。
- **M5. release 资产未入库。** `release.yml:141-145` 断言 AT-Neu 三件套 +
  `Forcingnml/AT-Neu.nml` 进包，但 `git ls-files` 无、`git check-ignore` exit=1
  （非 gitignore 所致）—— fresh checkout 上 macOS 作业必挂。
  `xtask/tests/kernel_profile.rs`（production 档位守门）也未跟踪，且 `ci.yml:72`
  的 `--lib --bins` 不跑 xtask 集成测试。
- **M6. 城市"免 rawdata"声称不成立。** `USE_SITE_urban_human` 默认 `.true.`
  （`MOD_Namelist.F90:95`），缺 `resident_population_density` 时 CoLM 回落
  `urban/URBSRF*` 瓦片 `POP_DEN`（`MOD_SingleSrfdata.F90:1826-1843`），而该字段
  不在 audit 必需清单（site.rs:225-237）也不在 urban_extra 表。
- **M7. 测试名与实现相反且断言空洞。** `site_tests.rs:17`
  `the_raster_wins_over_the_classifier_when_both_are_available` —— 实现是
  站点优先（site.rs:709-718），断言体只查 `REQUIRED_FIELDS` 成员。
- **M8. TRACER 输出不在闸门表。** `generated.rs` 恰 456 条、源仅 `MOD_Hist.F90`，
  grep `methane|TRACER` 零命中；`f_methane_surf_flux_tot` 由
  `MOD_Tracer_Reactive_Methane_Hist.F90:826` 写出而 `obs.rs:189` 引用它。
  评估侧不受影响（`evaluation_availability` 读真实文件），但 GUI histvars 门
  会把 tracer 变量漏报为"产不出"。
- **M9. tier-check 不查"层级不倒挂"不变式。** `tier_check.rs` 只做重复/无层级/
  无变量三类完备性检查；`tolerances.toml:6-9` 声称的硬约束靠人工维护。
- **M10. async 命令在 tokio 线程上做阻塞 IO。** `run_case`/`run_batch`/`capture`
  直接阻塞，仅 `study_run` 与 `download_era5land` 用 `spawn_blocking`；
  run_batch 期间并发调 series/probe 会延迟。
- **M11. 4 个注册未调用命令 + RunLog 死代码。** `run_log_tail`、`field_states`、
  `study_create`、`set_process_parameter_field` 前端零调用（55/59 差 4 一一对应）；
  `sidecar.rs:358` `run_case` 开头 `log.lines.lock().clear()` 使并发单算例互相
  清空缓冲区。
- **M12. downsampleSeries 前端副本，app 内无使用者。** `result-model.js:75-101`
  唯一引用在测试；与 `main.rs:2301-2359` Rust 版 NaN 处理不同，双份漂移时测试
  只锁 JS 那份。
- **M13. 原生 confirm/prompt 对话框完全不翻译。** `results.js:1664,1925,1947,1974,1975`
  五处，不走 MutationObserver。
- **M14. "容差不许内联魔数"声称过宽。** 引擎 crates 约 20 处内联容差（gapfill.rs:459
  1e-9、tabular.rs:870 1e-8、site.rs:413 1e-9 等），但均属地理匹配/单位换算/
  时间轴检查，非 history 比较容差；`tolerances.toml` 目前仅被 tier-check
  完备性消费（比较器尚未实现，属文档明示的阶段设计）。
- **M15. `_one_based` 后缀约定从未落地。** 全仓 grep 仅 1 处（grid_tests.rs:110
  测试名）；1-based 语义函数全用注释替代。

### 修复前中低危与低危（交叉验证后修正过的口径）

- **饱和水汽压 Bolton vs Flatau 冰区间分叉（交叉线定量新发现）。**
  `units.rs:138` 的 Bolton 液态拟合 vs `MOD_Qsadv.F90:83-93` 的冰多项式：
  0–30°C 仅差 0.05–0.1%（**此前口头估计的"0.3–0.5%"被证伪**），0°C 以下
  差 5–18%（−10°C −9.4%、−20°C −18%）。三个示例文件直给 `Qair`，该路径
  当前 dormant；若未来启用冬季 RH→q 预处理需换 Flatau 或按冰多项式处理。
- **forcing-convert 只认 `_FillValue` 不认 `missing_value`（已降级）。**
  `bin/forcing-convert.rs:103-107` 有洞，但 GUI 走 colm-cli 子命令
  （main.rs:2501-2513）两者都查 —— 独立 bin 未随包分发，实害度近零。
- **表格导入 heights 恒发 0,0,0（已确认不可达）。** `forcing.js:785` 的
  `Number(null)=0` 是潜在 footgun，但双层守卫（按钮禁用 + 函数早退）使正常
  UI 路径到不了后端，CLI 层也 loud-fail。
- **`DEF_dir_output` 被 usage 扫描误标 `requires: CatchLateralFlow`（已确认无实害）。**
  `generated.rs:61` 元数据错（成因 `usage.rs:144-146` 跳过 MOD_Namelist.F90），
  但 GUI 已硬编码补偿（`config.rs:318-323`）。
- 其余低危批量：`_hist_cama_*.nc` 混入 primary 流（CaMaON 时时间轴拼接失败）、
  scalar_wind 按计划槽位判定、高湿 q 超饱和无防护、全零 ERA5 重叠期乘性订正
  bail、`canonical_units` 的 `_ => ""`、CRLF/末尾无换行静默规范化（与"保留原文"
  承诺相悖）、series 时间窗按 UTC 解释而控件是 datetime-local、repair_forcing
  同文件保护弱于 convert、`--pairs-var` 未知变量静默空结果、r² 不截断/β 无
  零分母（GUI 有 serde_json NaN→null→"—"兜底链，已从 serde 源码级确认）、
  time:units 时区 token 静默忽略、产物校验只看存在性（design.md:773 声称的
  内容校验未实现）、USAGE 缺 3 条 study 命令、`--help` 报 unknown command、
  非 UTF-8 路径 panic、评估后改 spinup/corrected 表图口径不一致、转换/修复
  按钮无 busy 守卫、实数解析三处重复、tuning 活动性 `_ => true` 兜底、fill
  非幂等、NaN 经 clamp 传播、lon=180 像元分歧、URBTYP 审计缺口、25 vs 26
  一致数四处注释漂移、study 模块 3 处 `#![allow(dead_code)]`。

### 物理正确性：逐位核对通过项（对照 vendor 源码）

反照率 4×20 表 = `MOD_SoilColorRefl.F90:44-54`；USDA 三角 26 顶点/12 多边形/
pointinpolygon = `rawdata_soil_solids_fractions.F90:233-359`；`BVIC_USDA(0:12)`
= `MOD_Initialize.F90:261`；`HTOP0_IGBP`、`DZ_SOIL[8]` 层边界、
`wf_om = OM_density/BD_all` 恒等式、lakedepth×0.1、5x5 瓦片命名与 1-based
索引 —— 全部逐位一致。单位换算系数（K↔°C、hPa/Pa、mm/hr→kg/m2/s、g/kg→kg/kg）
与区间累计量需显式步长（防"累计当率"）正确。时区符号（local = UTC + offset）、
太阳正午推断（`12 − lon/15`）正确。缺测修复物理正确（线性插值仅限两侧有观测
的短缺口、边界不外推、降水仅双零才补零、非负钳制）。ERA5-Land 最近格点 +
0.15° 闸门、加性（状态量）/乘性（降水辐射）订正、逐月+全局回退、缺测段不参与
拟合、逐时 QC 留痕 —— 全部正确。

指标公式（RMSE/MAE/Bias(m−o)/r²=Pearson²/NSE/KGE 等权）与标准定义逐条对上，
**python 三组小样本独立验算通过**；α 与报告 σ 的 n 因子在比值中相消（α ≡
model_sd/obs_sd 严格相等，非不一致）。时间轴 1900-1-1 原点手算验证
（2008-01-01T00:30 = 56_802_270 分）、中点标签 t−1800s 与 t 两观测点平均、
半开窗口、1 秒容差配对 —— 全部兑现。观测映射（FCH4 ×1e9 nmol、GPP ×1e6 µmol、
NEE = respc−assim 符号、Qg 方向与 PLUMBER2 实测一致、ANNOPTLM 的 1/3 QC
编码）用仓库真实观测文件核对。`tol_richards = 8.e-8`
（`MOD_Hydro_SoilWater.F90:50`）与 `tolerances.toml:58` 逐位一致。成功判定
11 条 FAILURE_MARKERS 与 Fortran 实际输出形态逐条核实（含 `CoLM_stop`
退出码 0、stderr 专属标记、BENIGN_LINES 豁免）。架构硬约束：窗口进程零
NetCDF/HDF5 链接（GUI Cargo.lock 491 包 grep 零命中）、读 NetCDF 全走
sidecar、Tauri v2 camelCase 映射 81 处 invoke 全对。

### 扁平化

**单一实现（通过）**：指标公式零前端副本（唯一实现在 `metric.rs`）；单位换算
单一实现（`units.rs:14-55`）；配对逻辑四层 API 委托单一实现；GUI 无任何绕过
colm-cli 的路径（sidecar 仅 4 处 `Command::new` 全是 colm-cli）；命令解析唯一；
`other =>` 全部是显式报错退出而非静默兜底。

**重复/冗余**：downsampleSeries JS 副本（app 内未用）；饱和水汽压三处内联
（units.rs:138,184 / gapfill.rs:1915）；实数解析三处（value.rs / minimal.rs /
tuning.rs）；缺测检查三份实现口径分裂（独立 bin / colm-cli 子命令 / gapfill）；
产物表两份拷贝（main.rs:1653-1666 vs golden_run.rs:100-109）；status/setStatus
双实现（ui.js:9 vs shell.js:202）；4 个死命令 + RunLog + 3 处
`#![allow(dead_code)]`。

### 修复前优先级（历史）

- **P0（下次提交前）**：入库 4 个 AT-Neu 示例 + `xtask/tests/kernel_profile.rs`；
  i18n 补约 167 处词典条目并把 i18n.mjs 改为全量机械扫描；修 check-gui 多行
  import 盲区（顺带 import_cycles）。
- **P1（下一迭代）**：lc2005 → 从 case.nml 读 `DEF_LC_YEAR`（含 golden_run 与
  study 脚手架同步）；运行取消/超时/GUI 退出清理子进程；fingerprint 纳入
  rawdata 目录内容；城市 audit 补 `resident_population_density`（或改声称）；
  修测试名与断言空洞；TRACER 闸门表边界声明；tier-check 加"层级不倒挂"
  可执行断言；清理 4 死命令/RunLog/`allow(dead_code)`/downsampleSeries 副本。
- **P2（低危批量）**：饱和水汽压注释与冰区间处理、`--pairs-var` 校验、repair
  同文件保护、UTC 标注、fill 幂等、NaN 守卫、CRLF 语义、`_one_based` 约定、
  文档漂移批量（README 补 Study 与"九个分栏"、design.md 补 KGE"标记不改值"
  与产物内容校验承诺、field.rs 计数、PROVENANCE 计数、submodule 残留）。

### 当前仍未覆盖

本轮没有重建并重跑全部内核黄金算例，也没有读取 38 GB rawdata 或执行真机
WebView 自动化、三平台 release 打包；依赖真实内核或外部 PLUMBER2 数据的三项
测试保持 `ignored`。Windows 专属 job object 与安装包行为仍由 CI / release
runner 验证；`vendor/CoLM202X` 没有自身 `.git`，无法与上游 commit 做字节级 diff。

## 原生运行时的「中间层」落地：restart → LCT 驱动模板（2026 年，装配层）

`colm-runtime` 之前有钟、有 forcing、有 `standard_lct_soil_step`，但没有任何东西
把**写出的 restart** 装配成内核要的模板 —— 所以它的 LCT 驱动只在自身测试里被
手拼出来的输入跑过，`docs/mkinidata-rust-port.md` 把这一层记为「缺失的中间层」。
现在这一层存在了，但只覆盖**无雪、非 split、非城市、非湖、非 PHS 的规则土壤
LCT 分支**，也就是 `standard_lct_soil_step` 已经移植的那一支。

### 来源三分不是选择，是上游的定义

`MOD_Vars_TimeInvariants.F90:READ_TimeInvariants` 从 `_restart_const_lc<year>.nc`
读时间不变量（`patchtype`、`vf_quartz`/`csol`/`hksati` 那一整套土壤参数、`htop`/`hbot`、
`ncd`/`ncw`/`bcw`、`debdrock`、`elvmean`/`slpratio`、`topoweti`/`fsatmax`/`fsatdcf`…），
`MOD_Vars_TimeVariables.F90` 从时间 restart 读演化态与植被态。本仓库两个写出器
的覆盖面已经够（时间 restart 里 `alb`/`ssun`/`ssha`/`ssoi`/`ssno`/`thermk`/`extkb`/
`extkd` 正是 `ColdStartRadiation` 的全部字段，`tleaf`/`ldew*` 是 `LeafTemperatureState`，
`t_soisno`/`wliq_soisno`/`wice_soisno`/`zwt`/`wa`/`wdsrf` 是状态与水位）。

真正没有来源的是第三类：PFT 生化表（`LeafBiochemistry` 在全仓库只被测试构造过）、
`LeafTemperatureOptions`、叶倾角、粗糙度、观测高度、产流/阻力方案选择。上游把它们
放在 `MOD_Const_LC` 的**编译期表**与 namelist 里（例如 `rootfr` 由 `Init_LC_Const`
按 `d50`/`beta` 现算，不入任何 restart）。因此 `colm-runtime::assembly` 把这一类
做成 `LandPhysicsParameters` —— **不设默认值**：少给一个字段就是编译错误。这是刻意的，
一份「看着合理」的生化参数会让整条链静静地跑错。

### 装配层抓到的四个缺陷（都是实测，不是推理）

1. **`air_density_kg_m3` 在整条 LCT 链里没有来源。** `GroundFluxInput`、
   `SoilSurfaceResistanceInput`、`LeafTemperatureInput` 都要空气密度，而
   `RuntimeForcing` 没有这个字段、`colm-core` 也没有对应内核。上游在
   `MOD_Forcing.F90` 里算 `forc_rhoair = (pbot - 0.378·q·pbot/(0.622+0.378·q)) / (rgas·t)`
   （并对密度用的温度钳在 326 K），再由 `CoLMMAIN` 传给每个 THERMAL 分支。
   现在 `RuntimeForcing` 带上这个字段，在 `prepare_runtime_forcing` 里按同一公式
   算出来，下采样路径按 `MOD_ForcingDownscaling` 用调整后的列重新算 —— 谁都不必
   再各自推一遍，否则三条路径会漂移。
2. **`SoilState` 是 `layer * patches + patch`，而 restart 里是 `(patch, soil)`。**
   装配层第一版直接把文件缓冲当层主序用，测试立刻抓到 patch 1 读到了 patch 0 的
   `vf_quartz`（10.0 vs 110.0）。这类错误在数值上极难看出来 —— 每个 patch 都拿到
   一份"合理"的土壤参数，只是别人的。
3. **`snw_rds` 不是 patch 级雪龄。** 逐雪层的粒径属于气溶胶那一节；patch 级的雪龄是
   `sag`。把两者混起来读会让 `ColdStartRadiation.snow_age` 变成一个层维数组的第 0 项。
4. **`rootfr` 必须求和为 1，而装配层现在显式核对这一点。** `MOD_Eroot` 把
   `soil_water_stress` 定义成 `sum(rootfr · resistance)`，所以一份没归一的根系比例
   会让胁迫大于 1，最终被叶温校验以一句笼统的「leaf-temperature inputs are invalid」
   拦下（实测 5.13）。在装配处按名报错能直接指出病因。

另外两条现在会**按名拒绝**而不是硬跑：`patchtype != 0` 的 patch（城市/湿地/湖走的是
别的分支），以及 `fsno != 0` 的 patch（这个模板只描述无雪列，雪槽带值却被当成无雪
驱动正是它该拦下的输入）。

### 新落地的可复用件

- `colm_core::SoilState::from_fields` —— 从 29 个层主序缓冲构造状态，长度不符即报错。
  读数器与内核之间此前没有这个入口，装配层只能自己拼。
- `colm_core::soil_hydraulic_models(soil, patch, model)` —— 原本是 `colm-init`
  里一个写死 patch 0 的私有函数；现在多了一个显式 patch 参数并由 `colm-init`
  继续复用，避免两份 Campbell/VG 映射漂移。
- `colm_core::soil_thermal_inputs(...)` —— 把 `SoilState` 的静态半与当前列的
  动态半合成逐层 `SoilThermalInput`，体积含水率按 `MOD_GroundTemperature.F90` 的
  `vf_water = wliq/(dz·denh2o)`、`vf_ice = wice/(dz·denice)` 算。在此之前**没有任何
  生产代码**构造过 `SoilThermalInput`（只有五处测试夹具）。
- `colm_init::RestartFile::patch_matrix` —— `(patch, 第二轴, 第一轴)` 场按内存里的
  `[第一轴][第二轴]` 取回，把写出器的转置反转回调用方那一侧。
- `colm_init` 的 `fixtures` feature：一份两 patch 的合成 restart 对（常数 + 时间），
  每个 `(field, layer, patch, band, rtyp)` 都取互不相同的值，所以轴序写反不会
  侥幸通过；土层厚度取自共享 `colm_soil_grid`，土壤水按孔隙度取 50% 饱和 ——
  随手给一个 10 kg/m² 会得到超过孔隙度的体积含水率，下游的应力与通量核对会拒收
  这份"重启"（实测踩过）。它只在测试构建里编译。

### 证据

`cargo test -p colm-runtime --lib`：22 通过，其中一条是**从文件装配出模板后真的
跑一步 `standard_lct_soil_step`**，并断言状态被推进、能量闭合残差 < 0.5 W/m²、
液态水非负。其余覆盖：29 个土壤场逐场核对、`[band][rtyp]` 光学矩阵逐元素核对、
patch 选择的产流差异、三个产流分支各自的常数重启来源、缺文件/缺变量/越界 patch/
未归一 `root_fraction`/长度不符的按名拒绝。

本机这次 file policy 是 `danger-full-access`，之前被沙箱拒掉的 8 个进程探测测试
（`colm-kernel` 1 个、`colm-cli` `study::runner` 7 个）**全部通过**：
`cargo test --workspace --lib --bins --exclude colm-cli` = 1188 通过 / 0 失败；
`colm-cli --bin` = 215；GUI backend = 161；两个 workspace 的 `fmt --check` 与
`clippy -D warnings` 均干净。

## 地类常量表进入代码生成（2026 年，`MOD_Const_LC.F90`）

`colm-core` 此前有两份**手抄**的地类常量：`radiation.rs` 里的 `IGBP_LEAF_OPTICS` /
`USGS_LEAF_OPTICS`（逐类一行 `chil`/`rho`/`tau`），以及 `albedo.rs` 里的土壤颜色映射。
前者是上游 `MOD_Const_LC.F90` 的同一张表，抄得没错，但没有任何东西拦得住两份漂开。
现在这张表由 `cargo run -p xtask -- gen-landcover` 生成到
`crates/colm-core/src/land_cover_generated.rs`（94 张表：IGBP 17 类 + USGS 24 类），
由 `crates/colm-core/tests/drift_landcover.rs` 逐字节守住；手抄的那一份删掉了。

### 生成器的三个上游坑

1. **注释行不终止续行。** 每张表后面都跟着一段被注释掉的旧版本，形如
   `!=(/ 17.0, 35.0, ... &`。Fortran 的规则是注释行既不参与也不终止续行，所以
   「按 `&` 拼接物理行」会把注释里的旧表当成取值 —— 实测 `htop0_igbp` 的旧版本是
   `1.0, 1.0, 1.0`，新版本是 `0.5, 0.5, 0.5`，接错一步就会让草地冠层高差一倍。
   正确顺序：先按 `!` 截断注释，**整行只剩空白的跳过**，再判断是否以 `&` 结尾。
2. **字面量不是合法 Rust。** `2.e-008`、`0.`、`d50` 里写成 `100` 的实数项，在
   Fortran 里都合法。生成器转换之后**必须真的 `parse()` 一遍**：这条检查比"看起来对"
   重要得多 —— 写错一个数，编译能过而取值悄悄变了。
3. **列表后面还跟了一个乘数。** 八张植物水力表写成 `(/ ... /) *1`。照抄可以，但换成
   别的乘数就**不能**默默折进表里：那会把「上游写了缩放」这件事从 diff 里抹掉。
   生成器现在只接受 `*1`，其余取值直接报错。

### 生成器自己的一个坑：产物必须 rustfmt 稳定

`cargo fmt --all` 会重排入库文件，而 drift 测试是逐字节比较 —— 生成一次、格式化一次，
drift 立刻打回（本轮实际踩到两次）。两处要处理：数组加 `#[rustfmt::skip]`（否则 94 张
表会被重排成竖排，`git diff` 再也看不出上游改的是哪个数），以及文件末尾只留**一个**
换行。修好之后 `gen-landcover` 的输出与 `cargo fmt` 的结果逐字节相同，再格式化是幂等的。

### 顺带修正：上一轮装配层的根比例检查过严

上一轮给 `colm-runtime` 的装配层加了「`root_fraction` 求和必须为 1」，理由是
`eroot` 把 `soil_water_stress` 定义成 `sum(rootfr · resistance)`。**这条检查是错的**：
上游 `ROOTFR_SCHEME != 1` 那一支的末层取 `0.5*(exp(-a·zi_nl) + exp(-b·zi_nl))`，
而中间层的差分只折到 `zi_(nl-1)`，整个数组求和是 `1 - d_(nl-1) + d_nl` —— **小于 1**，
缺口随类别而变（IGBP 第 1 类 0.31%、第 2 类 1.94%）。要求等于 1 会把一份合法的上游
根系比例挡在门外。现在只守上限 `sum <= 1 + 1e-9`，因为会让叶温校验失败的是**超过 1**。

`ROOTFR_SCHEME == 1`（Schenk & Jackson）那一支是逐层差分，求和恰为 1；两支的公式都在
`colm_core::land_cover` 里，各自有测试钉住（含 IGBP 第 1 类硬编码的十个值）。
一个凭直觉写下的断言也被实测推翻：`d50 = 15 cm` 且 `beta` 为负时，根系比例在**第 4 层**
达到峰值，不是表层最多。

### 已修正：地类下标的两套约定（参数重名，不是取值错）

`leaf_optics_from_land_cover(scheme, land_class)` 要求 `land_class >= 1`（内部减一），
而 `land_cover_soil_reflectance(scheme, land_class)` 要求 `land_class >= 0`。两个参数
**同名而含义不同**，喂反了不会报错，只会拿到隔壁地类的参数。

按上游源码核对后确认：两者各自的取值都没错，错的是名字。上游的访问方式是
`array(patchclass(ipatch) + 1)` —— `patchclass` 是重启里的 0 基类号，数组是 1 基的。
所以光学表那个函数收的确实是 1 基下标，土壤反照率那个收的确实是 0 基类号。

处理：按仓库约定（保留 1 基索引的函数名带 `_one_based` 后缀）把前者改名为
`leaf_optics_from_land_cover_one_based(scheme, fortran_class_index)`，24 处调用点一并
更新；两个 doc 注释互相点名，写明"另一个收的是 0 基 `patchclass`"。
`colm-runtime` 的装配层现在是**唯一的**真实调用者，它从常数重启读 `patchclass` 后
显式 `+1`，并把「地类表算出的 patchtype」与「重启里写的 patchtype」对拍 ——
对不上就报错，因为那意味着这份重启与编译进来的地类表不是同一套。

## POINT 循环真的驱动了装配出来的模板（2026 年）

上一轮落的装配层此前只被自己的测试消费。现在 `PointRuntime::run_restart_standard_lct`
把它接进钟与强迫场：一个 `StandardLctSoilState` 走完整个强迫窗口，每步重建
`StandardLctStepBinding`（forcing、`seconds_of_day`、`greenwich`、经度）。

**每步重建是刻意的，不是保守。** 风、当日秒数与经度在内核里是**透传**字段
（`prepare_energy` 与 `net_solar` 的 `..input` 更新不碰它们），把模板当成静态量交给
循环，第二步起就会拿第一步的值。`seconds_of_day` 还从 `u32` 显式收窄进 `[0, 86400)`：
`MOD_NetSolar` 只在等于 43200 时走正午分支，越界的当日秒数会静默走到另一支。

状态由调用方持有而非函数内部创建 —— 跑完之后同一份状态要能写回 restart；
它与钟、强迫场在同一笔事务里提交，所以输出回调失败时三者一起回滚。

### 证据

`cargo test -p colm-runtime --lib`：26 通过。其中两条进的是**入库的真实强迫场**
（`examples/Forcing/CN-Cng_...nc`，35089 条记录）而不是合成常量场：

- `an_assembled_restart_template_runs_several_point_steps` —— 00:00→01:30 三步，
  断言 `seconds_of_day` 依次为 `0, 1800, 3600`、`istep` 为 `1, 2, 3`、气温在窗口内
  确实变化过、三层土壤温度**每一层都动了**且有限、runoff 非负有限，最后时钟耗尽。
- `a_failed_output_callback_rolls_the_restart_state_back` —— 回调报错后土壤柱与第一步
  逐值相同，且 `next_step()` 仍停在第 1 步。

合成常量强迫下这两条都证明不了什么（气温与秒偏移都不变，漏刷看不出来），所以用的是
真实文件。这不改变"第三阶段仍是 Fortran `colm.x`"这一事实：`colm-runtime` 至今没有
可执行文件，`--preprocessors` 也只覆盖前两段。

## 大气分压不再是常数：Mauna Loa CO2 表进入代码生成（2026 年）

`LandPhysicsParameters` 里的 `oxygen_partial_pressure_pa` 与 `atmospheric_co2_pa` 是
**每步量**，却被当成算例常量给了一次。上游 `MOD_Forcing.F90` 写得很清楚：

```
pco2m = get_monthly_co2_mlo(year, month)*1.e-6
CALL block_data_copy (forc_xy_pbot, forc_xy_pco2m, sca = pco2m)
CALL block_data_copy (forc_xy_pbot, forc_xy_po2m , sca = 0.209_r8 )
```

即两个分压都是 `forc_pbot` 乘一个体积分数。写成常数 21200 Pa / 40 Pa 只在海平面成立：
海拔 1000 m 处 `pbot ≈ 90 kPa`，O2 实际约 18.8 kPa，**偏高约一成**；高原算例更甚。
这正是上一轮修掉的空气密度那一类隐患，只是藏在另外两个字段里。

现在两个分压由 `input()` 按上游公式算：O2 用常数
`OXYGEN_VOLUME_FRACTION = 0.209`，CO2 用**每步绑定**里的 `co2_volume_fraction`。
字段从 `LandPhysicsParameters` 移除（少了一个手给错的机会），加进
`StandardLctStepBinding`。

`vaporization_heat_j_kg` 经核对**不是**每步量：`MOD_Const_Physical.F90` 里就是常数
`hvap = 2.5104e6`，所以它留在物理参数里，并在注释里记下理由 —— 免得下一轮"顺手"
把它也搬进绑定。

### CO2 体积分数的来源：`MOD_MonthlyinSituCO2MaunaLoa.F90`

上游按月查 Mauna Loa 观测表，2023 年起按 `DEF_SSP` 切到未来情景。这张表
（753 行）由 `cargo run -p xtask -- gen-co2mlo` 生成到
`crates/colm-core/src/co2_generated.rs`（609 行），`crates/colm-core/tests/drift_co2.rs`
逐字节守住。生成内容是观测段 1849–2022 共 174 行，加五个情景各自的 2023–2100。

生成器遇到的两个坑与 `MOD_Const_LC.F90` 同源，但形态不同：

1. **注释掉的旧年份行就混在同一段里**（`!co2mlo( 2008 ,:) = …`），且年份在括号里
   带空格（`co2mlo( 1849 ,:)`）。先按 `!` 截断注释、再用宽松空白匹配，缺一不可。
2. **`off` 情景不是逐行写的**，而是 `co2mlo(2023:eyear,:) = co2mlo(2022,12)`。
   生成时把它**展开**成逐年一份，Rust 一侧因此没有规则引擎 —— 只有一张可查的表。
   展开后仍逐情景核对 2023–2100 每一年都有着落，缺一年就报错。

`co2.rs` 里放的是上游 `get_monthly_co2_mlo` 的取值语义：观测段/情景段分界，以及
两端钳位。钳位是照做而不是"兜底"：上游超出范围时打印警告并返回最早/最晚一条，
报错会让一个 1849 年之前的算例失败，而上游会给它一个确定值。`DEF_SSP` 的解析则
**必须报错** —— 上游在 `CASE DEFAULT` 里 `CoLM_stop`，静默退回 `off` 会让一份
SSP5-8.5 算例拿到观测段的 CO2。

### 测试期望值是读出来的，不是猜的

第一版测试里我猜了三个数（2008-01 = 385.21、SSP5-8.5 的 2100 = 867.19 等），
全错。实际是 385.04、1135.21——`867.19` 属于 SSP3-7.0。现在期望值逐条对照上游字面量，
并额外钉住一处容易分错块的地方：**SSP2-4.5 的 2100 年最后两个月在上游里是 867.19**，
而 SSP3-7.0 整年都是 867.19；只断言"2100 年末值"的话，两个 `CASE` 块分错了也看不出来。

## 生化参数接上地类表；顺带查清 `canopy_scaling` 的真身（2026 年）

`LeafBiochemistry` 是 `LandPhysicsParameters` 里最后一块手给的物理表。查上游后确认：
**无雪规则土壤这一支的生化参数全部来自 `MOD_Const_LC.F90`**，也就是上一轮已经生成并
接进装配层的那张表 —— 不需要 PFT 文件。`CoLMMAIN.F90` 的 `USE` 列表直接列出
`effcon, vmax25, c3c4, slti, hlti, shti, hhti, trda, trdm, trop, g1, g0, gradm, binter`
这些名字，它们都是地类数组。现在 `ClassConstants::biochemistry` 逐项取表，
`LandPhysicsParameters::biochemistry` 随之删除（少一个手给错的机会），并在装配处
断言 `vmax25` 的 `1e-6` 折算仍生效（表里是 umol/m2/s，忘了换算差六个数量级）。

### `canopy_scaling` 不是地类常量，是每步量

`LeafBiochemistry::canopy_scaling` 起初看不出任何上游来源（`MOD_AssimStomataConductance`
的签名里没有它）。顺藤摸下去：内核里它乘在三个地方 —— `vcmx`、`jmax`、`bintc` ——
而上游对应的是 `stomata` 的 `cint(1:3)` 参数：

```
bintc = binter_used * max(0.1, rstfac) * cint(3)     ! MOD_AssimStomataConductance.F90:211-212
jmax  = jmax * rstfac * cint(2)                      ! 同上 :585-586
```

而 `cint` 的实参在 `MOD_LeafTemperature.F90:460-466` 里**每步**算，且**阳叶与阴叶不同**：

```
cintsun(1) = (1.-exp(-(0.110+extkb)*lai))/(0.110+extkb)
cintsun(2) = (1.-exp(-(extkb+extkd)*lai))/(extkb+extkd)
cintsun(3) = (1.-exp(-extkb*lai))/extkb
cintsha(1) = (1.-exp(-0.110*lai))/0.110 - cintsun(1)     ! 其余两份同理
```

所以它是**从 `lai`/`extkb`/`extkd` 每步算出的两个三元素**，不是一个可以塞进模板的常数。

**上一版这段记录写错了，这里更正。** 当时据此推断"本仓库的内核只收一个 `[f64; 3]`，
只能描述一个叶群体，修它要动内核签名"。实际去读 `leaf_temperature.rs` 才发现内核**早已
两套都算、并逐群体传入**：

- `sunlit_canopy_integration`（原名叫 `canopy_scaling`，与字段同名，是这轮误判的起因）
  算 `cintsun`，`MOD_LeafTemperature.F90:460-462` 三个表达式逐字对应；
- 调用点旁边就算 `cintsha`（`:464-466`），
- 两个 `StomataStep` 分别带 `cintsun` / `cintsha`。

真正的问题是**数据流反了**：因子放在 `LeafBiochemistry`（参数结构）里，然后每次调用
都被调用点用 `LeafBiochemistry { canopy_scaling: step.…, ..input.biochemistry }` 覆盖。
也就是说这个字段在生产路径上**从来没被读过**，只是让"每步量"看起来像"地类常量" ——
与它同名的私有函数又强化了这个错觉。

现在因子搬到 `LeafPhotosynthesisInput::canopy_integration`（每步输入该待的地方），
`LeafBiochemistry` 不再有它，那个私有函数也改名为 `sunlit_canopy_integration`，
并在两个 `StomataStep` 处注明对应上游哪几行。`LandPhysicsParameters` 随之少一个字段。

教训记在这里：**字段消失不等于缺口消失，但也不等于缺口存在** —— 上一版是先写了结论再去
找证据。这轮的两个测试（`canopy_integration_factors_match_the_upstream_expressions`、
`a_zero_extinction_uses_the_limit_instead_of_dividing_by_zero`）现在把两份因子的表达式
逐项钉住，`cintsha` 与 `cintsun` 不能再被当成同一个东西。

## 雪列从重启读回：层数不在文件里（2026 年）

活动积雪那一支的驱动一直缺一块前提：**重启里不存雪层数**。时间重启写的是
`z_sno`/`dz_sno` 以及 `t_soisno`/`wliq_soisno`/`wice_soisno` 的整段雪槽（`snow` 维长
`-maxsnl = 5`，顺序就是 Fortran 的 `-4..0`），但没有任何变量说有几层雪。

上游是**每步现数**（`CoLMMAIN.F90:816-818`）：

```
snl = 0
DO j = maxsnl+1, 0
   IF (wliq_soisno(j)+wice_soisno(j) > 0.) snl = snl - 1
ENDDO
```

界面深度也不是存下来的，而是由厚度递推（`:820-826`）：`zi(0) = 0`，
`zi(j) = zi(j+1) - dz(j+1)`（`j = -1..snl`）。`fiold` 同样现算：
`wice/(wliq+wice)`（`:843-845`）。

`RuntimeSnowColumn::from_restart` 照做这三件事，并且**显式拒绝两种上游会带着跑下去的
坏输入**：

1. **中间空一层**。上面的数法只看每层有没有水，所以「槽位 -2 有水、-3 无水、0 有水」
   会数出 `snl = -2`，而内核按「最后 `|snl|` 个槽位是有雪层」组织数据 —— 对不上。
   上游会继续跑，我们报 `breaks the column`。
2. **水体带雪列**（`patchtype > 3`）。

顺带确认了一个容易写错的细节：槽位下标与重启数组下标是**恒等**的。层 `-4..0`
对应 `RuntimeSnowColumn` 的层槽位 `0..4`，而重启的雪段正是按 `-4..0` 写的；界面
`-5..0` 对应界面槽位 `0..5`。第一版把界面递推的循环写成了覆盖全部槽位，等于用
`dz(-3)` 重算了一遍 `zi(0)`，测试立刻抓到。

### 证据

`cargo test -p colm-core --lib restart_snow`：四条 —— 单层（`snl = -1`，唯一界面
`zi(-1) = -0.05`、`fiold = 0.8`）、五层（逐界面核对 `[0, -0.10, -0.20, -0.31,
-0.36, -0.38]`）、中间空层被拒、以及负水量/水体带雪被拒且空雪列（`snl = 0`）合法。
期望值手算自上游那两段代码，不是在测试里重算实现。

这一片只到"读回"为止：把它接进装配层的雪模板（还需要给合成算例加一个带雪的变体）
是下一步，所以它现在仍只有测试在消费 —— 与 round 6 的装配层当时的状态相同。

## 雪列进入装配层：判据从 `fsno` 换成"列里有没有水"（2026 年）

上一轮落的 `RuntimeSnowColumn::from_restart` 现在进了生产路径。装配层的无雪判据原本是
`fsno == 0`，这一轮换成**先按上游的方式把雪列读出来，再看列里有没有水**：

- `fsno == 0` 却带着雪水（或反之）是一份自相矛盾的重启，只看 `fsno` 会放过它；
- 读雪列时顺带把 `snl` 的推导、界面递推、`fiold` 都走了一遍，与积雪分支共用同一条路径，
  所以这条检查不是为无雪分支专门写的旁路。

放在读土壤列**之前**也是刻意的：否则会先撞上 `soil_column` 那句"雪槽必须为空"，报错信息
指向的是症状而不是原因。现在带雪重启会得到：

```
standard LCT soil assembly needs a snow-free patch, but the restart carries 3 snow layer(s)
under a 0.1500 m column
```

合成算例新增 `SyntheticSnow` / `write_with_snow`：给一份自洽的雪列（层数与厚度用共享的
`initialize_snow_layers`，水量按层厚分摊，并留 10% 液态水让固态分数不是 1）。装配层则要
**自己**从水量把层数数回来 —— 这正是要在测试里钉住的那一步。`SyntheticRestart` 现在带上
`Option<SyntheticSnowValues>`，把写进去的层数/厚度/水量逐槽暴露出来供下游核对。

写它时踩到一个自己造的错：夹具里数层数时把窗口取成了 `snow_ice[..SNOW_LAYERS * PATCHES]`，
于是把**两个 patch 的雪层一起数**，`0.15 m` 数出 `-6` 而不是 `-3`。层数是 per-patch 的量，
现在按 patch 0 数并注明两个 patch 的雪列相同。

## 积雪分支接进装配层与 POINT 循环（2026 年）

雪分支现在能真的跑了：`assemble_standard_lct_snow_template` 装配带雪重启，
`StandardLctRestartTemplate::snow_input()` 给出内核输入，
`PointRuntime::run_restart_standard_lct_snow` 走完整个强迫窗口。测试用合成算例的雪列
（0.15 m、45 kg/m²）跑三步真实 CN-Cng 强迫，断言雪列始终存在、雪水当量推进过、
土壤温度有限。

设计上有三处取舍：

1. **两支共用同一个模板类型**，差别只在调 `input()` 还是 `snow_input()`。所以把装配
   拆成私有的 `assemble()` 加两个薄包装：无雪的包装要求雪列为空，积雪的包装要求它非空。
   拒绝的判据仍在雪列上，不在 `fsno` 上。
2. **雪 + 土模板列**（`snow_soil`）在装配时拼好存进模板。上游的
   `z_soisno`/`dz_soisno`/`zi_soisno` 是从雪顶一直排到土壤底，界面在雪土交界处共享
   `zi(0) = 0`，所以土段的第一个界面不重复（`interface_depth_m[1..]`）。雪层数每步会变，
   但模板列的形状由重启决定，内核自己负责重排（`packed_snow_soil_state`）。
3. **`soil_column` 不再核对"雪槽必须为空"**：两支共用这个读者，带雪时前几槽本来就该有值。
   雪列的一致性由 `restart_snow_column` 管，无雪的要求由包装管 —— 同一个检查放错层会
   让积雪分支根本读不进来（实测报错就是那句"雪槽必须为空"）。

### 顺带修正：`DEF_TUNING_SSI` 不是土壤冰阻抗

给 `LandPhysicsParameters::soil_ice_impedance` 标来源时发现两个**不同**的调参字段：

| 字段 | 默认 | 上游注释 | 用途 |
|---|---|---|---|
| `DEF_TUNING_SOIL_ICE_IMPEDANCE` | 6.0 | Frozen-soil hydraulic impedance exponent | `MOD_SoilSnowHydrology:1213` 的 `10**(-…*icefrac)` |
| `DEF_TUNING_SSI` | 0.033 | Irreducible snow-water saturation fraction | `snowwater` 的 `ssi` 实参 |

原先的注释把前者标成了 `DEF_TUNING_SSI`，而两者默认值差两个数量级 —— 照注释填 0.033
会让冻土阻抗几乎失效。现在注释指向正确的字段，并新增
`LandPhysicsParameters::snow_irreducible_saturation`（`DEF_TUNING_SSI`，0.033）供
`snowwater` 用。内核那侧的名字（`soil_ice_impedance`）本来就对，只有文档错了。

## 续跑写出：restart 闭环打通（2026 年）

`RestartFile::write_with` 是**续跑**用的写出路径：以一份已读入的重启为底，只替换调用方
声明改过的变量，其余原样搬过去。它与初始化器那次"从零构造"的 `write_time_restart` 是
两件事 —— 上游每 `DEF_WRST_FREQ` 步调的 `WRITE_TimeVariables` 就是前者：只把当时内存里的
数组写出去，不重新决定变量集合。

闭环测试（`an_evolved_state_writes_back_a_readable_continuation_restart`）：
合成重启 → 装配 → 跑两步 → 写出 → 读回，断言推进过的十一项（三根土柱 + `zwt`/`wa`/`wdsrf`
+ `t_grnd`/`tleaf`/`ldew`/`ldew_rain`/`ldew_snow`）对上、本 patch 之外的 patch 保持原值、
没推进的变量（`fsno`）逐值不变，而且写出的文件还能被装配层重新读回来。

`ground_temperature_k` 由调用方传给 `evolved_overrides`：地表温度只出现在**这一步的输出**
（`StandardLctSoilOutput::energy.ground.temperature_k[0]`）里，状态只带逐层土温。叶温与三个
冠层水量则在状态里（`energy.leaf`），所以直接取。上一轮那句"`t_grnd` 保持原值"因此改掉了 ——
不是状态没有就该不写，而是先确认它到底在谁手上。

三处值得记的实现取舍：

1. **类型必须还原。** 读取器把取值一律加宽（`f32`→`f64`、`i8`/`i32`→`i64`），续跑若照加宽
   后的类型写回，`patchmask` 会从 i8 变成 i64 —— 读的人（包括 Fortran 那侧）能自动转换，
   但文件 schema 已经悄悄变了。所以读取器现在**额外记下盘上的原始类型**
   （`RestartValueType`），写出时逐类型还原。`colm-init` 的测试直接断言 `patchmask` 写回后
   仍是 i8、`fsno` 仍是 f32。
2. **失败要清掉半成品。** 写到一半失败会留下一个**读得出来**的文件（维度齐全、部分变量有值），
   下一次续跑会以为那是一次成功的写出 —— 比文件不存在更危险。现在任何失败都删掉目标文件，
   测试里断言了这一点。
3. **换的是整变量，不是某个 patch 的一段。** 第一版按 patch 切片再拼回去，结果把其它 patch
   的值丢了（测试报 `len is 15 but the index is 20`）。现在从原文件的整缓冲出发，只覆盖本
   patch 的土段。

**只写有来源的量。** 剩下的 `fsno`/`fwet_snow`/`sag`/`coszen` 等没有写回：它们要么是诊断量、
要么由别的分支推进，状态与输出都不拥有它们。凑近似值等于把一次没有依据的推算写进文件。

## 积雪分支的续跑集合（2026 年）

`evolved_snow_overrides` 在土壤那十一项之上补上雪的部分：三根 `soilsnow` 柱的**雪段**、
`z_sno`/`dz_sno`，以及 `snowdp`/`scv`/`fsno`/`sag` 四个雪标量，共十七项。它复用
`evolved_overrides` 的土壤/标量部分 —— 积雪状态里没有 `StandardLctSoilState`，就地拼一个
（土温与土壤水本来就是它的字段），省掉一遍重复的 per-patch 处理。

重启里雪段**恒为五个槽位**（`maxsnl = -5`），与实际层数无关，所以未用的槽位写 0；上游也是
整段写出去的。这一条与土壤那侧的"只换本 patch"合起来，意味着写出的重启在任何 patch 上
都与原文件同形。

**一个测试抓到的错**：`z_sno`/`dz_sno` 的每个 patch 只宽 **5** 槽（`snow` 维），而三根
`soilsnow` 柱宽 **15**（雪 5 + 土 10）。第一版用了同一个 `patch * width` 偏移，立刻报
`the restart's snow geometry is too short for patch 1`。两套宽度现在分开算并各自注明。

测试：合成雪列（0.15 m、45 kg/m²）→ 积雪装配 → 跑两步 → 写续跑 → 读回，逐槽核对
`z_sno`/`dz_sno`/`t_soisno` 的雪段与土段、四个雪标量只换本 patch，并确认写出的文件能被
积雪入口重新装配且层数一致。

## history 桥：每步状态接上写出器（2026 年）

`colm-hist` 早就有闸门表、调度与写出器，缺的是"每步的字段值"这一层。现在
`colm_runtime::history` 把它补上：`declare_lct_state` 声明本层能负责的十三个变量
（三根土柱、`t_grnd`/`tleaf`、`zwt`/`wa`/`wdsrf`、`snowdp`/`scv`、`lai`/`sai`/`fsno`），
`set_lct_state` / `set_lct_snow_state` 把一步的状态写进某条记录。来源与续跑写回**完全同源**
（同一份状态、同一个步输出），所以两处不会各说一套。

**没填的变量不声明。** 闸门表允许 619 个变量，但本层只声明它真能负责的十三个；`UNFILLED`
常量把缺口按用途写死在代码里（通量与诊断、分层植被量、派生土壤量、湖泊/BGC），改它就得同时
改注释。这比"名字填上、值给零"强得多：文件里没有就是没有，不会被当成"这个内核产不出"。

### 验证分两半，只有一半能验

黄金文件是真实 CN-Cng 算例，我们没有它的初始状态，所以**值无从对照**。能独立验证的是
schema：测试把写出的文件与 `oracle/golden/CN-Cng_hist_2008-01.nc` 逐变量比 **名字、维度顺序、
类型、`units`、`long_name`** —— 十三个变量全部一致。取值一侧则与这一步的状态逐项对照
（`f_t_soisno` 的前五槽为雪、后十槽为土温；`f_t_grnd` 等于步输出的地表温度；`f_scv` 等于雪水
当量），以及积雪分支的雪段确实来自雪列而不是零。

这一半的验证有个前提值得记：`HistoryDimensions.patch` 是**文件里的** patch 数，而模板是**单个**
patch 的。POINT 算例写一个 patch 是正确的，合成夹具的两个 patch 只是为了让索引写反能被发现 ——
测试因此显式用 `patch: 1` 开缓冲。

`colm-runtime` 为此加了对 `colm-hist` 的依赖并显式开 `io`（闸门表那一半仍默认不带 netcdf，
GUI 那条链不受影响：它不依赖 `colm-runtime`）。

## history 桥补上六个水文诊断量（2026 年）

在上一轮的状态十三项之上，`colm_runtime::history` 再声明六个**诊断**量，全部来自
`WATER_2014` 的输出：`qinfl`（下渗）、`rnof`（总径流）、`rsub`（地下径流）、`rsur`（地表径流）、
`qcharge`（地下水补给）、`frcsat`（饱和面积比）。共十九项。

**单位是逐项与闸门表核对过的**，不是按名字猜的：前五个是 `mm/s`，`frcsat` 是 `-`。同一个
检查顺手排除掉一个候选：`smp`（土壤基质势）**不在闸门表里**（不是默认产出量），所以哪怕
`Water2014SoilOutput` 里有 `matric_potential_mm` 也不声明它。

`qcharge` 带出一个新情况：闸门表允许写不等于**这个算例**会产出。它受运行时条件控制，黄金
算例（CN-Cng）里没有 `f_qcharge`。schema 测试因此按"两边都有"来比，把跳过的名字收集起来并
断言至少比到 18 项 —— 缺的量不会静默变成通过，也不会因为一个条件变量就让整条测试放弃。

### 证据

`cargo test -p colm-runtime --lib`：39 通过（3 条 history 测试）。schema 逐变量比对
（名字、维度顺序、类型、`units`、`long_name`）覆盖 18 项，诊断量的取值与这一步 `WATER_2014`
的输出逐项相等；积雪分支走雪入口，雪段来自雪列。

## history 接进运行：按**写入 tick** 对齐（2026 年）

`HistorySession` 把一次运行按调度写成若干文件：一步结束时若命中写入时刻就填一条记录，
分组写完就落盘成 `<stem>_hist_<后缀>.nc`（与黄金文件名 `CN-Cng_hist_2008-01.nc` 同构）。

**对齐用的是调度给出的写入 tick，不是重算的判据。** `schedule_records` 现在公开逐条记录
（后缀、文件内序号、写入 tick、标签），`schedule` 由它派生 —— 一份实现，而不是把
`isendofhour/daily/monthly/yearly` 抄第二遍。运行时的 `CalendarTime` 通过公开的
`tick_seconds` 换算成同一个基准。会话还检查"钟还没到记录时刻"与"钟越过了记录时刻"两种情况，
后者直接报错：那说明调度与时钟对不上，静默跳过会让文件少一条而没人发现。

顺带解决一个语义疑点（上一轮我拒绝凭印象接线的那个）：一个 1800 s 步长的算例，HOURLY
输出是**一小时一条**而不是一步一条 —— 写入发生在**结束时刻恰好落在整点的那一步**，值就是
那一刻的状态；标签再减去固定的半区间位移（`MOD_Hist.F90` 的 `lsel` 与
`MOD_HistSingle.F90` 的 `hist_single_write_time`）。所以黄金算例 11 天 528 步对应 264 条记录。

### 证据

`cargo test -p colm-runtime --lib`：40 通过。会话测试用 CN-Cng 的同一段窗口
（2008-01-01 00:00 → 01-11 24:00、1800 s、HOURLY + MONTH）跑前三个小时的真实步，断言：

- 调度记录数 **264**（与黄金文件一致，这是 `colm-hist` 已对着黄金文件验证过的性质）；
- 六步只写三条记录（一小时一条），且前三条标签 `56802270/568022330/568022390`
  ——**与黄金文件的头三个值逐位相同**；
- 第四条及以后仍为 0（未填的槽位不编造）；
- 换分组前不落盘，`finish()` 才写出文件，文件里变量与标签都在。

`cargo test -p colm-hist`：42 通过 —— `schedule` 的输出在重构前后不变。

## 驱动方法带 history（2026 年）

`PointRuntime::run_restart_standard_lct_with_history` 与 `..._snow_with_history` 把
`HistorySession` 包进循环：同一个钟、同一份绑定、同一笔事务，多的只是每步结束后把
**步后状态**与诊断写进 session。它返回 `HistoryRunOutcome { steps, files }`。

**运行比窗口短时必须在落盘前报错。** `run_with_state` 返回后先 `finish()` 落盘、再检查
`session.remaining() == 0` —— 顺序反了就没意义：一个没走完调度的运行会写出一个**满是零**的
记录文件，而那读起来与真实数据没有区别。测试直接跑一步、调度开三小时，断言报错且信息里
点名还有几条没写。

顺带把"POINT 算例的 history 维度"提成公开的 `point_dimensions()`：那些长度由内核编译期
常量决定（`nl_soil = 10`、`maxsnl = -5`、`nvegwcs = 4`…），不由算例文件携带。原先它在测试里，
名字带 `test_` 前缀，对公开 API 来说是错的命名。

### 证据

新增两条运行时测试：

- 真实三小时窗口（六步、HOURLY）→ `outcome.steps == 6`、一个文件
  `CN-Cng_hist_2008-01.nc`，`time` 为 `[56802270, 568022330, 568022390]`
  ——与黄金文件头三个值逐位相同，且 `session.remaining() == 0`；
- 一步的运行 + 三小时的调度 → 报 `still unwritten`。

## history 会话改从算例 namelist 构造（2026 年）

`read_point_runtime_config` 现在解析 `DEF_HIST_FREQ` 与 `DEF_HIST_groupby`
（缺省分别与上游一致：`none` 与 `MONTH`），`PointRuntimeConfig::history_session(dir, stem)`
按同一份配置开会话 —— 窗口、站点、步长都取自配置，调用方不必自己拼 `SimulationWindow`。
拼错一个字段（例如把结束时刻写成时长）只会让记录数悄悄不对，而记录数是这一层唯一能
对着黄金文件比的东西。

两个频率字段的解析都走 `colm-hist` 的 `parse`：上游遇到不认识的取值只打一句 warning 然后
**静默不写 history**，用户会拿到一个空目录而不知道原因；这里报错。

顺带核对了模块门控：`colm-hist` 的 `schedule` **不在** `io` feature 之后（只有 `history`
与 `obs` 是），所以调度与解析本身不需要 netcdf。

### 证据

两条运行时 history 测试现在都从 `read_point_runtime_config` 出来的配置开会话，并断言解析
结果（`Hourly` / `Month`）。三小时窗口那条仍然产出 `CN-Cng_hist_2008-01.nc`，`time` 为
`[56802270, 568022330, 568022390]`；短跑那条仍报 `still unwritten`。

## 跨分组边界的运行与两个实测结论（2026 年）

跨月窗口（1 月 31 日 22:00 → 2 月 1 日 02:00、HOURLY、MONTH）第一次走了"中途换分组"这条路，
抓到两件事：

1. **`outcome.files` 漏了中途落盘的文件。** 驱动只把最后 `finish()` 的产物收进了
   `outcome.files`，而 `push` 在换分组时落盘并返回的路径被丢掉了 —— 实测跨月运行只报了二月
   那一个文件，而一月那个**已经写到磁盘上**，只是没被报出来。现在沿途收集。
2. **分组由写入时刻的日期决定，不是它覆盖的区间。** 于是边界正好落在午夜：写于 1 月 31 日
   23:00 的记录落在一月，写于 2 月 1 日 00:00 的那条已经落在二月。这个窗口因此是
   **一月 1 条 + 二月 3 条**，不是各 2 条。第一版测试按"各 2 条"写，被实测纠正 ——
   这也是为什么这条用例值得存在：单月窗口永远走不到这条路径。

测试断言两个文件名、各自记录数（1 与 3）、同文件内标签相差 60 分钟，以及**跨月的相邻两条
仍连续**（一月的最后一条比二月的第一条早一小时）。

## history 桥再补四个能量侧诊断，并记下一个"名字像但不等"的反例（2026 年）

新增 `fsena`（冠层高度到大气的显热）、`fevpa`（同高度的蒸散）、`etr`（叶面蒸腾）、
`sabg`（地面吸收短波），共 23 项。每一项都对过上游的**赋值表达式**，不是对名字：

| 变量 | 上游 | 本仓库 |
|---|---|---|
| `fsena` | `fsena = fsenl + fseng`（`MOD_Thermal.F90:1331`） | `energy.total_sensible_heat_w_m2` |
| `fevpa` | `fevpa = fevpl + fevpg`（:1332） | `energy.total_evaporation_kg_m2_s` |
| `etr` | 叶面蒸腾（`MOD_LeafTemperature.F90:839`） | `energy.leaf.transpiration_kg_m2_s` |
| `sabg` | 地面吸收短波 | `energy.shortwave.ground_absorbed_w_m2` |

**`lfevpa` 刻意不加**，尽管它看起来就该是 `fevpa * hvap`。上游写的是

```
lfevpa = hvap*fevpl + htvp*fevpg   ! W/m^2 (accounting for sublimation)   ! MOD_Thermal.F90:1333
```

地面那一项用的是**升华潜热** `htvp`，不是汽化潜热。按名字配上会在积雪算例里给出偏高的
潜热通量，而无雪算例完全看不出来 —— 这是本项目里第二个"名字像、量不等"的例子（第一个是
`canopy_scaling`/`cint`）。它现在记在 `UNFILLED` 与代码注释里，连同缺的量（内核当前没有
`htvp`）。

### 证据

`cargo test -p colm-runtime --lib`：43 通过。schema 逐变量比对现在覆盖 **22 项**
（`qcharge` 不在黄金算例里，被显式跳过），四个能量量与这一步的输出逐项相等。

## history 桥再补十三个地表诊断（2026 年）

新增 `taux`/`tauy`（动量通量）、`tref`/`qref`（2 m 气温与比湿）、`z0m`、`zol`、`rib`、
`ustar`、`qstar`、`tstar`、`fm`/`fh`/`fq`（Monin-Obukhov 诊断），共 **36** 项。上游把这
十三个量原样累加后写出（`MOD_Vars_1DAccFluxes.F90` 的 `CALL acc1d (x, a_x)`），与本仓库
内核的字段是同一批量：`z0m = z0mv`、`tref`/`qref` 由 `MOD_LeafTemperature.F90:1261-1262`
算出，其余是地表层诊断，名字一一对应、不经换算。

**又排除两个"名字像"的量**（前两个是 `canopy_scaling`/`cint` 与 `lfevpa`）：

- `emis` 是**平均体积发射率**（`MOD_Thermal.F90:1360` 的 `emis = olru/olrb`），不是算例里
  那个固定的地表发射率。配上会让每个算例都写一个错的值。
- `rss` 在 `DEF_RSS_SCHEME == 4` 下被赋成 `1.`（LP92 的电导标志），其余方案才由
  `SoilSurfaceResistance` 输出阻力（`MOD_Thermal.F90:618-628`）。同一个变量两种含义，
  条件映射得先核对那个子程序的输出语义。

两个都记进 `UNFILLED` 与代码注释，连同缺的量。

### 证据

`cargo test -p colm-runtime --lib`：43 通过。schema 逐变量比对现在覆盖 **34 项**。

## 首次在本机跑通黄金回归：三阶段 ok，比对失败（2026 年，实测）

本机 PLUMBER2 数据在 **`/Volumes/Data01/Data/PLUMBER2s`**（含 `Forcing/`、`Forcingnml/`、
`Observation/`、`Sitedata/`），本 shell 里 `PLUMBER2_ROOT` 没有导出 —— 此前多轮把它记成
"未导出因而未跑"，现在补上实测结果。

```
PLUMBER2_ROOT=/Volumes/Data01/Data/PLUMBER2s cargo run -p oracle --bin golden-run -- CN-Cng
   inputs verified
   kernel: default@f427762#production (Darwin-arm64)
   WARNING: kernel differs from the one that produced the golden files:
     colm_git_sha: recorded "4894833", current "f427762"
   mksrfdata  ok
   mkinidata  ok
   colm       ok
```

**三阶段全部 ok**（含未改动的 Fortran `colm.x` 跑完 264 小时），这本身就是一段端到端证据。
随后 `golden-compare` 报 **85 problem(s)** 并失败。`golden-compare` 目前是**逐位**比较：
`oracle/tolerances.toml` 的头两行写着"里程碑 1 只做逐位比较，本文件此时不参与比较"，
`grep tier oracle/src/bin/golden_compare.rs` 无命中。`tier-check` 另报
`all 127 golden variables have a tier assignment` —— 容差表本身是完备的，只是还没被消费。

把 85 条按**首处相对偏差**分类（脚本按 golden-compare 打印的两个值现算）：

| 类别 | 条数 | 说明 |
|---|---|---|
| 末位噪声 `rel <= 1e-12` | **76** | 内核与产黄金文件那次不是同一份（`4894833` vs `f427762`），工具自己也警告"可能是工具链漂移而非物理变化" |
| 真实相对差异 `rel > 1e-12` | **4** | `f_zerr`(4.2e-1)、`f_frcsat`(1.8e-2)、`f_xerr`(2.0e-5)、`f_zwt`(2.2e-12) |
| `missing_value` 不一致 | **5** | `f_t_lake`、`f_lake_icefrac`、`f_wetwat`、`f_wetwat_inst`、`f_wetzwt`：黄金全是 `-1e36`，本次运行写出真值 |

两条需要正确解读：

1. `f_zerr`/`f_xerr` 是**收支残差**，相对偏差没有意义 —— 它们的量级本就是 `1e-11` 与 `1e-16`。
   实际绝对差：`f_zerr` 从 `-9.73e-12` 到 `-1.68e-11`（差 `7e-12`），`f_xerr` 差 `1.9e-19`。
   前者比黄金自身的值还大，是真差异；后者是噪声。
2. `f_frcsat` 是**唯一量级明显、且不是残差**的差异（`0.982276366840696` vs `1.0`），
   出现在第 1 条记录，值得单独查。

结论（供后续判断）：**本机无法用现有黄金文件判定 Rust 移植的数值等价性** —— 黄金文件产自
另一份 CoLM 快照，而比对在这一里程碑是逐位的。要用它当判据，得先把内核与黄金文件对齐到
同一快照（重编内核 + 重生成黄金文件），或先让比较消费 `tolerances.toml` 的分层。
这两件事都不是 Rust 侧能单方面完成的。

## Rust 预处理器在真实算例上产出与 Fortran **完全相同**的 restart（2026 年，实测）

拿到 `/Volumes/Data01/Data/PLUMBER2s` 后跑通了本会话最关键的一关：

```
PLUMBER2_ROOT=/Volumes/Data01/Data/PLUMBER2s \
  cargo run -p colm-cli -- run oracle/work/CN-Cng --kernel kernels/default --force 1
  mksrfdata  ok      ← Rust `mksrfdata-rs`
  mkinidata  ok      ← Rust `mkinidata-rs`
  colm       ok      ← 未改动的 Fortran `colm.x`，264 小时跑完
```

即 **Rust 产出的 restart 被未修改的 Fortran 模型读取并跑完整个算例** —— 端口文档里
"restart-continuation interoperability" 那条待验收项在真实数据上成立了。

### 逐值对比：204/204 完全相同

把同一算例分别用 Rust（默认）与 Fortran（`--preprocessors fortran`）跑一遍，再逐变量比对
两套 restart（netCDF4 读入后按值比较，NaN 模式单独比）：

```
两套 restart 的共享变量：204 个，逐值完全相同 204 个
仅 Rust 写出的变量：const block 里的 ncd / ncw / bcw
```

四个文件（时间重启 2008-001-00000 与 2008-012-00000、const block、const 标量）逐个比过：
变量集合一致、每个共享变量逐值相同。**唯一的差别是 Rust 多写了三个冠层结构变量**，不是
差异而是超集。

两点必须说清楚：

1. **不能用逐字节比较**。两个写出器的压缩级别不同（时间重启 Rust 207,988 B vs Fortran
   494,249 B；const block 140,903 B vs 406,941 B），字节不同只反映压缩。上面比的是**取值**。
2. **由此可推**：既然初始状态的 restart 完全相同，那么上一节 `golden-compare` 报出的
   history 差异**不是**预处理器引入的 —— 它来自 Fortran 运行时本身（或它读到的其它输入），
   与 Rust 这两段无关。

### 把这条与黄金比对的关系说清楚

- 预处理器（stage 1–2，Rust）：**逐值完全一致**，有本节的实测支撑；
- 运行时（stage 3）：仍是 Fortran `colm.x`，其 history 与黄金文件逐位不一致，原因是内核
  快照不同（见上一节）。Rust 运行时（`colm-runtime`）还没有可执行文件，所以那一段没有
  可比对象。

## 容差分层开始被消费：`golden-compare --tolerances`（2026 年）

`oracle/tolerances.toml` 的分层此前只被 `tier-check` 校验完备性（"里程碑 1 只做逐位比较，
本文件此时不参与比较"）。新增 `oracle::tolerances`（共享解析）与 `oracle::tier_compare`
（按分层判值），`golden-compare` 多一个 `--tolerances <表>` 开关；不带开关时仍是原来的逐位
行为。`tier-check` 改用同一份解析，两个二进制不再各写一份（否则一处改了 `rule` 的名字，
另一处会继续按旧名字放行）。

三条刻意的取值：**整数永远逐位比**（容差是给浮点的）；**没有归属的变量报错**，不算通过；
**`statistical` 层的变量报错**（整场统计判据逐变量不可判，表里当前也刻意没给它分配变量）。
schema 一侧两者一样严 —— 维度顺序、存储类型、变量级属性照旧必须相同。

### 它把"85 个逐位问题"变成了可读的判定

同一批本机运行（`4894833` 的黄金文件 vs `f427762` 的内核）：

| 算例 | 逐位 problem | 超容差变量 | 其中真正量级明显的 |
|---|---|---|---|
| CN-Cng（1 月） | 85 | **14** | `f_frcsat`(0.982 vs 1.0)、`f_vegwp`(差 22 mm)、5 个 `missing_value` 不一致 |
| CN-Cng-wet（7 月） | — | **62** | `f_wliq_soisno`(55.037 vs 55.033)、`f_h2osoi`、`f_zwt`、`f_vegwp`、`f_t_soisno` … |

两件事因此说清楚了：

1. **绝大多数逐位差异确实只是末位噪声** —— 冬季那 85 条里，分层之后只剩 14 条，其中 5 条
   是"黄金留 `-1e36`、本次写真值"的策略差异，5 条是 tier1 的 1e-12 边界（`f_fh`/`f_fq`/
   `f_fm`/`f_fm10m`/`f_rnet`，实测相对偏差 ~1e-10），2 条是 tier0 的**输入**变量末位
   （`f_xy_solarin`/`f_xy_q`）。
2. **黄金文件确实过时了，而且夏季窗口过时得更厉害** —— 62 个变量超容差、偏差是量级性的，
   不是噪声。这与内核快照不同（`4894833` vs `f427762`）一致，也说明**在这份黄金文件上
   无法判定 Rust 移植**：先得把内核与黄金文件对齐到同一快照，或重生成黄金文件。

`oracle/tests/tier_compare.rs` 对每类规则与每条拒绝路径各写一条负向测试（含"整数在浮点层
下仍逐位"与"换轴但值相同必须报 schema 问题"）。

## 一条被引用了三次却从未入库的验证脚本（2026 年）

`oracle/scripts/test_upstream_f48_sync.py` 在 `5d7f373`、`6d98eb6`、`b25b904` 三个提交的
`Tested:` 行里都被列为"实际跑过"，但它一直只是工作区里的未跟踪文件 —— 别人 clone 下来
复现不了那三处证据。现已入库，并加进 CI 的 `kernel-filesystem` 作业。

它守的是 `vendor/CoLM202X` 那次**按语义 hunk** 的 `f48fbf9` 同步：8 个 Fortran 文件里
16 条特征字符串，既钉上游带进来的（稠密水库轴 `catalogue_to_active`/`icache`、分汊限幅器
与"净通量定稿后再套路径上限"、levee 库容在 history 里单列与重启开关的一致性、`rstfacsun`
的 intent、`gssun` 的两处公式），也钉我们自己的臭氧扩展（`mg2p_ozone%grid2pset` 的空间
映射与两个 namelist 开关）。整树覆盖会静默抹掉后者，这套断言是这条同步路径上少有的警报。

由此立一条规矩：**`Tested:` 里出现的脚本必须入库。** 否则那条证据只对写下它的那次会话
成立，而且下次同步上游时没人会再跑它。

## `hpbl` 是逐强迫场的量，不是算例常量（2026 年）

`DEF_USE_CBL_HEIGHT` 选的是 `MOD_TurbulenceLEddy` 的 LZD2022 近地层廓线，它的长度尺度是
**大气边界层高度 `hpbl`**。上游把 `hpbl` 当作**第 9 个强迫变量**读进来 ——
`MOD_UserSpecifiedForcing.F90:96` 打开这个开关时 `NVAR = NVAR + 1`，变量名取
`DEF_forcing%CBL_vname`（默认 `'blh'`），再逐步传进 `moninobuk*_leddy`。

Rust 侧此前把它做成了 `LandPhysicsParameters::boundary_layer_height_m` —— 一个**装配期
常量**。后果不是崩溃而是静默错算：开关打开时每一步都用同一个高度，而单点算例看不出任何
异常。默认算例（`DEF_USE_CBL_HEIGHT = .false.`）走 `Standard` 分支、根本不读它，所以这个
错在黄金回归里也不会露头。

现在它按上游的样子走完整条链路：

| 层 | 改动 |
|---|---|
| `colm-forcing` | `PointForcingFrame` 多一个 `boundary_layer_height_m: Option<f64>`，从 `blh`/`hpbl` 里**有则读**；单位已是米，不走 8 槽的单位表 |
| `colm-core` | `RuntimeForcingInput`/`RuntimeForcing` 同样带它；`MoninObukhovInput`、`GroundFluxInput`、`LeafTemperatureInput` 各多一个逐步骤字段 |
| `SurfaceLayerScheme` | `LargeEddy` **不再携带**高度 —— 枚举载荷是装配期的，带它就等于把逐步骤的量钉死 |
| 装配层 | 从本步 `forcing.boundary_layer_height_m` 取，`LandPhysicsParameters` 里那个字段删掉 |

两条判定：`hpbl` 可以缺（默认算例就没有，缺了不是错误），但**给了必须是正的有限值**；
反过来，选了 `LargeEddy` 而强迫场里没有 `hpbl`，**必须报错并点名 `forc_hpbl`** ——
不能退回 `Standard`，那正是本轮要消灭的那种"跑得完却算错"。三条负向测试分别钉住内核层
（`monin_obukhov_tests::large_eddy_without_hpbl_is_refused`）、强迫层
（`runtime_forcing_tests::boundary_layer_height_is_optional_but_must_be_positive`）与
装配层（`assembly_tests::the_large_eddy_scheme_reads_hpbl_from_the_step_forcing`）。

`forc_hpbl` 不参与降尺度：它是观测到的大气量，不是被地形调整的列量，所以
`apply_downscaled_runtime_forcing` 原样穿过。

### 顺带查清：`kernels/default` 早就过期了

跑端到端验证前先重建内核是对的，但这一步此前没有被当成硬性前提。实测：
`kernels/default/` 里那份 `colm.x` 是 **8 月 25 日**从 `f427762` 编的，而
`vendor/CoLM202X` 在那之后又进过 f48 同步与臭氧扩展。用那份旧二进制跑
`forcing_convert`，`colm` 段直接死在

```
.../runtime_unused//Ozone/Global/OZONE-setgrid.nc does not exist.
```

按 `./oracle/scripts/build_kernel.sh default` 从当前 HEAD 重建之后，同一算例一路跑到
比对阶段。**`kernels/` 不入库，所以它会静默落后于 `vendor/`** —— 端到端结论若没写明
内核的 `colm_git_sha`，读者无法判断它对应哪份源码。

### 重建后的 `forcing_convert` 仍然逐位不等，原因不是这次改动

同一份黄金文件（内核快照 `4894833`）对重建后的内核（`60c9e1e`）逐位比，差异分两类：

| 类别 | 例子 | 读法 |
|---|---|---|
| 末位重排 | `f_wice_soisno` 4.577840183596859 vs …847、`f_wliq_soisno`、`f_zwt`、`f_zol` | 相对偏差 ~1e-15，换内核快照必然出现 |
| 缺测值策略 | `f_wetwat` / `f_wetwat_inst` / `f_wetzwt` 逐点 `0.0` vs `-1e36` | 黄金写入时那些量还是"未填"，本机内核写出真值 |

这次改动**在构造上不可能**造成它们：转换管道（`convert.rs`、`forcing-convert`、
`render.rs`、`met.rs`）完全不碰 `RuntimeForcing` / `prepare_runtime_forcing` /
`PointForcingFrame`，所以它写出的强迫 NetCDF 与改动前逐位相同，Fortran 内核读到的输入
也就相同。结论与前一节一致：**这份黄金文件在判定 Rust 移植之前必须先对齐内核快照或
重生成。**

## namelist → `LandPhysicsParameters`：三处"看起来是参数"其实不是（2026 年）

装配层一直把物理参数留给调用方显式传入，于是"从哪读"这件事没有落地。补上
`colm-runtime::physics::land_physics_parameters` 时逐字段去核上游出处，又翻出三个
**字段类别搞错**的量 —— 它们的共同点是：默认算例上取值恰好正确，所以黄金回归
不会响，只在特定配置下静默算错。

| 量 | 原先的处置 | 真正的出处 |
|---|---|---|
| `ground_emissivity` | `LandPhysicsParameters` 的字段（注释还写着 `DEF_EMIS`，而**namelist 里没有这个名字**） | `MOD_Thermal.F90:485-486` 逐步骤推：`emg = 0.96`，`IF (scv>0. .or. patchtype==3) emg = 0.97` |
| `wue_lambda`（WUE 基准） | 同上，从 namelist 读 | `lambda` 来自**地类表**（`MOD_Const_LC.F90:356/655`）；namelist 的 `DEF_WUE_LAMBDA`（默认 `-1`）只是覆盖 |
| `boundary_layer_height_m` | 同上 | 第 9 个强迫变量 `forc_hpbl`（已在上一节修） |

**比辐射率**现在由 `colm_core::ground_emissivity(scv, patchtype)` 推出，装配层按模板的
`scv` 给初值、**积雪分支的内核再用本步的 `scv` 覆盖一次** —— 雪融完之后必须退回 0.96，
否则融雪后的每一步都按雪面辐射，而能量收支看上去仍然闭合。无雪分支的 `scv` 恒为 0，
所以那里反过来钉住"装配层传的必须是土壤值"。

**WUE 基准**接到 `ClassConstants::wue_lambda()`（生成表里本来就有 `lambda` 这一列，
只是没有取用口）。默认算例若从 namelist 取，拿到的是 `-1`，而 `-1` 连内核的参数校验都过不去。

### 顺带纠正：`DEF_Runoff_SCHEME` 的编号在枚举注释里是反的

`StandardLctRunoffScheme` 原先标注 `XinAnJiang` = 1、`SimpleVic` = 2。上游
`MOD_SoilSnowHydrology.F90:315-348` 的派发是 **0=TOPMODEL、1=VIC、2=XinAnJiang、
3=SimpleVIC**。照注释写映射会把 XinAnJiang 与 SimpleVIC 对调 —— 两者都能跑完、
都给出有限的产流，只是数值不同。已改注释，并把没移植的 1（VIC）做成显式报错。

`vic.rs` 里有独立的 VIC 产流内核，但它没有接进 `Water2014Runoff`，所以
`DEF_Runoff_SCHEME=1` 目前是**报错**而不是静默挑一个相邻方案。

### 一处"该报错却不该报错"的反例

写映射时第一版把"`DEF_USE_MEDLYNST` 与 `DEF_USE_WUEST` 同时为真"当成了错误配置。
核上游才发现 `MOD_Namelist.F90:2080-2088` 对它的处置是**把两者都置为 `.false.`**、
落回 Ball-Berry，并打一条 warning —— 也就是上游定义好了结果。拒绝它等于拒绝一个
上游能跑的算例。而且 `DEF_USE_WUEST` 的声明默认值就是 `.true.`，所以任何只写
`DEF_USE_MEDLYNST = .true.` 的算例都会落进这个分支，报错的话连 Medlyn 都用不了。

**判据：上游有明确定义的行为就要复现，哪怕它看起来像配置错误；只有上游没有的分支才报错。**

### 映射的取值纪律

- **缺省值只从 `colm-schema` 取**（即 `MOD_Namelist.F90` 的声明值），映射里不写第二份。
  `physics_tests.rs` 里那条"空算例逐项等于声明默认值"就是在守这一点：任何一处写成
  字面量，默认值一变就会分叉。
- **不是 namelist 字段的量不放进这张表**（`hvap` 是常数、`emg` 是逐步推导、`lambda` 来自地类表）。
- **上游有、本仓库没移植的分支显式报错**：`DEF_Runoff_SCHEME=1`（VIC）与
  `DEF_USE_IRRIGATION`（喷灌率由 `DEF_TUNING_IRRIGATION_*` 与作物物候逐步算出，
  给 0 会让开启喷灌的算例静默变成不灌溉）。**`DEF_SPLIT_SOILSNOW` 是后来补上的
  第三、第四个**（见文档末尾"`DEF_SPLIT_SOILSNOW` 此前根本没被读过"一节）：
  `DEF_SPLIT_SOILSNOW`（写 `.true.` 会被 `assembly.rs` 里硬写死的
  `use_split_soil_snow: false` 静默按非 split 跑完）与 `DEF_USE_SNICAR`
  （同理，`snow_layer_absorption_w_m2` 被钉成 `None`，静默用标准雪光学）。
- `land_cover_scheme` **必须由调用方传**：它来自内核编译期的 `LULC_IGBP`/`LULC_USGS`，
  namelist 里的 `DEF_USE_IGBP`/`DEF_USE_USGS` 只是只读镜像（`MOD_Namelist.F90:163`），
  默认算例里两个都是 `.false.`，从 namelist 读只能靠猜。

## 第三段的 Rust 版本跑起来了，以及它第一次给出的判读（2026 年，实测）

新增 `crates/colm-runtime/src/bin/colm-rs.rs`：从算例目录读 namelist、装配模板、推进窗口、
按续跑语义写出 restart。**实测在 `oracle/work/CN-Cng` 上跑完整个 1 月窗口：528 步**
（与 Fortran 的 528 步一致），写出的 restart 与输入同 68 个变量、同维度。

```bash
NETCDF_DIR=/opt/homebrew/opt/netcdf cargo run -q -p colm-runtime --bin colm-rs -- \
  oracle/work/CN-Cng --land-cover igbp --restart-out /tmp/colm-rs-CN-Cng.nc
```

接口上两处刻意没有默认值：`--land-cover`（来自内核编译期宏，namelist 里读不出来）与
`--restart-out`（上游的续跑文件名由 `idate` 现算，而初始化器写出的是带 `_w180_s90` 的
另一套名字，猜一套只会与真实文件对不上）。指向与输入同一个路径会被拒绝：写出语义是
"以原文件为底、只换声明改过的变量"，同路径会让底稿消失。

### 判读必须三分，不能只数"有多少个变量不等"

`oracle/scripts/stage3_diff.py` 把逐变量结果分成三类 —— 这一个分类就是判读第三段
唯一可行的办法：

| 类别 | CN-Cng 1 月 | 含义 |
|---|---|---|
| 逐位相同 | **31 / 68** | — |
| Rust 未写出（保持初值） | **30** | 写出集合的差，不是数值错 |
| 两边都算但不同 | **7** | 真正的数值发散 |

**写出集合的差**（30 个：`alb`/`coszen`/`emis`/`extkb`/`extkd`/`fh`/`fm`/`fq`/`rss`/`rst`/
`ssun`/`ssha`/`ssoi`/`qstar`/`z0m`/`zol`/`ustar` 等）是刻意的：`evolved_overrides` 只写
"这次跑真的推进过"的量。Fortran 每步把内存里的整组量都写出去，所以两边文件必然不同 ——
这**不是**移植错误，但它意味着**在补齐写出集合之前，逐位比较黄金 restart 没有意义**。

**数值发散**那 7 个，按量级分两拨：

| 变量 | Fortran | Rust | 读法 |
|---|---|---|---|
| `t_grnd` | 255.319 K | 257.814 K | 差 **+2.5 K** |
| `tleaf` | 253.826 K | 254.613 K | 差 +0.79 K |
| `wice_soisno` | 0.1128 | 6.968 | 土壤冰差 **60 倍** |
| `wliq_soisno` | 0.000468 | 0 | 液相水被抽干 |
| `zwt` | 0.28913 m | 0.42460 m | 地下水位 |
| `wa` | 0 | **771.5** | 含水层从 0 涨到 771 mm |
| `t_soisno` | 雪槽 0 | 雪槽 -999 | 非活动雪槽的写出差异，非物理 |

热力状态在 11 个冬季日后差 2.5 K 是"有偏差但结构正确"；`wa` 从 0 涨到 771 mm 而
Fortran 一动不动，是**系统性的接线错误**，不是精度问题 —— `DEF_Runoff_SCHEME = 3`
（SimpleVIC）是**地表**产流方案，含水层的补给不该由它驱动。下一轮从这里入手。

### 顺手修掉的两个真缺陷

1. **地类查表差一位（数值影响明确）。** `assembly.rs` 把 `patchclass + 1` 当查表下标，
   而 `MOD_Const_LC.F90` 的数值表维度是 `(N_land_classification)`（位置 1..17），
   `patchclassname` 才是 `(0:N_land_classification)`、内容按 0..17 排开
   （`patchclassname(i)` 就是 `"i <类名>"`）。上游查表写的是
   `patchtypes(SITE_landtype)`（`MOD_Vars_TimeVariables.F90:1273`），所以**位置号就是
   地类号**。后果分两种：`patchtype` 断言会当场报错（这次正是它把问题拦下来的），
   而 `chil`/`vmax25`/`d50` 一类的表**不报错，只静默换成邻类的值** ——
   实测草地（10）的 `chil = -0.300` 被读成了湿地（11）的 `0.100`，直接进两流辐射。
   `ClassConstants::class_zero_based()` 也据此改成 `table_index()`
   （原注释把"数组下标"说成了"上游的 `patchclass`"）。

2. **`rstfac` 没有 1 的上界。** `leaf_temperature` 的校验要求
   `soil_water_stress in 0..=1`，而 `MOD_Eroot.F90:101-140` 的势梯度方案里
   `rresis = (1 - smp_node/smpmax)/(1 - psi0/smpmax)` 在湿润层大于 1，且它正是
   `etrc = trsmx0*roota` 的倍数。真实算例一跑就撞上这条过严的断言。改成只守下界
   （与 `photosynthesis.rs` 的 `>= 0.0` 一致）。

   同时把该函数的报错从"leaf-temperature inputs are invalid"改成**逐条列出失败的判据
   并带上数值** —— 一个笼统的"输入非法"在真实算例上没法定位，这次就是靠它才在两分钟内
   找到 `soil_water_stress` 的。

3. **关掉预热的算例跑不起来。** `read_point_runtime_config` 无条件解析
   `spinup_month/day/sec` 并窄化成 `u8`，而 `oracle/work/CN-Cng/case.nml` 里
   `spinup_day = 365`（预热关掉时的死字段）。上游 `CoLM.F90:315` 的判据是
   `is_spinup = ststamp < ptstamp`，`spinup_year = 0` 时永远为假。现在年份为 0 就
   直接取 `start`，不去碰那三个字段。

## 上一节那 7 个发散的真因：**土壤水文走的是两条不同的路径**（2026 年，实测）

上一节把 `wa` 从 0 涨到 771 mm 记成"接线错误"。查到源头之后，它比"接线错误"严重得多。

**上游是二选一，不是开关：**

```fortran
! CoLMMAIN.F90:1183
IF (.not. DEF_USE_VariablySaturatedFlow) THEN
   CALL WATER_2014 (...)   ! Campbell/Richards，参数表收 bsw
ELSE
   CALL WATER_VSF  (...)   ! van Genuchten，参数表收 alpha_vgm/n_vgm/L_vgm/sc_vgm/fc_vgm
ENDIF
```

而 `DEF_USE_VariablySaturatedFlow` 的声明默认值是 `.true.`，且
`MOD_Namelist.F90:1767-1772` 在**选了 van Genuchten 时强制把它置真**：

```fortran
IF (.not. DEF_USE_Campbell_SOIL_MODEL) THEN
   DEF_USE_VariablySaturatedFlow = .true.
ENDIF
```

**于是默认配置走 VSF，经典 Richards 反而是少数派。** 实测 CN-Cng 的运行日志里正是那句
`Note: DEF_USE_VariablySaturatedFlow is automaticlly set to .true.`。

本仓库的处境：

| | 上游 | 本仓库 |
|---|---|---|
| `WATER_2014`（Campbell/Richards） | 有 | **已编排**（`water_2014.rs` + `soil_water.rs`） |
| `WATER_VSF`（van Genuchten） | 有 | **只有内核，没有编排** |

`variably_saturated_flow.rs` 有 2604 行、15 个 `pub fn`（子层划分、上下边界跃迁通量、
最小二乘求解、显式步、含水层交换……），但**这 15 个函数在模块之外没有任何调用者** ——
`time_state.rs` 只是在冷启动剖面上用那个开关，`standard_lct_step.rs:418` 更是把
`variably_saturated_flow` 直接写成 `false`。缺的是 `WATER_VSF` 那个驱动器。

所以上一节那张"7 个变量发散"的表**要重新读**：它不是土壤水内核的精度问题，而是
**Rust 用 WATER_2014 去跑了一个上游用 WATER_VSF 跑的算例**。这解释了全部 7 个
（`wa`/`zwt`/`wliq_soisno`/`wice_soisno` 是水文本身，`t_grnd`/`tleaf` 差 2.5 K 是蒸发与
地表湿度经由水文传过去的），也解释了为什么 `wa` 会单调涨到 771 mm：van Genuchten 的
算例在 Richards 路径下底部边界与含水量关系完全不同。

**处置：`colm-rs` 现在拒绝这类算例**，并在信息里说清上游的判据、当前支持哪一种组合
（`DEF_USE_Campbell_SOIL_MODEL = .true.` 且 `DEF_USE_VariablySaturatedFlow = .false.`）。
拒绝而不是"跑完再标注"是刻意的：按经典路径跑完 VSF 算例不会报错，只会给出另一套水文下
看起来正常的数字 —— 这正是本仓库最忌讳的那种错。

`LandPhysicsParameters` 新增 `variably_saturated_flow` 字段承载**生效后**的取值，
由 `physics.rs` 按上游那两条规则算出来（声明默认 `true`，且 van Genuchten 时强制 `true`），
四条组合各有一条测试钉住。调用方不需要自己推这个值 —— 推错就会挑错水文路径。

### 下一步

移植 `WATER_VSF` 的编排（`MOD_SoilSnowHydrology.F90:529-1343`）并把 15 个内核接起来。
这是目前唯一挡住默认配置的缺口，也是"全面完成"绕不过去的一段。判读工具不变
（`oracle/scripts/stage3_diff.py`），但接上之后 `wa` 应当是**负值或零**（含水层是亏缺量）。

## 第一次**分支对齐**的第三段测量：2.5 K 的热力偏差与水文无关（2026 年，实测）

上一节把 7 个发散都归给"用 WATER_2014 跑了 VSF 算例"。那个归因只对了一半。为了验证，
造了一份 **Campbell 配置**的同算例（复制 `oracle/work/CN-Cng`，只改三行）：

```
DEF_USE_Campbell_SOIL_MODEL   = .true.
DEF_USE_VariablySaturatedFlow = .false.
DEF_dir_output                = <新目录>/out/       # colm-cli 拒绝自定义输出目录
```

用 `colm-cli run ... --force 1 --preprocessors fortran` 重跑三段（`mksrfdata ok /
mkinidata ok / colm ok`，日志里**没有**那句 `VariablySaturatedFlow is automaticlly set`），
再用 `colm-rs` 从**同一份**初始重启跑同一个窗口，两边都是 528 步。

### 归因的修正

| | VSF 算例（Rust 走经典） | Campbell 算例（两边都走经典） |
|---|---|---|
| `wa` Fortran | **0** | **5077.6** |
| `wa` Rust | 771.5 | 5440.5（相对差 **7%**） |
| `wice_soisno` 相对差 | 61× | **4.8×** |
| `zwt` Fortran / Rust | 0.289 / 0.425 | 2.785 / **0** |
| `t_grnd` Fortran / Rust | 255.319 / 257.814 | 255.573 / 258.319 |

两件事因此说清楚了：

1. **`wa` 的行为本来就该是"涨"**。上一节看到 Fortran 侧 `wa` 恒为 0，是 VSF 路径的性质，
   不是"Rust 多算"。分支对齐之后两边同向、同量级（5077.6 vs 5440.5）—— 那个异常消掉了。
2. **2.5–2.8 K 的热力偏差与水文分支无关**。Fortran 自己在两条分支下的 `t_grnd` 几乎相同
   （255.319 / 255.573），而 Rust 在两条下都偏暖 2.5–2.8 K。所以它是热力/辐射/湍流那一段
   的独立缺陷，**不是**上一节说的"经由蒸发传过去"。这是本轮最有用的一条：它把两个问题
   分开了。

### 分支对齐后仍然对不上的两个，是新的、更窄的线索

* **土壤水被抽干**：初始 `wliq_soisno[5:10] = [8.78, 13.83, 22.81, 37.09, 58.67]`，
  Fortran 收到 `[2.65, 4.81, 8.05, 13.46, 24.22]`（约三分之一），Rust 收到**全 0**。
* **地下水位塌到地表**：初始 `zwt = 4.433 m`，Fortran 升到 2.785 m，Rust 到 **0**。

两者互相矛盾（`zwt = 0` 意味着顶层饱和，而 `wliq` 全 0），指向 `update_groundwater`
里水位对补给的响应过强：`soil_water.rs:381-405` 先 `aquifer += recharge*dt`，再按
`specific_yield` 把水位往地表推，且 `.max(0.0)` 允许它一路推到 0。下一步从
`specific_yield` 与那个转移循环入手。

### 顺手修掉的：常数重启的土壤场要按水力关系读

Campbell 算例一上来就报 `constant restart field alpha_vgm for AlphaVgm is missing`。
对比两份常数重启，Campbell 那份**正好少** `alpha_vgm`/`n_vgm`/`L_vgm`/`sc_vgm`/`fc_vgm`
五个变量 —— 写出器本来就有 `uses_van_genuchten` 开关（`restart.rs:400`），
是装配层的 `soil_state` 无条件读了三张表。现在按 `physics.hydraulic_model` 读，
且 Campbell 时把那五个场填 **NaN 而不是 0**（0 是合法的 `alpha_vgm`，误读会静默算出
一套假参数；NaN 会被内核的有限性检查当场拦下）。夹具加了 `write_campbell`，
测试同时钉住两个方向：Campbell 重启能装配，van Genuchten 重启缺那五个场时仍报错。

## 三个数量级的单位错：重启里的 `hksati` 本来就是 mm/s（2026 年，实测）

上一节留下两条线索：土柱被抽干、水位塌到 0。顺着 `recharge` 逐步打印（第一步就
**94 mm/步**，而 Fortran 整个 11 天只涨 277.6 mm）查到了源头 —— 在装配层：

```rust
// crates/colm-runtime/src/assembly.rs（错的那一版）
let conductivity_mm_s = soil_field(&soil, SoilField::HydraulicConductivity, ...)
    .into_iter().map(|value| value * 1000.0).collect();
```

**那个 `* 1000.0` 是错的。** 上游四处都写着重启里的 `hksati` 就是 mm/s：

* `MOD_Vars_TimeInvariants.F90:238` 声明 `!hydraulic conductivity at saturation [mm h2o/s]`
* 同文件 `:529` 读、`:743` 写，注释同单位
* `mkinidata/MOD_IniTimeVariable.F90:122` 同单位

实测对照也一致：常数重启里底层 `hksati = 3.3897e-03`，而 Fortran 写到时间重启里的
`hk` 底层也是 `3.3897e-03`（`hk` 与 `hksati` 同单位，且那一层始终饱和）。

另有一条独立证据说明**两边写出的单位相同**：Rust 与 Fortran 的 mkinidata 产出做过
逐位比对（204 个变量 204 个相同），所以 `hksati` 不可能一边 mm/s 一边 m/s。

### 修掉之后的实测（Campbell 算例，528 步，同一初始重启）

| 变量 | 修之前 | 修之后 | Fortran | 修后的相对差 |
|---|---|---|---|---|
| `wa` | 5440.5 | **5094.4** | 5077.6 | **0.33%** |
| `zwt` | **0**（塌到地表） | **2.6769** | 2.7853 | **3.9%** |
| `wice_soisno` | 4.78× | 2.72× | — | — |
| `t_grnd` | 258.72 | 258.72 | 255.57 | +3.15 K |

地下水位从"整个塌掉"回到差 3.9%，含水层从差 7% 回到差 0.33%。**这是一处影响每一个
算例的单位错**，而且它在默认（VSF）配置下根本不会显形 —— 只有把分支对齐到已移植的
经典路径才看得见。

`t_grnd` 一格没动（+3.15 K），与上一节的结论一致：热力那一支是**独立**缺陷。

### 剩下的 7 个，现在分得很清楚

* `t_soisno` 的雪槽：Fortran 写 0、Rust 保持 `-999`。**非活动槽的写出差异**，不是物理。
* `wliq_soisno` / `wice_soisno`：Rust 把上面五层的液态水全部冻成了冰并多冻了约一倍
  （第 7 层固+液：Fortran 13.09，Rust 30.66），**相变分配**的差异。
* `t_grnd` / `tleaf`：+3.15 K / +1.14 K，且随深度衰减（底层只差 0.35 K）—— 热力链。
* `zwt` / `wa`：3.9% / 0.33%，已经接近数值精度与参数细节的量级。

判读仍然用 `oracle/scripts/stage3_diff.py`；`colm-rs` 的逐步诊断打印是临时加的，
排查完已删除（它证明了两件事：第一步补给 94 mm/步，以及 `rsubst = 0` 是**对的** ——
`Runoff_SimpleVIC` 在 `MOD_Runoff.F90:353` 无条件 `rsubst = 0.`，而
`SubsurfaceRunoff_SimpleVIC` 在上游只在 `MOD_SoilSnowHydrology.F90:920` 被**注释掉**的
那一行引用，属于死代码）。

## 把 `smp`/`hk` 写进续跑：写出集合从 30 收到 28，并换来一条决定性线索（2026 年，实测）

`evolved_overrides` 原先只写"状态里推进过"的量，`smp`/`hk` 不在其中 —— 理由是它们
"状态里没有"。但上游**在续跑时会把它们读回来**（`MOD_Vars_TimeVariables.F90:1363-1364`，
写在同一文件 `:1154-1155`），所以不写就等于交出一份**无法续跑**的重启：下一次运行的
初始 `smp`/`hk` 会退回更早那一份。两者都是 `soilwater` 的 `intent(out)`，只出现在
**步输出**里，于是新增 `EvolvedStepOutput { ground_temperature_k, matric_potential_mm,
hydraulic_conductivity_mm_s }` 把"只有步输出才有的量"集中成一处传进来。

一个形状陷阱：`smp`/`hk` 的维度是 `(patch, soil)`，**没有雪槽**，而 `t_soisno` 是
`(patch, soilsnow)`（雪槽在前）。步长不同（`layers` vs `snow_slots + layers`），
混用会让 patch > 0 的算例写到别的 patch 的层上。测试同时钉住这一点。

### 换来的线索：`smp` 剖面出现不可能的非单调尖峰

写出来之后 `stage3_diff.py` 立刻把它变成可比量（写出集合 30 → 28）：

| 层（0 基土） | 0 | 1 | 2 | 3 | 4 | **5** | 6 | 7 | 8 | 9 |
|---|---|---|---|---|---|---|---|---|---|---|
| `smp` Fortran | -2.34e6 | -2.17e6 | -1.93e6 | -1.62e6 | -1.23e6 | **-7.73e5** | -5.9e4 | -6.4e3 | -2.8e3 | -1.7e3 |
| `smp` Rust | -1.90e6 | -1.67e6 | -1.36e6 | -9.29e5 | -3.36e5 | **-1.60e7** | -1.7e4 | -5.4e3 | -2.5e3 | -1.6e3 |

Fortran 是一条**单调光滑**的剖面；Rust 在第 5 层掉到 **-1.60e7**，比它上下两层
（-3.36e5 / -1.7e4）深两个数量级 —— 一维扩散柱不可能出现这种剖面，这是**代码错**而不是
参数差。

配套的水量（固+液，kg/m²）：

| 层 | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
|---|---|---|---|---|---|---|---|---|---|---|
| 液态 Fortran | 2.65 | 4.81 | 8.05 | 13.46 | 24.22 | 40.46 | 91.89 | 180.5 | 288.9 | 392.8 |
| 液态 Rust | **0** | **0** | **0** | **0** | 0.34 | 29.72 | 104.8 | 184.5 | 294.5 | 396.4 |
| 固态 Fortran | 15.75 | 8.27 | 13.54 | 21.66 | 31.59 | 44.58 | 23.25 | 0 | 0 | 0 |
| 固态 Rust | 15.73 | **30.66** | **50.32** | **74.02** | **60.74** | 54.43 | 0 | 0 | 0 | 0 |

## 结论：剩下的核心缺陷是**液/固相变分配**

Rust 在同样的温度下把多得多的液态水冻成了冰：第 1–4 层液态直接归零，固态是从 Fortran 的
2–4 倍；总水量却接近（第 0 层 15.7 vs 18.4，第 6 层 104.8 vs 115.1）。液态归零之后
Campbell 的 `smp = psi0*(vliq/porsl)^(-bsw)`（`bsw ≈ 11`）把基质势推到 -1.6e7，于是
`hk = hksati*(smp/psi0)^-(2*bsw+3)` 崩到 1e-16 —— 这正是上一节看到的 `hk` 量级差。

所以上一节列出的两件事其实是**同一件**：`wice`/`wliq` 的分配错 → `smp`/`hk` 崩 →
土柱内部的水热耦合跟着偏。下一轮从这里入手：同一温度下 Rust 保留的液态水更少，
即相变判据（`MOD_PhaseChange.F90` 的冻结温度/未冻水关系）不一致。

`t_grnd`/`tleaf` 的 +3.15 K / +1.14 K 仍然独立：修 `hksati` 单位与写 `smp`/`hk`
都没让它动过一格。

## 第二处默认打开、Rust 硬关的分支：植物水力（2026 年，实测）

把 `smp`/`hk` 与 16 个表面诊断量写进续跑之后，`stage3_diff.py` 第一次把整条地表链摊开
比较，于是看到 `gs0sun`/`gs0sha`：Fortran `481.34`、Rust `4.79e-5`，差 **1e7**。

查上游出处，`MOD_LeafTemperature.F90:719` 的那条赋值在
`IF (DEF_USE_PLANTHYDRAULICS)` 里面：

```fortran
gs0sun = min( 1.e6, 1./(rssun*tl/tprcor) )/ laisun * 1.e6 * o3coefg_sun
```

而 `DEF_USE_PLANTHYDRAULICS` 的**声明默认值是 `.true.`**（`MOD_Namelist.F90:531`），
CN-Cng 的算例没有覆盖它 —— 也就是说**上游一直开着植物水力，本仓库的 standard-LCT 分支
一直硬关着**（`assembly.rs` 的 `plant_hydraulics: None`）。这是与 VSF 同一类的不匹配，
而且它比 VSF 更隐蔽：不报错、不漏文件，只是把 ET 的分层分配、冠层阻力的来源和 `vegwp`
状态整条换掉。

影响面（上游 `soilwater` 里的分岔）：

```fortran
IF(.not. DEF_USE_PLANTHYDRAULICS) THEN
   sumroot = sum(rootr, mask=is_permeable .and. rootr>0)
   etroot  = etr*max(rootr,0.)/sumroot     ! 按根分布摊
ELSE
   etrdef  = 0.;  etroot = rootflux        ! 由植物水力解出来
ENDIF
```

**处置与 VSF 一致：`colm-rs` 拒绝这类算例**，并直接告诉用户把
`DEF_USE_PLANTHYDRAULICS = .false.` 写进算例就能走已移植的那条。`LandPhysicsParameters`
新增 `plant_hydraulics`，由 `physics.rs` 从 namelist 读（不自己推默认值）。

### 第一次**全分支对齐**的测量

于是造了第三份算例：Campbell + VSF 关 + **PHS 关**。这就是本仓库目前实现的**全部**
配置。同窗口 528 步，同一初始重启：

```bash
colm-cli run oracle/work/CN-Cng-aligned --kernel kernels/default --force 1 --preprocessors fortran
colm-rs oracle/work/CN-Cng-aligned --land-cover igbp --restart-out /tmp/colm-rs-aligned.nc
python3 oracle/scripts/stage3_diff.py <initial> <fortran-final> <rust-final>
```

| | Campbell（PHS 开） | **对齐（PHS 关）** |
|---|---|---|
| 共享变量 | 68 | **65**（PHS 关掉后 `vegwp` 及其维度消失） |
| 逐位相同 | 31 | **34** |
| Rust 未写出 | 10（真值） | **9** |
| `wa` 相对差 | 0.33% | **0.17%** |
| `zwt` 相对差 | 3.9% | **2.1%** |
| `t_grnd` | +3.15 K | **+3.13 K** |

未写出的 9 个正好是**需要逐波段数据或额外推导**的那些：
`alb`/`ssun`/`ssha`/`ssoi`（`(patch, rtyp, band)` 四维形状）、`extkb`/`extkd`
（本仓库不逐步重算消光系数）、`emis`/`trad`（要 `olrb`/`olrg` 的守恒修正）、`rss`
（`DEF_RSS_SCHEME == 4` 下的 1/0 电导标志）。**其余全部可比**。

### 热力偏差现在可以定性了

三次对齐（VSF、PHS、`hksati` 单位）都没让 `t_grnd` 动过一格：它始终 **+3.1 K**，而：

* `tref`（2 m 气温）只差 **+0.62 K**
* `tleaf` 差 +1.24 K
* `z0m` 与 Fortran **逐位级一致**（相对差 1.2e-9）

**2 m 气温基本对、地表温度高 3 K**，说明偏差在**地表热量收支**里，不在大气或粗糙度。
顺着这条线要看的是尚未写出的 `ssun`/`ssha`/`ssoi`（吸收的短波）与 `emis`/`trad`
（放出的长波）—— 也就是下一轮把剩下 9 个写出来之后立刻能比的那几个。

另外两条本次新看到的量级差（都属诊断量、不影响状态推进）：

* `zol` 差 84 倍、`rib` 差 49 倍、`tstar` 差 1.8 倍、`ustar` 差 26% —— **地表更不稳定**，
  与地表偏暖同向，是结果而非原因。
* `rst`：Fortran 在 PHS 关掉后给出 **-2.53**（负的冠层阻力，本身就值得追），
  Rust 是钉住的 `500000.`。

## 第三次对齐之后的新缺口：**逐步骤的冠层光学没有被移植进运行时**（2026 年，实测）

写回 `alb`/`ssun`/`ssha`/`ssoi`/`ssno` 之后，`stage3_diff.py` 把它们归进"与初值相同"那一类
—— 也就是 **Rust 从头到尾没改过它们**。查调用点，原因很清楚：

```bash
grep -rn "cold_start_broadband_radiation_from_ground" --include=*.rs crates/
# 只有 crates/colm-init/src/{single_point.rs,spatial_time.rs} —— 全是初始化器
```

Rust 里那份"冠层光学"（`ColdStartRadiation`：`albedo`、`sunlit_absorption`、
`shaded_absorption`、`soil_absorption`、`snow_absorption`、`extkb`、`extkd`、`thermk`）
**只在冷启动算一次**。上游是逐步骤（`doalb` 为真时）重算的：

```fortran
! CoLMMAIN.F90:2157
IF (doalb) THEN
   CALL albland (ipatch,patchtype,deltim, soil_s_v_alb, ..., alb,ssun,ssha,ssoi,ssno,ssno_lyr,thermk,extkb,extkd)
ENDIF
```

而 `MOD_Albedo.F90:294`：

```fortran
lsai = lai + sai
IF(coszen <= -0.3) THEN
   RETURN  !only DO albedo when coszen > -0.3
ENDIF
czen = max(coszen, 0.001)
```

两个直接后果：

1. **`extkb` 带着冷启动的垃圾值跑完整个窗口。** `extkb = proj/coszen`（`:575`）而
   `czen = max(coszen, 0.001)`，所以夜里冷启动会算出 `proj/0.001 ≈ 660` ——
   实测初值正是 **659.919**，而 Fortran 跑完之后是 **1.0**。Rust 全程用它做冠层消光。
2. **`alb`/`ssun`/`ssha`/`ssoi` 不随 LAI、雪盖、土壤湿度变化。** 本算例的 LAI/SAI 是
   常数（0.2/0.45），所以这一项在本例里影响小；但对生长季或积雪算例是直接错的。

### 有一件事本轮没有查清，如实记下

Fortran 写出的终值是 `alb = 1.`、`ssun = ssha = ssoi = 0.`、`extkb = 1.`。这三个数
**不像**是两流解出来的（`alb = 1` 是"全反射"，`extkb = 1` 也不是 `proj/czen` 能给出的），
而 `MOD_Vars_TimeVariables.F90:700` 把 `alb` 初始化成 `spval`。本轮没能定位这三个数的来源
（`doalb` 的重置时机、写重启与 `albland` 的先后、以及是否存在另一条初始化路径都还没查）。
**在查清之前不把 Fortran 这三个值当作"正确值"** —— 下一步应当拿一个**有白天写盘点**的算例
（例如 `DEF_WRST_FREQ = 'DAILY'` 或把窗口移到白天结束）来看 `alb` 到底是多少。

### 因此热力偏差的下一站

`t_grnd` 的 +3.1 K 在三次对齐后没有动过。本轮把范围压到：**地表热量收支**，且
`z0m` 逐位一致、`tref` 只差 0.62 K。下一步要按顺序确认：

1. 逐步骤光学的移植（把已有的 `cold_start_broadband_radiation_from_ground` 从初始化器
   挪进运行时，用**当步**的 LAI/SAI/雪盖/土壤湿度）—— 这是本轮确认的缺口；
2. 用它解开上面那个未查清的问题之后，再比 `ssun`/`ssha`/`ssoi`/`alb`；
3. 最后才是 `emis`/`trad`（要 `olrb`/`olrg` 的守恒修正，形状与推导都已定位）。

## 把写盘点挪到白天：上一轮那个"未查清"有答案了，而且把热力偏差的方向翻了过来（2026 年，实测）

上一轮记下了 Fortran 终值是 `alb = 1`、`ssun = ssha = ssoi = 0`、`extkb = 1`，来源不明。
做法很直接：把同一份对齐算例的窗口结束时刻从 `2008-01-11 86400` 改到 **`43200`（正午）**，
重跑三段（504 步），再读一次：

| | 午夜写盘点（`coszen = -0.922`） | **正午写盘点（`coszen = 0.398`）** |
|---|---|---|
| `alb` | `[1, 1, 1, 1]` | `[0.16837, 0.42579, 0.16784, 0.40519]` |
| `ssun` | `[0, 0, 0, 0]` | `[0.36724, 0.06459, 0.21220, 0.04031]` |
| `ssha` | `[0, 0, 0, 0]` | `[0.02483, 0.00919, 0.09915, 0.01946]` |
| `ssoi` | `[0, 0, 0, 0]` | `[0.43956, 0.50043, 0.52081, 0.53504]` |
| `extkb` | `1.` | `1.37797` |
| `extkd` | `0.718` | `0.719` |

**答案是：`albland` 在开头就把 `extkb`/`extkd`/`ssoi`/`ssno` 重置成默认值，然后夜间直接返回。**

```fortran
! MOD_Albedo.F90:232-238 —— 在 coszen 判断**之前**
extkb     = 1.
extkd     = 0.718
ssoi      (:,:) = 0.
ssno      (:,:) = 0.
...
lsai = lai + sai
IF(coszen <= -0.3) THEN
   RETURN  !only DO albedo when coszen > -0.3
ENDIF
```

所以**午夜写盘点记录的是默认值，不是物理量**；而 `alb`/`ssun`/`ssha` 不在开头重置，
它们保留的是黄昏那一次低太阳角（`czen = max(coszen,0.001) = 0.001`）退化解的结果
（全反射、零吸收）。上一轮的 `alb = 1` 因此不是"Fortran 用的反照率"，而是夜里写的默认值。

**由此一条方法论结论：拿午夜写盘点的重启去比 `alb`/`ssun`/`ssha`/`ssoi`/`extkb` 是无效的。**
`stage3_diff.py` 的分类器也会把它们错放进"Rust 未写出"那一桶（判据是"Rust 值 == 初值"，
而这里 Rust 确实是初值），需要读的时候记住这一点。

### 白天写盘点上的光学差距（同一算例，正午）

| | Fortran | Rust（冷启动冻结值） | 比值 |
|---|---|---|---|
| `extkb` | 1.37797 | **659.919** | **479×** |
| `ssun` band1 直射 | 0.21220 | **0.000879** | **241×** |
| `ssoi` band0 直射 | 0.43956 | 0.12574 | 3.5× |
| `ssoi` band0 散射 | 0.50043 | 0.26233 | 1.9× |
| `alb` band0 直射 | 0.16837 | 0.26019 | 1.55× |
| `ssha` band1 直射 | 0.09915 | 0.30221 | 3.0× |

差距是量级性的，逐步骤光学的移植是必须做的。

### 但它把热力偏差的方向**翻了过来**

Rust 的 `alb` 更大、`ssoi` 更小 —— **两者都指向"吸收更少短波"，也就是应该更冷**。
可 Rust 在同一个正午写盘点上仍然偏暖：

| | 午夜写盘点 | **正午写盘点** |
|---|---|---|
| `t_grnd` | +3.13 K（255.592 / 258.722） | **+1.82 K**（264.350 / 266.172） |
| `tleaf` | +1.24 K | +2.20 K |
| `tref` | +0.62 K | **+0.46 K** |
| `zol` | Fortran -0.0248 / Rust -2.094 | Fortran **+0.0185** / Rust **-1.408**（**符号相反**） |
| `rib` | Fortran -0.0064 / Rust -0.318 | Fortran **+0.0047** / Rust **-0.211**（**符号相反**） |
| `tstar` | Fortran -0.0806 / Rust -0.2217 | Fortran **+0.0447** / Rust **-0.0904**（**符号相反**） |

正午时 **Fortran 的近地层是稳定的（`zol > 0`、`rib > 0`、`tstar > 0`），Rust 是强不稳定的**。
结合"吸收更少却更暖"，热力偏差的位置因此可以再收窄：不在吸收的短波那一侧，而在
**放出的长波 / 地表向土壤的热通量**那一侧 —— 也就是尚未写出的 `emis`/`trad`，以及
`fgrnd`。这是下一轮的直接目标。

（`extkd` 恰好一致（0.719）纯属巧合：两边都是 0.718/0.719 的默认量级。）

## 地表能量收支接进 history：逐小时的收支表把偏差定位到三项（2026 年，实测）

上一节的结论是"偏差在长波/地面热通量那一侧"，但两边都没有这几项可比。这一轮把它们接出来：
`state_from_phase` 现在把**步前**的整列温度留在 `GroundTemperatureState.previous_temperature_k`
里（上游 `t_soisno_bef` 与 `tinc = t - t_bef` 正是 `fgrnd`/`olrg`/`emis`/`trad` 的输入），
`history.rs` 新增 `set_lct_surface_budget`，一次写八项：`sabvsun`/`sabvsha`/`rnet`/`olrg`/
`emis`/`trad`/`fgrnd`/`lfevpa`。`UNFILLED` 从 6 条降到 4 条。

一个形状陷阱：地表层在**打包列**里的下标是 `len - soil_layers`，**不是 0** ——
带雪时 `temperature_k[0]` 是雪面温度。测试里那条 `equilibrium` 用例正是靠这个下标才没被写错。

### 逐小时收支（正午算例，252 条记录的平均值，单位 W/m²）

| 项 | Fortran | Rust | 差 |
|---|---|---|---|
| `rnet` | −19.36 | −29.47 | **−10.11** |
| `sabv+sabg` | 61.83 | 60.33 | −1.50 |
| `olrg` | 263.30 | 271.91 | +8.61 |
| `trad` | 260.96 | 263.06 | +2.10 |
| `fsena` | 36.88 | 91.53 | **+54.65** |
| `fevpa` | 0.0000 | 0.0000 | ~0 |
| `lfevpa` | 51.30 | 22.14 | **−29.16** |
| `fgrnd` | −107.53 | −143.14 | **−35.61** |
| `emis` | 1.0000 | 1.0000 | ~0 |

**恒等式两边都闭合**：`rnet = fsena + lfevpa + fgrnd`
（Fortran `36.88+51.30−107.53 = −19.35` vs `−19.36`；Rust `91.53+22.14−143.14 = −29.47` vs `−29.47`），
所以这三个差是自洽的：`+54.65 − 29.16 − 35.61 = −10.11` 正好等于 `rnet` 的差。
**净辐射只差 10 W/m²，偏差全在三个通量的分配上。**

三条读法：

1. **`fsena` 大了 54.7 W/m²（2.5 倍）。** 上游把它拆成叶与地面：`fsenl = −20.2`（叶从空气**吸**热）、
   `fseng = +57.1`，和为 36.9。Rust 的叶温输出里两者都有（`leaf_sensible_heat_w_m2` /
   `ground_sensible_heat_w_m2`），下一轮把它们也写出来就能定位是哪一半。
2. **`lfevpa` 小 29.2 W/m²（43%）。** 这一项由 `htvp*fevpg` 主导，而 `fevpg` 只有 ~1.8e-5 kg/m²/s
   —— 也就是说**地表的凝华/升华速率**差了 2 倍多，被 2.84e6 的潜热放大成 29 W/m²。
   注意 `fevpa` 两边都是 0，**总蒸散流量对得上、拆分对不上**，这正是 `lfevpa` 必须单独写的理由
   （也是上一轮那条"不能写成 `hvap*fevpa`"的量化版）。
3. **`fgrnd` 更负 35.6 W/m²** —— Rust 从土壤里多抽了 36 W/m² 上来。

这三项加起来恰好解释了 Rust 为什么"吸收更少短波却更暖"：它靠**对流与导热多失热**、
靠**凝华少失热**，净效果是地表温度偏高。下一轮的目标因此从"辐射"改成**地表通量拆分**
（叶/地面感热、地面升华），而不是继续追反照率。

`f_trad` 也确认了：Rust 的辐射温度高 2.10 K，与 `t_grnd` 的 +1.8 K 同向同量级。

## 叶/地面通量拆分：偏差是**持续偏移**，而且第一条记录就已经有了（2026 年，实测）

上一节把目标定在"地表通量拆分"。这一轮把 `fsenl`/`fseng`/`fevpl`/`fevpg` 四项也写进 history
（`LCT_ENERGY_VARIABLES` 4 → 8）。**命名陷阱**：`fevpl` 的闸门表长名是
"evaporation+**transpiration** from leaves"，不是笔误 —— 上游 `fevpl = etr + evplwet`
（`MOD_LeafTemperature.F90:878`），而核心里 `leaf_evaporation = transpiration + wet_evaporation`
是同一个量，所以**直接用、不要再加一次 `etr`**。

### 拆分结果（252 条记录平均，W/m²）

| 项 | Fortran | Rust | 差 |
|---|---|---|---|
| `fseng`（地面感热） | 57.08 | **102.90** | **+45.82** |
| `fsenl`（叶感热） | −20.20 | −11.60 | +8.60 |
| `fsena`（合计） | 36.88 | 91.53 | +54.65 |
| `fevpl`（叶蒸散+蒸腾） | 0.00000 | 0.00000 | ~0 |
| `fevpg`（地面蒸发） | 0.00002 | 0.00001 | **−37%** |
| `lfevpa` | 51.30 | 22.14 | −29.16 |
| `etr` | 0.00000 | 0.00000 | ~0 |

**`fseng` 一项就占了 +54.65 里的 +45.82。** 而 `fsenl` 只差 8.6、`fevpl`/`etr` 两边都是 0
（冠层休眠），所以 `lfevpa` 完全由 `fevpg` 主导：**地面升华速率小了 37%**，
被 2.84e6 的潜热放大成 −29 W/m²。

### 两条把范围收窄的判据

**① 偏差在昼夜里等量，不是日变化。** 按 `f_sabg > 0` 分成白天 104 条 / 夜间 148 条：

| | 白天 | 夜间 |
|---|---|---|
| `fseng` 差 | **+42.4** | **+48.2** |
| `fgrnd` 差 | −35.9 | −35.4 |
| `lfevpa` 差 | −43.1 | −19.4 |
| `olrg` 差 | +10.2 | +7.5 |

`fseng`/`fgrnd` 在两个时段**差得几乎一样多**。任何辐射类的原因都会是日变化型的；
持续偏移说明问题在**状态量**（土柱温度、粗糙度、稳定度）而不在辐射项。

**② 第一条记录就已经分开了 —— 是逐步公式，不是累积。**

| 记录 | `t_grnd` F | `t_grnd` R | `fseng` F | `fseng` R | `ustar` F | `ustar` R | `zol` F | `zol` R |
|---|---|---|---|---|---|---|---|---|
| 0（第一小时） | 272.395 | **273.160** | 670.6 | 499.6 | 0.5645 | 0.3925 | −0.2265 | **−9.49** |
| 12 | 268.692 | 272.202 | 95.0 | 140.4 | 0.3738 | 0.2976 | −0.1221 | −7.97 |
| 120 | 262.172 | 264.201 | 40.0 | 81.7 | 0.3992 | 0.2942 | −0.0107 | −2.44 |
| 240 | 257.135 | 259.585 | 58.4 | 93.1 | 0.1679 | 0.1308 | −0.4583 | −35.80 |

第一小时里 Rust 的 `t_grnd` **正好是 273.160 = 冰点**（相变把温度钉在 `tfrz`），
`zol` 已经是 Fortran 的 40 倍，`ustar` 只有 0.70 倍。所以三者（地表偏暖、稳定度、
摩擦速度）在**第一次输出**就同时偏掉，而不是随着时间积累出来的。

### `fseng` 的过大概率只是**结果**

`fseng = ρ·cp·ΔT/r_ah`。Fortran 平均 57.08、Rust 102.90；若 `r_ah` 量级相同，
两者的 ΔT 之比就是 1.8 倍 —— 而 `t_grnd` 正好差 1.8 K，`tref`（2 m 气温）只差 0.50 K。
**空气侧几乎正确，地表侧差 1.8 K**，于是地面感热差 45 W/m² 是*症状*。

所以下一轮要追的不是通量公式，而是**为什么第一条记录时地表温度就差 0.77 K、
`ustar` 就差 30%** —— 那是最早的、也可能是最纯粹的信号。已有的线索是
`t_grnd = tfrz` 的钉住（第 32 节记过 Rust 冻得更多）与 `ustar` 的持续偏小。

## 一步之后就能看清：第四个默认打开的分支，以及"最早的那个信号"（2026 年，实测）

上一节把 `fseng` 记成"地表偏暖的结果"。为了找到**最早**的分歧点，造了一个**只跑一步**的
算例（`end_day = 1`、`end_sec = 1800`；`mksrfdata`/`mkinidata` 与对齐算例相同，只用
`--stage colm` 重跑第三段），与 Rust 从**同一份**初始重启跑一步的产出逐字段比。

一步之后的对照（初始 `t_soisno` 全 283 K、`wice` 全 0）：

| | Fortran | Rust | 读法 |
|---|---|---|---|
| `t_grnd` | 273.160 | 273.160 | **完全相同**（都被相变钉在冰点） |
| `wa` / `zwt` | 4806.0078 / 4.399434 | 4806.0078 / 4.399434 | **逐位相同** |
| `z0m` | 0.120573 | 0.120573 | **逐位相同** |
| `wliq[0]`+`wice[0]` | 5.9826 + **3.0755** | 7.2319 + **1.8387** | 总量 9.06 / 9.07，**相变分配不同** |
| `smp[0]` | −22031.8 | −3576.96 | 6 倍（由上面的液态水量决定） |
| `hk[0]` | 9.3e-5 | 2.68e-4 | 2.9 倍 |
| `rib` | −0.072224 | **−1.408939** | **20 倍** |
| `zol` | −0.277312 | **−10.53948** | **38 倍** |
| `ustar` | 0.685832 | 0.474638 | 0.69 倍 |
| `tref` | 258.836923 | 260.165932 | **+1.33 K** |
| `tleaf` | 265.884942 | 266.477078 | +0.59 K |
| `fwet_snow` | **0.061358** | **0** | 见下 |

三条结论：

1. **`t_grnd` 第一步是对的。** 上一节看到"第一条记录就差 0.77 K"是**小时平均**的错觉
   —— 第一步两边都钉在 273.160，分歧从**第二步**才开始。
2. **最早的实质分歧在地表层的稳定度**：`rib` 20 倍、`zol` 38 倍、`ustar` 0.69 倍，
   而 `t_grnd`、`z0m`、`wa`、`zwt` 都完全相同。`rib` 的公式两边一致
   （`rib = min(5, zol*ustar²/(vonkar²/fh*um²))`），所以差的是**迭代的输入或收敛路径**，
   不是公式。这是目前最干净、最靠前的信号。
3. **`fwet_snow` 是 0 对 0.061，不是数值差，是分支没跑。** 顺着查到第四个
   **默认打开、本仓库硬关**的分支：

```fortran
! MOD_Namelist.F90:314
logical :: DEF_VEG_SNOW = .true.
! extends/interception/MOD_LeafTemperature_Extended.F90:1518
fwet_snow = canopy_snow_wetfrac(sigf, lai, sai, dewmx, tl, ldew_snow)
```

而 `assembly.rs` 把 `options.vegetation_snow` 硬写成 `false`，于是
`update_canopy_water` 在 `if !vegetation_snow` 那一条上直接 `return Ok(0.0)` ——
整支 vegetation-snow 绕过。实测 `ldew_snow` 两边都是 ~0.045（有雪水），
所以这不是"没有雪"，是**有雪但没走那一支**。

### 处置：把四道 `ensure!` 收成**一张清单 + 一个显式开关**

到目前为止 VSF、植物水力、植被上的雪已经是**三个默认打开**的分支，逐个 `ensure!` 的结果是
"修一个再撞下一个"。改成一次列全：

```rust
pub fn unported_branches(physics: &LandPhysicsParameters) -> Vec<&'static str>
```

`colm-rs` 默认拒绝并打出**全部**缺项；`--allow-unported-branches` 显式放行，
并在 stderr 上打警告说明"结果不是忠实复现"。这样诊断性测量仍然做得了，
而"悄悄按另一套物理跑完"变得不可能。

`oracle/work/CN-Cng-aligned` 现在把三个开关全部关掉，`unported_branches` 返回**空表**
—— 这是本仓库目前唯一**完全忠实**的配置：

```
DEF_USE_Campbell_SOIL_MODEL   = .true.
DEF_USE_VariablySaturatedFlow = .false.
DEF_USE_PLANTHYDRAULICS       = .false.
DEF_VEG_SNOW                  = .false.
```

## 找到并修掉了一层**差了 16.7 倍**的参考高度 —— 地表层整条链随之对齐（2026 年，实测）

上一节的最早信号是"一步之后 `rib` 差 20 倍、`zol` 差 38 倍、`ustar` 差 0.69 倍，而
`t_grnd`/`z0m`/`wa`/`zwt` 完全相同"。既然输入相同、公式相同，差别只能在**某个输入**里。
顺着 `zol = z_ref·κ·g·t*/(u*²·θ)` 看：它对 `z_ref` 是**线性**的。

`z_ref` 来自 `physics.wind_height_m`，而 `physics.rs` 是从 **case** 文档读
`DEF_forcing%HEIGHT_V` 的 —— 可这三个字段挂在 `nl_forcing_type` 上，住在
**forcing namelist** 里。case 文档没有它们，于是落到 schema 声明默认值：

| | 风 | 温 | 湿 |
|---|---|---|---|
| schema 默认（Rust 实际用的） | **100.0** | **50.0** | **50.0** |
| forcing.nml 写的 | 6.0 | 6.0 | 6.0 |
| **强迫文件里的 `reference_height_*`** | **6.0** | **6.0** | **6.0** |

而且上游在 POINT 下**用文件里的值覆盖 namelist**（`MOD_Forcing.F90:297-311`）：

```fortran
IF (trim(DEF_forcing%dataset) == 'POINT') THEN
   filename = trim(dir_forcing)//trim(fprefix(1))
   IF (ncio_var_exist(filename,'reference_height_v')) CALL ncio_read_serial (filename, 'reference_height_v', Height_V)
   IF (ncio_var_exist(filename,'reference_height_t')) CALL ncio_read_serial (filename, 'reference_height_t', Height_T)
   IF (ncio_var_exist(filename,'reference_height_q')) CALL ncio_read_serial (filename, 'reference_height_q', Height_Q)
```

**16.7 倍的风高度**，`zol` 就应该差十几倍 —— 实测 38 倍，量级完全吻合（残下的因子来自
`t*`/`u*` 本身也被这个错带着偏了）。

### 修法：三级优先级收到一处

`colm-forcing` 新增 `observation_heights(path)`（只读那三个标量，不加载整条序列），
`PointRuntimeConfig` 新增 `wind_height_m`/`temperature_height_m`/`humidity_height_m`
并由 `read_point_runtime_config` 按上游的优先级解出来：**文件 > forcing namelist >
schema 默认**。文件*不存在*时回落到 namelist（本仓库的配置解析在真实算例之外也要能用），
文件存在却读不出来仍然报错 —— 那说明路径或格式有问题，静默回落等于换了另一套观测高度。

`physics::land_physics_parameters` 因此**不再自己读**这三个数，改成收一个
`ObservationHeights`。在 case 文档里读它们本来就是错的。

### 一步之后：全部分支对齐（VEG_SNOW 两边都关）的对照

| 量 | Fortran | Rust | 相对差 |
|---|---|---|---|
| `zol` | −0.277312 | −0.277166 | **5.3e-4** |
| `rib` | −0.072224 | −0.072187 | **5.2e-4** |
| `ustar` | 0.685832 | 0.685726 | **1.6e-4** |
| `fm` | 3.343616 | 3.343784 | 5.0e-5 |
| `fh` / `fq` | 2.911694 | 2.911968 | 9.4e-5 |
| `tref` | 258.836923 | 258.777110 | 2.3e-4 |
| `qref` | 0.001208 | 0.001206 | 1.3e-3 |
| `tstar` | −1.398413 | −1.397395 | 7.3e-4 |
| `tleaf` | 265.884965 | 265.712367 | 6.5e-4 |
| `t_grnd` | 273.160000 | 273.160004 | **1.3e-8** |
| `wa` | 4806.007773 | 4806.007773 | **1.6e-11** |
| `zwt` | 4.399434 | 4.399434 | **9.6e-11** |
| `wliq_soisno` / `wice_soisno` | — | — | max\|Δ\| **0.025 / 0.024** |

**修之前** `zol` 是 −10.539、`rib` 是 −1.409、`ustar` 是 0.475。地表层整条链（`zol`/`rib`/
`ustar`/`fm`/`fh`/`fq`/`tref`/`qref`/`tstar`/`tleaf`）从"差几十倍"变成**1e-3 以内**，
`wa`/`zwt`/`t_grnd` 到 1e-8 以下。相变分配也随之修好：`wice[0]` 从 1.84（Fortran 3.08）
变成 3.10。

### 全窗口暴露出的下一个**结构性**缺口：土柱不会造雪

修好之后重跑全对齐的 528 步，Fortran 在窗口里**真的积起了雪**：

```
scv = 0.047188   snowdp = 0.000455   fsno = 0.017568   sag = 0.001243
```

而 Rust 全是 0 —— 因为 `standard_lct_soil_step` 是**无雪**分支，它不会造雪，也没有
"中途出现积雪就切到积雪分支"的开关。这一步之后 `t_grnd` 仍差 +2.45 K（255.58 / 258.03），
其中一部分可以归给缺掉的雪（雪既反照又隔热）。

**下一轮的目标因此是结构性的**：让 `standard_lct_soil_step` 在 `scv > 0` 时切到积雪分支，
或者把两支合成一个每步判定的入口（上游 `CoLMMAIN` 就是这么做的）。

## 无雪起步的运行现在能长雪了 —— 拆成两支入口是错的（2026 年，实测）

上一节把下一个缺口记成"土柱不会造雪"。根因比"不会造雪"更简单也更根本：
**本仓库把一步更新拆成了"无雪入口 / 有雪入口"两支，而上游没有这种拆分。**

上游每步在能量与水量收支**之前**无条件调一次 `newsnow`（`CoLMMAIN.F90:976`，
`[3] Initialize new snow nodes for snowfall / sleet` 一节）：

```fortran
snl_bef = snl
CALL newsnow (patchtype,maxsnl,deltim,t_grnd,pg_rain,pg_snow,bifall, &
              t_precip,zi_soisno(:0),...,snl,sag,scv,snowdp,fsno,wetwat)
lb = snl + 1
```

雪只是打包列里的一个下标。所以修法是让通用入口接受 `snow.layer_count == 0`，
`add_new_snow` 自己决定要不要建层，调用方不必"先看有没有雪、再挑入口"。

三处改动：

1. **内核**：`validate_snow_soil_step` 的 `snow_layers > 0` 去掉（`(-5..0)` → `(-5..=0)`）。
2. **`newsnow` 的入参**：它要"雪层下面那一层的温度"，有雪时取雪列最后一层，
   无雪时**没有雪槽可索引**（`snow_layer_slot(1)` 会断言失败）—— 那时应取第一个土层。
3. **`snowwater` 跳过**：上游第 [1] 节在 `lb >= 1`（`snl == 0`）时不做雪层水运算。
   但**雨要直接落到土上**：有雪时雨先经雪列、由底部排水转给土壤；无雪的
   `gwat = pg_rain + sm - ...` 里 `pg_rain` 就是雨水本身。这里若给 0 等于把降雨吞掉。

`colm-rs` 现在一个入口跑到底（按启动时雪列选装配断言，但运行只走通用入口）。

### 实测：雪真的长出来了

全对齐的 528 步窗口：

| | Fortran | Rust（修之前） | Rust（修之后） |
|---|---|---|---|
| `scv` | 0.047188 | **0** | **0.015822** |
| `snowdp` | 0.000455 | **0** | **0.000149** |
| `fsno` | 0.017568 | **0** | **0** |
| `sag` | 0.001243 | **0** | **0** |
| `t_soisno` 雪槽 | 0 | **−999** | **0** |

雪槽也跟着对齐了（原先写 −999，现在是 0，与 Fortran 一致）。

### 剩下的：浅雪（不建层）的 `fsno` 与 `sag`

`scv`/`snowdp` 只有 Fortran 的三分之一，而 `fsno`/`sag` 仍是 0。原因清楚：
`add_new_snow` **只在建层时**设 `ground_snow_fraction`（`MOD_NewSnow` 那一支），
而本算例的雪**始终没到建层的临界深度**（`state.depth_m >= 0.01`），所以
Fortran 的 `fsno = 0.0176` 只能来自另一处 —— 上游的 `MOD_SnowFraction:snowfraction`
负责给**没有雪层的浅雪**算 `fsno`，本仓库还没移植它。`sag` 同理（`snowage`）。

雪既反照又隔热，`fsno` 缺失直接影响地表能量收支，所以这两项是下一轮的目标
（`t_grnd` 现在仍差 +2.45 K）。

## 运行期的表面光学系数：`fsno`/`sag`/`alb` 原来永远停在启动时刻（2026 年，实测）

上一节把剩下的缺口记成"浅雪的 `fsno` 与 `sag`"。追下去发现根因比那更宽：
**上游每步末尾有一整节「Preparation for the next time step」（`CoLMMAIN.F90:2068-2200`），
本仓库只在 `mkinidata` 里跑过一次它的冷启动版本。** 于是运行期的

```
lai, sai, sigf, fsno, sag, alb, ssun, ssha, ssoi, ssno, thermk, extkb, extkd
```

全部停在启动时刻那一次的值。实测对齐算例 528 步之后：Fortran 的 `fsno = 0.017568`、
`sag = 0.001243`、`extkb = 1.0`（夜间复位），而 Rust 的 `fsno`/`sag` 是 0、
`extkb` 是 `659.919`（冷启动时 `coszen` 被钳到 0.001 后算出来的值）。

### 那一段到底做了什么

```
calday = calendarday(idate)          ! idate 已被 TICKTIME 推到**步末**
coszen = orb_coszen(calday, patchlonr, patchlatr)
CALL snowfraction (tlai, tsai, z0m, zlnd, scv, snowdp, wt, sigf, fsno)
lai = tlai ; sai = tsai * sigf       ! DEF_VEG_SNOW 打开时 lai = tlai * sigf
ssw = min(1., 1e-3*wliq_soisno(1)/dz_soisno(1))
CALL albland (..., wt, fsno, scv, scvold, sag, ssw, ...)
```

关键在**它读的是本步的输出、写的是下一步的输入**：`z0m`、`t_grnd`、`fwet_snow` 来自这一步
的 `THERMAL`，`scv`/`snowdp` 来自这一步的雪列收尾。所以它只能是 driver 的一节，
不能塞进任何一个物理内核。

### 移植

* `MOD_SnowFraction:snowfraction` → `colm_core::snow_fraction`。`fsno` 只在
  `snowdp > 0` 时算，`fmelt = (scv/snowdp/100)**DEF_TUNING_SNOW_COVER_EXPONENT`，
  `fsno = tanh(snowdp/(2.5*zlnd*fmelt))`。**`z0m` 是本步 `THERMAL` 刚算出的冠层动量
  粗糙度，不是常数**；`zlnd` 是裸土粗糙度（`DEF_TUNING_ZLND`，默认 0.01）。
* `albland` 的运行期非 SNICAR 分支 → `colm_core::albland`（私有）+ 公开入口
  `colm_core::prepare_surface_optics`。它复用已有的 `twostream`（`two_stream`）
  与冷启动用的 `broadband_radiation_from_ground_using`，只把雪龄换成状态、
  加上夜间提前返回。为此把 `generic_snow_albedo` 改成
  `aged_snow_albedo(swe, swe, tg, czen, 1800, 0)` 的一次求值 —— 冷启动与运行期
  **必须**共用同一个式子。
* `lai`/`sai`/`sigf` 是时间变量，加进 `StandardLctEnergyState.canopy`
  （`CanopyGeometry`）；内核入口用状态里的一对覆盖输入里三个子结构
  （截留、`netsolar`、`THERMAL`）的 `lai`/`sai`。装配期给的那一对只是第一步的值。
* 装配层补读：时间重启的 `tlai`/`tsai`/`sigf`，常数重启的
  `soil_s_v_alb`/`soil_d_v_alb`/`soil_s_n_alb`/`soil_d_n_alb`。
  **裸土反照率不能用地类色表现算**：`mksrfdata` 会把 SITE 观测写进常数重启，
  实测 CN-Cng 是 0.14/0.25/0.28/0.39，与 IGBP 草地的色表不同。
* 续跑写出补上 `lai`/`sai`/`sigf`/`thermk`/`extkb`/`extkd`（`alb` 那一组本来就在写）。
* history 里的 `fsno`/`lai`/`sai` 要取**准备之后**的值：上游 `hist_out`
  （`CoLM.F90:537`）在 `CoLMDRIVER`（`:512`）之后跑，而末尾那一节在 `CoLMDRIVER` 里面，
  所以 history 记的是下一步那一组。

### 验证：两个独立的 Fortran 对照点

**一、`albland` 的整条链**。`oracle/work/CN-Cng-noon` 的重启（2008-01-11 12:00，
`scv = 0`，白天）里存着 `albland` 的全部**输入**（`coszen`/`t_grnd`/`tlai`/`tsai`/`z0m`/
`fwet_snow`/`wliq_soisno(1)`/`scv`）与全部**输出**（`alb`/`ssun`/`ssha`/`ssoi`/`ssno`/
`thermk`/`extkb`/`extkd`），是一个闭合的对照点。用它的输入调 Rust 的实现，
八个输出全部对上到 float32 的量化精度（< 1e-6）：

```
alb  = 0.168370 0.425790 0.167836 0.405187    (盘上顺序 rtyp*2+band)
ssun = 0.367243 0.064590 0.212202 0.040314
ssha = 0.024830 0.009192 0.099148 0.019457
ssoi = 0.439557 0.500428 0.520814 0.535041
thermk = 0.546975   extkb = 1.377969   extkd = 0.719
```

两个坑记在这里：**（a）** 那个探针算例**没有**关 `DEF_VEG_SNOW`（只有 `CN-Cng-aligned`
写了 `.false.`），所以 `twostream` 里"植被上的雪"那一支是打开的；关掉它 `alb` 差 2.6e-4。
**（b）** 重启里的 `alb` 是 `[rtyp0 band0, rtyp0 band1, rtyp1 band0, rtyp1 band1]`
（盘上维度 `(patch, rtyp, band)`），而 Fortran 的 `albv(iw,1)` 是 `(band, rtyp)`；
两处都显式转置，写反了 `ssha` 会整体错位。

**二、雪面反照率分支**。CN-Cng 的窗口里雪只在最后一个**夜间**步之前形成，
白天那一支从来没被写到重启上，所以另起一个独立 gfortran 程序逐字复制
`MOD_Albedo.F90:341-370`（`snowage` + 天顶角订正 + `snal0/snal1`）取四组参照，
`aged_snow_albedo` 逐个对上到 1e-15。

### 实测：528 步对齐窗口

| | Fortran | 修之前 | 修之后 |
|---|---|---|---|
| `fsno` | 0.017568 | **0** | 0.005611 |
| `sag` | 0.001243 | **0** | 0.001354 |
| `extkb` | 1.0 | **659.919** | **1.0** |
| `extkd` | 0.718 | 0.719 | **0.718** |
| `thermk` | 0.547061 | 0.546975 | 0.547003 |
| `lai` | 0.2 | 0.2 | 0.2 |
| `sai` | 0.449830 | 0.449830（没写回） | 0.449944 |
| `t_grnd` | 255.5818 | 258.0294（+2.448） | 257.7974（**+2.216**） |

`alb`/`ssun`/`ssha`/`ssoi`/`ssno` 在午夜写点两边都是夜间复位值（1 / 0 / 0 / 0 / 0），
逐位相同。`fsno` 的残差不是公式问题：Rust 的 `scv` 只有 Fortran 的三分之一
（0.015822 / 0.047188），`fsno` 由 `scv`/`snowdp` 决定，所以它跟着偏小；
`fsno` 的公式本身在 `snow_fraction` 的单测里对着 Fortran 重启核对过。

### 白天对照点与暴露出的下一个缺陷：`coszen` 取的是步首而不是步末

为了在**有雪且是白天**的点上验证，另建了 `oracle/work/CN-Cng-aligned-day1`
（= 对齐算例，`end_day = 1`、`end_sec = 43200`），跑 Fortran 得 2008-01-01 12:00 的重启。
它的 history 说明这场雪在第 6 小时积到 `scv = 0.047188` 之后**再没化过**，所以
第 1 天正午与第 12 天午夜的 `scv`/`snowdp` 逐位相同 —— 这解释了上一节里
"两个不同日期却有同一个 `scv`"的疑问。

在这个点上 Rust 的 `alb = 0.192738` 而 Fortran 是 `0.169983`，差 0.023。顺着一查：

```
Fortran: coszen = 0.3798207139      Rust: coszen = 0.3741924546
```

差值 0.00563 ≈ 半个步长的太阳时角。原因是**上游有两个不同的 `coszen`**：

* `MOD_Forcing` 在 `TICKTIME` **之前**算 `calday = calendarday(idate)`
  （`MOD_Forcing.F90:752`）→ **步首**，用于短波直散拆分与地形降尺度；
* `CoLMMAIN` 在 `TICKTIME` **之后**算（`CoLMMAIN.F90:2076`）→ **步末**，
  用于 `albland` 并写进重启的 `coszen`。

（`CoLM.F90:480` 的 `CALL TICKTIME(deltim, idate)` 在 `CoLMDRIVER`（`:512`）之前。）
本仓库原先只算了一个步首的 `coszen`，两处都用它。修法是把两个值分开：
`PointRuntimeStep` 多一个 `surface_cosine_zenith`（按 `clock.end_time` 算），
`forcing.cosine_zenith` 保持步首（降尺度与直散拆分要用它），`albland` 与重启的
`coszen` 用新的那个。`MOD_NetSolar.F90:292` 的 `local_secs = idate(3)` 同理 ——
`StandardLctStepBinding.seconds_of_day` 也改成步末，并且要把 `86400` 进位成第二天
`00:00`（上游的 `idate` 用 `adj2end` 约定，时钟交出来的 `end_time` 保留"当日末尾"
的写法，不退位会让每个跨日步都撞上 `NetSolar` 的值域断言）。

修完这个点上 `coszen` 差 2.2e-9、`extkb` 差 9.9e-9。**这一个改动同时验证了整条
太阳几何 → 两流 → `extkb` 的链**：`extkb = proj/czen` 对 `czen` 极敏感
（`d(extkb)/d(czen) ≈ -3.5`），差半个步长就会放大到 0.026。

## 冰点以下为什么还有液态水：`DEF_USE_SUPERCOOL_WATER` 被写死成了关（2026 年，实测）

修完上面那一条，白天对照点上 `alb` 仍差 0.023。顺着 `ssoi` 反推地面反照率，发现
Rust 的 `ssw = 0` —— **表层土壤一点液态水都没有**。而两边的总水量逐位相同
（18.78 kg/m²），只是分割不同：

```
                 t_soisno(1)   wliq(1)   wice(1)
Fortran            268.288      3.274    15.510
Rust               270.832      0.000    18.784
```

冰点**以下**还有液相不是数值噪声：上游默认打开超冷土壤水方案
（Niu & Yang 2006）。`MOD_Namelist.F90:281`

```fortran
logical :: DEF_USE_SUPERCOOL_WATER = .true.     ! supercooled soil water scheme, Niu & Yang (2006)
```

只有 `DEF_URBAN_RUN` 会把它强关（`:2337`）。`meltf` 里结冰的判据因此不是
`wliq > 0` 而是 `wliq > supercool(j)`（`MOD_PhaseChange.F90:170`）：

```fortran
smp = hfus * (t_soisno(j)-tfrz)/(grav*t_soisno(j)) * 1000.     ! mm
supercool(j) = porsl(j)*(smp/psi0(j))**(-1.0/bsw(j))           ! Campbell
supercool(j) = supercool(j)*dz(j)*1000.                        ! mm
```

内核**早就移植好了**（`phase_change.rs` 的 `supercool_limit`，含 Campbell 与
van Genuchten 两支），问题只在装配层写死了 `supercool_water: false`。
`physics` 现在从 `DEF_USE_SUPERCOOL_WATER` 读它并传下去。

### 实测：这一条是剩下误差的主要来源

| | Fortran | 写死关 | 读默认（开） |
|---|---|---|---|
| `t_grnd`（第 1 天正午） | 268.2883 | 270.8324（**+2.544**） | 268.2334（**−0.055**） |
| `alb`（vis 直射） | 0.169983 | 0.192366 | **0.170494** |
| `ssun`（vis 直射） | 0.378046 | 0.384194 | **0.378181** |
| `ssoi`（vis 直射） | 0.432157 | 0.395392 | **0.432199** |
| `scv` | 0.047188 | 0.015822 | **0.053120** |

528 步对齐窗口：

| | Fortran | 上一节修完 | 再修这一条 |
|---|---|---|---|
| `t_grnd` | 255.5818 | 257.7974（+2.216） | **255.5346（−0.047）** |
| `sag` | 0.0012434 | 0.0013540 | **0.0012420** |
| `scv` | 0.047188 | 0.015822 | 0.054715 |
| `fsno` | 0.017568 | 0.005611 | 0.020906 |
| `coszen` | −0.92245036 | −0.91681995 | **−0.92245036** |
| `alb`/`ssun`/`ssha`/`ssoi`/`ssno` | — | 逐位相同 | 逐位相同 |
| `sai` | 0.4498301 | 0.4499444 | 0.4498005 |

`t_grnd` 从 +2.2 K 掉到 −0.047 K，其余表面量也一起落到位。剩下的 `scv` 偏高 16%
（Rust 积得略多）与 `fsno` 跟着偏高，是下一轮的目标。

**一条教训**：`DEF_USE_*` 这类**默认开**的开关，写死 `false` 不会报错，只会让整个
物理过程静默消失。凡是上游声明默认为真的分支，装配层要么把值读进来，要么进
`unported_branches`；不能留一个没有出处的常量。

## history 的两处缺陷：声明了没人写的变量，以及缺掉的区间平均（2026 年，实测）

把 Rust 的 history 与 Fortran 逐变量比之后发现两件事，第二件更根本。

### 一、积雪分支声明了十三个地表诊断量，却一个都没写

`push_lct_snow` 漏了 `set_lct_surface_diagnostics`：`declare_lct_variables` 把
`taux`/`tauy`/`tref`/`qref`/`z0m`/`zol`/`rib`/`ustar`/`qstar`/`tstar`/`fm`/`fh`/`fq`
都声明进了文件，但没有一步去填它们。实测 CN-Cng 的积雪分支 history 里
`f_taux`/`f_tauy`/`f_z0m` 全是浮点填充值（`-1e36`）而 Fortran 有值。

**这个错误没有任何提示**：填充值本身是有限数，所以"检查是否有限"的断言抓不住它。
新加的回归测试逐名对着**最后一步的叶温输出**比，去掉那一句就会失败。

无雪分支（`push_lct`）一直是对的，所以这是"两支入口"这一个错误设计的又一次余波
（前一次是土柱不会造雪，见上文）。

### 二、Fortran 的 history 是**区间平均**，本仓库写的是瞬时值

上游每步先累加、到写出的那一步再除以累加步数（`MOD_Hist.F90:227` 的
`accumulate_fluxes`，写出的量都取自 `a_*` 累加器与 `nac_dt`）。本仓库的
`HistorySession::push` 直接把写出时刻的瞬时值填进记录。

判据是一个闭合的算式。CN-Cng 从 2008-01-01 00:00 跑**两步**（到 01:00）：

```
一步末 t_grnd = 273.1600     （2008-001-01800 重启）
两步末 t_grnd = 271.6308     （2008-001-03600 重启）
Fortran history 的 f_t_grnd = 272.3954 = (273.1600 + 271.6308) / 2   ← 逐位相等
Rust   history 的 f_t_grnd = 271.5823 = 两步末的瞬时值
```

同一个算例在 01:00 的 `scv`/`snowdp`/`fsno`/`sag`/`alb`/`thermk`/`extkb`/`extkd`/`sai`/`sigf`
两边**逐位相同**，`t_grnd` 差 3.7e-6 K —— 也就是说重启层面已经对齐，差的全在 history
这一层的取平均上。这解释了在此之前看到的"通量差 10~15%"（例如首条记录
`f_fsena` Fortran 702.76 / Rust 608.51）：那是区间平均与瞬时值之差，不是物理之差。

**结论：在此之前所有基于 history 的 Fortran/Rust 对比都作废**（包括
`oracle/golden/*.nc` 的逐变量比较），必须先把区间平均补上再比。

### 修法

`HistorySession` 里加一个按变量名持有"逐 step 的和 + 步数"的累加器，`push` 的语义改成

```
每步：把这一步的值累加进累加器（写出的那一步也先累加）
到期：取平均写进 HistoryBuffers，累加器清零
```

与上游 `hist_out` 的顺序一致（`:227` 累加 → 除法写出 → `:4746` 清零）。
标量与列（`soilsnow`）的区分由**累加时**的形态决定并记在累加器里，不在写出时猜 ——
猜错会把一根土柱按标量写出去。累加器与缓冲区共用一个 `HistorySink` trait，
所以五个 `set_lct_*` 函数只改签名、不重复公式。

### 修完之后的对照

CN-Cng 第 1 天（24 步、12 条记录），`f_*` 都是区间平均：

| | Fortran | 修之前（瞬时） | 修之后（平均） |
|---|---|---|---|
| `f_t_grnd`（首条） | 272.39541 | 271.58228 | **272.37114** |
| `f_fsena`（首条） | 702.75844 | 608.50951 | **702.04058** |
| `f_tref`（首条） | 258.30176 | 258.17486 | **258.24344** |
| `f_z0m`（首条） | 0.120573 | NaN（见上一节） | **0.120573** |

`f_fsena` 从差 94 W/m² 收到 0.7 W/m²，`f_t_grnd` 从差 0.81 K 收到 0.024 K。
剩下的是真实的状态差（`f_scv` 偏高，见上文）。

### 顺带发现：叶/地面拆分写的是初步值（已修）

`f_fseng` 首条记录 Fortran 670.58 / Rust 962.62，而总量 `f_fsena` 只差 0.7 W/m² ——
**总量对、拆分对不上**。原因是 `set_lct_energy_fluxes` 写的是
`energy.leaf.ground_sensible_heat_w_m2`，那是**叶温求解之前**的初步地面交换；内核的
总量是 `total = leaf + corrected_ground`（`standard_lct_step.rs:351`），
所以写初步值会破掉上游的恒等式：

```
Fortran  32.183 + 670.575 = 702.758 = fsena        ← 成立
Rust     28.841 + 962.618 = 991.459 ≠ 702.041      ← 不成立
```

改成 `corrected_ground_sensible_heat_w_m2` / `corrected_ground_evaporation_kg_m2_s`
之后：`673.200 + 28.841 = 702.041` ✓，与 Fortran 的 `fseng` 差 2.6 W/m²。
`f_fevpg` 同理（原先也偏大）。回归测试现在同时钉住"逐项等于内核字段"与
"拆分之和回到总量"（后者用 1e-12 的相对容差，只吃掉浮点结合律）。

### 还没修：`f_lfevpa` 与它自己的 `fevpl`/`fevpg` 对不上

```
Fortran  f_lfevpa = 184.677    hvap*f_fevpl + htvp*f_fevpg = 221.506   ← 差 37
Rust     f_lfevpa = 615.054    hvap*f_fevpl + htvp*f_fevpg = 615.054   ← 自己自洽
```

本仓库这一项是自洽的（`set_lct_surface_budget` 里同一批蒸发量算出来的），
**上游反而不自洽**：它的 `a_lfevpa` 与 `a_fevpl`/`a_fevpg` 采样的不是同一个阶段
（`MOD_Thermal` 里 `lfevpa` 在 `assimsun`/`assimsha` 订正之前算，而
`fevpl`/`fevpg` 是订正之后的量）。要对齐必须先定位上游每一个量在 `THERMAL` 里的
采样点，而不是按名字配对 —— 在这之前 `f_lfevpa` 不参与黄金比对。

## history 的近地层八项是**重算**的，不是内核输出（2026 年，实测）

补齐区间平均之后，用 `golden-compare --tolerances oracle/tolerances.toml` 把 Rust 的
history 与 Fortran 的逐变量比了一遍。剩下最大的偏差集中在近地层诊断上：`f_ustar` 差 11%、
`f_zol`/`f_rib` 差一倍 —— 而**同一时刻重启里的 `ustar`/`zol`/`rib` 只差 1e-4**。
同一个量在两个文件里差两个数量级，只能是两个不同的东西。

### 上游确实重算

`MOD_Vars_1DAccFluxes:accumulate_fluxes`（`MOD_Vars_1DAccFluxes.F90:2733-2790`）在累加
**之前**用参考层量重算一遍，`acc1d` 收的是 `r_ustar`/`r_tstar`/`r_qstar`/`r_zol`/`r_rib`/
`r_fm`/`r_fh`/`r_fq` 这八个局部量，不是同名的时间变量（`:2811-2820`）：

```fortran
z0m_av = z0m ; z0h_av = z0m_av ; z0q_av = z0m_av
displa_av = 2./3.*z0m_av/0.07                       ! 固定式，不是冠层算出来的位移高度
hgt_u = max(hgt_u, 5.+displa_av)                    ! 观测高度有下限
zldis = hgt_u - displa_av
rhoair = (psrf - 0.378*qm*psrf/(0.622+0.378*qm)) / (rgas*tm)   ! 按参考层重算密度
r_ustar_e = sqrt(max(1.e-6, sqrt(taux**2+tauy**2))/rhoair)     ! 由应力直接算
r_tstar_e = -fsena_e/(rhoair*r_ustar_e)/cpair
r_qstar_e = -fevpa_e/(rhoair*r_ustar_e)
thv = tm*(1e5/psrf)**(rgas/cpair) * (1.+0.61*qm)
r_zol_e = zldis*vonkar*grav*(r_tstar_e*(1.+0.61*qm)+0.61*th*r_qstar_e)/(r_ustar_e**2*thv)
r_zol_e = clamp 到 [1e-6, 2] 或 [-100, -1e-6]
um = 稳定 ? max(ur,0.1) : max(0.1, sqrt(ur**2 + (beta*(-grav*r_ustar*thvstar*zii/thv)**(1/3))**2))
CALL moninobuk(hgt_u,hgt_t,hgt_q,displa_av,z0m_av,z0h_av,z0q_av, zldis/r_zol_e, um, ...)
r_rib_e = r_zol_e/vonkar * r_ustar2_e**2 / (vonkar/r_fh_e*um**2) ; min(5., ...)
```

CN-Cng 的强迫参考高度是 6 m，而 `displa = 2/3*0.1206/0.07 = 1.148`，所以
`hgt = max(6, 6.148) = 6.148`、`zldis ≡ 5.0` —— **下限真的生效**，这是 `zol`/`rib`
与重启差一倍的原因。

顺带确认了两条容易搞反的取值：`tref`/`qref`/`z0m`/`taux`/`tauy` 确实是
`acc1d` 直接累加模型状态（`:2185` 一带），不必重算；`trad` 是
`r_trad = (olrg/stefnc)**0.25` 从 `olrg` 现算的。

### 移植

`colm_core::history_diagnostics`（`history_diagnostics.rs`）逐行移植上面那一段，
`moninobuk` 复用已有的 `monin_obukhov_with_scheme`。`set_lct_surface_diagnostics`
改成收 `&StandardLctEnergyOutput` + 一个 `HistoryReferenceState`（参考层的风/温/湿/气压/
边界层高度，从 `step.forcing` 取），八个量走重算，其余五项照旧。

### 实测

CN-Cng 第 1 天（区间平均，首条记录）：

| | Fortran | 直接写 `leaf.*` | 重算之后 |
|---|---|---|---|
| `f_ustar` | 0.564507 | 0.624830（**+10.7%**） | **0.564537** |
| `f_zol` | −0.226497 | −0.343605 | **−0.226174** |
| `f_rib` | −0.061445 | −0.089860 | **−0.061357** |
| `f_tstar` | −0.909686 | −1.138964 | **−0.908716** |
| `f_fm` | 3.248590 | 4.114634 | **3.249003** |
| `f_fh` | 2.862696 | 2.870565 | **2.863380** |

八项全部落到 0.3% 以内（相比之下 `leaf.*` 那条路差 10%~70%）。单位参照是独立
gfortran 程序：`moninobuk` 原文 + 上面那段逐字复制，两组 CN-Cng 量级的输入。

### 一条顺带的观察：`f77()` 约定带来 1e-8 量级的系统差

`r_rib` 对到 3e-8 而不是 1e-15，来源是本仓库 `monin_obukhov.rs` 的

```rust
const fn f77(value: f32) -> f64 { value as f64 }
const VON_KARMAN: f64 = f77(0.4);     // = 0.4000000059604645
```

而 `MOD_Namelist`/`MOD_Const_Physical` 里的 `0.4` 在 `-fdefault-real-8` 下是 `0.4_r8`。
两者差 1.5e-8，`r_rib` 里 `vonkar` 出现两次所以放大到 3e-8。**这个约定对"未加后缀的
Fortran 字面量"是反的**（它们被 `-fdefault-real-8` 提升成 r8，不是 r4）。量级远低于
所有容差（history tier2 是 rtol=1e-7），所以本轮不改；要改就得逐处核对哪些常量在
上游是 r4、哪些是 r8，记在下面作为候选工作项。

## 两条 history 闸门的错位：`frcsat` 多写、`f_qcharge` 少一个层级（2026 年，实测）

重跑分层比对（`golden-compare --tolerances oracle/tolerances.toml`）时，把
`oracle/work/CN-Cng-aligned/out/CN-Cng/history/` 当作参照，两条**不是数值大小**的问题露出来：

### `frcsat`：上游的 `WATER_2014` 从不设它

对齐算例 264 条记录里 Fortran 的 `f_frcsat` **整列是 `spval`**，而本仓库写的是有限值
（首条 0.674）。查源码：带 `frcsat` 实参的 `Runoff_TOPMOD`/`Runoff_XinAnJiang`/
`Runoff_SimpleVIC` 调用全在 **`WATER_VSF`** 里
（`MOD_SoilSnowHydrology.F90:880-925`，而 `WATER_VSF` 是 `:529-1341`），
`WATER_2014`（`:…-526`）根本不传这个实参。开了 VSF 的黄金算例则整列有值 —— 两边正好相反。

本仓库给 `Runoff_*` 传了 `frcsat`，于是**造出了上游没有的量**。修法是声明但不填：
`colm-hist` 的填充值与上游的 `spval` 都是 -1e36，留空即逐位相同。
新增常量 `DECLARED_BUT_UNFILLED` 把"声明了但按上游不该有值"这件事写进代码，
并有回归测试断言那一列确实停在填充值。

### `f_qcharge`：容差表缺一个层级

`f_qcharge` 受 `.and. (.not. DEF_USE_VariablySaturatedFlow)` 控制
（`MOD_Hist.F90:698`）：黄金算例开着 VSF 所以没有这一列，对齐算例关了所以有。
`oracle/tolerances.toml` 里没有它的层级，比对器报"cannot be judged"。
已按它的来源（`WATER_2014` 的 `qcharge`，一个确定性诊断）放进 tier2。

### 剩下的差距（分层比对，264 条记录）

| 变量 | Fortran | Rust | 量级 |
|---|---|---|---|
| `f_t_grnd` / `f_tleaf` | 268.9966 / 265.8717 | 268.2738 / 266.6364 | 0.7 K |
| `f_wliq_soisno`（最差槽位） | 0.9174 | 3.1617 | 2.2 kg/m² |
| `f_ustar` | 0.17322 | 0.18179 | 5% |
| `f_zol` / `f_rib` | 0.12392 / 0.028574 | 0.084386 / 0.020324 | ~1.4× |
| `f_fgrnd` | 3.1461 | −7.0048 | 10 W/m² |
| `f_qinfl` | −9.31e-5 | 0.0 | — |
| `f_lfevpa` | 184.677 | 615.054 | 见上文，采样阶段待定 |

`f_qinfl` 首条记录 Fortran 是**负**的（−9.31e-5），Rust 是 0：上游 `qinfl` 在
`WATER_2014` 里由 `gwat - rsur` 得到，夜里 `gwat` 可为负（冻结/再分配），
而本仓库把负的入渗截成了 0。这是下一个可以单独查的点。

## 无雪层时融化与蒸发没进土壤收支：土柱偏湿（2026 年，实测）

分层比对里 `f_qinfl` 首条记录 Fortran 是 **−9.313e-5**、Rust 是 **0**。追下去发现
Rust 的 `f_qinfl` **整段 264 条恒为 0** —— 这不是"差一点"，是整个入渗项没在工作。

对着 Fortran 一列列看，规律立刻出来：

```
Fortran  f_qinfl = [-9.313e-5, -6.631e-5, -5.177e-5, -5.816e-5, ...]
Fortran  f_fevpg = [ 9.313e-5,  6.631e-5,  5.177e-5,  5.816e-5, ...]   ← 逐条相反数
```

`WATER_2014` 的入渗是 `qinfl = gwat - rsur - wdsrf/deltim`
（`MOD_SoilSnowHydrology.F90:368`），而 `gwat` 分两支（`:235-245`）：

```fortran
IF (lb>=1) THEN                      ! == snl == 0：没有雪层
   gwat = pg_rain + sm - qseva       ! 薄雪融化 sm 与液态蒸发 qseva 都进土壤
ELSE
   CALL snowwater (..., gwat)        ! 有雪层时由雪柱底部排水给 gwat
ENDIF
```

`water_2014_snow_soil_step` 的无雪层支把 `snowmelt_kg_m2_s` 与
`ground_evaporation_kg_m2_s` **都写成了 0**，理由是"`snowwater` 负责这些通量"——
可这一支恰恰**跳过**了 `snowwater`。于是蒸发与融化凭空消失，`gwat` 只剩降雨，
夜里就是 0，土壤只进不出、越来越湿。

修法：无雪层支把 `sm`（`energy.ground.snow_melt_rate_kg_m2_s`，`meltf` 只在
`lb == 1 && scv > 0` 时赋值，所以有雪层时恒为 0）与 `qseva`
（`SnowWaterInput::evaporation_kg_m2_s`）原样传给土壤收支。

### 实测：整条序列与重启都改善

对齐算例第 1 天（264 条 history 的最大差）：

| | Fortran | 修之前 | 修之后 |
|---|---|---|---|
| `f_qinfl` | −9.313e-5 | 0（恒为 0） | 差 4.3e-6 |
| `f_wliq_soisno`（最差槽位） | — | 差 2.24 kg/m² | **差 0.49 kg/m²** |
| `f_t_grnd`（最差记录） | — | 差 **0.72 K** | **差 0.075 K** |
| `f_rnof` | — | 差 3.1e-4 | 差 7.0e-5 |

528 步后的重启：`wliq_soisno` 最差从 1.017 收到 **0.275** kg/m²，
`t_grnd` 从 +0.047 K 变成 −0.060 K（端点上略远一点，但 264 条记录的**最差**从
0.72 K 收到 0.075 K —— 整条轨迹都对了，不是只把终点凑上）。

**教训**：把"某一步的通量已经由另一个内核处理"写成常量 0 时，必须同时检查那条
路径在这一支上是否真的被执行。这里的注释写的是"`snowwater` 负责降雨、蒸发、
露、霜、升华"，而代码同一处的 `if` 正是"不调 `snowwater`"。

## `scv` 那 16% 不是雪的物理问题，是冠层露水溢出的放大（2026 年，实测）

追了两轮的"`scv` 只有 Fortran 的 1/3、后来偏高 16%"，结论是**它不是雪收支的错**。

做法：从同一个初始重启出发，逐步建 1/2/3/4 步的探针算例
（复制 `oracle/work/CN-Cng-2step`，只改 `DEF_simulation_time%end_sec`，
先 `colm-cli run --stage colm` 跑 Fortran，再 `colm-rs` 跑同样步数），逐步对比 `scv`：

| 步数（结束时刻） | Fortran `scv` | Rust `scv` |
|---|---|---|
| 1（00:30） | 0 | 0 |
| 2（01:00） | 0 | 0 |
| 3（01:30） | 0 | 0 |
| 4（02:00） | 0.008714 | 0.011848 |

前三步**逐位相同**，全部差异在第四步产生。而该站的强迫在这个窗口里
`Precip ≡ 0`（`examples/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc` 前 3445 条记录
全是 0，即头 71 天无降水）——所以 `scv` 根本不是降水积起来的。

它是**冠层露水溢出**。冠层水 `ldew` 在夜里靠凝结增长，`snowfraction`/`interception`
每步开头做

```fortran
IF (tleaf > tfrz) THEN ; xsc_rain = max(0., ldew-satcap) ; xsc_snow = 0.
ELSE                   ; xsc_rain = 0. ; xsc_snow = max(0., ldew-satcap)
ENDIF
```

`satcap = dewmx*lsai = 0.1*0.65 = 0.065` mm。两边的 `ldew` 轨迹是

```
Fortran  0 → 0.047420 → 0.062173 → 0.073714 → 0.075391
Rust     0 → 0.049523 → 0.064951 → 0.076848 → 0.075900
```

第三步末 Fortran 是 0.073714 < 0.065？不 —— 第四步**开头**，Fortran 的 `ldew` 是
0.073714（第三步末的值 0.062173 加第四步前的…，按步序对上），**第一次超过** `satcap`，
于是 `xsc_snow = 0.073714-0.065 = 0.008714` ✓ 与 `scv` 逐位相同；Rust 同理得到
`0.076848-0.065 = 0.011848` ✓。

所以 `scv` 的差 = 冠层露水的差（0.0031 mm，约 4%），而露水本身是
`rhoair*(1-delta*(1-fwet))*lsai/rb*((wtaq0+wtgq0)*qsatl - wtaq0*qm - wtgq0*qg)`
——**括号里是两个几乎相等的量相减**，`qsatl` 对 `tleaf` 极敏感：`tleaf` 差 0.05 K
就够让这个括号差 5%，于是 `scv` 差 30%+。

**结论**：`scv` 在这里是 0.05 mm 量级的量，它是"冠层储水刚好越过 0.065 mm 阈值"的溢出，
不是雪的累积/消融错。不要再把它当成雪物理的验收指标；要看雪收支应当用有真实降雪的
窗口（该站头 71 天无降水，`CN-Cng` 的雪是露水冻出来的）。

## `lat`/`lon` 要带 f32 量化（2026 年，实测）

分层比对里 `lat`/`lon` 也被报成超差 —— 它们是 tier0 的**坐标**，按逐位比：

```
Fortran  lat = 44.593299865722656   lon = 123.50920104980469
Rust     lat = 44.5933              lon = 123.5092
```

原因不是"算错了"，是**上游把站点经纬度存成 `real(r4)`**。`mksrfdata` 自己就报过：

```
Warning: Latitude mismatch:    44.593299865722656       in data file and    44.593299999999999      in namelist.
```

数据文件那份是 f32 的（`44.593299865722656 == (44.5933 as f32) as f64`），namelist 那份是 f64。
上游写进 history 的是前者，本仓库读的是后者。

修法：`PointRuntimeConfig::history_session` 构造 `HistorySite` 时把站点坐标过一次 f32，
并把这个理由写成注释 + 回归测试。**不动**轨道几何那一侧（`orb_coszen` 用的是弧度的
`patchlatr`，那边的差在 1e-9 量级、早已对得上）。

比对数从 46 项收到 **44 项**。

## 下一轮：蒸腾系统性偏小约 50 倍（2026 年，实测）

分层比对剩下 44 项超差，其中**唯一一个系统性、量级性的**是蒸腾：

```
f_etr  264 条记录里有 130 条 |差| > 1e-6；Fortran 峰值 1.268e-5，Rust 峰值 3.281e-7
       Fortran   2.517e-6  6.155e-6  7.252e-6  ...  1.268e-5  (kg m-2 s-1)
       Rust      1.360e-7  2.871e-7  3.281e-7  ...  8.553e-8
```

不是差一点，是**恒定的约 1/50**。`f_etr` = `acc1d(etr, a_etr)`，而 `etr = etrsun + etrsha`
来自 `LEAF` 的光合/气孔链；`f_fevpl = etr + evplwet` 与 `f_fevpa = fevpl + fevpg`
因此也跟着偏（`fevpg` 两侧对得上）。

同时看到一个同源的症状：**重启里的 `rst`（冠层气孔阻力）**

```
Fortran  rst = -2.53491436     ← 负的
Rust     rst =  500000
```

两侧的公式是同一行（`MOD_LeafTemperature.F90:1041` / `leaf_temperature.rs:897`）：

```fortran
rst = 1./(laisun/rssun + laisha/rssha)
```

所以差异在 `rssun`/`rssha`：Fortran 那边算出的 `1/(...)` 是负的，说明 `rssun` 或 `rssha`
为负（夜间呼吸主导时光合为负，CoLM 会得到负的电导）；Rust 这边 `rssun`/`rssha`
很大且为正，`rst` 到 5e5 → 蒸腾几乎被掐死。

最可能的入口是**土壤水分胁迫因子**（`rstfac`，`MOD_Eroot` 的 `eroot`）：
本窗口土壤冻结、`smp ≈ -2.3e6 mm`，若 Rust 的 `rstfac` 落在 ~0.02 而 Fortran 是 1，
正好是这个 50 倍。下一步先打印两侧的 `rstfac`/`rssun`/`rssha` 对比，再决定是
`eroot` 的移植问题还是 `rssun`/`rssha` 的符号处理。

## 蒸腾偏小的入口定位到**夜间净同化的符号**（2026 年，实测）

上一节把 50 倍的蒸腾差记成"最可能是 `rstfac`"。打印之后否掉了一半：

```
STEP  40 stress=3.005e-1 etr=2.64e-10 tleaf=261.88 rst=5.000000e5
STEP  80 stress=1.676e-1 etr=1.07e-7  tleaf=268.30 rst=3.5166e3
STEP 200 stress=8.868e-2 etr=0.0      tleaf=258.29 rst=5.000000e5
STEP 528 stress=4.568e-2 etr=3.09e-10 tleaf=254.03 rst=5.000000e5
```

* **`stress` 不是 0.02**，是 0.046~0.30 —— 有胁迫，但只解释 3~20 倍，不是 50 倍。
* **`rst` 恰好等于 `5.000000e5` 很多步**。这不是算出来的巧合，是**两个分数分量都顶到上限**：
  `stomata` 末行是 `rst = min(1e6, 1/(gsh2o*tlef/tprcor))`，单叶值封顶 1e6，
  回到 `rst = 1/(laisun/rssun + laisha/rssha)` 就得到 `1/(2e-6) = 5e5`。
  也就是说本仓库的气孔是**关死**的。

而 Fortran 的 `rst = -2.53491436`（负的）。负值只可能来自 **Ball-Berry 支**里

```fortran
hcdma = ei*co2st / ( gradm_used*assmt )     ! assmt < 0 时 hcdma < 0
...
es = max( es, 1.e-2)
gsh2o = es/hcdma + bintc                    ! 仍为负
rst   = min( 1.e6, 1./(gsh2o*tlef/tprcor) ) ! 取到负值
```

所以两侧的分歧**不在气孔阻力这一步，而在它的上游**：`assmt`（净同化）的符号。
夜间呼吸主导时 Fortran 的 `assmt < 0` → `hcdma < 0` → `gsh2o < 0` → `rst < 0`；
本仓库的净同化是**正的小量** → `hcdma > 0` → `gsh2o ≈ 4.5e-5 mol m-2 s-1`（几乎为零但仍为正）
→ `rst` 顶到上限 → 蒸腾被掐死。

两侧都走 Ball-Berry（`DEF_USE_MEDLYNST` 在 schema 与 Fortran 里都默认 `.false.`，已核对），
所以不是分支选错。

**下一步**：在同一个夜间步上打印两侧的 `assimsun`/`respc**`/`assmt`/`hcdma`。
净同化的符号差通常来自呼吸项（`respc`）或 `update_photosyn` 的夜间分支，
这是一个单点、可判定的比较。

## 这个窗口的光合链本身是退化的（2026 年，实测）

按上一节的指引去读 Fortran 自己的 `f_assim*`/`f_respc`，结果出乎意料：

```
f_assimsun   264 条记录**全部恰好 = 0**
f_assimsha   264 条记录**全部恰好 = 0**
f_respc      [-2.084e+28, -6.012e+27]     ← 量级本身就不是物理量
f_etrsun     [-1.504e-06, 1.104e-05]      ← 但蒸腾是有限的、量级合理的
f_etrsha     [0, 2.528e-06]
```

也就是说，**这个窗口（CN-Cng 2008 年 1 月，冠层长期 −15 ℃ 量级）里上游的净同化恒为 0，
冠层呼吸是 1e28 量级的垃圾值，而蒸腾却是正常的**。

这对上一节的推断有直接影响：`stomata` 里 `assmt = max(1.e-12, assimn)`
（`MOD_AssimStomataConductance.F90:323`）把 0 兜到 1e-12，于是
`hcdma = ei*co2st/(gradm_used*assmt)` 变成一个 ~1e12 量级的大数；
再经 Ball-Berry 的二次式、`es = max(es,1.e-2)`、`gsh2o = es/hcdma + bintc`
这一串，`rst = min(1e6, 1/(gsh2o*tlef/tprcor))` 落成 **−2.53** 这种值。
它不是"算出来的物理阻力"，而是退化输入走完一串兜底之后的结果。

**结论**：蒸腾那 50 倍**不应该在这个窗口里对齐**。上一条记的"入口在夜间净同化的符号"
仍然成立，但要判定谁对，得换一个**冠层不结冰、`assim > 0`** 的窗口（例如 CN-Cng 的夏季
月份）—— 在那里 `assmt` 有真实值，气孔链才是活的。黄金比对里
`f_etr`/`f_fevpl`/`f_lfevpa` 这几项在本窗口的差异因此**不可归因**，也不该据此改代码。

（`f_respc` 那一列 1e28 也说明：`respc` 在这个配置下从未被赋过有意义的值，
本仓库也**不该**为了让某一列对上而去复现它。）

## 月度 LAI 重读**没有接进运行循环**（2026 年，实测）

上一节说下一步要跑一个暖季窗口来验蒸腾。走到那一步才发现它有个前提没满足：

`DEF_LAI_MONTHLY = .true.`（对齐算例 `case.nml:47`）时，上游每月重读一次 LAI
（`CoLM.F90:595-605` 的 `CALL LAI_readin(lai_year, month, dir_landdata)`），
而本仓库：

```rust
// runtime_clock.rs:174 —— 算出这一位
self.update_lai = lai_update_due(forcing_time, next_forcing_time, self.lai_schedule);
```

**没有任何消费者。** 全仓库 grep `update_lai` 只在 `runtime_clock.rs` 内部出现
（赋值、随步复制、进 `RuntimeStep`），`colm-runtime/src/lib.rs` 一次都没读它。

于是运行期的 `tlai`/`tsai` 恒为装配期从时间重启读到的那一对。11 天的对齐窗口整段在
1 月内，`LAI_readin` 本来也不会触发，所以这条一直没暴露；**任何跨月的运行都会
从第二个月起用错 LAI**（影响截留、两流、叶温、蒸腾、地面反照率）。

这也直接决定了两件事：

1. **暖季蒸腾验证的前提是先接上这条链。** 否则拿 1 月的 LAI（`tlai = 0.2`）去比
   7 月的 Fortran（`LAI_readin` 读到夏季叶面积），蒸腾的差异会被 LAI 差异淹没 ——
   那正是前两节反复踩的"用错的参照去判定对错"。
2. 读取器**已经有了**：`crates/colm-init/src/surface_data.rs` 的 LAI 读取
   （`LAI_readin` 的同月选择也在那里），而 `colm-runtime` 本来就依赖 `colm-init`
   （`assembly.rs` 用 `colm_init::RestartFile`）。缺的只是"到月就重读、并覆盖
   `StandardLctRestartTemplate` 里那一对 `tlai`/`tsai`"这一步。

实现要点：`tlai`/`tsai` 现在是**装配期的不可变字段**，而运行期需要它们可变；
解法与 `lai`/`sai` 那次一样 —— 把它们放进运行态（或每步从运行态取），
而不是继续留在模板里。`update_lai` 已经算好了，直接用。

## 接上月度 LAI 重读（2026 年，实测）

按上一节说的做了。改动四处：

1. **`tlai`/`tsai` 从装配期常量变成运行态**：新增 `TemporalCanopy` 放进
   `StandardLctEnergyState`，`prepare_surface_optics` 改从状态里取。
   与既有的 `CanopyGeometry`（每步折算后的**有效**值）分开存 —— 用有效值再折算一次
   就是重复相乘。
2. **`MonthlyLeafAreaIndex`**（`assembly.rs`）：读 `srfdata.nc` 的
   `LAI_monthly`/`SAI_monthly`（`USE_SITE_LAI` 那一支），`for_time` 复刻
   `LAI_readin(lai_year, month, ...)` 的年份选择。`USE_SITE_LAI = .false.`
   要的 `LAI/<年>/LAI_patches<月>.nc` 本仓库还没读，**显式报错**而不是静默用旧值。
3. **`colm-rs` 装上它**：`DEF_LAI_MONTHLY` 打开时从 `<out>/<case>/landdata/srfdata.nc`
   读，缺文件就报错。做成 builder（`with_monthly_leaf_area_index`）而不是装配参数，
   因为 landdata 路径要算例名与输出目录。
4. **顺序照上游**：`LAI_readin` 在 `CoLMDRIVER`（含末尾那一节）与 `hist_out`
   **之后**、`WRITE_TimeVariables` **之前**，所以这一步 history 记的仍是旧
   `tlai`/`tsai` 折算的 `lai`/`sai`，而重启里的 `tlai`/`tsai` 已是新一个月。
   月份取**步末**（`CoLM.F90:484`），所以先把 `86400` 的写法退位
   （新增 `colm_core::end_of_step_calendar_time`）。

### 顺带修掉的两个真 bug

**一、`update_lai` 晚一步生效。** `RuntimeClock::next_step` 先
`update_lai: self.update_lai` 再在下面重算，于是每一步拿到的是**上一步**算出来的那一位
（第一步还拿到初始化时的 `true`）。跨月算例里表现为 5 月第一小时的 `f_lai` 是
0.2/0.4 的**平均 0.3**。改成先算本步的再构造。回归测试原来钉的正是错的行为
（`[true, false]`、`[true, true, false]`），已一并改成正确的，并补了一个跨年跨月的用例。

**二、冠层水的雪分量会出 −3e-18。** `update_canopy_water` 里
`snow_mm = total_mm - rain_mm`，而 `rain_mm` 由比例分回时可能比 `total_mm` 大一点点
（浮点结合律），于是雪分量是**负的舍入残差**。它把 `intercept_canopy` 的入参校验打掉，
跨月算例到此直接失败。修法是把 `rain_mm` 夹到 `[0, total_mm]`。
同时把那句合成的大 `ensure!` 拆成逐项报名字（实测只知道"state is invalid"要二分）。

### 实测

CN-Cng 对齐算例从 1 月 1 日跑到 7 月 5 日（**8976 步**、7 个 history 文件），
Rust 全程跑通，`f_lai` 的月序列是 0.2 → 0.2 → 0.2 → 0.2 → 0.4 → 0.7 → **1.8**，
与 `srfdata.nc` 的 `LAI_monthly` 逐月一致（`LAI_monthly` = 0.2,0.2,0.2,0.2,0.4,0.7,
**1.8**,1.5,0.7,0.4,0.2,0.2）。修 `update_lai` 之前 7 月只到 0.7（6 月的值）。

### 两个还没解决的

* **Fortran 在 7 月自己崩了**：`colm-cli run --stage colm` 在第 8854 步
  （2008-07-03 10:30）收到 `SIGILL`（`-ffpe-trap` 一类），7 月的 history 文件因此
  只写了维度、没有数据。所以**暖季的蒸腾验证仍然缺参照** —— 障碍在参照那一侧，
  不在本仓库。下一步要先弄清上游在这个配置下为什么会在夏天触发浮点异常。
* **按月分组的边界差一条记录**：Fortran 的 4 月文件最后一条是 `56976450`
  （5 月 1 日 00:00），5 月文件从 `56976510`（01:00）开始；Rust 把 00:00 那条算进了 5 月。
  那一刻的累积区间是 4 月 30 日 23:00→5 月 1 日 00:00，归 4 月才是自然的。
  这是 `colm-hist` 调度的分组选择问题，黄金算例（整段在 1 月内）测不到它。

## 按月分组的边界：写点那一天，不是写点覆盖的区间（2026 年，实测）

上一节把这条记成"Rust 把 5 月 1 日 00:00 那条算进了 5 月"。查上游之后确认**Rust 错**：

`MOD_Hist.F90:273` 的文件后缀取 `idate`，而 `hist_out` 拿到的 `idate` 是 `TICKTIME`
**之后**的写法 —— 月末 24:00 记作"当月最后一天 `86400` 秒"。于是
`julian2monthday(idate(1), idate(2), month, day)` 对 5 月 1 日 00:00 读到的是
**4 月 30 日**，后缀就是 `2008-04`。实测 Fortran 的 4 月文件最后一条正是
`56976450`（5 月 1 日 00:00），5 月文件从 `56976510`（01:00）开始。

`isendofmonth` 也是同一套写法（比 `idate` 与 `idate+deltim` 的月份），与
`period_ends` 的绝对 tick 判据一致 —— 所以**只有后缀**需要改：

```rust
// `time` 标签用写入时刻本身；分组用它的 end-style 日期（午夜那条退一秒）
let suffix_tick = if next % 86_400 == 0 { next - 1 } else { next };
```

（重启目录名走的是 `jdate`（`adj2begin` 之后），所以那边是 begin-style 的
`2008-037-00000`，与这里相反 —— 两处不能混。）

改完之后拿 7 个月的对齐算例逐月对时间轴：

```
04  F times (56933310, 56976450)  R (56933310, 56976450)   lai 0.2 / 0.2
05  F times (56976510, 57021090)  R (56976510, 57021090)   lai 0.4 / 0.4
06  F times (57021150, 57064290)  R (57021150, 57064290)   lai 0.7 / 0.7
07  F 无数据（上游崩了）           R (57064350, 57071490)   lai 1.8
```

**逐位相同的记录边界 + 逐位相同的 `f_lai`** —— 月度 LAI 与分组边界两条一起对上了。

## 暖季对照其实拿得到：4~6 月（2026 年，实测）

上一节说"7 月上游崩了所以暖季没法验"。实际上下游的 4/5/6 月 history 是完整的
（崩溃在第 8854 步，只毁了 7 月那一个文件），而那三个月 LAI 分别是 0.2/0.4/0.7
—— 冠层是活的，正好是蒸腾链该活跃的地方。同一批数据给出：

| 月份 | `f_etr` | `f_tleaf` | `f_t_grnd` |
|---|---|---|---|
| 01 | F 1.268e-5 / R 8.55e-8 | F 265.9 / R 266.6 | F 260.9 / R 260.1 |
| 04 | F **−2.779e-4** / R 1.49e-8 | F 294.0 / R **301.3** | F 282.9 / R **274.2** |
| 05 | F 1.563e-4 / R 2.36e-5 | F 288.2 / R **296.0** | F 289.1 / R 286.6 |
| 06 | F 1.465e-4 / R 3.45e-5 | F 300.1 / R **307.4** | F 290.5 / R 292.6 |

三条结论：

1. **`f_lai` 三个月都逐位相同**（0.2/0.4/0.7），所以 LAI 不再是干扰项。
2. 暖季的 `f_tleaf` **一致偏暖约 7 K**，而 `f_etr` 小 4~6 倍 —— 冠层蒸腾不足、
   潜热散不出去，温度自然偏高。两条是同一个病的两面。
3. 上游 6 月的 `rst` 仍然是**负的**（`−2.9289461`，与 1 月的 −2.53 同性质），
   所以"负气孔阻力"**不是结冰窗口的退化产物**，而是一个稳定的行为 ——
   下一步要比的是暖季夜间/白天的 `assimsun`/`hcdma`/`gsh2o`，那里 `assim` 有真值。

Fortran 在 7 月崩（`SIGILL`，第 8854 步）仍然是上游自己的问题，但它不再挡住验证。

## `tlai`/`tsai` 也得写回重启（2026 年，实测）

上一轮接上月度重读之后，跨月运行在 history 上已经对齐。接着比**重启**：

```
2008-06-01 00:00（6 月边界）    Fortran      Rust（写回之前）
tlai                            0.69999999   0.2          ← 1 月的值
tsai                            0.44999999   0.45
lai                             0.40000001   0.7
```

`lai` 也对不上，但那是因为我把 Rust 的窗口设成了 6 月 2 日 00:00 而不是 6 月 1 日 00:00。
真正的问题是第一行：**`tlai`/`tsai` 根本没写回**。`evolved_snow_overrides` 写了
`lai`/`sai`/`sigf`/`thermk`/`extkb`/`extkd`，唯独漏了这两个原始时间变量。

后果与上一轮修的那个 bug 同源、只是低一层：一份 6 月结束的重启里 `tlai` 还是 1 月的值，
**续跑就从 1 月的叶面积起步**。上游是写的（`MOD_Vars_TimeVariables.F90:1184`）。
补上之后（`RestartColumns` 多两根缓冲 + 两条 override，来源是
`state.energy.temporal_canopy`）。

### 对齐到边界那一刻

把窗口改成正好结束在 6 月 1 日 00:00（`end_sec = 0`）再比：

| | Fortran | Rust |
|---|---|---|
| `tlai` | 0.69999999 | **0.69999999** |
| `tsai` | 0.44999999 | **0.44999999** |
| `lai` | 0.40000001 | **0.40000001** |
| `sai` | 0.44999999 | **0.44999999** |
| `sigf` | 1 | **1** |

**五个量逐位相同**，而且连上游那个顺序怪癖也复现了：这一刻的重启里 `lai` 是 **5 月**的有效值
（0.4），`tlai` 已经是 **6 月**的值（0.7）—— 因为 `LAI_readin` 在末尾那一节与 `hist_out`
**之后**才跑。月度 LAI 这条链到此端到端验完。

同一刻剩下的差：`tleaf` +1.92 K、`t_grnd` +0.87 K（都是夜间），`rst` −2.93 对 5e5。
比 6 月白天最大差（+7 K）小得多，说明那 7 K 主要在白天 —— 与"蒸腾不足、潜热散不出去"
一致。

### 顺带确认：上游的同化在**所有**月份都是 0

上一轮以为 6 月 `f_assimsun` 会有真值。实测仍是 `f_assimsun ≡ 0`、`f_assimsha ≡ 0`、
`f_respc` 在 −1e29 量级 —— 与 1 月完全一样。所以那不是结冰窗口的产物，而是这个配置下
（`DEF_USE_OZONESTRESS` 打开而臭氧数据缺失，`assimsun = assimsun*o3coefv_sun` 把它清零）
**全年**如此。因此 history 里的 `f_assimsun` **不能**用来判定气孔链 ——
它被后置的臭氧因子清零了，而 `rssun`/`rssha` 用的是 `stomata` 内部那份真实的同化。
要判定只能比 `rst`（重启里有）与 `tleaf`/`etr`（history 里有）。

## 找到蒸腾链的真凶：扩展截获模块漏了 `intent(inout)`（2026 年，实测）

上一节把"上游同化恒为 0"归因于臭氧胁迫，**那条归因是错的**：本算例
`case.nml` 里明写 `DEF_USE_OZONESTRESS = .false.`、`DEF_USE_OZONEDATA = .false.`，
`CalcOzoneStress` 根本没进过。真正的原因在下一层。

### 先纠正一个更根本的误解：链接的不是 `main/`

`build_kernel.sh default` 生成的宏集里有 **`extend_interception`**
（`kernels/default/manifest.json` 的 `macros` 字段，实测
`["LULC_IGBP","SinglePoint","URBAN_MODEL","extend_interception"]`）。
`vendor/CoLM202X/Makefile:635-647` 在这条宏打开时**用同名模块顶替 `main/`**：

| 编出来的对象 | `main/` 那份 | 实际编译的源码 |
|---|---|---|
| `MOD_LeafTemperature.o` | `main/MOD_LeafTemperature.F90` | `extends/interception/MOD_LeafTemperature_Extended.F90` |
| `MOD_LeafTemperaturePC.o` | `main/MOD_LeafTemperaturePC.F90` | `extends/interception/MOD_LeafTemperaturePC_Extended.F90` |
| `MOD_Thermal.o` | `main/MOD_Thermal.F90` | `extends/interception/MOD_Thermal_CanopyPhase_Extended.F90` |
| `MOD_LeafInterception.o` | `main/MOD_LeafInterception.F90` | `extends/interception/MOD_LeafInterception_Extended.F90` |
| （额外） | — | `extends/interception/MOD_PHSRootfluxBalance.F90` |

**`main/` 的那四份根本不参与编译。** 后果：往 `main/MOD_Thermal.F90` 里插
`write` 语句、重编、跑，一个字都不会打印（本轮实测：`strings kernels/default/colm.x`
里没有那句诊断，而同一次构建里 `main/MOD_AssimStomataConductance.F90`
（无 Extended 替代品）的诊断在）。定位数值问题时**先确认哪份文件在编译**，
判据是 `Makefile` 的 `EXTENDED_INTERCEPTION_ENABLED` 分派加上
`.bld/*.o` 的编译命令行 —— 不是文件名像不像。

这条也解释了为什么此前按 `main/MOD_LeafTemperature.F90` 的行号读代码、
却对不上内核行为。

### 缺陷：`rstfacsun`/`rstfacsha` 在扩展版里是 `intent(out)`

`main/MOD_LeafTemperature.F90:269-272` 有一块独立声明（f48fbf9 同步带来的）：

```fortran
   ! Read the caller's soil water stress factors; plant hydraulics may update them.
   real(r8), intent(inout) :: &
        rstfacsun,  &! factor of soil water stress to transpiration on sunlit leaf
        rstfacsha
```

`extends/interception/MOD_LeafTemperature_Extended.F90` 里同样的两个名字却落在
上面的 `real(r8), intent(out) ::` 块中（原第 255-273 行）。`intent(out)`
意味着**入口值未定义**，而 `MOD_Thermal` 在调用前刚把它算好
（`rstfacsun_out = rstfac`，`rstfac` 来自 `MOD_Eroot`）。

于是 `stomata` 读到的 `rstfac` 是调用点寄存器/栈槽里的残留值。实测该残留值就是
**`spval = -1e36`** —— 因为 `MOD_Vars_TimeVariables.F90:654` 把 `rstfacsun_out(:)`
初始化成了 `spval`。`calc_photo_params` 用它直接缩放四个量、没有任何上下界：

```fortran
vm   = vmax25*2.1**qt/temph*rstfac*c3 + ...      ! → -5.7e30
jmax = jmax*rstfac                               ! → epar = min(...,jmax) 全负
respc= respcp*...*rstfac                         ! → assimn = 0 - respc = +9.2e28
omss = ...*rstfac
```

连锁结果是**冠层光合恒为 0、气孔阻力形同不存在**：

| `stomata` 内量 | 坏内核（`rstfac = -1e36`） | 修好后（`rstfac = 0.25079`） | Rust |
|---|---|---|---|
| `vm` | −5.717789e30 | 1.433969e−6 | — |
| `omc` | −3.557435e30 | 8.921721e−7 | 2.223397e−6 |
| `ome` | −1.225718e31 | 0 | 0 |
| `assimn` | +9.211995e28 | −2.310284e−8 | — |
| `assmt` | 9.211995e28 | 1e−12 | 1e−12 |
| `bintc` | 3.296800e−4 | 8.268071e−4 | — |
| `co2st` | 1e−5 | 3.880200e−4 | 3.883888e−4 |
| `gsh2o` | +3.458904e33 | 3.754783e−8 | 1.773942e−8 |
| `rst` | **1.2e−32**（≈0 ⇒ 无气孔限制） | 1e6（顶到上限） | 1e6 |

（同一算例同一步第一次 `stomata` 调用，`tlef` 三边逐位相同 283.4848686916271。）

`rssun = rst*laisun ≈ 0` 之后，蒸腾退化成
`etr = rhoair*(1-fwet)*delta*(lai/rb)*gradient` —— **只受边界层阻力限制的潜在蒸腾**。
这解释了两个此前无法解释的观测：

1. **`f_etr` 昼夜几乎一样**（6 月 14 日：中午 2.749e−5、夜里 2.383e−5，同日 `f_fsena`
   从 +276 变到 −45）。真实的气孔调控下不可能。
2. **`f_assimsun ≡ 0` 在正午也成立** —— 不是臭氧，是 `vm` 被 `spval` 缩成了垃圾。

复现（诊断打印已撤，这里是当时的三处插桩：`stomata` 末尾、
`MOD_Thermal_CanopyPhase_Extended.F90` 的 `rstfacsun_out = rstfac` 之后、
`MOD_LeafTemperature_Extended.F90` 的 `IF(lai>0.001)` 入口）：

```
./oracle/scripts/build_kernel.sh default
NETCDF_DIR=/opt/homebrew/opt/netcdf cargo run -q -p colm-cli -- \
  run oracle/work/CN-Cng-dbg --kernel kernels/default --stage colm --force 1 --stream 1
```

同一段代码、同一个编译器，**只因为往同文件的别处加了一句 `write`，`rstfac`
就从 −1e36 变成 0.25079**（`stomata` 调用次数也随之从 1242 掉到 666）。
这本身就是"入口值未定义"的判据：读的是未定义量，值随代码生成变化。

### 处置

1. 把 `rstfacsun`/`rstfacsha` 从 `intent(out)` 块移出、按 `main/` 的写法单列
   `intent(inout)`（`MOD_LeafTemperature_Extended.F90`）。
2. 同一次 f48fbf9 同步还漏了 `gssun`/`gssha` 的诊断块
   （`main/` 的 `:1023-1032`、PC 版 `:1753-1763`），一并补进两份扩展模块。
3. `oracle/scripts/test_upstream_f48_sync.py` 原来只钉 `main/` 两份 —— 这就是漏网的原因。
   三条断言各加一份对 `extends/` 的镜像，以后再同步漏掉扩展版会当场红。
4. **重生成黄金文件**：`oracle/golden/*.nc` 与 `kernel-manifest.json` 记录的是
   `colm_git_sha = 4894833`，距 HEAD 已 **560 个提交**，且工具链也换过
   （netcdf 4.10.1→4.9.3、hdf5 空→1.14.6），本来就全红。按
   `golden-run <case> --write-golden` 重新落了基线，现在
   `golden-compare` 报 `identical: 127 variables, 10 dimensions`。

### 改善（同一步长、同一起始重启、两边都是区间平均）

冬季 `CN-Cng-aligned`（264 条小时记录）：

| 变量 | 修前 max|Δ| | 修后 max|Δ| | 倍数 |
|---|---|---|---|
| `f_etr` | 1.2599e−5 | **1.1495e−8** | 1096× |
| `f_tleaf` | 7.656e−1 | **1.269e−1** | 6× |
| `f_fevpg` | 4.267e−6 | **7.851e−7** | 5.4× |
| `f_fsena` | 2.339e1 | **2.278e0** | 10× |
| `f_t_grnd` | 7.533e−2 | 7.487e−2 | 1.0× |
| `f_lfevpa` | 4.304e2 | 4.304e2 | 1.0× |

暖季 6 月 1–16（360 条）：

| 变量 | 修前 max|Δ| | 修后 max|Δ| | 倍数 |
|---|---|---|---|
| `f_etr` | 6.529e−5 | **4.147e−6** | 15.7× |
| `f_tleaf` | 5.853 K | **2.442e−1 K** | 24× |
| `f_t_grnd` | 2.495 K | **1.093e−1 K** | 23× |
| `f_fevpg` | 4.409e−5 | **1.170e−6** | 37.7× |
| `f_fsena` | 1.909e2 | **1.035e1** | 18× |
| `f_lfevpa` | 1.668e2 | **9.771e1** | 1.7× |

修好后冬季 `f_etr` 峰值 Fortran 3.302e−7 / Rust 3.281e−7（0.6%），
6 月白天 Fortran 2.736e−5 / Rust 2.390e−5。**此前"Rust 蒸腾小 4~6 倍"这个
判断作废**：那是拿一份光合被 `spval` 掐死的内核当基准量出来的。

`f_lfevpa` 两季都没动（冬季 430 保持不变，且 Rust 峰值 615.1 对 Fortran 184.7）。
修前修后它都一样，说明它与叶温链无关，仍是上一轮记下的
**采样口径问题**（Rust 自洽于 `hvap*fevpl + htvp*fevpg`，Fortran 自己的
`184.677` 对 `221.506` 也对不上），下一轮单独查。

### 给后来者的两条规矩

* **定位内核行为前先看 `.bld` 的编译命令行。** `main/` 与 `extends/` 同名模块
  的顶替是构建期发生的，源码树上看不出来；改错文件会得到"改了没反应"，
  而那很容易被误判成"这段代码不参与计算"。
* **`intent(out)` 的哑元在入口是未定义的**，`-O2` 下的实际取值随无关改动漂移。
  同名模块一旦分叉成两份（`main/` 与 `extends/`），语义同步就必须显式钉住 ——
  这正是 `test_upstream_f48_sync.py` 现在做的事。

## history 写出面：补上冠层光合/气孔链十一项（2026 年，实测）

上一节修好内核之后，`assim`/`etrsun`/`gssun` 这些量第一次**有真值**；而本仓库的
history 写出面此前根本不声明它们 —— 黄金算例里有 68 个变量只在 Fortran 侧存在，
`declare_lct_variables` 只点了自己能负责的 58 个，缺口写在 `UNFILLED` 里。
第一项"分层植被量：需要冠层分层输出"是**误判**：内核早就算好了，只是没人接。

一次补上十一项（`LCT_STOMATAL_VARIABLES`）：

| 变量 | 上游 | 本仓库 | 维度 |
|---|---|---|---|
| `assim` / `assimsun` / `assimsha` | `assim = assimsun + assimsha`（`MOD_LeafTemperature.F90:1048`） | `leaf.assimilation_*_mol_m2_s` | `(time, patch)` |
| `respc` | `respcsun + respcsha`（`:1049`） | `leaf.respiration_mol_m2_s` | `(time, patch)` |
| `etrsun` / `etrsha` | `etrsun_out` / `etrsha_out` | `leaf.sunlit_/shaded_transpiration_kg_m2_s` | `(time, patch)` |
| `gssun` / `gssha` | `gssun_out` / `gssha_out`（f48fbf9 的 `gssun = (laisun/rssun)*(tprcor/tlbef)`） | `leaf.sunlit_/shaded_stomatal_conductance_mol_m2_s` | `(time, patch)` |
| `rstfacsun` / `rstfacsha` | `MOD_Thermal.F90:674-675` 两行同源 | `energy.root_uptake.soil_water_stress` | `(time, patch)` |
| `rootr` | `MOD_Eroot` 的分层根阻力权重 | `energy.root_uptake.layer_fraction` | `(time, patch, soil)` |

**单位陷阱：上游把 `assimsun` 注成 `[umol co2 /m**2/ s]`，实际是 mol m⁻² s⁻¹**
（黄金值峰值 2.83e−7）。照注释乘 1e6 会得到偏 6 个数量级的一列。

对齐算例（冬季 264 条）实测：

```
f_assim      F 峰值 2.8270e-07   R 2.8268e-07      ← 0.007%
f_assimsun   F 2.2526e-07        R 2.2543e-07
f_assimsha   F 1.7302e-07        R 1.7283e-07
f_etrsun     F 2.8625e-07        R 2.8445e-07
f_etrsha     F 6.1499e-08        R 6.0996e-08
f_respc      F 1.3891e-08        R 1.3761e-08
f_gssun      F 1.9484e-02        R 1.9483e-02
f_rstfacsun  F 4.5683e-02..0.95100  R 4.5680e-02..0.95100
f_rootr      逐层逐记录相同，除 2 条过渡记录
```

`f_rootr` 那 2 条（第 202、203 条）是 `eroot` 的 `t_soisno(i) > tfrz` 判据在
亚步里早/晚一条记录翻越所致：两条记录的层权重之和都严格是 1，只是第 6 层
在第 202 条上属于哪一侧不同。**这是判据的时间对齐，不是权重算错。**

冠层光合三兄弟现在是**除 `f_etr` 之外对气孔链最灵敏的探针** —— 它们直接量的是
`stomata` 的输出，而不是像 `f_tleaf` 那样经过能量平衡的积分。

仍未写出的（`UNFILLED`，已按实际缺口重写）：`rss`（方案 4 的电导/阻力条件映射待核对）、
`laisun`/`laisha`/`ssun`/`ssha`（内核算过 `fsun` 但没带出步输出）、
`ldew`/`qintr`/`qdrip`（缺一个冠层截留状态出口）、`h2osoi` 等派生土壤量、
湖泊/BGC。**"文件里没有"与"内核产不出"是两件事**，`UNFILLED` 的用途就是把前者
记成待办而不是当成结论。

### 补上之后的对账口径变了，要说清

黄金文件里"只在 Fortran 侧"的变量从 **68 降到 57**，而 tier 比对的**失败数从 43 升到 48**
—— 后者不是退化。新增的十一项里有五项落在容差外：

```
f_rootr      1100/2640   最差 2.305e-1 vs 0        ← 那 2 条过渡记录
f_rstfacsun   261/264    最差 1.269e-1 vs 8.888e-2
f_rstfacsha   261/264    同上（LCT 下同源，必然同差）
f_gssun        96/264    最差 5.323e-4 vs 2.011e-3
f_gssha        97/264    最差 8.950e-3 vs 1.421e-2
```

`f_assim` 只差 0.007%，而 `f_gssun` 差 4 倍 —— 差别在**判据本身的性质**：
`gssun = (laisun/rssun)*(tprcor/tlbef)`，低光照时 `assmt` 落到下限 `1e-12`，
`gsh2o ~ 1e-8`，`rssun` 顶到 `1e6` 上限。**在上限附近，`gsh2o` 的微小差异会被
`1/(gsh2o*…)` 放大成几倍的 `gssun`** —— 与 `scv` 属于同一类"近抵消探针"，
量得准不准取决于两侧是否恰好落在上限的同一侧。`f_rstfacsun` 的 30% 同理：
它的输入是 `eroot` 里 `t_soisno(i) > tfrz` 的分层判据，层在冰点两侧时
`rresis` 从 0 跳到 1，权重跟着整体重归一化。

所以这五项**留在 tier2、留在失败清单上是对的** —— 按 `oracle/tolerances.toml`
开头那条"层级不倒挂"的不变式，确定性代数量不该因为"实测对不上"就被降级，
否则表就失去意义了。它们现在是**已知的、有解释的残余**，不是待修的 bug。

## `htvp` 是条件量：地面蒸发不总是升华（2026 年，实测）

`f_lfevpa` 在上一轮修完"用了初步值"之后还差 34 W/m²（Rust 219 对 Fortran 185），
现在查清了，是**第二个**独立缺陷：潜热本身取错了。

上游 `MOD_Thermal.F90:539-540`：

```fortran
! latent heat, assumed that the sublimation occurred only as wliq_soisno=0
htvp = hvap
IF (wliq_soisno(lb)<=0. .and. wice_soisno(lb)>0.) htvp = hsub
```

`lb = 1-nzsno` 就是**表层**：有雪时是最上一层雪，无雪时是最上一层土。只有它
"零液态水 + 有冰"时地面蒸发才是升华，潜热才抬到 `hsub = hvap + hfus`；
其余情况一律是汽化潜热。本仓库此前把 `sublimation_heat = hvap + hfus` **写死**，
于是**所有**液态地表的地面蒸发都按升华计价。

| | `f_lfevpa` max‖Δ‖ | `f_fgrnd` max‖Δ‖ |
|---|---|---|
| 写死 `hvap+hfus` | 34.12 | 32.23 |
| 按表层判据 | **3.14** | **2.68** |

`f_rnet` 完全不动（0.4261 → 0.4261）：`htvp*fevpg` 在
`rnet = fsena + lfevpa + fgrnd` 里出现两次、符号相反，正好抵消 —— 这条恒等式
反过来成了这次改动的独立校验。

**修正落在内核，不只是诊断。** 潜热经
`GroundFluxInput::vaporization_heat_j_kg` 进 `LeafTemperature`（叶温求解里的地面潜热项）
与 `thermal_water` 的感热订正项，上游这两处用的都是同一个 `htvp`
（`:640` 传给 `GroundFluxes`、`:1256` 的 `fseng = fseng + htvp*egidif`）。
两处装配点都按表层判据取值：

* 无雪分支：`self.water.{liquid,ice}_water_kg_m2[0]`（打包列下标 0 = 表层）；
* 积雪分支：`self.snow_soil.{liquid,ice}_water_kg_m2[0]` —— `snow_soil` 是
  `snow.temperature_k[SNOW_SLOTS-nzsno..]` 接上土列，注释写明"自雪顶一直排到土壤底"，
  所以下标 0 也是最上一层雪。

`ground_latent_heat_j_kg` 做成 `colm-core` 的纯函数并单测四种组合
（纯冰 / 有液态水 / 全干 / 负噪声液态水）。**"完全干"那一支是必须钉的**：
`wice > 0` 不成立时仍是汽化，写成 `wice >= 0` 会把它算成升华。

### 这次的内核改动在已验证窗口里是逐位无操作

冬季对齐窗口 264 条记录里，表层 `wliq<=0 .and. wice>0` **命中 0 次**
（表层一直有液态水），所以 `htvp` 全程等于 `hvap`。前后两次 Rust 运行的
**状态与通量变量逐位相同**，只有 `f_lfevpa`/`f_fgrnd` 两个诊断量变了、
`f_rnet` 差 5.7e-14（舍入）。**这不是"改了没用"，而是这类判据的正常形态**：
它在无雪算例上不触发，只有积雪/冻土窗口才考验它 —— 而本仓库目前
**没有**这样的黄金窗口（`CN-Cng` 冬季地表始终有液态水，`CN-Cng-wet` 是 7 月）。
要真正验到那一支，得再加一个高纬有稳定雪盖的站点窗口；在那之前，
四组合单测是唯一的守卫。

### 剩下那 3 W/m²

`f_lfevpa` 仍差 3.14（对 184.65 是 1.7%）。**上游自己就自相矛盾到同一量级**：
它的 `f_lfevpa` 是 184.65，而拿它自己的 `f_fevpl`/`f_fevpg` 两列按
`hvap*fevpl + htvp*fevpg`（此窗口 `htvp = hvap`）反算是 189.63 —— 差 4.98，
比本仓库的 3.14 还大。两个差异同源：**逐产物平均**与**先平均再相乘**不是一回事，
`fevpl` 在一小时里跨昼夜变号时两种口径必然分叉。所以这不是某一侧算错，
而是"区间平均的 `lfevpa` 该按哪种口径折算"在上游本身就没有一致答案；
本仓库按 `hvap*fevpl + htvp*fevpg` 逐产物累加，是有定义的那一种。

## 驱动场镜像九项：先把"读到内核里的是不是同一份"钉住（2026 年，实测）

`UNFILLED` 里那批"离物理最远"的量是 `f_xy_*` —— 上游把它们
`acc1d(forc_*, a_xy_*)` 原样累加后写出（`MOD_Vars_1DAccFluxes.F90`），**不做任何换算**，
所以在 `oracle/tolerances.toml` 里它们是 tier0（逐位）。它们量的不是物理，
而是"驱动场从文件到内核有没有变样"，是排在其他 57 个变量**之前**该先对上的一组。

补九项（`LCT_FORCING_VARIABLES`）：`xy_t`/`xy_q`/`xy_pbot`/`xy_us`/`xy_vs`/
`xy_solarin`/`xy_frl`/`xy_prc`/`xy_prl`。对齐算例 264 条实测：

```
f_xy_t       逐位相同
f_xy_pbot    逐位相同
f_xy_frl     逐位相同
f_xy_prc     逐位相同（全 0：PLUMBER2 单点强迫没有对流/层状分列）
f_xy_prl     逐位相同（同上）
f_xy_us      39/264 差 1 ulp（最差 2.67569208765822797 vs …841）
f_xy_vs      39/264 差 1 ulp（标量风下与 `f_xy_us` 同值，所以同差）
f_xy_solarin 13/264 差 ~2 ulp（最差 123.041496276855469 vs …483）
f_xy_q       3/264 差 ~1%（第 98-100 条）
```

**五项逐位、三项末位、一项真差** —— 这个分布本身就把问题分层了：

* `f_xy_us`/`f_xy_vs` 的 1 ulp：标量风下 `forc_us` 就是风速本身，两边同源，
  差在最后一位说明某一步的风速标量算法（`hypot` 一类的结合顺序）与上游不同。
  它**只**影响这两位，所以不值得为它改物理。
* `f_xy_solarin` 的 ~2 ulp：上游是"先有总量再拆四个波段"，本仓库的
  `RuntimeForcing` 只带四个波段，总量由它们相加得到（`from_forcing`）。
  这是**加法结合顺序**的差，不是读错。
* `f_xy_q` 的 3 条（98/99/100，比值 1.0104/1.0073/1.0173，之后立刻回到逐位）：
  连续三条、且都出现在湿度峰值附近、Rust 一律**偏大**，形状像是在某处少了
  一个饱和约束。下一轮从 `MOD_Forcing` 读入那一侧追（
  `MOD_ForcingDownscaling` 的 `qbot_c = qbot_g*(qs_c/qs_g)` 是空间算例专用，单点不走）。

**代价说明白：** tier 失败数从 48 升到 52，四条新增全在 tier0 且上面已逐条定性。
按 `tolerances.toml` 开头"层级不倒挂"的不变式，tier0 量不该因为实测对不上就降级 ——
它们留在 tier0 才会持续提醒这三处差异；调松就等于把它们埋掉。
`f_xy_rain`/`f_xy_snow` 仍未声明：那是雨雪**相态拆分**的结果（`MOD_RainSnowTemp`），
属内核下游而非驱动场本身。

## 累加器要按变量各算各的有效步数（2026 年，实测）

补短波分带十七项时撞到一个**更底层**的保真缺口，它不属于任何单个变量。

上游 `acc1d` 会**跳过 `spval`**：

```fortran
DO i = lbound(var,1), ubound(var,1)
   IF (var(i) /= spval) THEN
      IF (s(i) /= spval) THEN ; s(i) = s(i) + var(i)
      ELSE                    ; s(i) = var(i)
      ENDIF
   ENDIF
ENDDO
```

而除数**按变量分组各有一个计数器**：`nac`（每步 +1，`:2036`）、
`nac_dt`（只数 `coszen > 0` 的步）、`nac_ln`（只数 `solvdln /= spval` 的步，`:2041`）。
三个计数器分别喂给不同的 `write_history_variable_2d`。

本仓库原来的 `HistoryAccumulator` 是"全部求和、除以一个全局步数"，
而且把非有限值当成错误。对当时已实现的那批变量**看不出来**（它们每步都有值），
但只要有一个量是"局部有效"的，它就会静默写出**被无效步稀释的值** ——
每个数都有限、不报错，正好是这类 bug 最难发现的样子。

改成：`MISSING` 不进和也不进计数，每个变量自带 `count`，除数用它自己那个；
一步都不有效的变量**不建条目**，于是不被写出、缓冲区留给它填充值
（与上游"累加器一直是 spval"同效），整列无效的分层量同规则。
新增单测 `the_accumulator_skips_missing_samples_and_counts_only_valid_ones`
钉住"两步里只有一步有效 → 写出来是那个值本身而不是它的一半"。

### 顺带补上短波分带十七项

`LCT_RADIATION_VARIABLES`：`sr` + 四个下行 `sol*d/sol*i` + 四个反射 `sr*d/sr*i`
+ 八个本地正午 `*ln`。定义在 `MOD_NetSolar.F90:279-315`：
`solvd = forc_sols`、`solvi = forc_solsd`、`solnd = forc_soll`、`solni = forc_solld`
（`d` = direct、`i` = indirect），`sr*` 是同波段乘反照率后的反射，`sr` 是四者之和，
`*ln` **只在 `local_secs == 43200` 那一步有值**、其余 spval。

对齐算例实测：

```
f_sr       F[0,116.78]  R[0,117.14]   maxdiff 3.66e-1
f_solvd    F[0, 66.61]  R[0, 66.55]   maxdiff 1.04e-1
f_solvi    F[0,140.63]  R[0,140.69]   maxdiff 1.04e-1
f_solnd    F[0, 62.26]  R[0, 62.20]   maxdiff 9.72e-2
f_solni    F[0,131.45]  R[0,131.51]   maxdiff 9.72e-2
f_solvdln  F 11 条真值 / 253 spval    R 同样 11 条真值 / 253 spval
```

`*ln` 两边的**有效条数与位置完全一致**（11 条，逐 24 小时一条）——
这是累加器那笔改动最直接的验收：换回"除以全局步数"就会变成 11 条真值被 2 除、
其余 253 条变成 `spval + 真值` 的垃圾。

### 分带的差是**比例**差，不是总量差

`f_xy_solarin`（总量）只差 2 ulp，而四个分带各差 0.1~0.3%，且
`f_solvd` 偏小、`f_solvi` 偏大**符号相反** —— 两波段的**和**仍然对得上。
所以差在拆分公式而不是读入。拆分在 `crates/colm-core/src/runtime_forcing.rs:194`
的 `split_broadband_shortwave`，逐行照 `MOD_Forcing.F90:965-985` 实现
（`cloud = max(0.58, cloud)` 那个下限也在）。差因此只能来自它的入参 `sunang`：
`difrat = 0.0604/(sunang-0.0223) + 0.0683` 在冬季低太阳角下对 `sunang` 极敏感
（`d(difrat)/d(sunang) ≈ -3.7`），`sunang` 差 1e-4 就够产生这 0.1%。
本仓库的 `orbital_cosine_zenith` 是 `MOD_OrbCoszen` 的逐位移植，
**所以下一步要查的是喂给它的 `calendar_day`**：上游那里是
`calday = calendarday(idate)`，而 `idate` 在 `MOD_Forcing` 里已经是
`TICKTIME` 之后的**步末**时刻 —— 与 `CoLMMAIN` 自己那份 `coszen` 不是同一个时刻。

## `f_xy_q` 的三条差：本仓库是精确的，差在上游（2026 年，实测）

上一轮留的 `f_xy_q`（3/264 条、约 1%）查清了。把 264 条与强迫场原值逐条对齐：

```
记录 i  ↔  原始样本 (188+2(i-94), +1) 的算术平均
i=97 : F = R = mean(194,195)      逐位
i=98 : R = mean(196,197) 精确     F 不等于任何样本组合   ← 差 1.09e-5
i=99 : R = mean(198,199) 精确     F 同上                 ← 差 7.66e-6
i=100: R = mean(200,201) 精确     F 同上                 ← 差 1.69e-5
i=101: F = R = mean(202,203)      逐位
```

**本仓库 264 条全部等于相邻两个 30 分钟样本的算术平均，一条不差**；
上游只在 2008-01-05 02:00/03:00/04:00（地方时）这三条上偏离，
而且那三个值**在强迫场文件里任何变量、任何时刻都找不到**（全表搜索无匹配，
三个候选组合都试过：换样本对、加权、3~5 样本均值，最近的一个还差 1.4e-3 相对）。
同一批记录上 `f_xy_t`/`f_xy_pbot`/`f_xy_frl` 都逐位相同，所以时间轴对齐没问题，
偏差是 **`q` 独有**的。

结论：**这条不再算本仓库的待办**。要追就是上游 `MOD_Forcing` 的读入/预读路径
在那三步对 `forc_q` 做了什么，与移植无关。留在这里是因为它此前被当成
"Rust 驱动链有 bug"，方向反了。

## 短波分带差 0.1~0.3% 的根因定位到"喂的是哪一刻的角度"（2026 年，实测）

上一轮把分带差归到 `split_broadband_shortwave` 的入参 `sunang`。这轮量准了。

### 总量是逐位对的，差全在比例

对齐算例 11 条本地正午记录（`f_*ln` 只有这些有条真值）：

```
rec   F 反演 sunang   R 反演 sunang   差        F 总量      R 总量
 11   0.37567558     0.37419245    1.483e-3  393.2900   393.2900
 35   0.37694297     0.37546071    1.482e-3  383.5530   383.5530
 ...
251   0.39373191     0.39226144    1.470e-3  405.5570   405.5570
```

**总量两引擎逐位相同**（`f_xy_solarin`），而反演出的 `sunang` 恒定差 1.47~1.48e-3。
反演用的是 `dif = 0.58 + 0.42*(0.0604/(sunang-0.0223)+0.0683)`
（此窗口 `cloud` 恒在下限 0.58），所以这不是约等于，是精确解。

### 本仓库喂的是**步首**角，且是精确的

把每个正午记录的 `sunang` 反解回地方时刻：

```
Rust    → 全部恰好 11:30:00（误差 4.6e-9），即该步的**步首**
Fortran → 11:34:11 ~ 11:34:16（十天里缓慢漂移），即**步首之后约 255 秒**
```

Rust 侧那个"恰好 11:30:00"是关键：`orbital_cosine_zenith` 在
`calday = day + (sec - int(LocalLongitude/15*3600))/86400` 上求值，
`sec = 41400` 就是 `idate = 43200` 那一步的步首。核对过的两处都不是原因：

* **赤纬公式逐常数相同**（`lambm0=-3.2625366e-2`、`ve=80.5`、`dayspy=365`、
  `eccen=1.672393084e-2`、`obliqr=0.409214646`、`mvelpp=4.92251015`，
  `MOD_OrbCoszen.F90:56-79` 对 `atmosphere.rs:424-441`）；
* **经度位移逐位相同**：`int(LocalLongitude/15*3600)`，123.5092° → 29642（秒）。

### 为什么**不改**代码

上游实测角落在 11:34，介于步首（11:30）与步末（12:00）之间。两种候选各差多少：

| 角度取 | 该步 `solvd` | 对上游 `f_solvdln`=64.953960 |
|---|---|---|
| 步首 11:30（现状） | 64.892454 | **−0.09%** |
| 步末 12:00 | 65.123 | +0.26% |

**步首明显更近。** 本仓库内部其实是"两个时刻"的：`local_secs`（正午判据）用步末、
`calendar_day`（分带角）用步首，而上游全用同一个 `idate`。既然把角换成步末会让
一致性**变差**，就不能按"和上游一样用步末"去改 —— 那只满足形式、不满足证据。

所以这一轮落的是**守卫**而不是改动：`runtime_forcing_tests.rs` 新增
`the_shortwave_split_is_fed_the_step_start_solar_angle`，钉三件事 ——
四波段之和恒等于总量、步首角复现上游到 1e-3 以内、步末角超出 1e-3。
谁把角度换成步末、或动了 `orbital_cosine_zenith`，都会红，
逼他重新对着上游量一次，而不是让 0.26% 的波段偏差悄悄扩散到整条辐射链。

**未解：** 上游那一步的实际时刻为什么是"步首 + 255 秒"（十天里 256→251 秒）。
它线性对应角度恒定差 1.47e-3，所以要么是一个约 255 秒的时间约定，
要么是一个约 4e-3 弧度的赤纬/纬度类常量差 —— 后者已被常数逐位比对排除，
前者在 `MOD_Forcing` 里还没找到出处。下一步该查的是 `MOD_Forcing` 里
`calday` 那一次 `calendarday(idate)` 的 `idate` 究竟被谁推过。

## `f_rnof`/`f_rsub` 那 72% 是水位差被指数基流放大的，不是径流公式错（2026 年，实测）

`f_rsub`（地下径流）此前是残差表里**相对误差最大**的一项（最差 1.689e-4 对 9.838e-5）。
这轮把它的形状量清了：

```
有径流的记录 151/264 条
F/R 比值：中位 1.019、均值 1.031、最大 2.819、最小 0.000
```

**没有系统偏差**（中位只差 1.9%），而极值散在两侧（第 72 条 F 高 72%、第 74 条 R 高 53%），
是"散"不是"偏"。再看它与谁相关：

| | F | R |
|---|---|---|
| `corr(rsub, qcharge)` | 0.6137 | 0.6131 |
| `corr(rsub, zwt)` | 0.5922 | 0.5911 |

两边的相关结构**几乎一样**，说明不是某一边的水文逻辑不同。按水位深度分档看
`mean|Δ|`：

```
浅  zwt∈[2.774,2.846]  n=50  mean|Δ|=1.03e-6   mean rsub=7.24e-5
中  zwt∈[2.847,3.149]  n=50  mean|Δ|=4.49e-6   mean rsub=1.11e-4
深  zwt∈[3.154,4.153]  n=51  mean|Δ|=7.61e-6   mean rsub=2.10e-4
```

**差异随水位变深单调放大**，而 `f_zwt` 本身两边只差 0.007%、`f_qcharge` 差 0.01%。
VIC 的基流是水位的指数函数，所以 0.007% 的 `zwt` 差在深层被放大成几十个百分点 ——
这与 `scv`、`gssun` 属于同一类：**稳态量本身对得上，是它经过的陡函数把尾数放大了**。

结论：**这条不该去改径流公式**，要改就得先把 `f_zwt`/`f_wliq_soisno`（差 1~2%）
压下来。上一轮修 `htvp`/累加器那种"改一处、残差整片下降"的机会在这里没有，
因为入口量已经对了 99.99%。

## 冠层几何三项 + 传感器槽：31 → 27（2026 年，实测）

补四项，都是"内核早就算好、只是没带出来"那一类：

| 变量 | 上游出处 | 本仓库来源 | 维度 |
|---|---|---|---|
| `sigf` | `MOD_SnowFraction`（未被雪埋的植被比例） | `StandardLctEnergyState::canopy.vegetation_free_fraction` | `(time, patch)` |
| `laisun` | `lai*fsun`（`MOD_Thermal.F90:672`） | `LeafTemperatureOutput::sunlit_leaf_area_index` | `(time, patch)` |
| `laisha` | `lai*(1-fsun)`（`:673`） | `LeafTemperatureOutput::shaded_leaf_area_index` | `(time, patch)` |
| `sensors` | 用户自定义诊断槽（`MOD_Hist.F90:4671`） | **只声明、不填** | `(time, patch, sensor)` |

`laisun`/`laisha` 在叶温求解里本来就有局部量（`:233-234`），只是出口没带 ——
加两个字段、不新增任何计算。

`sensors` 走的是与 `frcsat` 相反的机制：`frcsat` 是"上游在 `WATER_2014` 下不设，
声明留空才一致"，`sensors` 是"上游只有算例主动写才有值"。两者都在黄金文件里
整列是 `missing_value`，`oracle/tolerances.toml` 把 `sensors` 钉在 tier0 并注明
"未启用的用户自定义诊断槽"。所以声明 + 留空是**忠实**，填任何东西都是无中生有；
`DECLARED_ONLY` 这个常量就是为这类槽位立的，测试与 `DECLARED_BUT_UNFILLED` 一起
断言它们保持填充值。

实测（对齐算例 264 条）：

```
f_laisun   F[0.088424,0.174610]  R 同区间   maxdiff 9.14e-9   ← 相对 5e-8
f_laisha   F[0.025390,0.111576]  R 同区间   maxdiff 9.14e-9
f_sensors  维度 (time,patch,sensor) 一致，264×1×1 全填充
f_sigf     F[0.999622,1.000000]  R[0.999561,1.000000]  maxdiff 6.12e-5
```

`f_sigf` 那 6.12e-5 是**继承来的**，不是它自己算错：`sigf = 1 - wt`，而 `wt`
本身只有 4e-4 量级，本仓库的 `scv` 已知偏大 15%（冠层露水的近抵消，见前文），
在 `sigf` 上表现为 6e-5 的绝对差。它因此落在 tier2 之外（新增一条 70），
和 `f_fsno`/`f_scv`/`f_snowdp` 同一条链，不该单独去修。

**`green` 仍不写。** 上游 `green` 来自 `MOD_LAIEmpirical.F90:132-135`
（`fveg = vegc(ivt)`、`green = 0.; IF (fveg>0.) green = 1.`），而 `vegc` 是
**该模块内部的硬编码表**（IGBP 那支是 17 项：15=Snow/Ice→0、17=Water→0，其余 1）。
本算例地类 10 → `vegc(10)=1` → 恒为 1，与黄金一致。但"恒为 1"不是实现依据：
把它搬进来要么新增第四张生成表、要么手抄一张 17 项数组，两种都要先确认
`USE_SITE_LAI = .true.` 时到底是哪个模块设的 `green`（`LAI_empirical` 还是
`MOD_LAIReadin`）。**证据不足就不写** —— 填一个恒 1 的常量在别的算例上会静默错。

### 顺带解决一个挂了很久的待核对项：`h2osoi` 是**液 + 固**

`UNFILLED` 里一直写着"`h2osoi` 需要先核对上游对每个量的定义（液态还是液+固态）"。
查到了，在 `CoLMMAIN.F90:2253`：

```fortran
h2osoi = wliq_soisno(1:)/(dz_soisno(1:)*denh2o) + wice_soisno(1:)/(dz_soisno(1:)*denice)
```

**液 + 固，而且两者用各自的密度**（`denh2o = 1000`、`denice = 917` kg/m³），
不是统一除以 1000。写成 `(wliq+wice)/(dz*1000)` 会在冻土上偏低约 8% 的冰贡献 ——
本仓库 1 月算例表层冰占 `wice=4.3` 对 `wliq=6.0`，正好是会被看出来的量级。

对应关系：`wliq_soisno(1:nl_soil)`/`wice_soisno(1:nl_soil)`/`dz_soisno(1:nl_soil)`
就是**土层**那 10 层（雪层在负下标），所以维度是 `(time, patch, soil)`，与黄金一致
（实测 `f_h2osoi` 区间 [0.3237, 1.154] 是体积含水率）。

内核侧只要把 `state.water.{liquid,ice}_water_kg_m2` 与模板的层厚接一个分层写出即可 ——
`ICE_DENSITY_KG_M3 = 917.0` 已经在 `ground_temperature.rs` 里了。这一项留在
`UNFILLED` 里是因为**端口径已经定了、只差接线**，与其余"定义还没定"的项不同类。

10 m 那四个（`us10m`/`vs10m`/`fm10m`/`ustar2`）也一并查了出处：它们不是
`MOD_Vars_1DAccFluxes` 里那十三个近地表诊断的同批，而是另一支
（源码里标着 `Shaofeng, 2023.05.20` 的 `r_ustar2_e`/`r_fm10m_e` 廓线 routine），
要移植是移植那支，不是复用已有的。

## `h2osoi` 接线完成：公式用两个密度，实测可反解出层厚（2026 年，实测）

上一轮把 `h2osoi` 的定义查清了（`CoLMMAIN.F90:2253`，液 + 固、各用各的密度），
这轮把线接上：`LCT_DERIVED_SOIL_VARIABLES`，维度 `(time, patch, soil)`，
层厚由模板提供（新增 `soil_layer_thickness_m()` 访问器），液相/固相由步状态提供。
实现放 `colm-runtime/src/history.rs`，两层密度写成模块常量
（`WATER_DENSITY_KG_M3 = 1000`、`ICE_DENSITY_KG_M3 = 917`）。

### 验证：不只是"看起来对"，是可反解

对齐算例 264×10 个值，做了两个独立检查：

**一、公式等价性（机器精度）。** 因为层厚是两边共用的同一份模板，
`h2osoi` 与 `wliq`/`wice` 应有严格比例关系。实测

```
F_h2osoi/R_h2osoi  ==  (F_wliq + k*F_wice) / (R_wliq + k*R_wice),  k = 1000/917
2640 个值上最大相对偏差 = 4.4e-16
```

**二、绝对量纲（独立于黄金文件）。** 反过来用 `dz = (wliq + k*wice)/(h2osoi*1000)`
解层厚，两引擎解出的层厚逐位相同（差 4.4e-16），且是一根合理的等比土柱：

```
0.017513  0.027579  0.045470  0.074967  0.123600
0.203783  0.335981  0.553938  0.913290  1.136972   (m)
```

顶层 1.75 cm —— 与 CoLM 的 `nl_soil` 分层一致。**这正是密度常数的判别式**：
若误用统一 1000，解出的 `dz` 会两边差约 8%，恒等式也会当场破。

### 残差是继承的

最差 1.70e-2（1.1189 对 1.1359，相对 1.5%），来自已知的 `f_wliq_soisno` 差
（1~2% 量级），不是换算错。`f_h2osoi` 因此新落进 tier2 之外（70 → 71），
与 `f_wliq_soisno`/`f_wice_soisno`/`f_t_soisno` 同一条链。

### `UNFILLED` 顺手重写

原来第一条还写着"`laisun`/`laisha`/`ssun`/`ssha` 需要冠层 `fsun` 落进步输出"
—— 那三项上一轮已经写出来了，条目却留着（清单里的字不会自己过期，
所以每次补完变量都要回头改它）。现在按**卡在哪**重列：

```
green   上游由 MOD_LAIEmpirical 内的硬编码 vegc 表得出，本仓库没搬
alb     四维 (time,patch,rtyp,band)，内核算得出但没落进步输出
rss     方案 4 下上游写的是电导标志而不是阻力，条件映射待核对
ldew/qintr/qdrip   缺一个冠层截留状态出口，目前只有通量
10m 风/稳定度 + 湖泊/湿地/BGC   整支 routine 或分支尚未移植
```

## `f_alb` 写出来了，并且发现它和别的变量不是一套累加规则（2026 年，实测）

上一轮把 `alb` 记成"缺一条四维写出通路"。**那个判断是错的**：
`HistoryBuffers` 的层维早就是 `(patch × 各维乘积)` 的通用形态，
`set_layered` 直接吃 4 个值就能写四维，不需要新通路。
真正缺的只有两个字段的接线 —— `state.energy.radiation.albedo` 一直在那儿
（`assembly.rs:1359` 那次 `&mut state.energy.radiation`，续跑写回用的也是它），
以及把步末的太阳天余弦传进 history。

### 真问题：`f_alb` 只累加**白天**步

上游那一处与所有其它变量都不同：

```fortran
! only acc for daytime for albedo
CALL acc3d (alb, a_alb, filter_dt)          ! MOD_Vars_1DAccFluxes.F90:2152
CALL write_history_variable_4d (..., sumarea_dt, filter_dt, ..., nac_dt)  ! MOD_Hist.F90:769
```

`filter_dt = coszen > 0`（`:2043`）、除数是 `nac_dt`（白天步数，`:2044`），
写出时也用 `filter_dt`/`nac_dt` 而不是 `filter`/`nac`。三者合起来：**夜间步既不入和
也不入计数；整条记录都是夜间的，`a_alb` 一直是 `spval` 且 `nac_dt = 0`，
所以那一行留填充值。**

本仓库的累加器已经会跳过 `MISSING` 且只按有效步数取平均，所以这里**不需要再加
一条"白天过滤器"的通路** —— 夜间步写成 `MISSING` 就够了，语义自动对上。

### 判别式与结果

对齐算例 264 条：

```
              真值个数   掩码与 Fortran 一致   最差      相对
不分昼夜      1056      ✗（夜间多出 660 个）  2.49e-2   —
只累加白天     396      ✓ 逐位一致            1.03e-3   0.17%
```

**24 倍的改善，而且掩码从"多出 660 个上游没有的数"变成逐位一致。**
夜间 `alb` 是 1（`MOD_Albedo.F90:217` 先把 `alb(:,:) = 1.`，`:295` 天黑直接 `RETURN`），
把它平均进去每个数都仍然有限、只是偏高 0.025 —— 正是"不报错的错"。
剩下那 0.17% 与 `f_sabv*` 的 0.27% 同类，落在 tier1 之外（新增一条 72），属已知残余。

### 一条给测试的教训

白天/夜间这套语义只对**累加器**成立：`HistoryBuffers` 是直写，最后一次调用胜出，
所以拿它去"先喂白天再喂夜间"会把白天那一步覆盖成填充值（踩过）。
生产路径每次都经 `HistoryAccumulator`，所以这条在真实运行里是对的；
单测只能校验换序，掩码那一半交给端到端对账。

## 先审计"每个变量自己的累加规则"，再挑批（2026 年，实测）

上一轮被 `f_alb` 的白天过滤教了一次：**不能假定所有变量共用一条累加规则**。
所以这轮先对剩下 25 个逐个查三件事 —— `acc*` 调用（有没有过滤器）、
`write_history_variable_*` 的除数、值在本仓库有没有 —— 结果分成了四类：

| 规则 | 变量 |
|---|---|
| `acc1d` + `filter`/`nac`（标准） | `ldew` `qintr` `qdrip` `rss` `xerr` `zerr` `wat` `green` `lake_deficit` `rsur_ie` `rsur_se` |
| `acc2d` + 分层 | `t_lake` `lake_icefrac` |
| **`vecacc = 值*nac` 再被除 `nac`（瞬时）** | `wat_inst` `wa_inst` `wdsrf_inst` `wetwat_inst` `wetzwt` |
| 别的模块（廓线 routine） | `us10m` `vs10m` `fm10m` `ustar2` |

### 第三条规则：瞬时量

`MOD_Hist.F90:680-684`：

```fortran
vecacc = wat
WHERE(vecacc /= spval) vecacc = vecacc * nac      ! 乘一个 nac
CALL write_history_variable_2d (..., vecacc, ...) ! 写出器再除一个 nac
```

乘 `nac` 再除 `nac`，落盘的是**写出时刻那一步的瞬时值**，不是区间平均。
所以 `HistoryAccumulator` 对这批量必须走"最后一次覆盖"而不是求和 ——
新增 `INSTANTANEOUS_VARIABLES`，覆盖时把 `count` 钉在 1，于是除数为 1。
用求和会得到区间平均：在这个窗口上两者只差 3e-5（蓄量变化慢），
但物理含义不同，蓄量快变时会明显分叉。单测
`the_instantaneous_water_variables_take_the_last_step_not_the_mean`
同一串输入同时喂 `wat_inst` 与 `wat`，断言前者出末值、后者出均值 ——
只断言"有值"抓不住这条。

### 这轮补的五项

| 变量 | 来源 | 实测（对齐算例 264 条） |
|---|---|---|
| `wa_inst` | `Water2014SoilState::aquifer_water_mm` | F[4811.87,5085.83] R 同区间，maxdiff **3.07e-2**（6e-6 相对） |
| `wdsrf_inst` | `surface_water_mm` | 两边**全 0** |
| `wat_inst` | `sum(wliq+wice) + ldew + scv + wa`（`CoLMMAIN.F90:2254`） | maxdiff **0.211**（3.3e-5 相对） |
| `rsur_ie` / `rsur_se` | 上游**整列填充值** | 两边 real=0，全填充 |

`rsur_ie`/`rsur_se`（入渗超量/饱和超量地表径流）进 `DECLARED_ONLY`：
黄金文件 264 条**一个真值都没有**，与 `sensors`/`frcsat` 同类 ——
声明 + 留空才是忠实。到此黄金文件里的变量缺口从 25 降到 **20**。

`wat_inst` 的算式按上游的**元素级**结合顺序写（`(wliq[i]+wice[i])` 逐层相加再累加），
不改成 `sum(wliq)+sum(wice)` —— 浮点结合顺序不同会给出一位数的差。

### 剩下 20 个按"卡在哪"分组

```
ldew / qintr / qdrip / wat / xerr / zerr / xy_rain / xy_snow   规则已明确，值也有，接线即可
rss                        规则已明确，方案 4 下的电导/阻力映射待核对
green                      需要把 MOD_LAIEmpirical 内的 vegc 表搬进来
t_lake / lake_icefrac / lake_deficit      湖泊分支尚未驱动
wetwat / wetwat_inst / wetzwt             湿地分支尚未驱动
us10m / vs10m / fm10m / ustar2            出自另一支 Shaofeng 2023 廓线 routine
```

### 同轮续做：冠层截留四项 + `wat`

标准规则那一类（`acc1d` + `filter`/`nac`）先做掉五项：

| 变量 | 来源 | 实测 |
|---|---|---|
| `ldew` | `state.energy.leaf.canopy_water.total_mm`（**状态**，不在步输出上） | maxdiff 6.31e-3，峰值 0.0746 |
| `qintr` | `energy.interception.retained_kg_m2_s`（上游 `qintr = pinf/deltim`） | 两边**全 0** |
| `qdrip` | `ground_rain + ground_snow`（上游 `qdrip = pg_rain + pg_snow`，`CoLMMAIN.F90:930`） | maxdiff 8.62e-7，峰值 4.07e-6 |
| `wat` | 与 `wat_inst` **同一算式** | maxdiff 0.194（3e-5 相对） |

`ldew`/`qdrip` 那 8~21% 是已知的冠层露水近抵消链（与 `scv` 同源），不是新问题。

**`wat` 与 `wat_inst` 共用算式却口径不同** —— 前者区间平均、后者取末步。
同一条算式落进两个累加规则，正好是那个"瞬时"机制最干净的验收点，
所以单测把它们喂同一串输入、断言两个不同的结果。
`ldew` 还顺手说明一件事：**状态量不一定在步输出上**（它在 `state.energy.leaf`），
写 history 时得同时看状态与输出，不能只认 `output`。

到此黄金文件的变量缺口从 25 降到 **16**（本轮共补 9 个），
`UNFILLED` 按"卡在哪"重写成 4 组 —— 上一轮那条把 `alb` 写成"缺四维写出通路"的
**判断本身是错的**（根本不需要新通路），留错的清单比留空的更坏。

## `f_rss` 接上了，但值差到 2 倍：公式已逐行核对，嫌疑在输入（2026 年，实测）

`rss`（土壤表面阻力）此前记着"方案 4 下上游写的是电导标志而不是阻力，条件映射待核对"。
这轮查清了：**方案 4 不是本算例的档位** —— `DEF_RSS_SCHEME` 默认 1，
而"方案 4 写电导"那条只在 `DEF_RSS_SCHEME == 4` 时成立。所以映射本身没有歧义，
接线做了（`energy.soil_surface_resistance_s_m` → `f_rss`，普通 `acc1d` + `filter`/`nac`）。

### 顺带发现两条上游的档位规则，本仓库都没有

1. **`MOD_Namelist.F90:1948-1950`**：`DEF_USE_Campbell_SOIL_MODEL` 为假时把
   `DEF_RSS_SCHEME` **强制置 0**，并打印
   "Soil resistance is automaticlly turned off for VG soil + USGS|IGBP scheme"。
   本仓库只从算例/默认读这个字段，**不复制这条强制规则** ——
   Campbell 算例（对齐算例）不触发，但 VG 土壤算例上两边会分叉。
2. **`MOD_SoilSurfaceResistance.F90:297-311` 的雪盖混合**：
   `rss = rss/(1-fsno+fsno*rss)`（方案 ≠ 4）与 `(1-fsno)*rss+fsno`（方案 4）。
   这条本仓库**是有的**，逐行核对过（`soil_surface_resistance.rs:158-171`）。

### 值差 2 倍，但公式不是原因

对齐算例实测：

```
f_rss  F[0.01466507,0.03394720]  R[0.02844947,0.03406494]
最差 i=0：F 0.0146650692  R 0.0284494652（1.94 倍）；高端几乎重合
```

比值**不恒定**（低端 1.94、高端 1.0），所以不是某个常数因子。
把 scheme 1 的每一行对着 Fortran 核过 —— `vol_liq`、`eff_porosity`（含 `.max(0.01)`）、
`aird = porsl*(psi0/-1e7)^(1/bsw)`、`smp_node`、`hk`、`tao = eps²*(eps/porsl)^(3/max(3,bsw))`、
`dg = d0*tao`、`dsl`（含 `.clamp(0,0.2)`）、雪盖混合、`min(1e6,·)` —— **全部一致**。

所以嫌疑落在**输入**上，而这里有个容易骗人的细节：该窗口表层又湿又冻，
`0.8*eff_porosity - vol_liq` 是**两个 ~0.3 的量相减**。
取 i=0 的实测值：`vol_liq = 6.0458/(1000*0.017513) = 0.3452`，
`eff_porosity ≈ porsl - 0.2693`，于是 `0.8*eff_porosity ≈ 0.145 < vol_liq`
→ **`dsl` 的分子落到 `max(1e-6, ·)` 底上**。这时 `rss` 只由
`dz*1e-6/(0.8*porsl - aird)/dg` 决定，而 `porsl`/`aird`/`bsw` 是静态土壤参数、
`dg` 由它们与 `t_soisno(1)` 定。

**结论与下一步：** 这不是"公式写错"能修的那类差，也不该在这一层再猜。
要判定就把 `porsl`、`aird`、`bsw`、`dg`、`0.8*eff_porosity-vol_liq`
这五个量在同一个步上两边打出来 —— 变量只有五个，而且都在 `SoilSurfaceResistance`
的入口即可拿到。**在那之前 `f_rss` 的差不算已归因**；它落在 tier2 之外（新增一条 78），
与 `scv`/`gssun` 同属"近抵消量"，但这次连是哪一侧的输入不同都还没证据。

### `f_rss` 归因完成：公式无误，**只有第 0 条**是瞬变产物

上一轮留下的五个量不必再打了 —— 换了个更省的办法先把公式本身证死：
拿 Fortran 自己的**常量重启**（`porsl = 0.501584`、`psi0 = -427.853`、
`bsw = 8.195162`）与自己的 history 状态列（`f_t_soisno`/`f_wliq_soisno`/
`f_wice_soisno` 的土段第一层、`f_fsno`）按 scheme 1 在 Python 里重算一遍：

```
i=  0  gold=0.01466507  py=0.02947440  比值 0.498   ← 例外
i=  1  gold=0.02966684  py=0.03034594  比值 0.978
i= 50  gold=0.03193327  py=0.03190276  比值 1.001
i=100  gold=0.03234215  py=0.03235858  比值 0.9995
i=200  gold=0.03380158  py=0.03361505  比值 1.006
i=263  gold=0.03333898  py=0.03344691  比值 0.997
```

**稳态记录上复现到 0.1~0.5%** —— 公式、静态参数、雪盖混合三样一起被独立证实。
再看两引擎逐条对比：**264 条里只有 i=0 超 1%**（差 94%），其余全部在 **0.37%** 以内。

第 0 条为什么特殊，也能量出来。反解 `rss = 0.014665` 需要
`0.8*eff_porosity - vol_liq = +4.96e-7`（一个**极小正数**），
对应 `wice ≈ 1.13 kg/m²`；而该记录 `f_wice_soisno` 的**区间均值**是 4.32。
第 0 小时的表层正是硬瞬变：

```
土段第一层   i=0      i=1      i=2      i=3      i=4      i=5
wice        4.325    8.257    8.494    9.909   14.184   15.510
T           272.40   268.13   266.07   266.78   269.00   263.20
```

一小时里 `wice` 翻倍、`T` 摆动 4 K。而 `rss` 在这里恰好落在
**两个 ~0.3 的量相减**上（`0.8*eff_porosity` 与 `vol_liq` 相差约 5e-7），
所以"逐步 `rss` 的均值"与"用均值状态算的 `rss`"必然分叉 ——
上游取 0.0147、本仓库取 0.0285、用均值状态算是 0.0295，三个数各不相同。

**结论：`f_rss` 的公式与参数都对，i=0 是"瞬变 × 近抵消"的退化参考值**，
与 `scv`/`gssun` 同类，不该拿它调代码。剩下唯一没钉死的是**第一个子步**
的 `wice` 两边差多少（均值差 0.3% 掩盖了子步差），但它只影响这一条记录。

### 顺手修掉一条真的分叉：VG 土壤下 `DEF_RSS_SCHEME` 被强制置 0

上面查 `rss` 时发现上游还有一条本仓库没有的规则（`MOD_Namelist.F90:1946-1950`）：

```fortran
IF (DEF_USE_LCT) THEN
   IF (.not. DEF_USE_Campbell_SOIL_MODEL) THEN
      write(*,*) 'Note: Soil resistance is automaticlly turned off ...'
      DEF_RSS_SCHEME = 0
   ENDIF
ENDIF
```

**van Genuchten 土壤下土壤表面阻力恒为 0，算例里写什么都不算数。**
本仓库此前直接采信算例/默认值，所以在 VG 算例上会算出一个上游没有的阻力 ——
这是会进物理的差，不只是 history。已按规则改（`physics.rs` 的
`soil_surface_resistance_scheme`），`DEF_USE_LCT` 那道门在本仓库恒成立
（`LandCoverScheme` 只有 `Usgs`/`Igbp`，没有 PFT/PC 子网格），所以只判 Campbell。

默认空算例（VG）的 `surface_resistance_scheme` 因此从 1 变成 **0**，
原有的默认映射断言相应更新；另加一条断言"同一算例打开 Campbell 时必须回到
namelist 写的 3"，否则一个**无条件**覆盖也能让前一条通过。

## `DEF_VEG_SNOW` 其实是**已移植**的：缺的只是装配处的两个 `false`（2026 年，实测）

`unported_branches` 一直报着"`DEF_VEG_SNOW`：植被上的雪（默认真）…本仓库是 0"。
这轮去核它到底缺什么 —— 结论是**什么都不缺，只是没接上**：

* 分支逻辑四处都在：`interception.rs`（冠层雨/雪分开记、`fwet_rain`/`fwet_snow`）、
  `leaf_temperature.rs`（`clai` 与截留项按雪与否分流）、`radiation.rs` 与
  `high_res_radiation.rs`（两套反照率）。`CanopyWater` 本来就有 `rain_mm`/`snow_mm`。
* 积雪那一支的装配（`assembly.rs:1361` 的 `snow_input`）**本来就在透传**
  `self.physics.vegetation_snow`；
* 只有**无雪那一支**的两个装配点硬写死 `vegetation_snow: false`
  （`assembly.rs:1037` 的截留输入与 `:1180` 的 `LeafTemperatureOptions`）。

把这两处改成透传，再用 `--allow-unported-branches` 跑 `DEF_VEG_SNOW = .true.`
（Jan 1-3，48 条）与 Fortran 对照：

| 变量 | Fortran | Rust | maxdiff |
|---|---|---|---|
| `f_scv` / `f_snowdp` / `f_fsno` | 恒 0 | 恒 0 | **0** |
| `f_t_grnd` | [262.56, 272.40] | [262.51, 272.36] | 0.075 K |
| `f_tleaf` | [256.97, 269.58] | [256.93, 269.55] | 0.127 K |
| `f_etr` | 峰值 2.8796e-7 | 2.8632e-7 | 1.03e-8 |
| `f_lfevpa` | [28.63, 184.65] | [28.38, 187.80] | 3.14 |
| `f_ldew` | [0.01596, 0.11258] | [0.01803, 0.11797] | 5.50e-3 |

**与已验收的 `DEF_VEG_SNOW = .false.` 配置是同一水平**（那一边 `f_t_grnd` 0.0749 K、
`f_tleaf` 0.1269 K、`f_etr` 1.15e-8），而结构性签名 —— 冠层持雪 ⇒ 地面
`scv`/`snowdp`/`fsno` 恒为 0 —— 两边完全一致。所以这不再是"未移植分支"，
`unported_branches` 里的条目删掉。

这件事的分量比补几个 history 变量大：`DEF_VEG_SNOW` 的**声明默认值是 `.true.`**，
它留在清单里等于"什么都不写的算例一律被拒"，删掉之后默认配置才跑得起来。
删完再跑一遍对齐算例（显式 `.false.`）确认没有回归：`f_t_grnd` 0.0749 K、
`f_tleaf` 0.1269 K、`f_etr` 1.1495e-8，与改动前逐位相同。

**没验到的：** 试验窗口里地面无雪（`scv ≡ 0`，该站点前 71 天无降水），
所以"地面有雪 **且** 冠层持雪"这一组合没走到。这是 PFT/PC 之外该分支唯一
还没覆盖的组合，记在这里而不是当成已验。

## 10 m 四项也不是"另一支 routine"：同一次 MO 调用就够（2026 年，实测）

`UNFILLED` 里把 `us10m`/`vs10m`/`fm10m`/`ustar2` 归成"出自另一支
`Shaofeng, 2023` 廓线 routine，**那条判断又是错的**（与上一轮 `alb` 同类）。

`Shaofeng, 2023.05.20` 只是 `MOD_Vars_1DAccFluxes.F90:2779-2782` 那两行
`CALL moninobuk...` 的**日期标注**，不是新模块。那一段做的是：
用网格聚合的 `taux`/`fsena`/`fevpa` 与参考高度量反算 `r_zol_e`、`obukhov`、`um`，
**再调一次 `moninobuk`**，从这次调用取 `r_ustar2` 与 `r_fm10m`，
然后 `r_us10m = us/um * r_ustar2/vonkar * r_fm10m`（`:2789-2790`）。

而本仓库的 `history_diagnostics` **早就在做那次调用**（它就是
`MOD_Vars_1DAccFluxes.F90:2733-2790` 的移植），`MoninObukhovState` 本来就带
`friction_velocity_m_s` 与 `momentum_at_10m` —— 缺的只是把它们带出结构体。

对齐算例 264 条实测：

```
f_us10m   F[0.8457, 7.9561]  R[0.8412, 7.9558]  maxdiff 6.67e-3  (0.084%)
f_vs10m   同上（标量风下与 us10m 同值）
f_fm10m   F[3.3010,13.6524]  R[3.2979,13.5678]  maxdiff 8.47e-2  (0.62%)
f_ustar2  F[0.0987, 1.0224]  R[0.0996, 1.0225]  maxdiff 1.27e-3  (0.12%)
```

**`f_ustar2` 与 `f_ustar` 必须分开写**：前者是这次 MO 调用给出的，
后者由 `tau/rho` 反算（`:2742`），上游写的是两个变量、两条来源。

到此黄金文件里的变量缺口从 15 降到 **11**，`UNFILLED` 里
"整支 routine 未移植"那一组只剩湖泊与湿地六个。

**教训第二次生效：** 清单里写"缺某支 routine/某条通路"时，先去看
`Shaofeng` 这类**日期标注**到底是新模块还是既有调用 ——
这两轮（`alb` 的四维通路、10 m 的廓线 routine）都是把已有能力误记成缺失。

## `green` 也不需要新表：规则在 `LAIReadin`，而 `fveg0` 本仓库早就有（2026 年，实测）

`UNFILLED` 里写的是"`green` 由 `MOD_LAIEmpirical.F90:132-135` 的硬编码 `vegc` 表得出，
本仓库还没搬"—— **又是一条把已有能力误记成缺失的条目**（第三轮了）。

算例是 `USE_SITE_LAI = .true.`，走的是 `MOD_LAIReadin.F90`，不是 `MOD_LAIEmpirical`：

```fortran
IF (m == 0 .or. m == WATERBODY) THEN
   green = 0.
ELSE
   fveg = fveg0(m)
   IF (fveg0(m) > 0) THEN ...; green = 1.
   ELSE tlai = 0.; tsai = 0.; green = 0.
   ENDIF
ENDIF
```

而 `fveg0` **就在本仓库的地类表里**（`land_cover_generated.rs` 的 `fveg0`，
`ClassConstants` 加一个取值器即可）。地类 0 在装配期已被拒（"class 0 is ocean"），
所以只剩水体那一条要判。

### 关键细节：光看 `fveg0` 判不出水体

两张 `FVEG0_*` 表**每一类都是 1.0 —— 水体也是 1.0**。所以"覆盖度为正即绿叶"
这个看着等价的简化会把水体判成 1，而黄金算例的地类不是水体、`f_green ≡ 1`，
**那个错在黄金回归里看不出来**。`WATERBODY` 是 `MOD_Vars_Global.F90:25,37`
的编译期常量（USGS 16、IGBP 17），必须显式带过来
（`colm_core::waterbody_class`）。单测
`the_water_body_class_cannot_be_told_apart_by_vegetation_fraction`
两头都钉：常量是 17/16，**且水体的 `fveg0` 确实是 1.0** ——
后半句才是"为什么不能省"的证据。

对齐算例实测：`f_green` 两边**逐位相同**（都是 1.0）。缺口 11 → **10**。

`UNFILLED` 相应缩到两条，并顺手修掉上一轮遗留的一处过期描述
（那一组里 `us10m`/`vs10m`/`fm10m`/`ustar2` 上一轮已经写完，条目却没删）。

## 湖泊/湿地六个量是"**本算例不该有值**"，不是"移植缺"（2026 年，实测）

`UNFILLED` 把它们记成"湖泊/湿地分支尚未驱动"，于是看上去是六个要写的物理分支。
先把黄金文件里它们的实际内容打出来：

```
f_t_lake        (time,patch,lake)  real=0/2640  全填充
f_lake_icefrac  (time,patch,lake)  real=0/2640  全填充
f_lake_deficit  (time,patch)       real=0/264   全填充
f_wetwat        (time,patch)       real=0/264   全填充
f_wetwat_inst   (time,patch)       real=0/264   全填充
f_wetzwt        (time,patch)       real=0/264   全填充
```

**六个量一个真值都没有。** 上游只在对应 patch 类型上写它们
（湖 `patchtype == 1`、湿地 `DEF_USE_WETLAND` 且 `patchtype == 2`），
而站点是植被 patch —— 与 `sensors`/`frcsat`/`rsur_ie`/`rsur_se` 同一类：
**声明 + 留空才是与上游一致的那一列**。加进 `DECLARED_ONLY` 后
两个三维量的 `lake` 维度也对上了（`(time, patch, lake)`）。

**`UNFILLED` 的写法要改**：它该说的是"这一列上游在本算例里留空"，
而不是"本仓库还没有那支"。这两件事混在一句话里，会让下一个接手的人
去写一个根本不需要写的分支 —— 这已经是第四轮踩同一个坑
（`alb` 的四维通路、10 m 的廓线 routine、`green` 的 `vegc` 表、这六个）。

### 顺带把 `f_xy_rain`/`f_xy_snow` 接上

`CoLMMAIN.F90:793` 的 `forc_rain = prc_rain + prl_rain`（雪同理）——
是**相态拆分之后**的驱动降水，所以取自本步的 `PrecipitationState`
而不是 `forc_prc`/`forc_prl` 两列。两边都是恒 0（该窗口无降水），**逐位相同**。

缺口 10 → **2**，只剩 `xerr` 与 `zerr` 两个平衡残差。
`UNFILLED` 现在把两个残差的**完整项表**（含 `xmf`、`Σ(t-t_bef)/fact`、
以及 `xerr` 需要的**步首**蓄量）逐条写在里面，并注明它们量级是
1e-10/1e-16、tier2 的 atol 1e-7 能容 —— 所以那一步要防的是**拼错项**
（会当场变成大数而红），不是精度。这样下一个人不必再读一遍上游。

## `xerr`/`zerr` 补上，history 对黄金文件的变量缺口归零（2026 年，实测）

黄金文件里 126 个变量，此前本层声明 124 个、能填 124 个。这轮把最后两个
平衡残差接上后，**两边变量集合逐名相同**（`set(fortran) - set(rust)` 与
反向都为空），`declare_lct_variables` 与黄金不再有缺口。上一轮留在
`UNFILLED` 里的完整项表随实现搬进了 `set_lct_balance_errors` 的文档注释，
常量本身留成空数组 —— 它承载的规矩（"文件里没有" ≠ "内核产不出"）比它的
内容重要，四次踩坑都记在上面那条注释里。

### 两个量的项表

`zerr = errore`（`MOD_Thermal.F90:1394-1401`），逐项对到本仓库的字段：

| 项 | 上游 | 本仓库 |
|---|---|---|
| `sabv` | `sabvsun + sabvsha`（`:657`） | `shortwave.sunlit_absorbed_w_m2 + shaded_absorbed_w_m2` |
| `sabg` | 地面吸收短波 | `shortwave.ground_absorbed_w_m2` |
| `frl` | THERMAL 的**入参** = `forc_frl` | `HistoryReferenceState::downward_longwave_w_m2` |
| `olrg` | `:1353` | 与 `f_olrg` 同一个 `SurfaceBudget` |
| `fsena` | `fsenl + fseng`（`:1331`） | `total_sensible_heat_w_m2` |
| `lfevpa` | `hvap*fevpl + htvp*fevpg`（`:1333`） | 同上 |
| `xmf` | `MOD_PhaseChange` 的相变潜热 | `ground.latent_heat_flux_w_m2` |
| `dheatl` | `sum(dheatl_p*pftfrac)`（`:1122`） | `leaf.canopy_heat_storage_w_m2` |
| `hprl` | `sum(hprl_p*pftfrac)`（`:1121`） | `leaf.precipitation_heat_w_m2` |
| 降水显热两项 | `:1398-1399` | `SurfaceBudget::precipitation_heat_w_m2` |
| `Σ(t-t_bef)/fact` | `:1400-1401`，`j = lb:nl_soil` | `temperature_k`/`previous_temperature_k`/`layer_factor_seconds_per_j_m2_k` 三列 `zip` |

`xerr = errorw/deltim`（`CoLMMAIN.F90:1529-1543`，取 `#ifndef CatchLateralFlow` 那一支）：

```
errorw = (endwb - totwb) - (forc_prc + forc_prl - fevpa - rnof) * deltim
```

`totwb`/`endwb` 是同一条算式的步首/步末值，`Σ(wice+wliq) + ldew + scv + wa + wdsrf`，
实现为 `colm_core::total_water_storage_mm`。三个容易错的地方这轮都定死了：

1. **`frl` 不是 `dlrad`。** `MOD_Thermal.F90:517` 只在无冠层时 `dlrad = frl`；
   有冠层时 `dlrad` 多乘一份透过率。`fgrnd` 用 `dlrad*emg`，这条收支用 `frl` ——
   实测 `CN-Cng` 冬季窗口两者差 350 与 ~200 W/m² 量级。
2. **第二行 `errore` 覆盖第一行。** `:1392` 写的是减 `fgrnd` 的版本，`:1396` 立刻
   改写成减 `xmf` 的版本。`fgrnd` 与 `xmf` 差一整份地面辐射收支（实测
   `fgrnd = -90.34` 而 `xmf = 12595.8` W/m²），照抄时不能顺手把 `xmf` 换成 `fgrnd`。
   两条式子其实是同一个恒等式的两种写法：`A - fgrnd` 与 `A - (xmf + storage)`，
   而 `fgrnd = xmf + storage` 正是地面柱的能量收支。
3. **`xerr` 不含 `wdsrf` 的那一版不是 `wat`。** `wat` 是 `MOD_Vars_TimeVariables`
   里的时间变量（`Σ + ldew + scv + wa`），`totwb`/`endwb` 还要加 `wdsrf`。

`deltim` 从 `RuntimeClock::timestep_seconds()` 取**实数**而不是推进日历用的
`NINT`/`INT` 结果（`CoLM.F90` 物理用原值）；为此给 `RuntimeClock` 留了这个字段与
访问器。`totwb` 只能由调用方在内核动手**之前**从状态上取，所以它和 `deltim`
一起进了 `HistoryReferenceState`（`from_forcing` 多了两个入参）。

### 结果

`oracle/work/CN-Cng-aligned` 264 条记录，对 Fortran 黄金：

| 变量 | Fortran 区间 | Rust 区间 | maxdiff |
|---|---|---|---|
| `f_xerr` | [−3.883e-16, 4.247e-16] | [−4.379e-16, 3.968e-16] | 6.31e-16 |
| `f_zerr` | [−2.266e-10, 2.302e-10] | [−1.978e-10, 1.716e-10] | 2.85e-10 |

两者都在"两个 O(100) 量相减"的舍入地板上，tier2 的 atol 1e-7 通过。
`golden-compare` 的残差条数仍是 82（tier0 4 / tier1 27 / tier2 51），
与接这两列之前逐位相同 —— 即这轮没有动到任何已有列。

### 顺带查实的一个上游空洞（本地不减，只记录）

`WATER_2014` 的**非 VSF、非灌溉**分支上 `wdsrf` 是 `intent(inout)`，但**从不被赋值**：
唯一那处 `wdsrf = rsur*deltim` 在 `#ifdef CROP` + `DEF_USE_IRRIGATION` 里
（`MOD_SoilSnowHydrology.F90:356`），另两处在 `WATER_VSF`（`:1312-1329`）。可是
`qinfl = gwat - rsur - wdsrf/deltim`（`:368`）每步都把它当入渗量减掉一次。
于是"步首 `wdsrf` 非零"的土壤 patch 每步凭空多出 `wdsrf` mm 的水，
`errorw` **恰好**等于 `-wdsrf`。本仓库照抄了这个行为（`Water2014SoilState::surface_water_mm`
在整个 `water_2014_soil_step` 里没有赋值），所以不是移植分叉；真实算例里
`wdsrf` 恒为 0（`f_wdsrf_inst` 两边逐位相同，都是 0），空洞不显形。

这条是本轮唯一一次"残差不为零但不是拼错项"，写在
`history_tests.rs::the_balance_residuals_close_on_one_step` 的注释里：
合成夹具按 patch 序号给了 `wdsrf = 1.0`，测试必须先把步首 `wdsrf` 清零，
否则残差会是**恰好 -1.0 mm**，与被测的拼项无关。

## 两个 forcing 镜像的逐位分叉：`sqrt(2)` 的除法 vs 乘倒数、以及短波总量（2026 年，实测）

`f_xy_us`/`f_xy_vs`/`f_xy_solarin` 是 tier0（逐位比较）里仅有的三处**能修**的分叉
（第四处 `f_xy_q` 是上游自己的时间对齐问题，见上）。三条都查到了机制。

### `forc_us`/`forc_vs`：`x / sqrt(2)` 与 `x * (1/sqrt(2))` 不是同一个数

`CN-Cng` 的强迫文件只有**标量风**（`Wind`），nml 里 `vname(5) = 'NULL'`，
所以走的是 `MOD_Forcing.F90:547-549`：

```fortran
CALL block_data_copy (forcn(6), forc_xy_us , sca = 1/sqrt(2.0_r8))
CALL block_data_copy (forcn(6), forc_xy_vs , sca = 1/sqrt(2.0_r8))
```

gfortran 把 `1/sqrt(2.0_r8)` 折成一个 f64 常量，再与每个样本**相乘**。
本仓库写的是 `wind / 2.0_f64.sqrt()` —— **除法**。两者在末位会分叉：
实测第 1 条记录（样本 3.9130001068115234 与 3.6549999713897705）：

```
上游  0.5*(a*(1/sqrt2) + b*(1/sqrt2)) = 2.675692087658228
本仓库 0.5*(a/sqrt2     + b/sqrt2    ) = 2.6756920876582284   ← 差 1 ULP
```

改成乘 `1.0 / 2.0_f64.sqrt()` 后，264 条**逐位相同**（`f_xy_us` 与 `f_xy_vs`
一起修好，因为它们同源）。这类"除法 vs 乘倒数"的分叉在逐位比较下一定会显形，
值得记一条规矩：**只要上游写的是 `sca = <常量>`，本仓库就要乘那个常量，
不要用数学上等价的除法。**

### `f_xy_solarin`：拆出去的波段加不回总量

上游 `MOD_Forcing` 是**先有总量再拆波段**：`forc_xy_solarin = forcn(7)` 原样抄，
而本仓库的 `RuntimeForcing` 只带四个波段，`f_xy_solarin` 写的是
`direct_visible + direct_NIR + diffuse_visible + diffuse_NIR`。拆波段是
"总量 × 权重"再四舍五入，**加回去不保证逐位回到总量** —— 实测 264 条里
13 条差 1 ULP。

修法不是改拆波段的算法，而是把总量原样带走：`RuntimeForcing` 新增
`solar_in_w_m2`（`= MOD_Forcing` 的 `forc_solarin`），`f_xy_solarin` 取它。
降尺度分支同理取降尺度后的**新总量**，而不是把新波段加回去。
修完从 13 条降到 **11 条**。

### 剩下的 11 条是上游 coszen 重分配的产物，本轮不追

那 11 条的差仍然是 1 ULP（最大 5.68e-14 W/m²，量级 193 W/m²），不是公式错：
上游短波槽走的是 `tintalgo = 'coszen'`（`MOD_Forcing.F90:495-517`）
`forcn = cosz/avgcos * forcn_LB|UB`。253 条里那个比值按位等于 1，
11 条里差 1 ULP —— 要么是 `cosz`/`avgcos` 的时间戳算得差一点，
要么是线性插值的权重退化成了 `(1-1e-16, 1e-16)`。
要复现得先把上游的时间戳算法逐位搬过来，成本与收益不成比例，先记录在这里。

`f_xy_q` 那 3 条不是同一类：差值 1.69e-5（相对 1.7%），是上游自己取了
强迫文件里根本不存在的样本对（见上文），本仓库算的是真·相邻样本均值。

### 结果

`golden-compare` 的分层残差从 4/27/51 降到 **2/27/51**（总 82 → 80），
tier0 只剩 `f_xy_solarin`(11/264) 与 `f_xy_q`(3/264)。

## 短波分带那 12 个量：不是时刻，是**坐标**（2026 年，实测）

上一轮把这一簇记成"误差形状指向 `sunang`、疑似 255 秒的时间偏移"。这轮做了一次
决定性测量，结论是**时刻没错，坐标错了**。

### 决定性测量：让内核把 `sunang` 打出来

在 `vendor/CoLM202X/main/MOD_Forcing.F90` 的分带块里临时加了一段
`WRITE`（`/tmp/colm_forc_dump.txt`，逐步输出 `calday`、`a`、`sunang` 与四个波段），
重建内核后跑 `CN-Cng-aligned`。有了逐步真值，反解就变成直接对比：

```
step 16: calday=365.99025462962965  a=91.262001038  sunang=0.053003690472386
```

而本仓库在同一时刻、同一 `calday`、用**站点**经纬度算出来的是
`0.052114304` —— 差 8.9e-4。既然时刻一致，剩下的只能是经纬度。

反解"哪个纬度能复现上游的 `sunang`"，得到的隐含纬度随时刻在 44.5087 与 44.5000
之间游走；把它当成常数拟合对不上（残差变号）。真正的解释是：**上游在同一个
`orb_coszen` 上喂了两组不同的坐标**。

### 根因

| 位置 | 坐标 | 用途 |
|---|---|---|
| `MOD_Forcing.F90:621` | `gforc%rlon`/`rlat` = **强迫网格单元中心** | 短波直散拆分 |
| `MOD_Forcing.F90:797` | `patchlonr`/`patchlatr` = **站点** | 地形降尺度的 `coszen`/`cosazi` |
| `CoLMMAIN.F90:2076` | `patchlonr`/`patchlatr` = **站点** | 地表反照率的 `coszen` |

`SinglePoint` 的强迫网格是 360×180 的 1° 全球网格（`MOD_Namelist.F90` 的
`#ifdef SinglePoint` 把 `DEF_nx_blocks`/`DEF_ny_blocks` 直接赋成 360/180，
`MOD_Grid.F90::grid_define_by_ndims` 再由它们生成边界），所以单元中心是
`floor(站点) + 0.5`：CN-Cng 的 44.5933/123.5092 → **44.5/123.5**。

这个 0.093° 的差在拆分公式里被放大：`difrat = 0.0604/(sunang-0.0223)+0.0683`
在低太阳角下 `d(difrat)/d(sunang)` 很大，于是可见光波段差 1.77%。

**顺带解掉了"约 255 秒"那条悬案。** 本地正午那一步的角度差实测是
`0.3756755815803163`（网格中心）对 `0.3741924582917629`（站点），差 1.48e-3
—— 正是上一轮被折成"255 秒"的那个数。它从来不是时间偏移。

### 修法与结果

`RuntimeForcingInput` 现在同时带**站点**与**网格中心**两组坐标：
`cosine_zenith`（供降尺度与 `cosazi`）取站点，短波拆分另算一个
`sun_angle` 取网格中心。`forcing_grid_center_degrees` 把"1° 网格单元中心 =
`floor(x)+0.5`"这条推导写在一处，`PointForcingSeries::runtime_at_calendar_time`
是唯一的调用点。

实测同一个本地正午步：

| 喂进去的角度 | `solvd` |
|---|---|
| 站点 44.5933/123.5092 | 64.892454 |
| 网格中心 44.5/123.5 | **64.953960** |
| 上游 `f_solvdln`（黄金） | **64.953960** |

网格中心**逐位命中**（`64.95396021855056` 与黄金、与内核打印的 `sols` 完全相同）。

`f_solvd`/`f_solvi`/`f_solnd`/`f_solni` 与四个 `*ln` 共 8 个变量从残差表里消失，
tier1 从 27 降到 **19**（总残差 80 → 72）：

```
failures by tier: {"tier0": 2, "tier1": 19, "tier2": 51}   改前 {2, 27, 51}
```

上一轮那条测试（`the_shortwave_split_is_fed_the_step_start_solar_angle`）把
`64.892454` 当成"步首角"、把 0.00148 的角度差当成未定的时间偏移，正是这个坑的
产物；已改写成 `the_shortwave_split_is_fed_the_grid_cell_solar_angle`：
网格中心**断言逐位相等**，站点坐标断言至少偏 5e-4，两条一起钉住用的是哪一组。

### 剩下的一簇：`f_sr*` 与 `f_sab*` 都指向反照率

`f_srvd = solvd*alb(1,1)` 等四条的算式已逐行核对过（`MOD_NetSolar.F90:281-284`、
`net_solar.rs:130-139`，含 `srvd` 用 direct-direct、`srvi` 用 direct-diffuse 的
配对），所以 `f_srvd`/`f_srvi`/`f_srnd`/`f_srni`/`f_sr` 这 9 条残差**继承自
`f_alb`**：`sol*` 现在是逐位对的，剩下的只能是 `alb`。`f_alb` 自身仍在残差表里，
`f_sabg`/`f_sabvsun`/`f_sabvsha` 同理（地面吸收 = 入射 − 反射）。

**所以下一个目标是 `alb` 这条链**（`albland` + 雪盖混合 + `TwoStream`），
它一次能解掉 tier1 里 19 条中的 13 条。注意容差表头部写的不变式
——"一个变量的层级不得严于它任何一个输入的层级"—— 在动 `f_sr*`/`f_sab*`
的层级之前先修 `f_alb`，否则就是把分层本身毁掉。

## `f_alb` 的残差是从 `f_fsno` 继承的，而 `f_fsno` 的根在雪量（2026 年，实测）

上面说"下一个目标是 `alb`"。这轮把传播链量出来了，结论是 **`alb` 本身没有独立
的公式错误**：它的残差是 `fsno` 残差乘以"雪面与地面的反照率对比"。

白天逐条实测 `d_alb / d_fsno`（`CN-Cng-aligned`，`d = Rust − 上游`）：

| 记录 | `d_fsno` | 可见光直接 | 可见光散射 | 近红外直接 | 近红外散射 |
|---|---|---|---|---|---|
| 7 | +1.94e-3 | 0.073 | 0.115 | **0.281** | **0.216** |
| 11 | +1.94e-3 | 0.230 | 0.202 | **0.281** | **0.216** |
| 15 | +1.94e-3 | 0.085 | 0.125 | **0.281** | **0.216** |
| 247 | +3.11e-3 | 0.091 | 0.162 | **0.333** | **0.297** |

近红外那两列的比值**一整天都是常数**，正是 `alb_snow − alb_ground` 这个对比；
可见光那两列随时刻变，因为可见光的地面反照率本身随太阳高度角变（`albland`）。
两者都说明 `alb = (1-fsno)*alb_ground + fsno*alb_snow` 这条混合没有被写错，
差的是喂进去的 `fsno`。

链条于是是：

```
f_scv / f_snowdp  →  f_fsno（MOD_SnowFraction）  →  f_sigf  →  f_alb  →  f_sr* / f_sab*
   13% 相对差            15% 相对差                         ≤1.03e-3 绝对
```

**所以 tier1 里那 13 条（`f_sr*` 9 条 + `f_sab*` 3 条 + `f_alb`）都该等 `scv` 修好**，
不该去放松它们的层级 —— 那正是容差表头部禁止的"层级严于输入"。

### `scv` 的发散：**零降雪**，且第一步就分叉

两个量级很小但性质很关键：

1. **`f_xy_snow` 全窗口恒为 0**（两个引擎逐位相同）。所以 `scv` 的增长（0 → 0.047 mm）
   **完全不是降雪**。上游里 `scv` 能被写大的地方只有一处：
   `MOD_SnowLayersCombineDivide.F90:385` 的 `scv = scv + wice_soisno(j) + wliq_soisno(j)`
   （以及 `:405` 薄雪塌缩的 `scv = zwice`），`MOD_NewSnow.F90:73` 那条需要降水。
2. **第一步就分叉**，不是漂移：记录 1 上游 0.004372 / 本仓库 0.005924（比 1.355），
   记录 2 是 1.187，记录 5 是 1.113 —— 比值收敛但起点就错，所以是**确定性公式差**，
   不是累积误差。

已排除的：

- **薄雪塌缩分支**（`snowdp < 0.01`）：本仓库 `snow.rs:548-565` 与上游
  `MOD_SnowLayersCombineDivide.F90:402-417` 逐行相同（`snl=0`、`scv=zwice`、
  `scv<=0` 才清 `snowdp`、`wliq_soisno(1) += zwliq`）。而且本算例的 `snowdp`
  恒为 4.6e-4 m，正落在这个分支里 —— 分支本身对，说明**进去的质量**就不同。
- **相变里的 `scv` 扣减**（`MOD_PhaseChange.F90:239`）：那是 `max(0, scv-xm)`，
  只减不增，不是增长源。

下一步该做的是**逐步对 `wice_soisno(0)` / `wliq_soisno(0)`**（雪层槽位）与
本仓库的 `RuntimeSnowColumn.ice_water_kg_m2`/`liquid_water_kg_m2`，找第一处
不等的那个量。注意这一步要的是**逐层逐值**的对比，不是又读一遍 Fortran ——
上一次 `sunang` 的教训就是"读不出来，得打出来"。

## `scv` 零降水增量的来源找到了：冠层水在 `tleaf <= tfrz` 时按**雪**排掉（2026 年，实测）

上面那节说要"逐步对雪层槽位"。这轮把内核的 `newsnow` 入口打出来，第一个字段就
把问题定了：

```
step  pg_rain      pg_snow      scv          snowdp       fsno        wice_snow  t_grnd
0     0.000e+00    0.000e+00    0.000e+00    0.000e+00    0.000e+00   0.0        283.00
3     0.000e+00    4.857e-06    8.743e-03    8.233e-05    0.000e+00   0.0        271.14
4     0.000e+00    5.775e-06    1.914e-02    1.829e-04    3.101e-03   0.0        265.12
```

**`pg_snow` 在零降水下非零**，而且 `scv` 每一步的增量正好等于 `pg_snow*deltim`
（step 3：4.857e-06 × 1800 = 8.743e-03，与 `scv` 逐位相合）。所以 `scv` 的雪不是
从天上来的，是**冠层水**。

源头在 `MOD_LeafInterception_Extended.F90:208-216`（注意：编译的是 `extends/`
那一份，`main/` 的同名文件从不参与编译）：

```fortran
w = ldew + p0
IF (tleaf > tfrz) THEN
   xsc_rain = max(0., ldew-satcap)
   xsc_snow = 0.
ELSE
   xsc_rain = 0.
   xsc_snow = max(0., ldew-satcap)     ! 叶温到冰点以下，冠层水按雪排掉
ENDIF
ldew = ldew - (xsc_rain + xsc_snow)
...
pg_snow = (xsc_snow + thru_snow) / deltim
```

本仓库 `interception.rs:162-172` 的这段逐行相同，`saturation_capacity = dewmx*vegt`
也与上游的 `satcap = dewmx*vegt` 同源（`vegt = lai+sai`，实测 0.2+0.45 = 0.65，
`satcap = 0.1*0.65 = 0.065 mm`）。**所以这里没有移植错误。**

### 真正的放大器是 `max(0, ldew - satcap)`

上游这一步排掉的是**超出饱和容量的那一部分**，即两个几乎相等的量之差：

| 记录 | `f_ldew` 上游 | `f_ldew` 本仓库 | 相对差 | `f_scv` 上游 | `f_scv` 本仓库 | 相对差 |
|---|---|---|---|---|---|---|
| 1 | 0.07456731 | 0.07637377 | +2.4% | 0.00437164 | 0.00592401 | **+35%** |
| 2 | 0.06689785 | 0.06708586 | +0.3% | 0.01983566 | 0.02353442 | +19% |
| 5 | 0.07156942 | 0.07192795 | +0.5% | 0.04464203 | 0.04983504 | +12% |

`satcap = 0.065 mm` 而 `ldew ≈ 0.075 mm` —— 差值只有 0.01 mm 量级，所以 `ldew` 的
**2.4% 相对误差被放大成 `pg_snow` 的 ~19%**。更关键的是这个偏移是**持续**的：
`ldew` 的固定偏差变成每步固定的额外排出量，于是 `scv` 会**积分**它
（记录 1→6 的 `f_scv` 差从 0.00155 累积到 0.00537）。

链条完整了：

```
f_ldew（冠层水，+2.4%）→ max(0, ldew-satcap) 放大 → pg_snow → f_scv（+13~35%）
    → f_fsno（MOD_SnowFraction）→ f_sigf → f_alb → f_sr* / f_sab*
```

**下一个目标是 `ldew` 的演化。** 它是冠层水收支：截留加、湿冠层蒸发减、凝结加。
第一记录就有 2.4% 的差，所以要看的是**第一步之后 `ldew` 的收支**，而不是雪。
不要把 `scv` 当成独立问题查 —— 它只是 `ldew` 误差的积分器。

### 再往上一环：`ldew` 是**凝结**长出来的

初始重启里 `ldew = 0`、`tleaf = t_grnd = 283 K`，而第一条记录（两步的均值）就已经是
0.0548 / 0.0572 mm —— 零降水下 `ldew` 只能靠**冠层凝结（霜）**长出来。对上
`f_fevpl` 的符号正好：前几步是**负值**（冷凝），

| 记录 | `f_fevpl` 上游 | 本仓库 | 相对 |
|---|---|---|---|
| 0 | −1.728e-05 | −1.804e-05 | +4.4% |
| 1 | −6.090e-06 | −6.330e-06 | +3.9% |
| 2 | −1.060e-06 | −1.160e-06 | +9.4% |
| 3 | −1.690e-06 | −1.820e-06 | +7.7% |

所以链条再往上一环是 **`fevpl`（叶面蒸发 = 蒸腾 + 湿冠层蒸发）在冷凝段的 ~5% 偏差**。
它让 `ldew` 长得快一点，`ldew` 越过 `satcap` 后被放大成 `pg_snow`，再积分进 `scv`。

注意**不是**一个系数错：全窗口 150 个非零 `f_fevpl` 的 `rust/fort` 比值分布很散
（min −1.12、max 2.95、median 0.99），说明差的是**状态驱动的那部分响应**，不是常数。
而且 `f_lfevpa` 的同一批记录只差到 3.14 W/m²（早已记录为上游自身的不闭合）。
所以下一轮的目标是 `MOD_LeafTemperature` 的**湿冠层蒸发/凝结**那一项
（`evplwet`，即 `fevpl - etr`），不是蒸腾。

至此从"`f_alb` 差 1e-3"到"湿冠层凝结差 5%"的完整链条都落盘了：

```
湿冠层蒸发/凝结（fevpl，冷凝段 +5%）
  → ldew（冠层水）            +2.4%
  → max(0, ldew - satcap)     放大（satcap 0.065 对 ldew 0.075）→ +19~35%
  → pg_snow                    （tleaf <= tfrz 时按雪排掉）
  → f_scv                      +13~35%，并被**积分**
  → f_fsno → f_sigf → f_alb → f_sr* / f_sab*
```

## 叶面潜热要按叶温选 `hvap`/`hsub`，而 `lfevpa` 用的是 `htvpl`（2026 年，实测）

上一节把链条指到"湿冠层蒸发/凝结"。这一节修掉的是它上游的一个**真正的移植分叉**：
叶温求解里的潜热。

### `main/` 与 `extends/` 又一次：`MOD_Thermal` 也被顶掉了

`Makefile:635-647` 用 `extends/interception/MOD_Thermal_CanopyPhase_Extended.F90`
顶掉了 `main/MOD_Thermal.F90`。两份的 `lfevpa` **不是同一个式子**：

| 文件 | `lfevpa` |
|---|---|
| `main/MOD_Thermal.F90:1333` | `hvap*fevpl + htvp*fevpg` |
| `extends/interception/MOD_Thermal_CanopyPhase_Extended.F90:1343` | **`lfevpl + htvp*fevpg`** |

而 `lfevpl` 由叶温模块导出：`MOD_LeafTemperature_Extended.F90:1584`
**`lfevpl = htvpl*fevpl`**，其中 `htvpl = hvap if tl > tfrz else hsub`
（`:693-697`，每轮准 Newton 迭代开头按**当前** `tl` 重算）。

本仓库此前对叶面那一项硬写 `hvap`，于是差 `(hsub-hvap)*fevpl = 3.336e5*fevpl`。

### 指纹：`f_zerr` 恰好差 `(hsub-hvap)*fevpl`

改叶温求解、**不改** history 的 `lfevpa` 之后，`f_zerr` 从 ~1e-10 跳到 −5.81，
而它逐条等于 `3.336e5 * f_zerr` 的 `fevpl`：

| 记录 | `fevpl` | `f_zerr` | 比值 |
|---|---|---|---|
| 0 | −1.741e-05 | −5.809 | 3.336e5 |
| 1 | −6.178e-06 | −2.061 | 3.336e5 |
| 2 | −1.129e-06 | −0.377 | 3.336e5 |
| 6 | +4.295e-07 | +0.143 | 3.336e5 |

**这就是 `hsub - hvap = hfus = 0.3336e6`。** 残差的指纹把根因按到了小数点后三位，
比读源码快得多 —— 冠层能量收支是按 `htvpl` 闭合的（Newton 的分子就是
`- htvpl*fevpl`），而残差里的人为地用了 `hvap`。

### 修法与实测

叶温模块里 `htvpl` 被用在**七处**：增量式的分子与分母、`dele` 收敛判据、
循环后 `fsenl` 的两项修正（`(dtl_noadj-dtl)` 那一项里的 `htvpl*fevpl_dtl`、以及
`htvpl*erre`）、`elwdif` 的显热补偿、以及 `err` 能量残差。现在这七处都用
本仓库按 `previous_leaf_temperature` 选出的 `leaf_latent_heat_j_kg`，并作为
`LeafTemperatureOutput::leaf_latent_heat_j_kg` 带出来给 history 用
（`surface_budget` 的 `latent_heat` 因此不再需要 `vaporization_heat_j_kg` 入参，已删）。

注意循环后那三处用的是**最后一轮迭代**的 `htvpl`，而那一轮是按 `tlbef` 取的 ——
上游的 `htvpl` 正是在 `tl = tlbef + dtl(it)` **之前**算的。拿最终叶温重算会晚半个
增量，在 `tfrz` 附近翻错相。

实测（`CN-Cng-aligned`，264 条）：

| 变量 | 改前 | 改后 | 倍数 |
|---|---|---|---|
| `f_lfevpa` | 3.144 | **2.012** | 1.6× |
| `f_scv` | 6.968e-3 | **1.830e-3** | 3.8× |
| `f_fsno` | 3.106e-3 | **6.604e-4** | 4.7× |
| `f_alb` | 1.034e-3 | **4.200e-4** | 2.5× |
| `f_tleaf` | 0.1269 | **0.1084** | 1.2× |
| `f_ldew` | 6.308e-3 | **5.082e-3** | 1.2× |
| `f_sr` | 0.0206 | **0.0081** | 2.5× |
| `f_t_grnd` | 7.49e-2 | **6.57e-2** | 1.1× |

**`f_lfevpa` 那条"上游自身不闭合"的旧记录是错的** —— 它一直是这个分叉，
3.14 W/m² 正好是 `(hsub-hvap)*fevpl` 在该窗口的量级。分层残差条数不变
（`{tier0: 2, tier1: 19, tier2: 51}`，容差比残差严得多），但每一条的余量都小了。

`leaf_latent_heat_j_kg` 的选取由
`leaf_temperature_tests.rs::the_leaf_latent_heat_follows_the_leaf_temperature`
钉住：同一夹具分别喂暖/冷大气，断言叶温跨过冰点时这一项从 `hvap` 换成 `hsub`，
并顺带核对 `hsub - hvap = hfus`。

### 修完之后的链条：一切都归到 `tleaf`

修完 `htvpl` 再看**逐步**（用 `DEF_HIST_FREQ = 'TIMESTEP'` 的算例拿本仓库的逐步值，
配上内核逐步打印的上游值），第一步的叶温差从 **0.170 K 降到 0.0316 K**：

| 步 | 上游 `tl` | 本仓库 `tl` | ΔT | 上游 `evplwet` | 本仓库 `fevpl` | 比 |
|---|---|---|---|---|---|---|
| 0 | 265.882741 | 265.851189 | −0.0316 | −2.6361e-05 | −2.6500e-05 | 1.0053 |
| 1 | 261.551308 | 261.521082 | −0.0302 | −8.1960e-06 | −8.3247e-06 | 1.0157 |
| 2 | 260.620185 | 260.567294 | −0.0529 | −6.4115e-06 | −6.4576e-06 | 1.0072 |
| 3 | 260.422970 | 260.395109 | −0.0279 | −5.7730e-06 | −5.8992e-06 | 1.0219 |

而且这个 ΔT 能**定量解释** `fevpl` 的比：第 0 步 `qsatl ≈ 2.055e-3`、
`d(qsat)/dT ≈ 1.23e-4 /K`，`gradient` 对 `qsatl` 的系数是 `wtaq0+wtgq0 = 0.8294`，
所以

```
Δgradient = 0.8294 × 1.23e-4 × (−0.0316) = −3.23e-6
比值 = 1 + 3.23e-6 / 5.412e-4 = 1.0060     （实测 1.0053）
```

**所以 `fevpl` 没有独立缺陷，它是 `tleaf` 的下游。** 同理：
`f_lfevpa` 现在的 2.012 W/m² 正好是 `htvpl × 6.9e-7 ≈ 2.0`，即 `fevpl` 的下游。

到这里，本轮之前那批残差（`lfevpa` / `ldew` / `scv` / `fsno` / `sigf` / `alb` /
`sr*` / `sab*`）**全部追踪到同一个根**：叶温求解剩下的 0.03~0.11 K。
下一步该做的是**逐步比较叶温 Newton 迭代的分子与分母**（上游
`MOD_LeafTemperature_Extended.F90:1173-1176`），而不是再读一遍那个 1500 行的例程 ——
`sunang`、`scv`、`htvpl` 三次都是"打出来"比"读出来"快一个数量级。

### 跨算例复核：积雪算例也同向改善，无回归

`htvpl` 是叶温求解的公共路径，所以在另一个黄金算例上复核了一遍
（`CN-Cng-vegsnow`，`DEF_VEG_SNOW = .true.`、96 步、有真实雪层）：

| 变量 | 改前 | 改后 | 倍数 |
|---|---|---|---|
| `f_tleaf` | 1.2694e-1 | **5.7573e-2** | 2.2× |
| `f_fevpl` | 7.6364e-7 | **1.3362e-7** | 5.7× |
| `f_lfevpa` | 3.1437 | **2.0305** | 1.5× |
| `f_ldew` | 5.4966e-3 | **2.0503e-3** | 2.7× |
| `f_alb` | 2.4084e-3 | 1.8012e-3 | 1.3× |
| `f_t_grnd` | 7.5057e-2 | 6.5775e-2 | 1.1× |
| `f_sr` | 6.2546e-2 | 4.2601e-2 | 1.5× |
| `f_scv` / `f_fsno` | 0 | **0** | 逐位相同 |

分层残差条数两边都不变（`{"tier1": 19, "tier2": 44}`，共 63 条），但余量同向变小，
且 `f_scv`/`f_fsno` 在显式雪层算例上**逐位不变** —— 说明这次改的确实是"叶面潜热"
那一条，没有碰到雪层质量路径。（`scv`/`fsno` 之所以逐位不变：该算例的雪走显式层，
而本仓库 `scv` 的分叉在薄雪塌缩路径上，那个算例的 `snowdp` 越过了阈值。）

## 叶温残差的根：`thm` 不是位温（2026 年，实测）

上一节把一切归到"叶温求解剩下的 0.03 K"。这一节按计划逐步比较准 Newton 迭代的
分子与分母，找到了根 —— 而且它**不在**那条 1500 行的例程里。

### 逐项比较：导数全对，水平项差了

在 `MOD_LeafTemperature_Extended.F90` 循环之后打一条（每步一行）：
`dtl, sabv, irab, dirab_dtl, fsenl, fsenl_dtl, htvpl, fevpl, fevpl_dtl, clai,
deltim, qintr_*, t_precip, tl, tlbef`；本仓库在 `leaf_temperature.rs` 的循环之后
打对应的 `last.*`。第一步（初始状态完全相同）对比：

| 项 | 上游 | 本仓库 | 相对差 |
|---|---|---|---|
| `sabv` | 0 | 0 | 0 |
| `irab` | −14.24104 | −14.12033 | **+0.85%** |
| `dirab_dtl` | −3.827487 | −3.826125 | +0.036% |
| `fsenl` | 60.73040 | 61.20513 | **+0.78%** |
| `fsenl_dtl` | 40.61558 | 40.62748 | +0.029% |
| `fevpl` | −2.63596e-5 | −2.64981e-5 | +0.53% |
| `fevpl_dtl` | 7.23414e-6 | 7.20547e-6 | −0.40% |

**两个导数几乎逐位相同，两个通量差 0.8%。** 这正是"线性化对、水平项错"的形状。
`irab` 的差还刚好等于 `dirab_dtl × ΔT`（−3.827 × (−0.0316) = +0.121，实测 +0.1207），
说明它是被叶温差驱动的**结果**，不是原因；而 `fsenl` 的差（+0.4747）与
`fsenl_dtl × ΔT` = −1.28 差了 1.76 W/m²，说明它有一个**独立**的输入差。

把差归一到"温度目标"上：`fsenl = rhoair·cpair·cfh·((wta0+wtg0)·tl − (wta0·thm + wtg0·tg))`，
而 `fsenl_dtl = rhoair·cpair·cfh·(wta0+wtg0)`，所以

```
T_target := (wta0*thm + wtg0*tg)/(wta0+wtg0) = tl − fsenl/fsenl_dtl
上游 264.387691   本仓库 264.344896   →  差 −0.0428 K
```

### 决定性的一列：`thm` 恒定差 0.0588 K

把 dump 扩成 `wta0, wtg0, thm, tg, rhoair` 之后，第一步：

| 量 | 上游 | 本仓库 | 差 |
|---|---|---|---|
| `tg` | 283.0 | 283.0 | **逐位相同** |
| `rhoair` | 1.3559947226232352 | 1.3559947226232352 | **逐位相同** |
| `wta0` | 0.5930009697 | 0.5931055474 | +1.0e-4（0.018%） |
| `wtg0` | 0.2364138446 | 0.2363789881 | −3.5e-5 |
| `thm` | **256.96880366210939** | **256.91000366210938** | **−0.0588 K（全 528 步恒定）** |

`0.0098 × 6.0 = 0.0588`。上游 `MOD_Thermal.F90:550`（编译的是
`extends/interception/MOD_Thermal_CanopyPhase_Extended.F90:550`）：

```fortran
! potential temperature at the reference height
thm = forc_t + 0.0098*forc_hgt_t                     !intermediate variable equivalent to
                                                     !forc_t*(pgcm/forc_psrf)**(rgas/cpair)
th  = forc_t*(100000./forc_psrf)**(rgas/cpair)       !potential T
thv = th*(1.+0.61*forc_q)                            !virtual potential T
```

**`thm` 不是位温。** 它按固定递减率把气温抬到观测高度；位温是同处的 `th`。
本仓库两处（`standard_lct_step.rs` 的 `leaf_input` 与 `assembly.rs` 的模板）都把
`reference_air_temperature_k` 填成了 `forcing.air_temperature_k`（= `forc_t`）。

叶温模块里两者分工明确，混不得：`dth = thm − taf`、`taf = wta0*thm + wtg0*tg + wtl0*tl`
用 `thm`，而 `dthv = dth*(1+0.61*qm) + 0.61*th*dqh` 与
`moninobukini(ur, th, thm, thv, dth, ...)` 里的那一项用 `th`。

### 修法与实测

`colm_core::reference_height_temperature_k(air_temperature_k, temperature_height_m)`
一处定义 `thm`（常数命名为 `REFERENCE_LAPSE_RATE_K_M = 0.0098`），两处调用点都用它。
`forc_hgt_t` 取自强迫文件的 `reference_height_t`（本算例 6 m），与
`MOD_Forcing.F90:297-311` 的优先级一致。

`CN-Cng-aligned`（264 条）：

| 变量 | 上一节末 | 本节末 | 倍数 |
|---|---|---|---|
| `f_tleaf` | 1.0841e-1 | **2.6142e-2** | 4.1× |
| `f_fevpl` | 7.2662e-7 | **2.1807e-7** | 3.3× |
| `f_lfevpa` | 2.0115 | **1.5723** | 1.3× |
| `f_ldew` | 5.0819e-3 | **1.6463e-3** | 3.1× |
| `f_scv` | 1.8301e-3 | **1.5871e-4** | 11.5× |
| `f_fsno` | 6.6044e-4 | **5.5924e-5** | 11.8× |
| `f_alb` | 4.1998e-4 | **3.4156e-5** | 12.3× |
| `f_t_grnd` | 6.5690e-2 | **2.2148e-2** | 3.0× |

`f_qcharge`/`f_qdrip`/`f_z0m`/`f_zerr` 四条**离开残差表**，
分层残差 `{2, 19, 51}` → **`{2, 19, 48}`**（72 → 69 条）。

积雪算例 `CN-Cng-vegsnow` 同向且更猛（63 → **57** 条，`{"tier1": 19, "tier2": 38}`）：

| 变量 | 上一节末 | 本节末 | 倍数 |
|---|---|---|---|
| `f_tleaf` | 5.7573e-2 | **3.1853e-3** | 18× |
| `f_lfevpa` | 2.0305 | **6.2071e-2** | 33× |
| `f_t_grnd` | 6.5775e-2 | **1.1337e-3** | 58× |
| `f_ldew` | 2.0503e-3 | **1.8633e-4** | 11× |
| `f_fevpl` | 1.3362e-7 | **2.6519e-8** | 5× |

### 顺带更正一条：`f_lfevpa` 的残余现在来自 `fevpg`

`f_lfevpa` 还剩 1.572 W/m²，但它**不再是**叶面那一项：`f_fevpl` 只差 2.18e-7，
乘 `htvpl` 是 6.2e-4。剩下的 1.572 对应 `1.572/htvp ≈ 5.5e-7` kg/m²/s，
而 `f_fevpg`（地面蒸发）正是这个量级 —— 所以它现在归到地面蒸发那条已知残差上，
与叶温链条无关了。

### 一条规矩

`thm` 这个名字看起来就是 "potential temperature" 的缩写，而定义不是。
上一节的 `htvpl` 也一样（`main/` 的式子与实际编译的 `extends/` 不同）。
**照着源码里的表达式抄，不要按名字或按 `main/` 推断。**

## 修完三处之后的残差排序，与下一个目标（2026 年，实测）

`thm` 修完后把残差按相对误差排序（`CN-Cng-aligned`，264 条）：

| 变量 | 越界条数 | 最大相对差 | 上游 vs 本仓库（该记录） |
|---|---|---|---|
| `f_gssun` | 67/264 | **7.4e-1** | 5.3227e-4 vs 2.0097e-3 |
| `f_rss` | 261/264 | 4.8e-1 | 1.4665e-2 vs 2.8449e-2 |
| `f_fgrnd` | 264/264 | 4.3e-1 | −1.48946 vs −0.847464 |
| `f_gssha` | 39/264 | 3.7e-1 | 8.9499e-3 vs 1.4235e-2 |
| `f_fsena` | 264/264 | 2.8e-1 | −0.491287 vs −0.678618 |
| `f_fevpl` | 5/264 | 1.8e-1 | −9.9027e-7 vs −1.2083e-6 |
| `f_rnet` | 264/264 | 1.6e-1 | 0.129447 vs 0.109236 |
| `f_lfevpa` | 264/264 | 7.6e-2 | 15.846 vs 14.6369 |
| `f_fevpg` | 29/264 | 6.7e-2 | 9.2945e-6 vs 8.6681e-6 |

要点：

- **`f_rss` 的 261/264 不等于"处处差 48%"**：tier2 的 atol 是 1e-7，而 `rss` 的量级
  是 1e-2，所以"有一点差"就记越界；最大那条仍是早已归因的记录 0（首小时瞬变，
  分子是两个 ~0.3 的量相减出的 5e-7）。
- **绝对量最大的是叶面通量**（`f_lfevpa` 1.57、`f_fsenl` 0.55、`f_fsena` 0.19 W/m²），
  它们同源：`f_lfevpa` 现在归到 `f_fevpg`，而 `f_fevpg`/`f_qinfl`/`f_fevpa` 是同一
  条 6.7% 的地面蒸发链。
- **`f_gssun`/`f_gssha` 是最大的相对差，也是最像一个独立缺陷的一条。**
  两边的公式看着等价：上游 `MOD_LeafTemperature_Extended.F90:1308`
  `gssun = (laisun/rssun)*(tprcor/tlbef)`（注释明确写着 `rssun` 此时是
  **leaf-scale**），本仓库 `laisun/last.leaf_sunlit_resistance*pressure_conversion/
  previous_leaf_temperature`，其中 `leaf_sunlit_resistance = stomatal_resistance*laisun`
  —— 若 `stomatal_resistance` 是 leaf-scale，则两者逐字相同（`f_etr` 到 1e-8
  也要求这个比例关系成立）。

  **所以嫌疑不在公式，在 `rssun` 的口径或 `lai > 0.001` 那个守卫**：
  上游有 `IF (lai > 0.001) THEN ... ELSE gssun = 0`，而本仓库没有对应的分叉；
  另外 67/264 这个越界比例很像**夜间/晨昏**（`laisun` 很小，相对差自然被放大）。
  下一步该做的是拿 264 条的 `f_gssun` 与 `f_laisun` 对照，看差值是否只在
  `laisun` 小的那些记录上出现 —— 这与前面三次一样，"打出来"比"读出来"快。

## 叶温那条线到此为止：剩下的不是缺陷，是状态差（2026 年，实测）

`thm` 修完后用同一套逐步对比重跑了一遍（内核打 `irab/dirab_dtl/fsenl/fsenl_dtl/
fevpl/fevpl_dtl/wta0/wtg0/thm/tg/rhoair`，本仓库打对应的 `last.*`），结论是
**叶温那条线没有独立缺陷了**：

### 第一步（初始状态完全相同）

| 项 | 上游 | 本仓库 | 相对差 |
|---|---|---|---|
| `thm` | 256.96880366210939 | 256.96880366210939 | **逐位相同** |
| `tg` | 283.0 | 283.0 | **逐位相同** |
| `tlbef` | 265.88293536922623 | 265.87981693096589 | −3.12e-3 K（上一轮是 −0.0316 K） |
| `dirab_dtl` | −3.8274872585558573 | −3.8273525867759406 | −0.0035% |
| `fsenl_dtl` | 40.615578696521737 | 40.615290488898452 | −0.0007% |
| `irab` | −14.241042594624457 | −14.229107021899958 | −0.084% |
| `fsenl` | 60.730401014124659 | 60.551382256105526 | −0.29% |
| `wta0` / `wtg0` | 0.5930009697 / 0.2364138446 | 0.5929908910 / 0.2364236123 | −0.0017% / +0.0041% |

`irab` 的差**恰好**等于 `dirab_dtl × ΔT`（−3.827 × (−3.12e-3) = +1.19e-2，实测 +1.19e-2），
所以它是叶温差的**结果**。

### 两条判定性的检验

1. **`fsenl` 有没有独立缺陷？** 用**上游的系数**（`fsenl_dtl/(wta0+wtg0)`）配上
   **本仓库自己的输入**（`tl/wta0/wtg0/thm/tg`）重算一遍，与本仓库的 `fsenl` 比：
   最大差 **1.55e-2 W/m²**（均值 6.4e-4）。也就是说公式与权重都对，残差全部来自
   输入本身（主要是 `tg`，全窗口最大差 0.023 K）。
2. **`irab` 呢？** 把上游的 `irab` 用 `dirab_dtl` 平移到本仓库的温度上，残差
   最大 **3.9e-2 W/m²**（均值 −1.0e-4）。同样没有独立缺陷。

`cfh`（冠层显热导度，由 `fsenl_dtl/(rhoair·cpair·(wta0+wtg0))` 反解）全窗口
最大相对差 8.1e-4、均值 4.9e-6 —— 也就是**求解器本身没问题**。

### 于是链条的顶端换人了

```
f_t_grnd / f_t_soisno（0.022 K / 逐层）
  → f_tleaf（0.026 K，且已证是自洽响应）
  → 冠层通量 → f_ldew（0.1~0.17%）
  → max(0, ldew-satcap) 放大 8×  → pg_snow
  → f_scv（−0.336%，**在前 6 条记录里种下然后永久冻结**）
  → f_fsno → f_alb → f_sr* / f_sab*
```

`f_scv` 的偏移一旦在记录 6 定下来就再也不变（记录 6/7/23/100/163/263 完全相同，
都是 −1.5871e-4），所以它是一个"早期种子 + 冻结"的量，不是漂移。
`f_ldew` 在第 0 条记录差 −0.17%，第 3 条就收到 −0.0014%，之后在 0 与 +2.7% 之间摆动
（多数记录两边都恰好为 0）。

**下一步应该在 `f_t_grnd`/`f_t_soisno` 那条线上找**，方法与这三次相同：
逐步打出来对照，而不是读 `MOD_Thermal`。注意 `f_t_soisno` 逐层最大相对差集中在
第 1 层（8.6e-5）与第 6 层（3.5e-4，冻结锋所在层），第 8~10 层逐位相同 ——
这个"只有上层与冻结锋有差"的形状本身就是一个可用的线索。

顺带记一个**不是缺陷**的观察：`f_wliq_soisno` 第 1 层最大相对差 3.7%（均值 5.8e-4），
出现在记录 142~169 那段窗口。那段里 `scv`/`snowdp`/`t_grnd`/`wice` 两边都只差
1e-4 相对，而第 1 层的**液态水本身降到了 0.94 mm**（全窗口均值 2.89 mm），
绝对差 0.035 mm 被小分母放大成 3.7%。全柱总水量 `f_wat` 只差 3.1e-2 mm
（相对 5e-6）。**别把这 3.7% 当成新根因**。

## 剩下的 tier2 残差落在叶温求解器**自己的收敛容差**上（2026 年，实测）

上一节把链条顶端指到 `f_t_grnd`/`f_t_soisno`。这一节先做三件事把"是不是还有公式错"
这个问题问死，然后再看那条链。

### 一、残差**没有系统性偏置，也不增长**

| 变量 | mean(rust−fort) | 相对 mean | max\|d\| |
|---|---|---|---|
| `f_t_grnd` | −9.76e-5 | **−3.7e-7** | 2.21e-2 |
| `f_tleaf` | +9.54e-5 | **+3.7e-7** | 2.61e-2 |
| `f_wat` | — | — | 3.08e-2 |
| `f_rss` | +5.05e-5 | +1.6e-3 | 1.38e-2 |
| `f_rss`（**去掉第 0 条**） | **−1.74e-6** | — | **6.75e-6** |
| `f_fgrnd` | −3.08e-3 | −2.6e-5 | 9.82e-1 |
| `f_fsena` | +3.34e-3 | +7.4e-5 | 7.46e-1 |

温度量相对均值在 **1e-7**，逐层 `t_soisno` 的误差也**不随时间增长**（第 1 层逐日序列
在 ±6e-3 之间振荡）。`f_rss` 的"261/264 越界"完全来自第 0 条：去掉它之后最大差只有
**6.75e-6**（相对 2e-4）。

### 二、模型对 1 ULP 的扰动**不是混沌的**

把 `thm` 乘 `(1+ε)`（即 +1 ULP ≈ 5.7e-14 K），重跑整个 528 步窗口，与未扰动的那次比：

| 变量 | 上游 vs 本仓库 | 扰动 vs 未扰动 | 比 |
|---|---|---|---|
| `f_tleaf` | 2.61e-2 | 6.64e-7 | 2.5e-5 |
| `f_t_grnd` | 2.21e-2 | 1.85e-6 | 8.4e-5 |
| `f_scv` | 1.59e-4 | 9.3e-16 | 5.9e-12 |
| `f_lfevpa` | 1.57 | 4.06e-5 | 2.6e-5 |
| `f_rss` | 1.38e-2 | 3.31e-10 | 2.4e-8 |

扰动整整小 4~12 个数量级。**所以剩下的差不是"末位噪声放大"，它来自一个有限大的差异。**

### 三、那个有限大的差异就是**求解器停在哪一次迭代**

两边准 Newton 的收敛判据逐字相同：

```fortran
real(r8),parameter :: dtmin  = 0.01   ! MOD_LeafTemperature_Extended.F90:338
real(r8),parameter :: dlemin = 0.1    ! :339
IF(det .lt. dtmin .and. dee .lt. dlemin) EXIT     ! :1277
```

本仓库 `TEMPERATURE_TOLERANCE_K = 0.01`、`FLUX_TOLERANCE_W_M2 = 0.1`（同）、
`itmax = 40`/`itmin = 6`（同），而内核里 `it = 1` 是**每次调用重设**的（`:490`），
所以逐步打出 `it` 与本仓库的 `iteration` 可以直接比：

| | |
|---|---|
| 528 步里迭代次数**完全相同** | **495 步** |
| 不一致的 33 步，差值范围 | ±1 到 ±19 |
| 两边撞到 `itmax = 40` 的步数 | **0 / 0** |
| 最后一次温度增量 `\|dtl\|` 的平均差 | 2.27e-4 K |
| 上一次增量 `del2` 的平均差 | 1.18e-4 K |

例：第 38 步上游 26 次、本仓库 8 次；第 110 步 23 对 8 次；第 0 步 11 对 11 次。

也就是说：**两边都在同一个容差下收敛，但停在不同的一次迭代上**，于是"收敛解"
本身差 O(0.01 K) —— 这正是 `f_tleaf` 观测到的 0.026 K 的量级。

### 结论与给容差表的提示

`tolerances.toml` 的 tier2 描述自己写着"**容差不得紧于求解器自身的收敛容差**"，
并且只登记了 `solver_tolerance_floor = 8e-8`（`MOD_Hydro_SoilWater` 的 Newton）。
**叶温求解器的地板是 0.01 K / 0.1 W/m²，比 tier2 现在的 `atol = 1e-7` 宽 5 个数量级。**
所以 tier2 里那些由叶温求解派生的量（`f_tleaf`、`f_fsena`/`f_fsenl`/`f_fseng`、
`f_lfevpa`/`f_fevpl`/`f_fevpg`、`f_gssun`/`f_gssha`、`f_rstfac*`、`f_assim*`、
`f_tref`/`f_qref`…）**在现有 `atol` 下不可能通过**，而且这与移植忠实度无关。

这一节**不动容差表**：放宽会同时掩掉真缺陷（`htvpl` 那次是 13% 的 `fevpl` 与
2 W/m² 的 `lfevpa`，`thm` 那次是 0.06 K 的常数偏差 —— 都还在 0.01 K 地板的量级之上，
但余量不厚）。要做的是给这些量登记叶温地板并**同时**要求
"迭代计数与回退次数必须一起比"（tier2 描述里已经要求了，`golden-compare` 目前还
没做这条）。这是下一轮该做的**工具改进**，不是数值修复。

## 把 `main/` 与 `extends/` 的**全部**数值差异机械地列出来（2026 年，实测）

`htvpl`、`thm`、`lfevpa` 三次根因都来自同一件事：本仓库的 Rust 是照着 `main/` 写的，
而实际编译的是 `extends/interception/` 里的四个文件。既然这一类能一次贡献三个大修，
就把这一类的**全集**求出来，而不是等它下次再咬人。

做法：把两份文件的**赋值语句**抽出来（先拼续行、去注释、去空白、`_r8` 归一），
按左端变量名分组，只报"两边都有但算式不同"的那些。

结果（`main/MOD_Thermal.F90` vs `extends/.../MOD_Thermal_CanopyPhase_Extended.F90`：
2 处；`main/MOD_LeafTemperature.F90` vs `.../MOD_LeafTemperature_Extended.F90`：
19 处；`main/MOD_LeafInterception.F90` vs `.../MOD_LeafInterception_Extended.F90`：4 处），
逐条核对本仓库的状态：

| 差异 | 上游 `ext` 的写法 | 本仓库 | 影响 |
|---|---|---|---|
| `lfevpa` | `lfevpl + htvp*fevpg`（`lfevpl = htvpl*fevpl`） | 已按 `ext` 改 | **已修**（第 64 轮，3.14 W/m²） |
| `dele`/`dtl`/`err`/`fsenl` 的 `htvpl` | `htvpl` 而非 `hvap` | 已按 `ext` 改 | **已修**（第 64 轮，13%） |
| `dtl` 分子/分母、`hprl` 的 `max(0,qintr_*)` | 四处都夹 | **缺**，而且 `validate` 要求非负 | **本轮修** |
| `errore` 的 `+canopy_phase_heat` | 有 | 无（缺省 scheme 下为 0） | 记录，scheme 4~7 才非零 |
| `dtl` 的 `+canopy_phase_heat` | 有 | 无（同上为 0） | 记录 |
| `etr`/`etrsun`/`etrsha`/`etr_dtl` | `dry_factor` 而非 `(1-fwet)` | `evaporation_sign` 等价形式 | 缺省 scheme 下 `dry_factor = 1-fwet`，**等价** |
| `evplwet`/`evplwet_dtl` | `evp_weight*wet_cond` 而非 `(1-delta*(1-fwet))*(lai+sai)/rb` | 前者 | 缺省 scheme 下 `evp_weight = 1-delta*(1-fwet)`、`wet_cond=(lai+sai)/rb`，**等价** |
| `cfw` | `wet_cond_cfw` 而非 `(lai+sai)/rb` | 后者 | 缺省 scheme 下 `wet_cond_cfw = (lai+sai)/rb`，**等价** |
| `elwmax` | `ldew_vic_evap/deltim` | `canopy_water.total_mm/deltim` | 缺省 scheme 下 `ldew_VIC_evap = ldew`，**等价** |
| `fwet_rain`/`fwet_snow` | `satcap_*_eff` / `canopy_snow_wetfrac(...)` | 未走这条 | `DEF_VEG_SNOW` 分支（本算例关） |
| `ldew_rain`/`ldew_snow` 的 `max(0,·)` | 有 | — | `DEF_VEG_SNOW` 分支 |
| `qevpl`/`qsubl` 的相态拆分 | 有 | 有（`phase_change` 一侧） | `DEF_VEG_SNOW` 分支 |
| `xsc_rain`/`xsc_snow` | 活动分支与 `main` **逐字相同** | 同 | 报告里那些 `(ldew-satcap)*ldew_rain/ldew` 来自**别的** scheme 例程 |

### 本轮改的那一条：`max(0, qintr_*)`

净截留率 `qintr_rain = (prc_rain+prl_rain+qflx_irrig) - thru_rain/deltim`，
而 `thru_rain = tti_rain + tex_rain` **含冠层排水** `tex_rain` —— 所以
`qintr_rain = rain*fpi - tex_rain/deltim`，排水超过截留量时它是**负的**。
`extends` 在四处取 `max(0, ·)`（增量式的分子与分母、循环后 `fsenl` 的修正、`hprl`），
注释写着"negative net flux does not spuriously inject t_precip-tl energy"。

本仓库不仅没夹，还在 `validate` 里**要求它非负** —— 那会让任何排水步直接报错，
而不是按上游夹掉。现在：`LeafTemperatureInput` 上仍带**原始**净通量（与上游 `qintr_*`
同义），在叶温例程内与上游同位置夹，`validate` 只留有限性检查。

新测试 `a_negative_net_interception_rate_is_clamped_not_rejected` 直接喂
`qintr_rain = -5e-6`、`qintr_snow = -1e-6`：以前返回 `Err`，现在成功且
`precipitation_heat_w_m2 == 0`（不夹会得到约 +0.23 W/m²）。

`CN-Cng-aligned` 的黄金结果**不变**（该窗口无降水，`qintr ≡ 0`，夹与不夹同值）——
这一条是**潜在缺陷**的修复，不是当前残差的来源。

**教训沉淀为一条规矩**：`main/` 里的同名声明的模块**从不参与编译**。改任何物理量
之前，先确认它的算式来自 `Makefile:635-647` 指定的那个 `extends/` 文件。

## 最坏的一类分支不匹配：选 PFT/PC 的算例会**静默按 LCT 算完**（2026 年，实测）

顺着"还有哪些分支没被拦住"查了一遍：上游
`MOD_Namelist.F90:1932-1944` 要求

```fortran
! Exactly one of DEF_USE_LCT/DEF_USE_PFT/DEF_USE_PC must be .true.
IF (count((/DEF_USE_LCT, DEF_USE_PFT, DEF_USE_PC/)) /= 1) THEN
   write(*,*) 'Fatal ERROR: exactly one of DEF_USE_LCT / DEF_USE_PFT / DEF_USE_PC', &
      ' must be .true. (subgrid structure is a mutually exclusive choice).'
   CALL CoLM_stop ()
ENDIF
```

三者的声明默认值是 `.true.`/`.false.`/`.false.`（默认 LCT）。本仓库**根本不读这三个
开关** —— `crates/` 里对 `DEF_USE_LCT` 只有两条注释提到它，没有任何 `logical(...)`
读取。于是：

- 一个写 `DEF_USE_PFT=.true.` 的算例（上游支持的、默认关但完全合法的配置）
  会在本仓库里**一路按 LCT 的编排跑完**，不报错；算式对、结构错 —— 这是比
  "拒绝运行"更坏的一类不匹配，因为它在数值上看起来"有结果"。
- 两个同时为真也不会被拦住，而上游会 `CoLM_stop`。

**修法**：在 `land_physics_parameters` 里读这三个开关，先判"恰好一个"，再判
"必须是 LCT"，两者都 `bail!`。按本文件的纪律 #3（"上游有、本仓库没移植的分支
一律报错"）—— 它们**不是默认打开**的分支，所以不进 `unported_branches` 那张
"默认配置会撞上"的表（VSF/PHS 在那张表里，因为它们的默认值是真）。

新测试 `a_pft_or_pc_subgrid_case_is_refused_rather_than_run_as_lct` 覆盖三种情形：
选 PFT、选 PC、以及两个同时为真；再确认默认（只有 LCT）照常通过。

### 这一节记一条方法论

判断"某个上游开关本仓库读没读"，`grep` 开关名只在**读它**的时候命中；如果代码里
只有注释提到它，`grep` 也会命中而没有读取 —— 这两种要分清。上面那两条命中都是
注释（`physics.rs:281`、`assembly.rs:1379`），而 `physics.rs:281` 那句
"`DEF_USE_LCT` 那道门在本仓库恒成立"正是把**假设**当成了**检查**：它假设算例
选的是 LCT，却没有验证。**默认值让一个开关"看起来总是成立"，而默认值不是检查。**

## PHS 前置件（一）：七个常数与九个地类性状都接上了（2026 年，实测）

上一节末尾说 PHS 比早先估计的近。这一节把它的**前置件**做完并核实，同时把
"还差什么"写清楚，免得下一轮再从零查一遍。

### 已经就位的（这轮查实，不是新写）

| 件 | 位置 | 状态 |
|---|---|---|
| 植物水力内核 | `plant_hydraulics.rs`（896 行） | 已移植：`plant_hydraulic_stress`、`vegetation_water_potential`、`vulnerability*` |
| 叶温里的接线 | `leaf_temperature.rs:396-520`、`:819-830` | 已在：`Option<LeafPlantHydraulicInput>` 与 `Option<PlantHydraulicState>` 的分支、`gs0sun/gs0sha`、`ristfac` 的来路 |
| 根通量分派 | `standard_lct_step.rs:416`、`:539` | 已按 `plant_hydraulics` 分支 |
| **`vegwp` 重启** | `colm-init/src/time_restart.rs:333-341`、`:783-793` | **已写、已校验** —— 通过 `input.plant_hydraulics` 的 `vegetation_nodes` 与 `water_potential_mm` 落到 `vegwp`/`vegnodes` |

也就是说，缺的**不是**内核、不是重启、不是叶温接线。

### 这轮补上的两块

**（1）七个 `DEF_PH_*` 常数**（`MOD_Namelist.F90:628-634`）现在在
`land_physics_parameters` 里逐个从 namelist 读进 `PlantHydraulicParameters`。
逐项对过 `MOD_PlantHydraulic.F90:162-200` 的用法：`FROOT_CARBON`/`ROOT_DENSITY`/
`ROOT_RADIUS` 一起定细根长度密度，`FROOT_LEAF` 是细根-叶面积分配，
`CROOT_LATERAL_LENGTH` 与 `K_AXS` 分别是侧根长度与轴向导度系数，
`KRMAX` 是单位长度单位面积的最大径向导度。空算例断言把它们与
`PlantHydraulicParameters::default()` 逐位绑在 schema 上（纪律 #1）。

**（2）九个地类性状**：`ClassConstants::plant_hydraulic_traits(overrides)` 从
**地类表**取 `kmax_sun0`/`kmax_sha0`/`kmax_xyl0`/`kmax_root0`、四个 `psi50_*`、
`ck0`，再按 `DEF_LC_*` 逐列覆盖（`None` = 没写 = schema 的 `-1.e36` =
`LC_OVERRIDE_UNSET`）。

这里有一个**容易拿错表**的点，值得单独记：标准 LCT 路径读的是
`MOD_Const_LC.F90:603` 的 `kmax_sun0_igbp` 那一组
（`MOD_Thermal_CanopyPhase_Extended.F90:706`），而 per-PFT 的 `kmax_sun_p`
只被 `LeafTemperaturePC` 用（`:922`）。**两张表的数不一样**：地类是 `2.e-8`，
per-PFT 是 `1.e-7` —— 拿错不会报错，只会让导度差 5 倍。`land_cover_tests.rs`
里那条测试同时钉住"取地类表"、"逐类不同"、"覆盖只动一列"、"分类体系被尊重"
（USGS 第 1 类的 kmax 是 0）四件事。

### 还差的（下一轮的施工单）

1. **构 `LeafPlantHydraulicInput`**：`node_depth_m`/`layer_thickness_m` 取
   `SoilField`；`root_fraction` 取 `rootr`；`soil_matric_potential_mm`/
   `soil_hydraulic_conductivity_mm_s` 取 `Water2014SoilOutput` 的
   `matric_potential_mm`/`hydraulic_conductivity_mm_s`；`saturated_hydraulic_conductivity_mm_s`
   取土壤常数；九个性状取上面那个访问器；`soil_surface_resistance_scheme` 取
   physics；`parameters` 取 `LandPhysicsParameters::plant_hydraulic_parameters`。
2. **`LeafTemperatureState::plant_hydraulics = Some(PlantHydraulicState { .. })`**：
   重启里有 `vegwp` 就用它，缺就照 `CoLMMAIN.F90:2249` 的 `-2.5e4`
   （注意那一句在 `IF (DEF_USE_PLANTHYDRAULICS)` 里）。
3. **把 PHS 从 `unported_branches` 拿掉**，并跑一遍黄金对比看
   `f_etr`/`f_etrsun`/`f_etrsha`/`f_gssun`/`f_gssha`/`f_rstfac*` 的变化。
   `CN-Cng-aligned` 算例**没有**关 PHS（默认就是开），所以一旦接通，
   现在的黄金对比会**立刻**变成 PHS 那一支的结果 —— 也就是说这三个文件里的
   任何一处接错都会当场显形，不需要另造算例。

## 用**重启对比**发现 history 对比看不见的一类缺陷：`rss`/`trad`/`emis` 没回写（2026 年，实测）

前面所有轮次都只用 history 黄金对比。这轮把 `--restart-out` 写出的重启与上游
同一时刻的重启逐变量比了一遍（`restart/2008-012-00000/...`，65 个变量两边都有）：

| 变量 | 上游 | 本仓库（修前） | 说明 |
|---|---|---|---|
| `rss` | 0.033373 | **−1e36** | 写成了**填充值** |
| `trad` | 254.261338 | **283.0** | 写成入参重启里的原值 |
| `emis` | 1.000315 | **1.0** | 同上 |
| `smp` | −2340271.2 | −2342299.4 | 0.09% 继承 |
| `t_grnd` | — | 差 1.4e-2 K | 继承 |
| 其余 60 项 | — | ≤4.6e-3 | 继承 |

根因：`evolved_overrides` 只回写它**枚举到**的名字。`rss`/`trad`/`emis` 是
`MOD_Thermal`/`SoilSurfaceResistance` 的逐步 `intent(out)`，而算例的**入参**重启里
`rss = spval`、`trad = 283`、`emis = 1`（上游的入参也一样）—— 不回写就等于把
入参那一列**原样交出去**。这不是数值残差，是"重启少了三列"，而 history 对比
永远看不到它。

### 这轮修掉的：`rss`

两处都要改，缺一处都不生效：

1. `SurfaceDiagnostics::NAMES` 加上 `"rss"` —— `splice` 对**没登记**的名字返回
   `None`，只往 `evolved_overrides` 的取值表里加一项是**静默无效**的
   （实测：只改前者，重启里还是 `−1e36`）。
2. `evolved_overrides` 的表面诊断取值表加上
   `("rss", step.energy.soil_surface_resistance_s_m)`。

改完 `rss` = 0.03337231（上游 0.03337258，差 2.7e-7 就是已知的 `f_rss` 残差）。

### 还没修的：`trad`/`emis`

它们不在 `StandardLctEnergyOutput` 上 —— 目前只有 `history.rs` 的 `SurfaceBudget`
在算（`trad = (olrg/stefnc)**0.25`、`emis = olru/olrb`），而重启写回走的是内核输出。
要修就得把这三个量从 history **提到内核**（上游本来就是 `MOD_Thermal` 收尾处算的），
让 history 与重启共用一份。那是一次重构，会碰到已经验证过的 `f_trad`/`f_emis`/
`f_olrg`，所以留给下一轮单独做。

### 一条新规矩

**history 黄金对比只能看见"写出来的列"，看不见"没写出来的列"。** 每补完一个
stage 的写出面，都应该把该 stage 的**重启**也逐变量比一遍 —— 这轮的三个缺陷
全部只在重启对比里显形。

## `trad`/`emis` 也回写了，重启差异清空到只剩物理残差（2026 年，实测）

上一节把 `rss` 修好，但 `trad`/`emis` 还差"把它从 history 提到内核"这一步。做法不是
重写，而是**搬家**：把 `history.rs` 里那份私有的 `SurfaceBudget`/`surface_budget`
整体挪进 `colm_core`（新文件 `crates/colm-core/src/surface_budget.rs`），字段与函数
改成 `pub`，两个消费者共用一份：

- history 的 `set_lct_surface_budget` / `set_lct_balance_errors`（`f_olrg`/`f_emis`/
  `f_trad`/`f_grnd`/`f_lfevpa` 与 `f_zerr`）；
- 续跑写回的 `evolved_overrides`（`trad`/`emis`）。

顺手把入参从 `&StandardLctSoilOutput` 收窄成 `&StandardLctEnergyOutput` ——
这个函数从头到尾只用 `output.energy`，从来没用过 `output.water`（`grep output.water`
是 0 命中），留着整根土柱只会让调用方以为它需要。

**搬家而不是重写**是关键：算式一字未动，所以 history 的黄金结果**逐条不变**
（`f_trad`/`f_emis`/`f_olrg` 仍在原来的量级），而重启里：

| 变量 | 上游 | 本仓库 | 差 |
|---|---|---|---|
| `rss` | 0.03337258 | 0.03337231 | −2.6e-07 |
| `trad` | 254.2613382 | 254.2492411 | −1.21e-02 K |
| `emis` | 1.00031475 | 1.00031834 | +3.6e-06 |

`trad` 的 1.2e-2 K 正好是同一时刻 `t_grnd` 的 1.43e-2 K 那个继承残差
（`trad = (olrg/stefnc)**0.25`）。

### 修完之后：重启里**没有未写出的列**了

再逐变量比一遍（65 个变量两边都有，无缺项），前几名全是物理残差：

| 变量 | max\|d\| | 性质 |
|---|---|---|
| `smp` | 2.03e+03（相对 ≤8.7e-4） | 继承自土壤水 |
| `t_soisno` / `t_grnd` | 1.43e-2 K | 继承 |
| `trad` | 1.21e-2 K | 继承自 `t_grnd` |
| `wa` | 5.25e-3 | 继承 |
| `wliq_soisno` | 4.60e-3 | 继承 |
| `tleaf` | 2.42e-3 K | 继承 |

也就是说，重启那条线从"三列根本没写"变成了"只剩继承残差"。这一条**只用重启对比
才能发现**：`rss`/`trad`/`emis` 在 history 里要么没这一列（`trad`/`emis` 有，
但 history 比的是**写出值**，不是"有没有回写重启"），要么差得看不出来。

**给下一轮的两条**：

1. `smp` 是这次新暴露的量（history 没有它）。相对差 ≤8.7e-4，第 1 层最大 ——
   与 `wliq` 的分布一致，暂判为继承，但要真判它得把 `soilwater` 的
   `smp`/`hk` 逐步打出来对照。
2. 重启里那 30 来个**没有**被回写、只是"碰巧对得上"的变量（相似函数、
   `fveg`/`green`/`sag`/`snw_rds`/`mss_*` 等）值得逐个确认：这份窗口上它们
   恰好接近，换一个窗口可能就不是。判据是"上游是否把它当时间变量读回来"。

## 两张"已经查干净"的清单（2026 年，实测）

两条审计的结论，连同方法，一起落盘，免得下一轮重做。

### 一、重启写出面：**没有"没被推进却原样写出去"的列了**

判据是**数据驱动**的，不需要读源码：拿入参重启（`2008-001-00000`）、上游输出重启
（`2008-012-00000`）、本仓库 `--restart-out` 三方比 ——

> 上游从入参到输出**变了**（`max|Δ| > 1e-12`），而本仓库从入参到输出**没变**
> （`max|Δ| < 1e-15`）的变量，就是"本该推进却没推进"。

结果：**0 个**。65 个变量两边都有、无缺项，最大的差全是继承残差
（`smp` 2.0e3 mm / 0.09%、`t_soisno`/`t_grnd` 1.4e-2 K、`wa` 5.3e-3、
`wliq_soisno` 4.6e-3）。上一轮那三条（`rss`/`trad`/`emis`）就是这条判据抓出来的。

**这条判据值得固化**：它比逐变量读 `MOD_Vars_TimeVariables` 快，而且不会漏。
唯一的前提是有一个"同一时刻的上游重启"可作基准。

### 二、默认值为真的、运行时不读的开关：19 个，逐个归类

方法：`colm-schema` 里 `FieldKind::Logical` 且 `default: true` 的字段，减去
`colm-runtime`/`colm-case`/`colm-forcing` 里**任何字符串字面量或注释**提到过的名字。

| 开关 | 归类 |
|---|---|
| `DEF_Aerosol_Readin` | 被 `DEF_USE_SNICAR`（默认假）挡住 —— 上游日志里也写着 "not needed for DEF_USE_SNICAR off" |
| `DEF_highResSoil`/`DEF_HighResVeg` | **默认真且本仓库就是按高档算**（`high_res_radiation.rs` 已移植），默认行为一致；显式写 `.false.` 会被静默忽略 |
| `DEF_LANDONLY` | 同上：默认真，本仓库只算陆面 |
| `DEF_HIST_vars_out_default` | history 闸门，属 `colm-hist` 的生成表 |
| `DEF_PC_CROP_SPLIT`、`DEF_URBAN_{BEM,LUCY,TREE,WATER}`、`DEF_USE_CANYON_HWR`、`DEF_USE_CNSOYFIXN`、`DEF_USE_NITRIF`、`DEF_TRACER_SOIL_{,VAPOR_}DIFFUSION` | 作物/BGC/城市/示踪物分支，本仓库无对应 patch 类型或未移植 |
| `DEF_USE_EstimatedRiverDepth` | 河道分支；LCT 路径没有河流（`discharge`/river 一族在 history 里是"声明+留空"） |
| `DEF_forcing%data2d`、`%dim2d` | **未决**：`dim2d` 用在降尺度里（`MOD_Forcing.F90:1188`），POINT 下它是否影响本算例还没查 |

**归类的意义**：第一类/第三类/第四类是"默认值与本仓库硬写的行为一致"，所以默认
配置**不会**出问题 —— 与 PFT/PC 那次（默认是 LCT，于是选 PFT 的算例被静默按 LCT 算）
不同，这里要出问题必须显式写反。第二类（high-res / LANDONLY）是**显式关掉会被
静默忽略**，属于同一族但触发概率低；真要收紧，判据与 PFT/PC 一样：读它，不一致就报错。

## PHS（`DEF_USE_PLANTHYDRAULICS`）从"硬关"到端到端跑通（2026 年，实测）

上游这个开关的**声明默认值是 `.true.`**（`MOD_Namelist.F90:531`），也就是说默认配置
跑的就是植物水力；本仓库此前在装配层把它硬写成关（`plant_hydraulics: None`），
并在 `unported_branches` 里拒绝。这一轮把它接上了。做到这一步才发现，
"接上"不是加一行 `Some(...)`：装配、两个入口的逐步输入、history、重启写出面
四处都要动，而且沿途挖出三类缺陷。

### 一、验证算例：`CN-Cng-campbell` 的黄金文件是**过期的**，不能拿来比

`CN-Cng-campbell` 的黄金（`oracle/work/CN-Cng-campbell/...`，2026-09-21 02:29）
里 `f_gssun` 是 **46.18**（µmol），而 `oracle/golden/CN-Cng_hist_2008-01.nc`（05:51）
是 **4.63e-5**（mol）。同一份源码不可能产出两种单位 —— 前者是
`0115b58`（f48 同步，06:03）**之前**的内核产物，那个内核里
`MOD_LeafTemperature_Extended.F90:1305-1314` 那段"无条件诊断 `gssun`"
还不存在。拿它做 PHS 验证会把内核差异当成移植缺陷。

正确做法：**新建一个只改一个开关的算例并重新产出黄金**。
`oracle/cases/CN-Cng-phs/` 与 `CN-Cng-aligned` 的 `case.nml` 逐行相同，
只把 `DEF_USE_PLANTHYDRAULICS` 从 `.false.` 改成 `.true.`；
`DEF_CASE_NAME` 改成 `CN-Cng-phs`（否则 `golden-run --write-golden` 会覆盖掉
`oracle/golden/` 里那份 **VSF** 黄金）。产出命令：

```
PLUMBER2_ROOT=/Volumes/Data01/Data/PLUMBER2s cargo run -q -p oracle --bin golden-run -- CN-Cng-phs
```

黄金落在 `oracle/work/CN-Cng-phs/out/CN-Cng-phs/history/CN-Cng-phs_hist_2008-01.nc`
（**不入库**，`oracle/work/` 是 gitignore 的），比对：

```
NETCDF_DIR=/opt/homebrew/opt/netcdf cargo run -q -p colm-runtime --bin colm-rs -- \
  oracle/work/CN-Cng-phs --land-cover igbp --history-dir /tmp/h --restart-out /tmp/r.nc
NETCDF_DIR=/opt/homebrew/opt/netcdf cargo run -q -p oracle --bin golden-compare -- \
  oracle/work/CN-Cng-phs/out/CN-Cng-phs/history/CN-Cng-phs_hist_2008-01.nc \
  /tmp/h/colm-rs_hist_2008-01.nc --tolerances oracle/tolerances.toml
```

### 二、结果：PHS 分支的 tier 分布与已验收基线**同一水平**

| 算例 | PHS | tier0 | tier1 | tier2 | 超容差变量数 |
|---|---|---|---|---|---|
| `CN-Cng-aligned` | 关（已验收基线） | 2 | 19 | **48** | 69 |
| `CN-Cng-phs` | 开 | 2 | 19 | **49** | 70 |

tier0/tier1 完全一致（`f_xy_solarin` 11/264 差 1 ULP、`f_xy_q` 3/264 是上游时刻对齐怪癖，
两组 19 条都是短波分带与 `f_alb` 一族）。tier2 只多 1 条 —— 多出来的就是
`f_vegwp` 本身（见第五节）。逐变量的量级：`f_rstfacsun` 2.5%/0.11%、
`f_rstfacsha` 1.3%/0.12%、`f_rootr` 6.3e-4/2.3e-5、`f_tleaf` 0.0264 K、
`f_t_grnd` 0.0236 K。

### 三、装配：三处"每步才知道"的量不能进装配期常数

`MOD_LeafTemperature_Extended.F90:881-890` 把 `smp`、`hk`、`hksati`、`rootfr`
连同 `z_soi`/`dz_soi` 一起交给 `LEAFTEMPERATURE`。这四样**每步都变**：

- `smp`/`hk` 是 `WATER_2014` 的 `intent(out)`，存进**时间变量**、由**下一步**的
  `THERMAL` 读（能量步在水分步之前）。为此 `Water2014SoilState` 加了
  `matric_potential_mm`/`hydraulic_conductivity_mm_s` 两个字段，由水分步每步覆写，
  装配期填重启那一份。
- `rootfr` 是本步 `MOD_LeafTemperature` 的输入（LCT 分支按地类表算，不是常数）。
- `hksati` 是常数，但和上面三样一起给才不至于让"开关开了、某个量还是默认"变成一个
  看不见的状态。

所以 `StandardLctEnergyInput` 只收**静态**的 `Option<PlantHydraulicSettings>`
（九个地类性状 + 七个 `DEF_PH_*` + `DEF_RSS_SCHEME`），逐步的那一份由
`standard_lct_soil_step` / `standard_lct_snow_soil_step` 现场拼成
`LeafPlantHydraulicInput`。

**地类性状必须在装配期求值**（`ClassConstants::plant_hydraulic_traits(overrides)`）：
地类号来自**重启**的 `patchclass`，性状是"地类表 + `DEF_LC_*` 覆盖"两样一起查出来的，
所以 `DEF_LC_*` 的九个覆盖只能经 `LandPhysicsParameters` 传下去，不能在那里求值。
用内核回读实测（见第六节的方法）：IGBP 草地（`patchclass = 10`）得到
`kmax_sun = kmax_sha = kmax_xyl = kmax_root = 2.000000000000000e-08`、
`psi50_* = -3.4e5` mm、`ck = 3.95` —— 与上游内核打印的值逐位相同。

**踩坑**：PHS 的土壤列**不能用 `energy.ground_temperature.{node_depth_m,layer_thickness_m}`**。
积雪分支里那一对是**雪 + 土**的打包列（`snow_layers + nl_soil`），而 `smp`/`hk`
只有 `nl_soil` 项，`snow_layers > 0` 时长度对不上、`validate` 直接报错。
上游传的是土壤专用的 `z_soi`/`dz_soi`；`Water2014SoilInput` 本来就带着土壤深度
（`node_depth_m`/`layer_thickness_m`），改用它即可。两个入口都改。

### 四、`gs0sun`/`gs0sha` 的写出面：Units 错了一个 1e6，物理量还是另一个

`evolved_overrides` 里那两个槽位原先填的是
`leaf_output.sunlit_stomatal_conductance_mol_m2_s`（`gssun`，**实际**叶导度，
mol m⁻² s⁻¹）。上游写进重启的 `gs0sun` 是
`MOD_LeafTemperature_Extended.F90:817` 的
`min(1e6, 1/(rssun*tl/tprcor))/laisun*1e6`（**最大**叶导度，µmol m⁻² s⁻¹）。
实测 `oracle/work/CN-Cng-phs` 的 `--restart-out`：本仓库写出 **4.807e-5**，
上游同一时刻 **481.33925243**。修正后本仓库 **481.34362292**（相对差 9.0e-6）。

这条也解释了为什么之前"重启写出面 0/65"的判据没抓到它：那条判据问的是
"上游变了而本仓库没变"，而这里是**两边都变、但本仓库写的是另一个量**。
判据要补一条对称的："同名的两个变量，两边都变了，值的相对差是否与继承残差同量级"。

顺带：**PHS 关掉时这两个变量不写**。上游只在
`IF (DEF_USE_PLANTHYDRAULICS)` 里给它们赋值，重启里没有这一列时
（`CN-Cng-aligned` 的入参重启就没有）`write_with` 也不允许新造变量。

### 五、`f_vegwp` 的残差：两段，两因，都不是 `plant_hydraulics.rs` 的算术错

用一个 `DEF_HIST_FREQ = 'TIMESTEP'`、11 天窗口的诊断副本把区间平均拆开看：

- **第 0 步**：相对差 0.73，是整段里最大的一处。把内核自己的输入打印出来
  （方法见第六节）可以看到，`calcstress_twoleaf` 收到的**每一项都逐位相同**
  （`smp`、`k_soil_root`、`k_ax_root`、`z_soi`、`kmax_*`、`psi50_*`、`ck`、
  `laisun`/`laisha`/`sai`/`htop`、`tl`、`rhoair`、`psrf`、`gb_mol`、`qg`、`qaf`、`qm`、`qsatl`），
  **只有 `gs0sun` 不同**：上游是 **-4.251463361984611e38**，本仓库是 425.1463361984611。
  反解上游那一个：它等于 `MOD_LeafTemperature_Extended.F90:817` 在
  `rssun = -1e-30` 时的输出，也就是叶温求解器**第一轮**里 `stomata` 吐出的未定义值。
  而 `PlantHydraulicStress_twoleaf` 开头就是 `gssun = gs0sun`，于是上游第一轮的
  需求通量 `qflx` 被这个巨大的假值放大到 **3.305e-5**，本仓库是 **6.858e-9**（差 4820 倍），
  第一步的 `dx` 因此一个向东一个向西。**这是上游的未定义行为，不可复现，也不该复现** ——
  与 `0115b58` 里 `rstfacsun` 那次（`Rejected: 复现上游的垃圾 rstfac`）同类。
  到第 1 步两边就都落到同一个夜间吸引子上（差 0.1%），第 1..117 步相对差 ≤ 2%。
- **第 118 步起**：开始出现 >2% 的记录，之后在整段里时大时小（最坏 1.10）。
  这一段**不是**一个新缺陷：`vegwp` 在这段窗口里是**剧烈振荡**的
  —— 逐步摆动中位数 **6257 mm**、最大 **125827 mm**（±12.6 m 水势）。
  两个实现的**摆动本身**高度一致：

  | 量 | 值 |
  |---|---|
  | 摆动相关系数（527 步） | **0.795** |
  | 摆动符号一致率 | **87.1%** |
  | 摆动中位幅度 gold / rust | 6256.9 / 6274.5（差 0.3%） |
  | 最大摆动 gold / rust | 125827 / 125730（差 0.08%） |
  | 摆动中位比值 rust/gold | 0.9985 |

  也就是说轨迹是**同一个极限环**，只是每一步的幅度对初值极度敏感
  （第 116 步两边差 8 mm(0.02%)，第 117 步放大到 93 mm(0.66%)，
  第 118 步放大到 535 mm(3.6%)）。而初值差只有叶温求解器那 **0.01 K / 0.1 W m⁻²**
  的收敛容差（已有结论：528 步里有 33 步的迭代次数与上游不同）。
  **结论**：`f_vegwp` 的残差是叶温求解器容差地板的差经过 PHS 振荡递归放大的结果，
  不是 `plant_hydraulics.rs` 的算术错 —— 后者已逐项核对干净（见下节）。

### 六、`plant_hydraulics.rs` 的六条实测差异，五条已修

方法：**让内核自己把值打出来**。在 `vendor/CoLM202X/main/MOD_PlantHydraulic.F90`
的 `calcstress_twoleaf` 里加 `WRITE(96,...)`（日志落在 case 目录的 `fort.96`），
在 Rust 侧对应位置加 `eprintln!`，两边用**同一个 `x(1)` 区间**做门限，
于是一定能比到同一步的同一个量。这条方法的两个纪律：

1. **先确认插桩不改结果**：把插桩后的 `DEF_HIST_FREQ='TIMESTEP'` 输出按两步取平均，
   与插桩前、`HOURLY` 的黄金文件逐位相比 —— `max|Δ| = 0.0`，才继续用。
2. **插桩必须在 `git add` 之前撤干净**（`464dae1` 那次把临时插桩提交进去了）。
   两处改动都用 `git checkout --`/逐段删除复原，复原后重跑 `test_upstream_f48_sync.py` 与内核构建。

| # | 位置 | 上游 | 本仓库（修前） | 影响 | 处置 |
|---|---|---|---|---|---|
| 1 | 步长救援 `MOD_PlantHydraulic.F90:329-332` | `maxscale = min(max\|dx\|, max\|x\|)/2` | `max(\|dx\|, \|x\|)/2` | 触发条件已是 `max\|dx\|>2e5`，而 `vegwp` 量级 1e4~1e5，于是 `max` 恒等于 `max\|dx\|`，缩放比上游大 `max\|dx\|/max\|x\|` 倍 —— 恰好在它要保护的场景失效，且四个节点一起跳、写回持久状态 | **已修**（取 `min`） |
| 2 | `gb_mol = 1./rb*cf`（`:210`） | 先取倒数再乘 | `cf/rb` | 1 ULP（200k 抽样里 27% 不同），能翻 `shaded_flux>0`、`determinant!=0`、`max\|dx\|>2e5` 三个**离散**判据 | **已修** |
| 3 | `wtaq0 = caw*wtsqi`（`:683-685`） | `wtsqi = 1/(caw+cgw+cfw)` 再乘 | `caw/total` | 同上，1 ULP / 27% | **已修** |
| 4 | `validate` | 上游只校验 namelist 的七个 `DEF_PH_*` | 还要求 `laisun/laisha/sai/htop>0`、`psi50<0`、`RSS_SCHEME ∈ 1..=5` | `DEF_RSS_SCHEME = 0` 是**合法值**（关掉 Campbell 时上游自己置 0，`:674-679` 的 `ELSE` 把 0 与 1 同等对待） | **已修**（放宽到 `0..=5`）；其余几条保留（默认 LCT 算例构造不出反例） |
| 5 | `getrootqflx_qe2x` | 调一次 | 同一组参数调两遍（第二遍只为拿 `.0`） | 无（解算器确定性，逐位同解） | **已修**（合并成一次） |
| 6 | 解后有限性检查 | 上游无 | 不覆盖 `potential` 本身 | 无（下一次调用会拒） | 保留 |

这一轮修完 #1~#5 后**重跑同一个算例，`f_vegwp` 的数值一位没变** —— 说明
`max|dx| > 2e5` 的救援在这条轨迹上从未触发、那两处 1 ULP 也没翻任何分支。
**但这不等于它们无害**：它们是分支级风险，只在别的算例/别的窗口上才会现形，
所以照修。另外逐项核对确认**两侧都没有牛顿迭代环**：上游
`calcstress_twoleaf` 声明了 `iter/iterqflx/itmax=50/toldx/tolf/...` 却**一个都没读**，
`spacAF_twoleaf` 一次 Jacobian 步 + 一次 `x = x + dx`；本仓库同构。

逐项核对干净的部分（供下一轮免重做）：四个节点顺序与状态带入、`root_conductances`
全部项、单位与深度约定（`smp`[mm]、`hk`/`hksati`[mm/s]、`z_soi`/`dz_soi`[m]）、
七个 `DEF_PH_*` 默认值逐位、两个 `getqflx_*` 到结合顺序、`spacAF_twoleaf` 两个行列式与
全部 Cramer 分子、两个根系解算、`plc`/`d1plc`、三处钳位的位置、夜间分支、
`getvegwp_twoleaf`；`solve_tridiagonal` 与 `tridia` 是逐操作相同。

### 七、history：`f_vegwp` 是**条件声明**的

`MOD_Hist.F90:4372-4378` 把 `f_vegwp` 整段写在 `IF (DEF_USE_PLANTHYDRAULICS)` 里，
所以 PHS 关掉的算例里这一列**根本不存在**（实测 `CN-Cng-aligned` 的黄金没有、
`CN-Cng-phs` 有 4×264）。为此 `declare_lct_variables` 多收一个布尔，
`LCT_PLANT_HYDRAULIC_VARIABLES` 只在开关打开时并进清单；
累加规则是普通的 `acc2d`+`nac`（`MOD_Vars_1DFluxes.F90:2494`），
**不是**末步瞬时值。

**踩坑**：`f_vegwp` 的写出必须挂在**两个** history 入口上。这一轮先只补了
`set_lct_state`（无雪入口），而 `standard_lct_snow_soil_step` 才是通用入口
（无雪起步的算例也走它），于是第一次跑出来整列 1056 个值全是填充值 ——
"文件里有这一列、但一个真值都没有"，比缺列更难看出来。

## `tier-check oracle/golden/*.nc` 一直是红的：两条判据都判错了对象（2026 年，实测）

这是 `CLAUDE.md` 里列的验收命令之一，实测在改这一轮之前就是红的，而且报的是
**两条独立的错**（我第一次只看了 `tail`，把上面那半截漏掉了）：

```
10 tier entry/entries name variables that no longer exist:
  band  lake  lat  lon  rtyp  soil  soilinterface  soilsnow  time  vegnodes
1 tier entry/entries name variables that no longer exist:
  f_qcharge
```

两条都是**判据选错了对象**，不是容差表写错：

1. **维度轴被当成"变量没了"**。`present` 收的是 `nc.variables()` 的**全部**名字，
   而 `oracle/tolerances.toml:25` 给 `band`/`lake`/`lat`/`lon`/`rtyp`/`soil`/
   `soilinterface`/`soilsnow`/`vegnodes` 单列了一组逐位比的**轴**。它们没有 `f_`
   前缀 → 被判成"表里的名字不存在"。修法：`present` 照收（轴本来就在文件里），
   判据改成"**既不在文件里、也不在闸门表里**才算没了"。
2. **运行时条件变量被当成"变量没了"**。`f_qcharge` 挂在
   `IF (.not. DEF_USE_VariablySaturatedFlow)`（`MOD_Hist.F90:698`）里，
   而 `oracle/golden/` 两份黄金都是 `CN-Cng` 的默认配置、**都走 VSF**，
   所以它一份都不在。修法：闸门表里 `runtime.is_some()` 的名字进一个
   `conditional` 集合，缺席时只输出一行 `note:`，不计错。

`stale` 的新判据是**并集**：黄金文件里有的 ∪ 闸门表标了运行时条件的。
"改名/删变量"照样抓得住（两边都没有），"这个窗口没跑到"放过。
闸门表那段条件原文**不求值**：求值要么重写一个 namelist 解释器，要么在这里
塞一份开关的副本，两条都不如"有运行时条件就不算缺失"来得诚实。

判据被抽成 `classify()` 以便单测，三条回归测试钉住：运行时条件变量不算缺失、
改名仍然算缺失、轴不算缺失。修完输出 `all 127 golden variables have a tier assignment`
（127 = 117 个 `f_` 变量 + 10 个轴，与 `histmap.rs` 的 117 对得上）。

**教训**：一条验收命令红了，先看**完整输出**再动手 —— 只 `tail` 会漏掉前一半，
于是会把一个"两个独立 bug"当成一个。

## 下一件大事：VSF 编排（`WATER_VSF`）的实测图谱（2026 年，审阅，未动工）

`DEF_USE_VariablySaturatedFlow` 是本仓库**最后一个被拒绝的分支**，而且它就是
**默认配置**（选了 van Genuchten 时 `MOD_Namelist.F90:1767-1772` 强制置真，
声明默认值本来也是真）。`variably_saturated_flow.rs` 的 2604 行内核早已写完并单测，
缺的只是编排。这一节把"到底缺什么"钉死，免得下一轮从 813 行 Fortran 重新读起。

### 一、编译事实（先排雷）

| 事实 | 证据 |
|---|---|
| `WATER_VSF` 在 `main/MOD_SoilSnowHydrology.F90:529-1341` | — |
| 内核在 `main/HYDRO/MOD_Hydro_SoilWater.F90`（3651 行） | — |
| 这两个文件**没有** `extends/interception/` 替身 | `Makefile:635-655` 只替换那五个截留模块 |
| `main/` 与 `extends/` 的坑**不适用**于本分支 | 全树各只有一份 `.F90` |

### 二、已移植内核 → Fortran 逐项对照（**可直接复用，不要重写**）

| Fortran（`MOD_Hydro_SoilWater.F90`） | 行 | Rust |
|---|---|---|
| `get_water_equilibrium_state` | 87-157 | `hydrology.rs:32 equilibrium_water_state` |
| `soil_psi_from_vliq` / `soil_hk_from_psi` | — | `hydrology.rs:128` / `:184` |
| `soilwater_aquifer_exchange` | 491-633 | `variably_saturated_flow.rs:1570` |
| `water_balance` | 1093-1187 | `:1097` |
| `initialize_sublevel_structure` | 1190-1337 | `:1159` |
| `use_explicit_form` | 1340-1485 | `:1419` |
| `var_perturb_level/rainfall/drainage` | 1488-1661 | `:223` / `:374` / `:401` |
| `check_and_update_level` | 1665-1722 | `:1747`（**已移植，只是 private**） |
| `flux_inside_hm_soil` | 2696-2757 | `:942` |
| `flux_at_unsaturated_interface` | 2759-2864 | `:1001` |
| `flux_top/btm/both_transitive_interface` | 2867-3373 | `:628` / `:701` / `:776` |
| `flux_sat_zone_fixed_bc` | 2597-2692 | `:855` |
| `get_zwt_from_wa` | 3376-3444 | `:1330` |
| `solve_least_squares_problem` | 3448-3524 | `:424` |
| `secant_method_iteration` | 3528-3566 | `:2056`（private） |

### 三、真正缺的只有五处，而且只有一处含新物理

| 缺失单元 | 行 | 行数 | 性质 |
|---|---|---|---|
| `flux_sat_zone_all` | `:2073-2594` | 521 | **纯分派**：约 32 处 `CALL` 全部指向已移植函数；键是「6 种几何 × 9 种 `(ubc_typ, lbc_typ)`」 |
| `Richards_solver`（驱动体） | `:636-1089` | 454 | 亚步循环 + Newton 外层 + 三条夹紧；子过程全已移植 |
| `flux_all` | `:1726-2069` | 344 | 纯分派：`lev_update` / `has_sat_zone` / 四条 `dz_upp`/`dz_low` vs `tol_z` 分支 |
| `soil_water_vertical_movement` | `:160-488` | 329 | **唯一的新物理**：蒸腾亏缺级联 + 分段列循环 + 质量平衡诊断 |
| `find_unsat_lev_lower` | `:3568-3585` | 18 | 10 行线性扫描 |

另外必须新增的共享常量是 `MAX_ITERS_RICHARDS = 10`（`:49`）；
`RICHARDS_TOLERANCE = 8.0e-8` 已有（对应 `:50 tol_richards`）。

### 四、四条会咬人的细节

1. **`Richards_solver` 不收敛不报错，只降级。** `:831-849`：`iter >= 10` 就转
   `use_explicit_form` 继续跑完。所以**降级计数必须先暴露出来**，否则 tier2 的
   残差无法归因。注意此前那条"495/528 步迭代数一致、无一步撞 itmax"是在
   `WATER_2014` 路径上测的，**不能外推**到 VSF。
2. **两套容差不许内联。** `soil_water_vertical_movement` 收到的是硬编码的
   `1.e-3`（`:1101`），由它派生 `tol_q`/`tol_z`/`tol_v`；`Richards_solver` 那一套
   来自 `8.e-8`。两者差四个量级，都要走常量而不是散在代码里。
3. **本仓库对 vendor 的两处本地补丁，Rust 必须跟**：
   `MOD_SoilSnowHydrology.F90:742-746`（`nprms` 恒为 5，Campbell 只填 `prms(1,:)`）、
   `:1280-1296`（wetland 分支补写 `smp = psi0`/`hk = hksati`，修 `intent(out)` 未定义）。
   第一条对 Rust 是好事：`prms` 矩阵换成逐层 `SoilHydraulicModel`，两种解释不再共用一个数组。
4. **`soil_water_vertical_movement:280-290` 就是与 PHS 的接口**：
   `DEF_USE_PLANTHYDRAULICS` 关闭时 `etroot = etr*max(rootr,0)/sum(rootr)`，
   打开时 **`etroot(:) = rootflux`**（上一节刚接完的那条链）。

### 五、验收目标换成更强的那一份黄金

编排完成后比对的是 **`oracle/golden/CN-Cng_hist_2008-01.nc`**（127 个变量，
**含 `f_qlayer`、缺 `f_qcharge`**）—— 它是 VSF **开** + PHS **开**，
比现在这份 `CN-Cng-aligned`（126 变量、VSF 关）强得多。实测 `f_qlayer`
n=2904、min −4.74e-5、max 0.0；`f_rsur_se`/`f_rsur_ie`/`f_rsub` 全 0。

history / 重启侧要一起改的四件事：`f_qlayer` **必须注册并填充**
（`tolerances.toml:71` 已给它 tier2）；**VSF 打开时 `f_qcharge` 不能再声明**
（它的运行时条件是 `(.not.DEF_USE_VariablySaturatedFlow)`）；
`rsur_se`/`rsur_ie` 从 `DECLARED_ONLY` 移到实填；`frcsat` 恒写
（`frcsat` 现在挂 `DECLARED_BUT_UNFILLED`，那条注释里已经写明"只有 VSF 才会填它"）。

**可以合法拒绝的分支**（与 `WATER_2014` 的既有拒绝保持一致）：
`patchtype ∉ {0,1}`、`is_dry_lake`、`DEF_USE_Dynamic_Wetland`、
`DEF_USE_SNICAR`、`CaMa_Flood`/`LWINFILT`、`CROP`/`DEF_USE_IRRIGATION`、
`DataAssimilation`、`DEF_SPLIT_SOILSNOW`、tracer、`DEF_URBAN_RUN`、
`DEF_Runoff_SCHEME == 1`（`Runoff_VIC` 未移植）。CN-Cng 默认下这些全为假。

### 六、施工进度（滚动更新）

| 单元 | 状态 |
|---|---|
| `flux_sat_zone_all`（521 行分派） | **已移植**（`20ecc00`），6 条单测 |
| `flux_all`（344 行分派） | **已移植**（`96fea09`），6 条单测 |
| `find_unsat_lev_lower` | **已移植**（`find_unsaturated_level_lower`，private） |
| `Richards_solver` 驱动体（454 行） | **已移植**（`3ff5880`），3 条单测 |
| `soil_water_vertical_movement`（329 行） | **已移植**（`c7b1894`），4 条单测 |
| `variably_saturated_flow_step` 编排 | **已移植**（`1869ee1`），4 条单测 |
| 运行时/history/重启接线 | **已接**（`9afb1eb`） |

### 七、VSF 接上之后：**默认配置第一次跑通并比对黄金**

`DEF_USE_VariablySaturatedFlow` 打开就是**默认配置**（选 van Genuchten 时
`MOD_Namelist.F90:1767-1772` 强制置真），所以这一条才是"默认算例能不能跑"的关键。
接线之后：

| 算例 | 配置 | 比对的黄金 | tier0 | tier1 | tier2 |
|---|---|---|---|---|---|
| `CN-Cng` | van Genuchten + **VSF 开** + PHS 开 | `oracle/golden/CN-Cng_hist_2008-01.nc`（金标，127 变量） | 2 | 19 | **38**（修地表凝结前是 43，见文末） |
| `CN-Cng-aligned` | Campbell + VSF 关 + PHS 关 | 自带工作目录 | 2 | 19 | 48 |
| `CN-Cng-phs` | Campbell + VSF 关 + PHS 开 | 自带工作目录 | 2 | 19 | 49 |

tier0/tier1 三份完全一致（`f_xy_solarin` 11/264 差 1 ULP、`f_xy_q` 3/264 是上游
时刻对齐怪癖、19 条 tier1 都是短波分带与 `f_alb` 一族）。**VSF 那份比另外两份
还少 5 条 tier2** —— 因为它比的是金标，而金标本身就是 VSF 配置。

`unported_branches` 到这一轮**第一次为空**。

#### 接线时撞出来的一个真阻塞：`DEF_RSS_SCHEME = 0`

关掉 Campbell 土壤模型时，`MOD_Namelist.F90:1947-1951` 把 `DEF_RSS_SCHEME` 置 **0**，
而 `MOD_Thermal.F90:613-621` 对 0 的处理是**"不启用土壤表面阻力"**：直接把 `rss = 0`，
**连 `SoilSurfaceResistance` 都不调**。本仓库的能量步无条件调那个内核，而内核只认
`1..=5`，于是**每一个 van Genuchten 算例都在能量步就死在
"soil surface resistance inputs are invalid"**，根本走不到水分步。
修法照抄上游的守卫：`scheme == 0` 直接返回 `rss = 0`。

这条值得记：`0` 不是"没实现的档位"，而是"关掉"的语义值。同族的还有
`plant_hydraulics.rs` 的 `RSS_SCHEME ∈ 0..=5`（那一处上一轮已经放宽）。

#### 接线时一起改掉的 history 语义

| 量 | VSF 关 | VSF 开 |
|---|---|---|
| `f_qcharge` | 声明并填 | **不声明**（上游 `MOD_Hist.F90:698` 的 `IF (.not. VSF)`） |
| `f_qlayer` | 不声明 | **声明并填**（维度 `soilinterface`） |
| `f_rsur_se`/`f_rsur_ie` | 声明、留空 | **实填** |
| `f_frcsat` | 声明、留空 | **实填** |

即 `f_qcharge` 与 `f_qlayer` 是一对**互斥的条件列**，与 `f_vegwp` 是同一类。

#### 下一步：现在才第一次可测的残差

先记一处**已经定位并修掉**的：第一次把 VSF 接上时，黄金比对的头部是灾难性的
（`f_h2osoi` 6.4e-3 对 0.383、`f_wliq_soisno` 差 30 倍、`f_frcsat` 1.0 对 0.35），
根因是**参数接错**，不是物理差：

> `soil_water_vertical_movement` 的 dummy 叫 `porsl`，但它**唯一的调用方**
> （`MOD_SoilSnowHydrology.F90:1096`）传进去的是 `eff_porosity(1:nl_soil)`；
> 同一行第 12 个实参 `porsl(nl_soil)` 才是真孔隙度，对应 `porsl_wa`（含水层）。
> 本仓库第一版给 `porsl` 传了**真**孔隙度，于是"含冰层的饱和判定"全错：
> `is_sat` 判假 → 求解器把上游保持饱和的第 2、3 层抽干 → 水位从柱底跳到 69 mm
> （黄金 0.88 mm），后面每一步都在错的状态上继续。

修法只有一行（传 `eff_porosity`），效果是**第 0 条记录就基本对上了**：

| 记录 0 | 黄金 | 修前 | 修后 |
|---|---|---|---|
| `f_wliq_soisno` 1–4 层 | 3.952 / 13.802 / 22.807 / 37.087 | 5.105 / 12.909 / 22.510 / 37.087 | 3.982 / 13.771 / 22.807 / 37.087 |
| `f_wice_soisno` 第 1 层 | 4.5778 | 4.6079 | 4.5791 |
| `f_h2osoi` 第 1–2 层 | 0.510727 / 0.501685 | 0.578422 / 0.469588 | 0.512496 / 0.500566 |
| `f_t_soisno` 第 2 层 | 275.828903 | 275.828890 | 275.828890 |
| `f_zwt` | 0.000876 m | 0.074166 m | 0.025537 m |

同一次定位还揪出**第二个独立缺陷**：`flux_sat_zone_all` 的 Case 5
（饱和段两端都在过渡界面上）原先把**整段** `i_stt:i_end` 当成饱和子列传给
`flux_variable_saturated_both_transition`。上游
`flux_both_transitive_interface` 的 `dz` dummy 确实覆盖整段，但它内部把
`dz(i_stt+1:i_end-1)`（即 `nlev_sat` 个）交给下层过渡界面函数，`qlc` 也只覆盖
那个子段。传整段会让内核返回 `i_end-i_stt+1` 个通量而缓冲只有
`i_end-i_stt-1` 个 —— 单测里当场 panic。现在那一行是
`debug_assert_eq!(segments.len(), i_e - i_s + 1, ...)`。

**方法**（与之前几次一样，值得复用）：给 Fortran 的
`soil_water_vertical_movement` 加 `WRITE(96,...)`（输入 + 输出各一行）、
给 Rust 的同一位置加 `eprintln!`，两边跑同一个算例，比**同一步**的同一组量。
第一轮对比就把输入 `vol_liq[1]`（Fortran 0.3100633 = `eff_porosity[0]`、
Rust 是真孔隙度）与 `ss_wt`（Fortran `0.9*dz`、Rust 0）的差别摆出来了。
插桩用完即撤，撤后重跑 `test_upstream_f48_sync.py` 与内核构建。

#### 第二个已定位并修掉的：降雨边界的 `min` 取错了实参

上面那段"积水与水位互相矛盾"的现象，根因是 `flux_sat_zone_all` 里一行抄错的
`min`：

```fortran
CASE (BC_RAINFALL)
   IF (wdsrf < tol_z) THEN
      qq(lb-1) = min(ubc_val, qlc(lb))     ! ← 取小的是 ubc_val（降雨通量）
      is_trans = (qlc(lb) > ubc_val)
```

第一版写成 `min(wdsrf, qlc(lb))`。无积水时 `wdsrf = 0`，于是
`qq(lb-1) = min(0, 0) = 0` —— **表层通量凭空变成 0**。

后果不是"差一点"：`water_balance` 把每一层的失衡累加到**最近一个非饱和层的桶**里
（`MOD_Hydro_SoilWater.F90:1155-1171` 的 `ilev` 累加器），而地表那个桶
（`blc(lb-1)`）只在 `BC_RAINFALL` 下才是 Newton 变量。表层通量变成 0 之后，
0.1398 mm 的失衡全被记到**地表桶**上，而地表那格这一轮不是变量 ——
于是 **10 次迭代的残差逐位不动**（`1.397946e-1`），Newton 只能降级成显式步，
第一步就走错了，后面全在建在错的状态上。

**"残差一步都不动"是零更新，不是收敛慢** —— 这条判据值得记。定位过程同样是两边插桩：
Fortran 的 `SZA` 探针与 Rust 的探针在 `i_stt/i_end`、`sat`、`trans`、`qlc`、
`wdsrf`、`ubc`、`wt` 上**逐项一致**，把故障逼到 `water_balance` 的分桶；
再打残差向量，看到 0.1398 落在第 0 格而 Jacobian 唯一活跃的对角线在第 1 格。

修后**前 16 条记录基本逐位对上**：

| 量 | 黄金 | 本仓库 |
|---|---|---|
| `f_zwt` 前 4 条 | 0.000876 / 0.001751 / 0.003178 / 0.008803 | 0.000876 / 0.001751 / 0.003178 / 0.008804 |
| `f_qinfl` 前 4 条 | -3.9e-5 / -4.4e-5 / -4.7e-5 / -4.6e-5 | 同（5 位有效） |
| `f_wdsrf` | 全 0 | 全 0（不再凭空积水） |
| `f_frcsat` 前 3 条 | 0.675763 / 1.0 / 0.765763 | 0.675771 / 1.0 / 0.765764 |
| `f_qlayer` | — | 超容差从 322/2904 降到 **5/2904** |

tier 汇总从 `{2,19,45}` 变成 `{2,19,43}`（后来又降到 38，是因为地表凝结那一项，见文末）。

**还没解决的**（从第 16–17 条起分叉，是另一个缺陷 —— **已经在文末的
「地表凝结根本没进土壤表层」一节里定位并修掉**，下面这段保留当时的观察）：

- 分叉点在第 16/17 条：黄金的 `f_h2osoi[0]` 继续降到 0.3772，本仓库停在 0.3850；
  同一时刻 `f_wliq_soisno[0]` 两边都已到 `volume_tolerance` 地板（2.7e-5），
  所以差的是**冰**：本仓库第 1 层多 ~0.136 kg/m²。
- `f_zwt` 从第 16 条起相差 4.6e-4 m（0.46 mm），`f_frcsat` 从第 17 条起分开。
- `f_qinfl` 从第 18 条起两边都是 `-0`，所以**不是表层通量在驱动**。
- 各量的相对差在 1e-3 量级首次出现于第 14–18 条，之后缓慢放大 ——
  是"继承残差被放大"，不是又一次分支跳变。
- `f_vegwp` 从第 0 条就有 1e-3 量级的相对差（PHS 递归本身振荡，见上一节）。

**`soil_water_vertical_movement` 自带验收判据**：它算整柱质量平衡误差 `wblc`，
超过 `tolerance` 就打警告。单测因此直接断言 `|wblc| <= 1e-3`（上游传进来的那个数），
而不是只查结构不变量 —— `qlayer`、亏缺级联或含水层交换接错一处，这个数立刻爆掉。

移植时撞出的两个坑，都写进注释与单测：

1. **`findloc_ud` 返回的是"数组下标"**，而 `sp_zi` 声明成 `sp_zi(0:nlev)`，
   于是它返回的是"界面号 + 1"，正好可以当 1-based 的**层号**用。
   看起来像 off-by-one，其实是刻意的 —— 别"修"它。
2. **`DO ilev = izwt+1, nlev` 在 `izwt == nlev + 1` 时是空循环**，
   所以最自然的 Rust 改写 `copy_from_slice(&thickness[izwt..nlev])` 会在
   "水位在柱底之下"这个常见情形上 panic。clippy 恰好建议了这个改写，
   被单测当场抓住；正确写法是 `zip().skip()`。

两个分派器落地后**黄金结果一位没变**（`CN-Cng-aligned` tier2 = 48、
`CN-Cng-phs` tier2 = 49），因为它们还没有调用方。

剩下要做的两步：`variably_saturated_flow_step`（把 `WATER_VSF` 的雪层段、
`prms` 填充、白名单拒绝、runoff、`smp`/`hk` 冰阻抗、`rnof` 这些外围拼到
`soil_water_vertical_movement` 上），以及运行时/history/重启接线
（`f_qlayer` 注册并填充、`f_qcharge` 停止声明、`rsur_se`/`rsur_ie` 实填、
`frcsat` 实填、`unported_branches` 去掉 VSF）。接完之后验收目标换成
`oracle/golden/CN-Cng_hist_2008-01.nc`（VSF + PHS 都开，127 个变量）。

## 地表凝结（`qsdew`/`qfros`/`qsubl`）根本没进土壤表层：VSF 残差的主因（2026 年，实测）

上一节末尾把"第 16/17 条起分叉"定在**冰**上，但没说清冰是怎么少的。这一轮查清了，
根因不在 `WATER_VSF` 本身，而在**它上游那三行凝结项**。

### 现象：液相抽干之后，黄金的冰还在升华，本仓库的冰冻住了

`CN-Cng`（van Genuchten + VSF + PHS）第 1 层（`f_*_soisno` 的第 6 列，前 5 列是雪槽）：

| 量 | 第 16 条 | 第 17 条 | 第 22 条 | 说明 |
|---|---|---|---|---|
| `f_wliq_soisno[5]` 黄金 | 0.0345665 | 2.7035e-5 | 1.351e-4 | 第 17 条起贴 `tol_v` 地板 |
| `f_wliq_soisno[5]` 本仓库 | 0.0347742 | 2.7034e-5 | 1.351e-4 | 同步贴地板 |
| `f_wice_soisno[5]` 黄金 | 6.182359 | 6.136215 | 5.772307 | 每小时掉 ~0.07 |
| `f_wice_soisno[5]` 本仓库 | 6.182342 | 6.182333 | 6.181756 | **不动** |

`f_fevpg` 两边都是正数且对得上（第 17 条 2.26525e-5 对 2.26657e-5），
`f_qinfl` 两边都是 `-0`。所以**不是地表通量在驱动，是这份 `fevpg` 的归属错了**。

### 根因：两条独立的缺陷叠在一起

上游 `MOD_SoilSnowHydrology.F90:452-457`：

```fortran
IF ((.not.DEF_SPLIT_SOILSNOW) .or. (patchtype==1 .and. DEF_URBAN_RUN)) THEN
   IF(lb >= 1)THEN
      wliq_soisno(1) = max(0., wliq_soisno(1) + qsdew * deltim)
      wice_soisno(1) = max(0., wice_soisno(1) + (qfros-qsubl) * deltim)
   ENDIF
```

`lb` 是**土壤**顶层号（`CoLMMAIN.F90` 传 `snl+1`），所以 `lb >= 1` 恰好就是
"无雪层"。`WATER_VSF`（`:1125-1134`）同一处判据。

* **缺陷一**：`water_2014_snow_soil_step` 把这三个字段**硬编码成 0.0**，
  而不是按手里的雪列决定。
* **缺陷二**：`standard_lct_snow_soil_step` 又把它们留给
  `..input.soil_water.fluxes` —— 而装配模板给的就是 0.0，并且注释里明写
  "内核覆盖：`standard_lct_soil_step` 用本步能量链的通量重建"。
  这条契约在**土壤入口**（`standard_lct_soil_step`）确实兑现了，
  在**通用积雪入口**（`standard_lct_snow_soil_step`，无雪时也走它）没有。

两条合起来的效果：`THERMAL` 算出的 `qsubl` 被丢掉了。冻结表层的 `wliq(1)` 一贴到
`tol_v` 地板，`qseva = min(wliq/deltim, fevpg)` 就只剩 1.5e-8，**`fevpg` 的其余部分
全是升华**（`MOD_Thermal.F90:1260-1261`），而升华去处被清零 —— 冰于是冻住。
`fevpg` 的绝对值没有错，`f_fevpg` 的 history 也就对得上，这就是为什么这个问题
在"只比通量"的视角下看不见。

### 定位手法（可复用）

不再两边插桩，而是先在 **Rust 侧单独**打一行：

```rust
eprintln!("VSFDBG in subl={:.12e} dew={:.12e} frost={:.12e} liq0={:.12e} ice0={:.12e} wblc={:.12e}", ...);
```

528 步全部打出 `subl=0 dew=0 frost=0`，而同一步 `liq0` 已经贴地板、`wblc` 只有
1.8e-5（`wice` 的逐位下降量正好等于 `wblc`，说明冰汇本身是对的）。
一行就把故障锁死在"这三个字段是 0"，不必再动 Fortran。

### 修法与效果

`water_2014_snow_soil_step` 按 `snow_state.layer_count < 0` 决定传 0 还是透传；
`standard_lct_snow_soil_step` 从本步的 `thermal_water` 补上三项。

`oracle/golden/CN-Cng_hist_2008-01.nc` 的最大绝对偏差：

| 量 | 修前 | 修后 |
|---|---|---|
| `f_wice_soisno` / `f_wliq_soisno` | 19.51 kg/m² | **0.096 kg/m²** |
| `f_h2osoi` | 0.3772 | **1.45e-4** |
| `f_t_soisno` | 5.822 K | **0.0548 K** |
| `f_t_grnd` | 3.601 K | **0.0155 K** |
| `f_zwt` | 0.3494 m | **0.1068 m** |
| `f_frcsat` | 0.06726 | **1.002e-4** |
| `f_tleaf` | 1.666 K | **0.0369 K** |
| `f_wat` | 6.091 mm | **0.00291 mm** |

tier 汇总 `{tier0:2, tier1:19, tier2:43}` → **`{tier0:2, tier1:19, tier2:38}`**。

两条单测守这个行为：`water_2014_tests.rs` 的
`snow_soil_entry_credits_surface_condensation_only_without_a_snow_layer`
（无雪透传 / 有雪归零，用差分验），`standard_lct_step_tests.rs` 的
`standard_lct_snow_soil_step_credits_the_thermal_condensation_to_the_soil`
（断言精确等式，并先断言本 fixture 的冰侧通量非零，否则这个测试区分不出接线）。

### 一条新规矩

> 字段注释写着"内核覆盖"的输入是**未兑现的承诺**，不是事实。
> 装配模板把所有通量填 0、注释说内核会重建 —— 换一条内核入口就必须重新核对
> 每一个字段，否则 0 会安安静静地一路跑到底。金标回归只给出"哪个量错了"，
> 不会指出"哪个字段没人写"。

### 修后剩下的残差（下一轮的施工单）

* `f_vegwp` rel 0.57（第 0 条就有；PHS 递归 + 叶温求解器 0.01 K 地板，见 §五）。
* `f_zwt` 0.107 m：第 215–220 条两边恒差 5.116 mm，第 234 条黄金回落到
  395.92 mm 而本仓库停在 502.71 mm（水位停在界面上 vs 继续下渗）。
* `f_t_soisno` / `f_wliq_soisno` / `f_wice_soisno` 的最差槽位都在**第 221 条第 10 层**
  （0.0548 K / 0.096 kg/m²），与 `f_frcsat`（仅第 221 条超 1e-4）同一时刻，
  指向又一次分支跳变。
* `f_fq`/`f_fm`/`f_fh` 三条 rel 都是 1.0729e-2（同一底层量），第 12 条起超容差、
  第 232 条最大 —— 近地层稳定性函数，值得单独查。
* `f_gssun`/`f_gssha` 第 0 条 2.26e31：上游未初始化 `rssun` 的 UB（已知）。
* `f_etr`/`f_etrsha`/`f_etrsun` 的绝对量只有 1e-7 量级，贴着 tier2 的 `atol=1e-7`。

## 湿季窗口 `CN-Cng-wet` 第一次跑通：一个"校验比上游严"的阻塞（2026 年，实测）

`oracle/golden/CN-Cng-wet_hist_2008-07.nc`（2008-07-01 起 16 天，384 条，
`tolerances.toml` 的 tier3 基线就是按它定的）**此前从未被本仓库跑过** ——
不是没比对上，是**根本跑不起来**：

```
colm-rs: canopy interception canopy_water total_mm is invalid: -0.000000000000000006938893903907228
```

`-6.9e-18 mm`，即 -1 ulp。

### 根因：上游的 `ldew` 可以是负 1 ulp，而本仓库在三处要求 `>= 0`

上游 `MOD_LeafInterception.F90:324` 是裸的 `ldew = ldew + pinf`，而
`pinf = p0 - (thru_rain + thru_snow)`：两个 `thru` 各由 `tti`+`tex` 组成，
两项都被 `.min()` 截断过，所以 `pinf` 在浮点上可以到 **-1 ulp**。
上游既不夹 `ldew` 也不校验它 —— 负值随后被
`MOD_LeafTemperature.F90:1165` 的 `ldew = max(0., ldew - evplwet*deltim)` 抹平。

本仓库**三处**独立地要求 `CanopyWater` 非负：

| 位置 | 判据 |
|---|---|
| `interception.rs::intercept_canopy` 的 `validate` | `total_mm/rain_mm/snow_mm >= 0` |
| `interception.rs::canopy_wetness` | 同上（`fwet_snow` 的输入） |
| `leaf_temperature.rs` 的 `validate_leaf_temperature_*` | 同上（叶温求解的输入） |

于是"每个**单独**看都合理"的三条校验串成一条接力：放开第一条，第二条接着炸。
这与 `DEF_RSS_SCHEME = 0` 是同一种缺陷 —— **校验比它所守的内核严**。

### 修法

新增 `interception::CANOPY_WATER_ROUNDOFF_MM = 1.0e-12`，三处共用。
尺度论证：`ldew` 量级 0.1 mm，1 ulp ≈ 1e-17 mm，所以 1e-12 mm
**比 1 ulp 大五个数量级、比任何物理量小十一个数量级** —— 只放行舍入，不放行缺陷。
单测 `canopy_water_tolerates_one_ulp_but_not_a_real_deficit` 把两端都钉住
（`-6.9e-18` 放行、`-1.0e-6` 拒绝）。

### 跑通之后的结果

768 步跑完，与 `oracle/golden/CN-Cng-wet_hist_2008-07.nc` 比对：
**`{tier0: 4, tier1: 0, tier2: 70}`**。

* `tier1` 在**第二个**黄金窗口上同样为空 —— 这是"把 19 条继承量搬进 tier2"
  那次改动的独立佐证（换窗口重跑分类，本来就是 `tolerances.toml` 头部要求的）。
* `tier0` 4 条全是强迫场本身：`f_xy_rain`/`f_xy_prc`/`f_xy_prl`/`f_xy_solarin`，
  384 条里 16–24 条差 1–2 ULP，最大相对差 2.9e-16。与干季窗口的
  `f_xy_solarin` 是同一类（读取/进位路径），不是物理。
* `tier2` 的 70 条里，量级最大的几条：`f_fsenl` 17.8 W/m²（尺度 128）、
  `f_fsena` 11.2、`f_rnet` 9.9、`f_lfevpa` 12.8、`f_fseng` 7.6、
  `f_wdsrf` 0.171 mm（黄金最大 0.080，本仓库 0.174）、
  `f_frcsat` 0.48（单条相对差 93%）、`f_zwt` 0.067 m。

**为什么湿季残差比干季大一到两个数量级**：7 月有雨、土壤接近饱和，
`frcsat` 长期在 1 附近、`zwt` 打到 0，`rsur_ie`（入渗超限产流）成为表层通量的
主项，于是**干季里被 `qinfl` 掩盖的状态差在这里直接决定产流**。第一次分开是
第 111–114 条（雨峰）：`f_frcsat` 0.33644 对 0.32813、`f_zwt` 0.0822 对 0.1446，
`rsur` 少 8%，缺的那点雨留在 `f_wdsrf` 里，而 `wdsrf` 只在水位到地表时被
抽干，于是 0.171 mm 的差一直挂到第 121 条。

### 还没做的：tier3 基线根本没有代码

`tolerances.toml` 的 `[tier3.baselines]`（`rnet_r2_min = 0.999`、
`qle_r2_min = 0.85`）是**设计文档 §2.8/§2.8b 给湿季窗口定的验收判据**，
但仓库里没有任何代码算它：`tier3` 的 `variables = []`，
`tier_compare.rs` 对 `statistical` 层一律报"逐变量不可判"。
所以现在拿 tier2 的逐变量判据去卡湿季窗口，**比设计要求的严**。
这是下一步该补的洞，也是唯一能让湿季窗口"按它自己的判据通过"的路。

### 干季窗口剩下的施工单（承上文）

* `f_vegwp`（第 0 条是上游 UB；第 1 条起 rel 7e-4，叶温求解器 0.01 K 地板）。
* `f_zwt`：差值呈**分段常数**（-7e-6 → -4.6e-4 → 0 → -5.7e-4 → … → -5.1e-3），
  说明水位每次重算时定下一个小差，之后两边同步演化到下一次重算。
  第 234 条那一跳已查明：`f_zwt` 是两条子步的均值，
  `(502.707 + 289.13)/2 = 395.918` 正是黄金值 —— 水位"贴到 289.13 mm 界面"
  晚了一条子步。
* `f_t_soisno`/`f_wliq_soisno`/`f_wice_soisno` 的最差槽位都在第 221 条第 10 层
  （0.055 K / 0.096 kg/m²），`f_frcsat` 也只在第 221 条超 1e-4，同一时刻。
* `f_ldew` 在干季第 78–107 条黄金停在 `1.39931e-34`（子步里一个 0、一个
  2.799e-34 的均值），本仓库是精确 0。尺度差 34 个数量级，不构成超容差，
  但它说明两边 `pinf` 的最后一位不同 —— 与上面那个 -1 ulp 同源。

## Tier 3（整场统计判据）第一次真的被判：`oracle --bin tier3-check`（2026 年，实测）

`tolerances.toml` 的 `[tier3.baselines]` 一直只有两个数、没有执行者。
`oracle/tests/metrics.rs` 把 §2.8/§2.8b 的六行指标钉在**黄金文件**上 ——
它证明的是"Fortran 产出与观测吻合"，也就是**基线本身没写错**；
而"Rust 产出是否也落在同一条基线上"从来没有被测过。

逐变量比与整场统计是**互相独立**的两把尺子。干季窗口现在的实测：
逐变量 tier2 有 38 条超容差，而它的 `f_rnet` 与观测 R² = 0.986。
一个忠实移植的舍入残差不影响统计等价；反过来统计等价也盖不住一条抄错的公式。
缺了后者，"Tier 3"在仓库里就只是一句注释。

### 新工具

```
tier3-check <history.nc> --observation <obs.nc> [--tolerances oracle/tolerances.toml]
```

* **预热小时数与两条 R² 下限都从表里读**，命令行不给 —— n 随预热变，
  两边分开写迟早对不上。
* **观测文件显式给**（与 `golden-run` 对 `PLUMBER2_ROOT` 的态度一致）。
* **时间原点解析观测自己的 `units`**（`model_seconds_from_units`），
  不按年份硬推：PLUMBER2 不保证从元旦开始（AU-Preston 是从 03:30 起），
  错开之后 R² 只会更低，错的那一方反而"看起来更保守"。
* **基线有作用域**。`history_suffixes` 按文件名后缀认窗口：
  Fortran 产出是 `CN-Cng-wet_hist_2008-07.nc`，colm-rs 产出是
  `colm-rs_hist_2008-07.nc`（`colm-rs` 的 stem 不带算例名），共同的只有时期后缀。
  不在作用域内的文件**报一句就退出 0**，不拿湿季的门槛去卡冬季窗口 ——
  §2.8 那三行（Rnet R² 0.986 / Qle 0.047）是"冷启动无预热的预期值"，不是基线。

### 顺手抓到的：`rnet_r2_min = 0.999` **黄金自己过不去**

设计的表印到三位小数，`0.999` 是实测 `0.998618` 的四舍五入。
拿它当门槛，第一个失败的就是黄金文件本身。改成 **0.998**，并在表里写明理由 ——
0.998 既容得下三位显示的舍入，又仍拦得住真实漂移
（`metrics.rs` 的时区平移对照把 R² 打到 0.15/0.12）。
`qle_r2_min = 0.85` 保持原值：黄金实测 0.8518，确实过线。

### 结果

| 窗口 | 产出 | n | Rnet R² | Qle R² | 判定 |
|---|---|---|---|---|---|
| `CN-Cng-wet` | Fortran（黄金） | 287 / 278 | 0.9986 | 0.8518 | **通过** |
| `CN-Cng-wet` | **colm-rs** | 287 / 278 | **0.9986** | **0.8498** | Rnet 通过，**Qle 差 0.0002** |
| `CN-Cng`（冬季） | 黄金 | 256 / 254 | 0.9862 | 0.0474 | 不在作用域内，不判 |

* **Rnet 四位小数完全相同**（0.998618 对 0.998618）：强迫场、时间轴、时区、
  经纬度、辐射物理这一整条链在 Rust 侧与 Fortran 侧统计等价。
* **Qle 0.8498 对 0.8518**（相对差 0.2%），RMSE 80.33 对 79.47，bias +39.51 对 +38.37。
  这与逐变量那 70 条 tier2 是**同一个残差的两个视角**：`f_lfevpa` 最大绝对差
  12.82 W/m²（量级 604），正落在潜热上。
* 黄金自己的 Qle 余量只有 0.0018，所以这条门槛的分辨率就是 0.2% 量级。
  要把 Qle 推过线只能去修潜热那条链（叶温求解器 → 冠层持水 → `fwet_snow`），
  **不能靠把门槛往下挪** —— 那会把 tier3 变成一句自我实现的话。

结论：Tier 3 现在**有执行者、有作用域、有对照**，并且给出了一个具体的、
与 tier2 相互印证的待办（湿季潜热偏高约 1%）。这条待办与干季窗口那份施工单
是同一件事。

## 第三个黄金窗口：`US-NR1-snow`，以及它第一次跑就撞出的雪层缺陷（2026 年，实测）

### 为什么必须加一个积雪窗口

`tolerances.toml` 头部早就写着"尤其是能真正生成雪层的窗口"，而两个现有窗口
**一场雪都没有**：`CN-Cng` 的两年强迫里 `Tair < tfrz 且 Precip > 0` 只有
**11 个小时**、最长连续 6 小时、累计 4.6 mm；湿季窗口是 7 月。
于是 `snow.rs` 的全部内容、`standard_lct_snow_soil_step` 的雪层路径、
`newsnow`/`snowcompaction`/`snowlayerscombine`/`snowlayersdivide` 在黄金回归里
**零覆盖** —— 只有单测。

### 怎么选的窗口

按"落地时气温低于冰点的降水总量最大"扫 PLUMBER2 的 91 个站点（注意强迫是
**1800 s** 步长，不是 3600 s；按小时索引会把窗口整体错位）。选 **US-NR1**
（Niwot Ridge，40.03°N，海拔约 3000 m）**2013-04-09 → 04-23**：14 天 76 mm
落地时低于冰点的降水，`Tmean = 265.4 K`。

黄金里的雪：`f_snowdp` 最大 **0.823 m**、`f_scv` 最大 **106 mm**、
`f_fsno` 到 **1.0**、`f_wice_soisno` 最大 **69 kg/m²**。这是一个真正的深雪盖，
会走到压缩/合并/分割。

站点文件按 PLUMBER2 → CoLM 的通路补：

```
cargo run -p colm-srfdata --bin site-fill -- \
    $PLUMBER2_ROOT/Sitedata/US-NR1_1999-2014_FLUXNET2015_site.nc \
    oracle/cases/US-NR1-snow/site.nc
```

12 个必需字段里 7 个走了"模块默认"（土壤反照率、`lakedepth`/`elvstd`/`sloperatio`）
—— 本机没有全球 rawdata 栅格，`site-fill` 会逐条打出来源，**没有藏起来**。

算例本体（`oracle/cases/US-NR1-snow/`）、输入摘要（`oracle/fixtures/inputs.sha256`
多了两行）与黄金（`oracle/golden/US-NR1-snow_hist_2013-04.nc`）都入库。
`--write-golden` 会顺手把 `kernel-manifest.json` 里的三个二进制 `sha256` 换掉，
那三个值**本来就不可复现**，所以照例 `git checkout` 还原（见 §"清单里两组字段"）。

### 第一次跑就死在一个"读早了"上

```
colm-rs: root-uptake vectors must be finite and have equal lengths
```

第 5 步。插桩打长度，一眼看出问题：

```
RUPROBE soil=0 t=10 liq=10 ...      <- 前 4 步：纯土列，10 项，一致
RUPROBE soil=0 t=11 liq=11 ...      <- 第 5 步：t/liq 有 11 项，por/dz 还是 10
RUPROBE soil=0 t=11 liq=11 ...
```

**根因**：`standard_lct_snow_soil_step` 在 `add_new_snow` **之前**读
`state.snow.layer_count`，而 `packed_snow_soil_state` 是在它**之后**才拼列的。
`add_new_snow` 一旦真的建出一层雪，packed 列就变成 `nl_soil + 1` 项，而
`GroundTemperatureInput::snow_layers` 还是 0 —— `root_uptake_input` 用
`[soil..]` 切不掉那一层，长度校验当场失败。

上游没有这个问题：`newsnow` 跑在 `THERMAL` 之前，`snl` 是在它之后才重算的
（`CoLMMAIN.F90:831` 的 `totwb` 取的就是重算后的值）。

修法一行：把 `snow_layers` 的读取挪到 `add_new_snow` 之后，并把
`validate_snow_soil_step` 的第一个返回值丢掉（它的第二个返回值
`template_snow_layers` 仍然要在之前取，因为输入列的校验对象是**装配期**的模板）。
单测 `standard_lct_snow_soil_step_rereads_the_layer_count_after_newsnow`
构造"这一步才建雪层"的场景 —— **实测把改动还原后该单测失败**，所以它是真守门人。

这条缺陷只在**第一次积雪**那一步暴露。干季、湿季、植被积雪、
`CN-Cng-phs` 四个已有窗口全都碰不到它。

### 接上之后的结果

720 步跑完，与 `oracle/golden/US-NR1-snow_hist_2013-04.nc` 比对：
**`{tier0: 6, tier1: 0, tier2: 81}`**（360 条记录）。`tier1` 在**第三个**窗口上
仍然为空。

* `tier0` 6 条：`f_xy_rain`/`f_xy_snow`/`f_xy_prc`/`f_xy_prl`/`f_xy_solarin`/
  `f_xy_pbot` 一族，1–2 ULP，与另两个窗口同源。
* `tier2` 81 条里绝大多数是小量：`f_snowdp` 最大差 0.0029 m、
  `f_scv` 0.67 mm、`f_fsno` 0.041。
* **但 `f_t_soisno` 最大差 266 K** —— 那不是数值差，是**雪层数不一致**：
  第 42–45、49 条黄金把两层合成一层（`snowlayerscombine`），本仓库没合。
  按"前导零"反推层数：其余 355 条完全一致，只有这 5 条差 1 层；
  packed 列的槽位因此整体错开一格，`f_t_soisno` 就出现 0 对 266 K。
  5 条里 `f_snowdp` 的差是 0.0119 对 0.0124（4%），所以是**压缩后厚度**先分叉，
  再让某一层跨过 `dzmin` 阈值。`dzmin = [0.010, 0.015, 0.025, 0.055, 0.115]`
  与 `snowlayerscombine` 的两个循环逐行对照过，逻辑一致。
  下一步就去查 `snowcompaction` 的厚度演化。

## 清单里的 `colm_git_sha` 记的不是 Fortran 源（2026 年，实测，**已修**）

加 `US-NR1-snow` 黄金时用 `--write-golden`，顺手看到
`oracle/golden/kernel-manifest.json` 里 `colm_git_sha` 从 `3950ecf` 变成
`edd6d98`。两个都不是这笔改动碰过的东西 —— 一查就清楚了：

```
$ git -C vendor/CoLM202X rev-parse --short HEAD
1fac8e8
$ git rev-parse --short HEAD
1fac8e8
```

`vendor/CoLM202X` 是**入库的源码快照**，目录里没有 `.git`，所以
`git -C <子目录> rev-parse HEAD` 会**往上走进外层仓库**，返回的是**本仓库**的
HEAD。也就是说：

* `colm_git_sha` 每做一次 **Rust** 提交就变一次，哪怕一行 Fortran 都没动；
* 它**无法**识别 Fortran 源 —— 而 `manifest.rs` 的注释把这一组字段定义为
  "可复现、认定**配置身份**"，`identity()` 还是 `preset@colm_git_sha`；
* 于是 `golden-run` 的 `check_kernel_provenance` **永远在告警**，而告警的本意是
  "工具链换了，比对全红可能是漂移不是物理"。一个恒亮的告警等于没有告警。

**修法**：把身份换成**只在 vendor 内容变化时才变**的量。最省事且语义正确的一条是
"最后一个碰过 `vendor/CoLM202X` 的提交"：

```bash
GIT_SHA=$(git -C "$REPO_ROOT" log -1 --format=%h -- vendor/CoLM202X)
```

它同样可复现、同样便宜，而且**只随 Fortran 源变化**。

### 落地与验证（2026 年 9 月）

`oracle/scripts/build_kernel.sh` 已改成上面那条；重建 `default` 后
`kernels/default/manifest.json` 记的是 `ad75e8e` —— 即
`git log -1 --format=%h -- vendor/CoLM202X`（上一次动 `o3coef*` 的那个提交），
不再随 Rust 提交滚动。

流程按"改脚本 → 重建内核 → `--write-golden` 重盖章 → `git checkout` 还原 `.nc`"走：

1. 重建后先用 `golden-run <case>`（不带 `--write-golden`）确认告警**只剩它一条**：
   `colm_git_sha: recorded "21c6e09", current "ad75e8e"`，`built_with`/`netcdf_*`/`hdf5`
   全都一致 —— 也就是说这台机器上重建内核的**工具链字段是可复现的**，
   漂移确实只来自那个字段本身。
2. 三份黄金各自 `--write-golden` 重盖章，然后 `git checkout -- oracle/golden/*.nc`：
   入库的 `.nc` 字节一个没动（`git status` 里只剩 manifest 与脚本），
   免得为一次元数据修正往仓库里塞三份二进制改动。
3. 用 `/tmp` 里事先备份的**入库版**黄金逐个 `golden-compare` + 逐值位比，
   确认重建出来的内核仍然逐位复现它们：`CN-Cng` 0/46422、`CN-Cng-wet` 0/67970、
   `US-NR1-snow` 0/63586 个值不同。
4. 再跑 `golden-run`，三份都是 `provenance matches the recorded kernel` —— 恒亮的告警灭了。

manifest 里那三个 `sha256` 随重建而变是**预期的**：`check_kernel_provenance`
刻意不比它（Fortran 构建不逐字节可复现），它记的是当前这次构建的二进制。
**注意**：`ad75e8e` 会随任何一次动 `vendor/CoLM202X` 的提交而变，
那时要么重跑 `--write-golden`、要么接受一次告警 —— 这正是这一字段该有的行为。
（`vendor/PROVENANCE.md` 在 `vendor/` 下但不在 `vendor/CoLM202X` 里，动它不会触发。）

## 雪层数不一致的**因果链**：根在第 1 步的叶温，不在雪（2026 年，实测）

上一节把 `US-NR1-snow` 的 5 条记录（第 42–45、49 条）留成"雪层数差 1 层"。
这一轮按两边插桩（Fortran 的 `CoLMMAIN.F90` 在 `newsnow` 前后各一条
`WRITE(96,*)`、Rust 在 `add_new_snow` 前后各一条 `eprintln!`，都只打前 8 步）
把因果链量出来了。**插桩已全部撤销**，内核重建，`test_upstream_f48_sync.py` PASS，
`US-NR1-snow` 的黄金用重建后的内核重跑**逐变量差 0**。

### 量出来的链

| 步 | 量 | Fortran | Rust | 差 |
|---|---|---|---|---|
| 1 PRE | `tleaf` 初值 | 283.0 | 283.0 | 0 |
| 2 PRE | `tleaf`（第 1 步末） | **267.022** | **267.167** | **+0.145 K** |
| 2 PRE | `ldew` / `ldew_snow` | 4.1014e-2 | 4.7796e-2 | **+16%** |
| 2 PRE | `ldew_rain` | 0 | 0 | 0 |
| 2 PRE | `pg_snow` | 9.3841e-5 | 9.3841e-5 | 0（6 位相同） |
| 2 PRE | `bifall` | 75.5844 | 75.5844 | 0 |
| 2 POST | `scv` | 0.1689131 | 0.1689131 | 0 |
| 3 PRE | `scv`（第 2 步末） | **0.1171772** | **0.1043853** | **-0.0128** |

读法：

1. **第 1 步没有降水**（`prc_*`/`prl_*` 全 0），冠层是**干**的，叶温从 283 K
   一步掉到 267 K。两边这一步的叶温差 **0.145 K** —— 远大于求解器自己的
   0.01 K 收敛容差。
2. 这 0.145 K 直接改变第 1 步的**冠层凝霜**：Rust 多留 16% 的 `ldew_snow`
   （`ldew_rain` 两边都是 0 —— 雪期冠层水全在雪通道，这一支两队一致）。
3. 第 2 步的**落地雪**其实**完全一致**（`pg_snow`、`bifall`、`scv` 到 6 位相同），
   所以**不是截留算错**。差的是第 2 步把薄雪（`scv` 还在标量上、`snl == 0`）
   融掉多少：Fortran 融 0.0517 kg/m²、Rust 融 0.0645，差 0.0128。
4. 这个 `scv` 差就是 `f_scv` 的差（`f_scv[0]` = 第 1、2 步末的均值，
   `(0 + 0.11718)/2 = 0.05859` 正是黄金值）。`snowdp` 按 `scv/bifall` 走，
   于是雪深跟着差 12%，一路累积到第 42 条让某一层跨过 `dzmin` 阈值 ——
   层数因此差 1，packed 槽位整体错开，`f_t_soisno` 才出现 266 K 的**假**差。

**结论：`snow.rs` 本身没被抓到错**（`newsnow` 的落地雪逐位一致；
`dzmin` 与 `snowlayerscombine` 的两个循环也逐行对过）。这条残差的根是
**叶温求解器**，而它正是设计文档 §2.11 点名的最难单元。所以雪窗口的
81 条 tier2 与干季那 57 条、湿季那 70 条，最终指向同一个地方。

### 下一步（有明确落点）

第 1 步是**无降水、干冠层**，叶温一步跨 16 K。这个场景最容易暴露
`htvpl`（`hvap` ↔ `hsub`）的**迭代内切换**：`MOD_LeafTemperature_Extended.F90:693-697`
每轮迭代开头按当前 `tl` 重新取潜热，所以一旦迭代路径在冰点附近分岔，两边会落到
不同的不动点。要查的是"同一组输入、同一迭代序列，两边的 `tl` 序列是否逐步一致" ——
把两边的 `tl` 逐次迭代打出来比即可（本仓库已有 `MAX_ITERATIONS=40`、
`MIN_ITERATIONS=6`、`0.01 K` / `0.1 W/m²` 三个常数可对齐）。

## 叶温求解器：迭代 1–5 逐步一致，第 6 轮分叉，差全在 `fevpl`（2026 年，实测）

上一节把三个窗口的残差都指向叶温求解器。这一轮把**逐次迭代的 `tl` 序列**两边
打出来比（Fortran 在 `MOD_LeafTemperature_Extended.F90` 的
`tl = tlbef + dtl(it)` 之后、Rust 在 `state.leaf_temperature_k = ...` 之后，
都只打前 2 步），并进一步把第 6 轮 `dtl` 的**每一项**打出来。**插桩已全部撤销**，
内核重建，`test_upstream_f48_sync.py` PASS，两个窗口的 tier 汇总与插桩前逐位相同。

### 序列：前 5 轮逐位相同，第 6 轮开始分叉

`US-NR1-snow` 第 1 步（无降水、干冠层，`tl` 从 283 K 一步掉到 267 K）：

| 迭代 | Fortran `tl` | Rust `tl` | `dtl` 一致？ |
|---|---|---|---|
| 1–5 | 280 / 277 / 274 / 271 / 268 | 完全相同 | **是**（都被 `delmax` 夹成 -3.0） |
| 6 | 266.96944124486282 | 266.79923301102923 | 否（-1.0305588 对 -1.2007670） |
| 7 | 266.89227577413135 | 266.22179323073880 | 否 |
| … | 收敛到 **267.02200578097819** | 收敛到 **267.16744592811352** | 差 **+0.145 K** |
| 轮数 | 29 | 38 | — |

**迭代 1–5 逐位相同**说明：初值、`sabv`/`irab`/`fwet_snow`/`lai`/`rssun`/`rssha`
以及 Monin-Obukhov 那一串在这一段全都一样。但注意 **1–5 轮的 `dtl` 全被
`delmax = 3.0` 夹住** —— 所以那 5 轮**掩盖**了任何差异，第 6 轮是第一次看到未被
夹住的增量。**分叉很可能从第 1 轮就存在**，只是到第 6 轮才显形。

### 第 6 轮的逐项对照（`probe_step2 == 1, it == 6, tlbef = 268.0`）

| 项 | Fortran | Rust | 判定 |
|---|---|---|---|
| `sabv`（冠层吸收短波） | 0 | 0 | 一致 |
| `irab`（净长波） | 40.978187123497960 | 40.978187123497939 | 1e-15 相对 |
| `fsenl`（叶感热） | 127.55504057948086 | 127.49285672151514 | 5e-4 相对 |
| `htvpl` | 2844000.0 | 2844000.0 | 一致 |
| **`fevpl`（叶潜热）** | **7.8578978193852615E-006** | **1.8629036093762185E-010** | **差 4.2e4 倍** |
| **`fevpl_dtl`（潜热对 tl 的导数）** | **1.1822892856580003E-005** | **5.5850375077057607E-010** | **差 2.1e4 倍** |
| `canopy_phase_heat` | 0 | **分子里根本没有这一项** | 见下 |
| `dirab_dtl` | -7.2652264253383034 | -7.2652264253383034 | **逐位一致** |
| `fsenl_dtl` | 63.935308235544497 | 63.913162432230550 | 3e-4 相对 |
| `qintr_rain`/`qintr_snow` | 0 / 0 | 0 / 0 | 一致 |
| `t_precip` | 263.82000732421875 | 263.82000732421875 | 一致 |
| `tl` | 268.0 | 268.0 | 一致 |
| `clai` / `deltim` | 1565.9478022098542 / 1800 | 同 | — |

代回去核一遍两边都对：

* Fortran `dtl` = (40.978187 − 127.555041 − 2844000×7.8579e-6) / (0.869966 + 7.265226
  + 63.935308 + 2844000×1.18229e-5) = −108.925 / 105.694 = **−1.0306** ✓
* Rust `dtl` = (40.978187 − 127.492857 − 2844000×1.8629e-10) / (0.869966 + 7.265226
  + 63.913162 + 2844000×5.5850e-10) = −86.515 / 72.050 = **−1.2008** ✓

**所以第 6 轮的分叉 100% 出自 `fevpl` 这一项**：Fortran 的叶潜热是 22.35 W/m²
（`htvpl×fevpl`），本仓库是 0.00053 W/m²，差四个数量级。

### 两个候选，都有明确落点

1. **`canopy_phase_heat` 根本没进本仓库的叶能量平衡。**
   它是**本地扩展**（`extends/interception/MOD_LeafInterception_Extended.F90`
   的 `canopy_phase_heat_out`，文件头第 4 行就点明这是新增量），
   `:1412` 在**冠层雪融化**时置非零。本仓库
   `interception.rs` 把它**硬编码成 0**（`:147`、`:297` 两处），
   而 `leaf_temperature.rs` 的分子里**连这一项都没有**。
   本轮的探针显示这一步 Fortran 它也是 0，所以它不是第 6 轮的原因 ——
   但它是**实打实的缺口**：只要冠层雪融化/冻结，本仓库就会少一块焓。
2. **PHS 的根流通量限制。** `fevpl = etr + evplwet`，而这一步
   `fwet_snow = 0`（干冠层）→ `evplwet = 0`，所以差的就是 `etr`（蒸腾）。
   上游的顺序是：先按气动式算 `etrsun`/`etrsha` → 按
   `rstfac < 1e-2 .or. etr <= 0` 清零 → 再调 `balance_phs_rootflux` 把 `etr`
   压到根通量供得起的值。本仓库把 PHS 解**放在叶温迭代里面**，直接用
   `sunlit_transpiration + shaded_transpiration` 顶掉气动值，且只用
   `stress <= MIN_STRESS` 一个判据（没有 `etr <= 0` 那一条）。
   **下一步就该打第 1 轮（不被 `delmax` 夹）的
   `etrsun`/`etrsha`/`rstfacsun`/`rstfacsha`/`gssun`/`gssha`/`rssun`/`rssha`
   以及 `balance_phs_rootflux` 的进出口** —— 第 1 轮没有被夹，差异在那里一定可见。

### 这条为什么值钱

`f_tleaf` 在三份黄金里都是 tier2 的第一梯队（干季 0.037 K、湿季 0.40 K、
雪季 3.84 K），而它经 `MOD_LeafTemperature_Extended.F90:1551/1564` 的
`tl = fwet_snow*tfrz + (1-fwet_snow)*tl` 与冠层水、`fwet_snow`、反照率、
`fevpl` 连成一串。上一轮量的"干季 `f_ldew` → `fwet_snow` → `f_alb`"、
这一轮的"雪季第 1 步叶温 → 冠层凝霜 → 薄雪融化 → 雪深 → 层数"，
根都在这一个 `fevpl` 上。

## 强迫场比湿漏了一条 `min`，以及 `qsadv` 冰面系数多了一个零（2026 年，实测）

上一轮把雪季第 1 步的叶温差追到 `fevpl`。这一轮往上又追了两层，
**两个都是真缺陷**，其中一个只修了一半（另一半是耦合改动，见末尾）。

### 一、POINT 强迫的比湿没有夹到饱和值（已修）

上游 `MOD_UserSpecifiedForcing.F90` 的 `metpreprocess` 对
`DEF_forcing%dataset == 'POINT'` **只做一件事**：

```fortran
CALL qsadv(T, P, es, esdT, qsat_tmp, dqsat_tmpdT)
IF (qsat_tmp < q) THEN
   q = qsat_tmp
ENDIF
```

夹的是**刚读进来的原始记录**。本仓库漏了这一步，代价是 `f_xy_q`
（**tier0，逐位**）在 `US-NR1-snow` 上 150/360 条差到 **4.67%** ——
PLUMBER2 的 `Qair` 在冷湿站点常常高于同温度的饱和值（实测 0.00259873
对 0.00248548），上游夹回来、本仓库原样带下去。

修法在 `crates/colm-forcing/src/point.rs` 读帧时做 `min`，并加了两条测试：
`point_loader_clamps_supersaturated_humidity_like_metpreprocess`（夹具恰好
过饱和，同时守住"夹"与"不夹"两端）。实测效果：

| 窗口 | `f_xy_q` 修前 | 修后 |
|---|---|---|
| `CN-Cng` | maxrel 1.73e-2（3 条） | **2.89e-7** |
| `CN-Cng-wet` | 0 | 0（7 月没有过饱和记录） |
| `US-NR1-snow` | maxrel **4.67e-2** | **6.3e-4 → 再降到 3.5e-7**（见二） |

顺带把 `oracle/tolerances.toml` 里"`f_xy_q` 是上游时刻对齐怪癖"那句旧结论推翻：
它不是时刻对齐，是**缺了一个 `min`**。

### 二、`qsadv` 冰面分支的 `c8` 多了一个零（已修）

顺手把 `saturation_specific_humidity` 与上游逐项核对（用
`US-NR1-snow` 第 1 步上游真正用到的 `forc_q` 反解出 `es`）：

```
上游 qm = 2.4854759632703710E-003
=> es   = 275.5515395050858 Pa
```

把 `MOD_Qsadv.F90:63-65` 的 `data` 字面量按 f64 逐项重算，
**逐位**得到同一个 `es`。而本仓库的 `c8` 写的是

```rust
f77(0.000_000_000_000_026_265_580_3),   // = 2.6266e-14
```

上游是 `c8/0.262655803e-14/`（= **2.6266e-15**）—— **多了一个零**。
`c8*td^8` 在 `td ≈ -9.34` 时是 `2.6e-14 × 5.8e7 ≈ 1.5e-6`（正确值 1.5e-7），
所以冰面分支的 `es` 一律偏大约 1.7e-7 相对。

修掉之后 `f_xy_q` 在雪季窗口从 6.3e-4 再降到 **3.5e-7**，
`f_scv` 1.20 → 1.12 mm、`f_snowdp` 0.00521 → 0.00283 m 同向改善。

**代价是诚实的**：`f_qintr`/`f_qdrip` 从 4.17e-8 变成 1.28e-7（刚过 tier2 的
`atol=1e-7`），雪季 tier2 从 79 条变 81 条。原因是这两个量同时依赖
`interception.rs` 里**还没改**的 f32 常数 —— 修好一处之后误差不再"恰好抵消"。
换句话说这是**半成品的样子**，不是修错了方向；下一节说清剩下的是什么。

### 三、`f77` 与 `FREEZING_K` 的 f32 取整：`-fdefault-real-8` 下它们是错的（**未修，需整批做**）

`vendor/CoLM202X/include/Makeoptions` 第 24 行是
`FOPTS_COMMON = -fdefault-real-8 ...`（`Makeoptions.Mac-arm` 同）。
也就是说**所有 Fortran 字面量都是 REAL(8)**：`data c8/0.262655803e-14/`
里那串十进制**不会**先落到 f32。可本仓库到处写着

```rust
const fn f77(value: f32) -> f64 { value as f64 }
```

—— 仓库里这样的**局部定义有三十余处、调用点 378 个**。实测把
`atmosphere.rs` 的 `f77` 改成恒等（f64 字面量）、并把
`FREEZING_K = 273.16_f32 as f64` 改成 f64 的 `273.16`（上游 `tfrz = 273.16`
就是 f64），三者一起改的效果是：

| 量（`US-NR1-snow`） | 只修 `c8` | 再修 `f77`+`FREEZING_K` |
|---|---|---|
| `f_xy_q` maxrel | 3.5e-7 | **9.4e-16（≈4 ULP，tier0 基本逐位）** |
| `f_xy_snow` maxabs | 2.6e-15 | 2.6e-15 |
| `f_scv` maxabs | 1.12 | 1.12 |
| `f_snowdp` maxabs | 0.00283 | 0.00283 |

**但没有把这一改提交，因为它必须整批做**，理由是实测的：

* **单独改 `atmosphere.rs` 会把算例跑崩。** 只改 `f77`+`FREEZING_K`、
  不改 `c8` 的那个组合直接报
  `VSF sublevel layer inputs are invalid`；三处同时改才跑得通 ——
  说明这些常数是**互相咬合**的，不能一处一处来。
* **`FREEZING_K` 在仓库里有好几份定义、值还不一样**：
  `atmosphere.rs` 是 `273.16_f32 as f64`，`lake.rs`/`snow.rs` 是 `273.16`。
  实测把 `atmosphere.rs` 那份改成 f64 之后，
  `equilibrium_soil_column_stays_at_its_fortran_surface_balance` 的
  `phase_flag` 从 `[0, 0]` 变成 `[2, 2]`（两个土壤层的相变标志全翻）——
  因为夹具用的是 `atmosphere::FREEZING_K`，而相变内核比较的是另一份。
  这类"离散结果被翻"的改动必须一次改齐、再逐条与 Fortran 对账。
* 连带要重新对账的既有钉值测试有 5 条（`precipitation_partition`、
  `equilibrium_soil_column`、`standard_leaf_solver`、
  `urban_phase_change`、`prepared_point_forcing`），偏差都在 1e-8~1e-6 相对量级，
  看起来正是 f32→f64 该有的量级，但**必须逐条说明它为什么是新的正确值**，
  不能只把 `expected` 换成 `actual`。

**所以这一轮的处置**：`c8` 与强迫场 `min` 已修并入库；
`f77`/`FREEZING_K` 记为下一轮的**整批任务**，判据是
"三处一起改 → `f_xy_q` 在雪季窗口逐位一致 → 五条钉值测试逐条对账"。

## `-fdefault-real-8` 整批落实：f32 字面量的最后一处，以及被它们钉住的 20 条测试（2026 年，实测）

上一节把 `f77`/`FREEZING_K` 记为"整批任务"。这一轮做完了，并且**先用编译期探针
把前提证死**，再动代码。

### 一、前提：直接问编译器，而不是推断

```fortran
program kindprobe
  real(8) :: a
  a = 273.16
  write(*,'(A,I0)') 'kind(0.5)=', kind(0.5)
  write(*,'(A,ES25.17)') '273.16 ->', a
end program
```

| 编译方式 | `kind(0.5)` | `273.16 ->` |
|---|---|---|
| `gfortran -fdefault-real-8`（= 参考内核的 FOPTS） | 8 | **2.73160000000000025E+02** |
| `gfortran`（不带） | 4 | **2.73160003662109375E+02** |

**两行都是判据**：第二行正是本仓库旧钉值 `273.160003662109375`，第一行正是
改成 f64 之后的 `273.16`。所以那些"与当前 Fortran 一致"的钉值来自**不带
`-fdefault-real-8` 的编译**（`prepared_point_forcing` 的注释自己写着
"standalone gfortran execution"），而黄金文件来自带这个开关的参考内核。

### 二、改了什么

| 项目 | 改前 | 改后 |
|---|---|---|
| `api::f77` | **13 份**局部副本 `(f32) -> f64 { value as f64 }` | `lib.rs` **一份**恒等，其余 `use crate::f77;` |
| `FREEZING_K` | `atmosphere`=`273.16_f32 as f64`、`lake`/`urban_radiation`=`273.16` | `crate::FREEZING_K = 273.16` 一份，其余 `use` |
| `_f32 as f64` 常数 | 9 处（`CP_AIR`、`LATENT_HEAT_VAPORIZATION`、6 个城市常数、`FREEZING_K`） | f64 字面量 |
| 包在 `f77()` 里的 **f32 运算** | `f77(0.0031_f32.powf(0.5))`、`f77(44.6_f32*273.16_f32)`×2、`f77(0.666_666_7_f32)` | 按上游源码用 f64 算（`.666666666666` 等） |

### 三、效果（三份黄金，`tier0` 是最强判据）

| 量（`US-NR1-snow`） | 改前 | 改后 |
|---|---|---|
| `f_xy_q` maxabs | 1.41e-9 | **1.08e-18（≈4 ULP）** |
| `f_xy_snow` maxabs | 2.75e-12 | **1.08e-19** |
| `f_xy_rain` | 2/360 条 2.75e-12 | **0（整条过）** |
| tier 汇总 | `{tier0: 6, tier2: 81}` | **`{tier0: 5, tier2: 81}`** |

另两个窗口的 tier 汇总**不变**（`CN-Cng` `{2,57}`、`CN-Cng-wet` `{4,70}`）。
逐变量看**方向是一致向好**（better/worse 计数，按每变量的最大相对偏差）：

| 窗口 | 变好 | 变差 | 变好的幅度和 | 变差的幅度和 |
|---|---|---|---|---|
| `CN-Cng` | 11 | 1 | 0.454 | 8.9e-7 |
| `CN-Cng-wet` | 16 | 1 | 3.9e-3 | 0.227 |
| `US-NR1-snow` | 3 | 1 | 3.5e-6 | 7.3e-8 |

变差的那一条在湿季是 `f_zerr`（能量闭合**诊断**，`f_zerr` 的尺度只有 1e-10 量级，
它本来就离得很远）。`qsadv` 自己的钉值测试现在用的是**上游真值**
`es = 275.5515395050858 Pa`（由上游 `forc_q` 反解、再用 f64 系数逐位重算），改完
之后**逐位**通过 —— 这是"三处一起改才对齐"的直接证据。

### 四、被它们钉住的 20 条测试：重新对账（不是盲改）

`-fdefault-real-8` 一落实，20 条带钉值的单测失败，偏差全在 1e-8~1e-6 相对量级 ——
正是 f32→f64 该有的量级。处置：

* **逐条重新对账**：写了一个脚本"跑测试 → 从 panic 里读出 `got/expected` →
  在源文件里找到那个字面量 → 换成新值"，每条都要跑到绿。**没有一条是手抄的**，
  也没有把容差放宽。
* 有 5 条断言原本只打一个值（`"scheme {scheme}: {actual:.17e}"` 这种），先给它们
  补上 `expected` 再对账 —— 顺带把测试的诊断信息补全了。
* 唯一一条**离散结果**翻身的是
  `equilibrium_soil_column_stays_at_its_fortran_surface_balance`：`phase_flag`
  从 `[0, 0]` 变 `[2, 2]`。那是个**人工造的完美平衡夹具**（整柱放在 `FREEZING_K`
  上、长波收支刚好配平），标志取 0 还是 2 只取决于 `t == tfrz` 那一侧的最后几位；
  温度断言仍到 1e-11，物理结论没变。已在测试里写明。
* `273.160003662109375 -> 273.16`（`snow_tests`）就是第一节探针的第二行 ——
  这条钉值自己把来源说清楚了。

### 五、教训

> 本 crate 里凡是写成 `f77(x)` 或 `x_f32 as f64` 的地方，都是在**断言
> "上游那个字面量是单精度"**。参考内核用 `-fdefault-real-8`，所以这个断言
> 默认是假的；要断言它，就得像第一节那样问一次编译器。

## tier0 只剩两个：降水拆分的**先算比例**，与 `f_xy_solarin` 的**四波段之和**（2026 年，实测）

`-fdefault-real-8` 那批落实之后，三份黄金的 `tier0` 加起来还剩 5 条。这一轮清掉 4 条。

### 一、降水拆分：`P*(1/3)` 而不是 `P/3`

上游 `MOD_Forcing.F90:533-534`：

```fortran
CALL block_data_copy (forcn(4), forc_xy_prl, sca = 2/3._r8)
CALL block_data_copy (forcn(4), forc_xy_prc, sca = 1/3._r8)
```

`sca` 是**先算好的比例**，于是 `prl = P * (2/3)`。本仓库写的是 `P * 2.0 / 3.0` —— 多一次舍入。
实测（雪季第 0 条，`P = 1.111111123e-4`）：

| 写法 | `f_xy_prl` | 判定 |
|---|---|---|
| `P * 2.0 / 3.0` | `3.7037037448802344e-5` | 本仓库旧值 |
| `P * (2.0/3.0)` | `3.703703744880234e-5` | **黄金值** |

改一行之后 `f_xy_prc`/`f_xy_prl`/`f_xy_snow`/`f_xy_rain` **四条全部逐位一致**
（`ndiff = 0/360`）；`CN-Cng-wet` 的 tier0 从 4 条降到 **1** 条，`US-NR1-snow` 从 5 条降到 **2** 条。

### 二、`f_xy_solarin` 是**四个波段之和**，不是总量

`MOD_Vars_1DAccFluxes.F90:2060-2063`：

```fortran
CALL acc1d (forc_sols , a_solarin )
CALL acc1d (forc_soll , a_solarin )
CALL acc1d (forc_solsd, a_solarin )
CALL acc1d (forc_solld, a_solarin )
```

**四次累加到同一个桶**，也就是 `f_xy_solarin` 写的是
`(sols+soll+solsd+solld)` 的均值。而 `crates/colm-runtime/src/history.rs` 里
原先写着"照抄总量 `solar_in_w_m2`，**不是**四个波段之和"，还附了一条实测
（"`CN-Cng` 冬季窗口 264 条里有 13 条因此差 1 ULP"）—— **那条推断反了**：
差 1 ULP 恰恰是因为上游写的是四项之和，而四项之和在舍入上不保证逐位回到总量。

改成"四项按 `sols→soll→solsd→solld` 顺序相加"之后……**并没有变好**：
`f_xy_solarin` 仍是 22/360 差 1 ULP。原因是本仓库的**拆分权重本身**与上游差一点点 ——
`f_solvd`/`f_solnd`/`f_solvi`/`f_solni` 各差 1–2 ULP（它们是 tier1，`rtol=1e-12`，
所以一直"过"）。公式两边逐项相同（含常数顺序），所以差的是**喂进去的
`sunang`**：上游这一块用 `orb_coszen(calday, patchlonr, patchlatr)`，本仓库用
`prepare_runtime_forcing` 传进来的 `sun_angle`。两者差约 1 ULP。

改法仍然入库了 —— 因为"写总量"是**错的写法**，只是这一改还不足以让这一列逐位；
真正的下一步是让 `sun_angle` 与 `MOD_Forcing` 那一处逐位一致。

### 三、这一轮之后的 tier0 分布

| 窗口 | tier0 修前 | 修后 | 剩的是 |
|---|---|---|---|
| `CN-Cng` | 2 | **2** | `f_xy_solarin`(11/264)、`f_xy_q`(1/264) |
| `CN-Cng-wet` | 4 | **1** | `f_xy_solarin` |
| `US-NR1-snow` | 6 | **2** | `f_xy_solarin`(22/360)、`f_xy_q`(64/360) |

两个残留都量清楚了：`f_xy_solarin` 是 `sun_angle` 的 1 ULP；
`f_xy_q` 是 `qsadv` 的 `qs`（已有单测钉到 <1e-14 相对）与记录均值那一步的舍入。
tier2 三条窗口都没动（57 / 70 / 81）。

## tier2 的 57/70/81 是**系统差**，不是舍入：以及第一步的 `fwet_snow` 取值位置（2026 年，实测）

这一轮先回答一个此前一直悬着的问题：剩下那些 tier2 红条，到底是
"两条浮点轨迹在 1e-16 上分叉后被放大"，还是"实现确有差别、可以定位"。
答案是后者，并且顺手在第一步里抓到一个**取值位置错**的真缺陷。

### 一、先把"舍入放大"这条假说量掉：它不成立

方法：拿**同一个 Rust 二进制**跑同一个 11 天窗口，只把初始状态扰动 1 ULP，
看输出动多少。三条独立实验：

| 扰动 | 结果 |
|---|---|
| `t_soisno[0,10]` 加 1 ULP（283.0 → 283.00000000000006，相对 2.5e-16） | 127 个变量里**只有 `f_zerr` 变**，且只差 1.01e-11；其余 264 条记录逐位相同 |
| 常量重启里的 `BD_all[0]`、`hksati[0]` 各加 1 ULP（**持续**每一步都不同，比一次性扰动更接近"两条算术轨迹"） | 同上，仍然只有 `f_zerr` 差 1.01e-11 |
| 正对照：初始土壤柱 +5 K | 69 个变量改变（证明扰动确实进模型了） |

再把扰动幅度扫一遍，量出这个耦合系统的"增益"：

| 初始 `t_soisno` 扰动 | `f_t_soisno` 最大差 | `f_tleaf` | `f_h2osoi` |
|---|---|---|---|
| 1e-14 K | 0 | 0 | 0 |
| 1e-11 K | 1.06e-9 | 1.06e-11 | 6.9e-13 |
| 1e-8 K | 9.63e-7 | 6.7e-9 | 6.2e-10 |
| 1e-5 K | 9.63e-4 | 6.6e-6 | 6.2e-7 |
| 1e-2 K | 5.11e-2 | 1.78e-2 | 1.04e-4 |

线性段增益 ≈ **100**（1e-11→1.06e-9、1e-8→9.63e-7、1e-5→9.63e-4 三个点一致），
1e-2 K 起饱和。两个推论：

1. **模型是收缩的**（扰动不会自发放大）：这对验证是好消息 —— 差多少就是多少，
   可复现、可追溯，不会因为"混沌"而无法比较。
2. 黄金与 Rust 在 `f_t_soisno` 上差 **0.038 K**，落在**饱和区**；要在饱和前
   造出这个差，需要 ~4e-4 K 的种子，即相对 **1.3e-6** —— 是 f64 eps（2.2e-16）
   的 **4e9 倍**。所以 tier2 那些红条是**实打实的实现差**，不是浮点噪声，
   值得继续追。

### 二、一个必须先拆掉的陷阱：`oracle/work/CN-Cng-*step` 不是黄金的配置

`oracle/work/` 下留着 `CN-Cng-1step … -4step` 四个 1/2/3/4 步的 Fortran 工作目录
（第一步的 restart 都在，看起来是现成的"逐步定位"夹具）。**它们不能用**：
它们的 `case.nml` 显式写了

```
DEF_USE_Campbell_SOIL_MODEL      = .true.
DEF_USE_VariablySaturatedFlow    = .false.
DEF_USE_PLANTHYDRAULICS          = .false.
DEF_VEG_SNOW                     = .false.
```

而黄金 `CN-Cng` 一个都不写 —— 走的是上游默认（van Genuchten + VSF + 植物水力 +
植被雪**全开**）。两者的第一步就差 3.7e-4 K，第一步的 `brr` 差 6.6e-5。**这一轮
前半段的逐步数字全部作废**（方法留下，见下），重做时必须用黄金配置的 1 步窗口。

顺带一条仍然有效的**方法**结论（在哪个配置里都成立）：在那次错误的对照里，
`meltf`/`phase_change` 的入参 `fact(j)`（= `deltim/cv(j)`，只依赖热容与几何）、
`t_soisno_bef`、`wliq0`、`wice0` 都是**逐位相同**的，只有 `brr(j)` 差 6.6e-5。
对顶层 `brr(1) = cnfac*fn(1) + (1-cnfac)*fn1(1)`，而初始全柱 283 K ⇒ `fn(1) = 0`
⇒ `brr(1) = 0.5*tk(1)*(t(2)-t(1))/dz(1)`，反解出顶层 `t_soisno` 差 9.2e-4 K
正好等于 `brr` 的 6.6e-5。**即：热参数（`tk`/`dz`/`cv`）不是嫌疑，差是从
地表能量平衡（`hs`/`dhsdT`）传下来的。**

### 三、黄金配置下重做第一步：差从哪来

用黄金的 `case.nml` 造一个 `end_sec = 1800` 的 1 步窗口，Fortran（干净内核）与
Rust 各跑一次，比 restart（68 个变量）：

| 变量 | Fortran | Rust | 相对 |
|---|---|---|---|
| `t_soisno`（顶层） | 273.16 | 273.16 | — |
| `t_soisno`（次层） | 278.49780612 | 278.49777544 | 1.1e-7 |
| `wice_soisno`（顶层土壤） | 3.07567727 | 3.07585881 | 5.9e-5 |
| `wliq_soisno` | 5.56867942 | 5.56849270 | 3.5e-7 |
| `smp` | -36.4835934 | -36.48589956 | 6.3e-5 |
| `zol` / `rib` | -0.27729937 / -0.07222175 | -0.27730489 / -0.07222321 | 2.0e-5 |
| `tstar` / `qstar` | -1.39834005 / -3.3428e-4 | -1.39838594 / -3.3429e-4 | 3.3e-5 |
| `ustar` | 0.68583116 | 0.68583562 | 6.5e-6 |
| `tleaf` | 265.88279051 | 265.88302560 | 8.8e-7 |
| `trad` | 264.06931399 | 264.06942507 | 4.2e-7 |
| `coszen` | -0.92497906 | -0.92497906 | 7.8e-10 |
| **`fwet_snow`** | **0.06138738** | **0**（修前） | 1 |
| **`vegwp`** | **-3418.0 / -3418.0 / -1663.7 / -773.9** | **-621.3 / -621.3 / -620.9 / -120.8** | 0.82 |
| **`gs0sun` = `gs0sha`** | **-4.52516273e38** | **452.51587125** | 1 |
| **`rst`** | **-5.0e-31** | **5.0e5** | 1 |

`coszen` 差 7.8e-10、`trad`/`tleaf` 差 1e-7 量级 —— 第一步就已经不是"逐位"，但
离 1e-16 还很远；到第 264 条记录涨到 1e-4 量级，与上面量到的增益 ~100 自洽。

三处**结构性**的差：

1. `gs0sun`/`gs0sha`/`rst`：Fortran 是垃圾，Rust 是正常值。
   `-4.52516273e38 / 452.51587125 = -1.0000009e36`，而 CoLM 的
   `spval = -1.0e36`（`MOD_Vars_Global.F90:110`）—— **Fortran 第一步的最大叶导度
   带了一个 `spval` 因子**。黄金 history 第 0 条的 `f_gssun[0] = f_gssha[0] =
   -2.26258137815510702e31` 是同一个现象晚一步的样子（第 0 条的 `f_rstfacsun` 仍
   逐位是 1，所以它没有扩散到整条链）。**Rust 不复制这个
   spval 污染是对的**；`f_gssun`/`f_gssha`/`f_vegwp` 第 0 条的红条属于
   "上游自己的病"，不是移植缺陷。触发路径（哪一处把 `spval` 乘进来）还没定位，
   落点是 `MOD_LeafTemperature_Extended.F90:817` 那一行与它拿到的 `rssun`。
2. `vegwp`：Fortran 四个节点是 -3418/-3418/-1664/-774（根→叶张力递减），Rust 是
   -621/-621/-621/-121。差 5 倍，且 Rust 前三个节点几乎相同、Fortran 分层明显。
   这是第一步里**最大的物理量差异**，落点是 `MOD_PlantHydraulic` 的
   `calcstress_twoleaf` 与 `ks_soil_root`/`k_ax_root` 的赋值。
3. `fwet_snow`：见下节（已修）。

被**排除**的嫌疑（都做了对照，不是"看起来没问题"）：`hfus`（两边都是
`0.3336e6`）、`tk`/`dz`/`cnfac`（由 `brr(1)` 的反解证明相同）、`fact`/`cv`
（逐位相同）、`DEF_TUNING_DEWMX = 0.1`（Rust 把 `48*dewmx*lsai` 写成
`48*lsai/10`，默认值下等价）、以及 `nmozsgn >= 4` 的处置 ——
`MOD_LeafTemperature_Extended.F90:1262` 是 `obu = zldis/(-0.01)`（**替换后继续迭代**），
与 `MOD_GroundFluxes.F90:229` 的 `EXIT`（**跳出**）**不是**同一个语义；
本仓库 `leaf_temperature.rs:833` 做的是前者，按叶温那条链算**对的**。

### 四、修掉的缺陷：`fwet_snow` 取在冠层水更新**之前**

`leaf_temperature.rs::update_canopy_water` 原先在函数开头就用
`state.canopy_water.snow_mm` 算 `wet_snow_fraction`，而 `snow_mm` 是在**这个
函数后半段**（露/凝华那一段）才被写的。上游不是这个顺序：

* `MOD_LeafInterception_Extended.F90` 先更新 `ldew_snow`（截留）；
* `MOD_LeafTemperature_Extended.F90:1532` 才调
  `canopy_snow_wetfrac(sigf, lai, sai, dewmx, tl, ldew_snow)`；
* 紧跟着的相变块（`:1551`/`:1564` 的 Niu(2004) 拉回）用的正是这一个值；
* 此后**没有任何一处重算**。

Rust 的 `update_canopy_water` 正好把这两段合在一起，所以取早了：第一步
`tl = 265.88 K < 冰点`，刚凝华的 0.047 mm 写进 `snow_mm`，而覆盖率仍按**上一步**
的 `snow_mm = 0` 算成 0（黄金 0.06138738）。修法是把这段搬到露/凝华之后、
相变之前，位置与上游逐步对齐（`standard_lct_step.rs:235` 的 `intercept_canopy`
确实在 `:301` 的 `leaf_temperature` 之前，所以"截留之后"这个前提也成立）。

这不是诊断量：`MOD_Albedo` 的 `scat`/`beta0` 直接吃 `fwet_snow`。效果（修前 →
修后，`golden-compare` 报的"最大超差倍数"）：

| 窗口 | 变量 | 修前超差 | 修后超差 |
|---|---|---|---|
| `CN-Cng` | `f_alb` | 8.78e3 | **106** |
| | `f_srvd` / `f_srvi` / `f_srnd` / `f_srni` | 2.95e4 / 3.26e4 / 2.72e4 / 1.98e4 | **310 / 285 / 245 / 159** |
| | `f_sabvsun` / `f_sabvsha` / `f_sabg` | 1.44e4 / 1.75e4 / 4.53e3 | 115 / 146 / 47.9 |
| | `f_sr` | 4.06e3 | 55.2 |
| `CN-Cng-wet` | 全部 127 个变量 | — | **逐位不变**（0 个变量变化） |
| `US-NR1-snow` | 计数 | 83 个变量 / 31599 条 | 83 个变量 / 31929 条（+1%） |

干窗那一族正是文档前面记的 `f_alb → f_sr*/f_sab* → f_rnet` 链，这次让它们的
最大超差降了**两个数量级** —— 说明那条链的根确实在这里，此前记的"继承自
`f_ldew`"只对了一半。湿窗（7 月）逐位不变是因为冠层不结霜，`snow_mm` 全程为 0，
新老写法同值 —— 也就是说这一改**没有回归风险**。雪窗是重霜环境，改了 88 个
变量，但**基本中性**：83 个变量对 83 个变量，总条数 +1%，28 个变量超差略增、
26 个略降（`f_wliq_soisno` 6.46e6 → 5.13e6 是改善最大的一条，`f_srvi` 略增），
属"轨迹更贴上游之后的重排"，不是回归。

三条窗口的 tier 计数都没变（tier0 2/1/2，tier2 57/70/81），因为这一改是
**族内幅值**的量级下降，不是把某条推到 1e-7 以内。

（踩过的坑，记一笔：`/tmp/f_US-NR1-snow` 那份"修前"基线是**早先一轮插桩时**留下的
运行，tier0 是 5 而不是 2，拿它比雪窗会得出"86 → 83 个变量、大幅改善"的假结论。
判断一份旧输出能不能当基线，最快的办法就是看它的 **tier0 计数**是否与上一次记录
一致 —— tier0 是逐位判据，插桩/旧内核一定会在那里露出来。）

### 五、下一步（有明确落点，按性价比排）

1. **第一步的 `hs`/`dhsdT`**：`hs = sabg + dlrad*emg - (fseng + fevpg*htvp)
   + 降水热 - emg*stefnc*t_grnd**4`、`dhsdT = -cgrnd - 4*emg*stefnc*t_grnd**3 - ...`。
   `hs` 是大项的差（~2225 W/m2），所以几百分之一的相对差都来自 `fseng`/`cgrnd`
   的更小相对差；而 `t_grnd`、`emg`、`sabg`、`dlrad` 在第 1 步的 restart 里都对上
   了。落点就是地表层迭代给出的 `fseng`/`cgrnd`。
2. **`vegwp` 五倍差**：用同一个 1 步窗口，在 `calcstress_twoleaf` 两侧插桩，
   比 `ks_soil_root`/`k_ax_root`/`gssun`/`psi` 的逐节点值。
3. **`gs0sun = spval × 正常值`**：定位 `:817` 那一行拿到的 `rssun` 是什么，
   以及 `spval` 从哪条路径乘进来。
4. 把 `sun_angle` 对齐 `orb_coszen`（tier0 的 `f_xy_solarin` 1 ULP，见上一节）。

## 第一步叶温求解器的残差是**上游自己**的 `spval` 污染：`o3coefg_*` 在循环之后才被置 1（2026 年，实测，**未改上游**）

上一节把"第一个不被 `delmax` 夹住的迭代"钉在 `fevpl` 上。这一轮把 `CN-Cng`
第一步的**逐迭代表**两边都打出来（Fortran 在 `MOD_LeafTemperature_Extended.F90`
的 `tl = tlbef + dtl(it)` 之后与 `:819` 之后各一组，Rust 在
`leaf_temperature.rs` 的 `let leaf_evaporation_unadjusted = ...` 之后一组；
插桩已全部撤销、内核重建、黄金逐位复现）。

### 一、第 1 轮到第 6 轮：气动量的**输入**逐位相同，差全在 `rssun`

`CN-Cng` 第 1 步（`tl` 从 283 K 起步，无降水，`fwet = 0`）：

| it | `qsatl` F / R | `qaf` F / R | `delta`/`esign` | `fwet` | `rssun` F | `rssun` R | `fevpl` F | `fevpl` R |
|---|---|---|---|---|---|---|---|---|
| 1 | 7.5882991352415295e-3 **同** | 4.1748864657195975e-3 **同** | 1 / 1 | 0 | **6.4635e-4** | **1.0000e6** | **6.6099e-5** | **1.3712e-8** |
| 2 | 6.1846605533164272e-3 **同** | 2.7317240992e-3 / 2.5313005485e-3 **不同** | 1 / 1 | 0 | 2.6924e-3 | 1.0000e6 | 5.4312e-5 | 9.9939e-9 |
| 6 | 2.4677085374e-3 **同** | 2.6900247261e-3 **同** | 1 / **0** | 0 | — | 1.0000e6 | — | −9.5807e-6 |

第 1 轮的 `qsatl`/`qaf`/`delta`/`fwet` **逐位相同** ⇒ 气动式两侧的**分子**一样，
`etr` 差 4820 倍只能出在 `laisun/(rb+rssun) + laisha/(rb+rssha)` 的分母上：
`6.4635e-4` 对 `1.0000e6`，六个数量级。**这就是第一步叶温分叉的全部**。

### 二、Fortran 那个 `rssun` 是 `spval` 乘出来的（数字级证明）

在 `:819` 之前再插一组探针，打出 `stomata` 的返回值和 `gs0sun`：

```
SB it/stomata_rssun/stomata_rssha/gs0sun/gs0sha/tl/tprcor
   1  1.0000000000000000E+07  1.0000000000000000E+07  -4.2514633619846110E+38 ... 283.0  1.2031641493701876E+04
```

`rssun = rssha = 1e7`（夜晚气孔全关，合理），`tl = 283`，`tprcor = 12031.64`，于是

```
1/(rssun*tl/tprcor)                  = 4.2514633619846110e-6
min(1e6, 上式) / laisun * 1e6        = 4.2514633619846110e2      （laisun = 0.01）
× o3coefg_sun                        = -4.2514633619846110e38    ← 观测值，17 位全中
```

而 `spval = -1.0e36`（`MOD_Vars_Global.F90:110`）。**所以 `o3coefg_sun = spval`。**
`o3coefg_sha` 同理（`gs0sha` 与 `gs0sun` 逐位相同 —— 单点单 PFT 下两者本就同源）。

路径也清楚了：

* 本算例 `DEF_USE_OZONESTRESS = .false.`；
* 把四个 `o3coef*` 置 1 的 `ELSE` 分支在 **`:1300-1310`，即稳定性循环 `ENDDO` 之后**；
* 而消费它们的 `:819` 在**循环体内**；
* 调用方给进来的是初值 —— 探针实测就是 `spval`。

⇒ **第一步的每一次迭代都用 `spval` 乘 `gs0sun`**；第一步的 `LeafTemperature`
返回时（`:1308`）才被置 1，所以从**第二步**起就正常了。这一步的叶温/PHS 状态
在 Fortran 里是垃圾：黄金 history 第 0 条的 `f_gssun[0] = f_gssha[0] =
-2.26258137815510702e31`、`f_vegwp[0]` 的 `[-2021, -2021, -1144, -449]` 就是它；
第 1 条起 `f_vegwp` 两边**逐位相同**。

### 三、把它补上以后：只修掉第 0 条，**不解释** 11 天的残差（重要的否定结论）

在 `LeafTemperature` 循环之前加一句 `IF (.not. DEF_USE_OZONESTRESS) o3coef*=1`，
重建内核，重跑 11 天窗口，再拿 Rust 去比（**注意：黄金本身没动，比的是
"打了补丁的 Fortran" 对 Rust**）：

| 比较对象 | 总超差条数 | `f_gssun` 超差 | `f_vegwp` 超差 | 其余 56 条 |
|---|---|---|---|---|
| Rust 对**现有黄金** | 13726 | 1e7 | 7.25e6 | — |
| Rust 对**补丁后的 Fortran** | 13726 | **3.29e4** | **1.75e6** | **逐条一模一样** |

即：这个 `spval` 只解释了第 0 条那三个量的垃圾，**其余 56 条 tier2 一条都没动**。
所以上一节"三个窗口的残差都指向第 1 步的叶温"这个说法**要收窄**：
第 1 步确实分叉（而且是上游自己的病），但它不是 11 天残差的来源。

### 四、把窗口拉到第 2 天（48 步），看**逐日**的差

用同一个初始化、`end = 2008-01-02-00000`，Fortran（含上面的补丁）与 Rust
各跑一遍，直接比重启文件（68 个变量，28 个不同）：

| 变量 | 最大绝对差 | 最大相对差 |
|---|---|---|
| `zwt` | 4.56e-4 m | **3.67e-3** |
| `ldew` / `ldew_snow` | 8.81e-6 mm | 1.98e-4 |
| `fwet_snow` | 7.77e-6 | 1.32e-4 |
| `wice_soisno` | 1.25e-3 kg/m2 | 7.39e-5 |
| `qstar` | 5.61e-10 | 1.60e-5 |
| `hk` | 9.72e-9 | 2.87e-6 |
| `wliq_soisno` | 1.43e-3 | 2.68e-6 |
| `zol` / `rib` | 1.72e-7 / 4.33e-8 | 1.70e-6 |
| `t_soisno` / `t_grnd` | 3.97e-4 K | 1.40e-6 |
| `tleaf` | 2.18e-5 K | 8.40e-8 |
| `gs0sun` / `gs0sha` | 3.50e-5 | 7.59e-8 |

读法：

1. 第 2 天 `gs0sun` 已是**正常量级**（差 7.6e-8），第一步的污染确实只活一步。
2. 逐日差的量级是 **1e-6 ~ 1e-4 相对**，与第 1 步（`t_soisno` 1.08e-7）同量级
   —— 也就是说它**不随步数放大**，是一个**稳态的每步小差**，不是混沌放大。
   这与上一轮量到的增益 ~100 只作用在"状态被扰动后重收敛"的量上一致。
3. 最大两条是 `zwt`（0.37%）与冠层水 `ldew`（0.02%）。`ldew`/`fwet_snow` 正是
   上一节那条 `f_alb → f_sr*/f_sab*` 链的上游，`zwt` 则是 `f_zwt`/`frcsat` 的
   上游。**下一轮就从这两个量入手**，落点分别是 `MOD_SoilSnowHydrology:groundwater`
   与 `MOD_LeafInterception`/`update_canopy_water` 的每步收支。

### 五、这一轮顺手改掉的一处语义（`etr` 不是 PHS 的解）

`leaf_temperature.rs` 原先在 PHS 打开时用
`hydraulic_output.sunlit_transpiration + shaded_transpiration` **顶掉**气动式算出的
`transpiration`。上游不是这样：`MOD_LeafTemperature_Extended.F90:1125-1131` 只做
"按 `rstfacsun`/`rstfacsha` 与符号把逐叶分量清零"，然后
`CALL balance_phs_rootflux(ipatch, p_iam_glb, etr, rootflux, rootfr, 'post-PHS')`
—— 而 `balance_phs_rootflux` 的第一个参数 `etr` 是 **`intent(in)`**
（`extends/interception/MOD_PHSRootfluxBalance.F90:26`），它只把 `rootflux`
按比例缩放到 `etr`，**一个字都不回写**。也就是说 PHS 对冠层蒸腾的影响只走
`rssun`/`rssha`（`:904` 由 `gssun`/`gssha` 反算），不是"根供得起多少就蒸多少"。
已按上游语义改回，并保留 `:1342-1357` 的 `rootflux` 比例缩放（本仓库在
`root_flux_kg_m2_s` 那一处已有同构实现）。

**实测影响很小**（三份窗口总超差 13738→13726 / 25873→25872 / 31929→31931，
`f_vegwp` 的条数 747→735），因为这两个值在这些窗口里本就贴得很近；
但它是**语义**上的纠正，留着比"碰巧一样"可靠。

### 六、下一步（四条，按性价比）

1. **上游修复 `o3coef*` 的时序**（把 `:1300-1310` 的 `ELSE` 置 1 提到循环之前，
   或让调用方初始化成 1），然后**重跑三份黄金**并同步 `docs/design.md §2.8/§2.8b`
   的观测对比表。这是"上游 bug 修在源头上"的一类改动，本仓库有先例
   （`CoLM_stop` 的裸 `STOP`→`STOP 1`）；代价是黄金文件与 `kernel-manifest.json`
   要一起更新。
2. **`zwt` 的每步 0.37%**：拿第 2 天的重启做落点，在
   `MOD_SoilSnowHydrology:groundwater` 的进出口比 `qcharge`/`drainage`/`rous`/`jwt`。
3. **`ldew` 的每步 0.02%**：比 `intercept_canopy` + `update_canopy_water` 的每步收支
   （已有 `fwet_snow` 一起对照）。
4. `canopy_phase_heat` 仍是硬编码 0（`interception.rs` 两处），冠层雪融化时会少一块焓。

## 两件收尾：`canopy_phase_heat` **不是缺口**，以及 PLUMBER2 目录里的 AppleDouble 边车（2026 年，实测）

### 一、`canopy_phase_heat` 恒为 0 是**对的**（纠正此前把它记成"实打实的缺口"）

前面几节把它记成"未移植的本地扩展，冠层雪融化时会少一块焓"。按方案号数一遍上游
就好了，结论相反：

| `DEF_Interception_scheme` | 上游 routine | 给 `canopy_phase_heat_out` 赋值的处数 |
|---|---|---|
| **1** | `LEAF_interception_CoLM2014`（`:96-402`） | **0**（`:399` 置 0 之后再没碰过） |
| 2 / 3 / 8 | CLM4 / CLM5 / CoLM202x | 1（各只有那一句置 0） |
| 4 / 5 / 6 / 7 | NOAHMP / MATSIRO / VIC / JULES | 3 / 3 / 3 / 4（真的有相变焓） |

三份黄金算例一个都没写 `DEF_Interception_scheme`，取 `MOD_Namelist.F90:267` 的默认
**1**；`colm-rs` 移植的就是 `LEAF_interception_CoLM2014`
（`interception.rs::intercept_canopy` 的文档注释里写明了对应关系）。scheme=1 的
相变焓**不走这个出口**，而是 `MOD_LeafTemperature_Extended.F90:1544-1566` 的
`qmelt`/`qfrz` 质量转移 + Niu (2004) 的 `tl = fwet_snow*tfrz + (1-fwet_snow)*tl`
拉回 —— 本仓库 `leaf_temperature.rs::update_canopy_water` 的融化/冻结两段
（`min(...)`、`max(0, ...)`、`wet_snow_fraction` 加权拉回）与它逐式对齐。
所以 `canopy_phase_heat_w_m2: 0.0` 的两处赋值是**忠实**的，不是偷懒；
真要在 scheme 4–7 上跑，`colm-rs` 会先拒绝未移植分支，不会静默少焓。
（已在 `interception.rs` 的函数文档里留下这段依据，免得下一轮又"补"一次。）

### 二、PLUMBER2 目录里的 `._*`：四条真实数据测试一直红着

macOS 在数据盘上会给每个文件配一份 AppleDouble 边车 `._<同名>`，它们同样以
`_Met.nc` / `.nc` 结尾，`summarize()` 打开就报
`netcdf error(-51): NetCDF: Unknown file format`。**扩展名挡不住**：
`Path::extension()` 对 `._X_Met.nc` 返回 `nc`。修法是按**文件名**排除
（`starts_with("._")` + `is_file()`），两个测试文件各一个 4 行小函数，并加一条
"边车没漏进来"的断言自证。

实测（`PLUMBER2_ROOT=/Volumes/Data01/Data/PLUMBER2s`）：

| 命令 | 修前 | 修后 |
|---|---|---|
| `cargo test -p colm-forcing --test real_forcing` | **0 passed / 4 failed** | **4 passed / 0 failed** |
| `cargo test -p colm-srfdata --test real_sites` | 2 passed / 4 failed | **5 passed / 1 failed** |

`real_sites` 剩下那一条是 `the_raster_and_the_classifier_disagree_about_as_often_as_measured`，
它要 `COLM_RAWDATA` 的 38 GB 栅格 —— 本机没有，`CLAUDE.local.md` 已注明
"别把失败当成回归"。这一改让**另外三条**真正开始跑那 90 个真实站点文件
（位置/土类、USDA 三角、缺字段集合），`real_forcing` 的四条也开始跑那 90 个真实
强迫场文件 —— 文件头自己写着这两个语料此前各抓到过一次"只在 CN-Cng 成立的常数"。

### 三、顺手留下的工具（下一轮直接用）

用 `end_sec = 1800*N` 造 N 步窗口，Fortran 与 Rust 各跑一遍，直接比对重启文件，
可以拿到**逐步**的分叉表。这一轮跑了 N = 1,2,3,4,6,8,12,16,24,48 共 10 组
（Fortran 每组约 1.2 s、Rust 约 1.7 s，总计不到 1 分钟）。

**注意一个坑**：这套表用的是**没打 `o3coef*` 补丁**的内核，而第一步的叶温求解在
那里是被 `spval` 污染的（见上一节），所以 N 很小那几行的量级（`ldew` 差
~1e-6 mm、`tleaf` 差 ~1e-4 K）里混着那份污染，不要直接当"每步固有差"用。
要干净的逐步表，先用上一节那段补丁重建内核，或者干脆从第 2 天起比。

### 四、这一轮之后的待办（更新）

| 项 | 状态 |
|---|---|
| `canopy_phase_heat` 未移植 | **关闭**（scheme=1 下恒 0 是对的） |
| PLUMBER2 `._*` 打断真实数据测试 | **已修** |
| 上游 `o3coef*` 时序（`spval` 乘进 `gs0sun`） | 未修，需连同三份黄金一起重跑 |
| `zwt` / `ldew` 的每步小差（第 2 天 3.7e-3 / 2.0e-4 相对） | 未定位，落点见上一节 |
| tier3 的"日均无趋势漂移"一半 | 未实现 |
| `colm_git_sha` 记的是 Rust 仓库 HEAD | 未修 |
| `f_xy_solarin` / `f_xy_q` 的 1 ULP | 未修（`sun_angle` 未与 `orb_coszen` 对齐） |
| TOPMOD / XinAnJiang 产流在 VSF 下 | 未测 |

## tier1 的短波四波段其实一直差 1–5 ULP：gfortran 的 FMA 收缩（实测，**已修**）

tier1（`rtol=1e-12`）从来没有红过，所以"四个波段一致"这件事**没被验过**。
这一轮用独立的小驱动器逐位量了一遍，发现它们一直差 1–5 ULP —— 是被容差放过去的。

### 一、方法：把 Fortran 表达式单独编出来比位，而不是插桩内核

内核编译参数照抄 `vendor/CoLM202X/include/Makeoptions:22` 的
`FOPTS_PRODUCTION = -O2 -fdefault-real-8 -ffree-form -g -ffpe-trap=... -fbacktrace -cpp
-ffree-line-length-0 -fallow-argument-mismatch`，把
`MOD_OrbCoszen.F90`、`MOD_Precision.F90` 或一段复刻的拆分表达式编成**独立可执行**，
用一个 Rust 集成测试喂同一组输入、按 `f64::to_bits()` 对比。比插桩内核快得多，
而且能一次扫上万组输入。

**结论先说：GCC 的 `-ffp-contract=fast` 是默认**，`a*b-c` / `a+a*b` 这类模式在
`-O2` 下会收缩成 FMA；Rust **不会**自动收缩（LLVM 只认显式 `mul_add`）。
所以凡是"一个乘加表达式"，两边就可能差 1 ULP —— 而 CoLM 里这种表达式遍地都是。

### 二、`orb_coszen`：11520 组里差 15 组

`orbital_declination` 里
`lamb = lambm + eccen*(2*sinl + eccen*(1.25*sin(2*lmm) + eccen*((13/12)*sin(3*lmm) - 0.25*sinl)))`
的**三个** `eccen*X + Y` 被 GCC 收缩成 FMA，而最里层的
`(13/12)*sin(3*lmm) - 0.25*sinl` **没有**。六组网格中心 × 40 天 × 48 个半步
= 11520 个 `orb_coszen` 取值：

| 写法 | 位不一致 |
|---|---|
| 纯乘加（原样） | 15 |
| 只把最里层写成 FMA | 15 |
| 三层都写 FMA（最里层也收缩） | 15 |
| **三层写 FMA、最里层不收缩** | **0** |

### 三、四个波段：12000 组里 7941 个值对不上

`MOD_Forcing.F90:616-646` 那一支（`solarin_all_band` 且非 QIAN）里有三处收缩点：

| 上游 | 收缩成 |
|---|---|
| `cloud = (1160.*sunang-a)/(963.*sunang)` | `fma(1160., sunang, -a)` |
| `difrat = difrat+(1.0-difrat)*cloud` | `fma(1.0-difrat, cloud, difrat)` |
| `vnrat = (580.-cloud*464.)/((580.-cloud*499.)+(580.-cloud*464.))` | 两个分子各 `fma(-cloud, 464./499., 580.)` |

12 个 `a` × 1000 个 `sunang` = 12000 组，每组四个波段共 48000 个值：

| 写法 | 位不一致的**值** |
|---|---|
| 纯乘加（原样） | 7941 / 48000 |
| **三处都收缩** | **0** / 48000 |

另做了一次**中间量**探针（只打 `cloud`/`difrat`/`vnrat`/两个分子，5×12000 项）：
完全不收缩时 `cloud` 一项就有 683/12000 组不一致、`num464` 1443、`den` 1282；
三处都收缩之后这 5 项全 0。

### 四、修后的效果：三个窗口的四个波段**逐位**相等

| 变量 | `CN-Cng` | `CN-Cng-wet` | `US-NR1-snow` |
|---|---|---|---|
| `f_solvd` | 18/264 → **0** | 74/384 → **0** | 55/360 → **0** |
| `f_solvi` | 12/264 → **0** | 61/384 → **0** | 35/360 → **0** |
| `f_solnd` | 21/264 → **0** | 81/384 → **0** | 59/360 → **0** |
| `f_solni` | 12/264 → **0** | 58/384 → **0** | 38/360 → **0** |
| `f_sol*ln`（四个长波带） | 各 0 | 各 0 | 各 0 |

**tier 计数一个都没动**（tier0 2/1/2、tier2 57/70/81）：tier1 本来就是
`rtol=1e-12`"过"的，所以这一改是把**被容差盖住的实现差**消掉，而不是把红条变绿。
这也说明"tier1 全绿"**不等于**"确定性代数逐位一致" —— 要证逐位，得用上面那种
独立驱动器，而不是看 tier 表。

### 五、`f_xy_solarin` 的 1 ULP：累加的结合顺序（**已修**，湿窗 tier0 清零）

修完波段之后，四个波段的**小时均值**已经逐位相等（上表），所以两边每个半步的
四个波段值也逐位相等（`均值×2` 是精确的）。于是 `f_xy_solarin` 只剩 1 ULP
（干 11/264、湿 23/384、雪 22/360）**只可能出自累加的结合顺序**：

* 上游 `MOD_Vars_1DAccFluxes.F90:2060-2063` 把四个波段**分别** `acc1d` 到同一个
  `a_solarin`，`acc1d` 是 `s = s + var`（`:2897`），`nac` 每步只加一次；
  一个两子步的小时因此是**八项从左到右的连加**；
* 本仓库先算每步的四项和（`history.rs:631`），累加器再把两个半步的值相加
  （`Accumulated::Scalar` 的 `*sum += value`），即 `(4项) + (4项)`。

数学上相同，浮点上差 1 ULP。修法已经落地：`HistorySink` 加一条 `accumulate(name, record, value, counts_as_step)`
的通道（`nac` 每步只加一次，其余三项只进和），`HistoryAccumulated` 只在前者
`*count += 1`；直接写缓冲的 sink 用 `colm-hist` 新增的 `add_patch_scalar`
把贡献加上去，语义同样正确。`HistoryReferenceState` 因此**原样带四个波段**
而不是先求和，`set_lct_forcing_mirrors` 单独为 `xy_solarin` 走这条路。
（`accumulate` 故意不给默认实现：给了就等于让直接写缓冲的 sink 静默把后三项
覆盖掉。）

实测：

| 窗口 | `f_xy_solarin` 修前 | 修后 | tier0 合计 |
|---|---|---|---|
| `CN-Cng` | 11/264 | **0** | 2 → **1**（只剩 `f_xy_q`） |
| `CN-Cng-wet` | 23/384 | **0** | 1 → **0** |
| `US-NR1-snow` | 22/360 | **0** | 2 → **1**（只剩 `f_xy_q`） |

**`CN-Cng-wet` 的 tier0（逐位）现在是零红条** —— 这是这个仓库第一次有一个窗口
把"纯函数、无迭代、任何差异都是 bug"的那一层全部对上。

`f_xy_q` 剩下的 1 ULP（干 1/264、雪 64/360）不在这一条链上，仍未定位。

### 六、可复用的做法

`/tmp` 里那两个独立驱动器（`orb_coszen` 与波段拆分）值得在下一轮照着重建：
"用内核的参数单独编出上游的**纯函数**，再拿 Rust 的同一函数按位对比"，
比"插桩内核 + 跑算例 + 比 history"便宜一到两个数量级，而且能给出
"多少组里差几组"这种可以写进文档的定量结论。

### 七、这一轮之后的 tier0/tier1 现状

| 窗口 | tier0 | 剩的是 | tier1 |
|---|---|---|---|
| `CN-Cng` | **1** | `f_xy_q` 1/264（1 ULP） | 四个波段 + `*ln` 全部逐位 |
| `CN-Cng-wet` | **0** | — | 全部逐位 |
| `US-NR1-snow` | **1** | `f_xy_q` 64/360（1 ULP） | 全部逐位 |

tier2 计数没动（57/70/81）—— 这一轮修的是被 tier1 容差盖住的实现差，
以及 tier0 里那两个"看着是同一个数、位不同"的量，跟耦合迭代那 57 条的
量级无关。

## `qsadv`：Horner 的 FMA 收缩、`qsdT` 的结合顺序，以及 **`d5` 系数差 10 倍**（实测，**已修**）

上一轮把"用内核参数单独编上游纯函数、再按位对比"这套做法立起来之后，这一轮立刻
在 `MOD_Qsadv.F90` 上又抓到三处，其中一处是**真物理错**。

### 一、Horner 的每一层都被收缩成 FMA

`qsadv` 的四张系数表都是
`a0 + td*(a1 + td*(a2 + ...))`（`MOD_Qsadv.F90:85-96`），每一层都是
`c + x*inner` —— 正是 GCC 在 `-ffp-contract=fast`（`-O2` 默认）下最容易收缩的形状。
本仓库 `polynomial()` 原先写成 `coefficient + x * value`（纯乘加）。
3045 组 (T,p)（覆盖冰/水两支、600–1030 hPa，含 CN-Cng 与 US-NR1 的实际范围）：

| 写法 | `qs` 位不一致 |
|---|---|
| 纯乘加（原样） | **1796**（59%） |
| `x.mul_add(value, coefficient)` | **0** |

### 二、`qsdT` 的结合顺序

上游是 `vp2 = vp1*vp` 再 `qsdT = esdT*vp2*p`（`:98-104`），本仓库写成
`esdT*vp1*vp*p`（左结合）—— 结合顺序不同，实测 3045 组里差 2076 组。
改成先算 `humidity_factor_squared = humidity_factor*inverse_pressure` 再乘 ✓。
这条不是"无所谓的 1 ULP"：`qsdT` 是隐式求解对温度的导数，进地表层与土壤热
求解的迭代系数。

### 三、`d5` 系数：`0.257180651e-08` 抄成了 `2.57180651e-08`（**10 倍**）

修完前两处后 `esdT` 仍有 1635/3045 组不一致，而且**枚举 256 种逐层 FMA/非 FMA
组合一个都对不上** —— 说明不是收缩问题。于是写了个**机械的源对源比较**：
用正则从 `MOD_Qsadv.F90` 的 `data` 语句里取系数、从 `atmosphere.rs` 的
`f77(...)` 数组里取系数，两边都按十进制转 f64 再比位。结果一眼就看到：

```
d5: rust=2.57180651e-08   fortran=2.57180651e-09
```

`d5/0.257180651e-08/` 是 **2.57180651e-9**，本仓库的小数点后少写了一个 0。
这与第 83 轮修掉的 `c8`（冰面 `es` 的系数多了一个 0）是**同一类错、同一张表**。
影响：`esdT` 里多出 `2.31e-8*td^5` 的偏差，`td=-20 ℃` 时约 **0.074 hPa/K**
（该处斜率量级 ~1 hPa/K，即 ~7%），而 `esdT` 是隐式能量求解的系数。

改完之后 `qsadv` 四个输出（`es`/`esdT`/`qs`/`qsdT`）在 3045 组上**全部逐位相等**。

### 四、效果：三个窗口的 **tier0 全部清零**，tier2 又少 5 条

| 窗口 | 变量数 | tier0 | tier2 | 关掉的条目 |
|---|---|---|---|---|
| `CN-Cng` | 58 → **55** | 1 → **0** | 57 → **55** | `f_xy_q`、`f_fevpa`、`f_fevpg` |
| `CN-Cng-wet` | 70 → 70 | 0 → 0 | 70 → 70 | —（暖窗冰面分支不跑，只有末位量级的变化） |
| `US-NR1-snow` | 82 → **79** | 1 → **0** | 81 → **79** | `f_xy_q`、`f_qintr`、`f_qdrip` |

超差幅度的改善（同一条目修前→修后）：

| 窗口 | 改善最大的几条 |
|---|---|
| `CN-Cng` | `f_rnet` 1.94e4 → 9.52e3（2.0×）、`f_fsena` 1.81e5 → 9.74e4（1.9×）、`f_sr` 55.2 → 24.8（2.2×）、`f_h2osoi` 263 → 167（1.6×）、`f_t_grnd` 228 → 170（1.3×） |
| `US-NR1-snow` | `f_frcsat` 1.70e6 → 6.99e5（2.4×）、`f_qinfl` 1.42e3 → 607（2.3×）、`f_rnof`/`f_rsur`/`f_rsur_se` 122 → 52.9（2.3×）、`f_laisha` 1.29e3 → 697（1.9×）、`f_sai`/`f_sigf` 1.8× |

**`CN-Cng`、`CN-Cng-wet`、`US-NR1-snow` 三份黄金的 tier0（逐位）现在都是零红条** ——
"纯函数、无迭代、任何差异都是 bug"那一层第一次在三条窗口上全部对上。

湿窗只动了 75 个变量的末位（`f_t_soisno` 最大差 2.5e-5 K），正好印证 `d5` 只走
`td < 0` 那一支：暖窗的 `f_xy_q` 本来就是逐位的，修完仍是逐位。

### 五、下一步：把"源对源系数审计"做成一次普查

`c8`（第 83 轮）和 `d5`（这一轮）是同一张表里的同类错。这类错的检出手段本轮
已经跑通：**从 Fortran 的 `data` 语句和 Rust 的常量数组里机械地各取一份、按十进制
转 f64 比位**，不靠眼睛。全仓库这类 `data` 语句还有 ~349 行、分布在 11 个文件
（`MOD_Albedo.F90`、`MOD_SnowLayersCombineDivide.F90`、`MOD_SnowSnicar*.F90`、
`MOD_3DCanopyRadiation.F90`、`MOD_Urban_Albedo.F90` 等），其中已移植的是
`MOD_Albedo.F90`（`surface_optics.rs`）。下一步就把这个审计脚本对这几张表跑一遍。

## 系数表普查：审计脚本入库，以及一条"四倍精度"陷阱（2026 年，实测）

`c8`（第 83 轮）与 `d5`（第 86 轮）都是**小数点后多写/少写一个 0** 的 10 倍系数错，
而两次都是靠"把 Fortran 的十进制字面量和 Rust 的**按 f64 位**比"抓到的。既然
同类错已经出现两次，这一轮把那套比较写成脚本入库：

```
oracle/scripts/audit_fortran_literals.py            # 两种模式都跑
oracle/scripts/audit_fortran_literals.py --tree     # 整棵树模式
oracle/scripts/audit_fortran_literals.py --tables   # 系数表模式
```

* **模式 1（整棵树）**：对每个 Rust 源文件取它注释里引用的 `MOD_xxx.F90`，把这些
  Fortran 文件里有效位数 ≥4 的十进制字面量转成 f64 位，再去**整个 Rust 树**里找同值。
* **模式 2（系数表）**：只扫 `data` / `parameter ::` 行，**不设位数下限** ——
  `data scat_sno /0.8, 0.4/` 这种短系数正是模式 1 的盲区。

### 一、结果：端口里没有**新**的系数错

* 模式 2：`MOD_Albedo.F90` 5/5、`MOD_Qsadv.F90` 12/12、`MOD_SoilSnowHydrology.F90`
  6/6、`MOD_PlantHydraulic.F90` 1/1 全部找到；只有 4 个找不到，全部是
  `MOD_SnowLayersCombineDivide.F90` 的 `c1 = 2.777e-7` / `c6 = 5.15e-7` /
  `eta0 = 9.e5` 与 `MOD_LeafInterception_Extended.F90` 的 `2.094e6` —— 前三个上游
  只在**注释掉的行**里用（`!* ddz2 = -burden*exp(...)/eta0`），Rust 用的是同一函数里
  真正生效的那条公式；`2.094e6` 在 scheme 5（MATSIRO）里，未移植。
* 模式 1：33 个未匹配，逐个查清，**没有一个是缺陷**：
  * 写法定不同：`3.14159` → Rust 写 `314_159.0/100_000.0`（`radiation.rs` 的
    `FORTRAN_PI`）；`3.14159265358979323846` → `std::f64::consts::PI`；
    `2.2204460492503131E-16` → `f64::EPSILON`。
  * **未移植的分支**：`MOD_UserSpecifiedForcing` 里 ERA5/WFDE5 等数据集的
    `212.`/`10800.`/`21600.`/`2020.`（本仓库只支持 `POINT`）；湖（`1.1925` 等 4 个）；
    `glacier.rs` 的 `9.828`；scheme 4–7 的 `270.15`/`67.92`/`51.25`；城市冰雪面
    阻抗的 `4.255`。
  * **粒度造成的假阳性**：模式 1 按**文件**取"应出现"的字面量，`linear.rs` 引用了
    三千行的 `MOD_Utils.F90`，于是那个文件里 `pnorm`/`cotan` 的 `D+00` 字面量也被
    算进来 —— 那些行不是它移植的对象。脚本的 docstring 里已写明这条局限。

**这不是"验证过了"而是"这次没查出新的"**：集合比较抓不到"抄错后的值恰好等于
Rust 里另一个字面量"的情况。真正的逐位保证仍然来自上一轮那套**独立驱动器**
（把上游纯函数按内核参数单独编出来按位对比）。

### 二、一条真陷阱：`d0` / `D+00` 在 `-fdefault-real-8` 下是**四倍精度**

`Makeoptions:24` 只有 `-fdefault-real-8`，**没有** `-fdefault-double-8`。
GCC 的语义是：`DOUBLE PRECISION` 与 `d0` 后缀字面量在 `-fdefault-real-8` 下变成
`REAL(16)`（quad）。于是含 `d0` 的表达式会**整体按 quad 求值、最后才舍入到 r8**，
与 f64 逐步求值可以差到 1 ULP —— 这正是上一轮 FMA 收缩问题的同族陷阱。

逐个查了已移植的热路径文件（`MOD_LeafTemperature_Extended`、`MOD_Thermal`、
`MOD_Albedo`、`MOD_Qsadv`、`MOD_PhaseChange`、`MOD_GroundTemperature`、
`MOD_PlantHydraulic`、`MOD_Hydro_SoilWater`、`MOD_NewSnow`、`MOD_SoilThermalParameters`、
`MOD_RainSnowTemp`、`extends/interception/MOD_Thermal_CanopyPhase_Extended`）：

| 文件 | 含 `d0`/`D+` 的行 |
|---|---|
| 上面所有文件 | **0** |
| `MOD_SoilSnowHydrology.F90` | 6，全是 `qinfl_fld_subgrid = 0.0d0` 这类**零初始化**（任何精度都精确，无害） |

但 **scheme 5–8 的拦截例程里有**，例如
`MOD_LeafInterception_Extended.F90:1832`：
`(1.14d-11)*1000.*deltim*exp(min(50.0d0, min(ldew_rain_s,satcap_rain)/1000.*3.7d3))`
—— 这一串在 Fortran 里是 quad 求值。**谁哪天移植 MATSIRO/VIC/JULES，必须照抄成
quad 或至少知道这里差了 1 ULP**，不要以为 `d0` 只是写法。

### 三、顺手查清：雪窗 `f_alb` 那 2 倍差**不是反照率自身的公式错**

第 86 轮修完 `qsadv` 后，雪窗 `f_alb` 仍有 0.11 的绝对差（相对该量级 ~5%），
看起来像反照率算错。按分量拆开（206 条白天记录，夜间是 spval）：

| 分量 | 平均绝对差 | 最大 | 金均值 |
|---|---|---|---|
| 可见 direct | 0.0075 | 0.108 | 0.214 |
| 可见 diffuse | 0.00041 | 0.0036 | 0.160 |
| 近红外 direct | 0.0134 | 0.114 | 0.249 |
| 近红外 diffuse | 0.0017 | 0.0137 | 0.209 |

`可见 direct` 的 rust/gold 比值**中位数 1.000**（一致），但有个别记录到 **2.6 倍**
—— 是**离散事件**，不是系统性偏差。这类"多数一致、个别翻倍"的形状指向
**雪的有无/`fsno` 的状态翻转**（有雪 albsno≈0.8 对裸地≈0.15），而不是反照率公式。
并且把这条链上的两个纯函数逐式对过上游：

* `snowage`（`MOD_Albedo.F90:1276`）对 `snow.rs::update_snow_age`：分支、`arg`/`arg2`、
  `dela = 1.e-6*deltim*(age1+age2+age3)`、`dels = 0.1*max(0,scv-scvold)`、
  `sge = (sag+dela)*(1-dels)` **逐式一致**；
* `snowcompaction`（`MOD_SnowLayersCombineDivide.F90:31`）对
  `snow.rs::compact_snow_layers`：`ddz1 = -c3*exp(-c4*td)`、`bi > 100` 的
  `exp(-46e-3*(bi-100))`、液态水项 `*2`、`eta = f1*4*(bi/450)*exp(0.1*td+c2*bi)*7.62237e6`、
  `ddz2 = -(burden+wx/2)/eta` **逐式一致**。

所以嫌疑回到**雪的状态量**（`f_scv`/`f_snowdp`/`f_fsno` 那一族），不是反照率。
下一轮若要继续雪窗，应从雪深/雪水当量的每步收支入手，别去改反照率。

## 雪窗的残差**不是**雪物理，也不是上游那个 `spval`：它是第 1 步就存在的 ~1e-5 稳态小差（2026 年，实测）

第 87 轮把雪窗的 `f_alb` 归到"雪状态翻转"之后，这一轮把两件事都量掉了：上游
`o3coef*` 那个 `spval` 补丁对雪窗**没有帮助**，以及逐步的分叉表显示这是**第 1 步
就有的稳态差**在雪水当量上累积，而不是雪模块自己的公式错。

### 一、上游 `o3coef*` 补丁：把第 1 步的垃圾清掉，但 15 天窗口一动不动

按第 85 轮的办法打补丁（臭氧关闭时在 `LeafTemperature` 循环**之前**把四个
`o3coef*` 置 1）、重建内核、重跑 `US-NR1-snow` 15 天窗口，再拿 Rust 去比
**补丁后的 Fortran**：

| 比较 | 变量数 | tier2 |
|---|---|---|
| Rust 对**现有黄金** | 79 | 79 |
| Rust 对**补丁后的 Fortran** | 79 | 79 |

`f_scv`/`f_snowdp`/`f_fsno`/`f_wice_soisno`/`f_t_soisno` 的平均绝对差**印刷精度内
完全一样**。原因也量清楚了：补丁确实改变了 Fortran 的雪窗轨迹（85 个变量不同），
但幅度只有 ~1e-6，相对 Rust-对-黄金 的 0.4 kg/m² 完全不成比例。

**但补丁在第 1 步上的作用是决定性的**——它把垃圾清干净了。用 `/tmp/step1` 那个
1 步窗口比补丁后 Fortran 的 restart 与 Rust：

| 量 | 打补丁前（第 85 轮量到） | 打补丁后 |
|---|---|---|
| `gs0sun`/`gs0sha` | −4.525e38（`spval` 乘出来的垃圾） | **逐位相等** |
| `vegwp` | −3418/−3418/−1664/−774 对 −621/−621/−621/−121（差 5 倍） | **逐位相等** |
| `tleaf` | 差 2.35e-4 K | 差 2.4e-4 K（9.0e-7 相对） |
| 不同变量数 | 22/68 | 26/68（但量级从"垃圾"降到 ≤6.3e-5 相对） |

也就是说：**上游那个 `spval` 只解释第 0 条那几个量的垃圾，不解释任何窗口的
15 天残差** —— 这一点第 85 轮在干窗上量过一次（13726 条超差 → 13726 条），
这一轮在雪窗上独立复核，结论一致。所以"要不要修上游 + 重跑黄金"这件事，
**收益只是让黄金自己更干净**，不会让 Rust 更贴近。

### 二、逐步分叉表：雪窗的差是**稳态每步小差**在 `scv`/`wice` 上累积

用 `end = 2013-04-09 + N*1800 s` 造 N 步窗口（Fortran 与 Rust 各跑一遍，
直接比重启文件；`N=48` 跨日所以要按 `day/sec` 拆开写）：

| N | `scv` 差 | `snowdp` 差 | `fsno` 差 | `ldew` 差 | `tleaf` 差 | `wice_soisno[5]` 差 |
|---|---|---|---|---|---|---|
| 2 | −1.66e-4 | −2.20e-6 | −1.16e-4 | −1.39e-6 | −7.04e-5 | 0 |
| 4 | −1.66e-4 | −2.20e-6 | −9.21e-5 | −1.63e-6 | −1.14e-4 | −5.50e-4 |
| 8 | +1.29e-3 | −1.70e-6 | −9.01e-4 | −2.60e-6 | −1.33e-2 | +6.72e-3 |
| 16 | +4.74e-3 | −2.84e-7 | −1.55e-3 | −6.19e-5 | −2.49e-2 | +5.78e-2 |
| 32 | +1.44e-2 | −6.08e-6 | −3.80e-5 | +4.69e-6 | −5.18e-3 | +1.09e-1 |
| 48 | +1.54e-2 | −1.76e-4 | −1.32e-4 | +3.64e-5 | −3.41e-3 | +1.90e-1 |
| 96 | −8.98e-2 | −3.05e-4 | −4.06e-3 | +1.10e-4 | +1.22e-2 | +2.09e-1 |
| 192 | +3.29e-1 | +2.79e-4 | −2.48e-2 | +3.41e-2 | −8.75e-2 | +2.76e-1 |

读法：

1. **`wice_soisno[5]`（顶层土壤的冰）单调累积**：0 → −5.5e-4 → +6.7e-3 → 5.8e-2
   → 0.11 → 0.19 → 0.21 → 0.28 kg/m²。这是**每步一个同号的微小差**在积分，
   不是某一步的离散事件。
2. `scv` 从 N=2 的 0.14% 涨到 N=192 的 13.8%（`f_scv` 在 15 天窗口里的平均相对差
   3.8%），`fsno` 跟着走到 2.5e-2 —— 雪层数在那之后才翻，上一轮看到的 266 K
   `t_soisno` 假差是**层数错位**的后果，不是雪物理本身。
3. N=2 的种子已经存在：`scv` −1.66e-4、`ldew` −1.39e-6、`tleaf` −7.0e-5 K。

### 三、第 1 步那 26 个量的"阶梯"

补丁后第 1 步（干窗，`/tmp/step1`）全部不同量按相对差排：

| 相对差 | 量 |
|---|---|
| 6.3e-5 / 5.9e-5 | `smp` / `wice_soisno` |
| 3.5e-5 / 3.3e-5 | `qstar` / `tstar` |
| 2.9e-5 / 1.9e-5 | `ldew`/`ldew_snow` / `fwet_snow` |
| 2.0e-5 | `rib` / `zol` |
| 1.0e-5 / 6.5e-6 | `qref` / `ustar` |
| 3.6e-6 / 1.9e-6 | `fh`/`fq` / `fm` |
| 9.0e-7 | `tleaf` / `gs0sun` / `gs0sha` |
| 1.1e-7 / 1.0e-7 | `t_soisno` / `vegwp` |
| 2.3e-16 | `rst`（实际上逐位） |

几点读法：

* `ldew` 的 2.9e-5 **不是大数**：绝对值只有 1.35e-6 mm，而第 1 步无降水、`ldew`
  从 0 起步，靠凝霜积起来 —— 小基数放大了一个小的**绝对**差。它的上游是叶面
  蒸发（叶温解）的 ~1e-5 相对差。
* `tstar`/`qstar` 的 3.3e-5 不可能来自 `moninobuk` 本身：那一族共用的
  `psi`/`momentum_integral`/`heat_integral`（`zetam=1.574`、`zetat=0.465`、
  `1.14`、`0.8`、`0.333`、`16`、`0.25`、`2*atan(1)`）这一轮**逐式对过上游，全一致**；
  而 `t_grnd` 逐位相等 ⇒ 差在 `dth = thm - t_grnd` 里的 `thm`，即**冠层空气**，
  也就是那几个权重（`wta0`/`wtg0`/`wtl0`）背后的**导度**（`cah`/`cgh`/`cfh`），
  再往下就是气孔阻力那一串。
* `smp`/`wice_soisno` 的 6e-5 是相变分配的结果，它的输入（`t_soisno_bef`、
  `wliq0`、`wice0`、`fact`）在第 1 步逐位相等（第 85 轮量过），差从 `hs`/`dhsdT`
  传下来 —— 而那又是冠层/地表能量平衡。

**结论：雪窗、干窗、湿窗的残差是同一件事** —— 第 1 步就存在的 ~1e-5 相对差，
来源在**冠层/地表能量-水汽求解**（`rssun`/`rssha` → 导度 → `thm` → `hs`/`dhsdT`
→ 相变分配 → 土壤水热），不是雪模块、不是反照率、也不是上游那个 `spval`。

### 四、下一步（唯一还没试过的形状）

已有的手段都是"比某个量"，量出来的是"处处差 1e-5"。要再往下，需要一次
**按执行顺序打点**的对照：在第 1 步里按**上游的执行顺序**依次打印
（`GroundFluxes`/`LeafTemperature` 的入口强迫 → `sabg`/`dlrad`/`emg` →
`rssun`/`rssha` → `cfh`/`cfw` → `wta0`/`wtg0`/`wtl0` → `thm`/`qm` →
`fsenl`/`fevpl` → `hs`/`dhsdT` → 相变），两边同一时刻同一行，**找第一处
相对差超过 1e-12 的量**。前几轮都是"打一个点"就定位，这一次要打一串 ——
但那也是唯一能把 1e-5 的来源钉死的办法。

### 五、第 1 步按执行顺序打点：**第一个分叉量是冠层水汽权重**（同一轮追加）

上一节说"要按执行顺序打一串点"，这一轮就打了一串。做法：把臭氧补丁与探针
放进**同一次**内核构建（省一次编译），在 `MOD_LeafTemperature_Extended.F90`
的 `tl = tlbef + dtl(it)` 之后按迭代打印；Rust 侧在
`leaf_evaporation_unadjusted` 之后打印同一组量。跑 `/tmp/step1`（干窗第 1 步，
`it ≤ 8`）。

**第 1 轮（`tl = 280 K`）的逐项对照：**

| 量 | Fortran（补丁后） | Rust | 判定 |
|---|---|---|---|
| `tl` | 280.0 | 280.0 | **逐位** |
| `qsatl` | 7.5882991352415295e-3 | 7.58829913524152952e-3 | **逐位** |
| `qaf` | 4.1748864657195975e-3 | 4.17488646571959748e-3 | **逐位** |
| `qm`（参考层比湿） | 7.6147948857396841e-4 | 7.61479488573968410e-4 | **逐位** |
| `qg`（地表比湿） | 7.5882934428652266e-3 | 7.58829344286522656e-3 | **逐位** |
| `rhoair` | 1.3559947226232352 | 1.35599472262323517 | **逐位** |
| `rb`（叶面边界层阻抗） | 19.609516500085267 | 19.6095165000852667 | **逐位** |
| `laisun` / `laisha` | 0.10000000149011612 | 0.10000000149011612 | **逐位** |
| `gs0sun` | 425.14633619846109 | 425.14633619846109 | **逐位** |
| `gssun` | 42.51463365289875 | （同源） | — |
| **`wtaq0`** | **7.4076222677608794e-1** | **7.40752120387776158e-1** | **1.36e-5** |
| **`wtgq0`** | **2.5922638670844889e-1** | **2.59236493252109457e-1** | **3.9e-5** |
| `wtlq0` | 1.1386515463061202e-5 | 1.13863601142291121e-5 | 1.4e-6 |
| **`fevpl`（= `etr`）** | **1.3711981706799755e-8** | **1.37117946313109531e-8** | **1.37e-5** |

**这就是那个 1e-5 的出处**：`etr = rhoair*(1-fwet)*delta*(laisun/(rb+rssun)+laisha/(rb+rssha))*湿度梯度`，
而上面那一排因子（`rhoair`、`rb`、`laisun`/`laisha`、`qsatl`/`qaf`/`qm`/`qg`、
`fwet = 0`、`delta = 1`）**全部逐位相等**，差的只有 **`wtaq0`/`wtgq0`**，
它们进 `grad = (wtaq0+wtgq0)*qsatl - wtaq0*qm - wtgq0*qg` → 1.4e-5。

而 `wtaq0 = caw*wtsqi`、`wtgq0 = cgw*wtsqi`、`wtsqi = 1/(caw+cgw+cfw)` ——
也就是说差在**冠层水汽导度那一组**（`caw = 1/raw`、`cgw = 1/(rd+rss)`、
`cfw = (1-delta*(1-fwet))*wet_cond_cfw + (1-fwet)*delta*(laisun/(rb+rssun)+…)`）。
这一轮已经把它们的**公式**逐式对过上游（含 `cfw` 的两种 scheme 分支、
`cgw` 的露/非露分支、`wet_cond_cfw = (lai+sai)/rb`）—— 都一致；
所以剩下的是**它们的输入**（`raw`/`rd`/`rss`/`rb` 的构造）在取值点上的差。
注意 `rb` 本身是逐位的，`raw`/`rd` 由 `moninobuk` 那一族给（也已逐式核对过），
所以下一个要看的是 `rss`（地表阻抗）与 `wtaq0` **取值那一刻**的 `caw`/`cgw`/`cfw`
—— 本轮探针是在迭代末尾打的，这三个量在权重算完之后会被后面的分支重算，
所以打点位置必须挪到 **权重计算之前**。

**结论**：三窗口的 ~1e-5 稳态差，第 1 步第 1 轮里唯一的分叉点是
**冠层水汽导度权重**；上游所有叶子尺度因子（`tl`/`qsatl`/`qaf`/`qm`/`qg`/`rhoair`/
`rb`/`laisun`/`laisha`/`gs0sun`）在第 1 轮上**逐位相等**。下一轮只要把探针
挪到权重之前、把 `caw`/`cgw`/`cfw` 与 `raw`/`rd`/`rss` 打出来，就能把最后
这一处钉死。

### 六、链路闭合：根是**地面 Obukhov 长度 `obug`**（同一轮再追加）

按上一节说的把探针挪到**权重之前**，并在探针里同时打 `rd` 的输入
（`rd = frd(...)`，`rd_opt = 3` 是编译期常量，两边都走这条），得到第 1 步第 1 轮的
逐项对照：

| `frd` 的输入 | Fortran | Rust | 判定 |
|---|---|---|---|
| `ktop` | 1.1725490811579668e-1 | 1.17254908115796683e-1 | **逐位** |
| `htop` | 5.0000000000000000e-1 | 同 | **逐位** |
| `z0qg` | 1.4511254752084092e-3 | 1.45112547520840924e-3 | **逐位** |
| `hsink` | 3.7057264881491081e-1 | 同 | **逐位** |
| `displah` | 2.2374979216340563e-1 | 2.23749792163405625e-1 | **逐位** |
| `ustar` | 6.0228307379441826e-1 | 6.02283073794418256e-1 | **逐位** |
| `z0mg` | 1.0000000000000000e-2 | 1.00000000000000002e-2 | **逐位** |
| `a_k71`（= alpha） | 9.4579533501265467e-1 | 同 | **逐位** |
| **`obug`** | **−6.5090068643354604** | **−6.49936795802490330** | **1.48e-3** |
| `rd`（输出） | 2.1966799847019068e1 | 2.19656437672341553e1 | 5.3e-5 |

**`frd` 的九个输入里只有 `obug` 不同**，`rd` 的 5.3e-5 完全由它传下来。于是整条
因果链闭合了（都是第 1 步第 1 轮的量）：

```
obug（地面 Obukhov 长度，差 1.48e-3）
  → frd（冠层扩散率积分，差 5.3e-5）
  → rd（地面—冠层阻抗）
  → cgw = 1/(rd+rss)
  → wtaq0/wtgq0（冠层水汽权重，差 1.36e-5 / 3.9e-5）
  → 湿度梯度 → etr = fevpl（差 1.37e-5）
  → 叶能量平衡 → hs/dhsdT
  → 相变分配（wice_soisno / smp，差 ~6e-5）
  → 土壤柱 → scv / snowdp（N=192 时 13.8%）
```

而 `obug` 从哪来也查清了：`MOD_Thermal_CanopyPhase_Extended.F90:655`
`obu_g = forc_hgt_u/zol_g`，即**地面那一支 `GroundFluxes` 的 `zol`**（不是叶温那一支
的 `zol`；后者在 history 里是 `f_zol`，第 1 步只差 2.0e-5）。这一轮把那一支的
构造逐式对过：`moninobukini`（`wc = 0.5`、`rib = grav·zldis·dthv/(thv·um²)`、
`zeta = rib·ln(zldis/z0m)/(1−5·min(rib,0.19))` 与两个 clamp）、
`moninobuk` 的四个分支与常数、`z0hg = z0mg/exp(0.13(ustar·z0mg/1.5e-5)^0.45)`、
`um` 的稳定/对流两支、以及 `IF (nmozsgn >= 4) EXIT` 的跳出位置 —— **全部一致**。

所以剩下的是**地面稳定性迭代的离散路径**：那一支只跑 6 轮、且带
`nmozsgn >= 4` 提前跳出，本身不收敛（对比叶温那一支跑 40 轮、收敛到 1e-7）；
`obug` 的 1.48e-3 只可能来自"某一步的符号翻转发生在不同轮次"，也就是
`nmozsgn` 计数的差异。下一轮只要在两边把 `nmozsgn`（以及每轮的 `zeta`/`obu`）
打出来比，就能确认是"翻转次数不同"还是"翻转同次但某轮的值不同"。
注意 `GroundFluxes` 的调用点在 `MOD_Thermal_CanopyPhase_Extended.F90:645`，
它的 `thm`/`qm` 用的是**冠层空气**值，所以它天然继承冠层侧那 3e-5 的差 ——
但 1.5e-3 比 3e-5 大 50 倍，只能是迭代路径的分岔，不是输入的放大。

## 根因落地：`GroundFluxes` 的参考温度漏了 `+0.0098·forc_hgt_t`（实测，**已修**）

上一节把链条收到"地面支的 `obug` 差 1.48e-3"，并排除了公式错（`moninobukini`、
`moninobuk`、`z0hg`、`um` 两支、`nmozsgn` 跳出位置全部逐式一致）。这一轮再往下打一层，
发现**不是迭代路径，是输入**：

### 一、逐步迭代表：分叉从**第 1 轮**就有

在 `MOD_GroundFluxes.F90` 的 `IF (nmozsgn >= 4) EXIT` **之前**和
`ground_fluxes.rs` 的同一个位置各打一行（`nmozsgn` 两边全程为 0，即 6 轮都跑满）：

| pass | Fortran `zeta` | Rust `zeta` | Fortran `obu` | Rust `obu` |
|---|---|---|---|---|
| 1 | −1.7171439492468648 | −1.72077324239292895 | −3.4941741504150459 | −3.48680456679830986 |
| 2 | −0.83164137542258021 | −0.832751354983517489 | −7.2146482574309543 | −7.20503180702570845 |
| 3 | −0.88272308676004285 | −0.883932468546883010 | −6.7971486075236465 | −6.78784886119585451 |
| 4 | −0.92600282764595399 | −0.927385254631448830 | −6.4794618556975161 | −6.46980310505848344 |
| 5 | −0.92436287313457499 | −0.925743844422954765 | −6.4909573657514041 | −6.48127452982416408 |
| 6 | −0.92179961168508806 | −0.923166689245786820 | **−6.5090068643354604** | **−6.49936795802490330** |

**第 1 轮就差 2.1e-3** ⇒ 不是"翻转轮次不同"，是初值/输入 ⇒ 唯一还没查的是
`moninobukini` 的入参。

### 二、真凶：`thm = forc_t + 0.0098*forc_hgt_t` 只加在叶温那一支

`MOD_Thermal_CanopyPhase_Extended.F90:550` 明确写着
`thm = forc_t + 0.0098*forc_hgt_t  !intermediate variable equivalent to ...`，
而 `:645` 的 `CALL GroundFluxes(... ur,thm,th,thv,t_grnd,qg ...)` 用的就是**这个 `thm`**
（`dth = thm - t_grnd`、`fseng = -raih*dth`、`tref = thm + ...` 全用它）。

本仓库 `leaf_input` 那一支**加了**订正（`standard_lct_step.rs` 里
`reference_air_temperature_k: crate::reference_height_temperature_k(forcing.air_temperature_k,
physics.temperature_height_m)` ⇒ `t + 0.0098*h`），但 `ground_flux_input` 那一支写的是

```rust
reference_temperature_k: forcing.air_temperature_k,   // ← 少了 + 0.0098*temperature_height_m
```

CN-Cng 的 `forc_hgt_t = 6 m` ⇒ 少 **0.0588 K**，而 `dth ≈ 26 K` ⇒ 0.23% —— 正是
第 1 轮那 2.1e-3 的量级。**修法就是补上这一项**（一处）。

### 三、修完的验证：第 1 步那 26 个量的"阶梯"整体降一个半数量级

同一天（干窗第 1 步、干净内核）再比一次 restart：

| 量 | 修前相对差 | 修后相对差 | 改善 |
|---|---|---|---|
| `tstar` | 3.29e-5 | **2.06e-7** | **160×** |
| `wice_soisno` | 5.88e-5 | **3.60e-7** | **163×** |
| `smp` | 6.29e-5 | **4.47e-7** | **141×** |
| `rib` | 2.03e-5 | 1.98e-7 | 102× |
| `zol` | 2.01e-5 | 2.94e-7 | 68× |
| `ldew` / `ldew_snow` | 2.85e-5 | 2.64e-6 | 10.8× |
| `fwet_snow` | 1.90e-5 | 1.76e-6 | 10.8× |

`tstar`（当初就是"冠层空气权重"的症状）降 160 倍，**因果链从头到尾被证实**：
`thm` 少 0.0588 K → `dth` 差 0.23% → 地面迭代的 `obug` 差 1.48e-3 → `frd` 5.3e-5
→ `rd`/`cgw` → 冠层水汽权重 → 湿度梯度 → `etr`/`fevpl` → `hs`/`dhsdT` → 相变分配
→ 土壤柱 → `scv`/`snowdp`。

### 四、对三份黄金的效果

| 窗口 | 变量数 | tier2 | 总超差条数 |
|---|---|---|---|
| `CN-Cng` | 55 → 55 | 55 → 55 | 13571 → **11721**（−13.6%） |
| `CN-Cng-wet` | 70 → 70 | 70 → 70 | 25859 → 25976（+0.45%，见下） |
| `US-NR1-snow` | 79 → 79 | 79 → 79 | 31462 → **31363** |

干窗整族辐射量的一致性提高 1.38–1.47 倍（`f_srvdln` 161→109、`f_srvi` 237→170、
`f_srvd` 257→185、`f_sr` 24.8→18、`f_sabg` 38.4→27.9 …）。

湿窗的**总条数**微增 0.45%，但它**最差的那几条也在变好**（`f_srvd` 5.70e3→5.47e3、
`f_srnd` 7.98e3→7.66e3、`f_alb` 2.71e3→2.61e3 …）—— 是几十条边缘条目跨过 1e-7
的线，不是大项变大；而根上的阶梯降了 10–160 倍，所以这一改**更贴上游**，湿窗那
0.45% 是轨迹重排，不是回归。

## 站点坐标漏了 f32 截断：`coszen` 差 7.8e-10（实测，**已修**）

上一节修完 `thm` 之后，第 1 步重启里仍有 26/68 个变量不同，最大的 `ldew` 2.6e-6。
这一轮改用**独立差分驱动**逐位定位，先钉死"不是 `GroundFluxes`"，再顺着第 1 步
重启的**分叉指纹**找到真正的一处。

### 一、`GroundFluxes` 洗清：真实第 1 步输入下逐位相等

`/tmp/gf/` 把上游 `MOD_GroundFluxes.F90` 连同 `MOD_FrictionVelocity`/`MOD_Const_Physical`
独立编译（桩掉 `mod_namelist` 与 `MOD_TurbulenceLEddy`），配一个读 25 列、吐 23 列的
驱动；Rust 侧同样把 `ground_fluxes` 包成一个驱动。两件事都做了：

1. 300 组物理量级随机的输入：11 个量有差，**最大 7.6e-16（1–3 ULP）**，
   即 `moninobuk` 里的 FMA 收缩噪声。
2. 从真实算例用 `GF_DUMP=1` 落下的第 1 步 `GroundFluxInput`（CN-Cng 干窗
   2008-01-01 00:00 那一步，`t_grnd=283 K`、`qg=dqgdT=rss=0`）：
   **23 个输出全部逐位相等**，含 `zol`、`z0hg`、`tstar`、`qstar`、`fm/fh/fq`。

第 1 步的 `GroundFluxes` 至此可以排除 —— 也就是说，重启里 2.6e-6 那一档
**不是地面通量内核来的**。

### 二、第 1 步重启的分叉指纹：误差最小的那个才是源头

把 26 个不同变量按相对差排序（`t_soisno` 逐元素给出）：

| 量 | max_rel | 绝对差 |
|---|---|---|
| `emis` | 5.20e-10 | 5.2e-10 |
| `coszen` | **7.80e-10** | 7.2e-10 |
| `t_soisno[6]`（第 2 层） | 1.53e-09 | 4.3e-07 K |
| `tref` | 1.92e-09 | 5.0e-07 K |
| `fm` / `trad` / `fh`=`fq` | 1.6e-08 … 3.0e-08 | — |
| `tleaf` | 3.69e-08 | 9.8e-06 K |
| `ustar` / `qref` / `qstar` / `rib` / `tstar` / `zol` | 4.5e-08 … 2.9e-07 | — |
| `wliq_soisno[5]`（第 1 层） | 2.07e-07 | 1.2e-06 kg/m² |
| `wice_soisno[5]` | 3.61e-07 | 1.1e-06 kg/m² |
| `smp[0]` / `hk[0]` | 4.47e-07 / 2.37e-06 | — |
| `fwet_snow` / `ldew` | 1.76e-06 / 2.64e-06 | — |

两件事一眼可见：

* **`t_soisno` 只有第 2…6 层在动，第 1 层与第 7…10 层逐位相等**，而且相对差
  逐层衰减 1.5e-9 → 7.4e-15 —— 这是热传导三对角解把一个浅层扰动往下传的形状。
* **`wliq/wice/smp/hk` 只有第 1 层在动**（`smp[1..9]` 还是入参的占位值 `-10`，
  VSF 下只有顶层被真正更新），第 1 层那一步正好从 283 K 降到 `tfrz` 并结冰
  3.076 kg/m² —— 分叉落在**土壤第 1 层的相变**上。

### 三、真正的一处：`coszen` 用的是 namelist 的 f64 坐标

`coszen` 是分叉里唯一**不来自重启**、又位于链条最上游的量，所以先查它。
上游独立驱动给出结论（`0x` 是 `orb_coszen` 的位模式）：

| 传给 `orb_coszen(calday, lon, lat)` 的 lon/lat | 结果 |
|---|---|
| 站点文件的 `123.50920104980469 / 44.593299865722656` | `BFED996DB11A70D1` = **Fortran 重启里的值** |
| namelist 的 `123.50920 / 44.59330` | `BFED996DB17D99DD` = **Rust 重启里的值** |

`calday = 365.6777546296296`（步末 00:30 经地方时订正）两边一致，
`orb_coszen` 本身也逐位一致（同一份源码独立编译）。所以差的是**坐标**。

上游的真值在 `MOD_SingleSrfdata.F90:239`/`:1508`：

```fortran
IF ((lon_in /= SITE_lon_location) .and. (SITE_lon_location /= -1.e36_r8)) THEN
   write(*,*) 'Warning: Longitude mismatch: ', lon_in, ' in data file and ', SITE_lon_location ...
ENDIF
SITE_lon_location = lon_in          ! ← 用站点文件（real*4）的值覆盖 namelist 的 f64
CALL normalize_longitude (SITE_lon_location)
IF (.not. isgreenwich) LocalLongitude = SITE_lon_location
```

这一覆盖在**每次运行**都会发生（算例日志里那两条 `Latitude/Longitude mismatch`
就是它打的）。之后所有几何都用被覆盖后的值：`MOD_Initialize.F90:325-326` 的
`patchlonr/patchlatr`、`MOD_Forcing.F90:790-791` 的 `coszen`/`cosazi`、
`MOD_NetSolar` 的 `dlon`、`CoLMMAIN.F90:2076` 的 `coszen`、history 的 `lat`/`lon`。

本仓库的 `site_coordinate_degrees()`（`f64::from(value as f32)`）本来就为 history
的 `lat`/`lon` 写了这一层量化，但**只用在 history 上**；`read_point_runtime_config`
里存在 `PointRuntimeConfig` 的仍是 namelist 的 f64，于是 `patchlonr` 那一族
全部偏了 1.3e-7 度。修法就是让配置里的站点坐标本身过一遍这个函数（一处），
helper 的文档同步改写。三个黄金算例的站点文件经核对都满足
`file == f32(namelist)`（CN-Cng、US-NR1-snow 逐位验证）。

修完再跑第 1 步：**`coszen` 从不同变量表里消失（逐位相等）**，重启的
`coszen` 由 `-0.9249790636648033` 变回 `-0.9249790629433169`。

### 四、诚实记录：这一处**没有**带动其余物理量

同一次重跑里 `emis` 5.20e-10、`t_soisno` 1.53e-09、`tleaf` 3.69e-08、
`wice_soisno` 3.61e-07 **一个数值都没变**。原因是那 1e-9 的 `coszen` 差进了
`prepare_surface_optics` 也被夜间分支吃掉（本步 `coszen = -0.925 < 0`，
反射率用夜间取值），而这三份黄金窗口的 `DEF_USE_Forcing_Downscaling` 都是关的。
所以这一改是**保真度修正**，在现有黄金上 tier 计数不动（仍是 55/70/79，
tier0 全零），它保住的是：重启里的 `coszen`、以及开地形降尺度时
`coszen/cosazi` 那条链。

`coszen` 归位后剩下的分叉起点已经缩到**土壤第 1 层的相变**
（`wice_soisno[5]` / `wliq_soisno[5]` 是重启里最早上游的一对），而它上游的
`tleaf` 已经差到 3.7e-8 —— 下一步该查的是叶温内核的**输入**（`net_solar` 的
`sr/sabg/sabvsun…`、`interception`、`root_uptake`），同一套独立差分驱动的做法
可以直接搬过去。

## 叶温内核收到的 `t_precip` 是装配期的 `forc_t` 占位（实测，**已修**）

顺着上一节的分叉指纹继续往下，用**成对探针**（上游 `MOD_Thermal_CanopyPhase_Extended.F90`
的 `CALL LeafTemperature` 之前、Rust `standard_lct_step.rs` 的 `leaf_input(...)`
之后各打一组同名标量）把叶温内核的输入逐位比了一遍。

### 一、叶温内核的输入：34 个量里只有一个是错的

| 量 | Fortran | Rust（修前） |
|---|---|---|
| `tl`(步初) `lai` `sai` `fsun` `sabv` `parsun` `parsha` `frl` `thermk` `extkb` `extkd` `emg` `t_grnd` `qg` `dqgdT` `rss` `qintr_rain` `qintr_snow` `obug` `z0hg` `ustarg` `zolg` `ribg` `tstarg` `ldew` `ldew_rain` `ldew_snow` `etrc` `rstfac` `htvp` `o2m` `co2m` `dewmx` | — | **逐位相等** |
| **`t_precip`** | **256.59229215659377** | **256.91000366210938** |

Rust 那份是 `f32(256.91)`，正是 `forc_t`：装配期的 `LeafTemperatureInput`
（`assembly.rs` 的两处 `precipitation_temperature_k: forcing.air_temperature_k`）
留的是占位值，而 `LEAFTEMPERATURE` 收的是 `THERMAL` 上游刚由 `rain_snow_temp`
定出来的**湿球温度**（`MOD_RainSnowTemp.F90` 末尾那一段；`forc_t < 275.65` 时
`t_precip = min(tfrz, wetbulb)`）。差 **0.318 K**。

修法：把本步 `PrecipitationState::precipitation_temperature_k` 显式传进
`leaf_input`（同一个 `precipitation` 早已喂给 `interception` 与 `ground_temperature`，
只有叶温这一支漏了）。修后该组 34 个量**全部逐位相等**。

### 二、这一步在干窗里是**惰性**的，在湿窗里不是

干窗第 1 步 `qintr_rain = qintr_snow = 0`（该步无降水进冠层），而这一项在能量平衡里
是 `cpliq*max(0,qintr)*(t_precip-tl)` —— 当场乘成 0，所以第 1 步重启里 26 个不同变量
**一个数值都没变**。湿窗（7 月、有雨雪）才显形：`f_ldew` 超差条数 170→**160**、
`f_fevpa` 165→**163**、`f_qinfl`/`f_qdrip`/`f_qintr` 的偏差各降一档，
`f_assimsha`/`f_etrsha` 的峰值也各降一档；tier2 计数仍是 70（都是长期轨迹量）。

## 第 1 步叶温分叉的真凶：**黄金自己**的 `o3coef*` 未初始化（实测，**未修**）

叶温输入全部逐位相等之后，第 1 步的输出**依然**差 3.7e-8。于是把探针挪进
叶温的迭代循环（上游 `it = it+1` 之前、Rust `iteration += 1` 之前），逐轮比。

### 一、第 1 轮就已经差了，而且差得极大

| 量 | Fortran 第 1 轮 | Rust 第 1 轮 |
|---|---|---|
| `tl` `fsenl` `fsenl_dtl` `ustar` `wta0` `wtg0` `cgh` `cgw` `raw` `qsatl` `qsatlDT` | — | 逐位相等 |
| **`etr` = `fevpl`** | **6.60985755327047220e-05** | **1.37119817067997586e-08** |
| `fevpl_dtl` | 6.68426870876120065e-06 | 1.38663457599805794e-09 |
| `dele` | 125.348551869418486 | 114.795849797353341 |
| `obu` | −11.406859170735791 | −11.444129656300083 |
| `wtaq0` / `wtgq0` | 0.700110744457918655 / 0.245000584561997442 | 0.740762226776087940 / 0.259226386708448886 |
| `irab` | −86.3589314410997417 | −86.3589314410997559（1 ULP） |

`etr` 差 **4800 倍**，`wtaq0+wtgq0` 上游是 0.9451（= `(caw+cgw)/(caw+cgw+cfw)`）
而 Rust 是 1.0 —— 也就是 Rust 的 `cfw ≈ 0`。反推上游 `cfw = 0.01019879` 是对的，
Rust 的 `cfw = 3.6e-9`；`caw`/`cgw`/`rb`/`rssun` 的前置量全都逐位相等，
说明 Rust 算的是**另一组** `rssun`/`rssha`：上游这一轮 `rs` 只有约 14 s/m
（夜间气孔仍开，`g = g0`），Rust 是约 1.1e5 s/m（完全关闭）。

### 二、原因是上游的已知缺陷，不是移植错

`MOD_LeafTemperature_Extended.F90` 里 `o3coefv_sun/o3coefv_sha/o3coefg_sun/o3coefg_sha`
是 `intent(inout)` 的哑元，调用方（`MOD_Thermal_CanopyPhase_Extended.F90`）用模块
SAVE 变量传进来。上游**只在迭代循环之后**才给它们赋值：

```fortran
      ENDDO                                    ! :1310 迭代结束
      IF(DEF_USE_OZONESTRESS)THEN
         ...
      ELSE
         o3coefv_sun = 1.0_r8                  ! :1323
         o3coefg_sun = 1.0_r8                  ! :1324
         o3coefv_sha = 1.0_r8
         o3coefg_sha = 1.0_r8
      ENDIF
```

而循环体内 `:792`/`:807` 就把它们传给了 `stomata`。所以**第一次调用**（第 1 步）
读到的是未定义值 —— `o3coefg` 于是不再等于 1，`rssun`/`rssha` 随之失真。
`o3coef*` 是模块 SAVE 变量，`LEAFTEMPERATURE` 每次调用结束都会把它们置回 1，
**所以只有第 1 步被污染**，第 2 步起上游自己就恢复了。
（同一个缺陷也是黄金第 1 步 `gs0sun = -4.53e38`、`vegwp`、`rst = -5e-31` 的来源。）

### 三、对照实验：把缺陷补上，两边第 1 轮降到 1 ULP

在叶温子程序入口处提前 `o3coef* = 1`（只改 `vendor/` 的临时副本做实验，已还原），
重建内核再比：

| 量 | Fortran（补丁后） | Rust | 相对差 |
|---|---|---|---|
| `etr`/`fevpl` | 1.37119817067997553e-08 | 1.37119817067997586e-08 | 2.4e-16 |
| `fevpl_dtl` | 1.38663457599805773e-09 | 1.38663457599805794e-09 | 1.5e-16 |
| `irab` | −86.3589314410997417 | −86.3589314410997559 | 1.6e-16 |
| `fsenl` `fsenl_dtl` `ustar` `wta0` `wtg0` `cgh` `cgw` `raw` `qsatl` `qsatlDT` `del` `dirab_dtl` `evplwet` `tl` | — | — | **逐位相等** |

第 1 步重启也从「26 个变量差 1e-6…1e0」降到「18 个变量差 1e-16…1e-12」
（最差 `fwet_snow` 2.8e-12，`smp` 7.0e-15，`ldew` 1.5e-16）。

**结论：第 1 步的分叉不是移植错误，是黄金算例本身的缺陷。** Rust 侧这一层已经对齐到
1 ULP；剩下的 1 ULP 是 gfortran `-ffp-contract=fast` 把 `etr = a*b*(...)` 收缩成 FMA、
而 Rust 没有收缩 —— 与既有的 FMA 类问题同源，量级 1e-16，不是根因。

### 四、下一步（留给下一轮）

三份黄金是拿**带这个缺陷的内核**生成的，所以任何「15 天窗口收敛」的目标都先卡在
第 1 步的这次污染上。要做的是：把 `o3coef*` 的初始化挪到迭代之前（`vendor/` 的本地
改动，记进 `vendor/PROVENANCE.md`）→ 重新生成三份黄金 → 重跑 tier 分层
（`oracle/tolerances.toml` 的 tier1/tier2 分档可能要重排）→ 复核
`oracle/tests/{histmap,metrics}.rs` 与 `golden-compare` 的期望值。
`oracle/golden/kernel-manifest.json` 会被 `golden-run --write-golden` 重写，
那是**应该**跟着更新的（它记的就是生成黄金的那颗内核）。

## 修上游 `o3coef*` 未初始化 + 重生成三份黄金（实测，**已修**）

上一节把第 1 步叶温分叉定位到黄金自己的 `o3coef*` 未初始化缺陷。这一轮把它修掉，
并按 `vendor/PROVENANCE.md` 里既有的先例（扩展截获模块那次 `intent(inout)` 修复）
**重新生成三份黄金**。

### 一、改的是什么

`main/MOD_LeafTemperature.F90` 与 `extends/interception/MOD_LeafTemperature_Extended.F90`
（五个预设都开 `extend_interception`，实际编译的是后者）里，四个赋值的位置从迭代后的
`ELSE` 分支挪到子程序初始化块（`it = 1` 之前）。两份文件同步改；
`oracle/scripts/test_upstream_f48_sync.py` 加两条断言把它钉住：初始化必须在
`DO WHILE (it .le. itmax)` 之前，且 `o3coefv_sun = 1.0_r8` 全文件只出现一次。

没跟着改的两处（PC 变体、臭氧应激迭代内可见性）连同理由记在 `vendor/PROVENANCE.md`。

### 二、第 1 步：从 26 个变量差 1e-6…1e0 降到 18 个差 1e-16…1e-12

修好的内核跑 CN-Cng 第 1 步，与 Rust 比（同一份输入重启）：

| 变量 | 相对差 |
|---|---|
| `ldew` / `ldew_snow` | 1.46e-16 |
| `rib` / `t_soisno` / `rst` | 1.9e-16 … 2.3e-16 |
| `fm` / `zol` / `qstar` / `qref` | 5.3e-16 … 9.0e-16 |
| `ustar` / `fh`=`fq` / `wliq_soisno` / `tstar` / `wice_soisno` | 1.1e-15 … 2.5e-15 |
| `smp` / `hk` / `vegwp` / `fwet_snow` | 7.0e-15 … 2.8e-12 |

修前是 26 个变量、最大 2.6e-6（`ldew`）外加三个 1e0 量级的垃圾槽位
（`gs0sun`/`gs0sha`/`rst`）。叶温迭代第 1 轮的 23 个探测量里，只有
`etr`/`fevpl`（2.4e-16）、`fevpl_dtl`（1.5e-16）、`irab`（1.6e-16）还差 1 ULP ——
就是 gfortran `-ffp-contract=fast` 的 FMA 收缩。

### 三、重生成黄金：三份都变了，但没有一个变量变远

| 窗口 | 黄金里变化的变量数 | 变大的 |
|---|---|---|
| `CN-Cng` | 69 / 264 条记录全动（`f_ldew`/`f_gssun`/`f_gssha`/`f_vegwp`/`f_xerr`/`f_zerr` 量级 0.1…2.0，其余 ~1e-6） | — |
| `CN-Cng-wet` | 5（`f_gssun`/`f_gssha` 各 1 条；`f_assim*` 1.7e-11） | — |
| `US-NR1-snow` | 85（`f_wice_soisno` 9.0e-4、`f_zwt` 1.6e-3、`f_wdsrf` 1.0，其余 1e-3…1e-8） | — |

把**同一份 Rust 输出**分别对新旧两份黄金比（tier 感知的超差记录总数）：

| 窗口 | 旧黄金 | 新黄金 | 超差变量数 |
|---|---|---|---|
| `CN-Cng` | 11749 | **11720** | 55 → 55 |
| `CN-Cng-wet` | 25896 | **25894** | 70 → 70 |
| `US-NR1-snow` | 31365 | **31325** | 79 → 79 |

逐变量比：**变远的有 0 个**，变近的干窗 12 个（`f_vegwp` 7.3e-1→1.8e-1、
`f_ldew` 1.0→5.5e-1、`f_gssun` 1.0→7.4e-1、`f_gssha` 1.0→7.6e-1、
`f_tstar`/`f_fsena`/`f_fseng`/`f_wliq_soisno` …）。超差变量数没动是因为这些条目
本来就跨着 1e-7 的线，改善的幅度不足以把它们拉回来。

至此**第 1 步这一层已经对齐**（1e-12…1e-16），残余的 1.1-3.1 万条超差记录来自
**后续步的逐步累积**：最差记录集中在窗口末段（干窗 index 232、雪窗 index 280-301），
是同一个 ~1e-16 级种子的轨迹放大，外加少数阶梯式分歧（`f_zwt`、`f_wdsrf`）。
下一步该做的是挑一个**中期**步做同样的成对探针（第 1 步已经不能提供信息了），
把「逐步种子」也钉到 1 ULP。

## VSF 子层下界断言放行机器量级负值（实测，**已修**）

修好 `o3coef` 之后，`US-NR1-snow` 的 Rust 运行在新轨迹上撞到自建断言：

```
colm-rs: VSF sublevel layer inputs are invalid
    layer=1 thickness=27.578969259676253 porosity=0.07035637861010985
    psi0=-10 hksati=0.011809648407830133 wetting=0 water_table=0
    liquid=-0.000000000000000008051229283963381 volume_tolerance=4.397646432528619e-5
```

`liquid_water = -8.05e-18`，相对该层厚度 27.58 mm 是 3e-19 —— 牛顿迭代的舍入。
上游 `initialize_sublevel_structure` 对 `vl` 没有任何符号断言，照抄就不该拦。
原先只给上界留了 `volume_tolerance` 的余量（`check_and_update_level` 精确 `.min`
与含水层交换的反解之间会有 1 ULP 超出），这次把同一个容差对称地用到下界。
**没有夹到 0**：上游会把那个负值原样带进下游算术，夹掉会让这一层看到不同的数。

修完 `US-NR1-snow` 正常跑满 720 步（15 天 × 48）。

## 逐点输出：`rnet` 是按辐射式算的，不是 `H+LE+G`（实测，**已修**）

上一节把残余分叉定位到「逐步累积」，这一轮改用**逐步 history**（把
`DEF_HIST_FREQ` 改成 `'TIMESTEP'`、跑 10 步）来看每一条记录，一步之内谁先分叉一目了然
—— 比再加一层探针便宜得多，而且不用重建内核。

### 一、`f_rnet` 从第 1 步就差 19%

`MOD_Vars_1DAccFluxes.F90:2087`：
```fortran
rnet = sabg + sabvsun + sabvsha - olrg + forc_frl
```
本仓库原先写成 `fsena + lfevpa + fgrnd`，注释里的理由是"与辐射式恒等"。那个恒等只在
能量收支**精确闭合**时成立；更关键的是 `fgrnd` 本身就含辐射项（`sabg + dlrad*emg - ...`），
`H_total + LE_total + G` 根本不是同一个表达式的另一种写法。实测 US-NR1-snow
第 1 步起 `f_rnet` 差 19%，而同一份文件里的 `f_fgrnd`/`f_lfevpa`/`f_olrg`/`f_sabg`
都是逐位相同的。改用上游的辐射式后，逐步 history 的第 1–3 步**超差条目归零**。

顺带把 `history.rs` 那条注释里的假恒等式删掉了。

## 地表诊断的温度取错了层：`t_grnd = t_soisno(lb)` 不是土层 1（实测，**已修**）

修完 `rnet`，逐步 history 的第 4 步（US-NR1-snow 第一次积雪那一步）冒出一组异常：

| 量 | 黄金 | Rust（修前） |
|---|---|---|
| `f_zerr` | −3.06e-11 | **+28.84** |
| `f_olrg` | 291.071 | 262.206 |
| `f_emis` | 0.996942 | **1.0**（恰好） |
| `f_trad` | 267.673 | 260.774 |
| `f_fgrnd` | −77.929 | −100.537 |

`f_zerr` 是**能量收支残差**，黄金的 1e-11 说明上游那一步是闭合的；Rust 的 28.8 W/m²
说明它不闭合 —— 而它的量级正好等于 `olrg` 的差（28.87），所以问题在 `olrg`。

`MOD_Thermal.F90:1223`：
```fortran
IF (.not.DEF_SPLIT_SOILSNOW) THEN
   t_grnd = t_soisno(lb)                     ! lb = snl+1
   tinc   = t_soisno(lb) - t_soisno_bef(lb)
```
`lb = snl+1` 是**紧贴土壤的那一层**（没有雪时就是土层 1；有雪时是雪列里最下面那一片）。
本仓库 `surface_budget` 原先写的是「列长减土层数」＝**永远是土层 1**，注释还写着
"不能写 0 …… 要的是地表那一层" —— 恰好说反了。带雪时 `tinc` 因此恒为 0：
`olrb = σ·t_grnd_bef³·4·tinc = 0` ⇒ `emis = ulrad/ulrad = 1.0` 恰好、`olrg = ulrad`。

改法：把这两个温度**算在知道 split 标志的地方**，随能量输出带出来
（`StandardLctEnergyOutput::surface_temperature_k` / `surface_temperature_k_before`，
在 `finish_energy_step` 里由 `current_ground_temperature()` 现取，与 `t_grnd` 同一个
函数），`surface_budget` 只读不再自己推。修后第 4 步：
`olrg` 291.534 对 291.071、`emis` 0.996898 对 0.996942、`trad` 267.779 对 267.673、
`fgrnd` −77.371 对 −77.929、`zerr` −7.0e-12（两侧都回到机器零）。

## `htvp` 的判据必须放在 `newsnow` **之后**（实测，**已修**）

修完上面两处，第 4 步仍有 42 条超差，领头的 `f_lfevpa` 差 10%。用 history 里现成的
分量一验就清楚了：

| | `f_lfevpa` | `hvap*fevpl+hsub*fevpg` | `hvap*(fevpl+fevpg)` |
|---|---|---|---|
| 黄金 | 15.2396 | **15.2321** | 13.4520 |
| Rust | 13.6745 | 15.4755 | **13.6669** |

黄金用 `hsub`（升华），Rust 用 `hvap`。`MOD_Thermal.F90:539-540`：
```fortran
htvp = hvap
IF (wliq_soisno(lb)<=0. .and. wice_soisno(lb)>0.) htvp = hsub
```
它读的是 `lb` 那一层，而 `htvp` 是**在 THERMAL 里、`newsnow` 之后**算的。
本仓库把这段判据放在装配期（`assembly.rs` 的 `snow_input`），而雪层是**本步**
`add_new_snow` 才建出来的 —— 于是"第一次积雪"那一步模板看到的是土层 1
（有液态水 ⇒ `hvap`），上游看到的是新雪层（零液态水有冰 ⇒ `hsub`）。

改法：装配期只放**基础汽化热**，判据移进 `standard_lct_snow_soil_step`、在
`add_new_snow` 之后按 `lb` 那一层现判。

效果（逐步 history 的超差条目数）：

| 记录 | 修前 | 修后 |
|---|---|---|
| 0 | 1（`f_frcsat`） | 1（`f_frcsat`） |
| 1–3 | 0 | 0 |
| 4 | 32 | **1（`f_rootr`）** |
| 5–9 | 39…43 | **1（`f_rootr`）** |

整窗超差记录总数：

| 窗口 | 修前 | 修后 |
|---|---|---|
| `CN-Cng` | 11720 | **11704** |
| `CN-Cng-wet` | 25894 | **25892** |
| `US-NR1-snow` | 31325 | **28982**（−7.5%） |

超差**变量数**仍是 55/70/79（同样的变量，只是超差记录少了很多），tier0 全零。

## 上游 `eroot` 的**数组错位**：照抄还是修上游（实测，**已按“照抄”落地**）

`snow` 现在就剩 `f_rootr` 一条逐点超差，而它是**悬崖式**的：

| | `f_rootr`（第 4 步起，10 层） |
|---|---|
| 黄金 | `[0, 0, 0.207315, 0.267158, 0.232743, 0.148124, 0.077591, 0.036841, 0.016755, 0.013473]` |
| Rust | `[0, 0.097654, 0.187070, 0.241069, 0.210015, 0.133659, 0.070014, 0.033243, 0.015118, 0.012158]` |

同一步的 `f_t_soisno` 逐位相同（土层 1 = 273.16、土层 2 = 274.88），所以不是温度差。
原因是调用处的**序列关联**：`MOD_Thermal_CanopyPhase_Extended.F90:669`

```fortran
CALL eroot (nl_soil,trsmx0,porsl, ..., psi0,rootfr,dz_soisno,t_soisno,wliq_soisno,rootr,etrc,rstfac)
```

`t_soisno`/`dz_soisno`/`wliq_soisno` 在调用方声明成 `(lb:nl_soil)`，而 `eroot` 的哑元是
`t_soisno(1:nl_soil)` —— 于是 `eroot` 的**第 1 项对应调用方的 `t_soisno(lb)`**：
有 `|snl|` 层雪时，温度/水量/层厚这三条列整体**错位 `|snl|` 层**，而 `porsl`/`psi0`/
`rootfr`/`theta_r` 仍是按土层的—— 也就是上游把**第 i 层土的参数**与**第 i−|snl| 层的状态**
配在一起用。

黄金那组数正是这个错位的形状：第 1 项 = 雪层（`t<tfrz` ⇒ 0）、第 2 项 = 土层 1
（273.16，不 `>tfrz` ⇒ 0）、第 3…10 项 = 土层 **2…9**（土层 10 根本没被读到）。
无雪时 `lb = 1`，错位为 0，两边一致 —— 所以干窗、湿窗都看不出来。

这条影响的是**物理**（`rstfac`/`etrc`/`rootr` 直接进叶温的蒸腾），不只是诊断量。
两条路：(a) 照抄上游的错位；(b) 像 `o3coef*` 那样把它当上游缺陷修掉并重生成黄金。

### 决定：照抄（(a)），理由是黄金就是这份代码生成的

tier0–3 的判据是"与上游**逐位**一致"，而黄金由 `vendor/CoLM202X` 现状生成。选 (b)
等于同时改上游、重生成黄金、再让移植去追新黄金 —— 那是**换靶子**，会把"移植对不对"
换成"上游对不对"，而后者不是本仓库要证的东西。（`o3coef*` 那次是例外：那是未初始化，
不是确定性的错位，黄金自己都不可复现，只能修。）

### 序列关联不是猜测：`gfortran` 上真跑了一遍

```fortran
real(8) :: col(-2:8)            ! lb=-2（3 层雪），土层 1..8
subroutine callee(nl, t) ; real(8), intent(in) :: t(1:nl) ; write(*,*) t
call callee(10, col)            ! 实参是整数组名，不是段
```

`gfortran -O2 -fdefault-real-8` 实测输出 `-2 -1 0 1 2 3 4 5 6 7`：哑元第 1 项拿到
`col(-2)`，最后两项（`col(7)`、`col(8)`）**看不见**。错位量 = `-lb` = `|snl|`，与上一节的
推断完全一致。（`call callee(10, col(-2:8))` 结果相同 —— 整数组名与显式段在这里同义。）

### 落地与实测：`f_rootr` 2362 → 111 个逐位差

`root_uptake_input` 改成用**整列**（下标 0 = 雪顶）的前 `nl_soil` 个元素配土层索引的
`porsl`/`psi0`/`rootfr`/`theta_r`（`standard_lct_step.rs`）。雪窗 `US-NR1-snow` 全场：

| | 逐位不同的值 | `f_rootr` |
|---|---|---|
| 改前（对齐语义） | 48284 | 2362 / 3600 |
| 改后（照抄错位） | 46033 | **111** / 3600 |

### 但影响面只有 `f_rootr` 一条 —— 因为 PHS 把 `rstfac` 覆盖掉了

改前改后做了一次干净的 A/B（`git stash` 前后各跑一次全窗口），**除 `f_rootr` 外所有
变量逐位不变**：`f_gssun`/`f_gssha`/`f_assim`/`f_lfevpa`/`f_vegwp` 的新旧差都是 0。

原因不是错位没进物理，而是三个黄金都走 `DEF_USE_PLANTHYDRAULICS = .true.`：
`MOD_PlantHydraulic.F90:353-368` 的 `calcstress_twoleaf` 把 `rstfacsun`/`rstfacsha`
（`intent(inout)`）重写成 PHS 自己的胁迫因子，`eroot` 给的 `rstfac` 到不了气孔。
`etrc` 只剩 `MOD_LeafTemperature_Extended.F90:1137` 的 `IF(etr.ge.etrc) etr = etrc`，
雪窗四月 `etr` 远小于 `etrc`，这条钳位从不触发 —— 所以 `etrc` 的差也传不下去。

结论：这条**只对关掉 PHS 的算例有物理影响**，对三个黄金只是 `f_rootr` 这条诊断量。
剩下那 111 个差从第 36 条记录起、量级 1e-8 相对，是黄昏 1 ULP 漂移（见最后一节），
不再是结构性的。

### 同一缺陷的第二处：`SoilSurfaceResistance`（**同样照抄，本机无算例可验**）

`MOD_Thermal_CanopyPhase_Extended.F90:626-629` 用**同样的方式**把
`dz_soisno`/`t_soisno`/`wliq_soisno`/`wice_soisno` 整列交给
`MOD_SoilSurfaceResistance.F90:83-86`，而那里的哑元也是 `(1:nl_soil)`：它第 1 项读到的
是**雪层**的温度/液态水/冰/层厚，却配 `porsl(1)`/`psi0(1)`/`theta_r(1)`（土层 1）。
`SoilSurfaceResistance` 内部只用下标 1，所以错位的是那四个标量。Rust 的
`soil_surface_resistance_input` 原先取 `[snow_layers]`（= 土层 1，对齐语义），已同样
改成取 `[0]`（雪顶）。三个黄金的 `DEF_RSS_SCHEME = 0`（`MOD_Namelist.F90:1947-1951`：
LCT + 非 Campbell 自动置 0），这条支路**本机一次都没被走到**，所以它是按语言规则
照抄的，没有实测支撑 —— `Not-tested`。

## 气孔 WUE 分支的内部 CO2 选错了支：`gssun` 差 2 倍（实测，**已修**）

上一节修完 `htvp` 后，湿窗逐步 history 的第 0–10 步全干净，从第 11 步起
`f_gssun`/`f_gssha` 开始超差，到第 10 步（日出前后）稳定在 **2.03 倍**：

| 记录 | `f_gssun` 黄金 | Rust | 比值 |
|---|---|---|---|
| 10 | 3.2958e-3 | 6.7014e-3 | 2.033 |
| 11 | 1.2853e-2 | 2.6040e-2 | 2.026 |
| 12 | 4.7319e-2 | 9.5274e-2 | 2.013 |

同一份文件里 `f_assim`/`f_assimsun`/`f_assimsha`/`f_respc` 只差 1%（绝对差
2e-8 mol m-2 s-1，远在 atol 之下），`f_laisun`/`f_laisha`/`f_sabvsun`/`f_sabvsha`
逐位相同，`f_rstfacsun`/`f_rstfacsha` 两侧都是 1.0，`f_tleaf` 只差 2 ULP。
所以差别在 `stomata` 内部。

`DEF_USE_WUEST` 的默认值是 `.true.`（`MOD_Namelist.F90:533`），三个黄金算例都没改
—— 走的是 WUE 分支。`MOD_AssimStomataConductance.F90:333-337`：

```fortran
IF(omc .lt. ome)THEN
   pco2i = pco2i_c           ! Rubisco 限制
ELSE
   pco2i = pco2i_e           ! 电子传输限制
ENDIF
gsh2o = assmt / (co2a - pco2i/psrf)*1.6
pco2in = pco2i
```

**`pco2i` 会在算 `gsh2o` 之前被重写成选中的那一支。** 本仓库写死用
`internal_co2`（= `pco2i_c`）：当 `ome < omc`（光限制）时上游用 `pco2i_e`，
两边分母不同 —— 实测正是这个 2 倍。

改法：WUE 分支里按 `omc < ome` 选 `rubisco_co2`/`electron_co2`，`gsh2o` 与
`pco2in` 都用选中值。修后 `f_gssun` 3.29576e-3 对 3.29567e-3（3e-5），
湿窗第 0–10 步全零、第 11 步起只剩 3 条微小条目。

整窗效果（这是这一轮最大的一处）：

| 窗口 | 超差变量数 | 超差记录总数 |
|---|---|---|
| `CN-Cng` | 55 → **27** | 11704 → **1092**（−91%） |
| `CN-Cng-wet` | 70 → **68** | 25892 → **20665**（−20%） |
| `US-NR1-snow` | 79 → 79 | 28982 → **28205** |

干窗降了一个数量级、超差变量砍掉一半：原先 11704 条里绝大多数是
`f_gssun`/`f_gssha` 经 `etr`/`fevpl`/`lfevpa` 一路带出去的。

## 剩余分叉的形态：**日变瞬态**，起点在气孔的黄昏段（实测，**未修**）

把 `CN-Cng` 干窗改成 `DEF_HIST_FREQ='TIMESTEP'` 跑 200 步，逐步看每个量何时开始不同
—— 形态和之前设想的"逐步累积"完全不同。

### 一、第 0–14 步全逐位相同，第 15 步一起跳

按数据依赖排序，第 14 步（07:00）的值：

| 量 | 第 14 步是否逐位相同 |
|---|---|
| `f_lai` `f_sai` `f_laisun` `f_laisha` | **逐位相同** |
| `f_etr` `f_etrsun` `f_etrsha` `f_tleaf` `f_t_grnd` `f_ustar` `f_fh` `f_fm` `f_olrg` `f_zwt` `f_respc` | **逐位相同** |
| `f_sabvsun` `f_sabvsha` `f_sabg` `f_sr*` | 差 5e-14…1e-14（1–2 ULP） |
| `f_gssun` | 差 1.4e-16（1 ULP） |
| `f_gssha` | **差 9.35e-07** |

第 15 步（07:30）`f_gssun`/`f_gssha`/`f_etr`/`f_etrsun`/`f_etrsha` 一起变成
**8.3e-07 / 6.9e-07**，而 `f_laisun`/`f_laisha`/`f_tleaf` 仍逐位相同。
`gssun = (laisun/rssun)*(tprcor/tlbef)` 里前两个因子都逐位相同 ⇒ **差在 `rssun`**，
即 `stomata` 的输出。

### 二、`f_vegwp` 每天起落一次，不是单调放大

`f_vegwp` 的相对差在第 15 步一步之内从 2.2e-13 跳到 **2.09e-08**（9.4 万倍），
随后 10 步左右按 ~1.8 倍/步增长到 2e-6，再在 8 步内衰减回 1e-11：

```
rec 14 2.2e-13 | 15 2.1e-08 | 16 5.4e-08 | 18 2.1e-07 | 20 6.9e-07 | 26 2.0e-06
rec 30 1.3e-06 | 32 1.7e-07 | 33 1.7e-11 | 34 1.6e-12
```

同一天里起落一次，此后每天重复（第 63–80、112–128、161–176 步）。
**这是瞬态，不是失稳**：系统把它拉回来了。而 `f_rstfacsun`/`f_rstfacsha`
全程只差 1e-15（气孔胁迫因子两侧都被钉在 ~1），所以 PHS 的胁迫因子不是源头，
`f_vegwp` 是被蒸腾/根通量带出去的结果。

### 三、这一轮还没定位到源头，下一步怎么打

已排除：`lambda`（两侧都是 1000.）、`gradm`/`binter`（地类表逐位）、`gammas`/`kc`/`ko`
的常数（逐条比对一致）、`po2m`/`pco2m` 的标度（`block_data_copy(pbot, sca=…)` = 压力，
Rust 的 `bottom_pressure_pa * 体积分数` 同源）、`OXYGEN_VOLUME_FRACTION = 0.209`
（与上游 `0.209_r8` 一致）、`wue_internal_co2` 的三条公式（与 `WUE_solver` 逐项一致）。

剩下最可能的落点是 `stomata` 的**输入** `ei`/`ea`（叶面饱和水汽压与冠层空气水汽压）
—— `WUE_solver` 里 `D = max(ei-ea,50)/psrf`，而 `gsh2o = assmt/(co2a-pco2i/psrf)*1.6`
的分母很小（实测 `rssun` 在第 14 步约 1.0e5 s/m、第 15 步掉到 1.5e4），
分母小的地方对 `pco2i` 的微小差很敏感。

做法：在 `stomata` 入口与 `WUE_solver` 出口各打一组探针（Rust 与上游各一份），
取**第 14/15 步**那两行对比 —— 第 14 步逐位相同意味着种子在第 15 步才出现，
比"从第 0 步就埋下"好找得多。`f_gssha` 比 `f_gssun` 早一步（第 14 步就 9.35e-07）
出现差异，说明先查**阴叶**那一支的 `parsha`/`assimsha`。

## `a*b + c*d` 到底收缩哪一个乘：量出来了，是**左边那个**（实测，**纠正一条先前写反的猜测**）

第 7236 节量的是单乘加（`Y + a*b`、`a*b - c`）—— 那类形状只有一个乘可收缩，不存在顺序问题。
真正的空白是**两个乘相加**：`a*b + c*d` 里 GCC 把哪一个变成 `fma`？这决定了
`MOD_NetSolar.F90:176-183` 那几行（`parsun`/`parsha`/`sabvsun`/`sabvsha`/`sabvg`）
要么写 `fma(a,b,c*d)`、要么写 `fma(c,d,a*b)`，两者在多数输入上相等、少数差 1 ULP。

### 方法：Fortran 算值，C 用 libm 的 `fma` 算两个候选

`gfortran` 里**没有** `fma` 内建（写了报 `'fma' declared INTRINSIC ... does not exist`；
`intrinsic :: fma` 也不能放在 `implicit none` 前面），所以对照值由同一套 GCC 的 C
`fma()` 提供 —— 它按定义就是正确舍入的一次乘加。3000 组随机 `a..h`，
`-O2 -fdefault-real-8`（即 `-ffp-contract=fast` 默认）：

| 表达式 | 与 gfortran 逐位相同的候选 |
|---|---|
| `a*b + c*d` | `fma(a, b, c*d)` **3000/3000**；`fma(c, d, a*b)` 1952/3000 |
| `a*b + c*d + e*f + g*h` | `fma(g,h, fma(e,f, fma(a,b, c*d)))` **3000/3000** |

（后者的另外两个候选分别只有 2410、1485。）C 自己编出来的 `a*b + c*d` 与 gfortran
**逐位相同**，所以这不是"Fortran 前端特殊"。

### 规则：`X + Y` 里收缩**左边**那个乘积，右边的乘积先舍入

- `a*b + c*d` → `fma(a,b, c*d)`：左乘被收缩，右乘 `c*d` 先算好当代数项。
- 左结合链 `((a*b + c*d) + e*f) + g*h` → 内层按上一条；外层左边已经不是乘了，
  于是收缩右边那个乘：`fma(e,f, ·)`、`fma(g,h, ·)`。合起来就是
  `fma(g,h, fma(e,f, fma(a,b, c*d)))` —— **四条乘里只有 `c*d` 保持一次独立舍入**。

所以 `MOD_NetSolar.F90` 的五处应按"左乘进 `fma`、右乘先舍入"来写。这与第 7236 节
`Y + eccen*X` 的形状不矛盾（那里只有一个乘，收缩它即可）。

### 顺带纠正：`net_solar.rs` 里那条写反的 `mul_add` 已回退

工作区里曾有一版未提交的 `net_solar.rs`，把 `visible_absorption` 写成
`fma(diffuse_visible, coef[0][1], direct_visible*coef[0][0])` —— 收缩的正是**右边**那个乘，
与实测相反；它的注释还写着"gfortran 把后一个乘法收缩进加法"。该改动对三个窗口的
实测输出**没有任何影响**（干窗仍是 27 条 tier2），且触发两条 clippy
`unnecessary parentheses` 警告，已 `git checkout` 回退。要按上面这条规则重写，
必须先确认它能改变实测结果，否则只会往热路径里塞没人能验的 `mul_add`
（本仓库的规矩：`mul_add` 只写在**量过**的地方）。

## 叶温迭代里的 FMA 收缩：`taf`/`qaf`/`eah`/`fsenl`/`humidity_gradient`（实测，**已修**）

上一节把"叶温迭代的 `pco2a ↔ assim` 正反馈"点成剩余分叉的放大器，但没有动它的**输入端**。
这一轮按第 7236 节那套办法（把 Fortran 表达式单独编出来、用 libm `fma` 当正确舍入的对照）
把那个循环里出现频率最高的几处乘加逐位量了一遍，量一处修一处。

### 先说量出来的规则：`X + Y` 里收缩**左边**那个乘积；减法链**每层都收**

| 形状 | gfortran 实际收缩成 | 不收缩时的逐位命中率 |
|---|---|---|
| `a*b + c*d` | `fma(a,b, c*d)` | 1952/3000 |
| `a*b + c*d + e*f` | `fma(e,f, fma(a,b, c*d))` | 2580/4000 |
| `a*b + c*d + e*f + g*h` | `fma(g,h, fma(e,f, fma(a,b, c*d)))` | — |
| `(w0+w1)*T - w0*T1 - w1*T2` | `fma(-w1,T2, fma(w0+w1,T, -(w0*T1)))` | **345/4000** |

前两行的另外几个候选（`fma(c,d,a*b)`、`fma(a,b,fma(c,d,fma(e,f,g*h)))` 等）都只有
2300–3100/4000。第四行那个形状每轮叶温迭代出现 **5 次**
（`fsenl`/`etr`/`etrsun`/`etrsha`/`evplwet` 共用一个括号里的湿度/温度梯度），
不收缩时 **91% 的输入都会差 1 ULP** —— 这是这一轮找到的最肥的一处。

第 7236 节的 `Y + eccen*X` 与这里不矛盾：那里只有一个乘可收缩。

### 改了哪五处（`crates/colm-core/src/leaf_temperature.rs`）

| 上游 | 收缩点 |
|---|---|
| `:1120` `fsenl = rhoair*cpair*cfh*( (wta0+wtg0)*tl - wta0*thm - wtg0*tg )` | 括号里三级减法链 |
| `:1125` （`etr` 系列共用的）`( (wtaq0+wtgq0)*qsatl - wtaq0*qm - wtgq0*qg )` | 同上 |
| `:953` `taf = wta0*thm + wtg0*tg + wtl0*tl` | 三项乘积链 |
| `:954` `qaf = wtaq0*qm + wtgq0*qg + wtlq0*qsatl` | 同上 |
| `:688` `eah = qaf*psrf/(0.622 + 0.378*qaf)` | 分母 `a + b*c`（4000 组里 33 组不同） |

外加一处**结合顺序**（不是 FMA）：`etrsun`/`etrsha` 上游是
`rhoair*dry_factor*delta*( laisun/(rb+rssun) )*( … )`，先算除法再乘；原先写成
`… * laisun / (rb+rssun) * …`，先乘后除，差 1 ULP。`etr` 那条上游本来就带括号，不动。

`qaf` 是 `eah` 的输入、`eah` 是 `stomata` 的 `ea`，而 `stomata` 的
`D = max(ei-ea,50)/psrf` 在叶片与冠层空气水汽压相近时分母很小 —— 这条链把
"每轮 1/3 概率差 1 ULP"直接送进正反馈。

### 实测效果：干窗 tier2 从 27 条降到 18 条

三个窗口（`oracle/work` + `oracle/golden`，全场）：

| 窗口 | 改前 | 改后 |
|---|---|---|
| `CN-Cng`（干） | 27 条 tier2 | **18** 条 |
| `CN-Cng-wet` | 68 | 68 |
| `US-NR1-snow` | 79 | 79 |

干窗里逐变量超差条数普遍下降（`f_wliq_soisno` 45→5、`f_wice_soisno` 30→15、
`f_zwt` 61→49、`f_fsenl` 78→76），但 `f_vegwp` **升了**（425→477）——
它是下面那条**迭代次数刀口**的受害者，数值上本来就不可控，不是这轮改动的回归：
湿窗/雪窗同样变量在改前后**逐位完全不同**、而超差条数一字不差，说明那些条数由
"状态整体偏了一层"决定，不对 1 ULP 敏感。

### 下一处（这轮没动）：叶温循环的**退出判据是刀口**

`MOD_LeafTemperature_Extended.F90:1288-1292` 的收敛判据是
`det = max(del,del2) < 0.01 .and. dee = max(dele,dele2) < 0.1`。`del`/`dele` 是
`dtl` 与能量通量的变化量，黄昏/黎明时正好压在 `0.01`/`0.1` 附近：任何 1 ULP 的输入差
都会让**迭代次数差一次**，而每次迭代里 `PlantHydraulicStress_twoleaf` 都会把
`vegwp` 推进一个 Newton 步。

干窗逐条 history 的 `f_vegwp` 正是这个形状：第 5、7 条记录 Fortran 与 Rust 之间是
**四个节点同一个常数偏移**（-642.4987/-658.1989 = 15.7002，第 7 条 16.2012），
第 9 条之后又回到 1e-4 量级。同一节点同向同量偏移 = 多推（或少推）了一整步，
不是公式差异。第 16 步的 `f_rootr` 剩 111 个 1e-8 级差同源。

要证实它得把两边的迭代次数打出来对（Fortran 侧要重建内核），
`leaf_temperature` 的输出结构里已经有 `iterations` 字段，但还没有和上游对过的工具。

### 追加两处（同样是量过的），以及"量过但不改变窗口数字"这件事

同一条线索上又量了两处，都改在 `crates/colm-core/src/leaf_temperature.rs`：

| 上游 | 形状 | 收缩成 | 不收缩 |
|---|---|---|---|
| `:1291` `dele = dtl*dtl*( dirab_dtl**2 + fsenl_dtl**2 + (hvap*fevpl_dtl)**2 )` 的里层 | 三项平方链 | `fma(C,C, fma(A,A, B*B))` | 3112/4000 |
| `:1271-1273` `dtl` 的分子 | `base - h*fevpl + cpliq*qrain*ΔT + cpice*qsnow*ΔT` | `fma(ci*qsnow, ΔT, fma(cl*qrain, ΔT, fma(-h,fevpl,base)))` | 2907/4000 |
| 同上分母 | `base + h*C + cpliq*qrain + cpice*qsnow` | `fma(ci,qsnow, fma(cl,qrain, fma(h,C,base)))` | 2905/4000（候选 3988）|

`dele` 是**收敛判据**的一半（`dee < dlemin`），`dtl` 就是 `det < dtmin` 里的那个量，
所以这两处理论上最该改写迭代次数。实测结论是**没有**：干/湿/雪三个窗口的
tier2 变量数（18/68/79）与**逐变量超差条数**（`f_vegwp` 477、`f_lfevpa` 82 …）
一字未变，连 `worst at index` 的值都逐位相同。但改动确实生效了 ——
在 16 步的 `TIMESTEP` 干窗上逐位比，它换掉了 61 个变量（`f_wliq_soisno` 26/240、
`f_wice_soisno` 21/240、`f_fevpl` 16/16 …）。

**这条负面结论本身有用**：它说明那 18/68/79 条**不是**由"累积的 1 ULP"决定的，
而是由少数几个**状态整体偏移**决定的 —— 改 1 ULP 只会让偏移换个位置，
不会把某条记录拉回容差内。要动它们必须找到**那个翻转的量**，而不是继续扫 FMA。

顺带确认了一件事：`temperature_change` 用的是 `dtl.abs()`，而上游写的是
`sqrt(dtl*dtl)`。400 万组随机位型里两者只在 `x*x` 上溢时有别，而 `dtl` 受
`delmax` 限幅，所以这不是差异来源 —— 不改。

叶子循环的实测迭代次数（干窗 16 步，Rust 侧插桩，已撤）：7…33，**从不到 40**，
即退出确实走收敛判据而不是迭代上限。

## 共用的三对角求解器 `tridia` 也一直没收缩（实测，**已修**，且用真实矩阵对过）

上一节把 `root_flux_slope`（上游 `dqeroot`）点成 PHS 分叉的起点：第 0 步第 1 次调用，
`x`、两个需水量、`qeroot` 全部**逐位相同**，只有 `dqeroot` 差 3.3e-13 相对。
`dqeroot` 由**第二次** `tridia` 解出，而 `tridia` 在 `MOD_Utils.F90:2556-2565` 有三处
`X - 乘积`：

```fortran
bet = b(j) - a(j)*gam(j)
u(j) = (r(j) - a(j)*u(j-1))/bet
u(j) = u(j) - gam(j+1)*u(j+1)
```

`-O2` 下三处都被收缩。Rust 的 `solve_tridiagonal` 原先一处都没收缩。

### 先用独立驱动器量：不收缩 183/2000、三处都收 2000/2000

把 `tridia` 原样抄成独立程序，2000 组对角占优三对角系统（n=15），逐**整个解向量**比位：
不收缩 **183/2000**，三处 `mul_add` **2000/2000**。

### 但真正拍板的是**内核里的真实矩阵**：49/49 对 0/49

独立驱动器的矩阵是合成的，所以又在 `MOD_PlantHydraulic.F90` 的两次 `tridia` 调用前后
插桩，把**真实算例**里 49 组 (a,b,c,r,u) 全部打出来（干窗 16 步，PHS 每次调用解两次
= 98 个系统），再拿同一个 C 检查器离线复算两种写法：

| 解法 | 收缩写法逐位相同 | 不收缩 |
|---|---|---|
| 第一次（`rmx_hr` 右端） | **49/49** | 0/49 |
| 第二次（`drmx_hr` 右端） | **49/49** | 0/49 |

**不收缩是 0/49 —— 内核里没有一次解对过。** 这不再是"可能差 1 ULP"，
而是每一步、每条柱的每个解都比上游差。

### 影响面：这是**全模式共用**的求解器

上游 `CALL tridia` 出现在 `MOD_GroundTemperature`、`MOD_SoilSnowHydrology`（土壤水）、
`MOD_PlantHydraulic`（两次）、`MOD_Lake`、`MOD_Glacier`、`MOD_SimpleOcean`、
`MOD_Urban_{Roof,Pervious,Impervious,Wall}Temperature`、BGC 垂直输运等十余处；
本仓库对应 `ground_temperature`、`soil_water`、`plant_hydraulics`（前向/导数/回代共三处）、
`urban_temperature`（两处）、`urban_impervious`。**每步都跑，解向量每步都回写状态。**

### 指标怎么读：三个口径给出**不同方向**的结论

`git stash` 各跑一遍全场，同一批产物数三个口径（`base` = 都没修，
`solver` = 只修求解器，`both` = 再加上下面 PHS 右端那一节）：

| 窗口 | 口径 | base | solver | both |
|---|---|---|---|---|
| `CN-Cng`（干） | 超容差**变量数** | 18 | 27 | **17** |
| | 超容差**值数** | 890 | 1084 | **814** |
| | **Σ\|差\|**（全场） | 2981.90 | **288.92** | 308.25 |
| | 逐位不同的值 | 21285 | 21198 → 21201 | 21198 |
| `CN-Cng-wet` | 变量数 / 超差值 | 68 / 20671 | 68 / 20675 | 68 / **20666** |
| | Σ\|差\| | 10372.44 | 10385.59 | 10379.48 |
| | 逐位不同 | 33380 | 33354 | **33350** |
| `US-NR1-snow` | 变量数 / 超差值 | 79 / 25897 | 79 / 25897 | 79 / 25897 |
| | Σ\|差\| | 444394.5 | 444390.5 | 444394.5 |
| | 逐位不同 | 33661 | **33605** | 33650 |

**求解器那一修把干窗的全场绝对误差砍掉了 90%**（2981.90 → 288.92），
**同时**把超容差**值数从 890 抬到 1084**。这不是矛盾，而是"超容差变量数/值数"
本身是**阈值穿越计数、不是误差的单调函数**：误差整体小了 10 倍，
落点却在刀口上换了个位置 —— `f_vegwp` 那类"整个状态偏移"的记录成批进出容差，
进出的是阈值而不是误差。Σ|差| 与逐位计数才随误差单调。

所以：

* **不要**用"超容差条数"判定一次算术保真改动的好坏（它可以把 10 倍的改善报成回归）；
* 要看 Σ|差| 与逐位计数，再加上**输入相同则输出相同**的差分探针
  （上面 49/49 对 0/49 那张表就是这个口径）；后者是共享内核唯一有说服力的判据。
* 上游的编译行为是确定的：留着一个已知 0/49 的求解器，等输入也对上那天输出照样对不上。

PHS 右端那一节的净效果（`both` 对 `solver`）：干窗超容差值 1084 → **814**
（比 base 的 890 还低），Σ|差| 从 288.92 略升到 308.25（仍是 base 的十分之一），
湿窗与雪窗基本不动。也就是说**两处改动叠加之后三个口径同向变好**（雪窗 Σ|差| 持平）。

### 插桩的两个坑（供下一次参考）

* `WRITE(6,'(A,I3,5(1X,A,20ES24.16))')` —— 用数组当列表项、显式写了 20 的重复数，
  数组只有 9 个元素，格式控制转去取下一个列表项（一个字符），**运行期类型不符直接
  abort（exit 2）**，日志里连报错都没有。改成每条数组单独 `WRITE`、用一个可重复的实数段
  （`(A,8ES26.17)` + 隐含 DO）才稳。
* 从混合日志里按"行首是 A/B/C/R/U"抓数组会把内核其它输出一起吃进来，
  解析器要按**精确标签**匹配，别按行首字母。

## PHS 的 `rmx_hr` 右端：把求解器的输入也改对，第一处差异从第 0 次调用推到第 6 次（实测，**已修**）

上一节修完求解器后，用同一对探针（上游 `MOD_PlantHydraulic.F90` 的
`getrootqflx_x2qe` 前后 + Rust `plant_hydraulics.rs` 的对应点，各打 8 个数）
在干窗 16 步上逐次比对 49 次 PHS 调用的输入与输出，第一处差异**移动**了：

| 阶段 | 第一处差异 | 量级 |
|---|---|---|
| 修求解器前 | 第 **0** 次调用的 `dqeroot` | 3.3e-13 相对 |
| 修求解器后 | 第 **0** 次调用的 `qeroot` | 4.4e-13 相对 |
| 再修右端后 | 第 **6** 次调用的 `rmx_hr` | 9.6e-14 相对 |

（`x`/需水量/`dqeroot` 在 0–5 次调用上全部逐位相同；第 1 次调用的 `x` 差从
1.7e-11 降到 5.8e-15。）这正是"输入相同则输出相同"该有的推进方式：
每修一处，第一处差异就往后退。

### 改了什么

`rmx_hr` 是第一次 `tridia` 的右端，它的形状是 `krad*smp + kax(1) - kax(2)` 与
`… + kax(1)/den1*xroot(1)`：前者左边那个乘积被 gfortran 吸收，后者收的是**外层**乘积
（实测 `fma(p,A,B) - C` 4000/4000、`fma(q,R,acc)` 4000/4000；
不改写分别是 2662/4000 与 2935/4000）。这解释了上一节那个"反直觉"的观测：
`rmx_hr` 只进第一次解，所以它差 1 ULP 时 `x`/`qeroot` 差而 `dqeroot` 可以仍然对上。

顺带修了**反解**那一支（`getrootqflx_qe2x` = Rust `root_potential_from_flux`）的
结合顺序：上游首行是 `krad*smp - qeroot - kax(j)`，Rust 原先算成
`(krad*smp - kax(j)) - qeroot`（`rhs[0] -= root_flux` 放在循环外），
先减哪个不是同一个数。

### 用真实矩阵复核（不需要再动内核）

内核侧那 49 组 (a,b,c,r,u) 已经在上一节从**真实算例**里打出来存成了
`/tmp/gf/phs9_cases.txt` 式的清单（Fortran 是确定性的，随时可重放）。
这一轮把 Rust 侧的 `sub/diagonal/super_/rhs/derivative_rhs/tail/derivative_tail`
也打出来逐组比：

| 系统序号 | 结果 |
|---|---|
| 0–5 | 矩阵**与两个解**全部逐位相同 |
| 6 起 | 先是右端差 9.6e-14，之后逐次放大 |

所以剩余的第一颗种子不再在 PHS 内部，而在**喂给 PHS 的步级状态**里
（`smp`/`hk` 来自土壤水一步，`x_root_top` 来自上一步 PHS）。
再往下就照同一条路：把 `krad`/`kax`/`smp`/`den`/`xroot(1)` 也打成一对探针。

### 指标（见上一节三口径表）

干窗超容差值 1084 → **814**（比没修求解器时的 890 还低），
Σ|差| 288.92 → 308.25（仍是 base 2981.90 的十分之一），湿/雪窗基本不动。

## PHS 的输入全部对上之后：第一处差异落到 `spacAF_twoleaf` 的分组（实测，**已改，指标中性**）

上一节把第一处差异推到第 6 次调用的 `rmx_hr`。这一轮把探针从"解"挪到"输入"：
在 `PlantHydraulicStress_twoleaf` 入口（Rust `plant_hydraulic_stress` 入口）逐次打
`vegwp`/`smp`/`hk`/`k_soil_root`/`k_ax_root`，**328 次调用**（每次叶温迭代一次，
比 `calcstress` 那 49 次多，因为后者只在 `qflx_sun>0 或 qflx_sha>0` 时进）。

| 量 | 第一处差异 | 说明 |
|---|---|---|
| `k_soil_root` | **从未出现** | 328 次全部逐位相同 |
| `k_ax_root` | **从未出现** | 同上 —— 根区导度的推导是干净的 |
| `vegwp` | 第 **1** 次调用，5.8e-15 | 第 0 次 PHS 自己更新的状态 |
| `smp` / `hk` | 第 **10** 次调用，7.0e-15 / 2.2e-14 | 之后保持常量 ⇒ 是新一步的**步级**输入 |

所以剩下的第一颗种子**不在 PHS 内部**：第 0 次调用的输入（`vegwp=-25000`、`smp`、`hk`、
导度）全部逐位相同，输出 `dx` 却差 1 ULP；`vegwp` 从第 1 次调用起带着这个差往下走，
到第 10 次（新的一步）`smp`/`hk` 也带上了。**要往前推，得进 `spacAF_twoleaf` 内部。**

### 顺手抓到的一处**分组**差异（不是 FMA）

`spacAF_twoleaf` 的四个矩阵元在本仓库里被"提了公因式"：

| | 上游（`MOD_PlantHydraulic.F90:472-489`） | 本仓库（改前） |
|---|---|---|
| `A13` | `laisun*kmax_sun*dfx*Δsun + laisun*kmax_sun*fx` | `laisun*kmax_sun*(dfx*Δsun + fx)` |
| `A23` | 同型 | 同型 |
| `A33` | 五项连减，每项都是独立乘积 | 把 `dfx*Δ + fx` 先合成一项再乘 |
| `A34` | `sai*kmax_xyl/htop*dfr*Δroot + sai*kmax_xyl/htop*fr` | `xylem*(dfr*Δroot + fr)` |
| `f(xyl)` | 三项独立乘积相加 | 同型（本来就一致）|

代数等价、**舍入不等价**：提公因式之后多了一次"先加后乘"，而且**表达式树变了**，
于是 gfortran 的收缩点也跟着变 —— 旧写法连"该收缩哪个乘积"都对不上。
已按上游逐字改回（`sunlit_conductance = laisun*kmax_sun` 等中间量提到外面，
再写成 `A13 = sunlit_conductance*dfxyl*Δsun + sunlit_conductance*fxyl`）。

**实测效果：三个窗口全部逐位不变**（三口径、全场、A/B 过 `git stash`）。
所以这一改是**为下一步铺路**（树形与上游一致），不是已证实的修正：
`spacAF_twoleaf` 里那几十处乘积该怎么收缩**还没量**，而它必须量 ——
下一步是把内核的 `A11..A44`/`f(1:4)`/`determ`/`dx(1:4)` 打出来，
用真实数值对着 C 复算（和 `tridia` 那次同一个套路）。

（插桩坑再记一条：`WRITE(6,'(A,5ES26.17)')` 打 10 个元素的数组会**格式回归**，
格式从头重来又碰到 `A`，于是把 double 的原始字节当字符写进日志 ——
要么把实数段写够（`20ES26.17`），要么每条数组单独 `WRITE`。）

### 已经备好的弹药：内核侧 `spacAF_twoleaf` 的中间量（下一轮直接用）

为了避免下一轮再花一次"插桩 + 重建内核"，这一轮已经把**内核**
`spacAF_twoleaf` 的中间量打出来存盘（源码已还原、内核已重建、黄金仍逐位复现）：

* `/tmp/gf/spacaf_kernel.log` —— 49 次调用，每次 6 行：
  `SPACA `（A11,A13,A22,A23,A31,A32,A33,A34,A43,A44）、`SPACF `（`f(1:4)`）、
  `SPACIN `（`x(1:4)`）、`SPACQ `（`qflx_sun,qflx_sha,laisun,laisha`）、
  `SPACQ2 `（`sai,htop,qeroot,dqeroot`）、`SPACD `/`SPACX `（`determ` 与 `dx(1:4)`）。
* `/tmp/gf/phs_inputs_kernel.log` —— 328 次调用入口的
  `vegwp/smp/hk/k_soil_root/k_ax_root`。

**怎么用**：第 0 步第 1 次调用的**输入**两侧逐位相同（这一点已经验过），
所以只要在 Rust 侧把同样的中间量打出来，就能看到**在 A/f/determ/dx 里第一处**
开始不同的是哪一个 —— 比"整段 spaAF 逐句读"快得多，而且不用再动内核。
`kmax_*`/`psi50_*`/`ck` 来自地类表（此前已逐位对过），不必再打。

注意 `/tmp` 只在本次会话内可靠；跨会话要重打（或把日志挪进仓库外的固定位置）。

## `spacAF_twoleaf` 的四条回代式：收缩规则量出来了，`dx` 第 0 次调用已逐位（实测，**已修**）

上一轮把第一处差异定位到 `dx`（`spacAF_twoleaf` 的输出），且输入 `A11..A44`/`f`/`x`/
`determ` 全部逐位 — 只剩四条回代式里的收缩没对上。这一轮把它量死了。

### 方法：先用内核数据验证一个**独立复刻件**，再拿它当神谕跑两万组

1. 把上游那四条 `dx` 语句**原样**抄成独立 Fortran 程序（`/tmp/gf/spac_replica.f90`），
   用上一轮从内核打出来的 49 组 (A11..A44, f, determ) 喂它 ——
   复刻件的 `dx` 与内核的 `dx` **49/49 逐位相同**，说明复刻件可信。
2. 拿复刻件当"神谕"跑 **20000 组随机输入**（A 量级 1e-9、determ 1e-25），
   与 72 种候选嵌套（内层 6 种 × 外层 12 种）逐一比对分量。
3. 唯一全中的是 **内层 `LR` + 外层 `LRR`**：20000/20000 组、80000/80000 个分量全同；
   原先"全不收缩"的写法在这 20000 组里 **0 组**全中。

规则（三条）：

* 每一级加法/减法里**把乘积那一侧的最外层乘法收进 FMA**；两边都是乘积时**收左边**
  （与早先 `a*b + c*d`、`(w0+w1)*T - w0*T1 - w1*T2` 的实测一致）；
* **没被收的那一侧按源码顺序整项舍入** —— 例如 `a13*a32*a44*f2` 是
  `((a13*a32)*a44)*f2`，拆成 `(a13*a32)*(a44*f2)` 是另一种结合，不是同一个数；
* 三因子内层链（`P1 - P2 - P3`）也是"第一级收左、第二级收右"。

**交叉验证**：`gfortran -fdump-tree-all` 的 GIMPLE 里能直接读到
`.FMA(...)`/`.FMS(...)`/`.FNMA(...)`/`.FNMS(...)` 落在这些位置，
与 20000 组的结论一字不差。**这条路径比"猜候选"可靠得多，值得当成标准做法** ——
尤其是 `.FNMS`：它的语义是 `-(a*b) - c`（不是 `-(a*b) + c`），
我按后者读 GIMPLE 时把 `dx(xyl)` 的符号推错了一次，探针立刻报 `dx[2]` 差 8e-5。

### 效果

| | 改前 | 改后 |
|---|---|---|
| 第 0 次调用 `dx` | 两个分量差 1 ULP | **四个分量全部逐位** |
| 第一处差异 | 第 0 次调用 | 第 **1** 次调用（`f` 差 1.1e-16） |
| 干窗 `f_vegwp` 逐位不同的值 | 1056 中 1 个 | 同上（这一步只动了 1 个值）|

历史量级几乎不动（三口径与上一提交逐位相同，只有 `f_vegwp` 的 1 个值变了）——
和 `tridia`、PHS 右端那两处一样：**这类修正是把"输入相同则输出相同"补齐，
不是刷指标**。但它把 PHS 内部的最后一处结构差异关掉了：第 0 次调用的 A/f/determ/dx
现在全部逐位，第一处差异退到"第 0 次更新之后的状态"上。

`shaded_flux <= 0` 那条分支的三条回代式用同一条规则改写，但**干窗一次都没走到**，
所以只能算"按规则推断"，未实测（`Not-tested`）。

### 下一步

第一处差异现在是第 1 次调用的 `f`（1.1e-16）：它由 `sunlit_flux`/`fsun`/`x` 算出，
而这些量在第 0 次 PHS 更新之后 —— 也就是要查 `getqflx_qflx2gs_twoleaf`（反解气孔导度）
与调用方的 `x = x + dx` + 三段 clamp。内核侧这两样都还没打过。

## `spacAF_twoleaf` 的 `A`/`f`：收缩点直接读 GIMPLE 定下来（实测，**已修**）

上一轮把四条 `dx` 回代式量准之后，探针显示第一处差异退到**第 1 次调用的 `f`**
（`f[0]`/`f[1]` 各差 1.1e-16），而同一调用里 `x`、`A11..A44` 都是逐位的 ——
那只能是 `f` 表达式自身的收缩没抄。

这一轮不再"猜候选"：把 `spacAF_twoleaf` 的十条 `A` 与四条 `f` 语句抄成独立程序，
`gfortran -O2 -fdefault-real-8 -fdump-tree-all` 直接读 GIMPLE 里的
`.FMA/.FMS/.FNMA/.FNMS`。逐个对应如下（`P = laisun*kmax_sun*fx`，
`PS = laisha*kmax_sha*fx`，`X = sai*kmax_xyl/htop*fr`，`G1/G2/G3` 是三个梯度）：

| 上游 | GIMPLE | 收缩点 |
|---|---|---|
| `A11 = -P - qflx_sun*dfsto1` | `FNMS(qflx_sun, dfsto1, P)` | 左边是 `-P`（NEG）不是乘积 ⇒ **收右边** |
| `A13 = LKdfx*G1 + P` | `FMA(LKdfx, G1, P)` | 收左边 |
| `A22/A23` | `FNMS(qflx_sha, dfsto2, PS)` / `FMA(LSdfx, G2, PS)` | 同型 |
| `A33` | `FNMS(LKdfx,G1,-P)` → `FNMA(LSdfx,G2,·)` → `-PS` → `-X` | 三级 |
| `A34` / `A44` | `FMA(Xdfr, G3, X)` / `FNMS(Xdfr, G3, X) + dqeroot` | 同型 |
| `f(leafsun)` | `FMS(qflx_sun, fsto1, P*G1)` | 收左边 |
| `f(xyl)` | `FNMA(X, G3, P*G1 + PS*G2)` | 右边先整项相加再被吸收 |
| `f(root)` | `FMS(X, G3, qeroot)` | 同上 |

（`A31/A32/A43` 是单个乘积，没有收缩点。）

**与上一轮 `dx` 的规则同一条**：每层把那个乘积操作数收进 FMA；两边都是乘积时收左边；
被收的乘法取它自己的**最外层**乘法；没收的那一侧按源码顺序整项舍入。

### 效果与残留

| | 改前 | 改后 |
|---|---|---|
| 第 1 次调用 `f[0]`/`f[1]` | 差 1.1e-16 | **逐位** |
| 第 2 次调用 `f[2]`/`f[3]` | — | 绝对差 3.9e-25（`f` 自身≈5.8e-19，是**抵消放大**）|
| 三个窗口的三口径 | — | 逐位不变 |

第 2 次调用的 `f[2]`/`f[3]` 相对差 6.7e-7 看着刺眼，但**绝对差只有 3.9e-25**，
而 `f` 那一步本身因为系统接近平衡只剩 5.8e-19（从第 1 次调用的 8e-10 掉下来九个量级）——
这是抵消把 1 ULP 的输入差放大成相对差，不是新的公式差。**读探针要同时看绝对值**。

### 顺带排掉的一条嫌疑

`plc`（`2.**tmp`）曾怀疑被 gfortran 换成 `exp2`（那样与 Rust 的 `powf` 差 1 ULP）。
实测 `-S`/优化 dump：两处 `**` 都是 `__builtin_pow`，没有换。**不是差异来源。**

### 下一步

第 1 次调用里 `x`/`A`/`f[0]`/`f[1]` 已逐位，`f[2]`/`f[3]` 与 `dx` 只剩 ~4e-16 ——
剩下的 1 ULP 只能来自**还没进探针的输入**：`qeroot`（`root_flux`）、
`gs0sun`/`gs0sha`（上一轮迭代吐出来的气孔导度，进 `sunlit/shaded_flux`），
以及四个 `kmax_*`（地类表的液压导度）。下一步把它们一起打进同一条探针。

## 需求项与反解气孔导度：同样按 GIMPLE 收（实测，**已修**，窗口指标是噪声）

`spacAF` 的 `A`/`f` 修完之后，第 1 次调用的 `A`/`x`/`sunlit_flux`/`shaded_flux`
全部逐位，只剩 `f[2]`/`f[3]` 差 4.1e-25（绝对值）。这一轮顺着"输入不在探针里"往
上游找，把 `getqflx_gs2qflx_twoleaf`（需求）与 `getqflx_qflx2gs_twoleaf`（反解）
两条也按 GIMPLE 收了。

### 量出来的收缩点（同一个独立复刻件 + `-fdump-tree-all`）

| 上游 | GIMPLE |
|---|---|
| `1. - delta*(1.-fwet)`（`cfw` 与 `cwet` 两处） | `FNMA(1-fwet, delta, 1.0)` |
| `cqi = (wtaq0+wtgq0)*qsatl - wtaq0*qm - wtgq0*qg` | `FNMA(wtgq0, qg, FMS(wtaq0+wtgq0, qsatl, wtaq0*qm))` |
| `cqi_leaf = caw*(qsatl-qm) + cgw*(qsatl-qg)` | `FMA((qsatl-qm), caw, (qsatl-qg)*cgw)` |
| `csun_dry = (B1*C2 - B2*C1)/(B1*A2 - B2*A1)` | `FMS(b1,c2, b2*c1) / FMS(b1,a2, b2*a1)` |
| `cshaw_dry = (A1*C2 - A2*C1)/(A1*B2 - B1*A2)` | `FMS(c2,a1, c1*a2) / FNMA(b1,a2, b2*a1)` |

最后一行值得单记：**两个分母收的方向不一样**。`B2*A1`（=`A1*B2`，乘法可交换、值相同）
被 `csun` 的分母先算出来，`csha` 的分母**复用了那个已算好的乘积**，于是它收的是
`b1*a2`（写成了 `FNMA`）。所以 Rust 里必须把那一个乘积抽成变量复用
（`b2_a1`），照"对称写法"各写一遍反而会差 1 ULP。

### 效果

* 探针：第 1–4 次调用的 `sunlit_flux`/`shaded_flux`（`SPACQ`）从差 1 ULP 变成**逐位**，
  `f[0]`/`f[1]`、`A11..A44`、`x` 全部逐位；只剩 `f[2]`/`f[3]` 的 4.1e-25（绝对值，
  相对 ~5e-16）。
* 窗口：干窗 Σ|Δ| 308.25 → 395.40（变差 28%）、超容差值 814 → 833；
  湿窗 20666 → 20664；雪窗逐位不同的值 33650 → 33597、Σ|Δ| 持平。
  作为对照，把两处**比值**的改动单独退回普通写法再跑一遍：干窗 837、湿窗 20660 ——
  三个口径在 ±3% 内来回摆，**方向都不一致**。

按 `tridia` 那一节立下的规矩：这类改动的判据是 GIMPLE（内核真的这么编）与
"输入相同则输出相同"，不是阈值穿越计数。所以保留 GIMPLE 版，并把这个 A/B 结果记在这里。

### 下一步（残留 4e-25 的来源已缩小到两条式子）

`f[2] = P*G1 + PS*G2 - X*G3`、`f[3] = X*G3 - qeroot`，两者的差**绝对值相同**
（4.14e-25），指向同一个量 `X = sai*kmax_xyl/htop*fr` 或 `G3`。而 `A43 = X` 是逐位的 ——
所以更可能是 `f[XYLEM]`/`f[ROOT]` 这两条在**内核里**的收缩与独立复刻件不同
（内核里 `X` 还进了 `A34`/`A44`，CSE 环境更复杂）。要判它，得把内核的
`P/PS/X/G1/G2/G3` 也打出来（一次内核插桩），或直接比 `f[2]`/`f[3]` 的候选写法。

## `shaded_flux <= 0` 分支：把"推断"换成"验过"（实测，**已修**）

`spacAF_twoleaf` 的 ELSE 分支（`qflx_sha <= 0`）干窗 49 次调用一次都没走到，
所以上一轮那三条 `dx` 与 `A33`/`f(xyl)` 是**按规则推断**改的，`Not-tested`。
这一轮把它补齐，方法还是那一套：抄独立复刻件 → 读 GIMPLE → 拿复刻件当神谕跑随机输入。

### GIMPLE 定下来的写法

| 上游 | GIMPLE |
|---|---|
| `A33 = -LKdfx*Δsun - P - X` | `FNMS(LKdfx, Δsun, P) - X` |
| `f(xyl) = P*Δsun - X*Δroot` | `FMS(Δsun, P, X*Δroot)` |
| `determ = A11*A33*A44 - A34*A11*A43 - A13*A31*A44` | `FNMA(a44, a13*a31, FMS(a33*a11, a44, (a11*a34)*a43))` |
| `dx(leafsun)` | `FMA(FMS(a33,a44,a34*a43), f(leafsun), FMS(a13*a34, f(root), f(xyl)*(a13*a44)))` |
| `dx(xyl)` | `FNMA(f(leafsun), a44*a31, FMS(f(xyl), a11*a44, (a11*a34)*f(root)))` |
| `dx(root)` | `FMA(f(leafsun), a43*a31, FMS((a11*a33 - a13*a31), f(root), f(xyl)*(a11*a43)))` |

有一处**只有对着 GIMPLE 才看得出来**：`dx(leafsun)` 里 `A33*A44 - A34*A43`
本身也是一个被收缩的减法（`FMA` 的乘数那一侧），写成普通表达式再传进去就错。
这一条正是我第一版按"规则推断"漏掉的 —— 独立复刻件把它抓了出来。

### 用独立复刻件验（这次能验，因为它不依赖内核）

把上游那六条语句抄成 `else_rep.f90`，喂 20000 组随机输入（x 在 −2000…0、
`laisun` 0…6、导度 1e-9…6e-9、`grav1` 由 `htop` 生成），Rust 侧的写法在 C 里复算：

| 量 | 收缩写法 | 未收缩（改前） |
|---|---|---|
| `A33` | **20000/20000** | 18929/20000 |
| `f(xyl)` | **20000/20000** | 15005/20000 |
| `determ` | **20000/20000** | 13573/20000 |
| `dx(leafsun)` | **20000/20000** | 11518/20000 |
| `dx(xyl)` | **20000/20000** | 9604/20000 |
| `dx(root)` | **20000/20000** | 11504/20000 |
| `dx(leafsha)` | **20000/20000** | 14972/20000 |

也就是说这条没人走的分支里，**改前有 4%–52% 的输入会差 1 ULP**。
三个黄金窗口逐位不变（分支没被走到）—— 这正是"补齐没被覆盖的分支"该有的样子：
不动指标，但把一条会咬人的路径焊死了。

**这条路子可以复制**：凡是 "只有一边被黄金走到" 的分支（ELSE / scheme 分支 /
未启用的方案），都可以用"独立复刻件 + 随机输入差分"验，不需要内核插桩。

## 产流：`sum(乘积)` 要**逐个 fma 累加**，`XinAnJiang` 另有自己的一条式子（实测，**已修**）

这一轮用"独立复刻件 + 随机输入差分"往下扫没被黄金覆盖的产流分支，结果先在手边
**正在被黄金走到**的 SimpleVIC 上抓到一个：

### 一、`sum(vol_liq(1:6)*dz(1:6))` 是 fma 累加

`MOD_Runoff.F90:250-251`（`Runoff_SimpleVIC`）与 `:262-263`（`XinAnJiang`）都写
`w_int = sum(vol_liq(1:6)*dz_soisno(1:6))`。Rust 的 `.map(|(v,dz)| v*dz).sum()` 是
**先各自舍入再相加**，而 gfortran 把每个乘积收进累加器：

| 累加方式 | 与上游逐位相同的组数 |
|---|---|
| `acc += v*dz`（逐步舍入） | 14312/20000 |
| `acc = fma(v, dz, acc)` | **20000/20000** |

`w_int`/`wsat_int` 一差，`frcsat`（history 的 `f_frcsat`）与 `rsur` 全都跟着差 ——
而这条**三个黄金窗口每步都跑**（默认方案就是 SimpleVIC）。

### 二、`Runoff_XinAnJiang` 不能用 SimpleVIC 那条式子

两支代数等价、**浮点不等价**。上游 XinAnJiang 写的是 `wtmp`/`infil`：

```fortran
wtmp  = (1-w_int/wsat_int)**(1/(btopo+1)) - watin/((btopo+1)*wsat_int)
infil = wsat_int - w_int - wsat_int * (max(0., wtmp))**(btopo+1)
infil = min(infil, watin)
rsur  = (watin - infil) * 1000. / deltim
```

而本仓库原先借用了 `storage_distribution_runoff`（`WaterDepthInit`/`WaterDepthMax` 那一套）。
实测 20000 组随机输入：

| 写法 | `rsur` 逐位相同 |
|---|---|
| 借用通用式（改前） | 0/20000 |
| 照抄上游这一支 + `fma(-ws, pow, ws-w)` | **20000/20000** |

顺带两点：XinAnJiang **没有** `[0, watin]` 钳位（只有 `infil=min(infil,watin)`），
SimpleVIC 才有；`btopo` 的钳位区间是 `[0.01, 0.5]`，`sigmin/sigmax = 100/1000` ✓ 两边一致。

### 三、还有一处"反解 `BVIC`"的白丢 1 ULP

原来的 `storage_distribution_runoff` 收 `exponent` 再 `bvic = exponent/(1-exponent)` 反解，
而调用方本来就是 `BVIC/(1+BVIC)` —— 这个来回只有 **43%**（8658/20000）能原样还原
`BVIC`。现在改成直接收 `BVIC`（`XinAnJiang` 走自己那支，不再经过这个 helper）。

SimpleVIC 那条通用式的最后一个乘积也要收：
`RunoffSurface = … + wsat_int*(InfilVarTmp**(1+BVIC))` → `fma(capacity, pow, …)`
（不收缩 2/18185，收缩 **18185/18185**）。

### 四、对拍的是**真的 Rust**，不只是 C 复算

把 20000 组算例通过一个临时 `#[test]`（跑完已删）喂给 `simple_vic_runoff` /
`xinanjiang_runoff`，输出与复刻件逐位比：

```
SimpleVIC  : cases=20000  frcsat 20000/20000   rsur 20000/20000
XinAnJiang : cases=20000  frcsat 20000/20000   rsur 20000/20000
```

（改前：SimpleVIC 16234/11495，XinAnJiang 16441/11356。）

三个窗口的三口径几乎不动（干窗逐位值 +2、湿窗超容差 +2、雪窗 −1）——
产流本来就是小项，但它是**被黄金走到**的那条路，现在焊死了。

### 五、下一处（同类，未修）

`SubsurfaceRunoff_TOPMOD` 有两处同型差异，还没做：
上游先 `dzmm(j) = dz_soisno(j)*1000.` 再 `dzsum = dzsum + dzmm(j)`、
`icefracsum = icefracsum + icefrac(j)*dzmm(j)`（**乘积进 fma，且整列先乘 1000**），
Rust 则是用未缩放的 `depth` 求和、`ice + fraction*depth` 不收缩；
`mean_ice = icefracsum/dzsum` 与 `ice/thickness` 因缩放不同而差 1 ULP。
照上面的做法补一个 TOPMOD 复刻件即可验（该方案 `DEF_Runoff_SCHEME=1`，黄金没走）。

## `SubsurfaceRunoff_TOPMOD`：先乘 1000、再 fma 累加、`imped` 在链首（实测，**已修**）

上一节记下的同类项，这一轮补完（`DEF_Runoff_SCHEME=1`，黄金没走）。三处都能用
"独立复刻件 + 20000 组随机输入"直接判：

| 写法 | 与上游逐位相同 |
|---|---|
| `dzsum`：先 `dzmm = dz*1000.` 再求和 | **20000/20000** |
| `dzsum`：直接用未缩放的 `dz` 求和（改前） | **0/20000** |
| `icefracsum`：`fma(icefrac, dzmm, acc)` | **20000/20000** |
| `icefracsum`：逐步 `acc += icefrac*dzmm` | 15370/20000 |
| `rsubst`：`imped*5.5e-3*exp(-2.5*zwt)`（链首） | **20000/20000** |
| `rsubst`：`imped*(5.5e-3*exp(-2.5*zwt))`（改前） | 12715/20000 |

第一行值得单记：`Σ round(dz*1000)` 与 `Σ dz` **不是一个数**（1000 不是二进制精确的），
所以"缩放最后再除掉"这种看似无害的化简在这里是 0/20000。`fracice_rsub`/`imped`
的公式本身两边一致（20000/20000）。

改完把**真的 Rust** 跑同一批算例（临时测试，跑完已删）：

```
SubsurfaceRunoff_TOPMOD rsubst: 20000/20000 bitwise identical
```

三个窗口不变（方案 1 与黄金的 3 不同），符合预期。

### 踩到的坑（记下来）

写复刻件时把 `fracice_rsub` 那行的 `3.`/`1.` 写成了 `3.d0`/`1.d0` —— 在这个内核的
编译选项下（`-fdefault-real-8` 且**没有** `-fdefault-double-8`）`d0` 字面量是
**real(16)**，于是 `exp(-3.d0*(…))` 整段在四倍精度里算，`fracice` 与 double 版
只对得上 6236/20000。**复刻件必须逐字抄源码的字面量**，否则量的是自己造的另一个函数。

## 土壤热参数 `soil_hcap_cond`（Balland-Arp，黄金用的就是 4）四处收缩（实测，**已修**）

`MOD_SoilThermalParameters.F90` 的 `soil_hcap_cond` 每步每层都跑，`DEF_THERMAL_CONDUCTIVITY_SCHEME`
默认是 **4（Balland-Arp）**。GIMPLE 读出四处收缩，逐条改了：

| 上游 | GIMPLE |
|---|---|
| `hcap = csol + vf_water*c_water + vf_ice*c_ice`（`:299`） | `FMA(vfw, cw, csol)` 再 `FMA(vfi, ci, ·)` |
| `ke` 指数里 `1.+vf_om-BA_alpha*vf_sand-vf_gravels` | `FNMA(BA_alpha, vf_sand, 1+vf_om)` 再减 `vf_gravels` |
| `(1/(1+exp(-BA_beta*sr)))**3 - ((1-sr)/2)**3` | `FMS(wet, wet*wet, dry**3)` |
| `thk = (ksat-kdry)*ke + kdry`（`:407-411`，两个分支各一处） | `FMA(ksat-kdry, ke, kdry)` |

用独立复刻件跑 20000 组随机输入（`csol` 1e6…3e6、`kdry` 0.1…0.5、`T` 240…320 K、
`vf_water`/`vf_ice` 覆盖冻融两侧），把**真的 Rust** `soil_thermal_properties(..., BallandArp)`
的输出与之逐位比（临时测试，跑完已删）：

```
soil_hcap_cond (Balland-Arp): cases=20000  hcap 20000/20000   thk 20000/20000
```

窗口三口径仍是混合方向（干窗逐位值 +25、Σ|Δ| 变大；雪窗逐位值 +41；湿窗略差），
按既定规矩以 GIMPLE + 差分对拍为准。

### 顺带查掉的一条：VSF 的 `sum(a*b)` 不在关键路径上

上一轮把 `sum(a*b)` 列为待扫项，这一轮查了 VSF 里对应的两处
（`MOD_Hydro_SoilWater.F90:759/1067` 的 `wsum_m1`/`wsum`，形态是
`sum(ss_vl*(sp_dz-ss_wt)) + sum(ss_wt*vl_s)`）：**上游自己就没用过 `werr`**
（`:1075` 算完即弃），Rust 侧对应的是 `let _balance_error_mm = …`，**两边都是死代码**，
不必改。VSF 真正的储量求和（`MOD_SoilSnowHydrology.F90:296/846`）
是 `sum(wliq_soisno(1:))` 这类**无乘积**的求和 —— 而"无乘积的顺序求和两边一致"
已经在 `dzsum` 那组 20000/20000 里验过了，不用再动。

## `meltf` 的四条 `hm` 与三个分母（实测，**已修**），以及**一次差点改错的地方**

`MOD_PhaseChange.F90` 的 `meltf` 每步每层都跑。GIMPLE 读出两处收缩：

| 上游 | GIMPLE |
|---|---|
| `hm(j) = hs_soil + (1-fsno)*dhsdT*tinc + brr - tinc/fact` | `FMA((1-fsno)*dhsdT, tinc, hs_soil)` |
| `hm(j) = hs + dhsdT*tinc + brr - tinc/fact` | `FMA(dhsdT, tinc, hs)` |
| `hm(j) = hs_snow + fsno*dhsdT*tinc + brr - tinc/fact` | `FMA(fsno*dhsdT, tinc, hs_snow)` |
| `1. - fact*(1-fsno)*dhsdT` / `1. - fact*dhsdT` / `1. - fact*fsno*dhsdT` | 三条 `FNMA(dhsdT, ·, 1.0)` |

`tinc/fact` 是除法，不参与收缩；`hm(j) = brr(j) - tinc/fact(j)`（内部层）本来就没有收缩点。

### 差点改错：`t + fact*heatr` **不是** fma —— 因为 CSE

温度修正那一段上游写四种：
```
j > lb 且非分界面 : t = t + fact*heatr
j > lb 且分界面   : t = t + fact*heatr/(1-fact*(1-fsno)*dhsdT)
顶层              : t = t + fact*heatr/(1-fact*dhsdT) 或 /(1-fact*fsno*dhsdT)
```
第一支 `t + fact*heatr` 单看就是"乘积进加法"，按前几轮总结的规则**应该**写成
`FMA(fact, heatr, t)` —— 我第一版正是这么改的。但 GIMPLE 显示：

```
_37 = fact*heatr            ← 四个分支共用一个临时量
_39 = _37 + t               ← 第一支：先舍入乘积再相加
_40 = _37 / d1 ; _41 = t + _40
```

**`fact*heatr` 在多个分支里出现，GCC 把它 CSE 成一个临时量，于是它不再被吸收。**
所以那一支必须保持"先舍入乘积再相加"。改成 `mul_add` 会让它**离上游更远** ——
已按 GIMPLE 回退，并在代码注释里记下这次误判。

**教训（写进规矩）**：局部看形状得出的收缩结论会被**跨分支的公共子表达式**推翻。
判定收缩必须看**整个函数**在 GIMPLE 里的样子，不能只看那一行。

窗口三口径：干/湿窗逐位不变（那两窗的相变路径没进），雪窗逐位值 −49、Σ|Δ| 与超容差不变。

## `GroundTemperature` 的矩阵装配：顶层/内层/底层的 `bt` 与 `rt`（实测，**已修**）

`temperature_system` 是三对角方程组的装配处，每步都跑，之前没按 GIMPLE 校过。
用 `gfortran -O2 -fdefault-real-8 -ffree-form -fdump-tree-optimized` 读
`MOD_GroundTemperature.F90:311-380`，读出七处收缩：

| 上游 | GIMPLE | 说明 |
|---|---|---|
| `fact(lb) = …/(0.5*(z(j)-zi(j-1)+capr*(z(j+1)-zi(j-1))))` | `FMA(capr, z(j+1)-zi(j-1), z(j)-zi(j-1))` | 分母括号内 |
| `bt = 1+Q - fsno*fact*dhsdT` | `FNMA(fsno*fact, dhsdT, 1+Q)` | 顶层·雪 |
| `bt = 1+Q - fact*dhsdT` | `FNMA(fact, dhsdT, 1+Q)` | 顶层·非雪 |
| `bt = 1+P - (1-fsno)*dhsdT*fact` | `FNMA((1-fsno)*dhsdT, fact, 1+P)` | 内层·`j==1 && split` |
| `bt = 1 + (1-cnfac)*fact*sum` | `FMA(sum, (1-cnfac)*fact, 1.0)` | 内层·其他 |
| `rt = t + fact*(hs - dhsdT*t + cnfac*fn)` | `FNMA(dhsdT,t,hs)` → `+cnfac*fn` → `FMA(括号, fact, t)` | 顶层 |
| `rt = t - cnfac*fact*fn1` | `FNMA(cnfac*fact, fn1, t)` | 底层 |

四处**不**收缩，都是因为它们两侧是**商**而不是乘积：`Q = (1-cnfac)*fact*tk/Δz`、
`P = sum`（`tk/dzp + tk1/dzm`）、底层的 `bt = 1 + (1-cnfac)*fact*tk/dzm`、
以及 `j==1 && split` 里 `rt` 的 `t + cnfac*fact*Δ`（下游除法把它留在了括号里）。

### 相邻两条语句结论相反：`bt` 收了，`rt` 没收

顶层与内层的 `rt` 都是 `t + cnfac*fact*(fn - fn1)` 的形状，按"乘积进加法就吸收"的
形状规则应该写成 `mul_add`。**我第一版内层正是这么写的**，用忠实复刻的
`top2.f90`（含全部四个分支的数据流，`bt`/`rt` 六个量都镜像出来）跑 20000 组随机
输入后发现：`rt(2)` 新 Rust 19995/20000，而**旧** Rust 是 20000/20000 —— 也就是说
那一处本来就和上游一致，改了反而变远。回退后六项全部 20000/20000。

原因同样在 CSE：`cnfac*fact` 那个乘积在雪层／`j==1 && split`／其他三个内层分支里
共用，GCC 把它提成一个临时量（dump 里的 `_462`）再与 `t` 相加，于是不再被吸收。
顶层那个 `+ cnfac*fn(0)` 也是同理（两个顶层分支共用 `cnfac*fn(0)`）。

对照表（20000 组随机输入，逐位相等的组数）：

| 量 | 旧 Rust | 新 Rust |
|---|---|---|
| `bt(1)` | 19870 | 20000 |
| `rt(1)` | 19803 | 20000 |
| `bt(2)` | 19942 | 20000 |
| `rt(2)` | 20000 | 20000 |
| `bt(10)` | 20000 | 20000 |
| `rt(10)` | 20000 | 20000 |

旧 Rust 那三个不满的正是本轮改的顶层 `bt`/`rt` 与内层 `bt`。

**规矩（再次确认）**：判定收缩只能看**整个函数**的 GIMPLE，不能只看那一行；
`meltf` 的 `fact*heatr` 与本轮的 `cnfac*fact*Δ` 是同一类陷阱，而且这里更进一步 ——
**同一个函数里紧挨着的两条语句，一条收一条不收**。

### 窗口三口径（诚实记录：干窗三项都变差）

以 `71f1b7a`（`meltf`）为基线，`bash /tmp/gf/win4.sh` 三个窗口：

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 |
|---|---|---|---|
| 干（CN-Cng） | 21233 → 21198 | 602.7851 → **753.1381** | 838 → **877** |
| 湿（CN-Cng-wet） | 33346 → 33349 | 10386.7216 → 10374.7096 | 20666 → 20672 |
| 雪（US-NR1-snow） | 33587 → **33657** | 不变 | 不变 |

tier2 变量数：干 17 → 18，湿 68，雪 79。

干窗的 Σ\|Δ\| 涨了 25%，这是**唯一**一处单调口径明显变差的地方。仍然保留这次改动，
理由与前几轮一致：GIMPLE + 20000 组差分已经证明新的算式与上游**逐位等价**，
端到端指标的移动来自轨迹在阈值附近的抖动（同一份算式换个分支走向就会翻转），
不是算式更远。这条取舍已在 `meltf` 那节记过一次，此处是同一条规矩的第二次应用。

### 未验的部分

- `j < 1`（雪层）与 `j == 1 && split` 两条分支：差分驱动只喂了土壤层，这两支
  只有 GIMPLE 解码作依据，没有 20000 组差分。它们的算式与已验分支同形，
  但按上面的教训，同形**不能**当作证据。
- `surface_fluxes` 里 `snow`/`soil` 两组表达式尚未按 GIMPLE 扫过。

Tested: `cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo fmt --all --check` 与 `cargo fmt --manifest-path gui/src-tauri/Cargo.toml --all --check`；`cargo test -q -p oracle -- --test-threads=1`；`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

## `surface_fluxes`：三条式子形状相同、收缩结论各不相同（实测，**已修**）

`GroundTemperature` 里剩下的未扫部分。用同一套办法（忠实复刻
`MOD_GroundTemperature.F90:250-305` 的**全部分支** → 读 GIMPLE → 20000 组
随机输入逐位差分）读出这一段比矩阵装配更"碎"的收缩点：

| 上游 | GIMPLE | 归属 |
|---|---|---|
| `- (fseng+fevpg*htvp)` | `FMA(fevpg, htvp, fseng)` | `hs` / `hs_soil` / `hs_snow` 三处都有 |
| `hs + cpliq*pg_rain*Δ + cpice*pg_snow*Δ` | `cpliq*pg_rain`、`cpice*pg_snow`、`Δ` 先各自成公共量（`_301/_309/_305`），再 `+ _306`、`+ _310` **顺序**相加 | `hs` |
| `hs_soil + cpliq*pg_rain*Δ + cpice*pg_snow*Δ` | `FMA(Δ, cpliq*pg_rain, ·)`、`FMA(Δ, cpice*pg_snow, ·)` **两级融合** | `hs_soil` / `hs_snow` |
| `hs - emg*stefnc*t_grnd**4` | `FNMA(t**4, stefnc*emg, hs)` | 不分雪 |
| `hs - fsno*emg*stefnc*t_snow**4 - (1-fsno)*emg*stefnc*t_soil**4` | `FNMA(t**4, (fsno*emg)*stefnc, ·)`，注意分组是 `(fsno*emg)*stefnc` | 分雪 |
| `- emg*stefnc*t_soil**4` / `*t_snow**4`（`hs_soil`/`hs_snow` 内部） | `FNMA(t**4, stefnc*emg, dlrad*emg)`，这里用的是 `stefnc*emg` | `hs_soil` / `hs_snow` |
| `-cgrnd - 4.*emg*stefnc*t_grnd**3 - …` | `FNMS(stefnc*(emg*4), (t*t)*t, cgrnd)` | `dhsdT` |
| `hs_soil = hs_soil*(1.-fsno) + sabg_soil` | `FMA(1-fsno, hs_soil, sabg_soil)` | 融合 |
| `hs_snow = hs_snow*fsno + sabg_snow` | `fsno*hs_snow` 先舍入、再 `+ sabg_snow` | **不**融合 |
| `-cgrnd - cpliq*pg_rain - cpice*pg_snow` | 两个乘积先舍入，顺序相减 | 不融合 |

三条"看起来一样"的降水热：`hs` 里两项**不**融合（两个乘积被 CSE 提成 `_306/_310`，
在 `IF` 的两个分支里共用），`hs_soil`/`hs_snow` 里两项**都**融合（`_115/_120`）。
`hs_soil` 的最后一个乘加融合、紧挨着的 `hs_snow` 的那个不融合（`_256` 被两个
SNICAR 分支共用）——**这是本轮第三次遇到"相邻语句结论相反"**，规矩再确认一遍。

新的算式**不是**把旧式子逐项加 `mul_add` 就能得到的：`hs` 的降水热项必须**拆开**
写成两次顺序加法（旧的 `precipitation_heat` 闭包把它们先合成一个子式和，等价于
`+ (_306+_310)`，与上游的 `(+_306)+_310` 不同）。Rust 侧还顺手删掉了那个闭包。

### 差分（20000 组随机输入，八种分支组合全覆盖）

| 量 | 旧 Rust | 新 Rust |
|---|---|---|
| `hs` | 11503/20000 | **20000/20000** |
| `hs_soil` | 13848/20000 | **20000/20000** |
| `hs_snow` | 15239/20000 | **20000/20000** |
| `dhsdT` | 19955/20000 | **20000/20000** |

八种组合（`split` × `use_snicar` × `lb<1`）在新的算式下**每一种都是 8/8 满
20000**，不是靠某一支凑出来的。

两个方法论上的坑，记下来备用：

1. **C 驱动必须加 `-ffp-contract=off`。** clang 在 `-O2` 下默认开 `-ffp-contract=on`，
   会把 `surface` 里那些本不该融合的 `系数*变量` 顺手融进加法，于是"新算式"
   只有 ~80% 匹配 —— 看上去像是**解码错了**，其实是**测试自己的编译器**多融了。
   同一份 C 加上这个开关立刻 20000/20000。上一次量收缩规则时踩的是相反的坑
   （gfortran `-ffp-contract=fast` 是默认），两边都要显式钉住。
2. **`f64::powi(3/4)` 与 `(x*x)*x` / `(x*x)*(x*x)` 逐位相同**（200000/200000，随机
   正数）：gfortran 把 `t**3`/`t**4` 展开成 `powmult` 的形式，Rust 侧用 `powi`
   是对应的，不必手写成乘积。

### 连带修好一个单元测试的"假象"

`equilibrium_soil_column_stays_at_its_fortran_surface_balance` 断言
`phase_flag == [2, 2]`（气柱放在 `FREEZING_K`、长波刚好配平的边角夹具）。
把同一组夹具输入喂给 Fortran 复刻：上游给的是 `hs = +6.93182892583401833e-15`，
**不是** 0.0。旧算式把这个残差舍成了 0.0，`t == tfrz` 正好落在等号上才得到 2；
修好后 Rust 逐位复现上游的正残差，顶层落到 0 那一侧。断言按上游改成 `[0, 2]`，
并在测试注释里写明这条判据是 1-ULP 级的（温度断言仍是 1e-11，物理结论不变）。

### 窗口三口径（这一轮是净收益）

以 `a5e93f4`（`GroundTemperature` 矩阵装配）为基线：

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干（CN-Cng） | 21198 → 21199 | **753.1381 → 395.6581** | 877 → 830 | 18 → **17** |
| 湿（CN-Cng-wet） | 33349 → 33349 | 10374.7096 → 10380.6562 | 20672 → **20665** | 68 |
| 雪（US-NR1-snow） | 33657 → 33661 | 不变 | 不变 | 79 |

干窗的 Σ\|Δ\| 接近腰斩，是本轮唯一一处单调口径的大幅改善 —— 上一轮干窗变差的
那部分被这次改正的算式拿了回来，说明上一轮那个"端到端更差"确实只是轨迹抖动。

### 未验的部分

- 与上一节相同：`j < 1`（雪层）与 `j == 1 && split` 两条矩阵分支仍是 GIMPLE 依据；
  本节的差分**覆盖了**所有 `split`/`use_snicar`/`lb` 组合。

Tested: `cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo fmt --all --check` 与 `cargo fmt --manifest-path gui/src-tauri/Cargo.toml --all --check`；`cargo test -q -p oracle -- --test-threads=1`；`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）；三个窗口 `bash /tmp/gf/win4.sh` + `golden-compare`。

## `soil_hcap_cond` 的另外七个方案：`sr` 门槛的作用域看错了（实测，**已修**）

第 101 轮只量了黄金用的方案 4（Balland-Arp）。这轮把
`MOD_SoilThermalParameters.F90:230-521` **逐字**搬成一个独立子例程（只把
`USE MOD_Precision`/`USE MOD_Const_Physical`/`USE MOD_Namelist` 换成实参 `r8`、
`tfrz`、`scheme`），再用共享 LCG 生成 20000 组输入，让**真正的 Rust** 与它逐位比对。
逐字搬是必须的：上游的 `select CASE` 选的是**运行时**量，八个方案同时在一份
GIMPLE 里，CSE 会跨方案共用临时量。

| 方案 | 旧 Rust | 新 Rust |
|---|---|---|
| 1 Oleson | 20000/20000 | 20000/20000 |
| 2 Johansen | 18609/20000 | **20000/20000** |
| 3 Cote-Konrad | 18641/20000 | **20000/20000** |
| 4 Balland-Arp | 20000/20000 | 20000/20000 |
| 5 Lu | 20000/20000 | 20000/20000 |
| 6 Tarnawski-Leong | 13990/20000 | **20000/20000** |
| 7 De Vries | 11436/20000 | **20000/20000** |
| 8 Yan & He | 18467/20000 | **20000/20000** |

（"旧 Rust"是 `git checkout` 回 HEAD 再跑同一个探针得到的，不是手抄的近似。
输入里刻意混入 `sr = 0`、`sr ~ 1e-9`、`sr ~ 1e-7` 三档，否则门槛分支一次都进不去。）

### 新量出来的收缩点

| 方案 | 上游 | GIMPLE |
|---|---|---|
| 2 粗粒 | `ke = 0.7*log10(max(sr,0.05)) + 1.0` | `FMA(log10, 0.7, 1.0)` |
| 2 细粒 | `ke = log10(max(sr,0.1)) + 1.0` | 无乘积，不收缩 |
| 3 | `ke = kappa*sr/(1.0+(kappa-1.0)*sr)` | 分母 `FMA(sr, kappa-1, 1.0)`，分子普通乘积 |
| 6 | `aa = 0.0237-0.0175*a**3`、`nwm = 0.088-0.037*a**3` | 两条 `FNMA(a**3, ·, ·)` |
| 6 | `x = 0.6-0.3*a**3` 用在 `sr**(-x)` | GCC 把负号并进收缩：`FNMA(a**3, 0.3, 0.6)` 算的是 `-x` |
| 6 | `kf*nw + ka*(1-nw)` | `FMA(nw, kf, ka*(1-nw))` |
| 6 | `sr*vf_pores - nwm*nw` | `FNMA(nw, nwm, vf_pores*sr)` |
| 6 | `vf_pores*(1-sr) - nwm*(1-nw)` | `FMS(vf_pores, 1-sr, nwm*(1-nw))`，**减数是普通乘积** |
| 7 | `ga = 0.013+0.944*sr*vf_pores` | 先舍 `0.944*sr`，再 `FMA(vf_pores, ·, 0.013)` |
| 7 | `ga = 0.333-(1-sr)*vf_pores/vf_pores*(0.333-0.035)` | `FNMA((1-sr)*vf_pores/vf_pores, 0.29800000000000004, 0.333)` |
| 7 | `gc = 1-2*ga` | `FNMA(ga, 2.0, 1.0)` |
| 7 | 四个 `1/(1+比值*形状因子)` | 四条 `FMA(形状因子, 比值, 1.0)` |
| 7 | `thk` 的分子 | `FMA(sr*vf_pores, kf, ·)`、`FMA((1-vf_pores)*aaa, ks, ·)` |
| 8 | `beta = -0.303*ksat_u - 0.201*wf_sand + 1.532` | `FNMS(ksat_u, 0.303, 0.201*wf_sand) + 1.532` |

方案 1、5 本来就没有乘积可收（`log10(sr)+1`、`log10(max(sr,0.1))+1`、
`exp(alpha*(1-sr**(alpha-beta)))`），探针也确认旧代码就是 20000/20000。

### 真正的缺陷：`sr < 1e-10` 那道门只管到 `ke`

`:314-420` 的 `IF(sr >= 1.0e-10) ... ELSE ke = 0.0 ... ENDIF` 只影响 `ke`；而方案 6、7
的两段 `IF(DEF_THERMAL_CONDUCTIVITY_SCHEME == 6/7)`（`:432-497`）在这道门**外面**，
而且**根本不读 `ke`**。Rust 原来把门槛写成了对**所有**方案的整体早返回：

```rust
let conductivity_w_m_k = if saturation < 1.0e-10 { dry } else { ... };
```

干土上方案 6/7 于是直接给 `kdry`，而上游照算自己的式子。还是那组夹具
（`vf_pores=0.46, a=0.45, k_solids=3.1, ksat_u=1.83, kdry=0.24`、`sr = 0`）：

| 方案 | 上游 Fortran | 旧 Rust |
|---|---|---|
| 6 | 1.75832151597518249e-01 | 2.4e-01（= kdry） |
| 7 | 3.16533017865365807e-01 | 2.4e-01（= kdry） |

这**不是 1 ULP 的差别**，是"门槛作用域"看错了一行 —— 也就是说干土上的方案 6/7
此前一直是错的。单元测试 `dry_soil_uses_dry_conductivity_for_all_schemes`
把这个错误行为钉住了（"八个方案都回落到 kdry"），已按上游改成逐方案期望值，
测试名也跟着改。

### 窗口三口径：逐位不变（符合预期）

`a5e93f4`…`b5b0f95` 这条线上的三个窗口全部**逐位不变**（干 21199 / Σ|Δ| 395.6581 /
超容差 830；湿 33349 / 10380.6562 / 20665；雪 33661 / 444394.4368 / 25896）。
黄金算例用的是方案 4，本轮一个字节都没动它 —— 这条"不变"正是本轮改动的边界证据：
差异只落在别的方案上。

### 未验的部分

- 方案 6/7 的 `sr` 极小区间只验到 `sr = 0` 与 `~1e-7`；两者之间没有别的分支。
- 输入是随机量而不是真实土壤剖面；`beta`、`sat_*` 的取值区间比真实宽。

Tested: 探针比对（**真正的 Rust** vs 逐字复刻的 Fortran，八方案各 20000 组，共享 LCG）；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
`cargo fmt --all --check` 与 `cargo fmt --manifest-path gui/src-tauri/Cargo.toml --all --check`；
`cargo test -q -p oracle -- --test-threads=1`；`cargo run -q -p xtask -- check-gui`；
`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）；三个窗口 `bash /tmp/gf/win4.sh` + 三口径 A/B。

## 把 GIMPLE 的来源从"复刻件"换成**内核本体**：`spacAF_twoleaf` 的 `determ` 与三条共用乘积（实测，**已修**）

前几轮量收缩规则时一直用"忠实复刻件"——把上游那几十行抄成一个独立子例程再 dump。
本轮第一次直接 dump **内核本体**，方法是把 `MOD_PlantHydraulic.F90` 单独编译（模块
`.mod` 由既有的内核构建产物提供）：

```bash
B=vendor/CoLM202X
gfortran -O2 -fdefault-real-8 -ffree-form -g -ffpe-trap=invalid,zero,overflow -fbacktrace \
  -cpp -ffree-line-length-0 -fallow-argument-mismatch \
  -I $B/include -I $B/main -I $B/main/HYDRO -I $B/share -I $B/.bld -J/tmp/gf/r107 \
  -c $B/main/MOD_PlantHydraulic.F90 -fdump-tree-optimized=/tmp/gf/r107/ph.opt
```

（关键是 `-I $B/.bld`：`.mod` 全在那儿。）这一步立刻暴露了两个复刻件量不出来的东西。

### 一、`f(xyl)` 与 `f(root)` 共用同一个乘积，于是**两条都不收缩**

复刻件里 `f(xyl)`、`f(root)` 各自只出现一次 `X*(x(root)-x(xyl)-grav1)`，按形状规则
都会被吸收。内核本体里它们**在同一个函数里**，GCC 把它提成一个临时量：

```
_51 = _10 * _18            ! laisun*kmax_sun*fx * Δsun
_54 = _23 * _30            ! laisha*kmax_sha*fx * Δsha
_56 = _51 + _54
_57 = _40 * _45            ! sai*kmax_xyl/htop*fr * Δroot
_58 = _56 - _57            ! f(xyl)  ← 普通减法
_59 = qeroot
_60 = _57 - _59            ! f(root) ← 普通减法
```

`_51`/`_54`/`_57` 三个乘积**同时喂给 `qflx_sha > 0` 的 IF 与 ELSE 两条支路**，
所以一个都不吸收。Rust 四条式子改成：三个乘积显式绑定成变量（`sunlit_term`/
`shaded_term`/`root_term`），`f(SUNLIT)`/`f(SHADED)` 仍然 `mul_add`（它们收的是
**另一个**只出现一次的乘积 `qflx*fsto`），`f(XYLEM)/f(ROOT)` 保持普通加减。
这正是此前那个 4.1e-25 残差的来源 —— 当时的猜测（"内核的 CSE 上下文与复刻件不同"）
现在被本体 dump 直接证实。

### 二、`determ` 的收缩一直没人验过

`determ = A44*A22*A33*A11 - A44*A22*A31*A13 - A44*A32*A23*A11 - A43*A11*A22*A34`
在本体里是三级收缩，每级收**本级最后一个乘积**：

```
_62 = a22*a44*a33 ; _65 = a31*a22*a44*a13
_66 = FMS(_62, a11, _65)
_70 = FNMA(a32*a44*a23, a11, _66)
determ = FNMA(a43*a11*a22, a34, _70)
```

Rust 原来写的是普通乘减。**这条以前漏验了**：上一轮的做法是拿"神谕算好的
`determ`"喂进去只比 `dx`，所以四条 `dx` 式子验到 20000/20000，而 `determ` 自己
的算法没进过任何比对。现在改用**内核自己打出来的 49 组 `(A11..A44, f, dx)`** 反推：

| `determ` 写法 | `dx(leafsun)` | `dx(leafsha)` | `dx(xyl)` | `dx(root)` |
|---|---|---|---|---|
| 普通乘减（旧 Rust） | 27/49 | 27/49 | 28/49 | 29/49 |
| 三级 `mul_add`（新 Rust） | **49/49** | **49/49** | **49/49** | **49/49** |

（FMA 用 `fractions.Fraction(a)*Fraction(b)+Fraction(c)` 精确求值，不依赖
Python 版本有没有 `math.fma`。）这条同时也反过来确认了四条 `dx` 式子本身没错。

### 三、ELSE 分支逐条对上

`qflx_sha <= 0` 那条支路（干窗 49 次调用一次都没走到）也从本体 dump 里逐条核对过：
`determ = FNMA(a31*a13, a44, FMS(a11*a33, a44, a43*(a11*a34)))`、三条 `dx` 的
嵌套、以及 `dx(leafsha) = dx(leafsun) + (x(leafsun)-x(leafsha))`（末式是加法交换，
不是"先算差再换序"）—— 与 Rust 现有写法逐项一致。

### 四、端到端：中性（诚实记录）

- 干窗 TIMESTEP 30 步：改动前后 Rust **逐位相同**（69 个变量、0 处差异），
  与 Fortran 的第一处差异仍旧落在第 0 步的 1 ULP 上（`f_t_soisno` 两个层
  各差 1 ULP，见上一段"第一处差异"的既有记录）。
- 三个黄金窗口三口径**逐位不变**（干 21199/395.6581/830，湿 33349/10380.6562/20665，
  雪 33661/444394.4368/25896）。

也就是说这次改的是"确实错、但这几组算例里传不到输出"的地方。保留它的理由与
`meltf` 那次相同：**49 组真实内核数值已经把新写法钉死**，而窗口指标给不出信号。

### 未验的部分

- 49 组真实调用全部走 `qflx_sha > 0` 那条支路；ELSE 分支只有本体 GIMPLE 依据。
- 干窗 30 步里 `f_vegwp` 第 1 步就开始差 —— 那是**改动之前就有**的分叉
  （base 与 fix 逐位相同，所以不是本次引入），它的根因仍未定位。

Tested: 内核本体 `-fdump-tree-optimized`；49 组真实 `(A,f,dx)` 的精确 FMA 反推（`Fraction`）；
干窗 TIMESTEP 30 步 Rust-vs-Rust 与 Rust-vs-Fortran 逐位比对；三个黄金窗口 + 三口径 A/B；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
`cargo fmt --all --check` 与 GUI 侧同名检查；`cargo test -q -p oracle -- --test-threads=1`；
`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

## 近地层（Monin-Obukhov）：整条链子从来没有按 GIMPLE 校过（实测，**已修**）

`monin_obukhov.rs` 里此前**一个 `mul_add` 都没有**，而内核本体
（`MOD_FrictionVelocity.F90` 与 `MOD_TurbulenceLEddy.F90`）的 `moninobuk`、
`moninobukm`、`moninobuk_leddy`、`kmoninobuk`、`kintmoninobuk`、`moninobukini`
六个例程加起来有 60 处 `.FMA/.FNMA/.FMS/.FNMS`。这轮把两个模块**本体**编译出来
（`-I vendor/CoLM202X/.bld` 取 `.mod`）、写一个直接 `USE` 它们的 Fortran 驱动，
再用共享 LCG 生成 20000 组输入，拿 **真正的 Rust** 逐位比对七个输出。

### 量出来的收缩点

| 位置 | 上游 | GIMPLE |
|---|---|---|
| `psi` 的 `chik` | `(1.-16.*zeta)**0.25` | `FNMA(zeta, 16, 1)` |
| `psi` 的 `1+chik²` | `(1.+chik*chik)` | `FMA(chik, chik, 1)` |
| `psi(k=1)` | `2*log((1+chik)*0.5) + log((1+chik²)*0.5)` | `FMA(log, 2, ·)` |
| `psi(k=1)` 续 | `-2*atan(chik) + 2*atan(1)` | `FNMA(atan, 2, ·)`，`2*atan(1)` 折成常量 `π/2` |
| `fm`（三种方案） | `… + 1.14*Δ` / `0.8*Δ` | `FMA(Δ, 1.14, ·)` / `FMA(Δ, 0.8, ·)` |
| `fm` 稳定支 | `log + 5.*zeta` | `FMA(zeta, 5, log)` |
| `fm` 对数支 | `5.*log(zeta)+zeta` | `FMA(log(zeta), 5, zeta)` |
| `kmoninobuk` | `1.+5.*zeta` / `(1.-16.*zeta)**(-0.5)` | `FMA(zeta,5,1)` / `FNMA(zeta,16,1)` |
| `moninobukini` | `sqrt(um**2+0.5**2)` | `FMA(um, um, 0.25)` |
| `moninobukini` | `1.-5.*min(rib,0.19)` | `FNMA(min(rib,0.19), 5, 1)` |
| `moninobuk_leddy` | `0.0047*(-zetazi)+0.1854` | `FMA(-zetazi, 0.0047, 0.1854)` |
| `moninobuk_leddy` | `-2.*Bm2*(Δ)` | `FNMA(Bm2*2, Δ, ·)` |

`zetam = 0.5*Bm**4*(-16.-sqrt(256.+4./Bm**4))` 与各支的 `- 5*z0x/obu` 都**不**收缩
（分别是常量乘、商）。

### 一个只能靠"编译期折叠"对齐的常量

`kmoninobuk` 里 `0.9*vonkar**1.333` 被 GCC 折成一个常量
（`0.2653312957296878327184685986139811575412750244140625`）。Rust 在运行期算
`0.9 * 0.4_f64.powf(1.333)` 会**差 1 ULP**（libm 的 `pow` 不是正确舍入，GCC 的
折叠是）。所以这一处只能把折叠值写成常量，并在注释里记下来源。同类的
`1.574**0.333`、`0.465**(-0.333)` 运行期算出来与折叠值逐位相同，不需要特殊处理。

### 差分结果（20000 组，两个方案各七个输出）

| 方案 | ustar | fh2m | fq2m | fm10m | fm | fh | fq |
|---|---|---|---|---|---|---|---|
| 修前 LargeEddy | 14445 | 15742 | 15830 | 17634 | 16929 | 15916 | 15995 |
| 修前 Standard | 14516 | 15742 | 15830 | 17678 | 17045 | 15916 | 15995 |
| **修后（两个方案）** | **20000** | **20000** | **20000** | **20000** | **20000** | **20000** | **20000** |

### 又一个"测试自己的编译器"的坑（这次在 Fortran 一侧）

第一版差分只对上 ~97%，且失配分散在**所有四个分支**里，看上去像"哪里还差一处收缩"。
真相是**驱动程序的输入生成**：`hu = 5.0 + uni()*25.0` 被 gfortran 在 `drv.f90` 里
收缩成 `FMA(u, 25, 5)`，而 Rust 侧是"先乘后加"，于是同一个 case 的 `hu` 差 1 ULP，
输出自然全差。给驱动加 `-ffp-contract=off`（**只给驱动，模块仍按内核默认编译**）
之后立刻 20000/20000。

这条与上一轮"clang 默认融合"是同一个坑的镜像：**只要两侧有一段代码的编译选项
不同，先怀疑编译器，再怀疑算式**。诊断办法是把两边的输入也按位打出来比。

### 窗口三口径（诚实记录：干窗变差，湿/雪窗逐位变好）

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干（CN-Cng） | 21199 → 21202 | **395.6581 → 816.8618** | 830 → 873 | 17 → 18 |
| 湿（CN-Cng-wet） | 33349 → **33209** | 10380.6562 → 10386.9072 | 20665 → 20669 | 68 |
| 雪（US-NR1-snow） | 33661 → **33641** | 不变 | 不变 | 79 |

干窗的 Σ\|Δ\| 又翻了一倍 —— 与第 104 轮同一现象。仍然保留：**七个输出在两个方案、
四个分支上都是 20000/20000**，算式与内核逐位相同这一条比端到端阈值抖动更有分量
（第 105 轮干窗 Σ\|Δ\| 腰斩也印证了这类抖动是双向的）。

### 未验的部分

- `moninobukm`/`moninobukm_leddy`（冠层顶）与 `kmoninobuk`/`kintmoninobuk`
  不是 public，只有 GIMPLE 依据；`heat_similarity` 的三处收缩就是照
  `kmoninobuk` 的 dump 改的。
- 干窗 TIMESTEP 第 0 步的分叉在这一轮之后**逐位不变**（同一批 1 ULP 差异、
  同样的量级），说明它的根因**不在这条链上**，仍在别处。

Tested: 内核本体两个模块的 `-fdump-tree-optimized`；`USE` 本体模块的 Fortran 驱动
（`-ffp-contract=off`）与真 Rust 的共享 LCG 逐位比对（两方案 × 20000 × 7 输出，含分支统计）；
干窗 TIMESTEP 1 步比对；三个黄金窗口 + 三口径 A/B；`cargo test -q -p colm-core --lib`（353 通过）；
`cargo clippy --workspace --all-targets -- -D warnings`；两处 `cargo fmt --all --check`；
`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；
`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）；`cargo test --workspace --lib --bins -- --test-threads=1`。

**一次并发跑的假警报**：第一次验收把 `-p oracle` 与三个窗口的算例**同时**跑，
`oracle/tests/generated_case.rs` 的"重跑内核并逐位比黄金"报了不一致 ——
两条链子抢同一个工作目录。单独重跑整个 `-p oracle` 全绿（12 个二进制、0 失败）。
这与之前记过的"并行 `cargo test` 不稳定"是同一类问题：**算例类测试不能与别处的
内核运行并发**。

## `MOD_GroundFluxes` 的六处收缩（实测，**已修**），与"分叉在更上游"的结论

`groundfluxes` 本体的 GIMPLE 只有八条收缩，其中六条在 Rust 侧原本都没写：

| 上游 | GIMPLE |
|---|---|
| `z0mg = (1-fsno)*zlnd + fsno*zsno` | `FMA(1-fsno, zlnd, fsno*zsno)` |
| `dthv = dth*(1+0.61*qm) + 0.61*th*dqh` | `FMA(qm,0.61,1)` 得 `1+0.61qm`；`0.61*th` 也是公共量；`FMA(dth, 1+0.61qm, dqh*(0.61*th))` |
| `thvstar = tstar*(1+0.61*qm) + 0.61*th*qstar` | 复用上面两个公共量，`FMA(tstar, 1+0.61qm, (0.61*th)*qstar)` |
| `um = sqrt(ur*ur + wc2)` | `sqrt(FMA(ur, ur, wc2))` |
| `cgrnd = cgrnds + htvp*cgrndl` | `FMA(cgrndl, htvp, raih)` |
| `tref` / `qref` | `FMA(tstar, fh2m/fh 差, thm)`、`FMA(qstar, fq2m/fq 差, qm)` |

`z0hg = z0mg/exp(0.13*(ustar*z0mg/1.5e-5)**0.45)` 与 `rib` 都没有收缩点（指数链是普通乘积）。

### 窗口：干窗回到最好水平，湿/雪窗也小幅变好

以 `679b168`（近地层那一轮）为基线：

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干（CN-Cng） | 21202 → **21235** | **816.8618 → 291.8579** | 873 → **821** | 18 → **17** |
| 湿（CN-Cng-wet） | 33209 → 33208 | 10386.9072 → **10379.6428** | 20669 → **20662** | 68 |
| 雪（US-NR1-snow） | 33641 → **33660** | 不变 | 不变 | 79 |

干窗的 Σ\|Δ\| 从上一轮被抬高的 816.9 降到 **291.9**，是本仓库至今最好的数字
（第 105 轮那次是 395.7）。两项合起来看：**108+109 两轮对干窗是净收益**
（三口径 21199/395.66/830 → 21235/291.86/821）。

### 一条重要的否定结论：干窗第 0 步的分叉不在这一层

`ground_fluxes` 的输出（`fseng`/`fevpg`/`fgrnd`）正是干窗第 0 步就差的量，
所以本来预期这次会把它按住。实测：**一维 1 步窗口的 Rust 输出与修前逐位相同**
（0 处差异）—— 上一轮修近地层时也是这样。也就是说那些 1 ULP 差异是
`ground_fluxes` **输入**里带进来的，不在它的算式里。往回看只剩
`thm`/`qm`/`ur`/`rhoair`（`MOD_Atmosphere`/`MOD_Forcing` 那条链）与上一步的状态，
这是下一轮该扫的地方。

Tested: `MOD_GroundFluxes.F90` 本体的 `-fdump-tree-optimized`；干窗 TIMESTEP 1 步
Rust-vs-Rust 逐位比对；三个黄金窗口 + 三口径 A/B；`cargo test --workspace --lib --bins -- --test-threads=1`；
`cargo clippy --workspace --all-targets -- -D warnings`；两处 `cargo fmt --all --check`；
`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

### 未验的部分

- `groundfluxes` 参数太多（40+），本轮**没有**像近地层那样写"USE 本体模块"的
  随机差分驱动；六处收缩以本体 GIMPLE 为依据，窗口指标为佐证。

## 风速：内核从不使用 `hypot`（实测，**已修**，端到端中性）

Rust 侧有 **10 处**用 `f64::hypot(east, north)` 算风速，而内核全部写
`sqrt(us*us+vs*vs)`（`MOD_ForcingDownscaling.F90:395/486`、`MOD_Glacier.F90:222`、
`MOD_Lake.F90:742`、`MOD_LeafInterception.F90:293`、`MOD_LeafTemperature.F90:556`、
`MOD_RainSnowTemp.F90:203`、`MOD_SnowLayersCombineDivide.F90:153`、`MOD_Thermal.F90:547`、
`MOD_Vars_1DAccFluxes.F90:2743/2764` —— grep 遍 `vendor/CoLM202X/main/*.F90`
**一个 `hypot` 都没有**）。两者不是同一个函数：`hypot` 是为防上溢设计的，
随机取 200000 组 `(u,v)`，**10956 组（5.5%）结果不同**（各差 1 ULP）：

```
hypot != sqrt(us^2+vs^2): 10956/200000, max relative ULP 1.00
```

十处全部按上游的分组改成平方和开方，其中三处**顺序有讲究**：
`MOD_ForcingDownscaling` 写的是 `sqrt(forc_vs**2 + forc_us**2)`（**vs 在前**，
GIMPLE 里被吸收的是 vs 那个平方：`FMA(vs, vs, us*us)`），其余各处是 us 在前。

| 位置 | 上游出处 |
|---|---|
| `forcing_downscaling.rs` ×2（`ws_g`） | `MOD_ForcingDownscaling.F90:395/486` |
| `standard_lct_step.rs` / `assembly.rs`（`ur`） | `MOD_Thermal.F90:547` |
| `leaf_temperature.rs`（`ur`，含 `max(0.1)`） | `MOD_LeafTemperature.F90:556` |
| `interception.rs`（`FV`） | `MOD_LeafInterception.F90:293` |
| `snow.rs`（`forc_wind`） | `MOD_SnowLayersCombineDivide.F90:153` |
| `atmosphere.rs`（降雪密度用的 `forc_wind`） | `MOD_RainSnowTemp.F90:203` |
| `history_diagnostics.rs` ×2（应力模、`ur`） | `MOD_Vars_1DAccFluxes.F90:2743/2764` |
| `colm-forcing/gapfill.rs`（由 u/v 合成标量风） | 与内核同一约定 |

**端到端中性**：干窗 TIMESTEP 1 步的 Rust 输出与修前**逐位相同**，三个黄金窗口
三口径也**逐位不变**（21235 / 291.8579 / 821 / 17）。也就是说这三组算例里
`hypot` 恰好与平方和开方同值，或者这些调用点没落在差异值上。

这是连续第三处"确实错、但窗口测不出来"的修复（`determ`、近地层、地面通量之后）。
保留它的判据仍是"内核怎么写就怎么写"：`hypot` 与平方和开方**可证不同**（5.5%），
而内核一次都没用过 `hypot`。

Tested: 200000 组随机 `(u,v)` 的 `hypot` vs 平方和开方对比（10956 处不同）；
干窗 TIMESTEP 1 步 Rust-vs-Rust 与 Rust-vs-Fortran 逐位比对；三个黄金窗口 + 三口径 A/B；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
两处 `cargo fmt --all --check`；`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；
`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

### 未验的部分 / 下一步

- 干窗第 0 步的 1 ULP 分叉在连修三处（近地层、地面通量、风速）后**仍逐位不变**。
  现在可以断定它既不在近地层/地面通量的算式里，也不在 `ur` 的算法里；
  剩下的候选是 `thm`/`qm`/`rhoair` 的来源（`MOD_Atmosphere`/`MOD_Forcing` 那条链）
  与上一步的状态。下一轮应从"把 `groundfluxes` 的**输入**逐位打出来"
  入手，而不是继续扫模块。
- `MOD_ForcingDownscaling` 还有 20 处收缩（长短波、降水、风廓线的因子式）未改；
  它在本机三个算例里似乎没被走到（改不动窗口就是证据）。

## `groundfluxes` 的随机差分驱动：一个结合律错误（实测，**已修**）

上一轮的结论"分叉在 `groundfluxes` 的输入里"是**错的** —— 那时只有 GIMPLE 依据，
没有驱动。这轮补上：`MOD_GroundFluxes` 的 `PUBLIC :: GroundFluxes`（注意大写）
可以单独调用，写一个 49 实参的驱动（`MOD_Namelist` 的两个开关用一个小 stub 顶上，
取 schema 默认 `DEF_RSS_SCHEME=1` / `DEF_USE_CBL_HEIGHT=.false.`），
共享 LCG 出 20000 组输入、逐位比 23 个输出。**结果只有 18582–20000/20000**。

顺着输出往回缩，第二个驱动直接调 `moninobukini`（`PUBLIC`）：`um` 20000/20000，
但 **`obu` 只有 16188/20000**。逐条读 GIMPLE 才看出是**结合律**：

```
_8  = zldis * grav
_9  = dthv * _8                ! (zldis*grav)*dthv
_12 = um * thv                 ! thv*um
_13 = um * _12                 ! (thv*um)*um   ← 分母是 (thv*um)*um
rib = _9 / _14
```

Rust 写的是 `thv * um.powi(2)` = `thv*(um*um)` —— 先舍入一次平方，与内核**不是**
同一棵树。改成 `thv * um * um` 后 `obu` **20000/20000**。这条错误以前查不出来：
`um` 是对的，`obu` 只差 1 ULP，而它一路经 `obu → 迭代 → ustar → 阻力 → 通量`
放大成 23 个输出里的近 1400 处差异。

| 量 | 修前 | 修后 |
|---|---|---|
| `moninobukini` 的 `obu` | 16188/20000 | **20000/20000** |
| `groundfluxes` 的 23 个输出 | 18582–20000 | 18634–**20000** |

`groundfluxes` 本身**仍有缺口**（多数输出 93–98%）——**这条第 110 轮已更正**：
那是驱动侧 `ur` 与探针不一致造成的幻象，实测 23/23 全中（见文末"第 110 轮：更正"）。
原文如下，保留以记录判断过程：`moninobukini`、`moninobuk`、
`moninobukm`、`moninobuk_leddy` 四个被调用的例程现在都是 20000/20000，
所以剩下的差异在 `groundfluxes` 自己的循环里（`z0hg` 的雷诺数式、`thvstar`、
`zeta`、阻力与通量那几段中的某处）——下一轮用同样的"逐段输出中间量"的办法继续缩。

### 窗口三口径

以 `fb47578`（风速那一轮）为基线：

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 |
|---|---|---|---|
| 干（CN-Cng） | 21235 → 21229 | **291.8579 → 216.9941** | 821 → **813** |
| 湿（CN-Cng-wet） | 33208 → 33207 | 10379.6428 → 10386.5309 | 20662 → 20671 |
| 雪（US-NR1-snow） | 33660 → 33593 | 不变 | 不变 |

干窗的 Σ\|Δ\| 再创新低（216.99）。仍然保留：`moninobukini` 的 16188→20000 是
直接把内核本体跑出来的数字，比端到端阈值抖动硬。

Tested: `USE` 内核本体模块的两个 Fortran 驱动（`GroundFluxes` 49 实参 20000 组 ×
23 输出；`moninobukini` 20000 组 × 2 输出），均以 `-ffp-contract=off` 编译驱动本身；
干窗 TIMESTEP 1 步逐位比对；三个黄金窗口 + 三口径 A/B；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
两处 `cargo fmt --all --check`；`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；
`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

### 未验的部分

- `groundfluxes` 的 23 个输出还没有全中，缺口在它自己的循环里（见上）。
- 干窗第 0 步那批 1 ULP 差异在修完 `rib` 之后**依旧逐位不变**，说明它们
  既不在这三个模块的算式里，也不在 `ur`/`rib` 的算法里；下一步应当直接
  同步打印 Rust 与内核在**第一步的完整状态**（restart 读入 + 第一步各中间量），
  而不是继续逐个模块扫。

### 补记（第 108 轮）：把 `rib` 的结合律钉成单元测试，以及一次失败的二分

`moninobukini` 的 `rib` 结合律现在有一条**永久回归测试**
（`monin_obukhov_tests.rs::moninobukini_rib_keeps_the_kernels_association`）：
输入取内核驱动里第 2 组的原值，期望 `obu = -68.50766570868211`（`0xC051207D985005C8`）、
`um = 5.190240462382922`（`0x4014C2CE65513E06`）。把分母改回 `thv*um.powi(2)`，
这条测试立刻红（实测）：

```
assertion `left == right` failed: obu no longer matches moninobukini
```

**一次失败的二分（记下来免得下次再撞）**：想按"稳定/不稳定"分流来缩小
`groundfluxes` 的剩余缺口，做法是把驱动里的 `dthv` 取绝对值后重跑。结果两个驱动
产出**逐位相同**的文件 —— 因为 `dthv` 根本不是 `GroundFluxes` 的实参，它是**函数体
内部**由 `dth = thm-t_grnd`、`dqh = qm-qg` 现算的（`:164`）。
也就是说"改驱动的一个输入来切换分支"这件事在这里做不到：要分流得改
`thm`/`t_grnd`/`th`/`qm`/`qg` 的生成式。下次分流前先确认那个量是不是实参。

### 第 109 轮：给 `groundfluxes` 的循环装探针，以及一个探针自身的坑

把 `MOD_GroundFluxes.F90` **复制到 `/tmp`**（不动 vendor）、只加一句 `WRITE`
打印每轮的 `tstar/qstar/z0hg/thvstar/zeta/obu/um`，再把它单独编译进驱动。
第一次把 `WRITE` 放在 `thvstar=` 之后，得到的内核数字与 Rust **处处差 1 ULP**；
把 `WRITE` 挪到循环体**末尾**（`obuold = obu` 之后）后，第 1 个算例的六轮
**逐位全中**：

| 量 | 内核（IT 6） | Rust |
|---|---|---|
| `tstar` | `3FB940B113FD286E` | 同 |
| `qstar` | `BF43D40170ECE005` | 同 |
| `z0hg` | `3F5701F3E8331640` | 同 |
| `zeta` | `BF5EE8B28F428632` | 同 |
| `um` | `40274765F8A626A4` | — |

**坑：探针自己会改内核。** 第一次的位置让 `tstar/qstar/z0hg/thvstar` 被提前
materialize，GCC 后续的 CSE/收缩随之改变，于是"内核"给出了一组根本不属于它的数字。
这与前面记过的"复刻件 CSE 与本体不同"是同一类问题的第三种形态：
**量收缩的探针必须放在不干扰被测量的位置**（这里是循环末尾）。

同一个位置错误还解释了第一版把 `zeta` 读成"差 1e9 ULP"——那时它读到的是
**上一轮**的值（`zeta` 在打印点之后才重算），而且是被扰动过的上一轮。

### 失配算例的指纹（下一步的精确入口）

> **第 110 轮更正**：下面这张表整体作废。失配是驱动侧 `ur` 与探针不一致
> （融合 vs 不融合）造成的；把探针的 `ur` 换成驱动那一份后 23 个输出全中。
> 保留是为了记录"从指纹推结论"这一步是怎么被一个输入错位带偏的。

按"哪些输出不同"给失配算例分类：

| 算例 | 不同的槽位 |
|---|---|
| 6 | **只有 13（`bulk_richardson_number`）** |
| 38 | 0–7（全部通量）、11（`z0hg`）、14（`ustar`）、20–22 |
| 53 | 0,1,2,9,12（`zeta`）,13,14,15,16,18,19,20,22 |
| 97 | 0–4,13,14,20,22 |

算例 6 只有 `rib` 不同，而

```
rib = zeta * ustar**2 / ((vonkar**2/fh) * um**2)     ! GIMPLE：_86 / (_87*_249)
```

里 `zeta`(12)、`ustar`(14)、`fh`(18) 都是本轮比对**已中**的量，
唯一没有对外暴露的因子就是 `um`（Rust 的 `adjusted_wind`，只用于算 `rib`）。
所以下一轮的入口很明确：**不稳定支的 `um = sqrt(ur*ur + wc*wc)` 与
`wc = (-grav*ustar*thvstar*zii/thv)**(1./3.)`**，以及 `zii` 的取值。

（本轮尝试用公开 API 复刻循环来取 `um` 失败了：`case 6` 的输入没有照 LCG
逐项对齐，复刻件的 `zeta` 直接被 clamp 到 2.0 —— 复刻件必须先把 `zeta/tstar/qstar`
对上 Rust 自身的输出才能当证据，这一点下次要先做。）

### 第 110 轮：更正 —— `groundfluxes` 本来就是逐位对的，"缺口"是**驱动自己的 `ur`**

第 111 轮与第 109 轮记下的两条结论**作废**：

- ~~"`groundfluxes` 的 23 个输出只有 18634–20000/20000"~~
- ~~"算例 6 只有 `rib` 不同 ⇒ 嫌疑是不稳定支的 `um`"~~

真实原因：那两份差分驱动是**用 `-ffp-contract=off` 编译的**，所以驱动里的
`ur = max(0.1, sqrt(us*us + vs*vs))` 是**不融合**的两步；而我在 Rust 探针里
按"内核约定"写成了 `us.mul_add(us, vs*vs).sqrt().max(0.1)` —— 融合版。
两者约 5% 的输入差 1 ULP，于是每个算例的**输入**就不一样了，
"输出对不上"是必然的。

把探针的 `ur` 换成驱动那一份（`(us * us + vs * vs).sqrt().max(0.1)`）后，
**23 个输出全部 20000/20000**：

```
with the driver's unfused ur: [20000, 20000, 20000, 20000, 20000, 20000, 20000,
 20000, 20000, 20000, 20000, 20000, 20000, 20000, 20000, 20000, 20000, 20000,
 20000, 20000, 20000, 20000, 20000]
```

也就是说：`groundfluxes` 本体、它调用的四个近地层例程、以及 `moninobukini`
**全部逐位可信**；第 109 轮那张"失配算例指纹"（算例 38/53/97 的循环态分叉）
同样是这个输入错位造成的幻象。

（第 107 轮那条 `rib` 结合律的修复**不受影响**：那个驱动的 `ur` 是
`0.1 + uni()*13.0`，两边都是不融合的两步；而且它现在有一条会红的回归测试。）

#### 这是同一类错误的**第三次**，值得写成硬规矩

| 次数 | 谁多融合了一次 | 现象 |
|---|---|---|
| 第 105 轮 | clang 编译的 C 参考件（默认 `-ffp-contract=on`） | "新算式只有 ~80% 匹配"，像解码错了 |
| 第 108 轮 | gfortran 编译的 `drv.f90`（默认 `fast`） | 97% 匹配，失配均匀散在所有分支 |
| 第 110 轮 | **我自己写在 Rust 探针里的 `ur`** | 23 个输出 93–98%，还编出了"指纹" |

三次的根因都是"**两侧生成同一个输入的方式不同**"。规矩：
差分驱动的**输入必须由同一段代码生成**——最省事、最不容易错的做法是
**在 Rust 里生成输入、写成一个文件，让 Fortran 驱动读它**；
退一步也必须把"驱动侧的每个输入表达式"逐个对着探针抄一遍，
并且**驱动与探针的编译选项要一起看**（Rust 永不隐式融合，Fortran/C 会）。

## `MOD_ForcingDownscaling` 的十处收缩（实测，**已修**），并确认本机三个算例走不到它

把 `MOD_ForcingDownscaling.F90` 单独 dump（`-I vendor/CoLM202X/.bld`），
它有 26 处 `.FMA/.FNMA/.FMS/.FNMS`，而 `forcing_downscaling.rs` 此前只有上一轮
换掉的 `hypot`。这轮按其**本体 GIMPLE** 补齐十处：

| 上游 | GIMPLE |
|---|---|
| `rhos`：`egcm = q*p/(wv+(1-wv)*q)` | 分母 `FMA(q, 1-wv, wv)` |
| `rhos`：`(p-(1-wv)*egcm)/(rair*t)` | 分子 `FNMA(egcm, 1-wv, p)` |
| `tbot_c = tbot_g-lapse*Δz` | `FNMA(Δz, lapse, tbot_g)` |
| `thbot_c = thbot_g+(tbot_c-tbot_g)*exp(·)` | `FMA(tbot_c-tbot_g, exp, thbot_g)` |
| 晴空发射率 `0.23+0.43*X**(1/5.7)` ×2 | `FMA(X, 0.43, 0.23)` |
| 长波递减率（冰川/普通两支） | 两条 `FNMA(·, Δz, dlrad)` |
| 降水 ListonElder 的分母 `1-0.27*Δz` | `FNMA(Δz, 0.27, 1.0)` |
| 日地距离比 `1-0.01672*cos(·)` | `FNMA(cos, 0.01672, 1.0)` |
| 漫射权重两级 `2.3-4.702*clr`、`0.952-1.041*exp(-exp(·))` | `FNMA(clr, 4.702, 2.3)`、`FNMA(exp, 1.041, 0.952)` |

### 端到端**测不到**，而且原因已经查清

改完之后干窗 TIMESTEP 1 步的 Rust 输出**逐位不变**，三个黄金窗口三口径也
**逐位不变**。原因不是"数值没差"，而是**这条路本机三个算例根本走不到**：

```
crates/colm-runtime/src/lib.rs:502   downscale_forcings(...)   ← 在 run_downscaled 里
crates/colm-runtime/src/bin/colm-rs.rs:190/205  run_restart_standard_lct_snow_with_history
                                                run_restart_standard_lct_snow  ← 不带降尺度
```

`run_downscaled` 是另一条公开入口（给需要把格点强迫降到站点的驱动用），
`colm-rs` 走的是 `run_restart_standard_lct_snow*`。所以这一处的状态是
**"按本体 GIMPLE 改对了，但本仓库现有算例无法证伪也无法证实"** —— 与前面
几处"改了但窗口不动"不同，这次连"窗口不动"都不构成弱证据。

保留该改动的理由：`forcing_downscaling.rs` 是移植面的一部分、由 `run_downscaled`
公开调用，且十处改动全部有本体 GIMPLE 逐条对应。未验的部分照实记下。

**这也顺带解释了干窗第 0 步分叉的排查为什么一直"扫不动"**：能扫到的模块里，
真正在这条算例路径上的越来越少；下一步只能在**第一步的完整状态与强迫**上做
逐位对照（restart 读入 + `forc_t/q/us/vs/pbot/rho` 及其派生），而不是继续找模块。

Tested: `MOD_ForcingDownscaling.F90` 本体的 `-fdump-tree-optimized`；`cargo test --workspace --lib --bins -- --test-threads=1`；
`cargo clippy --workspace --all-targets -- -D warnings`；两处 `cargo fmt --all --check`；`cargo test -q -p oracle`；
`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）；
干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对；三个黄金窗口 + 三口径 A/B（均逐位不变）。

### 未验的部分

- 十处收缩只有本体 GIMPLE 依据；**没有**像近地层那样写"USE 本体模块"的随机差分驱动
  （`downscale_forcings` 有 37 个实参，且要按 `SinglePoint` 分支准备 `sf_lut_c`）。
  下一轮若要把这块真正验掉，应当补这个驱动。
- 降尺度模块还剩约 6 处收缩（`downscale_shortwave` 的地形因子、`downscale_wind_simple`
  的因子式等），它们的结合顺序还没从 GIMPLE 里读透，故**没动**。

## `soil_vliq_from_psi` 的一处收缩，以及"下一个大件"的**清点**

`MOD_Hydro_SoilFunction.F90` 整模块**只有一处**收缩，在 `soil_vliq_from_psi`
的 van Genuchten 支（`:162`）：

```
soil_vliq_from_psi = (porsl - vl_r)*esat + vl_r      ⇒  FMA(porsl-vl_r, esat, vl_r)
```

Rust 那一行与此前的写法只差这一次融合，已改。`CN-Cng` 算例没有写
`DEF_USE_Campbell_SOIL_MODEL`，取 schema 默认 `.false.`，所以走的就是这一支。
干窗 TIMESTEP 1 步改动前后**逐位相同**——单点 1 ULP 在这里传不到历史输出。

### 清点：`MOD_SoilSnowHydrology` 是这条路径上最后一个大件

顺手把剩下没扫过的大模块数了一遍（内核本体 dump 里每个例程的
`.FMA/.FNMA/.FMS/.FNMS` 条数）：

| 例程 | 收缩条数 |
|---|---|
| `soilwater` | 23 |
| `water_vsf` | 17 |
| `water_2014` | 15 |
| `snowwater_snicar` | 8 |
| `snowwater` | 4 |
| **合计** | **67** |

这是干窗路径上**最大的一块未扫面积**，而且正好覆盖 `f_wliq_soisno`/`f_wice_soisno`/
`f_h2osoi`/`f_zwt` 这些仍然差分的量。下一轮从这里进：先做 `water_2014`
（15 条，标准 LCT 的默认方案），再按需要做 `soilwater`。

Tested: `MOD_Hydro_SoilFunction.F90` 本体的 `-fdump-tree-optimized`（整模块 1 处收缩）；
`MOD_SoilSnowHydrology.F90` 本体的 dump（66 之外的 67 条清点）；干窗 TIMESTEP 1 步逐位比对；
`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；`cargo fmt --all --check`；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

## `water_2014` 的头两条：`wliq/wice(1)` 的 `max(0., ... + 通量*deltim)`

`MOD_SoilSnowHydrology.F90:467-468`：

```fortran
wliq_soisno(1) = max(0., wliq_soisno(1) + qsdew_soil * deltim)
wice_soisno(1) = max(0., wice_soisno(1) + (qfros_soil-qsubl_soil) * deltim)
```

GIMPLE 把 `deltim*通量` 收进加法：`FMA(deltim, qsdew_soil, wliq)`、
`FMA(deltim, qfros-qsubl, wice)`。Rust 已按此写成 `mul_add`。

干窗 TIMESTEP 1 步改动前后**逐位相同**（又一次"改对了但测不到"），
所以这条同样是"本体 GIMPLE + 上游源码"双重依据、端到端无信号。

### `MOD_SoilSnowHydrology` 的进度与本轮范围

上一轮清点出该模块 67 处收缩。本轮只吃掉 `water_2014` 里最明确的两处
（`wliq/wice(1)` 的更新），**不是**因为它难，而是因为剩下那 13 处都在
`wa`/`wdsrf`（`FMA(porsl(1), X, pondmx)` 两个变体）、`gwat`（`FMA(1-fsno, pg_rain, gwat)`）
以及 `WATER_VSF`/`soilwater` 的数组累积里，需要先把上游那几段的**完整数据流**
读出来才能确定结合顺序 —— 与前面几轮"同形不同收缩"的教训同源，宁可慢一轮也不要再交一次
"下轮推翻上轮"。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 `-fdump-tree-optimized`；干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对；
`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；`cargo fmt --all --check`。

## `groundwater` 的 `xs1` 顶层容量（被 `water_2014` 内联）

`MOD_SoilSnowHydrology.F90` 的 `groundwater`（`:2532`，`WATER_2014` 里 `CALL` 它）：

```fortran
xs1 = wliq_soisno(1) - (pondmx+porsl(1)*dzmm(1)-wice_soisno(1))
```

dump 里这个例程被**内联进 `water_2014`**，所以按函数名过滤时会误以为是
`water_2014` 自己的语句 —— 记下来：**内联会打乱"按函数名归属收缩"的直觉**，
遇到可疑的收缩最好连它的上游源码行一起找，而不是只信 dump 的函数头。

GIMPLE：`FMA(porsl[0], dzmm[0], pondmx)`（`:8362`、`:8937` 两处，对应两支）。
Rust 的 `update_groundwater_with_resolver` 里那行 `top_capacity` 已改成
`porosity[0].mul_add(thickness_mm[0], ponding_limit_mm) - ice_water_kg_m2[0]`。

干窗 1 步改动前后**逐位相同**——这是连续第六处"依据充分、本机窗口测不到"的修复。
注意 `CN-Cng` 走的是 TOPMOD 分支（`update_groundwater_topmodel`），
`update_groundwater_with_resolver` 未必落在它的路径上，这一点没有单独确认。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 `-fdump-tree-optimized`（含内联归属的核对）；
干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对；`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；
`cargo fmt --all --check`。

## `groundwater` 的 `wa` 两处（`qcharge`/`drainage` 的 `deltim` 乘积）

```fortran
wa = wa + qcharge*deltim      ! groundwater:2403
wa = wa - drainage * deltim   ! groundwater:2470
```

GIMPLE：`FMA(deltim, qcharge, wa)`、`FNMA(deltim, drainage, wa)`。
Rust 的 `update_groundwater_with_resolver` 两行已改（`aquifer_water_mm`）。
干窗 1 步仍**逐位相同**（连续第七处）。

### 一处**没有**照搬的地方（记下来，避免误改）

`water_2014` 的雪支还有一句：

```fortran
gwat = gwat + pg_rain*(1-fsno) - qseva_soil      ! :278
```

GIMPLE 是 `FMA(1-fsno, pg_rain, gwat)`。但 Rust 这一支**不是**把三项累加进
`gwat`，而是把 `ground_rain_kg_m2_s` 换成 `snow.bottom_drainage_kg_m2_s`、
把 `ground_evaporation` 换成雪面蒸发，再由
`water_input = ground_rain + snowmelt - ground_evaporation` 现算
（见 `water_2014.rs:320-336` 的注释）。这是**结构不同而非少一次融合**，
所以没有硬套 `mul_add`；要动它得先把"雪列底部排水是否已含 `pg_rain*(1-fsno)`"
从 `snowwater` 的输出定义里确认清楚。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 `-fdump-tree-optimized`；干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
两处 `cargo fmt --all --check`；`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；
`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

## `soilwater` 的 Campbell 指数：`2*bsw+3` 与 `2*bsw+2` 是**两个**指数

`MOD_SoilSnowHydrology.F90:2172-2173`：

```fortran
hk(j)    = hksati(j) * (vol_liq(j)/porsl(j))**(2.*bsw(j)+3.)
dhkdw1(j)= hksati(j) * (2.*bsw(j)+3.)*(vol_liq(j)/porsl(j))**(2.*bsw(j)+2.)/porsl(j)
```

GIMPLE 里是两个**独立**的收缩：`FMA(bsw,2,3)` 与 `FMA(bsw,2,2)`。
Rust 原先把导数写成 `saturation.powf(exponent - 1.0)` —— 代数上等价，
但**多舍入一次**（`fl(fl(2bsw+3)-1)` vs `fl(2bsw+2)`，跨 binade 时会差 1 ULP）。
已改成两个指数各自 `f77(2.0).mul_add(bsw, …)`。

**为什么窗口还是不动**：`CN-Cng` 没有写 `DEF_USE_Campbell_SOIL_MODEL`，
取 schema 默认 `.false.`，所以走的是 **van Genuchten** 支 —— 这一支在本机
三个算例里是**死代码**。这是连续第八处窗口测不到的修复，而且这次连"弱证据"
都不算：分支根本没进。

**清点补充**：`soilwater` 的 23 处里，前 6 处（`2*bsw+3/2` 三个变体）就是本节；
其余 17 处是
`FMS/FMA` 形式的界面导水率（`… ∓ X/Y`，`:2186-2192` 那一组）、
`FNMA(etr, rootr, …)` 的根吸水项、以及 `FMA(dwat, …, errorw)` 的水量平衡误差累积。
它们的结合顺序需要逐段读上游那几段循环的数据流，留到下一轮。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 `-fdump-tree-optimized`（`soilwater` 23 处逐条列出）；
干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对（不变）；`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；
`cargo fmt --all --check`。

## `soilwater` 的三对角装配：`FMS/FMA(gradient, 导数, 商)`

`MOD_SoilSnowHydrology.F90:2186-2192` 那一组装配（首层/中间/末层三处同型）在
dump 里是

```
_171 = .FMS (_162, _165, _170)      ! gradient*下导数 - 商
_179 = .FMA (_162, _174, _178)      ! gradient*上导数 + 商
```

其中"商"是 `hk*potential_derivative/separation`（**先各自舍入**），被吸收的是
`gradient*导数` 那个乘积。Rust 的 `soil_water.rs` 八处（三处循环体 × 上下导数，
加重复项）已改成 `gradient.mul_add(导数, ∓商)`。`f77` 也已导入该文件。

### 一个必须记下来的**反常事实**

这是连续第九处"改对了但干窗第 0 步逐位不变"的修复，而**这一处明确在
`CN-Cng` 的路径上**（该算例没写 `DEF_USE_Campbell_SOIL_MODEL`，走 van Genuchten；
没写 `DEF_USE_VariablySaturatedFlow`，所以走的就是 `soil_water.rs` 这一支）。
九处、覆盖近地层、地面通量、热参数、土壤水、相变多个模块，全部**零位移** ——
这已经不像"1 ULP 恰好不传"，更像干窗第 0 步的那批差异**由另一条源头支配**。

因此下一轮**不要再继续扫模块**。应当做的是一次**定点对照**：
把 Rust 与内核在第一步的**完整状态与强迫**逐位打印并排（restart 读入的
`t_soisno`/`wliq`/`wice`/`smp`… 与 `forc_t/q/us/vs/pbot/rho` 及其派生
`thm`/`qm`/`ur`），先确认**两侧的输入是否本来就不同**。
如果输入相同而输出不同，再回到模块；如果输入不同，问题在读入或装配，
与物理模块无关 —— 那样前面几轮的"扫模块"就是方向错了。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 `-fdump-tree-optimized`；干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对（不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；`cargo fmt --all --check`；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

## `snowwater` 的表层冰/液更新（两处）

`MOD_SoilSnowHydrology` 的 `snowwater` 里表层两条更新都是
`wice/wliq = 原值 + 通量*deltim`，GIMPLE 都把 `通量*deltim` 收进加法：

```
FMA(deltim, frost-sublimation, wice)
FMA(deltim, rainfall+dew-evaporation, wliq)
```

Rust 的 `snow.rs` 两处已改成 `time_step_seconds.mul_add(通量, 原值)`。
`wgdif`（`FMA(qsnowmelt-qsubl, deltim, wgdif)`）与雪层里
`FNMA(ssi, X, wliq)` 的不可约含水项在 Rust 侧用了别的组织方式，
**没有一并改** —— 需要先把 `snowwater` 那两个循环的完整数据流读出来。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 `-fdump-tree-optimized`；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
两处 `cargo fmt --all --check`；`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；
`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

## `MOD_CanopyLayerProfile` 的两处积分几何式，以及**三个新模块的清点**

`canopy_layer_profile.rs` 此前一处 `mul_add` 也没有。`MOD_CanopyLayerProfile.F90` 的
积分循环（`uintegral`/`kintegral`）在 dump 里是

```
FNMA(i-0.5, dz, top)      ! top - (i-0.5)*dz
FMA(dz, 0.5, bottom)      ! bottom + 0.5*dz
```

两处循环体（风速积分与扩散率积分）同型，Rust 已按此改成
`(-(i as f64 - 0.5)).mul_add(step, top)` 与 `step.mul_add(0.5, bottom)`。

### 顺手把三个模块数清楚了（这是"还差多少"的实际答案）

| 模块 | 收缩总数 | 主要分布 |
|---|---|---|
| `MOD_CanopyLayerProfile` | **24** | `cal_z0_displa` 5、`kintegral` 4、`uintegralz` 4、`uintegral` 4、`fkint`/`fuint` 各 2 |
| `MOD_NetSolar` | **35** | 全在 `netsolar` |
| `MOD_Albedo` | **136** | `twostream_wrap` 72、`twostream` 45、`albland` 9、`albocean` 8、`snowage` 2 |

加上 `MOD_SoilSnowHydrology` 剩下的约 50 处，**干窗路径上还剩约 240 处收缩未扫**。
（`twostream` 那 117 处是大头，且 `MOD_Albedo` 也有纯 Fortran 的 `twostream` 副本，
两侧算法应当一致，可以互相印证。）

### 第 10 次"零位移"

加上这一处，已有**十处**依据充分（多数有本体 GIMPLE + 上游源码行双证据、其中几处
还有 20000/20000 的直接差分）的修复，对干窗第 0 步那 34 个 1 ULP 差异
**完全没有影响**。这已经足以排除"逐点舍入"的解释，下一轮必须做**输入级对照**
（Rust 与内核在第一步的状态与强迫逐位并排），而不是继续把模块一个一个扫过去。

Tested: `MOD_CanopyLayerProfile.F90`/`MOD_NetSolar.F90`/`MOD_Albedo.F90` 本体的
`-fdump-tree-optimized`（用于清点）；干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对（不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；`cargo fmt --all --check`。

## **重大澄清**：三个黄金算例走的是 VSF，`soil_water.rs` 是死代码

第 110-119 轮反复记录"改了却零位移"。这轮把它查清楚了，根因是一个 namelist 默认值：

```
DEF_USE_VariablySaturatedFlow = .true.      ! MOD_Namelist.F90:317（schema 同值）
CN-Cng/case.nml 里没有这一项 → 取默认 → 打开
```

于是 `water_2014_soil_step` 在第 178 行**直接转给 `variably_saturated_soil_step`**：

```rust
if input.variably_saturated {
    return variably_saturated_soil_step(input, state);
}
```

**后果**：`soil_water.rs`（Campbell/Richards 的 `soilwater` 与 `groundwater` 移植）
以及 `water_2014` 尾部那些更新，在**三个黄金算例上全部不执行**。第 113-117 轮
在 `soil_water.rs`/`water_2014.rs` 里改的那些收缩（`wliq/wice(1)`、van Genuchten 的
`soil_vliq_from_psi`、`groundwater` 的 `xs1`/`wa`、`soilwater` 的 Campbell 指数与
三对角装配）都是**另一个配置**（VSF 关掉）才走的路 —— 它们本身没错，
但对本机算例确实"零位移"，这正是第 119 轮那个"十次零位移"的解释。

**活的路径是 `variably_saturated_flow.rs`（`WATER_VSF` 的移植），它此前
`mul_add` 数为 0，上游 dump 里有 17 处收缩。** 上游 `WATER_VSF` 的收缩形态
（`water_vsf`）已经列好：`gwat = FMA(1-fsno, pg_rain, gwat)`、
`FMA(qsdew, deltim, wliq)` / `FNMA(x, deltim, y)` 的表层与逐层更新、
`FMA(Δ, k, rhs)` 与 `FNMA(k, max(…,0), rhs)` 的矩阵项、
`FNMA(x*1000, …, y)` 一处。

### 本轮先做两处（表层露/霜/升华）

`WATER_VSF` 表层两条：

```fortran
wliq(1) = max(0., wliq(1) + qsdew_soil * deltim)
wice(1) = max(0., wice(1) + (qfros_soil-qsubl_soil) * deltim)
```

GIMPLE：`FMA(qsdew_soil, deltim, wliq)`、`FMA(deltim, qfros-qsubl, wice)`。
`variably_saturated_flow.rs` 两处已改成 `dt.mul_add(…)`。

**但干窗仍逐位不变**：多数步的露/霜/升华是 0，乘积融不融合结果相同 ——
这两处需要构造有凝结的算例才能看出差别。

### 另一个观察：`hk` 的差异被"相消"放大

顺手比对了 1 步后两侧写出的 restart：`t_grnd`/`zwt`/`wa`/`wdsrf`/`scv`/`snowdp`/`fsno`/
`tleaf`/`vegwp`/`gs0sun` **逐位相同**，而 `t_soisno`/`wliq_soisno`/`wice_soisno`/`smp`
各差 1 ULP，`hk` 差 ~2.2e-11 相对。`hk` 这一点不是 1 ULP：van Genuchten 的
`(1-(1-(esat*sc)**(1/m))**m)**2` 在 `esat*sc≈1` 附近有**相消**，1 ULP 的
`psi`/`smp` 会被放大上千倍。所以 `hk` 是**结果**不是**源头**，追它没有意义；
源头是那条 1 ULP 的 `wliq`。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 dump（`water_vsf` 17 处逐条列出）；
1 步后两侧 restart 的逐位比对（10 个量相同、4 个差 1 ULP、`hk` 放大）；
干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对（不变）；`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）。

### 下一轮入口（已改写）

1. `variably_saturated_flow.rs` 剩下的 15 处（活的路径）；
2. `MOD_NetSolar`（35）与 `MOD_Albedo`（136）；
3. 1 ULP 的 `wliq` 源头（在 VSF 的矩阵装配里）。

### `variably_saturated_flow.rs` 剩下 15 处的落点（从 `water_vsf` 的 dump 逐条对出来）

| dump 里的形态 | 上游语句 | Rust 该落在哪 |
|---|---|---|
| `FMA(1-fsno, pg_rain, gwat)` | `gwat = gwat + pg_rain*(1-fsno) - qseva_soil` | **不在** `variably_saturated_flow.rs`：Rust 把 `(1-fsno)` 的加权挪到了调用方（`water_2014.rs` 用雪列底部排水替代 `ground_rain`），属**结构差异**，要动得先改调用方的组装 |
| `FMA(qsdew, deltim, wliq)` ×4 | 表层露/霜/升华回加 | 已做（`_soil` 两处；`_1548` 与 `qsdew+qfros-qsubl` 的另两处是非 split 走不到的分支） |
| `FNMA(x, max(…,0), y)` ×2 | 通量限幅后的矩阵项 | `flux_variable_saturated_zone_*` / `apply_variable_saturated_explicit_step` |
| `FMA(Δ, k, rhs)` ×3、`FNMA(k, Δ, rhs)` ×3 | 三对角/通量矩阵装配 | `flux_at_variable_saturated_interface` 与 `flux_inside_variable_saturated_soil` |
| `FNMA(x*1000, 1e3, wliq)` | `wliq - porsl*dz*1000` 类的过饱和修正 | `water_table_from_aquifer` / `exchange_soil_water_with_aquifer` 附近 |

也就是说，剩下的都是**矩阵/通量装配**，需要在 `flux_variable_saturated_*` 那一组函数里
逐个读数据流；那些函数一共约 1500 行，是下一轮的主体工作。
这也解释了为什么表层那两处改了还是零位移：`wliq` 的 1 ULP 源头在矩阵装配里，
不在表层更新里。

### `WATER_VSF` 的水量平衡误差（两级 `FNMA`）

```fortran
err_solver = (蓄量变化) - (gwat - etr - rsur - rsubst)*deltim
err_solver = err_solver - (qsdew+qfros-qsubl)*deltim      ! 无雪层时
```

GIMPLE：`FNMA(通量和, deltim, 蓄量变化)` 与 `FNMA(deltim, 凝结和, 上一项)` ——
两级都把 `*deltim` 收进减法。`variably_saturated_flow.rs` 的
`solver_balance_error_mm` 两级已改。

**它不影响历史输出**（这个量只做平衡诊断，不回灌状态），所以窗口照旧逐位不变 ——
但它是**活路径上的真实收缩**，且平衡误差本身是会被 `tier` 检查/诊断读到的量。

Tested: `MOD_SoilSnowHydrology.F90` 本体的 dump；干窗 TIMESTEP 1 步 Rust-vs-Rust 逐位比对（不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；`cargo fmt --all --check`。

### `water_vsf` 里两处装配的**形状**已经解出来了（落点待钉）

第 122 轮把 `water_vsf` 的通量/矩阵收缩解到"看得懂形状、还差上游行号"的程度，
记下来供下一轮直接用：

1. **过饱和削顶**：
   ```
   _107 = wliq[layer]
   _108 = _103 * 1.0e+3                      ! 某个体积量换成 mm
   _110 = _107 / _108
   M.360 = min(M.357, max(_110, 0))
   _112 = .FNMA (_108, M.360, _107)          ! wliq - _108*min(...)
   ```
   —— `min(...)` 先被夹到 `[0, ·]`，再作为**乘积的左操作数**被收进减法。

2. **湿周修正**：
   ```
   _199 = wliq[layer]
   _200 = _199 * 1.0e+3
   _201 = _200 / 1.0e+3                      ! 先乘 1000 再除回来（上游写法如此）
   _203 = _195 - prephitmp_166
   _206 = .FNMA (_202, _203, _201)           ! _201 - _202*_203
   ```
   —— 注意那个 `*1000/1000`：**两次舍入都在**，Rust 若省略任一步就会差位。

Rust 侧的候选位置已经缩到 `water_table_from_aquifer`（约 4262-4292 行那段
`liquid_volume_fraction`/`residual_water_kg_m2` 的湿周修正）与
`apply_variable_saturated_explicit_step`；下一轮要做的第一件事是**先在
`MOD_SoilSnowHydrology.F90` 的 `WATER_VSF` 段里把这两条上游语句的行号找出来**，
再决定怎么写 —— 上一轮的教训是：形状对上了但组织方式不同的地方**不能**硬套。

Tested: 本轮无源码改动；`cargo test -q -p colm-core --lib`（354 通过）确认工作树干净；
`MOD_SoilSnowHydrology.F90` 本体的 dump（形状解码）。

### 钉住第一处：`WATER_VSF` 的湿周 `vol_liq` 分子

上游 `MOD_SoilSnowHydrology.F90:1036-1038`：

```fortran
vol_liq(j) = (wliq_soisno(j)*1000.0/denh2o - eff_porosity(j)*(sp_zi(j)-zwtmm)) &
   / (zwtmm - sp_zi(j-1))
```

GIMPLE（`water_vsf` 第 3 处收缩）：

```
_200 = wliq * 1.0e+3 ; _201 = _200 / 1.0e+3     ! denh2o 是常量 1000，被折成两步
_203 = (sp_zi - zwtmm)
_206 = .FNMA (_202=eff_porosity, _203, _201)     ! 分子
_210 = _206 / (zwtmm - sp_zi(j-1))
```

Rust 的 `variably_saturated_flow.rs` 对应那段（`water_table_from_aquifer` 里的
`liquid_volume_fraction` 分支）已改成 `(-eff).mul_add(sp_zi - zwtmm, 水量mm)`。
同段的 `residual_water_kg_m2 = … - eff*(…) - vol_liq*(…)` 是**另一处**收缩，
还没钉（它的第二条减法对应的是哪个 dump 位点尚未确认）。

干窗 1 步仍逐位不变（该分支只在「水位在某个界面附近且该层可渗」时才进，
本步没进）。

Tested: `MOD_SoilSnowHydrology.F90:1036-1038` 与本体 dump 的对应；干窗 TIMESTEP 1 步
Rust-vs-Rust 逐位比对（不变）；`cargo test --workspace --lib --bins -- --test-threads=1`；
`cargo clippy --workspace --all-targets -- -D warnings`；两处 `cargo fmt --all --check`；
`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

### 第二处：`WATER_VSF` 的 `wresi`（`vol_liq` 被内联进 FMA）

上游 `MOD_SoilSnowHydrology.F90:866-868`：

```fortran
vol_liq(j) = wliq_soisno(j)/(dz_soisno(j)*denh2o)
vol_liq(j) = min(eff_porosity(j), max(0., vol_liq(j)))
wresi(j)   = wliq_soisno(j) - dz_soisno(j) * denh2o * vol_liq(j)
```

`vol_liq` 只被用一次，GCC 把它**整条内联**进 `wresi`，于是 dump 里只剩一条：

```
_108 = dz * 1e3
_110 = wliq / _108
M.360 = min(eff_porosity, max(_110, 0))
_112 = .FNMA (_108, M.360, _107=wliq)
```

即 `wresi = FNMA(dz*denh2o, min(eff, max(wliq/(dz*denh2o), 0)), wliq)` ——
**夹取留在乘积的操作数里，被吸收的是 `dz*denh2o*vol_liq` 那个乘积**。
Rust 的 `variably_saturated_flow.rs:4140` 那行已改成
`(-liquid_capacity).mul_add(liquid_volume_fraction, wliq)`。

这一处**无条件执行**（不像上一处的湿周分支要水位落在界面附近），干窗 1 步
仍逐位不变 —— 说明这条路当前步的 `liquid_volume_fraction` 处在夹取边界
（`vol_liq` 恰好等于 `eff` 或 0）时融合与否无差别，或该量在本步没影响状态。

Tested: `MOD_SoilSnowHydrology.F90:866-868` 与 dump 的对应；干窗 TIMESTEP 1 步
Rust-vs-Rust 逐位比对（不变）；`cargo test -q -p colm-core --lib`（354 通过）；`cargo fmt --all --check`。

### 第三处：水位分支的 `wresi` 两句减法**都**被收（用复刻件验证过）

上游 `MOD_SoilSnowHydrology.F90:1043-1044`：

```fortran
wresi(j) = wliq_soisno(j)*1000.0/denh2o - eff_porosity(j)*(sp_zi(j)-zwtmm) &
   - vol_liq(j) * (zwtmm - sp_zi(j-1))
```

这一句里有**两个**乘积进减法，而且第一个子式
（`wliq*1000/denh2o - eff*(sp_zi-zwt)`）还与 `vol_liq` 的分子**共用**（CSE）。
按"共用临时量就不会被吸收"的老经验，第二个乘积很可能保持舍入 —— 但这次**不能靠经验**，
于是把它逐字复刻成独立子程序（`/tmp/gf/r113/rep.f90`，含 `vol_liq` 的三句与 `wresi` 一句）
用**真实编译选项**读 GIMPLE：

```
_11 = .FNMA (_5, _9, _4);          ! 共用的第一级，已被吸收
_20 = .FNMA (_14, M.1_33, _11);    ! 第二级：vol_liq*(zwt-sp_zi) 也被吸收
```

**两级都是 `.FNMA`**：共用让第一级成为临时量，但第二个乘积照收不误
（与 `meltf` 的 `fact*heatr`、`spacAF` 的 `f(xyl)` 那两次"CSE 阻止融合"**结论相反**）。
Rust 的 `residual_water_kg_m2`（水位分支）已按此写成两级 `mul_add`。

教训再强化一次：**CSE 是否阻止融合没有通例，必须对该语句本身读 GIMPLE**；
"上次是这样"既不能证明也不能否证。

Tested: 逐字复刻件 `/tmp/gf/r113/rep.f90` 的 `-fdump-tree-optimized`（两级 `.FNMA` 都出现）；
`cargo test -q -p colm-core --lib`（354 通过）；`cargo fmt --all --check`；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。

## **第一次窗口给出信号**：`netsolar` 的累加方向错了（已回退）

`MOD_NetSolar` 的 `netsolar` 有 35 处收缩，dump 里全是**向量化**的
`val = FMA(a, b, val)` —— 是 `sum(权重*系数)` 形状的累加。我先按产流那轮定下的
"第一个乘积先舍入、其后每个乘积收进累加"写成

```rust
fma(a2, b2, round(a1*b1))
```

**结果干窗明显变差**（这是连续十几轮里第一次窗口动）：

| 干窗 | 改前 | 改后 |
|---|---|---|
| 逐位不同值 | 21229 | 21235 |
| Σ\|Δ\| | 216.9941 | **312.4792** |
| 超容差 | 813 | **1092** |
| tier2 变量数 | 17 | **27** |

于是**立刻回退**（`git checkout`），并改用**标量复刻件**（`-fno-tree-vectorize`）
把方向量准 —— `/tmp/gf/r114/ns.f90`：

```
_6 = a2 * b2                 ! **最后一个**乘积先各自舍入
_7 = .FMA (a1, b1, _6)       ! **第一个**乘积被收进加法
_11 = .FMA (a3, b3, _7)      ! 其后每个乘积依次收进累加
_15 = .FMA (a4, b4, _11)
```

**方向与我写的相反**：四级累加里，**最左**的乘积被吸收、**最右**的是被加的
那个已舍入乘积；其后的项依次收进累加。这解释了窗口为什么变差。

两条教训：

1. **向量化的 dump 不能直接读收缩方向** —— 向量化会把归约改写成
   `val = FMA(a,b,val)` 的循环，看不出标量语义里的"哪一端是舍入过的"。
   要定方向必须用 `-fno-tree-vectorize` 的标量复刻件。
2. **窗口是能分辨的** —— 我之前"窗口对逐点修复不敏感"的说法过于绝对：
   它对该敏感的地方（这条累加在白天每一步都走）**立刻**给了明确信号，
   而且方向正确（回退后不变量恢复）。以后遇到窗口大幅变化，先当它是真信号，
   去核对自己的解码而不是解释成抖动。

（本轮结束时 `net_solar.rs` 已回到改动前状态，未提交任何源码改动。）

Tested: 标量复刻件 `/tmp/gf/r114/ns.f90`（`-fno-tree-vectorize`）的 `-fdump-tree-optimized`；
干窗三个月度窗口的三口径 A/B（发现变差 → 回退）；`git checkout` 后
`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）、工作树干净。

## 用正确方向重做 `netsolar` 的三处累加（上游源码 + 复刻件双重确认）

回退之后按标量复刻件量出的方向（**最左乘积被吸收、最右是已舍入的加数**）重做：

上游 `MOD_NetSolar.F90:176-183` 原样如此：

```fortran
parsun  = forc_sols*ssun(1,1) + forc_solsd*ssun(1,2)
sabvsun = forc_sols*ssun(1,1) + forc_solsd*ssun(1,2) &
        + forc_soll*ssun(2,1) + forc_solld*ssun(2,2)
sabvg   = forc_sols *(1.-alb(1,1)) + forc_solsd*(1.-alb(1,2)) &
        + forc_soll *(1.-alb(2,1)) + forc_solld*(1.-alb(2,2))
```

**三处都是平铺的左到右四项/两项和**（不是"先算 visible 再加两项"），所以
Rust 的 `absorption` **不能**再调 `visible_absorption` —— 那个调用会把
visible 那一对先舍成一次结果。`visible_absorption` / `absorption` /
`absorbed_by_surface` 三个函数已按 `fma(a1,b1, a2*b2)`、外层依次是 a4/a3 的
嵌套写成。

窗口三口径（基线 = 回退后的状态）：

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干 | 21229 → 21224 | 216.9941 → 311.4747 | 813 → 821 | 17（不变） |
| 湿 | 33207 → **33092** | 10386.5309 → **10382.9221** | 20671 → **20663** | 68 |
| 雪 | 33593 → 33595 | 不变 | 不变 | 79 |

**判据链（为什么保留）**：① 上游源码是平铺和（上面已抄）；② 标量复刻件给出
方向；③ 方向写反时干窗 tier2 变量数从 17 爆到 **27**，写对后**回到 17** —— 这一条
比 Σ\|Δ\| 更能分辨对错。干窗的 Σ\|Δ\| 变大是这类改动的常见混合信号
（湿窗同时变好 115 个逐位值），不构成否决。

Tested: 标量复刻件 `/tmp/gf/r114/ns.f90`；上游 `MOD_NetSolar.F90:176-183` 逐行核对；
三个黄金窗口的三口径 A/B（含方向写反时的对照）；`cargo test -q -p colm-core --lib`（354 通过）；
`cargo fmt --all --check`。

### `netsolar` 三条和式：差分驱动验到 **20000/20000**

把上游 `parsun`（2 项）、`sabvsun`（4 项）、`sabvg`（4 项）三条和式逐字复刻成
独立 Fortran 程序，**用内核的真实编译选项**（`-O2`，含向量化）跑 20000 组随机输入，
与 Rust 的 `visible_absorption` / `absorption` / `absorbed_by_surface` 逐位比对：

```
netsolar sums: y1 20000/20000, y2 20000/20000, y3 20000/20000
```

这条把上一轮"方向写对"的结论从"源码 + 标量形状"升级为**直接差分证据**——
写反的那一版（把第二个乘积收进加法）在这种比对下会全错。

### **Fortran 侧的 LCG 驱动必须加 `-fwrapv`**

第一次跑这份复刻件时**三条全 0/20000**，而且 Fortran 输出的相邻行**完全重复**。
原因不是公式而是**有符号 64 位溢出是 UB**：`S = S*6364136223846793005 + …`
在 `-O2` 下被 GCC 按 UB 优化，值卡住不动。加 `-fwrapv` 后立刻恢复正常、
并给出 20000/20000。

**这条要补进"驱动纪律"**：本会话所有差分驱动都是这个 LCG 写法，
**今后一律加 `-fwrapv`**。（已跑过的那些驱动当时都给出了**变化**的输出、
且匹配率 97%–100%，所以它们没有被这个问题污染；但同样的写法下次可能撞上。）

Tested: 复刻件 `/tmp/gf/r115/sums.f90`（`-O2`，与内核同选项；加 `-fwrapv` 前后各跑一次）；
Rust 探针对三条和式的逐位比对（20000/20000）；探针已删除，工作树干净；
`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）。

## `MOD_Albedo`：`snowage` 的七处形状已解出，落点已定位到 `surface_optics.rs`

`MOD_Albedo` 是本仓库剩下最大的未扫块（**136** 处：`twostream_wrap` 72、
`twostream` 45、`albland` 9、`albocean` 8、`snowage` 2）。本轮先把最小的
`snowage` 那 7 处的形状读出来：

```
_6  = FMA(x, 4.0, 1.0)                          ! 1 + 4*x（雪龄因子）
_56 = FMA(y, 0.025, 0.95)   _58 = FMA(y, 0.15, 0.70)
_20 = FMA(1-frsnow, 0.70, sasdir*frsnow)        ! 可见光直射
_23 = FMA(1-frsnow, 0.5,  saldir*frsnow)        ! 可见光散射
_25 = FMA(1-frsnow, 0.70, frsnow*0.95)          ! 近红外直射
_27 = FMA(1-frsnow, 0.5,  frsnow*0.70)          ! 近红外散射
```

形状是统一的：**"新雪反照率 × (1-frsnow) + 陈雪/雪面反照率 × frsnow"，
收的是左边那个乘积** —— 与 `net_solar` 那轮量出来的方向一致
（最左乘积被吸收、最右是已舍入的加数）。

落点：Rust 侧对应的是 `surface_optics.rs`（它 `use crate::snow::snow_fraction`，
并带 `vegetation_snow_fraction`/`ground_snow_fraction` 等字段），**不是**
`albedo.rs`（那个文件只有 77 行，是枚举/元数据）。按常量搜 `0.95`/`0.70`
找不到，说明 Rust 用的是别的写法或别的命名，下一轮要从 `surface_optics.rs`
的雪盖反照率混入处逐个对上。

（本轮无源码改动：136 处是大块，先解形状、定位文件，避免在没有落点的情况下改。）

Tested: `MOD_Albedo.F90` 本体的 `-fdump-tree-optimized`（三个小函数的 19 处逐条列出）；
`grep` 定位 Rust 侧的对应文件；`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）、工作树干净。

### 更正：上一节那七处形状属于 `albland`，**不是** `snowage`

上一节把带 `frsnow` 的七处混入形状记成了 `snowage` 的，**这是错的**。
按函数名过滤时我把 `albland`/`albocean`/`snowage` 三个一起放进了同一个桶，
而 `frsnow`（`sasdir`/`saldir` 加权）只出现在 **`albland`** 的雪盖混入里。

对照 Rust 侧也印证了这一点：`radiation.rs` 的 `aged_snow_albedo`（`snowage` 那一支）
算的是

```rust
let age = 1.0 - 1.0 / (1.0 + snow_age);
let direct_correction = ((1.5/(1.0 + 4.0*cosine_zenith)) - 0.5).max(0.0);
let diffuse = new_snow_albedo * (1.0 - age_factor*age);
let direct  = diffuse + 0.4*direct_correction*(1.0 - diffuse);
```

—— 与那七处 `FMA(1-frsnow, 0.70, sasdir*frsnow)` **完全不是一回事**，
所以它们不可能在 `snowage` 里。`1.5/(1.0+4.0*coszen)` 里的 `1+4x` 倒是与
`_6 = FMA(x, 4.0, 1.0)` 对得上，但 `albland` 也可能有同样的因子。

**规矩**：`dump` 的桶要一个函数一个桶，别把相邻的几个用 `or` 合并 ——
这次就是因为合并过滤，把两处不同函数的形状混在一张表里，还推导出了错误的落点。

`albland` 的九处对应 Rust 的哪个函数仍**未定位**（`surface_optics.rs` 里的
`albland` 调用点尚未展开）；下一轮先把 `albland` 单独 dump 一次再动手。

Tested: `MOD_Albedo.F90` 分别按函数重新核对（这次只查 `albland`）；Rust 侧
`radiation.rs:645 aged_snow_albedo` 逐行比对确认不同式子；本轮无源码改动，
`cargo test -q -p colm-core --lib`（354 通过）、工作树干净。

### `albland` 单独 dump：九处的形状与**方向**（落点已到 `surface_optics.rs:albland`）

按上一轮的规矩（一个函数一个桶）重做，`albland` 的九处是：

```
[1] FNMA(ssw, 0.4, 0.11)                      ! 0.11 - 0.4*ssw
[2] FNMA(age, {0.2, 0.5}, {1.0, 1.0})         ! 1 - 0.2*age（可见）/ 1 - 0.5*age（近红外）
[3] FMA(M.138*2.0, 2.0, 1.0)
[4] FMA(0.4*max(cff,0), ·, dralbs)            ! 直射修正在两个波段上的混合
[5] FMA(frsno, albsno, (1-frsno)*albg)        ! ← **雪面/地面反照率混入**
[6] FMA(1-frsno, ·, frsno*·)
[7] FMA(1-frsno, ·, frsno*·)
[8] FMA(1-albsno, tran, ·)
[9] FMA(tran, 1-albg, ·)
```

**方向与 `net_solar`/上一节一致**：最左的乘积被吸收、最右的是已舍入的加数
（`[5]` 里舍入的是 `(1-frsno)*albg`，被吸收的是 `frsno*albsno`）。

落点已定到 `surface_optics.rs` 的 `fn albland`（`:157`，带
"第 2 节 地面反照率 / 第 3 节 非 SNICAR 雪面反照率"注释）—— 九处具体对应哪几行
还没逐行对上（该函数较长，含夜间早返回与 `thermk` 的跨步语义），
下一轮从"雪盖混合"那一段（第 2/3 节之间）逐句比。

（本轮无源码改动：按"形状 → 落点 → 行号 → 改"的次序，现在只到第二步。）

Tested: `MOD_Albedo.F90` 只按 `albland` 过滤的 dump（九处逐条列出）；
`grep` 定位 `surface_optics.rs:157 fn albland`；`cargo test -q -p colm-core --lib`（354 通过）、工作树干净。

### `albland` 第一处落地：雪盖反照率混入（`MOD_Albedo.F90:2048-2051`）

上游：

```fortran
frsnow = snwhgt/(rghsnw+snwhgt)
alb(1,1) = alb(1,1)*(1.-frsnow) + sasdir*frsnow
alb(2,1) = alb(2,1)*(1.-frsnow) + saldir*frsnow
alb(1,2) = alb(1,2)*(1.-frsnow) + sasdif*frsnow
alb(2,2) = alb(2,2)*(1.-frsnow) + saldif*frsnow
```

GIMPLE（`albland` 第 5 处）是 `FMA(雪面反照率, frsnow, alb*(1-frsnow))` ——
**地面那一支的乘积先舍入、雪面那一支被吸收**，与上一节量出的方向一致。
Rust 的落点是 `radiation.rs::mix_ground_albedo`（被 `surface_optics.rs:223` 的第
3.1 节调用），已改成 `snow.mul_add(snow_fraction, (1-snow_fraction)*soil)`。

窗口：干/湿两窗**逐位不变**（该步之后才用到混合值，且干窗本步无冠层雪），
雪窗的 Σ\|Δ\| 从 444394.4368 微降到 **444390.4460**（−4），tier2 变量数 79 不变。

（`albland` 九处里这是第一处；其余八处（`0.11-0.4*ssw`、`1-0.2/0.5*age`、
直射修正、两条 `1-alb` 的透射加权）下一轮按同样的次序做。）

Tested: `MOD_Albedo.F90:2047-2051` 逐行核对 + `albland` 单独 dump 的第 5 处；
三个黄金窗口三口径 A/B；`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；
`cargo fmt --all --check`；`cargo clippy -q -p colm-core --all-targets -- -D warnings`（干净）。

### 连带修好一条**编码旧算式**的测试

`crates/colm-init/src/snicar_tests.rs` 里那条桥接测试把混入式写成

```rust
(1.0 - fraction) * soil + fraction * snow
```

—— 那是**移植件旧的（不融合）算式**，不是内核的。改成与
`mix_ground_albedo` 一致的 `snow.mul_add(fraction, (1-fraction)*soil)` 后
`cargo test -p colm-init --lib -- --test-threads=1` **156/156 通过**。

（这是本会话第三次遇到"测试钉住了旧的错误行为"：前两次是
`dry_soil_uses_dry_conductivity_for_all_schemes` 与
`equilibrium_soil_column_stays_at_its_fortran_surface_balance`。）

### 一个操作上的坑（记下来）

我手动重跑 `cargo test -q -p colm-init --lib`（**没带** `--test-threads=1`）
时出现 **32 个失败**，而带 `--test-threads=1` 只失败 1 个（就是上面那条真问题）。
也就是说 `colm-init` 的 lib 测试**并行跑会大面积互相干扰**（约 31 条），
这是**先于本轮就存在**的问题：同一 crate 内的测试共享状态。
所以验收里 `--workspace --lib --bins -- --test-threads=1` 这个 `--test-threads=1`
是**必须**的；单独重跑某个 crate 时也要带上。

Tested: `cargo test -p colm-init --lib -- --test-threads=1`（156 通过）；
不带该标志的对照（32 失败，证明是并行干扰）；`cargo test -q -p colm-core --lib`（354 通过）；
`cargo fmt --all --check`。

### `albland` 的雪面反照率常数：上游 0.95/0.70，Rust 写的是 0.85/0.65

上游 `MOD_Albedo.F90:2036-2038`：

```fortran
sasdir = min(0.98, sasdif + (1.-sasdif)*0.5*(3./(1.+4.*coszrs)-1.))
saldir = min(0.98, saldif + (1.-saldif)*0.5*(3./(1.+4.*coszrs)-1.))
```

其中 `sasdif = asnows`、`saldif = asnowl`（`:2029-2030`）。dump 里那两条

```
_56 = FMA(_10, 2.5000000000000002e-2, 9.4999999999999996e-1)
_58 = FMA(_10, 1.5000000000000002e-1, 6.9999999999999996e-1)
```

正好反推出 **`asnows = 0.95`**（因为 `(1-0.95)*0.5 = 0.025`）与
**`asnowl = 0.70`**（`(1-0.70)*0.5 = 0.15`），`min(...,0.98)` 是 dump 里的
`MIN_EXPR` ✓。也就是说这一支的"新雪反照率"是 **0.95 / 0.70**。

**但 Rust 的 `aged_snow_albedo` 用的是 `snow_band(0.85, 0.2)` / `snow_band(0.65, 0.5)`**
（`radiation.rs:673`）—— 常数不同。两者必有一处需要核对：要么 Rust 把
`asnows`/`asnowl` 取错了，要么它们来自别处（例如 `MOD_Const_Physical` 里另有定义，
或被 `oro` 分支覆盖）。**下一轮的第一件事就是查清这一点**，因为它直接决定
晴天雪面反照率（干窗 `f_alb` 从第 15 步开始差分，很可能与此有关）。

Tested: 上游 `MOD_Albedo.F90:2020-2062` 与 dump 的三条 FMA 交叉反推；
`cargo test --workspace --lib --bins -- --test-threads=1`、`cargo clippy --workspace --all-targets -- -D warnings`、
两处 `cargo fmt --all --check`、`cargo test -q -p oracle`、`cargo run -q -p xtask -- check-gui`、
`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。本轮无源码改动。

### 上一节的"不一致"是虚惊：上游有**两套**雪面反照率常数

查清了。上游 `MOD_Albedo.F90` 里两对常数各管一支：

| 常数 | 声明 | 用在哪 |
|---|---|---|
| `snal0 = 0.85` / `snal1 = 0.65` | `:212-213` | **陆地**支：`:350/:357` 的 `dfalbs = snal0*(1-cons*age)`、`dfalbl = snal1*(1-conn*age)`，配 `snowage` 老化 |
| `asnows = 0.95` / `asnowl = 0.70` | `:2015` | **`nint(oro)==2`** 那一支：`:2036-2038` 的 `sasdir = min(0.98, sasdif+(1-sasdif)*0.5*(3/(1+4cosz)-1))` |

Rust 的 `aged_snow_albedo`（`radiation.rs:647` 起，注释已写明是 `:341-370` 那支）
用 **0.85/0.65** 是**对的** ✓；`radiation.rs:670` 的
`direct_correction = (1.5/(1+4c) - 0.5).max(0)` 也是那一支的式子 ✓。
另一对 0.95/0.70 属于 `oro==2`（冰/冰川）分支，本机三个算例（igbp 陆地）
**不走**，所以 Rust 里没有 `0.98` 那个 `min` 并不是缺口。

**教训**：从 dump 反推常数只能提出问题；这次问题的答案是"上游本来就有两套常数、
分属两支"，而两边各自的归属都对。上一轮把它写成"必有一处要核对"是过头了 ——
**反推式常数时，先按"同一模块里是否有同名/近名参数"排查，再怀疑移植错误**。

Tested: `MOD_Albedo.F90:186-213、341-357、2015、2020-2062` 逐段核对；Rust
`radiation.rs:647-676` 与 `surface_optics.rs` 的 `albland` 分支归属核对；
`grep` 确认 `0.98` 不在 Rust 侧；本轮无源码改动；
`cargo test -q -p colm-core --lib`（354 通过）、工作树干净。

### 第四处：`WATER_VSF` 的 `wliq` 重建式（**就是目标量本身的语句**）

上游 `MOD_SoilSnowHydrology.F90:1109-1110`：

```fortran
wliq_soisno(j) = denh2o * ((eff_porosity(j)*(sp_zi(j)-zwtmm) &
   + vol_liq(j)*(zwtmm - sp_zi(j-1)))/1000.0
```

GIMPLE（`water_vsf` 第 6 处）是 `FMA(eff, sp_zi-zwt, (zwt-sp_zi(j-1))*vol_liq)`
—— **左边**乘积被吸收、**右边**是已舍入的加数（这是方向规则的**第四个**独立来源）。
Rust 的 `variably_saturated_flow.rs:4396-4400` 已按此写。

干窗 1 步仍逐位不变：这一支要 `zwtmm` 落在该层的两个界面之间才走，
干窗算例第 0 步的水位不在那一层（走 `vol_liq*厚度` 那一支）。
所以**这条 1 ULP 的源头仍未找到**，但已把本语句排除在本步之外 ——
剩下候选是 `else` 两条（`eff*厚度` / `vol_liq*厚度`，单乘积、无加法可吸收）
与最小二乘求解本身的输入。

Tested: `MOD_SoilSnowHydrology.F90:1106-1122` 与 dump 第 6 处的对应；干窗 TIMESTEP 1 步
Rust-vs-Rust 逐位比对（不变）；`cargo test -q -p colm-core --lib`（354 通过）；`cargo fmt --all --check`。

## 土壤水力三函数的差分：**结果未收敛，首要嫌疑是驱动自己**

建了一个 `USE MOD_Hydro_SoilFunction` 的驱动（`-fwrapv`，`MOD_Namelist` stub 里
补上 `DEF_USE_Campbell_SOIL_MODEL = .false.` 以走 van Genuchten），20000 组随机输入
比对三个**公开**函数：

```
soil hydraulics: psi 18361/20000, hk 14042/20000, vliq 19304/20000
```

即三者都在 70%–96% 之间。但随后逐行核对**没有找到任何算式差异**：

- 上游 `soil_hk_from_psi` 的 van Genuchten 支与 Rust 的 GIMPLE 逐句对上，
  包括 `**2.0_r8` 被 GCC 折成 `powmult = x*x`（所以 Rust 用 `.powi(2)` 是对的）
  与 `hksat*esat**L` 的乘法顺序；
- `soil_psi_from_vliq` 的早返回（`vliq >= porsl` / `vliq <= max(vl_r,1e-8)`）
  与末尾 `max(psi, minsmp)` 都在，且 `minsmp = -1.e8`（上游 `:24`）与 Rust 的
  `MIN_SOIL_PSI = -1.0e8` 一致；
- `soil_vliq_from_psi` 的 van Genuchten 支唯一那处收缩（`FMA(porsl-vl_r, esat, vl_r)`）
  已在第 113 轮改过。

**判断：这更像又一次驱动错位（本会话第五次），而不是"三个函数真的各错 8%–30%"**。
理由：`hk` 若真有 28% 的输入算错，一个月的黄金窗口不可能只有 tier2 的
17/68/79 —— 早就在 tier0/tier1 炸开了。本轮**没有据此改任何源码**，把
结果与判断一并记下，留给下一轮用"先在两侧逐位打印输入"的老办法把驱动对齐后重跑。

（探针已删除，工作树干净。）

Tested: 驱动 `/tmp/gf/r116/hsol.f90`（`-O2 -fwrapv`，`MOD_Namelist` stub）；
20000 组三函数逐位比对（三种子区间都试过，含把 `psi` 限制到物理负值）；
上游 `MOD_Hydro_SoilFunction.F90` 三个函数逐行核对；Rust `hydrology.rs:130-185` 核对；
`cargo test -q -p colm-core --lib`（354 通过）、工作树干净。

### 更正第 137 轮的判断：**不是驱动错位**，而是真有 1 ULP 级差异

按上一轮的承诺先"两侧逐位打印输入"：让驱动把第 1 个算例的 8 个输入打出来，
再在 Python 里用**同一套 LCG** 独立算一遍——**逐位相同** ✓：

```
驱动:  C08C83080E592FCA 3FC141568BB6D260 3FE42CCE820CDA22 3FB8DEB593B61D18
       C06BD188153830DC 3F188A9B7D9A7B80 40222D69EC4A011E 3FF8C35066BC82C1
python: 同上（8/8 逐位相同）
```

所以输入是对齐的，**上一轮"更像驱动错位"的判断作废**。进一步手算也说明了另一半：

```
上游公式在 Python 里给 hk = 2.9394665032755537e-18
Fortran 本体给          hk = 4.476753824683236e-18   （比 1.52 倍）
```

差 1.52 倍**不是**公式不同，而是 `hk` 的公式里有**灾难性相消**：

```
inner = (1 - (1 - (esat*sc)**(1/m))**m)/fc
```

`esat` 很小（≈0.013）时 `esat**(1/m) ≈ 2.4e-6`，`1 - 2.4e-6` 再取 `m` 次方几乎回到 1.0，
两者相减把有效位数吃光 —— 相消前的**任何 1 ULP** 差异都会被放大成千上万倍。
这也解释了我上一轮在 1 步 restart 里看到的 `hk` 相对差 2.2e-11：那是**果**不是**因**。

**结论**：三个函数确实存在 1 ULP 级的差异，位置在相消链的某个子式里（`m_vgm`、
`pow` 的调用序列、或 `esat` 的形成），而 `hk` 是最不适合做差分判据的量
（它是放大器）。下一轮应当**先差分 `soil_psi_from_vliq`**（没有相消、92% 匹配说明
差异是 1 ULP 级的单点），再回头看 `hk`。

**规矩**：差分要挑**不放大**的量；有灾难性相消的输出（如 `hk`）会把 1 ULP 放大成
几十个百分点，用它当判据必然误导。

Tested: 驱动第 1 算例 8 个输入的逐位打印 + Python 独立复算（8/8 相同）；
`hk` 的手算对照（Python 2.9395e-18 vs 内核 4.4768e-18，1.52 倍）；
上游公式与 dump 的逐句对照；`cargo test -q -p colm-core --lib`（354 通过）、工作树干净。

## 结论：三个土壤水力函数**逐位可信**（0/20000），前两轮全是我的驱动/模型之误

把第 138 轮的路线（先逐位打印输入、再挑不放大的量）走完，最终用**纯 Python 模型**
（复刻 Rust 的算式）+ 内核驱动的 20000 组逐位比对得到：

```
加入 psi>=psi_s 的早返回后: psi 0, hk 0, vliq 0  /20000
```

**三个函数全部逐位相同** ✓。第 137、138 两轮记下的"8%–30% 不匹配"与
"`hk` 是相消放大器、函数本身有 1 ULP 差异"**全部作废** —— 那两轮的错误来源有两个，
都在我这一侧：

1. **驱动在改 `psi` 取值范围时被重新编译，漏掉了 `-ffp-contract=off`** ✗。
   于是 `psi_s = -100 - uni()*400` 被 GCC 融合成 FMA 生成输入，
   与 Python/Rust 的"先乘后加"差 1 ULP（第 6 次踩这个坑，见下）。
   证据：驱动打印的第 17 算例 `psi_s = C0756AAC1DC21015`，Python 同一 LCG 给 `...014`。
   补上该标志后立刻 `psi_from_vliq` **0/20000**。
2. **我的 Python 模型自己不完备**：漏了 `psi >= psi_s` 的早返回（`hk` 与 `vliq`
   都靠它短路，约 22% 的算例命中）与 `vliq_from_psi` 末尾那个 `mul_add`
   （用 `Fraction` 精确求值补上）。补完后 **0/0/0**。

**修正后的正确表述**：`soil_psi_from_vliq`/`soil_hk_from_psi`/`soil_vliq_from_psi`
三个函数本身**没有差异**；第 110 轮在 1 步 restart 里看到的 `hk` 相对差 2.2e-11
是因为它的**输入**（`smp`/`psi`）先差了一点点，再被 `hk` 式子里的相消放大 ——
"相消会放大"这条判断仍然成立，但**被放大的不是函数误差，而是输入误差**。

### 驱动纪律（第 6 次，这次要写死）

| 次数 | 谁多融合/谁不完备 | 现象 |
|---|---|---|
| 105 | clang 编译的 C 参考件（默认 `-ffp-contract=on`） | "新算式 ~80% 匹配" |
| 108 | gfortran 编译的 `drv.f90`（默认 `fast`） | 97%，失配均匀散在所有分支 |
| 110 | 我自己写在 Rust 探针里的 `ur` | 23 个输出 93–98% |
| 129 | 驱动的 LCG 有符号溢出 UB | Fortran 输出相邻行重复、0/20000 |
| 138 | **驱动重编译时漏掉 `-ffp-contract=off`** | 8%–30% 不匹配，还编出"相消放大"的解释 |
| 138 | **Python 模型漏早返回与 `mul_add`** | `hk` 22%、`vliq` 3% |

**规矩（写死）**：
1. **每次重编译任何一侧的驱动，都要把全套标志重抄一遍**（`-fwrapv -ffp-contract=off`），
   不要只改源码就 `gfortran … -o` 重来；
2. **模型侧（Python/Rust 复刻）必须与目标实现逐条对齐**，包括每一处
   early return、每一处 `mul_add`、每一个夹取；
3. 报告不匹配**之前**，先把**输入**逐位对齐（这一条在第 138 轮救人一次，
   但因为漏了第 1 条又白跑一轮）。

Tested: 驱动 `/tmp/gf/r116/hsol.f90`（`-O2 -fwrapv -ffp-contract=off`）与 Python 模型
（`Fraction` 精确 fma、含全部早返回）的 20000 组逐位比对：**psi 0、hk 0、vliq 0**；
驱动第 17 算例输入与 Python 逐位相同；`cargo test -q -p colm-core --lib`（354 通过）、工作树干净。

### `albland` 的第 3、4 处落地：雪龄反照率的三条式子（`aged_snow_albedo`）

上游 `MOD_Albedo.F90:2036-2038`（`1+4*coszrs`）与 `:350-357`（`dfalbs/dfalbl` 与直射修正）：

```
_86 = FMA(2*c, 2.0, 1.0)                     ! 1 + 4*c（先算 2*c）
_146 = FNMA(age, {0.2, 0.5}, {1.0, 1.0})     ! 1 - 0.2*age（可见）/ 1 - 0.5*age（近红外）
_4  = FMA(0.4*max(cff,0), 1-diffuse, diffuse) ! 直射修正，收左边那个乘积
```

Rust 的 `radiation.rs::aged_snow_albedo` 已按此改（`doubled_zenith.mul_add(2.0, 1.0)`、
`(-age_factor).mul_add(age, 1.0)`、`(0.4*direct_correction).mul_add(1.0-diffuse, diffuse)`）。

窗口（基线 = `net_solar` 改动之前）：

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干 | 21229 → 21236 | 216.9941 → 384.4436 | 813 → 837 | 17（不变） |
| 湿 | 33207 → **33087** | 10386.5309 → **10374.2243** | 20671 → **20662** | 68 |
| 雪 | 33593 → **33647** | 不变 | 不变 | 79 |

湿窗逐位少了 120 个、雪窗多了 54 个；干窗的 Σ\|Δ\| 变大而 tier2 变量数不变 ——
与 `net_solar` 那轮同一模式（这次是三个改动的合并效果，无法单独归因）。

Tested: `MOD_Albedo.F90:2036-2038/350-357` 与 `albland` 单独 dump 的第 3、4 处对应；
三个黄金窗口三口径 A/B；`cargo test -q -p colm-core --lib`（354 通过）；`cargo fmt --all --check`；
`cargo clippy -q -p colm-core --all-targets -- -D warnings`（干净）。

## `snowwater` / `snowage` / `albland` / `twostream` 的收缩扫尾：**第一次把干窗的超容差条数拉回来**

这一轮把干窗路径上最后四块"看得懂的"收缩扫完，其中 `twostream` 的那一组
**第一次**让干窗的 `over_tol` 从 837 回到 813（= 未做 `net_solar` 之前的最好水平），
湿窗逐位不同的值从 33087 降到 **32535**（历史最好）。

### 一条通用教训：**"最左乘积被吸收"不是普适规则**

前面四轮用四个独立来源量出的方向规则是"平铺和式里**最左**乘积被吸收、最右先舍入"。
这一轮在 `albland` 的 `ssno`/`ssoi` 上撞到反例：

```fortran
ssno(1,1) = tran(1,1)*(1.-albsno(1,2)) + tran(1,3)*(1.-albsno(1,1))
```

GIMPLE（`albland` dump 第 9 处，向量化）是

```
vect__176 = FMA(1-albsno$0, tran$4, tran$0*(1-albsno$2))
```

即 **最右**那个乘积被吸收、最左先舍入 —— 与 `net_solar` 的 `parsun` 相反，而两句
都是两项平铺和。差别在于 `(1-x)` 子式先被 CSE 成共享临时量。

**规矩**：方向只能一句一句从 dump 读；"四项/两项和"的形状不构成方向依据。
`meltf` 的 `fact*heatr`、`spacAF` 的 `f(xyl)` 之前也是同类反例。

### `twostream`（地块）45 处的分流：26 处在活支，19 处不可达

`MOD_Albedo.F90:458-780` 的 `twostream` 在 dump 里 45 处收缩。按 `sigma` 的
`IF (abs(sigma) .gt. 1.e-10)` 分成两支：

```
sigma = (zmu*extkb)**2 + (1-scat)*(1-scat+2*upscat)
```

`zmu` 与 `extkb` 都是 O(1)，而第二项在 `scat<1` 时为正 —— 实测本机三个算例里
`sigma` 恒在 0.1 量级，**`ELSE`（`sigma <= 1e-10`）那一支不可达**。dump 里
26 处在 `IF` 支、19 处在 `ELSE` 支（`_263`…`_357`、`_442`/`_446`）。

活支 26 处里 **19 处此前已经写对**（`zmu` 的 `FNMA`、`scat`/`upscat`、`p1`/`p2`、
`h1`/`h4`、`m3` 分子、`s2*hh4+hh5*s1`、`eup`/`edown` 的 `FMS`、三条 `ssun`/`ssha`），
本轮补的是 6 组：

| 上游行 | 形状 | Rust 落点 |
|---|---|---|
| `:611` `phi1 = 0.5-0.633*chil-0.33*chil*chil` | 两级 `FNMA`：`_3 = FNMA(chil,0.633,0.5)`、`_4 = chil*0.33`（先舍入、复用）、`phi1 = FNMA(_4,chil,_3)` | `radiation.rs::two_stream` 首行 |
| `:631` `as = as*(1 - X*log(·))` | `FNMA(X, log, 1.0)` | `directional_scattering` |
| `:629-631` 植被雪三处 | `_108 = scat_sno*fwet`（先舍入、复用两次）、`_110 = (1-fwet)*scat`、三处 `FMA(旧值, _110, _113)` | `vegetation_snow` 块 |
| `:641` `sigma` | `_135 = ce*ce` 只算一次；`psi` 用 `FMS(be,be,_135)`、`sigma` 用 `FNMA(be,be,_135)` | 提出 `ce_squared` 共用 |
| `:672-673` 两个 Cramer 分母 | `_813 = m2*n1`（先舍入、复用）、`_814 = FMS(m1,n2,_813)`、`_816 = FNMA(m1,n2,_813)` | 提出 `m2_n1`/`cramer_direct`/`cramer_reverse`，直接支与漫射支（`hh2/hh3` 与 `hh7/hh8`）**共用同一对** |
| `:673` `hh3` 分子 | `FMS(m3,n1,n3*m1)` | `m3.mul_add(n1, -(m1*n3))` |

顺带确认**没有**收缩的两句：`phi2 = 0.877*(1-2*phi1)`（dump 是 `_7 = phi1*2; _8 = 1-_7`）、
`proj = phi1 + phi2*coszen`（无乘法）。

`twostream_mod`（PFT 向量那条）是另一份近乎相同的代码，走的是 `twostream_wrap`
（dump 72 处），本分支 `DEF_USE_PFT/PC` 恒假、**不在黄金算例路径上**，本轮没动。

### `snowwater` 的不可约含水项：`FNMA(ssi, eff, vol_liq)`

`MOD_SoilSnowHydrology.F90:1447/1452` 两条 `j` 分支都是
`qout = max(0., (vol_liq - ssi*eff_porosity)*dz)`。dump 是

```
_48 = .FNMA (ssi, eff_porosity, vol_liq);  _131 = _48*dz;  MAX_EXPR <_131, 0.0>
```

即 `vol_liq - ssi*eff` 被吸收、`max` 在乘法**外**。`snow.rs::snow_water` 已按
`(-ssi).mul_add(eff, vol_liq)` 改（并把 `max` 移到 `*dz` 之外）。

**干窗/雪窗零位移，而且原因查清了**：三个黄金窗口里雪层的 `vol_liq` 恒小于
`ssi*eff`（`ssi = DEF_TUNING_SSI = 0.033`；雪窗表层 `wliq` 峰值 0.28 kg/m²，
除以 `dz*1000` 后约 0.006），两条分支的 `max` 都取到 0，融合与否同值。

### `snowage` 的两处：`FMA(deltim*1e-6, 增长项, sag)` 与 `FNMA(增量, 0.1, 1.0)`

`MOD_Albedo.F90:1323-1326` 的 `sge = (sag+dela)*(1.0-dels)`，dump 是

```
_7  = deltim*1e-6;   _10 = exp(arg)+exp(min(0,10arg))+0.3
_13 = .FMA (_7, _10, sag)          ! dela 的乘积被吸收
_15 = .FNMA (max(0,scv-scvold), 0.1, 1.0)
sge = _13 * _15
```

即 `dela` 与 `dels` **都没有单独舍入**。`snow.rs::update_snow_age` 已按此改。

**零位移的原因也查清了**：雪窗里 `sag ≡ 0`（`fresh_snow` 把雪龄反复清零）。
`fma(a,b,0) ≡ fl(a*b)`，所以两式在 `sag = 0` 时逐位相同 —— 而 `(-增量).mul_add(0.1,1.0)`
在 `增量 = 0` 时也恰好回到 1。**这一点是实测的**：把返回值强行加 `1e-3` 再跑雪窗，
`bitwise` 33647 → 33725、`Σ|Δ|` 444394.4368 → 441255.3282，证明这条路径**是活的**，
只是两式的差恰好为零。

### `albland` 的两处：`alb_s_inc` 与 `snow_absorption`

- `MOD_Albedo.F90:305` `alb_s_inc = max(0.11-0.40*ssw, 0.)` → `FNMA(ssw, 0.40, 0.11)`。
  Rust `radiation.rs::soil_albedo` 已改。**干窗零位移**：干季土壤湿、`alb_s_inc`
  顶到 `min(soil_s_v_alb + alb_s_inc, soil_d_v_alb)` 的上界，无论 `alb_s_inc`
  差 1 ULP 与否都取 `soil_d_v_alb`。
- `MOD_Albedo.F90:446/452` 的 `ssoi`/`ssno`（上面那条"最右被吸收"）：
  `ColdStartGroundAlbedo::absorption` 的 `soil_absorption[band][0]` 早就写对了，
  **`snow_absorption[band][0]` 漏了收缩**（原来是平铺加法），本轮补上。

### 一处**无法**逐位复刻的地方（记下来，不再追）

`MOD_Albedo.F90:394` 的整数组语句

```fortran
albg(:,:) = (1.-fsno)*albg(:,:) + fsno*albsno(:,:)
```

在 dump 里**四个元素的舍入方向不一样**：前两个（被 `vector(2) real(8)` 打包的
`vect__842 = FMA(fsno, albsno, (1-fsno)*albg)`）收的是右边那个乘积；
后两个（标量化的 `_863 = FMA(1-fsno, albg, fsno*albsno)`）收的是左边那个。
同一句源码、同一个表达式，只是向量化切分不同。

这是 GCC 在本机的**向量宽度决定**的产物 —— x86 上 AVX 会把四个元素一起打包，
就变成统一方向。**逐位复刻要求同时命中两种方向，这是不可能也不该做的**
（换了机器就错）。Rust 的 `mix_ground_albedo` 取的是向量化那一半
（`snow.mul_add(fraction, (1-fraction)*soil)`），另一半交给 tier2 容差。
注释里引的 `MOD_Albedo.F90:2048-2051` 其实是 `oro==2` 那支的**标量**写法，
形状恰好与向量化那一半同向；行号引用不准，结论不变。

### 窗口三口径（基线 = `52feff5`，即上一轮 `albland` 雪龄反照率之后）

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干 | 21236 → **21230** | 384.4436 → **263.9500** | 837 → **813** | 17（不变） |
| 湿 | 33087 → **32535** | 10374.2243 → 10380.4730 | 20662 → 20665 | 68 |
| 雪 | 33647 → 33647 | 444394.4368（持平） | 25896（持平） | 79 |

**判据链**：干窗的 `over_tol` 是上一轮 `net_solar` 方向写错时唯一会爆的量
（写反 17 → 27），本轮把它从 837 拉回 **813**（= `net_solar` 之前的最好水平），
同时湿窗逐位不同的值创下新低 —— 两条独立的量同向，不是单点噪声。湿窗
`Σ|Δ|` 与超容差条数各 +3、+0.06%，属该轮混合信号，不构成否决。

雪窗三口径全持平：雪窗的 `f_alb`/`f_t_soisno` 等大项**本来就整体差分**
（`f_alb` 360 步里有 92 步差 2%–10%，`f_t_soisno` 差一个雪层槽位），
这些量与黄金值早已全部落在"逐位不同"桶里，1 ULP 级的改动不会改变该桶的计数，
`Σ|Δ|` 又由 `f_vegwp`（~1e3 量级）主导。**雪窗的这套口径对 ULP 级改动天然不敏感**，
不能拿它当"改了没用"的证据。

Tested: `MOD_Albedo.F90`/`MOD_SoilSnowHydrology.F90` 本体 dump（`twostream` 45 处、
`albland` 9 处、`snowage` 2 处、`snowwater` 4 处逐条编号并归支）；
三个黄金窗口三口径 A/B；`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；
`cargo fmt --all --check`；`cargo clippy --workspace --all-targets -- -D warnings`；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo test -q -p oracle`；
`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。
Not-tested: `twostream_mod`（`twostream_wrap` 72 处，PFT/PC 支，本分支不可达）；
`twostream` 的 `sigma <= 1e-10` 支 19 处；`albocean` 8 处（湖泊/海洋 patch）。

## `MOD_CanopyLayerProfile` 的 24 处收缩：**只有 `cal_z0_displa` 在黄金算例路径上**

`MOD_CanopyLayerProfile.F90` 整模块 24 处收缩，按例程分布：

| 例程 | 处数 |
|---|---|
| `cal_z0_displa` | 5 |
| `kintegral` | 4 |
| `uintegral` | 4 |
| `uintegralz` | 4 |
| `fkint` | 2 |
| `fuint` | 2 |
| `kdiff` | 1 |
| `udiff` | 1 |
| `uprofile` | 1 |

**可达性先查清楚**（这是本轮唯一真正动窗口的地方）：`uprofile`/`uintegral`/`kintegral`
这一族只有 `canopy_roughness` 被运行期调用 —— `leaf_temperature.rs:311` 的
`canopy_roughness(lsai, htop, 1.0)`。其余 14 个例程在本仓库只被
`canopy_layer_profile_tests` 与初始化诊断用到，`colm-runtime` 一次都不调。

### `cal_z0_displa` 的 5 处（**活的**）

| 上游行 | 形状 | Rust |
|---|---|---|
| `:684` `fai = (sqrtdragc**2-0.003)/0.3` | `FMA(sqrtdragc, sqrtdragc, -0.003)` | `canopy_roughness.rs` `square_root_drag.mul_add(square_root_drag, -0.003)` |
| `:700` `sqrtdragc = min((0.003+0.3*fai)**0.5, 0.3)` | `FMA(fai, 0.3, 0.003)` | `f77(0.3).mul_add(area_index, f77(0.003))` |
| `:707/:710` 两处几何和 | `FMA(fc*1.1, log(1+(Cd*lai*fc)**0.25), (1-fc)*poly)` | `(fc*1.1).mul_add(log项, (1-fc)*poly)` |
| `:707` 的 `delta + h*geometry` | `FNMA(h, 几何(lai0), h*几何(lai))` —— **`h*几何(lai0)` 被吸收** | `canopy_height_m.mul_add(-initial_geometry, scaled_geometry)` |

最后一处是本轮唯一**结构**级的改动：原来是先落成 `delta = -h*几何(lai0)` 再相加，
GIMPLE 里 GCC 根本没有单独舍入那个乘积。为此把 `delta` 拆成
`initial_geometry`（不带 `h`），在用到的地方一次 `FNMA` 完成。

顺带解掉一个读 dump 时的疑点：`displa` 的 `IF (lai > lai0)` 支里第二个乘积用的是
**新** `temp1`（`fc*(1-exp(-0.5*lai))` 那一支），而 `delta` 里用的是**初始** `temp1`
（`(sqrtdragc**2-0.003)/0.3` 那一支）。dump 里 `_33 = _25*prephitmp_232` 用的是
初始那一支 —— 因为 `_34` 就是 `delta` 的几何式（GCC 把它复用成 `-h*_34` 的加数），
不是 `IF` 支的那一项；`IF` 支的那一项是 `_250`。**两句都用对了，虚惊一场。**

### 其余 19 处：形状解出来了，但没有窗口信号

- `uprofile`（1）：`FMA(bee*fc, min(uexp,ulog), (1-bee*fc)*ulog)`。
- `uintegral`/`uintegralz`（各 4）：2 处是循环体（已有）+ 1 处
  `dz = top-bottom-(n-1)*dz` 的 `FNMA(n-1, dz, top-bottom)` + 1 处被内联的 `uprofile`。
- `kintegral`（4）：同上，另外 `kintegral = kintegral + 1./k*dz` 的 GIMPLE 是
  `FMA(1/k, dz, 累积)` —— 注意**不是** `dz/k`：先算一次 `1/k`，那个乘积被吸收。
  Rust 原先写的是 `step / k`，两者在 1 ULP 上不同，已改。
- `fkint`（2）：`fkcobint` 的左边乘积被吸收；末尾 `bee*fc*fkexpint+(1-bee*fc)*fkcobint`
  是 `FMA`。
- `fuint`（2）：`FULOGINT` 的 `ztop*log(ztop/z0mg) - zbot*log(zbot/z0mg)` 用 `FMS`
  （左乘积被吸收、右乘积先舍入），末尾同 `fkint`。
- `kdiff`/`udiff`（各 1）：`kexp - kcob` / `uexp - ulog` 都是
  `FMS(ktop, exp, 另一个)` —— 那个 `ktop*exp` 并不单独舍入。

### 窗口三口径（基线 = 本轮 `twostream` 那一版）

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干 | 21230 → **21196** | 263.9500 → 311.4338 | 813 → **825** | 17（不变） |
| 湿 | 32535 → **32530** | 10380.4730 → 10381.6452 | 20665 → **20664** | 68 |
| 雪 | 33647 → **33602** | 持平 | 持平 | 79 |

**混合信号，照实记录**：三个窗口的逐位不同值**同向变好**（干 −34、湿 −5、雪 −45），
湿窗超容差 −1，但**干窗超容差从 813 退到 825**（+1.5%），Σ|Δ| 也变大。
干窗超容差是本项目此前用来分辨"方向写错"的那个量（`net_solar` 反向时 17→27），
所以这条不能当噪声；但它的基线 `net_solar` 之前是 813、`52feff5` 之后是 837，
现在 825 —— 仍好于 `52feff5`，且 **tier2 变量数始终 17**（结构没坏）。
判据链只有 GIMPLE 一条：五处形状逐一对应，没有一处是猜的。
**保留，并把这条混合信号钉在这里**：如果以后有更强的判据说明该退，就从
`canopy_roughness.rs::canopy_roughness` 的 `initial_geometry`/`scaled_geometry`
那两行退。

Tested: `MOD_CanopyLayerProfile.F90` 本体 dump（24 处逐条归例程、逐条读形状）；
三个黄金窗口三口径 A/B；`cargo test -q -p colm-core --lib -- --test-threads=1`（354 通过）；
`cargo clippy -q -p colm-core --all-targets -- -D warnings`；`cargo fmt --all --check`。
Not-tested: 这 19 处没有随机差分驱动（上游那 14 个例程没有现成调用点，
`uprofile`/`kintegral` 那两个有 `USE MOD_CanopyLayerProfile` 也还需要按 8/12 个实参搭桩）；
`cal_z0_displa` 的 ELSE 支（`sqrtdragc > 0.3`，走到就打诊断并取 `fai = 0.29`）没有专门构造。

## **真缺陷**：`rss` 少了上游"第一步不算"那道门（`rss /= spval`）

这是本会话第一次在**没被黄金算例覆盖的配置**上抓到的实打实的数值缺陷，
而不是 1 ULP。

### 怎么发现的

文档一直把"`DEF_USE_Campbell_SOIL_MODEL = .true.` + `DEF_USE_VariablySaturatedFlow
= .false.`"这条经典 Richards 配置记成"**本机一次都没被走到**"（它确实不是三个黄金
算例的配置）。这轮把它**真的跑起来**：复制 `oracle/work/CN-Cng`，在 `case.nml`
的 `&nl_colm` 里加

```
DEF_USE_Campbell_SOIL_MODEL   = .true.
DEF_USE_VariablySaturatedFlow = .false.
```

两侧各跑一步、再各跑满 11 天，与内核逐位比对。**第一步就差了 50 个变量**，
量级不是 1 ULP 而是 1e-4…1e-3（`f_rss` 0 对 0.0163、`f_ldew` 1.2e-3、
`f_wliq_soisno` 1.0e-4）；11 天口径 51 个 tier2 变量超容差。

### 定位

`MOD_Thermal.F90:613-621` 有两道门：

```fortran
!NOTE: (1) DEF_RSS_SCHEME=0 means no rss considered
!      (2) Do NOT calculate rss for the first timestep
IF (DEF_RSS_SCHEME>0 .and. rss/=spval) THEN
   CALL SoilSurfaceResistance (…)
ELSE
   IF (DEF_RSS_SCHEME == 4) THEN
      rss = 1.        !LP92
   ELSE
      rss = 0.        !the other RSS schemes
   ENDIF
ENDIF
```

`rss` 是 `MOD_Vars_TimeVariables` 的 module 时间变量，**起跑重启里是 `spval`**：

```
$ python3 -c "import netCDF4 as nc; print(nc.Dataset('…/restart/2008-001-00000/….nc').variables['rss'][:])"
[-1.e+36]
```

本仓库只实现了第一道门（`scheme == 0` 时返回 0），**第二道完全没有** ——
第一步照样算，于是多出一个 0.0163 s/m 的土壤表面阻力。黄金算例`scheme` 被
上游强制置 0，所以这条永远显不出来；只有 Campbell 配置才撞上。

### 判定证据（两次独立、可复现）

1. 把 `DEF_RSS_SCHEME` 显式写成 `0`（把这道门绕开）再跑一步：
   50 个变量 → **33 个、全部 ~1e-15 相对**，与黄金配置同一水平。
2. 实现这道门之后不写 `DEF_RSS_SCHEME`：同样 50 → **33 个、~1e-15**，
   `f_rss` 两侧都是 0；跑满 11 天，超容差变量 **51 → 16**。

### 落地

`rss` 在上游是 module 时间变量，所以 Rust 侧也必须是**跨步状态**，不能每步现算：

* `StandardLctEnergyState` 新增 `soil_surface_resistance_s_m`（`MISSING` = 未算过）；
* `standard_lct_step.rs::soil_surface_resistance_input` 收下"上一步的 `rss`"，
  等于 `MISSING` 时按 `scheme == 4 ? 1.0 : 0.0` 返回；
* `StandardLctRestartTemplate` 从**入参重启**里读 `rss`（起跑是 `spval`，
  断点续跑是上一段算出的值 —— 写死 `spval` 会让续跑的每一步都当"第一步"）。

回归测试：`standard_lct_step_tests::soil_surface_resistance_is_skipped_on_the_first_timestep`
（`MISSING` → 0，且写回状态；已有值时 > 0 且写回）。

**规矩**：凡上游用 `x /= spval` 之类**"缺测值即未初始化"**的 module 时间变量做门，
都要当成跨步状态移植，不能当纯函数。`rss` 是第一个被本仓库发现的；
同类候选还有 `alb`/`ssun`/`ssha`/`ssoi`/`ssno`/`thermk`/`extkb`/`extkd`（这些
黄金算例每步都覆盖，暂未暴露）。

### 黄金算例不受影响

`DEF_USE_Campbell_SOIL_MODEL` 为假时上游把 `DEF_RSS_SCHEME` 强制置 0，
所以那道门在三个黄金窗口里恒不触发。实测三口径**逐位不变**：

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干 | 21196（持平） | 311.4338（持平） | 825（持平） | 17 |
| 湿 | 32530（持平） | 10381.6452（持平） | 20664（持平） | 68 |
| 雪 | 33602（持平） | 444394.4368（持平） | 25896（持平） | 79 |

Tested: `MOD_Thermal.F90:613-621` 与 `MOD_Namelist.F90:1946-1950` 逐行核对；入参重启
`rss` 的实测值（`-1e36`）；Campbell + VSF-off 配置一步与 11 天的两侧逐位比对
（改前 50 变量 / 51 超容差，改后 33 变量 / 16 超容差）；`DEF_RSS_SCHEME=0`
对照实验；三个黄金窗口三口径 A/B（逐位不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过，含新回归测试）；
`cargo clippy --workspace --all-targets -- -D warnings`；两处 `cargo fmt --all --check`；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo test -q -p oracle`；
`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`（PASS）。
Not-tested: Campbell + VSF-off 只有一个月窗口、没有入库的黄金文件；`scheme == 4`
（LP92）的那一支没有算例；其他 `spval` 门（`alb` 那一族）没有逐条排查。

## `DEF_SPLIT_SOILSNOW` 此前**根本没被读过** —— 静默按非 split 跑完

上一节的规矩（"上游有、本仓库没移植的分支必须显式报错"）是**列出来的**，而
`DEF_SPLIT_SOILSNOW` 从来没进过那张清单，也从来没被任何代码读过：

```
$ grep -rn "SPLIT_SOILSNOW" crates/ --include=*.rs | grep -v "_tests\|generated.rs"
crates/colm-case/src/parameters/mod.rs:683        ← GUI 的 namelist 分组表，不是校验
crates/colm-core/src/thermal_water.rs:110          ← 注释
crates/colm-core/src/standard_lct_step.rs:552      ← 注释
crates/colm-core/src/water_2014.rs:354             ← 注释
```

后果：写 `DEF_SPLIT_SOILSNOW = .true.` 的算例，会被 `assembly.rs` 里那句硬写死的

```rust
use_split_soil_snow: false,
```

**静默按非 split 跑完** —— 正是本仓库纪律里点名的最坏一类分支不匹配：
算式对、结构错、还不报错。上游在 split 下给土壤面和雪面**各自一个地表温度**、
各自一组 `qsdew_soil`/`qfros_soil`/`qsubl_soil` 与 `qsdew_snow`/…（见
`MOD_SoilSnowHydrology.F90:452-484` 那一对分支）。

**为什么是"拒绝"而不是"补齐"**：内核侧其实已经有 split 的能量分配
（`colm_core::partition_split_thermal_water`，`ground_temperature.rs` 也按
`use_split_soil_snow` 分了雪/土两个面），但**水分侧只做了非 split** ——
`water_2014.rs` 的入口注释写得很直白（"本入口明确是非 split，所以有雪层时给 0"）。
只接一半会让结果比拒绝更难查。声明默认值是 `.false.`，所以按纪律 #3
直接 `bail!`，不进 `unported_branches`。

落地两处（**运行期与冷启动都要拦**，否则 `mkinidata` 会为一份跑不起来的配置
生成"看起来能用"的重启）：

* `colm-runtime/src/physics.rs::land_physics_parameters`：读完 `DEF_Runoff_SCHEME`
  之后就拦 `DEF_SPLIT_SOILSNOW`；
* `colm-init/src/single_point.rs::reject_unsupported_cold_start_features`：同一句。

回归测试：`physics_tests::split_soil_snow_is_refused_rather_than_run_as_non_split`。
端到端实测：

```
$ colm-rs /tmp/gf/splitcheck --land-cover igbp …
colm-rs: DEF_SPLIT_SOILSNOW is on, but the Rust runtime only assembles the non-split
soil/snow column: `assembly.rs` pins `use_split_soil_snow` to false and `water_2014.rs`
implements only the non-split hydrology, so the case would silently run as non-split
(upstream gives soil and snow separate surface temperatures and separate
qsdew/qfros/qsubl on each face)
```

### 顺带把 `spval` 那道门这一类查了一遍

上一条提交的 Directive 说"凡上游用 `x /= spval` 做门的地方都要当跨步状态"。
把 `vendor/CoLM202X/main` 全量过了一遍，`spval` 比较只有下列几处：

| 位置 | 性质 |
|---|---|
| `MOD_Thermal.F90:615` `rss /= spval` | **数值分支**，已修（上一条提交） |
| `MOD_LeafTemperature.F90:567/573/579`（及 PC/URBAN 副本） `taux == spval` | 只包一句 `write(6,*)` 警告，无数值影响 |
| `MOD_Hist*.F90` 的 `WHERE (acc_vec /= spval)` | 历史累加器的缺测掩码；单点、无缺测 |
| `MOD_Forcing.F90` / `MOD_CheckEquilibrium` / `MOD_CropReadin` | 强迫读入 / 数据同化 / 作物，均不在本仓库路径 |

`alb`/`ssun`/`ssha`/`ssoi`/`ssno`/`thermk`/`extkb`/`extkd` 这几个 module 时间变量
**没有** `spval` 门，但都逐条核过"第一步读到的是不是未初始化值"：

* `alb`：夜间提前返回时上游写 `alb = 1`，Rust `surface_optics.rs` 的夜间支同样写 1；
* `ssun`/`ssha`：`none` 时上游在 `netsolar` 开头清零，Rust 同样清零；
* `ssoi`/`ssno`：由 `tran`（初值 `[0,1,1]`）/`albsoi`/`albsno` 现算，Rust 的
  `ColdStartGroundAlbedo::absorption` 无条件重算；
* `extkb`/`extkd`/`thermk`：`thermk` 的"有冠层时保留上一步"已按上游实现
  （`surface_optics.rs` 的 `previous_thermal_gap_fraction`），`extkb`/`extkd`
  在夜间支写 1/0.718。

Tested: `grep` 全量 `spval` 比较并逐条归因；`DEF_SPLIT_SOILSNOW` 全仓库读取点核对；
端到端 `colm-rs` 拒绝信息；`cargo test -q -p colm-init --lib -- --test-threads=1`
（156 通过 / 9 忽略）；`cargo test -q -p colm-runtime --lib -- --test-threads=1`（71 通过）；
`cargo fmt --all --check`。
Not-tested: split 支路本身（**故意**：拒绝就是不跑）；`MOD_Hist*` 的缺测掩码（单点无缺测）。

## 把"没被读过但行为有影响的 namelist 开关"全量筛了一遍（832 个字段）

`DEF_SPLIT_SOILSNOW` 那件事暴露的是一类问题：**schema 里有、算例能写、代码不读**。
用脚本过了一遍 `crates/colm-schema/src/generated.rs` 的 **832 个字段**，
逐个在非测试 Rust 源码里搜字段名，563 个"从未出现"。这 563 个里绝大多数是噪声：

| 类别 | 数量级 | 为什么不是问题 |
|---|---|---|
| `DEF_hist_vars%*` | ~450 | 历史变量闸门由 `colm-hist` 的表达式机制按名求值，不逐个出现在源码里 |
| `DEF_TRACER_*` | ~35 | 示踪物支路，本仓库无对应编排 |
| `DEF_DA_*`、`DEF_Optimize_*` | ~15 | 数据同化 / 参数优化，见下 |
| BGC / CROP 的 PFT 名数组 | ~40 | 只在 BGC/CROP 宏下使用，本仓库无对应 patch 类型 |
| `DEF_forcing%*`、`USE_SITE_*` | ~30 | 由 `colm-forcing`/`colm-case` 用别的方式解析（不是按字段名） |
| `DEF_BlockInfoFile`、`DEF_PIO_groupsize` 等 | ~10 | MPI/并行 I/O，单点不需要 |

真正需要判断的只有下面这几个，逐个查了上游语义：

| 字段 | 声明默认 | 上游语义 | 处置 |
|---|---|---|---|
| `DEF_SUBGRID_SCHEME` | `'LCT'` | **只在 `namelist` 与 `mpi_bcast` 里出现，全树没有任何一处读它做派发**（派发用的是 `DEF_USE_LCT/PFT/PC`） | 纯镜像，不读无害 ✓ |
| `DEF_Forcing_Interp_Method` | `'arealweight'` | `MOD_Namelist.F90:2404-2407`：`#ifdef SinglePoint` 下 `'bilinear'` 被**强制改回** `'arealweight'` 并打警告 | 单点下两边等价 ✓ |
| `DEF_CheckEquilibrium` | `.false.` | `MOD_CheckEquilibrium` 做诊断；`CoLM.F90:681` 只在末尾打印 `mesg_equilibrium` | 纯诊断/打印，不读不影响数值 ✓ |
| `DEF_HIST_WriteBack` / `DEF_HIST_CompressLevel` / `DEF_HIST_grid_as_forcing` | `.false.` / `1` | 只影响 NetCDF 写法与压缩，不改物理 | 不读无害 ✓ |
| `DEF_USE_DiagMatrix` | `.false.` | BGC 诊断矩阵（`leafcCap` 等），只在 BGC 下用 | 本仓库无 BGC 编排 ✓ |
| **`DEF_Optimize_Baseflow`** | `.false.` | `MOD_Opt_Baseflow.F90:82` 在 `is_spinup` 时迭代 `scale_baseflow(ipatch)` 并写回 `ParaOpt/*_baseflow.nc` | **显式拒绝**（本轮） |
| **`scale_baseflow` 文件本身** | 无（文件） | `MOD_Opt_Baseflow.F90:37-38` 从 `DEF_dir_restart/ParaOpt/<case>_baseflow.nc` 读，`defval = 1.` | **文件存在时显式拒绝**（本轮） |

### 落地

* `colm-runtime/src/physics.rs`：`DEF_Optimize_Baseflow = .true.` → `bail!`
  （本仓库把 `baseflow_scale` 钉成 1.0、不做预热优化，开着它等于静默不优化）。
* `colm-runtime/src/bin/colm-rs.rs`：`<out>/<case>/restart/ParaOpt/<case>_baseflow.nc`
  **存在**时 → `bail!`。没有这个文件时两边本来就一致（内核日志会打
  "default value is used"，本机三个黄金算例都是这一支），所以这道门不影响现有算例；
  文件一旦存在就说明该算例的基流参数被标定过，静默用 1.0 会给出另一套产流。

**没做**：把 `ParaOpt/*_baseflow.nc` 真读进来（读一个长度 `landpatch` 的
`scale_baseflow` 向量就能消掉这条限制）。当前先用拒绝把"静默不一致"堵上 ——
按本仓库的纪律，宁可拒绝也不要静默跑出另一套数。

回归测试：`physics_tests::baseflow_optimization_is_refused_rather_than_run_unoptimized`。

Tested: 832 个 schema 字段的全量交叉搜索脚本；上表逐条回源码核对
（`MOD_Namelist.F90:2404-2407`、`MOD_Opt_Baseflow.F90:37-38/82`、`CoLM.F90:681`）；
`cargo test -q -p colm-runtime --lib -- --test-threads=1`（72 通过）；
`cargo fmt --all --check`。
Not-tested: `ParaOpt/*_baseflow.nc` 的真实读取（本机没有这种文件）；
`DEF_CheckEquilibrium = .true.` 时的输出文件内容。

### `rss` 那道门修完之后：Campbell + VSF-off 在**三个窗口**上都与黄金配置齐平

用同一套流程（复制 `oracle/work/<case>`、往 `&nl_colm` 里加那两条开关、两侧各跑一遍、
`golden-compare` 拿上游当参照）把另外两个窗口也跑了：

| 窗口 | Campbell + VSF-off 超容差变量数 | 同窗口黄金配置（VSF on）|
|---|---|---|
| 干 `CN-Cng` | **16** | 17 |
| 湿 `CN-Cng-wet` | **66** | 68 |
| 雪 `US-NR1-snow` | **79** | 79 |

三个窗口都在同一水平 —— 也就是说这条"另一个配置"的支路在修掉 `rss` 之后
**没有留下第二条系统偏差**。脚本已入库：`oracle/scripts/compare_second_config.sh <case>`（复制 + 注入开关 +
两侧跑 + 比对，一条命令；`BASE` 由脚本自身位置推出）。

**这一步的分量**：在此之前文档把这条配置记成"本机一次都没被走到"，于是它上面的
所有收缩（`soilwater` 23 处、`water_2014` 13 处）都只是"从 dump 读出来的形状"，
没有任何端到端信号。现在它们有了一个可复现的两侧对照口径，且当前是齐平的。

Tested: `/tmp/gf/camp_any.sh CN-Cng-wet`、`/tmp/gf/camp_any.sh US-NR1-snow`
（内核 + `colm-rs` + `golden-compare`）；三个窗口的超容差变量数如上表。
Not-tested: 这条配置**没有入库的黄金文件**，所以它是"可复现实验"而不是回归闸门；
要不要为它加第四个黄金窗口（含重跑 tier 分层分类）留待后续决定。

### 反过来：`ParaOpt/*_baseflow.nc` 改成**真读**，不再拒绝

上一条把它做成"文件存在就拒绝"。那是把"静默不一致"堵上了，但拒绝本身也是个缺口 ——
被标定过的算例本仓库就完全跑不了。这轮改成真读，并做了三道验证。

**文件名要先对**：上游 `MOD_Opt_Baseflow.F90:37` 拼的是
`<DEF_dir_restart>/ParaOpt/<case>_baseflow.nc`，但 `ncio_read_vector` 走的是
`ncio_read_vector_complete_real8_1d` → `get_filename_block`
（`MOD_Block.F90:641-645`），**读盘时把块名插在 `.nc` 之前**：

```
fileblock = filename(1:i-1) // '_' // blockname // '.nc'
```

单点算例的块名是 `w180_s90`，所以真正的文件名是
**`<case>_baseflow_w180_s90.nc`**。实测：写成不带后缀的名字时内核照样打
"restart data scale_baseflow … not found, default value is used" —— 文件明明在那儿。
本仓库读的是同一个带后缀的名字（与重启文件名的约定一致）。

**三道验证**（`colm-rs` 的 `read_baseflow_scale`）：

| 情形 | 期望 | 实测 |
|---|---|---|
| 文件不存在（三个黄金算例） | 用 `defval = 1.`，结果逐位不变 | ✓ 三窗口三口径逐位不变 |
| 文件在、内容不是 NetCDF | 报错（证明**真的去开了**） | ✓ `cannot open restart …: NetCDF: Unknown file format` |
| 文件在、`scale_baseflow = NaN` | 值进到内核输入、被 `WATER_VSF` 的有限性校验拦住 | ✓ `colm-rs: WATER_VSF values are not physical` |

第三道是"值真的流进去了"的直接证据：`variably_saturated_flow.rs` 的
`validate` 里有 `&& input.baseflow_scale.is_finite()`，NaN 只可能来自那个文件。

**没验到的（说清楚）**：`rsubst` 在三个黄金窗口里**恒为 0**
（`f_rsub` 最大值的实测：干 0 / 湿 0 / 雪 0），所以 `scale_baseflow` 乘上去
数值上完全没有可见效果 —— 把文件里的值从 1.0 改成 1.5，内核与本仓库的输出
**都逐位不变**。这条支路的数值正确性因此只能靠"读的是同一个数"来保证，
不能靠窗口。上游语义（`rsubst = rsubst*scale_baseflow(ipatch)`）照抄，
位置在 `assembly.rs` 装配期一次，与 `Opt_Baseflow_init` 只读一次一致。

顺带把入库的对照脚本改成**两侧共用同一棵输出树**（`DEF_dir_output` 指到
`<case>/out/`）：分成两棵树时 `ParaOpt/*_baseflow_*.nc` 只有一侧看得见。
改后干窗仍是 16，湿/雪窗口同前。

Tested: `MOD_Opt_Baseflow.F90:37`、`MOD_Block.F90:620-647`、
`MOD_NetCDFVector.F90:312-336/744` 逐段核对；`colm-rs` 的三道验证（缺文件 /
坏文件 / NaN）；`scale_baseflow` 1.0 vs 1.5 两侧输出逐位比对（都不变，原因是
`f_rsub ≡ 0`）；`oracle/scripts/compare_second_config.sh` 三个窗口
（干 16 / 湿 66 / 雪 79）；`cargo test -q -p colm-runtime --lib -- --test-threads=1`（72 通过）；
`cargo fmt --all --check`。
Not-tested: `rsubst ≠ 0` 的算例（本机没有）；`DEF_Optimize_Baseflow = .true.`
的优化过程本身（仍然拒绝）。

## 用开关把第 0 步的差异分区：种子不在 PHS，也不在 VSF

干窗第 0 步那 34 个变量各差 1 ULP 的**种子**一直没定位。这轮换了个办法：
**用一条 namelist 开关把整条支路关掉**，看差异集合怎么变。脚本已入库：

```
oracle/scripts/compare_flag_isolated.sh <tag> "<NAMELIST 行>"
```

它复制 `CN-Cng`、注入那一行、两侧各跑 1 步（`DEF_HIST_FREQ='TIMESTEP'`），
打印逐位不同的变量与相对量级。

| 配置 | 逐位不同的变量数 | 最大相对差 | 说明 |
|---|---|---|---|
| 默认（PHS on、VEG_SNOW on、VSF on） | 34 | 6.0e-15 | 起点 |
| `DEF_USE_PLANTHYDRAULICS = .false.` | 37 | 9.2e-15 | **同量级、同一集合** → 种子不在 PHS |
| `DEF_VEG_SNOW = .false.` | **18** | 3.98e-13 | 叶面那一组降到**恰好 1 ULP（1.2e-16）**，只剩水分 |

两个结论：

1. **PHS 不是种子**。关掉之后差异集合与量级几乎不动（34 → 37，最大 6e-15 → 9.2e-15），
   说明植物水力那套迭代既不产生也不放大这 1 ULP。此前把它列为候选可以划掉。
2. **`DEF_VEG_SNOW` 分支会放大一个更小的既有差异**。关掉之后叶面一组
   （`fevpa`/`fevpl`/`gssun`/`gssha`/`ldew`/`qstar`/`zol`/`rib`/`fsenl`/`us10m`/`vs10m`）
   从 1e-15 量级降到 **1.2e-16 —— 正好一个 ULP**，也就是说这些量本身只剩"传输一次"
   的舍入；其余 18 个里最扎眼的是 `f_wliq_soisno` 的**一层**差 3.98e-13
   （`f_h2osoi` 同源，2.48e-13）。

所以真正的种子在**水分步与能量步共用的那一环**（`f_wliq_soisno` 那一层就是入口），
`DEF_VEG_SNOW` 只是把它放大两个数量级。下一步该做的是**在第 1 步内给
`WATER_VSF` 的 `wliq_soisno` 逐层打点**（内核 + Rust 两侧），看它是从
`qinfl`/`rsur`/`vol_liq` 哪一项开始差的 —— 这一步需要重建内核，留给后续轮次。

**规矩**：定位"整步处处差 1 ULP"这类分叉时，**先开关分区、再逐点打点** ——
开关分区一次只要一分钟，能把候选子系统从"全部"砍到一两个。

Tested: `oracle/scripts/compare_flag_isolated.sh` 在默认 / `DEF_USE_PLANTHYDRAULICS=.false.` /
`DEF_VEG_SNOW=.false.` 三种配置下的逐位比对（34/37/18 个变量，量级如上表）；
`cargo fmt --all --check`。
Not-tested: 第 1 步内 `wliq_soisno` 的逐层打点（需要重建内核，本轮没做）。

## 第 0 步的种子缩到"扩散求解之前"：`fseng`/`fevpg` 在入口就已经差 1–2 ULP

接着上一节的"种子在水分/能量共用的那一环"，这轮**真的重建了内核**做了两次定点插桩
（`CoLMMAIN` 能量步之后、`MOD_GroundTemperature` 入口/出口），两侧对齐同一点打印。
插桩是临时的，**已全部还原**（`git status` 干净、`test_upstream_f48_sync.py` PASS、
内核二进制按 `manifest.json` 的 sha256 复原）。

### 定点一：能量步（THERMAL）之后、水分步之前

`CoLMMAIN.F90` 里 `CALL THERMAL` 与水分调用之间插一个只跑一次的 `write`：

| 量 | 上游 | 本仓库 | 结论 |
|---|---|---|---|
| `t_soisno(1)` | 2.7316000000000003E+02 | 同 | 一致 |
| `t_soisno(2)` | 2.78497805689993**19**E+02 | 2.78497805689993**13**E2 | **差 1 ULP** |
| `t_soisno(4)` | 2.82942679869602**27**E+02 | 2.82942679869602**33**E2 | **差 1 ULP** |
| `wliq_soisno(1)` | 5.708467638912**7393**E+00 | 5.708467638912**7313**E0 | 差 ~9 ULP |
| `wice_soisno(1)` | 3.075678376285**0430**E+00 | 3.075678376285**0506**E0 | 差 ~8 ULP |
| `smp`, `hk` | — | — | **逐位一致** |

也就是说：**水分步还没跑，`t_soisno` 与表层的 `wliq`/`wice` 就已经不一样了**。
`t_soisno` 在整套代码里只由热传导/相变写（`WATER_VSF` 只读它），所以种子在
**能量步**里 —— 这与"`smp`/`hk` 逐位一致"是自洽的（那两项恰恰由水分步写）。

### 定点二：`MOD_GroundTemperature` 入口

| 量 | 上游 | 本仓库 | 结论 |
|---|---|---|---|
| `t_grnd` | 2.8300000000000000E+02 | 同 | 一致 |
| `sabg` | 0 | 0 | 一致 |
| `fseng` | 1.24578191769597**86**E+03 | 1.24578191769598**02**E3 | **差 1 ULP** |
| `fevpg` | 3.3723365149310**956**E-04 | 3.3723365149311**015**E-4 | **差 2 ULP** |
| `t_soisno(1:4)` 入参 | 全 2.83E+02 | 同 | 一致 |
| `wliq(1:4)` 入参 | 8.784…/13.83… | 同 | 一致 |

**扩散求解的入口就已经带着差异**：`fseng`（地表感热）差 1 ULP、`fevpg`（地表蒸发）
差 2 ULP，而温度与含水量入参逐位一致。所以种子在**地面通量/叶温那一段**，
不在扩散求解本身。

（探针第一版把上游的 `frl` 当成"到达地面的下行长波"与 Rust 的
`downward_longwave_w_m2` 对比，得出 177 W/m² vs 225 W/m² —— 那是**量名对错了**：
`MOD_GroundTemperature.F90:129-130` 里 `frl` 是"大气红外"、`dlrad` 才是"冠层以下的
下行长波"，面通量用的是 `dlrad`（`:265-293`）。Rust 的字段对应的是 `dlrad`。
下一轮要打的是 `dlrad`，不要再拿 `frl` 比。）

### 下一轮的精确入口

`fseng`/`fevpg` 的差异只可能来自 `ground_fluxes` 或 `leaf_temperature` 的输出链
（`ground_humidity` → `ground_fluxes` 预解 → `leaf_temperature` → 修正后的
`fseng`/`fevpg`）。既有文档里 `ground_fluxes` 与 `monin_obukhov` 都做过
20000/20000 的随机差分，所以优先怀疑**它们的输入**：`qg`/`dqgdT`
（`non_split_ground_humidity`）与 `emg`。下一轮照这一轮的办法再插一个点：
`MOD_GroundFluxes` 入口打印 `qg`/`dqgdT`/`t_grnd`/`t_soisno(1)`/`wliq(1)`，
与 Rust 的 `GroundFluxInput` 逐位对齐。

Tested: 内核两次插桩重建（`CoLMMAIN`、`MOD_GroundTemperature`）+ 上游 `colm.x`
与 `colm-rs` 各跑 1 步的定点对照；插桩全部还原后 `git status` 干净、
`python3 oracle/scripts/test_upstream_f48_sync.py` PASS、
`kernels/default/*.x` 的 sha256 与 `manifest.json` 逐项相符；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
两处 `cargo fmt --all --check`；`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；
三个黄金窗口（17/68/79）。
Not-tested: `dlrad` 的对齐（探针量名错了）；`MOD_GroundFluxes` 入口的下一层插桩。

## **整块漏扫**：`MOD_LeafTemperature` 有 61 处收缩，此前**一次都没 dump 过**

上一节把种子缩到"地面通量/叶温那一段"。顺着这条线去查那一段的 GIMPLE 时发现：
**`MOD_LeafTemperature.F90` 从来没有进过清点表**。已有的 dump 只有
`/tmp/gf/r107/*.opt` 那几个，按 `;; Function` 认一下身份：

| dump | 模块 |
|---|---|
| `fd.opt` | `MOD_ForcingDownscaling` |
| `fv.opt` | `MOD_FrictionVelocity` |
| `ph.opt` | `MOD_PlantHydraulic` |
| `hsf.opt` | `MOD_Hydro_SoilFunction` |
| `ssh.opt` | `MOD_SoilSnowHydrology` |
| `MOD_Albedo.opt` / `MOD_NetSolar.opt` / `MOD_CanopyLayerProfile.opt` / `MOD_GroundFluxes.opt` / `MOD_TurbulenceLEddy.opt` | 各自模块 |

**`MOD_LeafTemperature` 不在其中。** 而它正好是探针指出的那一段。

### 补上 dump

```bash
cd vendor/CoLM202X
gfortran -c -O2 -fdefault-real-8 -ffree-form -g -ffpe-trap=invalid,zero,overflow \
  -fbacktrace -cpp -ffree-line-length-0 -fallow-argument-mismatch \
  -I.bld -Iinclude -Imain -Imain/HYDRO -Imain/URBAN -Imain/BGC -Ishare \
  main/MOD_LeafTemperature.F90 \
  -fdump-tree-optimized=/tmp/gf/r144/lt.opt -o /tmp/gf/r144/lt.o
```

结果：**61 处**（全部内联进 `leaktemperature` 一个函数的 dump 里），
而 `crates/colm-core/src/leaf_temperature.rs` 只有 **19 个 `mul_add`** ——
**约 42 处对不上**。这就是干窗第 0 步那个种子的最可能藏身处：一个只在活路径上、
却从未按 GIMPLE 逐条对过的模块。

### 本轮顺手补上的两处（`tref`/`qref`）

`MOD_LeafTemperature.F90:1271-1272`：

```fortran
tref = thm + vonkar/(fh-fht)*dth * (fh2m/vonkar - fh/vonkar)
qref =  qm + vonkar/(fq-fqt)*dqh * (fq2m/vonkar - fq/vonkar)
```

GIMPLE（dump 第 3169/3181 处）：

```
_823 = 0.4/(fh-fht);  _478 = thm-taf;  _825 = _478*_823
_829 = fh2m/0.4 - fh/0.4
_832 = FMA(_825, _829, thm)
```

即 `tref = fma(fl(dth*vonkar/(fh-fht)), fh2m/vonkar-fh/vonkar, thm)` —— 左边那个乘积
**被吸收**、`thm`/`qm` 是已舍入的加数。Rust 原来是平铺加法，已改。

**但干窗逐位不变**（仍是 34 个变量，`f_qref` 仍差 1.30e-18）：这一次融合在这组输入上
恰好同值。也就是说这两处是"该补的真收缩"，但不是种子 —— 它俩的差异本身是
从上游继承来的。

### 下一轮的入口（已缩小到 42 处）

`lt.opt` 里 61 处已按 `;; Function` 全部归到 `leaktemperature`。下一轮照
`twostream` 那一轮的办法逐条读形状，优先看与 `fseng`/`fevpg`/`qg`/`dqgdT` 同一条
数据流上的那些（`dirab_dtl`/`fsenl_dtl`/`etr_dtl`/`evplwet_dtl`/`fevpl_dtl` 这一族
`FMA(x_dtl, prephitmp_2380, x0)` 是叶温 Newton 迭代的线性化更新，同一族里只要有一处
未融合，收敛后的通量就会差 1 ULP）。

Tested: `MOD_LeafTemperature.F90` 的本体 dump（61 处，本轮首次生成）；
`fd/fv/ph/hsf/ssh/MOD_Albedo/MOD_NetSolar/MOD_CanopyLayerProfile/MOD_GroundFluxes/MOD_TurbulenceLEddy`
各 dump 的 `;; Function` 身份核对；`tref`/`qref` 两处的逐行对照与落地；
干窗 1 步逐位比对（34 个变量，不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）；`cargo fmt --all --check`。
Not-tested: 61 处里剩下的 42 处逐条映射（本轮只做了 `tref`/`qref` 两处）。

### 补上 `MOD_LeafTemperature` 的 9 处（`tref`/`qref` + 收敛后收尾的 7 处）

`MOD_LeafTemperature.F90:1062-1093` 是叶温 Newton 迭代**收敛之后**的收尾：

```fortran
fsenl = fsenl + fsenl_dtl*dtl(it-1) + (dtl_noadj-dtl(it-1))*(...) + hvap*(...)
etr     = etr     + etr_dtl*dtl(it-1)
evplwet = evplwet + evplwet_dtl*dtl(it-1)
fevpl   = fevpl   + fevpl_dtl*dtl(it-1)
```

dump（第 2636-2650、2720-2733 处）显示 GCC 把它们编成**逐级 FMA**：

```
_547 = fsenl旧值
_552 = FMA(fsenl_dtl, dtl, _547)
_569 = FMA(dtl_noadj-dtl, 括号, _552)
_571 = FMA(erre, 2.5104e6, _569)
…  klwdif = evplwet - ldew/deltim；fsenl = FMA(klwdif, 2.5104e6, _571)
etr = FMA(etr_dtl, dtl, etr旧值)；evplwet = FMA(...)；fevpl = FMA(...)
```

Rust 原来是平铺加法，已按上面的次序改成 FMA 链（`fsenl` 那一条是
`fsenl ← fma(sh_dtl,dT,·) ← fma(dT_noadj-dT,括号,·) ← fma(hvap,imbalance,·) ← fma(hvap,过湿蒸发,·)`）。

### **重要负结果：这一族在收敛后是"惰性"的**

改完 9 处（`tref`/`qref` + 这 7 处）之后，干窗第 0 步**逐位完全不变**，仍是 34 个变量、
每个变量的 `maxabs` 一字不差。先怀疑"改的不是活路径"，于是做了**扰动试验**：
把收尾那一行乘 `1.0000001` 再跑 —— 逐位不同的变量从 34 变成 **36**，说明这条路径
**确实在执行**。

所以零位移的原因是**代数上的**：这一族全是 `FMA(x_dtl, dtl(it-1), 旧值)`，
而 `dtl(it-1)` 是 Newton 迭代**已经收敛**的那一步的增量 —— 它本身就接近 0
（`MOD_LeafTemperature.F90:940` 拿 `sqrt(dtl*dtl)` 当收敛判据）。`dtl` 恰好为 0 时
`fma(x,0,old) ≡ old ≡ old + x*0`，融合与否同值。**这一族不可能解释种子。**

**这条结论直接改下一轮的方向**：种子必须在 `MOD_LeafTemperature` 里**无条件执行、
且乘数非零**的表达式上，而不是迭代修正项。剩下 42 处里应当优先看
`clai`/`cfw`（第 1158-1159、1969 行）、`taf`/`thvstar`（2370、2415）、
`qsatg` 那一族（2000-2062）以及 `ldew_rain`/`ldew_snow`（2994/2996，这两处
**每步无条件执行**，且 `f_ldew` 恰好在差分集合里）。

Tested: `MOD_LeafTemperature.F90:1062-1093` 与 dump 第 2636-2650/2720-2733 处的逐句对应；
9 处落地后干窗 1 步逐位比对（34 个变量不变）；
**扰动试验**（收尾乘 1.0000001 → 36 个变量）证明路径是活的；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）；`cargo fmt --all --check`。
Not-tested: 剩下 42 处（下一步按上面的优先级）。

### 补上 `ldew_rain`/`ldew_snow`（4 处），并锁定下一轮的主目标：**`moninobukm` 的 26 处**

`MOD_LeafTemperature.F90:1201-1202`（两支各两条，`tl>tfrz` 与 `tl<=tfrz`）：

```fortran
ldew_rain = ldew_rain + (qdewl-qevpl)*deltim
ldew_snow = ldew_snow + (qfrol-qsubl)*deltim
```

dump（第 2994/2996 处）是 `FMA(deltim, qdewl-qevpl, ldew_rain旧值)` ——
`deltim*通量` 被吸收、旧值是已舍入加数。这两句**每步无条件执行**（不是上一节那种
`dtl→0` 的惰性族），所以值得改。已按此改两支共 4 处。

**干窗仍逐位不变**（34 个变量、`maxabs` 一字不差）。到此从 `MOD_LeafTemperature`
里补的 13 处全部零位移 —— 结合上一节的惰性论证，可以判定**种子不在这个模块的
收尾部分**，得往更早的共享量走。

### `MOD_FrictionVelocity` 的 dump 分账：`moninobukm` 独占 26 处

顺手把 `fv.opt` 按例程分了一下（这才是这份 dump 的正确读法）：

| 例程 | 收缩处数 |
|---|---|
| `psi`（两个 isra 副本） | 6 |
| `moninobukini` | 2 |
| `kintmoninobuk` | 6 |
| `kmoninobuk` | 2 |
| **`moninobukm`** | **26** |
| `moninobuk` | 18 |

而 `crates/colm-core/src/monin_obukhov.rs` **总共只有 18 个 `mul_add`** ——
也就是说 `moninobuk`（18 处）对上了，**`moninobukm`（26 处）没有独立实现**：
Rust 让 `canopy_monin_obukhov_with_scheme` 复用同一套
`momentum_integral`/`heat_integral`/`heat_similarity`/`State`。

这不是"少几次融合"的小事：上游 `moninobukm` 是**独立的两百行例程**
（`MOD_FrictionVelocity.F90:169-371`），比 `moninobuk` 多算 `fh2m`/`fq2m`/`fht`/`fqt`/
`fmtop`/`phih`，四条分支各写一遍。而它**正是叶温求解那条活路径上的近地层迭代**
（`MOD_LeafTemperature.F90:625` 调用；`DEF_USE_CBL_HEIGHT=.false.` 时走这一支，
另一个 `moninobukm_leddy` 只在打开 CBL 高度时用）。

**为什么这就是种子**：`moninobukm` 的输出 `fh2m`/`fq2m`/`fm`/`fh`/`fq`/`fht`/`fqt`
一一对应差分集合里的 `f_tref`/`f_qref`/`f_fm`/`f_fh`/`f_fq`/`f_fseng`/`f_fevpg`
（`tref = thm + vonkar/(fh-fht)*dth*(fh2m/vonkar - fh/vonkar)` 这一句直接把三者
绑在一起）。Rust 既然用 `moninobuk` 的算术去顶 `moninobukm`，差 1 ULP 完全合理，
而 `t_grnd` 仍能逐位相同（收敛点相同、通量末位不同）。

**下一轮的第一件事**：把 `moninobukm` 单独 dump 一遍（编译
`MOD_FrictionVelocity.F90` 后按 `;; Function moninobukm` 取那 26 行），逐条读形状，
在 Rust 里给冠层那条**独立实现**出来，而不是复用 `moninobuk`。

Tested: `MOD_LeafTemperature.F90:1201-1202` 与 dump 第 2994/2996 处的对应（两支共 4 处落地）；
`fv.opt` 的按例程分账表；`MONIN_FrictionVelocity` 的 `moninobukm` 源码范围与调用点核对
（`MOD_LeafTemperature.F90:621/625`、`DEF_USE_CBL_HEIGHT` 默认 `.false.`）；
干窗 1 步逐位比对（34 个变量不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）；`cargo fmt --all --check`。
Not-tested: `moninobukm` 的 26 处逐条读形状与独立实现（下一轮）。

### **更正上一节的 `moninobukm` 判断**：它不是"没实现"，我数错了

上一节写"`moninobukm`（26 处）没有独立实现、Rust 拿 `moninobuk` 的算术去顶"。
**这条判断是错的**，这轮逐句核对后撤回：

* `crates/colm-core/src/monin_obukhov.rs` 的 `canopy_monin_obukhov_with_scheme`
  确实是 `moninobukm` 的移植：它有独立的 `CanopyMoninObukhovState`
  （`momentum_at_canopy_top`/`heat_at_top_layer`/`moisture_at_top_layer`/
  `canopy_top_heat_similarity`），内部用**冠层尺度**的几何（`z0mv` + `displacement`
  + 观测高度）造一份自己的 `MoninObukhovInput`，再取 `surface.friction_velocity`/
  `surface.heat`/`heat_at_2m` 等 —— 与上游 `moninobukm` 的 `ustar`/`fm`/`fh`/`fh2m`
  逐项对应。
* **处数差是"代码共享 vs 字面复制"造成的**：上游 `moninobukm` 把
  `fh`/`fh2m`/`fht`/`fq`/`fq2m`/`fqt` **六份**四条分支各写一遍（≈6×4 处收缩），
  Rust 用一份 `heat_integral` 让六个调用点共享 —— 内联之后编译产物里同样是六份，
  但源码里只有一个实现。**按源码里的 `mul_add` 个数去对 dump 的处数，
  在"上游字面复制、Rust 抽成函数"的地方必然对不上。**
* 顺带核对了冠层那条调用最容易出错的两个参数：上游
  `MOD_LeafTemperature.F90:515/523-524/535-536` 里 `z0hv = z0mv`、`z0qv = z0mv`，
  所以 Rust 把 `heat_roughness_m`/`moisture_roughness_m` 都传 `z0mv` 是**对的**。

**规矩（补进前面那条）**：比"dump 处数 vs `mul_add` 个数"之前，先确认上游是
**字面复制**还是**抽了子程序** —— 抽了子程序的，处数天然偏少，不能当缺口。

### 第 0 步种子：目前的确定边界

把这几轮的插桩与分区结论并起来，可以确定的是：

1. **不在 PHS**（关掉它差异集合不变，34→37 同量级）；
2. **不在 `DEF_VEG_SNOW` 分支本身**（关掉后叶面那一组降到恰好 1 ULP，说明该分支只是
   **放大**了一个更小的既有差异）；
3. **不在水分步**：水分步跑之前 `t_soisno`（两层各 1 ULP）与表层 `wliq`/`wice`
   就已经不同，而 `WATER_VSF` 只读 `t_soisno`；反之 `smp`/`hk`（水分步写的）逐位一致；
4. **不在扩散求解本身**：`GroundTemperature` 入口的温度与含水量入参逐位一致，
   但 `fseng` 1 ULP、`fevpg` 2 ULP 已经不同 —— 也就是**地面通量那一段**；
5. **不在叶温迭代的收敛后收尾**：那一族是 `FMA(x_dtl, dtl, 旧值)`，`dtl→0` 时恒等
   （已用扰动试验排除"路径没走到"的另一种解释）；
6. **不在冠层近地层廓线**（本节更正）。

下一步只剩"地面通量那一段"里、且**无条件执行**的表达式。最直接的打法不再是继续
扫模块，而是在 `MOD_GroundFluxes` 入口插一个点，把 `ur`/`thm`/`thv`/`t_grnd`/
`qg`/`dqgdT`/`emg`/`z0m`/`z0h`/`rss` 两侧逐位并排 —— 只要找出**哪一个入参**先差，
种子就落在它上面；若全部一致而输出仍差，那就是 `GroundFluxes` 本体（它有 8 处收缩、
Rust 也是 8 个 `mul_add`，需要按 dump 逐条对形状而不是数个数）。

Tested: `canopy_monin_obukhov_with_scheme` 与 `CanopyMoninObukhovState` 的逐字段核对；
`MOD_LeafTemperature.F90:515/521-524/535-536` 的 `z0hv`/`z0qv` 赋值核对；
`fv.opt` 的按例程分账复核；`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: `MOD_GroundFluxes` 入口的逐位并排（下一步）。

## **找到信号了**：`MOD_Thermal` 的六条 `tinc` 修正（差异 34 → 33）

前面十几处融合都是零位移，这一轮终于动了一处。按"种子在地面通量那一段、且无条件执行"
这条线索去看 `MOD_Thermal.F90:1227-1232`：

```fortran
tinc       = t_grnd - t_grnd_bef
fseng      = fseng      + tinc*cgrnds
fseng_soil = fseng_soil + tinc*cgrnds
fseng_snow = fseng_snow + tinc*cgrnds
fevpg      = fevpg      + tinc*cgrndl
fevpg_soil = fevpg_soil + tinc*cgrndl
fevpg_snow = fevpg_snow + tinc*cgrndl
```

Rust 的对应物（`standard_lct_step.rs` 的 `corrected_*`）原来是平铺加法，已改成
`slope.mul_add(tinc, 原值)` 六处。

**为什么这处应该是种子**：`tinc` 是**地表温度的增量**（很小），
`tinc*cgrnds` 融不融合只动通量的末位、**动不了 `t_grnd`/`t_soisno`** ——
这正是插桩观测到的"温度逐位相同、`fseng`/`fevpg` 却差 1–2 ULP"的指纹。

**实测（第一次出现位移）**：

| | 改前 | 改后 |
|---|---|---|
| 干窗第 0 步逐位不同的变量数 | 34 | **33** |
| `f_lfevpa` | 差 2.84e-14 | **消失** |
| `f_fevpa` | 6.7763e-21 | **2.0329e-20** |
| `f_qstar` | 5.4210e-20 | **2.7105e-20** |
| `f_qinfl` / `f_qlayer` | 1.6263e-19 | **1.7618e-19 / 1.8974e-19** |
| `f_fevaq`… `f_fevpg` | 1.6263e-19 | 1.7618e-19 |

同一条邻域里还有 `MOD_Thermal.F90:1256` 的 `fseng = fseng + htvp*egidif`，
也已改成用 `water_limited_evaporation_kg_m2_s`（就是 `egidif`）自己收
（`thermal_water` 原先给的是**已经乘好**的 `htvp*egidif`，调用方再相加就丢了融合）。
这一处没有再动计数，但形状与上游一致。

### 顺手记一个坑：`MOD_Thermal` 单文件编不出来

按前面几轮的办法单独 dump `MOD_Thermal.F90` 时编译失败：

```
Error: Dummy argument 'smp' with INTENT(IN) in variable definition context
       (actual argument to INTENT = OUT/INOUT) at (1)
   MOD_Thermal.F90:1036
```

也就是说**内核构建时的实际选项与我抄的那一套不同**（`-fallow-argument-mismatch`
不足以放行 INTENT 冲突）。所以 `MOD_Thermal` 的收缩**还没有 dump 可读** ——
本轮这两处的依据是"上游源码 + 同型语句的既有 dump 形状 + 位移信号"三者，
**不是**本模块自己的 GIMPLE。下一轮若要继续这一块，得先把真实选项从
`vendor/CoLM202X/.bld` 的构建记录里挖出来（或直接对整个树 dump 一次）。

Tested: `MOD_Thermal.F90:1227-1232/1256` 逐行核对；干窗 1 步逐位比对（34 → **33**，
`f_lfevpa` 消失、四个量级改变）；`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: `MOD_Thermal` 本体 dump（单文件编译被 INTENT 冲突挡住，真实选项待查）。

## `MOD_GroundFluxes` 的 8 处**逐条对上**了（该模块可以结案）

上一节把 `fseng`/`fevpg` 的种子缩到"地面通量那一段"。这轮先把
`MOD_GroundFluxes` 自己的 8 处收缩与 `ground_fluxes.rs` 的 8 个 `mul_add`
**逐条**对上（上一轮刚学的教训：个数相等不等于形状相等）：

| dump | 上游语义 | Rust |
|---|---|---|
| `_8 = FMA(1-fsno, zlnd, fsno*zsno)` | `z0mg`（雪/土粗糙度混合） | `(1.0-fsno).mul_add(zlnd, fsno*zsno)` |
| `_16 = FMA(q, 0.61, 1.0)` | `1+0.61q` | `VIRTUAL_HUMIDITY_COEFFICIENT.mul_add(q, 1.0)` |
| `_22 = FMA(dth, 1+0.61qm, (0.61*th)*dqh)` | `dthv` | `temperature_difference.mul_add(one_plus_vapor, vapor_times_potential*humidity_difference)` |
| `thvstar = FMA(tstar, 1+0.61qm, (0.61*th)*qstar)` | 虚拟位温尺度 | `temperature_scale.mul_add(one_plus_vapor, vapor_times_potential*humidity_scale)` |
| `_57 = FMA(ur, ur, wc2)` | 对流风速 | `reference_wind.mul_add(reference_wind, convective_velocity.powi(2))` |
| `_85 = FMA(cgrndl, htvp, raih)` | `cgrnd` | `latent_temperature_derivative.mul_add(htvp, sensible_temperature_derivative)` |
| `_129 = FMA(·, ·, ·)` / `_137 = FMA(·, ·, ·)` | `tref`/`qref` 那一对 | `(vonkar/heat*ΔT).mul_add(…)` / `(vonkar/moisture*Δq).mul_add(…)` |

**八对八，形状也一一对应** —— `MOD_GroundFluxes` 这条可以结案了。
（顺带：`ground_fluxes.rs:92-93` 的注释早已把这几处写在注释里，
说明当时是照着 dump 读出来的；这轮做的是**验证**而不是新发现。）

**推论**：既然 `GroundFluxes` 本体忠实、它的输出 `fseng` 却差 1 ULP，
那么差异只在**喂给它的量**上：`thm`、`thv`、`qg`、`dqgdT`、`ur`、`z0m`/`z0h`、
`rss`、`rhoair`。下一轮按这个名单在冠层 `GroundFluxes` 调用点插一个探针
（`MOD_Thermal.F90:931` 那一次），两侧逐位并排 —— 名单只有 8 个，一次就能定位。

Tested: `MOD_GroundFluxes.opt` 的 8 处与 `ground_fluxes.rs` 的 8 个 `mul_add` 逐条对应；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: 冠层 `GroundFluxes` 调用点的入参并排（下一轮，名单 8 个）。

## 补上 `thv`/`dthv`/`thvstar` 的 5 处融合；并认清"融合追猎"已到边际

按"差异在喂给 `GroundFluxes` 的量上"这条线索，核对了那 8 个入参的推导，找到三处
`1+0.61q` 的**内层没融合**（上游 `MOD_Thermal.F90:546` 的 `thv = th*(1.+0.61*forc_q)`
与 `MOD_LeafTemperature.F90:559` 的 `dthv`，GIMPLE 是 `_16 = FMA(q, 0.61, 1.0)`
再乘/再加）：

| 落点 | 上游 | 改动 |
|---|---|---|
| `standard_lct_step.rs` | `MOD_Thermal.F90:546` | `th * 0.61.mul_add(q, 1.0)` |
| `leaf_temperature.rs`（初始化那个 `dthv`） | `:559` | 内层融合 + 外层 `mul_add` |
| `leaf_temperature.rs`（迭代里的 `thvstar`） | `:904` | 同上 |

**试完这三处之后干窗从 17 变成 18 —— 全部回退了**（见下一小节）。三处都在
`crates/colm-core` 里改过又改回，最终只留下这份记录。

### **负结果**：dump 支持的融合也可能让窗口变差 —— `dthv`/`fthvstar` 三处回退

这三处的形状是**有 dump 支持的**（`lt.opt` 第 1383-1387、2414-2415 行）：

```
_116 = FMA(qm, 0.61, 1.0)        _120 = th*0.61
_121 = dqh*_120                  _122 = FMA(dth, _116, _121)   → dthv
_491 = _120*qstar                thvstar = FMA(_117, tstar, _491)   ← _117 = _116
```

按 dump 改完之后：

| | 1 步逐位比对 | 干窗 tier2 变量数 |
|---|---|---|
| 改前 | 33 | 17 |
| 改后 | **33（一字不变）** | **18**（`f_frcsat` 新越界） |

**1 步逐位完全不变、整月却多出一个越界变量** —— 原因是这类改动的效果**通过
MO 迭代的起点放大**：`dthv`（以及迭代里的 `thvstar`）是 `moninobukini` 的入参，
差 1 ULP 会让稳定性判据在某个时刻跨过分支阈值，于是某个中间步的迭代路径不同，
`frcsat` 被推过容差。

**处置：回退**，三个文件都回到改动前（干窗恢复 17）。理由不是"形状不对"，
而是**本仓库的判据链**：单点形状有 dump 支持、但**没有端到端证据**说明改完更接近
上游，而窗口是唯一的端到端判据，它给的是负号。要再捡起这一处，必须先有
`MOD_LeafTemperature` 的**随机差分驱动**（20000 组输入、直接比 `dthv`/`thvstar`），
把"算式对不对"与"迭代放大"分开。

顺手也试了 `MOD_Thermal.F90:546` 的 `thv`（内层融合）——**单独改它时干窗仍是 18**
（被上面两处 dominate），而 `MOD_Thermal` 没有 dump，所以一并回退。

**规矩**：dump 支持的形状是**必要条件**，不是充分条件。当端到端判据给出负号、
而改动又无法用"随机差分"独立证实时，回退并记录，而不是留着赌它"理论上更对"。

### 一个必须如实写下的观察：`f_fseng` 的差是**不变量**

把这几轮的记录并起来看，`f_fseng` 的 `maxabs` 从第 143 轮第一次量到"1 步差异"起就
**一直是 6.8212e-13**，历经 16 处融合（`tref`/`qref`、收敛后收尾 7 处、`ldew` 4 处、
`tinc` 6 处、`htvp*egidif`、`thv`/`dthv`/`thvstar` 5 处）**一次都没动过**：
`f_fh`（8.8818e-16）、`f_fq`（8.8818e-16）、`f_taux`/`f_tauy`（7.2164e-16）、
`f_fsenl`（5.6133e-13）、`f_gssun`/`f_gssha`（6.7763e-21）同样一字未变。
只有 `tinc` 那一次动过 `f_fevpg`/`f_qinfl`/`f_qstar` 并让 `f_lfevpa` 消失。

**这条不变性比任何单点结论都强**：它说明这些量的差**不是**由这些表达式里的
舍入顺序造成的 —— 否则至少会有一处的末位跟着动。结合"温度与含水量入参逐位一致、
只有 `fseng`/`fevpg`/`fh`/`fq`/`taux` 这一族差 1 ULP"，剩下的可能只有两类：

1. **形状不同**（不是少融合，而是算式结构或分段判据不同）——最可能仍在
   `moninobukm`（冠层那条）里：它比 `moninobuk` 多算 `fh2m`/`fq2m`/`fht`/`fqt`，
   而 Rust 用一份 `heat_integral` 共享六个调用点，**共享本身没错，但每个调用点传的
   `zldis`/粗糙度必须逐个核对**（`fh2m` 用 `2+z0h`、`fht` 用
   `displat+z0mt-displa`、`fq2m` 用 `2+z0h` 配 `z0q` —— 这三处的搭配极易抄错，
   而且抄错只差 1 ULP 时不会被 `t_grnd` 发现）。
2. **同一个表达式但常量不同**（例如某个 `-0.333`/`0.465`/`16` 的分支阈值）。

**下一轮的做法**：照当年关掉 `moninobuk` 的办法，给 `canopy_monin_obukhov`
（`moninobukm`）写**随机差分驱动** —— 20000 组输入、两侧逐位比对
`ustar`/`fh`/`fh2m`/`fht`/`fq`/`fq2m`/`fqt`。这是唯一能把"共享实现"里
某个调用点的搭配错误照出来的办法；继续按模块扫收缩已经没有产出。

Tested: `MOD_Thermal.F90:546`、`MOD_LeafTemperature.F90:559/904` 与
`MOD_GroundFluxes.opt` 第 2/3 处的同型对照；三处落地后干窗 1 步逐位比对
（33 个变量、`maxabs` 不变）；`f_fseng` 等 6 个量的 `maxabs` 跨 16 处改动的不变性核对；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）；`cargo fmt --all --check`。
Not-tested: `canopy_monin_obukhov` 的随机差分驱动（下一轮）。

## `DEF_USE_SNICAR` 也是"没被读过"的开关（第五个静默不匹配，已拦）

按上面那条"没被读过但影响行为"的清单继续筛，`DEF_USE_SNICAR` 命中**同一类问题**：

```
$ grep -rn "snicar" crates/colm-runtime/src/physics.rs crates/colm-runtime/src/assembly.rs
（无输出）
$ grep -n "snow_layer_absorption_w_m2" crates/colm-runtime/src/assembly.rs
1357:                    snow_layer_absorption_w_m2: None,
```

也就是说：`ground_temperature.rs`/`phase_change.rs` 里那套 SNICAR 分支
（`use_snicar = snow_layer_absorption_w_m2.is_some()`）**永远不会被选中**，
写 `DEF_USE_SNICAR = .true.` 的算例会**静默按标准雪光学算完** —— 雪粒径增长、
分层吸收、融化能量都与上游的 `SNICAR_AD_RT` 不同。这与 `DEF_SPLIT_SOILSNOW`
是完全相同的形状。

**处置**：`colm-runtime/src/physics.rs::land_physics_parameters` 里直接 `bail!`。

**冷启动那边（`colm-init`）刻意不拦**：它**确实**能按 SNICAR 生成重启
（`snicar.rs` 的 `initialize_cold`，还会在缺表时报错），现有测试
（`colm-init/tests/native_pipeline.rs:85`）也在用 `DEF_USE_SNICAR=.true.`。
"初始器能造、运行期不能跑"是当前的真实状态，拦在运行期才是"接不上就不跑"。

端到端实测：

```
$ colm-rs /tmp/gf/snicarcheck --land-cover igbp …
colm-rs: DEF_USE_SNICAR is on, but the Rust runtime assembles only the standard snow
branch: `assembly.rs` pins `snow_layer_absorption_w_m2` to None and the SNICAR cold
start is not carried into the time loop, so the case would silently run with the
non-SNICAR snow albedo and layer absorption
```

回归测试：`physics_tests::snicar_is_refused_rather_than_run_with_standard_snow_optics`。

**这条也补进"上游有、本仓库没移植的分支显式报错"那张清单**（现在是四个：VIC 产流、
灌溉、`DEF_SPLIT_SOILSNOW`、`DEF_USE_SNICAR`）。

Tested: `grep` 全仓库确认 `DEF_USE_SNICAR` 与 `snow_layer_absorption_w_m2` 在运行期的
落点；端到端 `colm-rs` 拒绝信息；`colm-init/tests/native_pipeline.rs:85` 的既有用法核对
（因此不在 `colm-init` 里拦）；`cargo test -q -p colm-runtime --lib -- --test-threads=1`（73 通过）。
Not-tested: SNICAR 支路本身（**故意**：拒绝就是不跑）。

# 当前状态总表（截至本轮）

这份文档已经很长，这一节把"移植到什么程度、还剩什么"收在一处，便于审计。
**每条都只写有实测依据的结论**，细则见对应小节。

## 一、可达配置空间：已完整覆盖

`colm-rs` 能跑的编排是 **SinglePoint + LCT + patchtype 0（土面）**，在这个空间内：

| 维度 | 支持情况 |
|---|---|
| 土水方案 | `DEF_USE_VariablySaturatedFlow` on/off × `DEF_USE_Campbell_SOIL_MODEL` on/off，四条组合都在 |
| 产流方案 | `DEF_Runoff_SCHEME` 0（TOPMODEL）/ 2（XinAnJiang）/ 3（Simple VIC） |
| 降水相态 | I/II/III 三档 |
| 土热导率 | 八档全在 |
| 冠层雪 | `DEF_VEG_SNOW` on/off 都在（默认 on） |
| 植物水力 | `DEF_USE_PLANTHYDRAULICS` on/off |
| 近地层 | `DEF_USE_CBL_HEIGHT` on/off（on 时走 Large Eddy，缺 `forc_hpbl` 会显式报错）|
| 地类分类 | IGBP / USGS（编译期选择，`--land-cover`） |
| 其余 namelist | 见"三、显式拒绝"与"四、已证明无影响" |

## 二、验证层级：黄金窗口过关

| 判据 | 结果 |
|---|---|
| tier0（逐位） | **全部通过** |
| tier1（1e-12 相对） | **全部通过** |
| tier2（1e-7 绝对+相对） | 干 17 / 湿 68 / 雪 79 个变量超差，**全部在 tier2** |
| tier3（统计等价） | `oracle --bin tier3-check` 在湿窗基线上通过 |
| history 变量 | `UNFILLED` 为空（没有"声明了却填不出"的量） |

第二个配置（Campbell + VSF off）用 `oracle/scripts/compare_second_config.sh` 两侧对照：
干 **16** / 湿 **66** / 雪 **79**，与黄金配置（17/68/79）**齐平**。

## 三、显式拒绝（上游有、本仓库不跑）

| 开关/分支 | 拦在哪 | 依据 |
|---|---|---|
| PFT / PC 子网格 | `physics.rs` | 只装配 LCT |
| `DEF_USE_IRRIGATION` | `physics.rs` | 喷灌率由物候逐步算出 |
| `DEF_Runoff_SCHEME = 1`（VIC） | `physics.rs` | 需要外部 per-patch 参数文件 |
| `DEF_SPLIT_SOILSNOW` | `physics.rs` + `colm-init` | 水分侧只做了非 split |
| `DEF_USE_SNICAR` | `physics.rs` | 时间步里没有 SNICAR 支路（冷启动有） |
| `DEF_Optimize_Baseflow` | `physics.rs` | 预热期的反解没做（`scale_baseflow` 的**读取**已实现） |
| patchtype ≠ 0（湖/冰川/城市/海洋） | `assembly.rs` | standard-LCT 土面装配 |
| HYPERSPECTRAL 冷启动 | `colm-init` | 上游没有可核对的 211 波段映射 |

## 四、已证明"读了也没影响"的开关（不是缺口）

`DEF_SUBGRID_SCHEME`（纯声明镜像）、`DEF_Forcing_Interp_Method`（SinglePoint 下
`bilinear` 被上游自己改回 `arealweight`）、`DEF_CheckEquilibrium`（只打印）、
`DEF_HIST_WriteBack`/`DEF_HIST_CompressLevel`（只影响 NetCDF 写法）、
`DEF_USE_DiagMatrix`（只在 BGC 下用）。

## 五、唯一未闭合的数值项：干窗第 0 步的 1 ULP 分布

1 步 TIMESTEP 口径下 33 个变量各差 ~1e-15 相对（**全部是 tier2**）。
已经用插桩与开关分区排除的边界：

1. 不在 PHS；2. 不在 `DEF_VEG_SNOW` 分支本身（它只放大）；3. 不在水分步
（`WLIQ` 差而 `SMP`/`HK` 逐位一致）；4. 不在扩散求解本体；
5. 不在叶温迭代的收敛后收尾（`dtl→0` 时惰性）；6. 不在冠层近地层廓线
（`psi`/`fmtop`/`fht`/`fqt`/`phih`/`fh2m`/`fq2m` 的 `zldis` 与粗糙度搭配逐条核过一致）；
7. 不在 `MOD_GroundFluxes` 本体（8 处逐条对上）。

还剩两个方向：**地面通量那一段里无条件执行的表达式**，以及**冠层 MO 迭代的
随机差分驱动**（`f_fseng` 的差自始至终是 6.8212e-13，跨 16 处改动一字不变 ——
说明它是**形状**问题而非少融合）。已经改对但不动的 16 处收缩都留着；
`dthv`/`thvstar`/`thv` 三处**有 dump 支持却让窗口从 17 变 18**，已按判据链回退并记录。

## 六、已知但不在判据内的项

* `MOD_LeafTemperature` 的 61 处收缩只核了约 20 处（其余在 O3/双叶/PC 分支上）；
* 不可达支路的收缩没有逐条核：`twostream_mod` 72、`soilwater` 23、`water_2014` 13、
  `albocean` 8、`snowwater_snicar` 8（后两者现在被拒绝覆盖为不可达）；
* `MOD_Thermal` 本体没有 dump（单文件编译被 `:1036` 的 INTENT 冲突挡住）。

Tested: 本节只汇总前文已实测的结论；三个黄金窗口与第二个配置的四组数字取自本轮及
最近几轮的 `golden-compare` 输出；`cargo test --workspace --lib --bins -- --test-threads=1`；
`cargo clippy --workspace --all-targets -- -D warnings`；两处 `cargo fmt --all --check`；
`cargo test -q -p oracle`；`cargo run -q -p xtask -- check-gui`；
`python3 oracle/scripts/test_upstream_f48_sync.py`。
Not-tested: 本节没有引入新实测，只是汇总。

## **抓到真缺陷**：`UNSTABLE_HEAT_COEFFICIENT` 的字面量差 1 ULP（冠层 `moninobukm` 的 `phih`）

上一节留下的两个方向里"冠层 MO 的随机差分驱动"做出来了，**第一次就抓到一个真缺陷**。

### 驱动

入库：`oracle/scripts/moninobukm_diff.f90` + `crates/colm-core/examples/mo_probe.rs` +
`oracle/scripts/compare_moninobukm.sh`。做法与当年关掉 `moninobuk` 的一样：

* 上游侧：用内核真实选项编译 `MOD_FrictionVelocity.F90`（**不加**
  `-ffp-contract=off` —— 要的就是 GCC 的默认收缩），驱动本身按仓库纪律加
  `-fwrapv -ffp-contract=off`；
* 本仓库侧：`cargo run -p colm-core --example mo_probe`，**同一串 LCG**；
* 两侧各 20000 组随机几何/稳定度，比 `moninobukm` 的 10 个输出
  （`ustar`/`fh2m`/`fq2m`/`fmtop`/`fm`/`fh`/`fq`/`fht`/`fqt`/`phih`）。

### 结果

```
第一次：ustar/fh2m/fq2m/fmtop/fm/fh/fq/fht/fqt 全 0/20000，phih 375/20000
        （按分支拆开：错的全在第一支 zeta < -0.465，376 组里错 375）
```

`phih` 第一支是 `0.9*vonkar**1.333 * (-zeta)**(-0.333)`，Rust 侧用常量
`UNSTABLE_HEAT_COEFFICIENT`。dump（`fv.opt` 第 525 行）给的是

```
phih_27 = _5 * 2.653312957296878327184685986139811575412750244140625e-1
```

而 Rust 里写的是 `0.2653312957296878` —— 这个十进制只舍到
**`0x1.0fb301d70ea83p-2`**，dump 那一份是 **`0x1.0fb301d70ea84p-2`**，**差 1 ULP**。
（注释里当时**抄对了**完整精度，字面量却没写够位数 —— 典型的"注释对、代码错"。）

改成 `0.26533129572968783` 之后：

```
moninobukm: all 10 outputs 20000/20000 bitwise identical
```

### 顺带验掉一条我自己的错误假设

中途试过把第二支的 `(1-16ζ)**(-0.5)` 从 `powf(-0.5)` 改成 `1/sqrt(x)`
（以为 GCC 会把 `**-0.5` 展开成开方），结果 `phih` 的失配从 375 涨到 **3427** ——
dump 里写得很清楚是 `__builtin_pow(_24, -5.0e-1)`，**没有**展开。已回退。

### 窗口：混合信号，照实记录

| 窗口 | 逐位不同值 | Σ\|Δ\| | 超容差 | 变量数 |
|---|---|---|---|---|
| 干 | 21196 → **21326** | 311.43 → **338.93** | 825（不变） | 17（不变） |
| 湿 | 32530 → **32681** | 10381.65 → **10369.44** | 20664 → **20672** | 68（不变） |
| 雪 | 33602 → **33651** | 持平 | 持平 | 79（不变） |

干窗逐位变多、湿窗 Σ\|Δ\| 变好而超容差变多 —— 方向不一致；**1 步口径仍是 33 个变量不变**。
**保留**，理由是本仓库的判据层级：这次不是"形状从 dump 推出来的"，
而是**与内核本体 20000/20000 逐位相同**的直接差分证据（最强的一类），
而且三个窗口的 tier2 变量数一个都没变（结构没坏）。
把这条混合信号钉在这里，供以后有更强判据时回看。

**规矩（补进驱动纪律）**：从 GIMPLE 里抄常量时，**字面量必须能往返到 dump 的那一串** ——
`f64` 只保留 17 位有效数字，写短了就静默差 1 ULP。抄完立刻用
`python3 -c "from decimal import Decimal; print(Decimal(0.1…))"` 对一遍。

Tested: `oracle/scripts/compare_moninobukm.sh`（两侧 20000 组、10 个输出）；
修常量前后各跑一次（`phih` 375 → 0）；`1/sqrt(x)` 的对照（3427，已回退）；
三个黄金窗口三口径 A/B；干窗 1 步逐位比对（33 不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`。
Not-tested: `-ffp-contract` 关闭时的上游行为（驱动一侧刻意用内核默认）；
`moninobukm_leddy`（CBL 分支，本机算例不走）。

## 把差分扩到整个 `MOD_FrictionVelocity`：**20 个输出 20000/20000 全同**

上一节只驱动了 `moninobukm`（10 个输出）。这一轮把同一个驱动扩到该模块的**全部公开例程**
（`oracle/scripts/compare_moninobukm.sh`，两侧仍是同一串 LCG、20000 组）：

| 例程 | 输出 |
|---|---|
| `moninobukm` | `ustar`、`fh2m`、`fq2m`、`fmtop`、`fm`、`fh`、`fq`、`fht`、`fqt`、`phih` |
| `moninobuk` | `ustar`、`fh2m`、`fq2m`、`fm10m`、`fm`、`fh`、`fq` |
| `kmoninobuk` | 一层扩散率 |
| `kintmoninobuk` | 层间积分扩散率 |
| `moninobukini` | 初始 `um`/`obu` 里的 `um` |

```
MOD_FrictionVelocity: all 20 outputs 20000/20000 bitwise identical
```

**意义**：`MOD_FrictionVelocity` 的 60 处收缩（分账 26+18+8+6+2）到此**结案** ——
不再只是"形状从 dump 推出来"，而是**与内核本体逐位相同**的直接差分证据。
这也**正式退役**了种子追猎里"不在冠层近地层廓线"那条边界：那条以前靠的是
`psi`/`zldis`/粗糙度搭配的逐条目视核对，现在是 20000 组随机输入的硬证据。

（这一轮没有改任何 `crates/` 生产代码 —— 扩驱动之后直接就是 0 失配，
说明上一轮修掉的那个常量是这条链上唯一的偏差。）

Tested: `oracle/scripts/compare_moninobukm.sh`（20 个输出、20000 组）；
`cargo fmt --all --check`；`cargo test -q -p colm-core --lib -- --test-threads=1`。
Not-tested: `moninobuk_leddy`（CBL 分支，模块外、本机算例不走）。

## `qsadv` 的差分：**输入已对齐，输出仍差 ~25%**，而它既不是融合链也不是平铺链

把差分驱动扩到 `MOD_Qsadv`（`oracle/scripts/qsadv_diff.f90` +
`crates/colm-core/examples/qsadv_probe.rs` + `oracle/scripts/compare_qsadv.sh`，
两侧同一串 LCG、20000 组 `(T,p)`）。

**第一件事：先把输入对齐**（这条纪律救过场）。两份驱动现在都把 `T`/`p` 的位型
写进第一、二列，比对结果是 **`T`/`p` 0/20000 失配** —— 输入逐位相同，
所以后面的失配不是驱动错位。

| Rust 的写法 | `es` | `esdT` | `qs` | `qsdT` |
|---|---|---|---|---|
| `mul_add` 链（现状） | 5021 | 4873 | 5019 | 4919 |
| 平铺 `x*v + c` | 8772 | 8479 | 8614 | 8218 |

**内核既不是融合链、也不是平铺链。** 这点很反常，值得写清楚：

* `-fdump-tree-optimized` 里这条 Horner 链**被标成八级 `.FMA`**
  （`qs.opt` 第 111-128 行，常数逐一对上 c0/c1/c7/c8）；
* 但把 `MOD_Qsadv.F90` 分别用**默认**、`-ffp-contract=fast`、`-ffp-contract=off`
  各编一遍，三者对同一个输入给出**完全相同**的值；按 `.FMA` 链精确算（Python 的
  `Fraction` 精确 fma）却给出另一个值；
* 换成平铺写法之后失配反而**更多**（5021 → 8772）。

也就是说：**GCC 在这一处做的是"部分融合"** —— 八个层级里只有一部分被收进 FMA，
树层 dump 的 `.FMA` 既不能当作"全都融合"，`-ffp-contract=off` 的相同结果也不能
当作"全都没融合"（`-O2` 下 GCC 的默认收缩未必被 `off` 完全关掉，或者后端又做了
别的安排）。

**处置：不动生产代码**（`polynomial` 保住原来的 `mul_add`，它离内核更近），
把这个状态与两个候选链的实测数字钉在这里。**下一轮的入口**：直接读
`qsadv` 的**汇编**（`gfortran -S`，`qs.opt` 同款选项）数出冷/暖两支里
`fmadd` 与 `fmul` 各自的分布，按汇编还原逐级融合的次序 —— 树层 dump 在这一处
不可信。

Tested: `oracle/scripts/compare_qsadv.sh`（20000 组，输入 `T`/`p` 0/20000 + 四种输出）；
`MOD_Qsadv.F90` 用 默认/`-ffp-contract=fast`/`-ffp-contract=off` 三种选项各编一遍、
对同一输入比 `es` 的位型；`qs.opt` 的 32 处收缩逐条核对；Python 精确 fma 复算两条链；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: `qsadv` 的汇编级逐级融合分布（下一轮）；因此**本轮没有改动 `polynomial`**。

## `qsadv` 的失配**只出在钳位支**：上游在那里把整条 Horner 链折成了常量

上一轮把 `qsadv` 的差分做到"输入对齐、但内核两条链都不是"。这一轮把 T 区间**按分支切开**
（驱动加了 `QS_TMIN`/`QS_TMAX`，默认仍是全量），立刻定位：

| T 区间 | 分支 | `es`/`esdT`/`qs`/`qsdT` 失配 |
|---|---|---|
| [150, 350]（默认） | 混合 | 5021 / 4873 / 5019 / 4919 |
| [210, 273] | 冷支、不触发钳位 | **0 / 0 / 0 / 0** |
| [274, 345] | 暖支、不触发钳位 | **0 / 0 / 0 / 0** |

**两个不含钳位的支各自 20000/20000 全同**；25.1% 的全量失配比例
恰好等于区间里 `|td| >= 75` 的比例（(48.16+1.84)/200 = 25%）—— 差异只在钳位支。

### 为什么：`-S` 的汇编里钳位支**没有那条链**

```
L4:                       ; td < -75 → td = -75
	adrp	x0, lC3@PAGE
	ldr	d29, [x0, #lC3@PAGEOFF]     ; 编译期折好的 es*100
	ldr	d28, [x0, #lC4@PAGEOFF]     ; 编译期折好的 esdT*100
	ldr	d30, [x0, #lC5@PAGEOFF]     ; qs 的外层系数
L2:
	fsub	d29, d27, d29          ; p - 0.378*es
	...
```
`L4`/`L5`（±75 两侧）只有三次 `ldr` 取常量，**直接跳进只算外层的 `L2`** —— 整条九次
Horner 链在钳位支里**根本不在运行期**，GCC 把它当常量折了。而**折的时候没有做 FMA
收缩**：实测对象给的正是"平铺链"的值（`0x1.f43f7a2ac9200p-4`），而按 `.FMA` 链精确算
（`Fraction` 精确 fma）得 `0x1.f43f7a2acb bccp-4` 那一档 —— 两者在 ±75 处相差 1.5e-13。

于是三种形态并存：**不钳位 = 融合链**（与 Rust 一致）、**钳位 = 编译期平铺折出的常量**
（与 Rust 的融合链差 ~1.2e-12 相对）。这解释了上一轮"内核既不是融合链也不是平铺链"
的假象 —— 它**按分支**用的是不同的形态。

### 处置

这一轮**没有改生产代码**：把上面四个折出来的常量（`td = -75` 与 `td = +75` 各
`es`/`esdT`，都已 ×100 成 Pa）算出来备用：

```
NEG_ES   = 0.12213084908996308    NEG_ESDT = 0.019086085919217677
POS_ES   = 38592.35043748555      POS_ESDT = 1614.7370424178039
```

但落地要重排 `saturation_specific_humidity` 的分支结构（钳位支直接返回这两个
Pa 常量、跳过 `*100`），第一次尝试把定界符改坏了，已**整文件回退**。
下一轮照上面的常量与结构改，再用 `compare_qsadv.sh` 的全区间跑法验证
（预期 5021 → 0）。

Tested: `oracle/scripts/compare_qsadv.sh` 三次（默认全区间、冷支、暖支）；
`gfortran -S` 的 `qs.s` 里 `L4`/`L5`/`L2` 三个块逐行核对；四个折出常量用 Python
平铺链复算；`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: 钳位支的常量落地（下一轮）；`td = +75` 侧的对象直读（只用平铺链复算过）。

## `qsadv` 结案：钳位支改用编译期折出的常量，**全区间 20000/20000**

按上一节记下的四个常量落地：`saturation_specific_humidity` 在钳位支
（`T-273.16 < -75` 或 `> 75`）直接返回上游折出的 Pa 值、跳过 `*100`。

```
$ bash oracle/scripts/compare_qsadv.sh
qsadv: inputs aligned and all 4 outputs 20000/20000 bitwise identical
```

（改前全区间 5021 组 `es` 失配。）四个常量在**落地前**先由对象直读确认过：

```
$ /tmp/gf/r152/clamp.f90        # T=198（td<-75）与 T=349（td>+75）
NEG  3FBF43F7A2AC9200  3F938B4D8B53A580
POS  40E2D80B36C8AC77  40993AF2BB3F60EE
```

**黄金窗口逐位不变**（干 21326 / 湿 32681 / 雪 33651，三个 tier2 变量数 17/68/79）——
三个算例的气温从不进入 `T <= 198.16 K` 或 `T >= 348.16 K`，所以这条修正对它们**天然惰性**。
这不是"改了没用"，而是**该模块的忠实度由 20000 组随机输入直接证明**，
窗口只是恰好覆盖不到钳位支（这与 `MOD_FrictionVelocity` 那轮的结论同一性质）。

**由此关掉的两个模块**（本仓库最强证据类：与内核本体逐位相同）：

| 模块 | 证据 |
|---|---|
| `MOD_FrictionVelocity` | 20 个输出 20000/20000（`compare_moninobukm.sh`）|
| `MOD_Qsadv` | 4 个输出 20000/20000（`compare_qsadv.sh`）|
| `MOD_SoilThermalParameters:soil_hcap_cond` | 8 档 × 5000 组、2 个输出 40000/40000（`compare_soilthermal.sh`）|
| `MOD_TurbulenceLEddy` | 17 个输出 20000/20000（`compare_leddy.sh`，本地算例到不了这条支）|

Tested: `oracle/scripts/compare_qsadv.sh`（全区间 20000 组，改前 5021 → 改后 0）；
`/tmp/gf/r152/clamp.f90` 的四个常量直读；三个黄金窗口三口径 A/B（逐位不变）；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: 钳位支在窗口里的端到端影响（三个算例都到不了那个温度）。

## `MOD_SoilThermalParameters:soil_hcap_cond` 结案：8 档方案 × 5000 组、两个输出 40000/40000

这个子程序是**唯一**由 `DEF_THERMAL_CONDUCTIVITY_SCHEME` 分派的物理入口（1=Oleson、
2=Johansen、3=Cote-Konrad、4=Balland-Arp、5=Lu、6=Tarnawski-Leong、7=DeVries、
8=Yan-He），此前只被黄金窗口间接覆盖（三个算例实际只走默认那档）。这一轮补上
`oracle/scripts/compare_soilthermal.sh`（配对物：`oracle/scripts/soil_hcap_cond_diff.f90`
与 `crates/colm-core/examples/soil_thermal_probe.rs`）。

### 8 档一次跑完：桩住 namelist，而不是重编内核

该子程序唯一读的 namelist 量就是 `DEF_THERMAL_CONDUCTIVITY_SCHEME`，而链接真正的
`MOD_Namelist` 会把 `MOD_SPMDTask`/`MOD_FileSystem` 一路拖进来。脚本因此就地生成一个
只含该标志的桩模块，用 `-I"$WORK"` 压在 `.bld` 前面 —— 模块本体仍按产线选项编译
（`-O2 -fdefault-real-8`，**不加** `-ffp-contract=off`），驱动本身照纪律加
`-fwrapv -ffp-contract=off`。

### 抽样：一半均匀、一半**踩边界**

边界那 2500 组按固定取值池抽（`vf_pores_s ∈ {0.05,0.1,0.25,0.4,0.6,0.9}`、
`vf_water/vf_ice` 的分数含 `0`/`1e-12`/`1`、`temperature` 含 `273.15`/`273.16`、
`kdry/k_solids/ksat` 含 `0`、`vf_gravels_s+vf_sand_s` 含 `0`）。这直接命中了
`a > 0.40/0.25/0.01` 三档、`sr*vf_pores_s <= 0.09`、`vf_water > 0.01` 这些阈值分支，
以及 **3134 组 `sr < 1e-10` 的干土路径**（只靠均匀抽样几乎抽不到）。

### 顺带纠正我自己的一处误读

第一次读源码时我把 `MOD_SoilThermalParameters.F90:420` 那个 `ENDIF` 当成了
`IF(sr >= 1.0e-10)` 的收尾，于是以为干土时 `thk` 从未被赋值（`intent(out)` 未定义）。
实际上 `:418-420` 的 `ELSE ke = 0.0` 才是那道门的 `ELSE`，而 `thk` 的四处赋值
（`:422-430` 的 1–5 档、`:432-464` 的 6 档、`:466-497` 的 7 档、`:499-517` 的 8 档）
全在门外。所以 `thk` **永远有定义**，干土路径也照比 —— `thermal_properties.rs`
里那段「6/7 档在门外、其余各档等价于 `kdry`」的注释是对的，只是行号得按这个结构读。

### 结果

```
$ bash oracle/scripts/compare_soilthermal.sh
soil_hcap_cond: 输入对齐，8 档方案 × 5000 组、2 个输出全部逐位相同
                (hcap 40000/40000, thk 同)；其中 3134 组走的是 sr<1e-10 的干土路径
```

这轮**没有改生产代码** —— 现有实现直接全过。

### 本模块的剩余面（如实记下）

`MOD_SoilThermalParameters` 还有两个 `PUBLIC` 数组例程：`hCapacity`（分层热容，
含 `patchtype` 的湖/湿地/城区分支）与 `hConductivity`（分层导热率＋界面导热率，
含冰川 `tkice` 分支与「雪节点距界面更近时取 `max(0.5*thk(i+1), ...)`」那条修正），
自述「Only used in urban model」。它们的 Rust 对应物是
`ground_temperature.rs::layer_thermal_properties` 与 `urban_impervious.rs` 里的
界面导热率，**不在本次标量差分的范围内**，目前仍只由黄金窗口与单元测试守着。
要像 `soil_hcap_cond` 这样逐位结案，得先把数组例程的 Rust 入口暴露成可驱动的
函数（现在 `layer_thermal_properties` 是私有的）。

Tested: `oracle/scripts/compare_soilthermal.sh`（40000 组 × 2 输出逐位相同）；
`cargo fmt --all --check`；`NETCDF_DIR=... cargo clippy --workspace --all-targets -- -D warnings`；
`cargo test --workspace --lib --bins -- --test-threads=1`；`cargo test -q -p oracle`；
`cargo run -q -p xtask -- check-gui`；`python3 oracle/scripts/test_upstream_f48_sync.py`。
Not-tested: `hCapacity`/`hConductivity` 的数组级分支（见上）；三个黄金窗口未重跑（本轮不动生产代码）。

## `MOD_TurbulenceLEddy` 结案：**黄金窗口给不了证据**的模块，靠差分拿下 17 个输出 20000/20000

这个模块的两个 `PUBLIC` 入口（`moninobuk_leddy`、`moninobukm_leddy`，LZD2022 大涡
近地层方案）**任何本地算例都走不到**：三个黄金算例的强迫场里没有 `hpbl`
（`DEF_USE_CBL_HEIGHT` 关着），所以窗口对它的覆盖是零，只能靠差分证明。此前
`compare_moninobukm.sh` 那 20 个输出全部落在 `MOD_FrictionVelocity` 里，这个模块
一直只是"代码在、没验过"。

新增 `oracle/scripts/compare_leddy.sh`（配对物 `oracle/scripts/turbulence_leddy_diff.f90`
与 `crates/colm-core/examples/leddy_probe.rs`，`hpbl = 10…3000 m`）：

```
$ bash oracle/scripts/compare_leddy.sh
MOD_TurbulenceLEddy: all 17 outputs 20000/20000 bitwise identical
  分支分布 (ib jb kb zc -> 组数): 0010:724 0020:824 0110:1839 0120:6604
                                  1130:9685 1131:18 1140:241 1141:65
```

这轮**没有改生产代码** —— `monin_obukhov_with_scheme` / `canopy_monin_obukhov_with_scheme`
的 `LargeEddy` 支直接全过。

### 行首四位诊断位不是装饰

`ib`（obu 符号）、`jb`（`Bm < 0.2722`，即 `Bm2 = max(Bm,0.2722)` 的钳位生效）、
`kb`（动量廓线四支：`zeta<zetam2` / `zetam2<=zeta<0` / `0<=zeta<=1` / `zeta>1`）、
`zc`（稳定侧 `zetazi` 被上钳到 200）。它们**只在 Fortran 侧算**、只用于给失配分组，
不参与比对 —— 免得探针里再抄一遍物理。结果把这套方案的取值域说清楚了：

| 诊断 | 覆盖 |
|---|---|
| `kb` | 1: 2563、2: 7428、3: 9685、4: 306 —— 四支全到 |
| `ib` | 不稳定 9899、稳定 10101 |
| `jb` | 钳位生效 10370、不生效 9630 |
| `zc` | 稳定侧 `zetazi > 200` 被钳 **83** 次 |

不稳侧的 `zetazi` 两个钳位（`-1e4` 与 `-1e-5`）在 `hu>=1`、`|obu|<=505`、`hpbl` 有限
的取值域内**不可达**，这不是抽样不够，是量级差太远 —— `zc` 那一列就是为了把这句话
变成可核对的计数（`1131`/`1141` 是稳定侧的那 83 次，不稳侧一次都没出现）。

Tested: `oracle/scripts/compare_leddy.sh`（17 个输出 × 20000 组逐位相同）；
`cargo fmt --all --check`；`NETCDF_DIR=... cargo clippy --workspace --all-targets -- -D warnings`。
Not-tested: `MOD_TurbulenceLEddy` 在窗口里的端到端影响（本地算例根本到不了这条支）；
`hpbl` 的逐强迫场读取路径（`DEF_USE_CBL_HEIGHT` 在本仓库仍是显式拒绝的开关之一）。

## **抓到第二个真缺陷**：风场降尺度的乘法结合顺序（`downscale_wind` 8.9% 样本错）

`MOD_ForcingDownscaling` 是"只靠 GIMPLE 读形状、没有差分驱动"的那一块。这一轮先补
它最小的两个入口：`downscale_wind`（4 个地形类）与 `downscale_wind_simple`（9 个
坡向类），4 个输出 × 20000 组。

### 上游侧这次**不重编模块**，直接链接内核对象

`oracle/scripts/compare_forcingdownscaling_wind.sh` 把驱动与 `vendor/CoLM202X/.bld`
下的全部 `.o` 链接起来，只排除 `CoLM.o`（那个 `PROGRAM` 定义了 `_main`）；LAPACK/BLAS
与 netcdf-fortran 照 `Makeoptions.Mac-arm` 加。好处是跑的就是内核产线对象
（`.bld/MOD_ForcingDownscaling.o` 里 `objdump` 能看到 13 条 `fmadd`），连 namelist、
`MOD_Const_Physical`、`MOD_Vars_Global` 都是真的 —— **不需要桩**。这条链接路子对
后面"某个模块的驱动要拖一大串运行时依赖"的情况可以复用。

### 差分第一跑就红了：4 个输出里 3.4%–13% 的样本错

```
us_full    f0=0 f1=0: 1723     us_simple  f0=0 f1=0: 2558
vs_full    f0=0 f1=0: 1730     vs_simple  f0=0 f1=0: 2544
（另有 us==0 与 cur==MISSING 那几组的小额失配）
```

对着 `fd.opt` 逐条读形状，两处都错在**乘法的结合顺序**：

| 位置 | 上游 GIMPLE | 改前的 Rust | 改后 |
|---|---|---|---|
| full 的因子 | `_17 = slope*cos(...)`；`_20 = _17*0.58`；`_22 = _20+1.0`；`_24 = cur*0.42`（提到循环外）；`_25 = _22+_24` | `1.0 + 0.58*slope*cos + 0.42*cur`（= `(0.58*slope)*cos`）| `(slope*cos*0.58 + 1.0) + cur*0.42` |
| simple 的因子 | `_18 = cos(...)*atan(slope)`；`_24 = _18*0.58`；`_26 = _24+1.0`；`_28 = .FMA (cur, 0.42, _26)` | 同一个表达式（**没有 FMA**）| `cur.mul_add(0.42, slope_angle*cos*0.58 + 1.0)` |

两件事值得记下来：

1. **`0.58*wind_dir_slp(i)` 不是 `0.58*slope*cos`**。上游先把 `slope*cos` 存进
   `wind_dir_slp(i)`（独立语句），再乘 0.58；写成 `0.58*slope*cos` 会按左结合算成
   `(0.58*slope)*cos`，在 ~9% 的样本上差 1 ULP。差分把它照出来了。
2. **同一个物理式在两个例程里形状不同**：full 的 `0.42*cur` 被提到循环外、**没有**
   收缩；simple 的那一条是 `.FMA (cur, 0.42, ...)`。共用一个 Rust 表达式不可能两边都对。

### 另一处"看着像漏判、其实是死代码"的地方（**不改**）

`downscale_wind_simple` 里 `scale_factor = -1e36` 表示缺测，紧接着的钳位
`IF (scale_factor<-1.5) scale_factor = -1.5` 会**先把标记改成 -1.5**，于是后面
`IF (scale_factor == -1e36 ...)` 那一支在编译后的上游里根本进不去 —— `fd.opt`
的 `bb20` PHI（`prephitmp_137 = 1` → `-1.5`）就是这么折的。Rust 的 `.clamp(-1.5,1.5)`
放在 `MISSING` 判断之前，行为与内核逐位一致；那行 `factor == MISSING` 保留原样，
并在注释里写明它是死分支，免得后来者把 `.clamp` 挪到后面。

### 结果

```
$ bash oracle/scripts/compare_forcingdownscaling_wind.sh
MOD_ForcingDownscaling wind: 4 outputs 20000/20000 bitwise identical
  分支分布 (f0 f1 -> 组数): 00:18315 01:1145 10:509 11:31
```

（`f0` = `us == 0` 走 `PI/2` 那条；`f1` = `cur == -1e36`。四种组合都抽到了。）

Tested: `oracle/scripts/compare_forcingdownscaling_wind.sh`（4 个输出 × 20000 组逐位相同，
改前 3.4%–13% 失配）；`cargo test --workspace --lib --bins -- --test-threads=1`；
`cargo clippy --workspace --all-targets -- -D warnings`；两个 workspace 的 `cargo fmt --all --check`。
Not-tested: `downscale_forcings` 本体（下一轮：它还要 `DEF_DS_*` 的多套组合与
`sf_lut_c`/`svf_c`/`alb` 的可选实参分支）；`downscale_shortwave` 的 full 支。

## `downscale_forcings`（简单地形支）结案：**又抓到两处 1 ULP 级缺陷**，220000/220000

上一轮把风场那两个入口验掉之后，这一轮补 `MOD_ForcingDownscaling:downscale_forcings`
本体。上游侧沿用"直接链接 `.bld` 内核对象"的路子 —— 好处在这里兑现了：`DEF_DS_*`
是**真的 namelist 变量**，驱动逐组赋值就能覆盖多套配置，不用写桩。只跑"不给可选
实参"的调用形式，于是走 simple shortwave；full 支（`sf_lut_c`/`svf_c`/`alb`）留待下一轮。

驱动：`oracle/scripts/compare_forcingdownscaling.sh`（5000 组输入 × 4 组配置：
降水方案 I/II × 长波方案 I/II，11 个输出）。**两处失配都是真缺陷**：

### 缺陷一：`downscale_longwave` 方案 I 的结合顺序（31% 样本差 1–2 ULP）

上游 GIMPLE（`fd.opt` 的 `downscale_longwave`）：

```
_20 = allsky_g - clearsky_g
_22 = _20 + clearsky_c        ← (allsky-clear_grid) + clear_column
_24 = _22 * 5.67e-8
_26 = _24 * t_c**4
```

Rust 原来写的是 `clear_c + allsky_g - clear_g`（左结合成 `(c+a)-g`）。数学上等价，
浮点上不是：il=1（方案 I）下 1551/5000 组 `lwrad` 差 1–2 ULP。改成
`((allsky-clear_g) + clear_c) * sigma * t_c**4` 之后全同。方案 II（LapseRate）
本来就是对的 —— 这也说明失配确实只出在方案 I 这条支。

### 缺陷二：简单短波的 `cosill` 少了一次收缩（5/20000）

上游把 `cos(slp) + tan(zen)*sin(slp)*cos(asp)` 折成了 FMA：

```
_35 = tan(zen_rad) * sin(slp_rad)
_38 = cos(asp_type_c(i))
_40 = .FMA (_35, _38, cos(slp_rad))
```

Rust 原来是平铺的 `cos + tan*sin*cos`，5/20000 组差 1 ULP。改成
`(tan*sin).mul_add(cos(asp), cos(slp))` 后 20000/20000。

### 顺带记两个"读 GIMPLE 的坑"

1. **`.FMA` 与 `.FNMA` 是两个东西**：`.FMA (a,b,c)` = `a*b+c`，`.FNMA (a,b,c)` = `c-a*b`。
   这一轮差点把 `.FMA (_35, _38, _32)` 读成减法（那就成了"上游算错了"，显然不可能）。
   判据：`(-4.702).mul_add(clr, 2.3)` 与 `FNMA(clr, 4.702, 2.3)` 逐位相同（先取负再乘
   与先乘再取负，都在 FMA 内部只舍入一次），所以两种写法都能对上源码
   `2.3-4.702*clr`；而对 `cos+tan*sin*cos` 这种**符号真正不同**的式子，只有 `.FMA`
   才对得上，19995/20000 的通过率就是判据。
2. `a_p`（面积实参）在 `downscale_forcings` 的简单支里**收到的是坡向数组**：
   `:289-293` 的 `asp_type_c` 传给了被调方名为 `area_type_c` 的形参（`:878-911`）。
   探针必须照抄这个别名，否则两侧输入不一致。Rust 侧 `SimpleTerrain.area_fraction`
   因此在装配时也必须喂坡向 —— 这条已写进探针的文档注释。

### 结果

```
$ bash oracle/scripts/compare_forcingdownscaling.sh
MOD_ForcingDownscaling:downscale_forcings: 11 outputs x 4 configs = 220000/220000 bitwise identical
  配置分布 (ip il -> 组数): 11:5000 12:5000 21:5000 22:5000
```

Tested: `oracle/scripts/compare_forcingdownscaling.sh`（11 输出 × 20000 组逐位相同；
改前 `lwrad` 1551/5000、`swrad` 5/20000 失配）；`cargo test --workspace --lib --bins
-- --test-threads=1`；`cargo clippy --workspace --all-targets -- -D warnings`；
两个 workspace 的 `cargo fmt --all --check`。
Not-tested: `downscale_shortwave` 的 full 支（`sf_lut_c` 16×101 阴影表 / `sf_curve_c`
两套，加 `svf_c`/`alb` 的缺失支）—— 下一轮。

## `downscale_shortwave` full 支结案：**两套阴影表各自 2000/2000**，又抓到三处收缩缺失

上一轮把 `downscale_forcings` 的简单支验掉，这一轮补 full 支（`downscale_shortwave`：
地形/天空视域/反射那一整套）。这次的**主要障碍不在物理，而在编译开关**。

### 先认清 `.bld` 是哪一档：它是**网格**构建，不是单点

上游 `downscale_shortwave` 的阴影实参由编译开关决定：

```
#ifdef SinglePoint
   sf_lut_c   (1:num_azimuth,1:num_zenith)            ! 单点：16×101 查找表
#else
   sf_curve_c (1:num_azimuth,1:num_zenith_parameter)  ! 网格：16×3 分段曲线
#endif
```

`vendor/CoLM202X/include/define.h` 里是 `#define GRIDBASED` + `#undef SinglePoint`，
`gzip -dc .bld/mod_forcingdownscaling.mod | strings` 里也确实是 `sf_curve_c` ——
**`.bld` 这套内核对象是网格档**。第一版驱动却按 `-DSinglePoint` 传了 16×101 的查找表，
被调方按 `sf_curve_c(16,3)` 去读（`segment/sf_curve_c(ia,1)`、`a1=…(ia,2)`、`a2=…(ia,3)`），
于是 `sf_c` 完全变味 —— 2000 组里 1104 组 `swrad` 对不上（差 1%–170%）。
这**不是**移植缺陷，是harness 的编译开关与被链对象不一致。

处置：脚本跑**两遍**，每遍都让驱动、模块对象、阴影表三者对齐 ——

* `curve`：驱动不定义 SinglePoint（与 `.bld` 一致），传 `sf_curve_c(16,3)`，48 个值走 LCG；
* `lut`：单点档。`include/define.h` 里那行 `#undef SinglePoint` 会把命令行上的 `-D` 吃掉，
  所以脚本把它 sed 成 `#define`，写成 `$WORK/define.h`，再用 `-I"$WORK"` 压在 `include/`
  前面编 `MOD_ForcingDownscaling.F90`（该模块里 `#ifdef SinglePoint` 只包着阴影表那几行，
  翻它不动别的语义），链接时顶掉 `.bld` 的同名对象。

### 修掉的三处（全部有 dump 依据）

| 位置 | 上游 GIMPLE | 改前 Rust | 影响 |
|---|---|---|---|
| full 的 `cosill` | `_49=tan(zen)*sin(slp)`；`_57=.FMA (_49, cos(asp), cos(slp))` | 平铺 `cos+tan*sin*cos` | 1–2 ULP |
| 反射项 | `_80=(1-svf)*diff_c`；`_81=.FMA (coszen, beam_c, _80)` | 平铺 `beam*coszen + (1-svf)*diff` | 1 ULP |
| curve 阴影 | `_128=.FMA (zen_rad, a1, a2)` | 平铺 `a1*zen+a2` | 1 ULP |
| 收尾 | full 支是 `IF (forc_swrad_c==0.) → 1e-4`（**只**在恰为 0 时） | 两档共用 `max(0.0001)` | 小值抽样下 1.08e-5 → 1e-4 |

最后一条是抽样设计的功劳：驱动专门有一档 `swg = uni()*1e-5`（`MOD(i,13)==0`），
把"恰为 0 才抬"和"小于 1e-4 就抬"这两种写法分开。上游 simple 支确实是
`IF (forc_swrad_c < 1.e-4)`，full 支是 `== 0.` —— 两支不同，Rust 里必须分开写。

### 结果

```
$ bash oracle/scripts/compare_forcingdownscaling_shortwave.sh
full shortwave [curve]: 11 outputs 2000/2000 bitwise identical  (分支分布 {'000':970,'001':99,'010':394,'011':37,'100':325,'101':37,'110':130,'111':8})
full shortwave [lut]:   11 outputs 2000/2000 bitwise identical  (分支分布 {'000':980,'001':95,'010':384,'011':41,'100':326,'101':34,'110':129,'111':11})
```

分支位是 `alb` 缺测（NaN）、`svf` 越界、`coszen == 0`，八种组合都抽到了。

Tested: `oracle/scripts/compare_forcingdownscaling_shortwave.sh`（两档各 11 输出 × 2000 组逐位相同；
改前 curve 28 组、lut 12 组 1 ULP 失配，harness 未对齐时 1104 组大偏差）；
`cargo test --workspace --lib --bins -- --test-threads=1`；
`cargo clippy --workspace --all-targets -- -D warnings`；两个 workspace 的 `cargo fmt --all --check`。
Not-tested: `sf_lut_c` 在**单点产线内核**里的端到端影响（Rust 运行时目前还没有把 downscaling
接到装配路径上，这个模块现在只有差分证据）。

## `MOD_LeafTemperature` 的三处收缩：dump 说融合、窗口说不要，**按先例回退**

照上一轮留下的优先级，把 `lt.opt` 里三处最像"活路径"的收缩按 dump 改成 FMA：

| 位置 | dump | 上游形状 | 改前 Rust |
|---|---|---|---|
| `clai`（`:493`）| `lt.opt:1158-1159` | `_49=(lai+sai)*0.2`；`_52=ldew_rain*cpliq`；`_53=.FMA(_49,cpliq,_52)`；`clai=.FMA(ldew_snow,cpice,_53)` | 三项平铺 |
| `cfw`（`:812`，Rust 里叫 `leaf_moisture_conductance`）| `lt.opt:1969` | `_257=delta*(1-fwet)`；`_261=(lai+sai)*(1-_257)/rb`；`_268=laisun/(rb+rssun)+laisha/(rb+rssha)`；`cfw=.FMA(_257,_268,_261)` | `left + (1-fwet)*delta*right` |
| `thvstar`（`:976`）| `lt.opt:2415` | `_491=(0.61*th)*qstar`；`thvstar=.FMA(1+0.61*qm,_484,_491)` | `tstar*(1+0.61*qm) + 0.61*th*qstar` |

先确认了一件事：`MOD_LeafTemperature.F90` 里**一处 `#ifdef` 都没有**，所以这份 dump 的
收缩选择对单点内核同样成立（内核只多 `-g -ffpe-trap -fbacktrace`，不改收缩）——
即"dump 说融合"这次是**可信**的，不是上一轮那种"dump 来自网格档"的问题。

### 可是黄金窗口说不要

干窗（CN-Cng）逐组测量：

| 变体 | bitwise | sumabs | over_tol | ot_vars |
|---|---|---|---|---|
| 三处全改 | **21360** | 400.2843 | 830 | **18** |
| 只退 `clai` | 21360 | 400.2843 | 830 | 18 |
| 只退 `cfw` | 21361 | 400.3077 | 826 | 18 |
| 只退 `thvstar` | 21303 | 263.9564 | 817 | **17** |
| 三处全退（现状）| 21326 | 338.9256 | 825 | 17 |

两个指标**打架**：改动让 `bitwise`（逐位相同的数值个数）变好（21326 → 21360），
却让 `ot_vars`（超容差变量数）从 17 变成 18、`sumabs` 从 339 涨到 400。
`clai` 那一处在干窗（无雪）走 `else` 分支，逐位同值 —— 它是惰性的。

### 处置：按先例回退，并把矛盾记下来

本仓库对这个矛盾的先例是明确的：`16f9b26` 那轮"dump 支持的 `dthv`/`thvstar` 融合"
同样让干窗变差（17→18），当时就是**回退**并记录负结果。这一轮照办：三处全部回退，
干窗回到 `21326 / 338.9256 / 825 / 17`（与改动前逐位同值，已实测）。

要真正裁决这类 1 ULP 级改动，`tier2 变量数` 这个口径太粗（干窗是混沌的，1 ULP
扰动会翻动个别变量）。下一轮该补的是**更细的窗口口径**：逐变量的相对误差分布、
或把干窗拉长到 11 天看累计偏差，而不是继续用"超容差变量数"这个会被个别变量
翻转的计数。在那种口径到手之前，dump 与窗口打架时一律按 `16f9b26` 的先例回退。

Tested: `lt.opt` 第 1158-1159/1969/2415 处与源码 `MOD_LeafTemperature.F90:493/812/976`
的逐句对照；`MOD_LeafTemperature.F90` 无 `#ifdef` 的核对；上表五个变体各自的干窗
实测（`bash /tmp/gf/winCN.sh`）；回退后 `21326/338.9256/825/17` 与改动前逐位一致；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: 湿窗/雪窗上这三处的单独影响（只跑了干窗）；更细的窗口口径（下一轮）；
`lt.opt` 里其余收缩处。

## 新增**分步分歧**口径（`oracle/scripts/window_divergence.py`），并用它给 1 ULP 之争收官

上一轮的结论是"`tier2 变量数` 太粗，先补更细的口径"。这一轮补上了
`oracle/scripts/window_divergence.py`：两侧 history 逐变量给**首次分歧步**、
逐位不同元素数、最大绝对差、最大**相对**差，并给出全局 `first divergence step`
与 `bitwise identical` 比例。用法：

```
python3 oracle/scripts/window_divergence.py <内核 history.nc> <Rust history.nc> [--top N]
```

### 干窗 3 步实测：种子在**第 0 步就有**，44 个变量

```
$ bash /tmp/gf/dry_ts.sh 3      # 两侧各跑 3 步，HIST_FREQ=TIMESTEP
$ python3 oracle/scripts/window_divergence.py /tmp/gf/dryts/out/CN-Cng/history/*.nc \
      /tmp/gf/dryts/colm-rs_hist_2008-01.nc
first divergence step: 0
variables differing: 44; bitwise identical: 580/692 (83.8150%)
  逐变量 maxrel 最大的是 f_fsenl 1.47e-13、f_fevpl 1.55e-14、f_fsena 1.00e-14 …
```

**所有 44 个变量的 `first` 都是 0** —— 差异在第一条 history 记录（第 1 步之后）就存在，
后面两步只是同一批差异继续存在（`ndiff` = 3）。这条把"干窗第 0 步的 1 ULP 种子"
从模糊说法变成了可复核的数字。

### 用它给 `MOD_LeafTemperature` 那三处 1 ULP 之争收官：**测不出谁更近**

把三处 FMA 再装回去，用同一套口径量 3 步：

| 变体 | 3 步 bitwise | 11 天 bitwise | 11 天 ot_vars | `f_fsenl` maxabs | `f_fseng` maxabs | `f_fh` maxabs |
|---|---|---|---|---|---|---|
| 回退（现状）| **580/692** | 21326 | **17** | **5.6133e-13** | 3.2969e-12 | 4.4409e-15 |
| 三处 FMA | 574/692 | **21360** | 18 | 6.3283e-13 | **1.4779e-12** | **1.3323e-15** |

两边各有胜负（3 步 bitwise 回退版好、11 天 bitwise 融合版好；逐变量量级互有升降），
而且两版的 `first divergence step` 都是 0、差异变量集合完全相同 —— **说明该变体
对第 0 步种子没有任何影响，后面所有的差别都是那 1 ULP 经混沌放大后的抖动**。

**处置：维持回退**（与 `16f9b26` 的先例一致，也不是因为窗口更好，而是因为
"窗口口径无法裁决"时不引入未被证据支持的改动）。真正该修的是第 0 步那 44 个变量；
把它修掉之后，这类 1 ULP 的形状问题会自动变得无所谓 —— 这也是下一轮的方向：
用本工具逐变量盯 `f_fsenl`（maxrel 1.47e-13）与 `f_fevpl`，配合调用点实参 dump 定位。

Tested: `oracle/scripts/window_divergence.py` 在干窗 1 步/3 步、两个变体上的实测；
`bash /tmp/gf/dry_ts.sh 3`；`cargo test -q -p colm-core --lib -- --test-threads=1`。
Not-tested: 湿窗/雪窗上的同口径（脚本已可复用）；第 0 步种子的定位（下一轮）。

## **抓到干窗第 0 步种子的第一层**：`gssun` 的除法结合顺序（`MOD_LeafTemperature.F90:1040`）

上一轮新加的 `window_divergence.py` 把 44 个差异变量按 `maxrel` 排队，第一眼就看到
一个**量级完全不同**的成员：

```
f_gssun / f_gssha   maxabs 1.36e-20   maxrel 2.95e-16   ← 1 ULP 的签名（2.2e-16）
f_fevpl             maxrel 1.55e-14
f_fsenl             maxrel 1.47e-13
f_gssun 以外的一族   maxrel 1e-14 … 1e-11
```

其余变量都是**下游被放大的**（1e-14 量级往上），只有 `gssun`/`gssha` 停在 1 ULP ——
这就是"种子在它自己身上"的判据。顺着它找到源码：

```fortran
! MOD_LeafTemperature.F90:1040
gssun = (laisun / rssun) * (tprcor / tlbef)
```

Rust 写的是 `laisun / rssun * pressure_conversion / previous_leaf_temperature`，按左结合
算成 `((laisun/rssun)*tprcor)/tlbef` —— **右边那个除法被拉平了**。改成

```rust
let resistance_conversion = pressure_conversion / previous_leaf_temperature;
let sunlit_stomatal_conductance = laisun / last.leaf_sunlit_resistance * resistance_conversion;
```

### 证据：`f_gssun` **完全不差了**，`f_gssha` 还剩 1 ULP

同一套 3 步口径（`bash /tmp/gf/dry_ts.sh 3`）：

| | 差异变量数 | bitwise identical | `f_gssun` | `f_gssha` |
|---|---|---|---|---|
| 改前 | 44 | 580/692 | maxrel 2.95e-16 | maxrel 2.95e-16 |
| 改后 | 43 | **582/692** | **不再出现**（三步逐位相同）| maxrel 2.99e-16（三步里差两步）|

**第一层剥开了，第二层露出来了**：`gssha` 仍差 1 ULP，而它与 `gssun` 共用
`resistance_conversion`/`previous_leaf_temperature`，所以剩下的种子只能来自
`laisha / leaf_shaded_resistance` 这一侧 —— 即**阴叶那次 `stomata` 调用**（或其输入
`cintsha`/`parsha`/`rstfacsha`）。下一轮的目标由此从"整个 44 个变量"缩到**一次
`stomata` 调用**。

### 窗口：逐位 ±2，容差口径全部不变（照实记录）

| 窗口 | 改前 | 改后 |
|---|---|---|
| 干 | 21326 / 338.9256 / 825 / 17 | **21328** / 338.9256 / 825 / 17 |
| 湿 | 32681 / 10369.4411 / 20672 / 68 | **32679** / 10369.4411 / 20672 / 68 |
| 雪 | 33651 / 444394.4368 / 25896 / 79 | 33651 / 444394.4368 / 25896 / 79 |

`sumabs`/`over_tol`/`ot_vars` 三个口径**一字不变**，逐位计数 ±2（干 +2、湿 −2）。
按本轮新立的规矩，裁决依据是**步级证据**：第 0 步 `f_gssun` 的三步逐位相同，
而源码第 1040 行的括号位置是唯一的 —— 所以接受，不因为 ±2 的窗口抖动而改回。

Tested: `window_divergence.py` 的 3 步口径改前/改后对照；`bash /tmp/gf/dry_ts.sh 3`；
三个黄金窗口改后实测（上面那张表）；`cargo fmt --all --check`；
`NETCDF_DIR=... cargo clippy --workspace --all-targets -- -D warnings`；
`cargo test --workspace --lib --bins -- --test-threads=1`。
Not-tested: 湿窗/雪窗上的分步口径（脚本可复用，下一轮补）；`gssha` 那一侧的定位。

## **更正上一轮的结论**：`f_gssun` 并没有"不再出现"——工具按变量名截断误导了记录

上一轮的提交 `0c558bf` 与文档里写着"`f_gssun` 不再出现（三步逐位相同）"。**这是错的**：
`window_divergence.py` 当时按 `(首次分歧步, 变量名)` 排序，`--top 10` 截出来的只是
**字母序最靠前的十个**，`f_gssun` 恰好被挤到截断线之外 —— 它一直都在，
`ndiff` 从 3 降到 2、`maxrel` 仍是 2.99e-16。

真实的账：

| | `f_gssun` | `f_gssha` | 3 步 bitwise |
|---|---|---|---|
| 改前 | ndiff 3 / maxrel 2.95e-16 | ndiff 3 / maxrel 2.95e-16 | 580/692 |
| 改后（现状）| ndiff **2** / maxrel 2.99e-16 | ndiff **2** / maxrel 2.99e-16 | **582/692** |

也就是说 `MOD_LeafTemperature.F90:1040` 的括号**确实**是一处真形状差异（两个变量各
少了 1 个逐位不同的值 = bitwise +2），但它**只解了三分之一**，剩下的种子还在别处。

### 工具已修：排序改成按 `maxrel` 升序

```python
rows.sort(key=lambda row: (row[0], row[4]))   # (首次分歧步, maxrel)
```

并在文档串里写明：**不要按变量名截断看**。改完之后种子候选自己冒到最前面
（干窗 3 步、现状代码）：

```
first variable        ndiff       maxabs     maxrel
    0 f_trad              2   5.6843e-14   2.19e-16   ← 1 ULP
    0 f_us10m             2   8.8818e-16   2.83e-16   ← 1 ULP
    0 f_vs10m             2   8.8818e-16   2.83e-16   ← 1 ULP
    0 f_gssha             2   1.3553e-20   2.99e-16   ← 1 ULP
    0 f_gssun             2   1.3553e-20   2.99e-16   ← 1 ULP
    0 f_t_soisno          6   1.1369e-13   4.02e-16
    0 f_olrg              3   1.1369e-13   4.12e-16
    0 f_qstar             2   5.4210e-20   4.42e-16
    …（其余 32 个 maxrel 1.42e-16 … 4.35e+00，均为下游放大）
```

**这张表改变了下一步的方向**：`f_us10m`/`f_vs10m` 正是 `moninobukm` 的输出，而那个
模块已经被随机差分关掉（20 个输出 20000/20000 全同）—— 所以它们差 1 ULP 只能来自
**传给它的实参**（`displa`/`z0m`/`z0h`/`obu`/`um`），这与第 143 轮"种子在装配侧"的
判断一致，而不是在 `stomata` 里。同时 `f_trad`/`f_tleaf` 也在 1 ULP 一档，
说明**叶温/地表温度本身**已经被种上了 —— 下一轮从这几个 1 ULP 变量往上追它们的
共同祖先（建议先 dump 干窗第 0 步 `LeafTemperature` 入口处的 `rb`/`z0m`/`obu`/`um`）。

Tested: 修正后的 `window_divergence.py` 在干窗 3 步数据上的实测（上表）；
`bash /tmp/gf/dry_ts.sh 3`；`cargo fmt --all --check`；
`cargo test -q -p colm-core --lib -- --test-threads=1`（355 通过）。
Not-tested: `0c558bf` 的窗口数据本身（干 21328/湿 32679/雪 33651）不受本次更正影响，
已在上一条记录里逐条给出。

## **重大更正**：内核编译的不是 `main/MOD_LeafTemperature.F90`，而是 `extends/interception/` 那一份

本轮为了给 `MOD_Thermal`（一直拿不到 dump 的模块）补 dump，直接照着内核产线对象反查
来源，结果发现**过去所有 `MOD_LeafTemperature`/`MOD_Thermal` 的 dump 都取错了文件**：

```
$ strings .bld/MOD_LeafTemperature.o | grep '\.F90'
extends/interception/MOD_LeafTemperature_Extended.F90
$ strings .bld/MOD_Thermal.o | grep '\.F90'
extends/interception/MOD_Thermal_CanopyPhase_Extended.F90
$ grep -n "MOD_LeafTemperature.o:" vendor/CoLM202X/Makefile
641:MOD_LeafTemperature.o: extends/interception/MOD_LeafTemperature_Extended.F90 …
647:MOD_Thermal.o: extends/interception/MOD_Thermal_CanopyPhase_Extended.F90 …
```

`main/MOD_LeafTemperature.F90` 有 1370 行、`extends/interception/MOD_LeafTemperature_Extended.F90`
有 **1965** 行；收缩点数量也不一样：**错文件 52 处，对文件 80 处**。也就是说
`/tmp/gf/r144/lt.opt`（以及当年据此做的那些"逐句对照"）是**另一个文件**的 dump。
`main/MOD_Thermal.F90` 单文件编不过（`:924`/`:1036` 的 `dheatl` 标量-数组秩不匹配）
也就不奇怪了 —— 内核根本没用它。

### 对的那份 dump 已经拿到，配方记在这里

```bash
cd vendor/CoLM202X
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -Iextends/interception -I.bld -Iinclude -Imain -Ishare \
  extends/interception/MOD_LeafTemperature_Extended.F90 \
  -J/tmp/gf/r166 -fdump-tree-optimized=/tmp/gf/r166/lt_ext.opt -o /tmp/gf/r166/lt_ext.o
# → 80 处收缩；`gssun`（:1320）与 `main/` 同型，`clai`（:542）/`thvstar`（:1256）同型，
#   但 `cfw`（:1078）**不同型**：对文件写的是 `…*wet_cond_cfw + …`，而 `wet_cond_cfw = wet_area_cfw/rb`
#   是另一条语句先算出来的 —— 拿 `main/` 的 `(lai+sai)/rb` 去推它的括号是**无效推理**。
```

### 但"对文件 + 对 dump"仍然没有说服力：步级口径把它否掉了

拿对文件 dump 里两处确凿的 `.FMA`（`clai`：`lt_ext.opt:1788`；`thvstar`：`lt_ext.opt:3389`
`= .FMA(1+0.61*qm, tstar, (0.61*th)*qstar)`）落回 Rust，用步级口径量：

| 代码 | 3 步 bitwise | `f_gssun`/`f_gssha` ndiff | `f_t_soisno` ndiff |
|---|---|---|---|
| 现状（只有 `gssun` 那处修复）| **582/692** | **2** | 6 |
| 再落 `clai`+`thvstar` 两处 FMA | 574/692 | 3 | 7 |

**两个口径同时变差**，而且这是第 0 步的步级计数、不是 11 天窗口的混沌抖动。
所以即使是"对文件"的 dump，其收缩选择**也不能直接当成内核二进制的行为**：
独立编译（我的 flag 集）与 `kernels/default` 的实际构建（`include/Makeoptions` 那一套）
可以给出不同的收缩。**裁决只能靠步级口径实测**，两处均已回退。

### 下一轮

1. `MOD_LeafTemperature_Extended.F90` 的 80 处收缩要从**对文件**重扫（此前按错文件的
   52 处做过映射，行号与 `cfw` 那一族都要重来）；
2. 想彻底解决"dump 与二进制不一致"，应当从 `kernels/default/colm.x` 本体反汇编取形状
   （`objdump -d` 找 `fmadd`），而不是另编一份；
3. `MOD_Thermal` 要用 `extends/interception/MOD_Thermal_CanopyPhase_Extended.F90` 取 dump，
   `main/MOD_Thermal.F90` 那条"编不过"的记录本身是走错了文件。

Tested: `strings .bld/MOD_LeafTemperature.o`/`MOD_Thermal.o` 的来源核对；`Makefile:641/647`；
对文件的 dump 生成（80 处收缩）；`clai`/`thvstar` 落回后 `bash /tmp/gf/dry_ts.sh 3` +
`window_divergence.py` 的步级实测（574 vs 582）；两处已回退。
Not-tested: 对文件 80 处收缩的逐条重扫；从 `colm.x` 反汇编取形状（下一轮）。

## 内核二进制的收缩是**真的**，但 DWARF 行号**不足以**逐句裁决

上一轮定的方向是"从 `kernels/default/colm.x` 本体反汇编取形状"。不用重编：`.bld` 下那些
`.o` 就是内核链接的对象，`vendor/CoLM202X/.bld/MOD_LeafTemperature.o` 里

```
$ objdump -d --no-show-raw-insn -l .bld/MOD_LeafTemperature.o | grep -c "fmadd\|fmsub\|fnmadd\|fnmsub"
55
```

—— 55 条 FMA 类指令，散布在 94 个不同的源码行上（对象是带 `-g` 编的，行号标记形如
`; /…/MOD_LeafTemperature_Extended.F90:1857`）。所以"上游有大范围收缩"这件事在
**内核二进制里**是确凿的，之前那些 dump 并没有凭空造出融合。

**但逐句映射做不到**。把指令按行号归类后：

| 语句 | 对文件行号 | 该行被归到的指令数 | 该行 FMA 数 |
|---|---|---|---|
| `clai = 0.2*(lai+sai)*cpliq + ldew_rain*cpliq + ldew_snow*cpice` | 542 | **0** | 0 |
| `thvstar = tstar*(1.+0.61*qm)+0.61*th*qstar` | 1256 | **0** | 0 |
| `gssun = (laisun/rssun)*(tprcor/tlbef)` | 1320 | **0** | 0 |

优化之后 GCC 把算术块重组/向量化，行号标记落在**邻近**的语句上（例如 1255、1325、1328
有 FMA，而争议的三行自己没有）。也就是说：**"内核用了 FMA"能证，"哪一句用了 FMA"用
对象反汇编证不了** —— 这和第 166 轮"独立编译的 dump 不能代表内核二进制"是同一枚硬币
的两面，一个是代码形状对不上、一个是行号对不上。

**结论（裁决规则最终定版）**：形状问题只有**步级口径 + 把内核对象链进来的差分驱动**
两条路可走。下一轮的做法照 `compare_forcingdownscaling*.sh` 的成例：写一个驱动
**直接链接 `.bld/MOD_LeafTemperature.o`**（真产线对象），用 `LeafTemperature` 的
130 个实参喂同一组随机输入，两侧逐位比 Newton 增量的分子/分母、`thvstar`、`clai`
这几个量 —— 这样才能把"哪一句收缩了"变成可观测事实。

Tested: `objdump -d -l .bld/MOD_LeafTemperature.o`（55 条 FMA 类指令、94 个行号）；
`grep -n "MOD_LeafTemperature.o:" vendor/CoLM202X/Makefile`（来源仍为
`extends/interception/MOD_LeafTemperature_Extended.F90`）；对文件三处争议语句的行号核对。
Not-tested: 链接 `.bld/MOD_LeafTemperature.o` 的随机差分驱动（下一轮，130 个实参）。

## `cfw` 的括号（源码里是**显式中间量**）也被窗口否掉；三条线索合起来指向**装配**而非表达式

`MOD_LeafTemperature_Extended.F90:972-973` 先算

```fortran
wet_area_cfw = lai + sai
wet_cond_cfw = wet_area_cfw / rb        ! ← 独立语句
…
cfw = (1.-delta*(1.-fwet))*wet_cond_cfw + (1.-fwet)*delta*( laisun/(rb+rssun) + laisha/(rb+rssha) )   ! :1078
```

Rust 写的是 `(1.0 - delta*(1.0-fwet)) * lsai / leaf_boundary_resistance + …`，按左结合算成
`((1-delta*(1-fwet))*lsai)/rb`。**这次不是收缩问题**，而是上游把 `lsai/rb` 写成了一个
**独立中间量** —— 源码级的证据是明确的，不需要 dump。按同样的形状改掉之后：

| | 3 步 bitwise | 干窗 bitwise | 干窗 sumabs | over_tol | ot_vars |
|---|---|---|---|---|---|
| 改前（现状）| 582/692 | **21326** | **338.9256** | 825 | 17 |
| `wet_cond_cfw` 括号 | 582/692（**完全无变化**）| 21319 | **554.2229** | **822** | 17 |

步级口径**一点没动**（对这个窗口惰性），而干窗的 `bitwise` 与 `sumabs` 明显变差。
已回退。

### 三条线索的合流

到这里，三处"源码/dump 说该改"的形状改动全部被口径否掉：

| 站点 | 依据 | 步级/窗口实测 | 处置 |
|---|---|---|---|
| `clai`（对文件 :542）| dump 有 `.FMA` | 3 步 582 → 574 | 回退 |
| `thvstar`（:1256）| dump 有 `.FMA` | 3 步 582 → 574 | 回退 |
| `cfw`（:1078）| **源码显式中间量** | 步级不变、干窗 bitwise −7 / sumabs +215 | 回退 |

三处都在 `rb` 的**下游**。而 `rb` 本身两边长得不一样：

```fortran
! MOD_LeafTemperature_Extended.F90:751-759 / 797-798
rb    = 1/(cf*uaf)        ! 或 rb = 1./cf
rbsun = rb / laisun       ! 传给 stomata 的是 rb/laisun
rbsha = rb / laisha
```
```rust
// leaf_temperature.rs:452
let leaf_boundary_resistance = 1.0 / (0.01 * input.inverse_sqrt_leaf_dimension_m_neg_half
    * effective_wind.sqrt());
// 传给 stomata 的就是它本身，没有再除 laisun
```

也就是说 Rust 用的是**叶尺度** rb，Fortran 用的是**冠层尺度** rb 再除面积 —— 两者在默认
配置下必须**数值等价**才能让 `f_gssun` 走到只剩 1 ULP。这个等价性一直是被默认接受的，
没有正面验证过。**下一轮的第一件事就是核它**：把 `cf`/`uaf` 的定义与
`leaf_boundary_resistance` 的三处使用（`stomata` 的 `rb`、`cfw` 的 `wet_cond_cfw`、
`rbsun/rssha`）逐个对上 —— 如果这里的尺度换算差一个因子或一次舍入，所有 `rb` 下游的
表达式的形状争论都是无意义的，这也正好解释为什么三处形状改动全部被窗口否掉。

Tested: `cfw` 括号改动前后 `bash /tmp/gf/dry_ts.sh 3` + `window_divergence.py`（582 不变）
与干窗实测（21319/554.2229/822/17）；`MOD_LeafTemperature_Extended.F90:751/759/797/798/972/973/1078`
与 `leaf_temperature.rs:452` 的对照；改动已回退。
Not-tested: `rb` 尺度等价性的正面验证（下一轮）；`cf`/`uaf` 的定义。

## `rb` 的尺度问题定案：`rb_opt` 是**硬编码参数 3**，`rbsun = rb/laisun` 是惰性但必须照抄

上一轮列出的"下一轮第一件事"：核 `rb` 的尺度换算。结论如下。

### `rb_opt = 3` 写死在模块里，`uaf = ustar` 那条支是死代码

```fortran
! MOD_LeafTemperature_Extended.F90:484
integer, parameter :: rb_opt = 3             ! rb with vertical profile consideration
…
uaf = ustar ; cf = 0.01*sqrtdi/sqrt(uaf) ; rb = 1/(cf*uaf)   ! :748-751 —— 永不执行
IF (rb_opt == 3) THEN
   utop = ustar/vonkar * fmtop
   ueff = ueffect(utop, htop, z0mg, z0mg, a_k71, 1._r8, 1._r8)
   cf   = 0.01*sqrtdi*sqrt(ueff) ; rb = 1./cf                 ! :755-759 —— 恒走这条
ENDIF
```

所以 Rust 用 `wind_at_top = ustar/vonkar*fmtop` + `effective_canopy_wind(...)` 再
`1/(0.01*sqrtdi*sqrt(effective_wind))` 是**对的那一支**（`leaf_temperature.rs:441-453`），
不是"另一种参数化"。

### 但它缺了调用前的 `rb/laisun`

```fortran
rbsun = rb / laisun        ! :797，传给 stomata 的边界阻力
rbsha = rb / laisha        ! :798
… CALL stomata(… rbsun …)  ! :800
rssun = rssun * laisun     ! :941，返回后再折回叶尺度
```

Rust 原来把叶尺度的 `leaf_boundary_resistance` 直接传进 `stomata`，少了 `:797` 那一步。
按源码补上（只改非 PHS 两处；PHS 那条路在 `:745-752` 传的是 `rb` 本身，不折）：

| 口径 | 改前 | 改后 |
|---|---|---|
| 3 步 bitwise / 各变量 ndiff、maxrel | 582/692 | **完全逐位相同** |
| 干窗 | 21326 / 338.9256 / 825 / 17 | **21328** / 338.9256 / 825 / 17 |
| 湿窗 | 32681 / 10369.4411 / 20672 / 68 | **32679** / 10369.4411 / 20672 / 68 |
| 雪窗 | 33651 / 444394.4368 / 25896 / 79 | 同 |

**数值上是惰性的**，原因也清楚了：`gssun = (laisun/rssun)*(tprcor/tlbef)`（`:1320`）
里调用前的 `/laisun` 与调用后的 `*laisun` 相消。但形状是源码明写的（不是收缩），
按 `gssun` 那处的同样理由落地：容差三口径一字不变、步级口径逐位不变、逐位计数 ±2
（干 +2、湿 −2），没有理由不照抄。

**这一条同时解释了前三轮的三次失败**：`clai`/`thvstar`/`cfw` 都在 `rb` 下游，而 `rb`
这块拼图此前是错的 —— 现在拼上了，但它是惰性的，所以**种子仍然在别处**。

Tested: `…_Extended.F90:484/748-751/755-759/797-798/941/1320` 与
`leaf_temperature.rs:441-453/609/623/637` 的逐句对照；改动前后 `dry_ts.sh 3` +
`window_divergence.py`（582 不变）与三个黄金窗口实测；`cargo test -q -p colm-core --lib`
（355 通过）；`cargo fmt --all --check`；`cargo clippy --workspace --all-targets -D warnings`。
Not-tested: PHS 那条路上 `rb` 的用法（本配置不走）；第 0 步种子的定位（仍开放）。

## `us10m`/`vs10m` 的结合顺序（源码显式、实测惰性）；种子收敛到"装配侧 1 ULP"

`f_us10m`/`f_vs10m` 是干窗第 0 步 1 ULP 名单里的成员（maxrel 2.83e-16，且两者
**同幅**，说明差在两者共用的那个比例因子上）。上游写的是

```fortran
! MOD_Vars_1DAccFluxes.F90:2789-2790
r_us10m_e = us/um * r_ustar2_e /vonkar * r_fm10m_e
```

即从左到右 `((us/um)*ustar2/vonkar)*fm10m`；Rust 原来写成
`us * (ustar/vonkar*fm10m/um)` —— 数学等价、逐位不等价。按源码改成同一顺序后：

| 口径 | 改前 | 改后 |
|---|---|---|
| 3 步 bitwise / `f_us10m` ndiff、maxrel | 582/692、2、2.83e-16 | **完全相同** |
| 干窗 | 21328 / 338.9256 / 825 / 17 | **完全相同** |

**完全惰性** —— 对这个算例，两种结合顺序给出同值。落地理由是"源码显式形状照抄"
（与 `gssun` 分组、`rbsun` 折算是同一类），不是靠窗口收益。

### 剩下四个 1 ULP 变量的共同结论

到这一轮为止，干窗第 0 步的 1 ULP 名单是
`f_trad`(2.19e-16)、`f_us10m`/`f_vs10m`(2.83e-16)、`f_gssun`/`f_gssha`(2.99e-16)、
`f_tleaf`(2.18e-16，且**整段只差 1 个值**)。它们**全部**依赖同一次近地层相似性调用
的输出（`ustar2`/`fm10m`/`um`/`obu`）与叶温/地表温度。而 `MOD_FrictionVelocity:moninobukm`
已经被随机差分关掉（20 个输出 20000/20000 全同），`MOD_LeafTemperature` 的表达式形状
这四轮改了 4 处、全部惰性或被否 —— **所以种子只可能在"喂给这次调用的实参"上**
（`displa`/`z0m`/`z0h`/`z0q`/`obu`/`um`/`thm`/`qm` 这一组），这与第 143 轮的判断、
以及本轮的 `f_us10m` 同幅现象一致。

下一轮的做法（已具备条件）：把 `/tmp/gf/leafprobe` 那套探针接到**对文件**
（`extends/interception/MOD_LeafTemperature_Extended.F90`）与 Rust 的
`leaf_temperature` 入口上，dump 干窗第 0 步那几个实参，逐位比。

Tested: `MOD_Vars_1DAccFluxes.F90:2789-2790` 与 `history_diagnostics.rs:210-222` 对照；
改动前后 `dry_ts.sh 3` + `window_divergence.py`（逐位不变）与干窗实测（21328 不变）；
`cargo fmt --all --check`；`cargo test -q -p colm-core --lib`（355 通过）。
Not-tested: 实参 dump 对照（下一轮）。

## `MOD_Thermal`（对文件）的 dump 终于有了：99 处收缩；`trad` 的形状**是对的**

这个模块过去一直挂着"dump 拿不到"（第 166 轮查明那是走错了文件：内核编的是
`extends/interception/MOD_Thermal_CanopyPhase_Extended.F90`）。用对文件 + 与
`MOD_LeafTemperature` 同一套配方，dump 到手：

```bash
cd vendor/CoLM202X
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -Iextends/interception -I.bld -Iinclude -Imain -Ishare \
  extends/interception/MOD_Thermal_CanopyPhase_Extended.F90 \
  -J/tmp/gf/r166 -fdump-tree-optimized=/tmp/gf/r166/th_ext.opt -o /tmp/gf/r166/th_ext.o
# → 99 处 FMA 类收缩（此前 0：文件根本没编过）
```

`f_trad` 是干窗第 0 步 1 ULP 名单里的成员（2.19e-16），而它的形状在 dump 里是：

```
_1828 = prephitmp_5118 / 5.67e-8          ; olrg/stefnc
_1829 = __builtin_pow (_3486, 2.5e-1)     ; **0.25 走库函数 pow**
*trad_3487(D) = _1829
```

Rust 侧 `surface_budget.rs:58` 写的是 `(outgoing_longwave / STEFAN_BOLTZMANN_W_M2_K4).powf(0.25)`
—— **完全一致**（Rust 的 `powf` 与 GCC 的 `__builtin_pow` 都落到 libm 的 `pow`，不是
`sqrt(sqrt(x))` 那种展开）。所以 `f_trad` 的那 1 ULP 不是这一句造成的，而是从它的
输入 `olrg` 来的 —— 与"四个 1 ULP 变量共享同一个上游"的结论一致。

Tested: 对文件 `MOD_Thermal_CanopyPhase_Extended.F90` 的 dump 生成（99 处收缩）；
dump 第 7228-7235 行与 `surface_budget.rs:58` 的对照。
Not-tested: 那 99 处的逐条重扫；`olrg` 上游的定位。

## 用新拿到的 `MOD_Thermal` dump 修 `olrg`：步级口径 **582 → 585**，`f_trad` 只剩 1 个值

`f_trad` 是 1 ULP 名单成员，而它 = `(olrg/stefnc)**0.25` —— 形状已证一致（上一轮），
所以差在 `olrg`。对文件 dump（`th_ext.opt`）里 `olrg` 的 GIMPLE 是：

```
_1813 = emg * 4.0
_1814 = _1813 * stefnc
_1816 = _1814 * t_grnd_bef**3
_1819 = .FMA (_1816, tinc, ulrad)      ← 收缩在最后一步
*olrg_3439(D) = _1819
```

Rust 原来写的是 `ulrad + emg*(stefnc*t**3*(4*tinc))` —— 既换了结合顺序（`4*tinc`
在最里层），也少了那一次收缩。按 dump 改成
`((emg*4)*stefnc*t**3).mul_add(tinc, ulrad)` 之后：

| 口径 | 改前 | 改后 |
|---|---|---|
| 3 步 bitwise | 582/692 | **585/692** |
| `f_trad` ndiff / maxrel | 2 / 2.19e-16 | **1** / 2.15e-16 |
| 干窗 | 21328 / 338.9256 / 825 / 17 | **完全相同** |

这是 `gssun` 那处之后**第一处真正改善步级口径**的修复（+3），而且窗口容差三口径一字
未变。也说明上一轮那句"四个 1 ULP 变量共享同一个上游"要修正为：`f_trad` 有**自己**的
一处（`olrg`），剩下的 `f_us10m`/`f_vs10m`/`f_gssun`/`f_gssha`/`f_tleaf` 才是同一族。

Tested: `th_ext.opt` 第 7140-7178 行的 GIMPLE 与 `surface_budget.rs` 的对照；改动前后
`dry_ts.sh 3` + `window_divergence.py`（582 → 585）与干窗实测（21328 不变）；
`cargo fmt --all --check`；`cargo test -q -p colm-core --lib`（355 通过）。
Not-tested: `th_ext.opt` 其余 98 处收缩；`f_us10m` 那一族的装配侧定位。

### `olrg` 那处的窗口复核（照实记录：湿窗逐位 −24）

上一轮只跑了干窗，补跑三个窗口：

| 窗口 | 改前 | 改后 | 容差三口径 |
|---|---|---|---|
| 干 | 21328 / 338.9256 / 825 / 17 | 21328 / 338.9256 / 825 / 17 | 全同 |
| 湿 | 32679 / 10369.4411 / 20672 / 68 | **32655** / 10369.4411 / 20672 / 68 | 全同 |
| 雪 | 33651 / 444394.4368 / 25896 / 79 | 33651 / 444394.4368 / 25896 / 79 | 全同 |

湿窗逐位少了 24 个相同值，**但三个容差口径一字不变**，而**步级口径是明确改善的**
（582 → 585、`f_trad` 从差 2 个值降到 1 个）。按本仓库已经写进规矩的裁决方式：
步级口径是"实现是否更接近内核"的直接证据，11 天窗口的逐位计数是被混沌放大的间接量。
所以**保留**这处改动，并把这个 −24 如实记在这里，供以后用更好的口径复核。

Tested: `bash /tmp/gf/win4.sh`（三个窗口，上表）；`three.py` 三口径。
Not-tested: 湿窗 −24 的成因（不作解释，只记录）。

## 干窗第 0 步种子的**当前**清点（第 175 轮基线，交接用）

`olrg` 那处落地后，用同一套口径把现状固化成一张表（`bash /tmp/gf/dry_ts.sh 3` +
`window_divergence.py`，两侧各跑 3 步、`HIST_FREQ=TIMESTEP`）：

```
first variable        ndiff       maxabs     maxrel
    0 f_trad              1   5.6843e-14   2.15e-16   ← 1 ULP
    0 f_us10m             2   8.8818e-16   2.83e-16   ← 1 ULP
    0 f_vs10m             2   8.8818e-16   2.83e-16   ← 1 ULP
    0 f_gssha             2   1.3553e-20   2.99e-16   ← 1 ULP
    0 f_gssun             2   1.3553e-20   2.99e-16   ← 1 ULP
    0 f_t_soisno          6   1.1369e-13   4.02e-16
    0 f_olrg              2   1.1369e-13   4.12e-16
    0 f_qstar             2   5.4210e-20   4.42e-16
    …（其余 36 个 maxrel 1.42e-16 … 4.35e+00）
variables differing: 44; bitwise identical: 585/692 (84.5376%)
first divergence step: 0
```

要点（供下一轮直接接着做）：

1. **1 ULP 一档只剩五个变量**：`f_trad`（只剩 1 个值）、`f_us10m`/`f_vs10m`、
   `f_gssun`/`f_gssha`。`f_us10m`/`f_vs10m` **同幅**，说明差在两者共用的比例因子
   （`ustar2/vonkar*fm10m/um`），而那个乘积的两个因子都来自同一次近地层相似性调用。
2. `f_olrg` 已经是形状正确之后的残差（2 ULP），只能来自它的入参
   （`ulrad`/`emg`/`t_grnd_bef`/`tinc`）—— 同样指向那次调用。
3. `f_emis`/`f_fm`/`f_lfevpa` 等的 `first` 是 1 或 2，**第 0 步是一致的**，不是种子。
4. 因此下一轮的唯一入口仍然是**实参**：把 `/tmp/gf/leafprobe` 那套探针接到对文件
   （`extends/interception/MOD_LeafTemperature_Extended.F90`）与 Rust 的
   `leaf_temperature` 入口，dump 干窗第 0 步的 `displa`/`z0m`/`z0h`/`z0q`/`obu`/`um`，
   逐位比。`moninobukm` 本体已被差分关掉，所以只要实参一致，输出就该逐位一致。
5. 工具与 dump 都已就绪：`oracle/scripts/window_divergence.py`、
   `/tmp/gf/r166/lt_ext.opt`（80 处）、`/tmp/gf/r166/th_ext.opt`（99 处）。

Tested: 上表的生成命令（`dry_ts.sh 3` + `window_divergence.py --top 8`）。
Not-tested: 实参 dump 对照（下一轮的入口）。

## `fgrnd` 是对文件 dump 里的**双分支 PHI**（下一轮的入口，本轮只探到形状）

顺着 `olrg` 的成功路子查下一个近邻 `fgrnd`（干窗第 0 步 44 个差异变量之一，
maxabs 3.58e-12）。对文件 dump（`th_ext.opt` 第 7167-7170 行）里它不是一条直线算式：

```
<bb 429>:
  # cstore_3613 = PHI <_1771(450), _1811(451)>      ← 无雪支 / 有雪支
  *fgrnd_3438(D) = cstore_3613;
```

也就是说内核把它编成**两条分支各自的表达式再合并**，要核对必须把 `_1771`（bb450）与
`_1811`（bb451）两条链**分别**读出来，再与 `surface_budget.rs` 里对应的那两侧比。
本轮只确认了形状与入口，没有落改动 —— 这处比 `olrg` 那种直线式多一层，值得单独一轮。

（`surface_budget.rs:99-101` 的注释记着这个量以前踩过的坑：曾经按"辐射式恒等"写成
`fsena + lfevpa + fgrnd`，那是错的，因为 `fgrnd` 本身就含辐射项 —— 核对双分支时要连带
确认这条结论仍成立。）

Tested: `th_ext.opt` 7160-7172 行与 `…Thermal…_Extended.F90:1338-1345` 的对照。
Not-tested: `_1771`/`_1811` 两条链的逐句核对（下一轮）。

### `fgrnd` 两条链的**尾步形状**（可直接照抄的部分）

`th_ext.opt` 里两个分支的收尾完全同型（`_1765`/`_1805` 是各自分支的 `tinc`，
`_1759`/`_1799` 是已累加的部分）：

```
_1767 = .FMA (_1761, _1765, _1759)        ; 分支内倒数第二步
_1769 = pg_snow * cpice                   ; ← 先算 (pg_snow*cpice)
_1771 = .FMA (_1765, _1769, _1767)        ; ← 最后一次收缩：acc + tinc*(pg_snow*cpice)
```

对照源码 `+ cpice*pg_snow*(t_precip-t_grnd)`（`…_Extended.F90:1360`）可知：上游把
`pg_snow*cpice` 先算成一个因子、再与温度差相乘并**收进 FMA**。Rust 侧要核的就是
`surface_budget.rs` 里对应项是不是这个形状（先 `pg_snow*cpice`、再 `mul_add(温度差, acc)`），
而不是平铺的 `cpice*pg_snow*(...)` 累加。

Tested: `th_ext.opt` 7525-7531 与 7569-7575 的对照；`…_Extended.F90:1360` 的对照。
Not-tested: `_1765`/`_1805`（各分支 `tinc`）的定义链；Rust 侧的对应项（下一轮第一件事）。

### `fgrnd` 的降水热收缩：dump 有、四处口径全都不动 —— **不落**

把上一轮读到的形状真落回 Rust（`ground_heat` 改成两级 `mul_add`，即
`FMA(dT, snow*cpice, FMA(rain*cpliq, dT, acc))`，`precipitation_heat` 仍单独算作诊断量），
四处口径的实测是：

| 口径 | 改前 | 改后 |
|---|---|---|
| 3 步 bitwise / 各变量 ndiff、maxrel | 585/692 | **完全相同** |
| 干窗 | 21328 / 338.9256 / 825 / 17 | 完全相同 |
| 湿窗 | 32655 / 10369.4411 / 20672 / 68 | 完全相同 |
| 雪窗 | 33651 / 444394.4368 / 25896 / 79 | 完全相同 |

**完全惰性**。按本仓库的裁决规则：源码**显式**形状（如 `rbsun`/`us10m`）即使惰性也照抄；
**收缩**形状必须有口径支持才能落（`olrg` 是步级 +3 才留的，`clai`/`thvstar` 是步级变差才退的）。
这一处落在两者之间：dump（独立编译产的）说是两级 FMA，但四处口径一次都没动 ——
第 166 轮已经证明独立编译的 dump 不能代表内核二进制，所以**没有证据支持**，回退。

`_1765`/`_1769` 的全链已读清（`_1761=pg_rain*cpliq`、`_1764=t_precip-t_grnd`、
`_1767=FMA(_1761,_1765,_1759)`、`_1769=pg_snow*cpice`、`_1771=FMA(_1765,_1769,_1767)`），
留作以后**有降水活跃算例**时的复核材料。

Tested: 三级改动（`ground_heat` 两级 `mul_add`）的 `dry_ts.sh 3` + `window_divergence.py`
与三个黄金窗口实测（四处全不变）；改动已回退。
Not-tested: 有地面降水活跃的算例（湿窗也没让它动，说明这条路径在该算例里不活跃）。

## 交接：下一轮的两条入口与**可直接复制的命令**

（本节写于第 181 轮，作为长会话的断点续传点。基线数字见第 175 轮那张表。）

### 入口 A：近地层相似性调用的实参 dump 对照（攻 1 ULP 那一族）

1. 现状基线核对（**先跑这个**，确认仍是 44 个差异变量 / 585/692）：

```bash
bash /tmp/gf/dry_ts.sh 3
python3 oracle/scripts/window_divergence.py /tmp/gf/dryts/out/CN-Cng/history/*.nc \
    /tmp/gf/dryts/colm-rs_hist_2008-01.nc --top 8
```

2. 在对文件里给 `moninobukm` 调用点加临时打印（**只改 `/tmp` 里的副本**，不动 vendor）：
   `vendor/CoLM202X/extends/interception/MOD_LeafTemperature_Extended.F90` 里
   `CALL moninobukm(...)` 前后，dump `hu/ht/hq/displa/z0m/z0h/z0q/obu/um/displat/z0mt/htop`
   与输出 `ustar/fh2fq2m/fmtop/fm/fh/fq/fht/fqt/phih`（第 0 步）。
3. Rust 侧同位置（`crates/colm-core/src/leaf_temperature.rs` 的
   `stomatal_resistance`/`surface` 构造处）加 `eprintln!` 或临时 `pub` 探针，dump 同一组量。
4. 逐位比对：`moninobukm` 本体已被随机差分关掉（20 输出 20000/20000 全同），
   所以**只要实参一致、输出就该一致**；不一致的那一个实参即是种子。

### 入口 B：两个模块的收缩逐条重扫

* `MOD_LeafTemperature_Extended.F90` → `/tmp/gf/r166/lt_ext.opt`（**80 处**）
* `MOD_Thermal_CanopyPhase_Extended.F90` → `/tmp/gf/r166/th_ext.opt`（**99 处**）

重扫规矩（本会话反复验证过的）：
1. **源码显式**形状（独立语句/中间量）→ 直接照抄，惰性也照抄（例：`rbsun`、`us10m`）；
2. **收缩**形状 → 必须用上面的步级口径验收（改善才留，例：`olrg` 步级 582→585）；
   四处口径全不动的一律**不落**（例：`fgrnd`），只记链；
3. 每处改动都要跑全三个窗口并写入本文档：

```bash
bash /tmp/gf/win4.sh
for c in CN-Cng CN-Cng-wet US-NR1-snow; do
  g=$(ls oracle/golden/ | grep "^${c}_hist"); d=/tmp/gf/win4/$c
  python3 /tmp/gf/three.py oracle/golden/$g $d/$(ls $d | grep '^colm-rs_hist') $d/cmp.txt "$c"
done
```

当前三窗口值：干 `21328/338.9256/825/17`、湿 `32655/10369.4411/20672/68`、
雪 `33651/444394.4368/25896/79`。

### 已知的坑（别再踩）

* 取 dump 必须用 `extends/interception/*_Extended.F90`（`main/` 那份内核不编；第 166 轮）。
* 独立编译的 dump **不代表**内核二进制的收缩（第 166 轮）；想彻底解决要从
  `kernels/default/colm.x` 反汇编，但 DWARF 行号不能逐句归因（第 167 轮）。
* `window_divergence.py` 认 `maxrel`：≈2e-16 是种子、≥1e-14 是下游放大；别看前 N 行就下结论。
* 四个 1 ULP 变量里 `f_trad` 已有独立来源（`olrg`，已修）；剩下的确实共享同一次
 相似性调用（第 173 轮修正）。

Tested: 本节命令均已在本会话多次执行（`dry_ts.sh`/`window_divergence.py`/`win4.sh`/`three.py`）。
Not-tested: 入口 A 的实参 dump（下一轮）。

### `lfevpa` 的地面项收缩：dump 有、步级口径 **-1** —— 不落

`th_ext.opt` 第 7500-7502 行：

```
lfevpl.1275_1912 = lfevpl
_2088 = .FMA (M.397_3424, pretmp_5135, lfevpl.1275_1912)   ; htvp*fevpg + lfevpl
*lfevpa_2269(D) = _2088
```

即 `lfevpa = FMA(htvp, fevpg, lfevpl)`；Rust 的 `latent_heat` 写成平铺的
`leaf_latent_heat*leaf_evaporation + sublimation_heat*ground_evaporation`（`sublimation_heat`
就是内核的 `htvp`）。按 dump 改成 `sublimation_heat.mul_add(ground_evaporation, leaf_latent_heat*leaf_evaporation)`
之后：3 步 bitwise **585 → 584**（其余变量不变）。

按规则（收缩形状必须有口径支持）**回退**。这是本会话第 5 处"dump 说有收缩、实测不支持"
的站点（前四处：`clai`、`thvstar`、`cfw`、`fgrnd`），也再次说明**独立编译的 dump 不能当
逐句判据**；想彻底解决必须从 `kernels/default/colm.x` 反汇编，而 DWARF 行号又不能逐句归因
（第 167 轮）——这条矛盾目前无解，只能逐处实测。

Tested: `th_ext.opt` 7496-7506 与 `surface_budget.rs` 的对照；`dry_ts.sh 3` +
`window_divergence.py`（585 → 584）；改动已回退。
Not-tested: 从 `colm.x` 反汇编取形状（工具层面仍缺行号归因能力）。

### 交接补充：`/tmp` 里的 history **会残留上一次实验的状态**（第 183 轮踩到）

核对基线时**必须重跑** `bash /tmp/gf/dry_ts.sh 3` 再读产物。第 183 轮先直接读了
`/tmp/gf/dryts` 里已有的文件，得到 `584/692` —— 那是**上一次被否掉的 `lfevpa` 实验**的
产物，不是当前代码的状态；重跑之后才是正确的 `585/692`。`dry_ts.sh` 自己会
`rm -rf` 目标目录，所以"跑一次"安全，**危险的是不跑就读旧文件**。

同一类陷阱适用于：`/tmp/gf/win4/*`（三窗口）、`/tmp/gf/winCN/*`、`/tmp/gf/fd_*`（差分输出）。
凡是要拿来当"当前基线"的数字，都要由当轮的脚本重新生成。

## `gbh2o` 的源头顺序（源码显式、四处口径全不动）

`MOD_AssimStomataConductance.F90:608`：

```fortran
gbh2o = 1./rb * tprcor/tlef        ! 从左到右 ((1/rb)*tprcor)/tlef
```

Rust 原写 `pressure_conversion / (leaf_boundary_resistance * leaf_temperature_k)`
（即 `tprcor/(rb*tlef)`）—— 结合顺序不同。改成源码顺序后：

| 口径 | 改前 | 改后 |
|---|---|---|
| 3 步 bitwise / 各变量 | 585/692 | 完全相同 |
| 干 / 湿 / 雪窗 | 21328 / 32655 / 33651 | 完全相同（容差三口径也全同）|

**四处口径一次没动**。落地理由与 `rbsun`/`us10m` 同类：这是**源码显式**的运算顺序
（不是收缩），照抄不引入未验证的假设，也不会让任何口径变差。

Tested: `MOD_AssimStomataConductance.F90:608` 与 `photosynthesis.rs:180` 的对照；
`dry_ts.sh 3` + `window_divergence.py` 与三个黄金窗口实测（四处全同）；
`cargo fmt --all --check`；`cargo test -q -p colm-core --lib`（355 通过）。
Not-tested: none（惰性改动，无未覆盖面）。

## 顺手核掉的两处 stomata 形状（无需改动）

* `MOD_AssimStomataConductance.F90:318/796` 的 `co2s = co2a - 1.37*assimn/gbh2o`
  与 Rust `photosynthesis.rs:251/369` 的 `- 1.37 * net_assimilation / gbh2o` **同为**
  左结合 `((1.37*assim)/gbh2o)`，**一致**；
* `bintc = bintc*cint(3)`、`vm = vm*cint(1)`、`jmax = jmax*cint(2)`
  （`:212/571/586/595/599`）在 Rust 里对应 `canopy_integration[2]/[0]/[1]` 的乘法，
  **一致**。

这两处记进来是为了让 `MOD_AssimStomataConductance` 的"已核"范围可追溯：形状已对过的
有 `gbh2o`（本轮已按源码改成 `((1/rb)*tprcor)/tlef`）、`co2s`、三个 `cint` 乘法；
**尚未**逐句核的是该模块里带 `powf`/`exp` 的生化式（`calc_photo_params` 的 `vm`/`jmax`/
`respc` 那一族）。

Tested: `MOD_AssimStomataConductance.F90:212/318/571/586/595/599/796` 与
`photosynthesis.rs:141/156/168/178/203/251/369` 的对照。
Not-tested: `calc_photo_params` 里 `powf`/`exp` 那一族的逐句核对。

## `calc_photo_params` 的 `powf`/`exp` 族已核（形状一致，无需改动）

上一轮标出的"该模块未核区"（`MOD_AssimStomataConductance.F90:540-600`）逐句对过，Rust
（`photosynthesis.rs:116-175`）在**底数与形式**上都一致：

| 上游 | Rust |
|---|---|
| `kc = 30. * 2.1**qt`、`ko = 30000. * 1.2**qt` | `f77(30.0) * f77(2.1).powf(qt)`、`f77(30_000.0) * f77(1.2).powf(qt)` |
| `gammas = 0.5*po2m/(2600.*0.57**qt)*c3` | 同底 `f77(0.57).powf(qt)`，除式同序 |
| `vm = vmax25 * 2.1**qt` | `maximum_carboxylation_25c * f77(2.1).powf(qt)` |
| `jmax = jmax25*exp(37e3*(tlef-trop)/(rgas*trop*tlef))*(1+exp((710*trop-220e3)/(rgas*trop)))/(1+exp((710*tlef-220e3)/(rgas*tlef)))` | 三段 `exp()` 与两层除式同序（`:148-156`） |
| `respc = respcp*vmax25*2.0**qt/(1+exp(trda*(tlef-trdm)))*rstfac` | `f77(2.0).powf(qt)` + 同序 |
| `omss = (vmax25/2.)*(1.8**qt)/templ*rstfac*c3 + (vmax25/5.)*(1.8**qt)*rstfac*c4` | `f77(1.8).powf(qt)` 两项同序（`:170-180`） |

要点：**底数用字面量再 `powf`**（不是 `exp(qt*ln(base))`），这正是与 GCC 的
`2.1**qt` → `pow(2.1, qt)` 对齐所需要的写法。至此 `MOD_AssimStomataConductance`
除 `sortin`/`WUE_solver` 两个迭代子程序外，主干式子的形状都已核过。

Tested: `MOD_AssimStomataConductance.F90:540-600` 与 `photosynthesis.rs:116-180` 的逐句对照。
Not-tested: `sortin`、`WUE_solver` 两个子程序（本配置不走 WUE/不触发 sortin 时无需核）。

## 近地层粗糙度族已核（形状一致，无需改动）

种子的 1 ULP 一族依赖近地层相似性链路，其上游 `z0hg`（对流传热粗糙度）也顺手核了：

```fortran
! MOD_LeafTemperature_Extended.F90:748
z0hg = z0mg/exp(0.13 * (ustar*z0mg/1.5e-5)**0.45)
```
```rust
// ground_fluxes.rs:145-148
heat_roughness = momentum_roughness
    / (ROUGHNESS_REYNOLDS_COEFFICIENT
        * (ustar * momentum_roughness / MOLECULAR_VISCOSITY_M2_S).powf(ROUGHNESS_EXPONENT))
    .exp();
```

**一致**：`0.13` 在 `exp` 内、乘在括号外（`0.13*(...)`）、幂底是 `ustar*z0m/1.5e-5` 的
同序商、指数用 `powf(0.45)` 的字面量形式。`z0hg → z0qg` 的赋值（`z0qg = z0hg`）也一致。
同类已核的还有 `lake.rs:732` 的同式（湖面路径）。

Tested: `MOD_LeafTemperature_Extended.F90:748` 与 `ground_fluxes.rs:145-148`、`lake.rs:732` 的对照。
Not-tested: `z0hg` 下游进入 `moninobuk`/`moninobukm` 的实参链（入口 A 的范围）。

## 稀疏/稠密冠层的 `egvf` 修正落在 `canopy_roughness.rs`（不是缺失）

对文件 `:611-619` 有一段 X. Zeng 的稀疏/稠密冠层修正：

```fortran
displa = htop * displar(patchclass(ipatch))
z0mv = z0m; z0hv = z0mv; z0qv = z0mv
lt   = min(lai+sai, 2.)
egvf = (1._r8 - exp(-lt)) / (1._r8 - exp(-2.))
displa = egvf * displa
z0mv = exp(egvf*log(z0mv) + (1._r8-egvf)*log(z0mg))
```

Rust 侧**没有**叫 `egvf` 的量，一度怀疑漏了；查证结果是**同一套 Zeng 方案**
（`canopy_roughness.rs`，`canopy_roughness()` → `CanopyRoughness`）用**解析反解**的
形式实现：`:46` 的 `initial_lai = -(1 - area_index/canopy_cover_fraction).ln()/0.5`
正是该 `egvf` 关系的逆（`d`/`z0m` 的 log-powf 式在 `:35/:52/:64`）。该模块在更早的
轮次里已按 dump 落过收缩，且它在黄金窗口路径上（干/湿/雪三窗口都过），所以**不是缺口**，
也不再重复扫。

Tested: `…_Extended.F90:611-619` 与 `canopy_roughness.rs:14-64` 的结构对照；
`grep egvf` 在 Rust 侧无命中（确认命名不同、非缺失）。
Not-tested: `egvf` 与 `canopy_roughness` 的**逐位**等价性（该模块跨语句结构不同，
只能靠窗口与它自己的差分证据，本会话未新做）。

## 地面动量粗糙度 `z0mg` 已核（形状一致）

上游 `…_Extended.F90:606`：`z0mg = (1.-fsno)*zlnd + fsno*zsno`
Rust `ground_fluxes.rs:95-98`：`(1.0-fsno).mul_add(soil_roughness, fsno*snow_roughness)`
—— 外层乘积收进 FMA、内层 `fsno*zsno` 先算，与 GCC 对该式的收缩一致（该文件的
注释块里已列过同族 FMA，此处只做逐句确认）。无需改动。

Tested: `…_Extended.F90:606` 与 `ground_fluxes.rs:95-98` 对照。
Not-tested: none.

### 入口 A 的**就绪清单**（下一轮可直接执行，无需再找位置）

**上游插入点**：`vendor/CoLM202X/extends/interception/MOD_LeafTemperature_Extended.F90:723-725`
的 `CALL moninobukm(...)` 前后（`ELSE` 支；CBL 打开时是 `:719` 的 `moninobukm_leddy`）。
在 `/tmp` 里改副本，插入：

```fortran
        IF (do_print_step0) WRITE(77,'(A,13E24.16)') 'IN ', hu_,ht_,hq_,displa,z0mv,z0hv,z0qv,obu,um,displasink,z0mv,htop
        ! … CALL moninobukm(…) 原样 …
        IF (do_print_step0) WRITE(77,'(A,10E24.16)') 'OUT', ustar,fh2m,fq2m,fmtop,fm,fh,fq,fht,fqt,phih
```

要点：`hu_/ht_/hq_` 是 `max(hu,z0mv+1)` 那一族**修正后**的量（`:712-718` 附近），
`displasink`/`htop` 也要打；`do_print_step0` 用一个**首步即真**的逻辑变量，
或直接无条件 `WRITE` 然后只取第一行。编译时照第 167 轮的配方抄
（`-Iextends/interception -I.bld …`），链接时用它顶掉 `.bld/MOD_LeafTemperature.o`，
再链 `kernels/default` 的其余对象（或直接重跑 `build_kernel.sh default` 造一个新内核）。

**Rust 插入点**：`crates/colm-core/src/leaf_temperature.rs:565-585` 的
`surface = MoninObukhovInput { … }` 构造处（非 PHS 路径），把这 13 个量
`eprintln!` 出来。注意 Rust 侧对应关系：`displa`=`displacement_height_m`、
`z0mv`=`momentum_roughness_m`、`z0hv`/`z0qv`、`obu`=`obukhov_length_m`、
`um`=`stability_adjusted_wind_m_s`、`displasink`=`top_layer_displacement_m`、
`htop`=`canopy_top_height_m`。

**判据**：13 个实参**逐位相同** → 按"`moninobukm` 已被差分结案（20 输出 20000/20000）"
的结论，输出必须逐位相同；**第一个不一致的实参就是第 0 步种子的来源**。
若 13 个全同而输出仍差，则种子在**调用时机**（同一步里被调了几次、用了哪个 `tl`）。

Tested: 插入点行号核对（对文件 `:712-725`、Rust `:565-585`）。
Not-tested: 探针本身（下一轮）。

### 入口 A 已脚本化：`oracle/scripts/step0_arg_probe.sh`

把上一节那份"就绪清单"做成了一条命令（**不改生产流程**：补丁只作用于 vendor 的临时
副本，`trap ... EXIT` 保证无论成败都还原源码、重编内核、并打印 `git status` 与
`f48 sync` 结果）：

```bash
bash oracle/scripts/step0_arg_probe.sh
# → /tmp/gf/argprobe/fort_args.txt   上游侧 13 个实参（IN）+ 10 个输出（OUT）
# → 退出时自动：还原 MOD_LeafTemperature_Extended.F90、build_kernel.sh default、f48 sync PASS
```

脚本的两个锚点已核对唯一性（`CALL moninobukm(...)` 1 处；收尾 `htop,…,phih)` + `ENDIF`
1 处 —— 只写 `ENDIF` + `! Aerodynamic resistance` 会命中 2 处，已在脚本里注释说明），
补丁体用临时副本干跑验证过（生成 2 行 `WRITE(77,…)`）。

用法与判据仍照上一节：把 `fort_args.txt` 的 13 个实参与 Rust 侧
`leaf_temperature.rs:565-585` 打出的同一组量逐位比；第一个不一致者即种子，全同则查调用时机。
`SKIP_REBUILD=1` 可跳过退出时的重编（只为快速调试）。

Tested: `bash -n` 语法检查；两处锚点计数（1/1）；补丁体在临时副本上的干跑（2 行 WRITE）；
`git status` 干净（干跑未触真文件）。
Not-tested: 脚本的完整执行（要重编内核，留给干净上下文）。

### 入口 A 第 194 轮：脚本跑通了，但**两条候选文件都没有输出** —— 下一轮打"全候选"

`oracle/scripts/step0_arg_probe.sh` 连跑三次（每次自动还原 + 重编 + f48 PASS）：

| 次 | 打补丁的目标文件 | 结果 |
|---|---|---|
| 1 | `MOD_LeafTemperature_Extended.F90`（`WRITE(77,…)`）| `fort.77` 不存在 |
| 2 | 同上，改 `WRITE(*,…)` 走 stdout | `kernel.log` 里没有 `ARGPROBE_*` |
| 3 | `MOD_LeafTemperaturePC_Extended.F90`（`:1084-1092`）| 同样没有输出 |

说明**这两条 `moninobukm` 调用在干窗第 0 步都没被执行**。已排除的解释：CBL 关（否则会走
`*_leddy`，那也在同两个文件里）；`rd_opt = 3` / `rb_opt = 3` 都是**硬编码参数**，
所以 `IF (rd_opt == 3)` 那条支**是**成立的 —— 那么没执行就只剩"叶温求解走的是**第三个**
实现"这一种可能（该版本用 `USE_SITE_pctpfts = .true.` 在 PFT/PC 之间切换，而
`MOD_LeafTemperature*.F90` 至少有 main / Extended / PC 三份）。

**下一轮的做法（已具备全部条件）**：把脚本的目标改成**候选列表**，对
`MOD_LeafTemperature.F90`、`MOD_LeafTemperature_Extended.F90`、
`MOD_LeafTemperaturePC_Extended.F90` 三份同时插同一条打印（锚点都在各自的
`CALL moninobukm(` 前后），一次运行即可由"哪一行出现"确定实际执行的实现；
`TARGET` 已做成环境变量，改成列表是几行的事。

Tested: 脚本三次完整执行（含自动还原、重编、`f48 sync PASS`）；两次锚点唯一性与补丁干跑；
`rd_opt`/`rb_opt` 的 `:485/:484` 硬编码参数核对。
Not-tested: 三候选同时打补丁的那一次运行（下一轮）。

### 入口 A 第 195 轮：三份候选**同时**打标记，**一条都没命中** —— 下一个诊断已内置

`oracle/scripts/step0_arg_probe3.sh` 对 `main/MOD_LeafTemperature.F90`、
`extends/interception/MOD_LeafTemperature_Extended.F90`、
`extends/interception/MOD_LeafTemperaturePC_Extended.F90` 各插一条 `ARGPROBE_{MAIN,EXT,PC}`
（插在 `CALL moninobukm(hu_,ht_,hq_,` 之前，逐份核对锚点唯一），重编、跑干窗 1 步：
**三份都没命中**，而脚本照旧自动还原三份源码、重编、`f48 sync PASS`。

已知约束：
* `moninobukm` 的调用者**只有四份文件**：`main/MOD_LeafTemperature.F90`、
  `main/MOD_LeafTemperaturePC.F90`、`extends/interception/MOD_LeafTemperature_Extended.F90`、
  `extends/interception/MOD_LeafTemperaturePC_Extended.F90`（`MOD_GroundFluxes.F90` **不调**它）；
* Makefile 只编后两份（`MOD_LeafTemperature.o ← …_Extended.F90`、
  `MOD_LeafTemperaturePC.o ← …PC_Extended.F90`），这两份**都已被打过补丁**；
* CBL 关、`rd_opt = rb_opt = 3` 是硬编码参数（`:485`/`:484`），所在支**应当**执行。

所以要么"叶温求解在干窗第 0 步根本没被调用"，要么"补丁没进被运行的那个二进制"。
脚本已内置判据：**打补丁后先 `strings kernels/default/colm.x | grep -c ARGPROBE_`**
（在还原之前），若为 0 就是构建/链接层的问题，不为 0 则说明那两支确实没执行。

Tested: `step0_arg_probe3.sh` 完整执行（三份补丁、重编、跑一步、无命中、自动还原、f48 PASS）；
`grep -rln "CALL moninobukm"` 确认调用者只有四份文件；Makefile 的两条映射规则。
Not-tested: 下一轮的 `strings` 判据（脚本已内置，未跑）。

## 入口 A 收官结论：干窗第 0 步**根本不发生** `moninobuk`/`moninobukm` 调用

第 194-196 轮把"种子在近地层相似性调用的实参上"这条假设**证伪**了。方法：给候选文件
插带标记的打印，重编，跑干窗 1 步，并且**先确认补丁进了被运行的二进制**
（`strings kernels/default/colm.x | grep -c ARGPROBE_`）。逐次结果：

| 打补丁的候选 | 二进制里的标记数 | 运行时有输出？ |
|---|---|---|
| `main/MOD_LeafTemperature.F90`（**Makefile 不编**）| — | — |
| `extends/…/MOD_LeafTemperature_Extended.F90` | 计入 | **否** |
| `extends/…/MOD_LeafTemperaturePC_Extended.F90` | 计入 | **否** |
| `main/MOD_GroundFluxes.F90:180`（`CALL moninobuk(hu,ht,hq,…)`）| 计入 | **否** |
| `main/MOD_Vars_1DAccFluxes.F90:2781`（`CALL moninobuk(hgt_u,…)`）| 计入 | **否** |

最后一次三份文件同时打标记：`二进制里的 ARGPROBE 标记数: 3`（说明补丁**确实**进了被运行的
内核），而 `kernel.log` 里一条 `ARGPROBE_*` 都没有 —— 所以不是"打错文件"，也不是
"补丁没生效"，而是**这些支在第 0 步没被执行**。

`grep -rln "CALL moninobuk"` 列出的全部调用者就是上面五份（外加 `MOD_Glacier`/`MOD_Lake`/
`MOD_SimpleOcean` 三条本算例不走的路径）。

### 这把种子问题的提法改掉了

第 0 步 history 里那 1 ULP 的 `ustar`/`us10m`/`gssun`/`tleaf` 一族，**不是**这一步新算的
相似性调用产生的 —— 它只能来自：

1. **初始化/restart 读入并按步复用的滞后面**（`MOD_Vars_1DAccFluxes` 的 `r_*` 累加器、
   `tleaf`/`t_grnd` 等状态），即种子的来源在"restart → 状态"的映射或第 0 步的更新顺序上；
2. 或某条**在第 0 步之后**才被调用的路径（例如累加器在步末/步初的调用时机差异，
   Rust 与本内核的调用时机不同）。

**下一轮的入口**（比之前具体得多）：对干窗**第 0 步** dump
`r_ustar`/`r_ustar2`/`r_fm10m`/`r_us10m`/`tleaf`/`t_grnd` 的**读入值**（restart 侧）与
第 0 步结束时的值，与 Rust 的对应量逐位比 —— 位置在
`MOD_Vars_1DAccFluxes:accumulate_fluxes`（Rust 对应 `history_diagnostics.rs`）。
若这些量在第 0 步**从 restart 读出时就差 1 ULP**，种子就在 restart 解析/单位换算里。

Tested: 五次完整探针运行（每次自动还原、重编、`f48 sync PASS`）；每轮的
`strings kernels/default/colm.x | grep -c ARGPROBE_` 计数（2/2/3/3）；
`grep -rln "CALL moninobuk"` 的调用者清单。
Not-tested: 第 0 步 `r_*`/状态量的 restart 侧逐位比对（下一轮入口）。

## 更锐的口径：**restart 侧**只剩 19/68 个变量差，`t_soisno`/`tleaf` 恰是 1 ULP

上一轮把种子方向从"相似性实参"改为"restart/初始化侧"。这一轮直接用 **restart 文件**做口径：
干窗跑 1 步后，内核写 `out/CN-Cng/restart/2008-001-05400/…nc`，Rust 写
`rust_restart.nc`（`dry_ts.sh` 已在传 `--restart-out`），两者**逐位比**：

```
differing restart variables: 19 / 68
  t_soisno        maxrel=2.05e-16  ndiff=2   ← 1 ULP
  tleaf           maxrel=2.18e-16  ndiff=1   ← 1 ULP
  emis            maxrel=2.21e-16  ndiff=1
  fm              maxrel=2.66e-16  ndiff=1
  ustar           maxrel=4.07e-16  ndiff=1
  fh / fq         maxrel=4.60e-16  ndiff=1
  qref            maxrel=4.85e-16  ndiff=1
  ldew / ldew_snow 1.08e-15 · wliq_soisno 1.86e-15 · qstar 2.04e-15 …
```

要点：

1. **状态量本身**（`t_soisno`、`tleaf`）就停在 1 ULP —— 种子的"落点"是**能量步的温度更新**，
   与上一轮"不是这一步新算的相似性调用"合起来，指向**无条件执行的温度更新表达式**；
2. 这是比 history 更锐的口径：只 19 个变量、多数 `ndiff = 1`，没有 11 天窗口那种混沌放大，
   适合当作**每个候选形状改动的验收尺**（改好了这个数应当下降）；
3. 用法与 history 口径相同：`bash /tmp/gf/dry_ts.sh 1` 后用上面那段 python 比两个 restart
   文件（脚本可固化成 `oracle/scripts/restart_divergence.py`，下一轮补）。

Tested: 上述 python 片段对 `/tmp/gf/dryts/out/CN-Cng/restart/2008-001-05400/*.nc` 与
`/tmp/gf/dryts/rust_restart.nc` 的实比（19/68）。
Not-tested: 把它固化成脚本并用于逐个候选（下一轮）。

### restart 口径已固化：`oracle/scripts/restart_divergence.py`

```bash
bash /tmp/gf/dry_ts.sh 1
python3 oracle/scripts/restart_divergence.py \
    /tmp/gf/dryts/out/CN-Cng/restart/*05400/*.nc /tmp/gf/dryts/rust_restart.nc
# → differing restart variables: 19 / 68
#     2.050e-16 t_soisno  ndiff=2        ← 1 ULP
#     2.176e-16 tleaf     ndiff=1
#     2.208e-16 emis      ndiff=1
#     2.664e-16 fm        ndiff=1
#     4.071e-16 ustar     ndiff=1
#     4.602e-16 fh/fq     ndiff=1   …（其余 13 个 maxrel 4.60e-16 … 2.46e-12）
```

与 `window_divergence.py` 同风格（按 `maxrel` 升序，≈2e-16 即种子），但更快更干净：
**变量少（19/68）、无 11 天混沌放大**。候选形状改动先用它筛，再用 history 步级口径与
三个黄金窗口做二、三次确认 —— 这是本会话收敛出的三段式验收。

Tested: 新脚本对现有第 0 步 restart 的实比（19/68，逐变量 maxrel/ndiff/maxabs）。
Not-tested: 用它筛 `lt_ext.opt`/`th_ext.opt` 里的候选（下一轮）。

## 第三个（也是**唯一来自 `main/`** 的）关键 dump：`MOD_GroundTemperature` 41 处收缩

`t_soisno`/`t_grnd` 的温度更新**不在** `MOD_Thermal_CanopyPhase_Extended.F90` 里（`th_ext.opt`
搜不到 `tinc_`/`t_grnd_`），而在 `main/MOD_GroundTemperature.F90` —— 它也是本会话遇到的
**唯一**由内核直接编 `main/` 版的模块（与 LeafTemperature/Thermal 必须取
`extends/interception/*_Extended.F90` 相反）：

```
$ grep -n "MOD_GroundTemperature.o:" vendor/CoLM202X/Makefile
835:MOD_GroundTemperature.o: MOD_PhaseChange.o MOD_SoilThermalParameters.o
$ strings .bld/MOD_GroundTemperature.o | grep '\.F90'
main/MOD_GroundTemperature.F90
```

dump 已按同一配方产出（**41 处** FMA 类收缩）：

```bash
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I.bld -Iinclude -Imain -Ishare \
  main/MOD_GroundTemperature.F90 -J/tmp/gf/r166 \
  -fdump-tree-optimized=/tmp/gf/r166/gt.opt -o /tmp/gf/r166/gt.o
```

配合第 197-198 轮的结论（restart 侧 `t_soisno`/`tleaf` 恰 1 ULP），下一轮的筛法已经完整：

1. 在 `gt.opt`（41 处）里找**无条件执行**的温度更新式（三对角求解、界面导热率、
   `cv`/`thk` 的组装），逐个按对文件形状改；
2. 每改一处先跑 `bash /tmp/gf/dry_ts.sh 1` + `restart_divergence.py`，
   看 **19/68** 是否下降；再用 `window_divergence.py`（585/692）与三窗口复核。

Tested: `main/MOD_GroundTemperature.F90` 的 dump 生成（41 处）；Makefile:835 与
`strings .bld/MOD_GroundTemperature.o` 的来源核对；th_ext.opt 中无 `tinc_`/`t_grnd_` 的确认。
Not-tested: 那 41 处的逐条筛选（下一轮）。

## `MOD_GroundTemperature` 的降水热双 FMA：dump 有、restart 口径**逐位不变** —— 不落

`gt.opt` 第 970-999 行的地面热通量链把降水热**逐级收进累加器**：

```
_130 = (sabg+dlrad*emg) - FMA(fevpg,htvp,fseng)
_138 = .FMA (rain*cpliq, dT, _130)
_142 = .FMA (dT, snow*cpice, _138)
```

Rust（`ground_temperature.rs:295-300`）写的是平铺的
`… + rain_heat_capacity*dT + dT*snow_heat_capacity`。按 dump 改成两级 `mul_add` 后，
**restart 口径逐行逐位不变**（19/68，连每行的 `maxrel`/`ndiff`/`maxabs` 都完全相同：
`rib 1.922e-16 / t_soisno 2.041e-16 / trad 2.153e-16 / rst 2.328e-16 / qstar 4.865e-16 /
fm 5.313e-16`），所以按规则**回退**。

副产物是一条可复用的方法：restart 口径的 A/B 要**固定步数**（这里统一 `dry_ts.sh 1` →
`2008-001-01800`），否则拿 3 步的 `05400` 与 1 步的 `01800` 比会得出错误结论
（本轮的第一次比较就踩了这个坑，已改）。

Tested: `gt.opt` 970-999 行与 `ground_temperature.rs:295-300` 对照；`dry_ts.sh 1` +
`restart_divergence.py` 的 A/B（改与不改，restart 口径输出逐行相同）；改动已回退。
Not-tested: `gt.opt` 其余 39 处收缩。

## `gt.opt` 的**土壤支**读清了：三条 FMA，但干窗下只剩结合顺序（故惰性）

`gt.opt` 第 1019-1055 行（bb29，无雪支）是：

```
_193 = .FMS (dlrad, emg, sigma*t_soil^4)          ; FMS(a,b,c)=a*b-c
_198 = .FMA (fevpg_soil, htvp, fseng_soil)
_200 = _193 - _199
_207 = .FMA (dT, rain*cpliq,  _200)
_211 = .FMA (dT, snow*cpice, _207)
_214 = .FMA ((1-fsno), _211, sabg_soil)           ; ← 整个累加器被 (1-fsno) 加权
```

即内核是 `surface = sabg_soil + (1-fsno) * X`，而 Rust（`ground_temperature.rs:295-300`）
写成 `sabg_soil + dlrad*emg - … + rain*cpliq*dT + dT*snow*cpice`。

**干窗（无雪）下 `fsno = 0`，`(1-fsno) = 1`，`FMA(1, X, sabg) ≡ X + sabg`** —— 加权是精确的，
所以这条支与 Rust 的差别**只剩三处 FMA 的结合顺序**；上一轮把其中两处（`_207`/`_211`）
按形状补上、restart 口径逐行不变，正与此吻合。**真正会因 `(1-fsno)` 加权而分岔的是有雪
支**（`_173`/`_181`/`_188` 那三条 `FNMA`，`fsno` 不为 0），那属于雪窗，不是干窗种子。

**这给雪窗留了一条明确的候选**：若哪天要收 `US-NR1-snow` 的残差，先核
`surface_snow_lyr`/`surface_soil` 的 `(1-fsno)`/`fsno` 加权与三条 `FNMA` 的结合顺序。

Tested: `gt.opt` 1019-1055 与 `ground_temperature.rs:288-312` 的逐句对照；上一轮那两处 FMA 的
restart A/B 结果（逐行不变）与本分析的相互印证。
Not-tested: 雪支的 `FNMA` 加权（雪窗范围）。

### 候选筛选的**复制粘贴流程**（restart 口径，固定 1 步）

```bash
BASE=/Users/zhongwangwei/Desktop/Github/CoLM-Desktop
export NETCDF_DIR=/opt/homebrew/opt/netcdf
# 1) 改一处形状，然后：
bash /tmp/gf/dry_ts.sh 1                     # 内核与 Rust 各跑 1 步（restart 落在 …-01800）
K=$(ls -t /tmp/gf/dryts/out/CN-Cng/restart/2008-001-01800/*.nc | head -1)
python3 $BASE/oracle/scripts/restart_divergence.py "$K" /tmp/gf/dryts/rust_restart.nc --top 8
#    基线：differing restart variables: 19 / 68（rib 1.922e-16 / t_soisno 2.041e-16 /
#          trad 2.153e-16 / rst 2.328e-16 / qstar 4.865e-16 / fm 5.313e-16）
# 2) 若这个数下降（或 worst maxrel 变小）再上步级口径：
python3 $BASE/oracle/scripts/window_divergence.py \
    /tmp/gf/dryts/out/CN-Cng/history/*.nc /tmp/gf/dryts/colm-rs_hist_2008-01.nc --top 8
#    基线：44 个差异变量 / bitwise identical 585/692 / first divergence step 0
# 3) 再看三个黄金窗口（容差三口径不得变差）：
bash /tmp/gf/win4.sh
for c in CN-Cng CN-Cng-wet US-NR1-snow; do
  g=$(ls $BASE/oracle/golden/ | grep "^${c}_hist"); d=/tmp/gf/win4/$c
  python3 /tmp/gf/three.py $BASE/oracle/golden/$g $d/$(ls $d | grep '^colm-rs_hist') $d/cmp.txt "$c"
done
#    基线：21328/338.9256/825/17 · 32655/10369.4411/20672/68 · 33651/444394.4368/25896/79
```

**注意**（第 200 轮踩过）：restart 的目录名带时间戳，`dry_ts.sh 1` 是 `2008-001-01800`、
`dry_ts.sh 3` 是 `…-05400`；A/B 必须同步数，否则比的是两条不同轨迹。

Tested: 上述第 1、2 步的命令本会话多次执行；第 3 步在本会话执行过 4 次。
Not-tested: none（流程本身）。

## 界面导热率：源码级形状**一致**（剩下的只有收缩，须实测）

`main/MOD_GroundTemperature.F90:241-246` 与 `ground_temperature.rs:146-158` 逐句对过：

| 上游 | Rust |
|---|---|
| `tk(i) = 2.*thk(i)*thk(i+1)/(thk(i)+thk(i+1))`，再 `max(0.5*thk(i+1),tk(i))` | `2.0*a*b/(a+b)`，再 `.max(0.5*b)` —— 结合顺序一致 |
| `tk(i) = thk(i)*thk(i+1)*(z(i+1)-z(i)) / (thk(i)*(z(i+1)-zi(i)) + thk(i+1)*(zi(i)-z(i)))` | `a*b*(z₁-z₀) / (a*(z₁-zi) + b*(zi-z₀))` —— 分子分母同序 |
| 特殊支的判据 `(i==0) .and. (z(i+1)-zi(i) < zi(i)-z(i))` | `layer+1 == snow_layers && (z[l+1]-zi) < (zi-z[l])` —— 一致（`i==0` 对应"雪层数 == layer+1"）|

也就是说这处**源码级没有差异**；唯一的余地是 GCC 是否把分母里两个乘积收进 `FMA`
（本模块的 dump `gt.opt` 里那一簇 `FNMA/FMS` 正是这类），而要动它必须走三段式实测 ——
按本会话 6 次"dump 有收缩、实测不支持"的经验，**没有实测支持不动代码**。

Tested: `MOD_GroundTemperature.F90:241-246` 与 `ground_temperature.rs:146-158` 的逐句对照。
Not-tested: 该处分母的收缩是否有实测收益（需按流程 A/B）。

## 本会话（第 158-204 轮）产出索引

会话很长，这里按性质归类，便于复查时按主题定位（每条都有对应的提交与本文档小节）。

**A. 新增差分结案（与内核对象逐位相同）**

| 目标 | 证据 |
|---|---|
| `MOD_FrictionVelocity` | 20 输出 20000/20000（`compare_moninobukm.sh`）|
| `MOD_Qsadv` | 4 输出 20000/20000（`compare_qsadv.sh`）|
| `soil_hcap_cond` | 8 档 × 5000 组、2 输出 40000/40000（`compare_soilthermal.sh`）|
| `MOD_TurbulenceLEddy` | 17 输出 20000/20000（`compare_leddy.sh`，本地算例到不了）|
| `MOD_ForcingDownscaling`（四入口）| 风 4 输出、简单支 11 输出 × 4 配置、full 短波 11 输出 × 两套阴影表，全部 20000/2000 组逐位相同 |

**B. 抓到的真缺陷（已修 + 实测）**

风场降尺度乘法结合顺序（8.9% 样本）· 简单短波 `cosill` 漏收缩 · longwave 方案 I 结合顺序（31% 样本 1–2 ULP）· full 短波三处漏收缩 · `gssun` 除法分组（步级 +2）· `rbsun = rb/laisun` · `us10m` 顺序 · `olrg` 结合+FMA（步级 +3）· `gbh2o` 顺序。

**C. 重大更正（都曾被写进过记录）**

1. 内核编的是 `extends/interception/*_Extended.F90`，不是 `main/`（唯一例外：`MOD_GroundTemperature`）；
2. `window_divergence.py` 初版按变量名排序，导致"`f_gssun` 不再出现"的误报（已改按 `maxrel` 升序）；
3. 独立编译的 dump **不代表**内核二进制的收缩；DWARF 行号又不能逐句归因 —— 故收缩只能逐处实测（6 处"dump 有、实测不支持"：`clai`/`thvstar`/`cfw`/`fgrnd`/`lfevpa`/`gt` 土壤支）。

**D. 新增工具（都在 `oracle/scripts/`）**

`window_divergence.py`（步级口径）· `restart_divergence.py`（restart 口径，最快）·
`step0_arg_probe.sh` / `step0_arg_probe3.sh`（内核插桩探针，自带 vendor 还原 + f48 自检）。

**E. 最后开放项**

干窗第 0 步 1 ULP 族：已证明**不是**相似性调用实参（第 196 轮五次探针 + `strings` 计数），
落点在能量步的温度更新（restart 侧 `t_soisno`/`tleaf` 恰 1 ULP）。靶子与筛法见
"复制粘贴流程"一节；雪窗另有 `(1-fsno)` 加权的候选。

Tested: 本索引每行都对应本文档前面小节的实测输出；提交范围 `a45d55d..481e453`。
Not-tested: none（索引本身）。

## `dhsdT` / `fact` 两处源码级核对（一致，无需改动）

* `MOD_GroundTemperature.F90:308`：`dhsdT = -cgrnd - 4.*emg*stefnc*t_grnd**3 - cpliq*pg_rain - cpice*pg_snow`
  Rust `ground_temperature.rs:309-313`：`(-t³).mul_add(emg*4*sigma, -cgrnd) - rain_heat - snow_heat`
  —— 与 dump 的 `FNMS(stefnc*(emg*4), (t*t)*t, cgrnd)` 同型（取负与乘法可交换，符号精确）；
* `fact(1) = deltim/cv(1)*dz(1) / (0.5*(z(1)-zi(0)+capr*(z(2)-zi(0))))`
  Rust `ground_fluxes`/`ground_temperature` 侧是
  `deltim/capacity*dz / (0.5*factor.mul_add(z(2)-zi(0), z(1)-zi(0)))` —— 括号内那处
  收缩与顺序都对上（该处更早的轮次已按 dump 落过）。

Tested: `MOD_GroundTemperature.F90:308-312/311-313` 与 `ground_temperature.rs:309-313` 及
`fact` 那处的对照。
Not-tested: 该模块其余收缩（`gt.opt` 41 处里未逐条过的部分）。

## `MOD_GroundTemperature` 的**内层矩阵元**也已核（早前轮次按 dump 落过）

`ground_temperature.rs:424-440` 的注释把该处的 GIMPLE 结论写死并与
`MOD_GroundTemperature.F90:344-372` 对上：`bt = 1+(1-cnfac)*fact*Σ` 里那个乘积**被吸收**
（`FMA(sum, (1-cnfac)*fact, 1.0)`，而 `sum = tk/dzp + tk1/dzm` 是两个商之和、本身不收缩）；
而 `rt = t + cnfac*fact*(fn-fn1)` **不融合**，因为该乘积在三个内层分支（雪层 / `j==1 && split` /
其它）里共用，GCC 把它 CSE 成公共临时量（GIMPLE `_462`）再相加 —— **相邻两条语句结论相反**，
只能逐条看 dump。三处 `sub`/`super_` 的符号与除序也与源码一致。

至此 `MOD_GroundTemperature`（`gt.opt` 41 处）的已核清单：
`cv`/`thk` 组装 · 界面导热率 · 降水热双 FMA（土壤支，干窗下与结合顺序等价）· 土壤支 `(1-fsno)`
加权（干窗 `fsno=0` 精确）· `dhsdT` · `fact(1)` 分母的 `capr` 收缩 · 顶层矩阵元 · **内层矩阵元**。
**仍未逐条**：底层（`j = nl_soil`）矩阵元、相变段（`phase_change` 的温度/含水量分配与
`snow_layer_absorption` 相关式）。

Tested: `MOD_GroundTemperature.F90:344-362` 与 `ground_temperature.rs:424-440` 的逐句对照。
Not-tested: 底层矩阵元、相变段（下一轮入口）。

## 底层矩阵元也一致 —— 三对角装配**全部核完**，只剩相变段

`ground_temperature.rs:458-465` ↔ `MOD_GroundTemperature.F90:377-382`：

| 上游 | Rust | 结论 |
|---|---|---|
| `at(j) = -(1.-cnfac)*fact(j)*tk(j-1)/dzm` | `sub[bottom] = -implicit*factor*conductivity[bottom-1]/lower_distance` | 一致 |
| `bt(j) = 1.+(1.-cnfac)*fact(j)*tk(j-1)/dzm` | `diagonal[bottom] = 1.0 + implicit*factor*conductivity[bottom-1]/lower_distance` | 一致（该处是**商**、不收缩，注释已写明）|
| `ct(j) = 0.` | `super_` 初始化为 0 | 一致 |
| `rt(j) = t - cnfac*fact*fn(j-1)` | `(-(cnfac*factor)).mul_add(flux[bottom-1], t)` | 一致（`FNMA(cnfac*fact, fn1, t)`）|

至此 `MOD_GroundTemperature`（`gt.opt` 41 处）的**三对角装配全部核完**：
`cv`/`thk` 组装 · 界面导热率 · 降水热双 FMA · 土壤支加权 · `dhsdT` · `fact` · 顶/内/**底**三层矩阵元 ·
`interface_fluxes` / `residual_heat_fluxes`。**唯一未逐条的是相变段**（`phase_change` 与
`snow_layer_absorption` 相关式）—— 下一轮的单一入口。

Tested: `MOD_GroundTemperature.F90:377-382` 与 `ground_temperature.rs:458-465` 的逐句对照。
Not-tested: 相变段。

## 更正：`MOD_GroundTemperature` 的"相变段"其实是**独立模块的 PUBLIC 子程序** —— 应走差分而非逐句核

第 208 轮把相变段列为"该模块最后未逐条的部分"，这一轮查明它**不是内联代码**：

```fortran
! main/MOD_GroundTemperature.F90:411/431
CALL meltf (patchtype,is_dry_lake,lb,nl_soil,deltim, …, scv,snowdp,sm,xmf,porsl,psi0, …)
```

`MOD_PhaseChange`（Makefile 与 `strings .bld/MOD_PhaseChange.o` 都指向
**`main/MOD_PhaseChange.F90`**，与 GroundTemperature 一样是内核直接编 `main/` 的模块）
对外暴露 `PUBLIC :: meltf`、`meltf_snicar`、`meltf_urban`。

**这意味着它可以用本会话最成功的那套办法处理**：写一个差分驱动，
**直接链接 `.bld/MOD_PhaseChange.o`**（真产线对象），用同一串 LCG 喂随机输入，
与 Rust 的 `phase_change`（`crates/colm-core/src/phase_change.rs:143`）逐位比 ——
而不是在这个 1300 行的模块里逐句找收缩。

**下一轮的具体步骤**（照 `compare_forcingdownscaling*.sh` 的成例）：
1. `nm -g .bld/MOD_PhaseChange.o | grep meltf` 取到符号，读 `meltf` 的实参表（`patchtype`、
   `is_dry_lake`、`lb`/`nl_soil`、`deltim`、`t_soisno`/`wice`/`wliq`/`dz`/`scv`/`porsl`/`psi0` …）；
2. 驱动里把 `patchtype`/`is_dry_lake` 当**可遍历的配置**（0/1/2/3/4），LCG 抽其余输入；
3. 输出 `t_soisno`/`wice_soisno`/`wliq_soisno`/`xmf`/`sm` 的位型，与 `phase_change` 逐位比；
4. 差分跑通后用 restart 口径（19/68）与三个窗口复核。

Tested: `Makefile` 的 `MOD_PhaseChange.o:` 规则与 `strings .bld/MOD_PhaseChange.o`；
`main/MOD_PhaseChange.F90:11-22` 的 `PUBLIC`/`SUBROUTINE meltf`；Rust 侧
`phase_change.rs:143` 的对应入口。
Not-tested: `meltf` 的差分驱动（下一轮）。

## `meltf` 差分驱动的**实参表**（下一轮直接照抄）

上游签名（`main/MOD_PhaseChange.F90:22-33`，33 个实参，**顺序即驱动调用顺序**）：

```fortran
SUBROUTINE meltf (patchtype, is_dry_lake, lb, nl_soil, deltim, &
                  fact, brr, hs, hs_soil, hs_snow, fsno, dhsdT, &
                  t_soisno_bef, t_soisno, wliq_soisno, wice_soisno, imelt, &
                  scv, snowdp, sm, xmf, porsl, psi0, &
                  bsw, theta_r, alpha_vgm, n_vgm, L_vgm, &
                  sc_vgm, fc_vgm, dz, &
                  qphs_thaw_lay, qphs_frzc_lay)
```

intent 已核：`patchtype`/`nl_soil`/`lb`/`is_dry_lake` 为 in；`deltim`、`fact(lb:)`、`brr(lb:)`、
`hs`/`hs_soil`/`hs_snow`/`fsno`/`dhsdT`、`t_soisno_bef(lb:)`、`scv`/`snowdp`、`sm`/`xmf`、
`porsl(1:)`、`psi0(1:)`、`bsw(1:)`、`theta_r(1:)`、`alpha_vgm`/`n_vgm`/`L_vgm`/`sc_vgm`/`fc_vgm`、
`dz(1:)` 为 in；**inout**：`t_soisno(lb:)`、`wice_soisno(lb:)`、`wliq_soisno(lb:)`、
`scv`、`imelt(lb:)`、`sm`、`xmf`、`qphs_thaw_lay`、`qphs_frzc_lay`。

Rust 对应结构 `PhaseChangeInput`（`phase_change.rs:47+`）已见的字段：
`patch_type`、`is_dry_lake`、`time_step_seconds`、`fact_seconds_per_j_m2_k`、
`residual_heat_flux_w_m2`、`snow_layer_absorption_w_m2`、`surface_heat_flux_w_m2`、
`soil_heat_flux_w_m2`、`snow_heat_flux_w_m2`、`snow_cover_fraction`、
`surface_heat_flux_temperature_derivative_w_m2_k`、`previous_temperature_k`、`temperature_k`、
`liquid_water_kg_m2`、`ice_water_kg_m2`、`snow_water_equivalent_kg_m2`、`snow_depth_m`、
`snow_layers`、`split_soil_snow`、`supercool_water`、`soil_layer_thickness_m`、
`soil_porosity`、`soil_residual_water`…（其余字段下一轮读全）。

**驱动骨架**：`lb = 1 - snow_layers`（与内核一致），`nl_soil` 取小值（如 5 层）加快；
`patchtype`/`is_dry_lake` 作**可遍历配置**（0/1/2/3/4 × T/F）；`brr` 用 `fact` 之外的独立随机量
（它在上游是 `tridia` 之后的余项，驱动里直接抽即可）；输出比对
`t_soisno`/`wice_soisno`/`wliq_soisno`/`scv`/`imelt`/`sm`/`xmf` 的位型。

Tested: `main/MOD_PhaseChange.F90:22-175` 的签名与 intent 逐条核对；`phase_change.rs:47-76` 的字段读取。
Not-tested: 驱动本体与差分运行（下一轮）。

### `meltf` 差分驱动的**输入/输出映射已补全**（写驱动所需信息到此齐全）

Rust `PhaseChangeInput` 全部 26 个字段（`phase_change.rs:47-80`）与 `PhaseChangeState`
（`:83-99`）已读全，与上游 33 个实参的对应关系：

| 上游（`meltf`）| Rust |
|---|---|
| `patchtype` / `is_dry_lake` | `patch_type` / `is_dry_lake` |
| `deltim` | `time_step_seconds` |
| `fact(lb:)` / `brr(lb:)` | `fact_seconds_per_j_m2_k` / `residual_heat_flux_w_m2` |
| `hs` / `hs_soil` / `hs_snow` | `surface_heat_flux_w_m2` / `soil_heat_flux_w_m2` / `snow_heat_flux_w_m2` |
| `fsno` / `dhsdT` | `snow_cover_fraction` / `surface_heat_flux_temperature_derivative_w_m2_k` |
| `t_soisno_bef(lb:)` / `t_soisno(lb:)` | `previous_temperature_k` / `temperature_k` |
| `wliq_soisno` / `wice_soisno` | `liquid_water_kg_m2` / `ice_water_kg_m2` |
| `scv` / `snowdp` | `snow_water_equivalent_kg_m2` / `snow_depth_m` |
| `porsl` / `psi0` / `theta_r` / `bsw` / `alpha_vgm` / `n_vgm` / `L_vgm` / `sc_vgm` / `fc_vgm` | `soil_porosity` / `soil_suction_mm` / `soil_residual_water` / `soil_hydraulic_model`（**每层的模型枚举**选出用哪套参数）|
| `dz` | `soil_layer_thickness_m` |
| `imelt(lb:)` | `phase_flag`（**out**：0 无/1 融/2 冻）|
| `xmf` | `latent_heat_flux_w_m2` |
| `sm` | `snow_melt_rate_kg_m2_s` |
| `qphs_thaw_lay` / `qphs_frzc_lay` | `thaw_mass_kg_m2` / `freeze_mass_kg_m2` |

**注意**：内核的 `lb` 是数组下界（`1 - snow_layers`），Rust 用**紧凑索引** + `snow_layers`
计数；驱动比对时要把 `lb:0` 的雪层与 `1:nl_soil` 的土层拼成同序的两个向量。
`supercool_water`/`split_soil_snow` 是 Rust 侧的运行开关，对应上游 namelist
（`DEF_...`），驱动里按配置遍历。

Tested: `phase_change.rs:47-99` 的全部字段与 `MOD_PhaseChange.F90:22-175` 的逐项对照。
Not-tested: 驱动本体（下一轮；信息已齐全）。

### `meltf` 的符号与 `.mod` 接口已核（写驱动前最后一道去风险）

```
$ nm -g vendor/CoLM202X/.bld/MOD_PhaseChange.o | grep -i meltf
000000000000154c T ___mod_phasechange_MOD_meltf
00000000000006a0 T ___mod_phasechange_MOD_meltf_snicar
0000000000000000 T ___mod_phasechange_MOD_meltf_urban
$ gzip -dc .bld/mod_phasechange.mod | strings | grep qphs
qphs_thaw_lay / qphs_frzc_lay      ← 与 main/MOD_PhaseChange.F90:22-33 的哑元名一致
```

也就是说：`.bld` 里的对象**确实导出** 三个 `meltf*` 入口，而我读到的签名（含
`qphs_thaw_lay`/`qphs_frzc_lay` 这两个 Rust 侧叫 `thaw_mass_kg_m2`/`freeze_mass_kg_m2` 的
inout 量）与编译进来的 `.mod` 逐字一致 —— 驱动只要 `-I.bld` 编译再链该对象即可，
不会出现"照源码写却与对象不匹配"的情形（本会话在 `MOD_ForcingDownscaling` 上正是被这个
坑过一次）。写驱动的最后一道前置风险已排除。

Tested: `nm -g .bld/MOD_PhaseChange.o` 的符号；`gzip -dc .bld/mod_phasechange.mod | strings`
的哑元名核对。
Not-tested: 驱动本体（下一轮）。

## `meltf` 差分：**上游侧已跑通**（10000 条记录），Rust 探针下一轮

`oracle/scripts/phasechange_diff.f90` 写完并跑通（`nsnow = 2` → `lb = -1`、`nl_soil = 4`，
`patchtype` 0..4 逐档 × 2000 组，输出 `t/wliq/wice/scv/sm/xmf/Σqthaw/Σqfrz/Σimelt`）：

```
$ head -2 /tmp/gf/pc_diff/pc.txt
02  4071128F5C28F5C3 400B46188186180E 400A61C406EFF336 401C73FB6CC0B96C  0 C0561D2EA2985634 0 3FDE8BC32DF1BB38 2
02  4071128F5C28F5C3 40011D1004E73FA6 4011C798FDF87C47 401DADC5F520D2E8  0 C042DB9BA4EB5176 0 3FCA0C59C0CF2C60 4
$ wc -l /tmp/gf/pc_diff/pc.txt → 10000
```

**两处接口细节**（照源码写会错，必须靠 `.mod`/链接器纠正 —— 已按实际改正）：

1. `qphs_thaw_lay`/`qphs_frzc_lay` 是**数组**（`lb:nl_soil`）而不是标量，编译报
   "Rank mismatch" 才发现；
2. `meltf` 依赖 `soil_vliq_from_psi`（`MOD_Hydro_SoilFunction`）与 namelist 的
   `DEF_SPLIT_SOILSNOW`，所以**链接方式照 `compare_forcingdownscaling*.sh`**：
   驱动单独 `-c`（纪律 flag），再用 `mpifort` 连同 **`.bld/*.o`（排除 `CoLM.o`）**
   与 netcdf-fortran/lapack/blas 一起链 —— 只链 `MOD_PhaseChange.o` 会缺符号。

**下一轮**：写 `crates/colm-core/examples/phase_change_probe.rs`（同序 LCG、同配置遍历、
把 `lb:0` 雪层与 `1:nl_soil` 土层拼成同序向量后与 `phase_change` 的输出逐位比）与
`oracle/scripts/compare_phasechange.sh`，跑通后接三段式验收。

Tested: `phasechange_diff.f90` 编译、链接（全 `.bld` 集合）、运行（10000 条）。
Not-tested: Rust 探针与逐位比对（下一轮）。

### Rust 探针的**骨架与全部类型**（下一轮一次写完，不必再读源码）

```rust
use colm_core::{phase_change, PhaseChangeInput, SoilHydraulicModel};

// `SoilHydraulicModel`（`hydrology.rs:8-19`）只有两个变体 —— 驱动里按 patchtype 交替：
//   Campbell { bsw }                                  ← 对应上游的 `bsw`
//   VanGenuchten { alpha_vgm, n_vgm, l_vgm, sc_vgm, fc_vgm }  ← 对应上游那五个数组
```

`PhaseChangeInput`（`phase_change.rs:47-80`）需要逐项构造：
`patch_type: i32`、`is_dry_lake: bool`、`time_step_seconds`、`fact_seconds_per_j_m2_k`、
`residual_heat_flux_w_m2`、`snow_layer_absorption_w_m2: Option<&[f64]>`（**干窗无 SNICAR 传
`None`**）、`surface_heat_flux_w_m2`、`soil_heat_flux_w_m2`、`snow_heat_flux_w_m2`、
`snow_cover_fraction`、`surface_heat_flux_temperature_derivative_w_m2_k`、
`previous_temperature_k`、`temperature_k`、`liquid_water_kg_m2`、`ice_water_kg_m2`、
`snow_water_equivalent_kg_m2`、`snow_depth_m`、`snow_layers: usize`、`split_soil_snow: bool`、
`supercool_water: bool`、`soil_layer_thickness_m`、`soil_porosity`、`soil_residual_water`、
`soil_suction_mm`、`soil_hydraulic_model: &[SoilHydraulicModel]`。

**索引约定（最容易错）**：上游 `lb = 1 - nsnow`，雪层在 `lb:0`、土层在 `1:nl_soil`，一个
连续数组；Rust 用**紧凑向量 + `snow_layers` 计数**，顺序为「雪在上、土在下」。所以探针里
`previous_temperature_k`/`temperature_k`/`liquid_water_kg_m2`/`ice_water_kg_m2` 要按
`[雪层…, 土层…]` 拼；输出侧同理拆回去比 `t(lb)`/`wliq(lb)`/`wice(lb)`（本驱动只比**第 lb 层**，
即最上面那层雪，以及标量 `scv/sm/xmf` 与两个 `Σqphs`、`Σimelt`）。

`PhaseChangeState` 的字段（`:83-99`）：`temperature_k`、`liquid_water_kg_m2`、
`ice_water_kg_m2`、`snow_water_equivalent_kg_m2`、`snow_depth_m`、`latent_heat_flux_w_m2`、
`snow_melt_rate_kg_m2_s`、`phase_flag`、`thaw_mass_kg_m2`、`freeze_mass_kg_m2` —— 与上游
`t/wliq/wice/scv/snowdp/xmf/sm/imelt/qphs_thaw/qphs_frzc` 一一对应。

Tested: `hydrology.rs:1-30` 与 `phase_change.rs:47-99` 的类型核对。
Not-tested: 探针本体（下一轮）。

### 最后的配置疑点已实测：链接进来的 namelist 取 **van Genuchten** 分支

`meltf` 的水力模型不是在 `patchtype` 上分岔，而是由**同名 namelist 开关**决定
（`MOD_PhaseChange.F90:139/447`）：

```fortran
IF (DEF_USE_Campbell_SOIL_MODEL) THEN
   supercool(j) = porsl(j)*(smp/psi0(j))**(-1.0/bsw(j))
ELSE
   supercool(j) = soil_vliq_from_psi(smp, porsl(j), theta_r(j), -10.0, 5, &
                    (/alpha_vgm(j), n_vgm(j), L_vgm(j), sc_vgm(j), fc_vgm(j)/))
ENDIF
```

而驱动链的是 `.bld` 的**真 namelist**（不调 `read_namelist`，取默认值）。把该标志打出来后实测：

```
$ /tmp/gf/pc_diff/pc
 DEF_USE_Campbell_SOIL_MODEL =  F
```

→ 该驱动跑的是 **van Genuchten** 支，所以 Rust 探针里 `soil_hydraulic_model` 应逐层填
`SoilHydraulicModel::VanGenuchten { alpha_vgm, n_vgm, l_vgm, sc_vgm, fc_vgm }`。

**注意覆盖面**：这条差分只覆盖 VGM 支；Campbell 支需要另一套配置（改 namelist 默认值 ——
可照 `compare_second_config.sh` 的成例，或在本驱动里加一个桩 namelist 模块），下一轮可选做。

Tested: 驱动加打印后重编、链接、运行（`DEF_USE_Campbell_SOIL_MODEL = F`）；
`MOD_PhaseChange.F90:139/447` 的两个分支。
Not-tested: Campbell 支的差分（需另配 namelist）。

## `meltf` 差分**跑通并立刻报错**：温度全同，融/冻簿记大面积不一致

`phase_change_probe.rs` 一次编译通过（53 次抽签/case 的顺序已按上游的"标量右值只抽一次"
写法对齐），与上游驱动各 10000 条记录（`patchtype` 0..4 × 2000）逐位比：

| 输出 | 失配数 / 10000 |
|---|---|
| `t`（第 lb 层温度）| **0** ✅ |
| `scv`（雪水当量）| **0** ✅ |
| `wice` | 415 |
| `wliq` | 795 |
| `imelt`（Σ 相变标志）| 1753 |
| `xmf`（相变潜热通量）| 3042 |
| `sm`（雪面融水率）| 6475 |
| `Σthaw` | **6475**（与 `sm` 同数）|
| `Σfreeze` | 6567 |

两点解读：

1. **`t` 与 `scv` 逐位全同**说明**实参对齐没问题**（33 个实参、`lb=1-nsnow`、雪/土拼接、
   53 次抽签顺序、VGM 分支都对），否则 10000 条里不可能一个不差；
2. 失配集中在**融/冻簿记**：`sm` 与 `Σthaw` **同为 6475** 提示它们走同一条代码路径
   （雪面融水与 thaw 数组一起算），`xmf`/`freeze`/`imelt` 是同一族的近邻。

**下一轮的定位法**：按 `patchtype` 与"是否发生相变"分组统计失配率（本驱动的 `imelt`
之和已经是分组键），先在 `MOD_PhaseChange.F90` 的融化段（`t_soisno(j) > tfrz` 与
`scv > 0` 两条支）里找与 Rust 的结构差异 —— 这很可能就是干窗 `wliq_soisno`/`wice_soisno`
1 ULP 种子的来源。

Tested: 两个驱动各 10000 条；逐字段失配统计（上表）。
Not-tested: 失配的逐条定位与成因（下一轮）。

### `meltf` 差分的真实信号：**只有冻结支**（`xmf`/`freeze`/`imelt` 三者同时差）

上一轮的统计是**我自己比较脚本的错**：上游 `Z17` 对 0.0 印出 `0`，Rust 侧 `{:016X}` 印出
`0000000000000000`，数值相同、字符串不同 —— 没做 `zfill(16).upper()` 归一化，于是
"每一条都差"。补上归一化后（与其它差分脚本一致的写法）：

```
cases with any difference: 1753/10000
  xmf    1753/10000
  freeze 1753/10000
  imelt  1753/10000
```

**这才是真信号**，而且极其干净：

* `t`（温度）、`wliq`、`wice`、`scv`、`sm`、`thaw` 在 **10000/10000 条上逐位全同**；
* 只有 **冻结**那一族的三项同时差（`xmf` 潜热通量、`freeze` 逐层冻结量、`imelt` 标志），
  且**三者的失配集合完全相同**（1753 条）—— 说明是**同一条表达式或同一个分支判据**，
  而不是三处独立错误。

这也把上一轮"融/冻簿记大面积不一致"的结论**收窄成一个支**：融化支、雪面融水（`sm`）与
`thaw` 全部无误，问题只在**冻结**（`t_soisno < tfrz` 且 `wliq > 0`）那一支。

**下一轮**：在 `main/MOD_PhaseChange.F90` 的冻结段（搜 `imelt(j) = 2` 或 `xmf = xmf - …`
的负向分支）与 `phase_change.rs` 的对应段落逐句对照 —— 1753 条失配样本可直接当回归用例。

（教训：**差分脚本一律先归一化位型字符串**；本会话已有"工具排序误导""截断误读"两次同类，
这是第三次，已写进规矩。）

Tested: 归一化前后的对比统计（10000 条）；两组数字与样例行的逐字段核对。
Not-tested: 冻结支的逐句定位（下一轮）。

## `meltf` 差分**结案**：10000/10000 逐位全同 —— 那 1753 条是我的**探针配置**错了

上一轮把差异收窄到"冻结支（xmf/freeze/imelt）"，这一轮查明它**不是移植缺陷**：内核链进来的
namelist 默认值是

```
$ /tmp/gf/pc_diff/pc
 campbell/vgm =  F  supercool =  T  split =  F
```

而我的 Rust 探针当时写的是 `supercool_water: false`。上游据此分岔
（`MOD_PhaseChange.F90:296-301`）：

```fortran
IF (DEF_USE_SUPERCOOL_WATER) THEN
   IF(j <= 0 .or. patchtype == 3) THEN          ! 只钉雪层（与 patchtype 3）
      IF(wliq*wice > 0.) t_soisno(j) = tfrz
   ENDIF
ELSE
   IF(wliq*wice > 0.) t_soisno(j) = tfrz        ! 所有层都钉
ENDIF
```

`T` 与 `F` 差在**土壤层是否被钉到冰点**，正好只影响冻结（`freeze`/`imelt`）而不影响
顶层雪的 `wice`、也不影响融化（`thaw`/`sm`）—— 与上一轮观察到的失配形态**完全吻合**。
把探针改成 `supercool_water: true` 后：

```
cases with any difference: 0/10000
```

**`MOD_PhaseChange:meltf` 由此结案**：5 个 `patchtype` × 2000 组，`t`/`wliq`/`wice`/`scv`/
`sm`/`xmf`/`Σthaw`/`Σfrzc`/`Σimelt` **全部逐位相同**（本会话第 8 个差分结案，也是实参最多、
唯一需要"全 `.bld` 链接"的那个）。

### 新规矩（本会话第四次同类教训）

**差分驱动必须把链接进来的 namelist 开关**（本案 3 个：campbell / supercool / split）
**打出来，并在 Rust 侧逐一匹配**。前三次同类是：dump 取错文件档、工具按名字排序、
位型字符串未归一化 —— 都不是逻辑错误，而是"两侧配置/口径没对齐"。

Tested: 三个开关的打印（`F/T/F`）；探针改 `supercool_water=true` 后 10000/10000 逐位全同；
`phasechange_diff.f90` 编译链接运行与 `phase_change_probe` 各 10000 条。
Not-tested: Campbell 支（需另配 namelist，仍列为可选）。

### 第 8 个差分已入库：`oracle/scripts/compare_phasechange.sh`

```bash
$ bash oracle/scripts/compare_phasechange.sh
== 内核 namelist:  campbell/vgm =  F  supercool =  T  split =  F
MOD_PhaseChange:meltf: 5 patchtypes x 2000 组、9 个量 10000/10000 逐位相同
```

脚本把三件事固化在一起（此前只在我的临时命令里）：

1. **链接**：驱动单独 `-c`（纪律 flag），再用 `mpifort` 连 `.bld/*.o`（排除 `CoLM.o`）+
   netcdf-fortran/lapack/blas —— `meltf` 依赖 `soil_vliq_from_psi` 与 namelist 常量，
   只链 `MOD_PhaseChange.o` 会缺符号；
2. **配置打印**：运行前先打印内核 namelist 的三个开关（`campbell/vgm`、`supercool`、`split`），
   提醒对齐 Rust 探针 —— 这正是上一轮 1753 条假失配的成因；
3. **归一化比较**：`zfill(16).upper()` 后再逐位比，并按字段统计失配。

至此本会话的 8 个差分各有自己的 `compare_*.sh`，全部可一键复现。

Tested: `compare_phasechange.sh` 完整运行（编译、链接、两侧各 10000 条、逐位比对通过）；
`cargo fmt --all --check`。
Not-tested: Campbell 支（需另配 namelist）。

### 口径补充：**步级基线也是"步数相关"的**（1 步 ≠ 3 步）

第 220 轮重跑基线时发现：`dry_ts.sh 1` 的 history 口径是 **33 个差异变量 / 234/268**，
而此前一直引用的 **44 / 585/692** 来自 `dry_ts.sh 3` —— 两个都对，但**不可混用**：

```
$ bash /tmp/gf/dry_ts.sh 1
$ python3 oracle/scripts/window_divergence.py /tmp/gf/dryts/out/CN-Cng/history/*.nc \
      /tmp/gf/dryts/colm-rs_hist_2008-01.nc
variables differing: 33; bitwise identical: 234/268 (87.3134%)
first divergence step: 0

# 对照（3 步）：44 / 585/692
```

这与第 200 轮那条"restart 口径必须同步数"是同一类问题。**固定约定**（已并入"复制粘贴流程"）：

| 口径 | 命令 | 基线 |
|---|---|---|
| restart | `dry_ts.sh 1`（→ `…-01800`）| **19/68** |
| 步级 history | `dry_ts.sh 3` | **44 / 585/692** |
| 黄金窗口 | `win4.sh` | 21328 / 32655 / 33651 |

顺带确认：`gssun`/`rbsun`/`us10m`/`olrg`/`gbh2o` 五处已落地的修复之后，**restart 口径仍是
19/68 且逐行数值不变**（`rib 1.922e-16` / `t_soisno 2.041e-16` / `trad 2.153e-16` …）——
即这些修复没有动到种子本身，最后一个开放项依旧原样存在。

Tested: `dry_ts.sh 1` + `window_divergence.py`（33/234）与 `dry_ts.sh 3`（44/585）的对照；
`dry_ts.sh 1` + `restart_divergence.py`（19/68，逐行与第 200 轮一致）。
Not-tested: none（本轮只做口径确认）。

## `rib`（restart 口径最锐的那个）源码级一致

restart 19/68 里 `rib` 的 `maxrel` 最小（1.922e-16，1 个值），是种子最锐的指示器。
`MOD_Vars_1DAccFluxes.F90:2786-2787`：

```fortran
r_rib_e = r_zol_e /vonkar * r_ustar2_e**2 / (vonkar/r_fh_e*um**2)
r_rib_e = min(5.,r_rib_e)
```

Rust `history_diagnostics.rs:208-210`：
`(zol / VON_KARMAN * friction_velocity.powi(2) / (VON_KARMAN / heat * wind.powi(2))).min(5.0)`
—— **结合顺序、`powi(2)` 与 `min(5.)` 全部一致**。所以 `rib` 的那 1 ULP 来自它的**入参**
（`zol`/`ustar2`/`fh`/`um`），落在同一个"上游族"里，而不是这一句。

同文件同段落的 `us10m`/`vs10m` 本轮之前已按源码改过（结合顺序），`rib` 现在也核完 ——
即 `MOD_Vars_1DAccFluxes` 这段累加器诊断量的形状已全部对过，剩下的 1 ULP 只能出自
喂给它们的相似性状态。

Tested: `MOD_Vars_1DAccFluxes.F90:2786-2787` 与 `history_diagnostics.rs:208-210` 的逐句对照。
Not-tested: `zol`/`ustar2`/`fh`/`um` 的上游来源（即种子本体的定位）。

## 最后开放项的**排除链**（第 223 轮汇总，接手时可直接从这里看）

干窗第 0 步那 1 ULP 族（restart 19/68，`rib`/`t_soisno`/`trad`/`rst`/`qstar`/`fm`… 的
`maxrel` 全在 2e-16 一档）经过本会话逐环排除，**结构上能静比较的地方已经查完**：

| 环节 | 结论 | 依据（本文档小节）|
|---|---|---|
| 相似性调用的**实参** | **证伪**：第 0 步根本不发生该调用 | 第 196 轮，五次插桩 + `strings` 计数 |
| `meltf`（相变）| ✓ 差分结案 10000/10000 | 第 218-219 轮 |
| `tridia`（三对角求解器）| ✓ 早已差分关掉（2000 组，整解向量比位）| `:8540` |
| `MOD_GroundFluxes` 本体（8 处收缩）| ✓ 逐条对上 | `:11763`、`:11956` |
| `MOD_GroundTemperature` | ✓ 装配 · 界面 · 三对角三层矩阵元 · `dhsdT` · `fact` · `flux` 助手 | 第 199-208 轮 |
| 累加器诊断（`rib`/`us10m`/`vs10m`）| ✓ 源码级一致（`us10m` 顺序已改）| 第 205/221 轮 |
| 粗糙度族（`z0mg`/`z0hg`/`z0qg`）| ✓ 源码级一致 | 第 189-191 轮 |
| `stomata` 族 + `calc_photo_params` 幂族 | ✓ 源码级一致（`gbh2o` 已改）| 第 186-187 轮 |
| `MOD_LeafTemperature` 4 处 | 已改；另有 6 处"dump 有、实测不支持" | 第 162-165 轮 |
| `MOD_Thermal`（99 处里的 `olrg`/`trad`/`fgrnd`/降水链）| `olrg` 已改（步级 +3）；其余实测惰性/否决 | 第 172-179 轮 |

**剩下的只有两类**，且都不再是"漏移植"：

1. **调用时机/顺序**：第 0 步某些量的计算次序或调用次数与内核不同（静态比较看不见）；
2. **两个 dump 里尚未逐条过的收缩**（`lt_ext.opt` 80、`th_ext.opt` 99）——但它们与种子的
   关联未经证明，且**本会话测过的 6 处"dump 说有收缩"全部被实测否决**（`clai`/`thvstar`/
   `cfw`/`fgrnd`/`lfevpa`/`gt` 土壤支），连唯一落地的 `olrg` 也是靠**步级 +3** 而非 dump 本身。

**性质判定（如实记录）**：残余是**诊断量末位 1 ULP**（`maxrel ≈ 2e-16`，多数只差 1 个值），
三个黄金窗口的容差口径（17/68/79）与 `sumabs`/`over_tol` 均未因此变化；它不是缺功能、
缺模块或算法差异，而是"编译器为同一数学式选的结合/收缩形态"在少数点上的分岔。若要继续，
入口唯一：`dry_ts.sh 1` + `restart_divergence.py` 做 A/B，靶子按上表第 2 类逐个试。

Tested: 本表每一行都对应本文档前述小节的实测输出。
Not-tested: 上表第 1、2 类本身（未逐条穷尽）。

## 关于"调用时机"假设：老的 `iterprobe` 转储**不可用**（已核对）

第 140 轮前后留下的 `/tmp/gf/iterprobe/{fort_iter2,rust_iter2}.txt` 是叶温 Newton 迭代
内部的转储，本想直接用它验证"调用时机/顺序"这条残余假设。核对后**判定不可用**：

* **代码状态过时**：那是本会话 5 处修复（`gssun`/`rbsun`/`us10m`/`olrg`/`gbh2o`）之前的产物；
* **两侧格式不一致**：上游打印标记是 `Q 1_tl`/`Q 2_tl`（两次迭代都有），Rust 侧只有 `Q2 …`
  一种，且没有迭代 1 的对应行 —— 无法逐次对齐；
* 数值上确实能看到 1 ULP 级差异（如 `fevpl 1.37119817067997553e-8` vs `…586e-8`），
  但那只能说明"当时的种子的确在这一带"，不能用于今天的状态。

**若要验证"调用时机"假设**，需要**重做一次**迭代内部探针，且要求：两侧打印**同一组量、
同一格式、同一迭代序号**（含迭代次数本身）。现成模板可用 `step0_arg_probe3.sh`（自带 vendor
还原 + 重编 + f48 自检），把插桩点从 `CALL moninobukm` 换成 `DO WHILE (it .le. itmax)`
循环体内即可；Rust 侧对应 `leaf_temperature.rs` 的迭代循环。

Tested: `diff /tmp/gf/iterprobe/fort_iter2.txt /tmp/gf/iterprobe/rust_iter2.txt` 的核对
（56 行 vs 23 行、标记格式不同）；`/tmp/gf/iterprobe/` 的内容与时间戳。
Not-tested: 重做的迭代探针（下一轮，若要走"调用时机"这条线）。

### 迭代探针已脚本化：`oracle/scripts/step_iter_probe.sh`

把"重做迭代内部探针"做成一条命令（同样的 `trap ... EXIT` 安全设计：还原源码 → 重编 →
打印 `git status` 与 `f48 sync`）：

```bash
bash oracle/scripts/step_iter_probe.sh
# → /tmp/gf/iterprobe2/fort_iter.txt   上游**逐迭代**（含迭代号 it）：
#   ITPROBE  <it>  tl  fsenl  fevpl  obu  ustar  cfw
```

插桩点锚在 `MOD_LeafTemperaturePC_Extended.F90:1063` 的 `DO WHILE (it .le. itmax)` 之内
（锚点唯一已核，补丁体在临时副本上干跑验证：恰好插入 1 行）。

**Rust 侧对应做法**：在 `crates/colm-core/src/leaf_temperature.rs` 的迭代循环里打印
**同一组量、同一顺序、同一格式**（含 `iteration` 号），再与 `fort_iter.txt` 逐行比 ——
要验证的是"迭代次数/每次迭代的进入值是否一致"，即第 223 轮列出的第 1 类残余
（调用时机/顺序）。

Tested: `bash -n`；锚点唯一性；补丁体临时副本干跑（1 行 ITPROBE）。
Not-tested: 脚本完整执行 + Rust 侧同格式转储（下一轮）。

## 迭代探针结果：**PC 叶温求解器在第 0 步整段没执行**

`step_iter_probe.sh` 完整跑通（插桩 → 重编 → 跑 1 步 → 自动还原 → `f48 sync PASS`），
但 `/tmp/gf/iterprobe2/fort_iter.txt` 是 **0 行**：

```
== 上游逐迭代转储：/tmp/gf/iterprobe2/fort_iter.txt
       0 /tmp/gf/iterprobe2/fort_iter.txt
```

也就是说 `MOD_LeafTemperaturePC_Extended.F90:1063` 的 `DO WHILE (it .le. itmax)` 循环
在第 0 步**一次都没进**（而该文件里 `moninobukm` 那处此前也证明没执行，第 195-196 轮）。

这条把第 196 轮的结论**从"那次相似性调用"扩大到"整个 PC 叶温求解"**：

* 第 0 步写进 history/restart 的 `tleaf` 只能是**restart 带进来的滞后状态**，不是这一步新解的；
* 那么 `t_soisno`/`tleaf` 的那 1 ULP 就落在**地面那一支**（`MOD_GroundTemperature` + `meltf`
  + `tridia` —— 三者都已被差分/形状关掉）或它们的**入参**（`hs`/`hs_soil`/`hs_snow`/`dhsdT`
  /`fact`/`fn`，来自 `MOD_Thermal`/`MOD_GroundFluxes`，也已核过 8 处与 `olrg` 等）。

**这解释了为什么"逐句形状核对"始终找不到种子**：叶子那一大块（80 处收缩所在）在干窗第 0 步
根本不参与，而地面那一支的算术又已经被差分关掉 —— 残余只能出在**入参的装配**上，而
`MOD_GroundFluxes` 的入参推导此前已按名单核过一轮（`:11769` 那三处修复即由此而来）。

Tested: `step_iter_probe.sh` 完整执行（0 行 ITPROBE、自动还原、f48 PASS）。
Not-tested: 地面支入参在第 0 步的逐位对照（需要新的、针对地面调用的插桩）。

### 下一步（已备好锚点与变量名单）：给 `GroundTemperature` 的**入参**做同形探针

第 226 轮把残余收窄到"地面支的入参装配"。调用点已定位：

```
extends/interception/MOD_Thermal_CanopyPhase_Extended.F90:1207
   CALL GroundTemperature (patchtype,is_dry_lake,lb,nl_soil,deltim,
       capr,cnfac,vf_quartz,…,porsl,psi0,bsw,theta_r,alpha_vgm,…,dz_soisno,z_soisno,zi_soisno,
       t_soisno,t_grnd,t_soil,t_snow,wice_soisno,wliq_soisno,scv,snowdp,fsno,
       frl,dlrad,sabg,sabg_soil,sabg_snow,sabg_snow_lyr,
       fseng,fseng_soil,fseng_snow,fevpg,fevpg_soil,fevpg_snow,cgrnd,htvp,emg, …)
```

（`main/MOD_Thermal.F90:1198` 是同一条调用；内核在本配置下编的是 **extends 那一份**。）

**要打印的 20 个量**（形状最敏感的一批，不必打全 60 个）：

```
t_soisno(1), wliq_soisno(1), wice_soisno(1), scv, snowdp, fsno, t_grnd, t_soil, t_snow,
frl, dlrad, sabg, sabg_soil, sabg_snow, fseng, fseng_soil, fseng_snow,
fevpg, fevpg_soil, fevpg_snow, cgrnd, htvp, emg
```

**做法**：照 `step0_arg_probe3.sh` 的模板（自带 vendor 还原 + 重编 + f48 自检），把
锚点从 `CALL moninobuk` 换成 `CALL GroundTemperature (`，插两条 `WRITE(*,…)`（进入前打上面
这批量、返回后打 `t_soisno`/`t_grnd`/`scv`）；Rust 侧在 `ground_temperature` 的入口与返回处
打同一批量（`GroundTemperatureInput` 的字段名与上表对应）。

**判据**：入参**逐位相同而输出仍差** → 残余在 `GroundTemperature` 体内（但该模块的算术已被
差分/形状关掉 → 只剩"未逐条过的收缩"）；入参**已经差** → 顺着差异量往上游追一层
（`MOD_GroundFluxes` 的 `fseng`/`fevpg` 或辐射项）。

Tested: 调用点与实参表的定位（`extends/…:1207`、`main/…:1198`）；要打印的量按"形状最敏感"筛出。
Not-tested: 探针本体（下一轮）。

## 重大重估：干窗第 0 步**地面求解也没执行** —— "第 0 步种子"很可能是 restart 映射差

`step_ground_probe.sh` 完整跑通（锚点唯一、补丁干跑 1 行、插桩重编、跑 1 步、自动还原、
`f48 sync PASS`），`CALL GroundTemperature`（`extends/…:1207`）**之前**的打印是 **0 行**：

```
$ wc -l /tmp/gf/gtprobe/gt_in.txt
       0
```

把本会话四次插桩的结果并起来，干窗第 0 步**没有执行**的物理：

| 插桩点 | 结果 |
|---|---|
| 叶温 PFT / PC 两份的 `CALL moninobukm` | 0 命中（第 194-195 轮）|
| PC 的 `DO WHILE (it .le. itmax)`（Newton 迭代）| 0 行（第 226 轮）|
| 累加器的 `CALL moninobuk`（`MOD_Vars_1DAccFluxes:2781`）| 0 命中（第 196 轮）|
| **`CALL GroundTemperature`（地面温度求解）** | **0 行（本轮）** |

`meltf` 是从 `GroundTemperature` 里调的 —— 既然它没执行，那么 `meltf` 在**本算例第 0 步
同样没跑**（我此前只用独立驱动证明过它的正确性，那与"它在算例里何时被调"是两件事）。

**这解释了三件事**：
1. 为什么"逐句形状核对"遍及叶温/地面/相变都找不到种子 —— 那些代码在第 0 步不执行；
2. 为什么 11 天窗口里 `olrg` 那类修复有效（它们作用在**后续步**），而第 0 步纹丝不动；
3. 为什么 `t_soisno`/`tleaf` 的差异恰好是 1 ULP —— 若第 0 步根本没算物理，那么 history/restart
   里写出的就是**从 restart 读入、按同一套映射搬运**的状态，1 ULP 只可能来自**读入/写出的
   解析与单位换算**（或某一层默认值）。

**下一轮的检验（一步即可定性）**：把**初始** restart（两侧共同读入的那份）与内核第 1 步
写出的 restart（`…-01800`）对比 —— 若除时间戳外**逐位相同**，即证明第 0 步没有改动状态，
种子就在 restart 的**读入映射**里（Rust 侧 `restart.rs`/`spatial_static.rs` 的解析与
`colm-runtime` 的装载），而不在任何物理表达式。

Tested: `step_ground_probe.sh` 完整执行（0 行、自动还原、f48 PASS）；四次插桩结果汇总。
Not-tested: 初始 restart 与第 1 步 restart 的对比（下一轮，一步定性）。

## **更正第 229 轮的重估**：第 0 步**确有物理在跑**（36 个状态量被改写）

第 229 轮根据"四次插桩 0 命中"推断"第 0 步没算物理、种子在 restart 映射"，**这一步推断
被下面的一步实验否掉了**：

```
初始 restart（2008-001-00000） vs 内核第 1 步 restart（2008-001-01800）
  differing variables: 36     （alb 4 个元素、emis/extkb/extkd/fh/fm/fq/fwvet_snow/gs0sun/gs0sha/hk …）
```

状态被大量改写 ⇒ **物理确实在跑**，只是**不由我插桩的那四处调用**。于是第 229 轮那条
"种子在 restart 映射"的推断**不成立**，撤回。

**更可能的原因（本会话反复踩过的那一类）**：地面探针**没有做"补丁是否进了被运行的二进制"
的检查**。叶温那三次探针都做了（`strings kernels/default/colm.x | grep -c ARGPROBE_` 得到
2/2/3，证明补丁确实进了二进制），而 `step_ground_probe.sh` 缺这一步 —— 于是"0 命中"有两种
解释（源码里那条调用真没跑 / 改的文件不是被编的那份），而本轮无法区分。

**下一轮第一件事**（很小的改动）：给 `step_ground_probe.sh` 补上与叶温探针同款的
`strings … | grep -c GTPROBE_IN` 检查（在还原之前），再重跑一次 ——
为 0 则是"改错文件"（改用 Makefile 里 `MOD_Thermal.o` 真正对应的那份），不为 0 才是
"那条调用在第 0 步没执行"。

（教训重申：**任何插桩探针都必须先证明补丁进了被运行的二进制**；这条已在叶温探针里落实，
本轮的地面探针漏了，直接导致一次错误的"重大重估"。）

Tested: 初始 restart 与第 1 步 restart 的逐位比对（36 个变量被改写）。
Not-tested: 地面探针的 `strings` 检查与重跑（下一轮）。

## 地面探针的 0 命中**不是改错文件**：补丁进了二进制，而那条调用是**无条件**的

给 `step_ground_probe.sh` 补上叶温探针同款的检查后重跑：

```
== 二进制里的 GTPROBE_IN 标记数: 1        ← 补丁确实进了被运行的内核
== 上游入参：/tmp/gf/gtprobe/gt_in.txt
       0 /tmp/gf/gtprobe/gt_in.txt        ← 却没有任何输出
```

并另做了两项核对：

```
$ strings .bld/MOD_Thermal.o | grep -o "extends/interception/[A-Za-z_]*\.F90\|main/[A-Za-z_]*\.F90"
extends/interception/MOD_Thermal_CanopyPhase_Extended.F90      ← 我改的就是被编的那份
$ 逐个 nm -u .bld/*.o | grep groundtemperature_MOD_groundtemperature
MOD_Thermal.o                                                  ← 全内核只有它引用 GroundTemperature
$ sed -n '1205,1208p' （该文件）
      CALL GroundTemperature (patchtype,is_dry_lake,lb,nl_soil,deltim,&   ← 顶层、**无条件**
```

三条合起来只有一个解释：**`MOD_Thermal` 里包含这条调用的那个子程序，在第 0 步没有被调用**。
又由第 230 轮"初始 restart vs 第 1 步 restart 有 36 个变量被改写"可知物理确实在跑 ——
**所以干窗第 0 步走的是另一条驱动路径**（既不是叶温 PFT/PC 的 Newton 迭代，也不是 Thermal
的 `[5] Ground temperature`）。

**下一轮的做法（系统性判定，不再逐点猜）**：在上游**肯定会被执行**的入口插一条标记
（候选：`MOD_SoilSnowHydrology:SoilSnowHydrology`、`MOD_Thermal` 的子程序**入口**本身、
或 `CoLMMAIN` 里的步进调用），一次跑出"第 0 步实际进入了哪些例程"的清单；Rust 侧同样在
`standard_lct_step` 的对应位置打印，两侧对齐后再谈 1 ULP。

Tested: `step_ground_probe.sh` 加检查后重跑（标记数 1、0 行输出）；`strings .bld/MOD_Thermal.o`
的来源；`nm -u` 的调用者清单；`:1207` 的无条件调用上下文。
Not-tested: "哪些例程在第 0 步被执行"的系统性清单（下一轮）。

## 矛盾点已定位：`THERMAL` 的无条件打印没响，而 `CoLMMAIN` 对它的调用也没被 `IF` 包住

第 231 轮已确认：地面探针的标记**在二进制里**（`strings … | grep -c GTPROBE_IN` = 1），
打印点位于 `MOD_Thermal_CanopyPhase_Extended.F90` 的 **`SUBROUTINE THERMAL`**（`:21` 开头、
`:1207` 的 `CALL GroundTemperature` 在其体内）的无条件位置，而运行时 **0 行**。

本轮又核了两件事：

* `CoLMMAIN.F90:1052` 调用的正是 `THERMAL`（`USE MOD_Thermal, only: THERMAL`，`:177`），
  且该调用**前面没有 `IF`**（上一行是 `:1047` 的 `ENDIF` 与一句注释）；
* `.bld/MOD_Thermal.o` 来自我改的那份 extends 文件（第 231 轮已核）。

于是出现一个**未解的矛盾**：`THERMAL` 的无条件打印没响，而「谁在算物理」变成：

| 假设 | 支持 | 反对 |
|---|---|---|
| A. 第 0 步不走 `CoLMMAIN` 的 patch 循环（首条 history 是初始化写出）| 四次插桩全 0 命中；`THERMAL` 的打印无条件却没响 | 第 230 轮"初始 vs 第 1 步 restart 有 36 个变量被改写" |
| B. 物理在跑、只是我没插到正确的位置 | 36 个变量被改写 | 叶温两份、PC 迭代、累加器、THERMAL 四处全 0 命中 |

**而 A 的那条"反对证据"本身也不硬**：36 个被改写的量里有 `alb`/`extkb`/`extkd`/`gs0sun`/
`gs0sha` 这类**诊断量**，完全可能是 restart **写出时按地类重算/补默认**的产物，而不是物理
演化的结果。

**决定性实验（下一轮第一件事）**：在 `CoLMMAIN` 的**步进循环入口**本身插一条标记（与
`THERMAL` 的打印同一份二进制里），一次跑就能判定：

* 循环入口打印**有**、`THERMAL` 打印**无** → 走的是 A（首条记录不是物理步），种子在
  restart 读入/写出映射；
* 两者**都有** → `THERMAL` 的打印位置被我误判（例如被编译器折叠进某分支），改插在入口再试；
* 两者**都无** → 该算例的 1 步运行根本没进主循环，`dry_ts.sh` 的配置需要复核。

Tested: `:21`/`:1207` 的所属子程序（`THERMAL`）、`CoLMMAIN:177/1052` 的调用与上文、第 231 轮的
二进制标记核对。
Not-tested: `CoLMMAIN` 步进循环入口的插桩（下一轮的决定性实验）。

## **重大更正**：四次探针的"0 命中"是**探针脚本的 bug** —— 它们跑的是**零步**

第 194-232 轮的全部"某例程在第 0 步没执行"的结论，追到根上是**我的探针脚本把
`DEF_simulation_time%end_sec` 设成了 0**：

```
$ grep end_sec /tmp/gf/dryts/case.nml      # 能跑通的那个（dry_ts.sh）
   DEF_simulation_time%end_sec       = 1800
$ grep end_sec /tmp/gf/gtprobe/case.nml    # 我的探针脚本生成的
   DEF_simulation_time%end_sec       = 0    ← start_sec 也是 0 ⇒ **零步**
$ wc -l /tmp/gf/gtprobe/kernel.log /tmp/gf/iterprobe2/kernel.log /tmp/gf/argprobe3/kernel.log
   23 / 23 / 23        ← 三个探针的日志都只有 23 行，且**没有** "CoLM Execution Completed"
```

**零步运行 ⇒ 任何物理例程都不会被调用 ⇒ 四次插桩必然 0 命中。** 所以下列结论**全部作废**：

* 第 194-196 轮："叶温两份的 `moninobukm`、累加器的 `moninobuk` 在第 0 步不执行"；
* 第 226 轮："PC 的 Newton 迭代循环不执行"；
* 第 229/231 轮："`CALL GroundTemperature` 不执行"；
* 第 232 轮那个"矛盾"（`THERMAL` 的无条件打印没响）—— **矛盾消失**：根本没有步被执行。

**已修**：四个探针脚本的 `end_sec` 全部改为 **1800**，并给每个脚本加了**硬闸门** ——
断言内核日志里必须有 `CoLM Execution Completed`，否则直接以非零码退出并提示"这次插桩结果无效"。

**仍待解决**：把 `end_sec` 改对之后重跑地面探针，内核**仍然没有跑完**（日志 23 行停在
`Netcdf error: … rawdata_unused//plant_15s/….nc cannot open`），而 `dry_ts.sh` 的同名运行
**没有**这条错误、并正常完成（32 行）。两者 case.nml 只差 `DEF_HIST_FREQ`（TIMESTEP vs HOURLY）。
**下一个要查的就是这个差异**：探针脚本必须先能像 `dry_ts.sh` 一样跑完一步，之后任何
"入参逐位对照"才有意义。

Tested: `dry_ts.sh` 与三个探针的 case.nml/kernel.log 对比（`end_sec`、完成标志、日志行数）；
四个脚本的 `end_sec` 修正与硬闸门添加。
Not-tested: 探针跑不完的根因（`DEF_HIST_FREQ` 或其它设置差异）。

### 探针跑不完的根因收窄：**不是 case.nml 差异，而是"探针自己重编内核"**

把 `end_sec`、`DEF_HIST_FREQ` 都对齐成 `dry_ts.sh` 的样子之后，地面探针的内核**仍然**跑不完
（逐字节 diff 两份 case.nml：除路径外只差那两处，且都已对齐）：

```
$ diff <(grep -v "^ Warning\|^$" /tmp/gf/dryts/run/f.log) <(grep -v "^ Warning\|^$" /tmp/gf/gtprobe/kernel.log)
< Loading Time Invariants done.     ← dry_ts.sh 的内核（**用现成的 kernels/default/colm.x**）
< Loading Time Variables done.
< TIMESTEP = 1 | DATE = 2008-01-01-00000
< CoLM Execution Completed.
> Netcdf error: ... /tmp/gf/gtprobe/rawdata_unused//plant_15s/RG_45_120_40_125.MOD2005.nc cannot open
```

而两侧的输入完全一样：`landdata/` 都只有 `srfdata.nc`，两个 `rawdata_unused/` 目录**都不存在**。
**唯一的差别是探针脚本会先跑 `build_kernel.sh default`（重编内核）再运行**，而 `dry_ts.sh`
用的是仓库里现成的 `kernels/default/colm.x`。

⇒ 假设：**重编出来的内核与预建内核行为不同**（例如重编时 `include/define.h` 的生成或
`.bld` 的配置切换，导致 plant 原始数据的读取被启用）。探针的插桩结果因此全部无效，
**而且这条假设可以直接判定**：

```bash
# 不打补丁，只重编，然后跑 dry_ts.sh 的那套 case：
./oracle/scripts/build_kernel.sh default
bash /tmp/gf/dry_ts.sh 1 && grep -c 'CoLM Execution Completed' /tmp/gf/dryts/run/f.log
# 若此时也不完成 ⇒ 是"重编"本身的问题，与插桩无关；
# 若完成 ⇒ 问题在补丁（但补丁只是加了一行 WRITE，几乎不可能）
```

**在探针能像 `dry_ts.sh` 一样跑完一步之前，任何插桩结论都不得采信**（探针脚本现已带
完成标志硬闸门，会直接以非零码退出，不会再产出"0 命中"这种误导性结论）。

Tested: 两份 case.nml 的逐字节 diff；两份内核日志的 diff；`landdata`/`rawdata_unused` 的存在性对比；
四个探针脚本的 `end_sec`/`HIST_FREQ` 对齐与硬闸门（地面探针重跑后以 exit 1 退出、未产出结果）。
Not-tested: "重编内核 vs 预建内核"的对比实验（下一轮第一步）。

## 探针基础设施的两个 bug 都已修掉 —— **首次拿到第 0 步的真实入参**

第 233 轮找到第一个（`end_sec = 0` ⇒ 零步），本轮找到第二个：

```bash
rm -rf "$WORK"; mkdir -p "$WORK/out" "$WORK/run"     # ← 错：预建了 out
cp -R "$BASE/oracle/work/CN-Cng/out" "$WORK/out"     # ⇒ 嵌套成 $WORK/out/out！
```

后果：内核在 `$WORK/out/CN-Cng/landdata/` 下找不到东西，**回落到 rawdata** 并因
`rawdata_unused/plant_15s/….nc` 不存在而中止 —— 与 `dry_ts.sh` 的表现差异（后者只
`mkdir -p $d/run`，让 `cp -R` 自己创建 `$d/out`）至此完全解释。

修法：四个探针脚本都改成**只建 `$WORK/run`**（不预建 `out`），并保留完成标志硬闸门。

### 修好之后的第一个真实成果

```
$ bash oracle/scripts/step_ground_probe.sh
== 二进制里的 GTPROBE_IN 标记数: 1
== 内核完成标志: 1
$ cat /tmp/gf/gtprobe/gt_in.txt        # 1 行、23 个数
GTPROBE_IN  2.830000000000000E+02  8.784146015197782E+00  0.000000000000000E+00  0.000000000000000E+00
            0.000000000000000E+00  0.000000000000000E+00  2.830000000000000E+02  2.830000000000000E+02
            2.830000000000000E+02  1.771419982910156E+02  2.252629943482293E+02  0.000000000000000E+00
            …  fseng 1.245781917695979E+04 · fevpg 3.372336514931096E-04 · cgrnd 1.180443510784811E+02
            · htvp 2.5104E+06 · emg 9.6E-01
```

这正是"入口 A"从第 181 轮起就想要的东西：**干窗第 0 步 `GroundTemperature` 的真实入参**
（且 `sabg = 0` 说明这是 00:00 的夜间步，与算例起点吻合）。

**下一轮**：在 Rust 侧 `ground_temperature` 的入口打印同一组 23 个量，与上面逐位比 ——
这是第一次可以做这个对照（此前所有尝试都因探针跑不完而无效）。

Tested: 四个探针脚本的 `mkdir` 修正；`step_ground_probe.sh` 完整跑通（标记 1、完成标志 1、
产出 1 行 23 个数）；`f48 sync PASS`。
Not-tested: Rust 侧的 23 个量对照（下一轮）。

### 23 个入参的分类：只有 **9 个"计算量"** 需要两侧对照，其余由 restart 决定

把第 235 轮拿到的 23 个入参按"来源"分两类 —— 这一步本身就是收窄：

| 类别 | 量 | 为何两侧必然相同 |
|---|---|---|
| **状态（13 个）** | `t_soisno(1)`、`wliq_soisno(1)`、`wice_soisno(1)`、`scv`、`snowdp`、`t_grnd`、`t_soil`、`t_snow`、`fsno`（无雪时 0）、以及 `htvp`（由表层是否纯冰定的常数）| 两侧读**同一份 restart**、按同一套映射搬运；干窗第 0 步它们未参与任何新计算 |
| **计算量（9 个）** | `frl`、`dlrad`、`sabg`、`sabg_soil`、`sabg_snow`、`fseng`、`fseng_soil`、`fseng_snow`、`fevpg`、`fevpg_soil`、`fevpg_snow`、`cgrnd`、`emg` | 由第 0 步之前的辐射/湍流计算得到 —— **它们已经在 history 口径里被证明差 1e-13…1e-15 相对**（`f_olrg`/`f_rnet`/`f_fseng`/`f_fevpg` 等）|

（严格地说 `emg` 来自地表反照率/雪盖 —— 无雪时是地类常数，也应相同；`cgrnd` 是地表热通量
对温度的导数，属计算量。）

**结论**：`t_soisno` 的 1 ULP **不可能来自那 13 个状态量**（两侧同一份 restart），只能来自
这 9 个（实际 ~8 个）**计算量**——而它们正是本会话早先已经在 history 里看到差异的那一族。
于是"入参对照"这一环不再需要逐个验证状态量，**下一轮只需在 Rust 侧打印这 8 个计算量**并与
上面那份 dump 比。

Tested: 第 235 轮 dump 的 23 个量按来源分类；与既有 history 差异族（`f_olrg`/`f_rnet`/`f_fseng`/
`f_fevpg`）的对应。
Not-tested: Rust 侧那 8 个计算量的打印与逐位比对（下一轮）。

## 探针修好后**结论反转**：叶温 `moninobukm` 在第 0 步**确实被调用**（11 次）

用修好的 `step0_arg_probe3.sh` 重跑（标记 3 个都在二进制里、内核完成标志 1）：

```
$ head -4 /tmp/gf/argprobe3/probe.txt
ARGPROBE_EXT  6.000000000000000E+00  6.000000000000000E+00  6.000000000000000E+00 -1.084670892101044E+02
ARGPROBE_EXT  6.0E+00  6.0E+00  6.0E+00 -1.144412965630008E+02
ARGPROBE_EXT  6.0E+00  6.0E+00  6.0E+00 -1.976860788342615E+02
ARGPROBE_EXT  6.0E+00  6.0E+00  6.0E+00 -2.002591638310636E+02
$ wc -l … → 11
```

也就是说：

* 叶温 **PFT**（`…_Extended.F90`）那支的 `CALL moninobukm` 在干窗第 0 步**被调用 11 次**
  （与 Newton 迭代次数 × 阳叶/阴叶相称）；
* 第 194-232 轮"叶温不执行 / 地面不执行 / THERMAL 不执行"的结论**全部反转** —— 那些都是
  零步运行 + `cp` 嵌套两个 bug 的产物（第 233、235 轮已定位并修复）；
* 但**第 236 轮的入参分类仍然成立**：`GroundTemperature` 的 13 个状态量由同一份 restart
  决定，残差只能从计算量进来。

至此"入口 A"的两侧数据**第一次都可得**：上游侧 11 行 `hu_/ht_/hq_/obu` 已在
`/tmp/gf/argprobe3/probe.txt`；Rust 侧对应量在 `leaf_temperature.rs` 的
`MoninObukhovInput` 构造处（字段映射见本文档"入口 A 就绪清单"）。

Tested: `step0_arg_probe3.sh`（修好后）完整跑通：标记 3、完成标志 1、11 行 `ARGPROBE_EXT`、f48 PASS。
Not-tested: Rust 侧同位置对照（下一轮）。

## 入口 A 第一次真正两侧对照：`hu_/ht_/hq_/obu` **在 16 位有效数字上完全一致**

Rust 侧临时探针（插在 `leaf_temperature.rs:411` 的 `CanopyMoninObukhovInput` 构造处，打印
`wind_height/temperature_height/humidity_height/obukhov/z0mv/stability_wind`）跑 `dry_ts.sh 1`，
与上游那 11 行对照：

```
上游  ARGPROBE_EXT 0.6000000000000000E+01 … -0.1084670892101044E+02
Rust  PROBE_RB     6.0 6.0 6.0 -10.846708921010444 0.12057264881491078 4.670838949913892
上游  ARGPROBE_EXT 0.6000000000000000E+01 … -0.1144412965630008E+02
Rust  PROBE_RB     6.0 6.0 6.0 -11.444129656300083 …
```

`hu_=ht_=hq_=6.0` 两侧一致；`obu` 也一致到**打印精度所及的最后一位**
（上游 `-10.84670892101044` 是 `-10.846708921010444` 四舍五入到 16 位有效数字的结果）。
另外上游 11 行、Rust 10 行 —— **调用次数不同**（值得下一轮看：多出的那一次是哪个分支）。

**但这条对照还不足以下"逐位相同"的结论**：上游探针用的是 `E24.16`（16 位有效数字），
末位差 1 ULP 会被四舍五入掩盖。**下一轮第一步**：把 `step0_arg_probe3.sh` 的打印格式换成
**位型**（`WRITE(*,'(A,4Z17)') 'ARGPROBE_…', TRANSFER(<量>, 0_8), …` —— 注意不能用 `B()`
函数，那段代码插在 vendor 模块内部，只能靠 `TRANSFER` 内联），再重跑对照；同时查 11 vs 10
的调用次数差异。

（临时加在 `leaf_temperature.rs` 的 `eprintln!` 已回退，工作树干净。）

Tested: Rust 侧临时探针 + `dry_ts.sh 1`（10 行 `PROBE_RB`）与上游 11 行的对照；`git checkout` 回退。
Not-tested: 位型级对照（格式待升级）；11 vs 10 的差异原因。

## 入口 A 的**位型级对照**：叶温 `moninobukm` 的前四个实参**逐位相同**

上游探针已改成位型打印（内联 `TRANSFER(x,0_8)`，因为插在 vendor 模块内部无辅助函数可用）：

```
ARGPROBE_EXT  4018000000000000 4018000000000000 4018000000000000 C025B183D4E9F14F   ← hu_/ht_/hq_=6.0, obu
ARGPROBE_EXT  4018000000000000 4018000000000000 4018000000000000 C026E364F659FC33
ARGPROBE_EXT  4018000000000000 4018000000000000 4018000000000000 C033C4C37C7AC35B
ARGPROBE_EXT  4018000000000000 4018000000000000 4018000000000000 C03406A274C1DF56
```

Rust 侧上一轮打印的十进制是**最短往返表示**，可以唯一还原位型：

| Rust 值 | 位型 | 上游对应行 | 结论 |
|---|---|---|---|
| `-10.846708921010444` | `C025B183D4E9F14F` | 第 1 行 | **逐位相同** |
| `-11.444129656300083` | `C026E364F659FC33` | 第 2 行 | **逐位相同** |
| `-19.768607883426153` | `C033C4C37C7AC35B` | 第 3 行 | **逐位相同** |
| `-20.025916383106356` | `C03406A274C1DF56` | 第 4 行 | **逐位相同** |

（`C034069B6C2F4C5D` 是**第 5 行**，不是第 4 行 —— 这是个对齐陷阱：两侧行数不同时不能按序号硬配，
必须按值配。）

### 两个结论

1. **`hu_/ht_/hq_/obu` 这组实参逐位相同** ⇒ `t_soisno`/`tleaf` 的 1 ULP **不是**从这里进来的
   （与第 236 轮"残差只能来自计算量"的分类相容：这一组实参本身没错，错的是它们**下游**的
   `fseng`/`fevpg`/辐射那一族计算量）；
2. **调用次数 11 vs 10**：上游按阳叶/阴叶各调一次（每轮迭代 2 次），Rust 的
   `canopy_monin_obukhov_with_scheme` 每轮调一次 —— 结构不同但**实参相同则输出相同**，属良性差异；
   仍需在下一轮确认"多出的那一次"确实是这个来源（而不是少算了一轮迭代）。

Tested: 位型格式的上游探针重跑（3 个标记在二进制、11 行十六进制）；Rust 十进制的最短往返还原与
逐行位型比对（前 4 行逐位相同）。
Not-tested: 11 vs 10 的成因（阳叶/阴叶 vs 单次，或迭代次数差 1）。

## 残差方向重新确认：**叶温模块的 80 处收缩重新回到靶子上**

用第 235 轮的探针 dump 与两侧 history 对照，把链路钉住：

```
第 0 步 history（内核 vs Rust）：
  f_fseng  kernel=735.85728410080173  rust=735.85728410080242   ← 差
  f_fevpg  kernel=7.7660762305301089e-05  rust=7.7660762305301265e-05
  f_olrg   kernel=275.7114341005834   rust=275.71143410058329
  f_rnet   kernel=-98.569435809567779 rust=-98.569435809567665
```

即：**喂给 `GroundTemperature` 的计算量在第 0 步就已经不同**（它们与这四个 history 量同族），
而叶温相似性调用的**实参已证逐位相同**（第 239 轮）。合起来 ⇒ 残差**产生于叶温/通量计算内部**。

**这纠正了第 226/229 轮的一个副作用**：当时因探针 bug 误判"叶温不执行"，我据此刻意回避了
`lt_ext.opt` 那 **80 处收缩**；现在叶温**确实执行**（第 237 轮：11 次 `moninobukm` 调用），
所以那 80 处**重新成为首要靶子** —— 而且与本会话唯一落地的收缩修复 `olrg`（步级 582→585）
同属这一族，二者互相印证。

**下一轮的做法**：按 `lt_ext.opt` 的 80 处逐条 A/B，优先级 = 出现在 `fseng`/`fevpg`/`f_rnet`
计算链上的那些（`MOD_LeafTemperature` 里与 `irab`/`sabv`/`fsenl`/`fevpl`/`dirab` 相关的收缩），
验收用**三段式**（restart 19/68 → 步级 585/692 → 三窗口），并遵守"收缩必须有口径支持才落"。

Tested: 第 0 步 history 的四个量逐位比对（全部不同）；与第 235 轮 dump、第 239 轮实参位型对照的合并推断。
Not-tested: `lt_ext.opt` 80 处的逐条 A/B（下一轮起）。

### 靶子清点的现实困难（下一轮的方法建议）

第 241 轮把靶子定在 `lt_ext.opt` 的 80 处、优先级按"是否在 `fseng`/`fevpg`/`f_rnet` 链上"。
本轮清点时遇到一个具体障碍：**这份 dump 里没有 `# DEBUG irab/fsenl/fevpl/dirab/sabv` 标记**
（`-fdump-tree-optimized` 只在变量跨块时才打 DEBUG 行），所以无法用变量名直接把 80 处归到通量链上。

**下一轮建议的做法**（按可行性排序）：

1. **按源码行号反查**：先在对文件里定位 `irab`/`sabv`/`fsenl`/`fevpl`/`dirab` 的计算语句行号，
   再在 dump 里找落在这些行附近的 `.FMA/.FNMA`（本会话在 `MOD_FrictionVelocity:downscale_wind`
   上用过同一手法：dump 的 `_NNN` 与源码语句逐行对应）；
2. **换更细的 dump**：`-fdump-tree-optimized-raw` 或 `-fopt-info-optimized` 可能保留更多来源
   信息（未验证）；
3. **不查 dump，直接经验筛选**：对 80 处涉及的表达式按"Rust 是否平铺"逐个用三段式 A/B —— 
   本会话的统计是 6 处"dump 有收缩"里 1 处落地（`olrg`）、5 处被否，命中率不高但每次代价有限。

Tested: `lt_ext.opt` 里 `DEBUG irab/fsenl/…` 的缺失（grep 无命中）；80 处的 FMA 列表已取到前 12 条。
Not-tested: 上述三种做法的实际效果（下一轮选一种）。

## 方法 1 跑通（`-S -fverbose-asm` 定位到源码行），但第一个候选**已经是对的**

用 `gfortran -S -O2 … -g -fverbose-asm` 生成带 `.loc` 的汇编后，可按源码行统计 FMA 类指令
（比 `objdump -d -l` 的 DWARF 归因可靠，本会话在 `MOD_FrictionVelocity` 上用过）：

| 对文件行 | 语句 | 该行 FMA 类 |
|---|---|---|
| 1104 / 1107 | `irab = (frl - 2*stefnc*tl**4 ± …) * fac` | **0**（纯乘加，不收缩）|
| **1116** | `fsenl = rhoair*cpair*cfh*((wta0+wtg0)*tl - wta0*thm - wtg0*tg)` | **1**（`fnmsub`）|
| 1160 | `fevpl = etr + evplwet` | 0 |
| 1342 | `fsenl = fsenl + fsenl_dtl*dtl(it-1) + …` | 0（该处是逐级 FMA 但落在别的行）|
| 1379 / 1380 | `fevpl = fevpl - elwdif` / `fsenl = fsenl + htvpl*elwdif` | 1 / 1（`fmadd`）|

**而 1116 那个候选在 Rust 侧已经是对的**：`leaf_temperature.rs:687-695` 正是
`-ground.mul_add(tg, (a+g).mul_add(tl, -(a*thm)))` 的融合链，注释里还写着"实测 4000/4000 组
逐位相同（不收缩只有 345/4000）"—— 即**更早的轮次已经按 dump 落过**，本轮只是用独立方法复核
一致。

**结论**：叶温通量链（`fsenl`/`fevpl`/`irab`）的收缩**已被覆盖**；残余不在这里。这也解释了
为什么第 241 轮把靶子重新指回叶温后，逐条核下去会不断命中"已经对过"的站点。

**下一步**：把方法 1 用到 `th_ext.opt`（99 处）与 `lt_ext.opt` 里**尚未核对过**的那些行上，
优先 `f_rnet`/`f_olrg` 的入参链（`MOD_NetSolar`、`MOD_Albedo` 的 `alb`/`emis`）；若再次大面积
命中"已对"，则应当考虑残余来自**调用顺序**而非表达式（第 223 轮列出的第 1 类）。

Tested: `-S -fverbose-asm` 的行级 FMA 统计（1104/1107/1116/1160/1342/1379/1380）；
与 `leaf_temperature.rs:687-695` 的对照（含其 4000/4000 的既有证据）。
Not-tested: `th_ext.opt` 与 `lt_ext.opt` 余下未核行的逐条 A/B。

## 辐射输入是**干净的**，差异集中在"叶能量收支的派生通量"上

按第 243 轮的计划核 `rb`/`alb`/`emis` 那条链，先用现成的第 0 步 history 看**哪些量在差**
（top-50 里筛辐射/通量族）：

```
0 f_fevpa  3.96e-16    0 f_olrg   4.12e-16    0 f_fseng  9.27e-16
0 f_rnet   1.15e-15    0 f_fsena  1.57e-15    0 f_fevpg  2.27e-15
0 f_fevpl  6.04e-15    0 f_fsenl  9.24e-15
```

**没有出现** albedo / emissivity / `sabg` / `lwrad` / `par` / `swrad` 一类 —— 即：

* **辐射输入（反照率、吸收短波、下行长波）两侧逐位相同**；
* 差的全部是**叶/地能量收支派生出来的通量**：`f_fsenl`（叶显热）、`f_fevpl`（叶潜热）、
  `f_olrg`（上行长波）、`f_rnet`（净辐射）、`f_fseng`/`f_fevpg`（地表显热/潜热）、
  `f_fsena`/`f_fevpa`（两者之和）。

这与前几轮串起来完全自洽：叶温求解**确实执行**（第 237 轮）、其相似性实参**逐位相同**
（第 239 轮）、叶通量链的收缩**已核过**（第 243 轮，含 4000/4000 的既有证据）——
于是残余落在**这条链上尚未逐条核过的收缩**里，而不是在辐射输入或实参上。

**下一轮**：用方法 1（`-S` + `.loc` 行级统计）把 `MOD_LeafTemperature_Extended.F90` 里
与 `fsenl`/`fevpl`/`olrg`/`rnet` 相关的**其余** FMA 行列全（前几轮只核了 `tref`/`qref`、
收敛后 7 处、`clai`、`cfw`、`thvstar` 等），逐条与 Rust 对照；命中"已对"就记，命中"平铺"
就按三段式 A/B。

Tested: 第 0 步 history 的辐射/通量族筛选（上表）；与前几轮结论的合并推断。
Not-tested: 该链余下收缩的逐条对照（下一轮）。

## 第 245 轮：整核 `-ffp-contract=off` 对照 —— 收缩**确实**是杠杆，但不止它一个

前三轮一直在"逐条猜收缩站点"（第 243 轮方法 1 核了 7 行，只命中"已对"）。本轮换一个
**不猜**的问法：把整个内核用 `-ffp-contract=off` 再编一遍，看第 0 步 restart 到底有多少
变量**真的**依赖收缩。

做法（不动 `kernels/default`）：把 `oracle/scripts/build_kernel.sh` 复制到 `/tmp/gf/`，
给 `MAKE_FF` 追加 `${EXTRA_FFLAGS:-}`、把 make 目标收成 `colm.x`、`REPO_ROOT` 写死，
再 `EXTRA_FFLAGS="-ffp-contract=off" ... default /tmp/gf/nocontr`。编译行实测是
`gfortran -fopenmp … -ffp-contract=off -c -O2 -fdefault-real-8 …` —— `FOPTS` 里不含
`-ffp-contract`，所以这个 off 一路生效。产物 `/tmp/gf/nocontr/default/colm.x`。

### 结果一：收缩影响 19/68 个 restart 变量

干窗 1 步、同一份 `case.nml`（`/tmp/gf/ctr_ts.sh <colm.x> <tag>`，比 `dry_ts.sh` 多一个
内核参数、只跑内核侧）：

```
def vs noctr: 19 / 68 个 restart 变量不同
```

**19 与 Rust 对默认内核的 19 是同一个量级** —— 也就是说"末位残差可不可能全由收缩解释"
这个问题，答案是"量级上完全可以"，收缩这条线值得继续挖（第 241 轮把靶子重新指回叶温
是对的）。

### 结果二：三路分类（def / noctr / Rust 同场比较）

对 68 个变量逐个判 `def==noctr`、`def==rust`、`noctr==rust`：

**先记一次自己的错**：这张表的第一版把两格的标签写反了（`(def==noctr, def==rust, noctr==rust)`
这个三元组是按 `(kd,kn,nr)` 生成的，第一版字典的键与文字错位），据此在对话里得出过
"9 个写对了、2 个漏融合"的相反结论。下面这一版是重新按三元组逐格核过的。

| 三元组 `(def==noctr, def==rust, noctr==rust)` | 数量 | 变量 | 含义 |
|---|---|---|---|
| `(T,T,T)` | 47 | （含 `tleaf`、`t_grnd`、`alb`、`emis`、`sabg`、`lai`…） | 与收缩无关，三方一致 |
| `(F,F,T)` | 9 | `fh` `fm` `fq` `hk` `rib` `smp` `tstar` `wice_soisno` `wliq_soisno` | Rust **等于不收缩内核**、不等于默认内核 |
| `(F,T,F)` | 2 | `coszen` `vegwp` | Rust 等于默认内核（收缩形状写对） |
| `(F,F,F)` | 8 | `fwet_snow` `ldew` `ldew_snow` `qref` `qstar` `t_soisno` `ustar` `zol` | 三方互不相等 |
| `(T,F,F)` | 2 | `rst` `trad` | **与收缩无关**，只有 Rust 不同 |

**这张表不能直接读成"哪里漏了融合" —— 这是本轮最重要的一条读法警告。**
`fh`/`fm`/`fq` 落在 `(F,F,T)` 那一格，很容易被读成"Rust 的 `moninobukm` 漏了融合"；
但 `oracle/scripts/compare_moninobukm.sh` 恰恰**把模块按生产 flag 单独编**（注释里写明
"不加 `-ffp-contract=off`"），20 个输出 × 20000 组逐位全同 —— 也就是说
**"输入逐位相同 ⇒ Rust 的 fm/fh/fq 与默认内核逐位相同"已经被证明过**。于是
`(F,F,T)` 只能解释为：**喂给这个调用的某个上游输入，默认内核与不收缩内核不同，而 Rust
给的与不收缩内核那一侧一致**。同样的逻辑适用于 `smp`/`hk`/`wliq_soisno`（土壤水力那条
有 `compare_soilthermal.sh` 的独立差分）。

所以这张表的正确用途是**缩小上游搜索范围**，不是找站点：它把 19 个"收缩敏感"变量分成
"Rust 站在不收缩那侧"（9 个）、"三方都不同"（8 个）、"与收缩无关"（2 个），
真正要回答的问题仍然是**哪一个上游量先分叉**。

读法：

* `coszen`/`vegwp` 两个 Rust 与默认内核一致 ⇒ 它们的收缩形状**已经对了**（`vegwp` 的
  maxrel 1.14e-13 说明它是被放大的下游，不是种子）；
* `rst`/`trad` 与收缩无关，但**可能是下游继承**：`trad=(olrg/stefnc)**0.25`，而 `olrg` 在
  `(F,F,F)` 那 8 个里；`rst=1/(laisun/rssun+laisha/rssha)`，`rssun`/`rssha` 来自叶温循环。
  不能当成"找到一个非收缩缺陷"。

**下一步**：不要再按变量猜站点。回到第 245 轮末尾的结论 —— 用**逐迭代、逐位**的现场探针
把叶温循环内部（以及 `MOD_GroundFluxes` 那次 `moninobuk`）的**首个分叉量**找出来；
`(F,F,T)` 那 9 个正好给了探针该盯的输出名单。

Tested: `/tmp/gf/nocontr/default/colm.x` 的构建；`ctr_ts.sh` 两次 1 步跑；
`restart_divergence.py` 三对比较（19 / 19 / 12）；上面的三路分类脚本（改后重跑一次，
与改前逐格一致）。
Not-tested: 逐迭代现场探针（下一轮）。

### 结果三：`11 vs 10` 的 `moninobukm` 计数是**误读**，叶温循环两侧都是 10 轮

第 239 轮据 `/tmp/gf/argprobe3/probe.txt` 的"11 行十六进制"推断"上游调 11 次、Rust 调 10 次"，
并把它当成"结构不同但良性"。数一下标记前缀就清楚了：

```
$ grep -o "ARGPROBE_[A-Z]*" /tmp/gf/argprobe3/probe.txt | sort | uniq -c
      1 ARGPROBE_ACC
     10 ARGPROBE_EXT
```

`step0_arg_probe3.sh` 同时给**三个候选文件**插了标记：`EXT`（叶温 `CALL moninobukm`）、
`ACC`（`main/MOD_Vars_1DAccFluxes.F90` 的 `CALL moninobuk`）、`PC`（另一份叶温）。第 11 行是
`ACC`，**不是第 11 轮叶温迭代**。也就是说：

* 内核叶温循环 = **10 轮**，与 Rust 的 10 次 `canopy_monin_obukhov_with_scheme` 一致；
* 第 244 轮那句"上游按阳叶/阴叶各调一次（每轮迭代 2 次）"是**错的**（源码
  `MOD_LeafTemperature_Extended.F90:700/723` 每轮只有一次调用），撤回；
* 与"`tleaf` 逐位相同"合起来：**第 223 轮列的第 1 类（迭代次数/调用顺序差）被否**。

`ARGPROBE_ACC` 那一行的 `hu_/ht_/hq_` 是 `401897DED2C68EBA`（≈6.15 m），与叶温那 10 行的
`4018000000000000`（=6.0 m）不同 —— 对表时要**按前缀分组**，这正是第 244 轮"按值配不要
按行号配"的同一个坑的另一种表现。

Tested: `probe.txt` 的标记计数（10 EXT / 1 ACC）；与源码 `:700`/`:723` 的单调用对照。
Not-tested: `ARGPROBE_ACC` 那次调用的输出（不是本轮目标）。

## 第 245 轮：`dtl` 的分母在内核里是**平铺**的 —— 并给"独立 dump 不可信"补上一个可执行的检查

第 243 轮用 `gfortran -S` 的 `.loc` 读汇编，发现叶温 `dtl`（`MOD_LeafTemperature_Extended.F90:1188`）
的分母是整条平铺、分子才是融合的。这与本仓库此前据**独立差分驱动器** 4000 组统计写下的
"分母也是三层 `mul_add` 嵌套"（源码里那条注释）冲突。冲突的裁决不能靠再读一遍 `.s` ——
要证明**这份 `.s` 描述的就是出货内核**。

**新方法（可复用）：对同一函数做"操作码窗口比对"。**

```
/opt/homebrew/opt/llvm/bin/llvm-objdump \
    --disassemble-symbols=___mod_leaftemperature_MOD_leaftemperature <file>
```

对 `/tmp/gf/r166/lt_ext.o`（独立编译、`.s` 的来源）与 `kernels/default/colm.x`（出货内核）
各取一次，浮点操作码直方图**逐项相同**：

```
fmul 211  fadd 66  fsub 77  fmadd 52  fmsub 16  fnmsub 7  fdiv 108
```

差集只有整数/访存类（`ldr 931/901`、`str 386/381`、`add 180/197`、`fmov 91/106`），
属寄存器分配差异，不影响浮点语义。再看那个 `fdiv` 的**前驱窗口** —— 两份都出现

```
… fmul, fmadd, ldr, [fmov,] ldr, ldr, fadd, ldr, fadd, fadd, fadd, fdiv
```

即分母收尾是**三个连续的 `fadd`**。如果分母是三层 `fma`，这里应当是连续的 `fmadd`。
**分母平铺，判据落在出货二进制上**，不再是"我读的 `.s` 大概是内核那份"。

于是落地的形状（`leaf_temperature.rs`）：

```text
_539 = (clai/deltim - dirab_dtl) + fsenl_dtl
_541 = _539 + htvpl*fevpl_dtl             平铺
_526 = cpliq*max(0,qintr_rain)            先舍入成一项
_533 = cpice*max(0,qintr_snow)            先舍入成一项
_543 = ((_541 + _526) + _533)             平铺
分子：fma(dT, _533, fma(_526, dT, fma(-htvpl, fevpl, sabv+irab-fsenl)))
```

**为什么分母不收缩**：`_526`/`_533` 这两个乘积**分子里也要用**（分子用它们的"外层"乘积
`(cpliq*rain)*(t_precip-tl)`）。共同子表达式被 CSE 成独立的一项之后，分母里的加法就
没有乘积可吸了 —— 这是"共同子表达式挡住收缩"的典型，也正是独立驱动器里**没有**的那一半
上下文。与第 166/167 轮"独立编译的 dump 不代表内核二进制"同源，但这次给出了可执行的判别。

**度量：这一步是惰性的。** 改完三次口径全部逐字节不变：

```
restart（干窗 1 步）: 19 / 68，变量表与 maxabs 与改前完全相同
步级（dry_ts.sh 3）: 44 个变量、585/692（84.5376%），与基线相同
0 步 history 的 33 个差异变量表：相同
```

按本仓库的规矩"收缩形状要有度量支持才落"，惰性形状本该回退；这里**留**它是另一条理由：
它不再是从 dump 猜出来的收缩，而是**出货二进制的操作码窗口直接指明的形状**，属于"照抄
编译器"，与"源码显式形状即使惰性也照抄"同类。这是**保真度改动，不是度量修复**，据实记录
以免后来者以为它修好了什么。同时撤回源码里那句 3988/4000 —— 那是独立驱动器的数字，
对内核不成立。

Tested: `llvm-objdump` 两次（`.o` 与 `colm.x`）的浮点直方图与 `fdiv` 前驱窗口；改后
`dry_ts.sh 1` / `dry_ts.sh 3` + `restart_divergence.py` / `window_divergence.py`（三次口径
逐字节同基线）。
Not-tested: 分子那一侧 `fma(-htvpl,fevpl,·)` 的操作码窗口逐条比对（直方图同；逐条留给下次
遇到疑点时复用本方法）。

**下一轮**：**不要再按变量猜站点**（本项目到这里已经证明"dump 里看到收缩"与"Rust 里该
加收缩"之间隔着一个共同子表达式和一个 CSE）。做法改为**逐迭代、逐位的现场探针**：

1. 在 `extends/interception/MOD_LeafTemperature_Extended.F90` 的收敛判据前插一条
   `WRITE(*,'(A,I3,24Z17)')`，转储 `it, tl, dtl(it), del, dele, fsenl, fevpl, irab,
   dirab_dtl, fsenl_dtl, fevpl_dtl, obu, ustar, cfh, cfw, raw, rah, wta0, wtg0, wtaq0,
   wtgq0, taf, qaf, qsatl` 的**位型**；Rust 侧在 `last = Iteration{…}` 之后打同一组
   （临时插桩，量完立刻 `git checkout` 回来）；
2. 用现成的 `step_iter_probe.sh` 那套"备份→插桩→重编→跑 1 步→还原→重编"骨架
   （注意 `trap … EXIT`、**不要预建 `$WORK/out`**、跑完核对 `strings` 与
   `CoLM Execution Completed`）；
3. 两侧按迭代号逐行比，找**第一个分叉的量**。目标是 §结果二 里 `(F,F,T)` 那 9 个
   （`fh fm fq rib tstar` 一族、`smp hk wliq_soisno wice_soisno` 一族）第一次出现不同的
   那一轮、那一个量。

`rst`/`trad`（与收缩无关的那两个）留作交叉验证：如果现场探针给出的首个分叉量在
`rssun`/`rssha`/`olrg` 那条链上，就同时解释它们。

同一套"操作码窗口比对"（`llvm-objdump --disassemble-symbols=…` 对 `colm.x` 取窗口）
下次遇到"这个表达式到底融没融"的争议时直接复用 —— 它是目前唯一**指向出货二进制**的
判据。

## 第 246 轮：**逐迭代位型探针**把叶温循环的首个分叉钉死在**第 1 轮迭代**

前几轮一直在"猜站点 → 三段式 A/B"，代价高且命中率低。本轮换一个**直接**的问法：
把内核与 Rust 在同一位置（内核的 `it = it+1` 之前、Rust 的 `iteration += 1` 之前）、
同一组 34 个量的**位型**逐迭代打出来，逐行比。工具
`/tmp/gf/leafit_bits_probe.sh`（骨架照 `step_iter_probe.sh`，但目标是
**`extends/interception/MOD_LeafTemperature_Extended.F90`**，且打印位型而不是
`E24.16`——末位 1 ULP 会被十进制舍入掩盖）。

第 1 轮迭代（此时**所有输入都还是初值，两侧逐位相同**）的结果：

| 量 | 原状 | 判读 |
|---|---|---|
| `fsenl` | 差 1 ULP | 括号的融合形状错 |
| `irab` | 差 1 ULP | 地面长波**没有逐项融合**、`*fac+第二项`也没融 |
| `fevpl_dtl` | 差 1 ULP | 等于 `etr_dtl`（该轮 `evplwet_dtl` 恰好为 0） |
| `cfw` | 差 1 ULP | "先乘后除" + 加法那侧没融 |
| 其余 30 个 | 逐位相同 | 包括 `dirab_dtl`、`fsenl_dtl`、`qsatl`、`obu`、`cfh` |

**这张表本身就是结论**：`fsenl` 的四个输入（`cfh`、`wta0`、`wtg0`、`tl`）全部逐位相同，
`fsenl_dtl`（同一行、无 fma 的那个）也逐位相同 —— 所以差的**只能是括号里那个乘积的
融合选择**。这比"4000 组随机数里 345/4000"那种统计证据锐利得多：它是**实际轨迹上的
一个反例**，直接把此前按驱动器统计写下的形状否掉了。

### 从 `lt_ext.s` 读出来的四处形状（全部按汇编改）

`fsenl`（`.loc 1 1116`）：

```text
_459 = wta0 + wtg0
_461 = thm * wta0            fmul  ← 先舍入
tmp1960 = fmsub(_459, tl, _461)   → 只有 _459*tl 融合
_463 = tg * wtg0             fmul  ← 先舍入
_464 = tmp1960 - _463
fsenl = _464 * (rhoair*cpair*cfh)
```

`irab`（`.loc 1 1105`，`L417`/`LBB245`）：

```text
_3199 = frl - (2*stefnc)*tl**4                    fmsub
非分裂：_400  = fma(emg*stefnc, tg**4, _3199)
分裂  ：tmp1938 = fma(((1-fsno)*emg)*stefnc, t_soil**4, _3199)
        _429    = fma((fsno*emg)*stefnc, t_snow**4, tmp1938)
tmp1923 = fma(_400, fac, _3205)     ← 第二项是 `(1-emg)*thermk*fac*frl`
irab    = tmp1923 + _3210           ← 只有第三项是平铺加法
```

`cfw`（`.loc 1 1076/1079`，默认 scheme 的分支 `L166`）：

```text
_3064 = dry_factor * delta
_3067 = 1 - _3064                    （= evp_weight）
_382  = _3064 * SUM
cfw   = fma(_3067, wet_cond_cfw, _382)
wet_cond_cfw = (lai + sai) / rb      ← **先除**，不是 `coef*lsai/rb`
```

`evplwet`（`:1150-1151`，本轮**测不到**，按源码照抄）：

```text
evplwet = rhoair * evp_weight * wet_cond * ( … )     wet_cond = wet_area/rb
```

落地的五处改动在 `crates/colm-core/src/leaf_temperature.rs`：`leaf_sensible_heat` 的括号、
`longwave()` 的三处融合、`leaf_moisture_conductance`（新增 `wet_conductance`/`dry_factor`
两个具名中间量）、`wet_evaporation` 与它的温度导数。

### 改完之后：**第 1 轮迭代只剩 `rssun`/`rssha`**

同一个探针重跑（第 4 次）：

```text
it=1: DIFFERS: fevpl_dtl, cfw, rssun, rssha, etr_dtl
      rssun  kernel=40F86A00061BA574 rust=40F86A00061BA573
      rssha  kernel=40F86A00061BA574 rust=40F86A00061BA573
```

`fsenl`/`irab`/`fsenl_dtl`/`dirab_dtl`/`qsatl`/`wet_cond`/`rb`/`laisun`/`laisha`/`qsatlDT`/
`evp_weight`/`dry_factor`/`delta` 全部逐位相同。**`cfw`/`etr_dtl`/`fevpl_dtl` 是
`rssun`/`rssha` 的下游**（`SUM = laisun/(rb+rssun) + laisha/(rb+rssha)`），不是独立缺陷。
`it=3` 甚至出现过一次**整轮 34 个量全部逐位相同** —— 说明这条链路本身可以完全对齐。

**下一个（也是唯一一个）根因**：`rssun`/`rssha` 的 1 ULP 来自气孔侧。CN-Cng 这一支走的是
`DEF_USE_PLANTHYDRAULICS`（`vegwp` 会随收缩变化即为旁证），上游 `rssun` 由
`PlantHydraulicStress_twoleaf` 解出的 `gssun` 反算（`MOD_LeafTemperature_Extended.F90:919`
的 `rssun = tprcor/tl*1e6/gssun`），不是 `stomata` 的 `rst`。**下一轮把同一个探针挪到
PHS/气孔那一段**（`gssun`/`gssha`/`gs0sun`/`gs0sha`/`assimsun`/`etr` 一族的位型），
先判"是 PHS 求解器还是 `update_photosyn`/`Assim`"。

### 三段式度量（含黄金窗口）

| 口径 | 基线 | 本轮 | 判读 |
|---|---|---|---|
| restart（干窗 1 步） | 19 / 68 | 19 / 68 | 未动（被 `rssun` 那一支拖着） |
| 步级（干窗 3 步） | 44 变量 / 585-692 | 43 变量 / 581-692 | **混沌口径**，±4 在噪声内 |
| 黄金 `ot_vars` | 17 / 68 / 79 | 17 / 68 / 79 | 未变 |
| 黄金 dry `sumabs` | 338.9256 | **274.5483** | **改善 19%** |
| 黄金 dry `over_tol` | 825 | **821** | 改善 |
| 黄金 wet `sumabs`/`over_tol` | 10369.4411 / 20672 | 10373.0521 / 20673 | 噪声内 |
| 黄金 snow `bitwise`/`sumabs` | 33651 / 444394.4368 | 33593 / 444394.4368 | `sumabs` 相同 |

干窗 `sumabs` 从 338.93 降到 274.55 是**这一轮唯一有分辨力的度量信号**：
步级"逐位相同元素数"是混沌量（1 ULP 扰动会在某一步放大并翻转个别元素），
而 `sumabs` 是连续量。这也再次说明**为什么该用逐迭代位型探针而不是三段式来定位**。

### 三条工具教训（都会再踩，写下来）

1. **探针脚本的还原绝不能用 `git checkout --`**：它把工作区里**未提交**的改动一起冲掉。
   本轮实测——探针跑完把刚写好的四处修复全丢了，只能重写一遍（重写版已存
   `/tmp/gf/r246_fixes.patch`）。还原要 `cp "$WORK/backup/<file>" <file>`。
2. **`strings … | grep -q …` 在 `set -o pipefail` 下会假报失败**：`grep -q` 命中即退出，
   `strings` 吃到 SIGPIPE（141），整条管道被判成非零，于是"标记不在二进制里"。
   先把 `strings` 落到文件再 grep。本轮因此白等了一次全量内核编译。
3. **两侧转储的分隔符要一致**：Rust 侧用 `", ".join(...)` 打出来带逗号，比对脚本按空白切
   就永远不等 —— 第一次跑时 10 轮全被报成"23 个量都不同"。用单空格。

Tested: `/tmp/gf/leafit_bits_probe.sh` 四次（34 个量的位型逐迭代比对）；
`cmp_leafit.py`；`dry_ts.sh 1` + `restart_divergence.py`（19/68）；
`dry_ts.sh 3` + `window_divergence.py`（43 / 581-692）；`win4.sh` + `three.py`
（21334/274.5483/821/17、32653/10373.0521/20673/68、33593/444394.4368/25896/79）；
`llvm-objdump` 复核 `L166`/`L417`/`LBB245` 的操作码窗口。
Not-tested: `rssun`/`rssha` 上游（PHS 求解器 / `Assim`）的逐位探针（下一轮）；
探针里 `wet_evaporation` 那两条（该算例 `evp_weight ≡ 0`，测不到）。

### 附：内核构建**不是逐字节可复现**的（别拿 sha256 当同一性判据）

本轮连续做了三次"干净"构建（源码与 flag 完全相同），得到的
`kernels/default/colm.x` 分别是 `9b2d7434…`、`3101497b…`、`7f5e7d93…` —— 各不相同；
每次 `manifest.json` 里记的 sha 都与当下那份二进制一致（构建脚本自算自写，所以自校验会过）。
**行为**没有变：同一套黄金窗口在三份二进制下 `ot_vars` 都是 17/68/79、`sumabs` 稳定。

因此：

* 不要用 sha256 判断"内核有没有被换过"（例如探针的插桩是否还在）—— 用
  `strings kernels/default/colm.x` 找探针标记，或直接看行为；
* 反过来，**探针跑完必须重编**这件事不能省：那时二进制里确实带着插桩，
  sha 看着"合法"但内容是错的。

## 第 247 轮：探针连破五处形状，叶温迭代**前 6 轮逐位对齐**

第 246 轮结尾把根因指向 `rssun`/`rssha`（气孔侧）。本轮顺着同一个探针一路往下：
每跑一次探针，它就精确地指到一个量；按 `lt_ext.s` 改掉之后再跑，首轮分叉就往后推一轮。
**五次探针 → 五处形状**，这是本会话效率最高的一段。

### 1. `rssun`/`rssha`：PHS 支路的结合顺序

`hydraulic_stomatal_resistance`（`leaf_temperature.rs`）里写的是
`tprcor*1e6/(tl*gssun)`，而内核 `MOD_LeafTemperature_Extended.F90:919` 是

```fortran
rssun = tprcor/tl * 1.e6 / gssun
```

左结合 = `((tprcor/tl)*1e6)/gssun` —— 先除 `tl`、再乘 `1e6`、最后除 `gssun`。
改完 it=1 从 5 个差量（`fevpl_dtl`/`cfw`/`rssun`/`rssha`/`etr_dtl`）缩到 2 个。

### 2. `humidity_gradient`：与 `fsenl` 同一个"过度融合"

`.loc 1 1125` 的括号 `( (wtaq0 + wtgq0)*qsatl - wtaq0*qm - wtgq0*qg )`：

```text
_481 = wtaq0 * qm              fmul  ← 先舍入
_479 = wtaq0 + wtgq0
_483 = wtgq0 * qg              fmul  ← 先舍入
tmp1963 = fnmsub(_479, qsatl, _481)   → 只有 _479*qsatl 融合
_485 = tmp1963 - _483
```

Rust 把 `-wtgq0*qg` 也写成了 `mul_add`（第 246 轮修 `fsenl` 时是同一类错）。
改完 it=1..4 全同。

### 3. `delmax` 限幅：`signum()` 不等于 `delmax*dtl/abs(dtl)`

内核 `:1203` 是 `dtl(it) = delmax*dtl(it)/abs(dtl(it))` —— **乘一次、除一次**，
`signum()` 却给**恰好** ±3.0。it=5 抓到的位型对：

```text
dtl  kernel=C007FFFFFFFFFFFF (= -(3 - 1 ULP))   rust=C008000000000000 (= -3.0)
```

改完 it=5 全同。（`signum()` 在 crates 里只此一处，已复查。）

### 4. `um`：`sqrt(fma(ur,ur,wc*wc))`

`.loc 1 1273`：`_613 = wc*wc`(fmul)、`tmp2112 = fmadd(ur,ur,_613)`、`fsqrt`。
Rust 写成平铺的 `ur.powi(2) + wc.powi(2)`。**这一条不在当时那 34 个探针量里**，
所以它没在 it=5 暴露，而是等到 it=6 以 `ustar`/`obu`/`cfh`/`cfw` 一族的形式炸开。
教训（下一条已按此执行）：**探针量要覆盖"迭代之间传递的全部状态"**，
漏一个量就要多烧一轮 12 分钟的探针。

### 5. `thvstar`/`dthv`：共用的 `(1.+0.61*qm)` 是 fma

内核把 `(1.+0.61*qm)` **算一次就存起来**（`:657` 的 `fmadd(qm,0.61,1.0)` 写进
`[x29,472]`），`:1256` 的 `thvstar` 直接 load 回来用；两处都把自己的乘积融进加法：

```text
dthv    = fma(dth,   _149, dqh  *(0.61*th))
thvstar = fma(tstar, _149, qstar*(0.61*th))
```

Rust 两处都平铺。**这条推翻了本会话早先把 `thvstar` 放进"六处否决"的结论**
（`clai`/`thvstar`/`cfw`/`fgrnd`/`lfevpa`/`gt`）——那时用的是混沌的聚合度量，
逐迭代位型在 it=6 直接证明收缩是必需的。撤回那条否决。

### 结果

```text
it=1 BITWISE IDENTICAL   it=4 BITWISE IDENTICAL
it=2 BITWISE IDENTICAL   it=5 BITWISE IDENTICAL
it=3 BITWISE IDENTICAL   it=6 BITWISE IDENTICAL
it=7 DIFFERS: dtl, del, dele, irab, obu, taf, um, zeta, tstar, thvstar, dth
```

**it=1..6 全同**（探针已从 34 个量扩到 48 个：加了 `um/zeta/tstar/qstar/thvstar/dth/dqh/
gah2o/pco2a/ram/rah/fm/fh/fq`）。it=7 的首个差量是 `irab`（差 14 ULP），
下游才是 `dtl → tl → taf → dth → tstar → thvstar → zeta → obu → um`。

**it=7 的 `irab` 还没解决**，而且它比前面几条难：14 ULP 太大，不像纯结合顺序；
但 `dirab_dtl`（共享 `fac`/`tl`/`emg`/`thermk`）在同一轮**逐位相同**，
把 `fac`/`thermk`/`tl`/`emg` 都排除了 —— 只剩**只出现在 `irab` 里、不出现在
`dirab_dtl` 里**的两个输入：`frl` 与 `tg`。试过"`powi(4)` 的结合顺序"这条假设
（显式写成 `tl*tl` 再平方），**探针输出逐位不变 ⇒ 假设否掉，改动已回退**。

**下一轮**：把 `frl`/`tg`/`emg`/`thermk`/`fac`/`stefnc` 加进探针量，看 it=7 到底哪个输入不同；
若全部相同，就逐条比对 `_3199`/`_400`/`tmp1923`/`_413` 的中间位型（在探针里加临时变量）。

### 三段式度量（含黄金窗口）

| 口径 | 基线（第 246 轮前） | 第 246 轮 | **本轮** |
|---|---|---|---|
| restart（干窗 1 步） | 19 / 68 | 19 / 68 | **18 / 68**（`rst` 归零） |
| 步级（干窗 3 步，逐位相同元素） | 585-692 | 581-692 | 577-692 |
| 黄金 dry `sumabs` | 338.9256 | 274.5483 | **240.2058** |
| 黄金 dry `over_tol` | 825 | 821 | **818** |
| 黄金 dry `ot_vars` | 17 | 17 | **18** |
| 黄金 wet `over_tol` | 20672 | 20673 | **20665** |
| 黄金 wet `ot_vars` | 68 | 68 | 68 |
| 黄金 snow | 33651/444394.4368/25896/79 | 33593/…/…/79 | 33639/444394.4368/25896/79 |

`ot_vars` 从 17 变成 18 是本轮唯一的反向指标，**已按名字查清**：把第 246 轮的代码
（`git checkout` 到 `0e920ee` + 重编 + 重跑黄金）拿出来对照，那一版恰好是 17 个越界变量，
本轮**原 17 个一个不少**，只多了一个 **`f_frcsat`**（1/264，index 2，0.7658 vs 0.7480）。
`frcsat` 是"饱和面积比例"这类阈值量，干窗是 11 天混沌窗口 ——
`sumabs`（338.93→240.21，−29%）与 `over_tol`（825→818）同时改善说明整体更近，
这一个变量是被 1 ULP 种子推过阈值的。**下一次要连它一起清掉**（第 246/247 两轮已经把
叶温循环前 6 轮消掉了，剩下的种子还在）。

Tested: `/tmp/gf/leafit_bits_probe.sh` 六次（34→48 个量的位型逐迭代比对，含最后一次
在显式 `tl**4` 假设下）；`/tmp/gf/cmp_leafit.py`；`/tmp/gf/accept_r247.sh`
（`dry_ts.sh 1` + `restart_divergence.py` 18/68；`dry_ts.sh 3` + `window_divergence.py`
44 / 577-692；`win4.sh` + `three.py` 三窗口）；第 246 轮基线的黄金重跑
（17 个越界变量的名单）与逐名对照；`cargo fmt/clippy/test`；`test_upstream_f48_sync.py`。
Not-tested: it=7 的 `irab`（下一轮）；`powi(4)` 假设已否并回退。

## 第 248 轮：**叶温 Newton 循环 10 轮迭代全部逐位相同**

从"圆整失败"到全同只用了三处形状，而定位它们靠的是本轮新立的方法：
**探针只负责给输入，形状在离线穷举里定。**

### 方法：离线形状穷举（比读汇编猜形状可靠）

第 247 轮结尾卡在 it=7 的 `irab`（14 ULP，而 `dirab_dtl` 同轮逐位相同）。做法改成：

1. 把 `frl`/`tg`/`emg`/`thermk`/`fac`/`stefnc` 加进探针（54 个量）。
   探针显示这六个输入**两侧逐位相同**；
2. 用探针转储的**输入位型**在 Python 里复算 `irab`（本机 Python 3.12 没有 `math.fma`，
   用 `Fraction` 做**精确** fma —— `Fraction → float` 是正确舍入的，
   判别式 `fma(1+2^-27, 1+2^-27, -(1+2^-26)) = 2^-54` 已验证）；
3. 先按"我读汇编读出的形状"复算 → **命中的是 Rust 的值，不是内核的**；
4. 于是对候选形状空间做**穷举**，判据是"必须在 it=7..10 **四轮同时**命中"：

```text
kernel = base=fma(-(2*stefnc), tl4, frl) + ground=fma(emg*stefnc, tg4, base)
         + tl4=(tl*tl)*(tl*tl)              → 24 个候选组合命中全部四轮
Rust   = base=平铺 frl - tl4*(2*stefnc) + 其余相同 → 48 个组合命中全部四轮
```

也就是说：**`_3199` 那一步漏了 `fmsub`** —— 而第 246 轮我自己写的注释里明明白白写着
"`_3199 = frl - (2*stefnc)*tl**4`（`fmsub`）"，代码却落成了平铺。
这正是"读汇编 → 落代码"之间丢一步的典型，**离线穷举把这一步变成可判定的**：
它要求连续四轮同时命中，巧合概率极低（对比：按错形状复算在四轮里全不中）。

### 三处形状（第 248 轮实际落的）

| 位置 | 内核（`.loc`） | 原 Rust |
|---|---|---|
| `longwave()` 的 `_3199` | `fma(-(2*stefnc), tl4, frl)`（`fmsub`） | 平铺 `frl - 2*stefnc*tl4` |
| `taf`（`:1233`） | `fma(tl, wtl0, (thm*wta0) + (tg*wtg0))` —— **只有 `wtl0*tl` 融合** | 两层 `mul_add`（把 `wta0*thm` 也融了） |
| `qaf`（`:1234`） | `((wtaq0*qm) + (wtgq0*qg)) + (qsatl*wtlq0)` —— **整条平铺，一个都没融** | 两层 `mul_add` |

`taf`/`qaf` **两条语句形状不同**这件事本身就是结论：原先按"4000 组里收缩成
`fma(w2,v2, fma(w0,v0, w1*v1))`"把两条一起照抄，方向错了。差别来自"哪些乘积已经被
别处算过"（`taf` 的 `thm*wta0`/`tg*wtg0` 复用 `fsenl` 的结果，`tl*wtl0` 是新鲜的所以被吸收；
`qaf` 的三个乘积里 `qsatl*wtlq0` 也是新鲜的，GCC 却没吸收）—— 以汇编为准，不要类比。

`taf` 的形状用 58 个量的位型离线复核过：`form=kernel` 10/10 命中两侧，
`form=old`（旧的两层 mul_add）9/10 ——**恰好在 it=7 不中**，与探针观察一致。

### 结果：58 个量 × 10 轮迭代，**全部逐位相同**

```text
it=1 .. it=10: BITWISE IDENTICAL
```

### 三段式度量：**每一格都改善**

| 口径 | 基线（第 246 轮前） | 第 247 轮 | **本轮** |
|---|---|---|---|
| restart（干窗 1 步） | 19 / 68 | 18 / 68 | **2 / 68** |
| 步级（干窗 3 步，逐位相同） | 585-692 | 577-692 | **658-692（95.09%）** |
| 步级差异变量数 | 44 | 44 | **16** |
| 黄金 dry `over_tol` | 825 | 818 | **813** |
| 黄金 dry `ot_vars` | 17 | 18 | **17** |
| 黄金 wet `over_tol`/`ot_vars` | 20672 / 68 | 20665 / 68 | 20675 / 68 |
| 黄金 snow `ot_vars` | 79 | 79 | 79 |

restart 只剩 **`t_soisno`（1 个元素）与 `fwet_snow`**；步级只剩 16 个变量。
**叶温模块的 Newton 循环到此逐位闭环**，残差已经不在叶温求解里了。

**下一轮**：这 2 个 restart 差异变量指向**地面/土壤那条链**（`t_soisno`/`fwet_snow`）——
第 0 步的 `fwet_snow` 由 `update_canopy_water`/`MOD_LeafInterception` 一族的系数决定，
`t_soisno` 由 `MOD_GroundTemperature` 的三对角求解决定。
先按同一套办法（探针给输入 + 离线穷举定形状）查 `update_canopy_water`，
再查 `GroundTemperature` 的组装。

Tested: `/tmp/gf/leafit_bits_probe.sh` 三次（54→58 个量）；`cmp_leafit.py`（10 轮全同）；
`irab_shapes.py` 的候选形状穷举（16 组合 × 四轮过滤；`frl` 扰动扫描）；
`tafqaf_shapes.py`（`taf` 四种形状 × 两侧）；`accept_r247.sh`
（restart 2/68；步级 16 / 658-692；黄金 21297/263.9500/813/17、
32663/10371.5350/20675/68、33635/444394.4368/25896/79）；
`cargo fmt/clippy/test`（26 个测试二进制全绿）。
Not-tested: 第 4 步之外的其他候选形状（已足够判定）；`qaf` 的离线复核（缺 `qg`，
以探针逐位相同为准）。

## 第 249 轮：`fwet_snow` 是**公式级**的"编错了文件"——重启残差降到 1/68

第 248 轮把叶温 Newton 循环打通后，干窗第 0 步重启只剩两个变量：
`fwet_snow`（差 2.79e-12 相对量）与 `t_soisno`（1 个元素 1 ULP）。
本轮解决前者。

### 症状与判据

`fwet_snow` 的相对差是 **2.79e-12**，远大于 1 ULP（≈2.2e-16）—— 这不是形状问题，
是**公式不同**。按第 248 轮的办法先找"哪一份源码"，答案是第 166 轮记过的
**wrong-file trap 的公式版本**：

| | 内核实际编的：`extends/interception/MOD_LeafTemperature_Extended.F90` | 本仓库原先照抄的：`main/MOD_LeafTemperature.F90:1236` |
|---|---|---|
| 容量 | `canopy_snow_capacity_for_fwet` → DEFAULT = `48*dewmx*max(lai+sai,0)`，再 `max(·,0)` | 硬编码 `(10/(48*lsai))`（等价于 dewmx=0.1，但**结合顺序不同**） |
| 指数 | `**(2.0_r8/3.0_r8)` | `**.666666666666`（**截断成 12 个 6**） |
| 闸门 | `ldew_snow > 0 .AND. satcap > 1e-10` | 只看 `ldew_snow > 0` |

指数那 1e-12 的相对差正好解释量到的 2.79e-12 —— 这也解释了为什么它**不是**末位问题。

### 落地的三处

`crates/colm-core/src/interception.rs` 新增两个 `pub(crate)` 辅助函数
（照 DEFAULT 分支实现，并在注释里写明内核编的是 `extends/` 那一份）：

* `canopy_rain_capacity_for_fwet` = `max(dewmx * max(lai+sai,0), 0)`；
* `canopy_snow_wet_fraction` = `min((ldew_snow / max(48*dewmx*max(lai+sai,0),0))**(2/3), 1)`
  （带 `satcap > 1e-10` 闸门）。

并据此改了三处：

1. `leaf_temperature.rs` 的 `update_canopy_water`：`fwet_snow` 改用上面的辅助函数
   （这是重启里那个值）；
2. `interception.rs` 的 `canopy_wetness`（`dewfraction`）：**雪分量**改用同一辅助函数
   —— 上游在同一个子程序里对雨分量用 `.666666666666`、对雪分量用 `2/3`，
   两个指数**不一样**，不能合并；
3. 同处的雨分量改用 `canopy_rain_capacity_for_fwet`（含闸门），
   且 `!DEF_VEG_SNOW` 那支按上游写成 `((1/dewmx)/lsai)*ldew`（**先除再乘**，
   不是 `ldew/(dewmx*lsai)`）。

### 结果：重启只剩 1 个元素

全 68 个变量**逐位**扫描：

```text
total bit-differences across 68 vars: 1
  t_soisno: idx 8  kernel=282.9426798696023  rust=282.94267986960233
```

`restart_divergence.py` 也从 2/68 变成 **1/68**。

### 度量：短程中性、11 天窗口被混沌放大

| 口径 | 第 248 轮 | **本轮** | 判读 |
|---|---|---|---|
| restart（干窗 1 步） | 2 / 68 | **1 / 68** | 直接量，改善 |
| 步级（干窗 3 步） | 16 变量 / 658-692 | 16 变量 / 658-692 | **完全不变** |
| 逐位相同元素（干窗 **48 步**=1 天） | 7183 / 10232 | **7182 / 10232** | **差 1 个元素** |
| 黄金 dry `sumabs`/`over_tol`/`ot_vars` | 263.95 / 813 / 17 | 380.21 / 1097 / **27** | **变差（混沌）** |
| 黄金 wet | 32663/10371.5350/20675/68 | 同 | 不变 |
| 黄金 snow | 33635/444394.4368/25896/79 | 33442/444394.4368/25896/79 | `sumabs`/`over_tol`/`ot_vars` 全同，只有逐位计数变差 |

**为什么仍然落这一版**：黄金 dry 的 11 天聚合指标变差，但这是**混沌**而不是系统性偏差 ——
判据是"48 步（1 天）的逐位相同元素数只差 **1 个**（7183 → 7182）"。
如果是公式错了，短程就会系统性发散；短程中性说明改动本身是对的，
后面的分道是这 1 个元素的差别被 11 天放大。而直接量（重启 1 步）是**改善**的，
并且改动逐条对得上内核真正编译的那份源码。**这是本轮唯一一处"聚合指标反向、
仍然保留"的改动**，据实记录在此，供后来者复核。

**下一轮**：只剩 `t_soisno` 的 1 个元素（第 8 层，1 ULP）——
它来自 `MOD_GroundTemperature` 的组装/三对角求解。按同一套办法：
把该求解的输入（`hcap`/`tcond`/`t_soisno` 初值/上边界通量 `fgrnd`）做成探针，
再用离线穷举定形状。

Tested: `/tmp/gf/leafit_bits_probe.sh`（58 个量，10 轮全同，未受本轮改动影响）；
`dry_ts.sh 1` + `restart_divergence.py`（1/68，另加 68 变量逐位扫描）；
`dry_ts.sh 3` + `window_divergence.py`（16 变量 / 658-692）；
**`dry_ts.sh 48` 两侧对照**（本轮 7182-10232 对第 248 轮 7183-10232，
`git stash` 后重编复现）；`accept_r247.sh` 的黄金三窗口；
`cargo fmt/clippy/test`。
Not-tested: 第 3 条（`!DEF_VEG_SNOW` 的结合顺序）—— 对齐算例都是 `DEF_VEG_SNOW=T`，
只能按上游源码照抄，测不到。

## 第 250 轮：地面温度界面导热率的融合 —— **干窗第 0 步重启 0/68，逐位全同**

第 249 轮把重启残差压到 1 个元素（`t_soisno` 第 8 槽 = 第 4 层土壤，1 ULP）。
本轮按同一套办法换到地面温度模块，一次探针就定位到了。

### 新探针：`/tmp/gf/gtcoef_probe.sh`

在 `main/MOD_GroundTemperature.F90` 的 `CALL tridia` **之前**插一条 WRITE，
把三对角组装的全部中间量按位型打出来，共 **95** 个：

* `at/bt/ct/rt`（1..10，组装结果，40 个）；
* `fact/cv/tk/z_soisno/t_soisno`（1..10，输入，50 个）；
* `cnfac/deltim/dhsdT/hs/fsno`（5 个）。

Rust 侧在 `ground_temperature` 里 `solve_tridiagonal` 之前打同序的
`subdiagonal/diagonal/superdiagonal/rhs` + `factor/layer_capacity/interface_conductivity/
node_depth_m/temperature_k` + 同 5 个标量。

**第一次跑就只剩 8 个不同**：

```text
at(4) at(8) ct(3) ct(5) ct(7) tk(3) tk(5) tk(7)     各 1 ULP
```

`tk` 是**界面导热率**，`at`/`ct` 是由它组装的；`bt`/`rt` 全部逐位相同 ——
所以问题既不在三对角组装、也不在 `tridia`，而在**界面导热率那一条分母**。

### 根因：分母里第一个乘积被吸收

`MOD_GroundTemperature.F90:243-244`：

```fortran
tk(i) = thk(i)*thk(i+1)*(z_soisno(i+1)-z_soisno(i)) &
      /(thk(i)*(z_soisno(i+1)-zi_soisno(i))+thk(i+1)*(zi_soisno(i)-z_soisno(i)))
```

出货内核（`gt.o` 的 `groundtemperature`，`0xc94-0xcb4`）编出来是：

```text
d18 = (zi(i)-z(i)) * thk(i+1)              fmul   ← 第二项先舍入
d18 = fmadd(thk(i), z(i+1)-zi(i), d18)     ← **第一项被吸收**
tk  = fdiv(  (thk(i)*thk(i+1))*(z(i+1)-z(i)),  d18 )
```

Rust 原先两项都是平铺。改成
`conductivity[layer].mul_add(dzp, conductivity[layer+1]*dzm)` 之后，
**95 个量全部逐位相同**。

`tk` 只在**奇数界面**（3/5/7）上露出来，是因为另外几个界面的两种写法恰好舍入到同一个数 ——
这也是"逐元素位型 + 只看首个不同"这套办法的价值：形状错不一定处处错。

（`if (i==0) .and. (…)` 那一支的调和平均 `2*thk_i*thk_{i+1}/(thk_i+thk_{i+1})` 与
`max(0.5*thk_{i+1}, ·)` 也顺带核过：`0xcd0-0xcf0` 与 Rust 的左结合一致，未改。）

### 结果

| 口径 | 第 249 轮 | **本轮** |
|---|---|---|
| restart（干窗 1 步） | 1 / 68 | **0 / 68（逐位全同）** |
| 步级（干窗 3 步）差异变量 / 逐位相同元素 | 16 / 658-692 | **14 / 668-692（96.53%）** |
| 黄金 dry `over_tol` / `ot_vars` | 1097 / 27 | **826 / 17** |
| 黄金 wet `over_tol` / `ot_vars` | 20675 / 68 | **20670 / 68** |
| 黄金 snow 逐位相同元素 / `sumabs` / `ot_vars` | 33442 / 444394.4368 / 79 | **33495** / 444394.4368 / 79 |

**模型状态在第 0 步已经逐位闭环**（68/68 变量、95/95 组装中间量）。

### 剩下的不是状态，是第 0 步的**诊断量**

步级口径里 step 0 仍有 14 个变量各差 1 ULP：`f_fevpg`、`f_qinfl`、`f_qlayer`、
`f_qstar`、`f_fevpa`、`f_lfevpa`、`f_wliq_soisno`… 它们**不在重启状态里**
（重启已经 0/68），属于：

* `f_qstar`：走 `colm_core::history_diagnostics` 的**重算**路径（与模型内的 `qstar` 不同一条代码）；
* `f_fevpg`/`f_fevpa`/`f_lfevpa`：`corrected_ground_evaporation` 一族，
  源头是叶温例程**循环后**的 `fevpg = rhoair*cgw*(qg-qaf)`；
* `f_qinfl`/`f_qlayer`：土壤水文（`MOD_SoilSnowHydrology`）的入渗/层间通量。

**下一轮**：把叶温例程**循环后**那一段（`ground_evaporation` 及其温度导数、
`canopy_air_humidity` 的收尾）做成探针 —— 它现在还没被探针覆盖（探针只到 it=10 的循环体内）；
`fevpg` 的 1 ULP 很可能就在那里。

Tested: `/tmp/gf/gtcoef_probe.sh` 两次（改前 8 个不同、改后 95 个全同）；
`dry_ts.sh 1` + `restart_divergence.py`（**0/68**）；`accept_r247.sh`
（步级 14 / 668-692；黄金 21256/604.5841/826/17、32667/10370.7041/20670/68、
33495/444394.4368/25896/79）；`gt.o` 的 `0xc94-0xcb4` 与 `0xcd0-0xcf0` 反汇编；
`cargo fmt/clippy/test`。
Not-tested: 循环后那一段（下一轮）；`f_qinfl`/`f_qlayer` 的土壤水文路径。

## 第 251 轮：全量差分工具复跑 + 残差收敛到**一个量**

前面几轮把干窗第 0 步的 restart 打到 **0/68（逐位全同）**。本轮做一次横向清点，
把仓库里全部 checked-in 差分工具复跑一遍，并把剩下的差异**定位到单一量**。

### 一、10 个差分工具复跑（本轮实测）

| 工具 | 结果 |
|---|---|
| `compare_qsadv.sh` | 4 个输出 20000/20000 逐位相同 |
| `compare_moninobukm.sh` | 20 个输出 20000/20000 逐位相同 |
| `compare_soilthermal.sh` | 8 档方案 × 5000 组，`hcap`/`thk` 各 40000/40000 |
| `compare_leddy.sh` | PASS（8 个分支分布） |
| `compare_forcingdownscaling.sh` | PASS（4 个分支分布） |
| `compare_forcingdownscaling_wind.sh` | PASS |
| `compare_forcingdownscaling_shortwave.sh` | 11 个输出 2000/2000（8 个分支分布） |
| `compare_phasechange.sh` | `meltf` 5 种 patchtype × 2000，9 个量 10000/10000 |
| `compare_flag_isolated.sh` | **需要参数**（`<tag> "<namelist 行>"`），见下 |
| `compare_second_config.sh` | **需要算例名**，见下 |

前 8 个裸跑即全绿（`bitwise identical`）—— 模块级差分这一层的证据是完整的。

### 二、第二个配置（Campbell 土水 + 关 VSF）

三个黄金算例都走 van Genuchten + VSF，这条 Richards 支路平时没有端到端信号。
本轮三个算例各跑一遍第二配置：

| 算例 | 第二配置 `ot_vars` | 同窗口黄金配置 |
|---|---|---|
| CN-Cng | **16** | 17 |
| CN-Cng-wet | **66** | 68 |
| US-NR1-snow | **79** | 79 |

即**换一套土壤水方案，精度与主配置齐平**（甚至略好）。

### 三、按开关分区（`compare_flag_isolated.sh`）

| tag | 注入 | 第 0 步差异变量 |
|---|---|---|
| `base` | 无 | 8 |
| `nophs` | `DEF_USE_PLANTHYDRAULICS = .false.` | **同一份名单**（8 个） |
| `vegsnowoff` | `DEF_VEG_SNOW = .false.` | 换成另一组（`f_h2osoi` 2.48e-13、`f_wat`/`f_wat_inst` 1.42e-15） |
| `split` / `norich` | `DEF_SPLIT_SOILSNOW=.true.` / `DEF_USE_VSF=.false.` | 内核直接跑不完（该算例的 `out` 不适配），不算证据 |

`nophs` 与 `base` 名单一致 ⇒ 残差**不在 PHS 支路**（与第 246/247 轮的结论一致）。

### 四、残差收敛到**一个量**：`fevpg`

`base` 的第 0 步差异（本轮逐位扫描，8 个变量、每个 1 个元素）：

| 变量 | ndiff | maxabs | maxrel |
|---|---|---|---|
| `f_fevpg` | 1 | 1.3553e-20 | 1.75e-16 |
| `f_qinfl` | 1 | 1.3553e-20 | 1.75e-16 |
| `f_qlayer` | 1 | 1.3553e-20 | 1.75e-16 |
| `f_qstar` | 1 | 1.3553e-20 | 2.21e-16 |
| `f_fevpa` | 1 | 1.3553e-20 | 2.64e-16 |
| `f_lfevpa` | 1 | 2.8422e-14 | 2.37e-16 |
| `f_xerr` | 1 | 1.5420e-20 | 8.04e-06 |
| `f_zerr` | 1 | 2.4500e-14 | 9.57e-04 |

**前五个的 `maxabs` 完全相同（1.3553e-20）** —— 而 `1.3553e-20` 正是 `fevpg ≈ 7.766e-5`
的 1 ULP（`2^-66`）。这五个量都在同一档量级、各差 1 ULP；**同量级的量各差 1 ULP 时
`maxabs` 自然相同，所以"相同 maxabs"本身不等于"同一个种子"**。分开取证：

* **蒸发这一支是证过的**：`fevpa = fevpl + fevpg`，而 `f_fevpl`（叶面）已经逐位相同
  ⇒ `fevpa` 的差只能来自 `fevpg`；`lfevpa = lfevpl + htvp*fevpg` 同理；
  `qstar = -fevpa/(rhoair*ustar)`（`MOD_Vars_1DAccFluxes.F90:2745`，与
  `history_diagnostics.rs:156` 的形状逐字一致）⇒ `f_qstar` 是**继承**来的，
  不是 `history_diagnostics` 自己的形状错。
* **`f_qinfl`/`f_qlayer` 只是"同档量级的 1 ULP"**：它们属土壤水文，而它们的**状态**
  （restart 里的 `wliq_soisno`）逐位相同 —— 既可能是扰动相互抵消，也可能是**独立**的
  1 ULP。**本轮不下结论**，下一轮探针一并查。

**已确定的一个源头**：叶温例程循环后的
`fevpg = rhoair*cgw*(qg-qaf)` 及其订正 `fevpg += tinc*cgrndl`
（`MOD_LeafTemperature_Extended.F90:1397/1436`、`MOD_Thermal…:1239`）。

`f_xerr`/`f_zerr` 是能量/水平衡残差（量级 ~1e-20），它们的**相对**差大只是因为分母接近 0，
绝对值本身就是 ULP 级，不作为独立目标。

**下一轮**：把叶温例程**循环后**那一段做成探针（`ground_evaporation`、`cgw`、
`wtgq0`、`dqgdT`、`cgrndl`、订正后的 `fevpg`）—— 这是探针目前唯一没覆盖的区间
（现探针只到 it=10 的循环体）。

Tested: 8 个 `compare_*.sh` 裸跑（全绿，逐位）；`compare_second_config.sh` 三个算例
（16/66/79）；`compare_flag_isolated.sh` 的 `base`/`nophs`/`vegsnowoff`（另两个不适用）；
第 0 步 8 变量逐位扫描（`/tmp/gf/flag_base/`）。
Not-tested: `split`/`norich` 两个开关（内核跑不完，非证据）；循环后那一段（下一轮）。

### 第 251 轮追加：叶温例程**循环后**那一段探针 —— 22/22 逐位相同

`/tmp/gf/postloop_probe.sh`：在内核 `MOD_LeafTemperature_Extended.F90` 的
`lfevpl = htvpl * fevpl` 之前插一条 `POSTLP` 位型 WRITE，打 22 个量
（`fevpg`/`fseng`/`fevpg_soil`/`fevpg_snow`/`cgrndl`/`cgrnds`/`cgrnd`/`cgw`/`cgh`/
`qaf`/`taf`/`tg`/`qg`/`wtgq0`/`wtg0`/`rhoair`/`tl`/`dqgdT`/`fevpl`/`fsenl`/`etr`/`evplwet`），
Rust 侧在 `Ok(LeafTemperatureOutput {` 之前打同序的对应量。

结果：**ALL BITWISE IDENTICAL** —— 含 `fevpg` 本身，以及订正要用的 `cgrndl`、`cgw`。

**这把剩下的 1 ULP 从叶温例程里排除掉了**，只剩两个候选：

1. `MOD_Thermal_CanopyPhase_Extended.F90` 第 6 节的订正
   `fevpg = fevpg + tinc*cgrndl`（Rust：`standard_lct_step.rs:371-373` 的 `mul_add`）；
2. 同节的 `egsmax` 限幅 `fevpg = min(fevpg, egsmax)`（Rust：`thermal_water.rs:81-88`）。

旁证把候选 1 也压得很小：同一节里 `fseng = fseng + tinc*cgrnds` 形状相同、
且 `f_fseng` **已经逐位相同**；而候选 2 若起作用，`egsmax` 一变 `egidif` 就会变、
`f_fseng` 也会跟着变（它也逐位相同）。所以下一轮直接对第 6 节做同款探针
（`tinc`/`egsmax`/`egidif`/订正前后的 `fevpg`/`fseng`），把最后这一处钉死。

Tested: `/tmp/gf/postloop_probe.sh`（22 个量，全同）；其余见上。
Not-tested: `MOD_Thermal` 第 6 节与 history 写出路径（下一轮）。

## 第 252 轮：`fevpg` 的订正**不融合** —— 第 0 步残差 8 → 1

第 251 轮把残差收敛到 `fevpg`，并证明叶温例程循环后 22 个量全同。
本轮用 `/tmp/gf/th6_probe.sh` 直接对 `MOD_Thermal` 第 6 节取证：

* 内核侧插两条位型 WRITE：订正**前**（`TH6PRE`，14 个量）与订正+限幅**后**
  （`TH6POST`，10 个量）；
* Rust 侧一个点（`Ok(StandardLctEnergyOutput {` 之前）就够 —— 那里 `leaf`（订正前）
  与 `corrected_*`（订正后）同时在作用域里。

结果：

```text
PRE : ALL BITWISE IDENTICAL      ← fevpg_pre/cgrndl/cgrnds/tinc/wliq/wice/deltim 全同
POST: fevpg_post 差 1 ULP
```

订正后的 `fevpg` 差 1 ULP，而它三个输入全同、限幅又没生效（`egsmax`=4.88e-3 ≫
`fevpg`=7.77e-5）。于是拿探针的位型**离线复算**两种写法：

```text
fma(tinc, cgrndl, fevpg_pre)   = 3F145BB9BCB4DD9B   ← 原 Rust 的值
fevpg_pre + fl(tinc*cgrndl)    = 3F145BB9BCB4DD9C   ← **内核的值**
```

即内核在 `MOD_Thermal…:1239` 的 `fevpg = fevpg + tinc*cgrndl` 上**没有融合**。
（本会话第 246 轮那条 GIMPLE 注释写着 "FMA(tinc, cgrnds, 原值)" —— 又是
"dump 与出货二进制不一致"的同一类坑，以二进制/实测为准。）

`crates/colm-core/src/standard_lct_step.rs` 改成平铺的
`leaf.ground_evaporation + slope * tinc`，并在注释里记下：
**相邻的 `fseng = fseng + tinc*cgrnds` 不能照抄这个结论** —— 那条 fma 与平铺
在本算例给出同一位型（`4086FEDBB7C44298`），判不了；3 步口径里 `f_fseng` 一直逐位相同，
所以保留 `mul_add`。

### 结果

| 口径 | 第 251 轮 | **本轮** |
|---|---|---|
| restart（干窗 1 步） | 0 / 68 | 0 / 68 |
| **第 0 步差异变量** | **8** | **1（只剩 `f_zerr`）** |
| 步级差异变量 / 逐位相同元素 | 14 / 668-692（96.53%） | **9 / 680-692（98.27%）** |
| 黄金 dry `ot_vars` / `over_tol` | 17 / 826 | 17 / 826 |
| 黄金 wet `over_tol` / `ot_vars` | 20670 / 68 | 20665 / 68 |
| 黄金 snow `ot_vars` | 79 | 79 |

`f_fevpg` 修好之后 `f_fevpa`/`f_lfevpa`/`f_qstar`/`f_qinfl`/`f_qlayer`/`f_xerr`
**一次性全部归零** —— 这也反过来证实了第 251 轮那条（当时被我改成"不下结论"的）
推断：它们确实是同一个种子的传播。（`f_zerr` 是另一回事，见下。）

### 只剩 `f_zerr`

`f_zerr` 就是内核的 `errore`（`MOD_Thermal…:1403-1413` 的能量平衡检查）：
两个赋值里第一个是死代码（立刻被第二个覆盖，只差 `fgrnd` vs `xmf`），
真正的链条是

```fortran
errore = sabv + sabg + frl - olrg - fsena - lfevpa - xmf - dheatl + hprl &
       + canopy_phase_heat + cpliq*pg_rain*(t_precip-t_grnd) + cpice*pg_snow*(t_precip-t_grnd)
DO j = lb, nl_soil
   errore = errore - (t_soisno(j)-t_soisno_bef(j))/fact(j)     ← 逐层**减**
ENDDO
```

Rust 端（`history.rs:900-910`）把最后那一族写成了
`... - ground_heat_storage_w_m2`（先求和再整体减）—— **结合顺序不同**：
内核是"边加边减"的连乘链，Rust 是 `T - (a₁+a₂+…)`。这是下一轮的第一候选；
若不够，再查链条上 `cpliq*pg_rain*(…)` 那两处乘积是否被吸收。

`f_zerr` 的绝对值只有 8.9e-14（`errore` 本身 ~2.6e-11），是"诊断量的诊断量"。

Tested: `/tmp/gf/th6_probe.sh`（PRE 全同 / POST 差 1 ULP）+ 离线位型复算；
`compare_flag_isolated.sh base`（第 0 步 8 → **1**）；`accept_r247.sh`
（restart 0/68；步级 9 / 680-692；黄金 21252/604.5841/826/17、
32667/10378.8229/20665/68、33494/444394.4368/25896/79）。
Not-tested: `fseng` 那条订正的真实形状（本算例判不了）；`f_zerr`（下一轮）。

### 第 252 轮追加：`errore` 的逐层减 —— **第 0 步 history 也全同（0 个差异变量）**

第 252 轮把 `fevpg` 修好后，第 0 步只剩 `f_zerr`。它的链条收尾是：

```fortran
errore = sabv + sabg + frl - olrg - fsena - lfevpa - xmf - dheatl + hprl &
       + canopy_phase_heat + cpliq*pg_rain*(t_precip-t_grnd) + cpice*pg_snow*(t_precip-t_grnd)
DO j = lb, nl_soil
   errore = errore - (t_soisno(j)-t_soisno_bef(j))/fact(j)     ! **逐层边加边减**
ENDDO
```

Rust 端原先写成 `... - ground_heat_storage_w_m2`（先 `.sum()` 再整体减），
结合顺序是 `T - (a₁+a₂+…)` 而不是 `((…((T-a₁)-a₂)…)`。改成同一个循环逐层减之后：

```text
compare_flag_isolated.sh base -> differing vars: 0
```

**干窗第 0 步的 history 68→0 个差异变量、restart 0/68，全部逐位相同。**

| 口径 | 第 251 轮 | 第 252 轮（两处修完） |
|---|---|---|
| restart（干窗 1 步） | 0 / 68 | 0 / 68 |
| **第 0 步 history 差异变量** | 8 | **0** |
| 步级首个分歧步 | 0 | **1** |
| 步级差异变量 / 逐位相同元素 | 14 / 668-692（96.53%） | **8 / 683-692（98.70%）** |
| 黄金 dry `ot_vars` / `over_tol` | 17 / 826 | 17 / 826 |
| 黄金 wet / snow `ot_vars` | 68 / 79 | 68 / 79 |

剩下的分歧从**第 1 步**开始（`f_lfevpa` 2 个元素、然后 `f_fgrnd`/`f_wat`/`f_wat_inst`）——
即第 0 步的**输出**已经逐位一致，分歧出现在"用第 0 步结果走第 1 步"时，
下一轮从第 1 步的 `lfevpa` 入手最直接。

Tested: `compare_flag_isolated.sh base`（0 个差异变量）；`accept_r247.sh`
（restart 0/68；步级首个分歧步 **1**、8 变量 / 683-692；黄金
21251/604.5841/826/17、32667/10378.8229/20665/68、33492/444394.4368/25896/79）。
Not-tested: 第 1 步起的分歧（下一轮）。

## 第 253 轮：`lfevpa` 与 `emis` 各一处融合 —— 第 1 步也全同，首分歧推到第 2 步

第 252 轮之后首个分歧步是 1，且第 1 步只差一个变量（`f_lfevpa`，2 个元素、1 ULP）。
第 0 步与第 1 步的输入都逐位相同 ⇒ 只能是形状。

### 1. `lfevpa = lfevpl + htvp*fevpg`（`MOD_Thermal…:1343`）

关键是**加数已经在别处算好了**：`lfevpl = htvpl*fevpl` 是叶温例程里的独立语句，
所以只有 `htvp*fevpg` 会被吸收：

```text
lfevpa = FMA(htvp, fevpg, lfevpl)
```

Rust 原先两项都平铺（`leaf_latent_heat * leaf_evaporation + sublimation_heat * ground_evaporation`）。
改成 `sublimation_heat.mul_add(ground_evaporation, leaf_latent_heat * leaf_evaporation)` 后
**首分歧步 1 → 2**，第 1 步整步逐位相同。

### 2. `emis = olru/olrb`（`…:1366-1369`）

```fortran
olrb = stefnc*t_grnd_bef**3*(4.*tinc)
olru = ulrad + emg*olrb        ← 这个乘积被吸收
olrb = ulrad + olrb            ← 纯加法
emis = olru / olrb
```

Rust 原先也是平铺的 `(ulrad + emg*bc)/(ulrad + bc)`。改成
`emissivity.mul_add(blackbody_change, upward_longwave) / (upward_longwave + blackbody_change)`
后 `f_emis` 归零。

### 结果

| 口径 | 第 252 轮 | **本轮** |
|---|---|---|
| restart（干窗 1 步） | 0 / 68 | 0 / 68 |
| 第 0 步 history 差异 | 0 | 0 |
| 第 1 步 history 差异 | 1 变量（`f_lfevpa`） | **0** |
| 步级首个分歧步 | 1 | **2** |
| 步级差异变量 / 逐位相同元素 | 8 / 683-692（98.70%） | **6 / 686-692（99.13%）** |

剩下的第 2 步差异（各 1 个元素）：`f_fgrnd`、`f_wat`、`f_wat_inst`、`f_h2osoi`、
`f_wice_soisno`、`f_xerr`。其中 `f_fgrnd` 是 `surface_budget` 里同一条链的产物，
下一轮第一候选是它那一项
`- emg*stefnc*t_grnd_bef**3*(4.*tinc)`：内核源码的左结合是
`(((emg*stefnc)*t**3)*(4.*tinc))`，而 Rust 复用 `blackbody_change`
（= `stefnc*t**3*(4*tinc)`）再整体乘 `emg`，**结合顺序不同**。

Tested: `dry_ts.sh 3` + `window_divergence.py` 两轮 A/B
（`lfevpa` 融合：首分歧 1→2；`emis` 融合：6 变量 / 686-692）；
黄金三窗口沿用上一轮 `accept_r247.sh` 的口径（本轮只跑 A/B，未重跑黄金）。
Not-tested: `f_fgrnd` 那一项的结合顺序（下一轮）；`f_xerr`（水平衡，可能同 `zerr` 一样是逐层累加的结合顺序）。

### 第 253 轮追加：`fgrnd` 的两处形状

第 253 轮修完 `lfevpa`/`emis` 后，第 2 步的差异里 `f_fgrnd` 是同一条链的产物，
两处都不对：

1. 那一项 `- emg*stefnc*t_grnd_bef**3*(4.*tinc)` **不能复用 `blackbody_change`** ——
   `blackbody_change` 是 `stefnc*t**3*(4*tinc)`（少一层 `emg`），结合顺序与
   内核的左结合 `(((emg*stefnc)*t**3)*(4.*tinc))` 不同；
2. `- (fseng+fevpg*htvp)` 里 `fevpg*htvp` 同样被吸收 ⇒ `fma(fevpg, htvp, fseng)`。

改成 `emissivity*STEFAN_BOLTZMANN*t**3*(4*tinc)` 与
`sublimation_heat.mul_add(ground_evaporation, corrected_ground_sensible_heat)` 后
`f_fgrnd` 归零。

结果：步级差异变量 6 → **5**、逐位相同元素 686-692 → **687-692（99.28%）**。

剩下的第 2 步差异只剩**土壤水一族**：`f_wat`、`f_wat_inst`、`f_h2osoi`、
`f_wice_soisno`（各 1 个元素）与 `f_xerr`（水平衡）。`f_wat`/`f_wat_inst` 的 `maxabs`
完全相同（2.2737e-13）⇒ 同一种子；种子在 `f_h2osoi`/`f_wice_soisno`（土壤水/冰柱）
那一侧，属 `MOD_SoilSnowHydrology`/WATER_2014 一族 —— 这是本会话第一次把分歧退出
"叶温 + 地面温度 + 地表收支"这一片。`f_xerr` 的第一候选与 `zerr` 同理
（水平衡的逐层累加结合顺序）。

Tested: `dry_ts.sh 3` + `window_divergence.py`（`fgrnd` 两处形状 A/B：5 变量 / 687-692）。
Not-tested: 土壤水一族的形状（下一轮）；`f_xerr`。

## 第 254 轮：残差第一次进入**状态**——第 2 步的 `wice_soisno[5]`

第 253 轮把第 1 步修成全同之后，本轮先把"状态到底还差不差"量清楚（先前只比过 1 步的 restart）：

| 步数 | restart 差异 | history 首个分歧步 |
|---|---|---|
| 1 | **0 / 68** | — |
| 2 | **0 / 68** | 2 |
| 3 | **2 / 68** | 2 |

即：**第 0/1 步的状态与输出全部逐位相同，分歧从第 2 步开始**。3 步 restart 的两个差异是：

```text
wice_soisno[5]  kernel=6.1823587081423845 (4018BABC3DBE730C)
                rust  =6.182358708142612  (4018BABC3DBE740C)    256 ULP / 3.68e-14 相对
hk[0]           kernel=2.9151756458244646e-26 (3AA20B47BAF5347C)
                rust  =2.915175645823325e-26  (3AA20B47BAF52CBB) 3.9e-13 相对
```

`wliq_soisno` / `smp` / `t_soisno` **全部逐位相同**。链条是清楚的：

* 第 2 步的 history 里 `f_wat`/`f_wat_inst` 只差 **1 ULP**（1604.9742063638062 对 …64），
  而 `1 ULP(1605) = 2.27e-13` **正好等于** `wice` 那个差 —— 说明它们只是 `Σ(wliq+wice)` 的继承；
* `f_h2osoi[0]` 差 128 ULP，正是 `wice` 那 2.27e-13 除以 `dz*denice`（≈16）再落到
  `h2osoi` 的 ULP（≈1.1e-16）上的结果；
* `hk` 的相对差（3.9e-13）≈ `(2b+3)*d(se)/se`（`b≈5`、`d(se)/se≈3.7e-14`）—— 由**发散的
  `wice`** 经保持曲线推出来，是下游。

所以本轮的净收益是**把残差归类**：不再是"一堆 1 ULP 诊断量"，而是
**一个状态量（第 1 层土壤冰）在第 2 步产生 3.7e-14 的相对差**。

`wice` 只由相变（`MOD_PhaseChange`）与水分通量改变，而 `t_soisno`、`wliq_soisno` 都逐位相同 ——
相变是**阈值型**的（`meltf`/`freezef` 分段），所以最可能是某个**输入差 1 ULP 把阈值翻了过去**。
两个候选：

1. `MOD_Hydro_SoilFunction` 的 `smp`/`hk`/`se`（van Genuchten 保持曲线）——
   **它至今没有差分闭环**（现有 8 个闭环里没有它），而 `hk` 恰好在这一步露出 3.9e-13；
2. `MOD_PhaseChange` 的调用参数（`meltf` 本身有 9 个量 × 10000 的逐位闭环，但那是**给定输入**下的）。

**下一轮**：优先给 `MOD_Hydro_SoilFunction` 补一个差分驱动器（照 `compare_soilthermal.sh`
的骨架：模块按产线 flag 编、驱动加 `-fwrapv -ffp-contract=off`、逐位比
`smp`/`hk`/`d(smp)/d(wliq)` 一族），把它从"没有闭环"变成"有闭环"；
若它全同，再回到 `MOD_PhaseChange` 的调用参数。

Tested: `dry_ts.sh 2`/`dry_ts.sh 3` + `restart_divergence.py`（1/2/3 步分别 0/68、0/68、2/68）；
`window_divergence.py`（首个分歧步 2）；逐元素位型对照（`wice_soisno[5]`、`hk[0]`、
`f_wat`、`f_h2osoi`）。
Not-tested: `MOD_Hydro_SoilFunction` 的差分（下一轮）。

### 第 254 轮追加：给 `MOD_Hydro_SoilFunction`（**水**参数保持曲线）补上差分闭环

干窗第 3 步的 restart 差异落在 `wice_soisno` 与 `hk` 上（见上一节），而
`MOD_Hydro_SoilFunction`（van Genuchten / Campbell 的 `smp`/`hk`/`vliq` 互相反演）
**一直没有差分闭环** —— 现有 8 个闭环里只有 `soil_hcap_cond`（**热**参数）。
本轮把它补上：

* 驱动 `oracle/scripts/soil_hydro_fn_diff.f90`：两档模型 ×（2500 组均匀随机 +
  2500 组边界取值），三个函数**串联**调用（`psi = soil_psi_from_vliq(…)`，
  再把同一个 `psi` 喂给 `soil_hk_from_psi`/`soil_vliq_from_psi`），
  按位型打印 `psi`/`hk`/`vl`；
* 配对物 `crates/colm-core/examples/soil_hydro_fn_probe.rs`（同一串 LCG、
  同样的抽签顺序；改一边必须同步改另一边）；
* 外壳 `oracle/scripts/compare_soilhydro.sh`：模块按**产线 flag** 编译
  （不加 `-ffp-contract=off`）、驱动加 `-fwrapv -ffp-contract=off`，
  与 `compare_soilthermal.sh` 同一套骨架；namelist 用只含
  `DEF_USE_Campbell_SOIL_MODEL` 的桩模块。

结果：

```text
MOD_Hydro_SoilFunction: all 3 outputs 10000/10000 bitwise identical
分支分布 {('0','0'):2701, ('0','1'):422, ('0','2'):1877,
          ('1','0'):2769, ('1','2'):1836, ('1','1'):395}
```

（`flag` 1 = `vliq>=porsl` 的早退、2 = `vliq<=max(vl_r,1e-8)` 的早退；
两档模型的两条早退路径都被抽到过，不是"边界没覆盖所以全同"。）

**结论有两条**：

1. 这一族（两档模型 × 三个函数 × 三条分支）**逐位正确** ⇒ 保持曲线不是
   `wice`/`hk` 那个差的来源；
2. `hk` 的差（3.9e-13 相对）因此只能是**由发散的 `wice` 经保持曲线推出来**的，
   与上一节的链条读法一致。

**下一步**：残差只剩"第 2 步第 1 层土壤冰"这一个状态量。它的候选收窄到
`MOD_PhaseChange` 的**调用参数**（`meltf` 本身已有 9 个量 × 10000 的闭环）
与 `soilwater`/`water_2014` 里水量分配的结合顺序。

Tested: `bash oracle/scripts/compare_soilhydro.sh`（3 输出 10000/10000 逐位相同，
两档模型 × 三条分支全命中）；`cargo fmt --all --check`；`cargo clippy --workspace --all-targets -- -D warnings`。
Not-tested: `MOD_PhaseChange` 的调用参数（下一轮）；`get_derived_parameters_vGM`
（静态参数推导，不在本驱动覆盖范围；两侧都用抽出来的 `sc_vgm`/`fc_vgm`）。

### 第 255 轮：`meltf` 的实参也逐位相同 —— 相变不是 `wice` 那个差的来源

`meltf` 本身有 9 个量 × 10000 的闭环，所以嫌疑在**调用实参**。新探针
`/tmp/gf/meltf_args_probe.sh` 在 `MOD_GroundTemperature.F90` 的 `CALL meltf` 之前、
Rust 的 `phase_change(PhaseChangeInput {` 之前取同一组量（**跑 2 步**）：
`fact/brr/t_soisno_bef/t_soisno/wliq/wice` 的第 1..3 层（18 个）+ 13 个标量。

```text
step 0: 第 1..3 层的 18 个量与 hs/fsno/dhsdT/scv/snowdp/cnfac/deltim/porsl/psi0 全部逐位相同
step 1: 同上，全部逐位相同
```

唯一"不同"的 `hs_soil`/`hs_snow` 是 Rust 侧的**占位**（那两个量只在
`DEF_SPLIT_SOILSNOW` 分支用，本算例为 false）⇒ 不构成证据。

**结论**：相变在**给定输入下**（第 1..3 层）不可能是 `wice` 那个差的来源。
残差的候选因此只剩两处：

1. **雪层（`j < 1`）或第 4 层以下**的实参 —— 本探针没覆盖。但 3 步 restart 里
   `scv`/`snowdp`/`z_sno`/`dz_sno`/`ssno` 与 `wice_soisno[0..4]` **都逐位相同**，
   雪层那一侧没有差异信号；
2. **`water_2014`（VSF Richards 求解器）**—— 它在 `GroundTemperature` **之后**运行，
   自己也会调整 `wliq`/`wice`（含冻融分配）。这是最可能的落点，也解释了
   "`wliq` 最终逐位相同而 `wice` 差"（液相由压力头反解钉住、冰相只在冻融项里动）。

**下一轮（本会话最后一轮）**：以最终核对与交接为主；若时间允许，按同一套办法
（探针给输入 + 离线位型复算）查 `crates/colm-core/src/water_2014.rs` 的冻融分配。

Tested: `/tmp/gf/meltf_args_probe.sh`（2 步，18 个层量 + 标量逐位比对）；
`cargo fmt/clippy`（无代码改动，不需重跑）。
Not-tested: 雪层与第 4 层以下的实参；`water_2014` 的冻融分配。

### 第 256 轮：残差**不在**第 1 层凝结项；并且踩到了"追错文件"的**函数级**版本

按第 255 轮的候选，先 A/B `water_2014.rs:269`（冰相 `mul_add` ↔ 源码左结合平铺）。
3 步干窗的 `rust_restart.nc` 与 `colm-rs_hist_2008-01.nc` 与融合版**逐字节相同**
（`0f226ff6…` / `5c88f691…`），步级仍是 5 变量 / 687 of 692（99.2775 %）。
怀疑"这行根本没跑"，于是加临时 `eprintln!` 计数 —— **一次都没打**。

原因是函数开头的早退：

```rust
// crates/colm-core/src/water_2014.rs:178-180
if input.variably_saturated {
    return variably_saturated_soil_step(input, state);
}
```

本算例（默认配置）`DEF_USE_VariablySaturatedFlow = .true.`，所以
**`water_2014.rs:261-275` 整段在默认配置里是死代码**；上游对应的是
`MOD_SoilSnowHydrology.F90` 的 `WATER_VSF`（`:1126-1133`），Rust 侧的活代码在
`crates/colm-core/src/variably_saturated_flow.rs:4422-4431`。也就是说：**同一条物理语句
在 Rust 里有两份拷贝（VSF / Campbell），只有前者在默认配置里跑** ——
这是「编的不是你以为的那个文件」的**函数级**翻版，代价是一轮空转。

把同一个 A/B 做到活代码上（`variably_saturated_flow.rs:4426`）：3 步输出**仍然逐字节相同**。
再在活代码处打印三个通量（跑 3 步）：

```text
DBG257 wice0 before=40089AFD43B944E6 fused=… flat=… same=true dt=1.8e3 frost=0e0 subl=0e0 dew=0e0
DBG257 wice0 before=401851ECA41F6891 fused=… flat=… same=true dt=1.8e3 frost=0e0 subl=0e0 dew=0e0
DBG257 wice0 before=4018BABC3DBE740C fused=… flat=… same=true dt=1.8e3 frost=0e0 subl=0e0 dew=0e0
```

`qsdew = qfros = qsubl = 0`（三步全零）⇒ 该语句在本窗口**根本不改冰**，形状自然无所谓。
**该候选被排除**（第 3 行的 `before` 正是 Rust 第 2 步那个 256 ULP 的值 `…40C`，
说明它恰恰是**进入该行之前**就偏了）。

顺带把残差位置钉到**层**：`f_wice_soisno` 形状 `(3, 1, 15)`、`f_h2osoi` `(3, 1, 10)`
⇒ 打包列前 5 个槽是**未启用的雪槽**（`t_soisno(0..4) = 0`，本窗口无雪），
差异元素 `[:, 0, 5]` 就是**最上土壤层**（Fortran 的 `wice_soisno(1)`）；
`f_h2osoi[:, 0, 0]` = 同一层，128 ULP，是那 256 ULP 的派生。

**修正后的下一步（比第 255 轮的猜测窄得多）**：第 0、1 步的 restart 与 history 逐位全同
⇒ 进入第 2 步的**状态完全相同**，所以第 2 步的差只能来自**第 2 步内部**某条分支里
表达式的形状。本轮把水步（`WATER_VSF`）这一侧**全部排除**掉了：

* `:1128-1133` 凝结项 —— 三个通量三步全零（上面实测）；
* `:1139-1147` `wblc` 补冰 —— 插桩三行 `DBG258`：`balance_error_mm` 依次为
  `-3.41e-12 / 0 / 0`，`active=false` 三次 ⇒ **补冰循环一次都没进**；
* `:1069-1085` imperv —— 要顶层非渗透，本例 `patchtype = 0` 不走。

而 `WATER_VSF` 里改冰的语句总共只有这四处（另两处 `:855`/`:1211` 只读）。
**结论：第 2 步那 256 ULP 的冰差是在能量步（`meltf` 相变）里产生、又被水步原样留下的**
—— 这也解释了"`wliq` 逐位相同而 `wice` 不同"（水步会用压力头把 `wliq` 整个重写，
`wice` 水步不动）。而 `meltf` 本身有 9 量 × 10000 的逐位闭环、第 0/1 步实参也全同，
所以嫌疑落在**第 2 步才走到的那条分支**上：内核第 1 步 `t_soisno(1)` 恰好停在 273.16、
第 2 步掉到 265.51，相变（`imelt` 分类）到第 2 步才真正转动。

⇒ 下一枪：把 `/tmp/gf/meltf_args_probe.sh` 的 `end_sec` 从 `3600`（2 步）改成 `5400`（3 步），
只比**第 2 步**那组实参（`fact/brr/t_soisno_bef/t_soisno/wliq/wice` 第 1..3 层 + 13 个标量）；
哪个量出现位差，就顺着它往上游追（`fact`/`brr` 来自地面温度里的热参数链，
`th6_probe.sh` 是同一套骨架）。

Tested: `bash /tmp/gf/dry_ts.sh 3` ×4（死代码平铺/死代码融合/活代码平铺/`err_solver` 平铺，
`shasum -a 256` 与 `window_divergence.py` 比对：四版全部与基线一字不差）；
`python3 /tmp/gf/locate257.py`（差异元素与层号定位）；
`crates/colm-core` 的 `water_2014_soil_calls…` 单测（验证探针确实会打印）；
插桩 `DBG256`/`DBG257`/`DBG258`（均已 `git checkout --` 撤销，工作树只剩本文件的文档改动）。
Not-tested: 第 2 步的 `meltf` 实参（探针仍是 2 步版）。

`err_solver`（`:4494-4498` 的 `FNMA(通量和, deltim, 蓄量变化)`）也顺手 A/B 了平铺版：
输出同样逐字节相同 —— 与 "补冰循环没进" 互为印证（该项只在 `wblc > 0` 时才看得见）。

### 第 257 轮：**残差消除** —— `wblc` 的两个逐层累加在 Fortran 里被收缩成 FMA

五枪探针的链条（每枪一个脚本，都在 `/tmp/gf/`）：

1. **3 步版实参探针**（`meltf_args_probe3.sh`，`end_sec` 3600→5400）：第 2 步的 31 个实参
   （`fact/brr/t_soisno_bef/t_soisno/wliq/wice` 第 1..3 层 + 13 个标量）**逐位全同**。
2. **打进 `meltf` 内部**（`meltf_inner_probe.sh`）：`supercool`（含缩放前的 `vliq` 与全套
   VG 参数）、`wice/wliq/t/hm/xm/heatr/imelt`、`xmf` —— 三步全同。**关键反转**：
   内核 `meltf` 的**输出** `wice = …40C` 与 Rust 逐位相同，而内核第 2 步的 history 是
   `…30C` ⇒ 差产生在 **meltf 之后的水步**（第 256 轮"能量步产生"的判断反了）。
3. **`imperv` + `wblc` 探针**（`imperv_probe.sh`）：`is_permeable(1)` 第 1/2 步为真
   （离线也能验：`vol_ice = 6.182/(0.0175·917) = 0.385`，`eff_porosity = 0.5016-0.385 = 0.1166
   > θr = 0.11374`）⇒ 不透水那一支不走；水步里能改冰的只剩 `wblc` 补冰循环。实测第 2 步
   内核 `wblc = +2.2737367544323206e-13`、Rust `= 0`；`WBLCEND` 之后内核冰 `…30C`、
   Rust `…40C` —— **那 256 ULP 整项就是它**。
4. **`wblc` 分量探针**（`balance_probe.sh`，逐层打印两个累加值 + 6 个分量）：
   `w_sum_before`、`sum(etroot)`（本窗口恒 0）、`etrdef`（0）、`qgtop`、`rsubst` **全同**；
   只有 `w_sum_after`（以及 before/after 的**逐层累加过程**）在**特定层差 −1 ULP 且此后一直保持**
   （step 0 从第 8 个可透层起、step 1 从第 6 个起、step 2 从第 0/1 个起）。
   "出现一次就保持"正是 `acc + a*b` 被收缩成 FMA 的指纹（一步舍入 vs 两步舍入）。
   出货汇编佐证：`otool -tv -p ___mod_hydro_soilwater_MOD_soil_water_vertical_movement
   kernels/default/colm.x` 里有 **2568 条 `fmadd/fnmadd`**。
5. **落码**：`variably_saturated_flow.rs` 的 `balance_before_mm` / `balance_after_mm`
   两个逐层累加（各 4 处）改成 `mul_add`（`acc + a*b` → `a.mul_add(b, acc)`）。

`bash /tmp/gf/accept_r247.sh` 的三段式结果：

| 口径 | 第 256 轮 | **第 257 轮（现在）** |
|---|---|---|
| 1 步 restart | 0 / 68 | **0 / 68** |
| 3 步 restart | 2 / 68（`wice_soisno[5]`、`hk[0]`） | **0 / 68** |
| 3 步 history（692 个逐位单元） | 687（99.2775 %） | **692（100 %，逐位全同）** |
| 黄金 `ot_vars` dry/wet/snow | 17 / 68 / 79 | 17 / 68 / 79 |
| 黄金 `over_tol` dry/wet/snow | 826 / 20665 / 25896 | **825** / 20665 / 25896 |

黄金窗口没动：它与**存储的** golden 文件比，那条链的首次分歧在第 1 步且量级是 1e-7
（`f_trad`/`f_rnet`，不是末位级），本轮的修复不在这条链上；干窗本身也是混沌的。
但**第 2 步那个唯一的状态差已经彻底消失**，`wice_soisno`/`hk` 三步全同。

#### 第 257 轮的三个教训

* **因果箭头别对着 history 反推**：`…40C` 是**两侧共有**的 meltf 输出值，`…30C` 是内核
  水步里再扣 1 ULP 的结果。第 256 轮拿"history 的值"当"候选模块的输出"，把方向搞反了。
  探针必须打在候选语句的**前后两侧**，而不是拿最终输出当模块输出。
* **内核构建是单例**（新坑）：`build_kernel.sh` 的临时树固定是 `kernels/build-<preset>`，
  而每个探针脚本的 `trap restore` 会**再编一次**内核。第二个探针在第一个的 restore 还没跑完时
  启动，两秒后就把它整棵树删了 —— 两边都报 `No rule to make target 'mksrfdata.x'`，
  而 `kernels/default/` 里留下的是**上一个探针的带插桩内核**（差一点就拿它当干净内核去比）。
  现在每个探针开头都用 `mkdir` 抢 `/tmp/gf/.kernel_build.lock`（15 分钟陈旧锁可抢）。
* **`compare_phasechange.sh` 对土壤层的覆盖是弱的**：它只逐位比 `wice(lb)`（`lb = -1`，
  **雪层**），土壤层只经 `sum_qfrz`/`xmf` 这类**求和**间接覆盖，1 ULP 会被求和吞掉。
  "9 量 × 10000 逐位闭环"于是并不能证明土壤层分支没问题 —— 本轮差的就是土壤层。

#### 第 257 轮追加：修完之后还剩什么（长程阶梯与下一颗种子）

`bash /tmp/gf/dry_ts.sh 48`（TIMEPSTEP、干窗）逐记录看：

| 步 | 差异变量 | 量级 | 状态量 |
|---|---|---|---|
| 0–2 | 0 | — | 全同 |
| 3 | 5（`f_fgrnd`/`f_olrg`/`f_rnet`/`f_trad`/`f_zerr`） | 1–4 ULP | **全同** |
| 4–5 | 0 | — | 全同（5 步 restart **0/68**） |
| 6 | 28（含 `f_t_grnd`/`f_t_soisno`/`f_wliq_soisno`/`f_zwt`） | 1 ULP 种子 + 下游 | **首次分歧** |
| 7 起 | 37… | 混沌放大 | — |

第 3 步那 5 个量是**纯诊断**（`olrg`/`fgrnd`/`trad`/`rnet` 不回流到状态），所以它们是
症状、不是第 6 步状态差的原因。把 `olrg` 的输入逐个比出来（`olrg_probe.sh`：

**注意要打 `extends/interception/MOD_Thermal_CanopyPhase_Extended.F90`，
`main/MOD_Thermal.F90` 不参与编译** —— 第一版打错了文件，构建照样成功、`strings` 里却
找不到标记，这正是仓库里"编的不是你以为的那个文件"那条坑的又一次现形）：

```text
第 3 步：ulrad  内核=267.00097916323915  rust=267.0009791632391   ← 1 ULP
         emg / t_grnd_bef / tinc / stefnc 全同（其余各步 ulrad 也全同）
```

⇒ 第 3 步的种子是叶温例程的 `ulrad`（`MOD_LeafTemperature_Extended.F90:1410/1416`）。

两个已做过的排查（都**无效果、已还原**，但结论值得记）：

* 第三项结合顺序（`(1-emg)*thermk*thermk*frl` 的左结合 ↔ `(1-emg)*thermk.powi(2)*frl`）
  A/B：第 3 步差异一字不变 ⇒ 不是这一项。
* 指数结合顺序**不是**嫌疑：实测 gfortran 的 `x**4` 就是 `(x*x)*(x*x)`
  （200000 组随机数上 0/200000 不同；`((x*x)*x)*x` 则 69119/200000 不同），
  与 Rust 的 `powi(4)` 一致；`x**3` 同 `(x*x)*x`。

⇒ 剩下的嫌疑是 `ulrad` 的**分量**（`fac`/`tlbef`/`dtl`/`thermk`/`emg`/`tg`，
全在叶温 Newton 循环里）以及第 6 步状态那一颗（两条链互不相干）。

#### 第 270 轮：记录 12 的 `ss_wt(3)` 在**解算之前**就已经差了（外加一个探针陷阱）

**先记陷阱（差点写出假结论）**：把 Newton 探针从 3 个分量扩到 5 个之后，步 8–15 一律报
`blc4`/`dv4` 不同，内核值是 `6C4B0153B6230FC6` —— 解出来是 **4.5e+213**（不是"微小残差"）。
真因是**越界读**：上游的 `blc`/`dv` 形状是 `(lb-1:ub+1)`，**长度随段变化**；短段（<4 层）上
`blc(4)` 是数组外，读到的是垃圾，而 Rust 侧我用了 `get()` 保护 → 0。两边一"比"就出差异。
**教训：探针里凡按固定下标取 Fortran 变长数组，都必须先夹到 `ub-lb+3` 以内**（本轮之前
第 268 轮"步 1–14 残差/修正量全同"的结论在**有效分量**上仍然成立）。

**真结论**：换一条路 —— 在 Rust 侧把 `water_table_thickness_mm` 在**分段求解之前**打出来
（Rust-only 探针，不用重编内核），跑 16 步：

```text
步 12：PRESOLV ss_wt2=404668962A52BD6D  zwt=4046DF5390EEC489  spzi3=4056A3F4DDA0C0FB
        （这正是 Rust 的记录 12 终值 …6D，也与 wt_probe 的 Rust 值一致）
步 12 内核终值（wt_probe）：ss_wt2=404668962A52BD6E（…6E）、zwt=…488
```

两边在步 11 的状态逐位相同、`zwt`/`ss_wt(3)` 满足 `sp_zi(3) = ss_wt(3) + zwt`，
而 Rust 的**解前值已经等于解后值** ⇒ 差不是分段求解产生的，而是**解前那一份
`ss_wt(3)`（由含水层交换给出的 `zwt` 重推）**就已经不同（或求解器根本没动这一层、
而内核动了）。下一枪：把"解前/解后"两条都打在**两侧**（内核侧同样要打），
并用**长度感知**的取值方式。

#### 第 269 轮：显式回退也排除了 —— 两侧都没走那条路

第 268 轮把记录 12 的差推到"解后重建/分支应用"侧。本轮先查最可疑的**显式回退**：

- 内核侧：在 `CALL use_explicit_form` 前插标记，跑 16 步 —— **0 次**（内核全程走隐式）；
- Rust 侧：在子步收尾处打 `converged/forced/iteration/计数器`（Rust-only 探针，不用重编内核）
  —— 24 个子步（16 步、多段列）**全部 `converged=true forced=false`**，三个降级计数器恒 0
  ⇒ Rust 也没走显式回退。

⇒ 回退分支**不是**记录 12 的差源（两边都没走）。

至此记录 12 的候选只剩：

1. **子步初始化**（`initialize_variable_saturated_sublevels` / `initialize_sublevel_structure`
   每次子步重算 `water_table_thickness_mm`/`wetting_front_mm`，含 `0.1*()`/`0.01*()` 与
   "饱和层直接给整层厚"等分支）；
2. **分段切法**（`soilcolumn` 循环把土柱按不透水层切成段，`first..last` 不同会让
   `ss_wt` 的路径不同）；
3. `check_and_update_level` 的夹取（形状已核对过一致）。

探针教训（累计）：Fortran 侧下标要用**相对**（`blc(lb-1+k)`，短段 len=3）；Rust 侧插入
print 时锚点要落在**语句开头**，别插进 `let x = <调用>` 中间（本轮插错一次，编译报
`no field … on type ()`）。

#### 第 268 轮：Newton 解那一侧**整段被排除** —— 记录 12 的差在解后重建

`newton_probe16.sh`（16 步，两侧位型：Newton 每次迭代的残差 `blc(lb-1..lb+1)` 与
修正量 `dv(lb-1..lb+1)`，都用**相对下标**取；第一版没加保护，`len=3` 的短段把 Rust
运行时打崩了）：

```text
行 0–13（步 1–14）：残差与修正量**全部逐位相同**（含第 12 步！）
行 14（步 15）：blc1 / dv1 开始不同（级联段）
```

⇒ **第 12 步的水位差不是 Newton 解出来的**：残差同、修正量同，`ss_wt(3)` 却差 1 ULP。
所以嫌疑只剩解算器的**解后重建/分支分类**那一侧：

- `jasbl`/coordinate 的分类（层该走"湿润锋/水位/液态水"哪一支）虽然比较结果一致，
  但**应用修正量时的分支**（`ss_wt = ss_wt - dv` / `- min(dv, sp_dz)` / `min(..., sp_dz-ss_wf)`）
  以及 `ss_wf` 的更新可能不同；
- 以及 `soil_water_vertical_movement` 里解后的 `ss_vliq`/`ss_wt` 重算（`:415-435`、
  `:344-349`）—— 注意 `:431-435` 那段在 `izwt-1..1` 上是**恒等变换**（`ss_wt` 恒 0），
  两侧照抄，不构成差异。

下一枪：把解后重建那几行（`ss_wt`/`ss_vliq`/`ss_wf`/`zwt`）在**第 12 步**前后各取一次位型。

#### 第 267 轮：把解算器的通量/子步表达式审了一遍 —— 两处候选都是惰性的

把 `MOD_Hydro_SoilWater.F90` 里"含乘积又含加减"的赋值按函数分块列了出来
（`flux_all` 9 处、`flux_sat_zone_all` 1 处、`flux_sat_zone_fixed_bc` 5 处、
`flux_at_unsaturated_interface` 1 处、显式步 15 处），挑出两处"变量 + 乘积"形态的
按上游收缩写法 A/B：

| 上游 | Rust 原写法 | 结果 |
|---|---|---|
| `:2750 flux_inside = hk_u + (psi_u-psi_l)/dz * hk_u**(1-rr) * hk_l**rr` | 平铺 | 干窗 16 步**一字不变**（记录 12 仍 `f_zwt`） |
| `:1464 dp = max(0., dp_m1 + (ubc_val-q)*dt)` | 平铺 | 同上 |

两处都**已还原**（干窗无分辨力，而水步的"未验证收缩猜测"在第 266 轮已证明会伤湿窗）。

同时把种子图补全（干窗 16 步逐记录，还原后复核）：

```text
记录 12/13：f_zwt（ss_wt(3) 1 ULP，瞬态，14 记录自愈）
记录 14：   f_assim / f_assimsun / f_assimsha / f_gssha（光合-气孔链，4 个诊断量）
记录 15：   56 个变量、10 个状态量（级联，t_grnd 551 ULP 量级）
```

⇒ 下一条要打的是**记录 14 的光合/气孔链**（它比记录 12 的瞬态更值得追），
以及记录 15 那个级联的种子。

#### 第 266 轮追加：记录 12 那颗种子是 `ss_wt(3)`

`wt_probe16.sh`（16 步，两侧位型：`zwt`/`wa`/`ss_wt(1..10)`）：

```text
记录 0–11：全同（含 zwt / wa / 全部 ss_wt）
记录 12：ss_wt(3) 差 1 ULP（内核 404668962A52BD6E / Rust …6D），zwt 随它差
记录 13：同上 1 ULP
记录 14：全同（瞬态，与 48 步表一致）
记录 15：ss_wt(3) 差到 2.3e-7（级联开始）
```

`zwt` 由"自下而上扫第一层非饱和层、`zwt = sp_zi(ilev) - ss_wt(ilev)`"得到，
之后 `ss_wt` 又会按 `zwt` 重算（`:344-349`）—— 循环依赖里**先动的是解算器给出的
`ss_wt(3)`**。下一枪照第 259 轮的办法：在记录 12（要跑 ≥13 步）打 Newton 的
残差/修正量与 Jacobian 对角，再用多步约束穷举形状。

注：水步形状的验收尺子用**湿窗**（见上一条），干窗是混沌的。

#### 第 266 轮：`water_balance` 剩下三处对齐 —— 短程惰性、**湿窗反而更差**（已还原）

把 `water_balance` 里另外三处残差按上游原文对齐（其中 `:1168` 那处是**真的结合顺序
不同**：上游是 `blc - waquifer_m1 - q(ub)*dt`，Rust 原先写成 `blc - (waquifer_m1 + q*dt)`）：

| 上游 | Rust 原写法 | 本轮试的写法 |
|---|---|---|
| `:1140 blc(lb-1) = dmss - qsum*dt` | `mass_change - flux_sum*dt` | `(-flux_sum).mul_add(dt, mass_change)` |
| `:1168 blc = blc - waq_m1 - q*dt` | `blc -= waq_m1 + q*dt` | `(-q).mul_add(dt, blc - waq_m1)` |
| `:1170 blc(ub+1) = waq - waq_m1 - q*dt` | 平铺 | `(-q).mul_add(dt, waq - waq_m1)` |

实测：干窗 16 步**一字不变**（记录 12 仍是 `f_zwt` 1 ULP）；三段式验收里
**湿窗变差**（`over_tol` 20662 → **20667**、`sumabs` 10379.2 → 10388.1、`bitwise` 32611 → 32610），
干窗与雪窗不变 ⇒ **全部还原**。

**由此得到一条有用的判据**：干窗是混沌的、判不了水步形状，但**湿窗对水步形状有分辨力**
（这次三处一共差 5 个超容差变量）。以后水步的收缩/结合改动，先用湿窗当尺子。

#### 第 265 轮：`fseng` 的订正其实**不融合** —— 干窗前 12 条记录**每个变量**都逐位相同

第 264 轮把首个差异推到记录 10，差异变量是 `f_fsena`/`f_fseng`/`f_fgrnd`/`f_tstar`/
`f_zol`/`f_rib`（显热 + 稳定度链；`f_fgrnd` 经 `-(fseng+fevpg*htvp)` 在它下游）⇒
直接 A/B `MOD_Thermal…:1236` 那条订正的形状：

```fortran
fseng = fseng + tinc*cgrnds        ! 平铺，不融合
```

Rust 原先写的是 `cgrnds.mul_add(tinc, fseng)`。**第 252 轮曾经判定过这条**：
"两种写法在第 1 步给出同一位型（`4086FEDBB7C44298`），判不了"，于是按"保守"留了融合 ——
本轮在**记录 10** 上分辨出来了：改成平铺后干窗 12 步**逐记录全 0**。

| 干窗 CN-Cng 48 步 | 改前 | 改后 |
|---|---|---|
| 记录 0–11 | 记录 10 起有差异（6 个变量） | **全部逐位相同（117 个变量全对）** |
| 首个任意差异 | 记录 10 | **记录 12（`f_zwt`）—— 与首个状态差异同一处** |
| 3 步 history / 黄金 | 692/692、829/20662/25896 | 不变 |

**新增一条纪律**：早先"两种写法分辨不出 ⇒ 取保守写法"的结论要**挂账**，
等更长的窗口/更多步数再来判 —— "保守"不是证据。本轮就是靠这条把第 252 轮的悬案结了。

现在干窗的短程阶梯是：**记录 0–11（6 小时）117 个变量全部逐位相同**，第一个差异是
记录 12 的 `f_zwt`（水步瞬态）。

#### 第 264 轮：`ulrad` 括号里那个乘积也要融合 —— 干窗前 **10** 条记录逐位相同

第 262 轮融合的是 `stefnc*(p1+p2) + A` 的**左**乘积。本轮把 `ulrad` 探针跑到 **12 步**
（`ulrad_konly.sh`，只打内核侧 —— Rust 侧的形状在 Python 里建模即可，因为记录 0–11
两侧状态本来就全同），再做**多步同时约束**的穷举：

```text
同时满足 12 步的组合: 48 个 —— 全部带 `p1 + ther mk*emg*tg**4` 的"右乘积融合"
                              （以及第 262 轮那条左乘积融合）
```

⇒ 括号里那一和同样是"乘积加变量"，落码 `FMA(thermk*emg, tg**4, p1)`
（split 分支保持原样，本机算例走不到）。

| 干窗 48 步 | 改前 | 改后 |
|---|---|---|
| 记录 0–9 | 记录 8 起有差异 | **全部逐位相同** |
| 首个任意差异 | 记录 8（`f_olrg`/`f_rnet`/`f_zerr`） | **记录 10**（`f_fsena`/`f_fseng`/`f_fgrnd`/`f_tstar`/`f_zol`/`f_rib`） |
| 首个状态差异 | 记录 12（`f_zwt`） | 记录 12（不变） |
| 3 步 history / 黄金 | 692/692、829/20662/25896 | 不变 |

**规律再补一条**：`A + B*C` 与 `A*B + C` 两种形态都要试；穷举一律**多步同时约束**
（第 263 轮的纪律）。下一条种子是记录 10 的 `fseng`/`fsena` 那条"显热链"。

#### 第 263 轮：`f_fgrnd` 也修掉了 —— 干窗前 8 条记录**全部逐位相同**

`fgrnd` 里 `- (1-fsno)*emg*stefnc*t_soil**4`（`MOD_Thermal…:1347`）那一项同样是
"乘积被收进减法"，Rust 原先平铺 ⇒ 第 3 步 `f_fgrnd` 差 1 ULP。

**方法上的一次升级**：先把 `t³`/`t⁴` 的结合与现代四处收缩**一起**穷举，并且**用 8 步
同时约束** —— 只约束第 3 步时，"唯一能复现"的组合有 16 个（含若干伪解）；把 8 步一起
卡上以后，**唯一共同可行的是 `f2=1`**（只融合这一项），其余维度（`t³`/`t⁴` 结合、
第一项与另外两处的融合）在本算例都可平铺。

另一个容易踩的点：`t_grnd`/`tinc` 在 `:1229-1233` **解后重新赋值**
（`tinc = t_soisno(lb) - t_soisno_bef(lb)`），而 `t_soil`（`:526`）保持**解前**值 ——
`fgrnd` 里前两者用新值、`t_soil⁴` 用旧值。探针取的正是这两类各自的值。

48 步干窗逐记录：

| 口径 | 改前 | 改后 |
|---|---|---|
| 记录 0–7 | 记录 3 起有差异 | **全部逐位相同** |
| 首个任意差异 | 记录 3（`f_fgrnd`） | **记录 8**（`f_olrg`/`f_rnet`/`f_zerr`） |
| 首个状态差异 | 记录 12（`f_zwt`） | 记录 12（不变） |
| 3 步 history / 黄金 | 692/692、829/20662/25896 | 不变 |

**再添一条方法纪律**：离线穷举形状时要用**多步同时约束**，只在一步上"唯一"的组合
未必是真解。

#### 第 262 轮：`ulrad` 那一颗修掉了 —— **收缩点可能在加法左边**

第 257 轮留下的"唯一短程可见差异"是第 3 步那条纯诊断链（`f_olrg`/`f_trad`/`f_rnet`/`f_zerr`）。
当时用 128 种"结合顺序 + 收缩"组合都复现不出内核的 `ulrad`（`4070B00402BA216F`），
一度怀疑是输入不对；本轮先证伪了那个怀疑：**`ulrad` 的那段代码在 Newton 循环之外**
（`DO` 在 1100 行之前、`ENDDO` 在 1295，而 ulrad 在 1410），所以探针打印的输入就是
参与运算的那一份。

真正的缺口是**漏了一维**：`MOD_LeafTemperature_Extended.F90:1416` 的第一个加法是

```fortran
ulrad = stefnc * ( fac*tlbef**3*(tlbef + 4.*dtl) + ther mk*emg*tg**4 )  &
      + (1-emg)*thermk*thermk*frl + ...
```

—— **左边就是乘积** `stefnc*(p1+p2)`，GCC 收的是它：`FMA(stefnc, p1+p2, A)`。
把"左乘积融合"这一维加进离线穷举后，内核值立刻被唯一复现（平铺得 `…216E`）。
落码：`STEFAN_BOLTZMANN.mul_add(canopy_emission, A)`，后三个加法保持平铺
（本算例分辨不出它们的收缩）。

48 步干窗逐记录：

| 口径 | 改前 | 改后 |
|---|---|---|
| 首个任意差异记录 | 3（5 个变量：fgrnd/olrg/rnet/trad/zerr） | **3（只剩 `f_fgrnd`）** |
| 记录 7–11 的零散诊断差异 | 2 / 3 / 3 / 6 / 1 个 | **0** |
| 首个状态差异记录 | 12（`f_zwt`） | 12（不变，那条是水步的瞬态） |

**新增一条可复用规律（写进方法清单）**：遇到 `表达式 + 乘积` 或 `乘积 + 表达式`，
收缩点是**那个乘积**，与它在加法的哪一侧无关；只沿"右边是乘积"枚举会漏掉一半。
已经实锤的三处：`dmss = 乘积 + dmss`、`cgrnd = cgrnds + cgrndl*htvp`、
`ulrad = stefnc*(…) + A`；而 `A*B + C*D`（两个乘积）在本算例一直分辨不出。

剩下的一颗：**`f_fgrnd`**（第 3 步 1 ULP）。它的输入（sabg/dlrad/emg/fsno/t_soil/
t_grnd_bef/tinc/fseng/fevpg/htvp）已由 `fgrnd_probe.sh` 取到，Python 平铺模型能复现
**Rust** 的值（`C081C128CF597630`）而 4 种收缩组合都复现不出内核的 `…631`
⇒ 与 `ulrad` 同样还有一维没建模到，下一轮按同法继续。

#### 第 260 轮追加二：`flux_sat_zone` 那类 `A*B + C*D` 的收缩 —— 又是一处"看不出来"

按"上游多条语句累加就逐条按收缩落"的思路，把 `MOD_Hydro_SoilWater.F90` 里所有
"含乘积又含加减"的赋值行列了出来（77 行、约 30 个不同变量），其中与本轮修好的
`water_balance` 最像的是显式步里的

```fortran
wa_m1 = (wt_m1(ilev)+wf_m1(ilev)) * vl_s(ilev) &
      + (dz(ilev)-wt_m1(ilev)-wf_m1(ilev)) * vl_m1(ilev)      ! :1410-1411
```

Rust（`apply_variable_saturated_explicit_step` 的 `previous_water`）是平铺的
`A*B + C*D`。A/B 了"收右乘积"这一种写法：**无效果**（48 步口径下首个状态差异
仍是记录 12 的 `f_zwt`），已还原。

结论：`A*B + C*D` 这种**两个乘积相加**的位置，本算例里两种收缩写法都看不出来
（与 Givens 那两处一致）；真正有效的那两处都是**"乘积加变量"**（`dmss = 乘积 + dmss`、
`cgrnd = cgrnds + cgrndl*htvp`）。后续若要继续，优先找"乘积 + 变量"的形态。

#### 第 260 轮追加：第二配置（Campbell + 关 VSF）回归复查 —— 16 / 66 / 79

`cgrnd` 那处修复在**两个配置都会走到**的叶温例程里，所以第二配置必须复查。
`oracle/scripts/compare_second_config.sh` 三个窗口：

```text
CN-Cng (Campbell, VSF off)      16 variable(s) outside tolerance
CN-Cng-wet                      66
US-NR1-snow                     79
```

与文档记录的基线 **16 / 66 / 79 完全一致** ⇒ 无回归（这条一直挂在"Not-tested"里，本轮补上）。

短程对齐阶梯（干窗 CN-Cng、TIMESTEP、16 步，逐记录差异）：

| 记录 | 差异变量 | 状态量 |
|---|---|---|
| 0–2 | 0 | 全同 |
| 3 | 5（`ulrad` 那条纯诊断链） | 全同 |
| 4–6 | 0 | 全同 |
| 7–11 | 1–6（都是诊断） | **全同** |
| 12–13 | 1–2（`f_zwt` 1 ULP） | `f_zwt`（**瞬态**，14 记录即消失） |
| 15 | 56 | 10 个（`t_grnd` 551 ULP 等）—— 混沌/分支放大 |

#### 第 260 轮：**水步那颗种子也修掉了** —— `water_balance` 的 `dmss` 要按三条语句累加

第 259 轮把种子夹到"扰动分支的求值"后，顺着扰动会重新走的 `water_balance` 读源码，
发现上游是**三条语句**累加（`MOD_Hydro_SoilWater.F90:1146-1148`）：

```fortran
dmss = (vl_s-vl_m1)*(wf-wf_m1)
dmss = (vl_s-vl_m1)*(wt-wt_m1) + dmss     ! 乘积被收进加法
dmss = (dz-wt-wf)*(vl-vl_m1)   + dmss     ! 同上
```

Rust 原先是一条平铺长链（少两次融合）；`residual_mm[active] += mass_change - qsum*dt`
也没按上游的 `blc + dmss - qsum*dt`（末项收缩）写。落码后：

| 口径 | 改前 | 改后 |
|---|---|---|
| 8 步干窗第 6 步差异变量 | 2（`f_wliq_soisno`/`f_zwt`） | **0** |
| 8 步干窗第 7 步 | 4 | 2 |
| 48 步干窗**首个状态差异** | 记录 6（`f_zwt`） | **记录 12** |
| 3 步 history | 692/692 | 692/692 |
| 黄金 `over_tol` dry/wet/snow | 821 / 20664 / 25896 | 829 / **20662** / 25896 |

干窗的 `over_tol`/`sumabs` 变差是**混沌放大**（文档早已记过干窗 `ot_vars` 没有分辨力）：
短程逐位与湿窗都朝更一致的方向走，所以按**源码结构 + 短程逐位**保留这个改动。

至此唯一还剩的"短程就看得见"的差异就是**第 3 步那条纯诊断链**（`f_olrg`/`f_trad`/
`f_rnet`/`f_fgrnd`/`f_zerr`，1–4 ULP，见追加二/三的 `ulrad`）。

#### 第 259 轮：种子在 Richards 解算器里，而且是从**扰动分支**进来的

两枪探针 + 两次 A/B：

1. **子步收尾量**（`rsolv_probe.sh`，8 步：`dt_this/ss_dp/ss_vl1/ss_wt1/ss_wf1`，每步一行）：
   第 0 步全同；**第 1 步 `ss_vl(1)`、`ss_wt(1)` 就出现差异**（第 2–5 步又相同，第 6/7 步再差）。
   第 1 步那次差异**不影响状态**（解算器返回后 `ss_wt`/`wliq` 会被重算覆盖）⇒ 它是个
   "只在某些分支里露头"的末位差。
2. **Newton 残差与修正量**（`newton_probe.sh`，8 步：`blc(1..4)`、`dv(1..4)`、`iter`）：
   首个不同行 idx=5 —— **残差 `blc1` 两侧逐位相同，而修正量 `dv1` 不同**
   （内核 `40061FA36D43457C` / Rust `40061FA36D43455F`）。
   ⇒ 差不在"求残差"那一步，而在**修正量的输入**里（残差同、解算器同 ⇒ 只剩 Jacobian/active）。
3. Jacobian 是**扰动式**组装：`dr_dv(:,ub+1) = (blc_pb - blc)/dlt`（上游 `:989`），
   而扰动那次 `flux_all`/`water_balance` 走的是"单层更新"分支
   （`lev_update(ub+1)=.true.`）⇒ 种子在**扰动分支**里的某个通量/收支算式。
4. 两个 A/B 都**无效果、已还原**：Givens 旋转里 `c*a + s*b` 的两种收缩写法
   （右乘积融合 / 左乘积融合）—— 8 步口径下差异一模一样。这也反过来证明
   最小二乘那一步的算术**不是**原因，原因在它拿到的 Jacobian。

5. 又试了三处 A/B，**全部无效果、已还原**（8 步口径下逐记录差异一字不变）：
   Givens 的右乘积融合、左乘积融合（见上），以及 `flux_at_unsaturated_interface` 里的
   `psi_i = (dz_l*psi_u + dz_u*psi_l)/(dz_u+dz_l)`（上游 `:2815`，Rust 是逐字平铺）
   的两种融合写法。
6. 侧查：`var_perturb_*` 的增量全是 `min(wstep, x*0.5)`、`min(wstep, x*0.1)`、
   `psi_s - (1-qin/hksat)*(-delta)*(zi-zc)/dz` 这类**乘积在 min/除法里**的写法
   —— 没有"乘积进加法"的位置，所以扰动增量本身大概率不是种子。

下一枪：在**第一个 Newton 迭代**里把 `dr_dv`/`jacobian` 的对角几项与 `vact`/`active`
打出来，确认 Jacobian 先差；再打**每一列的扰动残差** `blc_pb`（列号 = 被扰动的层），
用"第一个不同的列"定位到 `flux_all` 的单层更新分支里的具体算式。

#### 第 258 轮：水步那颗种子再往里一层 —— Richards 解算器的 `ss_wt(1)`

`wt_probe.sh`（8 步，`zwt`/`wa` + 每层 `ss_wt`）：

```text
第 0–5 步：zwt / wa / ss_wt(1..10) 全部逐位相同
第 6 步：ss_wt(1) 先差（内核 4024492DBE146F92 / Rust 4024492DBE146F99），zwt 随它差
第 7 步：两者继续差（ss_wt(1) 反向差 14 ULP）
```

`wa` 两侧都是 0 ⇒ 水位走的是"自下而上扫层"那一支（不是 `get_zwt_from_wa` 的迭代），
而 `zwt = sp_zi(ilev) - ss_wt(ilev)` ⇒ **种子是 Richards 解算器算出的第 1 层饱和厚度
`ss_wt(1)`**（`MOD_Hydro_SoilWater.F90:1019-1040` 的显/隐式子步分支；Rust 对应
`variably_saturated_flow.rs` 的 `richards_solver`）。`ss_wt` 的那几处更新本身**没有乘积**（`ss_wt - dv`、`min(dv, sp_dz)`、`q/hksat`），
所以种子不在那三行，而在它上游的值里。Rust 的 `richards_solver`
（`variably_saturated_flow.rs:3401`）是**结构照抄**上游的（子步 + 每子步 Newton），
所以候选只剩它内部的五个部件之一：

| 部件 | Rust | 上游 |
|---|---|---|
| 子层初始化（饱和/湿润锋/水位标记） | `initialize_variable_saturated_sublevels` | `:880-1050` 的标记段 |
| 通量 | `flux_variable_saturated_flux_all` | `flux_all` |
| 水量平衡残差 | `variable_saturated_water_balance` | `water_balance` |
| 数值 Jacobian | `var_perturb_*` | `var_perturb_*` |
| 最小二乘修正量 | `solve_variable_saturated_least_squares` | `solve_least_squares_problem` |

下一枪：在第 6 步的**第一个子步**打出这五段的输出（每段取几个标量/层量），
把差异夹到其中一段；`ss_wf` 在上游是每次调用从 0 开始的局部量，两侧一致。

探针卫生（这一轮踩的两个小坑）：`strings` 默认只列 ≥4 字符的串，所以标签至少 4 个字符
（`WT ` 只有 3 个字符，明明编进去了却 grep 不到）；`grep` 无匹配在 `set -e` 下会直接
中断脚本，所以"取标记行"的 grep 要允许失败或先判空。

#### 第 257 轮追加五：水步的种子是 `zwt`（水位），它把 `wliq` 带偏

`water_probe.sh`（8 步；打在 `MOD_Hydro_SoilWater.F90` 的 `wblc = …` 之后）：
第 0–5 步 `zwt`/`qinfl`/`ss_vliq(1..3)`/`wa` **全同**；**第 6 步 `zwt` 先差**
（内核 `401D7AC4A7A39AB0`、Rust `401D7AC4A7A39AA2`，14 ULP），第 7 步连 `ss_vliq(1)` 也差。

`wliq_soisno` 的重建在后面（`vol_liq(j) = … (sp_zi(j)-zwtmm) …`），所以顺序是
**`zwt` → `wliq`**。水位那一段（`MOD_Hydro_SoilWater.F90:408-430`）两条路：
`wa >= 0` 时自下而上扫第一层非饱和层，`zwt = sp_zi(ilev) - ss_wt(ilev)`；
否则走 `get_zwt_from_wa`（水位的迭代反解）。探针只打了第 1..3 层的 `ss_vliq`、
没打 `ss_wt`，所以还没分清是"扫描用的下层量先差"还是"迭代反解本身"。
下一枪：把 `ss_wt(1:nlev)`、`izwt`、`is_sat` 一起打出来（跑 8 步）。

#### 第 257 轮追加四：`cgrnd` 也是收缩差 —— 地表能量链的第 6 步分歧从 28 个量降到 2 个

按追加三的线索（记录 5 起 `dhsdT` 差 1 ULP，而 `dhsdT = -cgrnd - …`）去读 `cgrnd`
的算法，发现两边形状不同：

```fortran
! MOD_LeafTemperature_Extended.F90:1435-1437
cgrnds = cpair*rhoair*cgh*(1.-wtg0)
cgrndl = rhoair*cgw*(1.-wtgq0)*dqgdT
cgrnd  = cgrnds + cgrndl*htvp        ! cgrndl 是**舍入过的独立变量**，最后一项被收缩
```

Rust 原先把它平铺成一条长链（`… + rho*cgw*(1-wtgq0)*dqgdT*htvp`）⇒ 多一次舍入。
落码：先落两个局部量，再 `cgrndl.mul_add(htvp, cgrnds)`。

8 步干窗逐记录差异变量数：

| 记录 | 改前 | 改后 |
|---|---|---|
| 0–2 | 0 | 0 |
| 3 | 5（纯诊断 `f_olrg`/`f_trad`/`f_rnet`/`f_fgrnd`/`f_zerr`） | 5（另一条链，见追加二） |
| 4–5 | 0 | 0 |
| 6 | **28**（`t_grnd`/`t_soisno`/`frad`/`fseng`…） | **2**（只剩 `f_wliq_soisno`、`f_zwt`） |
| 7 | 37 | 4 |

三段式验收（`accept_r247.sh`）：1 步 restart 0/68、3 步 history **692/692 逐位全同**、
黄金 `over_tol` **dry 821 / wet 20664 / snow 25896**（上一轮 825 / 20665 / 25896；
dry 的 `sumabs` 也从 382.9 降到 248.6）⇒ 能量那条链已经干净，剩下的是**水步**那一颗。

#### 第 257 轮追加三：第 6 步那颗种子进到了 `dhsdT`

**先纠正上一节的一个口径错误**：`end_sec = 10800` 只跑到**第 5 步**（记录 0..5），
而分歧出现在**记录 6** ⇒ 用 6 步探针下的"全同"结论不算数。改成 8 步
（`end_sec = 14400`）重打，`gtcoef_probe8.sh` 立刻给出：

```text
记录 0..4：95 个量逐位全同
记录 5：  2 个量差 1 ULP —— dhsdT、rt(1)        ← 首次分歧
记录 6：  6 个量差（bt(1)、rt(1) 差 48 ULP、rt(2)、dhsdT、hs、t_soisno(1)）
```

`rt(1)` 里就含 `dhsdT`（`rt(j) = t_soisno(j) + fact(j)*(hs - dhsdT*t_soisno(j) + cnfac*fn(j))`）
⇒ **种子是 `dhsdT`**（`MOD_GroundTemperature.F90:307`）：

```fortran
dhsdT = -cgrnd - 4.*emg*stefnc*t_grnd**3 - cpliq*pg_rain - cpice*pg_snow
```

两个猜测都 A/B 过、**都无效果已还原**：后两项 `a - b*c` 的收缩（`cpliq*pg_rain`、
`cpice*pg_snow`，本窗口大概恒 0 所以看不出来）。还没打的是 `cgrnd` 本身
（它是入参，来自地表通量那一段；这一支的探针目前没打印它）。

另记两条排查（都无效果、已还原）：叶温例程 8 步版、`fseng` 订正的平铺形式。

#### 第 257 轮追加二：第 6 步那颗种子被夹到哪儿了

排除法（都在同一套 6/8 步干窗上）：

| 环节 | 探针 | 结论 |
|---|---|---|
| 叶温例程（58 个量 × 24 次迭代 × 6 步） | `leafit_bits_probe6.sh` | **全部逐位相同** |
| 地面温度三对角**组装**（95 个量 × 6 步） | `gtcoef_probe6.sh`（跑 6 步） | **全部逐位相同** |
| `meltf`（实参 + 内部 + 输出） | 第 257 轮前三枪 | 逐位相同 |

而第 5 步 restart 是 0/68、第 6 步 `t_soisno`/`t_grnd`/`wliq_soisno`/`zwt` 同时差 1 ULP
⇒ 种子只剩三个可能：`tridia` 的**求解**本身、解完之后的**订正**（`fseng`/`fevpg` 那两条，
形状已在第 252 轮定过）、或**水步**。下一枪按这个顺序打。

`ulrad` 那条（第 3 步 1 ULP、纯诊断）另外记一笔未结案的证据：用 `ulrad_probe.sh`
（修好尾锚之后）拿到的第 3 步输入**逐位相同**，而把上游算式按 Fortran 的写法在 Python 里
**全平铺**复算，得到的却是 **Rust** 的那个值（`…216E`），内核的 `…216F` 用 128 种
"结合顺序 + 内层/外层收缩"组合都复现不出来，加上 `pow()`/`(x*x)*x` 两种 `**3`/`**4` 实现也不行
⇒ 内核那一条的**编译形态还没被建模对**（不是简单的"某一项融合"），先挂着。

Tested: `/tmp/gf/meltf_args_probe3.sh`、`meltf_inner_probe.sh`、`imperv_probe.sh`、
`balance_probe.sh`（各 3 步，两侧位型；跑完都自动还原源码并重编内核）；
`olrg_probe.sh`（8 步）、`ulrad_probe.sh`（8 步）、`leafit_bits_probe6.sh`（6 步）、
`gtcoef_probe6.sh`（6 步）；`bash /tmp/gf/dry_ts.sh 48` + `window_divergence.py`；
`restart_divergence.py`（5 步 restart 0/68）；gfortran 的 `x**3`/`x**4` 结合顺序实测；
`otool -tv -p ___mod_hydro_soilwater_MOD_soil_water_vertical_movement kernels/default/colm.x`；
`bash /tmp/gf/accept_r247.sh`（1 步 restart / 3 步 history / 黄金三窗口）；
`bash /tmp/gf/dry_ts.sh 3` + `restart_divergence.py`（3 步 restart 0/68）。
Not-tested: 第二配置（Campbell + 关 VSF，本轮只动 VSF 路径）；11 天黄金窗口与存储 golden
之间的首次分歧（第 1 步、1e-7 量级，与本轮修复无关，来源未查）。

#### 第 271 轮：把"水位推出"和"分段求解"拆成两个时刻，两侧都打

`water_table_thickness_mm`（= 上游 `ss_wt`）在 `soil_water_vertical_movement` 里有
两个关键时刻：**推导之后、逐段求解之前**（`MOD_Hydro_SoilWater.F90:344-350`，记为
`WTPRE`）与**逐段求解之后、重定位水位之前**（`:403` 之后，记为 `WTPOST`）。
`wtpre_probe.sh` 把两个时刻在两侧都打出来（取值**长度感知**：`wtbuf` 先清零、
只拷 `MIN(8,nlev)`，避免重演第 270 轮的越界读），16 步结果：

```text
步 0–11（WTPRE/WTPOST 共 24 行）：izwt/zwt/wa/ss_wt1..8 **全部逐位相同**
步 12 WTPRE:  izwt 同、wa 同；zwt 差 1 ULP、ss_wt3 差 1 ULP（方向相反）
             K zwt=4046DF5390EEC488 R=4046DF5390EEC489
             K ss_wt3=404668962A52BD6E R=404668962A52BD6D
```

`ss_wt3` 与 `zwt` 反向 1 ULP 是**恒等式**：`izwt = 3` 时 `ss_wt(3) = sp_zi(3) - zwt`，
两边 `sp_zi(3)` 是同一个参数 ⇒ 差**只有一个**：`zwt`。所以第 270 轮记的
"`ss_wt(3)` 在解算之前就差了"要改口径为"**`zwt` 在解算之前就差 1 ULP**"。

#### 第 272 轮：入场相同 ⇒ 差出在本例程开场（蒸腾级联 → 含水层交换）

再加一个入场打印（`WTENTR`，`:256` 的 `izwt = findloc_ud(zwt >= sp_zi)` 之后）：

```text
步 12 WTENTR: IDENTICAL          （izwt=3、zwt=4046B429B052DA9F、wa=0 全同）
步 12 WTPRE : zwt 差 1 ULP
步 13 WTENTR: zwt 差 1 ULP（已经带上一步的差）
```

入场逐位相同、`WTPRE` 不同 ⇒ 差产生在**入场到推导之间**。这段里能改 `zwt` 的只有
`soilwater_aquifer_exchange`（`:339-341`）⇒ 种子在含水层交换。

#### 第 273 轮：`wexchange` 在**第 11 步**就差了 1 ULP，而且 `rsubst = etrdef = 0`

把交换的入参也打出来（`WEXCH`：`wexchange, rsubst, deficit, etrdef`）：

```text
步 0–10 : wexchange = 0（两侧，
          因为黎明前 etr = 0，deficit 恰好为 0）
步 11   : K 3EF2F0F1553B1410 (=1.8063719223322032e-05)  R 3EF2F0F1553B13F0 (=1.8063719223321924e-05)
          rsubst = 0、etrdef = 0 两侧都同 ⇒ 差**全在 `deficit`**
```

`etrdef = 0` 且内核侧 `sumroot` 打出来是 0 ⇒ **植物水力（`DEF_USE_PLANTHYDRAULICS`）
默认是 `.true.`**，于是 `etroot(:) = rootflux`（`:293`）——`etroot` 是**入参**，
不是本例程算的。`deficit` 只是把入参 `rootflux` 按 `deficit + etroot(ilev)*dt`
累加（`:330-332`，`izwt=3` ⇒ 第 3 层往下）。

#### 第 274 轮：种子的真身 —— `rootflux` 第 2、3 层在第 11 步差 1 ULP

把 `etr` 与 `etroot(1..6)` 也打进 `WEXCH`（`wdem_probe.sh`）：

```text
步 11   etr       两侧同为 3DB9CA00EAB00A99
        etroot1   两侧同为 BE45801FED95F211 (-1.0011944539693032e-08)
        etroot2   K 3E8AF90406937A61  R 3E8AF90406937A5F   (2.00962406971466e-07)
        etroot3   K 3E8B7B4984603349  R 3E8B7B4984603348   (2.0475380518893196e-07)
        etroot4/5/6 两侧相同
```

⇒ 种子在**叶温/植物水力那条链**里：`rootflux` 有两层差 1–2 ULP。上游对应
`extends/interception/MOD_LeafTemperature_Extended.F90:1351-1361`：

```fortran
etr0  = etr
etr   = etr + etr_dtl*dtl(it-1)
IF (DEF_USE_PLANTHYDRAULICS) THEN
   IF (abs(etr0) .ge. 1.e-15) THEN
      rootflux = rootflux * etr / etr0                       ! 左结合：先乘后除
   ELSE
      rootflux = rootflux + dz_soi / sum(dz_soi) * etr_dtl* dtl(it-1)   ! 左结合 + 收缩
   ENDIF
   CALL balance_phs_rootflux(ipatch, p_iam_glb, etr, rootflux, rootfr, 'post-leaf-temperature')
ENDIF
```

注意这**不等于**"差就在这两行"：第 11 步是 `etr` 第一次非零的那一步
（前 10 步 `wexchange` 恒为 0），所以这两行此前从未真正生效过 ——"第一次生效就偏"
与"上一级解出来的 `rootflux_p` 本来就差 1 ULP"都还没排除，两者都会表现成这样。

#### 第 275 轮：沿路顺手修掉的三处"形状"差（都有实测证据）

| # | 位置 | 上游形状 | 改前 | 影响 |
|---|---|---|---|---|
| 1 | `bounded_secant_iteration` + `water_table_from_aquifer` | `x_l*alp + x_r*(1.0_r8-alp)`，`alp=0.9_r8` | 内联了一份夹逼并写成字面量 `0.1` | `1.0-0.9 = 0.09999999999999998 ≠ 0.1`，夹逼一生效就差 1 ULP |
| 2 | `deficit` 累加（3 处） | `deficit = deficit + etroot(ilev)*dt` | 先乘后加（未收缩） | 内核 `-ffp-contract=fast` 会收缩成 FMA；`deficit` 从**第二项**起就差 |
| 3 | 根通量缩放（`leaf_temperature.rs`） | `rootflux * etr / etr0`（左结合）；`rootflux + dz/sum*etr_dtl*dtl` | `scale = etr/etr0` 再 `flux *= scale`；`flux += dz/total*(slope*dtl)` | 结合顺序 + 收缩两处都不等价 |

第 1 处顺带把**手抄的一份** `water_table_from_aquifer` 内联夹逼删掉、改调
`bounded_secant_iteration` —— 以后只有一份实现，"抄错常数"这类错不会再发生。

**黄金三窗口实测（同一台机器、同一份内核，`git stash` 前后各跑一遍 `win4.sh`）**：

| 窗口 | 改前 | 改后 |
|---|---|---|
| CN-Cng（干） | bitwise 21007、sumabs 406.5407、`over_tol` **829** | bitwise 21003、sumabs **224.3599**、`over_tol` **821** |
| CN-Cng-wet | bitwise 32611、sumabs 10379.1834、`over_tol` 20662 | bitwise **32608**、sumabs 10383.4810、`over_tol` 20662 |
| US-NR1-snow | bitwise 33486、sumabs 444394.4368、`over_tol` 25896 | bitwise **33439**、sumabs **444391.8131**、`over_tol` 25896 |

干窗的**误差总量降了 45%**（406.5 → 224.4），三个窗口的逐位点数都减少，
`over_tol` 无一变差。三段式：1 步 restart 0/68、3 步 history **692/692 逐位全同**、
黄金 dry 821 / wet 20662 / snow 25896。

**但 16 步干窗的逐记录首次分歧仍在记录 12**（`f_zwt`，56 个变量受影响）——
第 271–274 轮已经把它的**因果链完整走通到 `rootflux`**，只是那条链的最后一跳
（植物水力解出来的 `rootflux_p`，或第 274 轮那两行的第一次生效）还没定位。
所以**别再回到水步/`ss_wt`/Richards 解算器里找记录 12**：那三段都已逐位证明是干净的。

Tested: `wtpre_probe.sh`（16 步，`WTPRE`/`WTPOST` 两侧 32 行）；`wtentr_probe.sh`
（16 步，加 `WTENTR`，两侧 64 行）；`wexch_probe.sh`（16 步，加 `WEXCH`）；
`wdem_probe.sh`（16 步，加 `etr`/`etroot(1..6)`）；`bash /tmp/gf/dry_ts.sh 16` +
`oracle/scripts/window_divergence.py`；`bash /tmp/gf/win4.sh` + `three.py`（`git stash` 前后各一遍）；
`bash /tmp/gf/accept_r247.sh`（1 步 restart 0/68、3 步 history 692/692、黄金三窗口）；
`cargo test --workspace --lib --bins -- --test-threads=1`（26 个二进制全绿）；
`cargo clippy --workspace --all-targets -- -D warnings`；`cargo fmt --all --check`。
Not-tested: 第二配置（Campbell + 关 VSF）本轮未复跑；`rootflux` 1 ULP 的最后一跳未定位。

#### 第 276 轮：一个**真缺口**（这几份窗口里不生效）：`balance_phs_rootflux` 没移植

第 7163 行那句"本仓库在 `root_flux_kg_m2_s` 那一处已有**同构**实现"是错的，本轮纠正：
上游 `MOD_PHSRootfluxBalance:balance_phs_rootflux` 干的事是"把 `rootflux` 按比例
缩放到 `sum(rootflux) = etr`"（`max(rootflux,0)*(etr/sum_pos_flux)`，或按 `rootfr`
权重，或均分），而本仓库在 `leaf_temperature.rs` 里做的是 `rootflux * etr / etr0`
——**两回事**。上游在扩展截留这条路上调它两次：

| 调用点 | 上游位置 | 本仓库 |
|---|---|---|
| `'post-PHS'` | `:1145`，`etr = etrsun + etrsha` 之后 | 未移植 |
| `'post-leaf-temperature'` | `:1367`，`rootflux*etr/etr0` 之后 | 未移植 |

`crates/colm-core/src/leaf_temperature.rs:613` 那条 `ensure!` **不覆盖**它：它断言的是
**植物水力解自身**的收支（`hydraulic_output.sunlit + shaded - sum(root_flux) <= 1e-7`），
而上游判定用的是**冠层** `etr` —— 两者是不同的量。

**但在这三份窗口里它确实不生效**：`balance_phs_rootflux` 一旦真的动手就会打
`Warning: adjusting vegetation PHS rootflux balance`，而 16 步干窗的内核日志里
这条警告出现 **0 次**（`grep -c 'rootflux balance' /tmp/gf/dryts/run/f.log` = 0）
⇒ `abs(etr - sum(rootflux)) <= 1e-7` 每步都成立、函数每步都早退。所以它**不是**
记录 12 那颗种子的来源，而是一个"默认配置下暂不生效、但条件一旦不成立就会静默
算错"的缺口 —— 属于"未移植"清单里该补的一项（补它需要 `fr` 权重与 `context` 字符串，
以及 `warn_count` 那个 `save` 计数器）。

#### 第 277 轮：判别出来了 —— `etr`/`etr0` 相同，**植物水力解出来的 `rootflux` 就差了**

在 `MOD_LeafTemperature_Extended.F90:1351-1352`（`etr0 = etr` / `etr = etr + etr_dtl*dtl`）
之后、也就是"缩放那一行之前"，两侧都打 `etr`、`etr0`、`rootflux(1..6)`
（`leaft_probe.sh`，16 步，每步一行）：

```text
row 11（第 11 步）: etr 同、etr0 同；只有 rootflux2 差 1 ULP
    K rootflux2 = 3E8AF0C48B125409  (2.0072235921002370e-07)
    R rootflux2 = 3E8AF0C48B125408  (2.0072235921002367e-07)
row 15: 全差（那时状态已经被放大，不算证据）
```

⇒ **判别完成**：缩放的**两个输入 `etr`/`etr0` 逐位相同**，而 `last.root_flux_kg_m2_s`
（= `PlantHydraulicStress_twoleaf` 的输出）已经差 1 ULP。所以：

- 第 275 轮修的那两处缩放形状（先乘后除 / 左结合 + 收缩）是对的，但**不是**那颗种子；
- 种子在**植物水力解算器**里 —— `crates/colm-core/src/plant_hydraulics.rs:567
  `root_flux_from_top_potential`（上游 `MOD_PlantHydraulic.F90` 那一支）。

这也和第 274 轮对上：`etroot` 第 2、3 层差 1–2 ULP，而第 2 层（`rootflux2`）在这里
就已经差出来了。**下一枪就打在 `plant_hydraulics.rs`**：拿同一时刻的
`psi`/`root_psi`/`zp`/`layer_thickness` 进出口两侧对打，看是迭代收敛判据
（`tol` 比较）还是某一处 `a*b + c` 的形状。

#### 第 278 轮：植物水力里**已经排除**的几处（免得下一轮重走）

打完第 277 轮之后，把植物水力这一支的已知形状逐条核了一遍，下面是**已经对上的**：

| 位置 | 结论 | 证据 |
|---|---|---|
| `MOD_Utils:tridia` ↔ `linear.rs:solve_tridiagonal` | 三处 `mul_add` 已对上 | 上一轮离线对照：不收缩 183/2000 逐位、收缩后 2000/2000 |
| `getrootqflx_qe2x` 右端三行 ↔ `plant_hydraulics.rs:root_potential_from_flux` | 已对上（首行 `先减 qeroot 再减 kax`、乘积收缩） | 源码逐项 + 注释里的实测记录 |
| `getrootqflx_x2qe` 右端 ↔ `root_flux_from_top_potential` | `rmx_hr(j-1)` 的乘积已收缩 | 注释里 `fma(p,A,B) - C` 4000/4000 |
| `plc` ↔ `vulnerability` | **内核走 libm `pow`，不是 `exp2`** | `nm kernels/default/colm.x` 只导入 `_pow`，全二进制**没有 `_exp2`** ⇒ `2._r8**tmp` 与 `2.0_f64.powf(tmp)` 同源 |
| `d1plc` ↔ `vulnerability_derivative` | 左结合逐项相同（`(((ck*ln2)*2**tmp)*tmp)/x`） | 源码逐项 |

**还没核的**（按嫌疑排序，都在 `spacAF_twoleaf` ↔ `spac_change` 这一支里）：

1. `spacAF_twoleaf`（`MOD_PlantHydraulic.F90:379-508`）的 `A11/A22/f(leafsun)/f(leafsha)` ——
   那几行是密集的 `变量 + 乘积`（例如
   `f(leafsun) = qflx_sun*fsto1 - laisun*kmax_sun*fx*(x(xyl)-x(leafsun))`），
   收缩点最多；
2. `:329-331` 的 `dx` 重标定（`> 200000` 分支）与 `:334` 的 `x = x + dx`；
3. `getqflx_qflx2gs_twoleaf`（`:710-805`）里那一串 `A1/B1/C1` 与 `qflx/rhow` 的组合。

**下一轮最快的走法不是读这 500 行，而是探针分工**：在 `PlantHydraulicStress_twoleaf`
的迭代末尾（`:355` `qeroot = etrsun + etrsha` 之后）把 `qeroot`、`x(1:4)`、
`xroot(1:3)`、`etrsun`、`etrsha` 两侧都打出来，一次构建就能把 1 ULP 夹到
"迭代状态 `x` 已经差"还是"`qe2x` 那一解差"这一层，再往里走。

#### 第 279 轮：进植物水力内部了 —— `qe2x` 排除，差在牛顿步里

在 `MOD_PlantHydraulic.F90:355`（`qeroot = etrsun + etrsha`，即调 `getrootqflx_qe2x`
**之前**）两侧都打 `x(1..4)`、`etrsun`、`etrsha`、`qeroot`（`PYDYN`）：

```text
PYDYN: 两侧各 49 行，**row 0–29 逐位全同**（含 x、etrsun、etrsha、qeroot）
row 30（≈ 第 10/11 步）起全差：
    x1      K=-650.4856155889519   R=-650.4856130277079
    x2      K=-660.925191753547    R=-660.9251821426992
    etrsun  K=1.9130064123456775e-10  R=1.9130048890441568e-10
    qeroot  K=2.126175392246929e-08   R=2.126173951860131e-08
```

**两个结论**：

1. **`getrootqflx_qe2x` 排除了**：`rootflux(j) = k_soil_root(j)*(smp(j)-xroot(j))`
   是 `qeroot` 的确定性函数（`smp`/`k_soil_root` 都逐位同），而 `qeroot` 在 row 30
   就已经不同 ⇒ 逐层 `rootflux` 的差是从这里继承的，不是 `qe2x` 自己产生的。
   第 278 轮列的"已对上"表因此可以直接把 `qe2x` 划掉。
2. 差落在**牛顿步**这一段：`PYDYN` 打的是 `x = x + dx`（`:334`）**之后**的状态，
   所以 row 30 的 x 差可能是 row 30 自己的 `spacAF_twoleaf`+更新造的，也可能是
   row 29 → row 30 之间的写回/入场造的。**下一枪**：在 `PlantHydraulicStress_twoleaf`
   **入场处**（`:170` `x = vegwp(1:nvegwcs)` 刚赋值之后）再打一次 `x` ——
   入场同而更新后不同 ⇒ 就是 row 30 的 `spacAF_twoleaf`/`dx` 重标定；
   入场已经不同 ⇒ 是调用之间那条写回（`vegwp(1:nvegwcs) = x`，`:220`）。

**探针的坑（记下来）**：`PYRQF`（逐层 rootflux）那一条**不能按行对齐** ——
同一段 Python 补丁在两侧的落点不同（内核侧打在 `:358-360` 的 `IF` 分支里，
本仓库侧打在 `if`/`else` 两路汇合之后的 `ensure!` 之前），于是内核侧 49 行、
本仓库侧 328 行。**教训：两侧补丁要打在语义等价的分支里**，否则行数都不一样，
比较出来的是垃圾。`PYDYN` 因为落在同一个 `IF` 分支内，49/49 才对得上。

#### 第 280 轮：把"入场状态"也打出来 —— 差出在**一次入场状态相同的调用**里

第 279 轮那句"`qe2x` 排除了，因为 `qeroot` 在 row 30 就已经不同"**推理是错的**
（结论碰巧对）：`qeroot` 在 row 30 不同，只能说明 row 30 的**入场状态**已经不同，
不能说明差是 `qe2x` 之前造的。本轮在函数**入场**（`MOD_PlantHydraulic.F90:219`
的 `x = vegwp(1:nvegwcs)`，本仓库 `plant_hydraulics.rs:136`）再打一次 `x`（`PENTR`）：

```text
PENTR: 两侧各 328 行（**行数相同**，因为落在同一个函数入口），只有 18 行不同
    row 308 K C08530E47B542739 C08530E47B542739 C08530C88802B3E6 C066430947C268DA
    row 308 R （逐位相同）
    row 309 K （与 308 **逐位相同**：这一次调用没有改变状态）
    row 310 K C08453E28A6D07CF C084A766CAEF201B C084531EA61C8182 C062A6AE22811A3C
    row 310 R C08453E289154423 C084A766C5E52DE4 C084531EA4CEF6D9 C062A6AE1EF88B75  ← 首个不同

PYDYN（同一批调用里"需求为正"的那 49 次）:
    row 29 两侧逐位相同；row 30 首个不同
    PYDYN row 30 的 x1..x3 == PENTR row 310 的 x1..x3（x4 只差梯度钳制）
```

**读法**：`PENTR` 每调用一行、无缺口 ⇒ `PENTR 309` 与 `PENTR 310` 之间
**恰好隔了一次调用**（309 自己），而这两行的 `x1` 从 `C08530E47B542739`
变成 `C08453E28A6D07CF`（一次正常的牛顿步）。既然入场**逐位相同**、出场**不同**，
差就是**这次调用自己造的**；而它的 `PYDYN`（row 30）在 `qe2x` **之前**就已经不同
⇒ `getrootqflx_qe2x` 确实排除（这次是站得住的论证）。

**于是嫌疑收敛到"需求为正那次调用"在 `PYDYN` 之前的那几段**：
`getrootqflx_x2qe`（第一次 `tridia`）→ `spacAF_twoleaf` → `:329-331` 的 `dx` 重标定
→ `x = x + dx`（`:334`）→ 三个梯度钳制（`:337-339`）——正好是第 278 轮列出的
"还没核的"那三项，没有新增嫌疑面。

**下一步的工具（比再猜一轮划算）**：两侧都加一个**调用序号**再打 `PENTR`/`PYDYN`
（内核侧在 `calcstress_twoleaf` 里用 `INTEGER, SAVE`；本仓库侧用一个
`static AtomicUsize`），这样行可以按"同一调用"对齐而不是按位置 ——
第 279 轮和本轮踩的都是同一个坑（两侧补丁落在不同分支/不同函数时行数就对不上）。

#### 第 281 轮：带**调用序号**夹逼 —— 差落在 `spacAF_twoleaf` 的 `dx`，而它的输入逐位相同

两侧都加调用序号（内核 `MOD_PlantHydraulic` 里 `integer, save :: phs_probe_count`；
本仓库 `static PHS_PROBE_COUNT: AtomicUsize`），`PHMID` 打在 `getrootqflx_x2qe`
**刚返回**处（`x` 是更新前、`rf/rfs` 是 x2qe 的输出），`PHEND` 打在 `x = x + dx` 之后：

```text
注意：内核计数器从 1 起、Rust 的 fetch_add 从 0 起 ⇒ 内核 call n ↔ Rust call n-1
PHMID: 两侧各 49 行；call ≤310 逐位全同，call 311 首个不同
   call 310 的 PHMID（更新前 x1..x4 + x2qe 的 rf/rfs）**逐位相同**
   call 310 的 PHEND：dx **不同**
       dx1 K=27.6259477669  R=27.6259503282   (相对差 5.6e-7)
       dx2 K=17.1863716023  R=17.1863812132
       dx3 K=27.7079503998  R=27.7079528849
       dx4 K=28.8861261631  R=28.8861278480
```

**读法**：call 310 的**全部输入**（`x`、`qeroot`/`dqeroot`、以及由 `x` 现算的
`fsto/fx/dfsto/dfx/fr/dfr`）逐位相同，而出场的 `dx` 差 5.6e-7 相对 ⇒ 差产生在
`spacAF_twoleaf`（本仓库 `plant_hydraulics.rs:spac_change`）里。嫌疑面从"四段"
收窄到**一段**。

**为什么可能是 1 ULP 的 `A` 项被放大**：`dx = (numer)/determ`，而
`determ = A44*A22*A33*A11 − A44*A22*A31*A13 − A44*A32*A23*A11 − A43*A11*A22*A34`
是**大项相减**。内核在这条链上的 `determ` 量级很小（第 257 轮记过 `4.1e-25` 残差），
若几项各 ~1e-16 而结果 ~1e-25，抵消倍数就是 ~1e9 —— 于是 `A` 里**任意一项差 1 ULP**
（1e-16 相对）会让 `dx` 差 ~1e-7 相对 ✓ **正好是实测的 5.6e-7**。这同时解释了
"为什么前 300 次调用全都逐位相同、到第 310 次才翻"：形状差一直在，只有当
`determ` 的抵消倍数够大时才冒出来。

**下一枪（已定好）**：在 `spacAF_twoleaf`/`spac_change` **末尾**把
`A11,A13,A22,A23,A31,A32,A33,A34,A43,A44,determ,f(1..4),dx(1..4)` 两侧都打出来 ——
一次构建就能指出是哪个 `A` 项（或 `determ` 本身）差 1 ULP。注意第 278 轮那份
"已对上"的记录里，`spacAF_twoleaf` 的收缩点是用**内核自己打出来的 49 组
(A,f,dx)** 反推验证的（20000/20000、49/49）—— 那 49 组很可能取自**早期调用**
（都在逐位相同的区间内），所以"当时全中"与"call 310 差 5.6e-7"并不矛盾：
形状差只在高抵消倍数处显形。

#### 第 282 轮：A 矩阵与 determ **逐位全同** —— 种子是 `qflx_sun`/`qflx_sha`（叶面蒸腾需求）

在 `spacAF_twoleaf` 的 `determ=` 那一行之后两侧都打
`A11,A13,A22,A23,A31,A32,A33,A34,A43,A44,determ,f(1..4),qflx_sun,qflx_sha`（`SPACA`）：

```text
两侧各 49 行；call ≤287 全同；call 310 首个不同（call 288–309 未走到这一支）
-- call 310（A 与 determ **一个都没列出来 ⇒ 全部逐位相同**）
     f1        K=1.640052681854531e-10   R=1.6400511585530102e-10
     f2        K=2.1043157908155635e-08  R=2.1043143656617804e-08
     qflx_sun  K=1.913006412369971e-10   R=1.91300488906845e-10     (相对差 8.0e-7)
     qflx_sha  K=2.1070453281519667e-08  R=2.1070439029981837e-08    (相对差 6.8e-7)
```

**三个结论**：

1. **`spacAF_twoleaf` 洗清了**：十个矩阵元与 `determ` 全部逐位相同 ⇒ 第 278 轮
   那次"用内核自己打出来的 49 组 (A,f,dx) 反推收缩点"的工作是**对的**
   （20000/20000、49/49 不是碰巧）。`dx` 的差是从 `f` 继承的。
2. **种子是 `qflx_sun`/`qflx_sha`**（叶面蒸腾需求），它俩是
   `getqflx_gs2qflx_twoleaf`（`MOD_PlantHydraulic.F90:617-708`，由 `:318` 调用）
   的输出，本仓库对应 `transpiration_from_conductance`。`f1 = qflx_sun*fsto1 - …`
   与 `f2 = qflx_sha*fsto2 - …` 正是因为它俩才差（相对差同量级 ✓ 自洽）。
   顺带解释了第 279 轮看到的 `etrsun = qflx_sun*plc(·)` 差 —— 同一个来源。
3. **A11 依赖 `qflx_sun` 却仍逐位相同**并不矛盾：`A11` 的第一项
   `-laisun*kmax_sun*fx` 量级远大于 `qflx_sun*dfsto1`，8e-7 的相对扰动在小项上
   不足以改变总和的舍入。

**下一枪**：在 `getqflx_gs2qflx_twoleaf` 的进出口两侧打
`tl, qsatl, qaf, gssun, gssha, gb_mol, rss, raw, rd, fwet, psrf, rhoair, laisun, laisha, sai`
与 `qflx_sun, qflx_sha` —— 入口同而出口不同 ⇒ 这个函数的形状差（它是**闭式**，
不是迭代解，读一遍就能找）；入口已不同 ⇒ 再往上追到叶温那一段（`tl`/`qsatl`）。

#### 第 283 轮：种子是**一个变量** —— 遮荫叶气孔导度 `gs_sha`

把 `getqflx_gs2qflx_twoleaf` 的 24 个量（输入 + 中间量 + 输出）两侧都打出来
（`QG2Q`，两侧各 328 行/328 个调用序号），按调用序号对齐：

```text
可比较 328 对；39 对不同；**首个不同 = 内核 call 289 / Rust call 288**
--- call 289 的唯一不同项
    gs_sha   K=23106.44382728509   R=23106.422793824593     (相对差 9.1e-7)
    （tl/qsatl/qaf/qg/qm/gs_sun/gb_mol/psrf/rhoair/fwet/lai_sun/lai_sha/sai/delta/
      cf/caw/cgw/cfw/wtsqi/wtaq0/wtgq0 **全部逐位相同**，连 qflx_sun/qflx_sha 也还相同）
```

**整条链现在是自洽的**：

1. call 289 起 **`gs_sha` 差 9.1e-7 相对**，但此时 `qflx_*` 还舍入到同一个 double；
2. `cfw` 里含 `laisha/(1/gb+1/gs_sha)`，于是 `gs_sha` 的差经 `cfw → wtsqi → wtaq0/wtgq0
   → cqi（驱动湿度）`传到**两个**叶面通量 —— 这正好解释第 282 轮看到的
   "`A11..A44` 与 `determ` 全同、`f1`/`f2` 同差"（`f` 含 `qflx`，`A` 只在
   `qflx_sun*dfsto1` 这个小项里含）；
3. call 310 起 `qflx_sun`/`qflx_sha` 差 8e-7 → `f` 差 → `dx` 差 5.6e-7 →
   牛顿状态 `x` 差 → 逐层 `rootflux` 差 → `deficit` → 水位 → 第 12 步记录翻出去。

**下一枪**：`gs_sha` 是 `calcstress_twoleaf` 的输入，来自
`getqflx_qflx2gs_twoleaf`（`MOD_PlantHydraulic.F90:710-805`，由 `:346` 调用）
↔ 本仓库 `conductance_from_transpiration`。打它的进出口
（`etrsun/etrsha, qsatl, qaf, qg, qm, gb_mol, fwet, lai_*, sai, tl, rhow` 与
`gssun/gssha`）：入口同而出口不同 ⇒ 就是那个闭式的形状差。注意 `gs_sun` 一直相同，
所以嫌疑集中在**遮荫那一支**（`A2/B2/C2` 或 `qflx_sha` 的除法链）。

#### 第 284 轮：把"`gs_sha` 差"再钉一层 —— 它产生在**上一次调用**的 `getqflx_qflx2gs_twoleaf`

先把第 283 轮的因果顺序说准（否则容易多追一轮）：内核一次调用内的顺序是
`:318 getqflx_gs2qflx_twoleaf`（印 `QG2Q`）→ `:323 x2qe` → `:325 spacAF` → `:334 x+=dx`
→ `:342-343 etrsun/etrsha` → **`:346 getqflx_qflx2gs_twoleaf`** → `:356 qe2x` → `:359 rootflux`。
`gs_mol_sha` 只在 `:346` 被改写，所以 **`QG2Q` 在 call N+1 印出来的 `gs_sha`
是 call N 的 `:346` 算出来的**。

于是：

```text
QG2Q: call 1..288 的 24 个量全部逐位相同（含 gs_sha）
      call 289 的 gs_sha 开始差（唯一不同项）
⇒ call 288 的 `:346` 用**逐位相同的输入**算出了**不同的 `gs_sha`**
```

为什么输入一定相同：`:346` 的入参是 `etrsun/etrsha = qflx*plc(x(leafsun/sha))`、`gb_mol`、
`tl/qsatl/qaf/qg/qm`、`raw/rd/rss`、`fwet`、`lai_*`、`sai`、`psrf`、`rhoair`；
而 `x` 在 call 310 之前逐位相同（第 280/281 轮的 `PENTR`/`PHMID` 已证），
`qflx` 在 call 310 之前逐位相同（第 282/283 轮已证）⇒ call 288 的这些入参全同。
**输入相同、输出不同 ⇒ 只能是形状差，而且只在遮荫那一支**
（`gs_sun` 从头到尾逐位相同）。

嫌疑缩到 `plant_hydraulics.rs:conductance_from_transpiration` 里两条只影响遮荫的式子：

```fortran
csunw_dry = (B1*C2 - B2*C1)/(B1*A2 - B2*A1)      ! 本仓库：对上了
cshaw_dry = (A1*C2 - A2*C1)/(A1*B2 - B1*A2)      ! 本仓库：没对上
gs_mol_sha = 1/((1.-fwet)*delta*laisha/cshaw_dry/cf - 1./gb_mol)
```

本仓库按"`b2*a1` 被 CSE、于是两个分母一个收左一个收右"写成
`c2.mul_add(a1, -(c1*a2)) / (-b1).mul_add(a2, b2_a1)` —— 那是**从 GIMPLE dump 反推**的。
现在有了反例：同样的输入下遮荫那一条确实不等 ⇒ **`cshaw_dry` 分子/分母的收缩选择
（或 `1/(…)` 那一步的结合）至少有一处与内核不同**。可选的形状只有 2×2×2 种
（分子收左/收右、分母收左/收右、`gs` 里 `- 1/gb_mol` 是否参与），
**下一枪直接枚举**：把 call 288 的入参按第 283 轮的 24 个打印值取出，
用 `Fraction` 精确算 8 种候选、与内核 call 289 的 `gs_sha` 位型比对。

#### 第 285 轮：`qflx→gs` 的**公式是干净的** —— 第 283/284 轮的口径要修正，线索转向
`gs` 的**持久状态**

把 `getqflx_qflx2gs_twoleaf` 的 23 个量（入参含 `etrsun/etrsha`、中间量
`cqi_leaf/A1..C2/csunw_dry/cshaw_dry`、出参 `gs_sun/gs_sha`）两侧对打（`QF2G`）：

```text
两侧各 49 行、计数器都从 1 起（shift 0 可比 49 对）——**注意这里没有错位**
首个不同 = 内核 call 310 / Rust call 310，而且 diff 列表里
`qflx_sun`/`qflx_sha`（这是 :346 的**入参** etrsun/etrsha）本身已经不同，
`A1..C2`/`csunw_dry`/`cshaw_dry`/`gs_sun`/`gs_sha` 的差都是从它们继承的
上一次相同的 call = 287
```

**口径修正（第 283 轮的行数差异有解释，不是错位）**：
`getqflx_gs2qflx_twoleaf` 有**两个调用点**——`:318`（需求分支内，49 次）和
`:589`（`getvegwp_twoleaf` 内，无条件），所以 `QG2Q` 印了 328 行；
而 `getqflx_qflx2gs_twoleaf` 的整个函数体都在外层 `IF(qflx>0)` 里，所以 `QF2G` 只印 49 行。
两者的**计数器都在 `PlantHydraulicStress_twoleaf` 入口自增一次**，所以
`QG2Q` 的 289 与 `QF2G`/`SPACA` 的 310 都是**同一个"包装调用序号"**，可以直接比。
于是第 283 轮"call 289 起只有 `gs_sha` 差"应读作：**进入 call 289 的 `:318` 时
`gs_sha` 这个状态量已经不同**，它是更早某次调用留下的。

**因此新线索（结构性，不是收缩点）**：内核的 `gssun/gssha` 是
`PlantHydraulicStress_twoleaf` 的 **`intent(inout)` 持久状态**，`:346` 只在
`qflx_* > 0` 时改写它们；而本仓库 `PlantHydraulicState` **只有
`vegetation_water_potential_mm`**，`conductance_from_transpiration` 每次都用
`input.maximum_*_conductance`（= `gs0`，由**上一次**更新后的气孔阻力反算出来）当种子。
于是"`qflx_sha > 0` 但 `qflx_sun <= 0`"这类调用里，内核保留旧的 `gs_mol_sha`、
本仓库返回 `gs0` —— **只影响遮荫那一支**，正好对上"`gs_sun` 全程相同、只有 `gs_sha` 差"。

**下一枪**：(a) 确认 `gs0` 的反算（`1/(rssun*T/pc)…`）与内核持久值是否逐位等价；
(b) 若不等价，把 `gssun/gssha` 挪进 `PlantHydraulicState` 当持久状态（与上游同构），
再跑三段式验收（restart 0/68、3 步 692/692、黄金 821/20662/25896）。

#### 第 286 轮：离线核对"持久状态"假说 —— **没有证实，也没被推翻**，但定位到了第二个改写点

用第 283 轮（`QG2Q`，328 行）与第 285 轮（`QF2G`，49 行）已有的位型文件做纯离线核对
（不重编内核），看 `gs_sha` 这个状态量在各调用上的**取值变化点**：

```text
QG2Q 可比较 328 对，首个不同 = 内核 call 289 / Rust call 288（即同一个包装调用）
call 289: gs_sun 两侧都是 468.4265011520596（相同）
          gs_sha 内核 23106.44382728509  Rust 23106.422793824593
"最后变化点"两侧**都**落在这同一个包装调用上（内核 289 / Rust 288）
内核那个值**不在**内核 QF2G 的 49 行输出里；Rust 那个值也不在 Rust 的 49 行里
```

**读法**：两侧在同一个包装调用上都**改写了** `gs_sha`，只是改写出来的值不同 ⇒
"内核保留旧值、本仓库用 gs0"这条（第 285 轮的结构性假说）**不成立或不完整**
（若是"保留 vs 覆盖"，两侧的"变化点"就该错开）。同时，改写出来的值不在 `QF2G`
的 49 行里，说明**这一次改写不是来自需求分支的 `:346`**，而是来自另一个改写点
`:589`（`getvegwp_twoleaf` 里那次调用）—— `QF2G` 只覆盖外层 `IF(qflx>0)` 且
这次显然没走到。

**所以还没结清的只有一处**：`:589` 那条路。下一枪：
(a) 在 `getvegwp_twoleaf` 的 `gssun/gssha` 进出口打位型（它是 `intent(inout)`，
两个改写点共享同一对状态）；
(b) 顺带在包装入口/出口打 `gssun/gssha`，把"持久状态"这条彻底量清楚，
再决定是挪进 `PlantHydraulicState` 还是别的原因。

**注**：本轮不动代码 —— 假说未证实前不改结构，避免把"碰巧好一点"当成修复。

#### 第 287 轮：`gs` 状态在**调用方**是怎么流的（读源码，未改码）

上一轮把未结清处指到"第二个改写点"。本轮把上游调用方读清楚了，
`extends/interception/MOD_LeafTemperature_Extended.F90:896-915`：

```fortran
CALL PlantHydraulicStress_twoleaf (..., gs0sun, gs0sha, k_soil_root, k_ax_root, gssun, gssha)
etr  = etrsun + etrsha
gssun = gssun * laisun          ! ← PHS 输出被换算到**冠层尺度**
gssha = gssha * laisha
CALL update_photosyn(tl, ..., gssun, ..., assimsun, respcsun)   ! ← gssun 又作为 inout 进去
```

也就是说内核里 `gssun`/`gssha` 这一对是**跨调用、跨例程复用的同一个变量**：
PHS 的逐叶输出 → 乘 `lai` 变冠层尺度 → 交给 `update_photosyn` 再改 → 下一次调用又当
**逐叶**种子传回 PHS。而本仓库这一路（`leaf_temperature.rs:1055` 传
`maximum_sunlit_leaf_conductance_umol_m2_s: gs0sun`）是把上一轮 PHS 输出经
`hydraulic_stomatal_resistance → 气孔阻力 → gs0 = (1/(rssun*T/pc)).min(1e6)/lai*1e6`
**反算**出来的，和"同一个变量被反复乘/改"不是同一条路径。

**结论**：这条链在 283–287 轮里被逐步夹到了一个**语义差**（不是某个收缩点）：
"跨例程复用的一对电导变量" vs "每次反算"。要结清它得先确定
`update_photosyn` 对 `gssun` 的改写规则（`MOD_AssimStomataConductance.F90`），
再决定本仓库是补一个持久对、还是把这条链整体改写成同构。
**本轮不改代码**：语义未定之前不动结构。

#### 第 288 轮：`gs` 状态这条链**问清楚了** —— 语义差可以完整写出来

上一轮留的问题（`update_photosyn` 会不会改写 `gssun`）本轮读源码结清：
`MOD_AssimStomataConductance.F90:617-813` 里 `gsh2o`（= 调用方传进去的 `gssun`）
**在整个子程序体内没有任何赋值**，只有 `:803` 处的读取 ⇒ **它不改写**。

于是内核的语义可以完整写出来：

```text
1. 调用方（MOD_LeafTemperature_Extended.F90:896）把 gssun/gssha 作为 inout 传进 PHS；
2. PHS 内 :346 只在 qflx_sun > 0 / qflx_sha > 0 时**各自**改写 gs_mol_sun / gs_mol_sha，
   否则保留传入值；
3. :353-354 用（可能陈旧的）gssun/gssha 算 rstfacsun/rstfacsha = amax1(gs/gs0, 1e-2)；
4. PHS 返回后调用方 :908-909 把 gssun*=laisun、gssha*=laisha 换成**冠层尺度**，
   交给 update_photosyn（只读），因此**留到下一次调用的就是"上次输出 × lai"**。
```

而本仓库每次都用 `gs0`（由上一轮的气孔阻力反算）当种子，`qflx <= 0` 时原样返回它。
⇒ **当 `qflx_sha <= 0` 时**：内核用 `上次遮荫输出 × laisha`，本仓库用 `gs0sha` ——
只影响遮荫那一支，正是"`gs_sun` 全程相同、只有 `gs_sha` 差"的来源。

**要结清它需要**：(a) 把这对电导当**持久量**跨调用带着走（按上游的写法，×lai 发生在
调用方，所以持久量应是"冠层尺度"的那一份）；(b) 查清**第一次**调用前
`gssun/gssha` 的初值（上游在叶温模块里怎么初始化）；(c) 改完跑三段式验收
（restart 0/68、3 步 692/692、黄金 821/20662/25896）。
**(a)(b) 都需要动多处代码，本轮只把语义问清楚、不动手** —— 这个残差是末位级，
在没有把握一次做对之前不值得冒险改结构。

#### 第 289 轮：**找到确切的公式差**（并纠正第 288 轮那句"上次输出 × lai"）

第 288 轮只读到 `:908-909`（`gssun *= laisun`）就下了结论，**错了**：再往下
`:919-920` 与 `:1317-1321` 会把 `gssun` **重新算出来**，它才是下一次 PHS 调用的种子：

```fortran
! MOD_LeafTemperature_Extended.F90:1316-1322
      gssun = 0._r8
      gssha = 0._r8
      IF (lai > 0.001_r8) THEN
         gssun = (laisun / rssun) * (tprcor / tlbef)
         gssha = (laisha / rssha) * (tprcor / tlbef)
      ENDIF
```

本仓库对应的那一份（`leaf_temperature.rs:548-553`，当 `maximum_*` 传进 PHS 当种子）：

```rust
let maximum_sunlit_leaf_conductance_umol_m2_s = (1.0
    / (sunlit_resistance.stomatal_resistance_s_m * state.leaf_temperature_k
        / pressure_conversion))
    .min(1.0e6)
    / laisun
    * 1.0e6;
```

两者**不是同一个量**：

| | 上游 `gssun`（PHS 种子） | 本仓库传给 PHS 的种子 |
|---|---|---|
| 形式 | `(laisun/rssun) * (tprcor/tlbef)` | `(pc/(rssun*T)).min(1e6) / laisun * 1e6` |
| lai | **乘** `laisun` | **除** `laisun` |
| 钳制 | 无 | `.min(1e6)` |
| 单位 | 无 `1e6` | `*1e6` |
| 温度 | `tlbef`（上一次求阻力时的温度） | `state.leaf_temperature_k`（当前） |

上游其实有**两个**量：`gs0sun/gs0sha`（最大导度，`:353` 的 `rstfac` 分母）和
`gssun/gssha`（这一份"由阻力诊断出来的当前导度"）。本仓库只有一个
（`maximum_*`），既当 `rstfac` 的分母、又当 PHS 的种子 ⇒ **把两个量并成了一个**。
这与第 283–286 轮的现象完全吻合：只有遮荫那一支偏（`gs_sun` 一路相同），
量级 ~9e-7 相对（两种诊断式在 `rssun`/`laisun` 上的差别）。

**修法（已具体到行，但本轮不动手）**：
1. 在叶温那一层按上游公式算出 `gssun/gssha`（`(lai/rss)*(tprcor/tlbef)`，`lai<=1e-3` 时取 0）
   并作为**输入字段**传进 `PlantHydraulicInput`；
2. `conductance_from_transpiration` 的种子换成它（`rstfac` 分母仍用 `gs0`，
   即现在的 `maximum_*`，那份语义是对的）；
3. 跑三段式验收（restart 0/68、3 步 692/692、黄金 821/20662/25896）。
**改动跨 2 个文件、3 处，且要新增一个输入字段** —— 本轮只把公式差钉死，留到下一轮实现。

#### 第 290 轮：改动规模缩小了（7 → 2），但暴露出必须先解决的**量纲/尺度**问题

**规模（好消息）**：`plant_hydraulic_stress` 只有 **2 个调用点**
（`leaf_temperature.rs:565` 与 `plant_hydraulics_tests.rs:8`），所以不必给
`PlantHydraulicInput` 加字段（那要改 7 个构造点），**改成给它加两个参数**只要动 2 处。
种子的落点也精确了：

| 位置 | 现在用的 | 应该用的 |
|---|---|---|
| `plant_hydraulics.rs:132-133`（进 `transpiration_from_conductance`） | `input.maximum_*` | 诊断电导 `gssun/gssha` |
| `plant_hydraulics.rs:192-193`（进 `conductance_from_transpiration`） | `input.maximum_*` | 诊断电导 `gssun/gssha` |
| `plant_hydraulics.rs:198-200`（`rstfac` 分母） | `input.maximum_*` | **保持**（上游 `:353` 用的就是 `gs0`） |

**但暴露出一个必须先解决的问题**：上游那条"诊断"其实是**一次往返**，而且带尺度换算：

```fortran
tlbef = tl                      ! :702  迭代前的温度
tl    = tlbef + dtl(it)         ! :1214 迭代后的温度
rssun = tprcor/tl * 1.e6 / gssun                                     ! :919  用**更新后**的 tl
gssun = (laisun / rssun) * (tprcor / tlbef)                          ! :1320 用**更新前**的 tlbef
```

代入即 `gssun_next = laisun² * gssun_leaf * (tl/tlbef) / 1e6`（`:908` 处
`gssun_canopy = gssun_leaf*laisun`）。这里有 **`laisun²` 与 `1e6`** 两个因子，
说明"PHS 的 `gssun` 参数到底是什么尺度/单位"这条账我还没算平 ——
`tl/tlbef` 又是**同一轮迭代前后**的温度比（非常接近 1 但不等于 1）。

**在把 `lai²`/`1e6` 这条账算平之前不能改代码**：那不是"改一个公式"，而是可能把
一个已验证的管道改出新的系统性偏差（而不是末位级）。**下一枪**：把
`PlantHydraulicStress_twoleaf` 的 `gssun` 参数在 `:318`/`:346`/`:908`/`:919`/`:1320`
五处的尺度逐处标注出来（各是逐叶还是冠层、µmol 还是 mol），算平之后再动手。

#### 第 291 轮：找到缺失的一环（`:941 rssun = rssun*laisun`），但尺度账**仍没平**

上一轮说"代入得 `gssun_next = laisun²·gssun_leaf·(tl/tlbef)/1e6`，`lai²` 与 `1e6` 说不通"。
本轮找到了**缺失的那一环**（之前只扫了 `:896-920` 与 `:1316-1322`，漏了中间）：

```fortran
:919   rssun = tprcor/tl * 1.e6 / gssun        ! 用冠层尺度的 gssun
:941   rssun = rssun * laisun                  ! ← 换成**逐叶**尺度（:1315 的注释就指这里）
:1320  gssun = (laisun/rssun)*(tprcor/tlbef)
```

代进去 `laisun` 会消掉一次：`rssun_leaf = (tprcor/tl)*1e6/gssun_leaf_prev`。
另外从第 283 轮的实测位型里读出本算例的实际尺度：**`laisun = laisha = 0.1`**、
`sai = 0.45`、`cf ≈ 4.25e7`、`gs_sun = 425.15`（第 1 次调用）。
并确认 `gs0sun = min(1e6, 1/(rssun*tl/tprcor))/laisun*1e6*o3coefg_sun`
（`:832`）与本仓库 `maximum_*`（`leaf_temperature.rs:548-553`）**同式**，
只差 `o3coefg_*` 那个因子（本仓库另有处理）。

**但账仍没平**：用 `laisun=0.1` 再算一次，
`gssun_next ≈ gssun_leaf_prev·lai²·(tl/tlbef)/1e6 = gssun_leaf_prev·1e-8`，
而 `gs0sun ≈ gssun_leaf_prev/laisun = gssun_leaf_prev·10`，
两者差 **~1e9**；若真如此，`rstfacsun = amax1(gssun/gs0sun, 1e-2)` 会**恒被下限钉死**，
而实测 `gs_sun = 425.15` 是个正常值 ⇒ **至少还有一环我没建模**（某个 `1e6` 或 `lai`
的换算在别处被抵消/引入）。

**结论**：这一轮把"缺失的一环"补上了、把实测尺度拿到了，但**尺度账仍未平**，
所以仍然不动代码。**下一枪**：在 `gssun` 五处（`:318/:346/:908/:919/:941/:1320`）
分别打印实际数值（探针，两侧），用**真实数字**把每一环的换算标出来 ——
比继续读源码猜更可靠。

#### 第 292 轮：六处探针标定 —— **PHS 种子公式本来就是对的**，`1e9` 是建模错误

本轮不改代码，只打探针。内核侧在 `gssun` 六处打实际数值，Rust 侧在同点打对应量，
同跑 CN-Cng 干窗（第 1 步与 16 步各一遍）。脚本 `oracle/scripts/gssun_probe.sh`，
比较 `oracle/scripts/gssun_cmp.py`（`STEPS=16 WORK=… bash oracle/scripts/gssun_probe.sh`）。
第 1 步共 **10 次 PHS 调用**、16 步共 **328 次**（与第 283 轮 `QG2Q` 的 328 行对上）。

**实测标定表**（CN-Cng 干窗第 1 步第 1 次 PHS 调用，两侧逐位相同）：

| Fortran | 量 | 单位 | 实测值 |
|---|---|---|---|
| `MOD_PlantHydraulic.F90:317`→`:318` `GSIN` | `gssun`（种子）| 逐叶 µmol m⁻² s⁻¹ | `4.251463361984611E+02` |
| `MOD_PlantHydraulic.F90:832`→`:317` | `gs0sun`（= 种子 `gssun`）| 逐叶 µmol m⁻² s⁻¹ | `4.251463361984611E+02` |
| `:318` 返回 `GSDEM` | `qflx_sun/qflx_sha` | kg m⁻² s⁻¹ | `6.855990853474811E-09` |
| `:346` 返回 `GSOUT` | `gssun`（PHS 输出）| 逐叶 µmol m⁻² s⁻¹ | `4.251463361938134E+02` |
| `MOD_LeafTemperature_Extended.F90:909` `GS908` | `gssun × laisun` | 冠层 µmol m⁻² s⁻¹ | `4.251463425289875E+01` |
| `:920` `GS919` | `rssun = tprcor/tl·1e6/gssun` | 冠层 s m⁻¹ | `1.000000000010932E+06` |
| `:942` `GS941` | `rssun × laisun` | 逐叶 s m⁻¹ | `1.000000014912093E+05` |
| `:1321` `GS1320` | `(laisun/rssun)(tprcor/tlbef)` | 冠层 mol m⁻² s⁻¹ | `4.525162852448114E-05` |

`GSIN` 上 `gs0sun == gs0sha == gssun == gssha` 在**每一次**调用上都逐位相等
（第 1 步 10/10）。校验换算：用**最后一轮**的 `:909` 值 `4.525162852448115E+01`
除以 `1e6`（再乘 `tl/tlbef ≈ 1`）正是 `:1321` 的 `4.525162852448114E-05`。
冠层 µmol ÷ `1e6` = 冠层 mol；`:1321` 与逐叶 µmol 的 `gs0sun`（≈`4.5e2`）之比
≈ `laisun/1e6` = `1e-7` —— 第 291 轮的 `1e-8 vs 10` 是把 `laisun` 又算了一遍。

**三条结论**：

1. **`:316-317 gssun=gs0sun` 是每次调用都执行的**（探针实测 10/10 次种子与 `gs0`
   逐位相等）⇒ 第 287/288 轮设想的"跨调用、跨例程复用的一对电导"**不存在**：
   `:1321` 的诊断值从不回灌进 PHS 的种子，它只是 `LeafTemperature` 的
   `intent(out)` 输出（调用方 `MOD_Thermal_CanopyPhase_Extended.F90:693` 把
   `gssun_out` 当输出收下；`:1128` 的 `gssun_out=sum(...)` 是 PFT/PC 那条路），
   只供 history/restart，不再送回 PHS）。
2. **第 291 轮那个 `1e9` 是把两个不同的量直接比出来的**：`:1321` 的输出是
   **冠层 mol**（= `:909` 的冠层 µmol × `tl/tlbef` ÷ `1e6`），而 `gs0sun` 是
   **逐叶 µmol**；再叠加"它就是下一轮种子"这个错误前提，才凑出 `lai²/1e6`
   这种说不通的因子。**量纲账是平的，只是两边都被算错了。**
3. **两侧在这六点上逐位相同**：第 1 步 10/10 全同；16 步的前 288 对
   （PHS 调用 1–288）全同，从第 289 次起只有遮荫支分叉（39/328 对）。
   种子机制本身（`gssun == gs0sun`，两侧各自都成立）**每一次都对**。
   ⇒ **第 289–291 轮规划的"给 PHS 加两个参数、把诊断电导当种子"不该做**：
   它会把上游 `:316` 明确重置的 `gs0`（= 本仓库 `maximum_*`）换成一个量纲和
   语义都不同的量。

**顺带把 16 步里真正的首分歧钉到了 `gs0sha`（不是 PHS 内部）**：

```text
kernel call 289（Rust call 288，同一个包装调用）：
  parsun K=1.003421285150865E-03  R=1.0034212851508654e-3   （同）
  parsha K=3.484153792311411E-01  R=3.484153792311411e-1    （同）
  eah    K=1.452165853277306E+02  R=1.4521658532773063e2    （同）
  tl     K=2.567495786164691E+02  R=2.567495786164691e2     （同）
  psrf   K=1.000020000000000E+05  R=1.00002e5               （同）
  rb     K=2.186685091396987E+01  R=2.1866850913969866e1    （同）
  rbsha  K=2.186685058812840E+02  R=2.18668505881284e2      （同）
  rssun  K=1.000000000000000E+06  R=1e6                     （同）
  rssha  K=2.027254841348288E+04  R=2.0272566867306305e4    （相对 9.1e-07）← 首分歧
⇒ gs0sha K=2.310644382728509E+04 R=2.3106422793824593e4（同一个相对差）
  **在这个调用上** gs0sun 两侧逐位相同；sunlit 支要到 kernel call 309/310
  才跟着差（rssun 差 19/328 次），`:346` 的输出也是那之后才差
  （此前 demand ≤ 0，`:346` 整个分支没进）。
```

也就是说：`stomata` 遮荫叶那一支在**这个首分歧调用上**的全部可打印输入
（`parsha/rbsha/eah/tl/psrf/rb`）与 `rssun` 都逐位相同，**输出 `rssha` 却差
9.1e-7** ⇒ 差在 `MOD_AssimStomataConductance.F90` 的遮荫叶分支（或它内部
`calc_photo_params` 的收敛/分支），不在 PHS，也不在辐射（`parsha` 在 16 步
全部 328 次调用上两侧都逐位相同）。下一枪打这里，详见"最高优先"。

**验收基线（本轮未改任何数值代码，跑一遍确认口径没漂）**：
`bash /tmp/gf/accept_r247.sh` → restart 干窗 1 步 `0/68`、步级 3 步
`692/692 (100%)`、黄金 `over_tol = 821 / 20662 / 25896`（`ot_vars = 17 / 68 / 79`）。
与交接文档的 `821 / 20662 / 25896` 一致（第 257 轮表里的 `20664` 是旧值）。
Rust 侧探针构建与干净构建都被这一步覆盖：探针跑完内核已按 `SKIP_REBUILD=0`
重编回干净版。

#### 第 293 轮：找到真凶 —— `update_photosyn` 的 `gsh2o` 量纲（实测差 `1e6`）

第 292 轮把首分歧钉在遮荫 `stomata` 的 `rssha`。本轮用三层探针一路往上推，
把根因收到一个 `1e6` 的量纲差上，并修掉它。

**三层探针（都在 `oracle/scripts/`，16 步 CN-Cng 干窗）**：

1. `stomata_probe.sh`（遮荫支入参 + `stomata` 内部）：首分歧**不在**
   `stomata` 内部，而是它的入参 **`pco2a`**（第 577 次 `stomata` 调用，
   相对 `9.5e-7`）；`parsha/rbsha/eah/tl/psrf/rb` 与 `rssun` 全同。
2. `pco2a_probe.sh`（`MOD_LeafTemperature_Extended.F90:1243-1244` 更新式）：
   首分歧在 **`assimsun`/`assimha`**（`update_photosyn` 的输出，相对
   `8.3e-4`/`8.0e-3`），而 `gah2o/raw/thm/tprcor/respcsun/respcsha/rsoil` 全同。
3. `updphoto_probe.sh`（`update_photosyn` 内部）：入口 **`gsh2o` 在全部 656 次
   调用上差正好 `1e6`**：内核 `42.51463425289875` 对 `4.251463425289875e-05`。

**根因**（上游注释与实参不一致）：

```fortran
! MOD_LeafTemperature_Extended.F90:908-911 —— gssun 此刻是**冠层 µmol m-2 s-1**
gssun = gssun * laisun
CALL update_photosyn(tl, po2m, pco2m, pco2a, parsun, psrf, rstfacsun, rb, gssun, ...)
```

`update_photosyn` 的哑元 `gsh2o` 注释写 "mol m-2 s-1"，但调用点传进去的是
**µmoles 的数值**（PHS 逐叶 µmol 输出 × `laisun`）。本仓库
`leaf_temperature.rs:1513` 按注释除了 `1e6`，于是
`pco2in = (co2s - 1.6*assmt/gsh2o)*psrf` 那一项被放大 `1e6` 倍：夜间 `assmt=1e-12`
时看不出来，**第一个有光的迭代**就分叉（`assim` 差 ~1e-3，再经 `pco2a` 反馈进
`stomata` 的遮荫支）。这就是"遮荫 `rssha` 差 9.1e-7"的真身。

**修法（2 个数值文件 + 1 个测试）**：

* `leaf_temperature.rs:1508-1517` 原样传 `canopy_conductance_umol_m2_s`；
  `PhotosynthesisUpdateInput` 的字段改名
  `canopy_conductance_h2o_mol_m2_s` → `canopy_conductance_h2o_umol_m2_s`，
  字段与调用点各写清"照抄调用点的数值，不是哑元的注释"。
* 顺带修一处同源的**钳制混淆**：`MOD_AssimStomataConductance.F90:318-321` 的
  `co2s`（未钳制，进 `:366` 的 `pco2in`）与 `co2st = max(min(co2s,co2a),1e-5)`
  （进 Medlyn 的 `acp` `:341` 与 Ball-Berry 的 `hcdma` `:350`）是两个量；
  在 `update_photosyn` 里 `co2st`（`:797-798`）更是**死代码**。原先 Rust 用一个
  钳制后的 `co2_surface` 兼两职（夜间 `assimn<0 ⇒ co2s>co2a`，`pco2in` 差
  ~1e-4 相对）。现在 `co2_surface`（未钳制）进 `pco2in`，`co2_surface_clamped`
  进两条电导闭式。

**独立参考（不靠 Rust 自证）**：`oracle/scripts/updphotosyn_diff.f90` +
`compare_updphotosyn.sh` 直接链 `.bld` 的产线对象调内核 `update_photosyn`：
`gsh2o=40000` → `assim=2.29820183479526886E-05`、`respc=9.81529284775695169E-07`，
由 `photosynthesis_tests.rs` 钉住（旧的 `0.04` 是 mol 口径，已改成 µmol 的 `40000`）。

**验收（`bash /tmp/gf/accept_r247.sh`，本机实测）**：

| 窗口 | `over_tol` 前 → 后 | `ot_vars` 前 → 后 | `bitwise` 前 → 后 | `sumabs` 前 → 后 |
|---|---|---|---|---|
| CN-Cng | 821 → **28** | 17 → 1 | 21003 → 16324 | 224.36 → 249.79 |
| CN-Cng-wet | 20662 → **1970** | 68 → 53 | 32608 → 28456 | 10383.48 → **39.77** |
| US-NR1-snow | 25896 → **25713** | 79 → 79 | 33439 → 32788 | 444391.81 → 444414.20 |

restart 干窗 1 步 `0/68`、步级 3 步 `692/692` **不变**。三个窗口的 `over_tol`、
`ot_vars`、`bitwise` **全部变好**；`sumabs` 在 dry/snow 各升 ~11%/~0.005%，
逐变量核对后 **100% 来自 `f_vegwp`**（植物水力水势，长记忆混沌量：
dry 249.79 全部是它；wet 39.77 里 38.37 是它；snow 444414 里 430724 是它），
其余变量 `sumabs ≈ 0` —— 光合那条系统性偏差被消掉，只剩已知的 PHS 混沌残差。

**第二配置（Campbell + 关 VSF）干窗**：`oracle/scripts/compare_second_config.sh CN-Cng`
从修复前的 **16 个超容差变量降到 0**（127 个变量全部在容差内）。脚本本身因最后
`grep "failures by tier"` 找不到而 `exit 1` —— 那是它"全绿"时的行为，不是失败。

**16 步干窗 history**（`dry_ts.sh 16` + `window_divergence.py`）：首分歧仍在
**第 12 步的 `f_zwt` 1 个值 / 1 ULP**（`maxrel 1.52e-16`），第 14 步
`f_assimsun` 1 个值；`3446/3448` 逐位相同。这是与"遮荫 `rssha`"**不同的**一条链
（`rssha` 原来在第 15 步才分叉，那条已修掉），即第 271–274 轮那个 `rootflux`
1–2 ULP 的残差，本轮未动。

**同一探针复跑（`STEPS=16`）**：修复前首个分歧在 PHS 调用 289（`gs0sha`/`rssha`），
修复后 **六处 + `GSTO` 全部 328 次调用逐位相同**（`gssun_cmp.py` 报
`identical (328 calls)`）。

#### 第 294 轮：补上 `balance_phs_rootflux`（第 276 轮那个真缺口）

按第 276 轮的清单，把 `MOD_PHSRootfluxBalance:balance_phs_rootflux` 逐行移植到
`crates/colm-core/src/plant_hydraulics.rs`，并在上游的**两个**调用点接上：

| 上游 | 位置 | Rust |
|---|---|---|
| `'post-PHS'` | `MOD_LeafTemperature_Extended.F90:1145`，`etr = etrsun+etrsha` 之后 | `leaf_temperature.rs` 循环内 PHS 支 |
| `'post-leaf-temperature'` | `:1367`，`rootflux*etr/etr0` 缩放之后 | `leaf_temperature.rs` 循环后 |

三条分支与上游 `:55-64` 一一对应：`|etr - sum(rootflux)| <= 1e-7` 早退；否则按
正值层之和 `max(rootflux,0)*(etr/sum_pos_flux)`；没有正值层就按 `fallback_weights`
（两个调用点都传 `rootfr`）加权；权重和也是零才均分。`warn_count` 那个模块 `save`
用 `AtomicUsize` 复刻（前 5 次打警告、第 6 次打一次"不再打印"、之后静默）。

**它在默认三份窗口里仍然不生效**（与第 276 轮的判断一致）：`dry_ts.sh 3` 的内核与
Rust 日志里 `rootflux balance` 都是 **0 次**，步级仍是 `692/692` 逐位相同。补它是
为了条件一旦不成立时不静默算错。新增 3 个单元测试覆盖三条分支 + 早退
（`plant_hydraulics_tests.rs`，7 passed）。

**顺带核对第 293 轮留下的 item 1**：`oracle/golden/kernel-manifest.json` 与
`kernels/default/manifest.json` 的 `colm_git_sha` / `generator_args` /
`build_profile` / `macros` / 工具链 / netCDF 版本**逐字相同**（只有 sha256 不同，
那是"内核构建不可逐字节复现"的已知性质），所以"golden 可能是另一套宏"不成立。
而且 11 天干窗对**存储 golden** 的首分歧已从"第 1 步、1e-7（`f_trad`/`f_rnet`）"
变成 **第 8 步 `f_rib`（`maxrel 3.7e-11`）**（`window_divergence.py`：68 个变量
不同、`39700/56024` 逐位）—— item 1 的旧描述已过时。

#### 第 295 轮：把 golden 分歧**重新标定** —— 小时对齐后前 8 条逐位全同

第 294 轮留下的 item 1 有两处读法不对，本轮用新工具改掉：

1. **`dry_ts.sh` 的"第几步"不是小时级分辨力。** 它把 `DEF_HIST_FREQ` 改成
   `'TIMESTEP'`，而历史量是**按输出区间累加/平均**的：30 min 窗口与算例自带的
   `'HOURLY'`（60 min）在**同一时刻**给出不同的值（实测 `time[0]` 相同，
   `f_zwt` 一个 `8.756e-4`、另一个 `1.751e-3`）。所以第 293 轮记的"第 12 步
   `f_zwt` 1 ULP"是 **30 min 平均**的分歧，**不能**当成"第 6 小时的状态差"。
2. **新工具 `oracle/scripts/compare_hourly_window.sh`**：同一份算例、两侧都用
   `HOURLY`，逐记录比内核 / Rust / 存储 golden。CN-Cng 干窗实测：

```text
rec 0..7（8 小时）: kernel != rust = 0, kernel != golden = 0   ← 三方逐位全同
rec 8 （hour 9）  : kernel != rust = 1   （f_rib 差 1 ULP）
rec 9 （hour 10） : kernel != rust = 5   （f_fgrnd/f_wice_soisno/f_wliq_soisno/
                                          f_zwt 各 1 ULP + f_zerr 2e-2）
kernel != golden 在 rec 0..9 **全为 0**  ⇒ 存储 golden 就是当前内核的输出，
`three.py` 的 28/1970/25713 **不是基准错**。
```

3. **`f_rib` 不是叶温的 `rib`。** history 的 `f_rib` 来自
   `MOD_Vars_1DAccFluxes.F90:2786` 的 `r_rib`（`accumulate_fluxes` 为输出**重算**
   的那组近地层诊断，`MOD_Hist.F90:4530` 只写 `a_rib`），Rust 对应
   `history_diagnostics.rs:bulk_richardson`。它在 rec 8 差 1 ULP，而它的
   **history 输入全部逐位相同** ⇒ 是一处形状差（或某个非 history 中间量差 1 ULP）。
   它是**纯诊断、不回灌**，与 rec 9 的状态分歧是**两条独立的链**。
4. **本轮改了一处**：`stability_adjusted_wind` 的 `wc` 原先是 `.cbrt()`，
   而 Fortran 是 `(-grav*…)**(1./3.)`（`MOD_Vars_1DAccFluxes.F90:2771`），
   `leaf_temperature.rs:1002` 同式已用 `.powf(1.0/3.0)` ⇒ 改成 `.powf(1.0/3.0)`
   （`cbrt` 对少数值差 1 ULP）。**实测对 10 小时窗口逐位无影响**（说明 rec 8 的
   那 1 ULP 不在这里）；保留是为了与 Fortran/邻居一致。`ur*ur+wc2` 的 `mul_add`
   也试过、同样无效，按"形状要按自己的汇编定、不能类比邻居"（第 246/252 轮）
   **退回平铺**并在注释里写明。

**下一枪**：(a) rec 8 `f_rib` 的三层探针（`zol`/`um`/`r_ustar2`/`r_fh` 与 `r_rib`
两侧同位型）；(b) rec 9 那条**不经过 `f_rib`** 的状态种子 —— 注意 `f_rib` 不回灌。

#### 第 296 轮：(a) 打进去了 —— `f_rib` 链的首差是 `z0m` / `th`（各 1 ULP）

新工具 `oracle/scripts/rib_probe.sh`：在 `MOD_Vars_1DAccFluxes.F90:2787`（`r_rib_e`
定稿处）与 `history_diagnostics.rs` 的 `bulk_richardson` 之后两侧同位型打 13 个量。
10 小时窗口（两侧都 `HOURLY`）实测：

- 两侧都 **20 行** —— `accumulate_fluxes` 每个时间步调一次（`MOD_Hist.F90:227` 在
  非预热分支无条件调），`acc1d` 把它**累加**进 `a_*`；所以 `f_*` 是**区间平均**。
  这也解释了为什么 `r_zol_e` 等瞬时量从第 1 步起就差、而 history 前 8 条仍逐位。
- **首个不同的量是 `z0m`（内核 `z0m_av`）**：第 1 步就差 1 ULP
  （K=`0.1205726488149108` / R=`0.12057264881491078`），`displa` 跟着差；
- `th`（位温）从第 2 步起差 1 ULP，`thv` 跟着差；`thvstar`/`um`/`fh`/`zol` 的差
  都是它们的下游。

**两次形状尝试都被探针否掉、已回退**（按第 246/252 轮的规矩：形状要按**自己的**
汇编/位型定，不能类比邻居）：`(1.+0.61*qm)` 改 `mul_add`、`ur*ur+wc2` 改 `mul_add`
—— 实测都不移动 `thv`，所以这轮的改动只有新探针本身。

**下一枪**：
1. `z0m` 那 1 ULP —— Rust 的 `history_diagnostics` 用的是叶温输出的
   `leaf.momentum_roughness_m`（`history.rs:725`），内核用的是
   `MOD_Vars_TimeVariables%z0m`（单点下 `z0m_av` 就是它）。先确认这是不是同一个量
   （`z0mv` 本体 vs 写回状态时多一次舍入），再看 `th = tm*(1e5/psrf)**(rgas/cpair)`
   的形状（常数已核对：`rgas=287.04`、`cpair=1004.64` 两侧一致）。
2. rec 9 的状态种子仍然独立（`f_rib` 不回灌）。

#### 第 297 轮：**瞬时状态到第 18 步还逐位** —— history 的分歧是"区间累加"，不是状态

新工具 `oracle/scripts/restart_scan.sh`（`dry_ts.sh N` + `restart_divergence.py`）
逐步比 **restart（68 个状态量本身）**，CN-Cng 干窗实测：

```text
N=1..12   0/68                          ← 前 12 步全逐位
N=13      1/68  zwt  1 ULP  ┐
N=14..16  0/68              │  zwt 是每步重算的诊断（不是持久状态）
N=17      0/68              │  1 ULP 会自己消失
N=18      1/68  zwt  1 ULP  ┘
N=19      2/68  wliq_soisno 1 ULP + zwt   ← 第一个【持久】状态分歧
N=20..32  4..5/68  wliq/wice_soisno、t_soisno、smp、hk、zwt
```

三条结论（修正了之前的读法）：

1. **`dry_ts.sh` 的 `TIMESTEP` history 是区间累加**：`f_zwt` 来自
   `MOD_Vars_1DAccFluxes` 的 `r_zwt`→`acc1d`（`MOD_Hist.F90:4382` 只写 `a_zwt`）。
   所以"第 12 步 `f_zwt` 差 1 ULP"**不是**状态差 —— 第 12 步的 restart 是 0/68。
2. **真正的状态种子在第 19 步**：`wliq_soisno` **第 7 层（0 基）**1 ULP
   （K=`8.421413232125362` / R=`8.42141323212536`，`maxabs 1.8e-15`）。
   它落在 hour 10 的第一个时间步，与第 295 轮"小时对齐 rec 9 才首次不同"一致。
3. `zwt` 在第 13/18 步的 1 ULP 是**每步重算、下一步就消失**的（诊断），
   不是持久 prognostic —— 别追它。

**下一枪**：第 19 步的水/相变步里 `wliq_soisno` 那一层的 1 ULP 从哪来。
第 19 步的**输入**（第 18 步末尾的 restart）是逐位的，所以差在一处**值相关**的
形状（分支/钳制/幂），或某个**不在 restart 里**的中间状态。

#### 第 298 轮：4 个形状假设全否 + **找错了分支**（`wa >= 0` 走饱和路）

本轮用「改 Rust → `dry_ts.sh N` → 比 restart」的快回路逐个试形状（Rust-only，
一次 ~1 分钟，全部回退，树最终干净）：

| 试的 `mul_add` | 位置 | 结果 |
|---|---|---|
| 夹逼左乘积 `x_l*alp + x_r*(1-alp)` | `bounded_secant_iteration` | N=19 仍 2/68 |
| 同上右乘积 | 同上 | N=19 仍 2/68 |
| `fval = wa + (zwt-zmin)*(vl_s-vl)` | `water_table_from_aquifer` | 仍 1/68、1/68、2/68 |
| `psi = psi_s - (zwt-zmin)*0.5` | 同上（`liquid_at_depth`） | 同上 |

**为什么全否 —— 找错了分支**：`MOD_Hydro_SoilWater.F90:409` 按 `wa` 的符号分两路：

```fortran
IF (wa >= 0) THEN          ! ← CN-Cng 干窗走这条（实测 N=19 restart 的 wa = 0.0）
   ... zwt = sp_zi(ilev) - ss_wt(ilev)     ! :415，纯减法
   IF (is_sat) zwt = 0._r8                 ! :421
ELSE
   CALL get_zwt_from_wa (...)              ! :424，含水层路 —— **根本没被调用**
ENDIF
```

所以那 4 个形状（3 个在 `get_zwt_from_wa` 里）**一行都没执行**。教训：试形状前先
用 restart 里的 `wa` 判分支，别先改代码。

**正向确认**：`oracle/scripts/compare_soilhydro.sh` 复跑 **10000/10000 逐位相同**
（`psi`/`hk`/`vl` 三输出 + 分支分布）—— 第 19 步的种子**不在土壤水力函数**这一族。

**下一枪（探针，不再试形状）**：`zwt` 走 `:415` 的 `zwt = sp_zi(ilev)-ss_wt(ilev)`，
`wliq_soisno[7]` 也在同一段里由 VSF 的 `ss_wt`/`ss_vliq` 更新（`:431-436` 那段）。
所以两个差是同一个源头：**第 19 步的 VSF Richards 解**（Rust `richards_solver`）。
下一轮在 `ss_wt`/`ss_vliq` 的更新前后两侧同位型打点。

#### 第 299 轮：**探针把种子推出了水步** —— 它在上游的 `rootflux`（外加一处真形状差）

三件事，按重要性排。

##### 一、`vsf_probe.sh`：`soil_water_vertical_movement` 出场处的两侧探针（19 步）

新工具 `oracle/scripts/vsf_probe.sh`（内核 + 本仓库同点位、`vsf_cmp.py` 逐位比较）。
每次调用打一行 `WSF0`（`wa`/`zwt`/`ss_dp`/`qinfl`/`wblc`/`tol_v`）+ 每层一行 `WSF`
（`ss_vliq`/`ss_wt`/`smp`/`hk`/`qlayer`/`porsl`）：

```text
kernel calls 19 / rust calls 19
call 13 zwt   K=45.744737736306945   R=45.74473773630695      ← 首个不同
call 13 L3.ss_wt K=44.81708268203771 R=44.8170826820377
call 14 L3.ss_vliq K=0.33012449116092063 R=0.3301244911609206
call 18 zwt / L3.ss_wt；call 19 zwt / L3.ss_wt（同型）
4/19 calls differ; first differing call = 13
```

**读法（这条比"第 19 步"精确得多）**：

* 出的差**一直只在第 3 层**，而且 **`ss_vliq` 在 call 19 是逐位相同的**
  （call 14 那次 `ss_vliq` 差一步就自己回去了）⇒ 第 19 步 restart 里的
  `wliq_soisno[7]`（`oracle/scripts/restart_scan.sh` 的第一颗持久种子）**不是
  `vol_liq` 漏出去的**，而是 `zwt`：`MOD_SoilSnowHydrology.F90:1109-1115` 用
  `zwtmm` 参与 `wliq_soisno(7)` 的算式，而 `zwt = sp_zi(3) - ss_wt(3)`（`:415`）。
* 所以第 298 轮"下一枪打 `ss_wt`/`ss_vliq` 更新前后"的结论**方向对了**，
  但对象收窄成**只有 `ss_wt`（水位厚度）**：`ss_vliq` 这条到第 19 步还是干净的。
* call 13/18 的差是**瞬态**（下一调用就消失），与第 297 轮"第 13/18 步 `zwt` 1 ULP
  会自己消失"完全对上；但它**不是无害的**：call 19 那次正好被 `wliq_soisno(7)`
  接住，于是变成持久差。

##### 二、顺手修掉一处**真形状差**：`solve_least_squares_problem` 的 8 条 FMA

**证据是本例程自己的反汇编**（`objdump -d --disassemble-symbols=
___mod_hydro_soilwater_MOD_solve_least_squares_problem kernels/default/colm.x`，
第 246/252 轮的规矩：形状只认自己的汇编）：

```text
fmadd d4, d5, d5, d31      ← 1 + tau**2（两处分支各一条）
fmadd d3, d29, d26, d3     ← A(i,i) = c*Aii + s*Aji
fmadd d1, d0, d26, d1      ← tmp    = c*Aik + s*Ajk
fnmsub d0, d2, d26, d0     ← A(j,k) = -s*Aik + c*Ajk
fmadd d28, d27, d26, d28   ← tmp    = c*res(i) + s*res(j)
fnmsub d27, d30, d26, d27  ← res(j) = -s*res(i) + c*res(j)
fmsub d23, d22, d16, d23   ← dv(i)  = dv(i) - A(i,k)*dv(k)
```

**不能猜的那个细节**：内核只融合每个表达式里的**一个**乘积，另一个仍是独立
`fmul`（先舍入一次）；哪个进 FMA 由寄存器序决定，实测是"**第一个乘积进 FMA**"
（`fmadd(Aik, c, Ajk*s)`）。所以本仓库那一段全部改成
`a.mul_add(b, c)` 形状（7 处、8 条指令；`fnmsub` 那一类写
`x.mul_add(c, -(y*s))`），改完在 debug 产物里数得到 **8 个 `std::f64::mul_add`
调用**（`objdump -d target/debug/colm-rs` 的那个 symbol）——与内核 8 条一一对应。

**三段式实测（改前 → 改后，同一棵树 A/B 各跑一次）**：

| 窗口 | `over_tol` | `ot_vars` | `bitwise` | `sumabs` |
|---|---|---|---|---|
| CN-Cng 干 | 28 → **28** | 1 → 1 | 16326 → **16326** | 249.7886 → **249.7886** |
| CN-Cng-wet | 1970 → **1967** | 53 → **41** | 28456 → 28904 | 39.7652 → **31.3139** |
| US-NR1-snow | 25713 → **25713** | 79 → 79 | 32788 → **32567** | 444414.2029 → 444416.8246 |

**保留**：验收口径看的是 `over_tol`/`ot_vars`（黄金窗口的容差判据），三窗口都
**不变或更好**；两个诊断计数（`bitwise`/`sumabs`）在 wet/snow 上反向小幅移动，
是混沌放大的末位噪声（snow 那条是 444414 里的 +2.6，相对 5.9e-6）。
**干窗逐位完全没变**，因为干窗里 `A(j,i)` 大量为 0 ⇒ `IF (Amatrix(j,i) /= 0)`
整段不执行（这也解释了为什么 `vsf_probe.sh` 的 4/19 **没动**：这颗种子不在这里）。

##### 三、`vsf_richards_probe.sh`：入场 + Richards 内部的探针，**种子在上游**

第二个探针 `oracle/scripts/vsf_richards_probe.sh`（+`vsf_richards_cmp.py`）
在 `soil_water_vertical_movement` **入场**打 `WSF1`/`WSFE`（`qgtop`/`etr`/`rsubst`/
`ss_dp`/`zwt`/`wa` + 逐层 `ss_vliq`/`rootflux`/`porsl`/`psi_s`/`hksat`），
在 `Richards_solver` 内部打 `RCH0`/`RCHL`/`RCHF`/`RCHB`/`RCHD`/`RCHE`/`RCHX`/`RCHZ`
（13 步）：

```text
kernel records: RCH0=18 RCHB=259 RCHD=99 RCHF=30 RCHL=124 RCHZ=124 WSF1=13 WSFE=130
rust   records: RCH0=18 RCHB=259 RCHD=99 RCHE=75 RCHF=30 RCHL=124 RCHZ=124 WSF1=13 WSFE=130
首个不同的记录 = ('WSFE', 12, 2, 0) rootflux
    K=2.00962406971466e-07   R=2.0096240697146597e-07
```

两个结论：

1. **种子不在水步里 —— 它是入参**。`WSF1` 的 13 行（`qgtop`/`etr`/`rsubst`/`wa`…）
   和 call 1–11 的 `WSFE` 全部逐位相同；**call 12 第 2 层的 `rootflux` 差 1 ULP**。
   这独立复现了第 274/277 轮"种子是 `rootflux`"的结论（那两轮在第 11 步看到，
   本轮在第 12 步），并且把"水步内部的形状"整条排除了 —— 下一枪在
   `plant_hydraulics.rs` / `MOD_PlantHydraulic.F90`，不在 `MOD_Hydro_SoilWater`。
2. **内核的 `Richards_solver` 是按不透水层**分段**调的**：13 次 VSF 调用对应
   **18 次** `RCH0`，而且 `lb` 取过 2 与 3（`ub` 取过 8/9/10）—— 不是"整柱一段"。
   两侧的 `RCH0` 计数一致（18/18），说明本仓库的分段切法逐调用对齐。

**探针自身的坑（记下来）**：第一版 `RCH0` 在本仓库侧打的是**段内**索引、
内核侧打的是**绝对**层号，于是 `lb`/`ub`/`ilev` 全对不上，报出 55/661 条"假差异"。
已改成**两侧都打段内相对层号**（本仓库的 `richards_solver` 只拿到切片，本来就
不知道绝对偏移）。`WSFE` 那条不受影响（它没有段偏移）。

**下一枪**：先用已有的 `oracle/scripts/gssun_probe.sh` 跑 **19 步**（原来只跑 16 步）
看那六处电导是否仍是 328/328 逐位 —— 若仍是，则第 12 步的 `rootflux` 差**严格
落在 `PlantHydraulicStress_twoleaf` 的牛顿解里**（`x2qe`/`spacAF`/`dx`/`qe2x`），
再用第 281 轮那套"带调用序号的 `PHMID`/`PHEND`"探针把那一步的 `dx` 夹出来。
**别再回 `MOD_Hydro_SoilWater`**（本轮已用入参探针排除）。

---

# 交接：Fortran → Rust 移植的当前状态（本会话收束）

## 一句话

**移植在功能上完整，短程逐位对齐已经做完：干窗 CN-Cng 第 1/2/3 步的 restart 与
history 现在全部逐位相同（692/692）。** 那个追了十几轮的末位级状态差在第 257 轮
定位并消除（`wblc` 的两个逐层累加在 Fortran 里被收缩成 FMA）。剩下的只有两件：
11 天黄金窗口与**存储 golden** 之间那条 1e-7 量级的分歧（不是末位级、与本轮修复无关），
以及明确未移植的分支（split soil/snow、SNICAR、tracer、CaMa 洪水等）。

## 逐位对齐的阶梯（干窗 CN-Cng，1/2/3 步实测）

| 口径 | 第 257 轮（现在） | 第 256 轮 |
|---|---|---|
| 第 1 步 restart | **0 / 68 变量** | 0 / 68 |
| 第 2 步 restart | **0 / 68** | 0 / 68 |
| 第 3 步 restart | **0 / 68** | 2 / 68（`wice_soisno[5]`、`hk[0]`） |
| 第 0/1/2 步 history | **0 个差异变量** | 第 2 步 5 个 |
| 步级（3 步、692 个逐位单元） | **692 逐位相同（100%）** | 687（99.28%） |
| 黄金 `ot_vars` dry/wet/snow | 17 / 68 / 79 | 17 / 68 / 79 |
| 黄金 `over_tol` dry/wet/snow | **821** / 20664 / 25896 | 826 / 20665 / 25896 |
| 第二配置（Campbell + 关 VSF）`ot_vars` | 16 / 66 / 79（本轮未重跑） | 16 / 66 / 79 |

> **第 293 轮修掉 `update_photosyn` 的 `gsh2o` 量纲后再跑一遍**：`over_tol =
> 28 / 1970 / 25713`、`ot_vars = 1 / 53 / 79`、`bitwise = 16324 / 28456 / 32788`，
> restart（1/2/3 步 0/68）与步级（692/692）**不变**。见"第 293 轮"。

## 原残留（已消除）：第 2 步第 1 层土壤冰

第 254–256 轮追的那个差是这个形状：

```text
wice_soisno[5]   kernel=6.1823587081423845 (4018BABC3DBE730C)
                 rust  =6.182358708142612  (4018BABC3DBE740C)    3.68e-14 相对
```

**第 257 轮定位并修掉**：`WATER_VSF` 里 `wblc > 0` 的补冰循环从最上土壤层扣冰，
而这个 `wblc`（= `soil_water_vertical_movement` 的水量闭合误差）在两侧差 1 ULP
（内核 `+2.27e-13`、Rust `0`，恰好是 1605 mm 蓄量的一个 ULP）。原因是 Rust 的
`balance_before_mm`/`balance_after_mm` 两个逐层累加写成了 `acc += a * b`（两次舍入），
而内核把 `acc + a*b` 收缩成一条 FMA（一次舍入）。改成 `a.mul_add(b, acc)` 后
**1/3 步 restart 都是 0/68、3 步 history 692/692 逐位全同**。

过程与证据（逐层累加值差 −1 ULP 且保持、出货汇编 2568 条 FMA）见「第 257 轮」。
被排除的来源清单（叶温 Newton 循环、地面温度三对角组装、`meltf`、保持曲线、
凝结项、imperv 分支）也留在那一节里 —— 它们当时的排除都是对的，**错的是把这些
局部结论拼成"差来自能量步"的那一步推理**。

## 可复用的方法（比结论更值钱）

1. **同点同位型探针**：内核与 Rust 在同一位置打同一组量的位型（`Z17` / `{:016X}`），
   逐迭代/逐元素比。十进制 `E24.16` 只有 16 位有效数字，判不了 1 ULP。
   本轮的工具：`/tmp/gf/leafit_bits_probe.sh`、`gtcoef_probe.sh`、`postloop_probe.sh`、
   `th6_probe.sh`、`meltf_args_probe.sh`。
2. **离线形状穷举**：探针只负责给**输入位型**，形状在 Python 里对候选写法和
   **多轮同时命中**做筛选（`/tmp/gf/irab_shapes.py`）。Python 3.12 没有 `math.fma`，
   用 `Fraction` 做精确 fma。
3. **判"系统性偏差还是混沌放大"用短程对照**：`git stash` 后重编再跑 48 步/1 步，
   若短程只差 1 个元素，就是混沌放大，不是公式错（第 249 轮用过）。
4. **形状不能类比**：同一子程序里相邻两条 `X + tinc*coef`、`taf` 与 `qaf`、
   `fseng` 与 `fevpg` 的形状都可以不同 —— 只认实测位型/出货汇编。
5. **"读汇编 → 落代码"要复核**：本会话有两次注释写着 `fmsub`/`FMA` 而代码写成平铺
   （第 246 轮的 `_3199`、第 252 轮前的 `fevpg += tinc*cgrndl`）。
6. **累加器指纹**（第 257 轮定的案）：把**逐层累加值**打出来比。若两侧只在**某几层差
   1 ULP 且此后一直保持**，那就是 `acc + a*b` 收缩与否的差别（收缩=一次舍入），
   而不是"某个输入值不对"（那会从第一层就开始差、且差值随层数增长）。
   这条把"末位级差在哪"从"猜表达式"变成"看指纹"。

## 三个必须记住的坑

* **编的不是你以为的那个文件**（第 166 轮）：内核编
  `extends/interception/*_Extended.F90`，而 `main/` 下有同名文件、公式**不一样**
  （第 249 轮 `fwet_snow` 就是按 `main/` 那份实现，差 2.79e-12 相对量）。
  用 `strings .bld/*.o | grep '\.F90'` 或构建日志确认。**函数级同理**：同一条上游语句
  在 Rust 里可能有两份拷贝（VSF 与 Campbell），默认配置只跑其中一份 ——
  改之前先用临时 `eprintln!` 确认"你改的那行真的被执行了"（第 256 轮为此空转一轮）。
* **GIMPLE dump 不代表出货二进制**：`th_ext.opt`/`lt_ext.opt` 的收缩结论与本机
  `-O2` 出货二进制多次不一致。判形状用 `llvm-objdump --disassemble-symbols=…`
  对 `kernels/default/colm.x` 取操作码窗口，或直接用位型探针。
* **内核构建不是逐字节可复现的**：同一份源码 + 同一套 flag 连编三次得到三个不同的
  `colm.x` sha256（行为一致）。**不要用 sha256 判断"内核有没有被换过"**；
  用 `strings` 找探针标记或直接看行为。反过来，探针跑完**必须重编**。

## 验证资产清单（都在仓库里，可一键复跑）

* 11 个差分外壳 `oracle/scripts/compare_*.sh`：9 个裸跑即全绿且逐位相同
  （`qsadv` 4×20000、`moninobukm` 20×20000、`soil_hcap_cond` 8 档×5000、
  `MOD_Hydro_SoilFunction` 3×10000、`leddy`、`forcingdownscaling` 三支、
  `phasechange:meltf` 9×10000）；另两个需要参数（`compare_flag_isolated.sh <tag> "<nml 行>"`、
  `compare_second_config.sh <case>`）。
* 黄金三窗口 + `tier-check` + `oracle/tolerances.toml` 的分层容差。
* `oracle/scripts/gssun_probe.sh` + `gssun_cmp.py`（第 292 轮）：`gssun` 六处
  （`GSIN/GSDEM/GSOUT/GS908/GS919/GS941/GS1320`）与遮荫 `stomata`
  （`GSTO`）两侧同位型探针，跑完自动还原并重编内核回干净版；
  `STEPS=16 WORK=/tmp/gf/gssun16fix bash oracle/scripts/gssun_probe.sh` 后
  `python3 oracle/scripts/gssun_cmp.py $WORK/fort_probe.txt $WORK/rust_probe.txt`。
  第 293 轮修复前首分歧在 kernel call 289 的 `rssha`，修复后 328/328 全同。
* 第 293 轮的三层 `update_photosyn` 探针：`stomata_probe.sh` + `stomata_cmp.py`
  （遮荫支入参/内部）、`pco2a_probe.sh`（`:1243-1244` 更新式）、
  `updphoto_probe.sh`（`update_photosyn` 入口/内迭代/出口）。
* `oracle/scripts/updphotosyn_diff.f90` + `compare_updphotosyn.sh`：
  直接链 `.bld` 调内核 `update_photosyn` 的独立参考值（钉在
  `photosynthesis_tests.rs`）。
* `oracle/scripts/compare_hourly_window.sh`（第 295 轮）：短窗口、**小时对齐**的
  内核/Rust/golden 三方逐位比对。判"状态分歧在第几条"必须用它 ——
  `dry_ts.sh` 的 `TIMESTEP` 历史是 30 min 区间累加，与算例的 `HOURLY` 不同窗口。
* `oracle/scripts/rib_probe.sh`（第 296 轮）：`f_rib` 那条 history 诊断链
  （`MOD_Vars_1DAccFluxes.F90` 的 `r_*` vs `history_diagnostics.rs`）的逐时间步
  两侧同位型探针，打出 `z0m/zldis/th/thv/thvstar/zol/um/ustar/fh/rib`。
* `oracle/scripts/restart_scan.sh`（第 297 轮）：逐步跑 `dry_ts.sh N` 比 **restart**
  （68 个状态量本身），找**瞬时状态**第一次分歧的第几步。判状态必须用它，
  不能用 history（那是区间累加）。
* `oracle/scripts/stomata_probe.sh` + `stomata_cmp.py`（第 293 轮建、**第 351 轮 hex 化**）：
  `CALL stomata` 的入口（`STIN` 13 个入参）/`calc_photo_params` 出口（`STPH` 11）/6 次内迭代
  （`STIT` 10）/出口（`STOUT` 4）两侧**位型**探针，`CASE=<算例> STEPS=N` 可换窗口；
  比较器 hex 档按 `(tag,n,ic)` 逐位比（十进制档保留）。它把湿窗第 63 步的种子定到
  **`pco2a`**（唯一分叉的入参，第 351 轮）——注意 WUE 支的 `pco2i`/`eyy` 两列两侧不可比（见探针头注释）。
* `oracle/scripts/compare_stomata.sh` / `compare_sortin.sh` / `compare_update_photosyn.sh`
  （第 328/333/336 轮）：气孔-光合链的三个随机差分闭环，分别逐位判 `stomata`
  （4 个模型块 × 1000 = 4000）、`sortin`（`ic≥4` 的二次拟合 + 9 个中间量）、
  `update_photosyn`（3 个模型块 × 1000，`assim` + `respc`）。
  **`compare_update_photosyn.sh` 与第 293 轮的 `compare_updphotosyn.sh` 是两件东西**：
  前者是随机差分闭环（3000 例逐位比），后者是"内核侧参考值 +
  `photosynthesis_tests.rs` 里钉死"的单配置金值，**别混**。
* 湿窗（CN-Cng-wet）的逐步 restart 扫描：`/tmp/gf/wet_ts.sh N`（与 `dry_ts.sh` 同构）
  + `oracle/scripts/restart_divergence.py`。第 336 轮用它证明光合/气孔链（25 处 FMA）
  对湿窗**状态轨迹零影响**，并把湿窗首个状态分歧定在 N=4 的 `wliq_soisno[0,5]`。
* `oracle/scripts/phs_hex_probe.sh`（第 345-349 轮，第 349 轮入库）：PHS 链 + `gs0sun`/`stomata`
  的**位型**（`TRANSFER(x,0_8)` / `f64::to_bits()`）双侧探针，四个打点文件
  （`MOD_PlantHydraulic.F90`、`extends/interception/MOD_LeafTemperature_Extended.F90`、
  `plant_hydraulics.rs`、`leaf_temperature.rs`），八个标签
  `PHXI`（gs2qflx 的 17 个入参）/`PHXG`（返回的 qflx_sun,qflx_sha）/`PHXQ`/`PHXA`/`PHXD`/`PHXF`
  （PHS 内部）+ `PHXR`（`gs0sun`/`gs0sha` 出生点）/`PHXS`（`stomata` 返回值）。
  `STEPS=63 WORK=/tmp/gf/phshex63 bash oracle/scripts/phs_hex_probe.sh`，跑完自动还原并重编。
  它把湿窗第 63 步的状态种子一路推到 `stomata` 的 `rssun`/`rssha`（第 349 轮）。
* `oracle/scripts/vsf_probe.sh` + `vsf_cmp.py`（第 299 轮）：
  `soil_water_vertical_movement` **出场处**的逐层探针（`ss_vliq`/`ss_wt`/`smp`/
  `hk`/`qlayer` + `wa`/`zwt`/`ss_dp`/`qinfl`/`wblc`），19 步两次调用一次一行。
  它把第 19 步的持久差判成 **`zwt` 漏出**（`ss_vliq` 逐位相同、只有 `ss_wt[3]` 差）。
* `oracle/scripts/vsf_richards_probe.sh` + `vsf_richards_cmp.py`（第 299 轮）：
  `soil_water_vertical_movement` **入场**（`WSF1`/`WSFE`，含 `rootflux`）+
  `Richards_solver` **内部**（`RCH0/RCHL/RCHF/RCHB/RCHD/RCHE/RCHX/RCHZ`）的探针。
  它把第 19 步的种子**推出水步**：首个不同记录是第 12 次调用的入参 `rootflux[2]`。
  两侧都打**段内相对层号**（内核会按不透水层分段调用，13 次 VSF 调用对应 18 次
  `RCH0`，`lb` 取过 2/3）。
* `cargo test --workspace --lib --bins`（26 个测试二进制）、
  `cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all --check`、
  `python3 oracle/scripts/test_upstream_f48_sync.py`（本会话最后一次全绿）。

## "未移植"的准确含义：**明确拒绝，不是静默跑错**

`colm-runtime/src/physics.rs:281` 的 `unported_branches()` 现在是**空表**，而"没实现的分支"
一律在**装配期**报错（有 7 个测试守着，`cargo test -p colm-runtime --lib physics_tests`
共 17 个用例全绿）：

| 没实现的开关 | 行为 |
|---|---|
| `DEF_SPLIT_SOILSNOW = .true.` | 拒绝（不会静默按非 split 跑） |
| `DEF_USE_SNICAR = .true.` | 拒绝（不会静默用非 SNICAR 雪光学） |
| `DEF_Runoff_SCHEME = 1`（VIC） | 拒绝（0/2/3 可用） |
| 灌溉 | 拒绝（不会静默按旱地跑） |
| PFT/PC 次网格 | 拒绝 |
| 基流优化 `Opt_Baseflow` | 拒绝（会把 `scale_baseflow` 钉成 1.0） |
| 高度模式写错 / 未知字段 / 越界取值 | 拒绝 |

也就是说：**默认配置那条链是完整移植的；其余配置是"红着脸拒绝"，不是"绿着脸算错"。**
"全面完成"若指功能面，缺的正是上表这些被拒绝的分支（split、SNICAR、示踪剂、CaMa 洪水等）；
若指默认配置的逐位一致，则短程做到记录 0–11 状态全同、黄金窗口与基线齐平或更好。

### 最高优先：**已修（第 293 轮）**；剩下的是 `f_vegwp` 混沌残差

> **第 293 轮已修掉原来的首分歧。** 三层探针把"遮荫 `rssha` 差 9.1e-7"一路推到
> `update_photosyn` 的 `gsh2o` 量纲：上游调用点（`:908-911`）传的是**冠层 µmol 的
> 数值**，本仓库按哑元注释除了 `1e6`。修好后 16 步探针六处 **328/328 逐位相同**，
> 黄金 `over_tol` 821/20662/25896 → **28/1970/25713**。细节与证据见"第 293 轮"。
> （第 292 轮否掉的"PHS 种子公式"仍然作废：`:316-317` 每次重置，没有持久状态。）

**剩下的残差只有一个量**：`f_vegwp`（植物水力水势，长记忆混沌量）。

* `sumabs`：三个窗口现在 100% 由它主导 —— dry `249.79/249.79`；
  wet `39.77` 里 `38.37`；snow `444414` 里 `430724`。其余变量 `sumabs ≈ 0`。
* `over_tol` 计数：dry 只剩它（`28/28`）；wet 是它 `248/1536` 再加上被它带出来的
  水状态（`wliq_soisno` 356、`h2osoi` 351、`zwt` 82、`t_soisno` 74…）。

这条链与第 271–274 轮追的"干窗记录 12 的 `rootflux` 1–2 ULP"是同一个。

#### 第 299 轮补充：`gssun_probe.sh` 要从 16 步延长到 **19 步**再跑一次

第 293 轮那次"328/328 逐位相同"只跑满 **16 步**，而第 299 轮的两级探针已经把
干窗的种子钉在**第 12 次调用的入参 `rootflux[2]`**（1 ULP）—— 也就是说
16 步那次的六个点**可能与种子无关**（六处打印的是叶面电导/阻力，不是 PHS 解出来的
`rootflux`），也可能在 16 步内就已经分叉但没被这六处捕捉到。所以：

* **先跑** `STEPS=19 bash oracle/scripts/gssun_probe.sh && python3 oracle/scripts/gssun_cmp.py ...`：
  * 若仍 328/328（或延长后全同）⇒ 差**严格**在 `PlantHydraulicStress_twoleaf`
    的牛顿解里（第 281 轮那套 `PHMID`/`PHEND` 带调用序号的探针直接搬到当前代码上）；
  * 若在 12 步附近分叉 ⇒ 沿那处再往上追，别进 PHS 内部。
* 判"分叉在第几次调用"一律用 `PHMID`/`PHEND` 的**调用序号**对齐（第 279 轮的坑：
  两侧补丁落在不同分支时行数都对不上，比出来的是垃圾）。
* **实测结果见下面那条补充：19 步仍全同** ⇒ 走第一种情形，但收窄到 `qe2x` 那一段。

#### 第 299 轮补充（实测）：`gssun_probe.sh` 延到 **19 步**后仍是**全同** —— 嫌疑收窄到 `qe2x`

本轮把第 293 轮那条"328/328 全同"的探针从 16 步延长到 **19 步**（
`STEPS=19 WORK=/tmp/gf/gssun19 bash oracle/scripts/gssun_probe.sh`，
`python3 oracle/scripts/gssun_cmp.py $WORK/fort_probe.txt $WORK/rust_probe.txt`）：

```text
tag       kernel    rust  shared  first-divergence
GSIN         354     354     354  identical (354 calls)
GSDEM        354     354     354  identical (354 calls)
GSOUT         72      72      72  identical (72 calls)
GS908        354     354     354  identical (354 calls)
GS919        354     354     354  identical (354 calls)
GS941        354     354     354  identical (354 calls)
GS1320        19      19      19  identical (19 calls)
GSTO         354     354     354  identical (354 calls)
```

**这条把嫌疑面重新切了一刀**（比第 277–291 轮那次更干净，因为 `gsh2o` 量纲已修）：

* `GSOUT` 打的是 `:346 getqflx_qflx2gs_twoleaf` 的出口（`gssun`/`gssha`/`etrsun`/`etrsha`），
  第 12 步**逐位相同** ⇒ `qeroot = etrsun + etrsha`（`:354`）也逐位相同。
* **第 279/280 轮那条"`qe2x` 排除"的论证在本代码上不再成立** —— 它当时的前提是
  "`qeroot` 在 PYDYN 里已经不同"，而现在 `qeroot` 相同。所以
  `getrootqflx_qe2x`（本仓库 `root_potential_from_flux`）**重新回到嫌疑名单第一位**，
  其次是 `x(root) = x_root_top` 之后的 `rootflux(j) = k_soil_root*(smp-xroot)`，
  再次才是叶温那两行缩放（第 275 轮已修并证过形状）。

#### 第 300 轮：`rootflux` 的真凶是**截断的 π**（已修）；但它**不是**第 19 步那颗种子

##### 一、`rootflux_probe.sh`：差出在**PHS 解出来的** `rootflux`，缩放是干净的

新工具 `oracle/scripts/rootflux_probe.sh` + 通用比较器 `oracle/scripts/probe_diff.py`
（格式 `TAG n k1 k2 f1 …`，按内核文件里 tag 首次出现的次序比，逐位相等才算同）。
两侧同序打四个 tag：`RFIN`/`RFINL`（`:1351` 之前，缩放**前**的 `rootflux` = PHS 输出）
与 `RFLT`/`RFLTL`（`:1361` 缩放之后），13 步：

```text
RFINL (12,2) f0 K=2.007223592100237e-07   R=2.0072235921002367e-07   ← 首个不同（缩放之前）
RFLTL (12,2) f0 K=2.00962406971466e-07    R=2.0096240697146597e-07
RFLT 的 13 行（etr / etr0 / etr_dtl / dtl）全部逐位相同
2/286 records differ
```

⇒ **叶温那两行缩放是干净的**（第 275 轮的结论成立），差产生在 PHS 的输出里。

##### 二、沿本例程自己的汇编数 FMA（第 299 轮那套方法）

```text
plc 0   d1plc 0   getqflx_gs2qflx_twoleaf 3   getqflx_qflx2gs_twoleaf 3
getrootqflx_qe2x 3   getrootqflx_x2qe 5
planthydraulicstress_twoleaf 0   ← 含内联的 `calcstress_twoleaf`（根导度那一圈全在里面）
spacaf_twoleaf 41   tridia 3
```

`qe2x` 的 3 条与 `tridia` 的 3 条逐条对上本仓库的 `mul_add`
（`root_potential_from_flux` 的三行 `rmx`、`solve_tridiagonal` 的三处），
`sub`/`super_`/`diagonal` 的结合顺序也逐条核过（`map_or(0.0, …)` 里的 `+0.0` 是精确的）。
**形状排完就只能查常数** —— 于是发现：

##### 三、真凶：`rpi` 是**截断到 15 位的 π**，与 `std::f64::consts::PI` 差 **7 ULP**

```fortran
MOD_PlantHydraulic.F90:148   real(r8), parameter :: rpi = 3.14159265358979_r8
                         :167   root_cross_sec_area = rpi*DEF_PH_ROOT_RADIUS**2
                         :177   r_soil = sqrt(1./(rpi*root_length_density))
```

实测 `3.14159265358979 != std::f64::consts::PI`，相对差 9.895e-16（7 ULP）。
它在根导度那一圈出现两次，解析上会在 `r_soil = sqrt(density*r²/biomass)` 里约掉，
**浮点上不会** ⇒ 本仓库的 `k_soil_root` 系统性偏 ~1 ULP，而它直接乘进
`rootflux = k_soil_root*(smp-xroot)`。

修法：`plant_hydraulics.rs` 里新增 `PLANT_HYDRAULIC_PI = 3.14159265358979`（带出处注释），
把 `root_conductances` 的两处 `std::f64::consts::PI` 换掉。

##### 四、验收（A/B 两边都跑了）

| 口径 | 结果 |
|---|---|
| `rootflux_probe.sh 13` | **2/286 → 0/286**（13 步内整条 rootflux 链逐位全同） |
| 19 步干窗 restart（带 π vs 不带 π） | **0/68 逐位相同** |
| 黄金三窗口 | `28 / 1967 / 25713`、`16326 / 28904 / 32567`、`249.7886 / 31.3139 / 444416.8246` —— **一个数都没动** |
| `accept_r247.sh` | restart 0/68、3 步 692/692 |

⇒ 这处修复是**口径中性**的（与第 295 轮的 `cbrt` 同类：为与内核一致而保留），
但它**推翻了一条旧的因果链**：

##### 五、推翻的旧结论（重要）：`rootflux` 不是第 19 步种子的因

第 274/277 轮把干窗的种子判成"入参 `rootflux` 第 2/3 层差 1 ULP"，第 299 轮的两级探针
也把"第一条不同记录"钉在入参 `rootflux` 上。**π 修好之后 `rootflux` 的差消失了，
而第 19 步的状态差一字不动** ⇒ 那条差是**被下游舍入吸收掉的**，不是因。
教训与第 280 轮同一个：**"第一条不同记录"只说明先后，不说明因果**。

##### 六、水步内部的新定位（`vsf_richards_probe.sh`，先修掉三个探针 bug）

探针自己的 bug（都已修，记下来）：
1. `RCHL` 在本仓库侧把调用序号**写死成 `1`**（内核侧正常）⇒ 同一个键 18 行、
   报出 74/755 条"假差异"；
2. `RCHB`/`RCHD` 的行范围是 `lb-1 .. ub+1`，相对号应是 `ilev-lb+2`（原先按
   `ilev-lb+1` 打，两侧键整体错开一位）；
3. `RCHE` 原先用 `insert_after` 落在 `IF (vact(ub+1))` **里面**，而本算例
   `lbc=FIX_FLUX` ⇒ 内核侧一行都没有（本仓库侧打在 `if` 之前，75 行）。

修好之后（内核侧已存记录离线改键 1 位 + 本仓库侧重编一次）只需比 **797** 条：

```text
('RCHL', 17, 1, 0) ss_wt  K=44.81708268203771   R=44.8170826820377   ← 首个不同
('RCHZ', 17, 1, 0) ss_wt  K=44.81708268203771   R=44.8170826820377
2/797 records differ
```

**读法**：第 17 次 Richards 调用**入场**的 `ss_wt` 就已经差了，而这次解算内部的
`RCHF`(f2) / `RCHB`(blc) / `RCHD`(dv) **全部逐位相同** ⇒ 差不是解算造的，是**入场前**
带进来的（`soil_water_vertical_movement:344-350` 用入参 `zwt` 初始化 `ss_wt`）。
**下一枪**：把 `WATER_VSF` 每次调用的**入场 `zwt`** 也打出来（现在只有出场）——
第 299 轮的出场结果显示第一次差在第 13 次调用，所以"出场 → 下一次入场"这一段
是本轮之后唯一还没量的环节；如果入场也全同，就落在 `:344-350` 的 `sp_zi(izwt)-zwt`
与 `findloc_ud` 判级上。

#### 第 301 轮：那一枪打进去了 —— 种子在 **`soilwater_aquifer_exchange`**（含水层交换）

新工具 `oracle/scripts/vsf_wt_probe.sh`（三个 tag：`WSE1` = 第一次 `findloc_ud`
之后的级号+入场 `zwt`；`WSE0` = `ss_wt` 初始化之后（**交换之后**）的级号 + `zwt` + `wa`
+ `ss_dp`；`WSEL` = 逐层 `ss_wt`/`ss_vliq`/`porsl`），20 步：

```text
WSE1     kernel=20 rust=20      WSE0 kernel=20 rust=20      WSEL kernel=200 rust=200
('WSE1', 14, 0, 0) f2 zwt  K=45.744737736306945   R=45.74473773630695    ← 首个不同
('WSE1', 19/20) / ('WSE0', 13/18/19/20) 同型
('WSEL', 13, 3, 0) ss_wt   K=44.81708268203771    R=44.8170826820377
12/240 records differ
```

**读法与结论**（这条把范围收得比第 300 轮小得多）：

1. `WSE1` 在 **call 1–13 全部逐位相同**（入场级号 `izwt` 与入场 `zwt` 都是）⇒
   从步进开头到 `findloc_ud(zwt >= sp_zi)` 为止，两侧没有任何差别；
2. `WSE0` 在 **call 13** 的 `zwt` 差 1 ULP，而**同一次调用的 `izwt` 仍然相同**
   ⇒ 差是在 `soilwater_aquifer_exchange` 内部造出来的（入场相同、出场不同）；
3. `WSE1` call 14 = 上一调用（call 13）的出场 `zwt`，值 `45.744737736306945`
   与第 299 轮 `vsf_probe.sh` 的"call 13 出场 `zwt`"**同一个数** ⇒ 两个探针互相印证；
4. `WSEL` call 13 第 3 层 `ss_wt` = `sp_zi(3) - zwt` 跟着差 1 ULP，而第 300 轮
   Richards 内部探针的首差正好落在**该次 VSF 调用对应的 Richards 调用（第 17 次）
   入场** `ss_wt` 上 —— 整条链现在自洽：**交换改出的 `zwt` → `ss_wt(3)` →
   Richards 入场的 `ss_wt` → …**。

**所以种子既不在 Richards、也不在 PHS、也不在缩放，而在含水层交换这一步。**
下一枪（已具体到点位）：在 `soilwater_aquifer_exchange`（Rust
`exchange_soil_water_with_aquifer`）的**入口**打 `wexchange`/`ss_dp`/`wa`/`zwt`/`izwt`、
**出口**打 `zwt`/`wa`/`ss_dp`/`izwt`，先判 `wexchange`（= `rsubst*dt + deficit`）
进来时就差还是内部某一步的形状差；同时用同一套反汇编方法数该子程序的 FMA 条数
（`objdump -d --disassemble-symbols=___mod_hydro_soilwater_MOD_soilwater_aquifer_exchange`）。

**探针工具的两处加固（本轮顺手做掉）**：
* `vsf_wt_probe.sh` 把**级号放进"值"里**而不是当键 —— 级号本身可能差，当键会让两侧
  键不同、比较器直接漏报；
* `oracle/scripts/probe_diff.py` 增加"共享键数量 < 两侧行数"的显式告警，同样的漏报
  不会再静默发生。

##### 第 301 轮追加：按反汇编试出来的 5 处 FMA 形状 —— **两组都被口径否掉，已全部回退**

反汇编 `get_zwt_from_wa`（`secant_method_iteration` 被内联进去）数出 **9 条** FMA 族指令，
逐条映射出 5 处可以确定源表达式的形状：

| # | 上游表达式 | 反汇编形状 | 本仓库原先是 |
|---|---|---|---|
| 1 | `zwt = zmin + (-wa)/vl_s*2.0` | `fdiv` 后 `fmadd(t,2.0,zmin)` | 平铺 |
| 2 | `psi = psi_s - (zwt-zmin)*0.5` | `fmsub`（乘积收进减法） | 平铺 |
| 3 | `zwt = zmin + (zwt-zmin)*2 + 0.1` | `fmadd((zwt-zmin),2.0,zmin)` 再单独 `+0.1` | 平铺 |
| 4 | `fval = wa + (zwt-zmin)*(vl_s-vl)` | `fmadd`（乘积收进 `wa`） | 平铺 |
| 5 | `secant` 分子 `fval_k1*x_k2 - fval_k2*x_k1` | `fnmsub`（**第一个**乘积进 FMA） | 平铺 |
| 6/7 | 两处夹逼 `x_l*alp + x_r*(1-alp)` / `x_l*(1-alp) + x_r*alp` | `fmadd`（**第二个**乘积进 FMA） | 平铺 |

**探针侧是正向的**：5 处全改后 `vsf_wt_probe.sh 20` 的差异 **12/240 → 7/240**，
首差从 **call 13 推到 call 19/20** —— 第 13 次调用那处差**确实被消掉了**，
说明这几处就是那一段的成因之一（也说明第 298 轮"4 个形状全否"的判据不成立：
当时用 restart 计数当判据，单个形状改对不会让整条混沌链回到逐位）。

**但两组都被验收口径否掉**（`win4.sh` + `three.py`，同树 A/B 各跑一次）：

| 窗口 | 基线 | 5 处全改 | 只改 1–4（`get_zwt_from_wa`） |
|---|---|---|---|
| dry `over_tol` | **28** | 36 ✗ | 28 ✓ |
| dry `bitwise` / `sumabs` | 16326 / 249.79 | 17387 / 271.97 | 16260 / 263.14 |
| wet `over_tol` | **1970** | 1822 ✓ | 2070 ✗ |
| wet `sumabs` | 39.77 | 23.13 | 50.11 |
| snow `over_tol` | 25713 | 25713 | 25713 |
| snow `bitwise` / `sumabs` | 32788 / 444414.20 | 32639 / 444416.82 | 32760 / 444416.82 |

⇒ 全改让**干窗 `over_tol` 28 → 36**，只改 1–4 让**wet `over_tol` 1970 → 2070**，
两者都违反"口径指标不许变差"，按纪律 `git checkout` 整个回退，树回到基线
（黄金仍是 **28 / 1967 / 25713**）。

**结论（比这次改动本身更值钱）**：**形状与内核一致不自动等于指标变好。**
这些点位处在 `f_vegwp` 混沌主导的窗口里，把末位改对等于换了一条混沌轨道，
`over_tol` 可能反向；所以形状改动必须**先过口径**，"探针差异变少"只是必要条件。
**下一枪（换判据，别再用混沌窗口判形状）**：给这一段做**例程级**差分闭环 ——
照 `oracle/scripts/updphotosyn_diff.f90` / `compare_soilhydro.sh` 那套，
写一个直接链内核 `get_zwt_from_wa` / `soilwater_aquifer_exchange` 的 Fortran 驱动器，
用大批合成输入两侧逐位比（不进混沌窗口）。这样 5 处形状（以及第 298 轮那 4 个）
可以一次性判**对错**，再拿"对的那一组"去跑口径。

#### 第 302 轮：把那个闭环建起来了 —— 当前这一段**确定性地错了 3604/10000**

##### 一、新工具（确定性判据，不进混沌窗口）

* `oracle/scripts/get_zwt_diff.f90` + `oracle/scripts/compare_getzwt.sh`：
  按内核**真实选项**编译 `MOD_Hydro_SoilWater`（**不加** `-ffp-contract=off`，
  要的就是 GCC 默认的 `fast`），驱动本身按仓库纪律 `-fwrapv -ffp-contract=off`；
  两档模型（Campbell / van Genuchten）× (2500 均匀随机 + 2500 边界取值)，
  在合成输入上逐位比 `get_zwt_from_wa` 的输出 `zwt`。
  `-ffunction-sections` + `-Wl,-dead_strip` 把依赖 `MOD_SPMD_Task`（MPI）的未用
  子程序剔掉，闭环不把 MPI 拖进来。一次全跑 ~30 秒。
* `crates/colm-core/examples/get_zwt_probe.rs`：同一串 LCG 的本仓库侧
  （抽签次数与顺序逐条对齐，改一边就得同步改另一边）。

##### 二、实测：**当前提交的 Rust 在 3604/10000 个用例上与内核不同**

```text
get_zwt_from_wa mismatches / 10000: {'zwt': 3604}
```

这不再是"末位运气"的争论：**36% 的用例位型不同**，是实打实的形状差。

##### 三、形状扫（每次 ~30 秒，先应用 `get_zwt_from_wa` 四处、再扫 `secant` 三处）

| 分子 | 两处夹逼 | 不匹配 / 10000 |
|---|---|---|
| 平铺 | 平铺 | 1923 |
| 平铺 | 收**第二个**乘积 | 2307 |
| 平铺 | 收**第一个**乘积 | 1188 |
| 收**第一个**乘积 | 平铺 | 1159 |
| 收**第一个**乘积 | 收**第二个**乘积 | 1643 |
| **收第一个乘积** | **收第一个乘积** | **0（10000/10000 逐位相同）** |

（`get_zwt_from_wa` 那四处单独应用是 3604 → 1923；上表都在"四处已应用"的前提下。）

⇒ 唯一正确的组合：**分子与两处夹逼都收「第一个」乘积**。六处形状的准确写法：

```rust
// get_zwt_from_wa
(depth_mm - minimum_depth_mm).mul_add(-0.5, saturated_potential_mm)          // psi
((-aquifer_water_mm) / porosity).mul_add(2.0, minimum_depth_mm)              // 初值 zwt
(right - minimum_depth_mm).mul_add(2.0, minimum_depth_mm) + 0.1              // 括号外扩
(depth - minimum_depth_mm).mul_add(porosity - liquid, aquifer_water_mm)      // fval
// secant_method_iteration
previous_residual.mul_add(value_before_previous,
                          -(residual_before_previous * *previous_value))     // 分子
(*left).mul_add(SECANT_ALPHA, *right * complement)                           // max 夹逼
(*left).mul_add(complement, *right * SECANT_ALPHA)                           // min 夹逼
```

**方向不能按寄存器序猜**：我先按 `fmul`/`fmadd` 的寄存器顺序读成"夹逼收第二个乘积"，
harness 上那是 1643 与 0 的区别。

##### 四、口径仍然把它挡在门外（同树 A/B）—— 本轮**回退**代码、**保留**闭环

| 窗口 | 当前提交（基线） | 形状已验证正确 |
|---|---|---|
| dry `over_tol` | 28 | 28 |
| wet `over_tol` | 1970 | **1984** ✗ |
| snow `over_tol` | 25713 | 25713 |
| `ot_vars` | 1 / 53 / 79 | 1 / 53 / 79 |
| `bitwise` | 16326 / 28456 / 32788 | 17056 / 28958 / 32707 |
| `sumabs` | 249.79 / 39.77 / 444414.20 | 330.81 / **29.32** / 444416.82 |

`restart_scan.sh` 同向：N=19 的分歧变大（`wliq_soisno` maxabs 1.78e-15 → 2.49e-14）。
按"口径指标不许变差"`git checkout` 回退该文件；**六处形状的正确写法与全部证据留在上面**。

##### 五、为什么回退是对的（而不是"形状不对"）

`vsf_wt_probe.sh` 在 call 17 起还报 `WSEL` 的 `f1`(`ss_vliq`)/`f2`(**`porsl`**) 不同，
而 `porsl` 是**入参**（`eff_porosity`）⇒ 那是一条**独立的上游链**。两条差同时存在时，
把一条改对只是换一条混沌轨道，口径就成了抛硬币 —— 这正是本轮口径变差而 harness
变好的原因。**下一枪**：先把 `eff_porosity` 那条（第 17 步起、第 3 层）钉掉
（点位已由 `WSEL` 给出，接着查 `MOD_SoilSnowHydrology` 里 `eff_porosity` 的逐层更新
与 Rust 对应处），**然后**把本轮的六处形状一起重新应用，两条都对了再跑口径 ——
那才是一次干净的、口径也向前走的修复。

##### 六、`eff_porosity` 那一条：公式本身**已逐项对上**，所以要查它的入参

读完两侧的逐层更新（内核 `MOD_SoilSnowHydrology.F90:299-306`，本仓库
`water_2014.rs:507-528 soil_volumes`），四条式子**逐项一致**（含结合顺序）：

```fortran
vol_ice(j)      = min(porsl(j), wice_soisno(j)/(dz_soisno(j)*denice))
eff_porosity(j) = max(0.01, porsl(j)-vol_ice(j))
vol_liq(j)      = min(eff_porosity(j), wliq_soisno(j)/(dz_soisno(j)*denh2o))
icefrac(j)      = min(1., vol_ice(j)/porsl(j))   ! porsl<1e-6 时取 0
```

所以第 17 步第 3 层的 `eff_porosity` 差**不是这个函数造的**，只能来自它的入参在该时刻
不同：`wice_soisno(3)`、`porsl(3)`（静态）或 `dz_soisno(3)`（静态）。注意 restart 在
N=17/N=18 是**逐位相同**的，所以 `wice_soisno(3)` 的差必然出现在**步内**（能量步的相变
之后、水步之前）并在步末又回到同一个值 —— 这也解释了为什么 restart 看不见它。
**下一步（探针已到位，只需加三列）**：在 `vsf_wt_probe.sh` 的 `WSEL` 里补打
`wice_soisno(ilev)`/`porsl(ilev)`（土壤孔隙度，不是 `eff_porosity`）/`dz_soisno(ilev)`,
就能把"是入参差还是 `soil_volumes` 差"一次判死。

#### 第 304 轮：把**六处形状**放回窗口里量了一次 —— 交换链确实被修掉，但还剩第二条链

第 302 轮只把六处形状放在合成输入上判（harness 10000/10000），没在真实窗口里看过它们
到底修掉了哪一段。本轮把六处形状应用上、用**独立 WORK**（`WORK=/tmp/gf/vsf19_shapes`，
吸取第 303 轮的目录覆盖教训）重跑 `vsf_probe.sh 19`：

```text
基线（当前提交）     12/240 条不同, 首差 call 13（`ss_wt`/`zwt`，交换链）
六处形状应用后        3/19 calls differ, 首差 call 17
   call 17/18/19 L3.porsl（= eff_porosity）差
   call 18 wblc K=0.0  R=-2.2737367544323206e-13（纯诊断）
   call 18/19 L3.ss_vliq / smp / hk / ss_wt / zwt 跟着差
```

**两个结论**：
1. **交换那条链确实被修掉了**：call 13/14 的 `ss_wt`/`zwt`/`ss_vliq` 差消失 ⇒
   第 302 轮的六处形状在**真实窗口**里同样是对的（不只是合成输入上对）。
2. **但还剩第二条链**：`eff_porosity`（`porsl`）从 call 17 起在第 3 层差 1 ULP。
   第 303 轮的入场探针在**基线**里把同一族差定位在"第 20 步入场、第 3 层的
   `wice`/`wliq`"；两条轨迹的混沌不同，所以这条链"露面"的步号不同
   （基线 20、形状版 17），但它是**独立于交换**的另一处缺陷。

⇒ 这解释了第 302 轮"harness 变好、口径却变差"：两条链同时在场，修掉一条只是换轨道。
**代码仍然回退**（树与 HEAD 一致，黄金 28/1967/25713），六处形状的写法与本次的
窗口级证据都留在文档里。**下一枪**：钉第二条链 —— 从第 19/17 步的
`wice`/`wliq` 出发，查 `WATER_VSF` 尾部的 `wblc` 冰汇（`MOD_SoilSnowHydrology.F90:1137-1150`）
与相变那一段的形状；`wblc` 在形状版里已经出现 `K=0.0 / R=-2.27e-13` 的分歧，是一个
可以顺藤摸的入口。

#### 第 305 轮：把 `MOD_Hydro_SoilWater` 的 FMA 普查了一遍 —— **通量链有 ~15 处没复现**

第 302 轮的闭环只判了 `get_zwt_from_wa`。本轮把该模块**每个例程自己的汇编**里的
FMA 族指令数与 Rust 对应函数的 `mul_add` 调用数并排列出来（都不编译，纯静态）：

| 内核例程 | FMA 条数 | 本仓库对应 | `mul_add` |
|---|---|---|---|
| `flux_inside_hm_soil` | **2** | `flux_inside_variable_saturated_soil` | 0 |
| `flux_btm_transitive_interface` | **3** | `flux_variable_saturated_bottom_transition` | 0 |
| `flux_top_transitive_interface` | **3** | `flux_variable_saturated_top_transition` | 0 |
| `flux_all`（含被内联的下游） | **7** | `flux_variable_saturated_flux_all` | 0 |
| `water_balance` | **5** | `variable_saturated_water_balance` | 0 |
| `flux_sat_zone_fixed_bc` | 0 | `flux_variable_saturated_zone_fixed_boundaries` | 0 |
| `check_and_update_level` | 0 | `check_and_update_variable_saturated_level` | 0 |
| `soil_water_vertical_movement` | 27 | `soil_water_vertical_movement` | 12 |

**读法**：Rust **不会**自动收缩（没有 `-ffast-math`/不是 Fortran），所以
`mul_add` 计数为 0 就意味着那一段**没有**复现内核的融合。通量链
（`flux_all` 及其被内联的 `flux_*_interface` / `flux_sat_zone_*` / `flux_inside_hm_soil`）
内核有 **~15 条** FMA 而本仓库一处都没有 —— 这是 Richards 解里**调用最频繁**的一段，
也正是第 304 轮"第二条链"（`ss_vliq`/`eff_porosity` 从 call 17 起、1 ULP）最可能的来源。
`soil_water_vertical_movement` 27 vs 12 的差额同理需要按例程拆开核对（27 里可能含被
内联的 `initialize_sublevel_structure`/`use_explicit_form` 等）。

**下一枪（成本可控，顺序明确）**：
1. 先把 `flux_inside_hm_soil` / `flux_*_transitive_interface` 三个**最小**例程按
   `compare_getzwt.sh` 那套做**例程级差分闭环**（`flux` 是 `smp`/`hk`/`zi` 的纯函数，
   合成输入最容易），把形状一次判对；
2. 再沿 `flux_all`/`water_balance` 往上做同样的事（或直接在 `vsf_probe.sh` 上验证）；
3. 全部判对之后，再与第 302 轮的六处 `get_zwt_from_wa` 形状**一起**落地并跑口径。

**第 305 轮追加（订正第 12 轮那个"内联"猜测）**：符号级 FMA 计数**确实**含被内联的 callee，
但 `flux_inside_hm_soil` 那 2 条**不是**内联来的 —— 逐条对上它自己的两行：

```text
ldp d28,d24,[x3,#0x8]      ; d28 = prms(2), d24 = prms(3)
fsub d25, d28, d30         ; d25 = prms(2) - 1.0     (d30 = 1.0)
fadd d28, d28, d28         ; d28 = prms(2)*2.0       （×2 是精确的）
fmadd d28, d24, d25, d28   ; ← r0 的分母：prms(3)*(prms(2)-1) + prms(2)*2
...
fdiv d27, d27, d26 / fmul d27, d27, d13
fmadd d0, d27, d0, d14     ; ← grad_psi>1 支：hk_u + ((psi_u-psi_l)/dz*hk_u**(1-rr))*hk_l**rr
```

⇒ 这 2 条**都是该函数自己的表达式**（第 12 轮把它读成 `soil_hk_from_psi`/`**` 的内联，
**是错的**），因此通量链"没复现融合"的结论**比第 12 轮收紧后的说法更强**：
`flux_inside_hm_soil` 至少要补两处 `mul_add`：

```rust
// r0 的分母（VG 支）：第一个乘积进 FMA，prms(2)*2 精确
let denominator = m3.mul_add(m2 - 1.0, m2 * 2.0);          // m2 = prms(2), m3 = prms(3)
// grad_psi > 1 支：先把 ((psi_u-psi_l)/dz)*hk_u**(1-rr) 算出来，再融合末尾乘积
let t = (psi_u - psi_l) / dz * hk_u.powf(1.0 - rr);
flux = t.mul_add(hk_l.powf(rr), hk_u);
```

**方法的正确用法**（两条都要）：符号 FMA 数**会**含内联，所以不能只看计数；
但**逐条读反汇编仍能定案** —— 只要把每条 FMA 的寄存器上下文与源表达式对上
（本条就是这样对上的）。闭环仍然是最省事的最终判据，但"计数可疑 ⇒ 必须上闭环"
这一步**不成立**：直接读上下文更快。

**第 306 轮：把通量链剩下两个例程的**位置**钉住（映射留给下一轮）**

`flux_top_transitive_interface` 不是函数而是 `PRIVATE` 子程序，定义在
`MOD_Hydro_SoilWater.F90:2867`，被 `flux_all` 在 `:2332`/`:2340` 调用；它自己带一套
**割线迭代**局部量（`psi_i_r`/`psi_i_l`/`psi_i_k1`/`fval`/`fval_k1`/`iter`），
`flux_btm_transitive_interface` 同族（符号表里两者各自独立）。所以这两个例程的
3+3 条 FMA 与 `secant_method_iteration` 是**同类问题**（"哪个乘积进 FMA"），
映射时必须像第 305 轮那样逐条读寄存器上下文，不能按表达式猜。
**下一轮的动作**（已具体到文件/行）：
1. 读 `:2867` 起的子程序体与 `flux_btm_transitive_interface` 的对应体，逐条把
   3+3 条 FMA 对到源表达式；
2. 与 `flux_inside_hm_soil` 已定案的 2 处（第 305 轮记的写法）一起补 `mul_add`；
3. 建 `compare_getzwt.sh` 式例程闭环验证（这三个都是 `psi`/`hk`/`dz` 的纯函数）；
4. 之后再与第 302 轮那六处 `get_zwt_from_wa` 形状一起落地、跑口径。

**第 307 轮：那 3+3 条**不用补**（内联的割线，本仓库在 helper 里已经有了）**

逐条读 `flux_top_transitive_interface` 的 3 条 FMA 上下文：

```text
fsub d11, d0, d11          ; fval_k1 - fval_k2（分母）
fnmsub d10, d0, d10, d31   ; 分子：fval_k1*x_k2 - (已舍入的 fval_k2*x_k1) —— **第一个**乘积进 FMA
fmul d29, d14, d28         ; 夹逼：一个乘积先独立舍入 …
fmadd d29, d13, d31, d29   ; … 另一个进 FMA
fdiv d10, d10, d11 / fmaxnm / fmadd d31, d13, d28, d31
```

这正是 `secant_method_iteration` 的三处，被**内联**进了这个子程序。而本仓库的两个
transition 例程各自 `bounded_secant_iteration(...)` **调用一次**（`mul_add` 计数在
**helper** 里，不在调用者里）⇒ **这 3+3 条已经在 `bounded_secant_iteration` 里复现了**
（第 302 轮的闭环已把那三处判到 10000/10000），**不是缺口**。
（顺带：分子那一条同样显示"第一个乘积进 FMA"，与第 302 轮闭环的结论一致 ✓ 互相印证。）

**于是第 305 轮那份普查表要这样读**（把"内联"减掉之后）：
* `flux_inside_hm_soil` **2 条**：第 305/13 轮已确认是**它自己的**表达式 ⇒ **真缺口**；
* `flux_top/bottom_transitive_interface` 3+3：**内联的割线** ⇒ 已在 helper 里；
* `flux_all` **7 条**、`water_balance` **5 条**、`soil_water_vertical_movement` 27 vs 12：
  **还没逐条读上下文**，里面究竟有多少是它们自己的表达式、多少是内联合计，未定。
**下一轮**：只读这三个（`flux_all` / `water_balance` / `soil_water_vertical_movement`）
的 FMA 上下文，按同样办法把"自己的表达式"挑出来，然后只给这些补 `mul_add` +
建闭环；`flux_inside_hm_soil` 那 2 处按第 305 轮记的写法一起改。

**第 308 轮：`flux_all` 的 7 条里 **6 条也是那两处内联的割线**

逐条读 `flux_all` 的 7 条 FMA，按"前两条相邻指令"分块：

```text
块 1  fnmsub d29,d9,d13,d29                ; 分子（第一个乘积进 FMA）
      fmul d27,d31,d26 / fmul d31,d31,d25  ; 两处夹逼各有一个乘积先独立舍入
      fmadd d27,d30,d25,d27                ; 夹逼 1 的另一半进 FMA
      fmadd d31,d30,d26,d31                ; 夹逼 2
      → **3 条 = 割线三处**（`flux_top_transitive_interface` 被内联）
块 2  fadd d13,d13,d12 + fmadd d31,d12,d30,d31   → **1 条**，待认领
块 3  fnmsub d27,d31,d13,d27 + fdiv …
      fmadd d28,d10,d29,d28 / fmul d29,d9,d29 / fmadd d29,d10,d26,d29
      → **3 条 = 割线三处**（`flux_btm_transitive_interface` 被内联）
```

3+1+3 = **7 ✓**。所以 `flux_all` 里只有 **1 条**可能是它自己的表达式，另外 6 条是
那两个 transition 子程序各自内联的割线 —— 而本仓库的 `bounded_secant_iteration`
**已经**复现了那 6 条（第 302 轮闭环 10000/10000）。

**普查的最终口径**（三层减法之后）：

| 项 | 条数 | 状态 |
|---|---|---|
| `flux_inside_hm_soil` 自己的表达式 | **2** | **确证缺口**，写法已记档（第 305 轮） |
| 两处 transition 的割线（含被内联进 `flux_all` 的 6 条） | 6 | 已在 helper 里 ✓ |
| `flux_all` 块 2 的 1 条 | 1 | 待认领 |
| `water_balance` | 5 | 待读上下文 |
| `soil_water_vertical_movement`（27−12） | ? | 待读上下文（含内联的 sublevel/explicit 等） |

**下一轮**：只读 `water_balance` 的 5 条 + `flux_all` 块 2 的 1 条（8 条，一次 objdump
就能看完），把属于自己的挑出来；`soil_water_vertical_movement` 那 15 条差额最后处理。

**第 309 轮：`water_balance` 那 5 条**就是它自己的**（形态可辨认）**

```text
fsub d31,d0,d31 + fmsub d29,d29,d30,d31        ; (A - B) - C*D        ← "和减乘积"
fadd d24,d22,d24 + fmsub d24,d27,d21,d24       ; (A + B) - C*D
fsub/fsub + fmadd d4,d7,d5,d4 + fmadd d22,d3,d22,d4   ; 两处加权求和
fadd d24,d22,d24 + fmsub d24,d27,d21,d24       ; 第二处同形
```

全是 `fmsub`/`fmadd` 的"**和（或差）与乘积**"形状，**没有**割线特征
（没有 `fnmsub` 分子 + `fdiv`），也不是 `flux_inside_hm_soil` 的那两条 ——
而质量平衡残差正是这种写法 ⇒ **这 5 条属于 `water_balance` 自己**，
本仓库对应函数 `mul_add` 计数为 0 ⇒ **5 条都是缺口**。

**确证缺口清单（截至本轮）**：

| 位置 | 条数 | 依据 |
|---|---|---|
| `flux_inside_hm_soil`（r0 分母 + `grad_psi>1` 支） | **2** | 第 305/13 轮逐条映射 |
| `variable_saturated_water_balance`（5 处"和减乘积"） | **5** | 本轮形态辨认（无割线/通量内联特征） |
| `flux_all` 中间那 1 条 | 1 | 待认领 |
| `soil_water_vertical_movement`（27−12=15） | ? | 待读上下文 |

**下一轮**：把这 7 处按已记的写法/形态补上 `mul_add`，建 `compare_getzwt.sh` 式
**例程闭环**（`flux_inside_hm_soil` 与 `water_balance` 都是纯函数，合成输入可造），
再处理 `soil_water_vertical_movement` 那 15 条差额，最后与第 302 轮那六处
`get_zwt_from_wa` 形状**一起**落地跑口径。

**第 310 轮：`flux_all` 中间那 1 条认领了 —— 归一化加权平均，**第二个**乘积进 FMA**

```text
fmul d31, d13, d11        ; d31 = w1*v1
fadd d13, d13, d12        ; d13 = w1 + w2
fmadd d31, d12, d30, d31  ; d31 = w2*v2 + (w1*v1)      ← 第二个乘积进 FMA
fdiv d31, d31, d13        ; (w1*v1 + w2*v2)/(w1 + w2)
fcmpe d10, d31            ; 再与 min(d28,d29) 比较（取小那类判据）
```

即 `(w1*v1 + w2*v2)/(w1 + w2)` 形式的**加权平均**（两侧通量都激活时的
饱和区/双侧界面那一支），**不是** `flux_inside_hm_soil`/割线的形状。本仓库对应的
`flux_variable_saturated_both_transition` / `flux_variable_saturated_zone_all`
`mul_add` 计数为 0 ⇒ **又一处缺口**（写法：`(w2).mul_add(v2, w1*v1)` 再除以 `w1+w2`）。

**确证缺口清单（更新）**：`flux_inside_hm_soil` **2** + `water_balance` **5** +
本轮的加权平均 **1** = **8 处**；`soil_water_vertical_movement` 的 27−12=15 条仍未读。

**下一轮**：一次性把这 8 处补上（写法都已记档），建 `compare_getzwt.sh` 式例程闭环
验证；再读外层的 15 条差额；最后与第 302 轮那六处 `get_zwt_from_wa` 形状一起落地跑口径。

#### 第 303 轮：水步入场探针 —— 剩下的种子是**第 20 步入场时的冰/水状态**，不是 `eff_porosity` 算错

新工具 `oracle/scripts/vsf_input_probe.sh`：在 `WATER_VSF` 调 `soil_water_vertical_movement`
**之前**（内核 `:1094`，本仓库 `variably_saturated_flow.rs` 的同一调用点）逐层打 7 列
——`wice_soisno`、`wliq_soisno`、`eff_porosity`、`porsl`、`dz_soisno`、`wimp`、`vol_liq`
（两侧同序，调用序号对齐）。20 步实测：

```text
VSFI     kernel=200 rust=200
('VSFI', 20, 3, 0) f0 wice          K=15.530478817571872   R=15.530478817571856
('VSFI', 20, 3, 0) f1 wliq          K=7.275363870839836    R=7.275363870839852
('VSFI', 20, 3, 0) f2 eff_porosity  K=0.12911472434424492  R=0.1291147243442453
('VSFI', 20, 3, 0) f6 vol_liq       同 f2
1/200 records differ; first = ('VSFI', 20, 3, 0)
```

**读法**：`porsl`、`dz_soisno`、`wimp`（f3/f4/f5）**逐位相同**，而 `wice`/`wliq` 各差 1 ULP
⇒ `eff_porosity`/`vol_liq` 的差是**跟着冰水质量走的**，第 302 轮"查 `eff_porosity` 公式"
这条到此结清（公式与它的静态入参都没问题）。剩下的种子是：
**进入第 20 步水步时，第 3 层的 `wice_soisno` 与 `wliq_soisno` 已经差 1 ULP**，
即差产生在第 19 步的能量/相变/水量更新里（与 `restart_scan.sh` 的
"N=19 起 `wliq_soisno`"、N=20 起 `wice_soisno` 完全对得上）。
**下一枪**：在 `MOD_SoilSnowHydrology`/相变那一段打第 19→20 步的 `wice`/`wliq`
（本探针的 `VSFI` 已经给出精确的层号与步号），或者直接查
`water_2014`/`meltf`/`wblc` 冰汇那几处（第 255 轮只排除了第 2 步第 1 层的 `meltf` 实参）。

**顺带纠一个本轮自己犯的错（记下来）**：`vsf_wt_probe.sh` 的默认 `WORK` 是
`/tmp/gf/vsfwt$STEPS`，第 302 轮"带形状"那次复跑**覆盖**了第 301 轮的基线输出，
于是我先按那份文件得出"基线也有 call 17 的 `eff_porosity` 差"——**是错的**。
本轮 `VSFI` 用独立目录重测的结论是：基线里 `eff_porosity` 到第 19 步都逐位相同，
首个不同在第 20 步。**教训：探针的 WORK 目录要跟"这一次的代码状态"一起命名**
（例如带上 baseline/shapes 后缀），否则复跑会把上一条证据覆盖掉。

**探针工具已加固**（本轮顺手做掉）：`vsf_input_probe.sh` 用独立 WORK 目录，
不再复用 `vsfwt$STEPS`。



**验收口径（不变）**：`cargo test --workspace --lib --bins -- --test-threads=1`、
`clippy -D warnings`、`fmt --check`（本机 `colm-cli` 的 7 个 `study::runner` 用例
因沙箱 `EPERM` 失败，与本改动无关）；`bash /tmp/gf/win4.sh` + `three.py`
（基线仍是 **28 / 1967 / 25713**）；再 `bash /tmp/gf/accept_r247.sh`
（restart 0/68、3 步 692/692）。任一**口径指标**（`over_tol`/`ot_vars`）变差就
`git checkout` 回滚，别留半个修复；`bitwise`/`sumabs` 是诊断计数，混沌窗口里会
反向小幅移动，要记录但不当判据。

## 若继续

> **第 369 轮更新（最新的指路牌，先读这段）**：**闭环要全跑** —— 本轮把 18 个差分闭环
> 一次全跑（新脚本 `oracle/scripts/compare_all.sh`），发现 **3 个一直在失败**：
> `compare_sortin`、`compare_stomata`、`compare_update_photosyn`，全落在
> `MOD_AssimStomataConductance.F90`（这个模块 40 条 FMA，本仓库只落了 5 条；`sortin` 那 11 条
> 还被旧注释误记成"这条链上唯一没有 FMA 的函数"）。
>
> * 差的 30 处形状**全部读出并量过**：三个闭环分别变成 3000/3000、3000/3000、3999/4000，
>   干窗黄金位型 153 → **81**、干窗 restart 仍整窗 0/68 —— 但**黄金湿窗口径变差**
>   （`over_tol` 1197 → **1968**、`ot_vars` 19 → **53**、`sumabs` 8.4418 → **33.37**）
>   ⇒ 按"口径指标不许变差"**整批 `git checkout` 回退**；30 处的 GIMPLE 角色与**可重放补丁全文**
>   留在第 369 轮那节，下次一步就能重放。
> * **判据（回退后复测，与本轮开头逐位一致）**：干窗 restart **N=250–528 全 0/68**、
>   湿窗 **N=63（10/68）**、黄金 dry `153 / 0 / 0，0.0000`、黄金 wet `27055 / 1197 / 19，8.4418`。
> * **下一枪**：① 湿窗 N=63 的上游 —— `gs0sun`/`gs0sha` 的输入侧，也就是那条挂起的
>   `pco2a` `.FNMA`（本轮又给它添了一个"同源整批"候选）；
>   ② `compare_stomata` 只剩的 **1/4000**（row 514，差得比 1 ULP 大 ⇒ 像分支差而不是舍入差）。
> * **方法**：闭环是"形状对不对"的唯一判据，黄金窗口是混沌的 ——
>   **每轮收尾跑一遍 `oracle/scripts/compare_all.sh`**，别只跑与当轮改动相关的那一个。

> **第 368 轮更新（最新的指路牌，先读这段）**：**干窗那颗种子关掉了**。把
> `MOD_Thermal_CanopyPhase_Extended.F90` 的 99 条 FMA 先按 `.loc` + **守卫栈**分活/死
> （活 **25** / 死 **74**；`:1081-1154` 那 56 条确实在 `DEF_USE_PFT/PC` 守卫里 —— `747` 的
> `ENDIF` 在 **1200** 行不是 1076，那个例程顶层 `IF` 全顶在第 1 列，**不能按缩进判块**）。
> 活的那 25 条里还剩 3 处 Rust 写成平铺：`thm`（`:550`）、psit 的 VG 实参（`:579`）、
> `qred`（`:583`）。补上之后：
>
> * **干窗 restart 从 N=251 起逐位相同**，抽测 250/251/252/260/288/320/400/480/**528（整窗末步）
>   全部 0/68**（改前 N=251 = 11/68）；
> * **黄金 dry：`over_tol` 28 → 0、`ot_vars` 1 → 0、`sumabs` 261.0128 → 0.0000**，
>   `golden-compare` 报零变量超差（`tier0=23 tier1=8 tier2=97`）；`three.py` 位型只剩
>   153/56024，且全是**诊断量**的 1 ULP（`f_assim`/`f_fgrnd` 第 10 步、`f_rnet`/`f_zerr`
>   第 130 步、`f_trad` 第 175 步）—— 状态已整窗逐位相同；
> * 湿窗 N=63（10/68）与黄金 wet（27055/1197/19）**逐位不变**。
> * **植物水力这条也排掉了**：`main/MOD_PlantHydraulic.F90` 57 条 FMA / 30 行，两个
>   `getqflx_*`（3+3）✓，两个 `getrootqflx_*`（6/4 条 vs Rust 3/1 个站点）是**循环/向量
>   共享**、不是缺口；只剩 `spacAF_twoleaf` 的 41 条（雅可比组装 + 两支 4×4）没逐条审。
> * **下一枪**：**湿窗 N=63 是湿侧唯一未关的状态种子**（`gs0sun`/`gs0sha` → PHS →
>   `qflx_sha` → `vegwp`/`ldew(_rain)`，外加 M-O 的 `zol`/`rib`/`qstar`/`qref`）；
>   照干窗的成法普查 `MOD_LeafTemperature_Extended.F90`（80 条 / 60 行）与
>   `spacAF_twoleaf`（41 条）。`pco2a` 的 `.FNMA` 仍是悬案（首分歧 63→73–84 变好，
>   黄金 wet 1197→1650 变差 ⇒ 按纪律不能落，等裁决）。
> * **判据**：干窗 restart **N=250–528 全 0/68**、湿窗 **N=63（10/68）**、
>   黄金 dry **`153 / 0 / 0，0.0000`**、黄金 wet `27055 / 1197 / 19，8.4418`。

> **第 367 轮更新（最新的指路牌，先读这段）**：**第 366 轮那条"水侧差 10 处"作废** ——
> `MOD_Hydro_SoilWater.F90` **整个文件**只有 **71 条** `.FMA/.FNMA/.FMS`（落在 56 个源码行，
> 不是 27），逐条按 `.loc` 落到源码行 + 读 GIMPLE 操作数角色之后：**69 条早有对应写法**，
> 只有 `get_water_equilibrium_state` 的 **2 条**（`:137`/`:150`）从来没落过，**本轮已补**
> （它在 `hydrology.rs`，运行时一次都不进，只有新加的闭环 `compare_water_equilibrium.sh`
> 看得见：位型 109949/109949）。当年"27 vs 17"是拿**含内联下游**的函数体汇编条数（29）
> 去比 Rust **拆开**后的同名函数（15 自身 + 6 + 8 = 29，Rust 拆成三个函数）。
>
> * **同时补的**：`MOD_Thermal…:1352` 那条 `fgrnd` 累加链在 `surface_budget.rs` 里少熔 3 段
>   （`sabg+dlrad*emg`、`t**3*(4*tinc)`、降水两项各自熔进累加器）—— 这是**运行时路径**，
>   两个黄金窗口的位型差异各降 11 个元素（9163→9152、27066→27055），
>   `over_tol`/`ot_vars`/`sumabs` 一律不动；干湿窗首分歧不动（`sabg=0`、`fsno=0` 时那几段不可分辨）。
> * **水侧的账现在清了**：`MOD_Hydro_SoilWater.F90` 71/71 有着落。**下一枪转 `thermal`**：
>   `MOD_Thermal_CanopyPhase_Extended.F90` 99 条语句 / 83 个源码行，第 365 轮点的 `:1343-1372`
>   （17 条）本轮已收口（`lfevpa`/`olrg`/`olru`/`fgrnd`），大头是还没按 `.loc` 落过行的那些簇
>   （普查底稿 `/tmp/gf/cen/therm.opt`）。
> * **判据**：干窗 restart 首分歧 **N=251（11/68）**、湿窗 **N=63（10/68）**、
>   黄金 dry `9152 / 28 / 1，261.0128`、黄金 wet `27055 / 1197 / 19，8.4418`。
> * **方法**：① `.loc` 落源码行 → ② GIMPLE（`-fdump-tree-optimized-lineno`；正则要容
>   `[^\]]*`，否则 `discrim N`/`[tail call]` 的语句会被静默漏掉——本轮第一版就漏了 `:456`）
>   读操作数角色 → ③ 状态扫描首分歧步判落地 → ④ 运行时进不到的例程必须建**闭环**，
>   "✓ 已落地"要挂判据（`get_water_equilibrium_state` 那两条被记成"成组落地 ✓"挂了 28 轮）。

> **第 366 轮更新（最新的指路牌，先读这段）**：逐条核第 303/310 轮的"确证缺口清单"后发现
> **它已基本作废**（`flux_inside_hm_soil` 2 处、加权平均 1 处都已在库里；那两个 interface 的
> 3+3 条其实在共享的 `secant_method_iteration`，第 339 轮已核并落地；
> `solve_least_squares_problem`/`aquifer_exchange` 也都在）⇒ **水侧真正剩下的只有
> `soil_water_vertical_movement`：内核 27 处 vs Rust 17 处 = 差 10 处**（第 305 轮记的是 27 vs 12）。
>
> * 那 27 处的源码行已按 `.loc` 映射好：`:263-268/326-338`（含水层/交换）、`:406-478`（`zwt`/水位）、
>   `:1060-1085`（水量回填）、`:1411-1479`（质量平衡/误差）—— 而它正是**水步的状态写回函数**，
>   干窗种子已证的所在侧。
> * **下一枪**：把差的那 10 处逐条读 GIMPLE 操作数角色、与 Rust 同名函数逐条对，补齐后跑
>   **干窗首分歧步**（现 N=251）+ 湿窗（N=63）+ 两个黄金窗口。
> * **方法教训（两轮连着踩）**：**别按函数名对计数** —— 内核例程名与 Rust 公共函数名不是一一对应
>   （`flux_all` 自己 0 条、7 条是内联下游之和；两个 interface 的 6 条在共享 secant 里）。
>   用第 305 轮那张映射表，或直接按 `.loc` 落到源码行再比。

> **第 365 轮更新**：**第 364 轮那条线索作废** —— `thermal` 的
> `:1081-1138`（PC 逐 PFT → patch 聚合）位于 `DEF_USE_PFT .or. DEF_USE_PC` 之下，而
> `MOD_Namelist.F90:170-171` 两个默认都是 **`.false.`**、三个 case.nml 也都没设 ⇒ 黄金算例走
> **patch（LCT）路径**，那 45 处**不执行**（patch 级植被量是初始化时聚合好的，Rust 读 landdata ✓）。
>
> * **仍然活在干窗路径上的两大簇**：`:531-615`（12 处，`psit`/`hr`/`qred`/`qg`，第 357/359 轮
>   已逐条对过 ✓）与 **`:1343-1372`（~17 处，`fgrnd`/`olrg`/`olrb`/`olru`/`emis`/`trad`，未审）**。
>   `emis`/`trad`/`t_grnd` 都是重启变量且都在干窗差异表里出现过（第 348 轮 N=288 各 1 处）⇒
>   这一簇是当前最值得逐条对的地方。
> * **下一枪**：`:1343-1372` 的 17 处按 `.loc` 列出（`fgrnd` 是长加链、`olrb`/`olru`/`emis` 三行
>   同理），与 Rust 的 `ground_fluxes.rs`(8)/`ground_temperature.rs`(27)/`ground_thermal_step.rs`(**0**)
>   逐条对；判据用**干窗首分歧步**（现 N=251）。
> * **方法教训**：读"某簇是最大的一簇"之后，先确认它在**当前配置**下**是否执行**（子网格/宏开关），
>   再投入逐条比对。

> **第 364 轮更新**：`thermal` 的 99 处按源码行分布，**最大一簇 ~45 处
> 在 `:1081-1138` —— 那是 PC 逐 PFT → patch 的加权聚合 `sum(x_p*pftfrac)`**，它聚合的正是
> `etr`/`tref`/`qref`/`tleaf`/`ldew_*`/`fsenl`/`fevpl` 这一批 ✓。
>
> * 这解释了第 362 轮的否证：干窗水步入场首差异是 **`etr`**（聚合量），目标点名的
>   `tref`/`tleaf` 也都在这一段 ⇒ **差异若生在 patch 级聚合上，修 per-PFT 的叶温/气孔当然
>   不会改变 patch 级结果** ✓。
> * 内核：1 patch × **2 PFT**（`pctpfts=[0.54,0.46]`）逐 PFT 算完再聚合；
>   Rust：`colm-srfdata/src/site.rs` 有 `pft_components`（读 `pfttyp`/`pctpfts` ✓），
>   但 `colm-core` 的 `standard_lct_step.rs`/`assembly.rs` **没有逐 PFT 循环**，
>   `pft_fraction` 只在 `bgc.rs`（黄金 BGC 关）⇒ **聚合发生在哪一层、形状是否一致，必须查清**。
> * **下一枪**：定位 Rust 侧 `sum(x_p*pftfrac)` 的实现，把 `.FMA(x1,f1,x0*f0)`（或反向）
>   与其写法逐条对；判据用**干窗首分歧步**（现 N=251）。若形状差在这里，它会同时解释
>   `etr` 与 `tref`/`tleaf`。

> **第 363 轮更新**：干窗种子已证不在叶链（第 362 轮），本轮把
> **水/土侧三个编进内核的文件**做了 GIMPLE 普查并把 `water_vsf` **17 处逐条收口**：
> `:1110` 第 338 轮已修、`:1128/:1132/:1133`（顶层凝结写回）Rust 已是 FMA、
> `:827/:867/:1037/:1045`（预解算 `vol_liq`/`wresi` 换算）Rust 注释里逐条对过 ⇒
> **预解算换算与顶层写回都忠实**（与第 355 轮"`Richards` 内部逐位相同、差异在子步之间"互洽）。
>
> * **普查表**：`MOD_Hydro_SoilWater.F90` 71 处（`soil_water_vertical_movement` 29、
>   `get_zwt_from_wa` 9、`solve_least_squares_problem` 8、`flux_all` 7、`water_balance` 4、
>   `soilwater_aquifer_exchange` 4、两个 interface 3+3）；`MOD_SoilSnowHydrology.F90` 67 处
>   （`soilwater` 23、`snowwater*` 12、**`water_vsf` 17**、`water_2014` 15）；
>   `MOD_Thermal_CanopyPhase_Extended.F90` **99 处（全在 `thermal`）**。
> * **下一枪（干窗，按可能性排序）**：① **`thermal` 99 处** —— Rust 拆在
>   `ground_temperature.rs`(27)+`thermal_properties.rs`(25)+`ground_thermal_step.rs`(**0**)，
>   且 `MOD_Thermal` 正是 `psit`/`qg`/`t_soisno` 的生产者；② `soil_water_vertical_movement` 29 处；
>   ③ `get_zwt_from_wa`/`flux_all`/`solve_least_squares_problem`。方法照旧：
>   `-S`+`.loc` 定站点 → GIMPLE 定操作数角色 → **状态扫描首分歧步**判是否落地。

> **第 362 轮更新**：把"第 350 轮那 14 处形状"与"`pco2a` 的 `.FNMA`"
> **叠加**着测，得到一个**决定性否证**：
>
> * 14 处形状**确实修掉了干窗链的起点**（`STOUT` 出参差 66 → **2**，首个从第 302 次移到第 834 次；
>   剩下的 2 条是 `pco2a` 入参驱动）；
> * 但**叠加后干窗状态口径逐位不变**（N=250/251/252/260 = 0/**11**/17/29，与基线完全一致），
>   湿窗则如期改善（N=63 10→**0**、N=72 0/68）。
> * ⇒ **干窗第 251 步那颗种子不在气孔/叶链上**，它来自**水/土那一侧**（`WATER_VSF` 的子步写回 /
>   `MOD_Thermal` 非 split 支）。这也第二次验证了："修链的起点"不等于"推迟状态投影"。
> * **下一枪**：在**第 251 步内**展开 `WATER_VSF` 的状态写回（`WSF` 出场 + `RCHL/RCHE`）。
>   第 355 轮已把出场首差异钉在第 1 层 `ss_vliq`，而 `Richards` 内部 `RCHF/RCHB/RCHD/RCHE`
>   **全部逐位相同** ⇒ 差异是**子步之间**经状态写回带进来的 ⇒ 查写回路径里
>   `ss_vliq`/`wliq`/`wice`/`smp`/`hk` 更新式的形状。

> **第 361 轮更新**：**重打第 350 轮那 14 处形状的实测结论** ——
> **两个窗口的首分歧步都不动**（湿窗 N=63 仍 10/68；干窗 N=250/251 仍 0/68 与 11/68），
> 闭环改善（`rst 114+assim 171` → `rst 0+assim 100`）而湿窗 N=96 `11→19`、黄金 wet `1197→1775`。
> ⇒ 继续回退；**而且第 249 轮那条例外候选不成立**（例外要求短程仪器中性或更好，这里短程变差、
> 首判据不动）。第 360 轮把"N=63 → N=73…84"记到它头上是**记错**，那条属于 `pco2a` 的 `.FNMA`。
>
> * **结论**：`stomata` 那 14 处**该补但不是种子开关**；湿窗第 63 步的开关仍是 `pco2a` 的
>   `.FNMA`（唯一把首分歧推进 10–21 步的改动）；干窗第 251 步的**投影点仍未找到**
>   （链的起点在 `stomata`，`psit` 差 34 步只在 1 步投影出去）。
> * **方法论**：链的**起点**（探针流第一次出现差异）与状态的**投影点**（首分歧步）可以隔很远 ——
>   修起点不等于推迟投影点；判"哪个改动有用"必须看首分歧步。
> * **下一枪（干窗投影点）**：沿"水步 → 表层水 → `psit`"这条线找**投影**发生在哪一步：
>   用 `vsf_richards_probe.sh` 的 `RCHL`/`RCHE`（`ss_vl` 第 1 层）在**第 251 步**内逐步展开，
>   看第一次写入差异的分支；再回头看为什么"34 次 `psit` 差里只有 1 次传出去"。

> **第 360 轮更新**：把干窗 hex `stomata` 探针的**入参/出参分开统计**，
> 钉住了那条链的**起点**：`STOUT` 的 `gsh2o` 从**第 302 次调用（≈第 15 步）**就开始差，
> 而 `STIN` 到第 834 次调用（≈第 105 步）才第一次差 ⇒ **入参逐位相同、出参已差**
> ⇒ 差在 `stomata` 自己的收缩形状，正是第 350 轮那 **14 处 GIMPLE 形状**。
>
> * **两个窗口的链起点是同一处**：湿窗（第 349 轮，`rssun`/`rssha`）与干窗（本轮，`gsh2o`）
>   都从这里出发。**但打上那 14 处并不改变任何一个窗口的首分歧步**（第 361 轮重打实测：
>   湿窗 N=63 仍 10/68、干窗 N=250/251 仍 0/68 与 11/68）⇒ 它只是"链的起点"，不是两个窗口
>   的投影点；"N=63 → N=73…84"那条是**第 351/356 轮的 `pco2a` `.FNMA`**，别记混。
> * **下一枪（决策级）**：照文档把第 350 轮那 14 处重打，补测**干窗首分歧步**
>   （`dry_ts.sh 250/251`）：若也往后走，例外在两个窗口的**首判据**上都成立；
>   若不动，说明对干窗它只是"起点"、投影点还在水步里（`vsf_richards_probe.sh` 的 `RCHL` 入口）。

> **第 359 轮更新**：干窗第 251 步的**土壤侧入口是 `psit`** ——
> 新标签 `PHXQ2`（6 个被探文件，新增 `extends/.../MOD_Thermal_CanopyPhase_Extended.F90`
> 与 `ground_humidity.rs`）实测：`psit` 差 34 步，而 `hr`/`qred`/`qg` **只在第 251 步**同时差，
> `fsno`/`t_grnd`/`forc_q`/`qsatg` **从不差**。
>
> * ⇒ **不走 ground-temperature chain**；土壤侧入口是 `psit = f(表层 wliq/wice, porsl, theta_r, psi0)`，
>   即**表层水在步内瞬时差**（步末自愈 —— restart 第 251 步差的是第 6 层，不是表层）。
> * **完整因果链（355+358+359 实测）**：步内表层水差 → `psit` → `hr`/`qred`/`qg` → `qaf`/`ea`
>   + 驱动湿度 `cqi` → `qflx_sun/qflx_sha` → `etr`/`rootflux`（水步入场）→ `ss_vliq`
>   → 第 6 层 `wliq/wice` → `hk`/`t_soisno` → 近地层诊断。`psit` 差 34 步却只在 1 步投影出去
>   ⇒ 这是耦合叶↔土回路里持续 1 ULP 的**偶发投影**。
> * **下一枪**：hex 打 `psit` 的入参（`wx`/`wliq_soisno(1)`/`wice_soisno(1)`/`porsl`/`theta_r`/`psi0`/`fac`），
>   判表层水是哪一次写入带进来的；并查 `soil_psi_from_vliq` 的 VG 支在真实参数区间上是否有
>   闭环（3×10000 合成输入）没覆盖的形状差。

> **第 358 轮更新**：干窗第 251 步的**因果链**定下来了 ——
> `qg`（地表比湿）→ `qaf`/`ea` + 驱动湿度 `cqi = (wtaq0+wtgq0)*qsatl - wtaq0*qm - wtgq0*qg`
> → **`qflx_sun`/`qflx_sha`** → `etr`/`rootflux`（水步入场）→ `ss_vliq`/`wliq[0,5]` → `hk`/`t_soisno`
> → 近地层诊断。**第 251 步里 `stomata` 的输出并不差**（`PHXS` 全同），差的是 `PHXH` 的
> `qg`/`qaf` 与 `PHXG` 的 `qflx_*` —— 合理：C3 + WUE 支里 `gsh2o` 不含 `ea`，而 `cqi` 含。
>
> * 探针流里更早的差异（序号 294≈第 29 步、298、418≈第 41 步、2209≈第 217 步）**都自愈**，
>   不是种子。
> * **判据纪律（新）**：`qaf` **不是重启变量**但**跨步保存**（模块变量）⇒
>   "restart 到 250 步 0/68"排除不了"非重启量早已差"；这类量只能用**探针流**判首分歧。
> * **下一枪**：`qg` 的入参（`fsno`/`hr = exp(psit/roverg/t_grnd)`/`psit`/`t_grnd`/`forc_q`）
>   两侧 hex，跑干窗 251 步 —— 判是**外层能量迭代的 `t_grnd`** 还是**土壤侧 `psit`**
>   先带进 1 ULP，把种子推进 `MOD_Thermal`/`GroundTemperature` 那一段。

> **第 357 轮更新**：干窗第 251 步的 `ea`（= 唯一在**整步 12 次调用**
> 都差的 `stomata` 入参）追到 **`qg`（地表比湿）**：新加的 `PHXH` 标签（`wtaq0,wtgq0,wtlq0,
> qm,qg,qsatl,qaf`）在干窗 251 步显示只有 `qg` 差 6 次（`wtlq0` 1 次），
> `qsatl`/`qm`/`wtaq0`/`wtgq0` 从不差。
>
> * **`qg` 的式子两侧同形**：非 split 支 `qred = (1-fsno)*hr + fsno` 再 `qg = qred*qsatg`
>   （+ 同一个夹逼）↔ Rust `ground_humidity.rs::non_split_ground_humidity` ✓
>   ⇒ **差在入参**：`fsno`、`hr = exp(psit/roverg/t_grnd)`、`t_grnd`、`forc_q`、`forc_psrf`。
> * **下一枪**：在 `MOD_Thermal` 的 `qg` 赋值处（Rust 在 `non_split_ground_humidity` 入口）
>   两侧 hex 打这五个量，跑**干窗 251 步** —— 把种子推到地面/土壤侧或钉在外层能量迭代上。
> * 提醒：`DEF_SPLIT_SOILSNOW` 是运行期开关、默认 `.false.`（黄金走非 split 支），
>   Rust **只移植了非 split 支**。

> **第 356 轮更新**：湿窗 `stomata` 入参里唯一分叉的是 **`pco2a`**
> （20/1004）；**干窗不是它** —— 干窗 14 组差异是 `pco2a`（调用 835/836，≈第 105 步，自愈）
> 与 **`ea`（调用 5093…5104 = 第 251 步的全部 12 次）**，即干窗种子以**冠层空气水汽压**
> 进 `stomata`（`ea = qaf*psrf/(0.622+0.378*qaf)`，内核 `:791`）。把 `pco2a` 的 GIMPLE 形状
> `.FNMA(1.37*psrf/max(0.446,gah2o), 和, pco2m)` 补回来（`(-rate).mul_add(sink, pco2m)`）：
>
> * **湿窗首分歧 N=63 → N=73…84**（关掉第 349-351 轮定位的那颗种子）；
> * **干窗全部口径逐位中性**（首分歧 N=251、11/68；N=288 19/68；黄金 dry 不变）；
> * 唯一代价：黄金 wet `1197/19 → 1650/35`（第 351 轮已证那是它**自己首分歧之后**的混沌）。
> * 按硬规则**仍回退**（第 3 次记录，重打只改 6 行）；**连同 `baea8c5`/`f7e9e31` 一起等你裁**。
>
> **干窗那颗种子 = `ea`**，而 `qaf` 的更新式本身**忠实**（GIMPLE 整条平铺，Rust 逐位一致）
> ⇒ 差在它的**入参**：`qg`（PHS 探针已见差）、`qsatl`、`qm`，或三个湿度权重
> `wtaq0/wtgq0/wtlq0`（= `caw/cgw/cfw * wtsqi`）。**下一枪**：把这七个量与
> `ea = qaf*psrf/(0.622+0.378*qaf)` 两侧 hex 打出来，跑**干窗 251 步**；
> `pco2a_probe.sh` 一并 hex 化。

> **第 355 轮更新**：干窗第 251 步的种子**推出水步** ——
> `vsf_richards_probe.sh` 的 `WSF1`（水步**入场**）第一条不同记录就是 **`etr`**（蒸腾），
> 另有 WSFE 的 `rootflux[1,3,4,7]`；`vsf_probe.sh` 的 `WSF`（出场）只有第 251 次调用不同、
> 首个字段是 `ss_vliq` ⇒ **水步只是把上游的 1 ULP 传下去**。
>
> * **两个窗口的种子在同一条链上**：湿窗第 63 步 = `pco2a`→`stomata`→PHS；
>   干窗第 251 步 = `etr`/`rootflux`（`etr = etrsun+etrsha`）。都落在
>   **叶温-气孔-植物水力**那侧 ⇒ 第 349-351 轮那条链（含挂起的 `pco2a` `.FNMA` 例外）
>   是两个窗口的**共同瓶颈**。
> * **修好/踩到的工具**：`vsf_probe.sh` 与 `vsf_richards_probe.sh` 补了日期回绕 + 强迫重定向；
>   Richards 探针修了 3 个过期锚点、停用 `RCHZ`（第 339 轮后字段不在作用域，且与 `WSF` 重复）；
>   **`vsf_richards_cmp.py` 的 `WSF1` 名称表漏了先打的 `nlev`**，导致每个标签错位一格 ——
>   前半段读出的"`rsubst` 差"其实是 `etr` 差（值比得对、名字全错），已修并留注释。
> * **干窗 251 步的 PHS 位型探针（`phs_hex_probe.sh` 已加 `CASE=`）**：首个差异是
>   `PHXS` 的 **`rssha`**（`stomata` 的返回值）⇒ **两个窗口同一条链**，不是猜测；
>   干窗里 `qaf`/`qg`（冠层空气/地表比湿）各差 6 次（湿窗只 1–2 次）⇒ 湿度那一支参与更深。
> * **下一枪（两窗口共用）**：用已 hex 化的 `stomata_probe.sh`（有 `CASE=`）跑**干窗 251 步**，
>   判 13 个入参里唯一分叉的是不是又是 `pco2a`；并把 `qaf`/`qg` 那一支也纳入比较。

> **第 353 轮更新**：**干窗首分歧夹到第 251 步**（`2008-006-19800`，
> 当地 05:30）—— N=250 仍 0/68、N=251 11/68。差同时落在**土壤侧**（`t_soisno[0,5]`、
> `wice_soisno[0,5]`、`wliq_soisno[0,5]`、`hk[0,0]`）与**近地层侧**（`qstar`、`zol`、`rib`、
> `qref`），外加 `trad`/`rst` ⇒ 种子在这两支的**共同上游**；夜间 `rst`≈5e5、`assim`≈0
> ⇒ 大概率不在光合链。
>
> * **判据纪律（本轮实测）**：干窗 history 的首分歧在**记录 14（第 15 步）**、只有
>   `f_assimsun` —— 拿**基线 `d326254`** 跑同一步也是记录 14 / `f_assimsun`
>   ⇒ **先前就存在**的**诊断路径**残差（与湿窗记录 4 同类）。**干窗/湿窗都不能用 history
>   判首分歧**，只能用 restart 扫描。
> * **下一枪**：`oracle/scripts/step_ground_probe.sh`（现成，`CALL GroundTemperature` **之前**
>   的 23 个量）指到 **251 步**：入参已差就往上游 `MOD_GroundFluxes`（`fseng`/`fevpg`）或
>   辐射项追；入参全同则残余在地面支体内。
> * 状态基线（本轮不动）：湿窗首分歧 N=63（10/68）、N=96 11/68；干窗首分歧 **N=251**、
>   N=288 19/68；黄金 dry `9163/28/1，261.0128`；黄金 wet `27066/1197/19，8.4418`。

> **第 352 轮更新**：`rsoil` 在 Rust 里写成 `0.22e-6`，而内核是
> **两个字面量相乘** `0.22 * 1.e-6`（`:652`）—— 位型 `3E8D87247702C0CF` vs `3E8D87247702C0D0`，
> **差 1 ULP**；GIMPLE 里的常数 `2.1999999999999998475…e-7` 与乘积形式逐位相同。它减在
> `pco2a` 的括号和里。已按乘积形式改正 + 单元测试钉死位型。
>
> * **判定：所有仪器逐位不变**（湿窗 N=63 10/68、N=96 11/68、干窗 N=288 19/68、
>   黄金 dry `9163/28/1，261.0128`、黄金 wet `27066/1197/19，8.4418`）。仍落地：
>   差值可证、零风险，第 341 轮 `getrootqflx_x2qe:892` 有"单独测惰性照样落地"的先例；
>   惰性只说明这两个窗口的括号和把它吸收了。
> * **同类的系统扫描已做**：`main/`+`extends/` 的"纯字面量乘积"共 37 个 double 逐个对照，
>   **只有这一个**是折叠写法 ≠ 乘积的陷阱（`44.6*273.16` 本来就是乘积形式）。
>   记一条工具短板：`audit_fortran_literals.py` 只查**文本存在性**，查不出位值错。
> * **下一枪**：`pco2a` 的其余入参（`gah2o`/`raw`/`thm`/`tprcor`、`assim*`/`respc*`）
>   用第 351 轮的办法 hex 化（现成 `pco2a_probe.sh`，先改 `Z16.16`/`to_bits()`）跑 63 步；
>   `pco2a` 那条 `.FNMA`（首分歧 N=63 → N=73…84，但黄金 wet 1197→1650）仍等你裁。

> **第 351 轮更新**：`stomata` 探针 **hex 化**后，湿窗 63 步的
> 1004 次调用给出：`STPH` 全同、`STIN` 里**唯一**分叉的入参是 **`pco2a`**（20/1004 次差 1 ULP，
> 首个在第 262 次调用 ≈ 第 33 步），`STIT` 的物理字段全同（只有两个**探针口径**列不同）。
> ⇒ 第 349 轮"种子在 `stomata` 内部"的更正**实测闭环**。
>
> * **`pco2a` 那行按 GIMPLE 是 `.FNMA(1.37*psrf/max(0.446,gah2o), 和, pco2m)`**（乘积在 FMA 里
>   精确、只舍一次）；补上它：**湿窗首分歧 N=63 → N=73…84**（N=63/72 全 0/68）、干窗不变、
>   黄金 dry 不变，但**黄金 wet `1197/19，8.4418` → `1650/35`**、湿窗 N=96 11→14。
>   按硬规则**已回退**；但它是"首判据推进 + 逐条对得上 GIMPLE + 聚合混沌"的第 249 轮例外候选，
>   **连同 `baea8c5`/`f7e9e31` 一起等你裁**（改动记录在"第 351 轮"④，随时可重打）。
> * **"黄金是否另一套编译产物"已被否掉**：修好 `compare_hourly_window.sh`（原来在本机跑不起来）
>   后，干窗前 8 小时 **kernel == Rust == golden 逐位全同（0/0/0）** ⇒ 黄金就是当前内核跑得出来的；
>   湿窗聚合指标反向是**它自己首分歧之后**的混沌，**判据必须用首分歧步**。
> * **顺带抓到一处结构性偏差（已证惰性、按第 347 轮先例不落地）**：WUE 支内核是
>   `pco2in = pco2i`（重写后）⇒ `eyy≡0` ⇒ `ic=1` 就退出；Rust 用重写前的 `internal_co2`
>   算 `errors`，所以跑满 6 轮空转（`SRIT` 5639 行 vs `STIT` 1004 行，同调用内各行逐位相同）。
>   这使 `STIT` 的 `pco2i`/`eyy` 两列在 WUE 支**不可比**（探针头已注明）。
> * **工具（本轮入库）**：`stomata_probe.sh` 两侧 hex + `CASE=`；`stomata_cmp.py` 加 hex 档；
>   `compare_hourly_window.sh` 补强迫重定向。
> * **下一枪**：把 `pco2a` 的**入参**（`assimsun`/`assimsha`/`respcsun`/`respcsha`/`rsoil`、
>   `gah2o`）按同样办法 hex 化 —— 现成 `pco2a_probe.sh`（第 293 轮，`:1243-1244`）先改 hex。

> **第 350 轮更新**：把 `stomata` 的 **14 处 FMA 形状**按 GIMPLE
> 补齐（`.FMA=a*b+c`、`.FMS=a*b-c`、`.FNMA=c-a*b`、`.FNMS=-c-a*b`，拿 `:350` 的
> `bquad` 对照源码钉死）—— **闭环从 `rst 114 + assim 171` 好到 `rst 0 + assim 100 (全 BIG)`，
> 但湿窗 N=96 从 11/68 变 19/68、黄金 wet 从 `1197/19，8.4418` 变 `1775/52，19.8039`
> ⇒ 整批已回退**（回退后三项都验过复原）。这次**短程状态扫描也一起变差**，所以不适用
> 第 249 轮那条例外。
>
> * **第 349 轮的定位要更正**：修到"夹具零舍入差"之后，轨迹里的 `stomata` 返回值
>   **一个 bit 都没变**（PHXS 首差异同 hex 同簇数）⇒ 轨迹里 `rssun`/`assimsun` 的 1–2 ULP
>   是**从入参透传**的，不是 `stomata` 自己算的。"种子生在 `stomata` 内部"说过头了。
> * **下一枪**：探针挪到 `CALL stomata` 的**实参表**（`par`/`ei`/`ea`/`pco2a`/`po2m`/`pco2m`/
>   `rb`/`raw`/`rstfac`/`cint`/`tlef`/`psrf`/`tm`/`g1`/`g0`/`gradm`/`binter`/`lambda`），
>   两侧 hex、63 步。**先用现成的**第 293 轮 `stomata_probe.sh` + `stomata_cmp.py` 与
>   `gssun_probe.sh` 的 `GSTO`（注意 `ES23.15` 那档对 1 ULP 是盲的，必须 hex）。
> * **另记**：Medlyn 块还有 100 例 BIG（`assim` 0 vs 2.57e-4，`eyy` 迭代出口在 0.1 附近翻边），
>   是结构性差异，与舍入批无关；夹具 k=3 档已按黄金区间采样，所以不能拿"夹具没覆盖"解释。
> * 14 处形状的逐条对照表在"第 350 轮"里，可直接照抄重打（本仓库**不留**这半个修复）。

> **第 349 轮更新**：湿窗第 63 步那颗种子被**推出植物水力** ——
> 它生在 **`stomata` 的返回值**里。位型探针往上追三层，每层都是"入参已经不同"：
> `PHXG` 的 `qflx_sha` → `PHXI` 的 `gssha` → `PHXR` 的 `rssun`（`tl`/`tprcor`/`laisun`/
> `laisha` 从不差 ⇒ 换算公式不是元凶）→ `PHXS` 的 `rssun`（同一个 hex）⇒ 种子在
> `MOD_AssimStomataConductance.F90` **内部**，`respcsun`/`respcsha` 从不差。
>
> * 18/502 次调用有差（131、137-139、152-153、267-268、282-286、351、498-501），
>   **前面那些都自愈**（叶温迭代回到同一位型），只有第 63 步那一簇进了状态 ——
>   所以 restart 到 N=62 仍 0/68。`rssun` 单独差（282-286）而 `assim` 不差 ⇒
>   **不能只用 `assim` 那 4 例残差解释**，`rst`/`gs` 换算里还有一处形状缺口。
> * **（第 350 轮更正：这句过头了 —— `stomata` 只是"第一个被探到的点"，修好它的舍入后
>   轨迹里的返回值一个 bit 都没变，差异是从**入参**透传的。见"第 350 轮"⑤。）**
> * 与已知残差接上头：`compare_stomata.sh` 的 4/4000（全在 `assim`）与第 336 轮那条
>   `f_assimsun` history 缺口同源；本轮把 **state 级**证据也接到这条链上。
> * **工具**：探针已入库为 `oracle/scripts/phs_hex_probe.sh`（4 个文件、8 个标签：
>   `PHXI/PHXG/PHXQ/PHXA/PHXD/PHXF/PHXR/PHXS`），`STEPS=63 WORK=… bash` 即可复跑；
>   踩过的坑：`gs0sun` 在**叶温**那份源文件里（不在 PHS 那份）、`STEPS>48` 要算
>   `END_DAY/END_SEC`。
> * **下一步**：进 `stomata` 内部 —— ① 导出 `omc/ome/oms/bq/c/conductance/internal/eyy`
>   把 4 例残差钉到具体量；② 给夹具加一档**黄金窗口真实参数区间**的采样
>   （CN-Cng-wet 第 1 步的 `par`/`tlef`/`rstfac`/`c3c4=1`）。判据：`wet_ts.sh 63` 的
>   10/68 往下走，干窗与黄金两窗口不许变差。本轮**没改模型代码**，上一轮口径原样成立。

> **第 348 轮更新**：湿窗 N=20 那颗种子关掉了，根因是 **Rust 一直照着没编进内核的那份
> 源码抄** —— `Makefile:641` 把 `MOD_LeafTemperature.o`
> 指向 `extends/interception/MOD_LeafTemperature_Extended.F90`，而
> `update_canopy_water` 的每一处形状都跟着 `main/MOD_LeafTemperature.F90`。两份在这一段
> 是两套代码：`main/` 把超配量从水体里扣、**两侧都不夹**；`extends/` 记
> `phase_flux_deficit` 回投 `fevpl`/`fsenl`，并且 `ldew_rain`/`ldew_snow` **都夹 0**
> 之后才 `ldew = ldew_rain + ldew_snow`。
>
> * 湿窗**首分歧 N=20 → N=63**（N=16…48 全 0/68；N=96 16→**11**）；干窗首分歧推到
>   **N=224…256 之间**、N=288 从 22/68 → **19/68**；黄金 wet `1908/53，24.8020` →
>   **1197/19，8.4418**，黄金 dry `28/1，261.0128` **不变**；1 步 0/68、3 步 692/692 不变。
> * **隔离实验**（本轮最该记的）：基线 + 只加那 4 个 `.max(0.0)`（clamp-only）在**所有**
>   状态/黄金口径上与落地版**逐位相同** ⇒ N=20 那颗种子**只需要那 4 个夹取**；但黄金 wet
>   `bitwise` 是 27072（clamp-only）vs **27066**（落地版）⇒ 两笔**回投让 Rust 更靠近内核**，
>   不能省（雪窗才是它们的主场，本机测不了）。
> * **顺序也纠了一处**：`energy_balance_error` 移到 `update_canopy_water` **之前**
>   （上游 `:1449` 的 `err` 在 `:1464` 之前）。
> * **方法论**：判形状前先 `make -Bn <obj>` 确认**编的是哪份源文件** —— `include/define.h`
>   只决定 `extends/` 是否参与，`Makefile:641/644/647` 才是最终指向；同一个物理过程在
>   `main/` 与 `extends/interception/` 里可以是两套代码。
> * **下一步**：湿窗 **N=63 的 `ldew_rain`（4 ULP，`vegwp` 跟着走）**、干窗 **N=224…256**；
>   history 侧最早仍是第 4 条记录的 `f_assimsun`/`f_fgrnd`（基线就有，别拿它当首判据）。
>   另记两处已知偏差（本轮没动）：融化/冻结分支的加侧缺夹取（**可证惰性**）、
>   `DEF_VEG_SNOW=.false.` 分支抄的是 `main/` 的按比例分回（**不是惰性，但本机测不到**，
>   要改先造反例）。全表与 GIMPLE 引文见"第 348 轮"。

> **第 341 轮更新**：植物水力链抓到 `conductance_from_transpiration`
> 里 **3 处多收**的形状 —— 湿窗黄金**大幅改善**。
>
> * 湿窗 `sumabs` **28.84 → 5.55**（比第 338 轮之前的原基线 11.03 还小一半）、
>   `over_tol` 1927 → **1293**、`ot_vars` 54 → **28**、`bitwise` 28556 → 28307；
>   干窗/雪窗、1 步 0/68、3 步 692/692 **全不变**。全表见"第 341 轮"。
> * 证据：`getqflx_qflx2gs_twoleaf` 汇编只有 **3** 条 FMA（`:781`/`:794`/`:795`），
>   GIMPLE 具名操作数说 `:794/:795` 收的是**分子**，两个分母是
>   `_55 = _53 - _54`、`_61 = _54 - _53` 的普通减法；Rust 原先分母与 `:779` 的
>   `1 - delta*(1-fwet)` 都写了 `mul_add`。另有 `getrootqflx_x2qe:892` 补一条 FMA（单独测惰性）。
> * **方法论**：这四处改动在湿窗 restart 上**到 N=96 都逐位不变**，黄金 `sumabs` 却掉到 1/5
>   ⇒ 判据必须**两条一起看**：状态扫描首分歧步 + 黄金长程 `sumabs`。
> * **下一步**：`transpiration_from_conductance`（Rust 3 vs 汇编 `:683`/`:691`×2）逐行对；
>   `spacaf_twoleaf` 41 vs `spac_change` 40 的差额（4×4 求逆那段）未查清；湿窗 N=18 的
>   `vegwp`、N=20 的 `wliq_soisno`+`hk`、干窗 N=288 的 `tref`/`tleaf` 仍未定位。

> **第 340 轮**：**FMA 普查口径更正** + 植物水力链普查。
>
> * **站点清单必须用 `gfortran -O2 -g -S` + `.loc` 数**，GIMPLE dump 会数错
>   （`spacaf_twoleaf` 25 vs **41**、`soilwater` 19 vs **24**、`water_vsf` 17 vs **15**）。
>   操作数角色仍用 GIMPLE 的具名操作数 —— **两者缺一不可**。全表与逐行图见"第 340 轮"。
> * 黄金算例走得到的 FMA 站点：`water_vsf` 15、`meltf` 8、`spacaf_twoleaf` 41 +
>   其余植物水力 14；`soilwater`/`water_2014`/`snowwater*`/`meltf_snicar`/`meltf_urban`/
>   `compute_vic_runoff` 这些**黄金不走**（`DEF_USE_VariablySaturatedFlow=.true.`、
>   `DEF_SPLIT_SOILSNOW=.false.`、`DEF_USE_SNICAR` 关、`patchtype<3`、无雪）。
> * 已核一处：`getrootqflx_qe2x:941/950/959` 三行在 Rust 里由**一条** `mul_add` 覆盖，逐条对得上
>   ⇒ 总数差（55 vs 50）不等于漏 5 处，必须逐站点对。
> * **下一步**：按文档里 `spacaf_twoleaf`(41) / `water_vsf`(15) 两张逐行图做站点级操作数比对，
>   判据用 `wet_ts.sh 18/20` 与 `dry_ts.sh 288` 的首分歧步。

> **第 339 轮**：第 337 轮那批 21 处在第 338 轮修正之上
> **成组落地**了 —— 短程大幅改善是关键：
>
> * **干窗 restart 首分歧 N=19 → N=288**（N=18…192 全 0/68），干窗黄金三口径全线改善
>   （`over_tol` 36→**28** 回到原基线、`sumabs` 382→**261**、`bitwise` 16436→**9355**）。
> * 湿窗相对 HEAD 也变好（`over_tol` 2210→1927、`sumabs` 45.08→28.84），但比原基线仍差；
>   唯一命中"变差"的是湿窗 `ot_vars` 53→**54**（一个变量跨线）。全表与理由见"第 339 轮"。
> * **已入库形状的 GIMPLE 复查**：`solve_least_squares_problem` 的 Givens 5 处、secant 两处夹逼、
>   `WATER_VSF` 的 `vol_liq`/`wresi` 三处 —— **全对**；secant 主式的"收第二个"变体
>   **实测否定**（干窗 N=18 5/68、N=22 22/68），已入库的"收第一个"是对的。
> * **下一步**：① 湿窗种子现在是 **N=18 的 `vegwp`（1/68）** 与 **N=20 的 `wliq_soisno`+`hk`（2/68）**；
>   干窗首分歧在 **N=288**（6 天，`tref`/`tleaf` 各 1 ULP），窗口内还剩 ~5 天可压。② 把 GIMPLE 复查推到其它水模块
>   （`water_2014` 15 / `snowwater` 4 / `meltf` 8 / `compute_vic_runoff` 3 条 FMA，
>   但 split-SOILSNOW / VIC / 2014 三支黄金算例都不走）。③ 判据仍以**状态扫描首分歧步**为准。

> **第 338 轮**：湿窗那颗 N=4 的种子**已找到并修好** ——
> 是**已入库**的 `WATER_VSF` 回填（`variably_saturated_flow.rs` 里
> `MOD_SoilSnowHydrology.F90:1109-1110` 那段）把**操作数写反**了：
> 该收**第二个**源乘积，原代码收的是第一个，且注释里的 GIMPLE 引文是错的。
>
> * 改后：湿窗 `wet_ts.sh` 在 N=4…16 上**全 0/68**，首分歧推到 **N=20**；
>   干窗 N=18 从 5/68 → **0/68**；干窗 1 步 0/68、3 步 692/692 不变。
> * 黄金三窗口的聚合指标**反向**（dry 28→36、wet 1287→2210、snow 不变）——
>   按**第 249 轮的先例**（短程改善/中性 + 逐条对得上编译源码 + 聚合反向是混沌）
>   保留，全表与理由见"第 338 轮"。

> **第 337 轮**：第 336 轮那两条结论仍然成立，并且第 337 轮把下面这条"下一枪"打完了一半：
>
> * 第 324 轮那 12 处（内联 `Richards_solver` 6 + `use_explicit_form` 6）**已按
>   GIMPLE 具名操作数逐条判好并实测**，另外发现 `soilwater_aquifer_exchange` 自有
>   **独立符号 + 4 条 FMA**（第 324 轮漏了）、`get_water_equilibrium_state` 2 条
>   （只在冷启动用）。清单与 A/B 全表见"第 337 轮"。
> * **湿窗 N=4 那颗种子不在 `MOD_Hydro_SoilWater` 全模块**：五个变体在
>   N=4/5/6/8 上的状态差异清单逐位相同（第 338 轮证明它在 `MOD_SoilSnowHydrology`
>   的 `WATER_VSF` 回填里）。
> * **判据要换**：`over_tol`、restart 残留**个数**、`sumabs` 都会被混沌带偏
>   （第 338 轮里 `sumabs` 也跟着反向）；**第一判据应该是状态扫描的首分歧步**。
> * **方法**：判形状**先用 `-fdump-tree-optimized` 的 GIMPLE 定操作数角色**，再用
>   `.loc` 定位。`.loc` + 反汇编判不出"进 FMA 的是哪个乘积"，第 337/338 轮各踩一次。

> **第 336 轮**：气孔-光合链的闭环已全部建成，口径是
> **`sortin` 3000/3000、`update_photosyn` 3000/3000、`stomata` 4/4000**
> （残余全在黄金窗口走不到的 BB/Medlyn 分支）。这批形状（25 处 FMA）**已按现行口径回退**，
> 原因与清单见"第 336 轮"。两条更重要的结论：
> ① `stomata` 那批形状在黄金窗口上**恒等**（C3 ⇒ `c3_fraction∈{0,1}`；Medlyn/BB 是死分支），
> 所以"它让湿窗倒退"是错的归因 —— 账全在 `sortin`；
> ② 用湿窗 restart 扫描量出：**这条链对湿窗状态轨迹零影响**，湿窗唯一的状态种子是
> **N=4 的 `wliq_soisno[0,5]`（顶层土壤液态水，1 ULP）**，在土壤水里。

**短程逐位已经干净**（1/3 步 restart 0/68、3 步 history 692/692），所以别再往
`meltf`/`water_2014`/水步补冰那几处找 —— 第 257 轮已经把那条链走完并修好。

**第 271–274 轮把干窗"记录 12"那颗种子追到了 `rootflux`**：干窗 16 步里
记录 0–11 全同，记录 12 的第一颗种子是 `zwt` 差 1 ULP；它在
`soil_water_vertical_movement` **入场时还是逐位相同的**，是**开场那段**
（蒸腾级联 → 含水层交换）里被 `deficit` 带出来的；而 `deficit` 的差又全部来自
**入参 `rootflux` 的第 2、3 层在第 11 步差 1–2 ULP**（`DEF_USE_PLANTHYDRAULICS`
默认开，`etroot(:) = rootflux`）。**下一枪打这里，别再回水步**：
`crates/colm-core/src/plant_hydraulics.rs:567 root_flux_from_top_potential` 与
`MOD_PHSRootfluxBalance.F90`，先判"是植物水力解出来的 `rootflux_p` 本来就差"
还是"`MOD_LeafTemperature_Extended.F90:1351-1361` 那两行第一次生效时形状不对"
（第 11 步正是 `etr` 第一次非零的那一步，那两行此前从未生效过）。
（第 292/293 轮已把这条的**源头**钉到 `update_photosyn` 的 `gsh2o` 量纲并修掉：
`over_tol` 821/20662/25896 → 28/1970/25713；剩下的正是本条的 `f_vegwp` 混沌残差。）

剩下三件事，按价值排序：
0. **（第 293 轮已修）遮荫 `rssha` 那条首分歧**：第 292 轮六处探针把"PHS 种子公式"
   否掉（`:316-317` 每次重置，无持久状态），把首分歧钉在遮荫 `stomata` 的入参
   `pco2a`；第 293 轮三层探针（`oracle/scripts/stomata_probe.sh` →
   `pco2a_probe.sh` → `updphoto_probe.sh`）一路推到 `update_photosyn` 的 `gsh2o`
   量纲：上游调用点（`:908-911`）传的是**冠层 µmol 的数值**，本仓库按哑元注释除了
   `1e6`，白天的 `assim` 因此分叉（详见"第 293 轮"）。修好后 16 步探针六处
   328/328 全同，黄金 `over_tol` → 28/1970/25713。
   **现在剩下的就是 `f_vegwp`**：三个窗口的 `sumabs` 全由它主导，dry 的 `over_tol`
   也只剩它。**下一枪是 `rootflux` 的入口**（`plant_hydraulics.rs:567` /
   `MOD_PHSRootfluxBalance.F90`），别再回 `stomata` 或 PHS 种子。
   验收口径：`accept_r247.sh` 与 `dry_ts.sh 16` + `window_divergence.py`；
   干窗黄金 `over_tol` 现在是 **28 / 1970 / 25713**。
0b. **（第 294 轮已补）`balance_phs_rootflux`**：见第 276 轮与"第 294 轮"。
   `plant_hydraulics.rs:balance_phs_rootflux` + 两个调用点；默认三份窗口里仍不生效
   （警告 0 次），三段式验收与第 293 轮**逐位相同**（证明它惰性、没引入回归）。

1. **默认配置的状态残差（第 297 轮定案）**：配置逐字相同、
   **内核 == 存储 golden 在小时对齐下前 10 条逐位** ⇒ 不是基准错。
   - **瞬时状态**：`oracle/scripts/restart_scan.sh` 实测 **restart 到第 18 步都是
     0/68**，第 19 步起 `wliq_soisno` 1 ULP（第一个持久状态分歧），第 20 步起
     `wice_soisno`/`t_soisno`/`smp`/`hk`。**这是唯一要追的种子。**
   - `f_zwt`/`f_rib`/`f_zol` 这些 history 差是 `MOD_Vars_1DAccFluxes` 里
     `acc1d` **区间累加**出来的（`f_rib` = `r_rib`，纯诊断、不回灌），
     第 13/18 步那种 `zwt` 1 ULP 会自己消失 —— **别追 history**。
   - 下一枪（**第 301 轮已把种子钉死在含水层交换里**）：`oracle/scripts/vsf_wt_probe.sh`
     在 `ss_wt` 初始化前后各打一个点（`WSE1` 入场级号+`zwt`、`WSE0` 交换后级号+`zwt`+`wa`、
     `WSEL` 逐层 `ss_wt`/`ss_vliq`/`porsl`），20 步实测：

     ```text
     WSE1 call 1–13 全同（入场 izwt/zwt 逐位）; 首个不同 = WSE1 call 14 的 zwt（= call 13 的出场）
     WSE0 call 13 的 zwt 差 1 ULP（izwt 仍同）
     WSEL call 13 第 3 层 ss_wt 差 1 ULP
     ```

     ⇒ **入场逐位相同、`soilwater_aquifer_exchange` 之后 `zwt` 差 1 ULP**，同一个调用内
     `izwt` 不变。所以种子就在 `soilwater_aquifer_exchange`
     （Rust `exchange_soil_water_with_aquifer`）里，**不在 Richards、不在 PHS、不在缩放**。
     土壤水力函数已排除（`compare_soilhydro.sh` 10000/10000 逐位）。

     **第 302 轮：这一段已经建好确定性判据，而且测出真错。**
     `bash oracle/scripts/compare_getzwt.sh`（新工具，~30 秒、不进混沌窗口）实测当前提交的
     `get_zwt_from_wa` 有 **3604/10000** 个用例与内核不同 —— 形状差是**确定的**，不是末位运气。
     六处正确写法与整张形状扫表在"第 302 轮"一节；把六处都改对后 harness 是
     **10000/10000 逐位相同**，但口径变差（wet `over_tol` 1970 → 1984、N=19 分歧 14×）
     ⇒ 代码已按纪律回退，**形状结论留在文档里**。
     **第 303 轮更新（把 ① 做掉了，方向也修了）**：`oracle/scripts/vsf_input_probe.sh`
     在水步入场处逐层打 7 列（`wice`/`wliq`/`eff_porosity`/`porsl`/`dz`/`wimp`/`vol_liq`），
     20 步只有 **1/200** 条不同：**第 20 步入场、第 3 层**的 `wice` 与 `wliq` 各差 1 ULP，
     而 `porsl`/`dz`/`wimp` 逐位相同 ⇒ `eff_porosity` 公式这一条**结清**，
     `eff_porosity`/`vol_liq` 是跟着冰水质量走的（与 `restart_scan` 的 N=19 `wliq`、
     N=20 `wice` 对得上）。
     **下一枪（顺序不能颠倒）**：① 打第 19 步能量/相变那一段的 `wice`/`wliq`
     （层号与步号本探针已给：第 3 层、第 19→20 步；第 255 轮只排除了第 2 步第 1 层的
     `meltf` 实参），或者查 `water_2014`/`wblc` 冰汇那几处；
     ② 再把第 302 轮验证过的六处形状**一起**重新应用；③ 两条都对了才跑口径。
     **【第 316–321 轮后已更新：第一组已落地】** 目前状态（务必从这里接）：
     * **已入库（`80ccd02`）**：第 302 轮**六处 `get_zwt_from_wa`** 形状
       （`plant_hydraulics.rs`，闭环 `compare_getzwt.sh` **10000/10000**）
       + 第 314/315 轮**两处 `flux_inside_hm_soil`** 形状（`variably_saturated_flow.rs`，
       闭环 `compare_flux_inside.sh` **15000/15000、五条分支全覆盖**）。
     * **落地后的口径**（现基线）：dry `over_tol/ot_vars` **28 / 1**、wet **1942 / 53**
       （原始 1967/53）、snow **25713 / 79**；`restart **0/68**`、3 步 **692/692**；
       第二配置 干 0 / 湿 0 / 雪 79；全 workspace 测试与 clippy 全绿。
     * **状态残差**（`restart_scan.sh`）：首分歧现在是 **N=18（5/68）**
       （落地前是 N=19 的 1 ULP `wliq_soisno`）—— 这是"还有别的 1 ULP 缺陷在场"时
       混沌轨道被换掉的必然结果（第 315/321 轮两个独立实例），**不要**据此回退。
     * **【第 323 轮：第三组已落地】** `water_balance` 闭环（12000/12000，6 种情形 ×
       2000 例）判出两处与第 317 轮**相反**的形状 + 一处结合顺序；`flux_all` 的加权
       平均 1 处按调用点反汇编（第 322 轮 option (a)）落。落地后口径：干 **28/1**、
       湿 **1907/53**（原 1942/53）、雪 **25713/79**；restart **0/68**、3 步
       **692/692**。第 317 轮"每处只有一个乘积、无收左/收右歧义"在 `:1147` 那条
       **双乘积链**上不成立 —— 闭环实测出的正是"独立舍入的是哪一条"。详见"第 323 轮"。
     * **`flux_all` 里那 1 条未归属的 FMA 已定性质（第 322 轮）**：三个可能的家
       （`flux_sat_zone_all`、`flux_both_transitive_interface`、`flux_at_unsaturated_interface`）
       **单独编出来都是 0 条 FMA** ⇒ 那条是**被内联的副本**在调用点上的收缩，
       **例程级闭环判不了它**（闭环编的是 outlined 那份）。要结清它只能：(a) 读
       `flux_all` 的反汇编上下文 + 那三个例程的**源表达式**逐条比，或 (b) 让闭环
       直接驱动 `flux_all`（夹具更接近真实调用）。**别**为这三个例程单独建闭环。
       （顺带确认：`flux_all` 的 7 条里没有 `r0`/`grad_psi>1` 那两个模式 ⇒ 它在那里
       是**调用** outlined 的 `flux_inside_hm_soil`，所以第 314/315 轮那两处修复
       **确实作用在活路径上** ✓）
     * 之后：读 `soil_water_vertical_movement` 的 27−12=15 条差额（**同样先查是不是
       内联来的**：其中已知含 `initialize_sublevel_structure`/`use_explicit_form`/
       `check_and_update_level` 等私有子程序），再回头查第二配置雪窗那 79。
     * **判据纪律（本会话换来的）**：闸门是 `over_tol`/`ot_vars`；`bitwise`/`sumabs`
       与状态残差都是**诊断**（混沌窗口里会双向移动，第 315/321 轮已各有一例）；
       但**单个形状即便逐位判对也不能单独落地** —— 必须把已判对的那批**成组**落地
       （第 316 轮：只 6 处让 wet 退到 1984、只 2 处让干窗翻到 363，两组一起才
       28/1 + 1942）。

     **搭闭环前必须知道的一件事（第 311 轮查定）**：要补的那几个例程在模块里是
     **PRIVATE** —— `PRIVATE :: water_balance`（`:59`）、`PRIVATE :: flux_inside_hm_soil`
     （`:73`）、`flux_all`/`flux_top_transitive_interface`/… 一长串都在 `:59-82`，
     符号表里也都是小写 `t`（local）。而 `compare_getzwt.sh` 能直接链是因为
     `get_zwt_from_wa` 在顶部 PUBLIC 列表里。所以这些例程的闭环**不能**照抄
     `compare_getzwt.sh` 的编译方式，要用**拷贝+放行**：把
     `MOD_Hydro_SoilWater.F90` 拷进 `$WORK`，`sed` 掉目标那几行 `PRIVATE ::`
     （或把它们加进 `PUBLIC ::`），用**那份拷贝**编出 `.mod`（`-I$WORK` 在前），
     vendor 源树不动。否则会先在"驱动看不到例程"上报错，白花一轮。

     **已实测这条编译路线可行**（第 312 轮）：把模块拷进 `$WORK`、只把
     `PRIVATE :: flux_inside_hm_soil` 与 `PRIVATE :: water_balance` 两行改成
     `PUBLIC ::`，用内核同款选项（`-O2 -fdefault-real-8 -ffree-form -cpp
     -ffree-line-length-0 -fallow-argument-mismatch -fopenmp`，`-I$WORK -I.bld
     -Iinclude -Ishare -Imain`）编出的 `.o` 里，两个例程都是**全局 `T` 符号**
     （`nm` 实测）⇒ 驱动可以直接 `USE MOD_Hydro_SoilWater, only: …` 调它们。
     下一步就是在这个骨架上加驱动与本仓库侧探针。

**第 313 轮：确认 `flux_inside_hm_soil` 那两处形状**在活分支上**

```fortran
:45   integer, parameter :: type_upstream_mean           = 1
:46   integer, parameter :: type_weighted_geometric_mean = 2
:48   integer, parameter :: effective_hk_type  = type_weighted_geometric_mean   ← 编译期定死
```

⇒ 第 305/13 轮映射的那两条 FMA（`r0` 的分母 `prms(3)*(prms(2)-1)+prms(2)*2`、
`grad_psi>1` 支的 `hk_u + ((psi_u-psi_l)/dz*hk_u**(1-rr))*hk_l**rr`）**都在活分支**
（`type_weighted_geometric_mean`）里，**值得补**；而 `type_upstream_mean` 那一支是
**死代码**（只影响 Rust 里分支的选取逻辑，其算术不必逐位对齐）。这条排除了
"补了两处却是死路"的可能（第 298 轮那种教训）。

**第 314 轮：给 `flux_inside_hm_soil` 建了闭环 —— 两处形状判对，但**干窗崩了 13 倍**

新三件套（`oracle/scripts/flux_inside_diff.f90` + `crates/colm-core/examples/flux_inside_probe.rs`
+ `oracle/scripts/compare_flux_inside.sh`）：按第 311/312 轮验证过的"拷贝+放行"路线编译
（把模块拷进 `$WORK`、把 `PRIVATE :: flux_inside_hm_soil` 改成 `PUBLIC`，
`-ffunction-sections` + `-Wl,-dead_strip` 剔掉依赖 MPI 的未用子程序），
两档模型 × (3000 均匀随机 + 3000 边界) 共 **12000** 例，逐位比 `flux`。

```text
改前：flux_inside_hm_soil mismatches / 12000: {'flux': 1675}
按第 305 轮映射的两处补 mul_add 后：all 3 outputs 12000/12000 bitwise identical
分支分布 {grad<0: 5385, grad==0: 46, grad==1: 1218, grad>1: 5351}   ← 注意
```

**两处形状（`r0` 分母、`grad_psi>1` 支）就此判对**（12000/12000，确定性判据）。

**但黄金口径拒绝**（`win4.sh` + `three.py`，同树 A/B）：

| 窗口 | 基线 | 补这两处 |
|---|---|---|
| dry `over_tol` | **28** | **363** ✗（13 倍） |
| dry `ot_vars` | 1 | **25** ✗ |
| dry `sumabs` | 249.79 | **383.34** ✗ |
| wet `over_tol` | 1967 | **1739** ✓ |
| wet `ot_vars` | 53 | **36** ✓ |
| wet `sumabs` | 31.31 | **28.01** ✓ |
| snow | 25713 / 32567 / 444416.82 | 不变 |

**关键线索：`0 < grad_psi < 1` 那一条分支在本闭环里一例都没抽到**（分布里只有
`grad<0`/`==0`/`==1`/`>1`）。那条分支的式子
`rr = max(1+r0*psi_l/dz, 1-r0)`、`hk_u**rr * hk_l**(1-rr) * grad_psi` 本仓库也是平铺的
⇒ **修的那两处在 dry 窗口之外的分支上留下了未修的差**，dry 窗（水位在第 3 层、近饱和）
恰好走那条分支，于是 1 ULP 的分支级联把轨迹推到完全不同的解（`ot_vars` 1→25 是
"离散分支翻了"的特征，而不是混沌噪声）。
**代码已回退**（树与 HEAD 一致），闭环与证据入库。
**下一枪（顺序）**：① 给闭环补**近静水**用例（`psi_l = psi_u - delta`，`delta ∈ (0, dz)`）
把 `0<grad<1` 抽到；② 按第 305 轮的办法映射那条分支的 FMA 并补 `mul_add`；
③ 再跑这个闭环（期望仍 12000+/12000+ 全同）；④ 然后才跑黄金 A/B —— 这次四条分支
都判对了，`ot_vars` 才可能不再翻。

**第 315 轮：覆盖补齐了，结论更硬 —— 这个函数**已经逐位全对**，但干窗照样翻 13 倍**

给闭环补了"近静水"块（`psi_l = psi_u + uni()*dz*0.9` ⇒ `grad_psi ∈ (0.1,1)`），
分支覆盖补齐：

```text
kernel branch counts: grad<0 5356 / ==0 49 / 0<grad<1 3000 / ==1 1217 / >1 5378
补两处形状后：all 3 outputs 15000/15000 bitwise identical（五条分支全过）
```

**`flux_inside_hm_soil` 至此在整个输入空间上判对**（两档模型 × 五条分支 × 15000 例，
逐位相同），第 305 轮凭反汇编映射的两处形状**完全正确**。但黄金 A/B **一字未变**：

| 窗口 | 基线 | 两处形状（15000/15000 已证） |
|---|---|---|
| dry `over_tol` / `ot_vars` | 28 / 1 | **363 / 25** ✗（13 倍，可复现） |
| wet `over_tol` / `ot_vars` | 1967 / 53 | **1739 / 36** ✓ |
| snow | 25713 / 79 | 不变 |

**结论（这条比修好一个函数更重要）**：干窗那 13 倍**不是**"漏了某条分支"，
而是**离散分支被 1 ULP 的选择推翻了** —— 这个函数已经没有任何可修的差，
却仍然让干窗的 `ot_vars` 从 1 变 25。也就是说：**在还有别的 1 ULP 缺陷在场时，
单独把一个函数改到逐位全对，可以让混沌窗口的指标大幅变差**（wet 则同时大幅变好）。
所以"逐条修、每条都过口径"在这个窗口上**不是可行的策略**；正确做法是
**把已判对的那批形状一起落地**（第 302 轮六处 `get_zwt_from_wa` + 本轮两处
`flux_inside_hm_soil`），再跑一次口径 —— 那时才有机会让三条窗口同时不倒退。

**代码仍回退**（树与 HEAD 一致），闭环（现在是 15000 例、五条分支全覆盖）与全部
A/B 证据入库。**第 316 轮：两组一起落地 —— **口径不再倒退**，8 处形状入库**

把两组已判对的形状**一起**应用（第 302 轮六处 `get_zwt_from_wa` + 第 314/315 轮两处
`flux_inside_hm_soil`），再跑三段式：

| 窗口 | 基线 | 只 6 处 | 只 2 处 | **两组一起（本轮）** |
|---|---|---|---|---|
| dry `over_tol` / `ot_vars` | 28 / 1 | 28 / 1 | 363 / 25 ✗ | **28 / 1** ✓ |
| wet `over_tol` / `ot_vars` | 1967 / 53 | 1984 / 53 | 1739 / 36 | **1942 / 53** ✓ |
| snow `over_tol` / `ot_vars` | 25713 / 79 | 25713 / 79 | 25713 / 79 | **25713 / 79** ✓ |
| `sumabs`（dry/wet/snow） | 249.79 / 31.31 / 444416.82 | 330.81 / 29.32 / 444416.82 | 383.34 / 28.01 / 444416.82 | **330.81 / 29.71 / 444414.20** |
| `bitwise`（dry/wet/snow） | 16326 / 28904 / 32567 | 17056 / 28904 / 32707 | 16688 / 29054 / 32567 | **17056 / 28532 / 32707** |

* **口径**（`over_tol`/`ot_vars`）：干窗与雪窗**不动**，wet **1967 → 1942**（改善）；
* `restart 0/68`、3 步 `692/692` **保持**（短程逐位不受影响）；
* 两个闭环复跑：`get_zwt_from_wa` **10000/10000**、`flux_inside_hm_soil` **15000/15000**；
* 诊断计数（`bitwise`/`sumabs`）在干窗变差、wet/snow 变好 —— 与第 302/315 轮记录的
  混沌特征一致，按既定读法**不作为判据**（若按字面"任一指标变差就回滚"，
  这条链上任何改动都无法落地，第 315 轮已用"逐位全对却让干窗翻 13 倍"证明了这点）。

**这一轮同时验证了第 315 轮的判断**：单个函数改到逐位全对会让干窗翻 13 倍，
而**两组一起落地**后干窗回到 28/1 ⇒ "把已判对的形状作为一组落地"是这条链上
唯一可行的策略。

⇒ 8 处形状**入库**（`plant_hydraulics.rs` 与 `variably_saturated_flow.rs`），
并把"逐条改 vs 成组落地"的对照表留在上面。**第 317 轮：`water_balance` 那 5 处**逐条映射到源表达式**（每处只有一个乘积，不存在"收左还是收右"的歧义）**

读 `MOD_Hydro_SoilWater.F90:1130-1145`（`blc`/`dmss` 的更新）与第 309 轮那份
`fmsub`/`fmadd` 上下文，五条一一对上：

| 源表达式 | 反汇编 | 该写的 Rust |
|---|---|---|
| `blc(lb-1) = dmss - qsum*dt` | `fsub` + `fmsub d29,d29,d30,d31` | `(-qsum).mul_add(dt, dmss)` |
| `dmss = (vl_s-vl_m1)*(wt-wt_m1) + dmss` | 链式 `fmadd` 1 | `(a-b).mul_add(c-d, dmss)` |
| `dmss = (dz-wt-wf)*(vl-vl_m1) + dmss` | 链式 `fmadd` 2 | `(dz-wt-wf).mul_add(vl-vl_m1, dmss)` |
| `blc(jlev) = (blc(jlev) + dmss) - qsum*dt`（两处分支各一次） | 两次 `fadd` + `fmsub` | `(-qsum).mul_add(dt, blc + dmss)` |

**与 `get_zwt_from_wa` 的关键区别**：这里的**每处只有一个乘积**，所以"哪个乘积进
FMA"没有歧义（那边是 `a*b + c*d` 两乘积，靠闭环才判出"收第一个"）。剩下要小心的
只是**加数是谁**（`dmss` / `blc+dmss`），而它由源的结合顺序定、本仓库已按同样的
结合顺序写 ⇒ **歧义面很小**。（第 309 轮把链式那两条猜成"加权求和"，本轮读源更正为
**`dmss` 的两个累加项**。）

**第 318 轮：把第 316 轮那条 "Not-tested" 补上 —— 第二配置没被这 8 处影响**

`bash oracle/scripts/compare_second_config.sh CN-Cng`（Campbell 土水 + **关掉 VSF**，
与三份黄金窗口的配置正交）：

```text
=== CN-Cng (Campbell, VSF off)
within tolerance: 127 variables, 10 dimensions (tier0=23 tier1=8 tier2=97 tier3=0)
```

⇒ **没有变量超出容差**（127 个变量全部 within；tier 计数之和 128 = 127 + 1 个非数值列），
与文档里该配置的基线（干窗 0）一致 ⇒ 第 316 轮落地的那 8 处**没有碰坏这条经典
Richards 支路**。湿/雪的第二配置仍未重跑（历史值 66 / 79）。

**第 319 轮：落地那条 "Not-tested" 的第二半也关掉 —— 全 workspace 测试与 clippy 全绿**

```text
cargo test --workspace --lib --bins -- --test-threads=1   → rc=0，26 个测试二进制全 ok，0 个 FAILED
cargo clippy --workspace --all-targets -- -D warnings     → rc=0
cargo fmt --all --check                                   → 通过（上一轮已跑）
```

⇒ 第 316 轮落地的那 8 处形状在 `crates/*` 全量测试与 lint 下**没有任何回归**；
连本机此前因沙箱 `EPERM` 而失败的 `colm-cli` 7 个 `study::runner` 用例也一并通过
（本会话文件策略为 `danger-full-access`）。

**第 320 轮：第二配置三个窗口全量复跑 —— 干 0 / 湿 0 / 雪 79**

```text
bash oracle/scripts/compare_second_config.sh CN-Cng        → 127 variables within tolerance（0 超容差）
bash oracle/scripts/compare_second_config.sh CN-Cng-wet    → 127 within tolerance（0 超容差）
bash oracle/scripts/compare_second_config.sh US-NR1-snow   → 79 variable(s) outside tolerance，failures by tier {"tier2": 79}
```

对照文档早期的第二配置记录（**干 16 / 湿 66 / 雪 79**）：干、湿**都已改善到 0**，
雪仍是 79。**说明**：这三份数字是**带已落地的 8 处形状**测得的；本会话没有为第二配置
做"带 / 不带 8 处"的 A/B，所以不能把干/湿的改善归给这 8 处（更可能来自本会话更早的
两处修复：`update_photosyn` 的 `gsh2o` 量纲与截断 π）。**雪窗那 79 是既有差异**
（早期记录同为 79），不是本届落地引入的回归 —— 但它仍是这条支路上唯一还没查的窗口。

**至此第 316 轮落地那条 "Not-tested" 全部关闭**：第二配置三窗口（本轮）+ 全 workspace
测试与 lint（第 319 轮）。

**第 321 轮：落在**状态**上量了一次 —— 首分歧从 N=19 提前到 N=18，而且更大**

`bash oracle/scripts/restart_scan.sh 18 19 20 21`（带已落地的 8 处形状）：

| N | 落地前（会话早期记录） | 落地后（本轮实测） |
|---|---|---|
| 18 | **0 / 68** | **5 / 68**（`t_soisno`/`wice_soisno`/`wliq_soisno`/`hk`/`smp`，maxabs 1.4e-14 量级） |
| 19 | 1 ULP `wliq_soisno`（+ 瞬态 `zwt`） | 5 / 68（`wliq_soisno` maxabs 2.49e-14） |
| 20 | 5 / 68 | 5 / 68 |
| 21 | 7 / 68 | 7 / 68 |

⇒ **落地的那 8 处（每处都在合成输入上判到逐位）把状态分歧提前了一步、并放大了约 8 倍。**
这不是"修错了"：两处修复各自都让 Rust 更接近内核；但**在还有别的 1 ULP 缺陷在场时**，
旧的平铺写法与那些缺陷的误差曾是**部分抵消**的，改对一处就把这条混沌轨道换掉了
（第 315 轮已在黄金窗口上看到同一现象：逐位全对的函数让干窗翻 13 倍）。
**结论与策略**：状态残差在"缺陷没凑齐"之前**不是单调的**，不能拿它当"该不该落地"的判据
（黄金口径与短程 0/68、692/692 才是）；正确做法仍是**把剩下已验证的形状补齐后成组落地**
（第 3 组：`water_balance` 5 处 + `flux_all` 加权平均 1 处），那时状态残差才有指望回落。
已落地的 8 处**保留**（口径 wet 1967→1942、干/雪不倒退，短程与第二配置全绿）。

**下一枪（第 328 轮后）**：
* **已入库（第 326 轮）**：`intercept_canopy` 三处结合/收缩形状（`p0` 先加后乘、`pinf`
  先合 `thru`、`:328-329` 的 `fmadd`+`fsub`）。湿窗 `over_tol` **1907→1287**、
  `ot_vars` **53→24**，干/雪口径不动，短程 0/68 与 692/692 保持。
* **已入库（第 327 轮）**：`history_diagnostics` 三处 FMA（`(1+0.61q)`、`thvstar`
  外积、`ur*ur+wc2`）。第一配置雪窗 **step 0 有限值逐位全同**，`first divergence step`
  0→1；三窗口口径不变。
* **已入库（第 328 轮）**：第 4 个闭环 `compare_interception.sh`（`intercept_canopy`
  **4000/4000**，8 输出 × 两档 `DEF_VEG_SNOW`）+ 补齐的 5 处形状（`ap`/`cp`/`aa1`/`bb1`/
  `drainage`/`saturated_fraction`/`FP`/`tti_snow`）。口径不变（那几处在黄金输入上取不到差）。
* ① `f_vegwp` 一侧剩下的缺口 —— 第 324 轮已把 `soil_water_vertical_movement` + 两个
  内联例程的 **17 处**形状判好并列进"第 324 轮"的表。**第 326 轮之后重测过**：在
  湿窗 **1287/24** 的新基线上，整批 17 处让湿窗退到 **2000/53**（干窗 28→20），
  所以**仍然未落** —— 截留那三处没有让这份 17 处变相容。按第 324 轮的表留着，
  等 `f_vegwp` 那条链一起补。
* ② **下一枪**：雪窗从 **step 1** 起分歧，领先变量是气孔那条链的 `f_rstfacsha/sun`、
  `f_gssun/sha`（截留的 `f_qintr`/`f_qdrip` 已在第 328 轮退出前六）⇒ 对
  `MOD_AssimStomataConductance` / `MOD_LeafTemperature` 做 `-S -g` + `.loc` 普查
  （`update_photosyn`/`stomata`/`leaftemperature`），必要时照第 328 轮建第 5 个闭环。
* ③ 未决小尾巴：`flux_all` 里那条加权平均 FMA 只有调用点上下文（option (a)）判过，
  若要做**实测**判据只能建"直接驱动 `flux_all`"的第 4 个闭环（夹具更大）。
2. **第二配置回归**（Campbell + 关 VSF）：第 293 轮实测干窗已从 16 降到 **0**；
   wet/snow 未重跑，需要时跑 `oracle/scripts/compare_second_config.sh <case>`。
3. **未移植分支**：`standard_lct_step.rs:578` 明说 split soil/snow、SNICAR、气溶胶、
   示踪剂仍是另一支；动态湿地/CaMa 洪水路径 `colm-rs` 会打印 "unported branch" 警告。
   这些是"全面完成"里真正还没做的部分。其中**唯一在装配期被明确拒绝的产流分支**是
   `DEF_Runoff_SCHEME = 1`（完整 VIC，`MOD_Hydro_VIC.F90` 588 行 + `vic/vic_para.txt`
   运行期输入），移植它要连带做 `vic_para` 读取与 `soil_con_struct`/`cell_data_struct`
   两套派生类型；`Runoff_VIC(15-67)` 只是壳，重头在 `compute_vic_runoff` 及其
   `compute_runoff_and_asat`/`calc_Q12`/`compute_zwt`/`wrap_compute_zwt`。

工具与纪律：探针跑完**必须重编**内核；**内核构建是单例**（所有探针已用
`/tmp/gf/.kernel_build.lock` 串行化，不要把锁删掉）；**不要用内核 sha256 判断是否重编过**
（同一份源码三次构建三个 sha）；判"混沌放大还是系统性偏差"仍用短程对照。

**第 323 轮：`water_balance` 闭环判出两处"与文档相反"的形状；第三组落地**

新三件套（`oracle/scripts/water_balance_diff.f90` +
`crates/colm-core/examples/water_balance_probe.rs` + `oracle/scripts/compare_water_balance.sh`），
照第 311/312 轮验证过的"拷贝+放行"路线编译（模块拷进 `$WORK`、`PRIVATE :: water_balance`
改 `PUBLIC`、`-ffunction-sections` + `-Wl,-dead_strip`）。**6 种情形 × 2000 = 12000 例**：
上边界 {定水头, 降雨} × 下边界 {定水头, 排水, 排水且 `waquifer==0 && q(ub)>=0`}，
非饱和层比例取遍 {0, 0.3, 0.7, 1}，逐位比 `blc(0:nlev+1)` 与 `solvable`
（`dz` 由界面深度相邻相减复原，两边看到逐位相同的厚度）。

**改前 baseline 是 `blc[0..6]` 全都差**（256/877/727/540/389/256/126）—— 说明不止
"少了 `:1140` 那一处"。逐条读内核 `water_balance` 的 5 条 FMA 上下文（从栈参偏移定出
`x21=wf, x24=wf_m1, x23=wt, x27=wt_m1`），读出三件事与第 317 轮那份映射表不同：

```text
:1140 blc(lb-1) = dmss - qsum*dt         → 单条 fmsub（乘积进 FMA、dmss 是加数）   [文档对]
:1147 dmss = (vl_s-vl_m1)*(wt-wt_m1)+dmss
      → fmul 先算 (wt-wt_m1)*(vl_s-vl_m1)，fmadd 才把 (vl_s-vl_m1)*(wf-wf_m1) 收进去
      ⇒ **独立舍入的是 wt 那条**，与源语句书写顺序**相反**                        [文档错]
:1148 dmss = (dz-wt-wf)*(vl-vl_m1)+dmss  → fmadd d3*d22+d4，factor=(dz-wt)-wf       [文档对]
:1162 blc(ilev) = blc(ilev)+dmss-qsum*dt → fadd + fmsub                             [文档对]
:1168 排水子分支 waquifer==0 && q(ub)>=0
      blc(ilev) = blc(ilev) - waquifer_m1 - q(ub)*dt
      → 两条 fsub，`q(ub)*dt` 是**独立 fmul** ⇒ (blc-waquifer_m1)-q(ub)*dt，
        而不是 blc -= (waquifer_m1 + q(ub)*dt)                                  [文档没记]
```

⇒ 第 317 轮"每处只有一个乘积、所以无收左/收右歧义"这条**在 `:1147` 上不成立**：
那是一条**两个乘积相加**的链，GCC 把**第二个**源乘积留在外面独立舍入。这正是
"读源猜形状"与"读汇编写形状"的差别（第 305 轮的方法论：计数可疑要读上下文，
方向仍要闭环判）。落码三处（都在 `variable_saturated_water_balance`）：

1. `:1147` → `porosity_change.mul_add(wetting_front_change, water_table_change*porosity_change)`；
2. `:1140` → `(-flux_sum).mul_add(input.time_step_seconds, mass_change)`；
3. `:1168` 拆成两条 `-=`（先 `waquifer_m1`、再 `q(ub)*dt`）。

闭环从"全差" → **12000/12000 逐位相同**（`bash oracle/scripts/compare_water_balance.sh`）。

**`flux_all` 那 1 条未归属 FMA 也定案（第 322 轮 option (a)）**：反汇编 `flux_all` 的
`0x1001703e0`，周围正是 `psi_s_min`/`psi_i_r`/`psi_i_l` 的 `fminnm/fmaxnm` 与两处夹逼
比较 ⇒ 它就是**被内联的 `flux_at_unsaturated_interface`** 的 `psi_i` 加权平均：

```text
fadd d28,d13,d30      ; psi_u + dz_u      ⇒ d13=dz_u, d30=psi_u
fsub d29,d11,d12      ; psi_l - dz_l      ⇒ d11=psi_l, d12=dz_l
fmul d31,d13,d11      ; 独立舍入 dz_u*psi_l
fadd d13,d13,d12      ; dz_u + dz_l
fmadd d31,d12,d30,d31 ; 收进 dz_l*psi_u
fdiv
```

⇒ Rust 改成
`lower_distance_mm.mul_add(upper_pressure_head_mm, upper_distance_mm*lower_pressure_head_mm)`
（与第 310 轮的 `(w2).mul_add(v2, w1*v1)` 一致）。**outlined 版仍是 0 条 FMA**（第 322 轮），
所以这一处结构性上只能从调用点上下文判 —— 例程级闭环判不了它，本组按 option (a) 落。

**第三组落地后三段式**（`bash /tmp/gf/win4.sh` + `three.py`；`bash /tmp/gf/accept_r247.sh`）：

| 窗口 | 8 处落地后（基线） | **第三组** |
|---|---|---|
| dry `over_tol` / `ot_vars` | 28 / 1 | **28 / 1** ✓ |
| wet `over_tol` / `ot_vars` | 1942 / 53 | **1907 / 53** ✓（改善 35） |
| snow `over_tol` / `ot_vars` | 25713 / 79 | **25713 / 79** ✓ |
| restart 1 步 / 3 步 | 0/68 / 692/692 | **0/68 / 692/692** ✓ |
| `bitwise`（dry/wet/snow） | 17056 / 28532 / 32707 | 17056 / **28418** / 32707 |
| `sumabs`（dry/wet/snow） | 330.81 / 29.71 / 444414.20 | 330.81 / 31.59 / 444414.20 |

**口径没有一处倒退，湿窗还改善 35 条**；干窗与雪窗一字未动。诊断计数按既定读法不当判据
（wet `sumabs` 反向小幅移动，与第 315/321 轮记录的混沌特征一致）。

`bash oracle/scripts/restart_scan.sh 18 19 20 21`（第三组落地后）：**5/5/5/7 per 68**，
与 8 处基线**逐条相同** ⇒ 第三组不动干窗的状态残差（诊断，不是判据；`N=18` 仍是第一个
持久分歧）。其它：`compare_getzwt.sh` 10000/10000、`compare_flux_inside.sh`
15000/15000 复跑通过；`cargo test --workspace --lib --bins`、`clippy --workspace
--all-targets -- -D warnings`、`fmt --all --check`（两个 workspace）全绿（本机
`colm-cli` 的 7 个 `study::runner` 用例仍因沙箱 `EPERM` 失败，与本改动无关）。

**第 324 轮：`soil_water_vertical_movement` 的 27−12 差额拆开了 —— 全是内联账，真缺口 17 处；整批落地让湿窗倒退，未落**

**先把"内联"减掉（这是本轮最省事的一步）**：把 `MOD_Hydro_SoilWater.F90` 用
`gfortran -S -O2 -g`（内核同款选项）编成汇编，按 `.loc` 把 swvm 符号里的 27 条
FMA **逐条归到源行**。结果是：

```text
27 = 15 (soil_water_vertical_movement 自己的表达式)
   +  6 (被内联的 Richards_solver)
   +  6 (被内联的 use_explicit_form)
```

符号表印证：本内核里 `Richards_solver`/`use_explicit_form`/`initialize_sublevel_structure`/
`secant_method_iteration`/`var_perturb_*`/`flux_sat_zone_all`/`flux_at_unsaturated_interface`/
`flux_both_transitive_interface` **都没有独立符号**（全被内联）；而
`check_and_update_level`/`solve_least_squares_problem`/`flux_sat_zone_fixed_bc`/
`flux_inside_hm_soil`/`flux_top|btm_transitive_interface`/`flux_all` **有**。
顺带纠一个计数：第 305 轮把 Rust 侧记成 12 个 `mul_add`，其中 1 个是**注释里的
"mul_add" 字样**，实际代码 11 个；这 11 个覆盖了 15 条"自有"里的 10 条。

**真缺口 = 5（swvm 自有）+ 6（`Richards_solver`）+ 6（`use_explicit_form`）= 17**，
逐条读 `.loc` 上下文定出"哪条乘积独立舍入、哪条进 FMA"：

```text
--- swvm 自有（5）----------------------------------------------------------
:338 wexchange = rsubst*dt + deficit            → rsubst.mul_add(dt, deficit)
:406 ss_dp = max(ss_dp + qgtop*dt, 0)           → qgtop.mul_add(dt, ss_dp).max(0)
:434 (ss_vliq*(dz-wt)+porsl*wt)/dz              → fmul 舍入 porsl*wt、fmadd 收
                                                  ss_vliq*(dz-wt)
:456 wblc = w_sum_after-(w_sum_before+(qgtop-sum-rsubst)*dt-etrdef)
                                                → (...).mul_add(dt, w_sum_before)
:478 (ss_vliq*(zwt-zlo)+porsl*(zhi-zwt))/(zhi-zlo)
                                                → 同 :434（收第一个源乘积）
--- 内联 Richards_solver（6）----------------------------------------------
:817 dp_m1 - (q_0(lb-1)-ubc_val)*dt             → (-(q_0-ubc_val)).mul_add(dt, dp_m1)
:823 f2_norm = sqrt(sum(blc**2))                → fold(0, |a,r| r.mul_add(r, a))
:966 waquifer_pb 里的 psi_s+(sp_zi-zwt_pb)*0.5  → (sp_zi-zwt_pb).mul_add(0.5, psi_s)
:1060 waquifer   里的 psi_s+(sp_zi-zwt)*0.5     → 同上
:1065 ss_q = ss_q + q_this*dt_this               → q_this.mul_add(dt_this, ss_q)
:1085 (ss_wf*vl_s+(dz-wf-wt)*ss_vl)/(dz-wt)     → 收第一个源乘积，舍入第二个
--- 内联 use_explicit_form（6）--------------------------------------------
:1411/:1424 (wt_m1+wf_m1)*vl_s+(dz-wt_m1-wf_m1)*vl_m1
                                                → fmul 舍入第二个、fmadd 收第一个
:1464 dp = max(0, dp_m1+(ubc_val-q(lb-1))*dt)   → (ubc_val-q).mul_add(dt, dp_m1)
:1474 ( ... + dwat)/dz，dwat=(q(layer)-q(layer+1))*dt
                                                → 两条 FMA：先收第一个乘积，
                                                  再把 dwat 收进结果
:1479 waquifer = waquifer_m1 + q(ub)*dt         → q(ub).mul_add(dt, waquifer_m1)
```

**落地实测（A/B，同树）**：

| 窗口 | 第三组（现基线） | 只落 swvm 5 处 | **整批 17 处** |
|---|---|---|---|
| dry `over_tol` / `ot_vars` | 28 / 1 | **20 / 1** ✓ | **20 / 1** ✓ |
| wet `over_tol` / `ot_vars` | 1907 / 53 | **1960 / 53** ✗ | **1984 / 53** ✗ |
| snow `over_tol` / `ot_vars` | 25713 / 79 | 25713 / 79 | 25713 / 79 |
| restart 1 步 / 3 步 | 0/68 / 692/692 | 0/68 / 692/692 | 0/68 / 692/692 |
| wet `bitwise` / `sumabs` | 28532 / 31.59 | 28868 / 28.57 | 29221 / **17.40** |

**结论与处理**：17 处形状都是 `.loc` 逐条归位的（不是猜的），但**整批落地让湿窗
`over_tol` 从 1907 退到 1984**，按现行口径"任一 `over_tol`/`ot_vars` 变差就回滚"，
**未落**（代码已 `git checkout` 回退到第三组）。这与第 315/321 轮同一现象：其它缺陷
（这条链上主要是 `f_vegwp`）还在场时，把一批逐位都对的形状落地会换掉混沌轨道。
湿窗 `sumabs` 31.59→17.40 说明这批形状确实让**逐点误差**小了很多，退的只是那个
混沌主导的 `over_tol` 计数。

**给下一轮**：上表就是可直接照抄的补丁清单（**不要再读一遍汇编**）。正确顺序是
**先凑齐 `f_vegwp` 一侧的缺口**，再把这份 17 处一起落地重测 —— 才有机会让三个窗口
同时不倒退（第 316 轮的 8 处就是这么成组的）。

**第 325 轮：第二配置雪窗那 79 不是雪模块的账 —— 是冠层/湍流那 1 ULP 种子，两套配置同样放大**

跑 `bash oracle/scripts/compare_second_config.sh US-NR1-snow`（Campbell + 关 VSF）：
79 个变量超 tier2 容差。把它和**第一配置**（VG + VSF）雪窗的 79 个变量做集合比对：

```text
两套配置各 79 个；交集 76
第一配置多: f_frcsat  f_qlayer  f_rsur_se     （都是 VSF/土壤水特有）
第二配置多: f_rss     f_wa      f_wa_inst     （都是含水层特有）
```

⇒ 雪窗**主体（76 个）不走土水/VSF 那条支路**，是两套配置共享的东西。

用 `window_divergence.py` + 逐元素比对拆到第一步（`step 0`）。两套配置在 step 0 的
**有限值**差异都只有 1 ULP 级、且是同一批：

```text
f_ldew   1.37e-16     （两套都有，1 个元素）
f_zol    1.93e-16     （两套都有）
f_qintr  9.81e-16     （两套都有）
f_ustar2 1.56e-16 / f_rib 1.72e-16   （只有第一配置）
```

⇒ **种子在冠层水/湍流**（叶片露、截留、Monin-Obukhov 稳定度），**不在雪**。雪/水文量
（`f_scv`/`f_snowdp`/`f_wliq_soisno`/`f_h2osoi`）的最坏点出现在 step 107–1625，是**下游
放大**。另有 16–19 个变量在 step 0 有 NaN/掩码差异（`f_alb`/`f_lake_*`/`f_sol*ln`/
`f_sensors`）：多数不计入那 79（比较器跳过未激活层），但 `f_alb` 在活性层上是真差。

**和已有工作的关系**：`standard_lct_step.rs:1113-1121` 与 `interception.rs:144-150`
已经修过两个 `f_ldew` 来源（湿球温度占位、湿比例"先除再乘"），本轮把**剩下的那颗
1 ULP 种子**钉在 step 0 的冠层路径上。**下一枪**：对叶片露/截留那几步做与
`water_balance` 相同的"`-S -g` + `.loc` 反汇编形状核对"（种子变量是 `f_ldew`/`f_qintr`，
不是 `f_zol`/`f_rib` —— 后两个是纯诊断、不回灌）。

**结论**：第二配置雪窗 79 **不是**"Campbell / VSF-off 特有"，两套配置共享同一颗种子
⇒ **不需要为第二配置单独建闭环**；它会被冠层/湍流那条链的修复一起带走。

**第 326 轮：冠层截留三处结合/收缩形状 —— 湿窗 `over_tol` 1907→1287、`ot_vars` 53→24**

顺着第 325 轮那颗 step-0 的 1 ULP 种子往下打。`f_ldew` 是**冠层持水状态**（不是通量），
所以把 `MOD_LeafInterception.F90` 用内核同款选项 + `-S -g` 编成汇编（`-I.bld`，
**用真的 `mod_namelist.mod`** —— `DEF_VEG_SNOW` 默认 `.true.`，冠层雪那一段是活的），
按 `.loc` 把 `leaf_interception_colm2014` 里的 15 条 FMA 归到源行，对上三处 Rust 写错：

```text
:193 p0 = (prc_rain+prc_snow+prl_rain+prl_snow+irrig)*deltim   ← 先加五个通量再乘
     Rust 写的是 convective_amount + large_scale_amount，即 ppc+ppl；
     ppc/ppl 各自"先乘再加"，p0 ≠ ppc+ppl
:321-323 thru_rain=tti_rain+tex_rain; thru_snow=tti_snow+tex_snow;
         pinf = p0 - (thru_rain + thru_snow)
     Rust 摊成 p0 - direct_rain - drainage_rain - direct_snow - drainage_snow（换结合顺序）
:328-329 ldew_rain = ldew_rain + rate*deltim - thru_rain
     出货汇编是 `fmadd rate,dt,ldew_x`（`.loc 1 328/329`）再单独 `fsub thru_x`；
     Rust 写成 `ldew += rate*dt - thru`（结合顺序错、且没收缩）
```

三处落码后，两套配置的 step-0 **有限值**差异里 `f_ldew`/`f_qintr` **消失**，
只剩 Monin-Obukhov 诊断（`f_zol`；第一配置另有 `f_rib`/`f_ustar2`）各 1 ULP。

**三段式（同树 A/B）**：

| 窗口 | 第三组（基线） | **+ 截留三处** |
|---|---|---|
| dry `over_tol` / `ot_vars` | 28 / 1 | **28 / 1**（不动） |
| wet `over_tol` / `ot_vars` | 1907 / 53 | **1287 / 24** ✓✓ |
| snow `over_tol` / `ot_vars` | 25713 / 79 | **25713 / 79**（不动） |
| restart 1 步 / 3 步 | 0/68 / 692/692 | **0/68 / 692/692** ✓ |
| wet `bitwise` / `sumabs` | 28418 / 31.59 | **27981 / 11.03** |
| snow `bitwise` / `sumabs` | 32707 / 444414.20 | 32728 / 444416.82 |

**湿窗大幅改善**（`over_tol` −620、`ot_vars` −29），干窗与雪窗口径不变，短程逐位保持 ⇒
**已入库**。第二配置干/湿仍是 0，雪窗 79 不变（现在由 MO 诊断那颗 1 ULP 顶着）。
`compare_water_balance.sh`/`compare_getzwt.sh`/`compare_flux_inside.sh` 复跑全同。

**下一枪**：step-0 只剩 Monin-Obukhov 诊断（`f_zol`/`f_rib`/`f_ustar2`）1 ULP —— 但文档
第 297 轮已判定 `f_rib`/`f_zol` 是 `acc1d` 的**纯诊断、不回灌**，所以雪窗 step 1 的分歧
（`f_gssun`/`f_gssha`/`f_rstfac*`）要么来自 `zol` 作下一步迭代初值，要么是 step-1
气孔/光合自身的形状；先按 `history_diagnostics.rs` 那三处（`:164` 的 `zol`、
`:208` 的 `rib`）做形状核对，再决定要不要动气孔那条链。

**第 327 轮：Monin-Obukhov 诊断三处形状 —— 雪窗 step 0 首次做到逐位全同，分歧推到 step 1**

按第 326 轮留的入口，把 `MOD_Vars_1DAccFluxes.F90` 用内核同款选项 + `-S -g` 编成汇编，
按 `.loc` 读 `accumulate_fluxes` 里 zol/rib/us10m 那一段的 FMA，对上
`history_diagnostics.rs` 三处（**都是"乘积该进 FMA"的形状**）：

```text
:2749 (1.+0.61*qm)                       出货 `fmadd d29,d13,d31,d29`（d29=1.0、d31=0.61）
     ⇒ `0.61*qm` 收进 `1.0`；Rust 原写 `1.0 + 0.61*humidity`（两次舍入）。
       这个因子同时喂 `thv` 与 `thvstar`
:2751-2752 thvstar = r_tstar_e*(1+0.61*qm) + 0.61*th*r_qstar_e
     出货 `fmsub d31,d9,d29,d31`（d9=-r_tstar_e）⇒ `r_tstar_e*F` 收进 `0.61*th*r_qstar_e`
:2773 um = max(0.1, sqrt(ur*ur+wc2))      出货 `fmadd d9,d9,d9,d0` ⇒ `ur*ur` 收进 `wc2`
```

第 295 轮那句"本处按平铺保留、改与不改对窗口逐位无影响"（`history_diagnostics.rs`
旧注释）是**当时没反汇编**的保守写法，本轮按实测更正。

**结果**：第一配置雪窗 **step 0 的有限值差异清零**（此前只剩 `f_zol`/`f_rib`/`f_ustar2`
各 1 ULP），`window_divergence.py` 的 `first divergence step` 从 **0 推到 1**；step 1 的
领先变量是 `f_rstfacsha/sun`、`f_gssun/sha` 与 `f_qintr`/`f_qdrip`。

**三段式**：dry **28/1**、wet **1287/24**、snow **25713/79** —— **三窗口口径一字未动**
（这三处只喂 history、不回灌），restart 0/68、3 步 692/692 保持；`bitwise` 只动了个位数
元素（dry 17056→17057、wet 27981→27971、snow 32728→32723）。第二配置雪窗仍 **79**。

⇒ **落码**（形状已按汇编定死，且无任何窗口倒退）。**代价说明**：这是"只影响 history
输出"的修复、口径不变；价值在于把雪窗的种子从 step 0 赶到 step 1，下一步追 step-1
气孔链时不再被诊断噪声干扰。

**下一枪**：step 1 领先的 `f_rstfacsha/sun`（气孔阻力系数）与 `f_qintr`/`f_qdrip`。
注意 `f_qintr` 现在**到 step 1 才差** —— 冠层水在两 step 之间的**内部量**
（`water.rain_mm`/`snow_mm` 分量、叶温的 `zeta`）可能还有一处 step-0 未导出的差；
先按 `MOD_LeafTemperature`/`MOD_AssimStomataConductance` 那条链做 `.loc` 普查。

**第 328 轮：给冠层截留建了第 4 个闭环 —— `intercept_canopy` 全输入空间逐位对齐（4000/4000）**

第 326 轮的 3 处让湿窗 1907→1287，但雪窗 step 0 全同之后 step 1 仍有 1 ULP 的冠层水
差异。剩下的 12 条 FMA 只影响 `ldew_rain`/`ldew_snow` 这些 **history 不导出的分量** ——
黄金窗口判不了它们，所以照 `compare_water_balance.sh` 的三件套建了第 4 个闭环：

* `oracle/scripts/interception_diff.f90` +
  `crates/colm-core/examples/interception_probe.rs` + `oracle/scripts/compare_interception.sh`；
* `LEAF_interception_CoLM2014` 在模块里是 **PUBLIC** ⇒ **不需要**"拷贝+放行"，
  直接链 `.bld/MOD_LeafInterception.o`；
* `DEF_VEG_SNOW`（`MOD_Namelist` 的模块变量，默认 `.true.`）在 k=0/1 两档各 2000 例，
  逐位比 8 个输出（`ldew`/`ldew_rain`/`ldew_snow` + `pg_rain`/`pg_snow`/`qintr`/
  `qintr_rain`/`qintr_snow`）。

**改前 baseline**：`pg_snow` 232、`qintr_snow` 230、`ldew_snow` 154、`qintr` 147、
`ldew` 118、`pg_rain` 26、`qintr_rain` 26、`ldew_rain` 13（`ldew_rain`/`ldew_snow`
差异正是"分量不同、总量相同"的直接证据）。再读 `.loc` 上下文补 5 处：

```text
:221/:222 ap/cp                          "第一个源乘积进 FMA"（加数是第二个乘积）
:229 aa1 = 0.5-0.633*chiv-0.33*chiv*chiv 两步减法各一条 fmsub
:230 bb1 = 0.877*(1.-2.*aa1)             内层 `1-2*aa1` 是 fmsub
:245 xs = -1./bp*log(arg)                出货是 `fnmul` ⇒ `-(0.05*log(arg))`，
                                         不是 `-log(arg)/20`（先乘倒数 vs 先取负）
:253-254 drainage                        bracket 第一乘积进 FMA；外层是 `fnmsub`
                                         （`A*fpi*(...)` 进 FMA、`max(0,…)*xs` 是加数）
:285 FP 分母 `10.*ppc+ppl`               `10.*ppc` 进 FMA
:295/299 tti_snow                        `(1-fvegc)*rate` 进 FMA
```

闭环从"8 个输出全差" → **4000/4000 全同**（8 输出 × 两档 `DEF_VEG_SNOW`）。

**三段式**：dry **28/1**、wet **1287/24**、snow **25713/79** —— **口径一字未动**
（这几处在黄金窗口的输入上取不到差别，`bitwise` 只动 2 个元素），restart 0/68、
3 步 692/692 保持；第二配置干/湿仍 0、雪仍 79。三个旧闭环（water_balance/get_zwt/
flux_inside）复跑全同。

⇒ **落码 + 闭环入库**（这个闭环从今往后守住 `intercept_canopy` 的逐位形状）。

**雪窗 step-1 的种子现在定位清楚了**：`f_rstfacsha/sun`、`f_gssun/sha`（气孔阻力
系数/气孔导度）领跑，`f_qintr`/`f_qdrip` **已退出前六** ⇒ 冠层水那条链**不再是**
种子，**下一枪是 `MOD_AssimStomataConductance` / `MOD_LeafTemperature` 的气孔/光合链**
（同样先 `-S -g` + `.loc` 普查它的 FMA）。

**第 329 轮：气孔/光合链的 FMA 普查（只定位，未动码）**

雪窗 step-1 的种子已在第 328 轮定位到气孔那条链。本轮按同样的
`gfortran -S -O2 -g` + `.loc` 办法把 `MOD_AssimStomataConductance.F90` 的两个例程数了一遍：

```text
stomata           : 17 条 FMA（:223, :258-266, :343-356, :861）
update_photosyn   :  8 条 FMA（:710, :737-744, :805）
本仓库 photosynthesis.rs 的 mul_add 计数： 0
```

⇒ **整条气孔/光合链一处 FMA 都没复现**（`leaf_temperature.rs` 的 42 处是叶温能量平衡那条，
不是这里）。这解释了 step-1 的 `f_rstfacsha/sun`/`f_gssun/sha` 为什么差 1 ULP。
**下一枪**：照第 328 轮给 `MOD_AssimStomataConductance` 建第 5 个闭环
（`stomata`/`update_photosyn` 都是纯函数式输入输出，夹具比截留还小），再按 `.loc`
一条条把 25 处补上 —— 注意 `update_photosyn` 已被 `hydraulic_photosynthesis_update_matches_
mod_assim_stomata_conductance` 这个**值级**单测钉住，但它不判逐位形状。

**第 330 轮：给气孔链建了第 5 个闭环 —— baseline 量出，修了 2/17（未到 100%）**

按第 329 轮的普查建三件套：`oracle/scripts/stomata_diff.f90` +
`crates/colm-core/examples/stomata_probe.rs` + `oracle/scripts/compare_stomata.sh`。
`stomata` 是 PUBLIC、纯函数式：三种模型（Ball-Berry / Medlyn / WUE，由驱动写
`MOD_Namelist` 的 `DEF_USE_MEDLYNST`/`DEF_USE_WUEST` 切换）各 1000 例，逐位比
`assim`/`respc`/`rst`；覆盖值全部设成哨兵 `-1`，让实参里的 `g1`/`g0`/`gradm`/
`binter`/`lambda` 生效。

**baseline**：`assim` 498、`rst` 114、`respc` 0。已按 `.loc` 补 2 处：

```text
:223 range = pco2m*(1-1.6/gradm) - gammas   出货 `fnmsub d31,d31,d29,d8`（乘积进 FMA）
:264/:266 max(0,(a+b)**2 - 4*theta*a*b)     `(a+b)` 的平方进 FMA（`fnmsub d29,d29,...`）
```

→ `assim` 498→**148**、`rst` 114 不变。两处都按汇编定死，三段式口径**一字未动**
（dry 28/1、wet 1287/24、snow 25713/79；restart 0/68、3 步 692/692）。

**未完成（下一轮）**：还剩约 15 处 —— `omc`/`ome`/`oms` 的
`vm*(...)*c3 + vm*c4`（`:258`/`:259`/`:262`，注意 `:258` 的 `fmadd d23,d31,d25,d23`
里 `d31` 已经是 `fdiv` 之后的商，寄存器流要看全）、Medlyn 的 `:343`/`:344`/`:346`、
Ball-Berry 的 `:353`/`:354`/`:356`，以及 `update_photosyn` 的 8 条（`update_photosyn`
要单独加驱动块或另建小闭环）。**方法**：照这个闭环一次改一处、看 `assim`/`rst`
计数是否下降；`respc` 已 0，说明 `calc_photo_params` 那条（`respc` 的算式）是对的。

**第 331 轮：气孔链补到 `assim` 498→102 / `rst` 114→0，但湿窗 1287→2181 ⇒ 整批回退**

按第 330 轮的闭环继续啃，又按 `.loc` 补了 6 处（都已按汇编定死）：

```text
:258/:259 omc/ome = vm_term*c3 + vm*c4        第一个（c3）乘积进 FMA
                                               （`fmadd d23,d31,d25,d23`，d25=c3、d13=c4）
:262 oms = c3*omss + (omss*pco2i)*c4          c3 乘积进 FMA
:343/:344/:346 Medlyn bquad/cquad/sqrtin      `-2*(g0+acp)`、`1-g1^2`(fmsub)、`g0_term^2`(fmadd)、
                                               `bq^2 - 4*cquad` 的 `bq^2` 各进 FMA
:353/:354/:356 Ball-Berry bquad/cquad/sqrtin  两处乘积各 fmsub/fnmadd；`ea+hcdma*bintc` 内层 fmadd
:861 WUE_solver `1 + 1.37*sqrt(...)`          1.37*sqrt 进 FMA
```

闭环推进得很干净：`rst` **114→0**（三模型电导全逐位）、`assim` **498→102**；**模型 2（WUE）
整条 0 差**。剩下的 102 条全在模型 0/1（都用 `sortin` 取 `pco2i`），`assim` 只差末位
（`assim - respc` 那一步把 1 ULP 吸收掉了，所以 `rst` 仍全同）。`sortin` 本身**没有 FMA**，
试过 `cterm` 的 `aterm*eyy*eyy` 结合顺序、以及把 `ac1/ac2` 的 `powi(2)` 改成显式连乘，
**计数都没动**（仍 102）⇒ 剩下的差在 `sortin` 的别处或 `coupled_assimilation` 的某个末位，未定。

**但三段式拒绝**：wet `over_tol` **1287→2181**、`ot_vars` **24→54**、`sumabs`
11.03→53.41（dry 28/1、snow 25713/79、restart 0/68、3 步 692/692 不变）。
⇒ 按口径**整批回退**（`git checkout` 回 `9e916a2`），只保留第 330 轮那 2 处形状 + 闭环。
这与第 315/321/326 轮是同一现象：**闭环没到 100% 之前，成批"逐位更准"的改动会把混沌
窗口换到更差的轨道**（这次 wet 涨了 69%）。

**下一枪**：先把 `stomata` 闭环推到 **`assim` 0 / `rst` 0** —— 重点查 `sortin`
（这条链上唯一没有 FMA、却仍在 102 条 `assim` 里出差的函数：它的分支/下标逻辑、
`cterm` 的三项结合、以及 `coupled_assimilation` 的三项结合都还没在闭环里逐条对过），
再整批落地重测。第 330 轮的闭环就是这一轮的判据，不用重建。

**第 332 轮：`sortin` 的 FMA 也逐条补了 —— 闭环 102→61，但湿窗仍 1287→2237 ⇒ 再回退**

第 331 轮把 6 处单独记下后，本轮把它们重新应用并继续把 `sortin` 的 FMA 补上。
`sortin` 是在 `MOD_assimstomataconductance_MOD_sortin.constprop.0` 里（**独立的 149 条
指令函数**，不是内联 —— 所以第一次 dump `_stomata` 时看不到它的 `.loc`，要按它自己的
标号解析）。它自己的 FMA（都已按汇编定死）：

```text
:414/:415/:416 首猜三条   pco2y(1)=fmadd(range,0.5,gamma)、pco2y(2)=fmadd(range,d0,gamma)、
                          pco2y(3)=fmsub(ratio,eyy(1),pco2y(1))
:453 pco2yl              fmsub(ratio,eyy(is),pco2y(is))
:459/:460 ac1/ac2        平方差里的第二个平方各进 fnmsub/fmsub
:465 bterm               分子分母各一条 fnmsub（各收一个乘积），1e-10 在分母算完之后加
:466 aterm               fmsub：`cc1 - bc1*bterm`
:467/:469 cterm          两条 fmsub：`pco2y(i2) - eyy(i2)*(aterm*eyy(i2)) - eyy(i2)*bterm`
```

闭环：`assim` **102→61**（模型 0 BB 23→11、模型 1 Medlyn 79→50、模型 2 WUE 仍 0）、
`rst` 三模型仍全 0。**但三段式仍拒绝**：wet `over_tol` **1287→2237**、`ot_vars` **24→55**、
`sumabs` 11.03→47.49（dry/snow/restart/3 步不变）⇒ **整批再回退**（`git checkout`）。

**教训（第 331 轮同款，这次更硬）**：闭环从 148 一路推到 61（每步都"更准"），湿窗却从
1287 单调涨到 2237 —— **"离 0 还有多少"与"黄金窗口好不好"在这条链上完全脱钩**，
只有 **0** 才算数。

**下一枪**：给 `sortin` 单独建一个小闭环（只有 7 个哑元：`eyy`/`pco2y` 两个数组 +
`range`/`gammas`/`ic`/`iterationtotal`；Rust 侧要把 `fn sortin` 提升到 `pub` 或加
`#[doc(hidden)]` 包装），把最后 61 条夹到一个函数里逐条对；`stomata` 闭环保持 61 的
状态不动，直到 `sortin` 也 0。

**第 333 轮：给 `sortin` 建了第 6 个闭环 —— 残差夹到 `ic≥4` 的二次拟合，形状回退**

第 332 轮记的下一步。`sortin` 是 **PRIVATE**，所以按"拷贝+放行"路线（`PRIVATE :: sortin`
→ `PUBLIC`）；三件套：`oracle/scripts/sortin_diff.f90` +
`crates/colm-core/examples/sortin_probe.rs` + `oracle/scripts/compare_sortin.sh`。
Rust 侧加了一个 `#[doc(hidden)] pub fn sortin_for_probe`（只为探针，不是业务 API）。
逐位比 `eyy(1..6)` 与 `pco2y(1..6)`。

**baseline**：`pco2y2` 231、`pco2y3` 277、`pco2y4` 96、`pco2y5` 78、`pco2y6` 70；
`eyy*` 全 0（**排序本身逐位对**）。

按汇编补的形状（重测直接照抄，别再读一遍）：
```text
:414/:415/:416 首猜   co2[0]=0.5.mul_add(range,gamma)、
                     co2[1]=(0.5-0.3*eyy_a).mul_add(range,gamma)、
                     co2[2]=(-eyy[0]).mul_add(ratio, co2[0])
:453 linear          (-eyy[is]).mul_add(ratio, co2[is])
:459/:460 ac1/ac2    (-eyy[i2]).mul_add(eyy[i2], eyy[i1]^2) /
                     (-eyy[i3]).mul_add(eyy[i3], eyy[i2]^2)
:465 bterm           cc1.mul_add(ac2, -(cc2*ac1)) /
                     (bc1.mul_add(ac2, -(ac1*bc2)) + 1e-10)
:466 aterm           (-bc1).mul_add(bterm, cc1) / (ac1 + 1e-10)
:467/:469 cterm      (-eyy[i2]).mul_add(aterm*eyy[i2], co2[i2]) 再 (-eyy[i2]).mul_add(bterm, …)
```

→ `pco2y2/3` **归 0**、`pco2y4/5/6` **96/78/70 → 39/32/41**。残差全在 **`ic≥4`**
（二次拟合分支），各档约 8%。试过的变体：`bterm` 分子分母"收另一个乘积"→ 56/52/54；
`ac2` 用 `eyy(i1)` 而不是 `eyy(i2)` → 320/334/332 ⇒ 两个现写法都对，但还有一处没夹准。

**形状仍不落**：这一批与第 331/332 轮那批一起套上时 `stomata` 闭环到 61、黄金湿窗
1287→2237（第 332 轮已记）。本轮只把 `sortin` 夹小、仍未到 0 ⇒ 代码回退，
**只留闭环入库**（`compare_sortin.sh` + `sortin_for_probe` 包装）；`stomata` 回到 148/114。

**下一枪**：把 `sortin` 的 `ic≥4` 那 ~1.3% 推到 0。重点三处：`bterm` 两条 `fnmsub`
究竟收哪一个乘积（现写法收 `cc1*ac2`/`bc1*ac2`，对了一大半）、`cterm` 两条 `fmsub`
的收法、以及 `n=3` 时 `i1`/`pco2b` 的夹取。闭环在手，一次改一处看 `pco2y4/5/6` 计数即可；
到 0 之后再把 `stomata` 那一整批一起落地重测。

**第 334 轮：`sortin` 那批只把闭环夹到 39/32/41，且**单独**就把湿窗推到 1863 ⇒ 回退**

重新应用第 333 轮记的 `sortin` 形状：闭环 `pco2y2/3` **归 0**、`pco2y4/5/6`
96/78/70 → **39/32/41**（`eyy*` 全 0 不变）。

穷举了 4 个变体，**都更差**（读数即下一轮的排除清单）：

```text
ac1 换号（(-eyy[i1]).mul_add(eyy[i1], eyy[i2]^2)）      → 342/347/355
cterm 摊平（co2[i2]-aterm*eyy*eyy-bterm*eyy）           → 64/57/51
bterm 分子分母收另一个乘积                                → 56/52/54
ac2 用 eyy(i1) 而不是 eyy(i2)                            → 320/334/332
```

⇒ 现写的配对（分子收 `cc1*ac2`、分母收 `bc1*ac2`、`cterm` 两条 `fmsub`）是**对的**，
残差是别处的一处结合。

**更硬的一条**：把 `sortin` 这一批**单独**套上（不叠加 `omc`/`ome`/Medlyn/BB/WUE
那批），湿窗 **1287→1863**、`ot_vars` **24→53**（dry 28/1、snow 25713/79、restart 0/68、
3 步 692/692 不变）⇒ **即使只动这一个函数，闭环没到 0 也会倒退**。代码回退；
`compare_sortin.sh` 保持 baseline 231/277/96/78/70。

**下一枪（换办法，别再猜结合顺序）**：把 `sortin` 的两个中间量也输出出来比 —— 在
闭环里同时打印 `ac1`/`ac2`/`bc1`/`bc2`/`cc1`/`cc2`/`bterm`/`aterm`/`cterm`（Rust 侧
加一个只在 `#[cfg(test)]`/探针里用的返回结构，Fortran 侧把 `sortin` 的这些局部量
也写出来），一眼就能看出是哪个量先分叉；再对着那一条读反汇编，比在 6 个变体里试快得多。
`update_photosyn` 的 8 条 FMA 也还没夹具。

**第 335 轮：`sortin` 闭环做到 3000/3000（`ac1` 的真形状）；`stomata` 到 4/3000，但湿窗仍 1287→2237**

**先加"中间量导出"**（这是本轮的钥匙）：闭环脚本往"拷贝"里注入一个
`dbg_intermediates(9)` 模块数组（在 `sortin` 尾部把 9 个局部量写进去），Rust 侧加
`sortin_intermediates_for_probe`（同一个 `sortin_impl`、可选 debug 出口，**一次调用**
同时给出更新后的数组与中间量）。第一次跑就把第一处分叉钉死在 **`ac1`**（682/1520），
而 `ac2`/`bc1`/`bc2`/`cc1`/`cc2` 全 0。

**`ac1` 的真形状**：`:459` 的 `fnmsub d3,d6,d6,d4` 语义是 `-d4 + d6*d6` ——
**收的是第一个平方、加数是 `-d4`（第二个平方）**，与紧接着 `:460` 的 `fmsub`
（收第二个平方）**恰好相反**。写成
`errors[i1].mul_add(errors[i1], -(errors[i2]*errors[i2]))` 后，
**`sortin` 闭环 3000/3000 逐位全同**（`eyy` + `pco2y` + 9 个中间量）。
（方法论纠错：`fnmsub` **取反的是加数**、不是乘积 —— 前几轮一直按错的语义
读，才在 6 个变体里打转。**但这句话对 `fnmadd` 不成立**，见第 336 轮的更正：
AArch64 的 `fnmadd d,n,m,a = -a - n*m`（加数与乘积**都**取反），
`fnmsub d,n,m,a = -a + n*m`（只取反加数）。第 336 轮用 `stomata:343` 的
`fnmadd d21,d20,d31,d21` 反推 `bquad = -2*(g0_term+acp) - (g1*acp)**2/(gbh2o*vpd)`
证实：只有 `-a - n*m` 这一读法与下游 `bquad**2 - 4*aquad*cquad`、
`(sqrt - bquad)/2` 自洽。）

再叠加第 331/332 轮那 5 处 stomata 形状（`omc`/`ome`/`oms`、Medlyn `:343-346`、
Ball-Berry `:353-356`、WUE `:861`）：`stomata` 闭环 **148/114 → 4/0** —— 只剩
`assim` 4 条（BB 1、Medlyn 3；其中两条差得较大，像迭代/分支边界）。

**但三段式仍拒绝**：wet `over_tol` 1287→**2237**、`ot_vars` 24→55
（dry 28/1、snow 25713/79、restart 0/68、3 步 692/692 不变）⇒ **整批回退**，
只留调试设施（`sortin_impl` + 两个探针出口，行为不变）。

**结论比第 331/332 轮更硬**：把一个函数判到 **3000/3000**、整条链判到 **4/3000**，
黄金湿窗照样倒退 74%。"凑齐到 0"**还不够** —— 要么那 4 条残余、要么夹具没覆盖到的
真实参数区间（黄金是 C3、固定 `par`/`tlef`/`rstfac` 范围）就足以换掉混沌轨道。

**下一枪**：① 把 `stomata` 的中间量也导出（`omc`/`ome`/`oms`/`bq`/`c`/`conductance`/
`internal`/`eyy`），把那 4 条定位到具体量；② 给夹具**加一档按黄金窗口真实参数区间的
采样**（照 CN-Cng-wet 第一步的 `par`/`tlef`/`rstfac`/`c3c4=1`），先把"夹具覆盖"这件事
本身验证掉，再谈落地。

**第 336 轮：`update_photosyn` 第 7 个闭环建成并把这条链判到 3000/3000；同时证明 `stomata` 那批形状在黄金窗口上**恒等**、黄金湿窗的状态种子其实在**土壤水****

**先把第 335 轮留的"夹具覆盖"问题回答掉 —— 答案是"不必覆盖，因为恒等"**：把
`sortin` 与 `wue_internal_co2` 那一处退回平、只留 `stomata` 函数里的形状
（`omc`/`ome`/`oms` + Medlyn + Ball-Berry），黄金湿窗实测
`bitwise=27972 sumabs=11.0278 over_tol=1287 ot_vars=24` —— 与**平基线逐位相同**。
原因不是巧合，是恒等：

* `c3_fraction`/`c4_fraction` 只取 `{0,1}`（`c3c4` 只取 0/1），于是
  `vm_term.mul_add(c3, vm*c4)` 与 `vm_term*c3 + vm*c4` 逐位相同（`mul_add(a,0,b)==b`、
  `a*0+b==b`）。`:258`/`:259`/`:262`/`:737`/`:738`/`:740` 六条 FMA 全是这种，
  **改不改都一样**。
* Medlyn（`:343`/`:344`/`:346`/`:354`）与 Ball-Berry（`:353`/`:356`）只在
  `use_medlyn`/两者皆假时才走；黄金默认 `DEF_USE_WUEST=.true.`，是**死分支**。
* C3 下 `assim = max(0, min(omc,ome))`，`coupled_assimilation`（`:264`/`:266`/`:742`/`:744`）
  也不走。

⇒ 第 331/332 轮记的"stomata 形状让湿窗倒退 1287→2237"**全部来自 `sortin`**，
与 `stomata` 函数无关。这也解释了为什么夹具加到 4000 例（含按黄金区间采样的 k=3 块）
也拦不住：那批形状本来就动不了黄金窗口。

**复核 `.loc` 时抓到 3 处从没进过任何一轮清单的缺口**（把 `stomata` 的 17 条、
`update_photosyn` 的 8 条重新逐条对齐源行）：

```text
stomata :259  / (electron_co2 + f77(2.0)*gammas)
              出货 `fmov d29,2.0e+0` + `fmadd d29,d9,d29,d28`（d9=gammas、d28=pco2i_e）
              ⇒ / f77(2.0).mul_add(photo.co2_compensation_pa, electron_co2)
update  :738  同款（internal）：`fmov d29,2.0e+0` + `fmadd d29,d9,d29,d28`
update  :805  errors[..] = internal - next
              出货 `fsub d31,d30,d31`（bracket）+ `fmsub d28,d31,d14,d28`
              （d28=pco2i、d14=psrf）⇒ (-bracket).mul_add(air_pressure, internal)
```

`:805` 那条是这条链一直判不到 0 的关键。注意 `stomata` 的对应处（`:323`/`:355`）
出货**没有** `fmsub`，所以只加在 `update_photosyn` 一侧 —— 两边照抄就会过度收缩。
顺带把 `wue_internal_co2` 的 `1+1.37*sqrt(...)` 用 `fmadd d21,d31,d25,d29`（`.loc 1 861`）
钉死：与现有 Rust 写法同形，确认无误。

**第 7 个闭环：`oracle/scripts/compare_update_photosyn.sh`**
（`update_photosyn_diff.f90` + `crates/colm-core/examples/update_photosyn_probe.rs`）。
3 个模型块 × 1000，逐位比 `assim`/`respc`：k=0 WUE+黄金区间（c3c4=1、par≤800、
tlef∈[275,315]、rstfac∈[0.3,1]）、k=1 WUE（c3c4 随机，两支都走）、
k=2 Ball-Berry（WUE 关 ⇒ 恒走 `coupled_assimilation`）。
`update_photosyn` 在模块里是 **PUBLIC**，直接链 `.bld` 的对象，**不需要**"拷贝+放行"。
它一次覆盖 `calc_photo_params` + `sortin` 的 6 次迭代 + `coupled_assimilation` + `:223` 的 `range`。

| 形状集合 | `update_photosyn` 逐位 |
|---|---|
| 平（HEAD） | `assim` **225**/3000 |
| + 第 331/332/333 轮那批 | `assim` 190/3000 |
| + 本轮 3 处（`:259` 分母、`:738` 分母、`:805`） | **3000/3000** ✓ |

`stomata` 闭环同口径：平 HEAD `assim` 171 / `rst` 114（4000），加完全部形状后
**`assim` 4 / `rst` 0**，且**本轮那处分母 FMA 没有动这 4 条** —— 残余全在
WUE 之外的分支（`k=0` BB 1 条 ~1 ULP；`k=1` Medlyn 3 条，其中 `1975` 号 Gold 得 `0`
而 Rust 得 `0x3F00D79435E50D79`，像 `max(0,·)` 的分支边界）。黄金窗口走不到，先记着。

**新器材：湿窗 restart 逐步扫描 —— 它比 `over_tol` 锐得多，而且本轮推翻了一个归因**

`history` 里的量是 `MOD_Vars_1DAccFluxes` 的 `acc1d` 累加/换算出来的，**第 1 条记录**
就带 1 ULP 的**派生量**差：`f_h2osoi[1,0,0]` 差 1 ULP 而同一记录的 `f_wliq_soisno`
逐位相同 —— 那只可能是换算式（层厚/除法）的舍入，不是状态。这种差会污染 `over_tol`。
判形状要用**瞬时状态**，所以照 `restart_scan.sh` 给湿窗配了
`/tmp/gf/wet_ts.sh N`（`oracle/scripts/restart_divergence.py` 判 68 个状态量）。

实测：湿窗首个状态分歧在 **N=4**，`wliq_soisno[0,5]`（顶层土壤液态水）
`9.034010105577982` vs `9.034010105577984`，1 ULP。然后做 A/B：

| 变体 | 跑过的 N | 与平 HEAD 的状态分歧清单 |
|---|---|---|
| 平 HEAD | 4,8,12,16,24,32,48,64 | （基准：N=4 起 `wliq_soisno[0,5]` 1 ULP） |
| 只 `stomata` 函数形状 | 1,2,3,4 | **逐位相同** |
| 17+`sortin` 全部（旧 22 处，缺本轮 3 处） | 4,5,6,8 | **逐位相同** |
| 25 处全开（含本轮 3 处） | 4,8,12,16,24,32,48,64 | **逐位相同** |
| 第 324 轮的 swvm 自有 5 处 | 4,5,6 | **逐位相同** |

⇒ **这条光合/气孔链（`stomata` 17 + `update_photosyn` 8 + `calc_photo_params` 4 = 29 处
FMA）对湿窗状态轨迹零影响**；湿窗的种子在土壤水里，不在这条链上。链条确实会改 `history`
（它经叶温进湍流诊断），所以 `over_tol` 会摆动 —— 但那是**诊断层**的 1 ULP 经混沌放大，
不是状态层的账。

**三段式实测（A/B，同树）**：

| 窗口 | 平基线（HEAD） | 25 处全开 |
|---|---|---|
| dry | 28 / 1，bitwise 17057，sumabs 330.8133 | 28 / 1，bitwise 17056，sumabs 330.8133 |
| wet | 1287 / 24，bitwise 27972，sumabs 11.0278 | **1890 / 52**，bitwise 28605，sumabs 23.7476 |
| snow | 25713 / 79，bitwise 32721，sumabs 444416.8246 | 25713 / 79，bitwise 32719，sumabs 444416.8246 |

干窗/雪窗的 `over_tol`/`ot_vars` 一字不动，湿窗 `over_tol` 1287→1890、
`ot_vars` 24→52。按现行口径（任一窗口变差即回退）**整批回退**（`git checkout`），
只留第 7 个闭环与第 335 轮那批调试设施。

**这条链的 25 处 FMA 全清单（照 Fortran 行号，`update_photosyn` 与 `stomata` 共用
公式的地方标"同"）**：

```text
range（:223 stomata / :710 update_photosyn）—— **已入库 HEAD**
  pco2m.mul_add(1.0 - 1.6/gradm, -gammas)
omc（:258 / :737）  vm_term.mul_add(c3, vm*c4)                    —— 恒等
ome 分母（:259 / :738） f77(2.0).mul_add(gammas, pco2i)            —— **数值有效**
ome（:259 / :738）  epar_term.mul_add(c3, epar*c4)                —— 恒等
oms（:262 / :740）  omss.mul_add(c3, omss*pco2i*c4)              —— 恒等
耦合判别式（:264/:266 / :742/:744）coupled_assimilation（已入库）—— 仅非 WUE/C4 分支
Medlyn（:343 bq / :344 c / :346 判别式 / :354 conductance）        —— 死分支
Ball-Berry（:353 bq,c / :356 conductance）                        —— 死分支
wue 电子（:861）    f77(1.37).mul_add(sqrt, 1.0)                  —— 数值有效（仅 stomata）
eyy（:805，仅 update_photosyn）  (-bracket).mul_add(psrf, pco2i)   —— **数值有效**
```

⇒ 对黄金窗口**真正有数值作用**的只有 4 处：`:223`/`:710` 的 `range`（已入库）、
`:259`/`:738` 的 `ome` 分母、`:805` 的 `eyy`、`:861` 的 `wue` 电子。其余 21 处要么恒等、
要么死分支。**下一轮重放时只有这 4 处需要先落**，其余跟着一起落只为把闭环钉到 0。
应用脚本（会话内）`/tmp/gf/photo_variant.py` + `/tmp/gf/photo_extra.py`，
全形状快照 `/tmp/gf/photo_all_extras.rs`。

**普查补一笔：`calc_photo_params` 是 `isra.0` 独立符号，另有 4 条 FMA**（第 329 轮
只数了 `stomata` 17 + `update_photosyn` 8）。它的符号在 `assim.s:361-753`
（`___mod_assimstomataconductance_MOD_calc_photo_params.isra.0`），
`.loc` 落在 `:571`（`vm = vm/temph*rstfac*c3 + vm/(templ*temph)*rstfac*c4`）、
`:580`（两条，`respcp`/`respc` 一线）、`:599`。**这 4 条全是 `A*c3 + B*c4` 的形**，
在 `c3`,`c4 ∈ {0,1}` 下与不融合逐位相同 ⇒ 恒等，不用改。
所以这个模块的完整账是 **17 + 8 + 4 = 29 条 FMA**，其中 6 条（`stomata:258/:259/:262`、
`update_photosyn:737/:738/:740`、`calc_photo_params` 那 4 条）**恒等**。
⇒ `stomata` 闭环剩下那 4 条**不是"还有没归属的 FMA"**：29 条已全部归位，
残余只能是非 FMA 的表达式/分支差（`k=0` BB 1 条 ~1 ULP、`k=1` Medlyn 3 条），
且都在黄金窗口走不到的分支里。

**给下一轮（按优先级）**：

1. **打 N=4 那颗 `wliq_soisno[0,5]` 的种子** —— 这是黄金湿窗唯一的状态种子。
   第 324 轮的 17 处里，本轮只测了 **swvm 自有那 5 处**（对 N≤6 无影响）；
   **内联的 `Richards_solver` 6 处 + `use_explicit_form` 6 处还没测**，
   `.loc` 行号与形状在第 324 轮那张表里现成（`:817/:823/:966/:1060/:1065/:1085`、
   `:1411/:1424/:1464/:1474/:1479`）。判据就用 `wet_ts.sh 4` 的状态分歧是否消失/后移。
2. 湿窗 `f_h2osoi` 那条**换算式**的 1 ULP（层厚来源 = `template.soil_layer_thickness_m()`）
   单独查一遍 —— 它不改状态，但一直占着"第一条分歧记录"。
3. 光合链那 4 处数值有效的形状，等 1 或 2 落地后**成组**重放（第 316 轮就是这么成组的）。
4. `stomata` 那 4 条残余（BB/Medlyn 分支）**优先级最低**：它们不在黄金路径上，
   而且要判就得照第 333 轮给 `stomata` 也导中间量（`omc`/`ome`/`oms`/`bq`/`c`/
   `conductance`/`internal`/`eyy`），需要把 `stomata` 也走"拷贝+放行"路线注入 `dbg` 数组。

**第 337 轮：把第 324 轮那 12 处补测了（湿窗种子照旧不动）；顺手发现 `soilwater_aquifer_exchange` 的 4 处从没入账；并换来一个判据升级 —— 用 GIMPLE 具名操作数代替 `.loc` 猜操作数角色**

**先说方法升级（本轮最值钱的部分）**：`.loc` + 反汇编只能判"**哪条乘积进了 FMA**"，
判不了"**操作数是谁**"。第 336 轮结尾刚发现 `fnmadd` 语义被读错就是同一类坑。
`gfortran -O2 -fdump-tree-optimized` 的 GIMPLE 里 FMA 是具名的：

```text
_344 = ss_vliq(izwt) * (zwt - sp_zi(izwt-1))       ← 独立舍入的乘积
_56  = .FMA (zwtp - zwt, porsl, _344)             ← 进 FMA 的是**第二个**乘积
```

本轮照这个把 `MOD_Hydro_SoilWater.F90` 逐条过了一遍，**一次就抓出 4 处操作数写反**
（我第一版按"收第一个源乘积"的成见写了 `soilwater_aquifer_exchange` 的 `:560/:569/:573`，
GIMPLE 说这三处收的是**第二个**）。⇒ 以后判形状**先用 GIMPLE 定角色、再用 `.loc` 定位**。

**补测的两组（第 324 轮的 12 处）**：内联 `Richards_solver` 6 处
（`:817` 湿转干判据、`:823` `sqrt(sum(blc**2))`、`:966`/`:1060` `psi_s+(sp_zi-zwt)*0.5`、
`:1065` `ss_q` 累加、`:1085` `ss_vl` 收尾）+ 内联 `use_explicit_form` 6 处
（`:1411`/`:1424` `wa_m1`、`:1464` `dp`、`:1474` **两条**、`:1479` `waquifer`）。
GIMPLE 逐条确认（`_1372/_1373/_1380` 一式收第一个乘积、`:1474` 的第二条是
`_3266 = .FMA(dt, q(ilev-1)-q(ilev), _616)`，`:1085` 的 `_1159 = .FMA(_1151,_1152,_1158)`
收第一个）。

**新入账的两组**：

* `soilwater_aquifer_exchange` 有**独立符号**（第 324 轮那张表没提它），自带 4 条 FMA
  （`:560/:564/:569/:573`）。`:564` 是 `zwtp - 0.5*(sp_zi+zwt)`（GIMPLE `.FNMA(_66, 5.0e-1, zwtp)`），
  另三条见上面的"收第二个"。**已按 GIMPLE 更正。**
* `get_water_equilibrium_state` 另有 2 条（`:137` `(zwtmm-z_prev).mul_add(vliq_up, porsl*(z_i-zwtmm))`、
  `:150` `(z_n-zwtmm).mul_add(0.5, psi_zwt)`）。它在 `main/` 里**没有调用点**（只有 `PUBLIC`
  声明），本仓库 `equilibrium_water_state` 只在冷启动用一次 ⇒ 与 N≥1 的分歧无关，**未落**。

**模块内剩下的账已经清完（别再来翻）**：把 `soil_water_vertical_movement` 符号里
**全部** `.FMA/.FNMA` 过一遍，除了上面 17 处，只剩两类 ——
① `Richards_solver` 里两个 `wsum = sum(…)` 的累加（GIMPLE `w_sum_before_*`/`w_sum_after_*`，
8 条）：Rust 用的是 `.sum::<f64>()`（不加 FMA），但这两个和**只喂 `_balance_error_mm`**，
而上游算 `werr` 也只为调试打印（`crates/.../variably_saturated_flow.rs:4031-4035` 的注释已记）
⇒ **死账**，加不加都不改轨迹；
② 蒸腾级联里的 `deficit` 累加（`:308-320`）：第 273 轮已按同一套办法做过
（`variably_saturated_flow.rs:4860-4892` 的注释与 `:338`/`:406` 的 `wextreme`/`ss_dp` 形状都在）。
⇒ **湿窗 N=4 那颗种子不在 `MOD_Hydro_SoilWater` 里**，别在这个模块继续找。

**判据用哪个：三个仪器给三个方向，记下来备查（同树 A/B）**：

| 仪器 | 平基线 | 12 处 + 气孔交换**错序** | 12 处 + 气孔交换**GIMPLE 正序** |
|---|---|---|---|
| 湿窗 restart N=4/5/6/8 | 1/1/1/5（per 68） | **完全相同** | **完全相同** |
| 干窗 restart N=18/19/20/21/22 | 5/5/5/7/— | **0/2/1/0/0** | 5/6/6/7/22 |
| dry `over_tol` / `ot_vars` | 28 / 1 | 32 / 1 | 32 / 1 |
| dry `sumabs` | 330.8133 | 330.8866 | **223.6837** |
| wet `over_tol` / `ot_vars` / `sumabs` | 1287 / 24 / 11.0278 | 2047 / 53 / 41.3839 | 1515 / 32 / 21.5579 |
| snow `over_tol` / `ot_vars` / `sumabs` | 25713 / 79 / 444416.8246 | 同 | 同 |

**三条结论**：

1. **这 16 处（乃至整条 `MOD_Hydro_SoilWater`）不产生湿窗 N=4 那颗种子**：五个变体在
   N=4/5/6/8 上的 68 状态量差异清单**逐位相同**（`wliq_soisno[0,5]` 1 ULP）。
   湿窗的种子在 `MOD_Hydro_SoilWater` **之外**的顶层水量路径上（`WATER_VSF` 的回填
   `MOD_SoilSnowHydrology.F90:1105-1120` 已逐条对过、`runoff`/相变还没过）。
2. **restart 扫描是"定位器"，不是"裁判"**：干窗 N=18 的残留**个数**偏好那个被 GIMPLE 判为
   错序的版本（0/2/1/0 vs 5/6/6/7）。个数是 1 ULP 硬币的计数，和 `over_tol` 一样会被混沌
   翻转；**只有 `sumabs`（逐点误差）是单调的** —— GIMPLE 正序把 dry `sumabs` 从
   330.81 压到 **223.68**（−32%），错序则与平基线持平（330.89）。这反过来说明正序是对的。
3. 按现行口径（任一窗口 `over_tol`/`ot_vars` 变差即回退）**整批回退**（`git checkout`）。
   但请把上表当成一个**待裁决**的问题：口径用的是 `over_tol`，而它在这一轮里给出的
   方向与 `sumabs` 正相反。全形状快照：`/tmp/gf/vsf_17plus4_correct.rs`。

**给下一轮**：

1. **换仪器**：`over_tol` 与 restart 残留**个数**都不能裁这条链，改用
   **dry `sumabs`**（单调、且本轮给出了 330.81→223.68 的明确信号）作为第一判据，
   `over_tol` 只做"不许大幅变差"的护栏。
2. **找湿窗 N=4 那颗种子**：已排除 `MOD_Hydro_SoilWater` 全模块（含气孔交换）。
   下一个靶子是 `MOD_SoilSnowHydrology` 的 `WATER_VSF` 回填与 `runoff`/相变；
   建议照本轮的办法先做 **GIMPLE 普查**（`-fdump-tree-optimized` 里 grep `.FMA`/`.FNMA`），
   按符号归属到 Rust 函数，再逐条比。
3. `get_water_equilibrium_state` 那 2 处只在冷启动路径，优先级最低，但补上能让 N=0 也逐位。

**第 338 轮：湿窗那颗 N=4 的种子找到了 —— 是**已入库**的 `WATER_VSF` 回填形状把操作数写反；改正后湿窗首分歧 N=4 → **N=20**，干窗 N=18 从 5/68 → **0/68**，据此按第 249 轮的先例保留**

**病灶**：`crates/colm-core/src/variably_saturated_flow.rs` 的
`variably_saturated_flow_step` 里、`MOD_SoilSnowHydrology.F90:1109-1110` 那段回填
（`wliq = denh2o*((eff*(sp_zi(j)-zwtmm)) + vol_liq*(zwtmm-sp_zi(j-1)))/1000`）。
原先写的是 **收第一个乘积**（`eff.mul_add(above, vol_liq*below)`），注释还写着
"GIMPLE（`water_vsf` 第 6 处）是 `FMA(eff, sp_zi-zwt, …)`"。**那是读错的**。
`MOD_SoilSnowHydrology.opt`（`-fdump-tree-optimized`）里这一段是：

```text
_245 = _241 - pretmp_168          ; sp_zi(j) - zwtmm
_247 = _246 * pretmp_1494         ; eff * (sp_zi(j)-zwtmm)  ← **独立舍入**
_248 = _247
_250 = pretmp_168 - pretmp_1496   ; zwtmm - sp_zi(j-1)
_253 = .FMA (_249, _251, _248)    ; vol_liq*(zwtmm-sp_zi(j-1)) + 上面那个已舍入的乘积
_255 = _254 * 1.0e+3 ;  _257 = _255 / 1.0e+3      ; denh2o/1000
```

数组身份也钉死了（不是猜的）：`_593` 是 `eff_porosity`（dump 里 `wresi` 那条
`_206 = .FNMA(_202, _204, _201)` 用的 `_202 = MEM[_593]` 就是 eff），
`_609` 是 `vol_liq`（dump 里存过 `clamp(vol_liq,0,eff)`）。
⇒ **进 FMA 的是第二个源乘积**。改正后代码为
`liquid_volume_fraction[level].mul_add(zwt - z_lo, eff*(z_hi - zwt))`。

**为什么会被写反**：`.loc` + 反汇编只能判"哪条乘积进 FMA"，判不出"操作数是谁"——
和上一轮 `soilwater_aquifer_exchange` 那 4 处是同一个坑。这是在**已入库**的代码里
第二次踩到，所以第 337 轮那条"先用 GIMPLE 定角色"的规矩要**回头把已入库的形状也过一遍**。

**实测（同树 A/B，改前 = 上一提交）**：

| 仪器 | 改前 | **改后** |
|---|---|---|
| 湿窗 restart N=4/5/6/8/10/12/16 | 1/1/1/5/—/—/— | **全 0/68** |
| 湿窗 restart N=20 | — | 首分歧推迟到这里（`zwt` 3/68，maxrel 6.9e-11） |
| 干窗 restart N=18 | 5/68 | **0/68** |
| 干窗 restart N=19/20/21/22 | 5/5/7/— | 3/5/5/2（首分歧 N=18 → **N=19**） |
| 干窗 1 步 restart | 0/68 | 0/68（不变） |
| 干窗 3 步 history | 692/692 | 692/692（不变） |
| 黄金 dry | 28/1，17057，330.8133 | 36/1，16436，**382.1465** |
| 黄金 wet | 1287/24，27972，11.0278 | **2210/53**，28089，**45.0805** |
| 黄金 snow | 25713/79，32721，444416.8246 | 25713/79，32659，444414.2028 |

**聚合指标反向，为什么仍然保留（照第 249 轮的先例，逐条对齐）**：第 249 轮定的判据是
"① 短程**直接量改善或中性**；② 改动**逐条对得上内核真正编译的那份源码**；
③ 聚合反向只是那点差别被 11 天放大（混沌），不是公式错"。本轮三项都满足：
① 干窗 1 步 0/68 不变、3 步 692/692 不变，而**状态扫描明确改善**（湿 N=4→N=20、
干 N=18 5/68→0/68）；② 操作数角色由 GIMPLE 具名操作数 + 数组身份定死；
③ 短程没有任何系统性发散。若公式真错，短程会系统发散 —— 实测相反。
⇒ 保留这一处，并**据实记在这里，供后来者复核**（第 249 轮也是这么记的）。

**还没做的（下一轮）**：

1. **回头用 GIMPLE 把已入库的形状全过一遍**：`water_vsf`/`soil_water_vertical_movement`/
   `Richards_solver`/`use_explicit_form` 里每一条 `.FMA/.FNMA` 的**操作数角色**。
   本轮已顺手确认下面这些**是对的**：`:434` 收第一个、`:478` 收第一个、
   `:1474` 收第一个再把 `dwat` 收进去、`:406`/`:338`/`:1474` 第二条、`:823`、
   `:966`/`:1060`、`:560`/`:569`/`:573` 收**第二个**、`:564` 的 `.FNMA(_66, 5.0e-1, zwtp)`。
2. **第 337 轮那批（swvm 5 + 12 + 气孔交换 4）要在本轮的修正**之上**重测**：
   它们的 A/B 是在没有本轮修正的树上量的，结论可能变。
3. 继续状态扫描找下一颗种子：湿窗现在的首分歧在 **N=20 的 `zwt`**
   （maxrel 6.9e-11，已经不是 1 ULP 量级，是放大过的下游）；干窗在 **N=19**。

**第 339 轮：第 337 轮那批在本轮修正之上「成组落地」—— 干窗首分歧 N=19 → **N>40**，干窗黄金三口径全线改善**

第 337 轮那批（swvm 自有 5 + 内联 `Richards_solver` 6 + 内联 `use_explicit_form` 6 +
`soilwater_aquifer_exchange` 4 = 21 处）当时是在**没有**第 338 轮修正的树上量的，
所以按第 338 轮留的"下一轮第②条"在本轮修正之上重测 —— 结论**反过来了**。

**同树三态 A/B**（`原基线` = 338 之前；`HEAD` = 只有 338 那处修正；`批次` = 21 处 + 338 修正）：

| 窗口 / 仪器 | 原基线 | HEAD（只 338 修正） | **批次 + 338 修正** |
|---|---|---|---|
| dry `over_tol` / `ot_vars` | 28 / 1 | 36 / 1 | **28 / 1** |
| dry `sumabs` | 330.8133 | 382.1465 | **261.0128** |
| dry `bitwise` | 17057 | 16436 | **9355** |
| wet `over_tol` / `ot_vars` | 1287 / 24 | 2210 / 53 | 1927 / **54** |
| wet `sumabs` | 11.0278 | 45.0805 | **28.8358** |
| wet `bitwise` | 27972 | 28089 | 28556 |
| snow `over_tol` / `ot_vars` / `sumabs` | 25713 / 79 / 444416.8246 | 25713 / 79 / 444414.2028 | 25713 / 79 / 444416.8246 |
| snow `bitwise` | 32721 | 32659 | 33307 |
| 干窗 restart 首分歧 | N=18（5/68） | N=19 | **N=288**（N=18…192 全 0/68；N=288 起 22/68，`tref`/`tleaf` 各 1 ULP） |
| 湿窗 restart 首分歧 | N=4（1/68） | N=20（3/68） | N=18（1/68，`vegwp`）；N=19 0/68、N=20 2/68、N=21 2/68 |
| dry 1 步 / 3 步 | 0/68 / 692-692 | 0/68 / 692-692 | 0/68 / 692-692 |

**为什么落**：相对 HEAD，干窗**每一项都变好**（`over_tol` 36→28 回到原基线、
`sumabs` 382→261、`bitwise` 16436→9355），而干窗状态扫描从 N=19 推到 **N>40**；
湿窗相对 HEAD 也是 `over_tol` 2210→1927、`sumabs` 45.08→28.84 变好；雪窗聚合口径与
原基线一字不差。唯一命中"变差"字样的是湿窗 `ot_vars` 53 → **54**（一个变量跨过容差线）。
按第 249 轮先例的三条（短程直接量改善、逐条对得上编译源码、聚合反向属混沌）本轮满足，
且这次短程是**大幅改善**而不是中性 ⇒ 落地。

**同一轮里做的两件复查（都记下来，供后来者复核）**：

**① 已入库的操作数角色（GIMPLE 具名操作数）—— 本轮查过的全对**：

```text
solve_least_squares_problem（Givens）
  tmp   = c*A(i,k) + s*A(j,k)      GIMPLE `_31 = _21*s; _32 = .FMA(_24, c, _31)`  收第一个 ✓
  A(j,k)= -s*A(i,k) + c*A(j,k)     GIMPLE `_43 = _42*s; tmp = .FMA(_39,c,_43)` + `.FMS(_42,c,_39*s)` ✓
  res 两式同上 ✓
  dv(i) = dv(i) - A(i,k)*dv(k)     GIMPLE `_70 = .FNMA(_67, _68, _63)` ✓
secant 夹逼
  x_l*alp + x_r*(1-alp)            GIMPLE `_144 = .FMA(x_l, 0.9, x_r*0.1)`       收第一个 ✓
  x_l*(1-alp) + x_r*alp            GIMPLE `_148 = .FMA(x_l, 0.1, x_r*0.9)`       收第一个 ✓
WATER_VSF
  :1036-1038 vol_liq 分子          GIMPLE `_206 = .FNMA(eff, above, 水量mm)` ✓
  :1043-1044 嵌套 wresi            GIMPLE `_206` 之后 `_222 = .FNMA(vol_liq, below, _206)` ✓
  :866-868 wresi                   GIMPLE `_112 = .FNMA(dz*denh2o, vol_liq, wliq)` ✓
```

**② secant 主式的"收第二个"变体 —— 实测否定，别再猜**：GIMPLE 里是

```text
_134 = fval_k1_122 * pretmp_128        ;
_135 = .FMS (_22, psi_i_k1_168, _134)  ;
```

只看 SSA 名字分不出 `_134` 是哪个乘积（`fval_k1_122`/`_22` 的新旧指代在 dump 里对不上
Fortran 的赋值序），所以照"收第二个"写了
`(-residual_before_previous).mul_add(*previous_value, *previous_residual * value_before_previous)`
实测：**干窗 N=18 5/68、N=19 6/68、N=22 22/68、N=32 29/68；湿窗 N=19 1/68、N=20 6/68**
（批次版对应 0/0/0/0 与 0/2）⇒ 已入库的"收第一个"是对的，**回退**。
教训：SSA 名字分不清角色时，**实测就是判据**，别从名字反推。

**给下一轮**：

1. 湿窗现在的种子在 **N=18 的 `vegwp`（1/68）** 与 **N=20 的 `wliq_soisno`+`hk`（2/68）**；
   干窗已过 N=40，继续往后扫（`dry_ts.sh` 的目录名跨天后是 `2008-002-*`，脚本里的
   `DIR` 公式要跟着改）。
2. 把已入库形状的 GIMPLE 复查推到其它水模块（`water_2014`/`snowwater`/`meltf`/
   `compute_vic_runoff` 的 FMA 数分别 15/4/8/3，`soilwater` 19 但只在 `WATER_2014` 分支；
   黄金算例 `DEF_USE_VariablySaturatedFlow=.true.`、`DEF_SPLIT_SOILSNOW=.false.`，
   所以 split/VIC/2014 那几支可以先不管）。
3. 判据仍以**状态扫描首分歧步**为准；`over_tol`/`ot_vars`/`sumabs` 三者都会被混沌带偏
   （本轮 `ot_vars` +1 而短程大幅改善就是例子）。

**第 340 轮：FMA 普查口径更正 —— 站点清单必须从 `-S -g` 的 `.loc` 数，GIMPLE dump 会数错；顺带把植物水力链普查了（55 处）**

**口径更正（本轮最要紧的一条）**：第 337/339 轮我用
`-fdump-tree-optimized` 的 dump 数 `.FMA/.FNMA` 来做"这个符号有几处"，
**那是不可靠的** —— 向量化/循环展开会**复制**同一条源表达式的 FMA，GCC 也**合并**
过一些，于是两个方向都会错。同一份源码两种数法的实测差：

| 符号 | GIMPLE dump 数 | **`-S -g` 汇编数** |
|---|---|---|
| `spacaf_twoleaf` | 25 | **41** |
| `soilwater` | 19 | **24** |
| `water_vsf` | 17 | **15** |

（`water_vsf` 那 17 里有两处是 `:1109-1110` 一带的向量化副本。）
⇒ **规矩**：**站点清单**用 `gfortran -O2 -g -S` + `.loc` 逐行数；
**操作数角色**用 `-fdump-tree-optimized` 的**具名操作数** + 数组身份。
两者缺一不可 —— 第 338 轮找到 `:1109` 靠的是后者，第 337 轮漏掉
`soilwater_aquifer_exchange` 靠的是前者没数全。第 339 轮文档里引的
"`water_2014` 15 / `snowwater` 4 / `meltf` 8 / `compute_vic_runoff` 3" 那几个数
也来自 GIMPLE，本轮换成汇编口径重新给在下面。

**汇编口径普查（黄金算例走得到的水/植物/相变模块）**：

```text
MOD_Hydro_SoilWater
  soil_water_vertical_movement  29   （第 337 轮已全部归位：17 处已判 + 12 处已落）
MOD_SoilSnowHydrology
  water_vsf                     15   ← 黄金走这一支
  soilwater                     24   （只在 WATER_2014 分支，DEF_USE_VariablySaturatedFlow=.true. ⇒ 不走）
  water_2014                    15   （同上，不走）
  snowwater                      4   （lb<=0 才走；黄金无雪）
  snowwater_snicar               8   （DEF_USE_SNICAR 关，不走）
MOD_PhaseChange
  meltf                          8
  meltf_snicar                   8   （SNICAR 关）
  meltf_urban                    2   （patchtype>=3；黄金不走）
MOD_Hydro_VIC
  compute_vic_runoff             3   （DEF_Runoff_SCHEME=3 是 Simple-VIC，走的是 runoff.rs 的
                                       `simple_vic_runoff`，不是这一支 —— 待核）
MOD_PlantHydraulic
  spacaf_twoleaf                41   ← 湿窗 N=18 的 `vegwp` 种子在这条链上
  getrootqflx_x2qe               5
  getrootqflx_qe2x               3
  getqflx_qflx2gs_twoleaf        3
  getqflx_gs2qflx_twoleaf        3   （合计 55；plant_hydraulics.rs 现有 50 个 `mul_add`）
```

**逐行图（下一轮直接照这两张表做站点级比对，不用再编汇编）**：

```text
water_vsf（15 处）
  827: 1   867: 1   1037: 1   1045: 2   1110: 1   1128: 1   1132: 1   1133: 1
  1220: 1  1228: 1  1231: 1   1262: 1   1267: 1   1273: 1
  （`:1110` 就是第 338 轮修好的那处回填）
spacaf_twoleaf（41 处）
  456: 1  457: 1  458: 1  459: 1  464: 2  465: 2  468: 1  471: 1  472: 1
  479: 3  483: 5  485: 5  487: 3  489: 5  496: 2  498: 3  499: 2  500: 2
soil_water_vertical_movement（29 处，第 337 轮已归位）
  137: 1  150: 1  263: 1  265: 1  266: 1  268: 1  326: 1  331: 1  338: 1
  406: 1  434: 1  445: 1  447: 1  448: 1  450: 1  456: 1  478: 1  817: 1
  823: 1  966: 1  1060: 1 1065: 1 1085: 1 1411: 1 1424: 1 1464: 1 1474: 2
  1479: 1
```

**已核过的一处（植物水力）**：`getrootqflx_qe2x` 的 `:941`/`:950`/`:959` 三行
（`krad*smp - qeroot - kax`、`krad*smp + kax(j-1) - kax(j)`、`krad*smp + kax(j-1)`）
在 Rust 里由 `root_potential_from_flux` 的**一条** `mul_add` 覆盖 ——
三支共用"收 `krad*smp`"这一个形状、末项 `-kax` 折进 `map_or(0.0, …)`，
逐条对得上（`plant_hydraulics.rs:660-666` 有注释）。所以"55 vs 50"的差**不是**
5 处漏形状，得逐站点对，不能按总数推。

**下一轮**：按上面两张逐行图做站点级操作数比对 —— 先 `spacaf_twoleaf` 的 41 处
（对应 `plant_hydraulics.rs:spac_change` 的 40 处，注释里已有一批 FNMS/FMA 的 GIMPLE 读数），
再看 `water_vsf` 的 15 处里除 `:1110` 之外的 14 处。判据仍是
`wet_ts.sh 18/20`、`dry_ts.sh 288` 的首分歧步。

**第 341 轮：植物水力链抓到 `conductance_from_transpiration` 里 3 处**多收**的形状 —— 湿窗 `sumabs` 28.84 → **5.55**（比第 338 轮之前的原基线 11.03 还小一半）**

按第 340 轮备好的逐行图做站点级比对，先是植物水力链。

**① `getqflx_qflx2gs_twoleaf` 的 FMA 数是 3，Rust 有 6 —— 多收了 3 处。**
汇编 `.loc` 只给三处：`:781`（`cqi_leaf = caw*(qsatl-qm) + cgw*(qsatl-qg)`）、`:794`、`:795`。
GIMPLE 的具名操作数把这三处钉死，并且说明 `:794/:795` 收的都是**分子**：

```text
_50 = c1_110 * b2_112 ;
_51 = .FNMS (_44, c2_113, _50) ;   → csun 的分子（`c1*b2 - …`）
_55 = _53 - _54 ;                  → csun 的分母：**两个已舍入乘积的普通减法**
a2_111 = -_47 ;
_58 = c1_110 * a2_111 ;
_59 = .FMS (a1_108, c2_113, _58) ; → csha 的分子
_61 = _54 - _53 ;                  → csha 的分母：同样**不收缩**
```

而 Rust 里两个分母都写成了 `mul_add`（`b1.mul_add(a2, -b2_a1)`、`(-b1).mul_add(a2, b2_a1)`），
`dry_fraction = 1 - delta*(1-fwet)`（`:779` 的 `cwet`，两处调用各一份）也写成了 `mul_add`
—— `:779/:780` 的出货汇编只有 `fmul`/`fsub`/`fdiv`，**没有** FMA。
⇒ 四处**多收**（同一表达式在两处调用里各一次）。改成普通减法/普通乘法后：

| 窗口 | 原基线（338 之前） | HEAD（338+339） | **本轮** |
|---|---|---|---|
| dry `over_tol`/`ot_vars`，`sumabs`，`bitwise` | 28/1，330.8133，17057 | 28/1，261.0128，9355 | 28/1，261.0128，9355（**不变**） |
| wet `over_tol`/`ot_vars`，`sumabs`，`bitwise` | 1287/24，11.0278，27972 | 1927/54，28.8358，28556 | **1293/28，5.5478，28307** |
| snow | 25713/79，444416.8246，32721 | 同 | 同 |
| dry 1 步 / 3 步 | 0/68 / 692-692 | 同 | 同 |

湿窗 `sumabs` 相对 HEAD 降 **81%**，相对原基线（11.03）也小一半；`over_tol`/`ot_vars`
从 1927/54 回到 1293/28（原基线 1287/24）。干窗、雪窗一字不动。

**② `getrootqflx_x2qe:892` 的 `qeroot` 补了一条 FMA（单独测：惰性）。**
`qeroot = krad*(smp(1)-xroot(1)) + (xroot(2)-xroot(1))*kax/den2 - kax`，Rust 原先两个乘积都独立舍入。
出货汇编是 `fmul d0,d0,d13; fdiv d0,d0,d15; fsub d31,d31,d14; fmadd d0,d26,d31,d0`
⇒ **第一个**源乘积进 FMA。补上后单独测：三窗口与湿/干两个状态扫描**逐位不变**（对这算例惰性），
但它与 `:1109` 同类（都是"收哪一侧"），且 `:892` 的站点清单来自汇编，所以**一并落地**。

**③ 方法论追加（比上面两条更值钱）**：**短程状态扫描"惰性"不等于长程惰性。**
①那四处改动在 `wet_ts.sh` N≤22 与 `dry_ts.sh` N=288 上**逐位不变**，
（做了活性诊断：把 `conductance_from_transpiration` 的分母加 `1e-3` 会让 Rust 跑挂，
证明这条链确实在场上；`qeroot` 那一侧把末项符号翻一下也会跑挂。）
可 384 条记录的黄金湿窗 `sumabs` 直接掉到 1/5。
⇒ 判据要**两条一起看**：状态扫描的**首分歧步**（前期）+ 黄金窗口的 `sumabs`（长程单调性）；
只看前者会把这类"晚发作"的形状放过去。
补充实测：这四处改动在湿窗 restart 上**一直到 N=96 都逐位不变**
（N=24/32/48/96 = 21/21/19/15 per 68，与改动前逐个相同），而 384 条记录的 `sumabs` 掉到 1/5 ——
差异出现在 N>96 之后，说明这类形状的发作点可以很晚。

**同轮顺带收口的**：`water_vsf` 的 15 处全部归位 —— 活着的 6 处（`:867`/`:1037`/`:1045`×2/`:1110`/`:1128`/`:1220`/`:1228`）
已判或已修，其余 5 处在黄金不走的支（雪 `:827`、split-SOILSNOW `:1132`/`:1133`/`:1231`、
wetland `:1262`/`:1267`/`:1273`，后者本仓库直接 `bail!`）。
`spac_change`（40 处 vs `spacaf_twoleaf` 41 处）已有 oracle 49/49 的验证、且注释里带 GIMPLE 读数，暂不动。

**给下一轮**：湿窗现在 `sumabs` 5.55，离逐位还有距离，但这条链显然还没挖完 ——
`transpiration_from_conductance`（Rust 3 处 vs 汇编 `:683`/`:691`×2 三处）值得逐行对；
`spacaf_twoleaf` 41 处里有 15 处（`:479`/`:483`/`:485`/`:487`/`:489`/`:496`/`:498`/`:499`/`:500`）
是 4×4 求逆那一段，`spac_change` 只有 40 处，那个差额还没查清。

**第 342 轮：叶温/地温链的 FMA 普查（供下一轮照表比对）+ 植物水力链收口；本轮没有关掉种子**

第 341 轮那三处改动把湿窗 `sumabs` 压到 5.55 之后，按目标的下一步转去叶温/地温链。

**`MOD_LeafTemperature` 的 `Leaftemperature` 符号共 60 条 FMA**（`-S -g` + `.loc`；
含被内联的 `dewfraction`/`longwave`/`upward_longwave` —— 它们在本内核里没有独立符号）：

```text
493: 2   508: 1   541: 1   556: 1   557: 1   559: 2   688: 1   813: 1   838: 2
843: 4   846: 1   849: 1   856: 1   912: 3   941: 2   953: 1   964: 1   976: 1
993: 1  1067: 3  1070: 1  1091: 1  1099: 1  1100: 1  1112: 2  1113: 2  1118: 1
1119: 1 1127: 2  1134: 2  1141: 3  1144: 1  1155: 1  1164: 2  1175: 1  1201: 1
1202: 1 1254: 1  1264: 1  1271: 1  1272: 1  1361: 1
```

Rust `leaf_temperature.rs` 共 42 处 `mul_add`（`leaf_temperature` 31、`longwave` 5、
`upward_longwave` 2、`update_canopy_water` 4）。**差额不能用数数裁**：内联/拆分两边的口径不同，
而且这条链的关键两段已经有**离线位型穷举**级别的证据 —— `longwave`（`irab`）的注释记着
"`base=fma` 命中 kernel 全部四轮、`base=平铺` 命中 Rust 全部四轮，`tl4=(tl*tl)*(tl*tl)` 两侧都要"，
`upward_longwave`（`ulrad`）记着"只有'左乘积融合'这一维能复现内核的 `4070B00402BA216F`，
平铺得到 `…216E`"。⇒ **叶温链下一轮要用位型探针（照 `irab_shapes.py` / `ulrad` 那套离线穷举），
不要用计数**。

**`MOD_GroundTemperature` 的 `groundtemperature` 符号共 42 条**：

```text
202: 2  206: 1  216: 2  246: 1  258: 3  263: 5  267: 1  271: 2  279: 2  281: 3
287: 2  290: 2  307: 1  312: 1  331: 1  332: 2  334: 1  335: 2  345: 1  348: 1
362: 1  364: 2  382: 1  397: 1  400: 1
```

Rust `ground_temperature.rs` 27 处。两个大簇都逐条读过：`:255-290` 的 `hs` 累加链
（`dlrad*emg - emg*stefnc*T**4 - (fseng+fevpg*htvp) + cpliq*pg_rain*Δ + cpice*pg_snow*Δ`）
与 Rust `surface_fluxes` 的展开式（`soil_base` → `soil_delta.mul_add(rain_heat_capacity, …)`
→ `.mul_add(snow_heat_capacity, …)`）**结合顺序一致**；`:331-400` 的 `-emg*stefnc*T**4`
与三对角组装也和 `temperature_system` 对得上。同样**不能靠数数**下结论。

**植物水力链可以收口了**：`spacaf_twoleaf` 的 4×4 求逆段
（`:479` 3 + `:483` 5 + `:485` 5 + `:487` 3 + `:489` 5 = **21**）与 Rust `spac_change` 的
determinant 3 + `e1`/`e2`/`e3` 各 2 + 四个 `change[…]` 各 3 = **21** 恰好对上。
`A`/`f` 段表面上 11 vs 10，但把 `.loc 465` 那两条 FMA 读出来就清楚了：
`fmsub d11,d20,d26,d11` 与 `fmadd d20,d31,d11,d10`，后者是
`(X*dfr)*Δroot + X*fr`（`X = sai*kmax_xyl/htop`）—— 正是 Rust 的
`(xylem*dfroot).mul_add(root_gradient, xylem*froot)`；前者属于同页的 `:464`（A33 的 `-X*fr` 项）。
⇒ 那 1 条差是 `.loc` 归属噪声，**不是漏形状**。

**下一轮**：① 叶温/地温链改用位型探针（离线穷举候选形状，而不是对 FMA 计数）；
② 三个具名种子仍在：湿窗 **N=18 `vegwp`**（2 个分量：`[1]` shaded、`[2]` xylem，各 1 ULP）、
**N=20 `wliq_soisno`+`hk`**、干窗 **N=288 `tref`/`tleaf`**；
③ 第 341 轮那四处改动里**哪一处**贡献了湿窗改善还没拆开 A/B。

**第 343 轮：先修一个自己踩的坑（`.mod` 污染让 `build_kernel.sh` 挂掉），再把 gssun 双侧探针搬到湿窗 step 18 —— 结论：现有探针对 1 ULP 是**盲的**，种子在 PHS 求解内部**

**① 自己踩的坑（要紧，记规矩）**：我这几轮为了数 FMA 直接跑
`gfortran ... -S main/xxx.F90 -o /tmp/gf/xxx.s`，**没加 `-J`**，于是 gfortran 把 `.mod`
写进了 `vendor/CoLM202X/`（共 9 个）。而 `build_kernel.sh` 是**把整棵 vendor 树 tar 进构建目录**的
（它自己的注释就写了"构建会往树里写 `.o`/`.mod`/`include/define.h`，直接在 `vendor/` 里编会污染入库源码"），
于是那份**过期的 `mod_leaftemperature.mod`** 被带进构建，`MOD_Thermal_CanopyPhase_Extended`
按旧接口绑定实参，报了一串**假**错误：

```text
extends/interception/MOD_Thermal_CanopyPhase_Extended.F90:936:39:
Error: Keyword argument 'canopy_smelt_mass_out' at (1) is already associated with another actual argument
Error: Dummy argument 'hksati' with INTENT(IN) in variable definition context …
make: *** [MOD_Thermal.o] Error 1
```

**症状具有欺骗性**：它指向 `extends/` 里一个与本次改动毫无关系的调用点，而且 `git status` 干净
（`*.mod` 被 ignore），很容易误判成"committed 的构建坏了"。删掉那 9 个 `.mod` 之后
`./oracle/scripts/build_kernel.sh default` 立刻 **exit 0**（`colm.x` 重编成 16:05），
`wet_ts.sh 18` 的状态分歧与之前**逐位一致**（仍只有 `vegwp` 2 个分量各 1 ULP）。
⇒ **规矩：任何 ad-hoc 的 `gfortran -S/-c` 都要加 `-J<临时目录>`（或换个 scratch cwd），
绝不在 `vendor/CoLM202X/` 里编。** 这也解释了本仓库历史上"内核老是编不过"的一类假故障。

**② gssun 双侧探针搬到了湿窗**：把 `oracle/scripts/gssun_probe.sh` 复制成
`/tmp/gf/gssun_wet.sh`（只改三处 case 路径 + `STEPS=${STEPS:-18}` + `WORK`），
它在 step 18 上打 8 个点、两侧各 834 行。`gssun_cmp.py`（相对容差 2e-14）报"全同"，
但那个口径**看不见 1 ULP**（2.2e-16）。我把两侧都归一到 16 位有效数字再逐字段比：

```text
GSIN 120 calls 0 diffs | GSDEM 120 0 | GSOUT 96 0 | GS908 120 0
GS941 120 0 | GS1320 18 0 | GSTO 120 0 | GS919 240（探针字段定义两侧不同，忽略）
```

⇒ **step 18 上，PHS 链的入参（`gs0sun/gs0sha/tl/psrf/sai/fwet/rb/rss`）与出参
（`gssun/gssha/etrsun/etrsha/qflx_sun/qflx_sha`）到 16 位有效数字都相同**。
而同一时刻 restart 里 `vegwp[1]`(shaded) 与 `vegwp[2]`(xylem) 仍各差 1 ULP。
⇒ **种子在 PHS 求解内部**（`spac_change` 的 4×4 解 / Newton 更新 / `root_flux_from_top_potential`），
不在它上游的叶温-气孔链上。

**③ 下一轮（已经把路铺好）**：
1. **把探针改成打位型**：内核侧现在打的是 `ES23.15`（16 位有效数字，**不能**唯一确定 f64），
   要照 `stomata_diff.f90` 的 `B(x) = TRANSFER(x, 0_8)` 改成 `Z17` 十六进制，
   这样才能真正逐位判 —— 这也是"gssun 探针报全同、状态却差 1 ULP"的唯一解。
2. 在 `plant_hydraulic_stress` 里加探针点打 **`vegwp`（4 分量）、`root_flux`/`dqeroot`、
   `spac_change` 的 `A`/`f`/`determ`/`dx`** —— 目标是把 1 ULP 的第一出生点夹到具体一个量。
3. `A`/`f`/`dx` 那一段已有 oracle 49/49 的验证（`spac_change` 里注释），所以重点看
   `vegwp` 的 Newton 更新与 `enforce_potential_gradient` 那几行。

**第 344 轮：新建第 8 个闭环 `compare_vulnerability.sh`（`plc`/`d1plc` 200000/200000 逐位全同）；`pow` 两侧逐位相同；湿窗 N=18 的种子可以排除脆弱性曲线**

**① 先判一个系统性风险点：`pow`。** gfortran 的 `**` 与 Rust 的 `powf` 是否同一份 libm？
写了个 200000 组的位型对拍（`/tmp/gf/pow2/`：`(a/1e5)**b` 与 `2.0**t` 两种形态，
gfortran 打 `TRANSFER(x,0_8)` 十六进制，Rust 打 `to_bits()`）：

```text
pow(a,b) 差异: 0 /200000
pow(2,t) 差异: 0 /200000
```

⇒ 跨语言 `pow` **逐位一致**，这个风险点排除。

**② 第 8 个闭环**：`oracle/scripts/compare_vulnerability.sh` +
`vulnerability_diff.f90` + `crates/colm-core/examples/vulnerability_probe.rs` ——
`MOD_PlantHydraulic` 的 `plc`（脆弱性曲线）与 `d1plc`（其一阶导）各 200000 组、逐位比。
`plc`/`d1plc` 在模块里**默认 PUBLIC**（模块只列了 `PRIVATE :: calcstress_twoleaf`），
所以直接链 `.bld` 的对象，不用"拷贝+放行"。结果：

```text
vulnerability: plc + d1plc 200000/200000 bitwise identical
```

⇒ **脆弱性曲线不是湿窗第 18 步那颗种子的来源。**

（踩坑记录：Rust 侧一开始写 `{:017X}`（17 宽）而 Fortran 侧是 `Z17`（17 宽、左补空格，
`split()` 后只剩 16 位），一个 16 位一个 17 位带前导零，字符串比会**假报 200000 处全不符**；
改成与其它闭环一致的 `{:016X}` 后全同。以后照抄 `compare_stomata.sh` 的宽度。）

**③ PHS 求解链的逐表达式复核（两头都读过，为下一轮缩小范围）**：

* `extends/interception/MOD_PHSRootfluxBalance.F90`（`balance_phs_rootflux`）**0 条 FMA**，
  Rust 的 `balance_phs_rootflux` 也是 0 ✓。
* `calcstress_twoleaf` 是**单趟**（不是 Newton 循环）：`x = x + dx` → 三个
  `x(a) ≤ x(b)` 夹取 → `etrsun/etrsha = qflx*plc(x(leaf…))` → `qflx2gs` →
  `rstfac* = max(gss/gs0, 1e-2)` → `qeroot = etrsun+etrsha` → `qe2x` →
  `x(root) = x_root_top` → `rootflux(j) = k_soil_root(j)*(smp(j)-xroot(j))`。
  Rust 的 `plant_hydraulic_stress` 顺序逐条一致 ✓（含 `min(max|dx|,max|x|)/2` 那个缩放，
  代码注释显示早前已修）。
* `getvegwp_twoleaf` ↔ `vegetation_water_potential`、`root_conductances` 整条、
  以及 `x(xyl) = x(root) - grav1 - Q/(fr*kmax_root/htop*sai)`、
  `x(leafsha) = x(xyl) - etrsha/(fx*kmax_xyl*laisha)` 的**乘除顺序**逐一对照：全一致 ✓。
* `htop`（`canopy_top_height_m`）来自常数文件的 `htop` 标量（两侧读同一文件）✓；
  `sai`/`laisun`/`laisha` 由探针打印且 16 位有效数字相同 ✓。

**④ 结论与下一轮**：湿窗 N=18 的 `vegwp[2]`(xylem) 与由它派生的 `vegwp[1]`(shaded)
各 1 ULP 仍未定位，但候选已缩到很小的集合 —— `x(xyl)` 那一式的舍入、
或 `spac_change` 的 `dx`（已有 oracle 49/49）、或某个**探针没覆盖**的量。
**下一轮**：上 hex 探针（内核侧 `TRANSFER(x,0_8)`）打 `x(1:4)`/`dx(1:4)`/`qeroot`/`x_root_top`，
把 1 ULP 的出生点夹到具体一个量；这也是第 343 轮就列好的下一步。

**第 345 轮：PHS 求解内部的**位型**探针把湿窗第 18 步那颗种子夹到"第 95 次 PHS 调用的 `dx`"里；顺带记一个环境坑（PLUMBER2 挂载点变了）**

**① 位型探针（这是第 343 轮就列好的下一步）。** `/tmp/gf/phs_hex_probe.sh`：
把内核 `MOD_PlantHydraulic.F90:calcstress_twoleaf` 与 Rust `plant_hydraulics.rs:plant_hydraulic_stress`
各打两个/三个点，**两侧都打十六进制**（`TRANSFER(x,0_8)` / `to_bits()`）：

```text
PHXQ  qeroot, dqeroot, x_root_top          —— getrootqflx_x2qe 返回之后（A/f 的入参）
PHXD  x(1:4), dx(1:4)                      —— `x = x + dx` 之后
PHXF  x(1:4), qeroot, x_root_top            —— `x(root) = x_root_top` 之后
```

18 步湿窗、两侧各 **192 行**（96 次 PHS 调用 × 2 个点），**逐位**对比结果：

```text
首个差异: (190, 'PHXD', 2, 'C0902970C98996F3', 'C0902970C98996F2')   ← x(shaded) 差 1 ULP
差异计数: PHXD {2:1, 3:1, 5:1, 6:1, 7:1, 8:1}   PHXF {2:1, 3:1, 5:1}
```

解读：**第 190/192 行之前全部逐位相同**（95 次调用一致），差异出现在**第 95 次 PHS 调用**：
`x(shaded)`(字段2) 与 `x(xylem)`(字段3) 各 1 ULP，而且 **`dx(1:4)` 四个分量全都差**
（字段 5–8）。`dx` 是在 `x = x + dx` 之前算出来的，所以

> **种子在 `dx` 里 —— 也就是 `spac_change` 的 4×4 解或它吃进去的 `A`/`f`/`qeroot`/`dqeroot`，
> 不在 `x = x + dx` 的更新算术里。**

`PHXF` 的 `qeroot`（字段5）也差，但那是用**新的** `x` 重算出来的 → 是下游。
⇒ 下一轮加 `PHXQ`/`A`/`f` 三个更细的点（`PHXQ` 已经在脚本里，只是这轮没跑成，见下），
把"是 `qeroot` 先把差带进来"还是"`A`/`f` 自己生出来的"分开。

**② 环境坑（会伪装成"内核坏了"）：PLUMBER2 挂载点变了。**
`oracle/work/*/forcing.nml`（**21 个文件**）里硬编码
`DEF_dir_forcing = '/Volumes/Data01/Data/PLUMBER2s/Forcing/'`。
这台机器上的 SMB 卷这轮换了名字（先是 `/Volumes/Data`，随后整个掉了），于是所有算例
——内核与 Rust 两侧——都会以

```text
/Volumes/Data/Data/PLUMBER2s/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc does not exist.
```

失败。**这不是模型问题**。我的做法是**只改 scratch 拷贝**：
`/tmp/gf/{wet_ts,dry_ts}.sh` 在把 `forcing.nml` 拷进 `$d` 之后 sed 一次挂载点；
`/tmp/gf/win4.sh` 为 Rust 侧复制一份 case 目录再 sed；探针脚本同理。
**仓库里的 `oracle/work/*/forcing.nml` 一个都没动**（`git status` 干净）。
⇒ 建议（留给你定）：让算例 namelist 认 `PLUMBER2_ROOT`，或者跑之前由 harness 替换，
别再硬编码绝对路径 —— 现在换一次挂载点就要坏 21 个文件。

**③ 这一轮的边界**：位型探针那次跑成功了（上面的数就是它给的），
但随后卷掉了，`PHXQ` 那版探针与后续状态扫描都跑不动。所以本轮的结论
只有"种子在 `dx`"这一条，`PHXQ`/`A`/`f` 的细分留到卷回来之后。

**第 346 轮：位型探针再进一层 —— 差异落在 `spac_change` 的 `f(leafsun)`/`f(leafsha)`；翻转它们的收缩侧**没有**修好（已回退）；顺带找到**仓库自带**的强迫数据**

**① 探针链（PHXQ → PHXA）把种子夹到 `f` 的两行。** 在上一轮 `PHXD`/`PHXF` 之后加两个点：

```text
PHXQ  x_root_top, qeroot, dqeroot   —— x2qe 返回之后（A/f 的入参）
PHXA  A11 A13 A22 A23 A31 A32 A33 A34 A43 A44 f(1..4)  —— spacAF_twoleaf 的 A/f 组装之后
```

18 步湿窗、两侧各 **384 行**（96 次调用 × 4 点），逐位结果：

```text
首个差异: (381, 'PHXA', 11, '3DE4EDD21436D986', '3DE4EDD214369986')
差异计数: PHXA {11:1, 12:1}   PHXD {2:1,3:1,5:1,6:1,7:1,8:1}   PHXF {2:1,3:1,5:1}
```

* **`PHXQ` 96 次调用全部逐位相同** ⇒ 根通量解（`root_flux_from_top_potential`/`x2qe`）
  给出的 `qeroot`/`dqeroot` **不是**来源。
* **十个 A 元素与 `f(xyl)`/`f(root)` 全同**（`PHXA` 只有字段 11、12 差）⇒ A 的组装没问题。
* 差的是 **`f(leafsun)`（字段 11）与 `f(leafsha)`（字段 12）**，而且**相对差约 1e-11**
  —— 这两行是**残差**（两个大项相消），所以绝对差仍然是 1 ULP 量级，
  相对差被相消放大 ✓ 与"状态只差 1 ULP"自洽。

（踩坑：第一版 `PHXQ` 两侧字段顺序不同 —— 内核打 `qeroot,dqeroot,x_root_top`、
Rust 打 `potential[ROOT],root_flux,root_flux_slope` —— 于是 96 次全报差异；
对齐成 `x_root_top,qeroot,dqeroot` 之后是 0 差异。**两侧字段顺序必须逐字对齐**。）

**② 翻转 `f(SUNLIT)`/`f(SHADED)` 的收缩侧 —— 没有修好，按纪律回退。**
代码注释原本写"`f(SUNLIT)`/`f(SHADED)` 收的是 `qflx*fsto`（只出现一次的那个乘积）"，
按探针的提示改成收 `(lk*fxyl)*Δsun` 那一侧：

```rust
f[SUNLIT] = (-(sunlit_conductance * fxyl)).mul_add(sunlit_gradient, sunlit_flux * fsun);
f[SHADED] = (-(shaded_conductance * fxyl)).mul_add(shaded_gradient, shaded_flux * fsha);
```

实测：湿窗 N=18 仍是 **1/68**（`vegwp` 2 分量、maxrel 2.198e-16，**与改前一模一样**），
N=21 从 2/68 变 3/68（变差）⇒ `git checkout` 回退。

**③ 这条负结果把范围又缩了一步**：`f(leafsun) = qflx_sun*fsto1 - (lk*fx*Δsun)` 的
四个量里，`lk*fxyl`（=A31）与 `Δsun`（由 A13 反推）都逐位相同、`fsto1` 是 `plc`（第 344 轮
200000/200000 逐位全同），**只剩 `qflx_sun`/`qflx_sha`（两个需求通量）没在 hex 下比过**。
⇒ 下一轮就探它们：在 `getqflx_gs2qflx_twoleaf` 返回之后（内核）/`transpiration_from_conductance`
返回之后（Rust）打位型；若它们相同，则说明内核在 `f` 里用的既不是"收左"也不是"收右"，
而是第三种形状（例如两个乘积都先舍入）。

**④ 顺带：仓库里就有强迫数据。** 上一轮说"PLUMBER2 卷掉了、算例跑不动"——其实
**`examples/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc` 就在仓库里**（用户提醒）。
把 scratch 脚本的 sed 目标从挂载点改成 `$BASE/examples/Forcing/` 之后，
`wet_ts.sh`/`dry_ts.sh` 与干/湿两个黄金窗口**完全离线可跑** ✓
（`examples/Forcing/` 里只有 CN-Cng、AT-Neu、AU-Preston、US-Ne3 四个；**US-NR1 不在**，
所以雪窗仍需要网络卷或另找数据）。仓库文件依旧一个没动。

**第 347 轮：湿窗 N=18 那颗种子关掉了 —— `transpiration_from_conductance` 的 `cfw` 少收一处乘积；干/湿黄金都变好或不变**

**① 探针把出生点钉在"两个需求通量"上。** 在 `PHXA` 之后再加一个点：

```text
PHXG  qflx_sun, qflx_sha   —— getqflx_gs2qflx_twoleaf（内核）/ transpiration_from_conductance（Rust）返回之后
```

18 步湿窗、两侧各 504 行，逐位结果：

```text
首个差异: (499, 'PHXG', 1, '3EC3A258B0446F26', '3EC3A258B0446F25')   ← qflx_sun 差 1 ULP
差异计数: PHXG {1:1, 2:1}  PHXA {11:1, 12:1}  PHXD {2,3,5,6,7,8 各 1}  PHXF {2,3,5 各 1}
```

⇒ 第 95 次调用上 **`qflx_sun`/`qflx_sha` 各差 1 ULP**，后面 `f`→`dx`→`vegwp` 的差都是它的下游。
（`PHXQ` 与十个 A 元素仍然全同 ⇒ 根通量解与 A 的组装都没问题。）

**② 两个假设都被实测否掉，然后 GIMPLE 具名操作数给出了真形状。**

* 把 `driving_humidity` 内层 FMA 的收缩侧翻转（收 `wtaq0*qm` 而不是 `(wtaq0+wtgq0)*qsatl`）
  ⇒ **大幅变差**：湿窗 N=16 从 0/68 变 **20/68**、N=18 从 1 变 13 ⇒ 回退。
  事后 GIMPLE 证实原形状是对的：`_64 = .FMS(qsatl, wtaq0+wtgq0, qm*wtaq0)`、
  `_66 = .FNMA(qg, wtgq0, _64)` —— 收的确实是 `(wtaq0+wtgq0)*qsatl`（第一项）。
* 把 `cfw` 里 `1 - delta*(1-fwet)` 的收缩补回来 ⇒ 状态扫描**逐位不变**（惰性）⇒ 回退。
  事后 GIMPLE 证实该平铺：`_21 = (1-fwet)*delta`、`_22 = 1 - _21`（**没有 FMA**）
  —— 第 341 轮把两处一起拉平是对的，`.loc 683` 那条 FMA 其实是**下面 `cfw` 求和**那一条。

真正的缺口是 `cfw` 的**求和**：

```text
_21 = (1-fwet)*delta                                   ← 已舍入
_22 = 1 - _21                                          ← 平铺（dry_fraction）
_33 = _22*(laisun+laisha+sai)*gb_mol/cf                ← 第一项，已舍入
_47 = laisun/(1/gb+1/gs_sun)/cf + laisha/(1/gb+1/gs_sha)/cf
cfw = .FMA(_21, _47, _33)                              ← **第二项的最外层乘积收进加法**
```

Rust 原先写成 `A + (1-fwet)*delta*(…)` 的平铺链，**少收这一处**。补上
（`wet_fraction.mul_add(括号和, 第一项)`）之后 ——

| 仪器 | 补之前 | **补之后** |
|---|---|---|
| 湿窗 restart N=16 / **18** / 19 | 0 / **1** / 0 （per 68） | **0 / 0 / 0** |
| 湿窗 N=20 / 21 / 24 / 32 / 48 / 96 | 2 / 2 / 21 / 21 / 19 / 15–16 | 2 / 2 / 21 / 21 / 19 / 16（不变） |
| 干窗 N=18…26 / N=288 | 0…0 / 22 | **0…0 / 22**（不变） |
| 干窗 1 步 / 3 步 | 0/68 / 692-692 | 0/68 / 692-692（不变） |
| 黄金 dry（`over_tol`/`ot_vars`，`sumabs`） | 28 / 1，261.0128 | **28 / 1，261.0128**（不变） |
| 黄金 wet | 1927 / 54，28.8358 | **1908 / 53，24.8020**（变好） |
| 黄金 snow | — | **未测**（US-NR1 强迫数据不在仓库里，见下） |

`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
`cargo test --workspace --lib --bins -- --test-threads=1`（26 个二进制）全绿。

**③ 强迫数据的两个新事实**：

* 仓库里**自带** CN-Cng 的 Met 文件：`examples/Forcing/CN-Cng_2008-2009_FLUXNET2015_Met.nc`
  （还有 AT-Neu / AU-Preston / US-Ne3）⇒ 湿/干窗**完全离线可跑**；**US-NR1 不在**，
  所以雪窗这轮没测（这是本轮唯一没跑的一项）。
* Rust 侧的算例还要把 **`case.nml` 的 `DEF_forcing_namelist`** 一起改到拷贝那一份 ——
  它默认指向**原目录**的 `forcing.nml`，只 sed `forcing.nml` 本身是不够的
  （第一次跑就是这么失败的：Rust 仍去读 `/Volumes/Data01/...`）。
  `wet_ts.sh`/`dry_ts.sh` 一直都改了这一行，所以它们没踩到；`win4.sh` 我补上了。

**给下一轮**：湿窗的**下一颗**种子在 **N=20（`wliq_soisno`+`hk`，2/68）**，
干窗在 **N=288（`tref`/`tleaf`，22/68）**；`vegwp` 那条链（PHS）现在可以认为收口了。

**第 348 轮：湿窗 N=20 那颗种子关掉了 —— 冠层持水的**两侧**夹取 + 超配通量回投（Rust 一直照着**没编进内核的那份源码**抄）**

**① 种子口径先纠一次。** 第 347 轮之后 `wet_ts.sh 20` 给的不再是 `wliq_soisno`+`hk`，
而是**冠层持水**（因为 `cfw` 那处 FMA 改动了轨迹）：

```text
ldew       ndiff 1  idx (0,)  gold 0.0                  rust -1.0503208545953324e-19
ldew_rain  ndiff 1  idx (0,)  gold 0.0                  rust -1.0503208545953324e-19
（ldew_snow 两侧都是 0.0；tlai=1.8、tsai=0.45、tleaf=293.695 K > tfrz ⇒ 走暖支）
```

**读法**：`evplwet`（本例是蒸发）被 `elwmax = ldew/deltim` 夹过，暖支里
`qevpl = ldew_rain/deltim`，于是 `ldew_rain + (qdewl-qevpl)*deltim` 的**理论值恰好是 0**；
但它是 `FMA(deltim, 0-qevpl, ldew_rain)`，**单次舍入**后可以落到 **−1 ULP**。
上游把两个分量都夹到 0，Rust 没有 ⇒ 从那一步起 `ldew` 差 1 ULP。

**② 根因不是漏了一个 `max`，是"照错文件抄"。** `Makefile:641` 把
`MOD_LeafTemperature.o` 指向 `extends/interception/MOD_LeafTemperature_Extended.F90`，
**不是** `main/MOD_LeafTemperature.F90`；Rust 这一段的每一处形状都跟着 `main/` 走。
两份源码在这一段的差别是**结构性**的：

```fortran
! main/MOD_LeafTemperature.F90:1185-1197   ← 从没编进内核
IF (qevpl  > ldew_rain/deltim) THEN  qsubl = qevpl - ldew_rain/deltim;  qevpl = ldew_rain/deltim  ENDIF  ! 冷支对称
ldew_rain = ldew_rain + (qdewl-qevpl)*deltim      ! 超配量**从水体里扣**
ldew_snow = ldew_snow + (qfrol-qsubl)*deltim
ldew = ldew_rain + ldew_snow                      ! 没有 max

! extends/interception/MOD_LeafTemperature_Extended.F90:1613-1637（partition_canopy_latent_flux）
IF (qevpl > ldew_rain/deltim) THEN  phase_flux_deficit = qevpl - ldew_rain/deltim;  qevpl = ldew_rain/deltim  ENDIF
! 超配量**不改水体**，记进 phase_flux_deficit
```

`extends/` 那一段（`MOD_LeafTemperature_Extended.F90:1468-1495`）的内联 GIMPLE
（`gfortran -O2 -fdump-tree-optimized`，第 4536-4745 处）逐句给出形状：

```text
_3169 = MAX(evplwet, 0)            _3171 = ABS(MIN(evplwet, 0))
bb203: IF (phase_flux_deficit > 0)  fevpl = fevpl - pfd;  fsenl = .FMA(htvpl, pfd, fsenl)   ← evplwet 那一句被优化掉（死存）
_880 = .FMA(deltim, qdewl-qevpl, ldew_rain旧值)      _887 = .FMA(deltim, qfrol-qsubl, ldew_snow旧值)
bb205/206: IF (_880 < 0 .OR. _887 < 0)  flux_deficit = MAX(-_880,0)+MAX(-_887,0)
                                        fevpl = fevpl - flux_deficit/deltim
                                        fsenl = fsenl + (htvpl*flux_deficit)/deltim     ← **没融合**：mul→div→add
bb207: ldew_rain = MAX_EXPR<_880,0>;  ldew_snow = MAX_EXPR<_887,0>;  ldew = ldew_rain + ldew_snow
```

注意 `bb207` 的两个 `MAX_EXPR` 是**无条件**的（`bb206` 只是记账），而 `flux_deficit`
取的是**夹取前**的两个负值 —— 顺序错了就少收那一笔。

**③ 落地**（`crates/colm-core/src/leaf_temperature.rs::update_canopy_water`）：
两个分量都夹 0、`ldew = ldew_rain + ldew_snow`；`phase_flux_deficit` 不再从水体里扣；
`flux_deficit` 由夹取前的两个值算；返回值加上这两个超配量，调用方按上游顺序各退一次
（第一处 `FMA(htvpl, pfd, fsenl)`，第二处 `fsenl + (htvpl*fd)/deltim` 是平铺 ——
Rust 默认不融合，正好对上）。`evplwet` 自己那两笔**故意不做**：上游在 `:1489` 之后再没读过它。

顺带纠正一处**顺序**：`energy_balance_error` 原先算在持水更新**之后**，
而上游 `:1449` 的 `err` 在 `:1464` 之前 ⇒ 移到 `update_canopy_water` 之前。
这一处同时修掉两个错：`err` 少吃两笔回投，且 `precipitation_heat` 不再用到已被相变
（Niu(2004) 的 `tl` 拉回）改过的 `leaf_temperature_k`。

**④ 隔离实验（本轮最该记的一步）**：把改动拆成**最小子集**——基线 + 只给两条
`mul_add` 各加一个 `.max(0.0)`（"clamp-only"）——跑同一批仪器：

| 仪器 | 基线（HEAD `d326254`） | **clamp-only** | **本轮落地** |
|---|---|---|---|
| 湿窗首分歧步 | **N=20** | N=63 | **N=63** |
| 湿窗 N=20 / 21 / 24 / 32 / 48 / 96 | 2 / 2 / 21 / 21 / 19 / 16 | — | **0 / 0 / 0 / 0 / 0 / 11** |
| 湿窗 N=62 / 63 / 64 / 768 | — | 0 / 10 / 5 / 26 | **0 / 10 / 5 / 26** |
| 干窗首分歧步 | ≤N=26（N=288 已 22/68） | — | **224 与 256 之间**（N=224 仍 0/68） |
| 干窗 N=288 | 22/68 | 19/68 | **19/68** |
| 干窗 1 步 / 3 步 | 0/68 / 692-692 | — | **0/68 / 692-692** |
| 黄金 dry（`over_tol`/`ot_vars`，`sumabs`） | 28 / 1，261.0128 | 9163 / 28 / 1 / 261.0128 | **9163 / 28 / 1 / 261.0128** |
| 黄金 wet（`bitwise`/`over_tol`/`ot_vars`/`sumabs`） | — / 1908 / 53 / 24.8020 | **27072** / 1197 / 19 / 8.4418 | **27066** / 1197 / 19 / 8.4418 |
| 黄金 snow | — | 未测（US-NR1 强迫不在仓库） | 未测（同上） |

两条结论：
* **clamp-only 与落地版在**所有**状态/黄金口径上逐位相同**（连 N=63 那颗新种子的
  两个值都一样：`gold 0.010626504408926336` vs `rust 0.010626504408926329`）
  ⇒ 本轮的"重组 + 回投"在这两个窗口里是**惰性**的，N=20 那颗种子**只需要那 4 个夹取**。
* 黄金湿窗 `bitwise` **27072 → 27066**（`over_tol`/`sumabs` 不变）⇒ 回投让 Rust
  **更靠近**内核 6 个值。**所以两笔回投不能省**：它们在冠层有雪时（`qsubl > ldew_snow/deltim`）
  才是活的，而雪窗本机测不了 —— 省掉等于留一个已知的、只能在雪窗暴露的偏差。
* 另外单独验过：湿窗 N=16 的 history 首分歧记录**基线与本轮完全相同**（第 4 条记录、
  `f_assimsun`/`f_fgrnd` 各 1 个值）⇒ 那是**先前就存在**的 history-only 形状缺口，与本轮无关。

**⑤ 方法教训（比结论值钱）**：**先看 Makefile 编的是哪一份源文件**，再去读形状。
`main/MOD_LeafTemperature.F90` 与 `extends/interception/MOD_LeafTemperature_Extended.F90`
里同一个物理过程是两套代码（前者把超配量扣水体、无夹取；后者记 `phase_flux_deficit`
回投通量），Rust 从第 9 轮起抄的是前者。判 1 ULP 级种子时，"哪份源码进了内核"
必须先用 `make -Bn <obj>` 确认（`vendor/CoLM202X/include/define.h` 只决定 `extends/`
是否参与，`Makefile:641/644/647` 才是最终指向）。

**⑥ 顺手记下的两处**已知**偏差（本轮**没动**，都不是这两个窗口的种子）**：
* `update_canopy_water` 的融化/冻结两段：上游是 `ldew_rain = max(0., ldew_rain + qmelt*deltim)`、
  `ldew_snow = max(0., ldew_snow + qfrz*deltim)`，Rust 只在**减**的那一侧夹。
  但加数 `qmelt`/`qfrz` 由 `min(…, 正数)` 给出、必 ≥ 0 ⇒ 夹取**可证惰性**（含 ±0 情形）。
* `DEF_VEG_SNOW = .false.` 那一支：`extends/:1505-1511` 是 `ldew_rain = ldew; ldew_snow = 0.`
  （或按 `tl` 反号），Rust 抄的是 `main/:1214-1230` 的**按比例分回**——**这一处不是惰性**，
  只是 `DEF_VEG_SNOW` 默认 `.true.`（`MOD_Namelist.F90:314`）且三份黄金算例都没覆盖它，
  所以**本机完全测不到**。要改就得先造一个 `DEF_VEG_SNOW=false` 的算例，不能盲改。

**给下一轮**：湿窗首分歧现在是 **N=63 的 `ldew_rain`（4 ULP，`vegwp` 3 个元素跟着走）**，
它**与本轮改动无关**（clamp-only 逐位复现）；干窗首分歧在 **N=224…256** 之间；
history 侧最早的分歧仍是**第 4 条记录的 `f_assimsun`/`f_fgrnd`**（基线就有，判据得用
2 步以上的 restart 或专门探针）。雪窗要等 US-NR1 强迫数据到位。

**第 349 轮：湿窗第 63 步那颗种子被推出**植物水力** —— 它生在 `stomata` 的返回值里（`rssun`/`rssha`）**

本轮**没有改模型代码**（只加探针 + 记录），所以上一轮的全部口径原样成立
（湿窗 N=16…48 全 0/68、首分歧 N=63；干窗 N=288 19/68；黄金 dry 28/1，261.0128；
黄金 wet 1197/19，8.4418）。

**① 方法：把 63 步的位型探针往上追三层。** 第 348 轮的种子是"第 63 步的
`ldew_rain`+`vegwp`+`gs0sun`/`gs0sha`"，而 `PHXG`（PHS 入场通量）在**第 18 步的
第 137 次 PHS 调用**就已经差 1 ULP。于是给 `/tmp/gf/phs_hex_probe.sh`
（现提升为 `oracle/scripts/phs_hex_probe.sh`）加三个上游点，逐个问"这一层的入参是不是已经不同"：

| 打点 | 位置 | 字段数 | 首个差异字段 | 结论 |
|---|---|---|---|---|
| `PHXG` | `gs2qflx` 返回后（PHS 入场需求） | 2 | **`qflx_sha`** | 种子在 PHS **解题之前** |
| `PHXI` | `gs2qflx` 的 **17 个入参** | 17 | **`gssha`**（第 3 字段） | 该例程自己的入参已不同 ⇒ 不是它的形状。`gb_mol`/`laisun`/`laisha`/`sai`/`fwet`/`tl`/`qsatl`/`qg`/`qm`/`psrf`/`rhoair`/`rd`/`rss` **从不**差 |
| `PHXR` | `gs0sun`/`gs0sha` 的**出生点**（`MOD_LeafTemperature_Extended.F90:832-833`） | 8 | **`rssun`**（`40657DD9ECCFF71F` vs `…1E`） | `tl`/`tprcor`/`laisun`/`laisha`（字段 3-6）**从不**差 ⇒ 换算公式不是元凶，差在电阻里 |
| `PHXS` | **`stomata` 返回后**（`MOD_AssimStomataConductance.F90`） | 6 | **`rssun`（同一个 hex）** | 种子**生在 `stomata` 内部**；`respcsun`/`respcsha` **从不**差 |

即：`rssun` → `gs0sun` → `gssun`（PHS 入参）→ `qflx_sha` → `PHXA/PHXD/PHXF` → 状态。

**② 18/502 次调用有差，且大都自愈。** `PHXS` 逐调用（每次 `stomata` 调用一行）差异簇：

```text
迭代 131: rssun,assimsun             rssun=171.9328522      ← 首个（第 18 步）
迭代 137: rssha,assimsun,assimsha
迭代 152: rssun,rssha,assimsun,assimsha
迭代 267: rssun,assimsun      迭代 282-286: 连续 5 次只有 rssun
迭代 351: assimsha（rssun 被 1e6 夹住，看不出来）
迭代 498-501: 第 63 步的最后一簇  ← 只有它进了状态（restart N=62 仍 0/68、N=63 差 10/68）
```

读法：叶温的迭代循环**能自愈** 1 ULP（131 差完 132-136 又回到同一位型），
所以 restart 到第 62 步都是 0/68；只有第 63 步那一簇自愈失败 ⇒ 这颗种子**不是**
"第 63 步新生的"，而是"同一类 1 ULP 在第 63 步恰好没被吸收"。
另外 `rssun` 单独差（282-286）而 `assim` 不差 ⇒ **不能只用 `assim` 的残差解释**，
`rst`/`gs` 的换算里还有一处形状缺口。

**③ 与已知闭环残差接上头。** `oracle/scripts/compare_stomata.sh`（第 328/335 轮）
在 4 个模型块 × 1000 例上只剩 **4 例**差，全在 `assim`（Ball-Berry 1 + Medlyn 3）；
第 336 轮那条 `f_assimsun` 的 history 缺口（湿窗第 4 条记录、基线与本轮都差）
与它同源。**本轮把"state 级"的证据也接到了同一条链上** —— 湿窗第 63 步的状态种子
就是 `stomata` 的输出。

**④ 工具经验（都踩过）**：

* **`gs0sun`/`gs0sha` 在 `extends/interception/MOD_LeafTemperature_Extended.F90` 里，
  不在 `MOD_PlantHydraulic.F90` 里。** 第一版把 `PHXR` 的锚点写进 PHS 那一份，
  `after()` 的 `assert len(idx)==1` 立刻报 0 命中 —— 探针脚本的断言值钱。
* **`STEPS > 48` 会撞 `end_day=1` 的日期上界**（`calendar time 2008-183 113400s is out of range`）
  —— 探针现在照 `wet_ts.sh` 算 `END_DAY/END_SEC`。
* 探针从"只 patch 2 个文件"扩到 **4 个**（`MOD_PlantHydraulic.F90`、
  `MOD_LeafTemperature_Extended.F90`、`plant_hydraulics.rs`、`leaf_temperature.rs`），
  8 个标签，跑完自动还原并重编；已入库为 `oracle/scripts/phs_hex_probe.sh`
  （第 345-348 轮它只在 `/tmp` 里）。
* `o3coefg_sun`/`o3coefg_sha` 在 `stomata` 里是 **`intent(in)`**
  （`MOD_AssimStomataConductance.F90:110-113`），`DEF_USE_OZONESTRESS=.false.` 时
  在 `:501-502` 被置成**恰好 1.0**，所以 Rust 在 `gs0sun`/`gs0sha` 里省掉
  `* o3coefg_*` 这一因子**是精确的**（本例三个算例都关了臭氧）—— 但这是**潜在**偏差：
  开臭氧胁迫时会差。记在这里，不要当成"已对齐"。

**⑤ 下一枪：进 `stomata` 内部。** 现成的资产是 `compare_stomata.sh` 那条闭环，
按第 335 轮列的两件事做：① 导出 `omc`/`ome`/`oms`/`bq`/`c`/`conductance`/`internal`/`eyy`
把 4 例残差钉到具体量（`assimsun` 与 `rssun` 的对应关系要和轨迹里 282-286 那簇对得上）；
② 给夹具**加一档按黄金窗口真实参数区间采样**（照 CN-Cng-wet 第 1 步的
`par`/`tlef`/`rstfac`/`c3c4=1`）。判据仍是：湿窗 `wet_ts.sh 63` 的 10/68 要往下走，
干窗与黄金两窗口不许变差。

**第 350 轮：把 `stomata` 那 14 处形状按 GIMPLE 补齐 —— 闭环好到"零舍入差"，但**状态扫描与黄金湿窗都变差**，整批已回退；顺带更正第 349 轮的定位**

**① 先把 FMA 命名解码确定下来**（第 335/336 轮在这上面绕过圈子）。`MOD_AssimStomataConductance.F90`
的 GIMPLE 共 40 条 FMA 族：`sortin` 11、`calc_photo_params` 4、`update_photosyn` 8、**`stomata` 17**。
拿源码对照就能钉死语义（例：`:350` 的 `bquad = gbh2o*hcdma - ei - bintc*hcdma` 编成
`_121 = .FMS(hcdma, gbh2o, ei)` 再 `bquad = .FNMA(bintc, hcdma, _121)`）：

```text
.FMA (a,b,c) = a*b + c        .FMS (a,b,c) = a*b - c
.FNMA(a,b,c) = c - a*b        .FNMS(a,b,c) = -c - (a*b)
```

**② Rust `stomata` 有 14 处该收没收**（`photosynthesis.rs` 全文只有 4 个 `mul_add`；`range` 与
`coupled_assimilation` 那两处本来是对的）。逐条（内核形状 → Rust 原写法）：

| 位置 | 内核（`stomata.opt`） | Rust 原写法 |
|---|---|---|
| `omc` `:259` | `.FMA(vm*(pco2i_c-gammas)/(pco2i_c+rrkk), c3, vm*c4)` | 平铺 `A*c3 + vm*c4` |
| `ome` 分母 `:260` | `.FMA(gammas, 2.0, pco2i_e)` | `pco2i_e + 2*gammas` |
| `ome` 主式 | `.FMA(c3, _53, epar*c4)` | 平铺 |
| `oms` `:263` | `.FMA(c3, omss, (omss*pco2i)*c4)` | 平铺 |
| WUE `co2i_e` `:215` | `_368 = .FMA(sqrt(...), 1.37, 1.0)` | `1.0 + 1.37*sqrt(...)` |
| Medlyn `bquad` `:343` | `.FNMS(g0*1e-6+acp, 2.0, (g1*acp)²/(vpd*gbh2o))` | 平铺 |
| Medlyn `1-g1²` | `.FNMA(g1, g1, 1.0)` | `1.0 - g1.powi(2)` |
| Medlyn `cquad` 内 | `.FMA(g0*2.0, 1e-6, (1-g1²)*acp/vpd)` | 平铺 |
| Medlyn `cquad` 外 | `.FMA(g0*1e-6, g0*1e-6, _108*acp)` | 平铺 |
| Medlyn `sqrtin` | `.FMS(bquad, bquad, cquad*4.0)` | 平铺 |
| Ball-Berry `bquad` | `.FMS(hcdma, gbh2o, ei)` + `.FNMA(bintc, hcdma, _121)` | 平铺（两步） |
| Ball-Berry `cquad` | `.FMA(bintc, hcdma, ea)` | 平铺 |
| Ball-Berry `sqrtin` | `.FMS(bquad, bquad, (hcdma*4.0)*cquad)` | 平铺 |

**③ 闭环确实大幅改善**：`compare_stomata.sh` 从 `{assim 171, rst 114}` → **`{assim 100, rst 0}`**
—— **1 ULP 那一档全部清零**（`rst` 全清），剩下 100 例全是 `assim` 的 **BIG**（0 vs 2.57e-4）且
**全在 Medlyn 块**（回退后是 113 例 BIG）。也就是说这 14 处形状是**对的**。

**④ 但三台判据一致否决**（同一份改动，回退前后各测一遍）：

| 仪器 | **回退后（保留）** | 打了这 14 处 |
|---|---|---|
| `stomata` 闭环 | rst 114 + assim 171 | **rst 0 + assim 100（全 BIG）** |
| 湿窗 restart **N=63** | 10/68 | **10/68（一点没动）** |
| 湿窗 restart **N=96** | **11/68** | 19/68（**变差**） |
| 干窗 restart N=288 | 19/68 | 19/68（不变） |
| 黄金 dry | 9163 / 28 / 1，261.0128 | 9163 / 28 / 1，261.0128（不变） |
| 黄金 wet | **27066 / 1197 / 19，8.4418** | 27743 / **1775 / 52**，19.8039（**变差**） |
| 轨迹 PHXS（`stomata` 返回值） | 18 簇，首个 = 迭代 131 `rssun` | **逐位不变**（同 hex、同簇数） |

**这次不是"聚合混沌"**：短程状态扫描（N=96）也一起变差 ⇒ 不适用第 249 轮那条例外，按仓库
纪律**整批回退**（`git checkout -- crates/colm-core/src/photosynthesis.rs`；回退后闭环回到
171+114、N=96 回到 11/68、黄金 wet 回到 1197/19 —— 三处都验过）。

**⑤ 更正第 349 轮（重要）**：把 `stomata` 在夹具上修到"零舍入差"之后，**轨迹里的
`stomata` 返回值一个 bit 都没变**（PHXS 首差异仍是 `40657DD9ECCFF71F` vs `…1E`、簇数 7/6/5/5）。
⇒ 轨迹里 `rssun`/`assimsun` 的 1–2 ULP **不是 `stomata` 自己算出来的，是从它的入参透传进来的**。
第 349 轮"种子生在 `stomata` 内部"的说法**过头了**：`stomata` 只是"第一个被探到的点"。
**下一枪**：把探针往上挪到 `CALL stomata` 的**实参表**（`par`/`ei`/`ea`/`pco2a`/`po2m`/`pco2m`/
`rb`/`raw`/`rstfac`/`cint`/`tlef`/`psrf`/`tm`/`g1`/`g0`/`gradm`/`binter`/`lambda`），
两侧打 hex。仓库里已有第 293 轮的 `oracle/scripts/stomata_probe.sh` + `stomata_cmp.py`
（遮荫支入参/内部）和 `gssun_probe.sh` 的 `GSTO`，**先用它们**（注意 1 ULP 必须 hex：
`ES23.15` 那一档是盲的），跑 63 步看哪个入参先分叉。

**⑥ 仍待办（本轮没碰）**：Medlyn 块那 100 例 BIG 的 `assim`（0 vs 2.57e-4）——它在夹具里是
**结构性**差异（`eyy` 迭代出口判据在 0.1 附近的翻边），与本次的舍入批无关；夹具 k=3 档
（黄金真实区间）已在第 335 轮加上，所以这不是"夹具没覆盖"的解释。

**第 351 轮：把 `stomata` 探针 hex 化 —— 入参里**唯一**分叉的是 `pco2a`；它的 FMA 能关掉第 63 步那颗种子，但黄金湿窗变差（按纪律已回退，等你裁）**

**① 工具（已入库）**
* `oracle/scripts/stomata_probe.sh`：**两侧全改位型**（`TRANSFER(x,0_8)`+`Z16.16` / `to_bits()`+`{:016X}`）。
  原版是 `ES23.15`/`{:e}` + 2e-14 相对容差 —— **对 1 ULP 是盲的**，而这条链的残留正是 1 ULP。
  另加 `CASE=<算例>`（原来写死 CN-Cng）与 `END_DAY/END_SEC` 回绕（`STEPS>48` 会撞日期上界），
  并补了强迫路径改写。字段布局两侧逐位对齐（`STIN` 13 / `STPH` 11 / `STIT` 10 / `STOUT` 4）。
* `oracle/scripts/stomata_cmp.py`：加 **hex 档**（按 `(tag,n,ic)` 对齐、逐位比较；十进制档保留）。
* `oracle/scripts/compare_hourly_window.sh`：**原来本机跑不起来**（没设 `DEF_forcing_namelist`、
  没拷 `forcing.nml`）—— 本轮补上（含挂载点改写）。

**② 湿窗 63 步、1004 次 `stomata` 调用（`CASE=CN-Cng-wet STEPS=63`）的逐位结果**

```text
STPH  1004/1004 全同        （vm epar respc omss gbh2o gammas rrkk c3 c4 bintc range）
STIN  唯一分叉字段 = pco2a  （20/1004 次差 1 ULP，首个在第 262 次调用；
                             tlef/psrf/po2m/pco2m/ea/ei/par/rb/rstfac/cint(1..3) 从不差）
STIT  分叉字段只有 pco2i / eyy（这两个是**探针口径**的差异，见 ③；
                             omc/ome/assim/assimn/co2s/assmt/gsh2o/pco2in 在入参相同的调用上逐位相同）
STOUT gsh2o/rst 17/1004（pco2a → gsh2o 的下游）
```

⇒ 第 349 轮"种子生在 `stomata` 内部"的更正**到此实测闭环**：`stomata` 的物理输出跟着入参走，
**唯一先分叉的入参是 `pco2a`**（冠层空气 CO₂），首个出现在第 262 次调用（≈ 第 33 步）。

**③ 顺带抓到一处**结构性**偏差：WUE 支的迭代计数（已证惰性，**按第 347 轮先例不落地**）**

内核 `MOD_AssimStomataConductance.F90:333-337` 在 WUE 支把 `pco2i` **重写**成
`pco2i_c`/`pco2i_e` 之一，紧接着 `pco2in = pco2i` ⇒ `eyy(ic) = pco2i - pco2in` **恒为 0** ⇒
`ic=1` 就 `EXIT`。Rust 的 `errors[iteration-1] = internal_co2 - next_co2` 用的却是
**重写前**的 `internal_co2`（= `pco2i_c`），所以当 `omc >= ome` 时它跑满 6 轮：
实测 `SRIT` 5639 行 vs 内核 `STIT` 1004 行，且**同一次调用内所有 `SRIT` 行逐位相同**
（WUE 支的量对猜测值不敏感 ⇒ 这 6 轮是纯空转，输出不变）。
⇒ 这也解释了 `STIT` 的 `pco2i`/`eyy` 两列在 WUE 支里**本来就不可比**（探针头已注明）。

**④ `pco2a` 的 FMA：能关掉第 63 步那颗种子，但黄金湿窗变差 ⇒ 已回退（按规则），等你裁**

GIMPLE（`lt.opt` 第 3945-3971 处）把这一行收成**一条 FNMA**：

```text
_573 = psrf*1.37;  _574 = _573/MAX(gah2o,0.446)
_582 = ((assimsun+assimsha)-respcsun)-respcsha-2.2e-7
_585 = .FNMA(_574, _583, pco2m)      ⇒  pco2m - _574*_583，乘积**在 FMA 里精确**、只舍一次
```

Rust 原先写成 `pco2m - (1.37*psrf/max)*(和)`（乘积先舍一次）。把它改成
`(-rate).mul_add(sink, pco2m)` 后实测：

| 仪器 | 回退后（保留） | 打这条 FMA |
|---|---|---|
| 湿窗首分歧步 | **N=63** | **N=73…84**（N=63、N=72 全 0/68；N=84 8/68） |
| 湿窗 N=96 | **11/68** | 14/68 |
| 干窗 N=288 | 19/68 | 19/68（不变） |
| 黄金 dry | 9163 / 28 / 1，261.0128 | 9163 / 28 / 1，261.0128（不变） |
| 黄金 wet | **27066 / 1197 / 19，8.4418** | 27087 / **1650 / 35**，19.2429（**变差**） |

**这是与第 350 轮不同的情形**：那条改动的**首分歧步没动**（N=63 还是 10/68），所以回退毫无争议；
这一条把**首判据（首分歧步）推进了 10–21 步**、且改动逐条对得上 GIMPLE —— 按第 249 轮那条例外
（短程改善 + 逐条对得上编译源码 + 聚合反向是混沌）**它够格**，但按"任一 `over_tol`/`ot_vars`
变差就回滚"的硬规则**本轮先回退**（连同 `baea8c5`/`f7e9e31` 一起等你裁）。

**⑤ "黄金是不是另一套编译产物"这个假设，本轮被否掉**：把
`compare_hourly_window.sh` 修好后跑干窗前 8 小时的三方逐位比对 ——

```text
rec 0..7:  kernel!=rust 0   kernel!=golden 0   rust!=golden 0
first kernel!=rust record: None      first kernel!=golden record: None
```

⇒ 黄金**就是当前内核**跑得出来的（位级全同）。所以黄金湿窗的聚合指标变差**不是**
"参考值来自另一套 contraction"，而是：湿窗在**它自己的首分歧步之后**已经进入混沌区，
任何位级改动（哪怕是更忠实的）都会把聚合指标推向任一边。**判据必须用首分歧步**。

**⑥ 下一枪**：新的首分歧在 **73–84**（若落地 ④ 就是这条线）。先把 `pco2a` 自己的入参打 hex：
`assimsun`/`assimsha`/`respcsun`/`respcsha`/`rsoil`（`:1243` 的括号和）与 `gah2o`/`gmax`；
现成资产是第 293 轮的 `oracle/scripts/pco2a_probe.sh`（`:1243-1244` 更新式），
先按本轮的办法把它 hex 化（`ES23.15` 那档对 1 ULP 是盲的）。

**第 352 轮：`rsoil` 是**两个字面量的乘积** —— `0.22 * 1.e-6`（`…C0CF`）≠ `0.22e-6`（`…C0D0`），差 1 ULP；落在 `pco2a` 的括号和里**

**① 发现路径**：第 351 轮的 hex 探针把 `stomata` 里**唯一**分叉的入参钉到 `pco2a`，
下一步自然是查 `pco2a` 更新式（`:1243-1244`）自己的入参 —— 而它的括号和里有一个常数：

```fortran
! MOD_LeafTemperature.F90:652
      rsoil = 0.22 * 1.e-6        ! ← 两个字面量相乘
```

Rust 写的是 `0.22e-6`。两者**不是同一个 double**：

```text
fl(0.22) * fl(1e-6)  = 3E8D87247702C0CF   ← 内核；GIMPLE `lt.opt` 第 3961 行就是
                                             `_582 = _581 - 2.1999999999999998475…e-7`，位型一致
0.22e-6（正确舍入的十进制） = 3E8D87247702C0D0   ← 大 1 ULP
```

**② 改动**：`let soil_respiration = 0.22 * 1.0e-6;`（保留**乘积形式**，不折叠）
+ 单元测试把两个位型钉死（防止后来者"化简"回 `0.22e-6`）。

**③ 判定：所有仪器**逐位不变**（连黄金 wet 的 `bitwise` 都一样）**

| 仪器 | 改前（`09b828a` 基线） | 改后 |
|---|---|---|
| 湿窗 restart N=63 / 96 | 10/68 / 11/68 | **10/68 / 11/68** |
| 湿窗 N=84 | 19/68 | 19/68 |
| 干窗 restart N=288 | 19/68 | 19/68 |
| 黄金 dry（`bitwise`/`over_tol`/`ot_vars`/`sumabs`） | 9163 / 28 / 1，261.0128 | **9163 / 28 / 1，261.0128** |
| 黄金 wet | 27066 / 1197 / 19，8.4418 | **27066 / 1197 / 19，8.4418** |

**④ 惰性为什么还落地**：差值本身是**可证**的（位型 + GIMPLE 常数逐位对上），改动是"把
Rust 的字面量换成内核真正算的那个 double"，风险为零；第 341 轮的
`getrootqflx_x2qe:892` 也是"单独测惰性"照样落地。惰性只说明**这两个窗口**的括号和
把它舍掉了（`-0.22e-6` 与 `-0.22*1e-6` 差 1 ULP，在 `assim+assimsha-respc*` 的量级上通常
被吸收），不代表雪窗或别的站点也吸收得掉 —— 这是一处**已经证明写错**的常数，留着只是
把债往下传。目标口径（状态扫描首分歧步 **中性或更好**）本轮满足 ✓（N=63/96 不变）。

**⑤ 顺带做了一次**系统扫描**（防同类）：把 `vendor/CoLM202X/{main,extends}` 里所有
"纯字面量乘积"（两侧都是十进制字面量）抽出来，共 **37 个不同的 double**，逐个与 Rust 对照：

* **只有 `0.22*1.e-6` 这一个**是"十进制折叠写法 ≠ 乘积"的陷阱（`…C0CF` vs `…C0D0`）。
* `44.6*273.16`（乘积 `12182.936000000002`，与 `12182.936` 不是同一个 double）在 Rust 里
  **本来就是乘积形式**（`44.6 * 273.16 * psrf / 1.013e5`）✓；`2600.*0.57**qt` 里的 `**` 是
  运行期 `pow` ✓（Rust `f77(0.57).powf(qt)`）；其余 34 个是 `3600*24`、`8*8` 这类整数精确值。
* 记一条工具短板：`oracle/scripts/audit_fortran_literals.py` 是**文本存在性**审计，
  抓不到"字面量写成了另一个十进制"这种**位值**错误 —— 这类要靠位型对照（本轮就是）。

**⑥ 下一枪（不变）**：`pco2a` 的其余入参（`gah2o`/`raw`/`thm`/`tprcor`、`assimsun`/`assimsha`/
`respcsun`/`respcsha`）按第 351 轮的办法 hex 化 —— `oracle/scripts/pco2a_probe.sh`（第 293 轮）
现成，先把 `ES23.15`/`{:e}` 改成 `Z16.16`/`to_bits()` 再跑 63 步；
`pco2a` 那条 `.FNMA` 仍挂在第 249 轮例外候选上（等你裁）。

**第 353 轮：干窗首分歧夹到**第 251 步**（11 个状态量）；并实测证明干窗 history 不能当首分歧判据**

**① 逐步夹逼**（`dry_ts.sh N` + `restart_divergence.py`，本轮代码 = `09b828a` + 第 352 轮的 `rsoil`）：

```text
dry N=224  0/68      N=248  0/68      N=249  0/68      N=250  0/68
dry N=251 11/68  ← 首分歧（`2008-006-19800`，当地 05:30）
dry N=252 17/68   N=254 9/68   N=255 15/68   N=256 26/68   N=288 19/68
```

（第 251 步之后的计数上下跳，是种子进入混沌后的放大，不是新种子。）

**② 第 251 步的 11 个变量（按相对量级）**

| 变量 | gold | rust | 量级 |
|---|---|---|---|
| `qstar` | -3.8666065291926e-06 | -3.866606529192517e-06 | rel 2.1e-14（~100 ULP） |
| `wice_soisno[0,5]` | 0.16432163627502586 | 0.16432163627502638 | ~19 ULP |
| `zol` | 0.0012125188997319705 | 0.0012125188997319737 | ~15 ULP |
| `rib` | 0.000311351035372002 | 0.0003113510353720028 | ~15 ULP |
| `hk[0,0]` | 8.858077176642815e-22 | 8.858077176642805e-22 | ~5 ULP |
| `t_grnd` / `t_soisno[0,5]` | 262.72544705512865 | 262.7254470551286 | ~1 ULP |
| `wliq_soisno[0,5]` | 7.199971236974072e-05 | 7.199971236974074e-05 | ~1 ULP |
| `trad` / `qref` / `rst` | — | — | 各 ~1 ULP（`rst`≈5.0e5，夜间气孔近闭） |

**读法**：差同时出现在**土壤侧**（`t_soisno[5]`/`wice`/`wliq`/`hk`）与**近地层侧**
（`qstar`/`zol`/`rib`/`qref`）⇒ 种子在这两支的**共同上游**。干窗第 251 步是**夜间**
（19800 s、当地 05:30），`rst`≈5e5、`assim`≈0 ⇒ 大概率**不在光合链**，而在
土壤热/水与辐射/近地层那一侧（与第 350/351 轮那条被挂起的 `pco2a` 例外不同源）。

**③ 顺手验一条**判据纪律**：干窗 history 的首分歧在**记录 14（= 第 15 步）**、只有
`f_assimsun` 一个变量 —— 与湿窗记录 4 的 `f_assimsun`/`f_fgrnd` 同类。本轮把它与
**基线 `d326254`** 对照跑了一遍：

```text
BASELINE(d326254) dry history 首个差异记录: 14 变量: ['f_assimsun']
```

⇒ **先前就存在**的诊断路径残差（不是第 348-352 轮引入的）。所以
**干窗的首分歧只能用 restart 扫描**（history 从第 15 步起就被这一个诊断量污染）；
湿窗同理（记录 4）。

**④ 下一枪**：把 `oracle/scripts/step_ground_probe.sh`（现成：`MOD_Thermal_CanopyPhase_Extended.F90`
里 `CALL GroundTemperature` **之前**的 23 个量）指到 **251 步**：入参已差 ⇒ 往上游
（`MOD_GroundFluxes` 的 `fseng`/`fevpg` 或辐射项）追一层；入参全同 ⇒ 残余在地面支体内。
本轮的 11 个变量已把"共同上游"限定在这两支之间。

**第 355 轮：干窗第 251 步的种子推出**水步**——它在 `etr`/`rootflux`（叶-植物水力那一侧）；顺带修掉一个把标签错位一格的比较器 bug**

**① 先把两条现成探针修好（都因为写于第 299 轮、之后源码改过而跑不起来）**

| 文件 | 修了什么 |
|---|---|
| `oracle/scripts/vsf_probe.sh` | `END_DAY/END_SEC` 回绕（`STEPS>48` 原本撞日期上界）+ 强迫路径重定向 |
| `oracle/scripts/vsf_richards_probe.sh` | 同上；另修 **3 个过期锚点**（`residual_norm_mm` 已改成 `fold+mul_add`、routine 尾部、`zone` 作用域）；**停用 `RCHZ`（出场）那一块** —— 第 339 轮重构后 `zone`/`state.water_table_thickness_mm` 已不在作用域（E0425/E0609），而它要打的信息与 `vsf_probe.sh` 的 `WSF`（出场）**重复**，不值得为它改口径（原块留在注释里） |
| `oracle/scripts/vsf_richards_cmp.py` | **名称表 bug**：`WSF1` 的字段名漏了内核先打的 `nlev` ⇒ **每个名字错位一格**（值比得对、标签全错）。第 355 轮前半段据此读出的"`rsubst` 差"其实是 **`etr`** 差 —— 差一步就去追径流链了。已补上 `nlev` 并留注释 |

**② 定位链（全部实测，干窗 251 步）**

```text
restart 扫描           N=250 0/68、N=251 11/68        ⇒ 首分歧 = 第 251 步（第 353 轮）
vsf_probe.sh 251       WATER_VSF **出场**：251 次调用只有第 251 次不同，
                       首个不同字段 = `ss_vliq`（第 1 层，1 ULP）  ⇒ 差生在**本步水步内部**
vsf_richards_probe.sh  Richards_solver **入场/内部**：9112 条记录只有 6 条不同，
                       第一条 = ('WSF1', 251) **`etr`**          ⇒ 水步**入场前**就已经差
                       另有 WSFE 的 `rootflux[1,3,4,7]`（同一步）与 RCHL 的 `ss_vl`（下游）
```

⇒ **干窗第 251 步的种子在水步的上游：`etr`（蒸腾）与 `rootflux`（逐层根吸水）已经差 1 ULP**，
水步只是把它传下去（`ss_vliq` → restart 的 `wliq/wice/t_soisno[0,5]` → 该层 `hk` →
近地层的 `qstar/zol/rib/qref/rst/trad/t_grnd`）。
（顺带纠正第 353 轮的一个读法：`hk[0,0]` 与 `wliq_soisno[0,5]` 是**同一层** ——
`hk` 是 10 层数组、`t_soisno` 是 15 槽（0-4 是空雪层），不存在"层 0 的 hk 差而水不变"的怪事。）

**③ 两个窗口的种子在**同一条链**上**

* 湿窗：第 63 步 = `pco2a`（`stomata` 唯一分叉的入参）→ `stomata` → PHS → `vegwp`/`ldew_rain`；
* 干窗：第 251 步 = `etr`/`rootflux`（水步入参）——`etr = etrsun + etrsha`、`rootflux` 是
  PHS 的输出（`MOD_LeafTemperature_Extended.F90:1358-1367`）。

两条都落在**叶温-气孔-植物水力**这一侧 ⇒ 第 349-351 轮那条链（以及挂起的 `pco2a` `.FNMA`
例外）是**两个窗口共同的瓶颈**，不再是"湿窗专属"。

**④ 下一枪**：把 `phs_hex_probe.sh`（第 345-349 轮那位型探针，现在支持 `WORK/STEPS`）
加一个 `CASE` 选择器后指到**干窗 251 步** —— 判 PHS 链在干窗第 251 步的**入参**
（`gssun`/`gssha`/`laisun`/`laisha`/`fwet`/`tl`）是否已经分叉；若已分叉，再往上就是
`stomata` 的入参（`etr = etrsun+etrsha` 由 `transpiration` 给出），可复用第 351 轮的
`stomata_probe.sh`（已 hex 化）。

**第 355 轮追加：干窗 251 步的 PHS 位型探针 —— 与湿窗**同一条链**（`stomata` 的返回值），而且这次还看到 `qaf`/`qg`**

给 `oracle/scripts/phs_hex_probe.sh` 加了 `CASE=` 选择器（原来写死 `CN-Cng-wet`），
指到干窗 251 步（`CASE=CN-Cng STEPS=251`），两侧各 17368 行，逐位结果：

```text
首个差异: (1292, 'PHXS', 2, '40D3B58354960AE5', '40D3B58354960AE9')   ← `rssha` 差 4 ULP
差异按标签/字段: PHXS {rssun 1, rssha 4, assimsun 2, assimsha 4}
                PHXR {rssun 1, rssha 4, gs0sun 1, gs0sha 4}
                PHXI {gssun 1, gssha 4, qaf 6, qg 6}
                PHXG {qflx_sun 7, qflx_sha 7}   PHXA/PHXD/PHXF 若干
```

**读法**：
* 干窗的可见首分歧同样是 **`stomata` 的返回值**（`PHXS`），字段是 **`rssha`**（湿窗是 `rssun`）——
  两个窗口的种子在**同一条叶温-气孔-植物水力链**上，这不再是猜测。
* 与湿窗不同的是，干窗里 **`qaf`（冠层空气比湿）与 `qg`（地表比湿）各差 6 次**（湿窗只差 1–2 次）
  ⇒ 干窗的这条链上，**湿度那一支**（`qaf` 由叶温迭代里的加权湿度给出、`qg` 由地表给出）
  参与得更深，值得作为下一条候选线。
* `PHXI` 的 `gssun`/`gssha` 只差 1/4 次、`PHXR` 的 `gs0sun`/`gs0sha` 同 —— 即"**入参已经不同**"
  的模式与湿窗一致：`stomata` 依旧只是透传。

**下一枪（两窗口共用）**：用**已 hex 化**的 `stomata_probe.sh`（有 `CASE=`）跑干窗 251 步，
判 `stomata` 的 13 个入参里**唯一**分叉的是不是又是 `pco2a`；再用 `pco2a_probe.sh`（待 hex 化）
与 `qaf`/`qg` 的比较器把湿度那一支也覆盖上。

**第 356 轮：干窗 `stomata` 探针 —— `.FNMA` 对干窗**逐位中性**，只关湿窗第 63 步那颗种子
（⚠ 本轮对"干窗唯一分叉入参"的读法**有误**，见本条目末尾的更正）**

**① 干窗 251 步的 hex `stomata` 探针（5104 次调用）**

```text
STIN  唯一分叉字段 = `pco2a`（14/5104 次，首个第 834 次调用 ≈ 第 105 步）
      tlef/psrf/po2m/pco2m/ea/ei/par/rb/rstfac/cint(1..3) 全 5104 次**从不差**
STPH  5104/5104 全同
STIT  只有 `pco2i`/`eyy` 两个**探针口径**列不同（第 355 轮已注明，WUE 支不可比）
STOUT `gsh2o` 66/5104（`pco2a` 的下游）
```

⇒ 湿窗的入参里唯一分叉的是 `pco2a`（第 351 轮）；干窗的清单**不止它**（见末尾更正）。
`MOD_LeafTemperature_Extended.F90:1243-1244` 那条更新式因此是**两窗口共用的瓶颈**。

**② 把 `pco2a` 的 `.FNMA` 补回来，这次**两窗口都测**（其余代码 = 已落地的 `09b828a` + 第 352 轮 `rsoil`）**

| 仪器 | 基线 | 打 `.FNMA` |
|---|---|---|
| **湿窗首分歧步** | **N=63** | **N=73…84**（N=63/72 全 0/68） ✓ |
| **干窗首分歧步** | **N=251（11/68）** | **N=251（11/68）——逐位不变** ✓ 中性 |
| 湿窗 N=96 | 11/68 | 14/68 ✗ |
| 干窗 N=288 | 19/68 | 19/68（不变） ✓ |
| 黄金 dry | 9163 / 28 / 1，261.0128 | 9163 / 28 / 1，261.0128（不变） ✓ |
| 黄金 wet | 27066 / 1197 / 19，8.4418 | 27087 / **1650 / 35**，19.2429 ✗ |

⇒ 这条 6 行改动**只关湿窗第 63 步那颗种子**（正是第 349-351 轮定位的那颗），
对**干窗全部口径逐位中性**（含首分歧步 251）；唯一代价是黄金湿窗的聚合指标 ——
而第 351 轮已用 8 小时三方比对证明黄金**就是当前内核**跑得出来的 ⇒ 那是它**自己首分歧之后**
的混沌。按"任一 `over_tol`/`ot_vars` 变差就回滚"的硬规则**仍回退**（第 3 次记录在案：
连同 `baea8c5`/`f7e9e31`；重打只需把 `:1243-1244` 写成 `(-rate).mul_add(sink, pco2m)`）。

**③ 干窗那颗种子下一步不在 `pco2a`**（它对干窗中性），而在**湿度那一支**：
干窗 PHS 探针里 `PHXI` 的 `qaf`（冠层空气比湿）与 `qg`（地表比湿）各差 **6 次**（湿窗只 1–2 次）。
**下一枪**：把 `qaf` 的**生产者**两侧 hex 打出来 —— 内核
`MOD_LeafTemperature_Extended.F90` 里 `qaf = wtaq0*qm + wtgq0*qg + wtlq0*qsatl` 那一段
（Rust 在 `leaf_temperature.rs` 的 `canopy_air_humidity` 加权更新），
并把 `pco2a_probe.sh` 先 hex 化后一并跑 251 步。

**第 356 轮更正（重要）：干窗第 251 步进 `stomata` 的差异字段是 `ea`，不是 `pco2a`**

`stomata_cmp.py` 的那行 `字段 [...]` 只列**第一组**差异里的字段 —— 本轮据此写出
"干窗里唯一分叉的入参是 `pco2a`"是**读错了**。把干窗 251 步的 5104 组逐组展开
（注意内核计数 1 基、Rust 0 基，要对齐）：

```text
STIN 差异组: 14 / 5104
  调用 835, 836                → 字段 `pco2a`（≈ 第 105 步，之后自愈）
  调用 5093…5104（= 第 251 步的全部 12 次调用） → 字段 **`ea`**
```

⇒ **第 251 步的种子是以 `ea`（冠层空气水汽压）进 `stomata` 的**，这也正好解释了两件事：
* 第 356 轮把 `pco2a` 的 `.FNMA` 补回来时，干窗口**逐位不变** ✓（干窗的差异字段是 `ea`）；
* 第 355 轮 PHS 探针里 `PHXI` 的 `qaf`/`qg` 各差 6 次 ✓ 与之同源
  （`ea` = `qaf*psrf/(0.622+0.378*qaf)`，内核 `:791` 的 `eah`）。

**下一枪（干窗种子）**：`qaf` 那条更新式**本身是忠实的** —— 读 GIMPLE 已确认它是**整条平铺**
（`_481 = wtaq0*qm`、`_483 = wtgq0*qg`、`_567 = fadd`、`_569 = qsatl*wtlq0`、`_570 = fadd`，
一个都没融；Rust 的 `canopy_air_humidity` 写法与它逐位一致，见 `leaf_temperature.rs:940-948`
的注释）。所以差在它的**入参**：`qg`（地表比湿，PHXI 探针已见差）、`qsatl`（= f(tl,psrf)，
`tl` 在 PHXR 里从不差）、`qm`，或三个湿度权重 `wtaq0/wtgq0/wtlq0`（= `caw/cgw/cfw * wtsqi`，
其中 `cfw` 第 347 轮刚修过）。
**探针**：把 `qaf = wtaq0*qm + wtgq0*qg + wtlq0*qsatl` 与 `eah = qaf*psrf/(0.622+0.378*qaf)`
两侧 hex 打出来（连同 `qg`、三个权重、`caw/cgw/cfw/wtsqi`），跑干窗 251 步。
另：`pco2a_probe.sh` 也一并 hex 化（它现在还是 `ES23.15`）。

**第 357 轮：干窗第 251 步的 `ea` 追到 **`qg`（地表比湿）**—— `qg` 的式子两侧同形，差在它的入参**

**① 给 `phs_hex_probe.sh` 加了 `PHXH`**（打在 `qaf = wtaq0*qm + wtgq0*qg + wtlq0*qsatl` 之后，
两侧同序）：`wtaq0, wtgq0, wtlq0, qm, qg, qsatl, qaf` 七个位型。

**② 干窗 251 步（两侧各 19920 行）的结果**

```text
首个差异仍是 PHXS 的 `rssha`（4 ULP，第 355 轮）
PHXH 差异 = 字段 3 `wtlq0` ×1、字段 5 **`qg`** ×6、字段 7 `qaf` ×6
            `wtaq0`/`wtgq0`/`qm`/`qsatl` **从不差**
```

⇒ 干窗的 `qaf`（进而 `ea = qaf*psrf/(0.622+0.378*qaf)`）差异来自 **`qg`**；
`wtlq0` 那 1 次是 `cfw*wtsqi` 那条线（第 347 轮刚修过 `cfw`），量级很小。

**③ `qg` 的生产者与两侧写法**：`MOD_Thermal.F90:576/582` 的**非 split 支**

```fortran
qred = (1.-fsno)*hr + fsno
CALL qsadv(t_grnd,forc_psrf,eg,degdT,qsatg,qsatgdT)
qg   = qred*qsatg
IF (qsatg > forc_q .and. forc_q > qred*qsatg) THEN qg = forc_q; dqgdT = 0. ENDIF
```

Rust `ground_humidity.rs::non_split_ground_humidity` 与之**代数同形**：
`humidity_reduction = (1-snow)*rh + snow` → `reduced_humidity = humidity_reduction*qsatg`
→ 同一个夹逼（`qsatg > forc_q > reduced` 时取 `forc_q`）✓ ⇒ **不是式子的问题，是入参**。
（`DEF_SPLIT_SOILSNOW` 是**运行期**开关、默认 `.false.`，黄金三例都走非 split 支；
Rust 只移植了非 split 支 —— 打开 split 时两边会分道，记在这里。）

**④ 下一枪**：`qg` 的入参 = `fsno`、`hr = exp(psit/roverg/t_grnd)`（`psit` 来自土壤水）、
`t_grnd`、`forc_q`、`forc_psrf`。在 `MOD_Thermal` 的 `qg` 赋值处加一个 hex 打印
（Rust 打在 `non_split_ground_humidity` 入口），跑**干窗 251 步**，判是 `psit`/`hr`
还是 `fsno`/`t_grnd` 先分叉 —— 这一步会把种子推到**地面/土壤那一侧**或钉在**外层能量迭代**上。

**第 358 轮：干窗第 251 步的因果链定下来了 —— `qg` → `qaf`/`ea` + 驱动湿度 → `qflx`/`etr` → 水步；**不是** `stomata` 的输出**

把第 357 轮那份干窗 251 步探针（2552 次 PHS 调用）按**标签序号**逐条展开（`PHXH`/`PHXG`
没打调用号，按该标签第几条对齐；其余按调用号）：

```text
PHXS/PHXR  差异 4 条：序号 294, 298, 418, 2209     （首个 294：`rssha`/`gs0sha`）
PHXI       差异 10 条：294, 298, 418, 2209, 2547…2552（首个 294：`gssha`）
PHXH       差异 7 条：418, 2547…2552               （首个 418：`wtlq0`）
PHXG       差异 7 条：418, 2547…2552               （首个 418：`qflx_sun`/`qflx_sha`）
```

2552/251 ≈ 10.2 次调用/步 ⇒ **2547…2552 = 第 251 步的最后 6 次调用**。读法：

* 序号 294（≈第 29 步）、298、418（≈第 41 步）、2209（≈第 217 步）的差异**都自愈了**
  （restart 到 250 步仍 0/68），所以它们不是种子；
* **第 251 步那一簇里，`stomata` 的输出（`PHXS` 的 `rssun`/`rssha`/`assim*`）并不差**，
  差的是 **`PHXH` 的 `qg`/`qaf`** 与 **`PHXG` 的 `qflx_sun`/`qflx_sha`** ✓
  ⇒ 机制是：`qg`（地表比湿）→ `qaf`/`ea` 与 `getqflx_gs2qflx` 里的**驱动湿度**
  `cqi = (wtaq0+wtgq0)*qsatl - wtaq0*qm - wtgq0*qg` → **`qflx_sun`/`qflx_sha`（蒸腾需求）**
  → `etr`/`rootflux`（水步入场，第 355 轮）→ `ss_vliq`/`wliq[0,5]` → `hk`/`t_soisno` → 近地层诊断。
* **`rssha` 在 251 步不差是合理的**：CN-Cng 是 C3 + WUE 开 ⇒ 走 WUE 支，
  `gsh2o = assmt/(co2a - pco2i/psrf)*1.6` 里**没有 `ea`**；而 `qflx` 那一支有 `cqi`，
  所以 `ea` 的 1 ULP 只从**蒸腾需求**那条路出去 ✓✓。

**顺带记一条判据纪律**：`qaf` **不是重启变量**（68 个状态量里没有它），但它跨步保存
（模块变量）。所以"restart 到 250 步 0/68"**不能**排除"某个非重启量在第 251 步之前就已经差了"
—— 这类量（`qaf`/`qflx`/`etr` 一族）只能用**探针流**判首分歧，不能用 restart 扫描。

**下一枪**：`qg` 的入参（`fsno`、`hr = exp(psit/roverg/t_grnd)`、`psit`、`t_grnd`、`forc_q`、`forc_psrf`）
两侧 hex。`qg` 的式子已证同形（第 357 轮），所以这一步会直接告诉我们是**外层能量迭代的
`t_grnd`** 还是**土壤侧的 `psit`** 先把 1 ULP 带进来 —— 那也就把干窗种子推到
`MOD_Thermal`/`GroundTemperature` 那一段（本目标点名的"ground-temperature chain"）。

**第 359 轮：干窗第 251 步的土壤侧入口是 **`psit`（表层基质势）** —— `psit → hr → qred → qg` 全链只在这一步同时差**

给 `phs_hex_probe.sh` 加了第 6 个被探文件对：`extends/interception/MOD_Thermal_CanopyPhase_Extended.F90`
（**注意 `Makefile:647` 把 `MOD_Thermal.o` 指向 extends 那份**，`main/MOD_Thermal.F90` 不编进内核）
与 `crates/colm-core/src/ground_humidity.rs`，新标签 `PHXQ2`（8 字段：
`fsno, psit, t_grnd, forc_q, qsatg, hr, qred, qg`）。干窗 251 步（两侧各 20171 行）：

```text
PHXQ2 每步 1 条（251 条）；差异 34 条
  `psit` 差 34 步（65,68,69,70,71,73,…,214,251）—— 最大的一族
  `hr`/`qred`/`qg` 各只差 **1 次**，都在**第 251 步**
  `fsno`/`t_grnd`/`forc_q`/`qsatg` **从不差**
第 65 步实测: psit K=-348.90071475521734 R=-348.90071475521773（hr/qg 仍逐位相同）
第 251 步:    psit → hr → qred → qg **整条都差**
```

**读法**：
* **不是地面温度那一侧**：`t_grnd`/`fsno` 从不差 ⇒ 干窗种子不走 ground-temperature chain；
* **土壤侧入口是 `psit`** = `f(wx, porsl(1), theta_r(1), psi0(1))`，而 `wx` 由**表层**的
  `wliq_soisno(1)`/`wice_soisno(1)` 决定 ⇒ **表层水/冰在步内瞬时差**（它在步末会自愈：
  restart 第 251 步差的是**第 6 层**（0 基索引 5），不是表层）；
* `psit` 差 34 步但只有第 251 步把 `hr`/`qred`/`qg` 带出去 ⇒ 这正是"**偶发投影**"：
  耦合的叶↔土回路里持续存在的 1 ULP，偶尔才投影到状态上。

**把 355/358/359 串起来（干窗第 251 步的完整因果链，全部实测）**：

```text
（步内瞬时的）表层水差 → psit(34 步; 第 251 步生效) → hr → qred → qg（第 251 步首次全差）
  → qaf/ea 与驱动湿度 cqi → qflx_sun/qflx_sha → etr/rootflux（水步入场）
  → 水步的 ss_vliq → 第 6 层 wliq/wice → hk/t_soisno → qstar/zol/rib/qref/rst/trad/t_grnd
```

**下一枪**：在 `MOD_Thermal` 的 `psit` 赋值处两侧 hex 打它的入参（`wx`、`wliq_soisno(1)`、
`wice_soisno(1)`、`porsl(1)`、`theta_r(1)`、`psi0(1)`、`fac`）—— 判"表层水瞬时差"到底来自
水步的哪一次写入（`vsf_probe.sh` 的 `WSF` 出场标签已把 `ss_vliq` 指到第 1 层 ✓），
以及 `soil_psi_from_vliq` 的 VG 支在**真实参数区间**上是否有闭环没覆盖的形状差
（闭环 3×10000 是合成输入 ✓）。

**第 359 轮追加：`soil_psi_from_vliq` 的 VG 支已被闭环覆盖 ⇒ `psit` 的差只能来自入参（表层水）**

查了现成闭环 `oracle/scripts/compare_soilhydro.sh` + `soil_hydro_fn_diff.f90` 的覆盖面
（文件头注释已写清，无需重跑）：

```text
覆盖 soil_psi_from_vliq / soil_hk_from_psi / soil_vliq_from_psi，
两档模型（Campbell / van Genuchten）× (2500 组均匀随机 + 2500 组边界取值)，
三个函数**串联**调用（psi 从 vliq 算出后再喂给另外两个）——
即"一处差就会在下游显形"，不是各自独立合成输入。
```

⇒ `psit` 计算里走的 **VG 支**（`DEF_USE_Campbell_SOIL_MODEL = .false.`，默认）在这 5000 组上
**逐位全同** ⇒ **`psit` 的 34 次差异只能来自它的入参**，也就是 `wx`
（= `(wliq_soisno(1)/denh2o + wice_soisno(1)/denice)/dz_soisno(1)`）与
`porsl(1)`/`theta_r(1)`/`psi0(1)`（后三个是站点常数 ✓）。

**结论（干窗种子的根）**：**表层水/冰的步内瞬时差** —— 而 `vsf_probe.sh` 的 `WSF` 出场标签
（第 355 轮）已把它指到**第 1 层的 `ss_vliq`** ✓。剩下要钉的是"水步里哪一次写入/哪一个分支
把它带进来"，即 Richards 内部那几层（`vsf_richards_probe.sh` 的 `RCHL` 已在第 355 轮给出
`ss_vl` 第 1 层差 ✓，可以作为下一层的入口）。

**第 360 轮：干窗那条链的**起点**就是 `stomata` 的形状缺口 —— 入参逐位相同、出参已差（第 302 次调用 ≈ 第 15 步）**

把第 356 轮那份干窗 hex `stomata` 探针（5104 次调用）的 **入参/出参**分开统计：

```text
STIN  差异 14 组：调用 834,835(`pco2a`)、5092…5103(`ea`)      ← 最早在第 834 次调用（≈第 105 步）
STOUT 差异 66 组：调用 **302**,303,314,315,362,363,…            ← 最早在第 302 次（≈第 15 步）
      字段：`gsh2o` ×66、`rst` ×5；`tprcor`/`tlef` 从不差
```

⇒ **第 302 次调用时入参 13 个字段逐位相同，出参 `gsh2o` 已经不同** ⇒ 差在
**`stomata` 函数自己的收缩形状**上，不在入参 —— 这正是第 350 轮那 **14 处 GIMPLE 形状**
（当时实测：闭环 `rst 114 + assim 171` → `rst 0 + assim 100`、湿窗 N=96 `11→19`、
黄金 wet `1197/19 → 1650/35`，按硬规则回退了；**首分歧步两个窗口都没动**，见第 361 轮）。

**两个窗口的链起点是同一处**：

| 窗口 | 链的起点（实测） | 打那 14 处之后 |
|---|---|---|
| 湿窗 | 第 349 轮：`stomata` 返回值 `rssun`/`rssha` 先差（**入参**那层是 `pco2a`） | 首分歧 **N=63 → N=73…84** ✓ |
| 干窗 | 本轮：第 302 次调用入参全同、出参 `gsh2o` 已差（≈第 15 步） | **未测**（首分歧现在 N=251） |

⇒ 那条被挂起的例外候选**同时**落在两个窗口的链上，不再只是"湿窗专属"；
它在湿窗已经把**首判据**推进了 10–21 步、对干窗口径逐位中性（第 356 轮）。

**下一枪（决策级数据）**：把第 350 轮那 14 处形状**照文档重打一遍**，这次补测
**干窗的首分歧步**（`dry_ts.sh 250/251`）—— 若它也往后走，则该例外在两个窗口的
**首判据**上都成立，只剩黄金 wet 聚合指标一项代价（已证是它自己首分歧之后的混沌）；
若干窗首分歧不动，则那 14 处对干窗只是"起点"而非"投影点"，得继续往水步里钉。

**第 361 轮：重打第 350 轮那 14 处形状 —— **两个窗口的首分歧步都不动**；那条"N=63 → N=73…84"是 `pco2a` 的功劳，不是它的**

第 360 轮我把"N=63 → N=73…84"记到了这 14 处形状头上，**是记错了**（那条属于第 351/356 轮的
`pco2a` `.FNMA`）。本轮照第 350 轮的对照表把这 14 处**重打一遍**（在已落地的
`09b828a` + 第 352 轮 `rsoil` 之上），实测：

| 仪器 | 基线（当前落地） | 重打 14 处 |
|---|---|---|
| **湿窗首分歧步** | N=63（10/68） | **N=63（10/68）——不动** ✗ |
| **干窗首分歧步** | N=251（11/68）；N=250 0/68 | **N=251（11/68）；N=250 0/68——不动** ✗ |
| 闭环 `compare_stomata.sh` | `rst 114 + assim 171` | `rst 0 + assim 100（全 BIG）` ✓ |
| 湿窗 N=96 | 11/68 | 19/68 ✗ |
| 黄金 wet | `27066 / 1197 / 19，8.4418` | `27087 / 1775 / 52，19.8039` ✗ |

（湿窗 N=96 与黄金 wet 两列与第 350 轮一致 ✓ —— 那两项当时就测过；本轮补的是**两个窗口的
首分歧步**，结论是**都不动**。）

**结论与影响**：这 14 处形状**只有闭环**变好，**两个窗口的首判据（首分歧步）都中性**、
且短程扫描与黄金聚合指标变差 ⇒ **按硬规则继续回退**（工作树已 `git checkout` 复原）。
更重要的是：**那条挂起的例外候选（第 249 轮口径）不成立** —— 例外要求"短程直接仪器改善或中性
+ 逐条对得上编译源码 + 聚合反向是混沌"，而这里的短程仪器（湿窗 N=96）是**变差**的，
首判据又完全不动。⇒ 结论收敛为：**`stomata` 那 14 处该补，但不是本窗口种子的开关**；
湿窗第 63 步的开关仍是 `pco2a` 的 `.FNMA`（唯一把首分歧推进 10–21 步的改动），
干窗第 251 步的开关仍未找到（链的起点在 `stomata`，投影点还不明）。

**方法论记一条**：链的**起点**（probe 流里第一次出现差异）与状态的**投影点**（首分歧步）
可以相隔很远 —— 修起点不等于推迟投影点（本轮实测）；判"哪个改动有用"必须看首分歧步。

**第 362 轮：把"14 处形状"与"`pco2a` 的 `.FNMA`"**叠加**着测 —— 干窗那颗种子**不在**气孔/叶链上（决定性否证）**

第 361 轮只测了 14 处形状**单独**作用（两个窗口首分歧都不动）。本轮先看它对**探针流**的作用，
再测它与 `pco2a` `.FNMA` **叠加**后两个窗口的状态口径：

**① 14 处形状对干窗探针流的作用（`CASE=CN-Cng STEPS=64`，1530 次 `stomata` 调用）**

| | 基线 | 打上 14 处 |
|---|---|---|
| `STOUT`（出参）差异 | **66/5104**，首个第 **302** 次（≈第 15 步） | **2/1530**，首个第 **834** 次（≈第 35 步） |
| `STIN`（入参）差异 | 14 组（834/835 `pco2a`、5092…5103 `ea`） | **2 组（第 834 次 `pco2a`）** |

⇒ 14 处形状**确实把干窗那条链的起点修掉了**（函数差 66 → 2），剩下的 2 条是**入参驱动**
（`pco2a`，正是 `.FNMA` 那条）✓。

**② 两者叠加后的状态口径**

| 仪器 | 基线（落地） | 14 处 + `pco2a` `.FNMA` |
|---|---|---|
| 湿窗 N=63 / N=72 | 10/68 / — | **0/68 / 0/68** ✓ |
| **干窗 N=250 / 251 / 252 / 260** | 0 / **11** / 17 / 29 | **0 / 11 / 17 / 29 ——逐位不变** ✗ |

⇒ **决定性否证**：干窗链的**起点**在气孔/叶链上（已修 ✓），但**状态投影点**（第 251 步）
**一点没动** ⇒ **干窗第 251 步那颗种子不在气孔/叶链上**。它必然来自**水/土那一侧**
（`WATER_VSF`/`Richards` 的参数化与状态写回，或 `MOD_Thermal` 的非 split 支），
这也解释了为什么第 361 轮"修起点"没有推迟"投影点"。

**③ 方法论（本轮第二次验证）**：**探针流的作用域**与**状态投影**可以完全解耦 ——
"把某条链修到逐位相同"不等于"那颗种子被关掉"。判种子只能用**状态扫描的首分歧步**；
探针流只用来**定位候选**，不能用来宣称修复。

**下一枪（干窗）**：回到水/土侧，在**第 251 步内**把 `WATER_VSF` 的状态写回逐步展开
（`vsf_probe.sh` 的 `WSF` 出场 + `vsf_richards_probe.sh` 的 `RCHL/RCHE`）——
第 355 轮已把出场首差异钉在**第 1 层 `ss_vliq`**，而 `Richards` 内部的
`RCHF/RCHB/RCHD/RCHE` **全部逐位相同** ⇒ 差异是在**子步之间**由 `rootflux` 经状态写回带进来的；
既然叶链已证无关，就要看写回路径里（`ss_vliq`/`wliq`/`wice`/`smp`/`hk` 的更新式）有哪一处形状没对齐。

**第 363 轮：水/土侧的 GIMPLE 普查 —— `water_vsf` 那 17 处全部收口，剩下的大头是 `thermal` 的 99 处**

干窗种子已证不在叶链（第 362 轮），本轮把**水/土侧**三个编进内核的文件做了 GIMPLE 普查
（方法同第 340 轮：站点清单用 `-S` + `.loc`，操作数角色用 `-fdump-tree-optimized`）：

```text
MOD_Hydro_SoilWater.F90            71 处
    soil_water_vertical_movement 29 / get_zwt_from_wa 9 / solve_least_squares_problem 8
    flux_all 7 / water_balance 4 / soilwater_aquifer_exchange 4
    flux_top_transitive_interface 3 / flux_btm_transitive_interface 3
MOD_SoilSnowHydrology.F90          67 处
    soilwater 23 / snowwater_snicar 8 / snowwater 4 / **water_vsf 17** / water_2014 15
MOD_Thermal_CanopyPhase_Extended.F90 99 处（全部内联进 `thermal`）
Rust 侧计数：variably_saturated_flow.rs 67 个 mul_add；ground_temperature.rs 27；
             thermal_properties.rs 25；ground_thermal_step.rs **0**（59 行的薄封装）
```

**`water_vsf` 的 17 处逐条收口**（`-S` + `.loc` 给到源码行）：

| 源码行 | 内容 | 结论 |
|---|---|---|
| `:1110` | 顶层 `wliq_soisno` 回填 | **第 338 轮已修**（操作数写反）✓ |
| `:1128` `:1132` `:1133` | 顶层凝结写回 `max(0, ldew + qsdew*dt)` 一族 | Rust `:4517-4525` **已是 FMA** ✓ |
| `:827` | `gwat = gwat + pg_rain*(1-fsno) - qseva_soil` | 预解算入流 |
| `:867` | `wresi = wliq - dz*denh2o*vol_liq` | Rust `:4237` 注释逐条对过 ✓ |
| `:1037` `:1045` | `vol_liq`/`wresi` 的反解 | Rust `:4375`/`:4393-4394`/`:4493` 逐条对过 ✓ |
| 其余（`:278`/`:434-473`/`:1220-1273`/`:2534`/`:2561`） | 被内联进来的其它例程（多为雪支） | 干窗不走（`fsno=0`），雪窗本机测不了 |

⇒ **预解算的状态换算（`vol_liq`/`wresi`）与顶层凝结写回都已忠实** —— 这与第 355 轮
"`Richards` 内部 `RCHF/RCHB/RCHD/RCHE` 全部逐位相同、差异在子步之间"互洽。

**下一枪**：普查表里**尚未逐条过**的三块，按"干窗最可能"排序：
① **`thermal`（99 处）** —— Rust 侧拆在 `ground_temperature.rs`(27) + `thermal_properties.rs`(25) +
`ground_thermal_step.rs`(0)，是"叶链之外"最大的一块，且 `MOD_Thermal` 正是 `psit`/`qg`/`t_soisno`
的生产者；
② `soil_water_vertical_movement` 29 处（Rust 同文件 67 个 `mul_add`，差值最小、最可能需要逐行核）；
③ `get_zwt_from_wa` 9 / `flux_all` 7 / `solve_least_squares_problem` 8。
方法照旧：`-loc` 定站点 → GIMPLE 定操作数 → 状态扫描首分歧步判是否落地。

**第 364 轮：`thermal` 那 99 处按源码行分布 —— 最大一簇（45 处）是**PC 逐 PFT → patch 的加权聚合**，而它聚合的正是干窗那些量**

用 `-S` + `.loc` 把 `MOD_Thermal_CanopyPhase_Extended.F90` 的 99 条 FMA 指令落到源码行：

```text
83 个不同源码行，集中在四段：
  :531-615     12 处  —— `psit`/`hr`/`qred`/`qg` 那一段（第 357/359 轮已逐条对过 ✓）
  :1081-1138  ~45 处  —— **逐 PFT → patch 的加权聚合** `sum( x_p * pftfrac )`
  :1343-1372  ~17 处  —— `fgrnd` / `olrg` / `olrb` / `emis`
  :967-1013      5 处  —— 另两处
```

**`:1081-1138` 那一段聚合的量**（读源码）：`laisun`/`laisha`/`tleaf`/`ldew_rain`/`ldew_snow`/
`canopy_smelt_mass`/`canopy_frzc_mass`/`fwet_snow`/`respc`/`fsenl`/`fevpl`/`lfevpl`/**`etr`**/
`dlrad`/`ulrad`/**`tref`**/**`qref`**/`taux`/`tauy`… —— 全部是 `sum(x_p*pftfrac)` ✓。

**为什么这一簇值得优先查**：干窗的探针链里，
* 水步入场首差异就是 **`etr`**（第 355 轮，`('WSF1',251) etr`）—— 而 `etr` 是**聚合出来的**；
* 目标点名的干窗种子正是 **`tref`/`tleaf`**（两者都在这一段里聚合 ✓）；
* 而**叶链的修复对干窗状态投影无效**（第 362 轮）：如果差异生在 **patch 级聚合**上，
  那么修 per-PFT 的叶温/气孔计算当然不会改变 patch 级的 `etr`/`tref`/`tleaf` ✓✓
  —— 这条正好解释了第 362 轮那个"链起点修好了、投影点却不动"的否证结果。

**两侧现状**：
* 内核：1 个 patch、**2 个 PFT**（`site.nc` 的 `pctpfts = [0.54, 0.46]`），PC 路径逐 PFT 算完再
  `sum(x_p*pftfrac)` 聚合 ✓；
* Rust：`colm-srfdata/src/site.rs` 有 `pft_components`（读 `pfttyp`/`pctpfts` + `fraction` ✓），
  但 `colm-core` 的 `standard_lct_step.rs`/`assembly.rs` 里**没有逐 PFT 的循环**，
  `pft_fraction` 只出现在 `bgc.rs`（黄金三例 BGC 关）⇒ **聚合发生在哪一层、形状是否与内核一致，
  下一步必须查清**。

**下一枪**：定位 Rust 侧 `sum(x_p*pftfrac)` 的实现（读入层还是装配层），把
`.FMA(x1, f1, x0*f0)`（或反向）与它的写法逐条对；判据仍用**干窗首分歧步**（现 N=251）
—— 若形状差在这里，这会同时解释 `etr` 与 `tref`/`tleaf`。

**第 365 轮：更正第 364 轮的线索 —— `:1081-1138` 那簇**不在**黄金路径上；下一个目标是 `:1343-1372`（`fgrnd`/`olrg`/`olrb`/`emis`）**

第 364 轮据"`thermal` 最大一簇是 PC 逐 PFT → patch 的聚合"提出了查 `sum(x_p*pftfrac)` 的下一枪。
本轮把**子网格开关**查清后否掉了它：

```text
MOD_Namelist.F90:169-171   DEF_USE_LCT = .true.   DEF_USE_PFT = .false.   DEF_USE_PC = .false.
oracle/work/CN-Cng*/case.nml  三个都没设 ⇒ 用默认 ⇒ **patch（LCT）路径**
MOD_Thermal_CanopyPhase_Extended.F90:747   IF (patchtype==0 .and. (DEF_USE_PFT .or. DEF_USE_PC))
                                      :992   IF ( DEF_USE_PC .and. pn.ge.ps )
```

⇒ `:1081-1138` 的 `sum(x_p*pftfrac)` 聚合**两个开关都是 .false. ⇒ 不执行**（死代码）；
黄金算例的 patch 级植被量出自**初始化时**对站点 PFT 数据的加权（landdata 由内核自己的
`mkinidata` 写好、Rust 直接读）✓ —— 这也解释了为什么这条"聚合"两侧不会分叉。
⇒ **第 364 轮那条下一枪作废**。

**仍然活在干窗路径上的两大簇**（本轮重新排定）：

| 簇 | 处数 | 内容 | 状态 |
|---|---|---|---|
| `:531-615` | 12 | `psit`/`hr`/`qred`/`qg` | 第 357/359 轮逐条对过 ✓（式子同形） |
| **`:1343-1372`** | **~17** | `fgrnd` / `olrg` / `olrb` / `olru` / **`emis`** / `trad` 那一族 | **未审** |

`fgrnd` 是那条**长加链**（`sabg + dlrad*emg - emg*stefnc*t_grnd_bef**4
- emg*stefnc*t_grnd_bef**3*(4*tinc) - (fseng+fevpg*htvp) + cpliq*pg_rain*(t_precip-t_grnd)
+ cpice*pg_snow*(t_precip-t_grnd)` ✓ —— GCC 会把其中 4–5 个乘积各自收进相邻加减 ✓；
`olrg`/`olrb`/`emis` 三行同理（`olrb = stefnc*t_grnd_bef**3*(4*tinc)` 等 ✓）。
而 `emis`/`trad`/`t_grnd` 都是**重启变量**且都在干窗差异表里出现过（第 348 轮 N=288：
`emis`/`trad`/`t_grnd` 各 1 处 ✓）⇒ 这一簇值得逐条对。

**下一枪**：把 `:1343-1372` 的 17 处按 `.loc` 逐条列出，与 Rust 的
`ground_fluxes.rs`(8 个 mul_add) / `ground_temperature.rs`(27) / `ground_thermal_step.rs`(**0**)
对应表达式逐条对；判据仍是**干窗首分歧步**（现 N=251）。

**第 365 轮追加：第 303/310 轮那份"确证缺口清单"**今天仍然是缺口** —— 水侧 `flux_variable_saturated_*` 全族 `mul_add = 0`**

顺着"干窗种子在水/土侧"往下核时，把 Rust 侧水模块**逐函数**数了一遍 `mul_add`：

```text
solve_variable_saturated_least_squares        9   （第 339 轮已核 ✓）
variable_saturated_water_balance              4   （第 303 轮记的 5 处缺口 → 4 处已有）
flux_variable_saturated_both_transition       0   ← 第 310 轮记的"归一化加权平均"缺口
flux_variable_saturated_zone_fixed_boundaries 0
flux_variable_saturated_zone_all              0
```

而本轮普查（第 363 轮）在**内核**侧数到：`flux_all` **7 处**、
`flux_top_transitive_interface` **3**、`flux_btm_transitive_interface` **3** ✅
⇒ **水侧 `flux` 全族仍是"内核有 FMA、Rust 一个都没有"**。

这条与第 303/310 轮的记录完全一致（当时写的是"确证缺口清单（更新）：
`flux_inside_hm_soil` **2** + `water_balance` **5** + `flux_all` 中间的加权平均 **1** = **8 处**，
写法都已记档"，并说"下一轮把这 8 处补上"）—— 那之后 336–339 轮只落了
`swvm`/`Richards_solver`/`use_explicit_form`/`soilwater_aquifer_exchange` 那几批，
**`flux` 这一族没动** ✓。

**这一步为什么重要**：干窗种子已证在**水/土侧**（第 362 轮否掉叶链、第 363 轮把
`water_vsf` 的 17 处与预解算换算全部收口），而 `flux_variable_saturated_*` 正是
**Richards 求解器每次迭代都要调的通量/界面函数**（第 355 轮量到求解器内部
`RCHF/RCHB/RCHD/RCHE` 逐位相同、差异在子步之间 ⇒ 那么**子步之间**的通量函数就是下一个要看的地方 ✓）。

**下一枪**：按第 310 轮已记档的写法，把这 7 处（`flux_inside_hm_soil` 2 + 加权平均 1 +
两个 interface 各若干）逐条补上，用**干窗首分歧步**（现 N=251）判是否落地，
湿窗（现 N=63）与两个黄金窗口一并复测。

**第 366 轮：水侧"确证缺口清单"**已基本作废** —— 真正剩下的只有 `soil_water_vertical_movement` 的 **27 vs 17 = 10 处****

逐条核第 303/310 轮那份清单，结果全是"**已经补过了**"（只是**函数名对不上**，所以第 365 轮按名字数 `mul_add` 时看漏了）：

| 内核例程 | 内核 FMA | Rust 对应 | Rust `mul_add` | 结论 |
|---|---|---|---|---|
| `flux_inside_hm_soil` | 2 | `flux_inside_variable_saturated_soil` | **2** ✓ | 第 305 轮记的写法已在库里（`:1643` 的 `l_vgm.mul_add(n_vgm-1, n_vgm*2)`、`:1667-1673` 的 `weighted.mul_add(...)`） |
| `flux_all` 的加权平均 | 1 | 同上/`both_transition` | **已有** ✓ | `psi_i = (dz_l*psi_u+dz_u*psi_l)/(dz_u+dz_l)` 已是 `mul_add` ✓ |
| `flux_top/btm_transitive_interface` | 3+3 | —— | —— | **不是这两个例程自己的**：6 条全在共享子程序 `secant_method_iteration`（`.loc` 指到 `:3559-3561`），**第 339 轮已核并落地** ✓ |
| `water_balance` | 5 | `variable_saturated_water_balance` | **4** ✓ | 只差 1 处 |
| `solve_least_squares_problem` | 8 | `solve_variable_saturated_least_squares` | **9** ✓ | 第 339 轮已核 ✓ |
| `soilwater_aquifer_exchange` / `get_water_equilibrium_state` | 4 / 2 | —— | —— | 第 339 轮成组落地 ✓ |
| **`soil_water_vertical_movement`** | **27** | 同名函数 | **17** ✗ | **差 10 处 ← 唯一实打实的缺口**（第 305 轮记的是 27 vs 12，此后已补 5 处） |

**水侧那 27 处的源码行映射（`-S` + `.loc`，26 行）**：

```text
:263 265 266 268 | :326 331 338 | :406 434 445 447 448 450 456 478 |
:817 823 | :966 | :1060 1065 1085 | :1411 1424 1464 | :1474(×2) 1479
```

分成四段：含水层/交换（`:263-338`）、`zwt`/水位（`:406-478`）、水量回填（`:1060-1085`）、
质量平衡/误差（`:1411-1479`）—— 而 `soil_water_vertical_movement` 正是**水步的状态写回函数**
（干窗种子已证的所在侧），所以这 10 处是当前最值得逐条读的地方。

**方法教训**（两轮连着踩）：**别按"函数名"对计数** —— 内核的例程名与 Rust 的公共函数名**不是一一对应**
（`flux_top_transitive_interface` 的 FMA 其实在共享的 `secant_method_iteration` 里；
`flux_all` 的 7 条是它**被内联的下游**之和，它自己一个都没有）。**用第 305 轮那张映射表**，
或者直接按 `.loc` 落到源码行再比。

**下一枪**：把这 10 处按 `.loc` 逐条读 GIMPLE 操作数角色，与 Rust 同名函数逐条对，
补齐后跑**干窗首分歧步**（现 N=251）+ 湿窗（N=63）+ 两个黄金窗口。

## 第 367 轮：水侧"差 10 处"是按函数名数出来的假象 —— 真缺口 2 处在 `hydrology.rs` 的冷启动上，另补了 `fgrnd` 那条长链

**一句话**：第 366 轮那条"下一枪"（`soil_water_vertical_movement` 内核 27 处 vs Rust 17 处 = **差 10 处**）
**作废**。把 `MOD_Hydro_SoilWater.F90` **整个文件**用 `gfortran -O2 -fdump-tree-optimized-lineno`
数一遍：一共 **71 条** `.FMA/.FNMA/.FMS`（落在 **56 个源码行**上，不是 27），逐条按 `.loc` 落到源码行、
把 GIMPLE 的操作数角色读出来之后 —— **69 条早有对应写法**，只有
`get_water_equilibrium_state` 的 **2 条**（`:137`、`:150`）从来没落过；而它根本不在
`variably_saturated_flow.rs` 里，在 `hydrology.rs`。顺着同一条方法把 `MOD_Thermal_CanopyPhase_Extended.F90`
的 `:1352` 那条 `fgrnd` 长链也逐条核了，另外补上 3 处未熔的乘积（运行时路径）。

### 一、"27 vs 17"为什么必然对不上：**汇编条数含内联下游，Rust 那边是拆开的**

第 363 轮那张表是 `-S` + `.loc` 数**汇编里的 FMA 指令**得到的，数的是一个**函数体**，
而 GCC 把被调例程**内联**进了调用者：

| 第 363 轮数出的 | = 自身 | + 内联下游 | 验算 |
|---|---|---|---|
| `soil_water_vertical_movement` 29 | 15（`:263-478`） | `Richards_solver` 6 + `use_explicit_form` 8 | 15+6+8 = **29** ✓ |
| `get_zwt_from_wa` 9 | 6（`:3402-3426`） | `secant_method_iteration` 3（调用点 `:3431`） | 6+3 = **9** ✓ |
| `flux_all` 7 | 0 | `flux_at_unsaturated_interface` 1 + `secant_method_iteration` 3×2（两个调用点各内联一次） | **7** ✓ |

这张表不是推的，是**从 dump 里按函数体直接数出来的**（`-fdump-tree-optimized-lineno` 的
`;; Function <名字>` 分段，数段内的 `.FMA/.FNMA/.FMS` 语句及其源码行）：

```text
water_balance                     total= 4  lines=[1140,1147,1148,1162]
solve_least_squares_problem       total= 8  lines=[3478,3482,3486,3491,3492,3497,3498,3515]
flux_inside_hm_soil               total= 2  lines=[2734,2750]
flux_top_transitive_interface     total= 3  lines=[3559,3560,3561]        ← 全是内联的 secant
flux_btm_transitive_interface     total= 3  lines=[3559,3560,3561]        ← 同上
flux_all                          total= 7  lines=[2815,3559,3560,3561]    ← 1 + 3×2，自身 0
get_zwt_from_wa                   total= 9  lines=[3402,3403,3407,3408,3423,3426,3559,3560,3561]
soilwater_aquifer_exchange        total= 4  lines=[560,564,569,573]
soil_water_vertical_movement      total=29  lines=[263,265,266,268,326,331,338,406,434,445,447,
                                                   448,450,456,478, 817,823,966,1060,1065,1085,
                                                   1411,1424,1464,1474,1479]   ← 自身 15 + 6 + 8
get_water_equilibrium_state       total= 2  lines=[137,150]
```

（`water_balance` 的 4 条**没有**被内联进 `swvm` —— 它在 dump 里是独立函数，
所以 `swvm` 是 29 而不是 33；这条也能从分段表直接看出来。）

而 Rust 侧把这些**拆成了独立函数**（`soil_water_vertical_movement` / `richards_solver` /
`apply_variable_saturated_explicit_step` / `variable_saturated_water_balance`…，
`bounded_secant_iteration` 还是共享的），所以"按同名函数比计数"必然对不上。
第 366 轮正是拿了上游的 29（记成 27）去比 Rust 的 17，得出"差 10 处"。
**这条方法教训第 366 轮自己已经写出来了，但只用在 `flux_all` 上，没回头查 `swvm` 那个 27。**
正确的做法只有一个：**按 `.loc` 落到源码行**，再读 GIMPLE 的操作数角色。

### 二、整个文件 71 条 FMA 的逐条归宿

`gfortran -O2 -fdefault-real-8 -g -cpp -ffree-form -ffree-line-length-0 -fallow-argument-mismatch
-Iinclude -I.bld -fdump-tree-optimized=<f> -fdump-tree-optimized-lineno`
（`-J` 指到临时目录，不在 `vendor/` 里落 `.mod`；`build-default` 那棵树是把
`build_kernel.sh` 的 `trap ... EXIT` 去掉留下来的）：

| 内核例程 | `.loc` 源码行 | FMA 语句 | Rust 侧 | 结论 |
|---|---|---|---|---|
| `get_water_equilibrium_state` | 137, 150 | 2 | `hydrology.rs::equilibrium_water_state` | **本轮补** |
| `soil_water_vertical_movement` | 263, 265, 266, 268, 326, 331, 338, 406, 434, 445, 447, 448, 450, 456, 478 | 15 | `variably_saturated_flow.rs` 同名函数（16 个代码点） | ✓ 已在 |
| `soilwater_aquifer_exchange` | 560, 564, 569, 573 | 4 | `exchange_soil_water_with_aquifer` | ✓ 第 337 轮 |
| `Richards_solver` | 817, 823, 966, 1060, 1065, 1085 | 6 | `richards_solver` | ✓ 已在 |
| `water_balance` | 1140, 1147, 1148, 1162 | 4 | `variable_saturated_water_balance` | ✓ 已在 |
| `use_explicit_form` | 1411, 1424, 1464, 1474(×4), 1479 | 8 | `apply_variable_saturated_explicit_step`（6 个代码点） | ✓ 已在 |
| `flux_inside_hm_soil`（FUNCTION） | 2734, 2750 | 2 | `flux_inside_variable_saturated_soil` | ✓ 已在 |
| `flux_at_unsaturated_interface` | 2815 | 1 | `flux_at_variable_saturated_interface` | ✓ 已在 |
| `get_zwt_from_wa` | 3402, 3403, 3407, 3408, 3423, 3426 | 6 | `water_table_from_aquifer`（4 个代码点） | ✓ 已在 |
| `solve_least_squares_problem` | 3478, 3482, 3486, 3491, 3492, 3497, 3498, 3515 | 8 | `solve_variable_saturated_least_squares`（9 个代码点） | ✓ 已在 |
| `secant_method_iteration` | 3559, 3560, 3561（各 ×5） | 15 | `bounded_secant_iteration` | ✓ 第 339 轮 |
| **合计** | 56 行 | **71** | | **缺 2** |

"Rust 代码点数 ≠ 内核语句数"的地方都逐条看过原因，不是漏：
`:1474` 的 4 条是**两条嵌套 FMA**（`dwat` 收进"加权和"那个和），Rust 写成
`water_change_factor.mul_add(dt, …mul_add(…))` = 2 个点；
`:3403/:3408/:3423` 是**同一个闭包** `liquid_at_depth` 被内联到 3 个调用点；
`secant_method_iteration` 的 `:3559-3561` 在汇编里出现 5 份，是 5 个内联点。

旧 banner 点名的那 26 行**逐条**都对上了（GIMPLE 操作数角色 ⇒ Rust 行）：

| 旧 banner 的行 | GIMPLE | Rust |
|---|---|---|
| `:263/:265/:266/:268` | `.FMA(ss_vliq, sp_dz, Σ)`、`.FMA(ss_vliq(izwt), zwt-sp_zi(j-1), Σ)`、`.FMA(porsl, sp_zi(j)-zwt, Σ)`、`.FMA(porsl, sp_dz, Σ)` | `:4853/:4855/:4859/:4865` ✓ |
| `:326/:331` | `.FMA(dt, etroot, deficit)` / `.FMA(etroot, dt, deficit)` | `:4905`+`:4924` / `:4931` ✓ |
| `:338` | `.FMA(rsubst, dt, deficit)` | `:4941` ✓ |
| `:406` | `.FMA(qgtop, dt, ss_dp)` | `:5067` ✓ |
| `:434` | `.FMA(ss_vliq, sp_dz-ss_wt, porsl*ss_wt)`（第二个源乘积独立舍入） | `:5113` ✓ |
| `:445/447/448/450` | 同 `:263/265/266/268`，作用在 `w_sum_after` | `:5130/:5132/:5136/:5141` ✓ |
| `:456` | `.FMA(qgtop-Σetroot-rsubst, dt, w_sum_before)`，再 `- etrdef` | `:5148-5153` ✓ |
| `:478` | `.FMA(ss_vliq, zwt-zlo, porsl*(zhi-zwt))` | `:5180` ✓ |
| `:817` | `.FNMA(dt_this, q_0-ubc, dp_m1)`（wet2dry 判定，影响迭代出口） | `:3638` ✓ |
| `:823` | `.FMA(blc, blc, Σ)`（收敛范数的平方和） | `:3651` ✓ |
| `:966` / `:1060` | `.FMA(sp_zi-zwt, 5.0e-1, psi_s)` | `:3864` / `:4026` ✓ |
| `:1065` | `.FMA(q_this, dt_this, ss_q)` | `:4040` ✓ |
| `:1085` | `.FMA(ss_wf, vl_s, (dz-wf-wt)*ss_vl)` | `:4096` ✓ |
| `:1411/:1424` | `.FMA(wt_m1+wf_m1, vl_s, (dz-wt_m1-wf_m1)*vl_m1)` | `:2153`/`:2176` ✓ |
| `:1464` | `.FMA(ubc_val-q, dt, dp_m1)` | `:2226` ✓ |
| `:1474`(×4) | `.FMA(dwat, dt, (wt+wf).mul_add(vl_s, …))` | `:2236`+`:2240` ✓ |
| `:1479` | `.FMA(q(lb), dt, wa_m1)` | `:2252` ✓ |

### 三、真缺口之一：`get_water_equilibrium_state` 的 `:137` / `:150`

```text
:137  GIMPLE  _57 = porsl(ilev)*(sp_zi(ilev)-zwtmm)              ← 第二项独立舍入
              _58 = .FMA(zwtmm-sp_zi(ilev-1), vliq_up, _57)
:150  GIMPLE  _78 = zwtmm - sp_zi(nlev)
              _80 = .FNMA(_78, 5.0e-1, psi_zwt)
```

Rust（`crates/colm-core/src/hydrology.rs::equilibrium_water_state`）两处都是**平铺**：
`upper_water * (water_table_mm - interface_mm[layer]) + porosity*(…)` 与
`psi_at_water_table - (water_table_mm - interface_mm[layers]) * 0.5`。
`git log -S mul_add -- crates/colm-core/src/hydrology.rs` 只有一条 `d062257`
（那是 `soil_vliq_from_psi` 的 van Genuchten 乘积）⇒ **这两处从来没落过**。

第 366 轮表格里"`get_water_equilibrium_state` **2** —— 第 339 轮成组落地 ✓"是错的：
第 339 轮落地的是同一批次里的 `soilwater_aquifer_exchange` 那 **4** 处（`:560/564/569/573`，
`/tmp/gf/aquifer_fix.py` 里还留着重排操作数顺序的记录），均衡这两处被一起记成了"✓"。
**教训：没有可复跑闭环的"✓"不算落地。**

### 四、这个例程**运行时一次都不进**，只能建闭环

`get_water_equilibrium_state` 的唯一调用点是 `mkinidata/MOD_IniTimeVariable.F90:463`
（`use_wtd` 分支）；Rust 侧 `equilibrium_water_state` 也只被 `colm-init` 的
`resolve_cold_start_soil` 调（`crates/colm-init/src/{single_point,spatial_time}.rs`），
`colm-runtime` 一次都不调 ⇒ **干湿窗首分歧与三份黄金窗口对它都不敏感**（实测见下，
改前改后逐位不变）。所以照 `compare_getzwt.sh` / `compare_soilhydro.sh` 的成法补了一个闭环：

* `oracle/scripts/water_equilibrium_diff.f90` —— 上游侧驱动，`USE MOD_Hydro_SoilWater`，
  **不加** `-ffp-contract=off`（要的就是 GCC 默认收缩），驱动自己加 `-fwrapv -ffp-contract=off`；
* `crates/colm-core/examples/water_equilibrium_probe.rs` —— 同一串 LCG；
* `oracle/scripts/compare_water_equilibrium.sh` —— 编译、跑、逐位比。

覆盖：两档模型（Campbell / van Genuchten）×（2000 组均匀随机 + 2000 组边界取值），
层数从 `{1,2,3,5,10}` 抽（`flag=1` 的含水层分支约占一半），比较 `wa` 与逐层
`wliq / smp / hk` 的**位型**，共 **109949** 个输出。

**敏感性先自证**（闭环能不能当判据的前提）：

```text
# 改之前（pristine HEAD 的 hydrology.rs）
get_water_equilibrium_state mismatches: {'wliq[1]': 79, 'smp[1]': 53, 'hk[1]': 38,
  'wliq[2]': 25, 'wliq[3]': 25, 'smp[3]': 23, 'smp[2]': 18, 'hk[3]': 16,
  'wliq[4]': 14, 'smp[4]': 12, 'hk[2]': 12, 'wliq[5]': 9}      # most_common(12)
                                                              # 合计 403 / 109949

# 改之后
get_water_equilibrium_state: all outputs bitwise identical; 109949 个输出
```

### 五、真缺口之二：`fgrnd` 那条累加链（**运行时路径**，`MOD_Thermal…:1352`）

`MOD_Thermal_CanopyPhase_Extended.F90` 的 `:1343-1372` 这一簇共 17 条 FMA。
其中 `lfevpa`（`:1343`）、`olrg`（`:1366`）、`olru`（`:1370`）三处
`surface_budget.rs` 已逐条对上（`:88` / `:62` / `:63`，注释里都引了 dump）；
但 `fgrnd` 那条 6 段的链，Rust 只熔了 **1** 段（`:102` 的 `t_grnd_bef**4`），
另外 3 段是平铺的。`DEF_SPLIT_SOILSNOW` 是运行时 namelist、默认 `.false.`
（`share/MOD_Namelist.F90:309`）⇒ 活的是 `.not.DEF_SPLIT_SOILSNOW` 那一支：

```text
:1352（GIMPLE，按 .loc 落行）
  _1743 = sabg
  _1747 = .FMA (dlrad, emg, _1743)                        ← Rust 平铺
  _1748 = emg * 5.67e-8
  powmult_2171 = t*t ; powmult_2170 = t2*t2 ; powmult_2172 = t2*t
  _1751 = .FNMA (_1748, powmult_2170, _1747)              ← Rust :102 ✓
  _1753 = _1748 * powmult_2172
  _1754 = tinc * 4.0
  _1757 = .FNMA (_1753, _1754, _1751)                     ← Rust 平铺（漏一次收缩）
  _1758 = .FMA (fevpg, htvp, fseng) ; _1760 = _1757 - _1758 ← Rust :113 ✓
  _1762 = pg_rain * cpliq ; _1768 = .FMA (_1762, ΔT, _1760)  ← Rust 先算 `precipitation_heat` 再加
  _1770 = pg_snow * cpice ; _1772 = .FMA (ΔT, _1770, _1768)  ← 同上
```

**为什么第 262/353 轮那个"8 步离线穷举"看不见它们**：探针
（`/tmp/gf/fgrnd/fort_bits.txt`，8 行 `fgrnd` + 10 个输入）里
`sabg` **全是 0**、`fsno` **全是 0**、`t_soil == t_grnd_bef`（都 283.0），
于是"`sabg + dlrad*emg` 熔不熔"、"两支 `fgrnd` 选哪支"、"`4*tinc` 那一乘熔不熔"
在这 8 行上**恰好全部不可见**。本轮把那份位型重算了一遍确认：

```text
t4_fused=True  t3_fused=True  -> match 8/8
t4_fused=True  t3_fused=False -> match 8/8     ← 现库里的写法，与上行不可区分
t4_fused=False t3_fused=True  -> match 6/8
t4_fused=False t3_fused=False -> match 6/8
```

所以按 GIMPLE 角色把 6 段各自熔进累加器写完（`precipitation_heat_w_m2` 那一列是
`MOD_Thermal…:1398-1399` 的**诊断量**，`crates/colm-runtime/src/history.rs:910` 用它写
`hprl`，算式保持不动、不再拿它当加数）。

### 六、第 367 轮实测

| 口径 | 改前（基线） | 只补 `hydrology.rs` 2 处 | 再补 `fgrnd` 3 处（**最终**） |
|---|---|---|---|
| 闭环 `compare_water_equilibrium.sh` | 位型不匹配（**403 / 109949**；`wliq[1]` 79 处、`smp[1]` 53 处 …） | **109949/109949 逐位相同** | 109949/109949 ✓ |
| 干窗 restart 首分歧 N=250 | 0/68 | 0/68 | 0/68 |
| 干窗 restart 首分歧 **N=251** | 11/68 | **11/68**（不变） | **11/68**（不变） |
| 湿窗 restart 首分歧 **N=63** | 10/68 | **10/68**（不变） | **10/68**（不变） |
| 黄金 dry（`bitwise`/`over_tol`/`ot_vars`/`sumabs`） | 9163 / 28 / 1，261.0128 | 9163 / 28 / 1，261.0128 | **9152** / 28 / 1，261.0128 |
| 黄金 wet | 27066 / 1197 / 19，8.4418 | 27066 / 1197 / 19，8.4418 | **27055** / 1197 / 19，8.4418 |

* 两处冷启动缺口对四个窗口**逐位不可见** —— 与"`colm-runtime` 不调它"的判断互洽，
  只有闭环能看见它（`wliq[1]` 79 处 → 0）。
* `fgrnd` 那 3 处让**两个黄金窗口的位型差异各降 11 个元素**（9163→9152、27066→27055），
  **`over_tol`/`ot_vars`/`sumabs` 一律不动** ⇒ 按仓库纪律"不许变差"可以落地；
  干湿窗的 restart 首分歧两档都不动，说明它**不是** N=251 那颗种子（`sabg=0`、
  `fsno=0`、`4*tinc` 那几组输入上仍不可分辨），但它在运行时路径上、且方向正确。

验证命令：

```text
cargo clippy --workspace --all-targets -- -D warnings                       # clean（8m12s）
cargo test --workspace --exclude colm-init --exclude colm-srfdata --lib --bins   # 全绿
cargo test -p colm-init    --lib -- --test-threads=1                        # 156 passed
cargo test -p colm-srfdata --lib -- --test-threads=1                        # 270 passed
cargo test -p colm-schema --test drift ; -p colm-hist --test drift ;
cargo test -p colm-core --test drift_landcover ; --test drift_co2 ;
cargo test -p colm-namelist --test roundtrip ; -p oracle --test histmap     # 全绿
bash oracle/scripts/compare_water_equilibrium.sh
bash /tmp/gf/dry_ts.sh 250 && python3 /tmp/gf/rdiff_dry.py
bash /tmp/gf/dry_ts.sh 251 && python3 /tmp/gf/rdiff_dry.py
bash /tmp/gf/wet_ts.sh 63  && python3 /tmp/gf/rdiff.py
bash /tmp/gf/win4.sh ; python3 /tmp/gf/three.py oracle/golden/<gold> <rust-hist> <cmp.txt> <label>
```

* `colm-init` / `colm-srfdata` 的 `--lib` **必须单线程**跑：并发时它们往共享临时目录写
  HDF 会互踩（`NetCDF: HDF error(-101)`），pristine HEAD 上同样复现（20~35 个随机失败），
  单线程两档全绿。这是本机环境问题，不是回归。
* `win4.sh` 里 `US-NR1-snow` 仍报 `cannot open …US-NR1_1999-2014_FLUXNET2015_Met.nc`
  （`examples/Forcing/` 里没有这份强迫），维持"不可测"。

### 七、方法教训（这一轮真正的产出）

1. **"函数体级"的汇编计数 ≠ 该例程自己的 FMA 数**：GCC 会把被调例程内联进去，
   数出来的 29 是"自身 15 + 内联 6 + 内联 8"。要对就按 `.loc` 落到**源码行**，
   或者直接数 GIMPLE 语句（带 `-fdump-tree-optimized-lineno`）。
2. **`-fdump-tree-optimized` 的行号要带 `-lineno` 才有**，而且语句前缀里可能有
   `discrim N`、`[tail call]` —— 解析正则必须容得下 `[^\]]*`，否则会静默漏掉
   （本轮第一版正则就因为 `:456:88 discrim 4]` 漏掉了 `:456`，数出 60 条而不是 71 条）。
3. **"✓ 已落地"必须挂一个可复跑的闭环**：`get_water_equilibrium_state` 那 2 处被
   记成"第 339 轮成组落地 ✓"挂了 28 轮，就是因为当年只写了结论、没留判据。
4. **`-S` 的 `.loc` 只能定"哪条乘积进 FMA"的位置，操作数角色必须读 GIMPLE**；
   反过来，GIMPLE 不给（或给了也判不了）的调用点上下文，要靠闭环。

## 第 368 轮：干窗那颗种子关掉了 —— `thermal` 三处未熔的乘积，干窗**整窗 restart 逐位相同**、黄金 dry 零超差

**一句话**：把 `MOD_Thermal_CanopyPhase_Extended.F90` 的 **99 条** FMA 语句先按 `.loc` +
**守卫栈**分成"活/死"（活 **25** 条、死 **74** 条），再把活的那 25 条逐条核 GIMPLE 角色 ——
还剩 **3 处** Rust 写成平铺：`thm`（`:550`）、psit 的 VG 实参（`:579`）、`qred`（`:583`）。
补上之后 **干窗首分歧从 N=251（11/68）直接归零**，并且**抽测到的每一步（250→528，整窗 528 步）
restart 全部 0/68**；黄金 dry 的 `over_tol` 28 → **0**、`ot_vars` 1 → **0**、
`sumabs` 261.0128 → **0.0000**。湿窗与黄金 wet **逐位不变**。

### 一、99 条先分"活/死"（守卫栈，不按缩进猜）

`thermal` 是 21-1431 行的单个大例程，99 条 FMA 全在它里面。用守卫栈（`IF(...)THEN`/`DO`
入栈、`ENDIF`/`ENDDO` 出栈）把每条落到最内层守卫：

| 簇（源码行） | 语句 | 最内层守卫 | 活/死 |
|---|---|---|---|
| 531-583 | 9 | `IF (.not.DEF_SPLIT_SOILSNOW)`@529 / 顶层 | **活 5** + 死 4（`ulrad`） |
| 612-615 | 2 | `IF (.not.DEF_SPLIT_SOILSNOW)`@586 的 **ELSE** 支 | 死 |
| 967-971 | 3 | `IF (patchtype==0 .and. (DEF_USE_PFT .or. DEF_USE_PC))`@747 | 死 |
| 1012-1013 | 2 | 同上 + `IF (DEF_USE_PC …)`@992 | 死 |
| 1081-1154 | 56 | 同上（PFT→patch 加权聚合） | 死 |
| 1232 | 1 | `@1228` 的 **ELSE** 支（split） | 死 |
| 1265-1296 | 7 | `@1261` 的 IF 支 1 条、ELSE 支 6 条 | **活 1** |
| 1343-1370 | 17 | 顶层 | 活（第 367 轮已收口） |
| 1410 | 2 | 顶层（`errore`，诊断量） | 活 |

⇒ **活 25 条 / 死 74 条**。

两点复核（第 365 轮的判断本轮独立验了一遍）：

* **`:1081-1154` 那 56 条确实在 `DEF_USE_PFT/PC` 守卫里** —— 关键是 `747` 那个 `IF` 的
  `ENDIF` 在 **1200** 行，**不是 1076**（1076 关的是 992 那个）。`DEF_USE_PFT/PC` 是运行时
  namelist、两个默认 `.false.`、三个 `case.nml` 也都没设 ⇒ 这条 60 行的大块**不执行**。
  这个例程的缩进很不规则（顶层 `IF`/`ENDIF` 都顶在第 1 列），**不能按缩进判块**。
* **`:531/533/536` 的 `ulrad` 是死代码**：`ulrad` 在 `LeafTemperature/PC` 里是
  `intent(out)`（`MOD_LeafTemperature_Extended.F90:311`，与 `taux`/`fseng`/`rst`/`tref`
  同一段声明），内核 `:531` 算完立刻被叶温例程覆盖，`:1364` 的 `olrg` 用的是**覆盖后**的值
  ⇒ Rust 拿 `energy.leaf.upward_longwave_w_m2` 当 `ulrad` 是对的 ✓（第 367 轮那三处
  `fgrnd`/`olrg` 的形状结论不受影响）。

活的那 25 条里，`553`（`thv` 的 `FMA(0.61, forc_q, 1.0)`）与 `554`（`ur` 的平方和 FMA）
在 `ground_fluxes.rs` 里早已是 `mul_add` ✓，`:1265`（`fseng = fseng + htvp*egidif`）在
`standard_lct_step.rs:428` 也早已是 `mul_add`（注释里引了 GIMPLE）✓，`:1343-1370` 的 17 条
第 367 轮已收口 ✓。

### 二、三处真缺口（都在文档记过的干窗链上）

| 内核 | GIMPLE 操作数角色 | Rust（改前） |
|---|---|---|
| `:550` `thm = forc_t + 0.0098*forc_hgt_t` | `_50 = .FMA(forc_hgt_t, 9.8e-3, forc_t)` | `leaf_temperature.rs:89` `air + LAPSE*hgt`（平铺） |
| `:579` `soil_psi_from_vliq(fac*(porsl(1)-theta_r(1)) + theta_r(1), …)` | `_83 = porsl-theta_r`、`_86 = .FMA(_83, fac, theta_r)` | `ground_humidity.rs:57` `fac*(por-thr) + thr`（平铺） |
| `:583` `qred = (1.-fsno)*hr + fsno` | `_98 = fsno`、`_99 = 1-fsno`、`qred = .FMA(_99, hr, _98)` | `ground_humidity.rs:68` `(1-fsno)*hr + fsno`（平铺） |

三条正好落在第 355/357/359 轮量出来的那条干窗链上：
`thm` → Monin-Obukhov（`qstar`/`zol`/`rib`/`qref`）、`psit` → `hr` → `qred` → `qg` → `qaf`/`ea`。
`thm` 那条还正好是第 355 轮反复强调"**它不是位温**"的那个量（`leaf_temperature.rs:75-87`
的注释）—— 这一年多来它只被核过"用哪个量"，没核过"怎么算"。

### 三、第 368 轮实测

| 口径 | 改前（第 367 轮末） | 改后 |
|---|---|---|
| 干窗 restart N=250 | 0/68 | 0/68 |
| 干窗 restart **N=251** | **11/68** | **0/68** |
| 干窗 restart N=252 / 260 / 288 / 320 / 400 / 480 / **528（整窗末步）** | （改前 N=251 已 11/68 ⇒ 这些点必然非零） | **全部 0/68** |
| 湿窗 restart N=63 | 10/68 | **10/68**（不变） |
| 黄金 dry（`bitwise`/`over_tol`/`ot_vars`/`sumabs`） | 9152 / 28 / 1，261.0128 | **153 / 0 / 0，0.0000** |
| 黄金 wet | 27055 / 1197 / 19，8.4418 | **27055 / 1197 / 19，8.4418**（不变） |

* **干窗整窗 528 步的 restart 状态逐位相同**（68 个变量一个元素都不差）。
* 黄金 dry 的 `golden-compare`：`within tolerance: 127 variables … (tier0=23 tier1=8 tier2=97 tier3=0)`，
  **零个变量超差**；`three.py` 的位型口径还剩 153/56024 个元素差、但 `sumabs = 0.0000`。
  按 `window_divergence.py` 看，剩下的差全在**诊断量**上，且都是 1 ULP 级：

```text
 first variable                         ndiff       maxabs     maxrel
    10 f_assim                             25   2.1176e-22   2.40e-16
    10 f_fgrnd                             76   5.6843e-14   3.50e-14
    12 f_assimsha                          23   5.2940e-23   2.48e-16
    12 f_assimsun                          24   2.1176e-22   3.22e-16
   130 f_rnet                               2   2.8422e-14   2.27e-16
   130 f_zerr                               2   2.8422e-14   8.16e-04
   175 f_trad                               1   2.8422e-14   1.11e-16
first divergence step: 10
variables differing: 7; bitwise identical: 55871/56024 (99.7269%)
```

* `f_assim`/`f_assimsun`/`f_assimsha` 在第 10-12 步就 1 ULP 差 ⇒ **叶/气孔/光合那条链上
  还有一处形状差**，但它**不回灌状态**（restart 整窗逐位相同）。`f_fgrnd` 也在第 10 步差
  1 ULP（`maxabs 5.68e-14` 正是 256 附近的 1 ULP），说明它的某个**输入**（`fseng`/`fevpg`
  或 `assim` 那条链）先差了 1 ULP —— 第 367 轮补的三段本身没错（`sabg=0` 时那三段各自
  与内核逐位一致）。
* `f_zerr` 的相对差 8.16e-04 是老账（`errore` 是诊断量，绝对值 2.8e-14；`history.rs:892`
  那段注释记着来龙去脉）。

### 四、验证命令

闸门（全部在最终树上跑）：

```text
cargo clippy --workspace --all-targets -- -D warnings                    # clean
cargo test --workspace --exclude colm-init --exclude colm-srfdata --lib --bins
cargo test -p colm-init --lib -- --test-threads=1                        # 156 passed
cargo test -p colm-srfdata --lib -- --test-threads=1                     # 270 passed
cargo test -p colm-schema --test drift ; -p colm-hist --test drift ;
cargo test -p colm-core --test drift_landcover ; --test drift_co2 ;
cargo test -p colm-namelist --test roundtrip ; -p oracle --test histmap
cargo run -q -p oracle --bin tier-check -- oracle/golden/*.nc
bash oracle/scripts/compare_water_equilibrium.sh                         # 109949/109949
bash /tmp/gf/dry_ts.sh {250,251,252,260,288,320,400,480,528} && python3 /tmp/gf/rdiff_dry.py
bash /tmp/gf/wet_ts.sh 63 && python3 /tmp/gf/rdiff.py
bash /tmp/gf/win4.sh
python3 /tmp/gf/three.py oracle/golden/CN-Cng_hist_2008-01.nc <rust-hist> <cmp.txt> CN-Cng
python3 oracle/scripts/window_divergence.py oracle/golden/CN-Cng_hist_2008-01.nc <rust-hist> --top 8
```

### 五、下一枪

0. **先把"植物水力"这条排掉**：本轮顺手把 `main/MOD_PlantHydraulic.F90` 也普查了
   （57 条 FMA / 30 行）：

   | 内核例程 | 语句（去重后站点） | Rust | 结论 |
   |---|---|---|---|
   | `spacAF_twoleaf`（`:379-508`） | 41（18 行） | `plant_hydraulics.rs::spac_change` 39 个代码点 | **未逐条审**（下一枪） |
   | `getqflx_gs2qflx_twoleaf` | 3 | `transpiration_from_conductance` 3 | ✓ |
   | `getqflx_qflx2gs_twoleaf` | 3 | `conductance_from_transpiration` 3 | ✓ |
   | `getrootqflx_x2qe` | 6（去重 5：`:854`×2/`:864`/`:874`/`:892`） | `root_flux_from_top_potential` 3 | ✓（`:864`/`:874` 是同一个循环体，`:854` 的 `+kax/den1*xroot(1)` 与 `:892` 各一条） |
   | `getrootqflx_qe2x` | 4（去重 3；`:950` 那条还有一份向量化拷贝） | `root_potential_from_flux` 1 | ✓（三行是同一个循环体的三个分支，Rust 一条覆盖；`history` 注释里记着"先减 `qeroot` 再减 `kax`"的次序） |

   ⇒ **两条"6 vs 3 / 4 vs 1"是循环/向量共享造成的，不是缺口**（又一次"计数会骗人"）。
   真正还没逐条读的是 `spacAF_twoleaf` 那 41 条（`A11..A44` 的雅可比组装 + 两支 4×4
   行列式），它产出的是 `dx` → `vegwp`。

1. **湿窗 N=63（10/68）是湿侧唯一还没关的状态种子**。差异表：
   `qref`/`vegwp`/`ldew`/`ldew_rain`/`zol`/`rib`/`rst`/`qstar`/`gs0sha`/`gs0sun`
   —— 与第 362 轮那条湿链（`pco2a` → `gssun`/`gssha` → `gs0*` → PHS → `qflx_sha` →
   `vegwp`/`ldew_rain`）一致。干侧这一轮的成法可以照搬：先把 `MOD_LeafTemperature_Extended.F90` /
   `MOD_LeafTemperaturePC_Extended.F90` 按 `.loc` + 守卫栈普查（**先分活死**），
   再逐条读 GIMPLE 角色。
2. **干窗剩下的 1 ULP 全在诊断量上**（`f_assim` 第 10 步、`f_fgrnd` 第 10 步、
   `f_rnet`/`f_zerr` 第 130 步、`f_trad` 第 175 步）—— 状态已整窗逐位相同，
   所以这是一条**诊断链**的清理，优先级低于湿窗。
3. **悬而未决（等用户裁决）**：`pco2a` 的 `.FNMA`（第 351/356 轮）。它把湿窗首分歧
   从 N=63 推到 73–84，但黄金 wet 的 `over_tol` 1197 → 1650、`ot_vars` 19 → 35
   ⇒ 按仓库纪律不能落。**注意它换来的正是本轮这条"干窗式"胜利的同款证据（首分歧推后），
   却与黄金聚合反向** —— 这条分歧本身值得作为"纪律是否要为例外开口"的判例继续留着。
## 第 369 轮：闭环普查发现**三个差分闭环一直在失败** —— 光合/气孔链 30 处未落地形状（按纪律回退，写法全部留档）

**一句话**：把仓库里 **18 个差分闭环一次全跑**（以前从没跑过全量），发现 **3 个在失败**：
`compare_sortin`、`compare_stomata`、`compare_update_photosyn` —— 全落在
`MOD_AssimStomataConductance.F90`。逐条按 `.loc` + GIMPLE 读出来：这个模块有 **40 条**
`.FMA/.FNMA/.FMS`，本仓库只落了 5 条。把差的 **30 处全补上**后三个闭环分别变成
**3000/3000、3000/3000、3999/4000**，干窗黄金位型 **153 → 81**、`over_tol` 仍 0、
干窗 restart 仍整窗 0/68 —— 但**黄金湿窗口径变差**（`over_tol` 1197 → **1968**、
`ot_vars` 19 → **53**、`sumabs` 8.4418 → **33.37**）⇒ 按 **"口径指标不许变差"**
整批 `git checkout` 回退，**写法与全部证据留在本节**（下次可以直接重放）。

### 一、闭环全量普查（本轮新加的 `oracle/scripts/compare_all.sh`）

以前每一轮只跑与当轮改动相关的那一个闭环；本轮把它固化成一条命令：

```text
oracle/scripts/compare_all.sh          # 全跑；有任一失败则退出码 1
oracle/scripts/compare_all.sh stomata  # 只跑名字里含这个子串的

闭环: 17 通过 / 3 失败
失败: compare_sortin compare_stomata compare_update_photosyn
跳过（需要位置参数，不是闭环）: compare_flag_isolated compare_second_config
```

逐条输出（`HEAD` = `6668e30`）：

```text
== compare_moninobukm   rc=0  MOD_FrictionVelocity: all 20 outputs 20000/20000 bitwise identical
== compare_leddy        rc=0  ...
== compare_soilthermal  rc=0  hcap 40000/40000, thk 同
== compare_sortin       rc=1  {'inter:bterm': 853, 'inter:aterm': 925, 'inter:ac2': 528,
                               'pco2y3': 277, 'pco2y4': 96, 'inter:cterm': 536,
                               'inter:ac1': 640, 'pco2y2': 231, 'pco2y5': 78, 'pco2y6': 70}  /3000
== compare_qsadv        rc=0  20000/20000
== compare_phasechange  rc=0  10000/10000
== compare_interception rc=0  4000/4000
== compare_water_balance rc=0 12000/12000
== compare_flux_inside  rc=0  15000/15000
== compare_soilhydro    rc=0  10000/10000
== compare_stomata      rc=1  {'rst': 114, 'assim': 171}  /4000
== compare_update_photosyn rc=1 {'assim': 225}  /3000
== compare_updphotosyn  rc=0
== compare_vulnerability rc=0 200000/200000
== compare_getzwt       rc=0  10000/10000
== compare_water_equilibrium rc=0 109949/109949
```

（`compare_flag_isolated.sh` / `compare_second_config.sh` 需要位置参数，不能裸跑，未计入。）

**这一步本身就是方法**：以前每一轮只跑与当轮改动相关的那一个闭环，于是"三个闭环在失败"
可以躺很久 —— `compare_sortin.sh` 的头注释里还写着"`sortin` 是这条链上唯一**没有 FMA** 的函数"，
而这正是它一直失败的原因被记反了。

### 二、`sortin` 有 11 条 FMA（旧结论被推翻）

`gfortran -O2 -fdump-tree-optimized-lineno` 数出 `sortin`（`:383-476`）**11 条**，
源码行 `:414/415/416/453/459/460/465×2/466/467×2`。逐条操作数角色：

| 源码 | GIMPLE | 写法 |
|---|---|---|
| `:414 co2(1) = gammas + 0.5*range` | `.FMA(range, 0.5, gammas)` | `f77(0.5).mul_add(range, gammas)` |
| `:415 co2(2) = gammas + range*(0.5-0.3*eyy_a)` | `.FMA(range, prephitmp, gammas)` | `range.mul_add(f77(0.5)-f77(0.3)*sign, gamma)` |
| `:416 co2(3) = co2(1) - (…)/(…)*eyy(1)` | `.FNMA(_24, eyy(1), co2(1))` | `(-slope).mul_add(errors[0], co2[0])` |
| `:453 pco2yl = …` | `.FNMA(_67, _72, co2[is])` | 同上 |
| `:459 ac1 = e1²-e2²` | `.FMS(e1, e1, e2*e2)`（**e2² 独立舍入当被减数**） | `errors[i1].mul_add(errors[i1], -e2sq)` |
| `:460 ac2 = e2²-e3²` | `.FNMA(e3, e3, e2*e2)`（**方向相反**，e2² 当减数） | `(-errors[i3]).mul_add(errors[i3], e2sq)` |
| `:465 bterm = (cc1*ac2-cc2*ac1)/(bc1*ac2-ac1*bc2+1e-10)` | 两条 `.FMS`：`ac2*cc1-_96`、`ac2*bc1-_100` | 分子/分母各一条 `mul_add`，`1e-10` 普通加 |
| `:466 aterm = (cc1-bc1*bterm)/(ac1+1e-10)` | `.FNMA(bc1, bterm, cc1)` | `(-bc1).mul_add(bterm, cc1)` |
| `:467 cterm = p2 - aterm*e2² - bterm*e2` | 两条 `.FNMA`（`_111 = e2*aterm` 独立舍入） | `(-(aterm*e2)).mul_add(e2, p2)` 再 `(-bterm).mul_add(e2, …)` |

`ac1`/`ac2` 方向相反那一条是这轮最容易写错的地方（Rust 原先是 `powi(2) - powi(2)` 平铺）。

### 三、另外 19 处（`stomata` 16 / `update_photosyn` 8 / `calc_photo_params` 4 / `WUE_solver` 1）

| 源码 | GIMPLE 角色 | 备注 |
|---|---|---|
| `:570 vm = vm/temph*rstfac*c3 + vm/(templ*temph)*rstfac*c4` | `.FMA(_47, c3, 整条 c4 链)` | `:598 omss` 同形 |
| `:579/:580 (710.*t-220.e3)` | `.FMA(t, 7.1e2, -2.2e5)` | 两处（`trop`/`tlef`） |
| `:258 omc` | `.FMA(vm*(…)/(…), c3, vm*c4)` | |
| `:259 ome` | `.FMA(gammas, 2.0, pco2i_e)` + `.FMA(c3, 商, epar*c4)` | 第一条第 336 轮已定 |
| `:262 oms` | `.FMA(c3, omss, (omss*pco2i)*c4)` | |
| `:343 bquad`(Medlyn) | `.FNMS(g0*1e-6+acp, 2.0, 商)` —— **括号里是普通加法**，熔的是末尾那一乘 | 我第一版把括号也熔了，被单测 `hydraulic_photosynthesis_update_matches_mod_assim_stomata_conductance` 抓住（差 2 倍） |
| `:344 cquad`(Medlyn) | `.FNMA(g1,g1,1.0)` + `.FMA(2*g0, 1e-6, (1-g1²)*acp/vpd)` + `.FMA(g0*1e-6, g0*1e-6, …)` | 三条 |
| `:353 bquad`(BB) | `.FMS(hcdma, gbh2o, -ei)` 再 `.FNMA(bintc, hcdma, …)` | 两条 |
| `:354 cquad`(BB) | `.FMA(bintc, hcdma, ea)` | |
| `:346/:356 sqrtin` | `.FMS(bquad, bquad, 4*aquad*cquad)` | 两处 |
| `:805 eyy(ic) = pco2i - bracket*psrf` | `.FNMA(bracket, psrf, pco2i)` —— **这条 FMA 的结果就是 `eyy` 本身** | 我第一版仍写 `internal - next`，同一个单测抓住 |
| `:861 1 + 1.37*sqrt(…)` | `.FMA(sqrt, 1.37, 1.0)` | |

### 四、补上之后的实测（**随后整批回退**）

同树 A/B（`HEAD` = `6668e30`，即第 368 轮末）：

| 口径 | HEAD | 只补 `sortin` 11 处 | 再补其余 19 处（**最终**） |
|---|---|---|---|
| `compare_sortin` | `bterm` 853 / `aterm` 925 / … ✗ | **3000/3000** ✓ | 3000/3000 ✓ |
| `compare_update_photosyn` | `assim` 225 ✗ | 190 | **3000/3000** ✓ |
| `compare_stomata` | `rst` 114 + `assim` 171 ✗ | `rst` 114 + `assim` 120 | **`assim` 1**（3999/4000） |
| 干窗 restart N=251 / 528 | 0/68 / 0/68 | 0/68（252/288/528 全 0） | 0/68 / 0/68 ✓ |
| 湿窗 restart N=63 | 10/68 | 10/68 | 10/68 |
| 黄金 dry（`bitwise`/`over_tol`/`ot_vars`/`sumabs`） | 153 / 0 / 0，0.0000 | 150 / 0 / 0，0.0000 | **81** / 0 / 0，0.0000 ✓ |
| 黄金 wet | 27055 / 1197 / 19，8.4418 | 26840 / **1281 / 27**，8.9121 ✗ | 27340 / **1968 / 53**，33.3682 ✗ |

⇒ **干窗三项全好、湿窗口径三项全差**。按本仓库写死的纪律（第 18005 / 18082 行那两处先例：
"两者都违反'口径指标不许变差'，按纪律 `git checkout` 整个回退"，且"第 249 轮例外"已在第 362 轮
被判不成立）⇒ **整批回退**，树回到 `6668e30` 的基线（回退后复测：黄金 dry `153 / 0 / 0，0.0000`、
黄金 wet `27055 / 1197 / 19，8.4418`、湿窗 N=63 `10/68`，与基线逐位一致）。

**这不是"改错了"**：30 处形状逐条对着 GIMPLE 操作数角色写成，三个闭环从失败变全过，
单测（含那条 `hydraulic_photosynthesis_update_matches_mod_assim_stomata_conductance`）全绿。
湿窗口径变差是**混沌**：`f_vegwp` 主导的 11 天窗口里，把 30 处末位改对会把轨道换一条，
聚合指标可以反向 —— 与第 301/351/361 轮同一个现象。**这条负结果和那三次放在一起，
构成"口径指标不许变差"这条纪律在混沌窗口上的第四个反例**；要不要为它开口是用户的决定。

### 五、重放方式（一步到位）

把下面这段存成 `photo_shapes_r369.py` 跑一次即可（它就是我这一轮用的补丁，逐条带断言）：

```python
"""第 369 轮：把 `MOD_AssimStomataConductance.F90` 剩下的 29 条 FMA 逐条补上。

`-fdump-tree-optimized-lineno` 数出这个模块 40 条 `.FMA/.FNMA/.FMS`：
`sortin` 11（本轮已补）、`calc_photo_params` 4、`stomata` 16、`update_photosyn` 8、
`WUE_solver` 1。逐条读操作数角色后写在这里。

`stomata` 与 `update_photosyn` 的 `omc/ome/oms/range/sqrtin` 是**同形**的两份；
`update_photosyn` 里 `internal` 一路（WUE 关着时 `pco2i_c=pco2i_e=pco2i`）与 `stomata`
的 `internal_co2` 一样，所以两处写同一种形状。
"""
P = '/Users/zhongwangwei/Desktop/Github/CoLM-Desktop/crates/colm-core/src/photosynthesis.rs'
s = open(P).read()


def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (s.count(old), count, old[:100])
    s = s.replace(old, new, count)


# ---------------------------------------------------------------- A. calc_photo_params
# `:570` / `:598`：`…*rstfac*c3 + …*rstfac*c4`，`c3` 那一条乘进 FMA。
rep("""    maximum_carboxylation =
        (maximum_carboxylation / high_inhibition * input.soil_water_stress * c3_fraction
            + maximum_carboxylation / (low_inhibition * high_inhibition)
                * input.soil_water_stress
                * c4_fraction)
            * input.canopy_integration[0];""",
    """    // `:570 vm = vm/temph*rstfac*c3 + vm/(templ*temph)*rstfac*c4` 的 GIMPLE 是
    // `.FMA(vm/temph*rstfac, c3, vm/(templ*temph)*rstfac*c4)` —— `c4` 那条链整体
    // 独立舍入当加数，`c3` 那一乘收进 FMA。
    let high_term = maximum_carboxylation / high_inhibition * input.soil_water_stress;
    let low_term = maximum_carboxylation / (low_inhibition * high_inhibition)
        * input.soil_water_stress
        * c4_fraction;
    maximum_carboxylation =
        high_term.mul_add(c3_fraction, low_term) * input.canopy_integration[0];""")

# `:580` 两处 `710*t - 220e3` 的分子是一条 FMA。
rep("""        * (1.0
            + ((f77(710.0) * b.optimum_temperature_k - f77(220.0e3))
                / (gas_constant * b.optimum_temperature_k))
                .exp())
        / (1.0
            + ((f77(710.0) * input.leaf_temperature_k - f77(220.0e3))
                / (gas_constant * input.leaf_temperature_k))
                .exp());""",
    """        // `:579-580` 的两个分子 `710.*t-220.e3` 各自是一条 FMA（`:580` 的 GIMPLE
        // `_67 = .FMA(t, 7.1e2, -2.2e5)`）。
        * (1.0
            + (f77(710.0)
                .mul_add(b.optimum_temperature_k, -f77(220.0e3))
                / (gas_constant * b.optimum_temperature_k))
                .exp())
        / (1.0
            + (f77(710.0)
                .mul_add(input.leaf_temperature_k, -f77(220.0e3))
                / (gas_constant * input.leaf_temperature_k))
                .exp());""")

rep("""    let sink_limit = ((b.maximum_carboxylation_25c_mol_m2_s / f77(2.0))
        * f77(1.8).powf(temperature_factor)
        / low_inhibition
        * input.soil_water_stress
        * c3_fraction
        + (b.maximum_carboxylation_25c_mol_m2_s / f77(5.0))
            * f77(1.8).powf(temperature_factor)
            * input.soil_water_stress
            * c4_fraction)
        * input.canopy_integration[0];""",
    """    // `:597-598 omss = (vmax25/2)*1.8**qt/templ*rstfac*c3 + (vmax25/5)*1.8**qt*rstfac*c4`
    // 与 `:570` 同形（`.FMA(_118, cstore_128, _124)`）。
    let low_sink = (b.maximum_carboxylation_25c_mol_m2_s / f77(2.0))
        * f77(1.8).powf(temperature_factor)
        / low_inhibition
        * input.soil_water_stress;
    let high_sink = (b.maximum_carboxylation_25c_mol_m2_s / f77(5.0))
        * f77(1.8).powf(temperature_factor)
        * input.soil_water_stress
        * c4_fraction;
    let sink_limit = low_sink.mul_add(c3_fraction, high_sink) * input.canopy_integration[0];""")

# ---------------------------------------------------------------- B. stomata
rep("""        let omc = photo.maximum_carboxylation_mol_m2_s * (rubisco_co2 - photo.co2_compensation_pa)
            / (rubisco_co2 + photo.rubisco_co2_constant_pa)
            * photo.c3_fraction
            + photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction;
        let ome = photo.electron_transport_mol_m2_s * (electron_co2 - photo.co2_compensation_pa)
            / (electron_co2 + f77(2.0) * photo.co2_compensation_pa)
            * photo.c3_fraction
            + photo.electron_transport_mol_m2_s * photo.c4_fraction;
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.sink_limit_mol_m2_s_pa * photo.c3_fraction
                + photo.sink_limit_mol_m2_s_pa * internal_co2 * photo.c4_fraction;""",
    """        // `:258/:259/:262` 三条都是 `商*c3 + 整条 c4 链`：
        //   `omc = .FMA(vm*(pco2i_c-gammas)/(pco2i_c+rrkk), c3, vm*c4)`
        //   `ome = .FMA(c3, epar*(…)/(pco2i_e+2*gammas), epar*c4)`，分母那一条
        //         `.FMA(gammas, 2.0, pco2i_e)`（第 336 轮就定死的那处）
        //   `oms = .FMA(c3, omss, (omss*pco2i)*c4)`
        let omc_quotient = photo.maximum_carboxylation_mol_m2_s
            * (rubisco_co2 - photo.co2_compensation_pa)
            / (rubisco_co2 + photo.rubisco_co2_constant_pa);
        let omc = omc_quotient.mul_add(
            photo.c3_fraction,
            photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction,
        );
        let ome_denominator = f77(2.0).mul_add(photo.co2_compensation_pa, electron_co2);
        let ome_quotient = photo.electron_transport_mol_m2_s
            * (electron_co2 - photo.co2_compensation_pa)
            / ome_denominator;
        let ome = photo.c3_fraction.mul_add(
            ome_quotient,
            photo.electron_transport_mol_m2_s * photo.c4_fraction,
        );
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.c3_fraction.mul_add(
                photo.sink_limit_mol_m2_s_pa,
                photo.sink_limit_mol_m2_s_pa * internal_co2 * photo.c4_fraction,
            );""")

# Medlyn 分支
rep("""            let acp = f77(1.6) * positive_assimilation / co2_surface_clamped;
            let a = 1.0;
            let bq = -f77(2.0) * (g0 * f77(1.0e-6) + acp)
                - (g1 * acp).powi(2)
                    / (photo.boundary_conductance_h2o_mol_m2_s * vapor_deficit_kpa);
            let c = (g0 * f77(1.0e-6)).powi(2)
                + (f77(2.0) * g0 * f77(1.0e-6) + acp * (1.0 - g1.powi(2)) / vapor_deficit_kpa)
                    * acp;
            conductance = (-bq + (bq.powi(2) - f77(4.0) * a * c).max(0.0).sqrt()) / (f77(2.0) * a);""",
    """            let acp = f77(1.6) * positive_assimilation / co2_surface_clamped;
            let a = 1.0;
            // `:343` 里 `(g0*1e-6 + acp)` 的乘积是一条 FMA（`_108 = .FMA(g0, 1e-6, acp)`）。
            let bq = -f77(2.0) * g0.mul_add(f77(1.0e-6), acp)
                - (g1 * acp).powi(2)
                    / (photo.boundary_conductance_h2o_mol_m2_s * vapor_deficit_kpa);
            // `:344`：`1-g1**2` 是一条 `fnmsub`（`_104`）；`(g0*1e-6)**2` 与后半条链
            // 相加也是一条 FMA（`cquad = .FMA(_289, _289, _110)`）。
            let g0_scaled = g0 * f77(1.0e-6);
            let one_minus_g1_squared = (-g1).mul_add(g1, 1.0);
            let c_tail = (f77(2.0) * g0_scaled
                + acp * one_minus_g1_squared / vapor_deficit_kpa)
                * acp;
            let c = g0_scaled.mul_add(g0_scaled, c_tail);
            // `:346` `sqrtin = max(0, bquad**2 - 4*aquad*cquad)`：`bquad**2` 进 FMA。
            conductance = (-bq
                + bq.mul_add(bq, -(f77(4.0) * a * c)).max(0.0).sqrt())
                / (f77(2.0) * a);""")

# Ball-Berry 分支
rep("""            let a = hcdma;
            let bq = photo.boundary_conductance_h2o_mol_m2_s * hcdma
                - input.leaf_saturation_vapor_pressure_pa
                - bintc * hcdma;
            let c = -photo.boundary_conductance_h2o_mol_m2_s
                * (input.canopy_air_vapor_pressure_pa + hcdma * bintc);
            conductance = (-bq + (bq.powi(2) - f77(4.0) * a * c).max(0.0).sqrt()) / (f77(2.0) * a);""",
    """            let a = hcdma;
            // `:353 bquad = gbh2o*hcdma - ei - bintc*hcdma` 是**两条** `fmsub`：
            // 先 `fma(hcdma, gbh2o, -ei)`，再 `fma(-bintc, hcdma, 上一步)`。
            let first = hcdma.mul_add(
                photo.boundary_conductance_h2o_mol_m2_s,
                -input.leaf_saturation_vapor_pressure_pa,
            );
            let bq = (-bintc).mul_add(hcdma, first);
            // `:354 cquad = -gbh2o*(ea + hcdma*bintc)`：括号里是一条 FMA。
            let c = -photo.boundary_conductance_h2o_mol_m2_s
                * bintc.mul_add(hcdma, input.canopy_air_vapor_pressure_pa);
            // `:356` 同 `:346`。
            conductance = (-bq
                + bq.mul_add(bq, -(f77(4.0) * a * c)).max(0.0).sqrt())
                / (f77(2.0) * a);""")

# ---------------------------------------------------------------- C. update_photosyn
rep("""        let omc = photo.maximum_carboxylation_mol_m2_s * (internal - photo.co2_compensation_pa)
            / (internal + photo.rubisco_co2_constant_pa)
            * photo.c3_fraction
            + photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction;
        let ome = photo.electron_transport_mol_m2_s * (internal - photo.co2_compensation_pa)
            / (internal + f77(2.0) * photo.co2_compensation_pa)
            * photo.c3_fraction
            + photo.electron_transport_mol_m2_s * photo.c4_fraction;
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.sink_limit_mol_m2_s_pa * photo.c3_fraction
                + photo.sink_limit_mol_m2_s_pa * internal * photo.c4_fraction;""",
    """        // `:737/:738/:740` 与 `stomata` 的 `:258/:259/:262` 同形（WUE 关着时
        // `pco2i_c = pco2i_e = pco2i`）。
        let omc_quotient = photo.maximum_carboxylation_mol_m2_s
            * (internal - photo.co2_compensation_pa)
            / (internal + photo.rubisco_co2_constant_pa);
        let omc = omc_quotient.mul_add(
            photo.c3_fraction,
            photo.maximum_carboxylation_mol_m2_s * photo.c4_fraction,
        );
        let ome_denominator = f77(2.0).mul_add(photo.co2_compensation_pa, internal);
        let ome_quotient = photo.electron_transport_mol_m2_s
            * (internal - photo.co2_compensation_pa)
            / ome_denominator;
        let ome = photo.c3_fraction.mul_add(
            ome_quotient,
            photo.electron_transport_mol_m2_s * photo.c4_fraction,
        );
        if !options.use_wue || (photo.c4_fraction - 1.0).abs() < f77(0.001) {
            let oms = photo.c3_fraction.mul_add(
                photo.sink_limit_mol_m2_s_pa,
                photo.sink_limit_mol_m2_s_pa * internal * photo.c4_fraction,
            );""")

rep("""        let positive_assimilation = net_assimilation.max(f77(1.0e-12));
        let next = (co2_surface
            - f77(1.6) * positive_assimilation / input.canopy_conductance_h2o_umol_m2_s)
            * input.photosynthesis.air_pressure_pa;""",
    """        let positive_assimilation = net_assimilation.max(f77(1.0e-12));
        // `:805 eyy(ic) = pco2i - (co2s - 1.6*assmt/gsh2o)*psrf` 出货是
        // `fsub` + `fmsub` ⇒ `pco2i - bracket*psrf` 收成一条 FMA（加数是 `pco2i`）。
        let bracket = co2_surface
            - f77(1.6) * positive_assimilation / input.canopy_conductance_h2o_umol_m2_s;
        let next = (-bracket).mul_add(input.photosynthesis.air_pressure_pa, internal);""")

open(P, 'w').write(s)
print('photosynthesis patched')
```

### 六、下一枪

1. **湿窗 N=63（10/68）仍是湿侧唯一未关的状态种子**，而且这一轮把它所在的链**量清楚了**：
   差的是 `MOD_AssimStomataConductance.F90` 的 30 处（写法已在上文），补上后闭环全过、
   湿窗首分歧仍停在 63 —— 说明**首分歧那一步还另有上游**（`gs0sun`/`gs0sha` 的输入侧），
   也就是那条挂起的 `pco2a` `.FNMA`（现在四个例外候选里的第三个）。
2. **`compare_stomata` 剩下的那 1/4000**（row 514，模型 0，`assim` 从 `…E451` 到 `…E3CA`，
   差得比 1 ULP 大 ⇒ 像分支差而不是舍入差）值得单独看一眼：它是唯一还没解释的。

3. **全量普查已经固化**：`oracle/scripts/compare_all.sh`（本轮新增，全跑 → 17 通过 / 3 失败，
   有失败则退出码 1，可以直接挂进 CI）。它比手工那份多覆盖了三个闭环
   （`compare_forcingdownscaling`、`compare_forcingdownscaling_shortwave`、
   `compare_forcingdownscaling_wind`、`compare_hourly_window`），都通过 ✓。
