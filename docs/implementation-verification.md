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
