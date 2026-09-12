use super::*;

#[test]
fn glacier_water_matches_the_snow_free_fortran_branch() {
    // MOD_Glacier:GLACIER_WATER, lb=1 branch.
    let mut snow = RuntimeSnowColumn::empty();
    let mut surface = GlacierSurfaceWater {
        liquid_water_kg_m2: 4.0,
        ice_water_kg_m2: 5.0,
    };
    let drainage = glacier_water(
        GlacierWaterInput {
            time_step_seconds: 1800.0,
            irreducible_saturation: 0.033,
            impermeable_porosity: 0.05,
            rainfall_kg_m2_s: 0.001,
            snow_melt_kg_m2_s: 0.002,
            evaporation_kg_m2_s: 0.0004,
            dew_kg_m2_s: 0.0002,
            sublimation_kg_m2_s: 0.0001,
            frost_kg_m2_s: 0.0003,
            eastward_wind_m_s: 0.0,
            northward_wind_m_s: 0.0,
            melted: &[],
        },
        &mut snow,
        &mut surface,
    )
    .unwrap();
    assert!((drainage - 0.0026).abs() < 1.0e-15);
    assert!((surface.liquid_water_kg_m2 - 4.36).abs() < 1.0e-15);
    assert!((surface.ice_water_kg_m2 - 5.36).abs() < 1.0e-15);
}
