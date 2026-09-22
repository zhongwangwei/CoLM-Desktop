#!/usr/bin/env bash
# `MOD_PhaseChange:meltf` 的两侧逐位差分。
#
# 上游侧：驱动**直接链 `.bld` 的内核产线对象**（排除 `CoLM.o`），因此用的是真 namelist、
# 真的依赖链。链接必须用**全 `.bld` 集合**：`meltf` 依赖 `soil_vliq_from_psi`
# （`MOD_Hydro_SoilFunction`）与 namelist 的 `DEF_SPLIT_SOILSNOW`，只链 `MOD_PhaseChange.o`
# 会缺符号。驱动本身按纪律加 `-fwrapv -ffp-contract=off`，模块对象是现成的产线对象。
#
# **配置对齐（本脚本会打印，改 Rust 探针前先看它）**：实测 `.bld` namelist 默认为
#   DEF_USE_Campbell_SOIL_MODEL = F  → 走 van Genuchten
#   DEF_USE_SUPERCOOL_WATER     = T  → 只把雪层（与 patchtype 3）钉到冰点
#   DEF_SPLIT_SOILSNOW          = F
# 探针里必须逐一匹配（`SoilHydraulicModel::VanGenuchten`、`supercool_water: true`、
# `split_soil_snow: false`）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/pc_diff}
export PC_OUT="$WORK"
export NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}
NETCDFF_DIR=${NETCDFF_DIR:-/opt/homebrew/opt/netcdf-fortran}
rm -rf "$WORK"; mkdir -p "$WORK"

cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_phasechange.mod ] || {
  echo "缺少 .bld：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I.bld -Iinclude -Imain -Ishare -J"$WORK" \
  -c "$BASE/oracle/scripts/phasechange_diff.f90" -o "$WORK/drv.o"
mpifort "$WORK/drv.o" $(ls .bld/*.o | grep -v '/CoLM.o$') \
  -L"$NETCDF_DIR/lib" -L"$NETCDFF_DIR/lib" -lnetcdff -lnetcdf -llapack -lblas \
  -o "$WORK/pc"
"$WORK/pc" | sed 's/^/== 内核 namelist: /'
cd "$BASE"
cargo run -q -p colm-core --example phase_change_probe
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['t', 'wliq', 'wice', 'scv', 'sm', 'xmf', 'thaw', 'freeze']

def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            # 上游 Z17 对 0.0 印 "0"、Rust 印 16 个 0 —— 必须先归一化再比
            out.append((parts[0], [p.upper().zfill(16) for p in parts[1:9]], int(parts[9])))
    return out

f = rows(f'{work}/pc.txt'); r = rows(f'{work}/pc_rust.txt')
assert len(f) == len(r) == 10000, (len(f), len(r))
bad = collections.Counter(); cases = 0
for a, b in zip(f, r):
    differing = [names[k] for k in range(8) if a[1][k] != b[1][k]]
    if a[2] != b[2]:
        differing.append('imelt_sum')
    if differing:
        cases += 1
        for name in differing:
            bad[name] += 1
if bad:
    print('MOD_PhaseChange:meltf mismatches / 10000:')
    for name, count in sorted(bad.items()):
        print(f'  {name:10s} {count}')
    raise SystemExit(1)
print('MOD_PhaseChange:meltf: 5 patchtypes x 2000 组、9 个量 10000/10000 逐位相同')
PY
