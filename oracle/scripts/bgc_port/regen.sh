#!/bin/bash
# 由上游 Fortran 与其 GIMPLE 重新转写 BGC 状态更新模块。
#   GIMPLE=<目录>：`-fdump-tree-optimized-lineno` 的 *.F90.273t.optimized（见 README）
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
B=$ROOT/vendor/CoLM202X/main/BGC
SRC=$ROOT/crates/colm-core/src
: "${GIMPLE:?set GIMPLE to the directory holding the *.273t.optimized dumps}"
TMP=$(mktemp -d)
draft() { python3 "$HERE/f2rs.py" "$B/$1.F90" "$2" --gimple "$GIMPLE/$1.F90.273t.optimized" > "$TMP/$2.rs"; }
draft MOD_BGC_CNCStateUpdate1 CStateUpdate1
draft MOD_BGC_CNNStateUpdate1 NStateUpdate1
draft MOD_BGC_Soil_BiogeochemNStateUpdate1 SoilBiogeochemNStateUpdate1
python3 "$HERE/mkmod.py" "$SRC/bgc_c_state_update.rs" "$HERE/header_c_state_update.txt" \
  "$TMP/CStateUpdate1.rs" c_state_update1 '`CStateUpdate1`：光合、物候转移、分配与维持呼吸引起的 C 池变化。'
python3 "$HERE/mkmod.py" "$SRC/bgc_n_state_update.rs" "$HERE/header_n_state_update.txt" \
  "$TMP/NStateUpdate1.rs" n_state_update1 '`NStateUpdate1`：物候转移、分配与再转移引起的 N 池变化。'
python3 "$HERE/mkmod.py" "$SRC/bgc_soil_n_state_update.rs" "$HERE/header_soil_n_state_update.txt" \
  "$TMP/SoilBiogeochemNStateUpdate1.rs" soil_biogeochem_n_state_update1 '`SoilBiogeochemNStateUpdate1`：沉降、固氮、矿化/固持与植物吸收引起的矿质 N 变化。'
rm -rf "$TMP"
