#!/usr/bin/env bash
# 地面支入参探针：在 `CALL GroundTemperature` **之前**打印 23 个形状最敏感的量。
#
# 背景（第 226 轮）：干窗第 0 步**不执行**叶温求解（PC 与 PFT 两份的 moninobukm 调用、
# 以及 PC 的 Newton 迭代循环，三次插桩都是 0 命中），所以种子只能落在地面支
# （`GroundTemperature` 体内已被差分/形状关掉）或它的**入参**上。本探针就是为后者。
#
# 判据：入参逐位相同而输出仍差 → 残余在模块体内（只剩未逐条过的收缩）；
#       入参已经差 → 往上游追一层（`MOD_GroundFluxes` 的 fseng/fevpg 或辐射项）。
#
# 安全：`trap ... EXIT` 保证无论成败都还原源码、重编内核、打印 git status 与 f48 sync。
set -euo pipefail

BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/gtprobe}
TARGET="vendor/CoLM202X/extends/interception/MOD_Thermal_CanopyPhase_Extended.F90"
BACKUP="$WORK/$(basename "$TARGET").orig"
export NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}

rm -rf "$WORK"; mkdir -p "$WORK/out" "$WORK/run"
cp "$BASE/$TARGET" "$BACKUP"

restore() {
  cd "$BASE"
  if ! diff -q "$BACKUP" "$TARGET" >/dev/null 2>&1; then
    cp "$BACKUP" "$TARGET"
    echo "== 已还原 $TARGET"
  fi
  if [ "${SKIP_REBUILD:-0}" != 1 ]; then
    (cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/rebuild.log" 2>&1) \
      || echo "！！重编失败，见 $WORK/rebuild.log"
  fi
  cd "$BASE"
  git status --short vendor/CoLM202X | sed 's/^/== git status: /'
  python3 oracle/scripts/test_upstream_f48_sync.py | tail -1 | sed 's/^/== /'
}
trap restore EXIT

echo "== 插桩：CALL GroundTemperature 之前打印 23 个入参"
python3 - "$BASE/$TARGET" <<'PYEOF'
import sys
path = sys.argv[1]
src = open(path).read()
anchor = "      CALL GroundTemperature (patchtype,is_dry_lake,lb,nl_soil,deltim,"
assert src.count(anchor) == 1, f"锚点出现 {src.count(anchor)} 次"
stmt = (
    "      WRITE(*,'(A,23E24.16)') 'GTPROBE_IN ', t_soisno(1), wliq_soisno(1), wice_soisno(1), &\n"
    "         scv, snowdp, fsno, t_grnd, t_soil, t_snow, frl, dlrad, sabg, sabg_soil, sabg_snow, &\n"
    "         fseng, fseng_soil, fseng_snow, fevpg, fevpg_soil, fevpg_snow, cgrnd, htvp, emg\n"
)
open(path, "w").write(src.replace(anchor, stmt + anchor, 1))
print("   patched")
PYEOF

echo "== 重编内核（含插桩）"
(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "！！编译失败，见 $WORK/build.log"; exit 3; }

echo "== 先确认补丁**进没进被运行的二进制**（叶温探针都有这一步，本脚本此前漏了）"
strings "$BASE/kernels/default/colm.x" | grep -c 'GTPROBE_IN' | sed 's/^/== 二进制里的 GTPROBE_IN 标记数: /'

echo "== 跑干窗 1 步"
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$WORK/out/'#" \
    -e "s#^   DEF_forcing_namelist.*#   DEF_forcing_namelist = '$WORK/forcing.nml'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$WORK/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$WORK/runtime_unused/'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = 1#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = 0#" \
    "$BASE/oracle/work/CN-Cng/case.nml" > "$WORK/case.nml"
cp "$BASE/oracle/work/CN-Cng/forcing.nml" "$WORK/forcing.nml"
cp -R "$BASE/oracle/work/CN-Cng/out" "$WORK/out"
rm -rf "$WORK/out/CN-Cng/history"
(cd "$WORK/run" && "$BASE/kernels/default/colm.x" "$WORK/case.nml" > "$WORK/kernel.log" 2>&1) \
  || { echo "！！内核运行失败"; exit 4; }

grep 'GTPROBE_IN' "$WORK/kernel.log" > "$WORK/gt_in.txt" || true
echo "== 上游入参：$WORK/gt_in.txt"
head -3 "$WORK/gt_in.txt" || echo "！！没有 GTPROBE_IN 输出 —— 说明这条调用在第 0 步也没执行"
