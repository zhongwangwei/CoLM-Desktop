# 参数调优逐模块审计（2026-09-09）

状态：本地逐模块审计、修复及功能链路验收完成，独立代码复核通过；本文件不代表已经发布。跨平台 GitHub CI 以对应 PR 检查为准。

审计基线为 `2ef65070764cb4224039b51bd039810a8560766f`（v0.2.0-beta.3）。
范围覆盖参数目录与写入、Study 创建与冻结、DE、评分、多站点、恢复、应用、GUI/Tauri 和导出。
空间模式仍为 early state、不建议使用，禁止参数调优和不确定性分析。
不修改 Fortran 物理内核、原始 rawdata/runtime、既有 golden 或 Release。

## 已复现的缺陷

| 问题 | 根因及处理 | 当前证据 |
|---|---|---|
| 合法的正数 log 子区间被拒绝 | 目录只检查部分作用域下界；按正值定义域判断标量、LCT 与 PFT 的 log 能力，仍要求实际采样下界严格为正 | `continuous_study_ranges_match_runtime_parameter_domains`、`tuning_accepts_positive_scalar_log_ranges`：RED → GREEN |
| 播种日期连续采样必然失败 | Fortran 的 real 字段实际上要求整数日；共享注册信息、范围校验与目录统一排除连续采样，保留手工整数编辑 | `tuning_rejects_integer_crop_calendar_before_sampling`、`integer_calendar_ranges_are_rejected_by_the_shared_study_validator`：RED → GREEN |
| 标量激活条件使用未解析的地类 | `SITE_landtype=-1` 未解析站点文件；复用解析路径并遵守显式值、`USE_SITE_landtype`、相应 IGBP/USGS rawdata 的优先级 | `scalar_activity_uses_the_same_landtype_source_as_the_kernel`：RED → GREEN；包括来源冲突与原文件不变 |
| LCT 作用域绕过过程激活条件 | 只有 case-scalar 调用已有气孔参数校验；现按实际参数注册复用校验，并检查 LCT 植物水力开关 | `lct_study_parameters_require_their_active_process`：RED → GREEN |
| 灌溉范围在预检通过、物化时才失败 | 持续时间下界未与固定模型时间步比较；提前在共享算例范围校验中拒绝 | `irrigation_study_ranges_respect_the_fixed_model_timestep`：RED → GREEN |
| 反射开关可能覆盖真实编译方案 | 把 namelist 的 `DEF_USE_USGS/CROP` 与 kernel macros 合并；现以 native 实际采用的宏为准 | 地类回归包含与宏相反的 `DEF_USE_USGS`；RED → GREEN |
| 新增 rawdata 地类解析未接受 native 可规范化经度 | 点栅格接口要求规范经度；解析处复现 `MOD_Utils` 的有限值检查、哨兵拒绝及经度规范化，不改变共用栅格 API | `scalar_activity_uses_the_same_landtype_source_as_the_kernel`：180.001/-539.999 与 NaN/-1e36 边界，RED → GREEN |
| 旧目录或无 provenance 的非法规格绕过新合同 | 目录升至版本 2；冻结校验先执行当前规格校验，再检查目录/文件身份 | `frozen_catalog_identity_is_verified_but_legacy_manifests_still_open`、`legacy_provenance_does_not_bypass_current_sampling_validation`：RED → GREEN |
| GUI 可以展示并尝试应用失效的 best ID | 直接相信 `state.best_member`；展示和应用共用可行非 baseline 候选检查 | `gui/tests/study-model.mjs`、`gui/tests/results.mjs`：RED → GREEN |
| 缺失困难时刻的模型候选可获得更好分数 | 每个候选独立删除无效模型点，改变配对集和 NRMSE 分母 | 四点/两点反例已复现；修复后按配对时间及观测哈希比较，runner 回归通过 |
| 不完整缓存仍算可行候选 | 缓存只做 JSON 反序列化，未验证冻结目标全集与身份 | 删除目标行、混入错误 site/period、缺失支持等缓存回归通过；apply/export 独立生产复核通过 |
| 不可能的指标可以获得负损失并被应用 | `objective_loss` 只检查有限值；现在共享入口检查指标定义域，保留效率指标合法负值和 Bias 符号，仅夹紧极小舍入越界 | `objective_metrics_reject_impossible_domains_but_allow_roundoff`：RED → GREEN；独立复核已复现旧缓存 R²=2 的应用绕过 |
| 有限大权重或大损失的均值溢出 | 在归一化之前求权重和/加权和；现在先缩放并归一化权重，再求非负有限损失的加权平均 | runner 的大权重、大损失和非法损失回归通过 |
| 验证失败、支持不匹配或部分缺失影响校准 | 校准/验证共用可比性错误，且部分验证行会重新归一化 | 分离校准与验证错误，任一验证目标不完整则整体验证分数为空；单元、CLI 及独立复核通过 |
| 已选定 DE 代可被重试改写结果 | 旧任务重排后不重放历史选择，best 与 population 不一致 | `study-retry`、`study-run --retry-failed` 以及 stale-success 共用闭代限制；CLI 回归与独立复核确认不重跑，失败分数清空 |
| 等分或中途恢复导致 patience 计数错误 | 等分替换被当成改善；恢复后把未选择的新代候选当作前代最优 | 保留等分选择，但改善要求真实增益；按已选种群比较，参数相关目标的 open-generation 暂停恢复回归通过 |
| 导出可覆盖无关目录 | 未检查输出目录归属即写 manifest/status 并删除 results | 旧实现失败回归已复现；新增同 Study 快照识别与受管符号链接拒绝，导出测试通过 |
| 导出绕过缓存验证并复制过期最佳候选 | 直接复制 checkpoint 和派生表；现在在触碰导出目录前复用调优结果验证/刷新，并在导出目录重建目标表 | `tuning_export_rebuilds_scores_and_rejects_invalid_cache_before_overwriting`：stale-best 与 task score=999 明确 RED → GREEN；保留已有快照及原 Study 派生表 |
| DE 报告只显示初代预算 | 使用创建时 `manifest.members.len()`，遗漏后续代 | 全预算报告回归通过 |
| CSV 未正确处理 CR/目标名称分隔符 | 文本列转义不完整 | failures.csv、metrics.csv 边界回归通过，源文件保持 LF |

## 数学与实现合同

### DE

本仓库实现自己的有界 DE/rand/1/bin：三个不同的 donor 不包含当前 target，差分突变后做 binomial crossover，至少一个维度来自 mutant；整代完成后再选择。trial 与自己的 parent 比较，等分替换是允许的，并不意味着“目标有改善”。这些与作者的算法说明一致；SciPy 的 API 默认值、维度倍数和取值限制不作为本仓库的数学不变量。[Price 与 Storn，1997 年作者文章（存档镜像）](https://jacobfilipp.com/DrDobbs/articles/DDJ/1997/9704/9704a/9704a.htm)

失败候选使用不可行状态（比较意义上的正无穷），不引入任意有限罚分。边界 clamp、log 参数的编码/解码和停止阈值是本仓库的显式实现选择；生产调度路径的证据必须与测试专用选择函数区分。

### 指标与缺测

- NRMSE 为 RMSE / **配对观测的样本标准差**；这是本项目的定义，不声称 NRMSE 存在唯一通用分母。
- NSE 使用观测离均差平方和作分母；常量观测时没有有限的 NSE。[hydroGOF NSE 文档](https://search.r-project.org/CRAN/refmans/hydroGOF/html/NSE.html)
- KGE 使用 2009 形式的相关系数、标准差比、均值比；不偷换为 2012/2021 变体。观测均值接近零时均值比敏感，不能把有限但极端的 KGE 自动当成实现错误。[hydroGOF KGE 文档](https://search.r-project.org/CRAN/refmans/hydroGOF/html/KGE.html)
- 本项目 R² 是 **Pearson r 的平方**，不是预测决定系数；r 为常量序列时未定义。平方会丢失相关方向，这一后果不通过静默改变指标定义来修复。[SciPy pearsonr 文档](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.pearsonr.html)

单次评价中成对去除缺测是常见约定；但这不保证不同候选可比。**本审计的比较合同推论**：同一 Study 的同站点、同目标、同时间窗口必须使用固定支持，不能因候选缺失难拟合时刻而改变评分问题。固定目标权重也不能替代固定时间支持。校准与验证窗口分离，验证结果不能反向参与校准 winner 的选择。

## 逐模块验收台账

| 模块 | 验收证据 |
|---|---|
| 参数目录、激活、写入 | 67 个 colm-case 测试通过；10 个 engine 测试通过；1220 个版本 2 目录条目重新生成；真实 CNFAC 应用逐字段读回一致 |
| Spec、预检、冻结输入 | 创建/冻结/非法窗口/缺目标/空间禁止回归包含在 201 个 CLI 单元测试内，全部通过 |
| DE、评分、持久化、重试 | 45 个 runner、7 个 science 单元测试及 11 个实际 CLI 集成测试通过；共享/独立真实模型与失败重试通过 |
| GUI/Tauri、最佳候选应用 | 全部 Node 测试、160 个 Tauri 测试与接口检查通过；后端缓存篡改应用被拒绝，独立生产复核通过 |
| 导出 | 15 个导出测试通过（包括缺预算/预算溢出不 panic、缓存刷新及旧快照保护）；4 个真实 Study 均导出成功并验证应用读回 |
| 真实 CoLM | 2 次独立目标运行、共享双站 18 个任务、两个独立 Study 各 9 个任务、失败重试场景 1 次注入失败及 9 个成功任务；共 48 次尝试，47 次成功，预期失败恢复成功 |
| CI 与静态检查 | workspace（不含 CLI）458 个测试、其他 38 个 CI 集成测试、5 个 native 脚本、runtime contract、tier-check 与两份 golden 自读通过；workspace/Tauri clippy（`-D warnings`）、format 全通过。CI 显式执行新增 CLI 集成测试，不再只构建 |

## 证据与限制

本地运行日志保存在 `tmp/tuning-audit-20260909/`，RED 为旧逻辑的实际断言失败，不是编译失败。正式交付以跟踪到仓库的回归测试和最终验收记录为准。

兼容性：版本 1 目录冻结的 Study 需要重新创建，旧目录和文件不删除；普通状态读取仍可用于检查历史记录。无 provenance 的合法历史规格仍可读取，但不绕过当前采样校验；缺少固定支持信息的旧调优缓存不能冒充新合同下的有效结果。既有 beta.3 Release 不变。

### 生产调度与真实模型分开验收

CLI 合成目标的输出由参数值决定，而不是由 member ID 决定：seed=11、范围 [0.4,0.6]、阈值 0.59 时，初代最大值约 0.570921，第一代 trial 到达 0.6 并改善目标。`patience=1` 下，在第一代部分 trial 完成时暂停再恢复，与连续运行的每代样本字节、最终种群、最优候选及 patience 计数完全一致。只回退 patience 修复的独立副本实际失败于“恢复停在第 1 代，而连续运行完成第 2 代”；只回退 stale-success guard 的副本也实际失败。测试覆盖生产 CLI 调度，不以测试专用 sphere 选择函数代替。

真实算例为 CN-Cng（2008 年 1 月）和 AT-Neu（2010 年 1 月），使用 beta.3 的只读默认站点内核和独立运行生成的合成 Qle 观测；调优参数为 CNFAC，DE population=4、generations=1。

| Study | 成功任务 | 最优校准目标 | 最优候选 |
|---|---:|---:|---|
| 共享 CN+AT | 18/18 | 0.03346664398890922 | m000004 |
| 独立 CN | 9/9 | 0.0012829215803747142 | m000004 |
| 独立 AT | 9/9 | 0.06565036639744373 | m000004 |
| CN 失败重试 | 9/9（另有一次预期失败） | 0.0012829215803747142 | m000004 |

每个 Study 都有 9 个已评分向量（含 baseline；manifest 中的 5 个初始成员不是完整 DE 预算），最终种群含 4 个可行候选。独立 Study 根目录及 ID 互异，任务只包含各自站点。模型均值随参数变化，CN 和 AT 的均值跨度分别约 36.4725、10.5511。最优值等于全部可行非 baseline 校准分数的最小值，另存参数与预览逐字段一致。两个原始算例共 6 个 `case.nml/forcing.nml/site.nc` 文件由独立验证者和主执行者复查 SHA-256，均未改变。

失败场景仅使用临时派生内核：同一冻结 wrapper 通过测试自有 marker 注入一次失败，移除 marker 后执行与 Release 哈希一致的真实程序。派生 manifest/程序在创建、失败和重试之间保持不变，不通过切换内核绕过冻结校验。

验证工具自身曾因输出目录约定、把初代 manifest 当完整预算、派生 wrapper 清单哈希未更新而中断；这些均按实际合同修复，没有修改引擎或放松断言来通过。最终复用第三次运行中已经成功的共享/独立分支，仅重跑修正后的失败重试分支。最终 PASS 证据为本地 `real-e2e-harness/real_tuning_e2e_summary.json`、`retry-only-testready-4.log` 和 `logs/ctun-12178-e55d36e8/` 下的各运行/导出/应用日志；`full-run-testready-3.log` 仅保留重试分支修复前的中断上下文，不是完整成功日志（路径均位于上述证据目录）。

**限制：**真实一代 DE 没有进一步改善初代最优值，不能声称保证收敛；验证集最优与校准最优同序，本次真实数据不能单独证明排序隔离，该合同由代码路径及定向回归证明。这是月级功能验收，不证明参数的科学适用性，不替代全年、跨气候、crop/PFT/PC 各方案或空间科学黄金验证。原有两份 golden 的自比对只证明读取/分类链路，不冒充新的真实模型黄金比较。本地运行平台为 macOS ARM；Windows/Linux 的新提交执行结果需看对应 CI。

### 最小复验命令

```sh
cargo test --workspace --lib --bins --exclude colm-cli
cargo test -p colm-cli --bin colm-cli -- --test-threads=1
cargo test -p colm-cli --test study_cli_tuning -- --test-threads=1
cargo test --manifest-path gui/src-tauri/Cargo.toml
for script in gui/tests/*.mjs; do node "$script"; done
cargo run -q -p xtask -- check-gui
cargo run -q -p xtask -- parameter-audit
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Tauri 的独立 clippy/format、CI 所列的 golden/roundtrip/drift/kernel-profile 等 38 个集成测试和 native 脚本也已执行通过。本次不增加依赖、优化器、数据库或通用框架，不修改 Fortran、既有 golden、参数审计 baseline 或已发布安装包。

首轮 PR CI 在 Windows 暴露一个测试期望错误：导出路径使用已有 `colm_kernel::manifest::absolute` 去除 `\\?\` 前缀，而断言直接使用 `canonicalize`。已让断言复用同一跨平台路径合同；未改变产品路径处理或跳过 Windows 测试。本地 15 个导出测试及 CLI clippy/format 复验通过，远端最终结果见 PR checks。
