use super::*;

fn input() -> CanopyInterceptionInput {
    CanopyInterceptionInput {
        time_step_seconds: 1800.0,
        maximum_dew_mm: 0.1,
        eastward_wind_m_s: 2.0,
        northward_wind_m_s: 3.0,
        leaf_angle_distribution: 0.2,
        leaf_area_index: 2.0,
        stem_area_index: 0.5,
        leaf_temperature_k: 274.0,
        convective_rain_kg_m2_s: 0.001,
        convective_snow_kg_m2_s: 0.0005,
        large_scale_rain_kg_m2_s: 0.0002,
        large_scale_snow_kg_m2_s: 0.0001,
        sprinkler_irrigation_kg_m2_s: 0.0001,
        vegetation_snow: false,
    }
}

#[test]
fn scalar_canopy_interception_matches_current_fortran() {
    let mut water = CanopyWater {
        total_mm: 0.03,
        rain_mm: 0.02,
        snow_mm: 0.01,
    };
    let fluxes = intercept_canopy(input(), &mut water).unwrap();
    close(water.total_mm, 0.369_602_511_537_917_1);
    close(water.rain_mm, 0.02);
    close(water.snow_mm, 0.01);
    close(fluxes.ground_rain_kg_m2_s, 0.001_228_311_975_462_568_5);
    close(fluxes.ground_snow_kg_m2_s, 0.000_483_019_962_571_922_2);
    close(fluxes.retained_kg_m2_s, 0.000_188_668_061_965_509_5);
    close(fluxes.retained_rain_kg_m2_s, 0.000_071_688_024_537_431_64);
    close(fluxes.retained_snow_kg_m2_s, 0.000_116_980_037_428_077_87);
    assert_eq!(fluxes.released_rain_kg_m2_s, 0.0);
    assert_eq!(fluxes.released_snow_kg_m2_s, 0.0);
}

#[test]
fn leafless_canopy_releases_existing_water_by_leaf_temperature() {
    let mut water = CanopyWater {
        total_mm: 4.0,
        rain_mm: 3.0,
        snow_mm: 1.0,
    };
    let mut input = input();
    input.leaf_area_index = 0.0;
    input.stem_area_index = 0.0;
    input.leaf_temperature_k = 270.0;
    let fluxes = intercept_canopy(input, &mut water).unwrap();
    close(fluxes.ground_snow_kg_m2_s, 0.0006 + 4.0 / 1800.0);
    close(fluxes.ground_rain_kg_m2_s, 0.0013);
    assert_eq!(water.total_mm, 0.0);
    assert_eq!(water.rain_mm, 0.0);
    assert_eq!(water.snow_mm, 0.0);
}

#[test]
fn invalid_interception_inputs_are_rejected() {
    let mut water = CanopyWater {
        total_mm: 0.0,
        rain_mm: 0.0,
        snow_mm: 0.0,
    };
    let mut invalid = input();
    invalid.time_step_seconds = 0.0;
    assert!(intercept_canopy(invalid, &mut water).is_err());
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "got {actual:.17e}, expected {expected:.17e}"
    );
}

#[test]
fn vegetation_snow_partition_matches_current_fortran() {
    let mut water = CanopyWater {
        total_mm: 0.03,
        rain_mm: 0.02,
        snow_mm: 0.01,
    };
    let mut snow = input();
    snow.vegetation_snow = true;
    let fluxes = intercept_canopy(snow, &mut water).unwrap();
    close(water.total_mm, 0.257_962_949_017_247_26);
    close(water.rain_mm, 0.151_030_450_125_049_68);
    close(water.snow_mm, 0.106_932_498_892_197_58);
    close(fluxes.ground_rain_kg_m2_s, 0.001_227_205_305_486_083_8);
    close(fluxes.ground_snow_kg_m2_s, 0.000_546_148_611_726_557);
    close(fluxes.retained_kg_m2_s, 0.000_126_646_082_787_359_57);
    close(fluxes.retained_rain_kg_m2_s, 0.000_072_794_694_513_916_37);
    close(fluxes.retained_snow_kg_m2_s, 0.000_053_851_388_273_443_07);
}

#[test]
fn leaf_wetness_matches_leaf_temperature_dewfraction() {
    let water = CanopyWater {
        total_mm: 0.05,
        rain_mm: 0.03,
        snow_mm: 0.02,
    };
    let without_snow = canopy_wetness(2.0, 0.5, 0.1, water, false).unwrap();
    close(without_snow.wet_fraction, 0.341_995_178_399_476_24);
    close(without_snow.dry_leaf_fraction, 0.526_403_857_280_419);
    let with_snow = canopy_wetness(2.0, 0.5, 0.1, water, true).unwrap();
    close(with_snow.wet_fraction, 0.253_925_327_561_347_67);
    close(with_snow.dry_leaf_fraction, 0.596_859_737_950_921_8);
}
