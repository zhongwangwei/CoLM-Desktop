#!/usr/bin/env bash
# 用一条 namelist 开关隔离「干窗第 0 步那 1 ULP 差异来自哪个支路」。
#
# 复制 CN-Cng、注入指定的开关、两侧各跑 1 步（TIMESTEP 历史），报出逐位不同的
# 变量与相对量级。
#
# 用法: oracle/scripts/compare_flag_isolated.sh <tag> "<NAMELIST 行>"
#   e.g. compare_flag_isolated.sh nophs      "DEF_USE_PLANTHYDRAULICS = .false."
#        compare_flag_isolated.sh vegsnowoff "DEF_VEG_SNOW = .false."
#
# 这是**诊断探针**（用开关分区），没有通过/失败判据，所以 `compare_all.sh` 仍然跳过它。
#
# 实测（2026，**当时种子还在**）：默认 34 个变量、最大相对差 ~6e-15；
#   PHS 关掉 37 个（同量级，说明种子不在 PHS）；
#   VEG_SNOW 关掉 18 个、叶面那一组降到**恰好 1 ULP**（1.2e-16），
#   只剩 f_wliq_soisno 一层 4e-13 —— 种子在水分/能量共用的那一环。
# 见 docs/implementation-verification.md 的「用开关把第 0 步的差异分区」一节。
#
# 第 380 轮复测：那些种子都已修掉，`compare_flag_isolated.sh default` 现在是
# `differing vars: 0` —— 这条探针从此只在**新的** 1 ULP 种子出现时才需要拿出来用。
set -euo pipefail
set -e
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export NETCDF_DIR=/opt/homebrew/opt/netcdf
tag=$1; shift
d=/tmp/gf/flag_$tag
steps=1
total=$((1800*steps)); sec=$((total%86400)); dd=$((1 + total/86400))
rm -rf $d; mkdir -p $d/run
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$d/out/'#" \
    -e "s#^   DEF_forcing_namelist.*#   DEF_forcing_namelist = '$d/forcing.nml'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$d/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$d/runtime_unused/'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = $dd#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = $sec#" \
    -e "s#^   DEF_HIST_FREQ.*#   DEF_HIST_FREQ    = 'TIMESTEP'#" \
    $BASE/oracle/work/CN-Cng/case.nml > $d/case.nml
python3 - "$d" "$@" <<'PY'
import sys
d=sys.argv[1]; flags=sys.argv[2:]
p=d+'/case.nml'; s=open(p).read()
old="   DEF_CASE_NAME = 'CN-Cng'"
s=s.replace(old, old+"\n"+"".join("   "+f+"\n" for f in flags),1)
open(p,'w').write(s)
PY
cp $BASE/oracle/work/CN-Cng/forcing.nml $d/forcing.nml
# PLUMBER2 挂载点变了（/Volumes/Data01 -> 仓库自带 examples/Forcing）：只改这份拷贝
# （第 380 轮补：本脚本原先少了这一步，两侧都因为 forcing 打不开而跑不起来；
#   同第 351 轮给 compare_hourly_window.sh 补的那一步。）
sed -i '' "s#^   DEF_dir_forcing.*#   DEF_dir_forcing              = '$BASE/examples/Forcing/'#" $d/forcing.nml
cp -R $BASE/oracle/work/CN-Cng/out $d/out; rm -rf $d/out/CN-Cng/history
( cd $d/run && $BASE/kernels/default/colm.x $d/case.nml > $d/f.log 2>&1 ) \
  || { echo "!! kernel run failed"; tail -5 $d/f.log; exit 3; }
cargo run -q --manifest-path $BASE/Cargo.toml -p colm-runtime --bin colm-rs -- $d --land-cover igbp --restart-out $d/rust_restart.nc --history-dir $d > $d/r.log 2>&1 \
  || { echo "!! rust run failed"; tail -5 $d/r.log; exit 3; }
python3 - "$d" <<'PY'
import numpy as np, netCDF4 as nc, glob, sys
d=sys.argv[1]
f=nc.Dataset(glob.glob(d+'/out/CN-Cng/history/*.nc')[0]); r=nc.Dataset(d+'/colm-rs_hist_2008-01.nc')
rows=[]
for n in sorted(set(f.variables)&set(r.variables)):
    a=np.ma.filled(f.variables[n][:].astype('f8'),np.nan); b=np.ma.filled(r.variables[n][:].astype('f8'),np.nan)
    if a.shape!=b.shape: continue
    if np.array_equal(a.view(np.int64),b.view(np.int64)): continue
    rows.append((float(np.nanmax(np.abs(a-b)/np.maximum(np.abs(a),1e-30))), n))
print('differing vars:',len(rows))
for rel,n in sorted(rows,reverse=True)[:6]: print(f'  {n:16s} rel={rel:.2e}')
PY
