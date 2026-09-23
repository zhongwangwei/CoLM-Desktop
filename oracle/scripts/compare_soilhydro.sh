#!/usr/bin/env bash
# `MOD_Hydro_SoilFunction`（van Genuchten / Campbell 保持曲线）的两侧逐位差分。
#
# 上游侧用内核真实选项编译模块（**不加** `-ffp-contract=off` —— 要的就是 GCC 的
# 默认收缩），驱动本身按仓库纪律加 `-fwrapv -ffp-contract=off`。
# 本仓库侧：`cargo run -p colm-core --example soil_hydro_fn_probe`，同一串 LCG。
#
# 为什么单列一个脚本：`soil_hcap_cond`（**热**参数）早有闭环，而这一支
# （**水**参数：`smp`/`hk`/`vliq` 三者的互相反演）一直只有读源码对形状。
# 干窗第 3 步的 restart 差异（`wice_soisno` 与 `hk`）正好落在这一族上，
# 所以把它补成可执行的判据。
#
# 报出三个输出：`psi`（`soil_psi_from_vliq`）、`hk`（`soil_hk_from_psi`）、
# `vl`（`soil_vliq_from_psi`）；三个调用**串联**（同一个 `psi` 喂下游）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/hf_diff}
export HF_OUT="$WORK"
rm -rf "$WORK"; mkdir -p "$WORK"
cat > "$WORK/namelist_stub.F90" <<'EOF'
MODULE MOD_Namelist
  IMPLICIT NONE
  LOGICAL :: DEF_USE_Campbell_SOIL_MODEL = .FALSE.
END MODULE MOD_Namelist
EOF
cd "$BASE/vendor/CoLM202X"
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  "$WORK/namelist_stub.F90" -J"$WORK" -o "$WORK/stub.o"
gfortran -c -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  main/HYDRO/MOD_Hydro_SoilFunction.F90 -J"$WORK" -o "$WORK/hf_mod.o"
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I"$WORK" -I.bld -Iinclude -Imain -Ishare \
  "$BASE/oracle/scripts/soil_hydro_fn_diff.f90" \
  "$WORK/hf_mod.o" "$WORK/stub.o" -o "$WORK/hf"
"$WORK/hf"
cd "$BASE"
cargo run -q -p colm-core --example soil_hydro_fn_probe > "$WORK/hf_rust.txt"
python3 - "$WORK" <<'PY'
import collections, sys
work = sys.argv[1]
names = ['psi', 'hk', 'vl']
def rows(path):
    out = []
    for line in open(path):
        parts = line.replace('Z', '').split()
        if not parts:
            continue
        out.append(parts)
    return out
f = rows(f'{work}/hf.txt')
r = rows(f'{work}/hf_rust.txt')
assert len(f) == len(r) == 10000, (len(f), len(r))
bad = collections.Counter()
flags = collections.Counter()
for a, b in zip(f, r):
    assert a[0] == b[0] and a[1] == b[1], (a[:2], b[:2])
    flags[(a[0], a[1])] += 1
    for k, x in enumerate(a[2:5]):
        y = b[2 + k]
        if x.upper().zfill(16) != y.upper().zfill(16):
            bad[names[k]] += 1
if bad:
    print('MOD_Hydro_SoilFunction mismatches / 10000:', dict(bad))
    raise SystemExit(1)
print('MOD_Hydro_SoilFunction: all 3 outputs 10000/10000 bitwise identical;'
      f' 分支分布 {dict(flags)}')
PY
