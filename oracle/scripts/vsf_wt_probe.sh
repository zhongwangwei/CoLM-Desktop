#!/usr/bin/env bash
# 第 301 轮：`soil_water_vertical_movement` 里**水位级号 + `ss_wt` 初始化**的两侧探针。
#
# 为什么打这里：第 300 轮的 Richards 内部探针（`vsf_richards_probe.sh`）把首差钉在
# **第 17 次 Richards 调用入场**的 `ss_wt` 上，而那次解算内部 `f2`/`blc`/`dv` 全同
# ⇒ 差是**入场前**带进来的，来源只有这一段：
#
#     Fortran `MOD_Hydro_SoilWater.F90`
#       :256  izwt = findloc_ud(zwt >= sp_zi, back=.true.)      ← 入场级号（旧）
#       ...   soilwater_aquifer_exchange (..., zwt, wa, izwt)    ← 含水层交换会改 zwt/izwt
#       :343  ss_wt(:) = 0
#       :345  ss_wt(izwt) = sp_zi(izwt) - zwt                    ← 水位那一层
#       :348  ss_wt(ilev) = sp_dz(ilev)  (ilev > izwt)           ← 以下整层饱和
#
# 三个 tag：
#   `WSE1`  —— 第一次 `findloc_ud` 之后：`izwt`、入场 `zwt`、`nlev`
#   `WSE0`  —— `ss_wt` 初始化之后：`izwt`、`zwt`（**可能已被交换改过**）、`wa`、`ss_dp`
#   `WSEL`  —— 逐层 `ss_wt` / `ss_vliq` / `porsl`
#
# 判读：`WSE1` 全同而 `WSE0` 差 ⇒ 差在 `soilwater_aquifer_exchange` 改出来的
# `zwt`/`izwt`；两者都同而 `WSEL` 差 ⇒ 差在 `sp_zi(izwt)-zwt` 那一步（只可能是级号
# 或 `sp_zi` 本身）；`WSE0` 的 `izwt` 就差 ⇒ `findloc_ud` 的 `>=` 比较在边界上翻了。
#
# 用法: bash oracle/scripts/vsf_wt_probe.sh [STEPS]   （默认 20 步，覆盖 Richards 第 17 次调用）
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STEPS=${1:-20}
WORK=${WORK:-/tmp/gf/vsfwt$STEPS}
END_SEC=$((1800 * STEPS))
FORT="vendor/CoLM202X/main/HYDRO/MOD_Hydro_SoilWater.F90"
RUST="crates/colm-core/src/variably_saturated_flow.rs"
export NETCDF_DIR=${NETCDF_DIR:-/opt/homebrew/opt/netcdf}
LOCK=/tmp/gf/.kernel_build.lock
for _ in $(seq 1 240); do
  if mkdir "$LOCK" 2>/dev/null; then echo "== build lock acquired"; break; fi
  if [ -d "$LOCK" ] && [ -z "$(find "$LOCK" -mmin -15 2>/dev/null)" ]; then
    echo "== stealing stale build lock"; rm -rf "$LOCK"; continue
  fi
  sleep 5
done
[ -d "$LOCK" ] || { echo "!! another kernel build holds $LOCK" >&2; exit 9; }
rm -rf "$WORK"; mkdir -p "$WORK/run" "$WORK/backup"
cp "$BASE/$FORT" "$WORK/backup/soilwater.F90"
cp "$BASE/$RUST" "$WORK/backup/vsf.rs"
restore() {
  cd "$BASE"
  diff -q "$WORK/backup/soilwater.F90" "$BASE/$FORT" >/dev/null 2>&1 || { cp "$WORK/backup/soilwater.F90" "$BASE/$FORT"; echo "== restored $FORT"; }
  diff -q "$WORK/backup/vsf.rs" "$BASE/$RUST" >/dev/null 2>&1 || { cp "$WORK/backup/vsf.rs" "$BASE/$RUST"; echo "== restored $RUST"; }
  if [ "${SKIP_REBUILD:-0}" != 1 ]; then
    (cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/rebuild.log" 2>&1) || echo "!! rebuild failed"
  fi
  cd "$BASE"
  git status --short vendor/CoLM202X crates | sed 's/^/== git status: /'
  rmdir "$LOCK" 2>/dev/null || rm -rf "$LOCK"
}
trap restore EXIT

python3 - "$BASE/$FORT" <<'FEOF'
import sys

path = sys.argv[1]
lines = open(path).read().split("\n")


def insert_before(anchor, block):
    hits = [i for i, line in enumerate(lines) if line == anchor]
    assert len(hits) == 1, (anchor, len(hits))
    lines[hits[0] : hits[0]] = block.split("\n")


def insert_after(anchor, block, nth=0):
    """nth 用来在**重复锚点**里挑第 n 个（本文件里 `izwt = findloc_ud(zwt >= sp_zi...)`
    出现三次：:256 是入口、:430/:533 在出口之后，必须挑第一个）。"""
    hits = [i for i, line in enumerate(lines) if line == anchor]
    assert len(hits) > nth, (anchor, len(hits))
    lines[hits[nth] + 1 : hits[nth] + 1] = block.split("\n")


insert_after("   logical  :: is_sat", "   integer, save :: wsf3_n = 0")

# 入场级号（含水层交换**之前**）
insert_after(
    "      izwt = findloc_ud(zwt >= sp_zi, back=.true.)",
    """      wsf3_n = wsf3_n + 1
      WRITE(*,'(A,1X,I5,1X,I2,1X,I2,3(1X,ES26.17))') 'WSE1', wsf3_n, 0, 0, &
         real(izwt,r8), real(nlev,r8), zwt""",
    nth=0,
)

# `ss_wt` 初始化之后（`izwt`/`zwt` 都可能是交换改过的）
insert_before(
    "      ! Impermeable levels cut the soil column into several disconnected parts.",
    """      WRITE(*,'(A,1X,I5,1X,I2,1X,I2,5(1X,ES26.17))') 'WSE0', wsf3_n, 0, 0, &
         real(izwt,r8), real(nlev,r8), zwt, wa, ss_dp
      DO ilev = 1, nlev
         WRITE(*,'(A,1X,I5,1X,I3,1X,I2,3(1X,ES26.17))') 'WSEL', wsf3_n, ilev, 0, &
            ss_wt(ilev), ss_vliq(ilev), porsl(ilev)
      ENDDO""",
)

open(path, "w").write("\n".join(lines))
print("   patched MOD_Hydro_SoilWater.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
for tag in WSE1 WSE0 WSEL; do
  grep -q "$tag" "$WORK/strings.txt" || { echo "!! marker $tag missing"; exit 3; }
done

sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$WORK/out/'#" \
    -e "s#^   DEF_forcing_namelist.*#   DEF_forcing_namelist = '$WORK/forcing.nml'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$WORK/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$WORK/runtime_unused/'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = 1#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = $END_SEC#" \
    -e "s#^   DEF_HIST_FREQ.*#   DEF_HIST_FREQ    = 'TIMESTEP'#" \
    "$BASE/oracle/work/CN-Cng/case.nml" > "$WORK/case.nml"
cp "$BASE/oracle/work/CN-Cng/forcing.nml" "$WORK/forcing.nml"
cp -R "$BASE/oracle/work/CN-Cng/out" "$WORK/out"
rm -rf "$WORK/out/CN-Cng/history"
( cd "$WORK/run" && "$BASE/kernels/default/colm.x" "$WORK/case.nml" > "$WORK/kernel.log" 2>&1 ) \
  || { echo "!! kernel run failed"; exit 4; }
grep -q 'CoLM Execution Completed' "$WORK/kernel.log" || { echo "!! incomplete"; tail -5 "$WORK/kernel.log"; exit 4; }
grep -aE '^(WSE1|WSE0|WSEL) ' "$WORK/kernel.log" > "$WORK/fort.txt" || true
echo "   kernel probe lines: $(wc -l < "$WORK/fort.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()

# 入场级号（含水层交换之前）
before = """    let mut balance_before_mm = state.ponding_depth_mm;
"""
assert src.count(before) == 1
head = """    static VSF_WT_PROBE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    if std::env::var_os("COLM_VSF_WT_PROBE").is_some() {
        let count = VSF_WT_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        eprintln!(
            "WSE1 {:5} {:2} {:2} {:e} {:e} {:e}",
            count, 0, 0, water_table_level as f64, nlev as f64, water_table_depth_mm
        );
    }
"""
src = src.replace(before, head + before, 1)

# `ss_wt` 初始化之后
anchor = """    // 按不透水层把土柱切成互不相连的子段，逐段求解。
"""
assert src.count(anchor) == 1
block = """    if std::env::var_os("COLM_VSF_WT_PROBE").is_some() {
        let count = VSF_WT_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "WSE0 {:5} {:2} {:2} {:e} {:e} {:e} {:e} {:e}",
            count, 0, 0, water_table_level as f64, nlev as f64, water_table_depth_mm,
            state.aquifer_water_mm, state.ponding_depth_mm
        );
        for level in 0..nlev {
            eprintln!(
                "WSEL {:5} {:3} {:2} {:e} {:e} {:e}",
                count, level + 1, 0, water_table_thickness_mm[level],
                state.liquid_water[level], input.porosity[level]
            );
        }
    }
"""
src = src.replace(anchor, block + anchor, 1)
open(path, "w").write(src)
print("   patched variably_saturated_flow.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -30 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_VSF_WT_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -aE '^(WSE1|WSE0|WSEL) ' "$WORK/rust.log" > "$WORK/rust.txt" || true
echo "   rust probe lines: $(wc -l < "$WORK/rust.txt")"

echo "== compare =="
python3 "$BASE/oracle/scripts/probe_diff.py" "$WORK/fort.txt" "$WORK/rust.txt" || true
