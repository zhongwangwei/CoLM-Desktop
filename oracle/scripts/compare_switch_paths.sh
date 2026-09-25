#!/usr/bin/env bash
# 非默认运行时开关支路的**首差定位**（诊断，不判通过/失败）。
#
# 三个黄金算例都跑默认配置，所以 `DEF_VEG_SNOW = .false.`、
# `DEF_USE_PLANTHYDRAULICS = .false.` 这些开关的**另一支**在本机没有端到端信号。
# 这个脚本把每一支都在同一个算例上真跑一遍（短窗口 + TIMESTEP 历史），
# 再逐记录找"第一个出现**量级差**的步"，把种子钉到具体的步与量上。
#
# 为什么不用整月 + 容差当判据：整例是混沌的，**任何** 1 ULP 种子在一个月里都会被
# 放大成几十个超容差变量（实测 PHS 关掉：早期 1e-16 的种子 → 52 个变量超差），
# 于是"超没超容差"分不出"形状对不对"。逐记录首差才是可行动的信号。
#
#   `DEF_SPLIT_SOILSNOW = .true.` **故意不在表里**：本仓库的运行时只装配非分裂的
#   土/雪柱，`colm-rs` 遇到这个开关会直接报错退出（`assembly.rs` 把
#   `use_split_soil_snow` 钉成 false），它是**已知未移植**的支路。
#
# 用法:
#   oracle/scripts/compare_switch_paths.sh [<case>] [<steps>]     # 默认 CN-Cng 36 步
#
# 实测（2026-09，CN-Cng 2008-01 前 36 步）：
#   `DEF_VEG_SNOW = .false.`           ：rec 0–7 **逐位相同**，rec 8 首个量级差
#                                       —— `f_xerr` 1 个值、1.26e-16。
#   `DEF_USE_PLANTHYDRAULICS = .false.`：rec 0–10 只差**零的符号**（`f_etr`/
#                                       `f_etrsha`/`f_etrsun`，2–3 个值、maxabs=0），
#                                       rec 11 首个量级差 —— `f_zwt` 3.0e-06、
#                                       `f_wliq_soisno` 5.0e-09、`f_h2osoi` 2.9e-10
#                                       ⇒ 种子在 `soilwater` 里 PHS 关掉那一支的
#                                       根吸水/ET 分配上。
#   `DEF_Runoff_SCHEME = 0`（TOPMODEL）/ `= 2`（XinAnJiang）：
#                                       **前 36 条记录逐位完全相同**（连零的符号
#                                       都没有差）—— 黄金算例走的是 3（SimpleVIC），
#                                       这两档在本机是第一次拿到端到端信号，
#                                       `oracle/scripts/` 里也没有 runoff 的闭环。
# 前两条还是**未修的种子**，所以本脚本不进 `compare_all.sh` 的门禁。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
export NETCDF_DIR=/opt/homebrew/opt/netcdf
case=${1:-CN-Cng}
steps=${2:-36}

SWITCHES=(
  "vegsnow_off|DEF_VEG_SNOW = .false."
  "phs_off|DEF_USE_PLANTHYDRAULICS = .false."
  # 产流方案：黄金算例是 3（SimpleVIC），0（TOPMODEL）与 2（XinAnJiang）
  # 在本机从来没有端到端信号，`oracle/scripts/` 里也没有 runoff 的闭环。
  "runoff_topmodel|DEF_Runoff_SCHEME = 0"
  "runoff_xinanjiang|DEF_Runoff_SCHEME = 2"
)

patch() {  # $1 = 要打的 namelist 路径；结果写到 stdout
  local src=$1
  sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$d/case/out/'#" \
      -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$d/rawdata_unused/'#" \
      -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$d/runtime_unused/'#" \
      -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = $end_day#" \
      -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = $end_sec#" \
      -e "s#^   DEF_HIST_FREQ.*#   DEF_HIST_FREQ    = 'TIMESTEP'#" "$src"
}

for entry in "${SWITCHES[@]}"; do
  tag="${entry%%|*}"
  flag="${entry#*|}"
  d="/tmp/gf/switch_${tag}_${steps}"
  total=$((1800 * steps))
  end_day=$((1 + total / 86400))
  end_sec=$((total % 86400))
  rm -rf "$d"; mkdir -p "$d/run"
  cp -R "$BASE/oracle/work/$case" "$d/case"
  python3 - "$d" "$case" "$flag" <<'PY'
import re, sys
d, case, flag = sys.argv[1], sys.argv[2], sys.argv[3]
p = f"{d}/case/case.nml"
s = open(p).read()
old = f"   DEF_CASE_NAME = '{case}'"
assert old in s, "case name line not found"
s = s.replace(old, old + "\n   " + flag, 1)
# 两侧都读**拷贝**里那份 forcing.nml（下面已经把挂载点改到仓库自带目录）
s = re.sub(r"^   DEF_forcing_namelist.*$",
           f"   DEF_forcing_namelist = '{d}/case/forcing.nml'", s, flags=re.M)
assert f"DEF_forcing_namelist = '{d}/case/forcing.nml'" in s, "forcing namelist line not found"
open(p, "w").write(s)
PY
  # PLUMBER2 挂载点变了（/Volumes/Data01 -> 仓库自带 examples/Forcing）：只改这份拷贝
  sed -i '' "s#^   DEF_dir_forcing.*#   DEF_dir_forcing              = '$BASE/examples/Forcing/'#" "$d/case/forcing.nml"
  # **时间窗与 HIST_FREQ 必须写进两侧各自读的那份 namelist**：内核读 `$d/case.nml`、
  # Rust 读 `$d/case/case.nml`。只改前者会让内核跑 N 步、Rust 跑一整个月，
  # 记录条数都不一样，逐记录比较全是垃圾（第 381 轮踩过一次）。
  patch "$d/case/case.nml" > "$d/case.nml"
  patch "$d/case/case.nml" > "$d/case/case.nml.tmp" && mv "$d/case/case.nml.tmp" "$d/case/case.nml"
  rm -rf "$d/case/out/$case/history"
  ( cd "$d/run" && "$BASE/kernels/default/colm.x" "$d/case.nml" > "$d/f.log" 2>&1 ) \
    || { echo "!! $tag: kernel run failed"; tail -5 "$d/f.log"; continue; }
  ( cd "$BASE" && cargo run -q -p colm-runtime --bin colm-rs -- "$d/case" --land-cover igbp \
      --restart-out "$d/rust_restart.nc" --history-dir "$d" > "$d/r.log" 2>&1 ) \
    || { echo "!! $tag: rust run failed"; tail -5 "$d/r.log"; continue; }
  echo "=== $tag ($flag), $steps 步"
  python3 - "$d" "$case" <<'PY'
import glob
import sys

import netCDF4 as nc
import numpy as np

d, case = sys.argv[1], sys.argv[2]
f = nc.Dataset(glob.glob(f"{d}/case/out/{case}/history/*.nc")[0])
r = nc.Dataset(glob.glob(f"{d}/colm-rs_hist*.nc")[0])
recs = min(len(f.dimensions["time"]), len(r.dimensions["time"]))
print(f"  两侧记录数: kernel={len(f.dimensions['time'])} rust={len(r.dimensions['time'])}")
found = False
for i in range(recs):
    rows = []
    for n in sorted(set(f.variables) & set(r.variables)):
        if "time" not in f.variables[n].dimensions:
            continue
        a = np.ma.filled(f.variables[n][i].astype("f8"), np.nan).ravel()
        b = np.ma.filled(r.variables[n][i].astype("f8"), np.nan).ravel()
        if a.shape != b.shape:
            continue
        m = a.view(np.int64) != b.view(np.int64)
        if not m.any():
            continue
        dd = np.abs(a - b)
        dd[np.isnan(dd)] = 0.0
        rows.append((n, int(m.sum()), float(dd.max())))
    mag = [x for x in rows if x[2] > 0.0]
    if mag:
        print(f"  首个**量级差**在第 {i} 条记录:")
        for n, c, mx in sorted(mag, key=lambda t: -t[2])[:8]:
            print(f"    {n:16s} ndiff={c:4d} maxabs={mx:.4e}")
        found = True
        break
    if rows:
        print(f"  第 {i} 条记录: 只差零的符号（{sum(x[1] for x in rows)} 个值）")
if not found:
    print(f"  前 {recs} 条记录里没有量级差（只有零的符号或完全相同）")
PY
done
