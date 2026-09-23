#!/usr/bin/env bash
# 第 299 轮（第二枪）：`soil_water_vertical_movement` **入场** + `Richards_solver` **内部**
# 的两侧同位型探针。
#
# 为什么还要打这一层：`vsf_probe.sh` 已经钉住"第 13 次调用出场时 `ss_wt[3]` 差 1 ULP、
# `ss_vliq[3]` 逐位相同"，但出场差**可能不是本步产生的** —— 也可能入场前
# `etr`/`rootflux`/`rsubst` 就差了（它们是本步上游算出来的，不是重启状态）。
# 所以先打 `WSF1`/`WSFE`（入场）把这条排除掉，再按 Richards 内部的次序往下走：
#   `RCH0` 入场 → `RCHL` 逐层入场 → 每个 Newton 迭代 `RCHF`(f2) `RCHB`(blc)
#   `RCHD`(dv/vact) `RCHE`(逐层更新后) → `RCHX`(显式回退) → `RCHZ`(收尾)。
# 第一条不同的记录就是分歧的出生点，判读表见 `oracle/scripts/vsf_richards_cmp.py`。
#
# 用法: bash oracle/scripts/vsf_richards_probe.sh [STEPS]   （默认 13 步，够到第 13 次调用）
set -euo pipefail
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STEPS=${1:-13}
WORK=${WORK:-/tmp/gf/vsfr$STEPS}
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


def insert_after(anchor, block):
    """anchor 必须是**唯一**的整行（含缩进），在它后面插入 block 的各行。

    整行比较而不是子串比较：`IF (vact(ub+1)) THEN` 这类锚在**更深的缩进**下
    还会出现一次（`:959`），子串计数会数到 2（第 299 轮实测踩到）。
    """
    hits = [i for i, line in enumerate(lines) if line == anchor]
    assert len(hits) == 1, (anchor, len(hits))
    lines[hits[0] + 1 : hits[0] + 1] = block.split("\n")


# --- 计数器声明（save：活到下一次调用）---
insert_after("   logical  :: is_sat", "   integer, save :: wsf2_n = 0")
insert_after(
    "   real(r8) :: wsum_m1, wsum, werr", "   integer, save :: rch_n = 0"
)

# --- 入场：本步上游给的驱动量 + 逐层入参 ---
insert_after(
    "      tol_p = 1.0e-14",
    """      wsf2_n = wsf2_n + 1
      WRITE(*,'(A,1X,I4,1X,I2,1X,I2,8(1X,ES26.17))') 'WSF1', wsf2_n, 0, 0, &
         real(nlev,r8), qgtop, etr, rsubst, ss_dp, zwt, wa, tolerance
      DO ilev = 1, nlev
         WRITE(*,'(A,1X,I4,1X,I3,1X,I2,5(1X,ES26.17))') 'WSFE', wsf2_n, ilev, 0, &
            ss_vliq(ilev), rootflux(ilev), porsl(ilev), psi_s(ilev), hksat(ilev)
      ENDDO""",
)

# --- Richards 入场 ---
insert_after(
    "      dt_explicit = dt / max_iters_richards",
    """      rch_n = rch_n + 1
      WRITE(*,'(A,1X,I4,1X,I2,1X,I2,8(1X,ES26.17))') 'RCH0', rch_n, 0, 0, &
         real(ub-lb+1,r8), real(ubc_typ,r8), real(lbc_typ,r8), &
         dt, ubc_val, lbc_val, ss_dp, waquifer
      DO ilev = lb, ub
         WRITE(*,'(A,1X,I4,1X,I3,1X,I2,3(1X,ES26.17))') 'RCHL', rch_n, ilev-lb+1, 0, &
            ss_vl(ilev), ss_wt(ilev), ss_wf(ilev)
      ENDDO""",
)

# --- 每个 Newton 迭代：残差范数 + 逐行 blc ---
insert_after(
    "            f2_norm(iter) = sqrt(sum(blc**2))",
    """            WRITE(*,'(A,1X,I4,1X,I3,1X,I2,4(1X,ES26.17))') 'RCHF', rch_n, iter, 0, &
               dt_this, f2_norm(iter), ss_dp, waquifer
            DO ilev = lb-1, ub+1
               WRITE(*,'(A,1X,I4,1X,I3,1X,I3,1(1X,ES26.17))') 'RCHB', rch_n, iter, ilev-lb+1, blc(ilev)
            ENDDO""",
)

# --- 最小二乘解出的 Newton 步长 ---
insert_after(
    "            CALL solve_least_squares_problem (ub-lb+3, dr_dv, vact, blc, dv)",
    """            DO ilev = lb-1, ub+1
               WRITE(*,'(A,1X,I4,1X,I3,1X,I3,3(1X,ES26.17))') 'RCHD', rch_n, iter, ilev-lb+1, &
                  dv(ilev), dr_dv(ilev,ilev), merge(1.0_r8,0.0_r8,vact(ilev))
            ENDDO""",
)

# --- 逐层更新之后（`IF (vact(ub+1))` 之前；本算例 lbc=FIX_FLUX，那块不执行）---
insert_after(
    "            IF (vact(ub+1)) THEN",
    """            DO ilev = lb, ub
               WRITE(*,'(A,1X,I4,1X,I3,1X,I3,6(1X,ES26.17))') 'RCHE', rch_n, iter, ilev-lb+1, &
                  ss_vl(ilev), ss_wt(ilev), ss_wf(ilev), psi(ilev), hk(ilev), &
                  merge(1.0_r8,0.0_r8,is_sat(ilev))
            ENDDO""",
)

# --- 显式回退之后 ---
insert_after(
    "                     tol_q, tol_z, tol_v)",
    """                  DO ilev = lb, ub
                     WRITE(*,'(A,1X,I4,1X,I3,1X,I2,4(1X,ES26.17))') 'RCHX', rch_n, ilev-lb+1, 0, &
                        dt_this, ss_vl(ilev), ss_wt(ilev), ss_wf(ilev)
                  ENDDO""",
)

# --- 收尾（`ss_vl` 折成整层平均之后）---
insert_after(
    "      ss_q = ss_q / dt",
    """      DO ilev = lb, ub
         WRITE(*,'(A,1X,I4,1X,I3,1X,I2,4(1X,ES26.17))') 'RCHZ', rch_n, ilev-lb+1, 0, &
            ss_vl(ilev), ss_wt(ilev), ss_wf(ilev), ss_q(ilev-1)
      ENDDO""",
)

open(path, "w").write("\n".join(lines))
print("   patched MOD_Hydro_SoilWater.F90")
FEOF

(cd "$BASE" && ./oracle/scripts/build_kernel.sh default >"$WORK/build.log" 2>&1) \
  || { echo "!! compile failed"; tail -25 "$WORK/build.log"; exit 3; }
strings "$BASE/kernels/default/colm.x" > "$WORK/strings.txt"
for tag in WSF1 WSFE RCH0 RCHL RCHF RCHB RCHD RCHE RCHX RCHZ; do
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
grep -aE '^(WSF1|WSFE|RCH0|RCHL|RCHF|RCHB|RCHD|RCHE|RCHX|RCHZ) ' "$WORK/kernel.log" > "$WORK/fort.txt" || true
echo "   kernel probe lines: $(wc -l < "$WORK/fort.txt")"

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()


def insert_after(anchor, block):
    assert src.count(anchor) == 1, (anchor, src.count(anchor))
    return src.replace(anchor, anchor + block, 1)


# 计数器声明在本函数作用域里（后面的各块都用同一个 static，不再各自加一）。
src = insert_after(
    "    let explicit_time_step_seconds = input.time_step_seconds / MAX_ITERS_RICHARDS as f64;\n",
    """    static RICHARDS_PROBE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
""",
)

# --- Richards 入场 ---
src = insert_after(
    """    static RICHARDS_PROBE_COUNT: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
""",
    """    if std::env::var_os("COLM_VSF_RICHARDS_PROBE").is_some() {
        let count = RICHARDS_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let boundary_code = |kind: VariableSaturatedBoundaryKind| match kind {
            VariableSaturatedBoundaryKind::FixedHead => 1.0,
            VariableSaturatedBoundaryKind::Rainfall => 2.0,
            VariableSaturatedBoundaryKind::FixedFlux => 3.0,
            VariableSaturatedBoundaryKind::Drainage => 4.0,
        };
        eprintln!(
            "RCH0 {:4} {:2} {:2} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e}",
            count, 0, 0, layers as f64, boundary_code(input.upper_boundary.kind),
            boundary_code(input.lower_boundary.kind), input.time_step_seconds,
            input.upper_boundary.value, input.lower_boundary.value, state.ponding_depth_mm,
            state.aquifer_water_mm
        );
        for level in 0..layers {
            eprintln!(
                "RCHL {:4} {:3} {:2} {:e} {:e} {:e}",
                1, level + 1, 0, state.liquid_water[level],
                state.water_table_thickness_mm[level], 0.0_f64
            );
        }
    }
""",
)

# --- 每个 Newton 迭代：残差范数 + 逐行 blc ---
src = insert_after(
    """            let residual_norm_mm = balance
                .residual_mm
                .iter()
                .map(|residual| residual * residual)
                .sum::<f64>()
                .sqrt();
""",
    """            if std::env::var_os("COLM_VSF_RICHARDS_PROBE").is_some() {
                let count = RICHARDS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
                eprintln!(
                    "RCHF {:4} {:3} {:2} {:e} {:e} {:e} {:e}",
                    count, iteration, 0, time_this_seconds, residual_norm_mm,
                    ponding_depth_mm, aquifer_water_mm
                );
                for (row, residual) in balance.residual_mm.iter().enumerate() {
                    eprintln!(
                        "RCHB {:4} {:3} {:3} {:e}",
                        count,
                        iteration,
                        row + 1,
                        residual
                    );
                }
            }
""",
)

# --- 最小二乘解出的 Newton 步长 ---
src = insert_after(
    """            let search =
                solve_variable_saturated_least_squares(&jacobian, &active, &balance.residual_mm)?;
""",
    """            if std::env::var_os("COLM_VSF_RICHARDS_PROBE").is_some() {
                let count = RICHARDS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
                for row in 0..dimension {
                    eprintln!(
                        "RCHD {:4} {:3} {:3} {:e} {:e} {:e}",
                        count, iteration, row + 1, search[row],
                        jacobian[row * dimension + row], active[row] as i32 as f64
                    );
                }
            }
""",
)

# --- 逐层更新之后（`active[layers + 1]` 那一块之前）---
src = insert_after(
    """                    active_variable[level] == 2,
                    input.volume_tolerance,
                );
            }
""",
    """            if std::env::var_os("COLM_VSF_RICHARDS_PROBE").is_some() {
                let count = RICHARDS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
                for level in 0..layers {
                    eprintln!(
                        "RCHE {:4} {:3} {:3} {:e} {:e} {:e} {:e} {:e} {:e}",
                        count, iteration, level + 1, zone.liquid_water[level],
                        zone.water_table_thickness_mm[level], zone.wetting_front_mm[level],
                        pressure_head_mm[level], hydraulic_conductivity_mm_s[level],
                        zone.saturated[level] as i32 as f64
                    );
                }
            }
""",
)

# --- 显式回退之后 ---
src = insert_after(
    "                    water_table_depth_mm = explicit.water_table_depth_mm;\n",
    """                    if std::env::var_os("COLM_VSF_RICHARDS_PROBE").is_some() {
                        let count =
                            RICHARDS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
                        for level in 0..layers {
                            eprintln!(
                                "RCHX {:4} {:3} {:2} {:e} {:e} {:e} {:e}",
                                count, level + 1, 0, time_this_seconds,
                                zone.liquid_water[level], zone.water_table_thickness_mm[level],
                                zone.wetting_front_mm[level]
                            );
                        }
                    }
""",
)

open(path, "w").write(src)
print("   patched variably_saturated_flow.rs (richards)")
REOF

python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()
anchor = """                / (thickness - water_table);
        }
    }
"""
assert src.count(anchor) == 1, src.count(anchor)
block = """    if std::env::var_os("COLM_VSF_RICHARDS_PROBE").is_some() {
        let count = RICHARDS_PROBE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
        for level in 0..layers {
            eprintln!(
                "RCHZ {:4} {:3} {:2} {:e} {:e} {:e} {:e}",
                count, level + 1, 0, state.liquid_water[level],
                state.water_table_thickness_mm[level], zone.wetting_front_mm[level],
                state.interface_flux_mm_s[level]
            );
        }
    }
"""
open(path, "w").write(src.replace(anchor, anchor + block, 1))
print("   patched variably_saturated_flow.rs (exit)")
REOF

# `soil_water_vertical_movement` 的入场块（在另一个函数里，单独插）。
python3 - "$BASE/$RUST" <<'REOF'
import sys

path = sys.argv[1]
src = open(path).read()
anchor = "    let pressure_tolerance_mm = 1.0e-14;\n"
assert src.count(anchor) == 1, src.count(anchor)
block = """    if std::env::var_os("COLM_VSF_RICHARDS_PROBE").is_some() {
        static VSF2_PROBE_COUNT: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let count = VSF2_PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        eprintln!(
            "WSF1 {:4} {:2} {:2} {:e} {:e} {:e} {:e} {:e} {:e} {:e} {:e}",
            count, 0, 0, nlev as f64, input.ground_water_flux_mm_s,
            input.transpiration_mm_s, input.subsurface_runoff_mm_s, state.ponding_depth_mm,
            state.water_table_depth_mm, state.aquifer_water_mm, input.tolerance_mm
        );
        for level in 0..nlev {
            eprintln!(
                "WSFE {:4} {:3} {:2} {:e} {:e} {:e} {:e} {:e}",
                count, level + 1, 0, state.liquid_water[level], input.root_flux_mm_s[level],
                input.porosity[level], input.saturated_potential_mm[level],
                input.saturated_hydraulic_conductivity_mm_s[level]
            );
        }
    }
"""
open(path, "w").write(src.replace(anchor, anchor + block, 1))
print("   patched variably_saturated_flow.rs (entry)")
REOF

(cd "$BASE" && cargo build -q -p colm-runtime --bin colm-rs >"$WORK/rust_build.log" 2>&1) \
  || { echo "!! rust build failed"; tail -30 "$WORK/rust_build.log"; exit 3; }
( cd "$BASE" && COLM_VSF_RICHARDS_PROBE=1 cargo run -q -p colm-runtime --bin colm-rs -- \
    "$WORK" --land-cover igbp --restart-out "$WORK/rust_restart.nc" --history-dir "$WORK" \
    > "$WORK/rust.log" 2>&1 ) || { echo "!! rust run failed"; tail -5 "$WORK/rust.log"; exit 4; }
grep -aE '^(WSF1|WSFE|RCH0|RCHL|RCHF|RCHB|RCHD|RCHE|RCHX|RCHZ) ' "$WORK/rust.log" > "$WORK/rust.txt" || true
echo "   rust probe lines: $(wc -l < "$WORK/rust.txt")"

echo "== compare =="
python3 "$BASE/oracle/scripts/vsf_richards_cmp.py" "$WORK/fort.txt" "$WORK/rust.txt" || true
