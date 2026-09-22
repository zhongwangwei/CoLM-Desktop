#!/usr/bin/env bash
# 入口 A 的第二条线：**叶温 Newton 迭代内部**的逐次转储（用于验证"调用时机/顺序"假设）。
#
# 为什么需要重做：`/tmp/gf/iterprobe/{fort,rust}_iter2.txt`（第 140 轮前后）代码状态过时、
# 两侧标记格式也不同（上游 `Q 1_tl`/`Q 2_tl`，Rust 只有 `Q2 …`），无法逐次对齐。
#
# 本脚本在上游侧给出**逐迭代**打印（含迭代号 `it` 与关键量），产物
# `/tmp/gf/iterprobe2/fort_iter.txt` 可直接与 Rust 侧同格式的转储逐行比。
# Rust 侧对应 `crates/colm-core/src/leaf_temperature.rs` 的迭代循环，打印同一组量。
#
# 安全：`trap ... EXIT` 保证无论成败都还原源码、重编内核、打印 git status 与 f48 sync。
set -euo pipefail

BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/iterprobe2}
TARGET="vendor/CoLM202X/extends/interception/MOD_LeafTemperaturePC_Extended.F90"
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

echo "== 插桩：在 DO WHILE (it .le. itmax) 内逐迭代打印"
python3 - "$BASE/$TARGET" <<'PYEOF'
import sys
path = sys.argv[1]
src = open(path).read()
anchor = "      DO WHILE (it .le. itmax)\n"
assert src.count(anchor) == 1, f"循环锚点出现 {src.count(anchor)} 次"
stmt = ("         WRITE(*,'(A,I3,6E24.16)') 'ITPROBE ', it, tl, fsenl, fevpl, obu, ustar, cfw\n")
open(path, "w").write(src.replace(anchor, anchor + stmt, 1))
print("   patched")
PYEOF

echo "== 重编内核（含插桩）"
(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "！！编译失败，见 $WORK/build.log"; exit 3; }

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

grep 'ITPROBE' "$WORK/kernel.log" > "$WORK/fort_iter.txt" || true
echo "== 上游逐迭代转储：$WORK/fort_iter.txt"
head -6 "$WORK/fort_iter.txt" || echo "！！没有 ITPROBE 输出（可能这条叶温路径未被执行，先按入口 A 的候选清单判定）"
