#!/usr/bin/env bash
# `MOD_AssimStomataConductance:stomata` 的两侧逐位差分闭环。
#
# 为什么：第 329 轮普查发现 `stomata` 有 17 条 FMA、`update_photosyn` 有 8 条，
# 而本仓库 `photosynthesis.rs` 的 `mul_add` 是 0 —— 雪窗 step-1 的
# `f_rstfacsha/sun`/`f_gssun/sha`（气孔阻力/导度）差 1 ULP 的种子就在这里。
# 这条链只有合成输入才能逐位判（黄金窗口只给几个标量诊断）。
#
# `stomata` 在模块里是 PUBLIC，直接链 `.bld` 的对象。上游侧用内核同款选项编译
# （不加 `-ffp-contract=off`），驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`。
# 本仓库侧：`cargo run -p colm-core --example stomata_probe`。
#
# 三种气孔模型（Ball-Berry / Medlyn / WUE）各 1000 例，由驱动写 `MOD_Namelist`
# 的模块开关切换；覆盖值都设成哨兵 `-1`，让实参里的 g1/g0/gradm/binter/lambda 生效。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/stm_diff}
export STM_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_assimstomataconductance.mod ] || {
  echo "缺少 .bld/mod_assimstomataconductance.mod：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/stomata_diff.f90" \
  .bld/MOD_AssimStomataConductance.o .bld/MOD_Namelist.o \
  -Wl,-dead_strip -o "$WORK/stm"
"$WORK/stm"
cd "$BASE"
cargo run -q -p colm-core --example stomata_probe > "$WORK/stm_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['assim', 'respc', 'rst']
def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out
f = rows(f'{work}/stm.txt')
r = rows(f'{work}/stm_rust.txt')
assert len(f) == len(r) == 4000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0], (a[0], b[0])
    flags[a[0]] += 1
    for k, x in enumerate(a[1:4]):
        y = b[1 + k]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[names[k]] += 1
if bad:
    print('stomata mismatches / 4000:', dict(bad))
    print('  模型计数:', dict(sorted(flags.items())))
    raise SystemExit(1)
print('stomata: all 3 outputs 4000/4000 bitwise identical;'
      f' 模型计数 {dict(sorted(flags.items()))}')
PY
