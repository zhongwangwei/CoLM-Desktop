#!/usr/bin/env bash
# `MOD_AssimStomataConductance:sortin` 的两侧逐位差分闭环。
#
# 为什么：第 330–332 轮把 `stomata` 闭环 498→61，剩下的全在模型 0/1 的 `assim`，
# 而它们唯一的共同路径就是 `sortin`（`stomata` 这条链上唯一没有 FMA 的函数）。
# `sortin` 是 **PRIVATE** ⇒ 走"拷贝+放行"：把模块拷进 `$WORK`、`PRIVATE :: sortin`
# 改 `PUBLIC`，用拷贝编 `.mod`（`-I$WORK` 在 `-I.bld` 之前），vendor 源树不动。
#
# 上游侧用内核同款选项编译（不加 `-ffp-contract=off`），驱动本身加
# `-fwrapv -ffp-contract=off`。本仓库侧：`cargo run -p colm-core --example sortin_probe`。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/srt_diff}
export SRT_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
sed 's/^   PRIVATE :: sortin$/   PUBLIC  :: sortin/' \
  "$BASE/vendor/CoLM202X/main/MOD_AssimStomataConductance.F90" > "$WORK/asc_copy.F90"
grep -q '^   PUBLIC  :: sortin$' "$WORK/asc_copy.F90" || {
  echo "!! 拷贝里没找到 PRIVATE :: sortin（上游改过？）" >&2; exit 3; }
cd "$BASE/vendor/CoLM202X"
BASE_FLAGS=(-O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0
            -fallow-argument-mismatch -fopenmp -ffunction-sections)
gfortran -c "${BASE_FLAGS[@]}" -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$WORK/asc_copy.F90" -J"$WORK" -o "$WORK/asc.o"
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off -fopenmp \
  -I"$WORK" -I.bld -Iinclude -Ishare -Imain \
  "$BASE/oracle/scripts/sortin_diff.f90" \
  "$WORK/asc.o" \
  -Wl,-dead_strip -o "$WORK/srt"
"$WORK/srt"
cd "$BASE"
cargo run -q -p colm-core --example sortin_probe > "$WORK/srt_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
def rows(path):
    out = []
    for line in open(path):
        parts = line.split()
        if parts:
            out.append(parts)
    return out
f = rows(f'{work}/srt.txt')
r = rows(f'{work}/srt_rust.txt')
assert len(f) == len(r) == 3000, (len(f), len(r))
bad = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0], (a[0], b[0])
    for k in range(12):
        if a[1 + k].upper().zfill(16) != b[1 + k].upper().zfill(16):
            name = ('eyy' if k % 2 == 0 else 'pco2y') + str(k // 2 + 1)
            bad[name] += 1
if bad:
    print('sortin mismatches / 3000:', dict(bad))
    raise SystemExit(1)
print('sortin: eyy+pco2y 3000/3000 bitwise identical')
PY
