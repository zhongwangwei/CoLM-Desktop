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
