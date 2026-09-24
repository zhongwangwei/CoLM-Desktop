#!/usr/bin/env bash
# `MOD_Hydro_SoilWater:get_water_equilibrium_state` 的两侧逐位差分。
#
# 为什么要有它：这个例程**只在冷启动**被调用（`mkinidata/MOD_IniTimeVariable.F90:463`
# 的 `use_wtd` 分支经 `CALL get_water_equilibrium_state`），`colm.x` 运行时一次都不进 ——
# 干湿窗首分歧与三份黄金窗口对它**都不敏感**（实测：改对这两处形状后
# `dry_ts.sh 250` 仍 0/68 差异、黄金窗口逐位不变）。形状对不对只能靠这个闭环判。
#
# 上游侧用内核真实选项编译目标模块（**不加** `-ffp-contract=off` —— 要的就是 GCC
# 默认的 `-ffp-contract=fast`），驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`。
# 本仓库侧：`cargo run -p colm-core --example water_equilibrium_probe`，同一串 LCG。
#
# 用 `-ffunction-sections` + `-Wl,-dead_strip` 只保留真正被调用的过程：目标模块里
# 有些子程序依赖 `MOD_SPMD_Task`（MPI 那一支），这个闭环用不到，也不该把 MPI 拖进来。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/eqw_diff}
export EQW_OUT="$WORK"
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
BASE_FLAGS=(-O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0
            -fallow-argument-mismatch -fopenmp -ffunction-sections)
gfortran -c "${BASE_FLAGS[@]}" \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$WORK/namelist_stub.F90" -J"$WORK" -o "$WORK/stub.o"
gfortran -c "${BASE_FLAGS[@]}" \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  main/HYDRO/MOD_Hydro_SoilWater.F90 -J"$WORK" -o "$WORK/hsw.o"
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/water_equilibrium_diff.f90" \
  "$WORK/hsw.o" "$WORK/stub.o" \
  .bld/MOD_Hydro_SoilFunction.o .bld/MOD_UserDefFun.o \
  -Wl,-dead_strip -o "$WORK/eqw"
"$WORK/eqw"
cd "$BASE"
cargo run -q -p colm-core --example water_equilibrium_probe > "$WORK/eqw_rust.txt"
python3 - "$WORK" <<'PY'
import collections
import sys

work = sys.argv[1]


def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out


f = rows(f'{work}/eqw.txt')
r = rows(f'{work}/eqw_rust.txt')
assert len(f) == len(r) == 8000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
total = 0
for a, b in zip(f, r):
    assert a[:4] == b[:4], (a[:4], b[:4])
    flags[(a[0], a[1], a[2])] += 1
    for k in range(4, len(a)):
        total += 1
        if a[k].upper().zfill(16) != b[k].upper().zfill(16):
            if k == 4:
                name = 'wa'
            else:
                layer = (k - 5) // 3 + 1
                name = ['wliq', 'smp', 'hk'][(k - 5) % 3] + f'[{layer}]'
            bad[name] += 1
if bad:
    print('get_water_equilibrium_state mismatches:', dict(bad.most_common(12)))
    raise SystemExit(1)
print('get_water_equilibrium_state: all outputs bitwise identical;'
      f' {total} 个输出、分支分布 (k,flag,nlev) {dict(sorted(flags.items()))}')
PY
