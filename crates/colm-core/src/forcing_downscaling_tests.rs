use super::*;

fn close(actual: f64, expected: f64) {
    let tolerance = 5.0e-9 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "got {actual:.17e}, expected {expected:.17e}, tolerance {tolerance:.17e}"
    );
}

fn grid() -> GridForcing {
    GridForcing {
        surface_elevation_m: 500.0,
        maximum_elevation_m: 2_500.0,
        air_temperature_k: 280.0,
        potential_temperature_k: 281.25,
        specific_humidity: 0.005,
        bottom_pressure_pa: 85_000.0,
        density_kg_m3: 1.0,
        convective_precipitation_kg_m2_s: 1.0e-4,
        large_scale_precipitation_kg_m2_s: 2.0e-4,
        downward_longwave_w_m2: 300.0,
        reference_height_m: 30.0,
        downward_shortwave_w_m2: 500.0,
        eastward_wind_m_s: 2.3,
        northward_wind_m_s: -1.1,
    }
}

fn solar() -> DownscalingSolarGeometry {
    DownscalingSolarGeometry {
        calendar_day: 172.25,
        cosine_zenith: 0.7,
        cosine_azimuth: 0.4,
    }
}

#[test]
fn simple_downscaling_matches_current_fortran() {
    let slope = [0.10, 0.12, 0.20, 0.15, 0.08, 0.16, 0.05, 0.11, 0.0];
    let area = [0.08, 0.12, 0.10, 0.10, 0.10, 0.12, 0.08, 0.10, 0.20];
    let output = downscale_forcings(
        ForcingDownscalingInput {
            glacier: false,
            grid: grid(),
            column_surface_elevation_m: 1_200.0,
            solar: solar(),
            terrain: DownscalingTerrain::Simple(SimpleTerrain {
                slope_tangent: &slope,
                area_fraction: &area,
            }),
        },
        ForcingDownscalingConfig::default(),
    )
    .unwrap();
    close(output.air_temperature_k, 275.8);
    close(output.potential_temperature_k, 277.045_572_084_361_4);
    close(output.specific_humidity, 0.004_059_677_405_196_635);
    close(output.bottom_pressure_pa, 77_991.429_556_881_27);
    close(output.density_kg_m3, 0.932_050_088_118_492_2);
    close(output.downward_longwave_w_m2, 281.863_979_848_866_5);
    close(output.downward_shortwave_w_m2, 478.074_843_755_639_1);
    close(output.convective_precipitation_kg_m2_s, 0.000_128);
    close(output.large_scale_precipitation_kg_m2_s, 0.000_256);
}

#[test]
fn full_downscaling_preserves_the_fortran_curve_and_missing_albedo_branch() {
    let slope = [0.20, 0.10, 0.05, 0.12];
    let aspect = [0.0, 1.2, 2.7, 4.1];
    let area = [0.20, 0.25, 0.30, 0.25];
    let curve = [[0.5, 2.0, -1.0]; AZIMUTH_BINS];
    let output = downscale_forcings(
        ForcingDownscalingInput {
            glacier: true,
            grid: grid(),
            column_surface_elevation_m: 1_200.0,
            solar: solar(),
            terrain: DownscalingTerrain::Full(FullTerrain {
                slope_radians: &slope,
                aspect_radians: &aspect,
                area_fraction: &area,
                sky_view_factor: 0.75,
                blue_sky_albedo: f64::NAN,
                shadow: ShadowMask::Curve(&curve),
            }),
        },
        ForcingDownscalingConfig {
            longwave: LongwaveDownscaling::ClearSky,
            precipitation: PrecipitationDownscaling::ListonElder,
            ..ForcingDownscalingConfig::default()
        },
    )
    .unwrap();
    close(output.downward_longwave_w_m2, 279.864_623_540_246_5);
    close(output.downward_shortwave_w_m2, 250.0);
    close(output.convective_precipitation_kg_m2_s, 0.0);
    close(output.large_scale_precipitation_kg_m2_s, 0.0);
}

#[test]
fn wind_and_flat_simple_forcing_match_the_fortran_edge_behavior() {
    let full_slope = [0.1, -0.2, 0.0, 0.3];
    let full_aspect = [0.0, 1.0, 2.0, 3.0];
    let full_area = [0.2, 0.3, 0.1, 0.4];
    let (u, v) = downscale_wind(2.3, -1.1, &full_slope, &full_aspect, &full_area, 0.15).unwrap();
    close(u, 2.306_299_314_720_372);
    close(v, -1.103_012_715_735_830_5);

    let simple_slope = [0.0; ASPECT_TYPES];
    let simple_area = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let (u, v) = downscale_wind_simple(2.3, -1.1, &simple_slope, &simple_area, 0.0).unwrap();
    close(u, 2.3);
    close(v, -1.1);
}

#[test]
fn lookup_shadow_bins_follow_fortran_one_based_indices() {
    let mut table = [[0.0; ZENITH_BINS]; AZIMUTH_BINS];
    table[1][50] = 0.42;
    close(
        full_shadow_factor(0.7_f64.acos(), 0.4_f64.acos(), ShadowMask::Lookup(&table)),
        0.42,
    );
}

#[test]
fn invalid_physical_inputs_are_rejected_at_the_shared_boundary() {
    let slope = [0.0; ASPECT_TYPES];
    let area = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let mut invalid = grid();
    invalid.bottom_pressure_pa = 0.0;
    assert!(downscale_forcings(
        ForcingDownscalingInput {
            glacier: false,
            grid: invalid,
            column_surface_elevation_m: 500.0,
            solar: solar(),
            terrain: DownscalingTerrain::Simple(SimpleTerrain {
                slope_tangent: &slope,
                area_fraction: &area,
            }),
        },
        ForcingDownscalingConfig::default(),
    )
    .is_err());
}
