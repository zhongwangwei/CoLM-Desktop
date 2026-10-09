//! 内部过程 `reconcile_internal_soil_flow` 与 `move_dissolved_face`。
//!
//! VSF 可以在连通的饱和层之间搬水而不记进 `qlayer`。对每个封闭的可渗透块，未报告的
//! 界面水量由逐层水量失配的累积和唯一确定；向下的界面自上而下、向上的界面自下而上处理，
//! 保证水与溶解的示踪物先到再传。重建不可行时什么也不动，交给显式残差账。
//!
//! GIMPLE：`closure_tol = max(1, Σ|wliq| + Σ|ws|) * 2^-46`（`64*epsilon` 折成常数）；
//! 三个求和都从 0 顺序累加；`cumulative = (cumulative + wliq) - ws`；
//! `move_dissolved_face` 里 `moved_tracer = min(moved/ws,1)*max(trc,0)` 不单独舍入，
//! 供体 `.FNMA`、受体 `.FMA`。

use super::super::{soisno_slot, TracerDescriptor, TracerPools, SOIL_LAYERS, SOISNO_LAYERS};
use super::common::soil_slot;
use colm_numeric::Contract;

/// `64._r8 * epsilon(1._r8)`。
const CLOSURE_TOL_SCALE: f64 = 64.0 * f64::EPSILON;

/// `reconcile_internal_soil_flow`。`water_shadow` 下标 `j-1`；`permeable` 为
/// `permeable_soil(1:nl_soil)`。
pub(super) fn reconcile_internal_soil_flow(
    tracer: &TracerDescriptor,
    p: &mut TracerPools,
    water_shadow: &mut [f64; SOIL_LAYERS],
    wliq_soisno: &[f64; SOISNO_LAYERS],
    permeable: &[bool; SOIL_LAYERS],
) {
    let nl = SOIL_LAYERS as i32;
    // `remap_face_water(1:nl_soil-1)` 与 `remap_trial_water(1:nl_soil)`，下标 `j-1`。
    let mut face = [0.0; SOIL_LAYERS];
    let mut trial = [0.0; SOIL_LAYERS];
    let wliq = |j: i32| wliq_soisno[soisno_slot(j)];

    let mut block_begin = 1;
    while block_begin <= nl {
        if !permeable[soil_slot(block_begin)] {
            block_begin += 1;
            continue;
        }
        let mut block_end = block_begin;
        while block_end < nl {
            if !permeable[soil_slot(block_end + 1)] {
                break;
            }
            block_end += 1;
        }
        if block_end == block_begin {
            block_begin = block_end + 1;
            continue;
        }
        let block = block_begin..=block_end;
        if !block.clone().all(|j| wliq(j).is_finite())
            || !block
                .clone()
                .all(|j| water_shadow[soil_slot(j)].is_finite())
        {
            block_begin = block_end + 1;
            continue;
        }
        if block.clone().any(|j| wliq(j) < 0.0)
            || block.clone().any(|j| water_shadow[soil_slot(j)] < 0.0)
        {
            block_begin = block_end + 1;
            continue;
        }
        let mut sum_liq = 0.0;
        for j in block.clone() {
            sum_liq = wliq(j).abs() + sum_liq;
        }
        let mut sum_shadow = 0.0;
        for j in block.clone() {
            sum_shadow = water_shadow[soil_slot(j)].abs() + sum_shadow;
        }
        let closure_tol = (sum_liq + sum_shadow).max(1.0) * CLOSURE_TOL_SCALE;
        let mut column_resid = 0.0;
        let mut max_mismatch = f64::NEG_INFINITY;
        for j in block.clone() {
            let mismatch = wliq(j) - water_shadow[soil_slot(j)];
            column_resid = mismatch + column_resid;
            if mismatch.abs() > max_mismatch {
                max_mismatch = mismatch.abs();
            }
        }
        if column_resid.abs() > closure_tol || max_mismatch <= closure_tol {
            block_begin = block_end + 1;
            continue;
        }

        let mut cumulative = 0.0;
        for iface in block_begin..block_end {
            cumulative = (cumulative + wliq(iface)) - water_shadow[soil_slot(iface)];
            face[soil_slot(iface)] = -cumulative;
        }

        // 先在副本上试走一遍；任何界面超量都放弃整块。
        for j in block.clone() {
            trial[soil_slot(j)] = water_shadow[soil_slot(j)];
        }
        let mut feasible = true;
        for iface in block_begin..block_end {
            let (k, kn) = (soil_slot(iface), soil_slot(iface + 1));
            if face[k] <= 0.0 {
                continue;
            }
            if face[k] > trial[k] {
                feasible = false;
                break;
            }
            trial[k] -= face[k];
            trial[kn] += face[k];
        }
        if feasible {
            for iface in (block_begin..block_end).rev() {
                let (k, kn) = (soil_slot(iface), soil_slot(iface + 1));
                if face[k] >= 0.0 {
                    continue;
                }
                let moved_water = -face[k];
                if moved_water > trial[kn] {
                    feasible = false;
                    break;
                }
                trial[kn] -= moved_water;
                trial[k] += moved_water;
            }
        }
        if feasible {
            for iface in block_begin..block_end {
                let moved_water = face[soil_slot(iface)];
                if moved_water <= 0.0 {
                    continue;
                }
                move_dissolved_face(tracer, p, water_shadow, iface, iface + 1, moved_water);
            }
            for iface in (block_begin..block_end).rev() {
                let face_water = face[soil_slot(iface)];
                if face_water >= 0.0 {
                    continue;
                }
                move_dissolved_face(tracer, p, water_shadow, iface + 1, iface, -face_water);
            }
        }
        block_begin = block_end + 1;
    }
}

/// `move_dissolved_face`：先让供体的溶解相与固相平衡，再按水量比例搬溶解的示踪物。
fn move_dissolved_face(
    tracer: &TracerDescriptor,
    p: &mut TracerPools,
    water_shadow: &mut [f64; SOIL_LAYERS],
    donor: i32,
    receiver: i32,
    moved_water: f64,
) {
    let (d, r) = (soisno_slot(donor), soisno_slot(receiver));
    let (kd, kr) = (soil_slot(donor), soil_slot(receiver));
    tracer.equilibrate_dissolved(
        water_shadow[kd],
        &mut p.wliq_soisno[d],
        &mut p.solid_soisno[d],
    );
    let fraction = (moved_water / water_shadow[kd]).min(1.0);
    let available = p.wliq_soisno[d].max(0.0);
    p.wliq_soisno[d] = (-fraction).contract(available, p.wliq_soisno[d]);
    p.wliq_soisno[r] = fraction.contract(available, p.wliq_soisno[r]);
    water_shadow[kd] -= moved_water;
    water_shadow[kr] += moved_water;
}
