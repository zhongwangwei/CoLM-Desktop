#!/usr/bin/env bash
# 入口 A：把干窗第 0 步**近地层相似性调用的实参**在对文件里打出来。
#
# 为什么要脚本：这需要临时改 `extends/interception/MOD_LeafTemperature_Extended.F90`
# 并重编内核。手工做一旦中断，vendor 会被留在改动状态（f48 sync 会红）。这里用
# `trap ... EXIT` 保证**无论成败**都还原源码、并把内核重编回未打补丁的版本。
#
# 用法：  bash oracle/scripts/step0_arg_probe.sh
# 产物：  /tmp/gf/argprobe/fort_args.txt（上游侧 13 个实参 + 10 个输出）
#         后续按 docs/implementation-verification.md 的"入口 A 就绪清单"与 Rust 侧逐位比。
set -euo pipefail

BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/argprobe}
# 目标文件由 namelist 决定：CN-Cng 用 `USE_SITE_pctpfts = .true.` → 走 **PC** 叶温
# （`MOD_LeafTemperaturePC_Extended.F90`），不是 `MOD_LeafTemperature_Extended.F90`。
# 第 194 轮第一次跑就是打了错文件，内核里一条 ARGPROBE 都没出现。
TARGET=${TARGET:-vendor/CoLM202X/extends/interception/MOD_LeafTemperaturePC_Extended.F90}
BACKUP="$WORK/$(basename "$TARGET").orig"
export NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}

rm -rf "$WORK"; mkdir -p "$WORK"
cp "$BASE/$TARGET" "$BACKUP"

restore_kernel() {
  cd "$BASE"
  if ! diff -q "$BACKUP" "$TARGET" >/dev/null 2>&1; then
    cp "$BACKUP" "$TARGET"
    echo "== 源码已还原：$TARGET"
  fi
  if [ "${SKIP_REBUILD:-0}" != 1 ]; then
    echo "== 重编内核回未打补丁版本（build_kernel.sh default）"
    (cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/rebuild.log" 2>&1) \
      || echo "！！重编失败，见 $WORK/rebuild.log —— 下一轮**先**重跑 build_kernel.sh default"
  fi
  cd "$BASE"
  git status --short "$TARGET" | sed 's/^/== git status: /'
  python3 oracle/scripts/test_upstream_f48_sync.py | tail -1 | sed 's/^/== /'
}
trap restore_kernel EXIT

echo "== 打补丁：在 CALL moninobukm 前后插 WRITE(77,…)"
python3 - "$BASE/$TARGET" <<'PY'
import re, sys
path = sys.argv[1]
src = open(path).read()
anchor = "CALL moninobukm(hu_,ht_,hq_,displa_lays(toplay),z0mv,z0hv,z0qv,obu,um, &"
assert src.count(anchor) == 1, f"锚点出现 {src.count(anchor)} 次，脚本需更新"
dump_in = (
    "            WRITE(*,'(A,13E24.16)') 'ARGPROBE_IN ', hu_,ht_,hq_,displa_lays(toplay),"
    "z0mv,z0hv,z0qv,obu,um,displa_lay(toplay),z0m_lay(toplay),htop_lay(toplay)\n"
)
src = src.replace(anchor, dump_in + "            " + anchor, 1)
# 输出打印：插在 ELSE 支那次调用的收尾之后（用带缩进的实参尾行 + ENDIF 定位，
# 保证唯一 —— 只写 "ENDIF\n! Aerodynamic resistance" 会命中 2 处）
tail = " " * 28 + "htop_lay(toplay),fmtop,fm,fh,fq,fht,fqt,phih)\n         ENDIF\n"
assert src.count(tail) == 1, f"收尾锚点出现 {src.count(tail)} 次，脚本需更新"
dump_out = tail + "            WRITE(*,'(A,10E24.16)') 'ARGPROBE_OUT', ustar,fh2m,fq2m,fmtop,fm,fh,fq,fht,fqt,phih\n"
src = src.replace(tail, dump_out, 1)
open(path, "w").write(src)
print("   patched")
PY

echo "== 重编内核（含补丁）"
(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "！！编译失败，见 $WORK/build.log"; exit 3; }

echo "== 跑干窗 1 步（内核侧 77 号文件重定向到 $WORK/fort_args.txt）"
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$WORK/out/'#" \
    -e "s#^   DEF_forcing_namelist.*#   DEF_forcing_namelist = '$WORK/forcing.nml'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$WORK/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$WORK/runtime_unused/'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = 1#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = 0#" \
    "$BASE/oracle/work/CN-Cng/case.nml" > "$WORK/case.nml"
mkdir -p "$WORK/out" "$WORK/run"
cp "$BASE/oracle/work/CN-Cng/forcing.nml" "$WORK/forcing.nml"
cp -R "$BASE/oracle/work/CN-Cng/out" "$WORK/out"
rm -rf "$WORK/out/CN-Cng/history"
(cd "$WORK/run" && "$BASE/kernels/default/colm.x" "$WORK/case.nml" \
    > "$WORK/kernel.log" 2>&1) || { echo "！！内核运行失败，见 $WORK/kernel.log"; exit 4; }
# 走 stdout（被 kernel.log 捕获）比开文件稳：内核自己可能占用 77 号单元
grep -E 'ARGPROBE_(IN|OUT)' "$WORK/kernel.log" > "$WORK/fort_args.txt" || true
echo "== 上游实参：$WORK/fort_args.txt"
head -4 "$WORK/fort_args.txt" || echo "！！kernel.log 里没有 ARGPROBE 输出 —— 说明那两条支没被执行，先查分支"
echo "== 下一步：Rust 侧在 leaf_temperature.rs 的 MoninObukhovInput 构造处打同一组 13 个量，"
echo "   与上面逐位比较（判据见 docs/implementation-verification.md 的『入口 A 就绪清单』）。"
