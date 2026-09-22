#!/usr/bin/env bash
# `MOD_SoilThermalParameters:soil_hcap_cond`（8 档导热率方案）的两侧逐位差分。
# 上游侧用内核真实选项编译模块（不加 -ffp-contract=off），驱动本身按纪律加。
#
# 该子程序唯一读的 namelist 量是 DEF_THERMAL_CONDUCTIVITY_SCHEME，而链接真正的
# MOD_Namelist 会把 MOD_SPMDTask / MOD_FileSystem 等一大串运行时依赖拖进来。这里
# 因此就地生成一个只含该标志的桩模块，用 -I"$WORK" 压在 .bld 前面，让 8 档方案
# 能在同一次运行里遍历完；模块本身仍按产线选项编译。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/hc}
export HC_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cat > "$WORK/namelist_stub.F90" <<'EOF'
MODULE MOD_Namelist
  IMPLICIT NONE
  INTEGER :: DEF_THERMAL_CONDUCTIVITY_SCHEME = 4
END MODULE MOD_Namelist
EOF
cd "$BASE/vendor/CoLM202X"
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  "$WORK/namelist_stub.F90" -J"$WORK" -o "$WORK/stub.o"
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  main/MOD_SoilThermalParameters.F90 -J"$WORK" -o "$WORK/hc_mod.o"
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  "$BASE/oracle/scripts/soil_hcap_cond_diff.f90" \
  "$WORK/stub.o" "$WORK/hc_mod.o" -o "$WORK/hc"
"$WORK/hc"
cd "$BASE"
cargo run -q -p colm-core --example soil_thermal_probe
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
def rows(path):
    out = []
    for line in open(path):
        if not line.split(): continue
        head, *rest = line.replace('Z', '').split()
        out.append((head[0], int(head[1]), [p.upper().zfill(16) for p in rest]))
    return out
f = rows(f'{work}/hc.txt'); r = rows(f'{work}/hc_rust.txt')
assert len(f) == len(r) == 40000, (len(f), len(r))
bad = collections.Counter(); tot = collections.Counter(); ungated = collections.Counter()
for a, b in zip(f, r):
    assert a[:2] == b[:2], (a[:2], b[:2])
    scheme, flag = a[0], a[1]
    tot[scheme] += 1
    if a[2][0] != b[2][0]: bad[(scheme, 'hcap')] += 1
    if a[2][1] != b[2][1]: bad[(scheme, 'thk')] += 1
    if not flag: ungated[scheme] += 1
if bad:
    print('soil_hcap_cond mismatches:', dict(bad)); raise SystemExit(1)
if not ungated:
    print('soil_hcap_cond: sr<1e-10 那道门一次都没走到，边界抽样没起作用'); raise SystemExit(1)
print('soil_hcap_cond: 输入对齐，8 档方案 × 5000 组、2 个输出全部逐位相同 '
      f'(hcap {sum(tot.values())}/{sum(tot.values())}, thk 同)；'
      f'其中 {sum(ungated.values())} 组走的是 sr<1e-10 的干土路径')
PY
