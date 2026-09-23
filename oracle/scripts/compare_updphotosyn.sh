#!/usr/bin/env bash
# `MOD_AssimStomataConductance:update_photosyn` 的内核侧参考值（第 293 轮）。
#
# 直接链 `.bld` 的产线对象调 `update_photosyn`，打印 `assim`/`respc`；
# 值被 `crates/colm-core/src/photosynthesis_tests.rs` 的
# `hydraulic_photosynthesis_update_matches_mod_assim_stomata_conductance` 钉住。
#
# 驱动按纪律加 `-fwrapv -ffp-contract=off`；配置与 Rust 测试对齐：
#   DEF_USE_WUEST = T、DEF_USE_MEDLYNST = F、c3c4 = 1。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/upddiff}
export NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}
NETCDFF_DIR=${NETCDFF_DIR:-/opt/homebrew/opt/netcdf-fortran}
rm -rf "$WORK"; mkdir -p "$WORK"

cd "$BASE/vendor/CoLM202X"
[ -f .bld/mod_assimstomataconductance.mod ] || {
  echo "缺少 .bld：先跑 ./oracle/scripts/build_kernel.sh default" >&2; exit 2; }
gfortran -O2 -fdefault-real-8 -ffree-form -cpp -ffree-line-length-0 \
  -fallow-argument-mismatch -fwrapv -ffp-contract=off \
  -I.bld -Iinclude -Imain -Ishare -J"$WORK" \
  -c "$BASE/oracle/scripts/updphotosyn_diff.f90" -o "$WORK/drv.o"
mpifort "$WORK/drv.o" $(ls .bld/*.o | grep -v '/CoLM.o$') \
  -L"$NETCDF_DIR/lib" -L"$NETCDFF_DIR/lib" -lnetcdff -lnetcdf -llapack -lblas \
  -o "$WORK/upd"
"$WORK/upd"
