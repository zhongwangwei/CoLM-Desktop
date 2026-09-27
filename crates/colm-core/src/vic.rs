//! Three-layer VIC runoff from `MOD_Hydro_VIC.F90`.
//!
//! CoLM's `DEF_Runoff_SCHEME = 1` groups its fixed ten soil layers into
//! VIC layers `1:3`, `4:6`, and `7:10`.  Keeping that conversion beside the
//! runoff solve gives the Rust driver one callable kernel instead of a second
//! implementation in the initializer.

use anyhow::{ensure, Result};

const COLM_LAYERS: usize = 10;
const VIC_LAYERS: usize = 3;
const GROUP_END: [usize; VIC_LAYERS] = [3, 6, 10];

/// Inputs to one `MOD_Hydro_VIC:Runoff_VIC` update.
///
/// All water masses are millimetres (equivalently kg m⁻²), rates are mm s⁻¹,
/// and soil lengths are metres.  CoLM's implementation is fixed at ten soil
/// layers and aggregates them to three VIC layers.
#[derive(Debug, Clone, Copy)]
pub struct VicRunoffInput<'a> {
    pub time_step_seconds: f64,
    pub layer_thickness_m: &'a [f64],
    pub porosity: &'a [f64],
    pub residual_water: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub ground_evaporation_mm_s: f64,
    pub root_flux_mm_s: &'a [f64],
    pub water_input_mm_s: f64,
    pub infiltration_shape: f64,
    pub maximum_baseflow_mm_day: f64,
    pub baseflow_fraction: f64,
    pub baseflow_threshold: f64,
    pub baseflow_exponent: f64,
}

/// Outputs from [`vic_runoff`].
#[derive(Debug, Clone, PartialEq)]
pub struct VicRunoffState {
    pub surface_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    pub saturated_fraction: f64,
    /// Exact `wliq_soisno_tmp` projection emitted by the upstream wrapper.
    ///
    /// The upstream wrapper never copies its local VIC water update back to
    /// `cell%layer`, so this is a projection of the input column rather than a
    /// next state.  It is kept for file-level compatibility only; callers must
    /// use the fluxes above to advance their native Rust state.
    pub colm_equivalent_moisture_kg_m2: Vec<f64>,
}

#[derive(Debug, Clone, Copy)]
struct VicSoil {
    max_moisture_mm: [f64; VIC_LAYERS],
    residual_moisture_mm: [f64; VIC_LAYERS],
    conductivity_mm_day: [f64; VIC_LAYERS],
    exponent: [f64; VIC_LAYERS],
    infiltration_shape: f64,
    maximum_baseflow_mm_day: f64,
    baseflow_fraction: f64,
    baseflow_threshold: f64,
    baseflow_exponent: f64,
}

#[derive(Debug, Clone, Copy)]
struct VicCell {
    moisture_mm: [f64; VIC_LAYERS],
    evaporation_mm: [f64; VIC_LAYERS],
    ice_mm: [[f64; VIC_LAYERS]; VIC_LAYERS],
}

/// Ports the active `Runoff_VIC → compute_vic_runoff` path.
///
/// The upstream routine chooses one runoff substep per model timestep because
/// both counts are derived from `DEF_simulation_time%timestep`; this function
/// preserves that same relationship without exposing a second timestep knob.
pub fn vic_runoff(input: VicRunoffInput<'_>) -> Result<VicRunoffState> {
    validate(input)?;
    let (soil, cell, frost_fraction, frost_count) = build_state(input);
    let state = compute_vic_runoff(
        soil,
        cell,
        input.water_input_mm_s * input.time_step_seconds,
        frost_fraction,
        frost_count,
        input.time_step_seconds,
    )?;
    let colm_equivalent_moisture_kg_m2 = project_to_colm(
        &grouped_sum(input.liquid_water_kg_m2),
        input.layer_thickness_m,
    );
    Ok(VicRunoffState {
        surface_runoff_mm_s: if input.water_input_mm_s > 0.0 {
            state.surface_runoff_mm / input.time_step_seconds
        } else {
            0.0
        },
        subsurface_runoff_mm_s: state.subsurface_runoff_mm / input.time_step_seconds,
        saturated_fraction: state.saturated_fraction,
        colm_equivalent_moisture_kg_m2,
    })
}

#[derive(Debug, Clone, Copy)]
struct VicStepState {
    surface_runoff_mm: f64,
    subsurface_runoff_mm: f64,
    saturated_fraction: f64,
}

fn build_state(input: VicRunoffInput<'_>) -> (VicSoil, VicCell, [f64; VIC_LAYERS], usize) {
    let depth_m = grouped_sum(input.layer_thickness_m);
    let porosity = grouped_weighted(input.porosity, input.layer_thickness_m);
    let residual_water = grouped_weighted(input.residual_water, input.layer_thickness_m);
    let soil = VicSoil {
        max_moisture_mm: std::array::from_fn(|layer| porosity[layer] * depth_m[layer] * 1000.0),
        residual_moisture_mm: std::array::from_fn(|layer| {
            residual_water[layer] * depth_m[layer] * 1000.0
        }),
        conductivity_mm_day: grouped_weighted(
            input.saturated_hydraulic_conductivity_mm_s,
            input.layer_thickness_m,
        )
        .map(|value| value * 86_400.0),
        exponent: grouped_weighted(input.clapp_hornberger_b, input.layer_thickness_m)
            .map(|value| 2.0 * value + 3.0),
        infiltration_shape: input.infiltration_shape,
        maximum_baseflow_mm_day: input.maximum_baseflow_mm_day,
        baseflow_fraction: input.baseflow_fraction,
        baseflow_threshold: input.baseflow_threshold,
        baseflow_exponent: input.baseflow_exponent,
    };
    let moisture_mm = grouped_sum(input.liquid_water_kg_m2);
    let evaporation_mm =
        grouped_sum(input.root_flux_mm_s).map(|value| value * input.time_step_seconds);
    let mut evaporation_mm = evaporation_mm;
    evaporation_mm[0] += input.ground_evaporation_mm_s * input.time_step_seconds;
    let has_ice = input.ice_water_kg_m2.iter().any(|&value| value > 0.0);
    let frost_fraction = if has_ice {
        [0.25, 0.5, 0.25]
    } else {
        [1.0, 0.0, 0.0]
    };
    let frost_count = if has_ice { VIC_LAYERS } else { 1 };
    let mut ice_mm = [[0.0; VIC_LAYERS]; VIC_LAYERS];
    if has_ice {
        // `vic_para` 用同一个局部数组 `ice_tmp` 依次接三组的 `VIC_IceLay` 结果，
        // 第三组（4 个 CoLM 层）的累加起点就是第二组刚写下的值，见 [`partition_ice`]。
        let mut previous = [0.0; VIC_LAYERS];
        for (layer, range) in group_ranges().iter().enumerate() {
            ice_mm[layer] = partition_ice(&input.ice_water_kg_m2[range.clone()], previous);
            previous = ice_mm[layer];
        }
    }
    (
        soil,
        VicCell {
            moisture_mm,
            evaporation_mm,
            ice_mm,
        },
        frost_fraction,
        frost_count,
    )
}

fn compute_vic_runoff(
    soil: VicSoil,
    cell: VicCell,
    precipitation_mm: f64,
    frost_fraction: [f64; VIC_LAYERS],
    frost_count: usize,
    time_step_seconds: f64,
) -> Result<VicStepState> {
    let runoff_steps_per_day = (86_400.0 / time_step_seconds) as usize;
    ensure!(runoff_steps_per_day > 0, "VIC timestep exceeds one day");
    let conductivity_mm_step = soil
        .conductivity_mm_day
        .map(|value| value / runoff_steps_per_day as f64);
    let evaporation =
        distribute_evaporation(cell, soil.residual_moisture_mm, frost_fraction, frost_count);
    let mut moisture_mm = [0.0; VIC_LAYERS];
    let mut surface_runoff_mm = 0.0;
    let mut subsurface_runoff_mm = 0.0;
    let mut saturated_fraction = 0.0;

    for frost in 0..frost_count {
        let mut liquid = cell.moisture_mm;
        for (layer, value) in liquid.iter_mut().enumerate() {
            *value -= cell.ice_mm[layer][frost];
        }
        let ice = [
            cell.ice_mm[0][frost],
            cell.ice_mm[1][frost],
            cell.ice_mm[2][frost],
        ];
        let total = add(liquid, ice);
        // `runoff(fidx)`：本 frost 区间的地表径流，回灌溢出也先累加到它，最后才乘 `frost_fract`
        // （`MOD_Hydro_VIC.F90:401`）。
        let (_, mut runoff) = runoff_and_saturation(soil, total, precipitation_mm);
        let runoff_per_step = runoff;
        let mut inflow = precipitation_mm;
        let mut drainage = [0.0; VIC_LAYERS - 1];

        for layer in 0..VIC_LAYERS - 1 {
            let available =
                (liquid[layer] - evaporation[layer][frost]).max(soil.residual_moisture_mm[layer]);
            drainage[layer] = if available > soil.residual_moisture_mm[layer] {
                q12(
                    conductivity_mm_step[layer],
                    available,
                    soil.residual_moisture_mm[layer],
                    soil.max_moisture_mm[layer],
                    soil.exponent[layer],
                )
            } else {
                0.0
            };
        }

        for layer in 0..VIC_LAYERS - 1 {
            let runoff_here = if layer == 0 { runoff_per_step } else { 0.0 };
            // `liq = liq + (inflow - dt_runoff) - (Q12 + evap)`：照上游的括号结合。
            liquid[layer] = liquid[layer] + (inflow - runoff_here)
                - (drainage[layer] + evaporation[layer][frost]);
            let mut overflow =
                overflow_to_limit(&mut liquid[layer], ice[layer], soil.max_moisture_mm[layer]);
            if layer == 0 {
                drainage[layer] += overflow;
                overflow = 0.0;
            } else {
                while overflow > 0.0 {
                    let mut destination = layer;
                    loop {
                        if destination == 0 {
                            runoff += overflow;
                            overflow = 0.0;
                            break;
                        }
                        destination -= 1;
                        liquid[destination] += overflow;
                        overflow = overflow_to_limit(
                            &mut liquid[destination],
                            ice[destination],
                            soil.max_moisture_mm[destination],
                        );
                        if overflow <= 0.0 {
                            break;
                        }
                    }
                }
            }
            if liquid[layer] < 0.0 {
                drainage[layer] += liquid[layer];
                liquid[layer] = 0.0;
            }
            if liquid[layer] + ice[layer] < soil.residual_moisture_mm[layer] {
                // `Q12 = Q12 + (liq+ice) - resid`：左结合。
                drainage[layer] = drainage[layer] + (liquid[layer] + ice[layer])
                    - soil.residual_moisture_mm[layer];
                liquid[layer] = soil.residual_moisture_mm[layer] - ice[layer];
            }
            inflow = drainage[layer];
        }

        let bottom = VIC_LAYERS - 1;
        let relative_moisture = (liquid[bottom] - soil.residual_moisture_mm[bottom])
            / (soil.max_moisture_mm[bottom] - soil.residual_moisture_mm[bottom]);
        let mut baseflow_step = soil.maximum_baseflow_mm_day / runoff_steps_per_day as f64
            * soil.baseflow_fraction
            / soil.baseflow_threshold
            * relative_moisture;
        if relative_moisture > soil.baseflow_threshold {
            // GIMPLE（`MOD_Hydro_VIC.F90:330-331`）：`.FMA (Dsmax*(1-Ds/Ws), frac**c, dt_baseflow)`
            // —— `compute_vic_runoff` 里唯一一处收缩。
            baseflow_step = (soil.maximum_baseflow_mm_day / runoff_steps_per_day as f64
                * (1.0 - soil.baseflow_fraction / soil.baseflow_threshold))
                .mul_add(
                    ((relative_moisture - soil.baseflow_threshold)
                        / (1.0 - soil.baseflow_threshold))
                        .powf(soil.baseflow_exponent),
                    baseflow_step,
                );
        }
        baseflow_step = baseflow_step.max(0.0);
        // `liq = liq + Q12(lindex-1) - (evap + dt_baseflow)`：照上游的括号结合。
        liquid[bottom] =
            liquid[bottom] + drainage[bottom - 1] - (evaporation[bottom][frost] + baseflow_step);
        if liquid[bottom] + ice[bottom] < soil.residual_moisture_mm[bottom] {
            baseflow_step =
                baseflow_step + (liquid[bottom] + ice[bottom]) - soil.residual_moisture_mm[bottom];
            liquid[bottom] = soil.residual_moisture_mm[bottom] - ice[bottom];
        }
        let mut overflow = overflow_to_limit(
            &mut liquid[bottom],
            ice[bottom],
            soil.max_moisture_mm[bottom],
        );
        while overflow > 0.0 {
            let mut destination = bottom;
            loop {
                destination -= 1;
                liquid[destination] += overflow;
                overflow = overflow_to_limit(
                    &mut liquid[destination],
                    ice[destination],
                    soil.max_moisture_mm[destination],
                );
                if overflow <= 0.0 {
                    break;
                }
                if destination == 0 {
                    runoff += overflow;
                    overflow = 0.0;
                    break;
                }
            }
        }
        baseflow_step = baseflow_step.max(0.0);
        let (fraction, _) = runoff_and_saturation(soil, add(liquid, ice), 0.0);
        for layer in 0..VIC_LAYERS {
            moisture_mm[layer] += (liquid[layer] + ice[layer]) * frost_fraction[frost];
        }
        // `cell%runoff = cell%runoff + runoff(fidx)*frost_fract(fidx)`：GIMPLE `.FMA`（`:401`）。
        surface_runoff_mm = runoff.mul_add(frost_fraction[frost], surface_runoff_mm);
        subsurface_runoff_mm += baseflow_step * frost_fraction[frost];
        saturated_fraction += fraction * frost_fraction[frost];
    }
    ensure!(
        moisture_mm.iter().all(|value| value.is_finite())
            && surface_runoff_mm.is_finite()
            && subsurface_runoff_mm.is_finite()
            && saturated_fraction.is_finite(),
        "VIC runoff produced a non-finite state"
    );
    Ok(VicStepState {
        surface_runoff_mm,
        subsurface_runoff_mm,
        saturated_fraction,
    })
}

fn distribute_evaporation(
    cell: VicCell,
    residual: [f64; VIC_LAYERS],
    frost_fraction: [f64; VIC_LAYERS],
    frost_count: usize,
) -> [[f64; VIC_LAYERS]; VIC_LAYERS] {
    let mut output = [[0.0; VIC_LAYERS]; VIC_LAYERS];
    for layer in 0..VIC_LAYERS {
        let requested = cell.evaporation_mm[layer];
        output[layer][0] = requested;
        if requested <= 0.0 {
            for value in output[layer].iter_mut().take(frost_count).skip(1) {
                *value = requested;
            }
            continue;
        }
        let available = (0..frost_count)
            .map(|frost| {
                (cell.moisture_mm[layer] - cell.ice_mm[layer][frost] - residual[layer]).max(0.0)
            })
            .collect::<Vec<_>>();
        // `sum_liq = sum_liq + avail_liq*frost_fract`：GIMPLE `.FMA`（`MOD_Hydro_VIC.F90:165`）。
        let total = available
            .iter()
            .zip(frost_fraction)
            .take(frost_count)
            .fold(0.0, |sum, (&value, fraction)| value.mul_add(fraction, sum));
        let factor = if total > 0.0 { requested / total } else { 1.0 };
        for frost in 0..frost_count {
            output[layer][frost] = available[frost] * factor;
        }
    }
    output
}

fn runoff_and_saturation(soil: VicSoil, moisture: [f64; VIC_LAYERS], inflow: f64) -> (f64, f64) {
    let top_moisture =
        (moisture[0] + moisture[1]).min(soil.max_moisture_mm[0] + soil.max_moisture_mm[1]);
    let top_capacity = soil.max_moisture_mm[0] + soil.max_moisture_mm[1];
    let exponent = soil.infiltration_shape / (1.0 + soil.infiltration_shape);
    let saturation = 1.0 - (1.0 - top_moisture / top_capacity).powf(exponent);
    let maximum_infiltration = (1.0 + soil.infiltration_shape) * top_capacity;
    // GIMPLE（`MOD_Hydro_VIC.F90:453/459`）：`i_0` 被内联，`i_0 + inflow` 是
    // `.FMA (max_infil, 1-(1-A)**(1/b), inflow)`，`basis` 复用它；
    // `runoff = .FMA (basis**(1+b), top_max_moist, (inflow-top_max_moist)+top_moist)`。
    let infiltration_plus_inflow = maximum_infiltration.mul_add(
        1.0 - (1.0 - saturation).powf(1.0 / soil.infiltration_shape),
        inflow,
    );
    let runoff = if inflow == 0.0 {
        0.0
    } else if maximum_infiltration == 0.0 {
        inflow
    } else if infiltration_plus_inflow > maximum_infiltration {
        inflow - top_capacity + top_moisture
    } else {
        let basis = 1.0 - infiltration_plus_inflow / maximum_infiltration;
        basis
            .powf(1.0 + soil.infiltration_shape)
            .mul_add(top_capacity, inflow - top_capacity + top_moisture)
    }
    .max(0.0);
    (saturation, runoff)
}

/// `calc_Q12`（`MOD_Hydro_VIC.F90`）。
///
/// 源码里的 `1.0d0` 在 `-fdefault-real-8` 下被提升成 `real(kind=16)`，整条在 binary128 里
/// 求值（GIMPLE：`powq`、四倍精度的减法与除法）；只有 `(max_moist-resid_moist)**expt`
/// 与 `Ksat/…` 仍是 double。这里照同样的分段用双倍双精度复现，最后才转回 f64 ——
/// 全程 f64 在 AT-Neu VIC 算例第 0 步就让 `Q12` 差 1 ULP。
fn q12(conductivity: f64, moisture: f64, residual: f64, maximum: f64, exponent: f64) -> f64 {
    use crate::extended::DoubleDouble as Dd;
    let one_minus_exponent = Dd::new(1.0) - Dd::new(exponent);
    let first = Dd::new(moisture - residual).powf(one_minus_exponent);
    let scaled = conductivity / (maximum - residual).powf(exponent);
    let second = one_minus_exponent * Dd::new(scaled);
    let root = (first - second).powf(Dd::new(1.0) / one_minus_exponent);
    (Dd::new(moisture) - root - Dd::new(residual)).to_f64()
}

fn overflow_to_limit(liquid: &mut f64, ice: f64, maximum: f64) -> f64 {
    let overflow = (*liquid + ice - maximum).max(0.0);
    *liquid = (*liquid).min(maximum - ice);
    overflow
}

fn grouped_sum(values: &[f64]) -> [f64; VIC_LAYERS] {
    std::array::from_fn(|layer| values[group_ranges()[layer].clone()].iter().sum())
}

fn grouped_weighted(values: &[f64], weights: &[f64]) -> [f64; VIC_LAYERS] {
    std::array::from_fn(|layer| {
        let range = group_ranges()[layer].clone();
        // `CoLM2VIC_weight`：`vic = vic + colm*dz_soi` 逐层累加，GIMPLE 是
        // `.FMA (colm, dz, vic)`（`MOD_Hydro_VIC_Variables.F90:255/260`）。
        values[range.clone()]
            .iter()
            .zip(&weights[range.clone()])
            .fold(0.0, |sum, (&value, &weight)| value.mul_add(weight, sum))
            / weights[range].iter().sum::<f64>()
    })
}

fn project_to_colm(values: &[f64; VIC_LAYERS], thickness_m: &[f64]) -> Vec<f64> {
    (0..COLM_LAYERS)
        .map(|index| {
            let layer = GROUP_END.iter().position(|&end| index < end).unwrap();
            let range = group_ranges()[layer].clone();
            values[layer] * thickness_m[index] / thickness_m[range].iter().sum::<f64>()
        })
        .collect()
}

/// `VIC_IceLay`（`MOD_Hydro_VIC_Variables.F90`）：把一个 VIC 层的冰分到**三个冻土区**
/// （返回值的三个分量是冻土区，不是层）。
///
/// * 最后一律 `vic_ice(2) = totalSum - vic_ice(1) - vic_ice(3)`，所以中间那一区即使在
///   "三层直接拷贝"的情形下也要按这个差重算（直接拷贝会差 1 ULP）。
/// * `colm_lay > 3`（默认分组里最深的土层 7–10）那一支把 `intent(out)` 的 `vic_ice`
///   **当累加器用而不清零**（上游缺陷，见 `docs/upstream-bugs.md` 第 18 条）。gfortran 下它读到的是
///   调用方同一个局部数组里上一组（土层 4–6）刚写下的值，即 `previous`；这里照内核的实际行为。
///   累加是 `.FMA (ice, multiplier, acc)`，`multiplier` 是 1 或 0。
fn partition_ice(values: &[f64], previous: [f64; VIC_LAYERS]) -> [f64; VIC_LAYERS] {
    let total: f64 = values.iter().fold(0.0, |sum, value| sum + value);
    let layers = values.len();
    let vic_layers = VIC_LAYERS;
    let mut ice = previous;
    match layers {
        1 => ice = [total / 3.0; VIC_LAYERS],
        2 => {
            ice[0] = values[0] * 2.0 / 3.0;
            ice[2] = values[1] * 2.0 / 3.0;
        }
        3 => ice = [values[0], values[1], values[2]],
        _ => {
            // `DO idx = 1, min(int((colm_lay-1)/vic_lay), vic_lay)`（整数除法）。
            let last = ((layers - 1) / vic_layers).min(vic_layers);
            for idx in 1..=last {
                let multiplier = if layers > idx * vic_layers { 1.0 } else { 0.0 };
                ice[0] = values[idx - 1].mul_add(multiplier, ice[0]);
                ice[2] = values[layers - idx].mul_add(multiplier, ice[2]);
            }
            // 循环结束后 `idx = last + 1`；`merge((colm_lay-idx*vic_lay)/vic_lay, 0, …)` 是整数除法。
            let idx = last + 1;
            let multiplier = if layers <= (idx + 1) * vic_layers {
                ((layers as i64 - (idx * vic_layers) as i64) / vic_layers as i64) as f64
            } else {
                0.0
            };
            ice[0] = values[idx].mul_add(multiplier, ice[0]);
            ice[2] = values[layers - idx - 1].mul_add(multiplier, ice[2]);
        }
    }
    ice[1] = total - ice[0] - ice[2];
    ice
}

fn group_ranges() -> [std::ops::Range<usize>; VIC_LAYERS] {
    [
        0..GROUP_END[0],
        GROUP_END[0]..GROUP_END[1],
        GROUP_END[1]..GROUP_END[2],
    ]
}

fn add(left: [f64; VIC_LAYERS], right: [f64; VIC_LAYERS]) -> [f64; VIC_LAYERS] {
    std::array::from_fn(|index| left[index] + right[index])
}

fn validate(input: VicRunoffInput<'_>) -> Result<()> {
    for values in [
        input.layer_thickness_m,
        input.porosity,
        input.residual_water,
        input.saturated_hydraulic_conductivity_mm_s,
        input.clapp_hornberger_b,
        input.ice_water_kg_m2,
        input.liquid_water_kg_m2,
        input.root_flux_mm_s,
    ] {
        ensure!(
            values.len() == COLM_LAYERS && values.iter().all(|value| value.is_finite()),
            "VIC runoff needs ten finite CoLM soil layers"
        );
    }
    ensure!(
        input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.porosity.iter().all(|value| *value > 0.0)
            && input
                .residual_water
                .iter()
                .zip(input.porosity)
                .all(|(&residual, &porosity)| residual >= 0.0 && residual < porosity)
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0)
            && input.clapp_hornberger_b.iter().all(|value| *value > 0.0)
            && input.ice_water_kg_m2.iter().all(|value| *value >= 0.0)
            && input.liquid_water_kg_m2.iter().all(|value| *value >= 0.0),
        "VIC runoff layer inputs are invalid"
    );
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.time_step_seconds <= 86_400.0
            && input.ground_evaporation_mm_s.is_finite()
            && input.water_input_mm_s.is_finite()
            && input.infiltration_shape.is_finite()
            && input.infiltration_shape > 0.0
            && input.maximum_baseflow_mm_day.is_finite()
            && input.maximum_baseflow_mm_day >= 0.0
            && input.baseflow_fraction.is_finite()
            && input.baseflow_fraction >= 0.0
            && input.baseflow_threshold.is_finite()
            && input.baseflow_threshold > 0.0
            && input.baseflow_threshold < 1.0
            && input.baseflow_exponent.is_finite()
            && input.baseflow_exponent > 0.0,
        "VIC runoff scalar inputs are invalid"
    );
    Ok(())
}

#[cfg(test)]
#[path = "vic_tests.rs"]
mod vic_tests;
