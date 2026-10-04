#!/usr/bin/env bash
# `MOD_Hydro_SoilWater:water_balance` 的两侧逐位差分闭环。
#
# 为什么：第 309 轮在这个例程的汇编里认出 5 条"和（差）与乘积"形状的 FMA（没有割线
# 特征、也不是 `flux_inside_hm_soil` 那两条），第 317 轮把它们逐条对回源表达式。
# 这个闭环在合成输入上逐位判这 5 处 —— `water_balance` 是纯函数，输入最好造，
# 而且不进混沌窗口，所以"形状对不对"与"黄金口径好不好"彻底分开（第 315 轮的教训：
# 逐位全对的函数也能让干窗翻 13 倍）。
#
# **私有例程的编译路线**（第 311/312 轮查定并实测）：`water_balance` 在模块里是
# `PRIVATE`，所以把模块**拷进 `$WORK`** 并把那一行改成 `PUBLIC`，用拷贝编 `.mod`
# （`-I$WORK` 在 `-I.bld` 之前），vendor 源树不动。
#
# 上游侧用内核真实选项编译（**不加** `-ffp-contract=off` —— 要的就是 GCC 默认的
# `-ffp-contract=fast`），驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`。
# 本仓库侧：`cargo run -p colm-core --example water_balance_probe`。
#
# `-ffunction-sections` + 链接期 `-Wl,-dead_strip`：模块里有些子程序依赖
# `MOD_SPMD_Task`（MPI 那一支），本闭环用不到，也不该把 MPI 拖进来。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/wb_diff}
export WB_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cat > "$WORK/namelist_stub.F90" <<'EOF'
MODULE MOD_Namelist
  IMPLICIT NONE
  LOGICAL :: DEF_USE_Campbell_SOIL_MODEL = .FALSE.
  LOGICAL :: DEF_USE_PLANTHYDRAULICS = .TRUE.
  LOGICAL :: DEF_USE_CoLMDEBUG = .FALSE.
END MODULE MOD_Namelist
EOF
sed 's/^   PRIVATE :: water_balance$/   PUBLIC  :: water_balance/' \
  "$BASE/vendor/CoLM202X/main/HYDRO/MOD_Hydro_SoilWater.F90" > "$WORK/hsw_copy.F90"
grep -q '^   PUBLIC  :: water_balance$' "$WORK/hsw_copy.F90" || {
  echo "!! 拷贝里没找到 PRIVATE :: water_balance（上游改过？）" >&2; exit 3; }
cd "$BASE/vendor/CoLM202X"
BASE_FLAGS=(-O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0
            -fallow-argument-mismatch -fopenmp -ffunction-sections)
gfortran -c "${BASE_FLAGS[@]}" -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$WORK/namelist_stub.F90" -J"$WORK" -o "$WORK/stub.o"
gfortran -c "${BASE_FLAGS[@]}" -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$WORK/hsw_copy.F90" -J"$WORK" -o "$WORK/hsw.o"
gfortran -O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/water_balance_diff.f90" \
  "$WORK/hsw.o" "$WORK/stub.o" \
  .bld/MOD_Hydro_SoilFunction.o .bld/MOD_UserDefFun.o \
  -Wl,-dead_strip -o "$WORK/wb"
"$WORK/wb"
cd "$BASE"
cargo run -q -p colm-core --example water_balance_probe > "$WORK/wb_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out
f = rows(f'{work}/wb.txt')
r = rows(f'{work}/wb_rust.txt')
assert len(f) == len(r) == 12000, (len(f), len(r))
bad = collections.Counter()
scenarios = collections.Counter()
solvable = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0] and a[1] == b[1], (a[:2], b[:2])
    assert len(a) == len(b), (len(a), len(b))
    scenarios[a[0]] += 1
    solvable[a[-1]] += 1
    # a[0]=情形, a[1]=层数, a[2:2+nlev+2]=blc(0..nlev+1), a[-1]=solvable
    for idx in range(len(a) - 3):
        x = a[2 + idx]
        y = b[2 + idx]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[f'blc[{idx}]'] += 1
if bad:
    print('water_balance mismatches / 12000:', dict(bad))
    print('  情形分布:', dict(sorted(scenarios.items())), 'solvable:', dict(sorted(solvable.items())))
    raise SystemExit(1)
print('water_balance: blc 12000/12000 bitwise identical;'
      f' 情形分布 {dict(sorted(scenarios.items()))}')
PY
