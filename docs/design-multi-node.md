# CoLM Desktop Rust 引擎多节点（SPMD）设计

> 状态：Phase 0 方案稿（2026-10-24）。用户已定四件事：目标是 **0.1° 级全球**空间算例；**只做远程 Linux**，桌面本机不开放多进程；跨 rank **逐位一致是硬门槛**；按本文阶段实施。
> 实测结论照例追加到 `docs/implementation-verification.md`；本文只写设计 + Phase 0 的只读实测。文中数字凡标“估算”的，都要在 Phase 0 测量后回来改。

## 0. 一句话方案

N 个远程进程（rank）各领一部分 `landpatch` 块，在进程内用 rayon 推进自己 patch 的物理；每步把 patch 级数组按 rank 顺序 all-gather 成全局数组（连续分片保证拼接即全局顺序），**每个 rank 用与今天完全相同的代码跑同一份全局河湖汇流**（复制河网方案）；每个 rank 写自己块的重启与旁车，root 写全局 history、河道重启与 `.river` 历史文件。通信走纯 Rust TCP，**不引 MPI**。0.1° 全球的账（见 §2）使这个形状足够：每步通信约 0.3 GB，估算占比 <1%；逐位一致由“各 rank 用同一份全局输入、同一段代码”保证。

## 1. 目标与非目标

**目标**

1. 单个空间算例跨多节点运行，首个验收目标是 0.1° 全球、fastPC 子网格。
2. 同一 block 布局下，1 rank 与 N rank 的**全部输出逐字节相同**（history、重启、旁车、河道重启）。
3. N=1 时完全不初始化传输层，行为与今天逐字节相同。
4. 产品边界不变：安装包仍不带 MPI，macOS/Windows 本机构建与 CI 不变；多节点只在远程 Linux 上出现。

**非目标（首版明确拒绝，报错要写清原因）**

- 本机/桌面 `--ranks > 1`（仍只允许 1）。
- 流域网格的侧向流（`CatchLateralFlow`，全域网格耦合）与 baseflow 优化器（现状只支持单块）。
- 多块 + `pctshared`（现状单进程也不支持，见 `crates/colm-runtime/src/spatial/topology.rs:127-131`）。
- 河系内部再拆分（跨 rank 的邻接 halo + 每子步 MIN dt）。0.1° 全球不需要（§2.3）。
- 预处理阶段的多进程（另立任务，见 §10；首版只要求主循环多节点）。

## 2. 0.1° 全球的规模账

### 2.1 已有实测锚点

| 量 | 数值 | 出处 |
|---|---|---|
| 每 patch 状态内存 | 约 120 KB/patch（含两份状态、模板与每步输出） | `docs/implementation-verification.md:30708-30730` |
| history 缓冲 | 约 1.75 KB/patch/记录 | 同上 |
| 全球 15′ 单元流域网络 | 252,383 个单元、23,127 个河系，**与网格分辨率无关** | `docs/implementation-verification.md:33436-33460` |
| 该网络汇流耗时 | 128 线程 4.02 s/天，240 线程 3.93 s/天（修复嵌套并行后） | 同上 |
| 黑河案例（610 patch） | 主循环 145.3 s，其中汇流 101.85 s、patch 物理 22.89 s | `docs/implementation-verification.md:33474` |
| patch 物理单价（tianhe 实测） | LCT IGBP、无河湖/无 history：**0.53 ms/patch/步（1 线程）**；64 线程墙钟折合 0.014 ms/patch/步（并行效率 57%，1,408 patch × 144 步） | 第 785 轮 |
| 每 patch 状态内存（无 history） | **约 81 KB/patch**（1,408 与 22 patch 两次运行的峰值 RSS 差） | 第 785 轮 |
| 节点间 TCP | 8 流 iperf3 聚合 **40.1 Gb/s**、单流约 5.0 Gb/s；ping 平均 0.032 ms（tianhe gn0 以太网，两节点） | 第 785 轮 |
| JRA3Q 强迫读取 | 冷启动 32 ms/步（22 patch 首次），页缓存热后 6.7–12 ms/步 | 第 785 轮 |
| patch 物理单价（旧参照） | 珠江（LCT）约 1.1 ms/patch/步；黑河（PC）约 6.5 ms/patch/步（不同机器/负载，只能看量级） | 第 571/698 轮计时反推 |
| 0.5° 全球陆地格 | 61,178 / 259,200 = 23.6% | `docs/implementation-verification.md:36509-36510` |

### 2.2 0.1° 全球估算

0.1° 全球网格 3600 × 1800 = 6.48M 格。陆地格按 23.6%–30% 取 1.5M–2.0M。

patch 密度用 `/shr03/examples/Global_Grid_2x2_fastPC_VG_BGC-2005`（2.5°×1.875° 全球 fastPC 示例，2026-10-24 只读实测）标定：6,181 个陆地单元、18,497 个 patch = **2.99 patch/陆地单元**。据此：

| 量 | 0.1° 全球估算 |
|---|---|
| 陆地单元 | 1.5M–2.0M |
| patch | **4.5M–6.0M** |
| 模型状态内存 | **360–480 GB**（按实测 81 KB/patch；不含 history 缓冲，也不含每 rank 约 0.6 GB 的固定开销） |
| 每步 patch 物理 CPU | 2,400–5,500 core·s（LCT 实测 0.53–0.92 ms/patch；PC/BGC 另计，黑河 PC 可达 6.5 ms） |
| 512 核（8 节点 × 64）每步 | **5–11 s**；模型年 17,520 步 ≈ **1–2.2 天**（LCT，不含 I/O） |
| 节点间通信（每步约 0.3 GB） | tianhe 实测 40 Gb/s 下约 **60 ms/步**，占每步 0.5–1.2% |
| 每记录 history（示例的 221 个变量） | 约 **7.8 GB**（示例 16.7 MB × 469；见 §12 实测）；2026-10-25 0.1° 合成探针：未压缩 11.46 GB/记录（全部 6.48M 格）、压缩 1 级 3.36 GB/记录，写耗时 47.5 s / 175.3 s |
| 每次重启（main + pft/bgc + 旁车） | 约 **110 GB**（示例 19 KB/patch × 5.8M；见 §12） |
| 河湖汇流（固定全球网） | 每天约 4 s（128 线程）；相对 patch 物理可忽略 |

结论：

1. **必须多节点**：0.1° 全球的状态即 0.36–0.48 TB，单节点 512 GB 会被状态吃掉 70–94%，再加 history/强迫/分配器开销基本不可行；模型年 CPU 也在 10³ core·h 量级。tianhe 的 deimos 节点 64 核 / 512 GB，8–16 节点是合理起点（8 节点时每 rank 约 45–60 GB 状态）。
2. **0.1° 全球的瓶颈在 patch 物理，不在汇流**。黑河案例 97.6% 是汇流，因为只有 610 个 patch 而全球河网固定 252k 单元；0.1° 全球有 5M patch，汇流占比不到 1%。因此 **v1 不需要分布式汇流**，root 汇聚完全够；这也保住了逐位一致。
3. **history 与重启是 I/O 主要风险**：每记录 7.8 GB、每次重启 110 GB。history 必须改成“逐条记录写出”（今天按 `DEF_HIST_groupby` 整组缓冲，见 `crates/colm-runtime/src/history.rs:4293,4973`；第 603 轮已列为待做）。重启按块分布写，110 GB / 8 rank ≈ 14 GB/rank 可以接受，但必须实测文件系统吞吐与文件数。

## 3. 进程模型与分片

### 3.1 进程模型

- N 个 rank，每 rank T 个 rayon 线程；`N × T` 不超过申请的总核数（每节点 T × 每节点 rank 数 = 节点核数）。
- rank0 既当 root 也领一块计算任务，不设专职 writer（同上游 FLAT_SPMD 的 `p_root` 设计）。
- rank 数由用户给（GUI：进程数/节点数；CLI：`--ranks N`），默认建议 1 rank/节点起步，内存紧时 2 rank/节点。

### 3.2 分片：整块分配给 rank

- 分片单位是**完整 block**（`landdata/landpatch/<year>/landpatch_<block>.nc` 的块），不是连续 patch 区间。
- 理由：重启、restart-hist 旁车、tracer 旁车都是每块一份（`crates/colm-runtime/src/bin/colm-rs.rs:3902-3968`），整块归属后每个 rank 直接写自己的文件，文件名、内容、字节都不变；若按 patch 连续区间切，块的边界会被 rank 切开，重启要么汇聚、要么改名，逐位验证立刻复杂一个量级。
- 代价：rank 数不能超过活跃块数。0.1° 全球默认块布局 72×36（`DEF_nx_blocks`/`DEF_ny_blocks`，5° 一块，见 `crates/colm-srfdata/src/bin/mksrfdata-rs.rs:5440-5499`），活跃块约 2,300 个（示例 18×9 块里 145 个活跃，89.5%），远大于合理 rank 数，不构成限制。区域算例块数少（黑河 4 块、珠江 6 块），但本设计的目标是 0.1° 全球，不受影响。
- 分配算法：所有 rank 用同一份输入（`landdata/block.nc` 的块列表 + 各块 patch 数，或直接由块目录名与 `ipxstt/ipxend` 推算）做**全局连续块区间**切分：按块在全局顺序里的前缀 patch 数和均分边界。连续是关键 —— 各 rank 的局部 patch 数组按 rank 顺序拼接就是全局顺序，每步交换不需要按全局索引重排。结果写进日志与 provenance，不依赖文件系统遍历顺序。
- 负载不均：0.1° 全球块数多（约 2,300 个活跃 5° 块），连续切分的每 rank patch 数差异预计 <10%；Phase 2 实测，超过 20% 再考虑在连续区间内再按 patch 细分（那时交换需要全局索引）。

### 3.3 全局结构的归属

| 结构 | rank 上的形态 | 说明 |
|---|---|---|
| 本块 topology（patch 像素、element、settyp） | 局部 | 只读自己块的 `landpatch/mesh` |
| 全局 patch → rank 映射 | 复制（小） | 每个 rank 都算同一份确定性分配 |
| 状态 `states`、模板、forcing 映射、history 累加器 | 局部 | 大小 ≈ patch_count/N |
| 全局 river network + `RunoffRouting` + `RiverModel` | 仅 root | 252k 单元、23k 河系；root 读全局 mesh/topology 与 `DEF_UnitCatchment_file` |
| 全局 history 网格骨架与静态量 | root 有全局，rank 有局部 cell 部分 | 每 rank 只算自己的 cell，root 拼文件头 |
| 强制读取器 | 局部 | 每 rank 只打开自己 patch 覆盖的像元窗口（现有行为，窗口自动缩小） |
| `HistorySession` | 每 rank 有局部累加器；root 有写文件的 buffer | 见 §6.2 |

root 的额外内存：全局 topology 元数据（6M patch 的 `element/cells/land_type` 等，约 1–2 GB）+ river network（约几百 MB）+ 每记录 8 GB 的 history 接收与压缩缓冲。在 512 GB 节点上可接受。

## 4. 每步数据流与交换

| # | 阶段 | 位置 | 说明 |
|---|---|---|---|
| 1 | 时钟/CO2/月循环 | 所有 rank 复制 | 同一 namelist 推出，确定性 |
| 2 | 强迫读取 + 映射到 patch（含降尺度） | 局部 | 各 rank 读自己的像元窗口 |
| 3 | patch 物理（`par_iter`） | 局部 | 现有代码，不变 |
| 4 | tracer 报告 | root | 只影响诊断文字；收计数与最坏值 |
| 5 | 流域侧向流 | — | v1 拒绝 `CatchLateralFlow` + N>1 |
| 6 | 河湖汇流（含漫滩 debit/publish、示踪物、泥沙、水库/堤防/分汊） | 所有 rank 复制 | 每步 all-gather patch 级 runoff/漫滩输入；每个 rank 用同一份全局数组跑同一个 `RiverModel::step`（`crates/colm-runtime/src/river/mod.rs:540-677`），不做 scatter |
| 7 | 河湖 history / unitcat / basin | root | 各 rank 状态相同，只由 root 写全局文件 |
| 8 | 陆面 history 累加 | 局部 | 每 rank 累加自己的 patch → 自己的 history cell |
| 9 | LAI 刷新、BGC 非土壤更新、甲烷状态 | 局部 | 现有代码 |
| 10 | baseflow 优化器 | — | v1 拒绝 `DEF_Optimize_Baseflow` + N>1 |
| 11 | 重启写出 | 局部块 + root 河道 | 每 rank 写自己的块；root 写 `<case>_restart_gridriver_<date>.nc` |
| 12 | 进度行、成功标记 | root | 其它 rank 写 `log.rank<i>`，不污染 GUI 解析 |

### 4.1 每步交换的数组

不带示踪物/泥沙时（0.1° 全球首版的目标配置）：

- **收给 root**（patch 级）：`runoff_mm_s`（f64）、`included`（bool）、`flood.evap_mm_s`、`flood.infil_mm_s`，以及 `outputs` 里 root 需要的量。
- **发回各 rank**（patch 级）：`flood.depth_mm`、`flood.fraction`、`flood.credit`（漫滩/甲烷还要 `credit_patch` 等）。

按 5.8M patch、约 25 B/patch 收 + 24 B/patch 发估算：**每步约 145 MB 收、139 MB 发，合计约 0.3 GB**。tianhe 两节点实测 TCP 聚合 40.1 Gb/s，约 60 ms/步；相对实测每步 5–11 s 的计算占 0.5–1.2%。

带示踪物/泥沙/甲烷时数组变多（每个示踪物一组 patch 级向量），首版不验收这些配置；结构上预留按 `Option` 增删数组。

**河湖的第一步实现选择（2026-10-25）**：先让**每个 rank 都持有一份全局河网并同步跑同一个 `RiverModel::step`**（每步用 `PatchExchange` all-gather 局部 runoff/漫滩输入，所有 rank 得到同一份全局数组）。这样不需要 root-only 构造、不需要把 flood 结果 scatter 回去，逐位一致只依赖“输入数组相同 + 河网状态相同”。代价是 N 倍河湖计算与每 rank 一份全局 topology/河网内存；按 §2.3，0.1° 全球河湖占比 <1%，这个代价可接受。若真机实测河湖占比升高，再改回 root-only + scatter 或分布式。**该方案 2026-10-25 已实现**：hb 2 块、河湖开、history 关，1 rank（44 patch）与 2 rank（各 22 patch）跑 48 步后，主重启、旁车、`gridriver` 重启与 `.river` 历史文件全部逐字节相同。

## 5. 通信层

### 5.1 v1：纯 Rust TCP 星形

- root 监听 `0.0.0.0:0`，把实际 `host:port` 写到算例输出目录下的 rendezvous 文件（`out/<case>/.colm-spmd/endpoint`，共享文件系统本来就可写）；其它 rank 轮询该文件。
- rank/size 来源优先级：`--rank/--ranks` 显式参数 > `SLURM_PROCID`/`SLURM_NTASKS` > `OMPI_COMM_WORLD_RANK/SIZE` > `PMI_RANK/PMI_SIZE`。`srun` 与 `mpiexec` 都只当进程启动器。
- 消息：`u64` 长度前缀 + 原始小端字节（空帧是合法消息）；数组按全局 patch 索引切片。操作只需：`gather`（各 rank → root，root 按全局 patch 段填回）、`scatter`、`broadcast`、`barrier`。握手是唯一不带帧的交换（8 字节 rank/size + 1 字节 ack），这一点写进代码注释，避免同一连接上混用两种格式。
- 已落为独立 crate `crates/colm-spmd`（只依赖 `anyhow`，不碰 `unsafe_code = "forbid"`）：`RankInfo` 决定 rank/size，root 监听并把 `host\nport\n` 原子写进 rendezvous，worker 轮询连接；root 按 rank 顺序收发，worker 在 gather 后等空帧 ack，scatter/broadcast 由 root 按 rank 顺序发。
- 确定性：root 端所有数组按全局 patch 索引重排，不按到达顺序；接收/发送顺序不影响数值。浮点运算只在 root 的全局阶段和各自 rank 的 patch 物理里发生，次序与今天相同。
- 失败：任何 rank 退出 → root 在超时（例如 120 s 无进度）后 `abort`，整个作业非零退出；srun/调度器负责杀进程组。root 起跑前校验所有 rank 的块分配、文件存在与版本哈希，失败直接广播原因。
- 不引入新依赖：`std::net` + 手写帧；不碰 `unsafe_code = "forbid"`。

### 5.2 为什么首版不做 MPI

- 0.1° 全球的通信量（0.3 GB/步、8 GB/记录）在 10–200 Gb/s 网络上占计算比例很小（§4.1）；先把形状跑通、逐位守住，再谈 RDMA。
- 构建/分发代价不对称：MPI 要在**每个远程构建主机**上有 MPI 开发库（tianhe 系统只有 `mpiexec`，没有 `mpicc`；land02 只有 conda 里的 MPICH），还要绑定具体实现（OpenMPI/MPICH/Intel MPI），与“任何有 cargo 的服务器都能编”的现状冲突；与安装包自带的 `kernels/_runtime` MPI 运行时也有 ABI/install_name 风险（`docs/design.md:71-82`、`docs/implementation-verification.md:33760-33790`）。
- 逐位一致：手写 TCP 的归约可以强制按全局 rank/patch 次序做，比 MPI 的树形归约更容易证明。

**切换条件**（写进 Phase 0 测量）：2 节点网络实测中，若每步通信耗时 > 每步计算的 10%，或 history 单记录收集 > 1 min，则 Phase 3 后评估 MPI 后端或把 root 汇流改成河系分布式。切换只在远程 Linux、按 cargo feature 编译，不进入桌面安装包与默认 CI。

### 5.3 线程数

- 每 rank 的 `RAYON_NUM_THREADS = 节点核数 / 每节点 rank 数`，由启动脚本统一设置；用户覆盖优先。注意今天远程 Rust 作业把 `RAYON_NUM_THREADS` 设成整机线程数（`crates/colm-cli/src/remote_cmd.rs:1509`），N>1 时会让每个 rank 都开满线程，必须改。

## 6. I/O 设计

### 6.1 重启与旁车

- 每 rank 写自己的块：`write_block_restarts` 改成按“本 rank 的块列表”循环，文件名与内容不变（`crates/colm-runtime/src/bin/colm-rs.rs:3902-3968`）。
- 每块 restart-hist 旁车、tracer 旁车同样按块写。
- 河道重启 `<case>_restart_gridriver_<date>_lc<year>.nc` 由 root 写（本来就没有块后缀，`colm-rs.rs:3894-3900`）。
- 0.1° 全球每次重启约 110 GB（示例外推），8 rank 各 14 GB；写之前先测文件系统吞吐与目录文件数（2592 块 × 3 类文件/日期）。

### 6.2 history

- **先做逐条记录写出**（第 603 轮计划第 1 条）：每条记录到期即聚合、压缩、追加，时间维块长 1。这既是 0.1° 全球单进程也需要的（8 GB/记录 × 月度整组 = 240 GB 缓冲），也是多 rank 收集的前提。
- v1 仍是**单文件单 writer**：各 rank 算好自己的 history cell 值（block 对齐时 cell 不会跨 rank），按全局 cell 次序发给 root；root 用现有 writer 写同一个文件。收集量约记录大小（8 GB），200 Gb/s 上约 0.3 s。
- **block 对齐约束**：v1 要求 history 网格的 cell 边界落在 block 边界内（默认 5° 块，任何能整除 5° 的 history 分辨率都可以，例如 0.1°/0.25°/0.5°/1°；`DEF_HIST_lon_res/lat_res`）。依据是写侧已确认 patch 按 element 的块归属落到 `landpatch`（`element_blocks`，`crates/colm-srfdata/src/spatial.rs:3962` 等），history 与模型网格同分辨率时 cell 就是 element，天然不跨 rank。否则一个 history cell 会同时收到多个 rank 的 patch，求和次序无法与单进程一致；这种情况 N>1 直接报错并建议改成 block 对齐分辨率。`DEF_HIST_mode='block'` 仍未移植（`docs/implementation-verification.md:30045`），不作为 v1 路径。
- 单个 0.1° 记录 8 GB 在单节点写盘是分钟级；若用户要按天输出（365 记录/年 ≈ 2.8 TB），Phase 4 再评估多 writer/压缩，不在 v1 阻塞。

### 6.3 强迫场

- 每 rank 只读自己 patch 映射到的像元窗口（现有 `GriddedForcing` 行为，`crates/colm-runtime/src/spatial/forcing.rs:883-940`），窗口随分片自动缩小。
- 风险：很多产品的文件按“每条记录一整张全球场”分块，任何窗口都要解压整条记录；N 个 rank 会 N 倍解压/IO。JRA3Q 的月文件单个变量约 550 MB（`tmp2m_2010_01.nc`），0.1° 网格的窗口仍可能触发整条记录解压。Phase 0 测一遍；若单步强迫 >10% 时间，再做“按 rank 预裁强迫”或改分块（预处理任务）。

### 6.4 指纹与 provenance

- 现在空间算例的 `landdata`/`restart` 在阶段指纹里只是 `path-only-v1`（`crates/colm-cli/src/fingerprint.rs:1273-1300`），换块布局/换 rank 数不会让 `colm` 阶段失效 —— 多节点上线前必须把 **block 列表哈希 + rank 数 + 每 rank 块分配** 写进 `produced-by.json` 和 `colm` 阶段指纹。
- `produced-by.json` 增 `ranks/nodes/launcher/block-layout-sha256/block-assignment-sha256`。

## 7. 启动、远程与界面

- `colm-cli run --engine rust --ranks N`：**本地仍拒绝 N>1**（保留今天 `main.rs:2485-2489` 的规则，只把远程路径放开）；远程 `RunPlan` 放开（`remote_cmd.rs:1081-1084`）。N=1 时不生成任何 srun/MPI 参数，命令与今天一致。
- 启动层次：`srun/mpiexec` 只包**第三段（colm）**，前两段（mksrfdata、mkinidata）仍是单进程。也就是说 `colm-cli run` 正常跑前两段，然后以 `srun -n N --cpus-per-task=T <engine>/bin/colm-rs …`（Slurm，复用现有 `COLM_MPIEXEC=srun` 机制，`remote_cmd.rs:1525-1533`）或 `mpiexec -n N …`（其它调度/无调度）启动主循环；不能把整个 `colm-cli run` 复制 N 份。
- 资源申请：`--nodes`、`--ntasks=N`、`--cpus-per-task=T`；每 rank 的 `RAYON_NUM_THREADS=T`。
- 引擎构建：**不需要任何新依赖**；现有远程 `cargo build --release` 流程不变，官方预编 Linux 二进制也能直接用（比 MPI 路线少一整类构建问题）。
- 日志：root 日志保持现有 GUI 进度解析格式（`TIMESTEP = …`、`CoLM Execution Completed.`）；其它 rank 写 `<job>/log.rank<i>`，失败时把最后一个失败 rank 的尾部并入 root 日志。
- GUI（`gui/src`）：Rust 引擎在**服务器运行**时解锁“进程数/节点数/每节点核数”，沿用 Fortran MPI 的节点规划函数；本机运行继续锁 1（`gui/src/features/cases/NewCasePage.tsx:374,2057`、`gui/src-tauri/src/remote.rs:1286-1298`）。文案双语，说明“Rust 引擎在一个算例内跨进程运行，仅服务器可用”。
- `xtask check-gui` / 命令契约：若 `run_case`/`run_batch` 的 Tauri 参数或 CLI 参数变化，按 CLAUDE.md 跑 `npm run contract`、`cargo run -q -p xtask -- check-gui`。

## 8. 逐位一致契约

定义：同一 block 布局、同一算例、同一引擎提交，**1 rank 与 N rank 的全部输出逐字节相同**。

可实现的理由：

1. patch 物理今天已是线程数无关（`docs/implementation-verification.md:29674`、第 698 轮 1/8/16/128/240 线程逐字节）；每个 patch 只依赖本步强迫与只读共享数据。
2. 所有全局阶段在 root 上跑，输入数组按全局 patch 索引重排，运算次序与单进程逐位相同。
3. history 的 cell 级求和：block 对齐后每个 cell 的 patch 全在一个 rank，rank 内按全局 patch 相对次序求和，等式与单进程相同。
4. 文件写出者与代码路径不变（只是由不同进程写不同块）。

验收方式：对每个空间算例，`ranks=1` 跑一遍作基线，`ranks=2/4` 跑同一算例，`sha256sum` 比较所有 `*.nc`（history、restart、sidecar、gridriver）。

## 9. 验收矩阵

| 级别 | 环境 | 内容 | 通过标准 |
|---|---|---|---|
| V1 | 本机/tms17l；2026-10-25 已在 Mac 本机跑通 hb 2 块（无河湖/无 history、河湖开/history 关、河湖关/history 开、河湖开 + history 开）N=1,2 | 现有空间算例（黑河/珠江/全球示例）N=1,2,4 | 受限配置：重启/旁车/`gridriver`/`.river`/history/unitcat 文件 N=1 vs N=2/4 逐字节相同（已过，河湖+history 全配置 4 rank 10 个文件一致）；逐条写、多节点待做；N=1 路径逻辑不变 |
| V2 | tianhe deimos 2 节点 × 64 核 | hb64 河湖+history，N=1（1 task）vs N=2（每节点 1 rank） | **逐字节已过 2026-10-25**（131 个输出全部相同）；测试规模 1408 patch 下 2 节点河湖段反而更慢（12.7–15.4 s vs 3.17 s），加速比不作为本轮判据，Phase 4 按 0.1° 规模复核 |
| V3 | tianhe 8 节点 × 64 核 | 0.1° 全球 10 天片，fastPC，N=8 | 不 OOM；跑通；history/restart 可读；记录每步计时与峰值内存 |
| V4 | tianhe 8–16 节点 | 0.1° 全球 1 年（或用户指定窗口） | 可续跑；从续跑继续 1 天与 1 rank 结果逐字节 |
| 反向 | 任意 | `--ranks>1` + 本机 / CatchLateralFlow / baseflow / 非 block 对齐 history | 明确双语拒绝，不静默降级 |

## 10. 分阶段计划

| 阶段 | 内容 | 产出/验收 |
|---|---|---|
| **Phase 0**（完成 2026-10-25） | 设计文档 + 集群实测（M1 网络、M2 patch 单价/内存、M4 强迫读取）+ 0.1° 规模账 | 本文；M1/M2/M4 已完成并回填 §2；M3 history 写吞吐留到 Phase 4 |
| **Phase 1**（进行中；第一刀 2026-10-25 已落） | 分片账本（`spatial/partition.rs`）+ `SpatialTopology::read_selected` + 真实 fixture 的子集读对照；装配循环接线与 `--rank/--ranks` 参数随 Phase 2 一起做（单独接 CLI 没有可跑的路径） | 子集读与全量读逐 patch 相同；分片覆盖/确定性/负载测试通过；全部现有测试通过 |
| **Phase 2**（单节点验收完成 2026-10-25） | 传输层 + 启动握手 + `--spmd-check` + 局部装配/主循环/块级重启 + 复制河网每步 all-gather + 逐条 history（河湖开、HOURLY）；单节点 N=2/4 逐字节 | V1（已过：河湖+history 全配置 N=1 vs N=2/4 全部输出逐字节） |
| **Phase 3**（进行中；2026-10-25 tianhe 2 节点真算已过） | 远程启动（srun/mpiexec）、每 rank 线程、日志/provenance、GUI 解锁 | V2 逐字节已过；CLI 远程多进程入口已落（Rust `--ranks>1` + `COLM_MPIEXEC` 启动器）；GUI 已在远程 Rust 显示进程数（站批量仍 1）；剩余：GUI 节点数、`remote-run` 端到端 |
| **Phase 4**（部分完成 2026-10-25） | 0.1° 全球就绪：逐条 history 的 0.1° 压测、重启吞吐、mksrfdata 按块并行、强迫场测量 | history 写吞吐/并行直写已测；重启按 4.12 KB/patch 外推 ~27 GB/次；mksrfdata 单块 ~10.5 s 但按块并行未实现；V3 未做（缺全球 0.1° landdata） |
| **Phase 5**（按需） | MPI 后端、河系分布式、history 多 writer、patch 区间分片 | 只有 Phase 0/3 的测量证明需要时才做 |

依赖关系：Phase 2 依赖 Phase 1；Phase 4 的 mksrfdata 并行与 Phase 2/3 可并行开发；Phase 5 不阻塞 0.1° 全球首跑。

**Phase 2 进展（2026-10-25）**：传输层落盘为 `crates/colm-spmd`：`RankInfo`（显式 `--rank/--ranks` > `SLURM_*` > `OMPI_COMM_WORLD_*` > `PMI_*`）、root 监听 + 原子 rendezvous、worker 轮询连接、握手校验、按 rank 顺序的 `gather`/`scatter`/`broadcast`/`barrier`、`f64`/`bool`/`u64` 小端编解码。启动期分片握手落为 `crates/colm-runtime/src/spmd.rs`：`SpmdPlan` 读块清单并确定性分片，`SpmdSession` 建连后让 root 逐 rank 比对计划表（含 landdata 块清单指纹），不一致先广播错误再返回；5 项测试含 3 个真实进程的 TCP 同意与故意不一致。`colm-rs` 已接上：`--rank/--ranks/--spmd-check` 走 `SpmdSession`；真实多进程路径在 `--ranks>1`（或启动器 size>1）时只读本 rank 的块、装配局部状态、`worker_patch` 用全局偏移，root 打印进度与成功标记。2026-10-25 在本地把 hb 复制成 2 块、关河湖/history 跑了 144 步：1 rank（44 patch）与 2 rank（各 22 patch）的 4 个重启/旁车文件 **逐字节相同**；河湖仍打开的 2 rank 会在装配前被明确拒绝。河湖的每步交换 2026-10-25 也已落地：分片改成**全局连续区间**、新增 `spatial::exchange::PatchExchange`（3 进程真实 TCP 的 f64/bool all-gather），`SpatialRuntime` 在预置 exchange 时 all-gather runoff/漫滩 evap/infil、各 rank 跑同一个 `RiverModel::step`，flood 数组用 `patch_offset` 索引；河道重启与 `.river` 文件只由 root 写，块旁车保持一致。本地 hb 2 块、河湖开、48 步：1 rank 与 2 rank 的主重启/旁车/`gridriver`/`.river` 全部逐字节相同。逐条记录写 2026-10-25 也已落地（`HistoryBuffers::write_gridded_record` 首条建文件、之后 `netcdf::append` 按时间片写；内存从整组降到一条记录；单进程与多 rank root writer 共用），河湖开 + history 2026-10-25 也已打通（root 全局 writer 暂存河道量与 unitcat，worker 跳过；到期记录所有 rank 同步 `river.history.reset()`，否则块旁车 `history_river_required` 不一致），hb 2 块 48 步 1 vs 2 rank 的 history、unitcat、2 份主重启、2 份块旁车、gridriver 共 7 个文件全部逐字节相同；河湖关闭的 history 2026-10-25 已打通：每个 rank 本地外部写会话累加，到期用 `PatchExchange::all_gather_bytes` 把原始累加器收给 root，root 的全局会话（全局 grid/patch 数/forcing_mask）平均/聚合/写文件；hb 2 块 48 步 1 vs 2 rank 的 `hb_hist_2003-01.nc` 逐字节相同（sha256 `5f3c8e79…`）；写前要从本地会话复制 `plant_hydraulics`/`variably_saturated`/城市/BGC/动态湿地湖这些条件列开关，否则 schema 对不上。共用消息上限 16 GiB/条，0.1° 全球 history 单记录约 8 GB 仍可用，更大的流式化留到 Phase 4。

**Phase 1 进展（2026-10-25）**：`crates/colm-runtime/src/spatial/partition.rs` 落盘（`BlockInventory`：块清单、patch 数、全局偏移；`split(ranks)`：确定性连续块区间分片（2026-10-25 从 LPT 改为连续，为每步拼接交换铺路）），`SpatialTopology::read_selected` 可只读选中块；`read` 改为 `read_selected(None)`，N=1 路径不变。9 个分片单测 + 1 个真实 fixture 的子集读对照（`COLM_SPATIAL_FIXTURE=tmp/ep/hb/out/hb/landdata`）。CLI 参数与装配循环接线留到 Phase 2，避免先暴露一个不能跑的多进程开关。

## 11. 预处理与数据准备

- `mksrfdata` 仍是单进程。按 zh（20°×16°，0.25°）约 345 s / 5,120 元素的外推，0.1° 全球面积约 200 倍，单进程预计 **数十小时**；首版可接受一次性等待，但 Phase 4 要加“按块扇出”（`mksrfdata-rs --block`，目前没有；`mkinidata-rs --block` 已有）。
- `mkinidata`：2,592 个块，按 `--block` 并行提交作业数组，或内部并行；每块约 2 千 patch，写重启 110 GB 总量。
- 强迫场（用户 2026-10-24 定）：**JRA3Q**。tianhe `~/HDD_POOL/Forcing/JRA3Q/`（3.8 TB，1947-09 起逐月）的目录与文件名和 `vendor/CoLM202X/run/forcing/JRA3Q.nml` 完全一致：8 个变量、`<变量目录>/<变量>_<YYYY>_<MM>.nc`、`DEF_forcing%dataset='JRA3Q'`。tianhe 上的算例把 `DEF_dir_forcing` 指到该目录；模板默认 `startyr=2003`、`endyr=2024`、`dtime=3600`。
  注意：JRA3Q 原生网格是 0.5625°（TL479），0.1° 只是陆面/河道网格分辨率，不是大气强迫分辨率；需要更高的近地面变化时再换数据或开强迫降尺度。
- 块布局：0.1° 全球用默认 72×36 即可；如果用户已有算例的块数少，需重跑 mksrfdata（会改 landdata 身份）。
- 算例准备建议：先拿 `docs/implementation-verification.md` 里的空间回归算例把 V1 跑通，再切 0.1° 全球；不要一上来就烧 0.1°。

## 12. Phase 0 只读实测（2026-10-24）

只读 SSH 探测，没提交作业、没改服务器文件。

### 12.1 资源

| 资源 | 规格 | 关键约束 |
|---|---|---|
| **天河·星逸 tianhe**（`ssh tianhe`） | Slurm：`deimos` 1512 节点 × 64 核 / 512 GB；`phobos` 480 × 88 核 / 512 GB；`debug` 32 × 64 核（30 min）；MaxNodes 不限；IB（`mlx5_*`，挂载走 `o2ib200`）；登录 `ln201` 64 核 / 503 GB | 系统只有 `/usr/bin/mpiexec`，**没有 mpicc/libmpi 开发环境**；应用已在该服务器编好 Rust 引擎（`~/HDD_POOL/colm-desktop/engine/7752a17c7aa1aacc/bin/colm-rs`），计算节点上能直接跑；强迫只有 JRA3Q/JRA55/IsoGSM；`unitcatchment/` 有 6min/15min 两套。实测运维注意：计算节点**没有 `iperf3`、也没有 `/usr/bin/time`**（已把 iperf3+依赖库放到 `~/HDD_POOL/colm-desktop/tools/iperf`，内存从 `/proc/<pid>/status` 的 `VmHWM` 采样）；`srun -N2 -n2` 会挤到同一节点，要显式写 `--nodes=2 --ntasks=2 --ntasks-per-node=1 --distribution=block:block` |
| **land02**（`ssh land02`） | Slurm：`first` 7 × 96 逻辑 CPU / 320 GB（**MaxNodes=3**）；`largemem` tms04 96 CPU / 1.5 TB（MaxNodes=1）；`second` 7 × 128 CPU / 515 GB（MaxNodes=1）；共享盘 `/shr03` 89 TB | 共享数据 `/shr03/CoLM_Forcing/{JRA3Q,ERA5LAND,MSWX_V100,…}`、`/shr03/CoLMruntime`、`/shr03/CoLMrawdata`；系统无 MPI，miniconda 里有 MPICH 4.2.1（`mpicc`）；多节点最多 3 节点 × 320 GB |
| **tms17l**（`ssh tms17l`） | 单节点 2×Xeon 8490H，240 逻辑核 / 1 TB；无调度器；系统 MPI 齐全 | 适合单节点 N=2/4 的逐位验收与峰值内存测量，不适合真多节点 |

**对目标的影响**：0.1° 全球的状态 0.5–0.7 TB，在 land02 上 3 节点（0.96 TB）很紧，且没有更大 rank 数可用；tianhe 的 1512×512 GB 节点 + 无 MaxNodes 限制、JRA3Q 又已在本地盘，是合适的目标。代价是大气强迫只有 0.5625°，但这是用户已接受的取舍（§11）。

### 12.2 用现有全球示例标定

`/shr03/examples/Global_Grid_2x2_fastPC_VG_BGC-2005`（2.5°×1.875°，18×9 块，fastPC + BGC，2005 年）：

- 18,497 patch / 6,181 陆地单元 = 2.99 patch/单元；
- 一份日期（`2005-001-00000`）的 restart 共 429 个文件、351.6 MB → 19 KB/patch（含 main/pft/bgc/旁车）；
- 一个月度 history 文件 16.8 MB，`time=1`、221 个变量、96×144 网格 → 约 1.2 KB/cell（221 个变量里含分层与标量，不能按 8 B/变量反推）。

外推 0.1°（6.48M 格、5.8M patch、221 变量集）：

- history 每记录约 16.8 MB × (6.48M / 13,824) ≈ **7.9 GB**；
- restart 每次约 19 KB × 5.8M ≈ **110 GB**；
- 与 §2.1 的 120 KB/patch（含两份状态、模板与 history 缓冲）和 81 KB/patch（无 history 实测）两个锚点一致：restart 比内存小一个量级是合理的。

### 12.3 已完成测量（2026-10-25，tianhe，用户授权）

| 编号 | 结果 |
|---|---|
| M1 网络 | 两节点 cnode4691 / cnode4710（12.4.75.32 / 12.4.76.99），ping 平均 0.032 ms；iperf3 8 流 10 s：**40.1 Gb/s 聚合，单流约 5.0 Gb/s**。0.3 GB/步约 60 ms，相对 5–11 s/步可接受；TCP 星形够用，暂不需要 MPI。链路是 `gn0` 以太网（IB 只用于存储挂载）。 |
| M2 patch 单价与内存 | 本地 `tmp/ep/hb`（LCT IGBP，22 patch）上传到 tianhe，关河湖与 history、JRA3Q 强迫：480 步 t=1 墙钟 22 s、主循环 20.5 s（forcing 15.3 s、patch 5.1 s）、峰值 562 MB；t=8/64 分别 17/17 s。把块复制到 64 块（1,408 patch，3 天 144 步）：t=1 主循环 108.8 s（patch 107.4 s → **0.53 ms/patch/步**）、t=8 patch 14.3 s（7.5×）、t=64 patch 2.92 s（36.8×，效率 57%），墙钟 120/30/17 s；峰值 674/678/694 MB。两次峰值差 → **约 81 KB/patch**（无 history）。 |
| M4 强迫 | 同一批运行：22 patch 冷启动时 forcing 32 ms/步（JRA3Q 月文件按全球 slab 分块，读窗口也要解压整块），页缓存热后 6.7–12 ms/步；相对 0.1° 每步 5–11 s 可忽略，暂不做按 rank 预裁。 |
| M3 history 写吞吐 | **未做**。仍按第 603 轮的写入基准估算；逐条写实现后再在 0.1° 片段上测。 |

测量脚本与中间数据在 tianhe `~/HDD_POOL/colm-desktop/probes/{hb,hb64}`（13 MB / 52 MB，临时目录，不在仓库）。所有作业都是 `srun` 短作业（≤10 min），已确认队列为空。

## 13. 风险与开放问题

1. **history 单 writer**：8 GB/记录、按天输出时 2.8 TB/年。v1 靠逐条写 + 压缩；若用户要按天 + 全球 0.1° 十年，Phase 4 必须再设计（多 writer/按块文件）。
2. **重启 110 GB/次**：文件数约 2592 块 × 3 类；Lustre 类文件系统上要实测目录与元数据开销，可能需要打包/合并文件。
3. **root 读取全量 topology 的启动时间**：2592 个 landpatch 文件、约 1–2 GB 元数据；若太慢，改成“rank 发块清单 + root 只读 mesh”。
4. **history block 对齐约束**：用户若坚持 0.2° 之类不整除 5° 的分辨率，N>1 必须拒绝；长期可以改成“root 收 patch 级累加器再按全局次序聚合”，代价是 8 GB→更大的传输。
5. **`elmindex` 跨块唯一性**：`ElementGroups` 依赖全局 element 编号；latlon 路径未系统验证过（流域路径文档说 `eindex` 全局）。Phase 1 加一个跨块唯一性测试。
6. **块间负载不均**：5° 块的陆地占比差异；Phase 2 实测分配后的 patch 数偏差。
7. **强迫场 N 倍解压**（§6.3）。
8. **预处理**：mksrfdata 单进程数十小时；Phase 4 的按块扇出是独立工作项，可能比多节点主循环本身还大。
9. **tianhe 计算节点没有 C 链接器**（文档记录）：引擎必须在登录节点编好；多节点启动脚本不能假设计算节点能编译。
10. **rank 数/块数**：区域算例块数少的问题不影响 0.1° 全球，但 GUI 若允许对任意空间算例开多进程，要提前拒绝“rank 数 > 活跃块数”。

## 14. 否决的方案

| 方案 | 否决理由 |
|---|---|
| 首版直接上 MPI（含 rsmpi 或自写 FFI） | 构建/分发代价大（每个构建主机要 MPI 开发库；tianhe 没有），0.1° 全球的通信占比 <1% 时没有收益；先量再换 |
| 文件信箱/协调者进程做每步耦合 | 每步 0.3 GB 的交换用文件同步，延迟与失败模式不可控；改一个数就要扫目录 |
| 连续 patch 区间分片（Fortran 做法） | 重启/旁车按块写，边界块要 gather/merge；整块分片逐位最容易守。Phase 5 若负载不均再评估 |
| 河系分布式（跨 rank 邻接 halo + MIN dt） | 0.1° 全球汇流 <1%，不值得；先 root 单点跑，保留 Phase 5 |
| 把 MPI 链接进桌面安装包 | 与“安装包无需 MPI”的产品边界冲突，且和自带 `kernels/_runtime` MPI 运行时冲突 |
| 无通信的多进程（各跑各的算例） | 那是吞吐方案，不是“一个算例跨节点”；多站点/多算例今天已经能用调度器铺开 |

## 15. 需要用户确认的事项

1. ~~0.1° 全球的强迫场~~（2026-10-24 已定：JRA3Q）。
2. ~~目标集群~~（2026-10-25 已定：tianhe；land02 只作为备份/数据源）。
3. **算例配置**：0.1° 全球用 LCT 还是 fastPC/PFT/PC；是否开 BGC/甲烷/示踪物；模拟时段与输出频率（按天还是按月）、输出变量集。首批验收建议 fastPC + BGC、2003 年 1 个月、月度输出（变量集与现有全球示例一致），确认后写进验收矩阵。
4. ~~Phase 0 测量授权~~（2026-10-25 已授权；M1/M2/M4 已完成，见 §12.3）。
5. **下一步**：Phase 2c（把局部装配、每步交换与 root 全局阶段接进 `colm-rs`）。

**Phase 3 进展（2026-10-25）**：CLI 侧已接：`RunPlan` 允许 Rust `--ranks>1`，远程 command 生成 `COLM_MPIEXEC=srun` + `COLM_MPIEXEC_ARGS`（nodes/ntasks-per-node/distribution；非 Slurm 用 mpiexec）；`colm-kernel` 新增 external launcher，Rust 模型段在 ranks>1 时由 `COLM_MPIEXEC` 启动；本地 `colm-cli run` 仍要求 launcher 环境，否则拒绝。tianhe 两节点 `srun` 已跑通。构建用仓库自带 toolchain（cmake 4.4.4/rust）与共享 target；`--spmd-check` 2 节点通过（各 32 块/704 patch，offset 连续）。hb64 河湖+history 48 步：1 task vs 2 node 的 131 个输出（history 2 + 64 主重启 + 64 旁车 + gridriver）全部逐字节相同。踩坑：旧 `out/.colm-spmd/endpoint` 会让 worker 读到上一次的节点名（实测连接 cnode4751 超时），已改为 Slurm 下按 job/step 命名并支持 `COLM_SPMD_RENDEZVOUS` 覆盖；tianhe 2026-10-25 重构建后复验：保留旧 endpoint 的情况下 `--spmd-check` 与 2 节点真算都通过，131 个输出仍逐字节相同。2 节点河湖段 12.7–15.4 s vs 单节点 3.17 s，测试规模小、河湖占比高，Phase 4 按 0.1° 真实规模复核。

**Phase 4 进展（2026-10-25）**：新增 `crates/colm-hist/examples/gridded_stream_probe.rs`，用真实逐条 writer 写 0.1° 尺寸（6.48M 格、每格一个 patch）的 5 变量记录：未压缩 ~240 MB/s，外推 221 变量 47.5 s/记录、11.46 GB/记录；压缩 1 级只有 ~19 MB/s，外推 175.3 s/记录、3.36 GB/记录。磁盘 HDD_POOL 直写 440 MB/s。结论：history 的下一步优化是**逐记录并行压缩直写**（复用 `colm-h5chunk`，时间块长 1 正好并行）。2026-10-25 已实现：`ChunkWriter::write_record` + 按批 8 个变量并行打包，0.1° 探针 221 变量外推从 175 s/记录降到 **77.7 s/记录**（40 变量实测 14.07 s、607.7 MB），1 vs 2 rank 压缩 history 仍逐字节；重启 110 GB/次按磁盘上界约 250 s 单流、按 rank 分散约 32 s/rank。mksrfdata 按块并行与 V3 真实全球算例仍未做。

**Phase 4 进展（2026-10-25，续）**：history 已从串行 deflate（175 s/记录）优化到按记录并行直写（221 变量外推 77.7 s/记录，1 vs 2 rank 仍逐字节）；磁盘 HDD_POOL 直写 440 MB/s。重启实测 hb64 主重启 4.12 KB/patch、旁车 0.55 KB/patch → 0.1° 5.8M patch 约 27 GB/次（该配置），单流 ~62 s、8 rank 约 8 s/rank。mksrfdata 当前单块（hb 2×2 mesh 窗口）热运行 ~10.5 s、冷 ~32 s，`--blocks` 在小窗口不产生额外块；按块并行与全球测量仍未做。V3 需要带 `elmindex` 的 0.1° 网格与全球 rawdata，hb probe 的 mesh 不能作为 mksrfdata 输入，因此本环境无法完成 V3。

**Phase 4 全球预处理进展（2026-10-25）**：已从 tianhe 30 arcsec landtype 聚合出 0.1° 全球 mesh（2,211,055 个活跃格，`equal-lat-lon-landmask-v1`）。mksrfdata 全球 mesh 触发串行 raw-pixel 上限（400M），已加 `COLM_SRFDATA_MAX_RAW_PIXELS` 覆盖并提交全球 Slurm 作业；5° tile 试验成功（2500 elements/5092 patches），块并行+合并是备选路径。V3 尚未开始。

**Phase 4 全球预处理与 V3 进展（2026-10-25，续）**：0.1° 全球 mesh 已按 72×36 块扇出，1446 个活跃 5° tile 各自跑 mksrfdata，硬链接合并并重映射 `elmpixels` 到全球 `pixel.nc`，得到 **5.72M patch / 12 GB** 的 `global_landdata_v3`；8 rank `--spmd-check` 通过。`mkinidata-rs --block` 扇出生成 1446 块初场。**V3 已跑通**：8 节点、480 步（10 天 × 1800 s）、`CoLM Execution Completed.`；从第一次 37.5 min 主循环优化到 **21.3 min**（2.67 s/步 root），峰值约 62 GB/rank。优化项：删除每步 58 GB/rank 的状态 clone、只在 restart 到期时建快照、并行强迫映射、并行释放 PatchOutput、进度 flush、土壤水舍入容差 1e-12→1e-9。当前 V3 边界为河湖关闭、history 关闭；全局 `gridriver` 初场、河湖/history 开启、mpiexec 端到端仍未做。

**16 节点 V3 扩展（2026-10-25）**：同一 0.1° 全球 480 步算例，16 nodes × 64 核时主循环 616–634 s（10.3–10.6 min）、墙钟 16:26，相对 8 节点 1281 s 为 2.08×，每步 1.29 s；patches 占 64%、forcing 28%、restart 写出 6.6%。当前墙钟瓶颈已从主循环转到最终 restart 写盘（约 5.6 min），再优化应并行化每 rank 的按块重启写出。
