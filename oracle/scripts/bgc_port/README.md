# BGC 移植辅助工具

- `gx.py DUMP FUNC [FROM TO]`：把 gfortran `-O2 -fdump-tree-optimized-lineno` 的 GIMPLE 还原成
  按源码行的表达式（SSA 临时量展开、数组载入还原成数组名），用来看运算顺序、常数折叠与 FMA 收缩。
- `f2rs.py FILE SUB --gimple DUMP`：把 BGC 状态更新类子程序（赋值/IF/DO/`#ifdef CROP`）转写成 Rust。
  字段归属与下标来自 `gen_bgc_state.py`；`a ± b·c` 按 GCC `convert_mult_to_fma` 的规则收缩，
  并逐行对照 GIMPLE：GIMPLE 该行没有 FMA（乘积被 CSE 共享到别的基本块）就不收缩。
- `wrap.py`/`mkmod.py`：套上函数签名与模块头，处理 `DEF_USE_TRACER`/`#ifdef FUN` 分支，rustfmt。
- `regen.py`：按表重新生成 colm-core 里的规则化 BGC 模块（C/N 状态更新、间隙死亡、年更新、淋溶、
  汇总、收支检查、植被结构）。这些文件完全由它生成（文件头注明），改动请改工具或上游后重跑：
  `GIMPLE=<dump 目录> python3 oracle/scripts/bgc_port/regen.py`。
- 除 FMA 外还复现两种编译器行为：被 CSE 共享到别的基本块的乘积不融合；被向量化成保序归约的
  `sum(x(ps:pe)*pftfrac(ps:pe))`（GIMPLE 里有 `vect_`）用 `vectorized_dot`。

GIMPLE 的产生：在 kernel 构建树里对单个文件执行（与 `oracle/scripts/build_kernel.sh` 相同的 flags 外加
`-fdump-tree-optimized-lineno -c`）。正确性最终由 `bgc_replay` 逐过程回放把关（`oracle/scripts/gen_bgc_trace.py`）。
