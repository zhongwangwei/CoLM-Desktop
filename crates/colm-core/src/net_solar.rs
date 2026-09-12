//! Broadband surface shortwave fluxes from MOD_NetSolar.F90.
//!
//! This is the common non-SNICAR path. SNICAR layer absorption remains a separate
//! kernel because its optical state is not part of the broadband restart state.

use anyhow::{ensure, Result};

use crate::{ColdStartRadiation, MISSING};

const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;

/// Direct and diffuse visible/near-infrared forcing in W m-2.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShortwaveForcing {
    pub direct_visible_w_m2: f64,
    pub direct_near_infrared_w_m2: f64,
    pub diffuse_visible_w_m2: f64,
    pub diffuse_near_infrared_w_m2: f64,
}

impl ShortwaveForcing {
    fn total(self) -> f64 {
        self.direct_visible_w_m2
            + self.direct_near_infrared_w_m2
            + self.diffuse_visible_w_m2
            + self.diffuse_near_infrared_w_m2
    }
}

/// Timestep metadata consumed by the local-noon output branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetSolarInput {
    pub patch_type: i32,
    pub forcing: ShortwaveForcing,
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    pub snow_fraction: f64,
    pub greenwich_time: bool,
    pub seconds_of_day: i32,
    pub time_step_seconds: i32,
    pub longitude_radians: f64,
}

/// Local-noon radiation diagnostics. CoLM writes MISSING away from local noon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalNoonShortwave {
    pub direct_visible_w_m2: f64,
    pub diffuse_visible_w_m2: f64,
    pub direct_near_infrared_w_m2: f64,
    pub diffuse_near_infrared_w_m2: f64,
    pub reflected_direct_visible_w_m2: f64,
    pub reflected_diffuse_visible_w_m2: f64,
    pub reflected_direct_near_infrared_w_m2: f64,
    pub reflected_diffuse_near_infrared_w_m2: f64,
}

/// Broadband diagnostics produced by one netsolar call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetSolarFluxes {
    pub par_sunlit_w_m2: f64,
    pub par_shaded_w_m2: f64,
    pub sunlit_absorbed_w_m2: f64,
    pub shaded_absorbed_w_m2: f64,
    pub ground_absorbed_w_m2: f64,
    pub soil_absorbed_w_m2: f64,
    pub snow_absorbed_w_m2: f64,
    pub reflected_w_m2: f64,
    pub direct_visible_w_m2: f64,
    pub diffuse_visible_w_m2: f64,
    pub direct_near_infrared_w_m2: f64,
    pub diffuse_near_infrared_w_m2: f64,
    pub reflected_direct_visible_w_m2: f64,
    pub reflected_diffuse_visible_w_m2: f64,
    pub reflected_direct_near_infrared_w_m2: f64,
    pub reflected_diffuse_near_infrared_w_m2: f64,
    pub local_noon: LocalNoonShortwave,
}

/// Port of MOD_NetSolar.F90:netsolar excluding its SNICAR-only layer branch.
///
/// Radiation is updated in place when CoLM's soil/snow balance correction rescales
/// its absorption coefficients. The stored coefficients are then ready for the next step.
pub fn net_solar(
    input: NetSolarInput,
    radiation: &mut ColdStartRadiation,
) -> Result<NetSolarFluxes> {
    validate(input, radiation)?;
    if input.leaf_area_index + input.stem_area_index <= 1.0e-6 {
        radiation.sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
        radiation.shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    }

    let forcing = input.forcing;
    let mut par_sunlit_w_m2 = 0.0;
    let mut par_shaded_w_m2 = 0.0;
    let mut sunlit_absorbed_w_m2 = 0.0;
    let mut shaded_absorbed_w_m2 = 0.0;
    let mut ground_absorbed_w_m2 = 0.0;
    let mut soil_absorbed_w_m2 = 0.0;
    let mut snow_absorbed_w_m2 = 0.0;

    if forcing.total() > 0.0 {
        if input.patch_type < 4 {
            par_sunlit_w_m2 = visible_absorption(forcing, radiation.sunlit_absorption);
            par_shaded_w_m2 = visible_absorption(forcing, radiation.shaded_absorption);
            sunlit_absorbed_w_m2 = absorption(forcing, radiation.sunlit_absorption);
            shaded_absorbed_w_m2 = absorption(forcing, radiation.shaded_absorption);
            let absorbed_without_vegetation = absorbed_by_surface(forcing, radiation.albedo);
            ground_absorbed_w_m2 =
                absorbed_without_vegetation - sunlit_absorbed_w_m2 - shaded_absorbed_w_m2;
        } else {
            ground_absorbed_w_m2 = absorbed_by_surface(forcing, radiation.albedo);
        }

        soil_absorbed_w_m2 =
            absorption(forcing, radiation.soil_absorption) * (1.0 - input.snow_fraction);
        snow_absorbed_w_m2 = absorption(forcing, radiation.snow_absorption) * input.snow_fraction;

        let unadjusted_ground = soil_absorbed_w_m2 + snow_absorbed_w_m2;
        if (unadjusted_ground - ground_absorbed_w_m2).abs() > 1.0e-6 && unadjusted_ground > 0.0 {
            let adjustment = ground_absorbed_w_m2 / unadjusted_ground;
            soil_absorbed_w_m2 *= adjustment;
            snow_absorbed_w_m2 *= adjustment;
            scale(&mut radiation.soil_absorption, adjustment);
            scale(&mut radiation.snow_absorption, adjustment);
        }
    }

    let reflected_direct_visible_w_m2 = forcing.direct_visible_w_m2 * radiation.albedo[0][0];
    let reflected_diffuse_visible_w_m2 = forcing.diffuse_visible_w_m2 * radiation.albedo[0][1];
    let reflected_direct_near_infrared_w_m2 =
        forcing.direct_near_infrared_w_m2 * radiation.albedo[1][0];
    let reflected_diffuse_near_infrared_w_m2 =
        forcing.diffuse_near_infrared_w_m2 * radiation.albedo[1][1];
    let reflected_w_m2 = reflected_direct_visible_w_m2
        + reflected_diffuse_visible_w_m2
        + reflected_direct_near_infrared_w_m2
        + reflected_diffuse_near_infrared_w_m2;
    let local_noon = local_noon(input, radiation.albedo);

    Ok(NetSolarFluxes {
        par_sunlit_w_m2,
        par_shaded_w_m2,
        sunlit_absorbed_w_m2,
        shaded_absorbed_w_m2,
        ground_absorbed_w_m2,
        soil_absorbed_w_m2,
        snow_absorbed_w_m2,
        reflected_w_m2,
        direct_visible_w_m2: forcing.direct_visible_w_m2,
        diffuse_visible_w_m2: forcing.diffuse_visible_w_m2,
        direct_near_infrared_w_m2: forcing.direct_near_infrared_w_m2,
        diffuse_near_infrared_w_m2: forcing.diffuse_near_infrared_w_m2,
        reflected_direct_visible_w_m2,
        reflected_diffuse_visible_w_m2,
        reflected_direct_near_infrared_w_m2,
        reflected_diffuse_near_infrared_w_m2,
        local_noon,
    })
}

fn validate(input: NetSolarInput, radiation: &ColdStartRadiation) -> Result<()> {
    ensure!(
        input.patch_type >= 0
            && input.leaf_area_index.is_finite()
            && input.stem_area_index.is_finite()
            && input.snow_fraction.is_finite()
            && (0.0..=1.0).contains(&input.snow_fraction)
            && input.time_step_seconds > 0
            && (0..86_400).contains(&input.seconds_of_day)
            && input.longitude_radians.is_finite()
            && input.longitude_radians.abs() <= std::f64::consts::PI,
        "net-solar state is invalid"
    );
    for value in [
        input.forcing.direct_visible_w_m2,
        input.forcing.direct_near_infrared_w_m2,
        input.forcing.diffuse_visible_w_m2,
        input.forcing.diffuse_near_infrared_w_m2,
    ] {
        ensure!(
            value.is_finite() && value >= 0.0,
            "shortwave forcing must be finite and non-negative"
        );
    }
    for matrix in [
        radiation.albedo,
        radiation.sunlit_absorption,
        radiation.shaded_absorption,
        radiation.soil_absorption,
        radiation.snow_absorption,
    ] {
        ensure!(
            matrix.into_iter().flatten().all(f64::is_finite),
            "radiation coefficients must be finite"
        );
    }
    Ok(())
}

fn visible_absorption(
    forcing: ShortwaveForcing,
    coefficient: [[f64; RADIATION_TYPES]; BANDS],
) -> f64 {
    forcing.direct_visible_w_m2 * coefficient[0][0]
        + forcing.diffuse_visible_w_m2 * coefficient[0][1]
}

fn absorption(forcing: ShortwaveForcing, coefficient: [[f64; RADIATION_TYPES]; BANDS]) -> f64 {
    visible_absorption(forcing, coefficient)
        + forcing.direct_near_infrared_w_m2 * coefficient[1][0]
        + forcing.diffuse_near_infrared_w_m2 * coefficient[1][1]
}

fn absorbed_by_surface(forcing: ShortwaveForcing, albedo: [[f64; RADIATION_TYPES]; BANDS]) -> f64 {
    forcing.direct_visible_w_m2 * (1.0 - albedo[0][0])
        + forcing.diffuse_visible_w_m2 * (1.0 - albedo[0][1])
        + forcing.direct_near_infrared_w_m2 * (1.0 - albedo[1][0])
        + forcing.diffuse_near_infrared_w_m2 * (1.0 - albedo[1][1])
}

fn scale(matrix: &mut [[f64; RADIATION_TYPES]; BANDS], factor: f64) {
    for row in matrix {
        for value in row {
            *value *= factor;
        }
    }
}

fn local_noon(input: NetSolarInput, albedo: [[f64; RADIATION_TYPES]; BANDS]) -> LocalNoonShortwave {
    let local_seconds = if input.greenwich_time {
        let source_pi = f64::from(4.0_f32 * 1.0_f32.atan());
        let radians_per_second = source_pi / 12.0 / 3600.0;
        let offset_steps = fortran_nint(
            (input.longitude_radians / radians_per_second) / f64::from(input.time_step_seconds),
        );
        (i64::from(input.seconds_of_day) + offset_steps * i64::from(input.time_step_seconds))
            % 86_400
    } else {
        i64::from(input.seconds_of_day)
    };
    if local_seconds == 43_200 {
        LocalNoonShortwave {
            direct_visible_w_m2: input.forcing.direct_visible_w_m2,
            diffuse_visible_w_m2: input.forcing.diffuse_visible_w_m2,
            direct_near_infrared_w_m2: input.forcing.direct_near_infrared_w_m2,
            diffuse_near_infrared_w_m2: input.forcing.diffuse_near_infrared_w_m2,
            reflected_direct_visible_w_m2: input.forcing.direct_visible_w_m2 * albedo[0][0],
            reflected_diffuse_visible_w_m2: input.forcing.diffuse_visible_w_m2 * albedo[0][1],
            reflected_direct_near_infrared_w_m2: input.forcing.direct_near_infrared_w_m2
                * albedo[1][0],
            reflected_diffuse_near_infrared_w_m2: input.forcing.diffuse_near_infrared_w_m2
                * albedo[1][1],
        }
    } else {
        LocalNoonShortwave {
            direct_visible_w_m2: MISSING,
            diffuse_visible_w_m2: MISSING,
            direct_near_infrared_w_m2: MISSING,
            diffuse_near_infrared_w_m2: MISSING,
            reflected_direct_visible_w_m2: MISSING,
            reflected_diffuse_visible_w_m2: MISSING,
            reflected_direct_near_infrared_w_m2: MISSING,
            reflected_diffuse_near_infrared_w_m2: MISSING,
        }
    }
}

fn fortran_nint(value: f64) -> i64 {
    if value >= 0.0 {
        (value + 0.5).floor() as i64
    } else {
        (value - 0.5).ceil() as i64
    }
}

#[cfg(test)]
#[path = "net_solar_tests.rs"]
mod net_solar_tests;
