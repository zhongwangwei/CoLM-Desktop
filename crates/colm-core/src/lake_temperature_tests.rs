use super::*;

/// 十层 0.1 m 湖 + CoLM 标准十层湖底饱和土，无雪。
fn column(
    lake_temperature_k: f64,
    ice_fraction: f64,
) -> (LakeColumn, GlacierColumn, Vec<SoilThermalInput>) {
    let grid = crate::colm_soil_grid(10).unwrap();
    let porosity = 0.45;
    let soil_temperature = 280.0;
    let lake = LakeColumn {
        thickness_m: vec![0.1; 10],
        temperature_k: vec![lake_temperature_k; 10],
        ice_fraction: vec![ice_fraction; 10],
    };
    let column = GlacierColumn {
        thickness_m: grid.thickness_m.clone(),
        node_depth_m: grid.node_depth_m.clone(),
        interface_depth_m: grid.interface_depth_m.clone(),
        temperature_k: vec![soil_temperature; 10],
        liquid_water_kg_m2: grid
            .thickness_m
            .iter()
            .map(|dz| dz * porosity * DENH2O)
            .collect(),
        ice_water_kg_m2: vec![0.0; 10],
    };
    let soil = vec![
        SoilThermalInput {
            gravel_volume_fraction_of_solids: 0.0,
            organic_volume_fraction_of_solids: 0.02,
            sand_volume_fraction_of_solids: 0.4,
            pore_volume_fraction: porosity,
            gravel_mass_fraction: 0.0,
            sand_mass_fraction: 0.4,
            solid_conductivity_w_m_k: 3.0,
            dry_heat_capacity_j_m3_k: 1.2e6,
            dry_conductivity_w_m_k: 0.25,
            saturated_unfrozen_conductivity_w_m_k: 1.8,
            saturated_frozen_conductivity_w_m_k: 2.7,
            balland_alpha: 0.24,
            balland_beta: 18.1,
            temperature_k: soil_temperature,
            liquid_volume_fraction: porosity,
            ice_volume_fraction: 0.0,
        };
        10
    ];
    (lake, column, soil)
}

fn input(
    soil: &[SoilThermalInput],
    air_temperature_k: f64,
    shortwave: f64,
) -> LakeTemperatureInput<'_> {
    LakeTemperatureInput {
        time_step_seconds: 1800.0,
        latitude_radians: 47.0_f64.to_radians(),
        wind_height_m: 10.0,
        temperature_height_m: 10.0,
        humidity_height_m: 10.0,
        eastward_wind_m_s: 3.0,
        northward_wind_m_s: 1.0,
        air_temperature_k,
        specific_humidity: 4.0e-3,
        air_density_kg_m3: 1.2,
        surface_pressure_pa: 95_000.0,
        shortwave: ShortwaveForcing {
            direct_visible_w_m2: 0.3 * shortwave,
            direct_near_infrared_w_m2: 0.3 * shortwave,
            diffuse_visible_w_m2: 0.2 * shortwave,
            diffuse_near_infrared_w_m2: 0.2 * shortwave,
        },
        absorbed_shortwave_w_m2: 0.9 * shortwave,
        downward_longwave_w_m2: 300.0,
        boundary_layer_height_m: None,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
        lake_depth_m: 1.0,
        thermal_conductivity_scheme: ThermalConductivityScheme::Johansen,
        soil_thermal_inputs: soil,
        snow_layers: 0,
    }
}

/// 开水面：没有相变，湖面温度落在气温与水温之间，潜热等于 `hvap*fevpg`，
/// 能量闭合修正之后 `fsena` 与 `fseng` 一致（`laketem` 把残差并进感热）。
#[test]
fn open_water_stays_liquid_and_closes_its_budget_into_sensible_heat() {
    let (mut lake, mut column, soil) = column(284.0, 0.0);
    let mut saved_tke = 0.6;
    let mut ground = 284.0;
    let mut scv = 0.0;
    let mut snowdp = 0.0;
    let output = lake_temperature(
        input(&soil, 278.0, 400.0),
        LakeTemperatureState {
            lake: &mut lake,
            saved_tke: &mut saved_tke,
            ground_temperature_k: &mut ground,
            snow_water_equivalent_kg_m2: &mut scv,
            snow_depth_m: &mut snowdp,
            column: &mut column,
        },
    )
    .unwrap();
    let fluxes = output.fluxes;
    assert!(lake.ice_fraction.iter().all(|f| *f == 0.0));
    assert!(output.phase_flag.iter().all(|flag| *flag == 0));
    assert!(ground > 278.0 && ground < 285.0, "t_grnd = {ground}");
    assert_eq!(fluxes.lfevpa, HVAP * fluxes.fevpg);
    assert_eq!(fluxes.emis, LAKE_EMISSIVITY);
    assert_eq!(fluxes.fsena, fluxes.fseng);
    // `savedtke1 = kme(1)*cwat`：开水面时它至少是分子导热率 `tkwat`。
    assert!(saved_tke >= 0.6, "savedtke1 = {saved_tke}");
    assert!(fluxes.fseng > 0.0 && fluxes.fevpg > 0.0);
    assert_eq!(fluxes.sm, 0.0);
}

/// 冰点附近的湖遇上很冷的空气：表层结冰（`lake_icefrac(1)` 增加），湖面温度被钳在冰点以下，
/// 冰面的潜热用升华热。
#[test]
fn a_cold_night_freezes_the_top_lake_layer() {
    let (mut lake, mut column, soil) = column(273.2, 0.0);
    let mut saved_tke = 0.6;
    let mut ground = 273.2;
    let mut scv = 0.0;
    let mut snowdp = 0.0;
    let mut output = None;
    for _ in 0..48 {
        output = Some(
            lake_temperature(
                input(&soil, 250.0, 0.0),
                LakeTemperatureState {
                    lake: &mut lake,
                    saved_tke: &mut saved_tke,
                    ground_temperature_k: &mut ground,
                    snow_water_equivalent_kg_m2: &mut scv,
                    snow_depth_m: &mut snowdp,
                    column: &mut column,
                },
            )
            .unwrap(),
        );
    }
    let fluxes = output.unwrap().fluxes;
    assert!(
        lake.ice_fraction[0] > 0.0,
        "icefrac = {:?}",
        lake.ice_fraction
    );
    assert!(ground <= TFRZ, "t_grnd = {ground}");
    assert_eq!(fluxes.lfevpa, HSUB * fluxes.fevpg);
    // 冰只从顶上长：底层比顶层冻得少。
    assert!(lake.ice_fraction[9] <= lake.ice_fraction[0]);
}

/// 列的形状不一致时拒绝，而不是越界或静默截断。
#[test]
fn an_inconsistent_column_is_rejected() {
    let (mut lake, mut column, soil) = column(284.0, 0.0);
    column.ice_water_kg_m2.pop();
    let mut saved_tke = 0.6;
    let mut ground = 284.0;
    let mut scv = 0.0;
    let mut snowdp = 0.0;
    assert!(lake_temperature(
        input(&soil, 278.0, 0.0),
        LakeTemperatureState {
            lake: &mut lake,
            saved_tke: &mut saved_tke,
            ground_temperature_k: &mut ground,
            snow_water_equivalent_kg_m2: &mut scv,
            snow_depth_m: &mut snowdp,
            column: &mut column,
        },
    )
    .is_err());
}
