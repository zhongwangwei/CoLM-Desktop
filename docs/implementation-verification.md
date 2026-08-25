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
3. **另外六个栅格**：LCZ_DOM、LUCY_ID、土壤颜色档、湖深、地形、树 LAI/SAI
   → `crates/colm-srfdata/src/urban_extra.rs`（250 KB）。这六处**开不到就
   `CoLM_stop`，不是警告**；其中 `urban_lai_500m/` 单个瓦片 85 MB，21 个站
   要 15 块 × 23 年 ≈ 7 GB。
4. **`LUCY_rawdata.nc`（37 KB）随包发**，`colm-cli new` 自动铺到算例的
   `runtime/urban/`。

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

还有一条给下一个人的提醒，**表外的站点仍然要用**：城市栅格要摆两份。
`<rawdata>/urban/` 与 `<runtime>/urban/` 都要有 LUCY 表 —— 前者给
`mksrfdata`，后者给 `mkinidata`，路径由两处不同的代码各拼各的。
（21 个站走内置表时，`runtime/urban/LUCY_rawdata.nc` 由 `colm-cli new`
自动铺好，不用管这条。）

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
| M8 / M9 | 已修复 | history 闸门由主 history 与 TRACER/CH4 源共同生成，共 618 个写出点；tier-check 增加层级不倒挂断言 |
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
