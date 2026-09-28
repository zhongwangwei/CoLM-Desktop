# BGC 移植辅助工具

- `gx.py DUMP FUNC [FROM TO]`：把 gfortran `-O2 -fdump-tree-optimized-lineno` 的 GIMPLE 还原成
  按源码行的表达式（SSA 临时量展开、数组载入还原成数组名），用来看运算顺序、常数折叠与 FMA 收缩。
- `f2rs.py FILE SUB --gimple DUMP`：把 BGC 状态更新类子程序（赋值/IF/DO/`#ifdef CROP`）转写成 Rust。
  字段归属与下标来自 `gen_bgc_state.py`；`a ± b·c` 按 GCC `convert_mult_to_fma` 的规则收缩，
  并逐行对照 GIMPLE：GIMPLE 该行没有 FMA（乘积被 CSE 共享到别的基本块）就不收缩。
- `wrap.py`/`mkmod.py`：套上函数签名与模块头，处理 `DEF_USE_TRACER`/`#ifdef FUN` 分支，rustfmt。
- `regen.sh`：重新生成 `bgc_c_state_update.rs`、`bgc_n_state_update.rs`、`bgc_soil_n_state_update.rs`。
  这三个文件完全由它生成，改动请改工具或上游，再重跑。

GIMPLE 的产生：在 kernel 构建树里对单个文件执行（与 `oracle/scripts/build_kernel.sh` 相同的 flags 外加
`-fdump-tree-optimized-lineno -c`）。正确性最终由 `bgc_replay` 逐过程回放把关（`oracle/scripts/gen_bgc_trace.py`）。
