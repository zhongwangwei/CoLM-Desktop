#!/usr/bin/env bash
# 第 299 轮：`soil_water_vertical_movement` **出场处**的两侧同位型探针。
#
# 为什么要打这里：`oracle/scripts/restart_scan.sh` 实测「重启到第 18 步 0/68 逐位相同、
# 第 19 步起 `wliq_soisno[7]` 差 1 ULP」。第 19 步的**入场状态**因此是逐位相同的，
# 分歧只可能来自第 19 步执行的那条代码路径本身 —— 这正是探针最省力的情形：
# 两边同一条记录里第一个不同的字段，就是分歧的出生点，不用二分也不用猜形状。
#
# `wliq_soisno` 是 `MOD_SoilSnowHydrology.F90:1105-1120` 用 `vol_liq`（即本子程序的
# `ss_vliq`）+ `wresi` 重算的，`zwt` 也由这里（`:409-436`）定；所以先打
# **本子程序出口**的 `ss_vliq`/`ss_wt`/`smp`/`hk`/`qlayer`：
#   * 第一个不同的字段是 `ss_vliq`/`ss_wt` ⇒ 分歧在 `Richards_solver` 内（下一次探针钻那里）；
#   * 只有在 `smp`/`hk` 上差而 `ss_vliq` 逐位相同 ⇒ 分歧在出口的反解（`soil_psi_from_vliq`）。
# 打之前**不猜**任何形状：第 298 轮照式子猜的四个 FMA 形状全被实测否掉。
#
# 内核 `WSF0 n call_n nlev wa zwt ss_dp qinfl wblc tol_v`
#       `WSF  n call_n ilev ss_vliq ss_wt smp hk qlayer_l qlayer_u porsl`
# Rust  `WSF0 ...` / `WSF ...`（同格式，同字段序）
#
# 用法: bash oracle/scripts/vsf_probe.sh [STEPS]        （默认 19 步）
#       SKIP_REBUILD=1 ... 可跳过收尾重编（只在确认源码已还原时用）
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STEPS=${1:-19}
WORK=${WORK:-/tmp/gf/vsf$STEPS}
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

# 计数器必须活到下一次调用，而且要放在本子程序的声明区里 —— `IMPLICIT NONE`
# 在整份文件里出现很多次，所以锚在它独有的那行局部声明上。
decl = "   logical  :: is_sat"
assert lines.count(decl) == 1, lines.count(decl)
lines.insert(lines.index(decl) + 1, "   integer, save :: vsf_probe_n = 0")

anchor = "   END SUBROUTINE soil_water_vertical_movement"
assert lines.count(anchor) == 1, lines.count(anchor)
idx = lines.index(anchor)
lines[idx:idx] = [
    "      vsf_probe_n = vsf_probe_n + 1",
    "      WRITE(*,'(A,1X,I5,1X,I3,7(1X,ES26.17))') 'WSF0', vsf_probe_n, nlev, &",
    "         wa, zwt, ss_dp, qinfl, wblc, tol_v",
    "      DO ilev = 1, nlev",
    "         WRITE(*,'(A,1X,I5,1X,I3,7(1X,ES26.17))') 'WSF', vsf_probe_n, ilev, &",
    "            ss_vliq(ilev), ss_wt(ilev), smp(ilev), hk(ilev), qlayer(ilev-1), qlayer(ilev), porsl(ilev)",
    "      ENDDO",
]
open(path, "w").write("\n".join(lines))
print("   patched MOD_Hydro_SoilWater.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
grep -q 'WSF0' "$WORK/strings.txt" || { echo "!! marker missing"; exit 3; }

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
grep -a '^WSF' "$WORK/kernel.log" > "$WORK/fort.txt" || true
echo "   kernel WSF lines: $(wc -l < "$WORK/fort.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()
anchor = "    // `qlayer` 是唯一直接暴露给 history 的输出。"
assert src.count(anchor) == 1, src.count(anchor)
ins = (
    '    if std::env::var_os("COLM_VSF_PROBE").is_some() {\n'
    '        static VSF_PROBE_COUNT: std::sync::atomic::AtomicUsize =\n'
    '            std::sync::atomic::AtomicUsize::new(0);\n'
    '        let count = VSF_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;\n'
    '        eprintln!(\n'
    '            "WSF0 {:5} {:3} {:e} {:e} {:e} {:e} {:e} {:e}",\n'
    '            count, nlev, state.aquifer_water_mm, water_table_depth_mm,\n'
    '            state.ponding_depth_mm, infiltration_mm_s, balance_error_mm, volume_tolerance\n'
    '        );\n'
    '        for level in 0..nlev {\n'
    '            eprintln!(\n'
    '                "WSF {:5} {:3} {:e} {:e} {:e} {:e} {:e} {:e} {:e}",\n'
    '                count, level + 1, state.liquid_water[level],\n'
    '                water_table_thickness_mm[level], state.matric_potential_mm[level],\n'
    '                state.hydraulic_conductivity_mm_s[level], interface_flux_mm_s[level],\n'
    '                interface_flux_mm_s[level + 1], input.porosity[level]\n'
    '            );\n'
    '        }\n'
    '    }\n'
) + anchor
open(path, "w").write(src.replace(anchor, ins, 1))
print("   patched variably_saturated_flow.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -25 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_VSF_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -a '^WSF' "$WORK/rust.log" > "$WORK/rust.txt" || true
echo "   rust WSF lines: $(wc -l < "$WORK/rust.txt")"

echo "== compare =="
python3 "$BASE/oracle/scripts/vsf_cmp.py" "$WORK/fort.txt" "$WORK/rust.txt" || true
