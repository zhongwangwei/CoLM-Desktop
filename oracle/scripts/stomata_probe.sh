#!/usr/bin/env bash
# 第 293 轮：`stomata` 遮荫支的同位型探针（找 `rssha` 9.1e-7 的首个分叉点）。
#
# 第 292 轮已证：`stomata` 遮荫调用的**全部可打印入参**与 sunlit 的 `rssun` 都逐位
# 相同，输出 `rssha` 在 kernel call 289（PHS 调用序号）差 9.1e-7 ⇒ 差在 `stomata`
# 内部。本探针把 `MOD_AssimStomataConductance.F90:stomata` 的入口、`calc_photo_params`
# 出口、6 次内迭代的每一步、出口都打出来，Rust 侧在 `photosynthesis.rs:stomata`
# 同点对打。
#
# 第 351 轮：两侧全部改成**位型**（`TRANSFER(x,0_8)` / `f64::to_bits()` + `Z16.16`）。
# 原版是 `ES23.15`/`{:e}`（16 位有效数字）+ 2e-14 相对容差 —— 对 1 ULP 是**盲的**，
# 而这条链现在的残留正是 1 ULP。`CASE=CN-Cng-wet STEPS=63` 可直接打湿窗。
#
# 内核标签（`STIN/STPH/STIT/STOUT`），Rust 侧对应 `SRIN/SRPH/SRIT/SROUT`。
# 字段布局**两侧逐位对齐**（`stomata_cmp.py` 按位置比 hex）：
#   STIN  13：tlef psrf po2m pco2m pco2a ea ei par rb rstfac cint(1..3)   （`ra` 上游未用，省）
#   STPH  11：vm epar respc omss gbh2o gammas rrkk c3 c4 bintc range
#   STIT  10：pco2i omc ome assim assimn co2s assmt gsh2o pco2in eyy
#         ⚠ WUE 支（`DEF_USE_WUEST=.true.` 且 `|c4-1|>=0.001`，黄金算例走这一支）里
#           `pco2i`/`eyy` 两列**两侧不可比**：内核在支内把 `pco2i` 重写成
#           `pco2i_c`/`pco2i_e`（重写后 `pco2in = pco2i` ⇒ `eyy≡0` ⇒ `ic=1` 就退出），
#           而 Rust 打的是**重写前**的 `internal_co2`，`eyy = internal_co2 - next_co2`。
#           其余 8 列（omc/ome/assim/assimn/co2s/assmt/gsh2o/pco2in）在入参相同的调用上逐位可比。
#           这处 WUE 迭代计数偏差已证**惰性**（同调用内 6 轮的值逐位相同），未落地。
#   STOUT  4：gsh2o rst tprcor tlef
# 调用序号两侧 1:1（每个叶温迭代 2 次：先 sunlit 后 shaded；奇数=sunlit、偶数=shaded）。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/stomata16}
STEPS=${STEPS:-16}
CASE=${CASE:-CN-Cng}
TOTAL=$((1800 * STEPS))
END_SEC=$((TOTAL % 86400))
END_DAY=$((1 + TOTAL / 86400))
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
    "      WRITE(*,'(A,1X,I5,13(1X,Z16.16))') 'STIN ', st_probe_n, &",
    "         TRANSFER(tlef,0_8),TRANSFER(psrf,0_8),TRANSFER(po2m,0_8),TRANSFER(pco2m,0_8), &",
    "         TRANSFER(pco2a,0_8),TRANSFER(ea,0_8),TRANSFER(ei,0_8),TRANSFER(par,0_8), &",
    "         TRANSFER(rb,0_8),TRANSFER(rstfac,0_8), &",
    "         TRANSFER(cint(1),0_8),TRANSFER(cint(2),0_8),TRANSFER(cint(3),0_8)",
], "f STIN")
after(lines, "      range = pco2m * ( 1. - 1.6/gradm_used ) - gammas", [
    "      WRITE(*,'(A,1X,I5,11(1X,Z16.16))') 'STPH ', st_probe_n, &",
    "         TRANSFER(vm,0_8),TRANSFER(epar,0_8),TRANSFER(respc,0_8),TRANSFER(omss,0_8), &",
    "         TRANSFER(gbh2o,0_8),TRANSFER(gammas,0_8),TRANSFER(rrkk,0_8), &",
    "         TRANSFER(c3,0_8),TRANSFER(c4,0_8),TRANSFER(bintc,0_8),TRANSFER(range,0_8)",
], "f STPH")
after(lines, "         eyy(ic) = pco2i - pco2in                              ! pa", [
    "         WRITE(*,'(A,1X,I5,1X,I2,10(1X,Z16.16))') 'STIT ', st_probe_n, ic, &",
    "            TRANSFER(pco2i,0_8),TRANSFER(omc,0_8),TRANSFER(ome,0_8),TRANSFER(assim,0_8), &",
    "            TRANSFER(assimn,0_8),TRANSFER(co2s,0_8),TRANSFER(assmt,0_8), &",
    "            TRANSFER(gsh2o,0_8),TRANSFER(pco2in,0_8),TRANSFER(eyy(ic),0_8)",
], "f STIT")
after(lines, "      rst   = min( 1.e6, 1./(gsh2o*tlef/tprcor) )     ! s m-1", [
    "      WRITE(*,'(A,1X,I5,4(1X,Z16.16))') 'STOUT', st_probe_n, &",
    "         TRANSFER(gsh2o,0_8),TRANSFER(rst,0_8),TRANSFER(tprcor,0_8),TRANSFER(tlef,0_8)",
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
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = $END_DAY#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = $END_SEC#" \
    -e "s#^   DEF_HIST_FREQ.*#   DEF_HIST_FREQ    = 'TIMESTEP'#" \
    "$BASE/oracle/work/$CASE/case.nml" > "$WORK/case.nml"
cp "$BASE/oracle/work/$CASE/forcing.nml" "$WORK/forcing.nml"
# PLUMBER2 挂载点变了（/Volumes/Data01 -> 仓库自带 examples/Forcing）：只改这份拷贝
sed -i '' "s#/Volumes/Data01/Data/PLUMBER2s/Forcing/#$BASE/examples/Forcing/#" "$WORK/forcing.nml"
cp -R "$BASE/oracle/work/$CASE/out" "$WORK/out"
rm -rf "$WORK/out/$CASE/history"
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
    '        eprintln!("SRIN  {:5} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",\n'
    '            st_call, input.photosynthesis.leaf_temperature_k.to_bits(), input.photosynthesis.air_pressure_pa.to_bits(),\n'
    '            input.photosynthesis.oxygen_partial_pressure_pa.to_bits(), input.atmospheric_co2_pa.to_bits(), input.canopy_air_co2_pa.to_bits(),\n'
    '            input.canopy_air_vapor_pressure_pa.to_bits(), input.leaf_saturation_vapor_pressure_pa.to_bits(),\n'
    '            input.photosynthesis.absorbed_par_w_m2.to_bits(), input.photosynthesis.leaf_boundary_resistance_s_m.to_bits(),\n'
    '            input.photosynthesis.soil_water_stress.to_bits(), input.photosynthesis.canopy_integration[0].to_bits(),\n'
    '            input.photosynthesis.canopy_integration[1].to_bits(), input.photosynthesis.canopy_integration[2].to_bits());\n'
    '    }\n', 1)

# 文件里 `range` 出现两次（stomata 与 update_photosynthesis），stomata 在前。
# 第 350 轮起它是 `input\n .atmospheric_co2_pa\n .mul_add(...)` 三行式，
# 所以按"第一条以 `let range = input` 开头、以 `;` 结尾的语句"定位，别写死平铺串。
sph = "    let range = input"
assert src.count(sph) == 2
i = src.index(sph)
i = src.index(";\n", i) + 2
src = src[:i] + (
    '    if std::env::var_os("COLM_STOMATA_PROBE").is_some() {\n'
    '        eprintln!("SRPH  {:5} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",\n'
    '            st_call, photo.maximum_carboxylation_mol_m2_s.to_bits(), photo.electron_transport_mol_m2_s.to_bits(),\n'
    '            photo.respiration_mol_m2_s.to_bits(), photo.sink_limit_mol_m2_s_pa.to_bits(),\n'
    '            photo.boundary_conductance_h2o_mol_m2_s.to_bits(), photo.co2_compensation_pa.to_bits(),\n'
    '            photo.rubisco_co2_constant_pa.to_bits(), photo.c3_fraction.to_bits(), photo.c4_fraction.to_bits(),\n'
    '            bintc.to_bits(), range.to_bits());\n'
    '    }\n') + src[i:]

sit = "        errors[iteration - 1] = internal_co2 - next_co2;\n"
assert src.count(sit) == 1
src = src.replace(sit, sit +
    '        if std::env::var_os("COLM_STOMATA_PROBE").is_some() {\n'
    '            eprintln!("SRIT  {:5} {:2} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",\n'
    '                st_call, iteration, internal_co2.to_bits(), omc.to_bits(), ome.to_bits(),\n'
    '                assimilation.to_bits(), net_assimilation.to_bits(),\n'
    '                co2_surface.to_bits(), positive_assimilation.to_bits(), conductance.to_bits(),\n'
    '                next_co2.to_bits(), errors[iteration - 1].to_bits());\n'
    '        }\n', 1)

sout = "        f77(44.6 * 273.16) * input.photosynthesis.air_pressure_pa / f77(1.013e5);\n"
assert src.count(sout) == 1
src = src.replace(sout, sout +
    '    if std::env::var_os("COLM_STOMATA_PROBE").is_some() {\n'
    '        eprintln!("SROUT {:5} {:016X} {:016X} {:016X} {:016X}",\n'
    '            st_call, conductance.to_bits(),\n'
    '            (1.0 / (conductance * input.photosynthesis.leaf_temperature_k / pressure_conversion)).min(f77(1.0e6)).to_bits(),\n'
    '            pressure_conversion.to_bits(), input.photosynthesis.leaf_temperature_k.to_bits());\n'
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
