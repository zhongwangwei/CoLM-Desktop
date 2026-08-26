# CoLM-Desktop 深度代码审查与性能报告（2026-08-26）

## 0. 总评

相对 2026-08-24 审查，本轮没有重复上报已清零的 S1/S2，也没有发现新的 S1；新增确认并解决了 **3 个 S2、6 个 S3、1 个 S4**：Study 主动取消不落持久终态、内核实际宏与若干外部目录不进入阶段指纹、数值时区/PFT 整数入口过宽、前处理产物可安装符号链接、损坏 forcing 的非有限值错误不清楚、OAT 损坏样本可触发 panic、结果页快速切换异步竞态，以及完整导出被 LRU 长期保留。20 万点与 132 个 history 文件的实测表明 Rust/NetCDF/降采样路径不是当前瓶颈；本轮唯一有证据值得实施的性能改动把 12 份完整导出的隔离 Node 堆保留量从 **73.2 MiB 降到 0.0 MiB**，同时保留有界绘图缓存。

结论：本轮确认项均已修复；根 workspace、GUI workspace、全部前端 Node 检查、两张生成表 drift 与 namelist 逐字节 roundtrip 均通过。由于本机只有 macOS，Windows/Linux 仍由 GitHub CI 提供最终平台证据。

## 1. 发现的问题（按严重度）

| 严重度 | 位置 | 问题 | 影响 | 建议 |
|---|---|---|---|---|
| S2 严重 | `gui/src-tauri/src/sidecar.rs:1477-1494`；`crates/colm-cli/src/study/runner.rs:465-529` | GUI 原来写入 `cancel.request` 后直接杀死 `study-run` 进程树，被杀调度器来不及把 `Running/Evaluating/Queued` 写成终态。 | 磁盘权威 checkpoint 残留运行态；下次启动会进入 `NeedsReview`，用户主动取消反而需要人工恢复。 | **已修复**：GUI 记录并终止准确 PID；内部 finalize 命令核对 `run.lock` 所有者且确认进程已退出，再把未完成任务写成 `Cancelled`、清理 `stage/process`、追加事件并移除锁。PID 不匹配或仍存活时拒绝修改。 |
| S2 严重 | `crates/colm-kernel/src/manifest.rs:84-113`；`crates/colm-cli/src/main.rs:1678`；`crates/colm-cli/src/study/runner.rs:307-316` | 阶段指纹原来只使用 `preset@git#profile`，没有包含 `platform`、`generator_args` 与实际生效的 `macros`。 | 同名 preset/commit/profile 被不同宏重建时，旧 surface/restart/history 可能被错误判定为 current 并静默跳过。 | **已修复**：保留旧 `identity()` 给显示与 Study provenance，新增排序宏后的 `stage_fingerprint_identity()` 专供普通运行和 Study 的重跑判定。 |
| S2 严重 | `crates/colm-cli/src/fingerprint.rs:198-207`；`crates/colm-cli/src/fingerprint_tests.rs:193-238` | `looks_like_config_path()` 没识别 `DEF_dir_runtime`、`DEF_DA_obsdir`、`DEF_DS_HiresTopographyDataDir` 等目录字段；同一路径下文件变化只留下相同路径字符串。 | BGC/臭氧/同化/地形降尺度输入在原目录内更新后，阶段仍可能错误跳过并使用旧结果。 | **已修复**：字符字段名含 `dir` 时纳入外部输入内容指纹；分别覆盖标准 `DEF_dir_*` 与无分隔符 `*DataDir` 回归。 |
| S3 一般 | `crates/colm-forcing/src/tabular.rs:992-1015` | CSV/TXT 数值 `utc_offset` 原来只要求可表示为整秒，接受 UTC+05:00:01 这类民用时区之外的偏移。 | 时间轴可产生秒级错位，破坏去重、观测配对与 ERA5 对齐。 | **已修复**：统一为整分钟合同；`5.75`、`9.5`、`8.75` 等 15/30/45 分钟偏移继续有效。 |
| S3 一般 | `gui/src-tauri/src/config.rs:1983-2003` | PFT Integer 参数经 `f64 as i64` 写入；巨大但有限的 `1e20` 会在 Rust 中静默饱和。 | 专家参数可写出并非用户输入、也超出 Fortran 默认整数范围的值。 | **已修复**：整数性验证后再要求处于 `i32::MIN..=i32::MAX`，超界时保持全部算例不变并给出明确错误。 |
| S3 一般 | `gui/src-tauri/src/sitedata.rs:186-229` | staged 产物用 `is_file()` 校验会跟随 symlink，随后 `rename()` 把链接本体安装成正式 `site.nc`/forcing 文件。 | 正式产物可能逃逸目标目录并依赖外部可变文件；目标路径本身为 symlink 时也可能被错误接管。 | **已修复**：安装前用 `symlink_metadata()` 拒绝 staged 与 final symlink，再执行原有同目录、双文件原子安装与回滚。 |
| S3 一般 | `gui/dist/app/results.js:530-566`、`:571-611`、`:1379-1424` | `prepareActivePane()`、数据浏览器及多站点评估目录原来没有统一请求代次/作用域守卫。快速 A→B 站点或页面切换时，慢的 A 响应可覆盖 B 的变量、时间范围或比较目录。 | 用户看到的结果与当前站点/页面不一致；失败响应也可能在新页面显示旧错误。 | **已修复**：按 pane、data browser、comparison scope 分别使用请求代次，并同时核对当前 step、case、观测与 scope key；迟到成功和失败都不再写 UI。 |
| S3 一般 | `crates/colm-forcing/src/met.rs:37-50`；`crates/colm-cli/src/main.rs:3299-3305` | forcing `time` 轴的 `Inf/NaN` 未在摘要入口拒绝，参考高度只过滤 `NaN`；损坏文件可能最后表现为泛化 JSON 序列化错误。 | 用户无法定位 forcing 损坏，且非有限时间可能继续参与步长判断。 | **已修复**：摘要逐点拒绝非有限时间并报告索引；probe 对全部非有限高度统一输出 `null`。 |
| S3 一般 | `crates/colm-cli/src/study/runner.rs:1462-1467` | OAT 结果写出对 baseline 使用 `unwrap()`；手工损坏/不完整样本可让 CLI panic。 | Study 结果命令异常终止且缺少可处置的上下文。 | **已修复**：改为带 `OAT Study has no baseline member` 上下文的普通错误。 |
| S4 提示 | `gui/dist/app/results.js:648-665` | CSV 完整导出使用 `maxPoints:null`，但结果仍进入 12 项 `seriesCache`；每次导出都可能把 10 MiB 级 JSON 对象长期留在 WebView。 | 连续导出不同变量组合时造成不必要的堆增长和 GC 压力；不影响数值正确性。 | **已修复**：仅完整导出绕过 LRU；默认 2400 点和诊断用有界绘图请求继续缓存。 |

## 2. 性能分析（每条附基准数字）

### 2.1 结果工作台：Rust → JSON → Tauri → WebView

测试输入为单 history 文件 20 万步、3 个一维变量；另建 132 个 history 文件、总计同样 20 万步。命令使用当前 `target/release/colm-cli`，本机为 Darwin arm64。计时受当时系统安全扫描负载影响，因此本节把**字节数与堆保留量**作为优化收益主证据，不把 0.02/0.04 秒的差异解释为回归或加速。

| 路径 | 点数/变量 | 输出 | 实测耗时 | 峰值 RSS | 结论 |
|---|---:|---:|---:|---:|---|
| `series` 完整 | 200,000 × 3 | 10,552,931 B（10.1 MiB） | 基线 0.02 s；当前复测 0.04 s | 基线 41.9 MiB；当前 40.1 MiB | 完整 CSV 导出确实是大 payload，但 Rust 生成本身很快。 |
| `series --max-points 2000` | 实际 1,313 × 3 | 69,504 B | 基线 <0.01 s；当前 0.01 s | 基线 25.3 MiB；当前 25.3 MiB | 源侧保极值降采样把字节数减少 **99.34%**；默认绘图 2400 点策略正确。 |
| `metrics --summary-only` | 完整 20 万配对参与指标 | 1,287 B | 基线 0.03 s；当前 0.06 s | 基线 35.7 MiB；当前 39.6 MiB | 指标用全样本、IPC 只回摘要，现有两阶段设计正确。 |
| `metrics --pairs-var Rnet --max-points 2000` | 完整指标 + 1,000 绘图点 | 49,706 B | 基线 0.02 s；当前 0.04 s | 基线 38.2 MiB；当前 38.3 MiB | 图形诊断没有绕过降采样。 |
| `series`，132 文件 | 总 20 万步 × 3，最多 2000 点 | 约 69 KiB | 0.01 s | 22.6 MiB | 文件数不是当前热点。 |

**确认的瓶颈 → 证据 → 优化 → 收益：**完整导出的 10.1 MiB payload 一次传输是显式导出所需，不能靠删数据规避；真正无收益的是把它继续存入绘图 LRU。用实际 10.1 MiB JSON、同一个 `LruCache(12)` 做隔离 Node 堆测试，修复前 12 份解析结果保留 **73.2 MiB**，修复后完整导出绕过缓存，GC 后保留 **0.0 MiB**。一次性 JSON 解析仍需约 14–17 ms；若未来单次导出达到百万点并成为实测瓶颈，应新增后端流式 CSV 写盘，而不是让 uPlot 接收全量点。

### 2.2 `gapfill.rs`：逐点、逐月与 ERA5 donor

245,469 点 forcing（短缺口、50 点长缺口、48 点辐射缺口及 QC 越界）实测：`forcing-gap-probe` **0.02 s / 38.7 MiB**，完整 `forcing-repair` + donor **0.18 s / 76.0 MiB**。短缺口/QC 插值、长缺口 donor 校正及 1800 s 时间对齐全部完成，未留 unresolved。

代码中确有按月份组织 overlap 的循环，但整条 24.5 万点修复低于 0.2 秒，当前没有证据说明它是用户可感知热点。**本轮不做向量化/并行化**：预期收益小于 NetCDF IO 与输出复制噪声，反而会扩大数值路径改动面。未来只有在真实百万级、多变量批处理 profile 显示该循环占主导时，才把月份样本改成单次分桶。

### 2.3 NetCDF 读取模式

`history-catalog`：1 文件 <0.01 s / 18.6 MiB，132 文件 0.01 s / 12.1 MiB；132 文件的 `series --max-points 2000` 为 0.01 s / 22.6 MiB。当前实现虽然先整读一维变量再保极值降采样，但在 20 万点与 132 文件尺度没有形成瓶颈。

**本轮不改 hyperslab/chunk 读取**：极值保持要求看完整序列，过早按固定步长抽取会漏峰值；百万点以上若 RSS/耗时实测失控，再按 NetCDF chunk 遍历并在线维护每桶 min/max。

### 2.4 并发、资源与解析

1000 份 `case.nml` 解析与差异计算实测 **0.58 s / 4.3 MiB**，因此站点下拉卡顿不能归因于 Rust namelist parser。批量运行和 Study 已把并发限制在请求值与 `available_parallelism()` 的较小者，前端目录读取也限制为最多 4 路。

**本轮不引入 rayon/新依赖**：模型进程本身才是 CPU/内存主负载，无界并行会争抢 NetCDF/HDF5 IO 与物理内存。现有 worker 上限是有意的资源保护；将来应先以真实四站/多成员运行的 CPU 利用率和 RSS 决定是否提高上限。

## 3. 已修复/已改动的清单

| 文件 | 改动 | 验证命令 |
|---|---|---|
| `crates/colm-kernel/src/manifest.rs`、`manifest_tests.rs` | 增加稳定且完整的阶段内核配置身份，宏顺序规范化。 | `cargo test -p colm-kernel manifest_tests::`；根 workspace test/clippy/fmt。 |
| `crates/colm-cli/src/fingerprint.rs`、`fingerprint_tests.rs`、`main.rs` | 目录内容进入指纹；普通运行使用完整内核配置身份；forcing probe 过滤非有限高度；注册取消 finalize。 | `cargo test -p colm-cli fingerprint_tests::`；`cargo test -p colm-cli forcing_probe_tests::probe_never_serializes_non_finite_heights -- --exact`；根 workspace test/clippy/fmt。 |
| `crates/colm-cli/src/study/runner.rs` | Study 使用完整阶段身份；取消后安全落 checkpoint；OAT baseline 缺失返回错误。 | `cargo test -p colm-cli study::runner::tests::verified_gui_cancel_closes_active_tasks_and_rejects_a_new_scheduler -- --exact`；根 workspace test/clippy/fmt。 |
| `crates/colm-cli/src/forcing_probe_tests.rs`、`crates/colm-forcing/src/met.rs`、`met_tests.rs` | 非有限 forcing time/height 回归。 | `cargo test -p colm-forcing met::met_tests::forcing_summary_rejects_a_non_finite_time_axis -- --exact`；根 workspace test/clippy/fmt。 |
| `crates/colm-forcing/src/tabular.rs`、`tabular_tests.rs` | 数值 UTC offset 收紧为整分钟。 | `cargo test -p colm-forcing tabular::tabular_tests`；根 workspace test/clippy/fmt。 |
| `gui/src-tauri/src/config.rs`、`config_tests.rs` | PFT Integer 写入前增加 Fortran 默认整数范围检查。 | `cargo test --manifest-path gui/src-tauri/Cargo.toml pft_expert_defaults_and_sparse_batch_overrides_use_fortran_slots`；GUI workspace test/clippy/fmt。 |
| `gui/src-tauri/src/sitedata.rs`、`sitedata_tests.rs` | staged/final symlink 拒绝与不破坏旧产物回归。 | `cargo test --manifest-path gui/src-tauri/Cargo.toml sitedata::sitedata_tests::prepared_pair_rejects_symlinks_before_installing -- --exact`；GUI workspace test/clippy/fmt。 |
| `gui/src-tauri/src/sidecar.rs`、`sidecar_tests.rs` | Study 取消携带被终止调度器 PID 并触发持久终态。 | `cargo test --manifest-path gui/src-tauri/Cargo.toml sidecar::sidecar_tests::a_study_cancel_can_identify_the_scheduler_it_killed -- --exact`；GUI workspace test/clippy/fmt；`check-gui`。 |
| `gui/dist/app/results.js`、`gui/tests/results.mjs` | 结果页异步代次/作用域守卫；完整导出绕过 LRU。 | `node --check gui/dist/app/results.js`；`node gui/tests/results.mjs`；全部 `gui/tests/*.mjs`；`cargo run -q -p xtask -- check-gui`。 |

完整验证结果：

- `cargo test --workspace --lib --bins`：通过。首次在系统安全扫描高负载下，既有 `a_completed_mingw_stage_does_not_wait_for_broken_dll_cleanup` 的 5 秒墙钟断言失败；隔离复跑 0.36 秒通过，随后全套复跑通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo fmt --all --check`：通过。
- `cargo test --manifest-path gui/src-tauri/Cargo.toml --workspace --lib --bins`：133 项通过。
- `cargo clippy --manifest-path gui/src-tauri/Cargo.toml --workspace --all-targets -- -D warnings`：通过。
- `cargo fmt --manifest-path gui/src-tauri/Cargo.toml --all --check`：通过。
- `cargo run -q -p xtask -- check-gui`：61 个命令注册/调用与 6 个事件全部闭合。
- 全部 `gui/tests/*.mjs`：通过。
- `cargo test -p colm-schema --test drift`、`cargo test -p colm-hist --test drift`、`cargo test -p colm-namelist --test roundtrip`：通过。

## 4. 未修复项与理由

1. **KGE β/α、常数序列、n 与 n−1：复核为上一轮已经正确处理，不改。** `metric_tests` 仍覆盖少于两对、常数序列、近零均值与异号均值；本轮没有发现公式级新缺陷。
2. **普通/批量运行取消、空 history 目录、MINGW 成功标记 workaround：不是新增缺陷，不重复修改。** 普通取消已终止整个进程树，空 history 会拒绝跳过，MINGW 定向测试与全套复跑通过；本轮新增点仅是 Study checkpoint 的持久闭环。
3. **namelist 跨组字段与逐字节往返：复核通过，不改。** `roundtrip` 5 项与 root workspace 单元测试均通过。
4. **`gapfill` 月份循环不做向量化。** 24.5 万点完整 donor 修复仅 0.18 秒，没有性能证据支持扩大数值改动面。
5. **NetCDF 不改 hyperslab/chunk。** 132 文件、总 20 万点目录与降采样均约 0.01 秒；当前先整读再保极值比固定步长抽样更可靠。
6. **不引入 rayon 或新缓存层。** 1000 算例解析 0.58 秒；并发瓶颈在外部模型进程与 IO，新增依赖不满足收益门槛。
7. **完整 CSV 导出仍有一次性全量 IPC。** 这是用户明确要求完整数据的路径；本轮只删除无收益的长期缓存。达到百万点且实际导出变慢时，再做 sidecar 流式写盘。

## 5. 遗留风险

- 本机只实跑 macOS。Study finalize 的 Unix `ps` 与 Windows `tasklist /FO CSV` 都有 cfg 分支和 Clippy 覆盖，但 Windows/Linux 的真实进程终止与 CI 构建结果仍需 GitHub Actions 确认。
- Study 取消已用 checkpoint/PID 单元测试和现有 Unix 进程树测试覆盖；没有为本轮再启动一个耗时数小时的真实 Fortran 多成员 Study 做中途取消 E2E。
- 结果页是无类型检查的手写 ES module；本轮 Node 检查能钉住守卫与接口，但没有浏览器自动化模拟真实 Tauri WebView 的 A→B 网络时序。
- 性能计时期间本机 `XProtectService`/`syspolicyd` 负载较高，墙钟时间有噪声；输出字节数、RSS 与隔离 Node 堆保留量比单次耗时更可信。
- 当前环境没有独立 `rust-analyzer` diagnostics/`ast-grep` 命令；以双 workspace `cargo clippy -D warnings`、单元测试、drift、roundtrip、`check-gui` 和 Node 检查替代。
