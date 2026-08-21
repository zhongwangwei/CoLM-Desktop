# CoLM-Desktop

## 下载

[下载最新编译版（macOS / Windows / Linux）](https://github.com/zhongwangwei/CoLM-Desktop/releases/latest)

安装包已经包含 CoLM 内核和示例站点，使用桌面程序无需安装 Rust、Fortran 或 NetCDF 编译环境。

把 CoLM202X 的 SinglePoint 模式做成跨平台桌面程序。设计见 `docs/design.md`。

**当前状态**：命令行端到端可用 —— 一条命令从原始 PLUMBER2 站点文件跑到
指标表。PFT / PC / BGC / URBAN 已是运行时开关；IGBP / USGS 分别由两份编译产物覆盖。GUI 已按
GUI 能扫站点、按功能分类改参数、
批量运行、自动配对观测并出评估图。安装包由 `release.yml` 三平台产出，
内核随包走 —— **用桌面程序的人不需要装任何编译器**。

### GUI 能做什么

**进门先分流，然后走五步。** 启动时先问「这次要跑什么」——「站点」「区域」
「全球」三档。三种域要的前处理、地表数据与并行设置都不一样，将来各自展开
自己的步骤链；**现在只有「站点」能点，另外两档是灰的（「暂不支持」）**，
而不是点了报错——一个能点但必然失败的入口比一个灰着的更糟。这道门每次
启动都弹，它是分流点而不是一次性欢迎页。

选了「站点」之后是五步，**顺序由依赖链定，不是按界面好看排的**：

| | 步骤 | 要什么才能进 |
|---|---|---|
| ① | 前处理 | ——（这一页目前是预留的，把打算做的事写出来） |
| ② | 基本设定 | 扫站点、建算例；文件、站点、时间、网格、地表、初始场与强迫场分栏 |
| ③ | 过程参数 | 要先建过算例；只显示当前模型涉及的过程 |
| ④ | 运行 | 同上 |
| ⑤ | 结果 | 同上 |

物理和次网格已在进门向导选完。GUI 后台按选择自动匹配 IGBP 或 USGS 产物，
主界面不再给用户一个重复的“选内核”下拉框。

| | |
|---|---|
| 站点库 | 扫 `Sitedata` 目录，两套命名约定都认；列出「城市 / 无观测 / 读不了」 |
| 参数 | 按用途分节；向导已定义的字段不重复显示，当前配置不可用的也默认隐藏 |
| 输出变量 | 482 个开关独立成页，逐条说明「勾了到底写不写得出来」 |
| 运行 | 三段各自状态；**输入没变就跳过**（指纹，不是只看文件在不在）；批量跑并逐算例显示状态 |
| 评估 | 指标表（含 KGE 不可信的警告）、模型 vs 观测双线图、散点图、批量汇总表 |
| 预设 | 存下其它参数与输出设置跨算例复用；**向导字段与身份字段挡在外面** |

## 仓库与依赖

`vendor/CoLM202X` 是入库的源码快照；来源、基线 commit 与本地改动记录在
`vendor/PROVENANCE.md`，普通克隆不需要再初始化 submodule。

CI 分两层。每个 PR 在 ubuntu / macOS / Windows 三平台跑 **182/204** 条测试 ——
`cargo test --workspace --lib --bins` 的纯计算部分（161 条），加上五条各自点名的
集成测试：判官、namelist 往返、schema 漂移、输出变量闸门表的经验校验与其漂移。
这些只需要源码与已入库的黄金文件。其余 22 条要 5.5 GB 的 PLUMBER2 与 38 GB 的
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

城市站点由站点文件的形状自动认出（没有 `IGBP_classification` 就是城市），
没有 `--urban` 开关；那时两个栅格目录必填：

```bash
colm-cli new --site <Urban-PLUMBER>/Sitedata/AU-Preston_site_v1.nc \
             --out  ~/cases/AU-Preston \
             --rawdata ~/rawdata --runtime ~/runtime \
             --start 1993-01-01 --end 1993-01-11
colm-cli run ~/cases/AU-Preston --kernel kernels/default
```

`colm-cli` 是**唯一的编排可执行文件**（`design.md` §4.2：「GUI 只跟它说话」），
所以它是唯一一处同时依赖全部五层的地方；各层之间仍然互不依赖。四个子命令：
`new` 造算例、`run` 跑三段、`metrics` 出指标表、`all` 串起来。

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
| 1. 编译期宏 | `MOD_Hist.F90` 里的 `#ifdef` / `#ifndef` | 456 个写出点 → **123** |
| 2. 运行时 `DEF_*` 条件 | 同一文件里的内联 `.and.` 与外层 `IF (DEF_*) THEN` | 123 里 10 个带条件，本次 6 真 4 假 → **119** |
| 3. 变量自己的开关 | `DEF_hist_vars%X`，在 `colm-schema` 里 | 默认全开 |

`crates/colm-hist` 只回答闸门 1，输入是内核清单里的 `macros`：

```rust
// 清单里的 macros 是 Vec<String>（它要能从 JSON 反序列化），闸门表要 &str
let macros = manifest.macros.iter().map(String::as_str).collect();
colm_hist::writable(&macros)   // -> BTreeSet<&'static str>，default 下 123 个
```

闸门 2 的条件**原样记下来**而不求值 —— 求值需要一份具体的算例配置，那是调用方
的事；这一层的职责是如实报出 CoLM 写了什么条件，好让 GUI 说清「为什么你勾了它
却没有」。闸门 3 已经在 `colm-schema` 里，两张表在 GUI 层合并即可。

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

**大多数时候不用。** 物理预设是**编译期**的东西 —— 一个内核目录就是一组
CPP 宏，装出来的程序自带三个：

| 预设 | 宏组合 |
|---|---|
| `default` | `SinglePoint LULC_IGBP URBANOFF vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF` |
| `bgc` | `SinglePoint LULC_IGBP_PFT URBANOFF vanGenu CaMaOFF BGCON CROPOFF TRACEROFF` |
| `urban` | `SinglePoint LULC_IGBP URBANON vanGenu CaMaOFF BGCOFF CROPOFF TRACEROFF` |

要自己编，只有三种情况：**想要第四种宏组合**（开 CROP、开 TRACER、
换 Campbell 土壤、开 CaMa-Flood…）、**改了 CoLM 的 Fortran 源码**、
或者**要一个没发布的平台/架构**。

桌面端不暴露这些目录；它按向导的 IGBP/USGS 选择匹配 `generator_args`。

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
（改上游一行就能去掉，但那会让 submodule 离开一个干净的上游 commit，
不值得；装一个只提供头文件的包更便宜。）

### Windows 上二进制叫 `.exe`，不叫 `.x`

CoLM 的 Makefile 在所有平台都产出 `.x`；`build_kernel.sh` 在 Windows 上把
**拷进内核目录的那份**改名（不碰 `run/` 里 Makefile 的产物，submodule 因此
保持在一个干净的上游 commit 上）。

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

| 配置 | 构建 | 运行 | 备注 |
|---|---|---|---|
| `default` | ✅ 38 s | ✅ 黄金基准 | —— |
| `PC` | 共用 IGBP | ✅ 三阶段 + 96 步 | 运行时 `DEF_USE_PC` |
| `USGS` | ✅ 独立产物 | ✅ 三阶段 + 96 步 | 编译期地类数组不同 |
| `bgc` | ✅ 44 s | ✅ 三段跑通 | 需要两份 runtime 数据，见下 |
| `urban` | ✅ 38 s | ✅ 三段跑通，**不用给 rawdata/runtime** | 只对 Urban-PLUMBER 那 21 个站，见下 |

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
   根本不可达。修在 `vendor/CoLM202X` 的 `fix/urban-site-fallbacks` 分支
   （`ad77af53`），**尚未 push 到上游**：
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
**完全不给 `--rawdata` / `--runtime`**：三段全 `ok`，264 条小时记录，
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
