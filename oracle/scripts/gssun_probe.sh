#!/usr/bin/env bash
# 第 292 轮：gssun 六处探针标定（两侧同点同序）。
#
# 内核侧（MOD_PlantHydraulic.F90 / MOD_LeafTemperature_Extended.F90）：
#   GSIN   gsp_n gs0sun gs0sha gssun gssha laisun laisha tl tprcor sai fwet
#          —— :317 之后（`gssun=gs0sun` 之后、:318 调用之前）
#   GSDEM  gsp_n qflx_sun qflx_sha gssun gssha gb_mol tl tprcor laisun laisha rss
#          —— :318 调用返回之后
#   GSOUT  gsp_n gssun gssha etrsun etrsha qflx_sun qflx_sha tprcor tl laisun laisha
#          —— :346 调用返回之后（PHS 的 `gssun` 输出）
#   GS908  gsp_n it gssun gssha laisun laisha tl tprcor sai fwet lai o3coefg_sun
#          —— :909 之后（PHS 输出 ×laisun，冠层尺度）
#   GS919  gsp_n it gssun gssha rssun rssha tl tprcor laisun laisha rb rbsun
#          —— :920 之后
#   GS941  gsp_n it rssun rssha laisun laisha rb tprcor tl gssun gssha lai
#          —— :942 之后（rssun 折回逐叶尺度）
#   GS1320 gsp_n it gssun gssha rssun rssha laisun laisha tlbef tprcor tl lai
#          —— :1321 之后（诊断电导，子程序末尾）
#
# Rust 侧同序（GRIN/GRDEM/GROUT/GR908/GR919/GR941/GR1320）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/gssun1}
STEPS=${STEPS:-1}
END_SEC=$((1800 * STEPS))
FORT_PH="vendor/CoLM202X/main/MOD_PlantHydraulic.F90"
FORT_LT="vendor/CoLM202X/extends/interception/MOD_LeafTemperature_Extended.F90"
RUST_PH="crates/colm-core/src/plant_hydraulics.rs"
RUST_LT="crates/colm-core/src/leaf_temperature.rs"
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
for f in "$FORT_PH" "$FORT_LT" "$RUST_PH" "$RUST_LT"; do
  cp "$BASE/$f" "$WORK/backup/$(basename "$f")"
done
restore() {
  cd "$BASE"
  for f in "$FORT_PH" "$FORT_LT" "$RUST_PH" "$RUST_LT"; do
    b="$WORK/backup/$(basename "$f")"
    diff -q "$b" "$BASE/$f" >/dev/null 2>&1 || { cp "$b" "$BASE/$f"; echo "== restored $f"; }
  done
  if [ "${SKIP_REBUILD:-0}" != 1 ]; then
    (cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/rebuild.log" 2>&1) || echo "!! rebuild failed"
  fi
  cd "$BASE"
  git status --short vendor/CoLM202X crates | sed 's/^/== git status: /'
  rmdir "$LOCK" 2>/dev/null || rm -rf "$LOCK"
}
trap restore EXIT

python3 - "$BASE/$FORT_PH" "$BASE/$FORT_LT" <<'FEOF'
import sys
ph, lt = sys.argv[1], sys.argv[2]
def load(path):
    with open(path) as f:
        return f.read().split("\n")

def save(path, lines):
    with open(path, "w") as f:
        f.write("\n".join(lines))

def after(lines, exact, payload, tag):
    idx = [i for i, l in enumerate(lines) if l == exact]
    assert len(idx) == 1, f"{tag}: {len(idx)} matches"
    lines[idx[0] + 1:idx[0] + 1] = payload

def before(lines, exact, payload, tag):
    idx = [i for i, l in enumerate(lines) if l == exact]
    assert len(idx) == 1, f"{tag}: {len(idx)} matches"
    lines[idx[0]:idx[0]] = payload

lines = load(ph)
after(lines, "   SAVE", ["   integer, save :: gsp_probe_n = 0"], "ph save")
after(lines, "   PUBLIC :: PlantHydraulicStress_twoleaf", ["   PUBLIC :: gsp_probe_n"], "ph pub")
before(lines, "      x = vegwp(1:nvegwcs)", ["      gsp_probe_n = gsp_probe_n + 1"], "ph ent")
after(lines, "      gssha=gs0sha", [
    "      WRITE(*,'(A,1X,I5,10(1X,ES23.15))') 'GSIN', gsp_probe_n, &",
    "         gs0sun, gs0sha, gssun, gssha, laisun, laisha, tl, psrf, sai, fwet",
], "ph GSIN")
after(lines, "                                   rhoair,psrf,laisun,laisha,sai,fwet,tl,rss,raw,rd,qg,qm)", [
    "      WRITE(*,'(A,1X,I5,10(1X,ES23.15))') 'GSDEM', gsp_probe_n, &",
    "         qflx_sun, qflx_sha, gssun, gssha, gb_mol, tl, psrf, laisun, laisha, rss",
], "ph GSDEM")
after(lines, "                                      rhoair,psrf,laisun,laisha,sai,fwet,tl,rss,raw,rd,qg,qm)", [
    "         WRITE(*,'(A,1X,I5,10(1X,ES23.15))') 'GSOUT', gsp_probe_n, &",
    "            gssun, gssha, etrsun, etrsha, qflx_sun, qflx_sha, psrf, tl, laisun, laisha",
], "ph GSOUT")
save(ph, lines)
print("   patched MOD_PlantHydraulic.F90")

lines = load(lt)
use_old = "   USE MOD_PlantHydraulic, only:PlantHydraulicStress_twoleaf, getvegwp_twoleaf"
use_idx = [i for i, l in enumerate(lines) if l == use_old]
assert len(use_idx) == 1, f"lt use: {len(use_idx)}"
lines[use_idx[0]] = use_old + ", gsp_probe_n"
after(lines, "               gssha = gssha * laisha", [
    "               WRITE(*,'(A,1X,I5,1X,I3,10(1X,ES23.15))') 'GS908', gsp_probe_n, it, &",
    "                  gssun, gssha, laisun, laisha, tl, tprcor, sai, fwet, lai, o3coefg_sun",
], "lt GS908")
after(lines, "               rssha = tprcor/tl * 1.e6 / gssha", [
    "               WRITE(*,'(A,1X,I5,1X,I3,10(1X,ES23.15))') 'GS919', gsp_probe_n, it, &",
    "                  gssun, gssha, rssun, rssha, tl, tprcor, laisun, laisha, rb, rbsun",
], "lt GS919")
after(lines, "         rssha = rssha * laisha", [
    "         WRITE(*,'(A,1X,I5,1X,I3,10(1X,ES23.15))') 'GS941', gsp_probe_n, it, &",
    "            rssun, rssha, laisun, laisha, rb, tprcor, tl, gssun, gssha, lai",
], "lt GS941")
after(lines, "         gssha = (laisha / rssha) * (tprcor / tlbef)", [
    "         WRITE(*,'(A,1X,I5,1X,I3,10(1X,ES23.15))') 'GS1320', gsp_probe_n, it, &",
    "            gssun, gssha, rssun, rssha, laisun, laisha, tlbef, tprcor, tl, lai",
], "lt GS1320")
after(lines, "                 assimsha ,respcsha ,rssha    )", [
    "            WRITE(*,'(A,1X,I5,1X,I3,10(1X,ES23.15))') 'GSTO  ', gsp_probe_n, it, &",
    "               parsun, parsha, rssun, rssha, eah, tl, psrf, rb, rbsha, lai",
], "lt GSTO")
save(lt, lines)
print("   patched MOD_LeafTemperature_Extended.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
for m in GSIN GSDEM GSOUT GS908 GS919 GS941 GS1320 GSTO; do
  grep -q "$m" "$WORK/strings.txt" || { echo "!! marker $m missing"; exit 3; }
done

# ---- 内核跑 1 步 CN-Cng 干窗 ----
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
grep -a '^GSIN \|^GSDEM\|^GSOUT\|^GS908\|^GS919\|^GS941\|^GS1320\|^GSTO' "$WORK/kernel.log" > "$WORK/fort_probe.txt" || true
echo "   kernel probe lines: $(wc -l < "$WORK/fort_probe.txt")"
for t in GSIN GSDEM GSOUT GS908 GS919 GS941 GS1320 GSTO; do
  printf '   %-7s %s\n' "$t" "$(grep -ac "^$t " "$WORK/fort_probe.txt" || true)"
done

# ---- Rust 侧打点 ----
python3 - "$BASE/$RUST_PH" "$BASE/$RUST_LT" <<'REOF'
import sys
ph, lt = sys.argv[1], sys.argv[2]
src = open(ph).read()
anchor_mod = "const MIN_STRESS: f64"
assert src.count(anchor_mod) >= 1, f"mod {src.count(anchor_mod)}"
src = src.replace(anchor_mod,
    "/// 探针用：与内核侧 `gsp_probe_n` 同步的 PHS 调用序号。\n"
    "pub static PHS_PROBE_COUNT: std::sync::atomic::AtomicUsize =\n"
    "    std::sync::atomic::AtomicUsize::new(0);\n\n" + anchor_mod, 1)
ent = "    let layers = validate(input, *state)?;\n"
assert src.count(ent) == 1, f"ent {src.count(ent)}"
src = src.replace(ent, ent +
    "    let phs_call = PHS_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n", 1)
grdem = ("    let (sunlit_demand, shaded_demand) = transpiration_from_conductance(\n"
         "        input,\n"
         "        boundary_conductance,\n"
         "        input.maximum_sunlit_leaf_conductance_umol_m2_s,\n"
         "        input.maximum_shaded_leaf_conductance_umol_m2_s,\n"
         "        None,\n"
         "    );\n")
assert src.count(grdem) == 1, f"grdem {src.count(grdem)}"
src = src.replace(grdem, grdem +
    '    if std::env::var_os("COLM_GSSUN_PROBE").is_some() {\n'
    '        eprintln!("GRDEM {:5} qflx_sun={:e} qflx_sha={:e} gssun={:e} gssha={:e} gb={:e} tl={:e} laisun={:e} laisha={:e}",\n'
    '            phs_call, sunlit_demand, shaded_demand,\n'
    '            input.maximum_sunlit_leaf_conductance_umol_m2_s, input.maximum_shaded_leaf_conductance_umol_m2_s,\n'
    '            boundary_conductance, input.leaf_temperature_k,\n'
    '            input.sunlit_leaf_area_index, input.shaded_leaf_area_index);\n'
    '    }\n', 1)
grout = ("        let (sunlit_gs, shaded_gs) = conductance_from_transpiration(\n"
         "            input,\n"
         "            boundary_conductance,\n"
         "            input.maximum_sunlit_leaf_conductance_umol_m2_s,\n"
         "            input.maximum_shaded_leaf_conductance_umol_m2_s,\n"
         "            sunlit,\n"
         "            shaded,\n"
         "        );\n")
assert src.count(grout) == 1, f"grout {src.count(grout)}"
src = src.replace(grout, grout +
    '        if std::env::var_os("COLM_GSSUN_PROBE").is_some() {\n'
    '            eprintln!("GROUT {:5} gssun={:e} gssha={:e} etrsun={:e} etrsha={:e} qflx_sun={:e} qflx_sha={:e} tl={:e} laisun={:e} laisha={:e}",\n'
    '                phs_call, sunlit_gs, shaded_gs, sunlit, shaded, sunlit_demand, shaded_demand,\n'
    '                input.leaf_temperature_k, input.sunlit_leaf_area_index, input.shaded_leaf_area_index);\n'
    '        }\n', 1)
open(ph, "w").write(src)
print("   patched plant_hydraulics.rs")

src = open(lt).read()
ltc = "    let intercepted_rain = input.intercepted_rain_kg_m2_s.max(0.0);\n"
assert src.count(ltc) == 1, f"ltc {src.count(ltc)}"
src = src.replace(ltc,
    "    static LEAF_TEMP_PROBE_COUNT: std::sync::atomic::AtomicUsize =\n"
    "        std::sync::atomic::AtomicUsize::new(0);\n"
    "    let lt_call = LEAF_TEMP_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n" + ltc, 1)
grin = "            let hydraulic_output = plant_hydraulic_stress(\n"
assert src.count(grin) == 1, f"grin {src.count(grin)}"
src = src.replace(grin,
    '            if std::env::var_os("COLM_GSSUN_PROBE").is_some() {\n'
    '                eprintln!("GRIN  {:5} it={:3} gs0sun={:e} gs0sha={:e} laisun={:e} laisha={:e} tl={:e} psrf={:e}",\n'
    '                    crate::plant_hydraulics::PHS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed),\n'
    '                    iteration, maximum_sunlit_leaf_conductance_umol_m2_s, maximum_shaded_leaf_conductance_umol_m2_s,\n'
    '                    laisun, laisha, state.leaf_temperature_k, input.surface_pressure_pa);\n'
    '            }\n' + grin, 1)
g908 = "            root_flux_kg_m2_s = hydraulic_output.root_flux_kg_m2_s;\n"
assert src.count(g908) == 1, f"g908 {src.count(g908)}"
src = src.replace(g908,
    '            if std::env::var_os("COLM_GSSUN_PROBE").is_some() {\n'
    '                let pc = crate::plant_hydraulics::PHS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed) - 1;\n'
    '                let gssl = hydraulic_output.sunlit_stomatal_conductance_umol_m2_s;\n'
    '                let gssh = hydraulic_output.shaded_stomatal_conductance_umol_m2_s;\n'
    '                eprintln!("GR908 {:5} it={:3} gssun={:e} gssha={:e} gssun_can={:e} gssha_can={:e} laisun={:e} laisha={:e} tl={:e}",\n'
    '                    pc, iteration, gssl, gssh, gssl * laisun, gssh * laisha, laisun, laisha, state.leaf_temperature_k);\n'
    '                eprintln!("GR919 {:5} it={:3} gssun={:e} gssha={:e} rssun={:e} rssha={:e} tl={:e} laisun={:e} laisha={:e}",\n'
    '                    pc, iteration, gssl, gssh, sunlit_resistance.stomatal_resistance_s_m, shaded_resistance.stomatal_resistance_s_m,\n'
    '                    state.leaf_temperature_k, laisun, laisha);\n'
    '                eprintln!("GR941 {:5} it={:3} rssun={:e} rssha={:e} laisun={:e} laisha={:e}",\n'
    '                    pc, iteration, sunlit_resistance.stomatal_resistance_s_m * laisun,\n'
    '                    shaded_resistance.stomatal_resistance_s_m * laisha, laisun, laisha);\n'
    '            }\n' + g908, 1)
g1320 = "    let shaded_stomatal_conductance = laisha / last.leaf_shaded_resistance * resistance_conversion;\n"
assert src.count(g1320) == 1, f"g1320 {src.count(g1320)}"
src = src.replace(g1320, g1320 +
    '    if std::env::var_os("COLM_GSSUN_PROBE").is_some() {\n'
    '        eprintln!("GR1320 {:5} it={:3} gssun={:e} gssha={:e} rssun={:e} rssha={:e} laisun={:e} laisha={:e} tlbef={:e} tprcor={:e}",\n'
    '            lt_call, iteration - 1, sunlit_stomatal_conductance, shaded_stomatal_conductance,\n'
    '            last.leaf_sunlit_resistance, last.leaf_shaded_resistance, laisun, laisha,\n'
    '            previous_leaf_temperature, pressure_conversion);\n'
    '    }\n', 1)
grst = "        let mut root_flux_kg_m2_s = Vec::new();\n"
assert src.count(grst) == 1, f"grst {src.count(grst)}"
src = src.replace(grst,
    '        if std::env::var_os("COLM_GSSUN_PROBE").is_some() {\n'
    '            eprintln!("GRST  {:5} it={:3} parsun={:e} parsha={:e} rssun={:e} rssha={:e} eah={:e} tl={:e} psrf={:e} rb={:e} rbsha={:e}",\n'
    '                crate::plant_hydraulics::PHS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed),\n'
    '                iteration, input.sunlit_absorbed_par_w_m2, input.shaded_absorbed_par_w_m2,\n'
    '                sunlit_resistance.stomatal_resistance_s_m, shaded_resistance.stomatal_resistance_s_m,\n'
    '                canopy_vapor_pressure, state.leaf_temperature_k, input.surface_pressure_pa,\n'
    '                leaf_boundary_resistance, leaf_boundary_resistance / laisha);\n'
    '        }\n' + grst, 1)
open(lt, "w").write(src)
print("   patched leaf_temperature.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -25 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_GSSUN_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -a '^GRIN \|^GRDEM\|^GROUT\|^GR908\|^GR919\|^GR941\|^GR1320\|^GRST' "$WORK/rust.log" > "$WORK/rust_probe.txt" || true
echo "   rust probe lines: $(wc -l < "$WORK/rust_probe.txt")"

echo "== 内核侧前几轮 =="
grep -a '^GSIN ' "$WORK/fort_probe.txt" | sed -n "1,4p"
grep -a '^GSDEM' "$WORK/fort_probe.txt" | sed -n "1,4p"
grep -a '^GSOUT' "$WORK/fort_probe.txt" | sed -n "1,4p"
grep -a '^GS908' "$WORK/fort_probe.txt" | sed -n "1,4p"
grep -a '^GS919' "$WORK/fort_probe.txt" | sed -n "1,4p"
grep -a '^GS941' "$WORK/fort_probe.txt" | sed -n "1,4p"
grep -a '^GS1320' "$WORK/fort_probe.txt" | sed -n "1,4p"
echo "== Rust 侧前几轮 =="
grep -a '^GRIN ' "$WORK/rust_probe.txt" | sed -n "1,4p"
grep -a '^GRDEM' "$WORK/rust_probe.txt" | sed -n "1,4p"
grep -a '^GROUT' "$WORK/rust_probe.txt" | sed -n "1,4p"
grep -a '^GR908' "$WORK/rust_probe.txt" | sed -n "1,4p"
grep -a '^GR919' "$WORK/rust_probe.txt" | sed -n "1,4p"
grep -a '^GR941' "$WORK/rust_probe.txt" | sed -n "1,4p"
grep -a '^GR1320' "$WORK/rust_probe.txt" | sed -n "1,4p"
echo "== 输出目录 $WORK =="
