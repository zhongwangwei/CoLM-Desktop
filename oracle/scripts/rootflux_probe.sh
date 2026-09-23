#!/usr/bin/env bash
# 第 299 轮（第三枪）：叶温例程里 `rootflux` 的"缩放前 → 缩放后"两侧探针。
#
# 上游 `MOD_LeafTemperature_Extended.F90`（`SUBROUTINE LeafTemperature` 内）：
#
#     :1351  etr0 = etr
#     :1352  etr  = etr + etr_dtl*dtl(it-1)
#     :1354  IF (DEF_USE_PLANTHYDRAULICS)
#     :1357     IF (abs(etr0) >= 1.e-15) rootflux = rootflux * etr / etr0
#     :1360     ELSE                     rootflux = rootflux + dz/sum(dz)*etr_dtl*dtl(it-1)
#     :1367  CALL balance_phs_rootflux(..., 'post-leaf-temperature')
#
# 本仓库对应 `leaf_temperature.rs:1110`（`transpiration = fma(slope, dtl, last.transpiration)`）
# 与 `:1128-1163`（同一个 IF/ELSE + `balance_phs_rootflux`）。
#
# 打两个点、四个 tag：
#   `RFIN`/`RFINL`  —— `:1351` **之前**（即上一轮迭代缩放后的 `rootflux`，也就是
#                      进这一段时的值）+ `etr0`（= `etr` 更新前）
#   `RFLT`/`RFLTL`  —— 缩放之后、`balance_phs_rootflux` **之前** + `etr`/`etr0`/`etr_dtl`/`dtl`
#
# 判读：第 12 步若 `RFINL` 已不同 ⇒ 种子在 PHS 解算器（回到 `qe2x`）；
#       若 `RFINL` 相同而 `RFLTL` 不同 ⇒ 差就在这一缩放/上游的 `etr`/`etr_dtl`/`dtl` 链。
#
# 用法: bash oracle/scripts/rootflux_probe.sh [STEPS]   （默认 13 步）
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STEPS=${1:-13}
WORK=${WORK:-/tmp/gf/rf$STEPS}
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
cp "$BASE/$FORT" "$WORK/backup/leaft.F90"
cp "$BASE/$RUST" "$WORK/backup/lt.rs"
restore() {
  cd "$BASE"
  diff -q "$WORK/backup/leaft.F90" "$BASE/$FORT" >/dev/null 2>&1 || { cp "$WORK/backup/leaft.F90" "$BASE/$FORT"; echo "== restored $FORT"; }
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

path = sys.argv[1]
lines = open(path).read().split("\n")


def insert_before(anchor, block):
    hits = [i for i, line in enumerate(lines) if line == anchor]
    assert len(hits) == 1, (anchor, len(hits))
    lines[hits[0] : hits[0]] = block.split("\n")


def insert_after(anchor, block):
    hits = [i for i, line in enumerate(lines) if line == anchor]
    assert len(hits) == 1, (anchor, len(hits))
    lines[hits[0] + 1 : hits[0] + 1] = block.split("\n")


# 计数器要放声明区：从锚点往前找最近的 `IMPLICIT NONE`（`LeafTemperature` 那个）。
# 这个文件里**没有**现成的整型循环变量，所以自己声明一个 `rflt_j`。
anchor_index = [i for i, line in enumerate(lines) if line == "      etr0  = etr"][0]
decls = [i for i in range(anchor_index) if lines[i] == "   IMPLICIT NONE"]
assert decls, "no IMPLICIT NONE before the anchor"
lines[decls[-1] + 1 : decls[-1] + 1] = [
    "      integer, save :: rflt_n = 0",
    "      integer :: rflt_j",
]

insert_before(
    "      etr0  = etr",
    """      rflt_n = rflt_n + 1
      WRITE(*,'(A,1X,I5,1X,I2,1X,I2,4(1X,ES26.17))') 'RFIN', rflt_n, 0, 0, &
         etr, etr_dtl, dtl(it-1), 0._r8
      DO rflt_j = 1, nl_soil
         WRITE(*,'(A,1X,I5,1X,I3,1X,I2,1(1X,ES26.17))') 'RFINL', rflt_n, rflt_j, 0, rootflux(rflt_j)
      ENDDO""",
)

# 落在 `IF (abs(etr0) >= 1.e-15) ... ELSE ... ENDIF` 那个 `ENDIF` **之后**，
# 而不是 ELSE 分支里面（否则干窗走 IF 分支时一行都不打 —— 第一版就踩了这个）。
idx = lines.index("                rootflux = rootflux + dz_soi / sum(dz_soi) * etr_dtl* dtl(it-1)")
assert lines[idx + 1] == "            ENDIF", lines[idx + 1]
lines[idx + 2 : idx + 2] = """            DO rflt_j = 1, nl_soil
               WRITE(*,'(A,1X,I5,1X,I3,1X,I2,1(1X,ES26.17))') 'RFLTL', rflt_n, rflt_j, 0, rootflux(rflt_j)
            ENDDO
            WRITE(*,'(A,1X,I5,1X,I2,1X,I2,4(1X,ES26.17))') 'RFLT', rflt_n, 0, 0, &
               etr, etr0, etr_dtl, dtl(it-1)""".split("\n")

open(path, "w").write("\n".join(lines))
print("   patched MOD_LeafTemperature_Extended.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
for tag in RFIN RFINL RFLT RFLTL; do
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
grep -aE '^(RFIN|RFINL|RFLT|RFLTL) ' "$WORK/kernel.log" > "$WORK/fort.txt" || true
echo "   kernel probe lines: $(wc -l < "$WORK/fort.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()

before = """    let mut transpiration = last
        .transpiration_temperature_slope
        .mul_add(final_temperature_change, last.transpiration);
"""
assert src.count(before) == 1, src.count(before)
head = """    static ROOTFLUX_PROBE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    if std::env::var_os("COLM_ROOTFLUX_PROBE").is_some() {
        let count = ROOTFLUX_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        eprintln!(
            "RFIN {:5} {:2} {:2} {:e} {:e} {:e} {:e}",
            count, 0, 0, last.transpiration, last.transpiration_temperature_slope,
            final_temperature_change, 0.0_f64
        );
        for (level, flux) in last.root_flux_kg_m2_s.iter().enumerate() {
            eprintln!("RFINL {:5} {:3} {:2} {:e}", count, level + 1, 0, flux);
        }
    }
"""
src = src.replace(before, head + before, 1)

after = """        // `MOD_LeafTemperature_Extended.F90:1367` 的 `'post-leaf-temperature'`：
"""
assert src.count(after) == 1, src.count(after)
tail = """        if std::env::var_os("COLM_ROOTFLUX_PROBE").is_some() {
            let count = ROOTFLUX_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
            for (level, flux) in root_flux_kg_m2_s.iter().enumerate() {
                eprintln!("RFLTL {:5} {:3} {:2} {:e}", count, level + 1, 0, flux);
            }
            eprintln!(
                "RFLT {:5} {:2} {:2} {:e} {:e} {:e} {:e}",
                count, 0, 0, transpiration, last.transpiration,
                last.transpiration_temperature_slope, final_temperature_change
            );
        }
"""
src = src.replace(after, after + tail, 1)
open(path, "w").write(src)
print("   patched leaf_temperature.rs")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -30 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_ROOTFLUX_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -aE '^(RFIN|RFINL|RFLT|RFLTL) ' "$WORK/rust.log" > "$WORK/rust.txt" || true
echo "   rust probe lines: $(wc -l < "$WORK/rust.txt")"

echo "== compare =="
python3 "$BASE/oracle/scripts/probe_diff.py" "$WORK/fort.txt" "$WORK/rust.txt" || true
