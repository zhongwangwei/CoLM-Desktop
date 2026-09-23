#!/usr/bin/env bash
# 逐步 restart 扫描：找**瞬时状态**第一次分歧落在第几步。
#
# 为什么不能用 `dry_ts.sh` 的 history 判"第几步"：那套历史量（`f_zwt`/`f_wliq_soisno`/…）
# 是 `MOD_Vars_1DAccFluxes` 里 `acc1d` **按输出区间累加**出来的，`TIMESTEP` 窗口与
# 算例的 `HOURLY` 又是不同宽度（第 295 轮实测同一时刻两个值）。判状态只有 restart
# 是准的：`MOD_HistWriteBack`/`restart-out` 写的就是 68 个状态量本身。
#
# 用法: bash oracle/scripts/restart_scan.sh [N ...]     （默认 1..32）
# 依赖 `/tmp/gf/dry_ts.sh <N>`（跑 N 步的内核 + Rust，与 accept_r247.sh 同源）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$BASE"
NS=("$@")
if [ ${#NS[@]} -eq 0 ]; then
  NS=($(seq 1 32))
fi
for N in "${NS[@]}"; do
  bash /tmp/gf/dry_ts.sh "$N" > "/tmp/gf/rs_$N.log" 2>&1 || { echo "N=$N RUN FAILED"; continue; }
  DIR=$(printf "2008-001-%05d" $((1800 * N)))
  K="/tmp/gf/dryts/out/CN-Cng/restart/$DIR/CN-Cng_restart_${DIR}_lc2005_w180_s90.nc"
  R="/tmp/gf/dryts/rust_restart.nc"
  if [ ! -f "$K" ] || [ ! -f "$R" ]; then echo "N=$N missing restart ($K)"; continue; fi
  OUT=$(python3 oracle/scripts/restart_divergence.py "$K" "$R" --top 5 2>&1)
  echo "=== N=$N  $(echo "$OUT" | grep 'differing restart variables')"
  echo "$OUT" | grep -E "^ +[0-9]" | head -5
done
