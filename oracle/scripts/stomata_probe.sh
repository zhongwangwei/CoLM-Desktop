#!/usr/bin/env bash
# 第 293 轮：`stomata` 遮荫支的同位型探针（找 `rssha` 9.1e-7 的首个分叉点）。
#
# 第 292 轮已证：`stomata` 遮荫调用的**全部可打印入参**与 sunlit 的 `rssun` 都逐位
# 相同，输出 `rssha` 在 kernel call 289（PHS 调用序号）差 9.1e-7 ⇒ 差在 `stomata`
# 内部。本探针把 `MOD_AssimStomataConductance.F90:stomata` 的入口、`calc_photo_params`
# 出口、6 次内迭代的每一步、出口都打出来，Rust 侧在 `photosynthesis.rs:stomata`
# 同点对打。
#
# 内核标签（`STIN/STPH/STIT/STOUT`），Rust 侧对应 `SRIN/SRPH/SRIT/SROUT`。
# 调用序号两侧 1:1（每个叶温迭代 2 次：先 sunlit 后 shaded；奇数=sunlit、偶数=shaded）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/stomata16}
STEPS=${STEPS:-16}
END_SEC=$((1800 * STEPS))
FORT="vendor/CoLM202X/main/MOD_AssimStomataConductance.F90"
RUST="crates/colm-core/src/photosynthesis.rs"
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
cp "$BASE/$FORT" "$WORK/backup/stomata.F90"
cp "$BASE/$RUST" "$WORK/backup/photosynthesis.rs"
restore() {
  cd "$BASE"
  diff -q "$WORK/backup/stomata.F90" "$BASE/$FORT" >/dev/null 2>&1 || { cp "$WORK/backup/stomata.F90" "$BASE/$FORT"; echo "== restored $FORT"; }
  diff -q "$WORK/backup/photosynthesis.rs" "$BASE/$RUST" >/dev/null 2>&1 || { cp "$WORK/backup/photosynthesis.rs" "$BASE/$RUST"; echo "== restored $RUST"; }
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

def load(path):
    with open(path) as f:
        return f.read().split("\n")

def save(path, lines):
    with open(path, "w") as f:
        f.write("\n".join(lines))

def after(lines, exact, payload, tag, occurrence=1):
    idx = [i for i, l in enumerate(lines) if l == exact]
    assert len(idx) >= occurrence, f"{tag}: {len(idx)} matches"
    lines[idx[occurrence - 1] + 1:idx[occurrence - 1] + 1] = payload

def before(lines, exact, payload, tag):
    idx = [i for i, l in enumerate(lines) if l == exact]
    assert len(idx) == 1, f"{tag}: {len(idx)} matches"
    lines[idx[0]:idx[0]] = payload

path = sys.argv[1]
lines = load(path)
after(lines, "   SAVE", ["   integer, save :: st_probe_n = 0"], "f save")
before(lines, "      g1_used = g1", [
    "      st_probe_n = st_probe_n + 1",
    "      WRITE(*,'(A,1X,I5,14(1X,ES23.15))') 'STIN ', st_probe_n, &",
    "         tlef, psrf, po2m, pco2m, pco2a, ea, ei, par, rb, ra, rstfac, cint(1), cint(2), cint(3)",
], "f STIN")
after(lines, "      range = pco2m * ( 1. - 1.6/gradm_used ) - gammas", [
    "      WRITE(*,'(A,1X,I5,11(1X,ES23.15))') 'STPH ', st_probe_n, &",
    "         vm, epar, respc, omss, gbh2o, gammas, rrkk, c3, c4, bintc, range",
], "f STPH")
after(lines, "         eyy(ic) = pco2i - pco2in                              ! pa", [
    "         WRITE(*,'(A,1X,I5,1X,I2,14(1X,ES23.15))') 'STIT ', st_probe_n, ic, &",
    "            pco2i, omc, ome, oms, assim, assimn, co2s, co2st, assmt, hcdma, gsh2o, es, pco2in, eyy(ic)",
], "f STIT")
after(lines, "      rst   = min( 1.e6, 1./(gsh2o*tlef/tprcor) )     ! s m-1", [
    "      WRITE(*,'(A,1X,I5,4(1X,ES23.15))') 'STOUT', st_probe_n, gsh2o, rst, tprcor, tlef",
], "f STOUT")
save(path, lines)
print("   patched MOD_AssimStomataConductance.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
for m in STIN STPH STIT STOUT; do
  grep -q "$m" "$WORK/strings.txt" || { echo "!! marker $m missing"; exit 3; }
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
grep -a '^STIN \|^STPH \|^STIT \|^STOUT' "$WORK/kernel.log" > "$WORK/fort_probe.txt" || true
echo "   kernel probe lines: $(wc -l < "$WORK/fort_probe.txt")"
for t in STIN STPH STIT STOUT; do
  printf '   %-6s %s\n' "$t" "$(grep -ac "^$t " "$WORK/fort_probe.txt" || true)"
done

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()

anchor = "const ITERATIONS: usize = 6;"
assert src.count(anchor) == 1
src = src.replace(anchor,
    "/// 探针用：与内核侧 `st_probe_n` 同步的 `stomata` 调用序号。\n"
    "static STOMATA_PROBE_COUNT: std::sync::atomic::AtomicUsize =\n"
    "    std::sync::atomic::AtomicUsize::new(0);\n\n" + anchor, 1)

ent = "    validate_stomata(input, options)?;\n"
assert src.count(ent) == 1
src = src.replace(ent, ent +
    '    let st_call = STOMATA_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n'
    '    if std::env::var_os("COLM_STOMATA_PROBE").is_some() {\n'
    '        eprintln!("SRIN  {:5} tlef={:e} psrf={:e} po2m={:e} pco2m={:e} pco2a={:e} ea={:e} ei={:e} par={:e} rb={:e} rstfac={:e} c1={:e} c2={:e} c3={:e}",\n'
    '            st_call, input.photosynthesis.leaf_temperature_k, input.photosynthesis.air_pressure_pa,\n'
    '            input.photosynthesis.oxygen_partial_pressure_pa, input.atmospheric_co2_pa, input.canopy_air_co2_pa,\n'
    '            input.canopy_air_vapor_pressure_pa, input.leaf_saturation_vapor_pressure_pa,\n'
    '            input.photosynthesis.absorbed_par_w_m2, input.photosynthesis.leaf_boundary_resistance_s_m,\n'
    '            input.photosynthesis.soil_water_stress, input.photosynthesis.canopy_integration[0],\n'
    '            input.photosynthesis.canopy_integration[1], input.photosynthesis.canopy_integration[2]);\n'
    '    }\n', 1)

sph = ("    let range = input.atmospheric_co2_pa * (1.0 - f77(1.6) / gradm) - photo.co2_compensation_pa;\n")
# 文件里 `range` 出现两次（stomata 与 update_photosynthesis），stomata 在前
assert src.count(sph) == 2
i = src.index(sph) + len(sph)
src = src[:i] + (
    '    if std::env::var_os("COLM_STOMATA_PROBE").is_some() {\n'
    '        eprintln!("SRPH  {:5} vm={:e} epar={:e} respc={:e} omss={:e} gbh2o={:e} gammas={:e} rrkk={:e} c3={:e} c4={:e} bintc={:e} range={:e}",\n'
    '            st_call, photo.maximum_carboxylation_mol_m2_s, photo.electron_transport_mol_m2_s,\n'
    '            photo.respiration_mol_m2_s, photo.sink_limit_mol_m2_s_pa,\n'
    '            photo.boundary_conductance_h2o_mol_m2_s, photo.co2_compensation_pa,\n'
    '            photo.rubisco_co2_constant_pa, photo.c3_fraction, photo.c4_fraction, bintc, range);\n'
    '    }\n') + src[i:]

sit = "        errors[iteration - 1] = internal_co2 - next_co2;\n"
assert src.count(sit) == 1
src = src.replace(sit, sit +
    '        if std::env::var_os("COLM_STOMATA_PROBE").is_some() {\n'
    '            eprintln!("SRIT  {:5} ic={:2} pco2i={:e} omc={:e} ome={:e} assim={:e} assimn={:e} co2s={:e} assmt={:e} gsh2o={:e} pco2in={:e} eyy={:e}",\n'
    '                st_call, iteration, internal_co2, omc, ome, assimilation, net_assimilation,\n'
    '                co2_surface, positive_assimilation, conductance, next_co2, errors[iteration - 1]);\n'
    '        }\n', 1)

sout = "        f77(44.6 * 273.16) * input.photosynthesis.air_pressure_pa / f77(1.013e5);\n"
assert src.count(sout) == 1
src = src.replace(sout, sout +
    '    if std::env::var_os("COLM_STOMATA_PROBE").is_some() {\n'
    '        eprintln!("SROUT {:5} gsh2o={:e} rst={:e} tprcor={:e} tlef={:e}",\n'
    '            st_call, conductance,\n'
    '            (1.0 / (conductance * input.photosynthesis.leaf_temperature_k / pressure_conversion)).min(f77(1.0e6)),\n'
    '            pressure_conversion, input.photosynthesis.leaf_temperature_k);\n'
    '    }\n', 1)

open(path, "w").write(src)
print("   patched photosynthesis.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -25 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_STOMATA_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -a '^SRIN \|^SRPH \|^SRIT \|^SROUT' "$WORK/rust.log" > "$WORK/rust_probe.txt" || true
echo "   rust probe lines: $(wc -l < "$WORK/rust_probe.txt")"
echo "== 比较 =="
python3 "$BASE/oracle/scripts/stomata_cmp.py" "$WORK/fort_probe.txt" "$WORK/rust_probe.txt" || true
echo "== 输出目录 $WORK =="
