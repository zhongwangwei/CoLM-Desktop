<p align="center">
  <img src="gui/public/assets/colm-icon.png" width="96" alt="CoLM Desktop 图标">
</p>

<h1 align="center">CoLM Desktop</h1>

<p align="center">
  面向 CoLM202X 站点与空间模拟的跨平台桌面工作台<br>
  <strong>中文</strong> | <a href="README.en.md">English</a>
</p>

<p align="center">
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml"><img src="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.3"><img src="https://img.shields.io/badge/version-v0.3.0--beta.3-orange" alt="当前测试版：v0.3.0-beta.3"></a>
  <img src="https://img.shields.io/badge/status-Beta-orange" alt="状态：Beta">
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="许可证：MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="平台">
</p>

CoLM Desktop 将 CoLM202X 的站点与空间建例、参数约束、三阶段运行和结果评估整合到一个图形界面中。地表数据（`mksrfdata`）、初始场（`mkinidata`）与模型主循环（`colm`）三个阶段全部由 **Rust 引擎**完成，结果与 CoLM202X 原版**逐位一致**。安装包开箱即用，无需安装 Rust、Fortran、MPI 或 NetCDF 编译环境。

> [!WARNING]
> **当前版本为 `0.3.0-beta.3` 测试版。** 功能仍在快速迭代，可能存在已知或未知缺陷。请保留原始数据与算例备份，正式科研使用前务必独立核验结果。

> [!IMPORTANT]
> 支持**站点**与**空间**两类模拟。空间模拟覆盖流域、区域与全球范围，可用经纬度网格、非结构网格与流域网格；多个站点可作为独立算例并发运行。空间模式下**参数调优和不确定性分析暂不可用**，切回站点模式后恢复。

## 下载

前往 [v0.3.0-beta.3 测试版发布页](https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.3) 下载对应平台的安装包：

| 平台 | 架构 | 发行格式 |
|---|---|---|
| macOS | Apple Silicon | `.dmg` |
| macOS | Intel x86_64 | `.dmg` |
| Windows | x86_64 | `.msi` / `.exe` |
| Linux | x86_64 | `.AppImage` / `.deb` / `.rpm` |

安装包包含桌面端、命令行 `colm-cli`、Rust 引擎（`mksrfdata-rs`、`mkinidata-rs`、`colm-rs`）与示例站点。尚未发布的平台或开发版可以按下文从源码运行。

## 终端助手

终端无需打开桌面窗口：`colm-agent chat --project-root /你的项目目录` 可直接对话，支持流式回答、写操作审批和历史会话恢复。模型服务地址、模型、API 格式和思考程度可通过 `colm-agent config` 设置；也可选择现有 Codex、Claude Code、OpenCode 后端。

pip 分发暂时停用，不再生成或发布 wheel。桌面端与原生 CLI 继续可用，详见 [终端助手使用说明](docs/terminal-assistant.md)。

## 0.3.0-beta.3 的主要变化（发布记录）

本节保留已发布版本的界面说明；当前源码的入口见下文「使用流程」。

- **河湖汇流可选**：首页"还要打开哪些过程？"里与 BGC 并列的"河湖汇流"卡片决定是否计算河道（新 namelist 开关 `DEF_USE_GridRiverLakeFlow`）。不选时不建河网、不汇流，也不出现"河道与水库"页；陆面结果不变。卡片会按流域边界、区域或非结构 mesh 提醒它们与河网的单元流域可能不一致。
- **区域河网不再拉进无关河系**：只在打开分汊时才沿分汊通道合并河系；河道 unitcat 输出截到模拟范围，与陆面 history 逐格对齐。新建经纬度网格算例的历史输出分辨率跟模拟网格一致。
- **水库调度**：流域网格支持水库调度；经纬度网格的水库参数缺省读单元流域文件自带的 GRanD 表。
- **打开已有算例**：首页与"文件与目录"页可直接打开磁盘上的算例，自动恢复它的网格、次网格、各过程开关、内核与建例时的输入。
- **运行与日志**：Rust 引擎运行时进度条逐步前进；日志按阶段分段、写出文件汇总成一行；空间算例按引擎只显示线程数或 MPI 进程数；开跑前不再逐个检查 RawData 里的几十万个文件。
- **界面精简**：前处理入口隐藏；分块与 MPI IO 分组、区域单元流域等不需要用户决定的选项不再显示。
- 两个引擎的结果仍然逐位一致。

## 0.3.0-beta.2 的主要变化

- **大规模空间算例更快**：history 累加改为按 patch 并行，续跑快照只在写续跑时做；流域河湖汇流的四倍精度除法提速；装配阶段去掉重复的路径解析与 PFT 参数查表；植物水力、光合与变饱和土壤水里复用同参数的 `pow`。珠江流域（3.6 万 patch）3 天算例从约 9–10 分钟降到约 6.5 分钟。
- **所有改动都不改变计算结果**：流域、站点与经纬度网格算例与上一版及 Fortran 原版逐位一致。
- 设 `COLM_RS_TIMING=1` 运行 `colm-rs`，结束时打印主循环各段耗时。

## 0.3.0-beta.1 的主要变化

- **流域网格接入 Rust 引擎**：集水区与 HRU 水文单元上的坡面流、河湖汇流（四倍精度求和）、地下侧向流与动态湖全部移植，输出流域历史、流域重启与 `catch_parameters.nc`，与原版逐位一致。
- **全部网格类型默认使用 Rust 引擎**：站点、经纬度网格、非结构网格、流域网格。
- **大规模空间算例的装配与续跑写出**改为按 patch 局部存取：上万个 patch 的分块装配从数十分钟降到数十秒，内存不再随 patch 数平方增长。
- **共享内存并行**：patch 推进与大河系汇流按单元多线程；同时运行多个算例时自动平分 CPU 核。

## 核心能力

| 能力 | 说明 |
|---|---|
| 模型版本与算例 | 版本管理代码、参数、物理配置和网络；算例管理地点、输入、时段与输出，运行使用独立副本 |
| 站点与算例管理 | 扫描站点目录与匹配输入，打开已有算例，或从内置示例创建独立算例 |
| 参数覆盖 | 在版本库搜索目录，记录 IGBP/USGS/PFT/PC 与过程文件作用域，保存稀疏覆盖并导入导出 |
| CSV/TXT 多站点前处理 | 自动识别逗号、制表符、分号或空白长表，按站点拆分并统一生成标准单站 NetCDF；可同步批量生成站点文件 |
| 强迫场缺测修复 | 短缺口按变量物理含义插值；长缺口换算至 UTC 后匹配 ERA5-Land 最近格点，并基于重叠观测进行偏差订正与逐时 QC 留痕 |
| Rust 模拟引擎 | 地表数据、初始场与主循环全部由 Rust 实现，覆盖 LCT / PFT / PC、BGC、作物、城市、湖泊、臭氧、示踪剂、LULCC、网格河道/水库汇流与流域侧向流 |
| 空间模拟 | 流域、区域、全球范围；经纬度网格、非结构网格与流域网格；区域算例默认开启单元流域区域汇流 |
| 并行 | 单进程共享内存多线程（patch 推进与大河系汇流按单元并行），线程数由 `RAYON_NUM_THREADS` 控制，默认取全部逻辑核 |
| 多站点并发 | 并发运行独立算例并自动平分 CPU 核；逐站点显示进度、阶段和日志 |
| 分阶段运行 | 可分别运行 `mksrfdata`、`mkinidata`、`colm`，也可一键运行全部阶段 |
| 结果分析工作台 | 数据目录、时序、观测评估、多站比较、闭合诊断与报告；按真实运行出处读取结果 |
| 中英文界面 | 顶框切换中文/English 和 AI 助手，设置中调整主题与专家选项 |

当前配置体系覆盖 IGBP / USGS、LCT / PFT / PC、水热、BGC、作物、城市、河道与示踪剂过程；版本中的覆盖记录保留地类、PFT 与过程作用域，物化和运行前检查它们与当前算例及内核是否兼容。经纬度网格（含按 Shapefile 圈定的流域范围）配河湖汇流时，水库调度、LULCC 与示踪剂都可用；水库参数缺省读单元流域文件 `DEF_UnitCatchment_file` 自带的 GRanD 表。流域网格（`DEF_CatchmentMesh_data`）上支持水库调度，LULCC、示踪剂与向量历史尚未覆盖；这类组合会在运行前明确提示，不会等前处理跑完才失败。
参数目录与操作说明见 [`docs/parameter-catalog.md`](docs/parameter-catalog.md)、[`docs/parameter-gui-guide.md`](docs/parameter-gui-guide.md) 和 [`docs/parameter-default-preservation.md`](docs/parameter-default-preservation.md)。上游代码中发现并已在本仓库修复的缺陷记录在 [`docs/upstream-bugs.md`](docs/upstream-bugs.md)。

## 使用流程

以下说明对应当前 React 源码界面。打开后直接进入项目首页，先明确选择模型版本；未选择时，下方新建、打开、结果、示例和最近项目入口灰色不可选。左栏依次为项目、模拟、结果、研究、历史记录，底部为计算资源和设置。版本选择在独立浅色区，下方「再选择操作」提供新建模拟、打开项目、直接查看结果，另有 CN-Cng 内置示例和最近项目；顶框放 AI 助手与语言按钮，没有额外启动页或顶部流程栏。

1. **先选版本，再打开或试一个算例**：在上方独立版本区点击官方版，或在已有版本弹窗中选择准确修订，才会解锁下方操作。选择在本次软件会话内保留，返回首页无需重选；重启重新选择。「＋ 新建修改版」可先进入管理，选择或冻结修订后再开始任务。在「项目 → 打开项目」选择包含算例的目录，或点「新建模拟 → 自带 4 个示例」，选站点、名称、输出目录与时段，再创建。空间算例在新建页选择区域、流域或全球范围及输入网格。
2. **准备自己的数据（可选）**：「项目 → 准备输入数据」支持单站 NetCDF 或单站/多站 CSV、TXT、TSV。短缺口插值、长缺口 ERA5-Land 订正及 UTC 归一沿用已有流程；生成标准文件，不覆盖原始数据。
3. **修改模型（需要时）**：点「项目 → ＋ 新建修改版」，从已冻结修订派生草稿。参数、物理方案、AI 网络和代码分四块管理。保存草稿可继续编辑；冻结生成不可变修订，后续修改形成新修订。站点、地类及率定/验证时段在适用范围中记录。进入研究时，当前草稿先保存并冻结。
4. **配置模拟**：按「范围与网格 → 模型结构与方案 → 数据与时段 → 预热与输出 → 检查与运行」五步完成。新建页用卡片选择单点、流域、区域、全球及空间网格；开始与结束日期同行，输入 `YYYY-MM-DD`（如 `2001-01-01`）。第二步先选择 IGBP/USGS 分类，再查看模型结构、参数与物理方案，点击「调整参数与方案」进入版本管理，选择或冻结新修订后返回，保留待建配置。河湖汇流、BGC、TRACER 的后续选项按条件展开，真实过程文件与 PFT/PC 索引参数有独立入口；条件不明时保留待核对状态。第三步按过程确定数据与时段，第四步设置步长、预热截止、输出与重启，更多配置保留初始条件、强迫处理、诊断和实际内核可写的输出变量。冻结模型不能原地修改；已有算例的日内秒数保留，离页前保存或放弃本地修改。
5. **检查资源并运行**：「计算资源」顶部卡片选择本机或已登记服务器，下面编辑所选连接的地址、账号和 SSH 配置/密钥/密码；数据目录与高级维护按需展开。点击检查连接，分别查看此次连接、可用引擎快照和对应路径的真实状态；修改连接设置后须重新检查。该页选择只控制资源详情，计算位置仍在「模拟 → 检查并运行」选择。每次运行物化「模型修订 × 算例数据」的独立副本，记录实际出处与内容身份。
6. **分析真实结果**：左栏「结果」自动读取已记录运行的模拟曲线；「比较其他运行」弹窗切换记录，「评估设置」弹窗选择观测并更新指标。总览依次展示实际出处、评估摘要、曲线与偏差/RMSE/有效配对数、下一步；目录、时序、观测评估、多站比较、闭合诊断和报告从页尾进入。图形可降采样，指标使用完整数据；来源版本有同输入、时段和评估条件的真实结果才进行来源对比，缺少基准不会自动追加计算。
7. **开展研究并保存成果**：左栏「研究」先展示参数率定、不确定性、AI 参数学习、AI 过程替换和修改源码五张方法卡，再进入对应工作台。核对站点、观测、范围、训练/验证时段和预算，先预检再创建/运行；完成后将选定成员或训练网络存为模型版本，再选择该版本运行与比较。网络仅支持 Rust，引擎或编译期开关的变化仍需适用程序与内核。
8. **追溯操作**：「历史记录」以记录表和详情呈现操作，筛选运行、研究、版本、资源及助手操作，查看对象、状态、用时、来源和文件引用，导出单条或完整记录。中断或提交未确认的任务先核对真实进程/服务器作业，不自动重跑。

结果页支持完整时序 CSV，以及 JSON、CSV、Markdown、HTML 报告；PDF 使用系统打印窗口，选择“另存为 PDF”。下载或打印请求不代表文件已经保存。

点窗口顶部「AI 助手」展开右侧面板，拖动面板左边缘可调整宽度，在「设置」选择 API 服务或本机 Codex、Claude Code、OpenCode。API 可登记 Key、发现或填写模型，配置服务地址与模型支持的思考程度；高级协议及预算选项在专家设置中。重启默认新对话，可恢复会话历史，输入框用 ↑/↓ 选择历史输入；底部仅保留模型与思考程度，服务地址和后端放在设置中；示例建议先填入消息，发送后才执行。助手的数据分析、率定与 AI 任务使用对应的可见页面，并按操作审批规则执行。

### 推荐的数据目录

站点、强迫场和观测采用同级目录时，GUI 可以按站点名称自动匹配：

```text
data-root/
├── Sitedata/
│   └── <site>_site.nc
├── Forcing/
│   └── <site>_Met.nc
└── Observation/
    └── <site>_Flux.nc
```

在新建算例页填写 `Sitedata` 与 `Forcing` 目录后扫描站点；选中站点会带入匹配的强迫场与覆盖时段。已有算例的数据路径在「算例 → 设置」修改。

## 从源码运行

### 环境要求

- Rust **1.85.1** 或更新版本
- Git
- Node.js **24**（与 CI 一致）及 npm
- Tauri 2 CLI（`cargo tauri`）
- 当前平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)
- 运行算例所需的内核目录（安装包已自带；从源码运行时的生成方法见文末注）

### 启动桌面端

```bash
git clone https://github.com/zhongwangwei/CoLM-Desktop.git
cd CoLM-Desktop
npm --prefix gui ci
npm --prefix gui run build
cargo run -p xtask -- stage-sidecar
cd gui
cargo tauri dev
```

`cargo tauri dev` 启动 Vite 开发服务和原生窗口；sidecar 暂存步骤编译配套 CLI、引擎与助手，并绑定源码/程序摘要。修改 Rust 源码后重新暂存，避免把旧程序当成新源码的执行结果。只打开浏览器预览没有原生模型和助手后端。

要使用构建后的前端而不运行 Vite，在 `gui/` 执行 `cargo tauri build --no-bundle`，再运行构建输出中的原生程序（默认 `src-tauri/target/release/colm-desktop-gui`，Windows 加 `.exe`）。此命令按配置重建前端；构建目录可随 Cargo 配置变化。

启动日志报告 WebView 与后端连接、解析到的 `colm-cli` 路径和可用内核。启动后的模型、算例和助手入口与上面的工作台流程一致。

### 命令行工作流

GUI 通过同一个 `colm-cli` 编排算例；命令行也可以独立使用：

```bash
# 扫描站点并检查强迫场匹配
cargo run -p colm-cli -- scan \
  --dir /path/to/Sitedata \
  --forcing-dir /path/to/Forcing

# 探测一份单站或多站 CSV/TXT 长表
cargo run -p colm-cli -- forcing-table-probe /path/to/sites.csv --json 1

# 创建算例
cargo run -p colm-cli -- new \
  --site /path/to/Sitedata/site.nc \
  --out /path/to/cases/site-name

# 运行全部阶段（Rust 引擎）
cargo run -p colm-cli -- run /path/to/cases/site-name \
  --kernel kernels/default

# 与观测计算指标
cargo run -p colm-cli -- metrics /path/to/cases/site-name \
  --obs /path/to/Observation/site_Flux.nc \
  --pairs-var Rnet --pairs-var GPP --pairs-var NEE

# 检查当前算例和观测共同支持哪些评估变量
cargo run -p colm-cli -- evaluation-catalog /path/to/cases/site-name \
  --obs /path/to/Observation/site_Flux.nc

# 浏览 history 目录并导出保极值降采样序列
cargo run -p colm-cli -- history-catalog /path/to/cases/site-name
cargo run -p colm-cli -- series /path/to/cases/site-name \
  --vars f_rnet --max-points 2400
```

`--kernel` 指向的内核目录提供编译期配置（站点/经纬度/非结构/流域网格，IGBP/USGS，作物）。运行 `cargo run -p colm-cli --` 可查看完整命令和参数。

## 架构

```text
React + TypeScript GUI (Vite)
          │ Tauri IPC
          ▼
Rust window backend ──► colm-cli sidecar ──► mksrfdata-rs / mkinidata-rs / colm-rs
          │                    │
          ├── schema           ├── forcing / surface data
          ├── namelist         ├── stage orchestration
          └── history gates    └── metrics / time series
```

| 模块 | 职责 |
|---|---|
| `gui/src` | React 页面、模型/算例上下文、图表与助手页面契约 |
| `gui/src-tauri` | Tauri 窗口、文件选择、IPC、批量任务与事件转发 |
| `crates/colm-model` | 模型草稿、不可变修订、稀疏覆盖和运行副本 |
| `crates/colm-history` | SQLite 操作台账、旧记录导入与证据引用 |
| `crates/colm-cli` | GUI 与命令行共用的唯一编排入口 |
| `crates/colm-case` | 算例目录与 namelist 生成 |
| `crates/colm-namelist` | 保留格式的 CoLM namelist 读写 |
| `crates/colm-schema` | 从 CoLM 源码生成的配置字段与默认值 |
| `crates/colm-kernel` | 内核身份校验、三阶段执行、成功判定与覆盖消息 |
| `crates/colm-core` | CoLM 物理过程（能量、水文、雪、冠层、BGC、作物、城市、湖泊、四倍精度算术等） |
| `crates/colm-runtime` | 主循环 `colm-rs`：站点与空间驱动、强迫读取、河道/水库汇流、流域侧向流、历史与重启 |
| `crates/colm-init` | 初始场 `mkinidata-rs`（含流域网络与冷启动） |
| `crates/colm-srfdata` | 地表数据 `mksrfdata-rs`（站点与空间聚合）、单点地表数据补全与来源记录 |
| `crates/colm-lapack` | 城市辐射矩阵求逆，按平台复刻原版所链 LAPACK 的结果 |
| `crates/colm-forcing` | 强迫场探测、缺测修复、ERA5-Land 订正、校验与配置生成 |
| `crates/colm-hist` | 输出变量闸门、时间序列与评估指标 |
| `oracle` | 黄金结果回归与数值一致性验证 |

窗口进程不直接链接 NetCDF/HDF5；需要读取 NetCDF 的操作通过 `colm-cli` sidecar 完成，从而保持桌面进程轻量并隔离原生 I/O 依赖。

## 开发与验证

```bash
# 引擎测试
cargo test --workspace --lib --bins
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# React 类型、接口与行为检查
npm --prefix gui ci
npm --prefix gui run lint
npm --prefix gui test
npm --prefix gui run build
cargo run -q -p xtask -- check-gui

# Tauri 后端
cargo test --manifest-path gui/src-tauri/Cargo.toml
cargo clippy --manifest-path gui/src-tauri/Cargo.toml --all-targets -- -D warnings
```

CI 在 macOS、Windows 和 Linux 上编译并测试整个 Rust 工作区（含 Rust 引擎的物理、主循环、初始场与地表数据各 crate）、GUI 接口、格式与静态分析，并用入库的黄金文件检查数值容差分类与 NetCDF 读写。每一轮与原版的逐位对照（站点、经纬度、非结构与流域网格算例）记录在验证文档中。

真实多站 GUI 验收使用 `scripts/gui-multisite-e2e.py`：先按上文暂存 sidecar，编译带 `e2e-driver,tauri/custom-protocol` 的原生程序，在独立测试环境设置 `COLM_E2E_PORT=18899` 后启动，再运行：

```bash
python3 scripts/gui-multisite-e2e.py --data-root examples --runtime examples/Runtime \
  --project-root /absolute/new/test-projects --name multisite \
  --site CN-Cng,2008-01-01,2009-01-01 --site AT-Neu,2010-01-01,2011-01-01 \
  --report-dir /absolute/new/gui-evidence --timeout 1800
```

测试通过真实窗口创建并运行算例，核对两个站点的三阶段执行、实际并发、导航恢复及各自结果曲线；报告保留事件、IPC 来源和界面状态。Linux 无显示器时可在 Xvfb 中运行；仅在测试显示器设置 `COLM_E2E_TEST_DISPLAY=1` 时捕获截图，截图还需人工核验。`gui-linux-private-deps.py` 和 `prepare-private-cargo-vendor.py` 可为隔离计算任务准备私有依赖，不安装系统软件或改写共享目录；输入与输出约定见各脚本说明。

## 文档

- [设计与总体架构](docs/design.md)
- [GUI 入口与约束设计](docs/design-gate.md)
- [GUI 工作流设计](docs/design-gui3.md)
- [前处理设计](docs/design-prep.md)
- [强迫场缺测修复设计与验收矩阵](docs/plan-forcing-gap-repair.md)
- [CSV/TXT 多站点前处理契约与验收矩阵](docs/plan-tabular-multisite-prep.md)
- [结果分析工作台设计](docs/plan-results-workbench.md)
- [实现、缺陷复盘与验证记录](docs/implementation-verification.md)
- [上游 CoLM202X 缺陷与本地修复](docs/upstream-bugs.md)
- [CPU 并行约定](docs/cpu-parallelism.md)

## 发布

`main` 上出现尚未发布的新版本号（或推送 `v*` 标签）时，`.github/workflows/release.yml` 自动为 macOS（Apple Silicon / Intel）、Windows 和 Linux 构建安装包，检查 sidecar 与示例数据随包分发后发布。本地打包：

```bash
npm --prefix gui ci
cd gui
cargo tauri build --config src-tauri/tauri.bundle.conf.json
```

打包配置会重建 React 前端和配套 sidecar，并携带对应源码版本的文档；发布记录不代替当前源码的完整构建和远程验收。

## 开发与维护

- **开发与维护**：魏忠旺 @ CoLM陆面模式开发团队
- **单位**：中山大学大气科学学院
- **邮箱**：[weizhw6@mail.sysu.edu.cn](mailto:weizhw6@mail.sysu.edu.cn)
- **项目主页**：<https://github.com/zhongwangwei/CoLM-Desktop>

**版权所有：CoLM陆面模式开发团队，中山大学大气科学学院。**

## 许可证

Rust 与桌面端代码按 `MIT OR Apache-2.0` 双许可证发布。第三方依赖与 CoLM202X 上游源码的许可请参阅各自文件。

---

> 注：安装包同时内置 CoLM202X 原版 Fortran 内核，可在运行页的"模拟引擎"中或用 `colm-cli run … --engine fortran` 切换使用；默认使用 Rust 引擎。从源码运行时，`kernels/` 下的内核目录由 `./oracle/scripts/build_kernel.sh <preset>` 生成（需要 gfortran、NetCDF-Fortran、LAPACK 与 MPI）。
