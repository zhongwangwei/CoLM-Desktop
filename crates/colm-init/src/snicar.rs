//! Once-loaded SNICAR tables and cold-state orchestration; physics stays in colm-core.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    age_snow_grains, snicar_ad_rt, snow_aerosol_concentrations, ColdStartGroundAlbedo,
    SnicarAgingTable, SnicarIncident, SnicarInput, SnicarOptics, SnowGrainAgingInput, SnowState,
    FRESH_SNOW_RADIUS_MIN_UM,
};
use colm_forcing::{read_snicar_aging, read_snicar_optics};
use colm_namelist::{Document, Value};

/// Immutable native tables, loaded before output and shared across cold patches.
#[derive(Debug, Clone, PartialEq)]
pub struct SnicarInitialization {
    optics: SnicarOptics,
    aging: SnicarAgingTable,
}

pub(crate) struct ColdSnicarState {
    pub ground: ColdStartGroundAlbedo,
    /// Fortran -4:0 slots, with active snow at the end.
    pub grain_radius: [f64; 5],
    /// Logical restart order [band][direct/diffuse][Fortran -4:1 slot].
    pub layer_absorption: [[[f64; 6]; 2]; 2],
}

impl SnicarInitialization {
    /// Like read_namelist, derive both filenames from DEF_dir_runtime; the
    /// original unconditionally replaces the individual snow file overrides.
    /// Disabled SNICAR does not access any files.
    pub fn from_document(document: &Document) -> Result<Option<Self>> {
        match document.get("DEF_USE_SNICAR") {
            None | Some(Value::Bool(false)) => return Ok(None),
            Some(Value::Bool(true)) => {}
            Some(_) => bail!("DEF_USE_SNICAR must be a logical value"),
        }
        let runtime = match document.get("DEF_dir_runtime") {
            Some(Value::Str(path)) => path.trim(),
            None => "path/to/runtime/",
            Some(_) => bail!("DEF_USE_SNICAR: DEF_dir_runtime must be a quoted path"),
        };
        ensure!(
            !runtime.is_empty(),
            "DEF_USE_SNICAR: DEF_dir_runtime is empty"
        );
        let directory = Path::new(runtime).join("snicar");
        let optics = read_snicar_optics(directory.join("snicar_optics_5bnd_mam_c211006.nc"))
            .context("DEF_USE_SNICAR: cannot load snow optics")?;
        let aging = read_snicar_aging(directory.join("snicar_drdt_bst_fit_60_c070416.nc"))
            .context("DEF_USE_SNICAR: cannot load snow aging")?;
        Ok(Some(Self { optics, aging }))
    }

    /// Follow IniTimeVar -> AerosolMasses -> SnowAge_grain -> SnowAlbedo.
    /// Initial aerosol masses, snowfall and refreezing are zero in the source;
    /// no observational aerosol field is substituted or synthesized here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_cold(
        &self,
        mut ground: ColdStartGroundAlbedo,
        cosine_zenith: f64,
        snow: &SnowState,
        snow_water_equivalent_mm: f64,
        ground_snow_fraction: f64,
        soil_temperature_k: f64,
        soil_thickness_m: f64,
    ) -> Result<ColdSnicarState> {
        ensure!(
            (-5..=0).contains(&snow.layer_count)
                && snow.thickness_m.len() == 5
                && snow.thickness_m.iter().all(|x| x.is_finite() && *x >= 0.0)
                && soil_temperature_k.is_finite()
                && soil_thickness_m.is_finite()
                && soil_thickness_m > 0.0
                && cosine_zenith.is_finite()
                && cosine_zenith > 0.0
                && (0.0..=1.0).contains(&ground_snow_fraction),
            "SNICAR cold snow/soil geometry, temperature or solar angle is invalid"
        );
        let layers = (-snow.layer_count) as usize;
        let top = 5 - layers;
        let mut thickness = [0.0; 6];
        thickness[..5].copy_from_slice(&snow.thickness_m);
        thickness[5] = soil_thickness_m;
        let mut temperature = [-999.0; 6];
        temperature[top..5].fill(soil_temperature_k.min(272.16));
        temperature[5] = soil_temperature_k;
        let ice = std::array::from_fn(|i| snow.thickness_m[i] * 250.0);
        let liquid = [0.0; 5];
        let mut grain_radius = [FRESH_SNOW_RADIUS_MIN_UM; 5];
        let mut masses = [[0.0; 8]; 5];
        let concentrations = snow_aerosol_concentrations(
            layers,
            1800.0,
            None,
            &ice,
            &liquid,
            &mut grain_radius,
            &mut masses,
        )?;
        age_snow_grains(
            &self.aging,
            SnowGrainAgingInput {
                timestep_seconds: 1800.0,
                snow_layers: layers,
                thickness_m: &thickness,
                snowfall_kg_m2_s: 0.0,
                snowcap_ice_kg_m2_s: 0.0,
                refreezing_kg_m2_s: &[0.0; 5],
                snow_capping: false,
                snow_fraction: ground_snow_fraction,
                snow_water_equivalent_kg_m2: snow_water_equivalent_mm,
                liquid_water_kg_m2: &liquid,
                ice_water_kg_m2: &ice,
                temperature_k: &temperature,
                air_temperature_k: 273.15,
            },
            &mut grain_radius,
        )?;

        ground.snow_age = 0.0;
        ground.snow = [[1.0; 2]; 2];
        let mut layer_absorption = [[[0.0; 6]; 2]; 2];
        if snow_water_equivalent_mm > 0.0 {
            let mut input = SnicarInput {
                incident: SnicarIncident::Direct,
                cosine_zenith,
                snow_water_equivalent_kg_m2: snow_water_equivalent_mm,
                active_layers: layers,
                liquid_water_kg_m2: [0.0; 5],
                ice_water_kg_m2: [0.0; 5],
                snow_radius_microns: [55; 5],
                aerosol_mass_concentration: [[0.0; 8]; 5],
                // SnowAlbedo uses diffuse soil albedo under both incident types.
                underlying_albedo_5band: [
                    ground.soil[0][1],
                    ground.soil[1][1],
                    ground.soil[1][1],
                    ground.soil[1][1],
                    ground.soil[1][1],
                ],
            };
            for row in 0..layers {
                input.ice_water_kg_m2[row] = ice[top + row];
                input.snow_radius_microns[row] = grain_radius[top + row].round() as i32;
                input.aerosol_mass_concentration[row] = concentrations[top + row];
            }
            for (incident, index) in [(SnicarIncident::Direct, 0), (SnicarIncident::Diffuse, 1)] {
                input.incident = incident;
                let result = snicar_ad_rt(&self.optics, &input)?;
                for (band, absorption) in layer_absorption.iter_mut().enumerate() {
                    ground.snow[band][index] = result.albedo_broadband[band];
                    for row in 0..layers {
                        absorption[index][top + row] = result.absorbed_broadband[row][band];
                    }
                    absorption[index][5] =
                        result.absorbed_broadband[result.absorption_rows - 1][band];
                    if result.temporary_snow_layer {
                        // albland folds the fictitious snow layer into the first soil layer.
                        absorption[index][5] += result.absorbed_broadband[0][band];
                    }
                }
            }
        }
        ground.ground =
            colm_core::radiation::mix_ground_albedo(ground.soil, ground.snow, ground_snow_fraction);
        Ok(ColdSnicarState {
            ground,
            grain_radius,
            layer_absorption,
        })
    }
}

#[cfg(test)]
#[path = "snicar_tests.rs"]
mod tests;
