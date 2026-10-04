#!/usr/bin/env bash
# `MOD_AssimStomataConductance:update_photosyn` 的两侧逐位差分闭环（第 7 个）。
#
# 为什么：第 329 轮普查把这条链的 25 条 FMA 分成两段，`stomata` 17 条（已有闭环
# `compare_stomata.sh`）、`update_photosyn` 8 条。第 336 轮实测 `stomata` 那批形状
# 在黄金窗口上是**恒等**的（C3 植被 `c3_fraction==1` 时 `mul_add` 退化，
# Medlyn/Ball-Berry 在默认 WUE 下是死分支），所以"改 `stomata` 会不会动黄金窗口"
# 这个问题已经被回答；但 `update_photosyn` 那 8 条一直没有判据。这个闭环补上。
#
# `update_photosyn` 在模块里是 PUBLIC，直接链 `.bld` 的对象（不用"拷贝+放行"）。
#
# **别和第 293 轮的 `compare_updphotosyn.sh` 混**：那个是"内核侧参考值 +
# `photosynthesis_tests.rs` 里钉死"的单配置（WUE、c3c4=1）金值测试；这个是把
# 3000 例随机输入逐位比到底的差分闭环，覆盖 c4/Medlyn/Ball-Berry 三个分支。
# 上游侧用内核同款选项编译（**不加** `-ffp-contract=off`），驱动本身按仓库纪律加
# `-fwrapv -ffp-contract=off`。
# 本仓库侧：`cargo run -p colm-core --example update_photosyn_probe`。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/upsyn_diff}
export UPSYN_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_assimstomataconductance.mod ] || {
  echo "缺少 .bld/mod_assimstomataconductance.mod：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/update_photosyn_diff.f90" \
  .bld/MOD_AssimStomataConductance.o .bld/MOD_Namelist.o \
  -Wl,-dead_strip -o "$WORK/upsyn"
"$WORK/upsyn"
cd "$BASE"
cargo run -q -p colm-core --example update_photosyn_probe > "$WORK/upsyn_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['assim', 'respc']
def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out
f = rows(f'{work}/upsyn.txt')
r = rows(f'{work}/upsyn_rust.txt')
assert len(f) == len(r) == 3000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0], (a[0], b[0])
    flags[a[0]] += 1
    for k, x in enumerate(a[1:3]):
        y = b[1 + k]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[names[k]] += 1
if bad:
    print('update_photosyn mismatches / 3000:', dict(bad))
    print('  模型计数:', dict(sorted(flags.items())))
    raise SystemExit(1)
print('update_photosyn: both outputs 3000/3000 bitwise identical;'
      f' 模型计数 {dict(sorted(flags.items()))}')
PY
