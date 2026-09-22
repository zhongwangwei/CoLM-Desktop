#!/usr/bin/env bash
# `MOD_ForcingDownscaling` full 短波支（`downscale_shortwave`）的两侧逐位差分。
#
# 两套阴影表各跑一遍（模块对象必须与编译开关一致，否则被调方会把查找表当曲线表读）：
#   curve：网格内核的 `.bld` 对象（`include/define.h` 是 GRIDBASED + #undef SinglePoint），
#          驱动**不**定义 SinglePoint，传 `sf_curve_c(16,3)`，48 个值走 LCG；
#   lut  ：单点内核那一套，脚本自己用产线选项编 `MOD_ForcingDownscaling.F90`（加
#          -DSinglePoint），链接时顶掉 `.bld` 里的同名对象，驱动传 `sf_lut_c(16,101)`。
# 驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`；模块本体不加。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/fd_sw}
NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}
NETCDFF_DIR=${NETCDFF_DIR:-/opt/homebrew/opt/netcdf-fortran}
LIBS=(-L"$NETCDF_DIR/lib" -L"$NETCDFF_DIR/lib" -lnetcdff -lnetcdf -llapack -lblas)
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_forcingdownscaling.mod ] || {
  echo "缺少 .bld：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
BLOBS=($(ls .bld/*.o | grep -v '/CoLM.o$'))

run_phase () {  # $1 = curve|lut
  local variant="$1"
  local defs="" modobj=""
  [ "$variant" = lut ] && defs="-DSinglePoint" 
  if [ "$variant" = lut ]; then
    # `include/define.h` 现在是 GRIDBASED 那一版，里面有一行 `#undef SinglePoint`，
    # 只加 `-D` 会被它吃掉。这里把该行改成 `#define`，用 -I"$WORK" 压在 include/ 前面，
    # 让 `#include <define.h>` 取到这一份。该模块里 `#ifdef SinglePoint` 只包着阴影表
    # 那几行（:110/:154/:269/:281/:659/:702/:755），所以这样翻不动别的语义。
    sed 's/^#undef SinglePoint$/#define SinglePoint/' include/define.h > "$WORK/define.h"
    gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
      -fallow-argument-mismatch -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
      main/MOD_ForcingDownscaling.F90 -J"$WORK" -o "$WORK/fd_singlepoint.o"
    modobj="$WORK/fd_singlepoint.o"
  else
    modobj="$(ls .bld/*.o | grep '/MOD_ForcingDownscaling.o$')"
  fi
  gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
    -fallow-argument-mismatch -fwrapv -ffp-contract=off $defs \
    -I"$WORK" -I.bld -Iinclude -Imain -Ishare -J"$WORK" \
    "$BASE/oracle/scripts/forcingdownscaling_shortwave_diff.f90" -c -o "$WORK/drv_$variant.o"
  mpifort "$WORK/drv_$variant.o" $modobj \
    $(printf '%s\n' "${BLOBS[@]}" | grep -v '/MOD_ForcingDownscaling.o$') \
    "${LIBS[@]}" -o "$WORK/fdsw_$variant"
  FD_OUT="$WORK/$variant" mkdir -p "$WORK/$variant"
  FD_OUT="$WORK/$variant" "$WORK/fdsw_$variant" >/dev/null
  cd "$BASE"
  FD_OUT="$WORK/$variant" FD_VARIANT="$variant" \
    cargo run -q -p colm-core --example forcing_downscaling_shortwave_probe
  cd "$BASE/vendor/CoLM202X"
}
run_phase curve
run_phase lut
cd "$BASE"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['t', 'th', 'q', 'pbot', 'rho', 'prc', 'prl', 'lwrad', 'swrad', 'us', 'vs']
failed = False
for variant in ('curve', 'lut'):
    f = []
    for line in open(f'{work}/{variant}/fdsw.txt'):
        parts = line.split()
        if not parts: continue
        f.append((parts[0], [p.upper().zfill(16) for p in parts[1:]]))
    r = [[p.upper().zfill(16) for p in line.split()]
         for line in open(f'{work}/{variant}/fdsw_rust_{variant}.txt') if line.split()]
    assert len(f) == len(r) == 2000, (variant, len(f), len(r))
    bad = collections.Counter(); groups = collections.Counter()
    for (flags, row), other in zip(f, r):
        assert len(row) == len(other) == 11, (len(row), len(other))
        groups[flags] += 1
        for k, (x, y) in enumerate(zip(row, other)):
            if x != y: bad[(names[k], flags)] += 1
    if bad:
        failed = True
        print(f'full shortwave [{variant}] mismatches / 2000:')
        for (name, flags), count in sorted(bad.items()):
            print(f'  {name:6s} alb_nan={flags[0]} svf_oob={flags[1]} coszen0={flags[2]}: {count}')
    else:
        print(f'full shortwave [{variant}]: 11 outputs 2000/2000 bitwise identical'
              f'  (分支分布 {dict(sorted(groups.items()))})')
raise SystemExit(1 if failed else 0)
PY
