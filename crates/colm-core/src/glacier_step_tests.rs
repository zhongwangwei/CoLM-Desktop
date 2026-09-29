use super::*;

/// 十层裸冰柱：`GLACIER_TEMP` 的能量闭合误差 `errore` 应在舍入量级，
/// 且冰温低于冰点时没有相变、`sm = 0`。
#[test]
fn a_bare_ice_column_closes_its_energy_budget() {
    let layers: usize = 10;
    let thickness: Vec<f64> = (0..layers).map(|j| 0.02 * 1.5_f64.powi(j as i32)).collect();
    let mut interface = vec![0.0; layers + 1];
    for j in 0..layers {
        interface[j + 1] = interface[j] + thickness[j];
    }
    let node: Vec<f64> = (0..layers)
        .map(|j| 0.5 * (interface[j] + interface[j + 1]))
        .collect();
    let column = GlacierColumn {
        thickness_m: thickness.clone(),
        node_depth_m: node,
        interface_depth_m: interface,
        temperature_k: vec![265.0; layers],
        liquid_water_kg_m2: vec![0.0; layers],
        ice_water_kg_m2: thickness.iter().map(|dz| dz * DENICE).collect(),
    };
    let porosity = vec![0.5; layers];
    let residual = vec![0.0; layers];
    let suction = vec![-100.0; layers];
    let model = vec![crate::SoilHydraulicModel::Campbell { bsw: 5.0 }; layers];
    let output = glacier_temperature(
        GlacierTemperatureInput {
            time_step_seconds: 1800.0,
            surface_temperature_factor: 0.34,
            crank_nicolson_factor: 0.5,
            wind_height_m: 10.0,
            temperature_height_m: 10.0,
            humidity_height_m: 10.0,
            eastward_wind_m_s: 3.0,
            northward_wind_m_s: 1.0,
            air_temperature_k: 262.0,
            specific_humidity: 1.5e-3,
            boundary_layer_height_m: None,
            air_density_kg_m3: 1.3,
            surface_pressure_pa: 85_000.0,
            absorbed_shortwave_w_m2: 120.0,
            downward_longwave_w_m2: 220.0,
            snow_cover_fraction: 0.0,
            rainfall_kg_m2_s: 0.0,
            snowfall_kg_m2_s: 0.0,
            precipitation_temperature_k: 262.0,
            snow_layers: 0,
            snow_water_equivalent_kg_m2: 0.0,
            snow_depth_m: 0.0,
            surface_layer_scheme: SurfaceLayerScheme::Standard,
            split_soil_snow: false,
            supercool_water: false,
            soil_porosity: &porosity,
            soil_residual_water: &residual,
            soil_suction_mm: &suction,
            soil_hydraulic_model: &model,
            snow_layer_absorption_w_m2: None,
        },
        column,
    )
    .unwrap();
    let fluxes = output.fluxes;
    assert!(fluxes.errore.abs() < 1.0e-6, "errore = {}", fluxes.errore);
    assert_eq!(fluxes.sm, 0.0);
    assert!(output.phase_flag.iter().all(|flag| *flag == 0));
    assert_eq!(fluxes.emis, GLACIER_EMISSIVITY);
    // 冰面：`htvp = hsub`，`lfevpa = htvp*fevpg`。
    assert_eq!(fluxes.lfevpa, fluxes.fevpg * HSUB);
    assert!(output.column.temperature_k.iter().all(|t| *t < TFRZ));
}
