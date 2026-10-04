#!/usr/bin/env bash
# `MOD_Eroot:eroot` 的两侧逐位差分闭环。
#
# 为什么：`eroot` 的三个输出里只有 `rstfac` 被黄金窗口走到（气孔阻力用）。
# `rootr`（逐层根阻力份额）与 `etrc`（最大可能蒸腾率）只在
# `DEF_USE_PLANTHYDRAULICS = .false.` 那一支里被用，而三个黄金算例全开植物水力
# —— 第 381 轮就是在这个开关关掉时先看到"零的符号"差、第 11 步起放大到水文量上，
# 所以这两个输出必须单独判。
#
# 四种配置：Campbell / van Genuchten × `DEF_RSTFAC` 1/2，各 2000 例
# （1000 组均匀随机 + 1000 组边界/零值 —— 零值那批是刻意加的，
# `etrc = trsmx0*(...)` 在零上的符号随机取值几乎撞不到）。
#
# 上游侧用内核同款选项编译（**不加** `-ffp-contract=off`）；
# 本仓库侧：`cargo run -p colm-core --example eroot_probe`。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/eroot_diff}
export EROOT_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cd "$BASE/vendor/CoLM202X"
[ -f .bld/MOD_Eroot.o ] || {
  echo "缺少 .bld/MOD_Eroot.o：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -fdefault-double-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/eroot_diff.f90" \
  .bld/MOD_Eroot.o .bld/MOD_Hydro_SoilFunction.o .bld/MOD_Namelist.o \
  -Wl,-dead_strip -o "$WORK/erd"
"$WORK/erd"
cd "$BASE"
cargo run -q -p colm-core --example eroot_probe > "$WORK/eroot_rust.txt"
python3 - "$WORK" <<'PY'
import collections
import sys

work = sys.argv[1]
names = [f"rootr{j}" for j in range(1, 11)] + ["etrc", "rstfac"]


def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out


f = rows(f"{work}/eroot.txt")
r = rows(f"{work}/eroot_rust.txt")
assert len(f) == len(r) == 8000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0], (a[0], b[0])
    flags[a[0]] += 1
    for k, x in enumerate(a[1:]):
        y = b[1 + k]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[names[k]] += 1
if bad:
    print("eroot mismatches / 8000:", dict(bad))
    print("  配置计数:", dict(sorted(flags.items())))
    raise SystemExit(1)
print("eroot: all 12 outputs 8000/8000 bitwise identical;"
      f" 配置计数 {dict(sorted(flags.items()))}")
PY
