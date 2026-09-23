#!/usr/bin/env bash
# 第 293 轮续 2：`update_photosyn` 的两侧同位型探针。
#
# 第 293 轮已把 `pco2a` 的首分歧收到 `MOD_LeafTemperature_Extended.F90:1243-1244`
# 更新式里的 `assimsun/assimha`（= `update_photosyn` 的输出），而 `gah2o/raw/thm/
# respcsun/respcsha/rsoil` 全部逐位相同。本探针打进 `update_photosyn` 本身：
# 入口（尤其 `gsh2o` 的单位）、`calc_photo_params` 出口、6 次内迭代、出口。
#
# 内核标签 `UPIN/UPPH/UPIT/UPOUT`，Rust 侧 `URIN/URPH/URIT/UROUT`。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/upd16}
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

path = sys.argv[1]
lines = load(path)
after(lines, "   SAVE", ["   integer, save :: up_probe_n = 0"], "f save")
after(lines, "      gradm_used = gradm", [
    "      up_probe_n = up_probe_n + 1",
    "      WRITE(*,'(A,1X,I5,10(1X,ES23.15))') 'UPIN ', up_probe_n, &",
    "         gsh2o, pco2a, tlef, psrf, po2m, pco2m, par, rstfac, rb, pco2a/psrf",
], "f UPIN", occurrence=2)
after(lines, "      range = pco2m * ( 1. - 1.6/gradm_used ) - gammas", [
    "      WRITE(*,'(A,1X,I5,10(1X,ES23.15))') 'UPPH ', up_probe_n, &",
    "         vm, epar, respc, omss, gbh2o, gammas, rrkk, c3, c4, range",
], "f UPPH", occurrence=2)
after(lines, "         eyy(ic) = pco2i - pco2in                         ! pa", [
    "         WRITE(*,'(A,1X,I5,1X,I2,10(1X,ES23.15))') 'UPIT ', up_probe_n, ic, &",
    "            pco2i, assim, assimn, co2s, co2st, assmt, pco2in, eyy(ic), gsh2o, gammas",
], "f UPIT")
after(lines, "      ENDDO ITERATION_LOOP_UPDATE", [
    "      WRITE(*,'(A,1X,I5,4(1X,ES23.15))') 'UPOUT', up_probe_n, assim, respc, gsh2o, pco2a",
], "f UPOUT")
save(path, lines)
print("   patched MOD_AssimStomataConductance.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
for m in UPIN UPPH UPIT UPOUT; do
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
grep -a '^UPIN \|^UPPH \|^UPIT \|^UPOUT' "$WORK/kernel.log" > "$WORK/fort_probe.txt" || true
echo "   kernel lines: $(wc -l < "$WORK/fort_probe.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys
path = sys.argv[1]
src = open(path).read()
anchor = "const ITERATIONS: usize = 6;"
assert src.count(anchor) == 1
src = src.replace(anchor, anchor +
    "\n\n/// 探针用：与内核侧 `up_probe_n` 同步的 `update_photosyn` 调用序号。\n"
    "static UPDATE_PROBE_COUNT: std::sync::atomic::AtomicUsize =\n"
    "    std::sync::atomic::AtomicUsize::new(0);", 1)
ent = "    let photo = photosynthesis_parameters(input.photosynthesis)?;\n"
assert src.count(ent) == 2
i = src.index(ent, src.index(ent) + 1)  # 第二处 = update_photosynthesis
i += len(ent)
src = src[:i] + (
    '    let up_call = UPDATE_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n'
    '    if std::env::var_os("COLM_UPD_PROBE").is_some() {\n'
    '        eprintln!("URIN  {:5} gsh2o={:e} pco2a={:e} tlef={:e} psrf={:e} po2m={:e} pco2m={:e} par={:e} rstfac={:e} rb={:e} co2a={:e}",\n'
    '            up_call, input.canopy_conductance_h2o_mol_m2_s, input.canopy_air_co2_pa,\n'
    '            input.photosynthesis.leaf_temperature_k, input.photosynthesis.air_pressure_pa,\n'
    '            input.photosynthesis.oxygen_partial_pressure_pa, input.atmospheric_co2_pa,\n'
    '            input.photosynthesis.absorbed_par_w_m2, input.photosynthesis.soil_water_stress,\n'
    '            input.photosynthesis.leaf_boundary_resistance_s_m,\n'
    '            input.canopy_air_co2_pa / input.photosynthesis.air_pressure_pa);\n'
    '    }\n') + src[i:]
rng = "    let range = input.atmospheric_co2_pa * (1.0 - f77(1.6) / gradm) - photo.co2_compensation_pa;\n"
assert src.count(rng) == 2
j = src.index(rng, src.index(rng) + 1) + len(rng)
src = src[:j] + (
    '    if std::env::var_os("COLM_UPD_PROBE").is_some() {\n'
    '        eprintln!("URPH  {:5} vm={:e} epar={:e} respc={:e} omss={:e} gbh2o={:e} gammas={:e} rrkk={:e} c3={:e} c4={:e} range={:e}",\n'
    '            up_call, photo.maximum_carboxylation_mol_m2_s, photo.electron_transport_mol_m2_s,\n'
    '            photo.respiration_mol_m2_s, photo.sink_limit_mol_m2_s_pa,\n'
    '            photo.boundary_conductance_h2o_mol_m2_s, photo.co2_compensation_pa,\n'
    '            photo.rubisco_co2_constant_pa, photo.c3_fraction, photo.c4_fraction, range);\n'
    '    }\n') + src[j:]
sit = "        errors[iteration - 1] = internal - next;\n"
assert src.count(sit) == 1
src = src.replace(sit, sit +
    '        if std::env::var_os("COLM_UPD_PROBE").is_some() {\n'
    '            eprintln!("URIT  {:5} ic={:2} pco2i={:e} assim={:e} assimn={:e} co2s={:e} assmt={:e} pco2in={:e} eyy={:e} gsh2o={:e} gammas={:e}",\n'
    '                up_call, iteration, internal, assimilation, net_assimilation, co2_surface,\n'
    '                positive_assimilation, next, errors[iteration - 1],\n'
    '                input.canopy_conductance_h2o_mol_m2_s, photo.co2_compensation_pa);\n'
    '        }\n', 1)
ok = "    Ok(PhotosynthesisUpdateState {\n"
assert src.count(ok) == 1
src = src.replace(ok,
    '    if std::env::var_os("COLM_UPD_PROBE").is_some() {\n'
    '        eprintln!("UROUT {:5} assim={:e} respc={:e} gsh2o={:e} pco2a={:e}",\n'
    '            up_call, assimilation, photo.respiration_mol_m2_s,\n'
    '            input.canopy_conductance_h2o_mol_m2_s, input.canopy_air_co2_pa);\n'
    '    }\n' + ok, 1)
open(path, "w").write(src)
print("   patched photosynthesis.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -25 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_UPD_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -a '^URIN \|^URPH \|^URIT \|^UROUT' "$WORK/rust.log" > "$WORK/rust_probe.txt" || true
echo "   rust lines: $(wc -l < "$WORK/rust_probe.txt")"

python3 - "$WORK" <<'CEOF'
import re, sys
work = sys.argv[1]
spec = {
    "UPIN": (["gsh2o", "pco2a", "tlef", "psrf", "po2m", "pco2m", "par", "rstfac", "rb", "co2a"], "URIN"),
    "UPPH": (["vm", "epar", "respc", "omss", "gbh2o", "gammas", "rrkk", "c3", "c4", "range"], "URPH"),
    "UPIT": (["pco2i", "assim", "assimn", "co2s", "co2st", "assmt", "pco2in", "eyy", "gsh2o", "gammas"], "URIT"),
    "UPOUT": (["assim", "respc", "gsh2o", "pco2a"], "UROUT"),
}
def fl(x):
    try:
        return float(x)
    except ValueError:
        m = re.match(r"^([-\d.]+)([+-]\d+)$", x)
        return float(m.group(1) + "E" + m.group(2))
def kern(p):
    out = {}
    for line in open(p, errors="replace"):
        q = line.split()
        if not q or q[0] not in spec:
            continue
        if q[0] == "UPIT":
            out.setdefault(q[0], {})[(int(q[1]), int(q[2]))] = [fl(x) for x in q[3:]]
        else:
            out.setdefault(q[0], {})[(int(q[1]), 0)] = [fl(x) for x in q[2:]]
    return out
def rust(p):
    out = {}
    for line in open(p, errors="replace"):
        q = line.split()
        if not q or q[0] not in [v[1] for v in spec.values()]:
            continue
        n = int(q[1])
        ic = int(re.search(r"\bic=\s*(\d+)", line).group(1)) if re.search(r"\bic=\s*(\d+)", line) else 0
        out.setdefault(q[0], {})[(n, ic)] = {k: float(v) for k, v in re.findall(r"(\w+)=\s*([-\w.eE+]+)", line) if k != "ic"}
    return out
def close(a, b):
    return a == b or abs(a - b) / max(abs(a), abs(b), 1e-300) <= 2e-14
fk, fr = kern(work + "/fort_probe.txt"), rust(work + "/rust_probe.txt")
for ktag, (names, rtag) in spec.items():
    kk, rr = fk.get(ktag, {}), fr.get(rtag, {})
    keys = sorted(set(kk) & set(rr))
    first = None
    nd = 0
    for k in keys:
        diffs = [(n, kk[k][i], rr[k][n]) for i, n in enumerate(names) if n in rr[k] and not close(kk[k][i], rr[k][n])]
        if diffs:
            nd += 1
            if first is None:
                first = (k, diffs)
    if first is None:
        print(f"{ktag:6s} identical ({len(keys)})")
    else:
        k, diffs = first
        print(f"{ktag:6s} n={k[0]} ic={k[1]}; {nd}/{len(keys)} differ; first:")
        for n, a, b in diffs:
            print(f"     {n:8s} K={a!r:24s} R={b!r:24s} rel={abs(a-b)/max(abs(a),abs(b),1e-300):.3e}")
CEOF
