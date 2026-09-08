# 不确定性分析审计（2026-09-08）

范围：OAT/LHS 参数不确定性工作流，从输入、采样、成员物化到调度、统计、GUI 和导出。
不把参数情景分位数解释成置信区间；不扩展 Sobol、MCMC 或新的调优算法。
main 与 `colm-destop-spatial` 独立发展，共用修复分别提交 PR，不合并两条分支历史。

> 后续范围更新：用户在本审计完成后明确授权 spatial 合入 main。原审计的独立分支记录保留作历史；整合版本的空间功能标记为 **early state、不建议使用**，空间模式禁用参数调优与不确定性分析，站点模式继续受回归验证保护。

## 算法依据

- [SciPy LatinHypercube](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.qmc.LatinHypercube.html)：每个维度的每个分层恰有一个样本；检查固定 seed、线性/对数映射及基准成员分离。
- [NumPy quantile](https://numpy.org/doc/stable/reference/generated/numpy.quantile.html)：采用 linear / Hyndman–Fan type 7；插值采用凸组合，避免两个有限极值作差溢出。
- [SciPy Spearman](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.spearmanr.html)：秩相关衡量单调关联，不是因果或方差贡献百分比；常量输入不提供有效相关系数。

## 已确认缺陷与修复

| 模块 | 缺陷 | 修复与验证 |
|---|---|---|
| 规格/预算 | 隐式 LHS 和 OAT 候选数绕过 1000 上限 | 共用 checked 预算计算；合法多 PFT 维度覆盖 1000 边界及 1010/1002 拒绝 |
| 数值归约 | 有限极值的分位数作差或均值累加产生 NaN/Inf | Type-7 凸组合、有限均值归约；实际 NetCDF 输入回归覆盖均值路径 |
| 原生路径/并发 | 256 字节 restart 截断使多个成员写入同一个文件 | 普通运行和成员物化共用路径上限校验，过长路径在启动前拒绝；短路径真实 jobs=2 一次通过 |
| 创建/GUI | 元数据、预检、创建和结果异步响应可串项目/设计，旧响应覆盖新状态 | scope/design/request guards；创建防重入；真实生产函数的延迟 IPC 回归 |
| GUI 诊断 | 后端支持不足等警告未展示，高失败率不可见 | 监控和结果页面直接显示；候选成员口径排除基准，多站点不重复计数；保留分 Study 警告来源 |
| 恢复 | retry 可在校验冻结输入/Study 身份前修改 checkpoint | 校验先于重排任务和清除控制标记；篡改输入、错误身份回归 |
| 恢复 | 遗留/复用 PID 被当成已确认调度器 | 无新鲜心跳则 NeedsReview；保留活 PID 以阻止误取消和重复运行 |
| 统计快照 | 成功任务结果损坏被吞掉、失败聚合残留上一轮统计 | 读取错误带上下文上报；重建前只清本工作流拥有的摘要，不删任务结果/history；更新失效警告 |
| 导出 | 遗留 Running 状态未协调；报告混淆执行失败、取消和待复核 | 复用状态协调；分别统计 execution-failed 与 non-success |
| 导出 | 无有效 checkpoint 仍可成功导出并混入旧快照状态 | 在任何导出目录写入前拒绝；保留旧快照与用户自建文件 |

## 逐模块覆盖

1. **规格与参数**：连续参数边界/尺度、站点适用性、PFT/PC/LCT scope、输出/时间窗、显式和隐式预算。拒绝无依据地把调优验证窗限制成必须连续。
2. **采样**：固定 seed、每维 LHS 分层、线性/对数映射、OAT 单维双端扰动、基准独立。
3. **物化与冻结**：原算例不变、私有 namelist/输出、PFT 1/2 的真实 NetCDF fixture 经过 `validate_spec → baseline/validate_case_parameters → design → member_case`，写入 Fortran slots 2/3，不误写 1/4。
4. **调度恢复**：并发任务、暂停/取消/重试、冻结输入、checkpoint 回退与身份；不通过强制删除锁或盲目重跑解决未知进程。
5. **统计**：成功且有限的成员、基准排除、时间轴、支持阈值、Type-7、Spearman/OAT 的解释；不把缺失结果补为零。
6. **GUI/Tauri**：当前项目/设计身份、最新请求、事件路由、按需读取、图表回收、警告可见性；保留现有 IPC，不引入新框架。
7. **导出**：冻结输入、状态协调、失败表、manifest/samples/results、输出路径边界、重复结果快照。不会因用户导出目录含额外文件就清空该目录。
8. **真实 CoLM**：实际 LHS 20 候选 + 1 基准，检查参数注入、原算例不变、基准排除、分位带和导出；见下方限制，不用伪内核测试冒充物理模型验证。

## 验证记录

本地原始证据：`tmp/uncertainty-audit-20260908/`（不入库的大体积日志与运行输出）；main 单独证据位于其 worktree 的 `tmp/main-backport-2026-09-08/uncertainty/`。

- 两条线分别运行 CLI 全单测（spatial 177 / main 176）、各 6 个 CLI 集成测试、workspace Clippy、fmt、GUI IPC 检查以及全部 11 个 Node 测试文件；Tauri Study 10 tests 通过。
- 回归包含实际 red/green：分位数、均值、隐式预算、GUI 旧响应/自清空选择/创建/残留节点/警告、缺失 checkpoint 导出、stale lock、缺失成功任务结果。
- 真实长路径：新 `study-create` 明确报 256-byte 错误并清理未完成目录；不启动模型；原共享碰撞文件 hash 未变。
- 真实短路径：固定复制的 CLI，LHS 20 候选 + 1 基准，`jobs=2` **一次成功 21/21**，没有串行重试；无 task_failed/NetCDF/HDF/Permission 错误，warnings 为空。
- 分层及注入：每层一个候选，所有成员参数与设计文件一致；基准 case/forcing hash 不变；envelope `members=20`、`n_eff=20`、stable=true，基准未混入分位数。
- 实际输出为 **48 个小时中点**，UTC 2008-01-01 00:30 到 2008-01-02 23:30，步长 3600 秒；不是半小时分辨率或一年模拟。
- 导出包含 manifest/samples/status/results/report，失败表只有表头，导出 envelope 与原结果相同。
- 第一轮反例和成功复验分别留在 `real-smoke/` 与 `/private/tmp/colm-uq-vfy093434-75590/`；复制二进制、源码 diff、内核 hash 均有记录。物理模型为发布的 Darwin-arm64 default kernel，不修改发布二进制、原始数据或 golden。

## 验证边界

- 本轮修复是拒绝不安全长路径，不是扩大 Fortran 的 256 字节缓冲区。长项目路径需缩短；不以强制串行掩盖文件碰撞。测试目录标签也缩短到支持范围，保留覆盖。
- 进程身份采取保守恢复：过期心跳转 NeedsReview，但活 PID 仍阻止删除锁、取消或双跑；没有实现跨平台 OS 级 executable/argv/start 身份探测。
- 真实运行覆盖单站点短时 LHS；多站点/PFT 由 fixture 验证，不代表多年、所有预设/平台、收敛性或参数可辨识性。OAT 有确定性采样/物化单测，未另跑真实 OAT。
- 科学数值等价性 golden 不在此烟测中宣称；GitHub CI 结果以对应 PR 的实际运行状态为准，不用历史绿灯代替当前提交。

## 排除的非必要修改

先前子代理对 `engine.rs` 添加“路径不存在时跳过 canonicalize”，并删除真实目录名称测试 fixture。操作日志确认其来自本会话，不是用户编辑；没有证实的业务故障，已撤销，保留原有严格路径解析。`engine.rs` 最终只调整测试临时目录标签，以适应已证实的 native 路径上限。
不添加重复 manifest 身份别名，不为旧草案中的字段命名制造协议迁移。

## 合入 main 后的目标完成复核

以 main `447eca2` 为基线重新核对全部八个验收领域，而非仅复核 spatial 合并差异。独立后端复核确认规格、采样、冻结、恢复、统计和导出符合上述合同；GUI 又发现并通过失败回归确认三处遗漏：

- **空间 NeedsReview 按钮误启用**：空间限制之后，冗余的 NeedsReview 条件重新启用了启动按钮。删除该条件，复用既有 `studyActionState`；站点恢复入口、暂停和取消不变。
- **图表诊断均值溢出**：带宽/中位数偏离的累加或相减可能先溢出，即使最终均值可表示。仅在溢出时先缩放再累加；确实不可表示的诊断返回 null，不展示 Infinity，也不改变普通数值结果。
- **结果读取失败被隐藏**：已列出的 JSON/CSV 读取失败或 JSON 损坏曾被当成“尚未生成”，图表加载则无反应。现在保留文件路径和错误，显示当前结果页/图表的警告；过期项目、请求或变量选择的错误不污染新界面。

| 验收领域 | 当前直接证据 |
|---|---|
| 1. 规格/参数 | `spec.rs` 的隐式 LHS、OAT 及显式预算上限、PFT scope、输出路径回归 |
| 2. 采样 | `sample.rs` 固定 seed、多维线性/对数分层和 PFT 独立维度测试；OAT 每维双端实现 |
| 3. 创建/物化/冻结 | `engine.rs` 冻结内容/目录身份回归；`materialize.rs` 私有文件、实际 NetCDF PFT 槽位、长路径拒绝回归 |
| 4. 调度/恢复 | `checkpoint.rs` 损坏回退/校验和；`runner.rs` 单写者、并发上限、活 PID/过期心跳、取消及重试前校验回归 |
| 5. 统计 | `science.rs` Type-7/均值极值/并列秩；`runner.rs` 实际 history 归约、基准排除、错位时间轴和支持不足回归 |
| 6. GUI/Tauri | 全部 11 个 Node 文件；生产函数 VM 的创建/刷新/结果/队列竞态，以及本补遗三项反例；10 个 Tauri Study 测试、IPC 检查 |
| 7. 导出 | `export.rs` 无 checkpoint 拒绝、状态协调、重复快照、失败/取消/待复核区分、非自有文件保留回归 |
| 8. 端到端 | `runner.rs` 故障/恢复 fixture；固定 CLI/default 内核的真实 LHS20+基线 21/21 成功、48 时点、注入/分层/包络/34 文件导出证据 |

本次重新运行 92 个后端 Study 测试；JS 语法检查、全部 Node 测试、Tauri Study 测试、workspace fmt/Clippy 和 GUI IPC 均通过。反例与复验日志位于 `tmp/uq-completion-20260908/`。本补遗只修改 GUI 和回归，不修改 Rust、Fortran、采样算法或原始数据；固定原生证据仍为 `/private/tmp/cfi-103532-57586`（CLI SHA256 `b94ae7942ad51c5bdbf83246aaf9042e789872c1492fe515afb50ef16942451c`），不冒充新增的多年或多预设试验。当前补遗提交的远程 CI 以关联 PR 实际结果为准。

再次核查上文官方算法资料，并补查 [SciPy rankdata 的 average 并列秩定义](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats.rankdata.html)。这些资料用于核对数学合同，不是新增依赖；SciPy `seed` 参数更名为 `rng` 不影响本项目自有 Rust 采样器的固定整数 seed 合同。此前列明的科学验证边界保持不变。
