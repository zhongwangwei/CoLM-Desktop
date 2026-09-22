#!/usr/bin/env bash
# `MOD_ForcingDownscaling` 风场降尺度（`downscale_wind` / `downscale_wind_simple`）
# 的两侧逐位差分。
#
# 上游侧**不重编模块**：直接链接 `vendor/CoLM202X/.bld/*.o` —— 那是内核真实的
# 产线对象（`objdump` 里能看到 `fmadd`，即 GCC 默认收缩已开），只把 `CoLM.o`
# 这个 PROGRAM 排除掉。于是连 namelist、常量模块都是真的，不需要桩。
# 驱动本身仍按仓库纪律加 `-fwrapv -ffp-contract=off`。
# 本仓库侧：`cargo run -p colm-core --example forcing_downscaling_probe`。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/fd_diff}
NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}
NETCDFF_DIR=${NETCDFF_DIR:-/opt/homebrew/opt/netcdf-fortran}
export FD_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_forcingdownscaling.mod ] || {
  echo "缺少 .bld：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I.bld -Iinclude -Imain -Ishare -J"$WORK" \
  "$BASE/oracle/scripts/forcingdownscaling_wind_diff.f90" -c -o "$WORK/drv.o"
mpifort "$WORK/drv.o" $(ls .bld/*.o | grep -v '/CoLM.o$') \
  -L"$NETCDF_DIR/lib" -L"$NETCDFF_DIR/lib" -lnetcdff -lnetcdf -llapack -lblas \
  -o "$WORK/fdwind"
"$WORK/fdwind"
cd "$BASE"
cargo run -q -p colm-core --example forcing_downscaling_probe
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['us_full', 'vs_full', 'us_simple', 'vs_simple']
f = []
for line in open(f'{work}/fdwind.txt'):
    parts = line.split()
    if not parts: continue
    f.append((parts[0], [p.upper().zfill(16) for p in parts[1:]]))
r = [[p.upper().zfill(16) for p in line.split()]
     for line in open(f'{work}/fdwind_rust.txt') if line.split()]
assert len(f) == len(r) == 20000, (len(f), len(r))
bad = collections.Counter(); groups = collections.Counter()
for (flags, row), other in zip(f, r):
    assert len(row) == len(other) == 4, (len(row), len(other))
    groups[flags] += 1
    for k, (x, y) in enumerate(zip(row, other)):
        if x != y: bad[(names[k], flags)] += 1
if bad:
    print('MOD_ForcingDownscaling wind mismatches / 20000:')
    for (name, flags), count in sorted(bad.items()):
        print(f'  {name:10s} f0={flags[0]} f1={flags[1]}: {count}')
    raise SystemExit(1)
print('MOD_ForcingDownscaling wind: 4 outputs 20000/20000 bitwise identical')
print('  分支分布 (f0 f1 -> 组数):', ' '.join(f'{k}:{v}' for k, v in sorted(groups.items())))
PY
