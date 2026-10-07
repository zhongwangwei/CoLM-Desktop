use super::*;

#[test]
fn climate_files_follow_the_constant_restart_blocks() {
    let case = Path::new("/case");
    assert_eq!(
        climate_file(
            case,
            Path::new("/o/zb/restart/const/zb_restart_const_lc2005_e100_n20.nc")
        )
        .unwrap(),
        Path::new("/case/hybrid_climate/climate_e100_n20.nc")
    );
    assert_eq!(
        climate_file(
            case,
            Path::new("/o/CA-Qfo/restart/const/CA-Qfo_restart_const_lc2005.nc")
        )
        .unwrap(),
        Path::new("/case/hybrid_climate/climate.nc")
    );
    // 算例名里也有 `_lc`：取最后一个。
    assert_eq!(
        climate_file(case, Path::new("x_lcd_restart_const_lc2010_w180_s90.nc")).unwrap(),
        Path::new("/case/hybrid_climate/climate_w180_s90.nc")
    );
    assert!(climate_file(case, Path::new("restart.nc")).is_err());
}

#[test]
fn accumulated_climate_matches_hand_values() {
    let mut acc = ClimateAccumulator::default();
    let sample = |month, temperature_k, precipitation| ClimateSample {
        month,
        temperature_k,
        specific_humidity: 0.0,
        pressure_pa: 100_000.0,
        precipitation_kg_m2_s: precipitation,
        shortwave_w_m2: 200.0,
    };
    acc.add(sample(1, 263.15, 0.0));
    acc.add(sample(7, 293.15, 2.0 / 86400.0));
    acc.add(sample(7, 303.15, 0.0));
    let [tair, amplitude, prec, sw, vpd] = acc.finish().unwrap();
    assert!((tair - 286.483_333_333_333_3).abs() < 1e-9);
    // 一月 263.15，七月 298.15。
    assert!((amplitude - 35.0).abs() < 1e-9);
    assert!((prec - 2.0 / 3.0).abs() < 1e-12);
    assert_eq!(sw, 200.0);
    // 比湿为 0：饱和差就是饱和水汽压。20 °C 时约 2.338 kPa。
    let expected = (saturation_vapour_pressure_kpa(263.15)
        + saturation_vapour_pressure_kpa(293.15)
        + saturation_vapour_pressure_kpa(303.15))
        / 3.0;
    assert!((vpd - expected).abs() < 1e-12);
    assert!((saturation_vapour_pressure_kpa(293.15) - 2.338).abs() < 1e-3);
    assert!(ClimateAccumulator::default().finish().is_err());
}

#[test]
fn humid_air_has_a_smaller_deficit_and_never_a_negative_one() {
    let base = ClimateSample {
        month: 5,
        temperature_k: 298.15,
        specific_humidity: 0.0,
        pressure_pa: 101_325.0,
        precipitation_kg_m2_s: 0.0,
        shortwave_w_m2: 0.0,
    };
    let deficit = |q| {
        let mut acc = ClimateAccumulator::default();
        acc.add(ClimateSample {
            specific_humidity: q,
            ..base
        });
        acc.finish().unwrap()[4]
    };
    assert!(deficit(0.01) < deficit(0.0));
    // 过饱和（比湿超过饱和）截到 0。
    assert_eq!(deficit(0.05), 0.0);
}
