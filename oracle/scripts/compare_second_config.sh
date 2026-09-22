#!/usr/bin/env bash
# 第二个配置（Campbell 土水 + 关掉 VSF）的两侧对照。
#
# 三个黄金算例都走 van Genuchten + VSF，所以这条经典 Richards 支路在本机
# 长期没有端到端信号（``soilwater``/``water_2014`` 的收缩只能从 dump 读形状）。
# 这个脚本把算例复制一份、往 ``&nl_colm`` 里注入两行开关，两侧各跑一遍，
# 再用 ``golden-compare`` 以上游为参照报出超容差变量数。
#
# 用法: oracle/scripts/compare_second_config.sh <CN-Cng|CN-Cng-wet|US-NR1-snow>
# 实测（2026）：干 16 / 湿 66 / 雪 79，与同窗口的黄金配置 17 / 68 / 79 齐平。
# 见 docs/implementation-verification.md 的 "rss 那道门修完之后" 一节。
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
open(p,'w').write(s)
PY
# 两侧**共用同一棵输出树**（`<case>/out/<case>/restart`）：上游从
# `DEF_dir_restart/ParaOpt/<case>_baseflow_w180_s90.nc` 读 `scale_baseflow`，
# 本仓库从同一路径读。分成两棵树会让这类文件只有一侧看得见。
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$d/case/out/'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$d/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$d/runtime_unused/'#" \
    $d/case/case.nml > $d/case.nml
rm -rf $d/case/out/$case/history
(cd $d/run && $BASE/kernels/default/colm.x $d/case.nml > f.log 2>&1)
cargo run -q --manifest-path $BASE/Cargo.toml -p colm-runtime --bin colm-rs -- $d/case --land-cover igbp --restart-out $d/rust_restart.nc --history-dir $d > $d/r.log 2>&1
G=$(ls $d/case/out/$case/history/*.nc); R=$(ls $d/colm-rs_hist*.nc)
cargo run -q --manifest-path $BASE/Cargo.toml -p oracle --bin golden-compare -- $G $R --tolerances $BASE/oracle/tolerances.toml > $d/cmp.txt 2>&1 || true
echo "=== $case (Campbell, VSF off)"; head -1 $d/cmp.txt; grep "failures by tier" $d/cmp.txt
