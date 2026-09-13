use super::*;
use crate::{leaf_optics_from_land_cover, LandCoverScheme};

#[test]
fn au_preston_cold_start_matches_fortran_urban_shortwave() {
    let state = cold_start_urban_radiation(au_preston_input()).unwrap();
    close(state.sunlit_wall_fraction, 0.4811002822645794);
    close(state.change_in_sunlit_wall_fraction, -0.0188997177354206);
    close_matrix(
        state.albedo,
        [
            [0.128130404566396, 0.128977608452442],
            [0.174905278232711, 0.189463114364827],
        ],
    );
    close_matrix(
        state.sunlit_tree_absorption,
        [
            [0.926924692150317, 1.19862448688493],
            [0.505235135269596, 0.653329455884808],
        ],
    );
    close_matrix(state.roof_absorption, [[0.7827, 0.7827], [0.7827, 0.7827]]);
    close_matrix(
        state.sunlit_wall_absorption,
        [
            [0.355509473831149, 0.241071024912283],
            [0.38344682419948, 0.277197360237844],
        ],
    );
    close_matrix(
        state.shaded_wall_absorption,
        [
            [0.0406540834534927, 0.241071024912283],
            [0.0685914338218238, 0.277197360237844],
        ],
    );
    close_matrix(
        state.impervious_absorption,
        [
            [0.432556581673735, 0.284430208888391],
            [0.499631738286416, 0.371166408558605],
        ],
    );
    close_matrix(
        state.pervious_absorption,
        [
            [0.427526853979854, 0.281122880878061],
            [0.493822066910993, 0.366850520086993],
        ],
    );
    close_matrix(
        state.lake_absorption,
        [[0.946010368587962, 0.9], [0.946010368587962, 0.9]],
    );
    assert_eq!(state.shaded_tree_absorption, [[0.0; 2]; 2]);
}

#[test]
fn night_keeps_the_fortran_initialized_shortwave_state() {
    let mut input = au_preston_input();
    input.cosine_zenith = -0.31;
    let state = cold_start_urban_radiation(input).unwrap();
    assert_eq!(state.albedo, [[1.0; 2]; 2]);
    assert_eq!(state.roof_absorption, [[0.0; 2]; 2]);
    assert_eq!(state.change_in_sunlit_wall_fraction, 0.0);
}

fn au_preston_input() -> UrbanRadiationInput {
    UrbanRadiationInput {
        roof_fraction: 0.444999992847443,
        pervious_ground_fraction: 0.684684667269157,
        water_fraction: 0.0,
        building_height_to_length: 0.224719108084118,
        roof_height_m: 6.40000009536743,
        roof_albedo: [[0.2173; 2]; 2],
        wall_albedo: [[0.25; 2]; 2],
        impervious_albedo: [[0.14; 2]; 2],
        pervious_albedo: [[0.15; 2]; 2],
        leaf_optics: leaf_optics_from_land_cover(LandCoverScheme::Igbp, 13).unwrap(),
        vegetation_fraction: 0.224999994039536,
        vegetation_center_height_m: 3.34999990463257,
        lai: 3.16774039100746,
        // `UrbanIniTimeVar` receives the snow-free patch SAI (`sai`), not
        // `urb_sai` / `tsai` serialized by the restart writer.
        sai: 0.141374021550932,
        wet_snow_fraction: 0.0,
        vegetation_snow: true,
        cosine_zenith: 0.776103747929124,
        previous_sunlit_wall_fraction: 0.5,
        lake_temperature_k: 283.0,
        roof_snow_fraction: 0.0,
        impervious_snow_fraction: 0.0,
        pervious_snow_fraction: 0.0,
        lake_snow_fraction: 0.0,
        roof_snow_water_mm: 0.0,
        impervious_snow_water_mm: 0.0,
        pervious_snow_water_mm: 0.0,
        lake_snow_water_mm: 0.0,
        roof_snow_age: 0.0,
        impervious_snow_age: 0.0,
        pervious_snow_age: 0.0,
        lake_snow_age: 0.0,
    }
}

fn close_matrix(actual: [[f64; 2]; 2], expected: [[f64; 2]; 2]) {
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        close(actual, expected);
    }
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2.0e-12,
        "{actual} != {expected}"
    );
}
