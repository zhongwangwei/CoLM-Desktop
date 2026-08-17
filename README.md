# colm-desktop

把 CoLM202X 的 SinglePoint 模式做成跨平台桌面程序。设计见 `docs/design.md`。

**当前状态**：里程碑 0–1。仓库骨架 + 成败判定 + 黄金输出回归基准。
还没有 GUI，也还没有编排层。

## 为什么有 `crates/colm-kernel/src/outcome.rs`

CoLM 在单点模式下，**成功与失败都以退出码 0 结束，但走的是两条不同的路**：

- 失败走 `share/MOD_SPMD_Task.F90` 的 `CoLM_stop`，其 `#ifndef USEMPI` 分支是裸 `STOP`。
- 成功不执行任何收尾调用，直接跑到 `main/CoLM.F90:764` 的 `END PROGRAM CoLM`
  （`spmd_exit` 只定义并调用于 `#ifdef USEMPI` 内）。

退出码相同是两条路径的巧合，不是共用一条路径。所以判定成败必须同时满足三件事：
无错误标记、有正向成功标记、产物齐全。

附带结论：既然 `CoLM_stop` 是失败专用的，把那个裸 `STOP` 改成 `STOP 1`
是安全的上游修复。即便上游改了，本模块仍然必要 —— 产物硬校验能抓住
「跑完了但没写出该写的文件」，错误标记扫描能抓住部分失败。

## 跑黄金回归

需要 PLUMBER2 数据（不入库）与 gfortran + netcdf-fortran。

```bash
export PLUMBER2_ROOT=/path/to/PLUMBER2s
./oracle/scripts/build_kernel.sh waterheat
cargo run -p oracle --bin golden-run -- CN-Cng
cargo run -p oracle --bin golden-compare -- \
  oracle/golden/CN-Cng_hist_2008-01.nc \
  oracle/work/CN-Cng/out/CN-Cng/history/CN-Cng_hist_2008-01.nc
```

## 两个窗口，以及它们各自不覆盖什么

| 算例 | 窗口 | 覆盖 | 不覆盖 |
|---|---|---|---|
| `CN-Cng` | 2008-01-01 → 01-11 | 冻结土壤、雪、辐射 | 产流与入渗（窗口内无降水） |
| `CN-Cng-wet` | 2008-07-01 → 07-16 | 饱和超渗产流、入渗、地下水位动态 | `f_rsur_ie`（超渗产流）、`f_rsub`（地下产流）—— 两个窗口都为 0 |

在 `f_rsur_ie` 与 `f_rsub` 被覆盖之前，不得声称产流模块已验证。
