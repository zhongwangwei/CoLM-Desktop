<p align="center">
  <img src="gui/dist/assets/colm-icon.png" width="96" alt="CoLM Desktop 图标">
</p>

<h1 align="center">CoLM Desktop</h1>

<p align="center">
  面向 CoLM202X 站点与空间模拟的跨平台桌面工作台<br>
  <strong>中文</strong> | <a href="README.en.md">English</a>
</p>

<p align="center">
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml"><img src="https://github.com/zhongwangwei/CoLM-Desktop/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.2"><img src="https://img.shields.io/badge/version-v0.3.0--beta.2-orange" alt="当前测试版：v0.3.0-beta.2"></a>
  <img src="https://img.shields.io/badge/status-Beta-orange" alt="状态：Beta">
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="许可证：MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey" alt="平台">
</p>

CoLM Desktop 将 CoLM202X 的站点与空间建例、参数约束、三阶段运行和结果评估整合到一个图形界面中。地表数据（`mksrfdata`）、初始场（`mkinidata`）与模型主循环（`colm`）三个阶段全部由 **Rust 引擎**完成，结果与 CoLM202X 原版**逐位一致**。安装包开箱即用，无需安装 Rust、Fortran、MPI 或 NetCDF 编译环境。

> [!WARNING]
> **当前版本为 `0.3.0-beta.2` 测试版。** 功能仍在快速迭代，可能存在已知或未知缺陷。请保留原始数据与算例备份，正式科研使用前务必独立核验结果。

> [!IMPORTANT]
> 支持**站点**与**空间**两类模拟。空间模拟覆盖流域、区域与全球范围，可用经纬度网格、非结构网格与流域网格；多个站点可作为独立算例并发运行。空间模式下**参数调优和不确定性分析暂不可用**，切回站点模式后恢复。

## 下载

前往 [v0.3.0-beta.2 测试版发布页](https://github.com/zhongwangwei/CoLM-Desktop/releases/tag/v0.3.0-beta.2) 下载对应平台的安装包：

| 平台 | 架构 | 发行格式 |
|---|---|---|
| macOS | Apple Silicon | `.dmg` |
| macOS | Intel x86_64 | `.dmg` |
| Windows | x86_64 | `.msi` / `.exe` |
| Linux | x86_64 | `.AppImage` / `.deb` / `.rpm` |

安装包包含桌面端、命令行 `colm-cli`、Rust 引擎（`mksrfdata-rs`、`mkinidata-rs`、`colm-rs`）与示例站点。尚未发布的平台或开发版可以按下文从源码运行。

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
| 引导式建模 | 通过空间结构、地类、次网格和物理配置卡片确定模型约束 |
| 站点与算例管理 | 扫描站点目录、自动匹配强迫场与观测、批量创建当前会话算例 |
| 约束感知参数界面 | 统一目录搜索、常用/专家分层、IGBP/USGS/PFT/PC 作用域、稀疏覆盖与安全导入导出 |
| CSV/TXT 多站点前处理 | 自动识别逗号、制表符、分号或空白长表，按站点拆分并统一生成标准单站 NetCDF；可同步批量生成站点文件 |
| 强迫场缺测修复 | 短缺口按变量物理含义插值；长缺口换算至 UTC 后匹配 ERA5-Land 最近格点，并基于重叠观测进行偏差订正与逐时 QC 留痕 |
| Rust 模拟引擎 | 地表数据、初始场与主循环全部由 Rust 实现，覆盖 LCT / PFT / PC、BGC、作物、城市、湖泊、臭氧、示踪剂、LULCC、网格河道/水库汇流与流域侧向流 |
| 空间模拟 | 流域、区域、全球范围；经纬度网格、非结构网格与流域网格；区域算例默认开启单元流域区域汇流 |
| 并行 | 单进程共享内存多线程（patch 推进与大河系汇流按单元并行），线程数由 `RAYON_NUM_THREADS` 控制，默认取全部逻辑核 |
| 多站点并发 | 并发运行独立算例并自动平分 CPU 核；逐站点显示进度、阶段和日志 |
| 分阶段运行 | 可分别运行 `mksrfdata`、`mkinidata`、`colm`，也可一键运行全部阶段 |
| 结果分析工作台 | 七个分栏覆盖总览、变量目录、时间序列、可选变量评估、多站点排名、过程诊断及 PDF/HTML/CSV/JSON/Markdown 导出 |
| 中英文界面 | 首页与主工作流均可切换中文/English，保留常规与专家模式入口 |

当前配置体系覆盖 IGBP / USGS、LCT / PFT / PC、水热、BGC、作物、城市、河道与示踪剂过程；GUI 会根据站点/空间模式与过程约束自动隐藏不适用的参数。经纬度网格（含按 Shapefile 圈定的流域范围）配河湖汇流时，水库调度、LULCC 与示踪剂都可用；水库参数缺省读单元流域文件 `DEF_UnitCatchment_file` 自带的 GRanD 表。流域网格（`DEF_CatchmentMesh_data`）上支持水库调度，LULCC、示踪剂与向量历史尚未覆盖；这类组合会在运行前明确提示，不会等前处理跑完才失败。
参数目录与操作说明见 [`docs/parameter-catalog.md`](docs/parameter-catalog.md)、[`docs/parameter-gui-guide.md`](docs/parameter-gui-guide.md) 和 [`docs/parameter-default-preservation.md`](docs/parameter-default-preservation.md)。上游代码中发现并已在本仓库修复的缺陷记录在 [`docs/upstream-bugs.md`](docs/upstream-bugs.md)。

## 使用流程

1. **选择模拟类型**：在启动卡片中选择站点、地类、次网格和物理过程。
2. **准备自己的数据（可选）**：可选择单站 NetCDF，或包含单站/多站的 CSV、TXT、TSV；表格按站点拆分并归一到 UTC 后逐站诊断。短缺口直接插值，长缺口可下载或复用 ERA5-Land 缓存，经重叠期订正后生成不覆盖原始数据的标准文件。使用内置示例时可跳过。
3. **设置文件与目录**：选择准备好的 `Sitedata` 与 `Forcing` 目录，指定算例根目录。
4. **创建算例**：扫描站点并勾选一个或多个站点，自动匹配强迫场与观测文件。
5. **检查基本设定**：配置预热、地表数据、初始场、强迫场和并行选项。
6. **配置过程参数**：只处理当前模型约束下实际生效的过程。
7. **选择输出并运行**：按阶段或全部运行，查看每个站点的实时进度和日志。
8. **分析结果**：浏览实际 history 变量与维度，按站点绘图、缩放和导出完整 CSV。
9. **评估与诊断**：从净辐射、能量通量、摩擦速度、GPP、生态系统呼吸和 NEE 等可用观测中勾选评估内容，计算 RMSE、MAE、Bias、R²、Pearson r、NSE、KGE 及其分量，比较多个站点并导出报告或 PDF。

结果工作台只纳入本次任务创建的算例，不会混入算例根目录中的旧结果。长序列和模型—观测配对图按需保极值降采样；指标仍使用完整样本。多站点评估采用有上限的并发池，单个站点缺观测或失败不会中断其余站点。

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

选择 `Sitedata` 后，界面会自动更新强迫场目录和可用性；也可以在基本设定中显式选择其他目录。

## 从源码运行

### 环境要求

- Rust **1.85.1** 或更新版本
- Git
- 当前平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)
- 运行算例所需的内核目录（安装包已自带；从源码运行时的生成方法见文末注）

### 启动桌面端

```bash
git clone https://github.com/zhongwangwei/CoLM-Desktop.git
cd CoLM-Desktop
cargo run --manifest-path gui/src-tauri/Cargo.toml
```

启动日志会报告 WebView 是否到达后端、解析到的 `colm-cli` 路径以及可用内核数量。程序默认直接进入卡片选择首页。

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
Static HTML/CSS/JS GUI
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
| `gui/src-tauri` | Tauri 窗口、文件选择、IPC、批量任务与事件转发 |
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

# 静态 GUI 接口与行为测试
cargo run -q -p xtask -- check-gui
for test in gui/tests/*.mjs; do node "$test"; done

# Tauri 后端
cargo test --manifest-path gui/src-tauri/Cargo.toml
cargo clippy --manifest-path gui/src-tauri/Cargo.toml --all-targets -- -D warnings
```

CI 在 macOS、Windows 和 Linux 上编译并测试整个 Rust 工作区（含 Rust 引擎的物理、主循环、初始场与地表数据各 crate）、GUI 接口、格式与静态分析，并用入库的黄金文件检查数值容差分类与 NetCDF 读写。每一轮与原版的逐位对照（站点、经纬度、非结构与流域网格算例）记录在验证文档中。

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
cargo run -p xtask -- stage-sidecar
cd gui/src-tauri
cargo tauri build --config tauri.bundle.conf.json
```

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
