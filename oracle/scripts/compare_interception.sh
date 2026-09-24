#!/usr/bin/env bash
# `MOD_LeafInterception:LEAF_interception_CoLM2014` 的两侧逐位差分闭环。
#
# 为什么：第 326 轮修好 `p0`/`pinf`/`:328-329` 三处之后，湿窗 `over_tol` 1907→1287，
# 但剩下 12 条 FMA（`ap`/`cp`/`aa1`/`bb1`/`drainage`/`qintr_snow`/`tex_snow` 等）只影响
# `ldew_rain`/`ldew_snow` 这些 **history 不导出的分量** —— 只看黄金窗口判不了它们，
# 必须像 `compare_water_balance.sh` 那样在合成输入上逐位判。
#
# 这个例程在模块里是 **PUBLIC**，所以不需要"拷贝+放行"，直接链 `.bld` 的对象。
# 上游侧用内核同款选项编译（不加 `-ffp-contract=off`），驱动本身按仓库纪律加
# `-fwrapv -ffp-contract=off`。本仓库侧：`cargo run -p colm-core --example interception_probe`。
#
# `DEF_VEG_SNOW`（`MOD_Namelist` 的模块变量，默认 `.true.`）在驱动里按 k=0/1 两档设置。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/icp_diff}
export ICP_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_leafinterception.mod ] || {
  echo "缺少 .bld/mod_leafinterception.mod：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/interception_diff.f90" \
  .bld/MOD_LeafInterception.o .bld/MOD_Const_Physical.o .bld/MOD_Namelist.o \
  -Wl,-dead_strip -o "$WORK/icp"
"$WORK/icp"
cd "$BASE"
cargo run -q -p colm-core --example interception_probe > "$WORK/icp_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['ldew', 'ldew_rain', 'ldew_snow', 'pg_rain', 'pg_snow', 'qintr',
         'qintr_rain', 'qintr_snow']
def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out
f = rows(f'{work}/icp.txt')
r = rows(f'{work}/icp_rust.txt')
assert len(f) == len(r) == 4000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0], (a[0], b[0])
    flags[a[0]] += 1
    for k, x in enumerate(a[1:9]):
        y = b[1 + k]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[names[k]] += 1
if bad:
    print('LEAF_interception_CoLM2014 mismatches / 4000:', dict(bad))
    print('  分档计数:', dict(sorted(flags.items())))
    raise SystemExit(1)
print('LEAF_interception_CoLM2014: all 8 outputs 4000/4000 bitwise identical;'
      f' 分档计数 {dict(sorted(flags.items()))}')
PY
