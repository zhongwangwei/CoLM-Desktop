//! Campbell/Richards soil-water solve from `MOD_SoilSnowHydrology:soilwater`.
//!
//! This is the non-VSF branch used by `WATER_2014`.  It owns no restart or
//! forcing I/O, so the initializer and the eventual Rust `colm` driver share
//! the same tridiagonal solve.

use crate::LibmPow;
use anyhow::{ensure, Context, Result};
use colm_numeric::Contract;

use crate::f77;

use crate::{solve_tridiagonal, topmodel_subsurface_runoff, TopmodelSubsurfaceInput, FREEZING_K};

/// One `soilwater` call. All layer vectors use top-to-bottom order; water and
/// flux lengths are in millimetres and seconds, matching the upstream routine.
#[derive(Debug, Clone, Copy)]
pub struct CampbellSoilWaterInput<'a> {
    pub patch_type: i32,
    pub time_step_seconds: f64,
    pub impermeable_porosity: f64,
    pub minimum_potential_mm: f64,
    pub infiltration_mm_s: f64,
    pub transpiration_mm_s: f64,
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub temperature_k: &'a [f64],
    pub liquid_water: &'a [f64],
    pub ice_fraction: &'a [f64],
    pub effective_porosity: &'a [f64],
    pub porosity: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub root_fraction: &'a [f64],
    pub root_flux_mm_s: &'a [f64],
    pub plant_hydraulics: bool,
    pub urban_run: bool,
    pub soil_ice_impedance: f64,
}

/// Resolved hydraulic state and fluxes from one `soilwater` call.
#[derive(Debug, Clone, PartialEq)]
pub struct CampbellSoilWaterState {
    pub liquid_water_change: Vec<f64>,
    pub recharge_mm_s: f64,
    pub matric_potential_mm: Vec<f64>,
    pub hydraulic_conductivity_mm_s: Vec<f64>,
    /// Surface through bottom interface, positive downward.
    pub interface_flux_mm_s: Vec<f64>,
    pub root_uptake_mm_s: Vec<f64>,
    pub root_uptake_amount_mm: Vec<f64>,
}

/// Ports `MOD_SoilSnowHydrology.F90:soilwater` for the Campbell hydraulic model.
///
/// The upstream routine deliberately always uses free drainage at the bottom;
/// its former water-table boundary branch is commented out, so water-table
/// depth is not an input to this active equation.
pub fn solve_campbell_soil_water(
    input: CampbellSoilWaterInput<'_>,
) -> Result<CampbellSoilWaterState> {
    let layers = validate(input)?;
    let plant_hydraulic_uptake =
        input.plant_hydraulics && (input.patch_type != 1 || !input.urban_run);
    // `rmx = qin - qout - etr*rootr`（`:2391/:2411/:2438`）：非 PHS 时 `etr*rootr` 收进
    // `.FNMA (etr, rootr, qin - qout)`；PHS 时减的是 `rootflux`，没有乘积。
    let source_term = |net_inflow: f64, layer: usize| -> f64 {
        if plant_hydraulic_uptake {
            net_inflow - input.root_flux_mm_s[layer]
        } else {
            (-input.transpiration_mm_s).contract(input.root_fraction[layer], net_inflow)
        }
    };
    let root_uptake_mm_s = if plant_hydraulic_uptake {
        input.root_flux_mm_s.to_vec()
    } else {
        input
            .root_fraction
            .iter()
            .map(|fraction| input.transpiration_mm_s * fraction)
            .collect()
    };
    let root_uptake_amount_mm = root_uptake_mm_s
        .iter()
        .map(|flux| flux.max(0.0) * input.time_step_seconds)
        .collect();

    let mut matric_potential_mm = vec![0.0; layers];
    let mut potential_derivative = vec![0.0; layers];
    for layer in 0..layers {
        if input.temperature_k[layer] >= FREEZING_K {
            if input.porosity[layer] < 1.0e-6 {
                matric_potential_mm[layer] = input.saturated_potential_mm[layer];
            } else {
                let saturation =
                    (input.liquid_water[layer] / input.porosity[layer]).clamp(0.01, 1.0);
                let potential = input.saturated_potential_mm[layer]
                    * saturation.lpow(-input.clapp_hornberger_b[layer]);
                matric_potential_mm[layer] = potential.max(input.minimum_potential_mm);
                potential_derivative[layer] = -input.clapp_hornberger_b[layer]
                    * matric_potential_mm[layer]
                    / (saturation * input.porosity[layer]);
            }
        } else {
            matric_potential_mm[layer] = (1.0e3 * 0.3336e6 / 9.80616
                * (input.temperature_k[layer] - FREEZING_K)
                / input.temperature_k[layer])
                .max(input.minimum_potential_mm);
        }
    }

    let mut separation_mm = vec![0.0; layers];
    let mut gradient = vec![0.0; layers];
    let mut hydraulic_conductivity_mm_s = vec![0.0; layers];
    let mut conductivity_lower_derivative = vec![0.0; layers];
    let mut conductivity_upper_derivative = vec![0.0; layers];
    for layer in 0..layers {
        if layer + 1 < layers {
            // `den = zmm(j+1) - zmm(j)`，`zmm(j) = z_soisno(j)*1000`（`:2263`）：先各自换算
            // 成毫米再相减。写成 `(z(j+1)-z(j))*1000` 舍入不同，Campbell 首步就差 1 ulp（第 406 轮）。
            separation_mm[layer] =
                input.node_depth_m[layer + 1] * 1000.0 - input.node_depth_m[layer] * 1000.0;
            gradient[layer] = (matric_potential_mm[layer + 1] - matric_potential_mm[layer])
                / separation_mm[layer]
                - 1.0;
        }
        if input.effective_porosity[layer] < input.impermeable_porosity
            || input.effective_porosity[(layer + 1).min(layers - 1)] < input.impermeable_porosity
            || input.liquid_water[layer] <= 1.0e-3
        {
            continue;
        }
        let source = if layer + 1 < layers && gradient[layer] > 0.0 {
            layer + 1
        } else {
            layer
        };
        let saturation = input.liquid_water[source] / input.porosity[source];
        // `soilwater:2172-2173`：`hk = hksati*(vol/porsl)**(2*bsw+3)`、
        // `dhkdw1 = hksati*(2*bsw+3)*(vol/porsl)**(2*bsw+2)/porsl`。
        // **两个指数是各自算出来的**（GIMPLE：`FMA(bsw,2,3)` 与 `FMA(bsw,2,2)`），
        // 不能写成 `exponent - 1.0` —— 后者多舍入一次，跨 binade 时差 1 ULP。
        let exponent = f77(2.0).contract(input.clapp_hornberger_b[source], 3.0);
        let exponent_lower = f77(2.0).contract(input.clapp_hornberger_b[source], 2.0);
        let conductivity =
            input.saturated_hydraulic_conductivity_mm_s[source] * saturation.lpow(exponent);
        let derivative = input.saturated_hydraulic_conductivity_mm_s[source]
            * exponent
            * saturation.lpow(exponent_lower)
            / input.porosity[source];
        let impedance = 10_f64.lpow(
            -input.soil_ice_impedance
                * 0.5
                * (input.ice_fraction[layer] + input.ice_fraction[(layer + 1).min(layers - 1)]),
        );
        hydraulic_conductivity_mm_s[layer] = impedance * conductivity;
        if source == layer {
            conductivity_lower_derivative[layer] = impedance * derivative;
        } else {
            conductivity_upper_derivative[layer] = impedance * derivative;
        }
    }

    let thickness_mm = input
        .layer_thickness_m
        .iter()
        .map(|thickness| thickness * 1000.0)
        .collect::<Vec<_>>();
    let mut lower = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut upper = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];
    let mut outflow = vec![0.0; layers];
    let mut outflow_lower_derivative = vec![0.0; layers];
    let mut outflow_upper_derivative = vec![0.0; layers];

    outflow[0] = -hydraulic_conductivity_mm_s[0] * gradient[0];
    // `soilwater` 的三对角装配（`:2186-2192` 那一组）：GIMPLE 是
    // `FMS(gradient, 导数, 商)` / `FMA(gradient, 导数, 商)` —— 商先各自舍入，
    // 与 `gradient` 相乘的那个乘积被吸收。三处循环体（首层/中间/末层）同型。
    outflow_lower_derivative[0] = -gradient[0].contract(
        conductivity_lower_derivative[0],
        -(hydraulic_conductivity_mm_s[0] * potential_derivative[0] / separation_mm[0]),
    );
    outflow_upper_derivative[0] = -gradient[0].contract(
        conductivity_upper_derivative[0],
        hydraulic_conductivity_mm_s[0] * potential_derivative[1] / separation_mm[0],
    );
    diagonal[0] = thickness_mm[0] / input.time_step_seconds + outflow_lower_derivative[0];
    upper[0] = outflow_upper_derivative[0];
    rhs[0] = source_term(input.infiltration_mm_s - outflow[0], 0);

    for layer in 1..layers - 1 {
        let inflow = -hydraulic_conductivity_mm_s[layer - 1] * gradient[layer - 1];
        let inflow_lower_derivative = -gradient[layer - 1].contract(
            conductivity_lower_derivative[layer - 1],
            -(hydraulic_conductivity_mm_s[layer - 1] * potential_derivative[layer - 1]
                / separation_mm[layer - 1]),
        );
        let inflow_upper_derivative = -gradient[layer - 1].contract(
            conductivity_upper_derivative[layer - 1],
            hydraulic_conductivity_mm_s[layer - 1] * potential_derivative[layer]
                / separation_mm[layer - 1],
        );
        outflow[layer] = -hydraulic_conductivity_mm_s[layer] * gradient[layer];
        outflow_lower_derivative[layer] = -gradient[layer].contract(
            conductivity_lower_derivative[layer],
            -(hydraulic_conductivity_mm_s[layer] * potential_derivative[layer]
                / separation_mm[layer]),
        );
        outflow_upper_derivative[layer] = -gradient[layer].contract(
            conductivity_upper_derivative[layer],
            hydraulic_conductivity_mm_s[layer] * potential_derivative[layer + 1]
                / separation_mm[layer],
        );
        lower[layer] = -inflow_lower_derivative;
        diagonal[layer] = thickness_mm[layer] / input.time_step_seconds - inflow_upper_derivative
            + outflow_lower_derivative[layer];
        upper[layer] = outflow_upper_derivative[layer];
        rhs[layer] = source_term(inflow - outflow[layer], layer);
    }

    let last = layers - 1;
    let inflow = -hydraulic_conductivity_mm_s[last - 1] * gradient[last - 1];
    let inflow_lower_derivative = -gradient[last - 1].contract(
        conductivity_lower_derivative[last - 1],
        -(hydraulic_conductivity_mm_s[last - 1] * potential_derivative[last - 1]
            / separation_mm[last - 1]),
    );
    let inflow_upper_derivative = -gradient[last - 1].contract(
        conductivity_upper_derivative[last - 1],
        hydraulic_conductivity_mm_s[last - 1] * potential_derivative[last]
            / separation_mm[last - 1],
    );
    outflow[last] = hydraulic_conductivity_mm_s[last];
    outflow_lower_derivative[last] = conductivity_lower_derivative[last];
    lower[last] = -inflow_lower_derivative;
    diagonal[last] = thickness_mm[last] / input.time_step_seconds - inflow_upper_derivative
        + outflow_lower_derivative[last];
    rhs[last] = source_term(inflow - outflow[last], last);

    let liquid_water_change = solve_tridiagonal(&lower, &diagonal, &upper, &rhs)
        .map_err(anyhow::Error::msg)
        .context("soilwater tridiagonal solve failed")?;
    ensure!(
        liquid_water_change.iter().all(|value| value.is_finite()),
        "soilwater solve produced a non-finite water-content change"
    );
    let recharge_mm_s = outflow[last] + outflow_lower_derivative[last] * liquid_water_change[last];
    let mut interface_flux_mm_s = Vec::with_capacity(layers + 1);
    interface_flux_mm_s.push(input.infiltration_mm_s);
    for layer in 0..last {
        interface_flux_mm_s.push(
            outflow[layer]
                + outflow_lower_derivative[layer] * liquid_water_change[layer]
                + outflow_upper_derivative[layer] * liquid_water_change[layer + 1],
        );
    }
    interface_flux_mm_s.push(recharge_mm_s);
    ensure!(
        recharge_mm_s.is_finite() && interface_flux_mm_s.iter().all(|value| value.is_finite()),
        "soilwater solve produced a non-finite flux"
    );
    Ok(CampbellSoilWaterState {
        liquid_water_change,
        recharge_mm_s,
        matric_potential_mm,
        hydraulic_conductivity_mm_s,
        interface_flux_mm_s,
        root_uptake_mm_s,
        root_uptake_amount_mm,
    })
}

fn validate(input: CampbellSoilWaterInput<'_>) -> Result<usize> {
    let layers = input.node_depth_m.len();
    ensure!(layers >= 2, "soilwater needs at least two soil layers");
    for values in [
        input.layer_thickness_m,
        input.temperature_k,
        input.liquid_water,
        input.ice_fraction,
        input.effective_porosity,
        input.porosity,
        input.saturated_hydraulic_conductivity_mm_s,
        input.clapp_hornberger_b,
        input.saturated_potential_mm,
        input.root_fraction,
        input.root_flux_mm_s,
    ] {
        ensure!(
            values.len() == layers,
            "soilwater vectors must have equal lengths"
        );
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "soilwater vectors must be finite"
        );
    }
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.impermeable_porosity.is_finite()
            && input.impermeable_porosity >= 0.0
            && input.minimum_potential_mm.is_finite()
            && input.minimum_potential_mm < 0.0
            && input.infiltration_mm_s.is_finite()
            && input.transpiration_mm_s.is_finite()
            && input.soil_ice_impedance.is_finite()
            && input.soil_ice_impedance > 0.0,
        "soilwater scalar inputs are invalid"
    );
    for layer in 0..layers {
        ensure!(
            input.layer_thickness_m[layer] > 0.0
                && input.porosity[layer] >= 0.0
                && input.effective_porosity[layer] >= 0.0
                && input.effective_porosity[layer] <= input.porosity[layer]
                && input.liquid_water[layer] >= -crate::SOIL_WATER_ROUNDOFF_KG_M2
                && (0.0..=1.0).contains(&input.ice_fraction[layer])
                && input.saturated_hydraulic_conductivity_mm_s[layer] >= 0.0
                && input.clapp_hornberger_b[layer] > 0.0
                && input.saturated_potential_mm[layer] < 0.0,
            "soilwater layer inputs are invalid"
        );
        if layer > 0 {
            ensure!(
                input.node_depth_m[layer] > input.node_depth_m[layer - 1],
                "soilwater node depths must increase"
            );
        }
    }
    Ok(layers)
}

#[cfg(test)]
#[path = "soil_water_tests.rs"]
mod soil_water_tests;

/// The post-Richards aquifer and excess-water update from `groundwater`.
#[derive(Debug, Clone, Copy)]
pub struct GroundwaterInput<'a> {
    pub time_step_seconds: f64,
    pub ponding_limit_mm: f64,
    pub effective_porosity: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    /// Soil interfaces, including the zero-depth top interface.
    pub interface_depth_m: &'a [f64],
    pub ice_water_kg_m2: &'a [f64],
    pub liquid_water_kg_m2: &'a [f64],
    pub porosity: &'a [f64],
    pub saturated_potential_mm: &'a [f64],
    pub clapp_hornberger_b: &'a [f64],
    pub water_table_depth_m: f64,
    pub aquifer_water_mm: f64,
    pub recharge_mm_s: f64,
    /// The active runoff scheme's already-resolved subsurface runoff.
    pub subsurface_runoff_mm_s: f64,
}

/// State returned by `groundwater`, including its nonphysical excess-water
/// corrections that are part of the upstream model contract.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundwaterState {
    pub liquid_water_kg_m2: Vec<f64>,
    pub water_table_depth_m: f64,
    pub aquifer_water_mm: f64,
    pub subsurface_runoff_mm_s: f64,
}

/// Ports `MOD_SoilSnowHydrology.F90:groundwater` after TOPMODEL/CaMa has
/// resolved the supplied subsurface runoff flux.
pub fn update_groundwater(input: GroundwaterInput<'_>) -> Result<GroundwaterState> {
    update_groundwater_with_resolver(input, |_| Ok(input.subsurface_runoff_mm_s))
}

/// Runs groundwater correction with TOPMODEL baseflow evaluated after recharge
/// moves the water table, matching `MOD_SoilSnowHydrology:groundwater`.
pub fn update_groundwater_topmodel(
    input: GroundwaterInput<'_>,
    topmodel: TopmodelSubsurfaceInput<'_>,
) -> Result<GroundwaterState> {
    ensure!(
        topmodel.layer_thickness_m.len() == input.layer_thickness_m.len()
            && topmodel.interface_depth_m == input.interface_depth_m
            && topmodel.ice_fraction.len() == input.layer_thickness_m.len(),
        "TOPMODEL groundwater inputs must use the same soil column"
    );
    update_groundwater_with_resolver(input, |water_table_depth_m| {
        topmodel_subsurface_runoff(TopmodelSubsurfaceInput {
            water_table_depth_m,
            ..topmodel
        })
    })
}

fn update_groundwater_with_resolver(
    input: GroundwaterInput<'_>,
    resolve_subsurface_runoff: impl FnOnce(f64) -> Result<f64>,
) -> Result<GroundwaterState> {
    let layers = validate_groundwater(input)?;
    let thickness_mm = input
        .layer_thickness_m
        .iter()
        .map(|thickness| thickness * 1000.0)
        .collect::<Vec<_>>();
    let mut liquid_water_kg_m2 = input.liquid_water_kg_m2.to_vec();
    let mut water_table_depth_m = input.water_table_depth_m;
    let mut aquifer_water_mm = input.aquifer_water_mm;
    let initial_water_table_layer = water_table_layer(water_table_depth_m, input.interface_depth_m);
    let lower_specific_yield = specific_yield(
        input.porosity[layers - 1],
        water_table_depth_m,
        input.saturated_potential_mm[layers - 1],
        input.clapp_hornberger_b[layers - 1],
    );

    // `wa = wa + qcharge*deltim`（`:2561`）：`main/` 里 `deltim*qcharge` 与 `:2566`/
    // `qcharge_tot` 共用，于是**不**收缩（扩展版内核是 FMA；第 406 轮按 `main/` 改回）。
    aquifer_water_mm += input.time_step_seconds * input.recharge_mm_s;
    if initial_water_table_layer == layers {
        water_table_depth_m = (water_table_depth_m
            - input.recharge_mm_s * input.time_step_seconds / 1000.0 / lower_specific_yield)
            .max(0.0);
    } else {
        let mut recharge = input.recharge_mm_s * input.time_step_seconds;
        if recharge > 0.0 {
            for layer in (0..=initial_water_table_layer).rev() {
                let yield_ = specific_yield(
                    input.porosity[layer],
                    water_table_depth_m,
                    input.saturated_potential_mm[layer],
                    input.clapp_hornberger_b[layer],
                );
                let transferred = recharge
                    .min(yield_ * (water_table_depth_m - input.interface_depth_m[layer]) * 1000.0)
                    .max(0.0);
                water_table_depth_m =
                    (water_table_depth_m - transferred / yield_ / 1000.0).max(0.0);
                recharge -= transferred;
                if recharge <= 0.0 {
                    break;
                }
            }
        } else {
            for layer in initial_water_table_layer..layers {
                let yield_ = specific_yield(
                    input.porosity[layer],
                    water_table_depth_m,
                    input.saturated_potential_mm[layer],
                    input.clapp_hornberger_b[layer],
                );
                let transferred = recharge
                    .max(
                        -yield_
                            * (input.interface_depth_m[layer + 1] - water_table_depth_m)
                            * 1000.0,
                    )
                    .min(0.0);
                recharge -= transferred;
                if recharge >= 0.0 {
                    water_table_depth_m =
                        (water_table_depth_m - transferred / yield_ / 1000.0).max(0.0);
                    break;
                }
                water_table_depth_m = input.interface_depth_m[layer + 1];
            }
            if recharge > 0.0 {
                water_table_depth_m =
                    (water_table_depth_m - recharge / 1000.0 / lower_specific_yield).max(0.0);
            }
        }
    }

    let drainage_mm_s = resolve_subsurface_runoff(water_table_depth_m)?;
    ensure!(
        drainage_mm_s.is_finite(),
        "groundwater subsurface runoff must be finite"
    );
    if initial_water_table_layer == layers {
        // `wa = wa - drainage*deltim`（`:2628`）：乘积与 `:2629` 共用，不收缩。
        aquifer_water_mm -= input.time_step_seconds * drainage_mm_s;
        water_table_depth_m = (water_table_depth_m
            + drainage_mm_s * input.time_step_seconds / 1000.0 / lower_specific_yield)
            .max(0.0);
        liquid_water_kg_m2[layers - 1] += (aquifer_water_mm - 5000.0).max(0.0);
        aquifer_water_mm = aquifer_water_mm.min(5000.0);
    } else {
        let mut drainage = -drainage_mm_s * input.time_step_seconds;
        for (layer, liquid_water) in liquid_water_kg_m2
            .iter_mut()
            .enumerate()
            .skip(initial_water_table_layer)
        {
            let yield_ = specific_yield(
                input.porosity[layer],
                water_table_depth_m,
                input.saturated_potential_mm[layer],
                input.clapp_hornberger_b[layer],
            );
            let transferred = drainage
                .max(-yield_ * (input.interface_depth_m[layer + 1] - water_table_depth_m) * 1000.0)
                .min(0.0);
            *liquid_water += transferred;
            drainage -= transferred;
            if drainage >= 0.0 {
                water_table_depth_m =
                    (water_table_depth_m - transferred / yield_ / 1000.0).max(0.0);
                break;
            }
            water_table_depth_m = input.interface_depth_m[layer + 1];
        }
        water_table_depth_m =
            (water_table_depth_m - drainage / 1000.0 / lower_specific_yield).max(0.0);
        aquifer_water_mm += drainage;
    }

    water_table_depth_m = water_table_depth_m.clamp(0.0, 80.0);
    let mut subsurface_runoff_mm_s = drainage_mm_s;
    for layer in (1..layers).rev() {
        let capacity = input.effective_porosity[layer] * thickness_mm[layer];
        let excess = (liquid_water_kg_m2[layer] - capacity).max(0.0);
        liquid_water_kg_m2[layer] = liquid_water_kg_m2[layer].min(capacity);
        liquid_water_kg_m2[layer - 1] += excess;
    }
    // `MOD_SoilSnowHydrology.F90` 的 `groundwater`（被 `water_2014` 内联）：
    // `xs1 = wliq(1) - (pondmx+porsl(1)*dzmm(1)-wice(1))`；
    // GIMPLE 把 `porsl(1)*dzmm(1)` 收进加法：`FMA(porsl[0], dzmm[0], pondmx)`。
    let top_capacity = input.porosity[0].contract(thickness_mm[0], input.ponding_limit_mm)
        - input.ice_water_kg_m2[0];
    let excess_top = (liquid_water_kg_m2[0] - top_capacity).max(0.0);
    liquid_water_kg_m2[0] = liquid_water_kg_m2[0].min(top_capacity);
    subsurface_runoff_mm_s += excess_top / input.time_step_seconds;

    let mut deficit = 0.0;
    for value in &mut liquid_water_kg_m2 {
        if *value < 0.0 {
            deficit += *value;
            *value = 0.0;
        }
    }
    subsurface_runoff_mm_s += deficit / input.time_step_seconds;
    if subsurface_runoff_mm_s < 0.0 {
        // `:2719 wa = wa + rsubst*deltim` 是 `.FMA (deltim, rsubst, wa)`。
        aquifer_water_mm = input
            .time_step_seconds
            .contract(subsurface_runoff_mm_s, aquifer_water_mm);
        subsurface_runoff_mm_s = 0.0;
    }
    ensure!(
        liquid_water_kg_m2.iter().all(|value| value.is_finite())
            && aquifer_water_mm.is_finite()
            && subsurface_runoff_mm_s.is_finite(),
        "groundwater update produced a non-finite state"
    );
    Ok(GroundwaterState {
        liquid_water_kg_m2,
        water_table_depth_m,
        aquifer_water_mm,
        subsurface_runoff_mm_s,
    })
}

fn water_table_layer(water_table_depth_m: f64, interfaces: &[f64]) -> usize {
    interfaces[1..]
        .iter()
        .position(|depth| water_table_depth_m <= *depth)
        .unwrap_or(interfaces.len() - 1)
}

fn specific_yield(porosity: f64, water_table_depth_m: f64, psi0_mm: f64, bsw: f64) -> f64 {
    (porosity * (1.0 - (1.0 - 1.0e3 * water_table_depth_m / psi0_mm).lpow(-1.0 / bsw))).max(0.02)
}

fn validate_groundwater(input: GroundwaterInput<'_>) -> Result<usize> {
    let layers = input.layer_thickness_m.len();
    ensure!(layers > 0, "groundwater needs at least one soil layer");
    ensure!(
        input.interface_depth_m.len() == layers + 1,
        "groundwater needs one more interface than soil layers"
    );
    for values in [
        input.effective_porosity,
        input.ice_water_kg_m2,
        input.liquid_water_kg_m2,
        input.porosity,
        input.saturated_potential_mm,
        input.clapp_hornberger_b,
    ] {
        ensure!(
            values.len() == layers,
            "groundwater vectors must have equal lengths"
        );
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "groundwater vectors must be finite"
        );
    }
    ensure!(
        input.time_step_seconds.is_finite()
            && input.time_step_seconds > 0.0
            && input.ponding_limit_mm.is_finite()
            && input.ponding_limit_mm >= 0.0
            && input.water_table_depth_m.is_finite()
            && input.water_table_depth_m >= 0.0
            && input.aquifer_water_mm.is_finite()
            && input.recharge_mm_s.is_finite()
            && input.subsurface_runoff_mm_s.is_finite(),
        "groundwater scalar inputs are invalid"
    );
    ensure!(
        input.interface_depth_m[0] == 0.0
            && input
                .interface_depth_m
                .windows(2)
                .all(|pair| pair[1].is_finite() && pair[1] > pair[0]),
        "groundwater interfaces must start at zero and increase"
    );
    for layer in 0..layers {
        ensure!(
            input.layer_thickness_m[layer] > 0.0
                && input.effective_porosity[layer] >= 0.0
                && input.effective_porosity[layer] <= input.porosity[layer]
                && input.porosity[layer] >= 0.0
                && input.ice_water_kg_m2[layer] >= 0.0
                && input.saturated_potential_mm[layer] < 0.0
                && input.clapp_hornberger_b[layer] > 0.0,
            "groundwater layer inputs are invalid"
        );
    }
    Ok(layers)
}
