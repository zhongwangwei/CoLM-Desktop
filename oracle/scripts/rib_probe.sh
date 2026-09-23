#!/usr/bin/env bash
# 第 296 轮：`f_rib` 那条 history 诊断的中间量两侧同位型探针。
#
# `MOD_Vars_1DAccFluxes.F90:2786` 的 `r_rib_e = r_zol_e/vonkar*r_ustar2_e**2/
# (vonkar/r_fh_e*um**2)`，`MOD_Hist.F90:4530` 把它写进 `f_rib`。
# `accumulate_fluxes` 由 `MOD_Hist.F90:227` **每个 history 记录调一次**，
# 与 Rust 的 `history_diagnostics`（`history.rs:set_lct_surface_diagnostics`）
# 一一对应 —— 所以两侧计数器可以直接对齐。
#
# 内核 `RFRIB n r_zol_e um r_ustar2_e r_fh_e r_rib_e`
# Rust  `RRRIB n zol um r_ustar2 r_fh rib`
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK=${WORK:-/tmp/gf/rib10}
HOURS=${1:-10}
END_SEC=$((HOURS * 3600))
FORT="vendor/CoLM202X/main/MOD_Vars_1DAccFluxes.F90"
RUST="crates/colm-core/src/history_diagnostics.rs"
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
cp "$BASE/$FORT" "$WORK/backup/acc.F90"
cp "$BASE/$RUST" "$WORK/backup/hd.rs"
restore() {
  cd "$BASE"
  diff -q "$WORK/backup/acc.F90" "$BASE/$FORT" >/dev/null 2>&1 || { cp "$WORK/backup/acc.F90" "$BASE/$FORT"; echo "== restored $FORT"; }
  diff -q "$WORK/backup/hd.rs" "$BASE/$RUST" >/dev/null 2>&1 || { cp "$WORK/backup/hd.rs" "$BASE/$RUST"; echo "== restored $RUST"; }
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
start = lines.index("   SUBROUTINE accumulate_fluxes")
decl = start + lines[start:].index("   IMPLICIT NONE")
lines[decl + 1:decl + 1] = ["   integer, save :: rib_probe_n = 0"]
anchor = "               r_rib_e = min(5.,r_rib_e)"
assert lines.count(anchor) == 1, lines.count(anchor)
idx = lines.index(anchor)
lines[idx + 1:idx + 1] = [
    "               rib_probe_n = rib_probe_n + 1",
    "               WRITE(*,'(A,1X,I5,13(1X,ES23.15))') 'RFRIB', rib_probe_n, &",
    "                  r_zol_e, um, r_ustar2_e, r_fh_e, r_rib_e, thvstar, r_ustar_e, thv, th, qm, &",
    "                  zldis, displa_av, z0m_av",
]
open(path, "w").write("\n".join(lines))
print("   patched MOD_Vars_1DAccFluxes.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
grep -q 'RFRIB' "$WORK/strings.txt" || { echo "!! marker missing"; exit 3; }

sed -e "s#^   DEF_dir_output.*#   DEF_dir_output  = '$WORK/out/'#" \
    -e "s#^   DEF_dir_rawdata.*#   DEF_dir_rawdata = '$WORK/rawdata_unused/'#" \
    -e "s#^   DEF_dir_runtime.*#   DEF_dir_runtime = '$WORK/runtime_unused/'#" \
    -e "s#^   DEF_simulation_time%end_day.*#   DEF_simulation_time%end_day       = 1#" \
    -e "s#^   DEF_simulation_time%end_sec.*#   DEF_simulation_time%end_sec       = $END_SEC#" \
    "$BASE/oracle/work/CN-Cng/case.nml" > "$WORK/case.nml"
cp -R "$BASE/oracle/work/CN-Cng/out" "$WORK/out"
rm -rf "$WORK/out/CN-Cng/history"
( cd "$WORK/run" && "$BASE/kernels/default/colm.x" "$WORK/case.nml" > "$WORK/kernel.log" 2>&1 ) \
  || { echo "!! kernel run failed"; exit 4; }
grep -q 'CoLM Execution Completed' "$WORK/kernel.log" || { echo "!! incomplete"; tail -5 "$WORK/kernel.log"; exit 4; }
grep -a '^RFRIB' "$WORK/kernel.log" > "$WORK/fort.txt" || true
echo "   kernel RFRIB lines: $(wc -l < "$WORK/fort.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()
anchor = """    let bulk_richardson = (zol / VON_KARMAN * similarity.friction_velocity_m_s.powi(2)
        / (VON_KARMAN / similarity.heat * stability_adjusted_wind.powi(2)))
    .min(5.0);
"""
assert src.count(anchor) == 1, src.count(anchor)
ins = anchor + (
    '    if std::env::var_os("COLM_RIB_PROBE").is_some() {\n'
    '        static RIB_PROBE_COUNT: std::sync::atomic::AtomicUsize =\n'
    '            std::sync::atomic::AtomicUsize::new(0);\n'
    '        let count = RIB_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;\n'
    '        eprintln!(\n'
    '            "RRRIB {:5} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e}",\n'
    '            count, zol, stability_adjusted_wind,\n'
    '            similarity.friction_velocity_m_s, similarity.heat, bulk_richardson,\n'
    '            virtual_scale, friction_velocity, virtual_potential_temperature,\n'
    '            potential_temperature, humidity, height_scale, displacement, momentum_roughness\n'
    '        );\n'
    '    }\n')
open(path, "w").write(src.replace(anchor, ins, 1))
print("   patched history_diagnostics.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -25 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_RIB_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -a '^RRRIB' "$WORK/rust.log" > "$WORK/rust.txt" || true
echo "   rust RRRIB lines: $(wc -l < "$WORK/rust.txt")"

echo "== compare =="
python3 - "$WORK" <<'CEOF'
import sys

work = sys.argv[1]
names = ["zol", "um", "ustar2", "fh", "rib", "thvstar", "ustar", "thv", "th", "qm", "zldis", "displa", "z0m"]


def rows(path, tag):
    out = {}
    for line in open(path, errors="replace"):
        q = line.split()
        if q and q[0] == tag:
            out[int(q[1])] = [float(x) for x in q[2:]]
    return out


f = rows(f"{work}/fort.txt", "RFRIB")
r = rows(f"{work}/rust.txt", "RRRIB")
keys = sorted(set(f) & set(r))
print(f"kernel {len(f)} / rust {len(r)} / shared {len(keys)}")
ndiff = 0
for k in keys:
    diffs = [
        (names[i], f[k][i], r[k][i])
        for i in range(min(len(f[k]), len(r[k])))
        if f[k][i] != r[k][i]
    ]
    if diffs:
        ndiff += 1
        if ndiff <= 8:
            for n, a, b in diffs:
                print(f"  rec {k} {n:6s} K={a!r:24s} R={b!r:24s}")
print(f"  {ndiff}/{len(keys)} records differ")
CEOF
