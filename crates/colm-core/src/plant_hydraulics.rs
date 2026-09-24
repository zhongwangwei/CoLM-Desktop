//! Two-big-leaf plant hydraulics from `MOD_PlantHydraulic.F90`.
//!
//! The solver is intentionally independent of canopy radiation and NetCDF so
//! the normal LCT leaf solver and a future PFT runtime use the same hydraulic
//! root-to-leaf network.

use anyhow::{ensure, Context, Result};

use crate::solve_tridiagonal;

const SUNLIT: usize = 0;
const SHADED: usize = 1;
const XYLEM: usize = 2;
const ROOT: usize = 3;
/// `nvegwcs`（`MOD_Namelist.F90` 的 `nvegwcs = 4`）：`vegwp` 的四个节点
/// —— 阳生叶、阴生叶、木质部、根。
///
/// 公开是因为**重启里的 `vegwp` 就是这个长度**，装配层要按它读
/// （`(patch, vegnodes)`），不能另写一个 4。
pub const VEGETATION_SEGMENTS: usize = 4;
const MIN_STRESS: f64 = 1.0e-2;
const MIN_CONDUCTANCE: f64 = 1.0e-16;

/// `MOD_PlantHydraulic.F90:148` 的 `rpi = 3.14159265358979_r8`。
///
/// 它是**截断到 15 位有效数字的 π**，与 `std::f64::consts::PI` 差 **7 ULP**
/// （实测相对差 9.895e-16）。两处用途都在根导度那一圈：
/// `root_cross_sec_area = rpi*r**2`（`:167`）与 `r_soil = sqrt(1/(rpi*rld))`（`:177`）。
/// 解析上 `rpi` 会在 `r_soil = sqrt(density*r**2/biomass)` 里约掉，**浮点上不会** ——
/// 用标准 π 会让 `k_soil_root` 系统性偏 ~1 ULP，而它直接乘进
/// `rootflux = k*(smp-xroot)`。第 299 轮实测：`rootflux` 在干窗第 12 步第 2 层
/// 差 1 ULP（`RFINL`，缩放**之前**就已经差），换回这个字面量后全同。
/// 差不是笔误：**故意**用上游那个截断值，`clippy::approx_constant` 在这里必须让路。
#[allow(clippy::approx_constant)]
const PLANT_HYDRAULIC_PI: f64 = 3.14159265358979;

/// Runtime equivalents of the `DEF_PH_*` namelist parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantHydraulicParameters {
    pub coarse_root_lateral_length_m: f64,
    pub axial_root_conductivity: f64,
    pub fine_root_carbon_g_c_m2: f64,
    pub fine_root_radius_m: f64,
    pub root_tissue_density_g_m3: f64,
    pub fine_root_to_leaf_area: f64,
    pub maximum_radial_root_conductance: f64,
}

impl Default for PlantHydraulicParameters {
    fn default() -> Self {
        Self {
            coarse_root_lateral_length_m: 0.25,
            axial_root_conductivity: 2.0e-1,
            fine_root_carbon_g_c_m2: 288.392_056_287_006,
            fine_root_radius_m: 2.9e-4,
            root_tissue_density_g_m3: 310_000.0,
            fine_root_to_leaf_area: 1.5,
            maximum_radial_root_conductance: 3.981_071_705_534_969e-9,
        }
    }
}

/// Leaf-to-soil inputs used by the normal LCT plant-hydraulic solve.
#[derive(Debug, Clone, Copy)]
pub struct PlantHydraulicInput<'a> {
    pub node_depth_m: &'a [f64],
    pub layer_thickness_m: &'a [f64],
    pub root_fraction: &'a [f64],
    pub soil_matric_potential_mm: &'a [f64],
    pub soil_hydraulic_conductivity_mm_s: &'a [f64],
    pub saturated_hydraulic_conductivity_mm_s: &'a [f64],
    pub surface_pressure_pa: f64,
    pub leaf_saturation_specific_humidity: f64,
    pub canopy_air_specific_humidity: f64,
    pub ground_specific_humidity: f64,
    pub reference_specific_humidity: f64,
    pub leaf_temperature_k: f64,
    pub leaf_boundary_resistance_s_m: f64,
    pub soil_surface_resistance_s_m: f64,
    pub reference_to_canopy_moisture_resistance_s_m: f64,
    pub ground_to_canopy_moisture_resistance_s_m: f64,
    pub air_density_kg_m3: f64,
    pub wet_canopy_fraction: f64,
    pub sunlit_leaf_area_index: f64,
    pub shaded_leaf_area_index: f64,
    pub stem_area_index: f64,
    pub canopy_top_height_m: f64,
    pub maximum_sunlit_leaf_conductance_umol_m2_s: f64,
    pub maximum_shaded_leaf_conductance_umol_m2_s: f64,
    pub maximum_sunlit_leaf_hydraulic_conductance: f64,
    pub maximum_shaded_leaf_hydraulic_conductance: f64,
    pub maximum_xylem_hydraulic_conductance: f64,
    pub maximum_root_hydraulic_conductance: f64,
    pub sunlit_leaf_psi50_mm: f64,
    pub shaded_leaf_psi50_mm: f64,
    pub xylem_psi50_mm: f64,
    pub root_psi50_mm: f64,
    pub vulnerability_shape: f64,
    /// `DEF_RSS_SCHEME`; scheme 4 uses a conductance-style soil factor.
    pub soil_surface_resistance_scheme: i32,
    pub parameters: PlantHydraulicParameters,
}

/// Persistent potential of the four source compartments: sunlit leaf, shaded
/// leaf, xylem, and root (mm water).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantHydraulicState {
    pub vegetation_water_potential_mm: [f64; VEGETATION_SEGMENTS],
}

/// Outputs of `PlantHydraulicStress_twoleaf`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlantHydraulicOutput {
    pub sunlit_stress: f64,
    pub shaded_stress: f64,
    pub sunlit_transpiration_kg_m2_s: f64,
    pub shaded_transpiration_kg_m2_s: f64,
    pub root_flux_kg_m2_s: Vec<f64>,
    pub soil_root_conductance_mm_s: Vec<f64>,
    pub axial_root_conductance_mm_s: Vec<f64>,
    pub sunlit_stomatal_conductance_umol_m2_s: f64,
    pub shaded_stomatal_conductance_umol_m2_s: f64,
}

/// Runs `MOD_PlantHydraulic:PlantHydraulicStress_twoleaf`.
///
/// The upper leaf solver supplies the unstressed stomatal conductances from
/// `stomata`; this routine returns their hydraulic reduction and soil-layer
/// uptake.  It mutates only the persistent vegetation water potentials.
pub fn plant_hydraulic_stress(
    input: PlantHydraulicInput<'_>,
    state: &mut PlantHydraulicState,
) -> Result<PlantHydraulicOutput> {
    let layers = validate(input, *state)?;
    let (soil_root_conductance_mm_s, axial_root_conductance_mm_s) = root_conductances(input)?;
    let conversion = conductance_conversion(input.surface_pressure_pa, input.leaf_temperature_k);
    // 上游写的是 `1./rb * cf`（先取倒数再乘），不是 `cf/rb`。两者差 1 ULP，
    // 而 `boundary_conductance` 会流到整个 A/f 方程组，能翻掉
    // `shaded_flux > 0`、`determinant != 0`、`max|dx| > 2e5` 这几个**离散**判据
    // （与已修的 `sqrt(2)` 倒数同类）。所以照抄结合顺序。
    let boundary_conductance = 1.0 / input.leaf_boundary_resistance_s_m * conversion;
    let (sunlit_demand, shaded_demand) = transpiration_from_conductance(
        input,
        boundary_conductance,
        input.maximum_sunlit_leaf_conductance_umol_m2_s,
        input.maximum_shaded_leaf_conductance_umol_m2_s,
        None,
    );
    let mut potential = state.vegetation_water_potential_mm;
    let (
        sunlit_transpiration_kg_m2_s,
        shaded_transpiration_kg_m2_s,
        sunlit_stomatal_conductance_umol_m2_s,
        shaded_stomatal_conductance_umol_m2_s,
        root_flux_kg_m2_s,
        sunlit_stress,
        shaded_stress,
    ) = if sunlit_demand > 0.0 || shaded_demand > 0.0 {
        let (root_flux, root_flux_slope) = root_flux_from_top_potential(
            input,
            potential[ROOT],
            &soil_root_conductance_mm_s,
            &axial_root_conductance_mm_s,
        )?;
        let change = spac_change(
            potential,
            sunlit_demand,
            shaded_demand,
            input,
            root_flux,
            root_flux_slope,
        );
        // `MOD_PlantHydraulic.F90:329-332`：这一步的**目的**是把过大的牛顿步压到
        // 「当前水势尺度的一半」以内，所以它取的是 `min(max|dx|, max|x|)/2`。
        // 原先两处都写成 `max`：触发条件是 `max|dx| > 2e5`，而 `vegwp` 量级是
        // 1e4~1e5 mm，于是 `max` 恒等于 `max|dx|`，缩放比成了 `max|dx|/2` ——
        // 比上游大 `max|dx|/max|x|` 倍，恰好在它要保护的场景里失效，而且
        // 四个节点一起跳、写回持久状态，误差会一路带下去。
        let mut change = change;
        let largest_change = change.iter().copied().map(f64::abs).fold(0.0, f64::max);
        if largest_change > 200_000.0 {
            let largest_potential = potential.iter().copied().map(f64::abs).fold(0.0, f64::max);
            let scale = largest_change.min(largest_potential) / 2.0;
            change = change.map(|value| scale * value / largest_change);
        }
        for (value, delta) in potential.iter_mut().zip(change) {
            *value += delta;
        }
        enforce_potential_gradient(&mut potential);
        let sunlit = sunlit_demand
            * vulnerability(
                potential[SUNLIT],
                input.sunlit_leaf_psi50_mm,
                input.vulnerability_shape,
            );
        let shaded = shaded_demand
            * vulnerability(
                potential[SHADED],
                input.shaded_leaf_psi50_mm,
                input.vulnerability_shape,
            );
        let (sunlit_gs, shaded_gs) = conductance_from_transpiration(
            input,
            boundary_conductance,
            input.maximum_sunlit_leaf_conductance_umol_m2_s,
            input.maximum_shaded_leaf_conductance_umol_m2_s,
            sunlit,
            shaded,
        );
        let sunlit_stress =
            (sunlit_gs / input.maximum_sunlit_leaf_conductance_umol_m2_s).max(MIN_STRESS);
        let shaded_stress =
            (shaded_gs / input.maximum_shaded_leaf_conductance_umol_m2_s).max(MIN_STRESS);
        // `MOD_PlantHydraulic.F90:355-359` 只有一次 `getrootqflx_qe2x`：它同时给出
        // 逐层根系水势与顶端水势。原先调了两遍（参数逐位相同，解算器确定性），
        // 第二遍只为拿 `.0` —— 纯重复。
        let (root_potential, root_top) = root_potential_from_flux(
            input,
            sunlit + shaded,
            &soil_root_conductance_mm_s,
            &axial_root_conductance_mm_s,
        )?;
        potential[ROOT] = root_top;
        let root_flux = input
            .soil_matric_potential_mm
            .iter()
            .zip(&root_potential)
            .zip(&soil_root_conductance_mm_s)
            .map(|((&soil, &root), &conductance)| conductance * (soil - root))
            .collect();
        (
            sunlit,
            shaded,
            sunlit_gs,
            shaded_gs,
            root_flux,
            sunlit_stress,
            shaded_stress,
        )
    } else {
        enforce_potential_gradient(&mut potential);
        (
            0.0,
            0.0,
            input.maximum_sunlit_leaf_conductance_umol_m2_s
                * vulnerability(
                    potential[SUNLIT],
                    input.sunlit_leaf_psi50_mm,
                    input.vulnerability_shape,
                )
                .max(MIN_STRESS),
            input.maximum_shaded_leaf_conductance_umol_m2_s
                * vulnerability(
                    potential[SHADED],
                    input.shaded_leaf_psi50_mm,
                    input.vulnerability_shape,
                )
                .max(MIN_STRESS),
            vec![0.0; layers],
            vulnerability(
                potential[SUNLIT],
                input.sunlit_leaf_psi50_mm,
                input.vulnerability_shape,
            )
            .max(MIN_STRESS),
            vulnerability(
                potential[SHADED],
                input.shaded_leaf_psi50_mm,
                input.vulnerability_shape,
            )
            .max(MIN_STRESS),
        )
    };
    ensure!(
        [
            sunlit_transpiration_kg_m2_s,
            shaded_transpiration_kg_m2_s,
            sunlit_stomatal_conductance_umol_m2_s,
            shaded_stomatal_conductance_umol_m2_s,
            sunlit_stress,
            shaded_stress,
        ]
        .iter()
        .all(|value| value.is_finite())
            && root_flux_kg_m2_s.iter().all(|value| value.is_finite()),
        "plant hydraulic solve produced non-finite state"
    );
    state.vegetation_water_potential_mm = potential;
    Ok(PlantHydraulicOutput {
        sunlit_stress,
        shaded_stress,
        sunlit_transpiration_kg_m2_s,
        shaded_transpiration_kg_m2_s,
        root_flux_kg_m2_s,
        soil_root_conductance_mm_s,
        axial_root_conductance_mm_s,
        sunlit_stomatal_conductance_umol_m2_s,
        shaded_stomatal_conductance_umol_m2_s,
    })
}

/// Ports the public `getvegwp_twoleaf` initialization/diagnostic branch.
///
/// Unlike [`plant_hydraulic_stress`], this follows the source's direct
/// conductance-to-potential calculation without a Newton stress update.
pub fn vegetation_water_potential(
    input: PlantHydraulicInput<'_>,
    sunlit_stress: f64,
    shaded_stress: f64,
) -> Result<(PlantHydraulicState, PlantHydraulicOutput)> {
    validate(
        input,
        PlantHydraulicState {
            vegetation_water_potential_mm: [-25_000.0; VEGETATION_SEGMENTS],
        },
    )?;
    ensure!(
        sunlit_stress.is_finite()
            && shaded_stress.is_finite()
            && sunlit_stress >= 0.0
            && shaded_stress >= 0.0,
        "plant-hydraulic stress factors must be finite and non-negative"
    );
    let (soil_root_conductance_mm_s, axial_root_conductance_mm_s) = root_conductances(input)?;
    let boundary_conductance = 1.0 / input.leaf_boundary_resistance_s_m
        * conductance_conversion(input.surface_pressure_pa, input.leaf_temperature_k);
    let (sunlit_transpiration_kg_m2_s, shaded_transpiration_kg_m2_s) =
        transpiration_from_conductance(
            input,
            boundary_conductance,
            input.maximum_sunlit_leaf_conductance_umol_m2_s,
            input.maximum_shaded_leaf_conductance_umol_m2_s,
            Some((sunlit_stress, shaded_stress)),
        );
    let (root_potential, root_top) = root_potential_from_flux(
        input,
        sunlit_transpiration_kg_m2_s + shaded_transpiration_kg_m2_s,
        &soil_root_conductance_mm_s,
        &axial_root_conductance_mm_s,
    )?;
    let root_scale = vulnerability(root_top, input.root_psi50_mm, input.vulnerability_shape);
    let xylem = root_top
        - input.canopy_top_height_m * 1000.0
        - (sunlit_transpiration_kg_m2_s + shaded_transpiration_kg_m2_s)
            / (root_scale * input.maximum_root_hydraulic_conductance / input.canopy_top_height_m
                * input.stem_area_index);
    let xylem_scale = vulnerability(xylem, input.xylem_psi50_mm, input.vulnerability_shape);
    let shaded = xylem
        - shaded_transpiration_kg_m2_s
            / (xylem_scale
                * input.maximum_xylem_hydraulic_conductance
                * input.shaded_leaf_area_index);
    let sunlit = xylem
        - sunlit_transpiration_kg_m2_s
            / (xylem_scale
                * input.maximum_xylem_hydraulic_conductance
                * input.sunlit_leaf_area_index);
    let root_flux_kg_m2_s = input
        .soil_matric_potential_mm
        .iter()
        .zip(&root_potential)
        .zip(&soil_root_conductance_mm_s)
        .map(|((&soil, &root), &conductance)| conductance * (soil - root))
        .collect();
    Ok((
        PlantHydraulicState {
            vegetation_water_potential_mm: [sunlit, shaded, xylem, root_top],
        },
        PlantHydraulicOutput {
            sunlit_stress,
            shaded_stress,
            sunlit_transpiration_kg_m2_s,
            shaded_transpiration_kg_m2_s,
            root_flux_kg_m2_s,
            soil_root_conductance_mm_s,
            axial_root_conductance_mm_s,
            sunlit_stomatal_conductance_umol_m2_s: input.maximum_sunlit_leaf_conductance_umol_m2_s,
            shaded_stomatal_conductance_umol_m2_s: input.maximum_shaded_leaf_conductance_umol_m2_s,
        },
    ))
}

fn root_conductances(input: PlantHydraulicInput<'_>) -> Result<(Vec<f64>, Vec<f64>)> {
    let mut soil_root = Vec::with_capacity(input.node_depth_m.len());
    let mut axial_root = Vec::with_capacity(input.node_depth_m.len());
    for layer in 0..input.node_depth_m.len() {
        let root_biomass_density =
            (2.0 * input.parameters.fine_root_carbon_g_c_m2 * input.root_fraction[layer]
                / input.layer_thickness_m[layer])
                .max(2.0);
        let root_cross_section_m2 =
            PLANT_HYDRAULIC_PI * input.parameters.fine_root_radius_m.powi(2);
        let root_length_density_m_m3 = root_biomass_density
            / (input.parameters.root_tissue_density_g_m3 * root_cross_section_m2);
        let root_area_index =
            (input.stem_area_index + input.sunlit_leaf_area_index + input.shaded_leaf_area_index)
                * input.parameters.fine_root_to_leaf_area
                * input.root_fraction[layer];
        let root_spacing_m = (1.0 / (PLANT_HYDRAULIC_PI * root_length_density_m_m3)).sqrt();
        let soil_conductance = input.saturated_hydraulic_conductivity_mm_s[layer]
            .min(input.soil_hydraulic_conductivity_mm_s[layer])
            / (1000.0 * root_spacing_m);
        let root_scale = vulnerability(
            input.soil_matric_potential_mm[layer].max(-1.0),
            input.root_psi50_mm,
            input.vulnerability_shape,
        );
        let root_conductance =
            root_scale * root_area_index * input.parameters.maximum_radial_root_conductance
                / (input.parameters.coarse_root_lateral_length_m + input.node_depth_m[layer]);
        let resistance = 1.0 / soil_conductance.max(MIN_CONDUCTANCE)
            + 1.0 / root_conductance.max(MIN_CONDUCTANCE);
        soil_root.push(if root_area_index * input.root_fraction[layer] > 0.0 {
            1.0 / resistance
        } else {
            0.0
        });
        axial_root.push(
            input.root_fraction[layer] / (input.layer_thickness_m[layer] * 1000.0)
                * input.parameters.axial_root_conductivity
                * 0.6,
        );
    }
    Ok((soil_root, axial_root))
}

fn transpiration_from_conductance(
    input: PlantHydraulicInput<'_>,
    boundary_conductance_umol_m2_s: f64,
    sunlit_stomatal_conductance_umol_m2_s: f64,
    shaded_stomatal_conductance_umol_m2_s: f64,
    stress: Option<(f64, f64)>,
) -> (f64, f64) {
    let conversion = conductance_conversion(input.surface_pressure_pa, input.leaf_temperature_k);
    let delta =
        f64::from(input.leaf_saturation_specific_humidity > input.canopy_air_specific_humidity);
    let air = 1.0 / input.reference_to_canopy_moisture_resistance_s_m;
    let ground = if input.ground_specific_humidity < input.canopy_air_specific_humidity {
        1.0 / input.ground_to_canopy_moisture_resistance_s_m
    } else if input.soil_surface_resistance_scheme == 4 {
        input.soil_surface_resistance_s_m / input.ground_to_canopy_moisture_resistance_s_m
    } else {
        1.0 / (input.ground_to_canopy_moisture_resistance_s_m + input.soil_surface_resistance_s_m)
    };
    // `1. - delta*(1.-fwet)`：乘积被吸收成 `FNMA(1-fwet, delta, 1.0)`
    // （`-fdump-tree-all` 实测；`1. - delta*(1.-fwet)` 不是 `1. - delta` 那种单乘加，
    // 但 gfortran 照样把 `delta*(1-fwet)` 收进减法里）。两处（`cfw` 与 `cwet`）同型。
    // `:779 cwet = (1.-delta*(1.-fwet))*…`：出货汇编在 779/780 上没有 FMA（只有
    // `fmul`/`fsub`/`fdiv`），所以这里**不收缩**（第 341 轮更正）。
    let dry_fraction = 1.0 - delta * (1.0 - input.wet_canopy_fraction);
    let leaf = dry_fraction
        * (input.sunlit_leaf_area_index + input.shaded_leaf_area_index + input.stem_area_index)
        * boundary_conductance_umol_m2_s
        / conversion
        + (1.0 - input.wet_canopy_fraction)
            * delta
            * (input.sunlit_leaf_area_index
                / (1.0 / boundary_conductance_umol_m2_s
                    + 1.0 / sunlit_stomatal_conductance_umol_m2_s)
                / conversion
                + input.shaded_leaf_area_index
                    / (1.0 / boundary_conductance_umol_m2_s
                        + 1.0 / shaded_stomatal_conductance_umol_m2_s)
                    / conversion);
    // 上游 `MOD_PlantHydraulic.F90:683-685` 是 `wtsqi = 1./(caw+cgw+cfw)` 再
    // `wtaq0 = caw*wtsqi`（先取倒数再乘），不是 `caw/total`。差 1 ULP，
    // 一样能翻离散判据（见 `boundary_conductance` 那处的说明）。
    let inverse_total = 1.0 / (air + ground + leaf);
    let air_weight = air * inverse_total;
    let ground_weight = ground * inverse_total;
    // `cqi = (wtaq0 + wtgq0)*qsatl - wtaq0*qm - wtgq0*qg`（`:685`）：
    // 与叶温内核里 `humidity_gradient` 同一个三级减法链，实测收缩成
    // `fma(-wtgq0, qg, fma(wtaq0+wtgq0, qsatl, -(wtaq0*qm)))`（4000/4000）。
    let driving_humidity = (-ground_weight).mul_add(
        input.ground_specific_humidity,
        (air_weight + ground_weight).mul_add(
            input.leaf_saturation_specific_humidity,
            -(air_weight * input.reference_specific_humidity),
        ),
    );
    let sunlit = input.air_density_kg_m3
        * (1.0 - input.wet_canopy_fraction)
        * delta
        * input.sunlit_leaf_area_index
        / (1.0 / boundary_conductance_umol_m2_s + 1.0 / sunlit_stomatal_conductance_umol_m2_s)
        / conversion
        * driving_humidity;
    let shaded = input.air_density_kg_m3
        * (1.0 - input.wet_canopy_fraction)
        * delta
        * input.shaded_leaf_area_index
        / (1.0 / boundary_conductance_umol_m2_s + 1.0 / shaded_stomatal_conductance_umol_m2_s)
        / conversion
        * driving_humidity;
    match stress {
        Some((sunlit_stress, shaded_stress)) => (
            if sunlit_stress <= MIN_STRESS {
                0.0
            } else {
                sunlit
            },
            if shaded_stress <= MIN_STRESS {
                0.0
            } else {
                shaded
            },
        ),
        None => (sunlit, shaded),
    }
}

fn conductance_from_transpiration(
    input: PlantHydraulicInput<'_>,
    boundary_conductance_umol_m2_s: f64,
    mut sunlit_stomatal_conductance_umol_m2_s: f64,
    mut shaded_stomatal_conductance_umol_m2_s: f64,
    sunlit_transpiration_kg_m2_s: f64,
    shaded_transpiration_kg_m2_s: f64,
) -> (f64, f64) {
    if sunlit_transpiration_kg_m2_s <= 0.0 && shaded_transpiration_kg_m2_s <= 0.0 {
        return (
            sunlit_stomatal_conductance_umol_m2_s,
            shaded_stomatal_conductance_umol_m2_s,
        );
    }
    let conversion = conductance_conversion(input.surface_pressure_pa, input.leaf_temperature_k);
    let delta =
        f64::from(input.leaf_saturation_specific_humidity > input.canopy_air_specific_humidity);
    let air = 1.0 / input.reference_to_canopy_moisture_resistance_s_m;
    let ground = if input.ground_specific_humidity < input.canopy_air_specific_humidity {
        1.0 / input.ground_to_canopy_moisture_resistance_s_m
    } else if input.soil_surface_resistance_scheme == 4 {
        input.soil_surface_resistance_s_m / input.ground_to_canopy_moisture_resistance_s_m
    } else {
        1.0 / (input.ground_to_canopy_moisture_resistance_s_m + input.soil_surface_resistance_s_m)
    };
    // `:779 cwet = (1.-delta*(1.-fwet))*…`：出货汇编在 779/780 上没有 FMA（只有
    // `fmul`/`fsub`/`fdiv`），所以这里**不收缩**（第 341 轮更正）。
    let dry_fraction = 1.0 - delta * (1.0 - input.wet_canopy_fraction);
    let wet = dry_fraction
        * (input.sunlit_leaf_area_index + input.shaded_leaf_area_index + input.stem_area_index)
        * boundary_conductance_umol_m2_s
        / conversion;
    // `cqi_leaf = caw*(qsatl-qm) + cgw*(qsatl-qg)`：乘积 + 乘积，收左边（GIMPLE 实测）。
    let leaf = air.mul_add(
        input.leaf_saturation_specific_humidity - input.reference_specific_humidity,
        ground * (input.leaf_saturation_specific_humidity - input.ground_specific_humidity),
    );
    let a1 = leaf - sunlit_transpiration_kg_m2_s / input.air_density_kg_m3;
    let b1 = -sunlit_transpiration_kg_m2_s / input.air_density_kg_m3;
    let c1 = sunlit_transpiration_kg_m2_s * (air + ground + wet) / input.air_density_kg_m3;
    let a2 = -shaded_transpiration_kg_m2_s / input.air_density_kg_m3;
    let b2 = leaf - shaded_transpiration_kg_m2_s / input.air_density_kg_m3;
    let c2 = shaded_transpiration_kg_m2_s * (air + ground + wet) / input.air_density_kg_m3;
    // `:794/:795` 的两条 FMA 都在**分子**上 —— GIMPLE 是
    // `_51 = .FNMS(_44, c2, _50)`（`_50 = c1*b2`）与 `_59 = .FMS(a1, c2, _58)`（`_58 = c1*a2`），
    // 而两个分母是 `_55 = _53 - _54`、`_61 = _54 - _53`，即**两个已舍入乘积的普通减法**，
    // **不收缩**。所以分母要把 `b2*a1` 抽出来复用、并且两边都留成普通减法。
    // （第 341 轮更正：原先分母也写了 `mul_add`，那是多收了一处。）
    let b1_a2 = b1 * a2;
    let b2_a1 = b2 * a1;
    let sunlit_leaf_conductance = b1.mul_add(c2, -(b2 * c1)) / (b1_a2 - b2_a1);
    let shaded_leaf_conductance = c2.mul_add(a1, -(c1 * a2)) / (b2_a1 - b1_a2);
    if sunlit_transpiration_kg_m2_s > 0.0 {
        sunlit_stomatal_conductance_umol_m2_s = 1.0
            / ((1.0 - input.wet_canopy_fraction) * delta * input.sunlit_leaf_area_index
                / sunlit_leaf_conductance
                / conversion
                - 1.0 / boundary_conductance_umol_m2_s);
    }
    if shaded_transpiration_kg_m2_s > 0.0 {
        shaded_stomatal_conductance_umol_m2_s = 1.0
            / ((1.0 - input.wet_canopy_fraction) * delta * input.shaded_leaf_area_index
                / shaded_leaf_conductance
                / conversion
                - 1.0 / boundary_conductance_umol_m2_s);
    }
    (
        sunlit_stomatal_conductance_umol_m2_s,
        shaded_stomatal_conductance_umol_m2_s,
    )
}

fn root_flux_from_top_potential(
    input: PlantHydraulicInput<'_>,
    root_top_mm: f64,
    radial: &[f64],
    axial: &[f64],
) -> Result<(f64, f64)> {
    let depth_mm = input
        .node_depth_m
        .iter()
        .map(|value| value * 1000.0)
        .collect::<Vec<_>>();
    let layers = depth_mm.len();
    let mut sub = vec![0.0; layers - 1];
    let mut diagonal = vec![0.0; layers - 1];
    let mut super_ = vec![0.0; layers - 1];
    let mut rhs = vec![0.0; layers - 1];
    let mut derivative_rhs = vec![0.0; layers - 1];
    let root_one = root_top_mm + depth_mm[0];
    for layer in 1..layers {
        let row = layer - 1;
        let previous_distance = depth_mm[layer] - depth_mm[layer - 1];
        let next_distance = (layer + 1 < layers).then(|| depth_mm[layer + 1] - depth_mm[layer]);
        sub[row] = if layer == 1 {
            0.0
        } else {
            -axial[layer - 1] / previous_distance
        };
        diagonal[row] = axial[layer - 1] / previous_distance
            + next_distance.map_or(0.0, |distance| axial[layer] / distance)
            + radial[layer];
        super_[row] = next_distance.map_or(0.0, |distance| -axial[layer] / distance);
        // `rmx_hr(j-1) = krad(j)*smp(j) + kax(j-1) - kax(j)`；左边那个乘积被
        // gfortran 吸收（实测 `fma(p,A,B) - C` 4000/4000，改写成 `p*A+(B-C)`
        // 只有 2662/4000）。`rmx` 是**第一次** `tridia` 的右端：它差 1 ULP，
        // `x` 就差，而 `dqeroot`（第二次解的右端 `drmx_hr` 与它无关）可以仍然对上 ——
        // 实测就是先看到 `qeroot` 差、`dqeroot` 相同。
        rhs[row] = radial[layer].mul_add(input.soil_matric_potential_mm[layer], axial[layer - 1])
            - next_distance.map_or(0.0, |_| axial[layer]);
        if layer == 1 {
            // `… + kax(j-1)/den1*xroot(1)`：`X + 乘积` 形状，收的是外层那个乘积
            // （`fma(q,R,acc)` 4000/4000，不收缩 2935/4000）。
            rhs[row] = (axial[0] / previous_distance).mul_add(root_one, rhs[row]);
            derivative_rhs[row] = axial[0] / previous_distance;
        }
    }
    let tail = solve_tridiagonal(&sub, &diagonal, &super_, &rhs)
        .map_err(anyhow::Error::msg)
        .context("plant-hydraulic root-potential solve failed")?;
    let derivative_tail = solve_tridiagonal(&sub, &diagonal, &super_, &derivative_rhs)
        .map_err(anyhow::Error::msg)
        .context("plant-hydraulic root-potential derivative solve failed")?;
    let root_two = tail[0];
    // `:892 qeroot = krad*(smp(1)-xroot(1)) + (xroot(2)-xroot(1))*kax/den2 - kax`：
    // 出货汇编 `fmul d0,d0,d13; fdiv d0,d0,d15; fsub d31,d31,d14; fmadd d0,d26,d31,d0`
    // ⇒ **第一个**源乘积 `krad*(smp-xroot(1))` 进 FMA，第二个是独立舍入的加数，
    // 末尾 `- kax` 仍是普通 fsub。（第 341 轮补上；原先两个乘积都独立舍入。）
    let root_flux = radial[0].mul_add(
        input.soil_matric_potential_mm[0] - root_one,
        (root_two - root_one) * axial[0] / (depth_mm[1] - depth_mm[0]),
    ) - axial[0];
    let slope = -radial[0] + (derivative_tail[0] - 1.0) * axial[0] / (depth_mm[1] - depth_mm[0]);
    Ok((root_flux, slope))
}

fn root_potential_from_flux(
    input: PlantHydraulicInput<'_>,
    root_flux_kg_m2_s: f64,
    radial: &[f64],
    axial: &[f64],
) -> Result<(Vec<f64>, f64)> {
    let depth_mm = input
        .node_depth_m
        .iter()
        .map(|value| value * 1000.0)
        .collect::<Vec<_>>();
    let layers = depth_mm.len();
    let mut sub = vec![0.0; layers];
    let mut diagonal = vec![0.0; layers];
    let mut super_ = vec![0.0; layers];
    let mut rhs = vec![0.0; layers];
    for layer in 0..layers {
        let previous_distance = (layer > 0).then(|| depth_mm[layer] - depth_mm[layer - 1]);
        let next_distance = (layer + 1 < layers).then(|| depth_mm[layer + 1] - depth_mm[layer]);
        sub[layer] = previous_distance.map_or(0.0, |distance| -axial[layer - 1] / distance);
        diagonal[layer] = previous_distance.map_or(0.0, |distance| axial[layer - 1] / distance)
            + next_distance.map_or(0.0, |distance| axial[layer] / distance)
            + radial[layer];
        super_[layer] = next_distance.map_or(0.0, |distance| -axial[layer] / distance);
        // `getrootqflx_qe2x` 的右端三行分别是
        // `krad*smp - qeroot - kax(j)`、`krad*smp + kax(j-1) - kax(j)`、
        // `krad*smp + kax(j-1)`（`MOD_PlantHydraulic.F90:940-960`）。
        // 所以首行是**先减 `qeroot` 再减 `kax`** —— 原先先减 `kax`、
        // 出了循环再减 `qeroot`，结合顺序与上游不同；同时那个乘积也要收缩。
        rhs[layer] = radial[layer].mul_add(
            input.soil_matric_potential_mm[layer],
            previous_distance.map_or(-root_flux_kg_m2_s, |_| axial[layer - 1]),
        ) - next_distance.map_or(0.0, |_| axial[layer]);
    }
    let potential = solve_tridiagonal(&sub, &diagonal, &super_, &rhs)
        .map_err(anyhow::Error::msg)
        .context("plant-hydraulic root-flux solve failed")?;
    Ok((potential.clone(), potential[0] - depth_mm[0]))
}

fn spac_change(
    x: [f64; VEGETATION_SEGMENTS],
    sunlit_flux: f64,
    shaded_flux: f64,
    input: PlantHydraulicInput<'_>,
    root_flux: f64,
    root_flux_slope: f64,
) -> [f64; VEGETATION_SEGMENTS] {
    let fsun = vulnerability(
        x[SUNLIT],
        input.sunlit_leaf_psi50_mm,
        input.vulnerability_shape,
    );
    let fsha = vulnerability(
        x[SHADED],
        input.shaded_leaf_psi50_mm,
        input.vulnerability_shape,
    );
    let fxyl = vulnerability(x[XYLEM], input.xylem_psi50_mm, input.vulnerability_shape);
    let froot = vulnerability(x[ROOT], input.root_psi50_mm, input.vulnerability_shape);
    let dfsun = vulnerability_derivative(
        x[SUNLIT],
        input.sunlit_leaf_psi50_mm,
        input.vulnerability_shape,
    );
    let dfsha = vulnerability_derivative(
        x[SHADED],
        input.shaded_leaf_psi50_mm,
        input.vulnerability_shape,
    );
    let dfxyl = vulnerability_derivative(x[XYLEM], input.xylem_psi50_mm, input.vulnerability_shape);
    let dfroot = vulnerability_derivative(x[ROOT], input.root_psi50_mm, input.vulnerability_shape);
    let xylem = input.stem_area_index * input.maximum_xylem_hydraulic_conductance
        / input.canopy_top_height_m;
    let gravity = input.canopy_top_height_m * 1000.0;
    // 下面四个矩阵元的**分组**必须与上游逐字一致（`MOD_PlantHydraulic.F90:472-489`）。
    // 上游写的是 `A13 = laisun*kmax_sun*dfx*(x(xyl)-x(leafsun)) + laisun*kmax_sun*fx`，
    // 本仓库原先提成 `laisun*kmax_sun*(dfx*Δ + fx)` —— 代数等价、**舍入不等价**：
    // 上游先把两个乘积各自算出来再相加，提公因式之后多了一次"先加后乘"。
    // 实测：这一步让第 0 次 PHS 调用的 `dx` 差 1 ULP（输入已逐位相同）。
    let sunlit_conductance =
        input.sunlit_leaf_area_index * input.maximum_sunlit_leaf_hydraulic_conductance;
    let shaded_conductance =
        input.shaded_leaf_area_index * input.maximum_shaded_leaf_hydraulic_conductance;
    let sunlit_gradient = x[XYLEM] - x[SUNLIT];
    let shaded_gradient = x[XYLEM] - x[SHADED];
    let root_gradient = x[ROOT] - x[XYLEM] - gravity;
    // `A`/`f` 的收缩点**直接读 GIMPLE 定下来的**（`gfortran -fdump-tree-all`，
    // 见 docs 里那一节的说明）。逐条对应：
    // * `A11 = FNMS(qflx_sun, dfsto1, P)` —— 左边是 `-P`（NEG 节点）不是乘积，
    //   所以收的是右边那个乘积；
    // * `A13 = FMA(laisun*kmax_sun*dfx, Δsun, P)` / `A23` 同型（收左边）；
    // * `A33` 三级：`FNMS(…, Δsun, -P)` → `FNMA(…, Δsha, ·)` → 再两次普通减法；
    // * `A34 = FMA(xylem*dfr, Δroot, X)`、`A44 = FNMS(xylem*dfr, Δroot, X) + dqeroot`；
    // * `f(leafsun) = FMS(qflx_sun, fsto1, P*Δsun)`、`f(xyl) = FNMA(X, Δroot, P*Δsun+PS*Δsha)`、
    //   `f(root) = FMS(X, Δroot, qeroot)`。
    // 也就是"每层收那个乘积操作数；两边都是乘积时收左边"。被收的那个乘法取的是
    // **它自己的最外层乘法**（如 `laisun*kmax_sun*dfx` 收成 `fma(laisun*kmax_sun, dfx, ·)`
    // 的乘数一侧），没收的那侧按源码顺序整项舍入。
    let a11 = (-sunlit_flux).mul_add(dfsun, -(sunlit_conductance * fxyl));
    let a13 = (sunlit_conductance * dfxyl).mul_add(sunlit_gradient, sunlit_conductance * fxyl);
    let a22 = (-shaded_flux).mul_add(dfsha, -(shaded_conductance * fxyl));
    let a23 = (shaded_conductance * dfxyl).mul_add(shaded_gradient, shaded_conductance * fxyl);
    let a31 = sunlit_conductance * fxyl;
    let a32 = shaded_conductance * fxyl;
    let mut a33 = (-(shaded_conductance * dfxyl)).mul_add(
        shaded_gradient,
        (-(sunlit_conductance * dfxyl)).mul_add(sunlit_gradient, -(sunlit_conductance * fxyl)),
    ) - shaded_conductance * fxyl
        - xylem * froot;
    let a34 = (xylem * dfroot).mul_add(root_gradient, xylem * froot);
    let a43 = xylem * froot;
    let a44 = (-(xylem * dfroot)).mul_add(root_gradient, -(xylem * froot)) + root_flux_slope;
    let mut f = [0.0; VEGETATION_SEGMENTS];
    // 三个乘积（真实内核 dump 里的 `_51`/`_54`/`_57`）同时喂给下面 IF 与 ELSE
    // **两条支路**（`f(xyl)` 的两支、`f(root)`），所以 GCC 一律把它们保持成
    // **已经舍入**的临时量，任何一处都不吸收。表面上看 `f(xyl)`、`f(root)` 都是
    // "乘积进加减"，按形状规则该写 `mul_add` —— 那样每条都会差 1 ULP，
    // 也正是此前那 4.1e-25 残差的来源。`f(SUNLIT)`/`f(SHADED)` 收的是**另一个**
    // 乘积（`qflx*fsto` 那一个，只出现一次），所以那两条仍然收缩。
    let sunlit_term = sunlit_conductance * fxyl * sunlit_gradient;
    let shaded_term = shaded_conductance * fxyl * shaded_gradient;
    let root_term = xylem * froot * root_gradient;
    f[SUNLIT] = sunlit_flux.mul_add(fsun, -sunlit_term);
    f[SHADED] = shaded_flux.mul_add(fsha, -shaded_term);
    f[XYLEM] = sunlit_term + shaded_term - root_term;
    f[ROOT] = root_term - root_flux;
    let mut change = [0.0; VEGETATION_SEGMENTS];
    if shaded_flux > 0.0 {
        // `determ = A44*A22*A33*A11 - A44*A22*A31*A13 - A44*A32*A23*A11 - A43*A11*A22*A34`
        // 的收缩点读的是**真实内核**的 dump（`MOD_PlantHydraulic.F90` 本体，不是复刻件）：
        // `FMS(a22*a44*a33, a11, a31*a22*a44*a13)` → `FNMA(a32*a44*a23, a11, ·)`
        // → `FNMA(a43*a11*a22, a34, ·)`，每一级收的都是**本级最后一个乘积**。
        //
        // 这条以前漏验了：当时是拿"神谕算好的 determ"喂进去只比 `dx`，所以四条 `dx`
        // 式子验到 20000/20000，而 `determ` 自己的算法没人管。现在用内核自己打出来的
        // 49 组 `(A, f, dx)` 反推：融合式四个分量**全 49/49**，普通乘减只有 27/49。
        let determinant = ((a22 * a44) * a33).mul_add(a11, -(((a22 * a44) * a31) * a13));
        let determinant = (-((a32 * a44) * a23)).mul_add(a11, determinant);
        let determinant = (-((a43 * a11) * a22)).mul_add(a34, determinant);
        if determinant != 0.0 {
            // `spacAF_twoleaf` 的四条 `dx` 回代式（`MOD_PlantHydraulic.F90:499-509`）。
            // **收缩规则是量出来的，不是猜的**：把上游那四条语句原样抄成独立 Fortran
            // 程序（`/tmp/gf/spac_replica.f90` 那一份逻辑），先用内核自己打出来的
            // 49 组 (A11..A44, f, determ, dx) 验证复刻件 49/49，再拿它当"神谕"跑
            // 20000 组随机输入，与 72 种候选嵌套逐一比对。唯一全中的是：
            //
            // * 内层 `P1 - P2 - P3`（P 都是三因子乘积）：第一级收**左**边那个乘积的
            //   最外层乘法（`fma(a22*a33, a44, -P2)`），第二级收右边的
            //   （`fma(-a44, a23*a32, acc)`）；
            // * 外层四项链：第一级同样收左边，之后每一级只有右边是乘积、于是收右边；
            // * **没被收的那个操作数按源码顺序整项舍入**（例如 `a13*a32*a44*f2` 是
            //   `((a13*a32)*a44)*f2`，不能拆成 `(a13*a32)*(a44*f2)`）。
            //
            // 20000/20000 组（80000/80000 个分量）与 49/49 个真实调用全中；
            // 原先的"全不收缩"写法在这 20000 组里 0 组全中。
            // 交叉验证：`gfortran -fdump-tree-all` 的 GIMPLE 里能看到 `.FMA/.FMS/.FNMA`
            // 恰好落在上面这些位置，与实测一致。
            let e1 = (a22 * a33).mul_add(a44, -(a22 * a34 * a43));
            let e1 = (-a44).mul_add(a23 * a32, e1);
            let e2 = (a11 * a33).mul_add(a44, -(a11 * a34 * a43));
            let e2 = (-a44).mul_add(a13 * a31, e2);
            let e3 = (a11 * a22).mul_add(a33, -(a11 * a23 * a32));
            let e3 = (-(a13 * a22)).mul_add(a31, e3);
            change[SUNLIT] = (a13 * a22 * a34).mul_add(
                f[ROOT],
                (-(a13 * a22 * a44))
                    .mul_add(f[XYLEM], e1.mul_add(f[SUNLIT], a13 * a32 * a44 * f[SHADED])),
            ) / determinant;
            change[SHADED] = (a11 * a23 * a34).mul_add(
                f[ROOT],
                (-(a11 * a23 * a44)).mul_add(
                    f[XYLEM],
                    (a23 * a31 * a44).mul_add(f[SUNLIT], e2 * f[SHADED]),
                ),
            ) / determinant;
            change[XYLEM] = (-(a11 * a22 * a34)).mul_add(
                f[ROOT],
                (a11 * a22 * a44).mul_add(
                    f[XYLEM],
                    (-(a22 * a31 * a44)).mul_add(f[SUNLIT], -(a11 * a32 * a44 * f[SHADED])),
                ),
            ) / determinant;
            change[ROOT] = e3.mul_add(
                f[ROOT],
                (-(a11 * a22 * a43)).mul_add(
                    f[XYLEM],
                    (a22 * a31 * a43).mul_add(f[SUNLIT], a11 * a32 * a43 * f[SHADED]),
                ),
            ) / determinant;
        }
    } else {
        // `qflx_sha <= 0` 那条分支。**干窗 49 次调用一次都没走到** —— 但写法不能靠猜：
        // 同样把那六条语句抄成独立程序读 GIMPLE（`/tmp/gf/r96/spacaf_else.f90`），
        // 收缩点与 IF 分支同一套规则，逐个对应：
        //   `A33 = FNMS(LKdfx, Δsun, P) - X`
        //   `f(xyl) = FMS(Δsun, P, X*Δroot)`
        //   `determ = FNMA(a44, a13*a31, FMS(a33*a11, a44, (a11*a34)*a43))`
        //   `dx(leafsun)` 的两项：`FMS(a34*a13, f(root), f(xyl)*(a44*a13))`
        //                    再 `FMA((a33*a44 - a34*a43), f(leafsun), ·)`
        //   `dx(xyl)`：`FNMA(f(leafsun), a44*a31, FMS(f(xyl), a11*a44, (a11*a34)*f(root)))`
        //   `dx(root)`：`FMA(f(leafsun), a43*a31,
        //                    FMS((a11*a33 - a13*a31), f(root), f(xyl)*(a11*a43)))`
        a33 = (-(sunlit_conductance * dfxyl))
            .mul_add(sunlit_gradient, -(sunlit_conductance * fxyl))
            - xylem * froot;
        f[XYLEM] = sunlit_term - root_term;
        let determinant = (a11 * a33).mul_add(a44, -((a11 * a34) * a43));
        let determinant = (-a44).mul_add(a13 * a31, determinant);
        if determinant != 0.0 {
            change[SUNLIT] = a33.mul_add(a44, -(a34 * a43)).mul_add(
                f[SUNLIT],
                (a13 * a34).mul_add(f[ROOT], -(f[XYLEM] * (a13 * a44))),
            ) / determinant;
            change[XYLEM] = (-(a44 * a31)).mul_add(
                f[SUNLIT],
                f[XYLEM].mul_add(a11 * a44, -((a11 * a34) * f[ROOT])),
            ) / determinant;
            change[ROOT] = (a43 * a31).mul_add(
                f[SUNLIT],
                (a11 * a33 - a13 * a31).mul_add(f[ROOT], -(f[XYLEM] * (a11 * a43))),
            ) / determinant;
            change[SHADED] = x[SUNLIT] - x[SHADED] + change[SUNLIT];
        }
    }
    change
}

fn enforce_potential_gradient(potential: &mut [f64; VEGETATION_SEGMENTS]) {
    if potential[XYLEM] > potential[ROOT] {
        potential[XYLEM] = potential[ROOT];
    }
    if potential[SUNLIT] > potential[XYLEM] {
        potential[SUNLIT] = potential[XYLEM];
    }
    if potential[SHADED] > potential[XYLEM] {
        potential[SHADED] = potential[XYLEM];
    }
}

fn conductance_conversion(surface_pressure_pa: f64, leaf_temperature_k: f64) -> f64 {
    44.6 * 273.16 * surface_pressure_pa / 1.013e5 / leaf_temperature_k * 1.0e6
}

/// CoLM's Weibull vulnerability curve (`plc`).
pub fn vulnerability(potential_mm: f64, psi50_mm: f64, shape: f64) -> f64 {
    let exponent = (-(potential_mm / psi50_mm).powf(shape)).max(-500.0);
    2.0_f64.powf(exponent).max(1.0e-5)
}

/// First derivative of [`vulnerability`] (`d1plc`).
pub fn vulnerability_derivative(potential_mm: f64, psi50_mm: f64, shape: f64) -> f64 {
    let exponent = (-(potential_mm / psi50_mm).powf(shape)).max(-500.0);
    shape * 2.0_f64.ln() * 2.0_f64.powf(exponent) * exponent / potential_mm
}

fn validate(input: PlantHydraulicInput<'_>, state: PlantHydraulicState) -> Result<usize> {
    let layers = input.node_depth_m.len();
    ensure!(
        layers >= 3,
        "plant hydraulics needs three or more soil layers"
    );
    for values in [
        input.layer_thickness_m,
        input.root_fraction,
        input.soil_matric_potential_mm,
        input.soil_hydraulic_conductivity_mm_s,
        input.saturated_hydraulic_conductivity_mm_s,
    ] {
        ensure!(
            values.len() == layers && values.iter().all(|value| value.is_finite()),
            "plant-hydraulic soil arrays must be finite and equal in length"
        );
    }
    ensure!(
        input.layer_thickness_m.iter().all(|value| *value > 0.0)
            && input.node_depth_m.windows(2).all(|pair| pair[1] > pair[0])
            && input.root_fraction.iter().all(|value| *value >= 0.0)
            && input
                .soil_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0)
            && input
                .saturated_hydraulic_conductivity_mm_s
                .iter()
                .all(|value| *value >= 0.0)
            && state
                .vegetation_water_potential_mm
                .iter()
                .all(|value| value.is_finite() && *value < 0.0),
        "plant-hydraulic soil or vegetation state is invalid"
    );
    for value in [
        input.surface_pressure_pa,
        input.leaf_saturation_specific_humidity,
        input.canopy_air_specific_humidity,
        input.ground_specific_humidity,
        input.reference_specific_humidity,
        input.leaf_temperature_k,
        input.leaf_boundary_resistance_s_m,
        input.soil_surface_resistance_s_m,
        input.reference_to_canopy_moisture_resistance_s_m,
        input.ground_to_canopy_moisture_resistance_s_m,
        input.air_density_kg_m3,
        input.wet_canopy_fraction,
        input.sunlit_leaf_area_index,
        input.shaded_leaf_area_index,
        input.stem_area_index,
        input.canopy_top_height_m,
        input.maximum_sunlit_leaf_conductance_umol_m2_s,
        input.maximum_shaded_leaf_conductance_umol_m2_s,
        input.maximum_sunlit_leaf_hydraulic_conductance,
        input.maximum_shaded_leaf_hydraulic_conductance,
        input.maximum_xylem_hydraulic_conductance,
        input.maximum_root_hydraulic_conductance,
        input.sunlit_leaf_psi50_mm,
        input.shaded_leaf_psi50_mm,
        input.xylem_psi50_mm,
        input.root_psi50_mm,
        input.vulnerability_shape,
        input.parameters.coarse_root_lateral_length_m,
        input.parameters.axial_root_conductivity,
        input.parameters.fine_root_carbon_g_c_m2,
        input.parameters.fine_root_radius_m,
        input.parameters.root_tissue_density_g_m3,
        input.parameters.fine_root_to_leaf_area,
        input.parameters.maximum_radial_root_conductance,
    ] {
        ensure!(value.is_finite(), "plant-hydraulic inputs must be finite");
    }
    ensure!(
        input.surface_pressure_pa > 0.0
            && input.leaf_temperature_k > 0.0
            && input.leaf_boundary_resistance_s_m > 0.0
            && input.reference_to_canopy_moisture_resistance_s_m > 0.0
            && input.ground_to_canopy_moisture_resistance_s_m > 0.0
            && input.air_density_kg_m3 > 0.0
            && (0.0..=1.0).contains(&input.wet_canopy_fraction)
            && input.sunlit_leaf_area_index > 0.0
            && input.shaded_leaf_area_index > 0.0
            && input.stem_area_index > 0.0
            && input.canopy_top_height_m > 0.0
            && input.maximum_sunlit_leaf_conductance_umol_m2_s > 0.0
            && input.maximum_shaded_leaf_conductance_umol_m2_s > 0.0
            && input.maximum_sunlit_leaf_hydraulic_conductance > 0.0
            && input.maximum_shaded_leaf_hydraulic_conductance > 0.0
            && input.maximum_xylem_hydraulic_conductance > 0.0
            && input.maximum_root_hydraulic_conductance > 0.0
            && input.sunlit_leaf_psi50_mm < 0.0
            && input.shaded_leaf_psi50_mm < 0.0
            && input.xylem_psi50_mm < 0.0
            && input.root_psi50_mm < 0.0
            && input.vulnerability_shape > 0.0
            && input.parameters.coarse_root_lateral_length_m > 0.0
            && input.parameters.axial_root_conductivity > 0.0
            && input.parameters.fine_root_carbon_g_c_m2 > 0.0
            && input.parameters.fine_root_radius_m > 0.0
            && input.parameters.root_tissue_density_g_m3 > 0.0
            && input.parameters.fine_root_to_leaf_area > 0.0
            && input.parameters.maximum_radial_root_conductance > 0.0
            // `DEF_RSS_SCHEME` 的 0 是**合法值**：`MOD_Namelist.F90:1947-1951` 在
            // 关掉 Campbell 土壤模型时把它置 0，而 `MOD_PlantHydraulic.F90:674-679`
            // 的 `ELSE` 把 0 与 1 同等对待（只有 4 走导通支）。
            && (0..=5).contains(&input.soil_surface_resistance_scheme),
        "plant-hydraulic inputs are physically invalid"
    );
    Ok(layers)
}

/// `MOD_PHSRootfluxBalance:balance_phs_rootflux` 的移植。
///
/// 把逐层根系吸水按比例缩放到 `sum(rootflux) = etr`：优先保留正值层的分配
/// （`max(rootflux,0) * etr/sum_pos_flux`），没有正值层时按 `fallback_weights`
/// （上游两个调用点都传 `rootfr`）加权，权重和也是零才均分。与上游
/// `MOD_PHSRootfluxBalance.F90:40` 一致，`|etr - sum(rootflux)| <= 1e-7` 时**早退**。
///
/// 上游在扩展截留这条路上调它两次（`MOD_LeafTemperature_Extended.F90:1145`
/// 的 `'post-PHS'` 与 `:1367` 的 `'post-leaf-temperature'`），对应
/// `leaf_temperature.rs` 里同名 `context` 的两个调用点。
///
/// 默认配置的干/湿/雪三份窗口里它每步都早退（内核日志里
/// `Warning: adjusting vegetation PHS rootflux balance` 出现 0 次），所以它
/// **不改黄金数字**；补它是为了条件一旦不成立时不静默算错（第 276 轮）。
pub fn balance_phs_rootflux(
    etr: f64,
    root_flux: &mut [f64],
    fallback_weights: &[f64],
    context: &str,
) {
    const TOL: f64 = 1.0e-7;
    const TINY_FLUX: f64 = 1.0e-15;
    // 上游 `warn_limit`；`warn_count` 是模块 `save` 变量，跨调用保留：
    // 前 5 次真正动手时打警告，第 6 次打一次"不再打印"，之后静默。
    const WARN_LIMIT: usize = 5;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static WARN_COUNT: AtomicUsize = AtomicUsize::new(0);

    let sum_flux: f64 = root_flux.iter().sum();
    if (etr - sum_flux).abs() <= TOL {
        return;
    }
    let warn_count = WARN_COUNT.load(Ordering::Relaxed);
    if warn_count < WARN_LIMIT {
        eprintln!(
            "Warning: adjusting vegetation PHS rootflux balance {context} etr={etr} sum={sum_flux} gap={}",
            (etr - sum_flux).abs()
        );
    } else if warn_count == WARN_LIMIT {
        eprintln!(
            "Warning: suppressing further vegetation PHS rootflux balance messages on this task."
        );
    }
    WARN_COUNT.store(warn_count + 1, Ordering::Relaxed);

    // 上游 `sum(rootflux, rootflux > 0._r8)`（掩码 SUM）——只累加正值项。
    let sum_pos_flux: f64 = root_flux.iter().copied().filter(|value| *value > 0.0).sum();
    if sum_pos_flux.abs() > TINY_FLUX {
        let scale = etr / sum_pos_flux;
        for flux in root_flux.iter_mut() {
            *flux = flux.max(0.0) * scale;
        }
    } else {
        let sum_weight: f64 = fallback_weights.iter().sum();
        if sum_weight.abs() > TINY_FLUX {
            for (flux, weight) in root_flux.iter_mut().zip(fallback_weights) {
                *flux = etr * weight / sum_weight;
            }
        } else {
            let equal_share = etr / root_flux.len() as f64;
            for flux in root_flux.iter_mut() {
                *flux = equal_share;
            }
        }
    }
}

#[cfg(test)]
#[path = "plant_hydraulics_tests.rs"]
mod plant_hydraulics_tests;
