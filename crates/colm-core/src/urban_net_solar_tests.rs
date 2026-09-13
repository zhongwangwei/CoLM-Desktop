use super::*;

#[test]
fn urban_net_solar_uses_shared_band_and_direct_diffuse_order() {
    let fluxes = urban_net_solar(input(), &radiation()).unwrap();
    assert_eq!(fluxes.roof_absorbed_w_m2, 37.0);
    assert_eq!(fluxes.sunlit_wall_absorbed_w_m2, 74.0);
    assert_eq!(fluxes.shaded_wall_absorbed_w_m2, 111.0);
    assert_eq!(fluxes.impervious_absorbed_w_m2, 148.0);
    assert_eq!(fluxes.pervious_absorbed_w_m2, 185.0);
    assert_eq!(fluxes.lake_absorbed_w_m2, 222.0);
    assert_eq!(fluxes.vegetation_absorbed_w_m2, 259.0);
    assert_eq!(fluxes.vegetation_par_w_m2, 91.0);
    assert_eq!(fluxes.reflected_w_m2, 92.0);
    assert_eq!(fluxes.local_noon.direct_visible_w_m2, 100.0);
}

#[test]
fn urban_net_solar_keeps_reflection_but_zeros_absorption_without_sun() {
    let mut input = input();
    input.forcing = ShortwaveForcing {
        direct_visible_w_m2: 0.0,
        direct_near_infrared_w_m2: 0.0,
        diffuse_visible_w_m2: 0.0,
        diffuse_near_infrared_w_m2: 0.0,
    };
    let fluxes = urban_net_solar(input, &radiation()).unwrap();
    assert_eq!(fluxes.roof_absorbed_w_m2, 0.0);
    assert_eq!(fluxes.reflected_w_m2, 0.0);
}

fn input() -> UrbanNetSolarInput {
    UrbanNetSolarInput {
        forcing: ShortwaveForcing {
            direct_visible_w_m2: 100.0,
            direct_near_infrared_w_m2: 200.0,
            diffuse_visible_w_m2: 30.0,
            diffuse_near_infrared_w_m2: 40.0,
        },
        greenwich_time: false,
        seconds_of_day: 43_200,
        time_step_seconds: 1800,
        longitude_radians: 0.0,
    }
}

fn radiation() -> UrbanRadiationState {
    UrbanRadiationState {
        sunlit_wall_fraction: 0.5,
        change_in_sunlit_wall_fraction: 0.0,
        diffuse_extinction: 0.718,
        albedo: [[0.1, 0.2], [0.3, 0.4]],
        sunlit_tree_absorption: [[0.7; 2]; 2],
        shaded_tree_absorption: [[0.0; 2]; 2],
        roof_absorption: [[0.1; 2]; 2],
        sunlit_wall_absorption: [[0.2; 2]; 2],
        shaded_wall_absorption: [[0.3; 2]; 2],
        impervious_absorption: [[0.4; 2]; 2],
        pervious_absorption: [[0.5; 2]; 2],
        lake_absorption: [[0.6; 2]; 2],
    }
}
