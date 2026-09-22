#!/usr/bin/env bash
# `MOD_TurbulenceLEddy`（LZD2022 大涡近地层方案）的两侧逐位差分。
#
# 这个模块**任何本地算例都走不到**（三个黄金算例的强迫场里没有 `hpbl`，
# `DEF_USE_CBL_HEIGHT` 关着），所以它只能靠差分证明，黄金窗口给不了证据。
#
# 上游侧：编译 vendor 的 MOD_TurbulenceLEddy.F90（内核真实选项，**不加**
# `-ffp-contract=off`），再与 `oracle/scripts/turbulence_leddy_diff.f90` 链接
# （驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`）。
# 本仓库侧：`cargo run -p colm-core --example leddy_probe`，同一串 LCG。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/leddy_diff}
export LEDDY_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I.bld -Iinclude -Imain -Ishare \
  main/MOD_TurbulenceLEddy.F90 -J"$WORK" -o "$WORK/leddy.o"
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  "$BASE/oracle/scripts/turbulence_leddy_diff.f90" "$WORK/leddy.o" -o "$WORK/leddy"
"$WORK/leddy"
cd "$BASE"
cargo run -q -p colm-core --example leddy_probe
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = (['ustar', 'fh2m', 'fq2m', 'fm10m', 'fm', 'fh', 'fq']
         + ['ustar_m', 'fh2m_m', 'fq2m_m', 'fmtop_m', 'fm_m', 'fh_m', 'fq_m',
            'fht_m', 'fqt_m', 'phih_m'])
f = []
for line in open(f'{work}/leddy.txt'):
    parts = line.split()
    if not parts: continue
    f.append((parts[0], [p.upper().zfill(16) for p in parts[1:]]))
r = [[p.upper().zfill(16) for p in line.split()]
     for line in open(f'{work}/leddy_rust.txt') if line.split()]
assert len(f) == len(r) == 20000, (len(f), len(r))
bad = collections.Counter()
groups = collections.Counter()
for (flags, row), other in zip(f, r):
    assert len(row) == len(other) == 17, (len(row), len(other))
    groups[flags] += 1
    for k, (x, y) in enumerate(zip(row, other)):
        if x != y: bad[(names[k], flags)] += 1
if bad:
    print('MOD_TurbulenceLEddy mismatches / 20000:')
    for (name, flags), count in sorted(bad.items()):
        print(f'  {name:8s} ib={flags[0]} jb={flags[1]} kb={flags[2]} zc={flags[3]}: {count}')
    raise SystemExit(1)
print('MOD_TurbulenceLEddy: all 17 outputs 20000/20000 bitwise identical')
print('  分支分布 (ib jb kb zc -> 组数):',
      ' '.join(f'{k}:{v}' for k, v in sorted(groups.items())))
PY
