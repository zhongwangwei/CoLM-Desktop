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
const VEGETATION_SEGMENTS: usize = 4;
const MIN_STRESS: f64 = 1.0e-2;
const MIN_CONDUCTANCE: f64 = 1.0e-16;

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
    let boundary_conductance = conversion / input.leaf_boundary_resistance_s_m;
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
        let maximum = potential
            .iter()
            .copied()
            .chain(change)
            .map(f64::abs)
            .fold(0.0, f64::max);
        let mut change = change;
        if change.iter().copied().map(f64::abs).fold(0.0, f64::max) > 200_000.0 {
            let scale = maximum / 2.0;
            let largest = change.iter().copied().map(f64::abs).fold(0.0, f64::max);
            change = change.map(|value| scale * value / largest);
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
        let (_, root_top) = root_potential_from_flux(
            input,
            sunlit + shaded,
            &soil_root_conductance_mm_s,
            &axial_root_conductance_mm_s,
        )?;
        potential[ROOT] = root_top;
        let root_potential = root_potential_from_flux(
            input,
            sunlit + shaded,
            &soil_root_conductance_mm_s,
            &axial_root_conductance_mm_s,
        )?
        .0;
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
    let boundary_conductance =
        conductance_conversion(input.surface_pressure_pa, input.leaf_temperature_k)
            / input.leaf_boundary_resistance_s_m;
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
            std::f64::consts::PI * input.parameters.fine_root_radius_m.powi(2);
        let root_length_density_m_m3 = root_biomass_density
            / (input.parameters.root_tissue_density_g_m3 * root_cross_section_m2);
        let root_area_index =
            (input.stem_area_index + input.sunlit_leaf_area_index + input.shaded_leaf_area_index)
                * input.parameters.fine_root_to_leaf_area
                * input.root_fraction[layer];
        let root_spacing_m = (1.0 / (std::f64::consts::PI * root_length_density_m_m3)).sqrt();
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
    let leaf = (1.0 - delta * (1.0 - input.wet_canopy_fraction))
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
    let total = air + ground + leaf;
    let air_weight = air / total;
    let ground_weight = ground / total;
    let driving_humidity = (air_weight + ground_weight) * input.leaf_saturation_specific_humidity
        - air_weight * input.reference_specific_humidity
        - ground_weight * input.ground_specific_humidity;
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
    let wet = (1.0 - delta * (1.0 - input.wet_canopy_fraction))
        * (input.sunlit_leaf_area_index + input.shaded_leaf_area_index + input.stem_area_index)
        * boundary_conductance_umol_m2_s
        / conversion;
    let leaf = air * (input.leaf_saturation_specific_humidity - input.reference_specific_humidity)
        + ground * (input.leaf_saturation_specific_humidity - input.ground_specific_humidity);
    let a1 = leaf - sunlit_transpiration_kg_m2_s / input.air_density_kg_m3;
    let b1 = -sunlit_transpiration_kg_m2_s / input.air_density_kg_m3;
    let c1 = sunlit_transpiration_kg_m2_s * (air + ground + wet) / input.air_density_kg_m3;
    let a2 = -shaded_transpiration_kg_m2_s / input.air_density_kg_m3;
    let b2 = leaf - shaded_transpiration_kg_m2_s / input.air_density_kg_m3;
    let c2 = shaded_transpiration_kg_m2_s * (air + ground + wet) / input.air_density_kg_m3;
    let sunlit_leaf_conductance = (b1 * c2 - b2 * c1) / (b1 * a2 - b2 * a1);
    let shaded_leaf_conductance = (a1 * c2 - a2 * c1) / (a1 * b2 - b1 * a2);
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
        rhs[row] = radial[layer] * input.soil_matric_potential_mm[layer] + axial[layer - 1]
            - next_distance.map_or(0.0, |_| axial[layer]);
        if layer == 1 {
            rhs[row] += axial[0] / previous_distance * root_one;
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
    let root_flux = radial[0] * (input.soil_matric_potential_mm[0] - root_one)
        + (root_two - root_one) * axial[0] / (depth_mm[1] - depth_mm[0])
        - axial[0];
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
        rhs[layer] = radial[layer] * input.soil_matric_potential_mm[layer]
            + previous_distance.map_or(0.0, |_| axial[layer - 1])
            - next_distance.map_or(0.0, |_| axial[layer]);
    }
    rhs[0] -= root_flux_kg_m2_s;
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
    let a11 =
        -input.sunlit_leaf_area_index * input.maximum_sunlit_leaf_hydraulic_conductance * fxyl
            - sunlit_flux * dfsun;
    let a13 = input.sunlit_leaf_area_index
        * input.maximum_sunlit_leaf_hydraulic_conductance
        * (dfxyl * (x[XYLEM] - x[SUNLIT]) + fxyl);
    let a22 =
        -input.shaded_leaf_area_index * input.maximum_shaded_leaf_hydraulic_conductance * fxyl
            - shaded_flux * dfsha;
    let a23 = input.shaded_leaf_area_index
        * input.maximum_shaded_leaf_hydraulic_conductance
        * (dfxyl * (x[XYLEM] - x[SHADED]) + fxyl);
    let a31 = input.sunlit_leaf_area_index * input.maximum_sunlit_leaf_hydraulic_conductance * fxyl;
    let a32 = input.shaded_leaf_area_index * input.maximum_shaded_leaf_hydraulic_conductance * fxyl;
    let mut a33 = -input.sunlit_leaf_area_index
        * input.maximum_sunlit_leaf_hydraulic_conductance
        * (dfxyl * (x[XYLEM] - x[SUNLIT]) + fxyl)
        - input.shaded_leaf_area_index
            * input.maximum_shaded_leaf_hydraulic_conductance
            * (dfxyl * (x[XYLEM] - x[SHADED]) + fxyl)
        - xylem * froot;
    let a34 = xylem * (dfroot * (x[ROOT] - x[XYLEM] - gravity) + froot);
    let a43 = xylem * froot;
    let a44 = -xylem * froot - xylem * dfroot * (x[ROOT] - x[XYLEM] - gravity) + root_flux_slope;
    let mut f = [0.0; VEGETATION_SEGMENTS];
    f[SUNLIT] = sunlit_flux * fsun
        - input.sunlit_leaf_area_index
            * input.maximum_sunlit_leaf_hydraulic_conductance
            * fxyl
            * (x[XYLEM] - x[SUNLIT]);
    f[SHADED] = shaded_flux * fsha
        - input.shaded_leaf_area_index
            * input.maximum_shaded_leaf_hydraulic_conductance
            * fxyl
            * (x[XYLEM] - x[SHADED]);
    f[XYLEM] = input.sunlit_leaf_area_index
        * input.maximum_sunlit_leaf_hydraulic_conductance
        * fxyl
        * (x[XYLEM] - x[SUNLIT])
        + input.shaded_leaf_area_index
            * input.maximum_shaded_leaf_hydraulic_conductance
            * fxyl
            * (x[XYLEM] - x[SHADED])
        - xylem * froot * (x[ROOT] - x[XYLEM] - gravity);
    f[ROOT] = xylem * froot * (x[ROOT] - x[XYLEM] - gravity) - root_flux;
    let mut change = [0.0; VEGETATION_SEGMENTS];
    if shaded_flux > 0.0 {
        let determinant = a44 * a22 * a33 * a11
            - a44 * a22 * a31 * a13
            - a44 * a32 * a23 * a11
            - a43 * a11 * a22 * a34;
        if determinant != 0.0 {
            change[SUNLIT] = ((a22 * a33 * a44 - a22 * a34 * a43 - a23 * a32 * a44) * f[SUNLIT]
                + a13 * a32 * a44 * f[SHADED]
                - a13 * a22 * a44 * f[XYLEM]
                + a13 * a22 * a34 * f[ROOT])
                / determinant;
            change[SHADED] = (a23 * a31 * a44 * f[SUNLIT]
                + (a11 * a33 * a44 - a11 * a34 * a43 - a13 * a31 * a44) * f[SHADED]
                - a11 * a23 * a44 * f[XYLEM]
                + a11 * a23 * a34 * f[ROOT])
                / determinant;
            change[XYLEM] = (-a22 * a31 * a44 * f[SUNLIT] - a11 * a32 * a44 * f[SHADED]
                + a11 * a22 * a44 * f[XYLEM]
                - a11 * a22 * a34 * f[ROOT])
                / determinant;
            change[ROOT] = (a22 * a31 * a43 * f[SUNLIT] + a11 * a32 * a43 * f[SHADED]
                - a11 * a22 * a43 * f[XYLEM]
                + (a11 * a22 * a33 - a11 * a23 * a32 - a13 * a22 * a31) * f[ROOT])
                / determinant;
        }
    } else {
        a33 = -input.sunlit_leaf_area_index
            * input.maximum_sunlit_leaf_hydraulic_conductance
            * (dfxyl * (x[XYLEM] - x[SUNLIT]) + fxyl)
            - xylem * froot;
        f[XYLEM] = input.sunlit_leaf_area_index
            * input.maximum_sunlit_leaf_hydraulic_conductance
            * fxyl
            * (x[XYLEM] - x[SUNLIT])
            - xylem * froot * (x[ROOT] - x[XYLEM] - gravity);
        let determinant = a11 * a33 * a44 - a34 * a11 * a43 - a13 * a31 * a44;
        if determinant != 0.0 {
            change[SUNLIT] =
                (-a13 * a44 * f[XYLEM] + a13 * a34 * f[ROOT] + (a33 * a44 - a34 * a43) * f[SUNLIT])
                    / determinant;
            change[XYLEM] =
                (a11 * a44 * f[XYLEM] - a11 * a34 * f[ROOT] - a31 * a44 * f[SUNLIT]) / determinant;
            change[ROOT] =
                (-a11 * a43 * f[XYLEM] + (a11 * a33 - a13 * a31) * f[ROOT] + a31 * a43 * f[SUNLIT])
                    / determinant;
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
            && (1..=5).contains(&input.soil_surface_resistance_scheme),
        "plant-hydraulic inputs are physically invalid"
    );
    Ok(layers)
}

#[cfg(test)]
#[path = "plant_hydraulics_tests.rs"]
mod plant_hydraulics_tests;
