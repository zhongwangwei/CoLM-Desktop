#!/usr/bin/env bash
# `MOD_AssimStomataConductance:sortin` 的两侧逐位差分闭环。
#
# 为什么：第 330–332 轮把 `stomata` 闭环 498→61，剩下的全在模型 0/1 的 `assim`，
# 而它们唯一的共同路径就是 `sortin`（`stomata` 这条链上唯一没有 FMA 的函数）。
# `sortin` 是 **PRIVATE** ⇒ 走"拷贝+放行"：把模块拷进 `$WORK`、`PRIVATE :: sortin`
# 改 `PUBLIC`，用拷贝编 `.mod`（`-I$WORK` 在 `-I.bld` 之前），vendor 源树不动。
#
# 第 335 轮起，拷贝里再注入一个 `dbg_intermediates(9)` 模块数组，把二次拟合分支的
# 中间量（`ac1,ac2,bc1,bc2,cc1,cc2,bterm,aterm,cterm`）导出来 —— 闭环就能指出
# **第一处分叉的量**，不用再靠猜。
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
python3 - "$WORK/asc_copy.F90" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
decl = ("   real(r8) :: dbg_intermediates(9)\n"
        "   PUBLIC :: dbg_intermediates\n\n")
assert "\nCONTAINS\n" in s
s = s.replace("\nCONTAINS\n", "\n" + decl + "CONTAINS\n", 1)
tail = ("      dbg_intermediates(1) = ac1\n"
        "      dbg_intermediates(2) = ac2\n"
        "      dbg_intermediates(3) = bc1\n"
        "      dbg_intermediates(4) = bc2\n"
        "      dbg_intermediates(5) = cc1\n"
        "      dbg_intermediates(6) = cc2\n"
        "      dbg_intermediates(7) = bterm\n"
        "      dbg_intermediates(8) = aterm\n"
        "      dbg_intermediates(9) = cterm\n\n")
marker = "\n   END SUBROUTINE sortin\n"
assert marker in s
s = s.replace(marker, "\n" + tail + "   END SUBROUTINE sortin\n", 1)
open(p, 'w').write(s)
print("injected dbg_intermediates")
PY
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
inter = ['ac1', 'ac2', 'bc1', 'bc2', 'cc1', 'cc2', 'bterm', 'aterm', 'cterm']
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
    # `ic<4` 走首猜分支，核心里那 9 个中间量是**上一次 ic>=4 的旧值**（局部量没赋），
    # 不参与比较；只比输出。
    limit = 21 if a[0] in ('4', '5', '6') else 12
    for k in range(limit):
        if a[1 + k].upper().zfill(16) != b[1 + k].upper().zfill(16):
            if k < 12:
                name = ('eyy' if k % 2 == 0 else 'pco2y') + str(k // 2 + 1)
            else:
                name = 'inter:' + inter[k - 12]
            bad[name] += 1
if bad:
    print('sortin mismatches / 3000:', dict(bad))
    raise SystemExit(1)
print('sortin: eyy+pco2y+9 intermediates 3000/3000 bitwise identical')
PY
