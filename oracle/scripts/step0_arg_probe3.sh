#!/usr/bin/env bash
# 入口 A 的**第一步**：先判明干窗第 0 步到底执行了哪一份 LeafTemperature。
#
# 背景（第 194 轮）：分别给 `MOD_LeafTemperature_Extended.F90` 与
# `MOD_LeafTemperaturePC_Extended.F90` 插打印、各跑一次，**两条都没输出**；CBL 关、
# `rd_opt`/`rb_opt` 是硬编码 3，所以只能是"第三个实现"。本脚本对三份候选**同时**插一条
# 带各自标记的打印，一次运行即可由"哪条出现"确定实现，再照
# `docs/implementation-verification.md` 的"入口 A 就绪清单"打 13 个实参。
#
# 安全：`trap ... EXIT` 保证无论成败都还原三份源码、重编内核、打印 git status 与 f48 sync。
set -euo pipefail

BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/argprobe3}
export NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}
# 第 196 轮判定：干窗第 0 步**不走**叶温那两支（补丁确实进了二进制 —— `strings` 数到 2 个
# 标记，就是 Makefile 真正编译的那两份 Extended），真正被执行的是
# `main/MOD_GroundFluxes.F90:180` 的 `CALL moninobuk(hu,ht,hq,displax,z0mg,z0hg,z0qg,obu,um,…)`。
# 叶温那两份保留在列表里只作对照；锚点前缀按文件区分。
FILES=(
  "main/MOD_Vars_1DAccFluxes.F90:ACC"
  "extends/interception/MOD_LeafTemperature_Extended.F90:EXT"
  "extends/interception/MOD_LeafTemperaturePC_Extended.F90:PC"
)

rm -rf "$WORK"; mkdir -p "$WORK/out" "$WORK/run"
for spec in "${FILES[@]}"; do
  rel=${spec%%:*}
  mkdir -p "$WORK/backup/$(dirname "$rel")"
  cp "$BASE/vendor/CoLM202X/$rel" "$WORK/backup/$rel"
done

restore() {
  cd "$BASE"
  for spec in "${FILES[@]}"; do
    rel=${spec%%:*}
    if ! diff -q "$WORK/backup/$rel" "$BASE/vendor/CoLM202X/$rel" >/dev/null 2>&1; then
      cp "$WORK/backup/$rel" "$BASE/vendor/CoLM202X/$rel"
      echo "== 已还原 $rel"
    fi
  done
  if [ "${SKIP_REBUILD:-0}" != 1 ]; then
    (cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/rebuild.log" 2>&1) \
      || echo "！！重编失败，见 $WORK/rebuild.log"
  fi
  cd "$BASE"
  git status --short vendor/CoLM202X | sed 's/^/== git status: /'
  python3 oracle/scripts/test_upstream_f48_sync.py | tail -1 | sed 's/^/== /'
}
trap restore EXIT

echo "== 给三份候选各插一条标记打印"
python3 - "$BASE" "${FILES[@]}" <<'PYEOF'
import sys
base, files = sys.argv[1], sys.argv[2:]
for spec in files:
    rel, tag = spec.split(":")
    path = f"{base}/vendor/CoLM202X/{rel}"
    src = open(path).read()
    prefix = ("CALL moninobuk(hgt_u,hgt_t,hgt_q," if tag == "ACC"
              else "CALL moninobukm(hu_,ht_,hq_,")
    if src.count(prefix) != 1:
        print(f"   SKIP {tag}: 锚点 {src.count(prefix)} 次")
        continue
    at = src.index(prefix)
    line_start = src.rindex("\n", 0, at) + 1
    indent = src[line_start:at]
    args = "hgt_u,hgt_t,hgt_q,obu" if tag == "ACC" else "hu_,ht_,hq_,obu"
    stmt = f"{indent}WRITE(*,'(A,4E24.16)') 'ARGPROBE_{tag} ', {args}\n"
    open(path, "w").write(src[:line_start] + stmt + src[line_start:])
    print(f"   patched {tag}")
PYEOF

echo "== 重编内核（含三处标记）"
(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "！！编译失败，见 $WORK/build.log"; exit 3; }

echo "== 先确认补丁**进没进二进制**（第 194 轮三次都"没输出"，必须先排除这一层）"
strings "$BASE/kernels/default/colm.x" | grep -c 'ARGPROBE_' | sed 's/^/== 二进制里的 ARGPROBE 标记数: /'

echo "== 跑干窗 1 步"
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$WORK/out/'#" \
    -e "s#^   DEF_forcing_namelist.*#   DEF_forcing_namelist = '$WORK/forcing.nml'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$WORK/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$WORK/runtime_unused/'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = 1#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = 1800#" \
    -e "s#^   DEF_HIST_FREQ.*#   DEF_HIST_FREQ    = 'TIMESTEP'#" \
    "$BASE/oracle/work/CN-Cng/case.nml" > "$WORK/case.nml"
cp "$BASE/oracle/work/CN-Cng/forcing.nml" "$WORK/forcing.nml"
cp -R "$BASE/oracle/work/CN-Cng/out" "$WORK/out"
rm -rf "$WORK/out/CN-Cng/history"
(cd "$WORK/run" && "$BASE/kernels/default/colm.x" "$WORK/case.nml" > "$WORK/kernel.log" 2>&1) \
  || { echo "！！内核运行失败"; exit 4; }

grep -E 'ARGPROBE_' "$WORK/kernel.log" > "$WORK/probe.txt" || true
echo "== 命中的实现（$WORK/probe.txt）："
cut -c1-60 "$WORK/probe.txt" | head -6
[ -s "$WORK/probe.txt" ] || echo "！！三份候选都没命中 —— 需要继续找第四个实现"
