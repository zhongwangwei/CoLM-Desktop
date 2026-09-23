#!/usr/bin/env bash
# 第 293 轮续：`pco2a` 更新式的两侧同位型探针。
#
# 第 293 轮已证：`stomata` 遮荫支的全部入参（含 `pco2a`）里，**首个分叉是 `pco2a`**
# （kernel stomata 调用 577；相对 9.5e-7），而 `pco2a` 由上一轮迭代末尾
# `MOD_LeafTemperature_Extended.F90:1243-1244` 更新：
#   pco2a = pco2m - 1.37*psrf/max(0.446,gah2o)*(assimsun+assimsha-respcsun-respcsha-rsoil)
# 本探针把这条更新式的每个输入都打出来，判"差在 gah2o(=1/raw*tprcor/thm)"
# 还是"差在 update_photosyn 的四个输出之和"。
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/pco2a16}
STEPS=${STEPS:-16}
END_SEC=$((1800 * STEPS))
FORT="vendor/CoLM202X/extends/interception/MOD_LeafTemperature_Extended.F90"
RUST="crates/colm-core/src/leaf_temperature.rs"
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
cp "$BASE/$FORT" "$WORK/backup/lt.F90"
cp "$BASE/$RUST" "$WORK/backup/lt.rs"
restore() {
  cd "$BASE"
  diff -q "$WORK/backup/lt.F90" "$BASE/$FORT" >/dev/null 2>&1 || { cp "$WORK/backup/lt.F90" "$BASE/$FORT"; echo "== restored $FORT"; }
  diff -q "$WORK/backup/lt.rs" "$BASE/$RUST" >/dev/null 2>&1 || { cp "$WORK/backup/lt.rs" "$BASE/$RUST"; echo "== restored $RUST"; }
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

def after(lines, exact, payload, tag):
    idx = [i for i, l in enumerate(lines) if l == exact]
    assert len(idx) == 1, f"{tag}: {len(idx)} matches"
    lines[idx[0] + 1:idx[0] + 1] = payload

path = sys.argv[1]
lines = load(path)
after(lines, "   SAVE", ["   integer, save :: lt_probe_n = 0"], "f save")
after(lines, "                (assimsun + assimsha  - respcsun -respcsha - rsoil)", [
    "         lt_probe_n = lt_probe_n + 1",
    "         WRITE(*,'(A,1X,I5,10(1X,ES23.15))') 'GPCO ', lt_probe_n, &",
    "            gah2o, raw, thm, tprcor, assimsun, assimsha, respcsun, respcsha, rsoil, pco2a",
], "f GPCO")
save(path, lines)
print("   patched MOD_LeafTemperature_Extended.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
grep -q 'GPCO ' "$WORK/strings.txt" || { echo "!! marker GPCO missing"; exit 3; }

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
grep -a '^GPCO ' "$WORK/kernel.log" > "$WORK/fort_probe.txt" || true
echo "   kernel GPCO lines: $(wc -l < "$WORK/fort_probe.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys
path = sys.argv[1]
src = open(path).read()
anchor = "const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;"
assert src.count(anchor) == 1
src = src.replace(anchor, anchor +
    "\n\n/// 探针用：与内核侧 `lt_probe_n` 同步的叶温迭代序号。\n"
    "static LT_PROBE_COUNT: std::sync::atomic::AtomicUsize =\n"
    "    std::sync::atomic::AtomicUsize::new(0);", 1)
target = "                    - 0.22e-6);\n"
assert src.count(target) == 1
src = src.replace(target, target +
    '        let lt_probe = LT_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;\n'
    '        if std::env::var_os("COLM_PCO2A_PROBE").is_some() {\n'
    '            eprintln!("GRCO {:5} gah2o={:e} raw={:e} thm={:e} tprcor={:e} assim_sun={:e} assim_sha={:e} resp_sun={:e} resp_sha={:e} rsoil={:e} pco2a={:e}",\n'
    '                lt_probe, air_conductance, raw, input.reference_air_temperature_k, pressure_conversion,\n'
    '                sunlit_resistance.assimilation_mol_m2_s, shaded_resistance.assimilation_mol_m2_s,\n'
    '                sunlit_resistance.respiration_mol_m2_s, shaded_resistance.respiration_mol_m2_s,\n'
    '                0.22e-6, canopy_air_co2);\n'
    '        }\n', 1)
open(path, "w").write(src)
print("   patched leaf_temperature.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -25 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_PCO2A_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -a '^GRCO ' "$WORK/rust.log" > "$WORK/rust_probe.txt" || true
echo "   rust GRCO lines: $(wc -l < "$WORK/rust_probe.txt")"

python3 - "$WORK" "$BASE" <<'CEOF'
import re, sys
work, base = sys.argv[1], sys.argv[2]
names = ["gah2o", "raw", "thm", "tprcor", "assim_sun", "assim_sha", "resp_sun", "resp_sha", "rsoil", "pco2a"]

def kern(p):
    out = {}
    for line in open(p, errors="replace"):
        q = line.split()
        if not q or q[0] != "GPCO":
            continue
        vals = []
        for x in q[2:]:
            try:
                vals.append(float(x))
            except ValueError:
                m = re.match(r"^([-\d.]+)([+-]\d+)$", x)
                vals.append(float(m.group(1) + "E" + m.group(2)))
        out[int(q[1])] = vals
    return out

def rust(p):
    out = {}
    for line in open(p, errors="replace"):
        q = line.split()
        if not q or q[0] != "GRCO":
            continue
        out[int(q[1])] = {k: float(v) for k, v in re.findall(r"(\w+)=\s*([-\w.eE+]+)", line)}
    return out

def close(a, b):
    return a == b or abs(a - b) / max(abs(a), abs(b), 1e-300) <= 2e-14

fk, fr = kern(work + "/fort_probe.txt"), rust(work + "/rust_probe.txt")
keys = sorted(set(fk) & set(fr))
print(f"GPCO/GRCO: kernel {len(fk)} / rust {len(fr)} / shared {len(keys)}")
first = None
ndiff = 0
for k in keys:
    diffs = []
    for i, n in enumerate(names):
        if not close(fk[k][i], fr[k][names[i]]):
            diffs.append((n, fk[k][i], fr[k][names[i]]))
    if diffs:
        ndiff += 1
        if first is None:
            first = (k, diffs)
if first is None:
    print("  identical")
else:
    k, diffs = first
    print(f"  first divergence iter {k}; {ndiff}/{len(keys)} iters differ")
    for n, a, b in diffs:
        print(f"     {n:10s} K={a!r:24s} R={b!r:24s} rel={abs(a-b)/max(abs(a),abs(b),1e-300):.3e}")
sys.exit(1 if first else 0)
CEOF
