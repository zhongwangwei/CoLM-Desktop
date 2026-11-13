# 修复验证：全链选项审计（46 条发现的修复情况）

**日期**：2026-11-11
**被验证提交**：`1db67c4a`（"Keep configuration and run results consistent across the option chain"）+ `59780a6b`（Clippy 跟进）
**基线审计**：[`option-chain-audit-2026-11-10.md`](option-chain-audit-2026-11-10.md)（commit `237ec3a6`，46 条发现：P1×2 / P2×16 / P3×28）
**方法**：5 名只读验证员按审计分组逐条验证（A 选项数据层、B GUI 配置流程、C 运行链、D 输出数据层、E 结果页）+ Lead 跑全量基线、独立复核关键修复、检查生成物与文档可复现性。过程报告在 `tmp/verify/*.md`（gitignore）。
**性质**：只读验证；除本文件外未改动任何受版本控制文件；未提交。

---

## 1. 总体裁定

| 级别 | 总数 | FIXED | PARTIAL | NOT FIXED | 结论 |
|---|---|---|---|---|---|
| **P1** | 2 | **2** | 0 | 0 | 两个最危险缺陷均闭环：`_r8` 自毁配置、metrics 空数组静默漏站 |
| **P2** | 16 | 12 | 3（F-04、F-11、F-12） | 1（F-15） | 主链选项/运行/结果语义已修；剩余是「能力呈现/并发策略」类 |
| **P3** | 28 | 21 | 5（F-22、F-23、F-34、F-38、F-41） | 2（F-30、F-44） | 文档/注释/双语/死入口类大部分清理，剩余为小尾巴 |
| 合计 | 46 | **35** | **8** | **3** | 无 REGRESSION；但修复本身引入 6 条新问题（§4），其中 2 条建议本批补 |

**一句话结论**：`1db67c4a` 正确修掉了全部 P1 和 12/16 条 P2，方向与审计建议一致，测试与生成物同步更新，主链（GUI 选项 → 落盘 → 运行 → 输出 → 结果）未发现回归。**但"都已修好"不成立**：F-15（多站比较并发池仍硬编码 6）、F-04（10/14 个空间输出变量仍无法在 GUI 勾选）、F-11/F-12（provenance 未展示、远程光谱选项无预检）以及一批 P3 与 6 条新问题仍需收尾，另有**随安装包分发的合并版用户手册未随附录重生成**。

---

## 2. Lead 全量基线（`59780a6b` 工作树）

| 检查 | 命令 | 结果 |
|---|---|---|
| GUI 契约 | `cargo run -q -p xtask -- check-gui` | **268 已注册 / 234 被调用（26 动态）/ 事件配对全过** |
| 参数审计 | `cargo run -q -p xtask -- parameter-audit` | 1369 descriptors；`artifacts/` **零漂移**（可复现） |
| 生成表漂移 | `cargo test -p colm-schema --test drift`、`-p colm-hist --test drift`、`-p colm-core --test drift_landcover`、`-p colm-core --test drift_co2` | 4/4 通过（histmap 重生成后仍逐字节一致） |
| namelist 往返 | `cargo test -p colm-namelist --test roundtrip` | 5 passed |
| oracle histmap | `cargo test -p oracle --test histmap` | 5 passed |
| 手册附录可复现 | `python3 scripts/generate-manual-appendices.py` | 重生成后 `git status docs/user-manual/appendices` **无 diff**；附录 A/B `_r8` 计数 110/153 → **0/0** |
| GUI 前端 | `npm test` / `typecheck` / `lint` / `format:check` | **1803 passed / 0 failed**（审计时为 1776，净增 27）；类型/风格全过 |
| GUI Rust | `cargo test --manifest-path gui/src-tauri/Cargo.toml` | **682 passed / 7 failed / 1 ignored**；7 条全部是本机沙箱禁止 `ps`（`/bin/ps: Operation not permitted`，`process_image_name` 拿不到进程镜像），与修复无关；`59780a6b` 已解决上一轮 Clippy 问题，GUI clippy/fmt 通过 |
| 根 workspace | `cargo fmt --all --check` / `cargo clippy --workspace --all-targets -- -D warnings` | 均通过 |
| 根 workspace 测试 | `cargo test --workspace --lib --bins --no-fail-fast` | 见下：5 个 target 失败，**全部为沙箱/并行环境限制，非修复回归** |

**根 workspace 测试的 87 个失败逐类归因**（证据：`tmp/verify/root-tests-full.log`）：

| target | 通过/失败 | 归因 |
|---|---|---|
| `colm-cli --bin colm-cli`（并行） | 834 / 53 | **已知 HDF5/NetCDF 并行串扰**：CI 注释明确 `colm-cli/colm-init/colm-srfdata` 三 crate「共享 NetCDF/HDF5 进程状态，必须 `--test-threads=1`」，失败以 `Netcdf(-101)` 起始后连锁；同一二进制串行后降为 12 条且均为环境项（见下） |
| `colm-srfdata --lib`（并行） | 272 / 27 | 同上，CI 文档化并行串扰（历史实测 255/15、211/59） |
| `colm-workspace --lib` | 75 / 5 | 本机沙箱禁止嵌套 `sandbox-exec`（`sandbox_apply: Operation not permitted`）→ sandbox/build/parity 5 条；CI 正常环境不受影响 |
| `colm-kernel --lib` | 51 / 1 | `a_top_level_sidecar_leads_its_own_process_group` 探测进程组被沙箱拒绝（`Operation not permitted`） |
| `colm-agent --lib` | 430 / 1 | `aborted_password_input_is_hidden...`：沙箱内 `pty.openpty()` 报 `OSError: out of pty devices` |
| 其余 33 个 target | 1903+ 通过 | 全通过（含 gui 契约、drift、histmap、xtask、oracle 等） |

补充：对失败 crate 单独复跑（`tmp/verify/colm-cli-serial.log`）串行后 **875 passed / 12 failed**，12 条已全部归因为环境：
- 11 条为 `cannot inspect Study scheduler process: Operation not permitted` / 锁存活探测被沙箱拒绝（另有 `remote_bench::a_dead_holder_does_not_wedge_the_active_job_lock` 在 cargo 下单测也失败：脚本用 `kill -0` 判死进程，沙箱把信号调用拒绝成“仍存活” → 等成 `lock_timeout`，不是修复回归）。
- 1 条 `selected_source_app_alias_resolves_the_application_checkout` 是**直接运行测试二进制**（cwd=仓库根）导致 `--source app` 回落到当前检出；已在 cargo 下单独复跑该测试 **1 passed**（cargo 的 cwd 是包目录，不会回落）——属测试对 cwd 敏感，非修复回归。
- colm-init 串行复跑通过（如 `23 passed` target）；colm-srfdata 串行复跑在本会话时间预算内未跑完（并行失败已由 CI 注释的 NetCDF/HDF5 串扰解释，未发现修复相关失败）。**未发现任何由修复引入的测试回归。**

---

## 3. 逐条裁定

### 3.1 P1（全部 FIXED）

| F-ID | 原问题 | 裁定 | 证据 |
|---|---|---|---|
| F-01 | `_r8` 默认值可被 GUI 写回，造成自毁配置 | **FIXED** | `real_literal()` 归一化三路全覆盖：展示 `config.rs:93-105`、写入 `typed_scalar/typed_like/typed_pft_value`（4923-4934 等）、**读取时自愈旧文件**（`parse.rs:102-133` + `value.rs:137-158`）。测试 `config_tests.rs:6013-6063`（默认值全部可被 namelist 解析、写回不含 `_`、非法 kind 拒绝）+ `parameters_tests.rs` 默认值可读测试；附录 A/B `_r8` 110/153 → 0/0 |
| F-02 | metrics 返回空数组的站点从表与失败清单同时消失 | **FIXED** | `ResultsPage.tsx:264-271` 空数组抛双语错 → 逐站 failure（`:311-325`）；属审计允许的前端方案，CLI 未改 |

### 3.2 P2（12 FIXED / 3 PARTIAL / 1 NOT FIXED）

| F-ID | 裁定 | 关键证据 / 剩余 |
|---|---|---|
| F-03 材料族不随上游自动关闭 | **FIXED** | `model-settings.ts:437-479` 新增 BGC→甲烷、无河湖→泥沙/甲烷 routing 级联；`processFiles` 仅用 verified 资产判定 routing 依赖；测试新增 |
| F-04 histmap 漏 14 个真实写出变量 | **PARTIAL** | 生成链已正确：`xtask/src/hist.rs:93-100 extract_spatial` + `main.rs:719-771` 读 3 个新源（最小写点 7/3/3/7），`generated.rs` 新增 `SPATIAL_VARS` 20 条逐条与上游吻合，drift 守住。**但 GUI 仅 4/14 可勾选**：3 个 reservoir 的 `DEF_Reservoir_Method > 0` 与 7 个 sediment 的 `sediment_particle_enabled()` 数值 gate 无法被 `eval_runtime_gate` 求值 → 仍 `writable=None`；17 个死开关仍与"未知"不区分，缺全局 allow-list 门禁，无空间 golden 对账 |
| F-05 新建向导输出变量全标可写 | **FIXED** | `model_configuration` 回传 `history_variables`（`model_configuration.rs:183-187`），NewCasePage `:3700-3720` 按 `settable && writable` 判定并给原因；卡片「全部变量」改为列全量+原因（`HistoryVariablesCard.tsx:92,106-111`）。残留：切引擎后旧 `DEF_hist_vars%*.true.` 未剪枝（P3 N-02） |
| F-06 catalog storage 误导 639 项 | **FIXED（兼容式）** | 保留 legacy `storage: case-nml`（提交正文 Rejected 拆分，Study 已冻结 ID 依赖它），新增 `storage_target`（history 586/forcing 35/isotope 18/case 359/None 371）+ 真实 `write_strategy` + docs 说明；`parameters_tests.rs` 全字段断言 |
| F-07 新向导同日时段可过 | **FIXED** | `simulation-model.ts:550` 改 `form.end <= form.start`；时间步长校验移至第 5 步（F-29 一并修） |
| F-08 线程数被环境变量覆盖 | **FIXED** | `effective_threads` 显式优先（`sidecar.rs:2587-2601`），预算与子进程一致；`execution_environment` 记录进台账；测试 `sidecar_tests.rs:2669-2691` |
| F-09 运行登记键不规范化 | **FIXED** | `run_process_key` 覆盖 prepare/remember/take_cancelled/forget/cancel/startup_lock（`sidecar.rs:407-420,462+`）；符号链接别名测试 |
| F-10 运行前迁移未登记 | **FIXED** | `capture_migration` + 独立迁移登记表（`sidecar.rs:5103-5180`），取消先于 spawn/真进程 kill/import 失败释放均有测试；`cancel_runs`/关窗同时取消迁移 |
| F-11 provenance 缺实际并行信息 | **PARTIAL** | 数据侧已记录：`execution-launches.jsonl`（`provenance.rs:14-24,120-143`）+ `execution_environment`；`colm-kernel` 记录真实 program/args/launcher/ranks/threads（`run.rs:79-85,408-437`）。**但 GUI 未消费**：`run://started` payload 未变（`sidecar.rs:2697-2699`），历史页「线程数」仍读从未写入的 `evidence.threads`（`history.ts:312`） |
| F-12 远程拒绝 highres/soil-hyper | **PARTIAL** | 报错改双语（`remote_cmd.rs:2363-2366`）+ `docs/remote-toolchain.md` 说明；GUI 无预检/映射，仍在 staging 晚失败 |
| F-13 组合校验太晚 | **FIXED** | `run-model.ts:138-152 kernelRunProblem`（USEMPI/CROP 组合）在 RunPanel 迁移前预检（`RunPanel.tsx:700-715`） |
| F-14 多站预检一处失败中止整批 | **FIXED** | 池前 throw 移除，出处解析移入逐项（`ResultsPage.tsx:245,1247`），`comparisonSources` 不再吞错 |
| **F-15 并发池硬编码 6** | **NOT FIXED** | `ResultsPage.tsx:1260-1263,1330-1333` 仍字面量 `6`，无 `min(4, logical CPUs)`/本机-远程区分；与 `plan-results-workbench.md:35` 不符 |
| F-16 指标表/CSV 缺 KGE 分量与分布 | **FIXED** | 表头/单元格（`ResultsPage.tsx:3256-3267,3302-3322`）+ CSV 末尾 6 列（`analysis-state.ts:174-199,275-282`）+ 真实断言测试 |
| F-17 混基准时间标签误导 | **FIXED** | 条件摘要改「各运行时间基准见结果行」+ 逐行 time_basis 列（`:3219,3245-3248,3282-3289`） |
| F-18 可见性不感知引擎 | **FIXED** | `VisibilityContext.rust_engine` + `runtime_is_rust`；CasesPage/NewCasePage/版本页均传 engine；config_tests 新增 rust/fortran 对照测试 |

### 3.3 P3（21 FIXED / 5 PARTIAL / 2 NOT FIXED）

| F-ID | 裁定 | 说明 |
|---|---|---|
| F-19 docs catalog 计数 | FIXED | v2/1369/1008/187 已更新 |
| F-20 source_location MT 字段 | FIXED | 39/39 修正；CI audit 强制 `source_location==文件:行`；artifacts/附录同步 |
| F-21 派生字段写入口不一致 | FIXED | `put` 对 group=None 拒绝（`config.rs:5001`），白名单仅 `DEF_USE_CROP`；测试真实 |
| F-22 ESACCI/派生理由过期 | **PARTIAL** | `docs/upstream-bugs.md` 新增 ESACCI 不可达记录；但 `field.rs:46` 仍称 wetland 由宏决定（实为 MT 文件赋值），`config.rs:81` 派生理由仍旧 |
| F-23 注释数字漂移 | **PARTIAL** | `field.rs`/`source.rs` 已清；`config.rs:81-88` 仍写「实测 6 个 / 行号 1406 / 30 个 / 68 个」（实为 10 / 1843-1845 / 31 / 34） |
| F-24 audit 双向覆盖 | FIXED | `xtask` 增加 namelist 成员/声明双向差集 + CI 强制执行；docs 写明 rawdata/PFT include 排除 |
| F-25 `&end` 被拒 | FIXED | `parse.rs:24-33` 与 `/` 同义；parse 测试含负向用例 |
| F-26 附录类型列 Rust Debug | FIXED | `scripts/generate-manual-appendices.py` 在文档边界转 `character(len=256)`；附录已重生成 |
| F-27 两套 visibility 元数据 | FIXED | docs 明确 static vs runtime 语义 |
| F-28 新向导缺 CBL/臭氧 | FIXED | `NewCasePage.tsx:3102-3108` 增加指向算例第 3 步的说明（审计允许方案） |
| F-29 时间步长校验位置 | FIXED | 移入第 5 步 |
| **F-30 touched() null 三态** | **NOT FIXED** | `model-settings.ts:391-394` 仍两态；审计自评 suspected、现有调用点不可达 |
| F-31 失败保留旧解析结果 | FIXED | 请求开始/失败清空 5 个能力状态 + request key 新鲜度 |
| F-32 case-run-settings LRU | FIXED | RunPanel `:1593-1599` delete+set 实现真 LRU（`serializeCaseRunSettings` 本身保持插入序） |
| F-33 PFT/PC 改写提示 | FIXED | ModelSettings 说明「新建 PFT/PC 自动使用 IGBP 地类数据」 |
| F-34 运行链错误双语 | **PARTIAL** | mpiRanks/threads/preprocessors 3 条已双语；`failure_reason` 无「内核失败原因（原文）」前缀；`validate_run_id`/登记错误/`missing migration case` 等仍单语，前端无映射 |
| F-35 本机 Rust 多 rank 文案 | FIXED | RunPanel `:2044-2053` 明确本机单进程语义 |
| F-36 批量准备可取消 | FIXED | `RunPanel.tsx:767,1214-1216` + 迁移登记 |
| F-37 threads/ranks 上下界 | FIXED | 前端 256→4096，与后端一致 |
| F-38 stale/远程写保护 | **PARTIAL** | 预处理重跑现在任意阶段都置 stale（`main.rs:4149-4156`，测试 `run_stage_tests.rs:888+`）；远程写保护未动（`sidecar.rs:772` 注释仍「远程不拦」），`mark_results_stale` 死白名单项仍在（`ipc.ts:68`，生产零调用） |
| F-39 甲烷去重键错 | FIXED | `is_methane_diagnostic`（`switch.is_none() && runtime 含 DEF_USE_METHANE`），33 条异名行不再产生；测试 |
| F-40 单点 CompressLevel 误导 | FIXED | 单点一律隐藏并给准确原因（`config.rs:2478-2480`），上游 `MOD_HistSingle`/`MOD_MT_Hist` 单点分支确认不压缩；测试 + 双语映射 |
| F-41 报告分栏/死别名 | **PARTIAL** | `guide.ts` 已删 `result-export`；但 `report` 死分支仍在（`ResultsPage.tsx:1826,3458+`）、PNG 快照仍无入口；**且 `crates/colm-agent/src/prompt.md:24` 仍把 `result-export` 列为可用步骤** → 助手链路必然报「不是可用页面」 |
| F-42 缓存 FIFO/重跑不清 | FIXED | 命中删后重插（`:1040-1048`）+ 晚到写回身份守卫（`:1050`）+ `completedRunCase` 触发清缓存（`:819-842`）；副作用见 N-05 |
| F-43 多站观测无自动映射 | FIXED | 勾选时 `defaultObservation` 回填（`:1614-1637`，对象身份防覆盖）；竞态见 N-03 |
| **F-44 自动评估失败不重试** | **NOT FIXED** | `ResultsPage.tsx:1189-1196` 仍先占位后调用、失败不回滚，无重试入口 |
| F-45 比较页 tim_res 值域 | FIXED | ComparePage `:2462-2477` 补 Year/3Hour/6Hour/8Day 共 9 项 + hint |
| F-46 双语漏写 | FIXED | 字段标签/报告 notice/未知 warning（含测试）；但 warning 判定有 N-04 |

---

## 4. 修复引入的新问题 / 本次验证新发现

> 编号沿用分报告：N-01 由 A/B/D 三方独立发现，最值得本批修。

| ID | 严重度 | 问题 | 证据 | 建议 |
|---|---|---|---|---|
| **N-01** | **P2（本 fix 引入）** | 新建向导初始 `engine=''`，F-18 起无条件把 `engine:""` 发给 `model_configuration`，后端对 `Some("")` 报「未知模型引擎」，确定模型版本后即出现全局 warning，直到第 1 步选引擎 | `NewCasePage.tsx:178,885`；`config.rs:1002-1012`；`optionError` 渲染于 `NewCasePage.tsx:2853`；测试因默认 engine='rust'+假 IPC 未覆盖 | 发 `engine: form.engine \|\| null`，或后端把空串当未指定；补一个空引擎初始态用例 |
| **N-02** | **P2（发布物）** | 合并版用户手册（`CoLM-Desktop-用户手册.md/.html/.pdf`，随安装包分发）未随附录重生成：仍含 **263 处 `_r8`**、70 行 `Character { len: … }`，附录 B 缺「写入目标」列、39 条 MT 字段仍写 `MOD_Namelist.F90`（F-01/F-20/F-26 在源码/附录修好但用户手册未体现） | `docs/user-manual/CoLM-Desktop-用户手册.*` 时间戳为 11-10，未在 `1db67c4a` 中更新；合并脚本 `tmp/manual/build-final.py`（未入库） | 重跑合并/HTML/PDF 生成并提交；把该生成步骤写进发布检查清单 |
| N-03 | P3 | F-43 自动观测异步回填与比较前置校验竞态：勾选后立即点比较时 `obs` 尚未落地，池前 `entries.some(!obs)` 仍整批中止（与 F-14 逐项失败不一致） | `ResultsPage.tsx:1240-1246,1621-1636` | 「缺观测」并入逐项 failure，或等 pending 回填 settle 后再放行 |
| N-04 | P3 | F-46③ 的未知 warning 判定先翻译再扫输出中文：已翻译但捕获组含中文标签的 warning 会被误标「后端中文原文」 | `result-overview.ts:119-166`；测试只覆盖纯未知中文/英文 | 用「是否命中已知 pattern」布尔量决定是否加标注 |
| N-05 | P3（suspected） | F-42 使目录/运行 effect 依赖 `completedRunCase`：用户正查看旧运行时，同算例后台新运行完成会清缓存、变量回默认、曲线消失 | `ResultsPage.tsx:810-842` | 比较 `source`/`run_case` 后再决定是否重置；只清新运行的缓存 |
| N-06 | P3 | F-41 删别名后 `crates/colm-agent/src/prompt.md:24` 仍列 `result-export for reports`，助手被系统提示引导去打开不存在页面 | prompt.md:24 vs `guide.ts:201-232` | 同步 prompt；加 guide 别名 ↔ prompt 步骤交叉校验测试 |
| N-07 | P3 | F-04 新增的 blocked_by 文案（「仅已核对 Fortran 空间写出点…」「条件 … 需要人工判断」）未接入 `historyBlockedReason`，英文界面显示中文 | `histvars.rs:96-110`（新原因）；`history-variables.ts:394-402` 只认三种句式 | 补映射或统一走 bilingual |
| N-08 | P3 | 运行链杂项：`append_execution` 失败用 `?` 让「阶段已成功」的运行报失败（与 provenance 尽力而为策略冲突）；迁移登记在独立 registry，写保护/互斥仍不覆盖；迁移成功后迟到停止会把 ledger 标 cancelled；新增迁移/登记错误全英文 | `main.rs:4266-4267`；`sidecar.rs:5103+`；`model.rs:53,5117` | 记录类写入改尽力而为或与 produced-by 策略统一；补双语；迁移纳入写保护设计 |
| N-09 | P3 | F-05 残留：切引擎/内核/前提后，之前勾选的 `DEF_hist_vars%*.true.` 仍留在 `caseValues` 并随创建 payload 写入（卡片已显示「—」但未剪枝） | `NewCasePage.tsx`（caseValues 与 history cap 解耦） | 引擎/内核变化时清掉不再 settable 的 hist 覆盖 |
| N-10 | P3（suspected） | F-16 加列后指标表 20/22 列无 `.table-scroll` 包裹（全局有该样式、比较页在用），窄窗可能溢出 | `ResultsPage.tsx:3242+`；`styles.css:475` | 包一层 `.table-scroll` 并补窄窗回归 |
| N-11 | P3 | A 组残留：`config.rs:81-88` 旧注释（F-22/F-23）；`field.rs:46` wetland 派生理由仍错；`real_literal` 未收敛到 runtime/physics 的 trim/split（当前无 `_r4` 默认值，latent）；附录生成脚本未入 CI | 见 §3.3 | 随文档批清理 |

---

## 5. 测试质量评估（供后续补测参考）

- **真实新增断言**：F-01/F-06/F-20/F-21/F-24/F-25（数据层）；F-03/F-07/F-18/F-31（GUI）；F-08/F-09/F-10/F-13/F-38（运行链）；F-39/F-40（输出层）；F-16/F-46（结果层）。这些测试核对过，不是空转。
- **零新增测试的已修项**：F-02、F-14、F-17、F-41、F-42、F-43、F-45（E 组）、F-32、F-33（GUI 细节）、F-12/F-35/F-36/F-37（运行链文案/前端）；建议按分报告 §④ 的清单补：空 metrics→failure、未登记副本只该行失败、LRU 顺序与重跑清缓存、勾选自动回填观测、timRes 写入 YAML、guide↔prompt 交叉校验。
- **已知测试环境限制**：本机沙箱禁止 `ps`/`kill`/进程组探测/`sandbox-exec`/PTY，导致 7 条 GUI Rust + 87 条根 workspace 失败；CI 正常环境无此问题。串行复跑 colm-cli 后 875/887 通过，剩余 11 条为进程检查 EPERM，1 条为直接跑测试二进制时 cwd 造成的 `--source app` 解析差异（已在 cargo 下单独复跑该测试并 **1 passed**，确认非修复回归）。
- **提交信息声称**：前端 1803、根 2090、CLI 887、init 239、srfdata 357、Study 24、roundtrip/drift、all-target Clippy——本次基线能复核的部分（前端 1803、契约 268、drift/roundtrip/histmap、clippy/fmt、附录可复现）全部吻合；根 suite 因环境无法完整复核。

---

## 6. 建议的收尾批次（等待批准后动手）

**批次 R1（小、低风险，建议本批修）**：N-01（空 engine → `engine: form.engine || null` + 测试）、N-03（缺观测并入逐项失败）、N-04（warning 标注判定）、N-06（prompt.md 同步 + 交叉校验）、N-07（新原因双语）、F-30（touched 三态）、F-44（自动评估失败可重试）、F-15（并发池 `min(4, cpu)` 并区分本机/远程）、N-05（后台完成不重置当前查看的旧运行）、N-10（`.table-scroll`）。
**批次 R2（需先设计，中等）**：F-04 剩余 10/14（`DEF_Reservoir_Method` 与 `sediment_particle_enabled()` 的求值策略或显式 allow-list；17 个死开关与未知区分）、F-11（GUI 展示实际启动命令/线程）、F-12（远程光谱选项预检/映射）、F-34/F-38 剩余（批量双语、远程写保护、死白名单）、N-08（provenance 失败策略、迁移登记互斥、双语）、A 组残留注释与 normalization 收敛、附录生成入 CI。
**批次 R3（发布物）**：重生成合并版手册 md/html/pdf 并检查打包；把 `python3 scripts/generate-manual-appendices.py` + 合并脚本纳入发布清单。

---

## 7. 局限

- 本机沙箱限制导致根 workspace 全套测试无法按 CI 语义完整复核（见 §2）；结论基于 CI 已文档化的并行串扰说明 + 串行复跑 + 失败类别归因，未在无沙箱环境重跑。
- 未实跑真实单点/多站/远程算例；F-01 的 gfortran 读取行为沿用审计期实测（gfortran 16.1.0：`-1.0` 可读、`-1.0_r8` 不可读），未在本轮重跑。
- 未逐行重审 `crates/colm-agent`、研究（Study）全链与打包链；N-02 的手册合并脚本在 `tmp/`（未入库），只能确认产物未更新。
- 过程证据：`tmp/verify/verify-options-data.md`、`verify-gui-flow.md`、`verify-run-chain.md`、`verify-output-data.md`、`verify-results-ui.md`、`root-tests-full.log`、`colm-cli-serial.log`、`gui-tests.log`、`gui-rust-tests.log`。
