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
  给 0 会让开启喷灌的算例静默变成不灌溉）。
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
