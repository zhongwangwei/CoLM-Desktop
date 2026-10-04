#!/usr/bin/env bash
# `MOD_Hydro_SoilWater:get_zwt_from_wa`（含内联的 `secant_method_iteration`）的两侧逐位差分。
#
# 为什么要有它：这一段是干窗第 19 步那颗持久种子的所在地（第 301 轮把首差钉在
# `soilwater_aquifer_exchange` 改出来的 `zwt` 上），但**不能用黄金窗口判形状** ——
# 那三份窗口由 `f_vegwp` 混沌主导，改对末位会换一条混沌轨道，指标可能反向
# （第 301 轮实测：探针 12/240 → 7/240，干窗 `over_tol` 却 28 → 36）。
# 这个闭环在**合成输入**上逐位比 `zwt`：形状对不对与指标好不好彻底分开。
#
# 上游侧用内核真实选项编译目标模块（**不加** `-ffp-contract=off` —— 要的就是 GCC
# 默认的 `-ffp-contract=fast`），驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`。
# 本仓库侧：`cargo run -p colm-core --example get_zwt_probe`，同一串 LCG。
#
# 用 `-ffunction-sections` + `-Wl,-dead_strip` 只保留真正被调用的过程：目标模块里
# 有些子程序依赖 `MOD_SPMD_Task`（MPI 那一支），这个闭环用不到，也不该把 MPI
# 拖进来。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/gz_diff}
export GZ_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cat > "$WORK/namelist_stub.F90" <<'EOF'
MODULE MOD_Namelist
  IMPLICIT NONE
  LOGICAL :: DEF_USE_Campbell_SOIL_MODEL = .FALSE.
  LOGICAL :: DEF_USE_PLANTHYDRAULICS = .TRUE.
  LOGICAL :: DEF_USE_CoLMDEBUG = .FALSE.
END MODULE MOD_Namelist
EOF
cd "$BASE/vendor/CoLM202X"
BASE_FLAGS=(-O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0
            -fallow-argument-mismatch -fopenmp -ffunction-sections)
gfortran -c "${BASE_FLAGS[@]}" \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$WORK/namelist_stub.F90" -J"$WORK" -o "$WORK/stub.o"
gfortran -c "${BASE_FLAGS[@]}" \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  main/HYDRO/MOD_Hydro_SoilWater.F90 -J"$WORK" -o "$WORK/hsw.o"
gfortran -O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/get_zwt_diff.f90" \
  "$WORK/hsw.o" "$WORK/stub.o" \
  .bld/MOD_Hydro_SoilFunction.o .bld/MOD_UserDefFun.o \
  -Wl,-dead_strip -o "$WORK/gz"
"$WORK/gz"
cd "$BASE"
cargo run -q -p colm-core --example get_zwt_probe > "$WORK/gz_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['wa', 'zmin', 'vl_s', 'psi_s', 'zwt']
def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out
f = rows(f'{work}/gz.txt')
r = rows(f'{work}/gz_rust.txt')
assert len(f) == len(r) == 10000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0] and a[1] == b[1], (a[:2], b[:2])
    flags[a[1]] += 1
    for k, x in enumerate(a[2:7]):
        y = b[2 + k]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[names[k]] += 1
if bad:
    print('get_zwt_from_wa mismatches / 10000:', dict(bad))
    raise SystemExit(1)
print('get_zwt_from_wa: zwt 10000/10000 bitwise identical;'
      f' 早退分支计数 {dict(flags)}')
PY
