#!/usr/bin/env bash
# 第 345-349 轮：PHS 求解链与 `gs0sun`/`stomata` 的**位型**（hex）双侧探针。
#
# 为什么必须打 hex：内核侧 `ES23.15`（16 位有效数字）+ `gssun_cmp.py` 的 2e-14 相对
# 容差对 **1 ULP 是盲的** —— 它报"全同"，而 restart 里 `vegwp`/`ldew` 却差 1 ULP。
# 只有 `TRANSFER(x,0_8)`（内核）/ `f64::to_bits()`（Rust）的位型才是逐位判据。
#
# 六个打点（两侧同序、字段顺序必须一致）：
#   PHXI  gb_mol,gssun,gssha,laisun,laisha,sai,fwet,tl,qsatl,qaf,qg,qm,psrf,rhoair,raw,rd,rss
#         —— `getqflx_gs2qflx_twoleaf`（内核）/ `transpiration_from_conductance`（Rust）**入参**
#   PHXG  qflx_sun,qflx_sha          —— 同一例程**返回后**（PHS 的入场需求通量）
#   PHXQ  x_root_top,qeroot,dqeroot  —— `getrootqflx_x2qe` 返回之后
#   PHXA  A11,A13,A22,A23,A31,A32,A33,A34,A43,A44,f1..f4 —— `spacAF_twoleaf` 组装后
#   PHXD  x(1:4),dx(1:4)             —— `x = x + dx` 之后
#   PHXF  x(1:4),qeroot,x_root_top   —— `x(root) = x_root_top` 之后
#   PHXR  rssun,rssha,tl,tprcor,laisun,laisha,gs0sun,gs0sha
#         —— `gs0sun`/`gs0sha` 的**出生点**（`MOD_LeafTemperature_Extended.F90:832-833`）
#   PHXS  rssun,rssha,assimsun,assimsha,respcsun,respcsha
#         —— `stomata`（`MOD_AssimStomataConductance.F90`）**返回后**
#
# 用法: STEPS=63 WORK=/tmp/gf/phshex63 bash oracle/scripts/phs_hex_probe.sh
#       （跑完自动还原四个源文件并重编内核回干净版；SKIP_REBUILD=1 跳过重编）
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/phshex}
STEPS=${STEPS:-18}
CASE=${CASE:-CN-Cng-wet}   # 第 355 轮起可换窗口（干窗用 CASE=CN-Cng）
TOTAL=$((1800 * STEPS))
END_SEC=$((TOTAL % 86400))
END_DAY=$((1 + TOTAL / 86400))
FORT_PH="vendor/CoLM202X/main/MOD_PlantHydraulic.F90"
# `gs0sun`/`gs0sha` 与 `stomata` 的出生点在**叶温**那一份里（不是 PHS 那一份）。
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

python3 - "$BASE/$FORT_PH" <<'FEOF'
import sys
path = sys.argv[1]
lines = open(path).read().split("\n")

def after(lines, anchor, block, tag, which="only"):
    idx = [i for i, l in enumerate(lines) if l.strip() == anchor]
    if which == "first":
        assert idx, (tag, len(idx))
        at = idx[0]
    else:
        assert len(idx) == 1, (tag, len(idx))
        at = idx[0]
    lines[at + 1:at + 1] = block
    return lines

hex8 = ("         WRITE(*,'(A,8(1X,Z16.16))') 'PHXD', &\n"
        "              TRANSFER(x(1),0_8),TRANSFER(x(2),0_8),TRANSFER(x(3),0_8),TRANSFER(x(4),0_8), &\n"
        "              TRANSFER(dx(1),0_8),TRANSFER(dx(2),0_8),TRANSFER(dx(3),0_8),TRANSFER(dx(4),0_8)").split("\n")
hex6 = ("         WRITE(*,'(A,6(1X,Z16.16))') 'PHXF', &\n"
        "              TRANSFER(x(1),0_8),TRANSFER(x(2),0_8),TRANSFER(x(3),0_8),TRANSFER(x(4),0_8), &\n"
        "              TRANSFER(qeroot,0_8),TRANSFER(x_root_top,0_8)").split("\n")

# 字段顺序必须两侧一致：x_root_top(入参) → qeroot → dqeroot
hexq = ("         WRITE(*,'(A,3(1X,Z16.16))') 'PHXQ', &\n"
        "              TRANSFER(x_root_top,0_8),TRANSFER(qeroot,0_8),TRANSFER(dqeroot,0_8)").split("\n")
lines = after(lines, "CALL getrootqflx_x2qe(nl_soil,smp,x_root_top ,z_soi,k_soil_root,k_ax_root,qeroot,dqeroot)", hexq, "PHXQ")
hexa = ("         WRITE(*,'(A,14(1X,Z16.16))') 'PHXA', &\n"
        "              TRANSFER(A11,0_8),TRANSFER(A13,0_8),TRANSFER(A22,0_8),TRANSFER(A23,0_8), &\n"
        "              TRANSFER(A31,0_8),TRANSFER(A32,0_8),TRANSFER(A33,0_8),TRANSFER(A34,0_8), &\n"
        "              TRANSFER(A43,0_8),TRANSFER(A44,0_8), &\n"
        "              TRANSFER(f(1),0_8),TRANSFER(f(2),0_8),TRANSFER(f(3),0_8),TRANSFER(f(4),0_8)").split("\n")
lines = after(lines, "f(root) = sai * kmax_xyl / htop * fr * (x(root)-x(xyl)-grav1) - qeroot", hexa, "PHXA")
hexg = ("         WRITE(*,'(A,2(1X,Z16.16))') 'PHXG', &\n"
        "              TRANSFER(qflx_sun,0_8),TRANSFER(qflx_sha,0_8)").split("\n")
# 这个尾巴在 gs2qflx / qflx2gs / getvegwp 三处都有；gs2qflx（calcstress 里那个）在文件里最靠前
lines = after(lines, "rhoair,psrf,laisun,laisha,sai,fwet,tl,rss,raw,rd,qg,qm)", hexg, "PHXG", which="first")
# 第 349 轮：gs2qflx 的**入参**（判断 1 ULP 是"这个例程算错"还是"入参已经不同"）
hexi = ("         WRITE(*,'(A,17(1X,Z16.16))') 'PHXI', &\n"
        "              TRANSFER(gb_mol,0_8),TRANSFER(gssun,0_8),TRANSFER(gssha,0_8), &\n"
        "              TRANSFER(laisun,0_8),TRANSFER(laisha,0_8),TRANSFER(sai,0_8),TRANSFER(fwet,0_8), &\n"
        "              TRANSFER(tl,0_8),TRANSFER(qsatl,0_8),TRANSFER(qaf,0_8),TRANSFER(qg,0_8), &\n"
        "              TRANSFER(qm,0_8),TRANSFER(psrf,0_8),TRANSFER(rhoair,0_8), &\n"
        "              TRANSFER(raw,0_8),TRANSFER(rd,0_8),TRANSFER(rss,0_8)").split("\n")
lines = after(lines, "gssha=gs0sha", hexi, "PHXI")
# 第 349 轮：`gs0sun/gs0sha` **出生点**（判"种子在 stomata 的 rssun/rssha，还是在这个公式里"）
hexr = ("         WRITE(*,'(A,8(1X,Z16.16))') 'PHXR', &\n"
        "              TRANSFER(rssun,0_8),TRANSFER(rssha,0_8),TRANSFER(tl,0_8),TRANSFER(tprcor,0_8), &\n"
        "              TRANSFER(laisun,0_8),TRANSFER(laisha,0_8),TRANSFER(gs0sun,0_8),TRANSFER(gs0sha,0_8)").split("\n")

lines = after(lines, "x=x+dx", hex8, "PHXD")
# `x(root) = x_root_top` 在 calcstress_twoleaf 与 getvegwp_twoleaf 各出现一次；
# calcstress 在前，取**第一处**（只有它带 dx/qeroot 的上下文）。
lines = after(lines, "x(root) = x_root_top", hex6, "PHXF", which="first")
open(path, "w").write("\n".join(lines))
print("   patched MOD_PlantHydraulic.F90")
FEOF

python3 - "$BASE/$FORT_LT" <<'FEOF2'
import sys
path = sys.argv[1]
lines = open(path).read().split("\n")

def after(lines, anchor, block, tag):
    idx = [i for i, l in enumerate(lines) if l.strip() == anchor]
    assert len(idx) == 1, (tag, len(idx))
    lines[idx[0] + 1:idx[0] + 1] = block
    return lines

# PHXR：`gs0sun`/`gs0sha` 的出生点 —— 判"1 ULP 是在 stomata 的 rssun/rssha 里，
# 还是在这个换算公式里"。字段：rssun, rssha, tl, tprcor, laisun, laisha, gs0sun, gs0sha
hexr = ("               WRITE(*,'(A,8(1X,Z16.16))') 'PHXR', &\n"
        "                    TRANSFER(rssun,0_8),TRANSFER(rssha,0_8),TRANSFER(tl,0_8),TRANSFER(tprcor,0_8), &\n"
        "                    TRANSFER(laisun,0_8),TRANSFER(laisha,0_8),TRANSFER(gs0sun,0_8),TRANSFER(gs0sha,0_8)").split("\n")
lines = after(lines, "gs0sha = min( 1.e6, 1./(rssha*tl/tprcor) )/ laisha * 1.e6 * o3coefg_sha", hexr, "PHXR")
# 第 349 轮追加：`stomata` 的**返回值**（判轨迹里的 1 ULP 是 `assim` 还是 `rst`）
hexs = ("               WRITE(*,'(A,6(1X,Z16.16))') 'PHXS', &\n"
        "                    TRANSFER(rssun,0_8),TRANSFER(rssha,0_8),TRANSFER(assimsun,0_8), &\n"
        "                    TRANSFER(assimsha,0_8),TRANSFER(respcsun,0_8),TRANSFER(respcsha,0_8)").split("\n")
lines = after(lines, "assimsha ,respcsha ,rssha    )", hexs, "PHXS")
open(path, "w").write("\n".join(lines))
print("   patched MOD_LeafTemperature_Extended.F90")
FEOF2

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -20 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
for m in PHXD PHXF PHXI PHXR PHXS PHXG PHXQ PHXA; do
  grep -q "$m" "$WORK/strings.txt" || { echo "!! marker $m missing"; exit 3; }
done

# ---- 内核跑 STEPS 步 $CASE ----
sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$WORK/out/'#" \
    -e "s#^   DEF_forcing_namelist.*#   DEF_forcing_namelist = '$WORK/forcing.nml'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$WORK/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$WORK/runtime_unused/'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = $END_DAY#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = $END_SEC#" \
    -e "s#^   DEF_HIST_FREQ.*#   DEF_HIST_FREQ    = 'TIMESTEP'#" \
    "$BASE/oracle/work/$CASE/case.nml" > "$WORK/case.nml"
cp "$BASE/oracle/work/$CASE/forcing.nml" "$WORK/forcing.nml"
# PLUMBER2 挂载点变了（/Volumes/Data01 -> /Volumes/Data）：只改这份拷贝
sed -i '' "s#/Volumes/Data01/Data/PLUMBER2s/Forcing/#/Users/zhongwangwei/Desktop/Github/CoLM-Desktop/examples/Forcing/#" "$WORK/forcing.nml"
cp -R "$BASE/oracle/work/$CASE/out" "$WORK/out"
rm -rf "$WORK/out/$CASE/history"
( cd "$WORK/run" && "$BASE/kernels/default/colm.x" "$WORK/case.nml" > "$WORK/kernel.log" 2>&1 ) \
  || { echo "!! kernel run failed"; exit 4; }
grep -q 'CoLM Execution Completed' "$WORK/kernel.log" || { echo "!! incomplete"; tail -5 "$WORK/kernel.log"; exit 4; }
grep -a '^PHX' "$WORK/kernel.log" > "$WORK/fort_probe.txt" || true
echo "   kernel probe lines: $(wc -l < "$WORK/fort_probe.txt")"

# ---- Rust 侧打点 ----
python3 - "$BASE/$RUST_PH" <<'REOF'
import sys
path = sys.argv[1]
src = open(path).read()

anchor = """        for (value, delta) in potential.iter_mut().zip(change) {
            *value += delta;
        }
"""
block = """        for (value, delta) in potential.iter_mut().zip(change) {
            *value += delta;
        }
        println!(
            "PHXD {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",
            potential[SUNLIT].to_bits(),
            potential[SHADED].to_bits(),
            potential[XYLEM].to_bits(),
            potential[ROOT].to_bits(),
            change[SUNLIT].to_bits(),
            change[SHADED].to_bits(),
            change[XYLEM].to_bits(),
            change[ROOT].to_bits()
        );
"""
assert src.count(anchor) == 1, src.count(anchor)
src = src.replace(anchor, block)

anchorc = """    let (sunlit_demand, shaded_demand) = transpiration_from_conductance(
"""
blockc = """    println!(
        "PHXI {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",
        boundary_conductance.to_bits(),
        input.maximum_sunlit_leaf_conductance_umol_m2_s.to_bits(),
        input.maximum_shaded_leaf_conductance_umol_m2_s.to_bits(),
        input.sunlit_leaf_area_index.to_bits(),
        input.shaded_leaf_area_index.to_bits(),
        input.stem_area_index.to_bits(),
        input.wet_canopy_fraction.to_bits(),
        input.leaf_temperature_k.to_bits(),
        input.leaf_saturation_specific_humidity.to_bits(),
        input.canopy_air_specific_humidity.to_bits(),
        input.ground_specific_humidity.to_bits(),
        input.reference_specific_humidity.to_bits(),
        input.surface_pressure_pa.to_bits(),
        input.air_density_kg_m3.to_bits(),
        input.reference_to_canopy_moisture_resistance_s_m.to_bits(),
        input.ground_to_canopy_moisture_resistance_s_m.to_bits(),
        input.soil_surface_resistance_s_m.to_bits()
    );
    let (sunlit_demand, shaded_demand) = transpiration_from_conductance(
"""
assert src.count(anchorc) == 1, src.count(anchorc)
src = src.replace(anchorc, blockc)

anchorg = """    let mut potential = state.vegetation_water_potential_mm;
"""
blockg = """    println!(
        "PHXG {:016X} {:016X}",
        sunlit_demand.to_bits(),
        shaded_demand.to_bits()
    );
    let mut potential = state.vegetation_water_potential_mm;
"""
assert src.count(anchorg) == 1, src.count(anchorg)
src = src.replace(anchorg, blockg)

anchora = """    f[ROOT] = root_term - root_flux;
"""
blocka = """    f[ROOT] = root_term - root_flux;
    println!(
        "PHXA {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",
        a11.to_bits(), a13.to_bits(), a22.to_bits(), a23.to_bits(),
        a31.to_bits(), a32.to_bits(), a33.to_bits(), a34.to_bits(),
        a43.to_bits(), a44.to_bits(),
        f[SUNLIT].to_bits(), f[SHADED].to_bits(), f[XYLEM].to_bits(), f[ROOT].to_bits()
    );
"""
assert src.count(anchora) == 1, src.count(anchora)
src = src.replace(anchora, blocka)

anchorq = """        )?;
        let change = spac_change(
"""
blockq = """        )?;
        println!(
            "PHXQ {:016X} {:016X} {:016X}",
            potential[ROOT].to_bits(),
            root_flux.to_bits(),
            root_flux_slope.to_bits()
        );
        let change = spac_change(
"""
assert src.count(anchorq) == 1, src.count(anchorq)
src = src.replace(anchorq, blockq)

anchor2 = """        potential[ROOT] = root_top;
"""
block2 = """        potential[ROOT] = root_top;
        println!(
            "PHXF {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",
            potential[SUNLIT].to_bits(),
            potential[SHADED].to_bits(),
            potential[XYLEM].to_bits(),
            potential[ROOT].to_bits(),
            (sunlit + shaded).to_bits(),
            root_top.to_bits()
        );
"""
assert src.count(anchor2) == 1, src.count(anchor2)
src = src.replace(anchor2, block2)
open(path, "w").write(src)
print("   patched plant_hydraulics.rs")
REOF

python3 - "$BASE/$RUST_LT" <<'REOF2'
import sys
path = sys.argv[1]
src = open(path).read()
anchors = """        let mut root_flux_kg_m2_s = Vec::new();
"""
blocks = """        println!(
            "PHXS {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",
            sunlit_resistance.stomatal_resistance_s_m.to_bits(),
            shaded_resistance.stomatal_resistance_s_m.to_bits(),
            sunlit_resistance.assimilation_mol_m2_s.to_bits(),
            shaded_resistance.assimilation_mol_m2_s.to_bits(),
            sunlit_resistance.respiration_mol_m2_s.to_bits(),
            shaded_resistance.respiration_mol_m2_s.to_bits()
        );
        let mut root_flux_kg_m2_s = Vec::new();
"""
assert src.count(anchors) == 1, src.count(anchors)
src = src.replace(anchors, blocks)

anchorr = """            gs0sha = Some(maximum_shaded_leaf_conductance_umol_m2_s);
"""
blockr = """            println!(
                "PHXR {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}",
                sunlit_resistance.stomatal_resistance_s_m.to_bits(),
                shaded_resistance.stomatal_resistance_s_m.to_bits(),
                state.leaf_temperature_k.to_bits(),
                pressure_conversion.to_bits(),
                laisun.to_bits(),
                laisha.to_bits(),
                maximum_sunlit_leaf_conductance_umol_m2_s.to_bits(),
                maximum_shaded_leaf_conductance_umol_m2_s.to_bits()
            );
            gs0sha = Some(maximum_shaded_leaf_conductance_umol_m2_s);
"""
assert src.count(anchorr) == 1, src.count(anchorr)
src = src.replace(anchorr, blockr)
open(path, "w").write(src)
print("   patched leaf_temperature.rs")
REOF2

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -25 "$WORK/rust_build.log"; exit 3; }
( cd "$WORK" && "$BASE/target/debug/colm-rs" "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -a '^PHX' "$WORK/rust.log" > "$WORK/rust_probe.txt" || true
echo "   rust probe lines: $(wc -l < "$WORK/rust_probe.txt")"

python3 - "$WORK" <<'PY'
import sys
work = sys.argv[1]
def rows(p):
    out = []
    for line in open(p):
        parts = line.split()
        if parts and parts[0] in ("PHXG", "PHXA", "PHXQ", "PHXD", "PHXF", "PHXI", "PHXR", "PHXS"):
            out.append(parts)
    return out
f = rows(f'{work}/fort_probe.txt')
r = rows(f'{work}/rust_probe.txt')
print(f'   kernel {len(f)} 条 / rust {len(r)} 条')
n = min(len(f), len(r))
first = None
diff_kinds = {}
for i in range(n):
    if f[i][0] != r[i][0]:
        print(f'   标签错位 @{i}: {f[i][0]} vs {r[i][0]}'); break
    for k in range(1, len(f[i])):
        x = f[i][k].upper().lstrip("0") or "0"
        y = r[i][k].upper().lstrip("0") or "0"
        if x != y:
            diff_kinds.setdefault(f[i][0], {}).setdefault(k, 0)
            diff_kinds[f[i][0]][k] += 1
            if first is None:
                first = (i, f[i][0], k, f[i][k], r[i][k])
print('   首个差异:', first)
print('   差异按标签/字段计数:', {k: dict(v) for k, v in diff_kinds.items()})
PY
echo "== 输出目录 $WORK =="

