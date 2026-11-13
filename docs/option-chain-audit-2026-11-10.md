# 全链选项审计：GUI → 结果输出

**日期**：2026-11-10
**基线**：`237ec3a6`（beta.8 发布后状态）
**方法**：Lead + 5 名只读审计员并行（A 选项数据层、B GUI 配置流程、C 运行链、D 输出/历史数据层、E 结果页/比较页/历史页），各写独立报告于 `tmp/audit/`（gitignore）；Lead 跑统一基线、复核每个审核线的 top 发现、合并重复项。除本文件外未改动任何受版本控制文件。

---

## 1. 结论摘要

**主链是通的，没有 P0。** 逐项追踪结果：

- **964 个 schema 字段**与上游 `MOD_Namelist.F90` + `MOD_MT_Namelist.F90` 的 407 个 namelist 成员逐名对齐，0 处 group 错配；4 个派生类型容器、87 个 PFT include 键、`&nl_colm_rawdata` 容器各有归属，`parameter-audit` 的 `unclassified_total=0` 成立。手册附录 A（964 项）/附录 B（1369 项）与生成表**逐 ID 完全一致**。
- **五步配置**每一类选项都有 GUI 入口并真实写入（无「GUI 有入口但 payload 丢字段」，无「后端可写但 GUI 无入口的算例标量」）；`new_case` 的 `apply_fields` 会把 `DEF_hist_vars%*` 正确路由到 `history.nml`。
- **运行选项 24 项**（引擎/前处理/内核/日期/步长/输出频率/重启/历史变量/路径/ranks/线程/并发/节点预算/运行位置/调度器/资源/阶段/强制/指纹/模型修订/参数覆盖/打包/运行身份/provenance）逐项对上 GUI → invoke → argv/env → 内核，未发现「GUI 传了但 CLI 静默丢弃」。
- **§2.4 的「失败也 exit 0」历史包袱已闭合**：`colm-kernel::outcome` 的失败标记 + 成功标记 + 产物四道检查由 CLI 每段执行，远程作业脚本再传 CLI 退出码；GUI 按 CLI 退出码判定。
- **输出数据层**：读 NetCDF 全部经 `colm-cli` sidecar（窗口进程不链接 NetCDF/HDF5，约束满足）；缺测/`spval`、UTC/local/model-clock 时间基准、全量统计 vs 抽稀展示、频率标签反推与冲突警告、删除的 plan+token+quarantine+inode 校验均已实现并有测试。

**但确实存在一批 bug 与完备性缺口**，按严重度：

| 级别 | 数量 | 一句话 |
|---|---|---|
| P0 | 0 | 未发现会静默产出错误科学结果或不可恢复丢数据的缺陷 |
| **P1** | **2** | ① GUI 展示的 `_r8` 默认值被接受并原样写回后，`case.nml` 连自己都解析不了、gfortran 也读不进；② 多站比较中 `metrics` 返回空数组的站点从结果表与失败清单同时消失（静默漏站） |
| P2 | 16 | 输出变量闸门漏变量、新建向导输出变量全可勾、材料族不自动关闭、运行线程被环境变量静默覆盖、运行登记键不规范化、迁移不可取消、provenance 缺线程、远程/本地选项不等价、比较并发/指标列/时间基准、Fortran 专属选项不可见等 |
| P3 | 28 | 文档/注释/元数据漂移、解析器不收 `&end`、双语漏写、缓存策略与计划不符、交互细节等 |

**最该先修的 5 条**：F-01（`_r8` 写入后自毁配置）、F-02（静默漏站）、F-04（流域/河湖/泥沙输出变量 GUI 无法勾选）、F-05（新建算例勾了写不出的输出变量且无提示）、F-03（材料族不随上游自动关闭导致创建被拒）。

---

## 2. 审计范围与全链选项矩阵（压缩版）

```
GUI 表单                    Tauri 命令（268 个契约）           落盘                       内核/输出                   读取/展示
NewCasePage 五步向导 ──┐
CasesPage 五步设置  ──┼─ new_case/new_spatial_case ── case.nml/forcing.nml/history.nml/参数文件 ─ colm-cli run ─ Fortran/Rust ─ out/*_hist_*.nc ─ colm-cli history-catalog/series/metrics ─ ResultsPage/HistoryPage/ComparePage ─ 导出
VersionHub/参数目录  ──┘   case_data_save/set_field*        （原子写：staged+备份+回滚）       （失败判定在 colm-kernel）      （全部经 sidecar）              （比较/评估/删除/重跑）
```

各层完备性结论（详细矩阵见 `tmp/audit/*.md` 各自的 ② 节）：

| 层 | 覆盖面 | 结论 | 主要缺口 |
|---|---|---|---|
| 选项数据层 A | 964 schema 字段、1369 descriptors、6 个 namelist 组、4 类落盘位置 | 主干可靠；字典/默认值/类型一致；读不 materialize 默认值、reset 只删覆盖、写为空跳过 | 目录元数据 `storage`/`source_location` 与真实落盘不符；`_r8` 默认值不是 namelist 合法输入 |
| GUI 配置流程 B | 五步全部选项类、创建 payload 逐字段、互斥/依赖规则、校验、草稿/脏标记 | 入口完备、payload 不丢字段；history 变量随创建写入 | 材料族自动关闭缺失；新建时段校验弱于已有算例；新向导输出变量不做可写性判定；新向导缺 CBL/臭氧入口 |
| 运行链 C | 24 项运行选项、失败判定、取消/关窗 owner、并发/去重、远程等价、provenance | 选项主链完整；失败判定与取消覆盖可靠 | 线程数被 `RAYON_NUM_THREADS` 静默覆盖；登记键不规范化；运行前迁移未登记；provenance 缺线程；远程拒绝 highres 选项 |
| 输出数据层 D | 586 输出开关、频率/分组、catalog/series/metrics/energy/diagnostics、删除/重跑 | 单点默认链路守卫齐全 | histmap 漏 14 个真实写出变量（流域/河湖/泥沙）；甲烷块去重键错；单点压缩项误导 |
| 结果 GUI E | 结果工作台各分栏、比较、历史、报告/导出、缓存、多站点 | 快照/过期响应防护、2400 点极值降采样、删除计划、图表销毁等已核实正确 | 空 metrics 静默漏站；预检一处失败中止整批；并发硬编码 6；指标列缺 KGE 分量；混基准标签 |

---

## 3. 缺陷总表

> 编号 **F-xx** 为本报告主编号，括号内为分报告编号。**confirmed** = 读实现/测试或 Lead 实测可直接证明；**suspected** = 机制成立但触发条件/实机后果未闭环。

### 3.1 P1（2 条，建议本批修复）

#### F-01（A-04）`_r8` 源码字面量被当作可写默认值：GUI 展示 → 接受 → 写回后配置文件不可读【confirmed】

- **证据**：`crates/colm-schema/src/generated.rs` 有 110 个 Real 字段默认值带 Fortran kind 后缀（如 `SITE_lon_location=-1.e36_r8`、`DEF_BALL_BERRY_GRADM=-1.0_r8`）；`gui/src-tauri/src/config.rs:93-104` 的 `default_literal`（经 `:106-133` 的 `describe_fields` 返回）原样返回；`gui/src/features/cases/CasesPage.tsx:508` 用它预填可编辑输入框；`case-model.ts:86` 的 `displayLiteral` 只剥引号不剥 `_r8`；用户把它改一个数字再保存时，`config.rs:1062` 的 `parse_real` 按 `_` 切分**接受**，`typed_scalar` 的 Real 分支 `config.rs:4923-4934` 保留原文写成 `Value::Real{text}`；`colm-namelist` 的 Display 原样输出（`value.rs:142`），于是 `case.nml` 里出现 `-1.0_r8`。而仓库自己的 namelist 解析器 `parse.rs:163-190` 对 `-1.0_r8` 报 `unrecognised value`，读回即失败。
- **Lead 实测**（gfortran 16.1.0，与本机内核同工具链）：最小 namelist 读取程序对 `a = -1.0` 返回 `iostat=0, a=-1.0`；对 `a = -1.0_r8` 与 `a = -1._r8` 均返回 `iostat=-1`（读失败）。`&end` 终止符则返回 `iostat=0`（见 F-25）。
- **影响**：用户编辑一个预填的专家实数（面板上显示 `-1.0_r8` 这种值）并保存后，算例变成 GUI 打不开、内核也读不进的配置；写入路径还通过了后端校验，备份/回滚不触发。手册附录 B 也有 153 行 `_r8` 默认值，进一步把人往这个写法引导。
- **修法建议**：给 `Field` 增加 `nml_default`（和 `source_default` 分开），`describe_fields` 返回 namelist 合法形式（剥 `_r8/_r4`），`typed_scalar` 对 Real 也拒绝非 namelist 合法形式（不要用 `split('_')` 兜）；加测试：①全部 964 字段 `default` 能被 `colm_namelist::parse` 解析成同类型；②`describe_fields` 默认值不含 `_r8`。同步更新 `gui/src-tauri/src/config_tests.rs:98`（该测试当前**锁定**了 `-1.0_r8`）、重生成附录 B。

#### F-02（E-02）多站比较中 `metrics` 返回空数组的站点被静默丢弃【confirmed】

- **证据**：前端 `ResultPage.tsx:244-253` 以 `pairVars:null, summaryOnly:true` 调 `metrics`；后端 `crates/colm-cli/src/main.rs:5626-5644` 在没有任何可用评估变量时 `selected` 为空，最终返回 `[]` 且退出码 0；`splitSettledComparison`（`ResultsPage.tsx:274-300`）把 `{value: []}` 当成功，`rows.push(...[])` 不加行，也不加 failures。
- **影响**：90 站比较里某站观测文件填错/缺标准变量时，该站同时从指标表、失败区、CSV 消失；页面不显示「N 站里少了一站」，用户会认为全部站点都评估了。单站路径没有此问题（先取 `evaluation_catalog` 发布可用性原因），两条路径语义不一致。
- **修法建议**：`comparisonMetricsFor` 在 `rows.length===0` 时抛出带双语原因的错（或先取 `evaluation_catalog`），由 failures 展示；`splitSettledComparison` 对空 `value` 生成一条 `{error:'没有可配对的变量'}`；补前端测试（空数组 → 失败行）。

### 3.2 P2（16 条）

| ID | 来源 | 缺陷 | 关键证据 | 修法 |
|---|---|---|---|---|
| F-03 | B-01 | **材料族不随上游选择自动关闭**，违反 2026-10-15 约定：BGC 被自动关闭时不停 CH4；河湖汇流被自动关闭时不停泥沙/CH4-routing → 创建/保存被后端以互斥错误拒绝 | `gui/src/features/cases/model-settings.ts:373-536`（`exclusionPatch` 无这两条路径）；后端 `gui/src-tauri/src/config.rs:1710-1730,1798-1819` 有校验 | 固定点循环补 `!BGC → closeMaterials` 与 `!GridRiverLakeFlow → 关泥沙/对应甲烷方案`，复用 `autoClosedText`；补 `model-settings.test.ts` |
| F-04 | D-01 | **histmap 闸门表漏 14 个真实写出变量**：`xtask/src/main.rs:551-568` 的 `gen_histmap` 只扫 `MOD_Hist.F90`+甲烷，漏 `MOD_Catch_Hist.F90`（xsubs_bsn/xsubs_hru/wdsrf_hru/veloc_hru）、`MOD_Grid_RiverLakeHist.F90`（volresv/qresv_in/qresv_out）、`MOD_MT_Sediment.F90`（sedcon/sedout/bedout/sedinp/netflw/sedlayer/shearvel）。586 个开关中 31 个无 writer 条目，其中 14 个确有写出点 | `crates/colm-hist/src/generated.rs`（724 VARS / 586 SWITCHES，31 开关无匹配；Lead 复核 `xsubs_bsn`/`sedcon` 等写出点）；`gui/src-tauri/src/histvars.rs:86-90` 判 `writable=None`；`HistoryVariablesCard.tsx:92` 把不可写项从「全部变量」过滤掉 | 扩展 `xtask/src/hist.rs` 提取 HYDRO/泥沙写点并重生成 + drift；在完成前允许 GUI 勾选并标「由其他源文件写出、尚未纳入闸门表」；补流域/泥沙 history 的 oracle 对账 |
| F-05 | B-03/D-03 | **新建向导输出变量页把 586 个开关全部标为可写**（`settable:true`），且「尚未按编译配置核对」提示只在 `defaultOn` 时渲染、新建页从不传；用户勾选当前内核写不出的变量 → 静默写进 `history.nml`，创建后才发现不可写 | `gui/src/features/cases/NewCasePage.tsx:816-826,3661-3666`；`HistoryVariablesCard.tsx:105-111,235-241`；`simulation-page.test.ts:5224` 断言 `defaultOn === undefined`；后端 `config.rs:5996-6036 stage_history_file` 不查可写性 | 第 5 步已有 `configurationKernel`：按内核调 `hist_vars` 后传 `settable/reason`；内核未定时默认不可勾 + 恒显示「未核对」提示 |
| F-06 | A-03 | **catalog 的 `storage`/`write_strategy` 把 639 个描述符说成写 `case.nml`**，实际 forcing→`forcing.nml`、history→`history.nml`、isotope→参数文件 | `crates/colm-case/src/parameters/mod.rs:252,287-291` 对 `field.group.is_some()` 一刀切；1008 case-nml 中 `nl_colm_forcing` 35 + `nl_colm_history` 586 + `nl_colm_isotope_parameter` 18 并不落 case.nml | 按 group 映射 storage，新增 Forcing/History 枚举；`write_strategy` 同步；加「descriptor.storage 与 schema group 一致」测试。当前 Study 只采 `calibration_eligible`（均 Real/LCT）故未爆，但元数据是错的 |
| F-07 | B-02 | **新建向导时段校验弱于已有算例**：只判 `form.end < form.start`，起止同日可过；`casePeriodValid`（严格晚于、含秒）只有 `CasesPage` 用；后端 `validate_runtime_contract` 无起止先后校验 → 可建出零长度窗口算例 | `gui/src/features/cases/simulation-model.ts:549-554` vs `:268-276`；`CasesPage.tsx:646-660`；后端无对应校验 | `newCaseProblem` 的 step≥3 改用与 `casePeriodValid` 同口径（抽接收 start/end/sec 的纯函数）；补测试 |
| F-08 | C-01 | **GUI 显式线程数被 `RAYON_NUM_THREADS` 静默覆盖**；批量还按环境值算 CPU 预算/并发宽度 → 界面显示的预算与实际不符，用户无法从界面纠正 | `gui/src-tauri/src/sidecar.rs:2574-2577`（`filter(|_| env is none())`）；批量 `:2383-2399`；`cpu-parallelism.md` 要求显式设置保留语义 | 显式运行设置永远覆盖继承值；若坚持环境优先，则在运行页/`run://started`/provenance 显示最终生效值（并解决 F-11） |
| F-09 | C-02 | **运行登记键不做路径规范化**：`/p/case`、`/p/case/`、符号链接别名是不同 key，可同时通过 `prepare` → 两个 `colm-cli run` 同时写同一 `out/`（history/restart 可能混写）。同文件 `creation_key`/`study_process_key` 都做了规范化，说明是登记层漏做 | `sidecar.rs:449-471`（`reserve` 原样存 key）vs `:63-95`、study key；正常 GUI 路径传入扫描后的稳定字符串，故界面常规路径未证实可触发 | 登记/取消/查询统一先 canonicalize（复用 `creation_key` 的逐级解析，处理不存在目录） |
| F-10 | C-03 | **运行前 `model_migrate_case` 未登记长任务**：停止按钮/关窗不能终止在途迁移；首次对新修订 `cache_code` 会 `git clone --local` 源码（秒~分钟级），违反 CLAUDE.md「长任务必须登记」 | `RunPanel.tsx:740-760` → `model.rs:50-53` `with_model_activity` → `sidecar.rs:5040-5041,5066-5069` `capture_untracked`；`model_version.rs:236-250` 迁移路径会 `cache_code`（`model_version.rs:881-911` git clone） | 迁移走 `register_child`/`ChildOwner::Preparation` 级封装，停止时按算例取消；或纳入 `RunProcesses.prepare` 生命周期 |
| F-11 | C-04 + Lead | **本机运行 provenance 缺实际生效并行信息**：`produced-by.json` 只有 ranks，无 threads/`RAYON_NUM_THREADS`/`COLM_MPIEXEC`/launcher；GUI `run://started` 事件也不带 threads。远程有 `parallel_plan`，本地没有 → 同一算例两次不同并行度不可区分，也掩盖 F-08 | `crates/colm-cli/src/provenance.rs:29-40,82-115`；`gui/src-tauri/src/history.rs` 的 `command_event` detail 仅 action/engine/preprocessors/kernel/stage/ranks/force；`history.rs:1530` 附近 | CLI 增加显式 `--threads`（或记录 env 实际值）并写入 `Record`；`command_event` 补 threads/launcher；GUI 显示 |
| F-12 | C-07 | **远程显式拒绝 `--highres-params`/`--soil-hyper-albedo-dir`**（本地可跑），GUI 无入口/无提前提示；远程提交在 staging 阶段报英文错误 → 本地/远程选项唯一实质不等价 | `crates/colm-cli/src/remote_cmd.rs:2360-2366`；`gui/src-tauri/src/{sidecar.rs:2005-2019, remote.rs:1728-1755}` 无这两个参数 | 远程运行页对物化 run 预检这两个 option 并给双语说明/映射入口；或 GUI 支持显式映射 |
| F-13 | C-08 | **引擎×内核×前处理组合校验只在 CLI 启动后**：`run_case` 只校验 threads/ranks 数值；`ranks>1 需 USEMPI`、Rust CROP + Fortran 前处理等检查在 `main.rs:4007` 等，GUI 已显示「已开始」后才失败；批量 90 站逐站失败 | `sidecar.rs:2549-2560`；`main.rs:3952,3971-3983,4007`；`run-model.ts:126-158` 只按网格/地类/CROP 挑内核 | GUI `compatibleKernel` 旁增加 USEMPI/CROP 组合预检，或新增 `preflight_run` 命令复用 CLI 同一组检查 |
| F-14 | E-01 | **多站比较预检一处失败即中止整批**：任一「未登记运行副本」缺失就 throw，此时一条指标都未算；其余站点无结果、无 failures、无重试 | `ResultsPage.tsx:1211-1221`（throw）、`:218-233`（`model_case_runs` 失败被 `.catch(()=>[])` 吞） | 把出处解析并入逐项 settled（缺失条目→`ComparisonFailure`）；只有结构性参数错误才池前中止 |
| F-15 | E-04 | **多站比较并发池硬编码 6**，未按 plan「默认 `min(4, logical CPUs)`」；本机 2 核会同时起 6 个 metrics sidecar | `ResultsPage.tsx:1234-1237,1304-1307`；`series.ts:118-146` | 本机 `min(4, navigator.hardwareConcurrency||4)`，远程按会话上限单独给值并注释；测试断言 |
| F-16 | E-05 | **指标表/比较 CSV 缺 KGE r/α/β 分量与模型/观测均值、标准差**（后端已返回），plan 明确要求；KGE 只能看一个数，无法判断是相关还是偏差拖累 | `ResultsPage.tsx:3200-3217`；`analysis-state.ts:133-154`；后端 `crates/colm-cli/src/main.rs:5938-5942`；`series.ts:151-165` 已声明 alpha/beta | 表加 α/β（可选折叠均值/标准差），CSV 同步；无后端改动 |
| F-17 | E-06 | **混基准比较的时间基准标签取「当前打开站点」**：条件摘要显示单值基准，参与比较的运行可能是 UTC 格点 + 地方标准时单点混排；CSV 已逐行按 `metric.time_basis`，UI 标签仍是单值 | `ResultsPage.tsx:651-661,3175-3178` | 条件摘要改为逐运行基准列表或明确提示「各运行基准见结果行/CSV」；比较时预取各 run 的 time_basis |
| F-18 | Lead（新增） | **字段可见性判定不感知运行引擎**：`VisibilityContext` 没有 engine，`rust_unported()` 在「未改动」时一律 hidden。因此 Fortran 专属可写字段（如 `USE_SITE_urban_*`、空间 `DEF_HIST_mode=block`、`DEF_HIST_WriteBack`、`DEF_HIST_grid_as_model_mesh`）在用户选 Fortran 引擎时仍不可发现；只有导入算例已带非默认值才显示 | `gui/src-tauri/src/config.rs:1001-1040`（无 engine 字段）、`:2003-2012`（`rust_unported`）、`:2171+`；`CasesPage.tsx:822-826` 已有 engine；`simulation-model.ts:405-431` 仅 step2 用 engine | 把引擎纳入 `field_states`/`model_configuration` 输入，Rust 专属限制在 Fortran 引擎下解除；同时复核 `rust_unported` 里已过期的「Rust 尚未移植」条目（CROP 合并后部分已不成立） |

### 3.3 P3（28 条，按主题合并）

**文档/元数据漂移**

| ID | 来源 | 缺陷 | 证据 | 修法 |
|---|---|---|---|---|
| F-19 | A-01 | `docs/parameter-catalog.md` 仍写 catalog version 1 / 1220 descriptors / 876 case-nml / 170 process；实际 v2 / 1369 / 1008 / 187 | `docs/parameter-catalog.md:18` vs `crates/colm-case/src/parameters/mod.rs:8`、`artifacts/parameter-audit/report.md` | 用 audit 产物更新；加文档数字与代码对账测试 |
| F-20 | A-02 | catalog `source_location` 对 39 个 MaterialTransport 字段写成 `MOD_Namelist.F90`，实为 `MOD_MT_Namelist.F90`（`generated.rs` 的 line 来自 MT 文件） | `crates/colm-case/src/parameters/mod.rs:231,266-271`；例 `DEF_USE_ISOTOPE` = MT 文件 line 42 | `Field` 加 `source_file`，生成器写入；重生成 artifacts 与附录 B |
| F-21 | A-05 | 派生字段（group=None）写入口不一致：`reset_field_batch` 拒绝，`set_field_batch`/`apply_fields` 却可写；写入值被内核忽略且 reset 清不掉 | `config.rs:5779-5782` vs `put` `:4960-4992`、`set_field_batch` `:5802+` | `put` 对 group=None 直接拒绝（保留 `DEF_USE_CROP` 白名单） |
| F-22 | A-06 | 10 个派生字段的「派生理由」过期：`DEF_USE_ESACCI` 在全 vendor 只有声明与广播、无任何赋值（上游缺陷，选项不可达）；`DEF_dir_*` 覆盖行号写 1406，实为 1843-1845 | `vendor/CoLM202X` grep；`crates/colm-schema/src/field.rs:55-60`、`config.rs:76-80` | 修正注释/理由；把 `DEF_USE_ESACCI` 记入 `docs/upstream-bugs.md` |
| F-23 | A-07 | 代码注释里「实测 N 个」数字随上游更新漂移（713/108/6/30/68 → 964/154/10/31/34 等） | `crates/colm-schema/src/field.rs:25,55-59,78,93-96`；`source.rs:8`；`gui/src-tauri/src/config.rs:78,127` | 注释只留判据，数字移入 audit 产物或用测试钉住 |
| F-24 | A-08 | `parameter-audit` 的 `unclassified=0` 只证明 catalog 分类，不证明 Fortran namelist → schema 覆盖：`DEF_rawdata_nml` 容器与 87 个 PFT include 键不在 schema 也未登记排除；手册附录 A 称「覆盖全部字段」 | `xtask/src/main.rs:104-430`；`docs/user-manual/appendices/appendix-config-fields.md:3` | audit 增加双向差集检查；文档写明排除项 |
| F-25 | A-09 | namelist 解析器拒绝 Fortran 合法终止符 `&end`，且报错误导为「组内又开了一个组」；导入的手写 `.nml` 会打不开 | `crates/colm-namelist/src/parse.rs:38-53`；Lead 实测 gfortran 接受 `&end`（iostat=0），仓库解析器拒绝 | 把 `&end` 与 `/` 同义处理；加 parse/roundtrip 负向用例 |
| F-26 | A-10 | 手册附录 A 的「类型」列输出 Rust Debug 形态（`Character { len: 256 }`，70 行） | `gui/src-tauri/src/config.rs:110`；附录 A | 返回 namelist 风格类型名，重生成附录 |
| F-27 | A-11 | catalog `visibility` 与运行时 `field_runtime_state` 是两套可见性元数据（586 history 开关在 catalog 里 editable-common，实际归输出变量页；材料字段在 catalog 里 common，无材料时 hidden） | `crates/colm-case/src/parameters/mod.rs:255-263` vs `config.rs:2171-2849` | 明确 catalog visibility 为「静态可见性」并改 docs，或统一规则表 |

**GUI 交互/状态细节**

| ID | 来源 | 缺陷 | 证据 | 修法 |
|---|---|---|---|---|
| F-28 | B-04 | 新建向导缺 CBL/臭氧入口，已有算例步骤 3 有（`configure_cbl_batch`/`configure_ozone_batch`） | `CasesPage.tsx:1510-1575`；NewCasePage 全文无匹配；手册 6.3 列为关键字段 | 向导复用同一对按钮，或明确提示建后配置且文案一致 |
| F-29 | B-05 | 新建向导在第 3 步校验第 5 步才显示的时间步长；草稿/seed 带空串时报「时间步长须…」而界面无该项 | `simulation-model.ts:549-564` vs `NewCasePage.tsx:2660-2690` | 校验挪到 `problem(5)` 或对隐藏字段跳过 |
| F-30 | B-06 | `exclusionPatch` 的 `touched()` 把显式删除（null）当未触碰，互斥对可能关错一方（suspected，现有调用点未构造出该 patch） | `model-settings.ts:387-389,432-435` | `touched` 区分「写值/显式删除/未提及」三态 |
| F-31 | B-07 | `historySwitches`/`optionStates` 请求失败时保留上一修订结果（切换修订/引擎后失败 → 页面仍显示旧解析值） | `NewCasePage.tsx:869-892`；对比 `ScientificSetupCards` 的 `loaded.key` 新鲜度保护 | 请求开始/失败时重置，或用请求 key 比较后再应用 |
| F-32 | B-08 | `serializeCaseRunSettings` 按插入序而非最近使用淘汰（更新已存在目录不会移到末尾） | `gui/src/lib/case-run-settings.ts:44-52` | 写入前删 key 再放末尾，或按时间戳排序 |
| F-33 | B-09 | 选 PFT/PC 时把已有 USGS 选择静默改写为 IGBP，无提示（与准绳一致但缺说明） | `NewCasePage.tsx:1629-1634`；`ModelSettings.tsx:404-417` | 切换时补一条提示「PFT/PC 新建固定使用 IGBP 地类数据」 |

**运行链细节**

| ID | 来源 | 缺陷 | 证据 | 修法 |
|---|---|---|---|---|
| F-34 | C-05 | 运行链后端校验错误全英文（`mpiRanks must be…`、`threads must be…`），前端未按固定文字映射双语 | `sidecar.rs:1949-1950,1987-1999`；`gui/src` 无对应映射 | 后端改 `bilingual` 或前端加映射；`failure_reason` 加「内核失败原因（原文）」前缀 |
| F-35 | C-06 | 本机 Rust 引擎 `--ranks>1` 无 GUI 入口（`submitRanks` 恒 1），CLI 支持但界面不说明「本机单进程」；能力与文案不一致 | `gui/src/features/runs/run-model.ts:105-116`；`RunPanel.tsx:1633` | 运行页对本地 Rust 显式说明；或支持 `COLM_MPIEXEC` 后解锁 |
| F-36 | C-09 | 批量准备阶段逐站串行迁移；该阶段无已登记进程，停止语义只是前端不继续下一站（与 F-10 同根） | `RunPanel.tsx:735-770` | 随 F-10 一起解决 |
| F-37 | C-10 | 前端 threads 上界 256、后端 4096；错误语言不一致 | `RunPanel.tsx:682-690`；`sidecar.rs:2554-2556` | 收敛到同一常量 + 双语 |
| F-38 | C-11 | 远程运行不受 `refuse_writes_into_running_cases` 保护（注释明说远程不拦）；`mark_results_stale` 只在前端白名单里、生产代码无调用（死入口） | `sidecar.rs:739-765`；`gui/src/lib/ipc.ts:53-71`；`command-contract.ts` | 删除死白名单项；远程写保护单独设计；非 Colm 阶段重跑也置 stale |

**输出/结果层细节**

| ID | 来源 | 缺陷 | 证据 | 修法 |
|---|---|---|---|---|
| F-39 | D-02 | `hist_vars` 甲烷块去重键用输出变量名比开关名（34 处不相等：一名多变量、DA_* 前缀、大小写），`DEF_USE_METHANE` 打开时多出 35 条只读行并套错 CH4 原因，「另有 N 个写不出」计数虚高 | `gui/src-tauri/src/histvars.rs:146-154,201-228` | 循环改 `if v.switch.is_some() { continue; }`；`methane_state` 只对含 `DEF_USE_METHANE` 条件的变量调用 |
| F-40 | D-04 | `DEF_HIST_CompressLevel` 对 Rust 单点 history 不生效（上游单点本就不压缩，逐字节一致，不是数值 bug），但 GUI 高级选项仍展示 → 单点改它无效的误导 | `crates/colm-hist/src/history.rs:315,1003-1075`；`MOD_HistSingle.F90:241` | 单点算例隐藏该项或注明「仅空间/向量历史」 |
| F-41 | E-03 | 结果页「报告/导出」分栏不可达（用户 2026-10-28 要求从界面移除、保留内部 tab），但 `guide.ts` 仍把 `result-export` 作为助手可用页 → 助手执行必抛错；plan 里第七分栏/PNG 快照入口与实现不一致 | `ResultsPage.tsx:350,370,1654-1662,3388+`；`guide.ts:169,227,282-289` | 二选一：恢复页签/入口，或删除 `report` 死分支与 `result-export` 别名；PNG 快照另补按钮 |
| F-42 | E-07 | 序列缓存是 FIFO 不是 LRU；重跑成功不清缓存（现代 `runs/<id>` 路径 key 含 source 通常不串，legacy/重放路径存在陈旧数据可见风险） | `ResultsPage.tsx:1006-1018,754-780`；`model.rs:1005` | 命中时删后重插实现 LRU；完成事件清缓存或把 mtime/文件数纳入 key |
| F-43 | E-08 | 多站观测没有 `site_file → case dir` 自动映射；逐行观测要手填（4 站可忍受，90 站不可行） | `ResultsPage.tsx:1585-1593,611,705`；单站有 `defaultObservation` | 勾选运行/进入多站时按 case_dir 预填 `defaultObservation`，手改优先 |
| F-44 | E-09 | 自动评估失败后不重试（`automaticEvaluation` 先占位后调用，失败不清回；条件不变时不再触发） | `ResultsPage.tsx:1147-1161` | 失败清 ref 或记录失败 key 并给「重试」按钮 |
| F-45 | E-10 | 比较页时间尺度下拉缺 `Year/3Hour/6Hour/8Day`，而解析器/OpenBench 接受；手改 YAML 能跑但 UI 不可选 | `ComparePage.tsx:2455-2476`；`compare/config.ts:36-47` | 按 OpenBench 值域全量列出，hint 解释 Year 与 climatology 区别 |
| F-46 | E-11 | 三处双语漏写：助手字段标签硬编码英文、报告 `notice` 纯英文、未识别的后端 warning 在英文界面原样中文 | `ResultsPage.tsx:1755-1763,1396`；`features/results/result-overview.ts:116-163` | 分别改 `tr/bilingual`；对未知 warning 标注语言并保留原文 |

**测试环境说明（非仓库缺陷）**

| ID | 说明 |
|---|---|
| F-47 | `cargo test --manifest-path gui/src-tauri/Cargo.toml` 在本会话为 669 passed / 8 failed / 1 ignored；7 条 sidecar 进程测试失败均因本机沙箱禁止 `ps`（`/bin/ps: Operation not permitted`，与直接跑 ps rc=126 一致），与本仓库逻辑无关；`model::tests::model_worker_lease_…` 在整套并行下失败、单跑通过，属测试隔离性问题，建议在有 `ps` 权限的环境复跑确认。 |

---

## 4. 基线验证（Lead 实际跑过）

| 命令 | 结果 |
|---|---|
| `cd gui && npm test` | **1776 passed / 0 failed**（含 `check-gui` 自检、完整契约检查、全部 `*.test.ts`） |
| `cd gui && npm run typecheck && npm run lint && npm run format:check` | 全部通过 |
| `cargo run -q -p xtask -- check-gui` | 268 个已注册命令、234 个被调用（26 个经动态名解析），事件 emitter/listener 配对全部通过 |
| `cargo run -q -p xtask -- parameter-audit` | 写入 1369 descriptors；`git status --porcelain artifacts/` 为空（无漂移） |
| `cargo test -p colm-schema --test drift` | 1 passed |
| `cargo test -p colm-namelist --test roundtrip` | 5 passed |
| `cargo test -p colm-hist --test drift` | 1 passed |
| `cargo test -p oracle --test histmap` | 5 passed |
| `cargo test -p colm-case -p colm-hist -p colm-cli --lib` | 101 passed |
| `cargo test --manifest-path gui/src-tauri/Cargo.toml` | 669 passed / 8 failed / 1 ignored（7 条为沙箱 `ps` 限制，1 条单跑通过；见 F-47） |

**Lead 独立复核**（不依赖审计员结论）：

- **F-01**：gfortran 16.1.0 最小 namelist 读取程序实测 `-1.0` 可读（iostat=0），`-1.0_r8`/`-1._r8` 不可读（iostat=-1）；仓库 `parse.rs` 同样拒绝 `_r8`；写路径 `typed_scalar` 却接受并原样保留 → 自相矛盾成立。
- **F-25**：同一 gfortran 程序实测 `&end` 终止符合法（iostat=0），仓库解析器报「group opened inside a group」。
- **F-04**：脚本核对 586 个 schema 开关中 31 个在 `colm-hist::generated::VARS` 无 writer 条目；逐文件核实 `MOD_Catch_Hist.F90:273+`、`MOD_Grid_RiverLakeHist.F90:585+`、`MOD_MT_Sediment.F90:4422-4484` 确有写出点（另 17 个为注释掉/无写出点的死开关）。
- **F-19**：`docs/parameter-catalog.md:18`（v1/1220）vs `parameters/mod.rs:8`（v2）与 `artifacts/parameter-audit/report.md`（1369）不符。
- **手册附录完备性（负结果）**：用脚本比较 `appendix-config-fields.md` 964 行、`appendix-tuning-parameters.md` 1369 行与生成表/产物，**逐 ID 完全一致、无缺漏**；文档问题只在 F-19/F-20/F-26 描述的元数据与类型格式。

---

## 5. 已核实无问题的关键点（防漏报/防重复排查）

1. **选项主链无静默丢弃**：运行链 24 项 GUI → invoke → argv/env → 内核逐项对上；C 报告矩阵齐全。
2. **失败判定**：`colm-kernel::outcome`（失败标记 + 成功标记 + 产物）由 CLI 每段执行；远程作业脚本 `set -e` + `COLM_JOB_EXIT_CODE` 传递 CLI 退出码；本地/远程 §2.4 均已闭合。
3. **取消/关窗 owner 覆盖**：本机运行、批量、建例、研究、远程提交/编引擎/取回、数据准备、历史重跑、历史同步、OpenBench、助手均有 owner，未再发现 G-04/P1-2 类缺口（F-10 是迁移这一条漏网）。
4. **写盘安全**：`case_data_save`/`set_fields_*` 先全改完再落盘、staged + 备份 + 回滚、内容未变跳过写；`case_data_save` 提交前重读快照，期间变化即拒。
5. **默认值保全**：读不 materialize 默认值；reset 只删显式覆盖；history 开关与引擎共用 `colm_hist::selection` 判定。
6. **输出数据层守卫**：缺测/`spval`、UTC/local/model clock 时间基准随行返回、频率标签反推与冲突警告、全量统计 vs 抽稀展示、2400 点降采样保留极值、删除的 plan+token+quarantine+inode 校验、NetCDF 只经 sidecar。
7. **前端结果状态**：快照/导出闸门、过期异步响应防护、离开分栏销毁图表、多站点单站失败隔离（除 F-02/F-14 两个例外）已实现。
8. **生成表漂移**：schema/histmap/landcover/co2 四张表由 drift 测试逐字节守住，本次全部通过。

---

## 6. 修复方案（分批，按风险/价值排序，**等待批准后动手**）

> 约束：涉及生成表的改动必须改上游 Fortran 或 `xtask` 生成器后重新生成，不能手改产物；每个批次按 CLAUDE.md 跑对应层的验证（`cargo test --workspace --lib --bins` + clippy + fmt；GUI 侧 `npm run typecheck/lint/format:check/test` + `cargo test --manifest-path gui/src-tauri/Cargo.toml`；改 `#[tauri::command]` 再 `npm run contract` + `check-gui`）。

### 批次 1（P1，建议立即做，1–2 个提交）

- **F-01**：`Field` 增加 namelist 合法默认值（或 `default_literal` 清洗 + `typed` 拒绝 `_r8`）；改 `config_tests.rs:98` 这条锁定测试；加「全部默认值可被本仓库 parser 解析」测试；重生成附录 B（及受影响的 A）。
  - 文件：`crates/colm-schema/src/{field,lib,generated}.rs`（生成器侧）、`gui/src-tauri/src/config.rs`、`gui/src-tauri/src/config_tests.rs`、`xtask/src/*`、`docs/user-manual/appendices/*`。
  - 风险：中（生成表 + 附录重生成）；但这是唯一会**写坏配置**的缺陷。
- **F-02**：`comparisonMetricsFor` 空数组 → 逐站失败；`splitSettledComparison` 空 value 生成 failure；前端测试。
  - 文件：`gui/src/features/results/ResultsPage.tsx`、相关 `*.test.ts`。
  - 风险：低。

### 批次 2（输出变量完备性，与批次 1 可并行）

- **F-04**：`xtask/src/hist.rs` 扩展解析 `MOD_Catch_Hist.F90`/`MOD_Grid_RiverLakeHist.F90`/`MOD_MT_Sediment.F90` 的写出点，重生成 `colm-hist` 闸门表 + drift + oracle 对账；若生成器暂难覆盖，先把 31 个开关标「其他源文件/无写出点」并允许勾选。
- **F-05**：新建向导第 5 步按 `configurationKernel` 调 `hist_vars`，卡片传 `settable/reason`；未定时默认不可勾并恒显「未核对」提示。
- **F-39**：`histvars.rs` 甲烷块去重键改用 `v.switch.is_none()`。
- **F-06**：catalog group→storage 映射 + 测试；重生成 artifacts/附录 B。
- **F-21/F-25**：`put` 拒绝派生字段；parser 接受 `&end`（+ 测试）。
- 风险：中（生成表/产物重生成），可与批次 1 合并验证。

### 批次 3（GUI 配置流程）

- **F-03**（exclusionPatch 材料自动关闭 + 测试）、**F-07**（时段校验统一）、**F-28/F-29/F-31/F-32/F-33**（小修）、**F-18**（引擎感知可见性——需先确认产品语义：Fortran 用户是否应看到 Rust 未移植选项；可能要和 `model_configuration` 接口一起改）。
- 风险：中；F-18 涉及接口扩展，建议先出小设计。

### 批次 4（运行链）

- **F-08**（线程优先级 + 记录）、**F-10**（迁移登记）、**F-09**（登记键规范化）、**F-11**（provenance threads/launcher）、**F-13**（预检）、**F-12**（远程 highres 预检/文档）、**F-34/F-35/F-36/F-37/F-38**。
- 风险：中；F-09/F-10 涉及进程登记与取消语义，需配 sidecar 测试；F-34 涉及文案，需双语。

### 批次 5（结果页/比较页）

- **F-14/F-15/F-16/F-17/F-41/F-42/F-43/F-44/F-45/F-46**。
- 风险：低–中（纯前端为主，F-41 需确认报告分栏是否恢复）。

### 批次 6（文档/注释/元数据，可穿插进行）

- **F-19/F-20/F-22/F-23/F-24/F-26/F-27/F-40/F-47**：更新文档与注释、加对账测试、在 `docs/upstream-bugs.md` 记录 `DEF_USE_ESACCI`。
- 风险：低。

**建议顺序**：批次 1 + 批次 2 →（验证通过后）批次 3/4/5 任选 → 批次 6 穿插。批次 1、2 都动生成表/产物，最好在同一个验证轮里跑全量 drift + `parameter-audit` + `check-gui`。

---

## 7. 未覆盖与限制

- 本轮**未实跑**真实算例/真实服务器作业：运行期后果（如 F-01 写坏后内核报错文本、F-10 迁移耗时、F-02 的真实 metrics 空返回场景）按代码路径与最小程序实测推断，报告中已标 confirmed/suspected；建议批次 1/2 修复时补一条真实单点算例回归。
- D（输出数据层）为部分覆盖：远程 cleanup 全部分支、tier0–3 逐项核对、`colm-history` 台账 schema 迁移未深挖。
- 未审：AI 助手工具链的行为面、OpenBench 比较页生成默认 `comparison.config` 的字段值、研究（Study）运行链、资源页/远程编内核链（除与运行登记交叉处）。
- 上游 `vendor/CoLM202X` 的 bug（如 `DEF_USE_ESACCI` 永不赋值）只记录不修改。
- `tmp/audit/*.md` 五份分报告为过程证据（gitignore），核心结论已并入本文；如需长期保留可分拆进 `docs/`。

---

## 附：分报告索引（过程证据）

| 文件 | 内容 |
|---|---|
| `tmp/audit/a-options-data.md` | 选项数据层：schema/namelist/catalog/default 保全/写入校验 |
| `tmp/audit/b-gui-flow.md` | GUI 配置流程：五步向导/模型配置/参数对话框 |
| `tmp/audit/c-run-chain.md` | 运行链：run_case/run_batch → colm-cli → 内核、本地/远程、取消、provenance |
| `tmp/audit/d-output-data.md` | 输出/历史数据层：闸门表、频率、catalog/series/metrics/删除 |
| `tmp/audit/e-results-ui.md` | 结果页/比较页/历史页 GUI 选项行为、缓存、多站点 |
