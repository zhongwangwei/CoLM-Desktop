#!/usr/bin/env bash
# 短窗口的**小时对齐**内核/Rust/golden 三方逐位比对。
#
# 为什么需要它：`dry_ts.sh` 把 `DEF_HIST_FREQ` 改成 `'TIMESTEP'`，而历史量是
# **按输出区间累加/平均**的 —— 30 min 窗口与算例自带的 `'HOURLY'`（60 min 窗口）
# 在**同一时刻**给出不同的值（第 295 轮实测：`time[0]` 相同，`f_zwt` 一个是
# `8.756e-4`、另一个是 `1.751e-3`）。所以 `dry_ts.sh` 的"第几步"分歧**不能**
# 直接当成小时级的状态分歧。要判状态，必须让两侧都用算例自己的 `HOURLY`。
#
# 本脚本在同一份算例上跑 N 个小时（默认 8），两侧都用 `HOURLY`，然后逐记录比
# 内核 / Rust / 存储 golden 的逐位不等元素数。用法：
#   bash oracle/scripts/compare_hourly_window.sh [hours]
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOURS=${1:-8}
W=${WORK:-/tmp/gf/hourly}
export NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}

rm -rf "$W"; mkdir -p "$W/run"
# 起止时间按整点算：end_sec = HOURS*3600。
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$W/out/'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$W/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$W/runtime_unused/'#" \
    -e "s#^   DEF_forcing_namelist.*#   DEF_forcing_namelist = '$W/forcing.nml'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = 1#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = $((HOURS * 3600))#" \
    "$BASE/oracle/work/CN-Cng/case.nml" > "$W/case.nml"
grep -q "DEF_HIST_FREQ    = 'HOURLY'" "$W/case.nml" || {
  echo "!! 算例的 HIST_FREQ 不是 HOURLY，本脚本的窗口对齐假设不成立" >&2; exit 2; }
cp "$BASE/oracle/work/CN-Cng/forcing.nml" "$W/forcing.nml"
# PLUMBER2 挂载点变了：只改这份拷贝（第 351 轮补，原先本脚本没有这一步，本机跑不起来）
sed -i '' "s#/Volumes/Data01/Data/PLUMBER2s/Forcing/#$BASE/examples/Forcing/#" "$W/forcing.nml"
cp -R "$BASE/oracle/work/CN-Cng/out" "$W/out"
rm -rf "$W/out/CN-Cng/history"
( cd "$W/run" && "$BASE/kernels/default/colm.x" "$W/case.nml" > "$W/f.log" 2>&1 ) \
  || { echo "!! kernel run failed"; tail -5 "$W/f.log"; exit 3; }
(cd "$BASE" && cargo run -q -p colm-runtime --bin colm-rs -- "$W" --land-cover igbp \
    --restart-out "$W/rust_restart.nc" --history-dir "$W" > "$W/r.log" 2>&1) \
  || { echo "!! rust run failed"; tail -5 "$W/r.log"; exit 3; }

python3 - "$W" "$BASE" "$HOURS" <<'PY'
import glob
import sys

import netCDF4 as nc
import numpy as np

work, base, hours = sys.argv[1], sys.argv[2], int(sys.argv[3])
k = nc.Dataset(f"{work}/out/CN-Cng/history/CN-Cng_hist_2008-01.nc")
r = nc.Dataset(glob.glob(f"{work}/colm-rs_hist*")[0])
g = nc.Dataset(f"{base}/oracle/golden/CN-Cng_hist_2008-01.nc")
n = int(k.variables["f_zwt"].shape[0])
print(f"records: kernel {n} (hours={hours})")
names = sorted(set(k.variables) & set(r.variables) & set(g.variables))


def take(ds, name, i):
    return np.ma.filled(ds.variables[name][i].astype("f8"), np.nan).ravel()


first_kr = first_kg = None
for i in range(n):
    dk = dr = dg = 0
    for name in names:
        if k.variables[name].ndim < 1 or k.variables[name].shape[0] != n:
            continue
        a, b, c = take(k, name, i), take(r, name, i), take(g, name, i)
        dk += int(np.sum((a != b) & ~(np.isnan(a) & np.isnan(b))))
        dr += int(np.sum((a != c) & ~(np.isnan(a) & np.isnan(c))))
        dg += int(np.sum((b != c) & ~(np.isnan(b) & np.isnan(c))))
    if dk and first_kr is None:
        first_kr = i
    if dr and first_kg is None:
        first_kg = i
    print(f"  rec {i:2d}: kernel!=rust {dk:5d}   kernel!=golden {dr:5d}   rust!=golden {dg:5d}")
print(f"first kernel!=rust record: {first_kr}")
print(f"first kernel!=golden record: {first_kg}")
PY
