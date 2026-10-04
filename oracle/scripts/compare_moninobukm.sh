#!/usr/bin/env bash
# `MOD_FrictionVelocity:moninobukm`（冠层近地层廓线）的两侧逐位差分。
#
# 上游侧：编译 vendor 的 MOD_FrictionVelocity.F90（内核真实选项，**不加**
# `-ffp-contract=off` —— 要的就是 GCC 的默认收缩），再与驱动
# `moninobukm_diff.f90` 链接（驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`）。
# 本仓库侧：`cargo run -p colm-core --example mo_probe`，同一串 LCG。
# 两侧各 20000 组，比 **20 个输出**，覆盖整个 `MOD_FrictionVelocity` 模块：
#   `moninobukm` 10 个（ustar/fh2m/fq2m/fmtop/fm/fh/fq/fht/fqt/phih）、
#   `moninobuk` 7 个（ustar/fh2m/fq2m/fm10m/fm/fh/fq）、
#   `kmoninobuk`、`kintmoninobuk`、`moninobukini` 各 1 个。
#
# 实测（2026）：修掉 UNSTABLE_HEAT_COEFFICIENT 的 1 ULP 之前 phih 第一支
# 376 组错 375；修完 20/20 输出 20000/20000 全同（该模块的 60 处收缩由此结案）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/mo_diff}
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
gfortran -c -O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I.bld -Iinclude -Imain -Ishare \
  main/MOD_FrictionVelocity.F90 -J"$WORK" -o "$WORK/fv.o"
gfortran -O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  "$BASE/oracle/scripts/moninobukm_diff.f90" "$WORK/fv.o" -o "$WORK/mo"
"$WORK/mo"
cd "$BASE"
cargo run -q -p colm-core --example mo_probe
python3 - "$WORK" <<'PY'
import collections, sys
work=sys.argv[1]
names=['ustar','fh2m','fq2m','fmtop','fm','fh','fq','fht','fqt','phih',
       'ustar_m','fh2m_m','fq2m_m','fm10m_m','fm_m','fh_m','fq_m',
       'kcob','kint','uat']
def rows(path, skip_first):
    out=[]
    for line in open(path):
        parts=line.replace('Z','').split()
        if not parts: continue
        out.append([p.upper().zfill(16) for p in (parts[1:] if skip_first else parts)])
    return out
f=rows(f'{work}/mo.txt', True); r=rows(f'{work}/mo_rust.txt', False)
assert len(f)==len(r)==20000, (len(f), len(r))
bad=collections.Counter()
for a,b in zip(f,r):
    for k,(x,y) in enumerate(zip(a,b)):
        if x!=y: bad[names[k]]+=1
if bad:
    print('MOD_FrictionVelocity mismatches / 20000:', dict(bad)); raise SystemExit(1)
print('MOD_FrictionVelocity: all 20 outputs 20000/20000 bitwise identical')
PY
