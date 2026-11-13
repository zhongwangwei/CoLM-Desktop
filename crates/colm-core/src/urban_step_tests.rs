use super::*;

#[test]
fn remembered_ice_fraction_clears_inactive_snow_and_keeps_active_mass_ratios() {
    let mut snow = RuntimeSnowColumn::empty();
    snow.layer_count = -2;
    snow.previous_ice_fraction.fill(1.0);
    snow.ice_water_kg_m2[snow_layer_slot(-1)] = 3.0;
    snow.liquid_water_kg_m2[snow_layer_slot(-1)] = 1.0;
    snow.ice_water_kg_m2[snow_layer_slot(0)] = 1.0;
    snow.liquid_water_kg_m2[snow_layer_slot(0)] = 1.0;
    remember_ice_fraction(&mut snow);
    assert_eq!(snow.previous_ice_fraction, [0.0, 0.0, 0.0, 0.75, 0.5]);

    snow.layer_count = 0;
    remember_ice_fraction(&mut snow);
    assert_eq!(snow.previous_ice_fraction, [0.0; MAX_SNOW_LAYERS]);
}
