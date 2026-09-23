#!/usr/bin/env bash
# 第 303 轮：`WATER_VSF` **入口**（`soil_water_vertical_movement` 调用点之前）的两侧探针。
#
# 为什么打这里：`vsf_wt_probe.sh` 报出第 17 次调用第 3 层的 `eff_porosity`（`WSEL f2`）
# 与 `vol_liq`（`WSEL f1`）各差 1 ULP，而**步末的 restart 到第 17 步仍是 0/68**
# （本轮实测）⇒ 差是**步内**的、不在持久状态里。`eff_porosity` 的公式两侧已逐项对上
# （`MOD_SoilSnowHydrology.F90:855-863` ↔ `variably_saturated_flow.rs:4150-4161`），
# 所以只能查它的入参：`wice_soisno` / `porsl` / `dz_soisno` / `wimp`。
#
# 一个 tag、每层一行、7 列（两侧同序）：
#   `VSFI n ilev 0  wice wliq eff_porosity porsl dz_soisno wimp vol_liq`
# 判读：
#   * `wice` 就不同 ⇒ 差在能量步的相变/冰（水步之前）；
#   * `wice`/`porsl`/`dz`/`wimp` 全同而 `eff_porosity` 不同 ⇒ 差在这个表达式本身；
#   * `porsl` 不同 ⇒ 差的是一条更上游的静态量链。
#
# 用法: bash oracle/scripts/vsf_input_probe.sh [STEPS]   （默认 20 步）
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STEPS=${1:-20}
WORK=${WORK:-/tmp/gf/vsfin$STEPS}
END_SEC=$((1800 * STEPS))
FORT="vendor/CoLM202X/main/MOD_SoilSnowHydrology.F90"
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
cp "$BASE/$FORT" "$WORK/backup/sshyd.F90"
cp "$BASE/$RUST" "$WORK/backup/vsf.rs"
restore() {
  cd "$BASE"
  diff -q "$WORK/backup/sshyd.F90" "$BASE/$FORT" >/dev/null 2>&1 || { cp "$WORK/backup/sshyd.F90" "$BASE/$FORT"; echo "== restored $FORT"; }
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
anchor = [i for i, line in enumerate(lines) if line == "      CALL soil_water_vertical_movement ( &"]
assert len(anchor) == 1, len(anchor)
at = anchor[0]
decls = [i for i in range(at) if lines[i] == "   IMPLICIT NONE"]
assert decls, "no IMPLICIT NONE before the anchor"
lines[decls[-1] + 1 : decls[-1] + 1] = ["   integer, save :: vsfi_n = 0"]
lines[at:at] = """      vsfi_n = vsfi_n + 1
      DO j = 1, nl_soil
         WRITE(*,'(A,1X,I5,1X,I3,1X,I2,7(1X,ES26.17))') 'VSFI', vsfi_n, j, 0, &
            wice_soisno(j), wliq_soisno(j), eff_porosity(j), porsl(j), &
            dz_soisno(j), wimp, vol_liq(j)
      ENDDO""".split("\n")
open(path, "w").write("\n".join(lines))
print("   patched MOD_SoilSnowHydrology.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
grep -q 'VSFI' "$WORK/strings.txt" || { echo "!! marker missing"; exit 3; }

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
grep -aE '^VSFI ' "$WORK/kernel.log" > "$WORK/fort.txt" || true
echo "   kernel probe lines: $(wc -l < "$WORK/fort.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()
anchor = "    let soil = soil_water_vertical_movement(\n"
assert src.count(anchor) == 1, src.count(anchor)
block = """    if std::env::var_os("COLM_VSF_INPUT_PROBE").is_some() {
        static VSF_INPUT_PROBE_COUNT: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let count = VSF_INPUT_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        for level in 0..nlev {
            eprintln!(
                "VSFI {:5} {:3} {:2} {:e} {:e} {:e} {:e} {:e} {:e} {:e}",
                count, level + 1, 0, state.ice_water_kg_m2[level],
                state.liquid_water_kg_m2[level], effective_porosity[level],
                input.porosity[level], input.layer_thickness_m[level],
                input.impermeable_porosity, liquid_volume_fraction[level]
            );
        }
    }
"""
open(path, "w").write(src.replace(anchor, block + anchor, 1))
print("   patched variably_saturated_flow.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -30 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_VSF_INPUT_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -aE '^VSFI ' "$WORK/rust.log" > "$WORK/rust.txt" || true
echo "   rust probe lines: $(wc -l < "$WORK/rust.txt")"

echo "== compare =="
python3 "$BASE/oracle/scripts/probe_diff.py" "$WORK/fort.txt" "$WORK/rust.txt" || true
