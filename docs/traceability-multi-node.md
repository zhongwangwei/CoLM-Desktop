# 多节点研究：问题—源码—修改—实验结果索引

本文只做**追溯索引**，不复制设计或实测细节。目标是让每个验收问题都能从“为什么做”追到“改了哪段代码、哪次提交、哪次作业”。

## 1. 研究问题与验收判据

| 编号 | 问题 | 验收判据 | 设计依据 |
|---|---|---|---|
| RQ1 | 0.1° 全球空间算例能否跨节点完成、成本多少 | 8 节点完成 480 步；记录墙钟、主循环、内存、I/O | [`design-multi-node.md`](design-multi-node.md) §1、§2 |
| RQ2 | N=1 与 N>1 是否逐字节一致 | 同 writer/同路径下，主重启、块旁车、`.river`、unitcat history 全部 `cmp` 相同 | 设计 §1 目标 2、§3–§5 |
| RQ3 | 河湖与 history 放进多进程后语义是否仍与上游一致 | hb64 河湖开/history 开 1 vs 2 rank 逐字节；全球 history 记录能完整写出 | 设计 §6–§8 |
| RQ4 | GUI/CLI 能否真实操作远程多节点 | config/key/password 认证、远程提交、状态、取消、取回端到端 | [`design-ai-assistant.md`](design-ai-assistant.md) 连接段 |
| RQ5 | 0.1° 全球预处理是否可完成 | 全球 mesh、landdata、初场产物可用；记录 fan-out 与耗时 | 设计 §10、§12 |

## 2. 问题到证据的索引

### RQ1：0.1° 全球多节点

- **源码**
  - `crates/colm-spmd/`：rank 发现、TCP star、gather/scatter/broadcast
  - `crates/colm-runtime/src/spmd.rs`、`spatial/partition.rs`：块清单与连续分片
  - `crates/colm-runtime/src/spatial/runtime.rs`：局部装配、每步交换、输出
  - `crates/colm-runtime/src/bin/colm-rs.rs`：`--rank/--ranks`、启动、restart
- **修改记录**
  - `5abf96af` Add multi-node SPMD execution to the Rust engine
  - `0ec276d6` Make the 0.1 degree global V3 run finish in 21 minutes
  - `1189041b` / `0298ee0d` / `63581324` / `d3ef774f`：强迫映射与并行化
  - `12e9d6d5` Skip the flood all-gathers when flood feedback is off
  - `885d4554` Bound restart writer threads and skip optimizer-only copies
  - `e004ab79` Add a lightweight carry path for ordinary patch steps
- **实验结果**
  - [`implementation-verification.md`](implementation-verification.md) 第 804–824 轮
  - 8 节点 10 天：job 8773503（23:51）、8774754（22:27）、8776408（**21:55**）
  - 河湖开 10 天：job 8778693（**28:30**）
  - 16 节点对照：job 8766586（已按用户要求不作为基线）
- **产物**
  - 全球 mesh：`phase3/global_mesh_0p1.nc`
  - landdata：`phase3/global_landdata_v3`（219 块 / 5.72M patch / 12 GB）
  - 运行目录：`phase3/global-case`

### RQ2：逐位一致

- **源码**
  - `crates/colm-spmd/src/lib.rs`：按 rank 顺序 gather、确定性帧
  - `crates/colm-runtime/src/spatial/exchange.rs`：patch 级交换
  - `crates/colm-runtime/src/spatial/runtime.rs`、`spatial/restart.rs`：每块重启/旁车
  - `crates/colm-runtime/src/history.rs`：根 writer 的全局 session
- **修改记录**
  - `5abf96af` 多进程骨架
  - `12e9d6d5` river gather 只在 flood 开启时发 evap/infil
  - `4cbd0821` `initial_totals` 在 `states` move 前计算
  - `885d4554` restart 写线程有界
  - `e004ab79` carry 路径
- **实验结果**
  - hb64 河湖+history 1 vs 2 node：**131/131**、**130/130**、**635/635**、**633/633** 等记录见 implementation-verification 第 802–823 轮
  - 全球 1 天 carry vs 参考：**2894/2894** `cmp` 相同
- **被否决的路径**
  - root-only 河网：旁车字节不一致，已回退（第 805 轮）
  - 无限制并行 restart / 并行 patch 装配：墙钟或稳定性回归（第 806、808 轮）
  - 16 节点：用户明确拒绝作为基线（第 804 轮）

### RQ3：河湖与 history

**Phase A 第一批（2026-10-27）**：漫滩曲线改累积面积、`qresv_ups_prev` 级联水库、`DEF_GRIDBASED_ROUTING_STORAGE_DT_LIMIT`、`DEF_Reservoir_Method=2` 保留；提交 `4fb639f3`，8 节点全球 10 天复验 job 8784590（22:27）。详见 implementation-verification 第 826 轮。

- **源码**
  - `crates/colm-runtime/src/river/`：河网、路由、reservoir/levee/bifurcation/flood
  - `crates/colm-runtime/src/history.rs`：累加器、逐记录 writer、sidecar
  - `crates/colm-runtime/src/spatial/exchange.rs`：history 根收集
- **修改记录**
  - `12e9d6d5` river gather 跳过
  - `4cbd0821` history-on 回归修复
  - history 累加器只 `gather` 给 root，不再 broadcast（修复 31 GB > 16 GiB 单帧）
- **实验结果**
  - 河湖开 8 节点 10 天：job 8778693，**28:30**；river gather 30–37% 主循环
  - 全球 history 开 10 天：job **8780132**（DAILY）**1:37:09** 完成；10 条日记录、`hb_hist_2003-01.nc` 940 MB
  - hb64 history 1 vs 2：**635/635**；root-only gather 改后 **633/633**
- **已知边界**
  - 河湖 `river gather` 仍是河湖开启时最大增量；下一步应按河系/分片减少全局 gather
  - 全球 history 单条记录原始累加器约 31 GB，必须保持 root-only gather

### RQ4：远程连接与 GUI/CLI

- **源码**
  - `gui/src-tauri/src/remote.rs`：服务器、认证方式、密码会话、路径映射
  - `gui/src-tauri/src/sidecar.rs`：远程子进程、密码 stdin、进程组
  - `crates/colm-remote/src/{auth,ssh}.rs`：CLI 认证与 ssh 调用
  - `gui/src/features/resources/ResourcesPage.tsx`：资源页认证表单
- **修改记录**
  - `1870b6c2` reservoir 文件门控 + 动态湖 label
  - `5a136889` `ps` 不可用时用 `pgrep` 兜底
  - `988e6252` 切换认证时清空 identity_file
  - `68e9c399` key/agent 与 password-only 提示
- **实验/证据**
  - tianhe config 模式 `remote-probe` 实际成功（hostname/分区/可写性）
  - GUI Rust lib 444 通过；前端 `npm test` 892 通过
  - CI：`ci`、`release` 通过；`windows-kernel` 待最新结果
- **未闭环**
  - 真实 password 登录 sshd（目前 broker/stub 测试）
  - 带口令私钥 + ssh-agent、Windows、ProxyJump 边界
  - GUI/CLI `remote-run` 真正提交/取回一个 tianhe 作业

### RQ5：预处理

- **源码**：`crates/colm-srfdata/`、`crates/colm-init/`、`phase3` 扇出脚本
- **修改记录**
  - `01901ac0` Allow explicit global raw-pixel preprocessing
  - `0ec276d6` 全球 landdata/V3
- **实验**
  - 0.1° mesh 2,211,055 活跃格；1446 个 5° tile 扇出合并为 5.72M patch
  - `mkinidata-rs --block` 生成 1446 块初场
  - 详见 implementation-verification 第 795–804 轮
- **未闭环**：`mksrfdata` 按块并行仍是 Phase 4 独立项

## 3. 原始产物与日志位置

这些**不入 git**，但可以从实验记录里的 job id 追溯：

- 运行日志：tianhe `~/HDD_POOL/colm-desktop/engine/phase3/global-case/colm-global-v3b-<job>.log`
- Slurm 脚本：同目录 `v3b.sbatch`、`mkini-gridriver.sbatch`
- hb64 探针：`~/HDD_POOL/colm-desktop/probes/hb64-bit1`、`hb64-bit2`
- 8 节点 release：`~/HDD_POOL/colm-desktop/target/release/colm-rs`
- 源码快照：`~/HDD_POOL/colm-desktop/engine/7752a17c7aa1aacc/src`

## 4. 已知连接缺口

1. **源码链接目前主要靠路径与提交号**，没有自动生成的代码引用；行号会漂移，优先用提交 URL。
2. **作业原始日志不在仓库**；依赖 tianhe 上的目录和 job id。若 job 记录过期，日志仍在，但没有归档。
3. **设计文档的 Phase 表有滞后**：Phase 3 的“GUI 节点数”实际已有进展，Phase 4 的 history 全球运行正在补；以 implementation-verification 的最新轮次为准。
4. **拒绝方案**已记录在提交和文档，但还没有统一“为什么不采用”的索引；本文件第 2 节列了主要几项。
5. **CI 与实验的边界**：CI 只跑单元/契约/构建，不跑 tianhe 作业；作业证据必须看 job id + 日志。

## 5. 更新规则

- 新实验先写 `implementation-verification.md` 的轮次，再更新本文对应行的 job id / 提交号；
- 新提交若改变验收判据，必须同时更新 §1 或 §4；
- 原始日志/产物路径变化时更新 §3。
