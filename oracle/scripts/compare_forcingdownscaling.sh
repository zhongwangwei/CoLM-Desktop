#!/usr/bin/env bash
# `MOD_ForcingDownscaling:downscale_forcings`（简单地形支）的两侧逐位差分。
#
# 与 compare_forcingdownscaling_wind.sh 同一套路：驱动直接链接 `.bld` 下的内核
# 产线对象（只排除 `CoLM.o`），所以 `DEF_DS_*` 是真的 namelist 变量 —— 上游驱动
# 逐组赋值、逐组调用；本仓库侧把同样的值组装成 `ForcingDownscalingConfig`。
# 驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/fd_full}
NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}
NETCDFF_DIR=${NETCDFF_DIR:-/opt/homebrew/opt/netcdf-fortran}
export FD_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_forcingdownscaling.mod ] || {
  echo "缺少 .bld：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -DSinglePoint \
  -I.bld -Iinclude -Imain -Ishare -J"$WORK" \
  "$BASE/oracle/scripts/forcingdownscaling_full_diff.f90" -c -o "$WORK/drv.o"
mpifort "$WORK/drv.o" $(ls .bld/*.o | grep -v '/CoLM.o$') \
  -L"$NETCDF_DIR/lib" -L"$NETCDFF_DIR/lib" -lnetcdff -lnetcdf -llapack -lblas \
  -o "$WORK/fdfull"
"$WORK/fdfull"
cd "$BASE"
cargo run -q -p colm-core --example forcing_downscaling_full_probe
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['t', 'th', 'q', 'pbot', 'rho', 'prc', 'prl', 'lwrad', 'swrad', 'us', 'vs']
f = []
for line in open(f'{work}/fdfull.txt'):
    parts = line.split()
    if not parts: continue
    f.append((parts[0], [p.upper().zfill(16) for p in parts[1:]]))
r = []
for line in open(f'{work}/fdfull_rust.txt'):
    parts = line.split()
    if not parts: continue
    r.append((parts[0], [p.upper().zfill(16) for p in parts[1:]]))
assert len(f) == len(r) == 20000, (len(f), len(r))
bad = collections.Counter(); groups = collections.Counter()
for (flags, row), (other_flags, other) in zip(f, r):
    assert flags == other_flags, (flags, other_flags)
    assert len(row) == len(other) == 11, (len(row), len(other))
    groups[flags] += 1
    for k, (x, y) in enumerate(zip(row, other)):
        if x != y: bad[(names[k], flags)] += 1
if bad:
    print('MOD_ForcingDownscaling:downscale_forcings mismatches / 20000:')
    for (name, flags), count in sorted(bad.items()):
        print(f'  {name:6s} ip={flags[0]} il={flags[1]}: {count}')
    raise SystemExit(1)
print('MOD_ForcingDownscaling:downscale_forcings: 11 outputs x 4 configs '
      f'= {sum(groups.values()) * 11}/{sum(groups.values()) * 11} bitwise identical')
print('  配置分布 (ip il -> 组数):', ' '.join(f'{k}:{v}' for k, v in sorted(groups.items())))
PY
