//! Urban absorbed shortwave diagnostics from `MOD_Urban_NetSolar.F90`.

use anyhow::{ensure, Result};

use crate::{
    net_solar::{absorption, local_noon_shortwave, visible_absorption},
    LocalNoonShortwave, ShortwaveForcing, UrbanRadiationState,
};

/// Time and forcing data used by the urban net-solar diagnostic.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanNetSolarInput {
    pub forcing: ShortwaveForcing,
    pub greenwich_time: bool,
    pub seconds_of_day: i32,
    pub time_step_seconds: i32,
    pub longitude_radians: f64,
}

/// Fluxes produced by one `netsolar_urban` call, all in W m⁻².
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UrbanNetSolarFluxes {
    pub reflected_w_m2: f64,
    pub vegetation_absorbed_w_m2: f64,
    pub vegetation_par_w_m2: f64,
    pub roof_absorbed_w_m2: f64,
    pub sunlit_wall_absorbed_w_m2: f64,
    pub shaded_wall_absorbed_w_m2: f64,
    pub impervious_absorbed_w_m2: f64,
    pub pervious_absorbed_w_m2: f64,
    pub lake_absorbed_w_m2: f64,
    pub reflected_direct_visible_w_m2: f64,
    pub reflected_diffuse_visible_w_m2: f64,
    pub reflected_direct_near_infrared_w_m2: f64,
    pub reflected_diffuse_near_infrared_w_m2: f64,
    pub local_noon: LocalNoonShortwave,
}

/// Ports `MOD_Urban_NetSolar::netsolar_urban` using the shared Urban albedo state.
pub fn urban_net_solar(
    input: UrbanNetSolarInput,
    radiation: &UrbanRadiationState,
) -> Result<UrbanNetSolarFluxes> {
    validate(input, radiation)?;
    let forcing = input.forcing;
    let active = forcing.total() > 0.0;
    let flux = |coefficient| {
        if active {
            absorption(forcing, coefficient)
        } else {
            0.0
        }
    };
    let reflected_direct_visible_w_m2 = forcing.direct_visible_w_m2 * radiation.albedo[0][0];
    let reflected_diffuse_visible_w_m2 = forcing.diffuse_visible_w_m2 * radiation.albedo[0][1];
    let reflected_direct_near_infrared_w_m2 =
        forcing.direct_near_infrared_w_m2 * radiation.albedo[1][0];
    let reflected_diffuse_near_infrared_w_m2 =
        forcing.diffuse_near_infrared_w_m2 * radiation.albedo[1][1];
    Ok(UrbanNetSolarFluxes {
        reflected_w_m2: reflected_direct_visible_w_m2
            + reflected_diffuse_visible_w_m2
            + reflected_direct_near_infrared_w_m2
            + reflected_diffuse_near_infrared_w_m2,
        vegetation_absorbed_w_m2: flux(radiation.sunlit_tree_absorption),
        vegetation_par_w_m2: if active {
            visible_absorption(forcing, radiation.sunlit_tree_absorption)
        } else {
            0.0
        },
        roof_absorbed_w_m2: flux(radiation.roof_absorption),
        sunlit_wall_absorbed_w_m2: flux(radiation.sunlit_wall_absorption),
        shaded_wall_absorbed_w_m2: flux(radiation.shaded_wall_absorption),
        impervious_absorbed_w_m2: flux(radiation.impervious_absorption),
        pervious_absorbed_w_m2: flux(radiation.pervious_absorption),
        lake_absorbed_w_m2: flux(radiation.lake_absorption),
        reflected_direct_visible_w_m2,
        reflected_diffuse_visible_w_m2,
        reflected_direct_near_infrared_w_m2,
        reflected_diffuse_near_infrared_w_m2,
        local_noon: local_noon_shortwave(
            input.greenwich_time,
            input.seconds_of_day,
            input.time_step_seconds,
            input.longitude_radians,
            forcing,
            radiation.albedo,
        ),
    })
}

fn validate(input: UrbanNetSolarInput, radiation: &UrbanRadiationState) -> Result<()> {
    ensure!(
        input.time_step_seconds > 0
            && (0..86_400).contains(&input.seconds_of_day)
            && input.longitude_radians.is_finite()
            && input.longitude_radians.abs() <= std::f64::consts::PI,
        "urban net-solar clock is invalid"
    );
    for value in [
        input.forcing.direct_visible_w_m2,
        input.forcing.direct_near_infrared_w_m2,
        input.forcing.diffuse_visible_w_m2,
        input.forcing.diffuse_near_infrared_w_m2,
    ] {
        ensure!(
            value.is_finite() && value >= 0.0,
            "urban shortwave forcing must be finite and nonnegative"
        );
    }
    for matrix in [
        radiation.albedo,
        radiation.sunlit_tree_absorption,
        radiation.shaded_tree_absorption,
        radiation.roof_absorption,
        radiation.sunlit_wall_absorption,
        radiation.shaded_wall_absorption,
        radiation.impervious_absorption,
        radiation.pervious_absorption,
        radiation.lake_absorption,
    ] {
        ensure!(
            matrix.into_iter().flatten().all(f64::is_finite),
            "urban radiation coefficients must be finite"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "urban_net_solar_tests.rs"]
mod urban_net_solar_tests;
