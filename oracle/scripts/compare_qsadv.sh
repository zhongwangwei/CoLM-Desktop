#!/usr/bin/env bash
# `MOD_Qsadv:qsadv`（饱和比湿及其导数）的两侧逐位差分。
# 上游侧用内核真实选项编译模块（不加 -ffp-contract=off），驱动本身按纪律加。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/qs_diff}
export QS_TMIN QS_TMAX
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I.bld -Iinclude -Imain -Ishare \
  main/MOD_Qsadv.F90 -J"$WORK" -o "$WORK/qs.o"
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  "$BASE/oracle/scripts/qsadv_diff.f90" "$WORK/qs.o" -o "$WORK/qs"
"$WORK/qs"
cd "$BASE"
cargo run -q -p colm-core --example qsadv_probe
python3 - "$WORK" <<'PY'
import collections, sys
work=sys.argv[1]
names=['T','p','es','esdT','qs','qsdT']
def rows(path):
    return [[p.upper().zfill(16) for p in line.replace('Z','').split()]
            for line in open(path) if line.split()]
f=rows(f'{work}/qs.txt'); r=rows(f'{work}/qs_rust.txt')
assert len(f)==len(r)==20000, (len(f), len(r))
bad=collections.Counter()
for a,b in zip(f,r):
    for k,(x,y) in enumerate(zip(a,b)):
        if x!=y: bad[names[k]]+=1
if bad:
    print('qsadv mismatches / 20000:', dict(bad)); raise SystemExit(1)
print('qsadv: inputs aligned and all 4 outputs 20000/20000 bitwise identical')
PY
