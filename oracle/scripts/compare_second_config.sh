#!/usr/bin/env bash
# 第二个配置（Campbell 土水 + 关掉 VSF）的两侧对照。
#
# 三个黄金算例都走 van Genuchten + VSF，所以这条经典 Richards 支路在本机
# 长期没有端到端信号（``soilwater``/``water_2014`` 的收缩只能从 dump 读形状）。
# 这个脚本把算例复制一份、往 ``&nl_colm`` 里注入两行开关，两侧各跑一遍，
# 再用 ``golden-compare`` 以上游为参照报出超容差变量数。
#
# 用法: oracle/scripts/compare_second_config.sh <CN-Cng|CN-Cng-wet|US-NR1-snow>
# 实测（2026）：干 16 / 湿 66 / 雪 79，与同窗口的黄金配置 17 / 68 / 79 齐平；
# 后续各轮修完形状后干/湿都归 **0**，雪窗 79（本机无 US-NR1 的 forcing，跑不了）。
# 见 docs/implementation-verification.md 的 "rss 那道门修完之后" 一节。
#
# 退出码：超容差变量数 != 0 → 1（可以直接挂进 compare_all.sh / CI）。
#
# 第 380 轮补：本脚本原先**没**把 forcing.nml 里的 PLUMBER2 挂载点改掉，
# 而算例的 `DEF_forcing_namelist` 指向仓库里那份（挂载点还是老的 `/Volumes/Data01`）
# ⇒ 本机从挂载点变更起就一直跑不起来；又因为它要位置参数、被 compare_all.sh 当
# "整例对照"跳过，这个整例对照静默失效了很久。同第 351 轮给
# `compare_hourly_window.sh` 补的那一步：**只改拷贝**，仓库里的
# `oracle/work/*/forcing.nml`（由 colm-forcing 生成）一律不动。
set -euo pipefail
set -e
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export NETCDF_DIR=/opt/homebrew/opt/netcdf
case=$1
d=/tmp/gf/camp_$case
rm -rf $d; mkdir -p $d/run
cp -R $BASE/oracle/work/$case $d/case
python3 - <<PY
p='$d/case/case.nml'
s=open(p).read()
old="   DEF_CASE_NAME = '$case'"
assert old in s, 'case name line not found'
s=s.replace(old, old+"\n   DEF_USE_Campbell_SOIL_MODEL   = .true.\n   DEF_USE_VariablySaturatedFlow = .false.", 1)
# 两侧都读**拷贝**里那份 forcing.nml（下面已经把它的挂载点改到仓库自带目录）
import re
s=re.sub(r"^   DEF_forcing_namelist.*$", "   DEF_forcing_namelist = '$d/case/forcing.nml'", s, flags=re.M)
assert "DEF_forcing_namelist = '$d/case/forcing.nml'" in s, 'forcing namelist line not found'
open(p,'w').write(s)
PY
# PLUMBER2 挂载点变了（/Volumes/Data01 -> 仓库自带 examples/Forcing）：只改这份拷贝
sed -i '' "s#^   DEF_dir_forcing.*#   DEF_dir_forcing              = '$BASE/examples/Forcing/'#" $d/case/forcing.nml
# 两侧**共用同一棵输出树**（`<case>/out/<case>/restart`）：上游从
# `DEF_dir_restart/ParaOpt/<case>_baseflow_w180_s90.nc` 读 `scale_baseflow`，
# 本仓库从同一路径读。分成两棵树会让这类文件只有一侧看得见。
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$d/case/out/'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$d/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$d/runtime_unused/'#" \
    $d/case/case.nml > $d/case.nml
rm -rf $d/case/out/$case/history
( cd $d/run && $BASE/kernels/default/colm.x $d/case.nml > $d/f.log 2>&1 ) \
  || { echo "!! $case: kernel run failed"; tail -5 $d/f.log; exit 3; }
cargo run -q --manifest-path $BASE/Cargo.toml -p colm-runtime --bin colm-rs -- $d/case --land-cover igbp --restart-out $d/rust_restart.nc --history-dir $d > $d/r.log 2>&1 \
  || { echo "!! $case: rust run failed"; tail -5 $d/r.log; exit 3; }
G=$(ls $d/case/out/$case/history/*.nc); R=$(ls $d/colm-rs_hist*.nc)
cargo run -q --manifest-path $BASE/Cargo.toml -p oracle --bin golden-compare -- $G $R --tolerances $BASE/oracle/tolerances.toml > $d/cmp.txt 2>&1 || true
n=$(python3 - "$d/cmp.txt" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
print(sum(int(m) for m in re.findall(r': (\d+)/\d+ values outside tolerance', text)))
PY
)
echo "=== $case (Campbell, VSF off): $n variable(s) outside tolerance"
[ "$n" = "0" ] || { grep "failures by tier" $d/cmp.txt; exit 1; }
