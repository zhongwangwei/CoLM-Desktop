# CoLM-Desktop 深度代码审查报告（2026-08-24）

> 审查方式：6 条独立检查线（子代理并行、各自读源码取证）+ 主控交叉验证。
> 范围：`crates/*`（约 48.8k 行 Rust）、`oracle`、`xtask`、`gui/src-tauri`、`gui/dist`（前端）、`vendor/CoLM202X`（Fortran 内核快照）、`docs/`。
> 交叉验证原则：子代理转述不直接采信，所有高严重度判定均回源核对；线间矛盾以源码为准。

## 0. 总评

**无 S1（致命）问题。** 项目整体架构自洽度显著高于一般代码库：三层契约（generated.rs ↔ MOD_Namelist.F90/MOD_Hist.F90 ↔ Fortran 读取代码）抽查 30+ 处全部对齐；单位体系（kg/m²/s≡mm/s、K/°C、W/m²）全线一致；评估指标公式（RMSE/MAE/Bias/R²/r/NSE/KGE 分解）全部正确；黄金回归判官逐位比较且输入 sha256 溯源。多数"严重"候选在交叉验证后被降级或排除（见 §3）。真实风险集中在**进程健壮性**与**文档漂移**两类。

> **修复复核（2026-08-24）**：下面保留最初候选，便于追溯；本表是最终结论，取代原始严重度标签。

| 候选 | 最终结论 |
|---|---|
| S2-1 | **已修复**：普通运行、批量运行和 Study 都可取消；Unix 杀独立进程组，Windows 用 `taskkill /T /F`，关闭窗口也清理运行树。未加任意 24 小时 watchdog，避免误杀合法长模拟。 |
| S2-2 | **已修复**：注释与设计文档已区分能量警告和水收支停止路径。 |
| S2-3 | **非 bug**：并发已受本机逻辑 CPU 上限约束；固定上限 8 会无故限制高配工作站。内存感知调度应作为独立产品策略。 |
| S2-4 | **已修复**：Study 失败/取消立即发送终态事件，前端同步闭环。 |
| S3-1/3/6/7/11 | **误报或已存在保护**：`is_ok()` 对空 history 返回 `false`；KGE α 的共同归一因子相消；SMP 成对约束、schema 使用守卫、分钟级/15 分钟时区均已实现。 |
| S3-2/9/10 | **设计/平台策略**：MINGW workaround、发布物自洽校验和 Warning 原样呈现不是运行 bug；其中过期 manifest 测试样本已同步。 |
| S3-4/5/8/12 | **已修复/加固**：经纬度边界校验、namelist 组校验、动态 `DEF_LC_YEAR` 产物名和 Fortran 行号均已更新。 |

原始候选分布：S2 × 4、S3 × 12、S4 × 若干；复核后没有遗留的 S1/S2 代码缺陷。

---

## 1. 原始 S2 候选（保留作审计输入）

### S2-1 子进程无超时，模型挂死不可取消（可取消性已修复）
- **位置**：`crates/colm-kernel/src/run.rs:189`（`child.wait()` 无 timeout）、`gui/src-tauri/src/sidecar.rs:363`。
- **证据**：全仓无一处对 `colm.x` 执行设置超时。仅 MINGW 平台在命中 success_marker 后 kill（`run.rs:172-183`），其余平台挂死则 `colm-cli` 与 GUI 永远阻塞。
- **影响**：数值发散或 MPI 死锁时用户无法取消（`TaskStop` 杀 colm-cli，子进程成孤儿）。
- **建议**：加 watchdog（如 24h 无输出或超时阈值），超时 kill 子进程并报 `Failure::Timeout`；GUI 侧取消按钮直接杀进程组。

### S2-2 balance violation 的"不停"论述对水收支路径不成立（已修复）
- **位置**：`vendor/CoLM202X/main/CoLMMAIN.F90:1588`；`crates/colm-kernel/src/outcome.rs:57-60`；`docs/design.md §6.5`。
- **证据**：实测 `CoLMMAIN.F90:1576-1589` 水收支路径在 `CoLMDEBUG` 下有 `CALL CoLM_stop()`（:1588），仅能量路径（:1502-1505）只警告。`outcome.rs` 与 design.md §6.5 均声称"打印后继续跑、没有 CoLM_stop"——对水路径错误。
- **影响**：失败标记 `balance violation` 检测本身不失效，但"CoLM 自己不执行宁可炸"的设计论述与实际行为不符，维护者按文档排查时会被误导。
- **建议**：更新 outcome.rs 注释与 design.md §6.5；或明确区分能量/水收支两条路径的停止策略。

### S2-3 GUI run_batch 无绝对并发上限（复核为产品策略，不改）
- **位置**：`gui/src-tauri/src/sidecar.rs:518-520`（`batch_width` 只做 `clamp(1, available)`）、`:544-552`。
- **证据**：worker 池 = `min(max_concurrent, available_parallelism)`，用户把 `cpu-workers` 填满时一次起 `available_parallelism` 个内核进程（64 核机器即 64 进程），每个进程多线程/多 MPI rank，可能 OOM。
- **建议**：绝对上限（如 8）+ UI 提示；或按物理内存估算。

### S2-4 study_run 事件+Err 双通道无回包协议（终态事件已补齐）
- **位置**：`gui/src-tauri/src/sidecar.rs:1070-1080`；`gui/dist/app/results.js:1872,1901`。
- **证据**：`study_run` 失败时既返回 `Err` 又 emit `study://event`；前端进度推进依赖 invoke 返回值，失败路径靠手动"刷新"，无事件驱动补位。
- **影响**：多成员研究批量失败时 UI 进度停滞、状态不闭环。
- **建议**：失败时 emit 一个带成员 id 的终态事件，前端据此推进 boundedMap。

---

## 2. 原始 S3 候选（最终结论见 §0）

| # | 位置 | 问题 | 备注 |
|---|---|---|---|
| S3-1 | `crates/colm-cli/src/main.rs:3477-3494` + `:1680` | `history_files` 对空目录 `bail!`，而 `have_all` 用 `is_ok()`：colm 失败后 history 被 `clear_history` 清空 → 下次判定 have_all=true，配合指纹（若仅 HISTORY 类字段变化恰好被忽略的路径）可静默跳过 | 需区分"目录存在但空"与"目录不存在" |
| S3-2 | `crates/colm-kernel/src/run.rs:172-183,215-219` | MINGW 命中 success_marker 即 kill，用硬编 `Some(0)` 掩盖真实退出码 | 依赖 MSYS2 netCDF 缺陷的版本行为，ponytail 注释已说明，但版本漂移即失效 |
| S3-3 | `crates/colm-hist/src/metric.rs:72 vs 81,99` | KGE 的 α 用 n 归一 `sm_ss.sqrt()/so_ss.sqrt()`，展示 sd 用 n−1：小样本/常数序列时 α 与展示值自相矛盾 | 公式本身正确（KGE 原文也用标准差），建议统一为 n−1 |
| S3-4 | `crates/colm-case/src/build.rs:137-138` | `{x:?}` 渲染 f64 无 `is_finite` 守卫：正常值无碍，但 lon/lat 等若为 inf/NaN 会写出 gfortran 不认的 `inf` | 建议 `is_finite` 校验或 `{:.17e}` |
| S3-5 | `crates/colm-namelist/src/document.rs:90-124` | `insert` 对已存在路径走 `set` 且不校验目标组，跨组同字段可被静默改写 | |
| S3-6 | `crates/colm-case/src/tuning.rs:171-176` | `validate_value` 单字段不校验 SMPMAX>SMPMIN 成对约束（F90:2015 有硬校验，本地不兜底） | |
| S3-7 | `crates/colm-schema` 的 drift 测试（`tests/drift.rs`） | 测试运行期会写回 generated.rs（虽还原）；且只守护 F90 声明，`values`/`requires` 的源码扫描器（xtask/usage.rs）无守护 | |
| S3-8 | `crates/colm-cli/src/main.rs:1660-1661` | mkinidata 产物名硬编码 `_lc2005_w180_s90`：契约本身正确（见 §3 验证），但启用 LULCC 或改 `DEF_LC_YEAR` 即失效 | 建议产物名从 manifest/源码推导 |
| S3-9 | `crates/colm-kernel/src/manifest.rs:106-156` + `oracle/scripts/build_kernel.sh:354-373` | sha256 只验"二进制与清单自洽"，不验"与源码一致"；`manifest_tests.rs:11` 的 SAMPLE 用旧 8 参数 generator_args，与真实 manifest（4 参数）脱钩 | |
| S3-10 | `crates/colm-kernel/src/outcome.rs:67-70` + `overrides.rs` | 同一行 `Warning: balance violation` 既判失败又报"覆盖"，UI 语义矛盾 | |
| S3-11 | `crates/colm-forcing/src/gapfill.rs:236-239` | ERA5 donor 对 Local 时区推断取整小时，忽略 15/30/45 分钟时区 | 印度等站点会错半格 |
| S3-12 | `docs/` 与 `crates/colm-srfdata/src/site.rs:785-813` 注释 | Fortran 行号系统性漂移约 6 行（MOD_SingleSrfdata 47/87/88/89 实为 41/80/81；CoLMMAIN 1545/1620 实为 1502/1577） | vendor 同步后未回改 |

---

## 3. 交叉验证排除的"假警报"（重要）

以下候选由子代理报为高严重度，主控回源核对后**排除或降级**：

1. **`_w180_s90` 产物名"Fortran 不生成"（B 线 S2）→ 排除**。
   `vendor/CoLM202X/share/MOD_Block.F90:578-605` 的 `get_blockname` 在 SinglePoint 宏下（:183-184 固定 360×180 块）由 `lon_w(1)=-180, lat_s(1)=-90` 必然拼出 `w180_s90`；且 `AU-Preston/restart/const/` 实测存在 `AU-Preston_restart_const_lc2005_w180_s90.nc`。B 子代理只 grep 字面量未追踪运行时拼接。
2. **`soil_texture` 编号冲突（F 线 S1）→ 排除**。
   `crates/colm-srfdata/src/texture.rs:7-11` 注释 + `MOD_Initialize.F90:261` BVIC_USDA 表逐值一致（1=clay…12=sand），CN-Cng=8→BVIC 0.100 正确。design.md §2.7 的"BVIC=0.23"是**文档漂移**（golden 再生后未同步），非代码 bug。
3. **"改 history 变量后被静默跳过"（B 线 S2）→ 降级 S3**。
   `crates/colm-cli/src/fingerprint.rs:41-59` 的 `_ => false` 分支确认 colm 阶段 `DEF_HIST*` **参与指纹**，正常改字段会触发重跑。真实缝隙只剩 S3-1 描述的"失败后空目录"组合。
4. **`landtype as i32` 截断（C 线 S2）→ 降级 S3**（上游分类应为整型，风险低）。
5. **KGE 公式错误（无）**：r/R²/NSE/KGE 全部正确，β 近零/反号只标记不改值是有意设计。

---

## 4. 六维结论

| 维度 | 结论 |
|---|---|
| **合理性** | 高。三阶段编排（mksrfdata→mkinidata→colm）、指纹跳过机制（防"产物在但输入已变"）、判成败三合一（退出码+日志标记+产物）、单位换算表、评估公式均合理且有测试钉住 |
| **完整性** | 高。单点必需 namelist 21 字段全写、482→346→119 变量门控闭环、CSV/ERA5 前处理验收矩阵与测试一一对应；缺口集中在进程级（无超时、无绝对并发上限） |
| **物理缺陷** | 无量级错误（W/m²↔MJ/m²、°C↔K、mm/s↔kg/m²/s 全部正确）。真实风险：无剖面站点 fallback 全层 loam 标称剖面（S2 提示级，已留痕 synthesized）；`tintalgo=nearest` 与内核 `uniform` 的边界语义差异未文档化 |
| **Bugs** | 无致命 bug。S3 级：KGE α 归一不一致、{x:?} 无 is_finite、insert 跨组覆盖、SMPMAX 成对校验缺失、空 history 目录误判 |
| **自洽性** | 高（代码层）；文档层漂移明显：design.md §2.7/§6.5 与 golden 现状矛盾、plan-m0-m1.md 仍写 git submodule、Fortran 行号系统性偏移 |
| **扁平化** | 好。crate 边界清晰、依赖单向、无上帝模块（排除）。候选拆分层：`colm-cli/src/main.rs` 142KB 单文件（27 子命令）、`gapfill.rs` 2090 行、`config.rs` 2604 行、`results.js` 2033 行 |

---

## 5. 修复结果（按原建议对照）

1. **运行取消**（S2-1）—— 已完成；没有用任意超时误杀合法长模拟。
2. **history 判定**（S3-1）—— 复核为误报：空目录已令 `have_all=false`，无需修改。
3. **守恒说明**（S2-2）—— 已同步代码注释和设计文档。
4. **数值入口**（S3-3/S3-4）—— KGE 无公式错误；站点经纬度已在读取边界拒绝非有限值和越界值。
5. **动态地类年份**（S3-8）—— 已按 `DEF_LC_YEAR` 推导 mkinidata 产物名，缺省仍为 2005。

*报告由主控交叉验证产出；子代理原始六线报告可作为追溯材料。*
