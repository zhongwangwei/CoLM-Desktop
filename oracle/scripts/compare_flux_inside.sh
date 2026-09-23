#!/usr/bin/env bash
# `MOD_Hydro_SoilWater:flux_inside_hm_soil` 的两侧逐位差分闭环。
#
# 为什么：第 305/13 轮逐条读反汇编，在这个函数里映射出**两处**内核会融合
# （`r0` 的分母 `prms(3)*(prms(2)-1)+prms(2)*2`、`grad_psi>1` 支
# `hk_u + ((psi_u-psi_l)/dz*hk_u**(1-rr))*hk_l**rr`），而本仓库对应函数一处
# `mul_add` 都没有；第 313 轮确认 `effective_hk_type` 编译期钉在
# `type_weighted_geometric_mean`，所以两处在**活分支**上。这个闭环在合成输入上
# 逐位判它们（纯函数，输入最好造、不进混沌窗口）。
#
# **私有例程的编译路线**（第 311/312 轮查定并实测）：`flux_inside_hm_soil` 在模块里是
# `PRIVATE`，所以把模块**拷进 `$WORK`** 并把那一行改成 `PUBLIC`，用拷贝编 `.mod`
# （`-I$WORK` 在 `-I.bld` 之前），vendor 源树不动。
#
# 上游侧用内核真实选项编译（**不加** `-ffp-contract=off`），驱动本身按仓库纪律加
# `-fwrapv -ffp-contract=off`。本仓库侧：`cargo run -p colm-core --example flux_inside_probe`。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/fi_diff}
export FI_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cat > "$WORK/namelist_stub.F90" <<'EOF'
MODULE MOD_Namelist
  IMPLICIT NONE
  LOGICAL :: DEF_USE_Campbell_SOIL_MODEL = .FALSE.
  LOGICAL :: DEF_USE_PLANTHYDRAULICS = .TRUE.
  LOGICAL :: DEF_USE_CoLMDEBUG = .FALSE.
END MODULE MOD_Namelist
EOF
sed 's/^   PRIVATE :: flux_inside_hm_soil$/   PUBLIC  :: flux_inside_hm_soil/' \
  "$BASE/vendor/CoLM202X/main/HYDRO/MOD_Hydro_SoilWater.F90" > "$WORK/hsw_copy.F90"
grep -q '^   PUBLIC  :: flux_inside_hm_soil$' "$WORK/hsw_copy.F90" || {
  echo "!! 拷贝里没找到 PRIVATE :: flux_inside_hm_soil（上游改过？）" >&2; exit 3; }
cd "$BASE/vendor/CoLM202X"
# `-ffunction-sections` + 链接期 `-Wl,-dead_strip`：模块里有些子程序依赖
# `MOD_SPMD_Task`（MPI 那一支），本闭环用不到，也不该把 MPI 拖进来。
BASE_FLAGS=(-O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0
            -fallow-argument-mismatch -fopenmp -ffunction-sections)
gfortran -c "${BASE_FLAGS[@]}" -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$WORK/namelist_stub.F90" -J"$WORK" -o "$WORK/stub.o"
gfortran -c "${BASE_FLAGS[@]}" -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$WORK/hsw_copy.F90" -J"$WORK" -o "$WORK/hsw.o"
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/flux_inside_diff.f90" \
  "$WORK/hsw.o" "$WORK/stub.o" \
  .bld/MOD_Hydro_SoilFunction.o .bld/MOD_UserDefFun.o \
  -Wl,-dead_strip -o "$WORK/fi"
"$WORK/fi"
cd "$BASE"
cargo run -q -p colm-core --example flux_inside_probe > "$WORK/fi_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['flux', 'grad_psi', 'hk_u']
def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out
f = rows(f'{work}/fi.txt')
r = rows(f'{work}/fi_rust.txt')
assert len(f) == len(r) == 15000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0] and a[1] == b[1], (a[:2], b[:2])
    flags[(a[0], a[1])] += 1
    for k, x in enumerate(a[2:5]):
        y = b[2 + k]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[names[k]] += 1
if bad:
    print('flux_inside_hm_soil mismatches / 15000:', dict(bad))
    raise SystemExit(1)
print('flux_inside_hm_soil: all 3 outputs 15000/15000 bitwise identical;'
      f' 分支分布 {dict(sorted(flags.items()))}')
PY
