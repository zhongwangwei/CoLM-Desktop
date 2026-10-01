//! 漫滩水面通量的基本性质；逐位对照在端到端算例里做（`g1ff`）。

use super::*;

fn input(surface_temperature_k: f64) -> FloodEvaporationInput {
    FloodEvaporationInput {
        wind_height_m: 30.0,
        temperature_height_m: 30.0,
        humidity_height_m: 30.0,
        wind_east_m_s: 3.0,
        wind_north_m_s: -1.0,
        air_temperature_k: 295.0,
        specific_humidity_kg_kg: 0.010,
        air_density_kg_m3: 1.18,
        surface_pressure_pa: 100_500.0,
        surface_temperature_k,
        boundary_layer_height_m: 1000.0,
        scheme: SurfaceLayerScheme::Standard,
    }
}

#[test]
fn warm_water_under_dry_air_evaporates_and_heats_the_air() {
    let flux = flood_evaporation(input(300.0)).unwrap();
    assert!(flux.evaporation_mm_s > 0.0, "{flux:?}");
    assert!(flux.sensible_heat_w_m2 > 0.0, "{flux:?}");
    assert!(flux.stability < 0.0, "unstable over warm water: {flux:?}");
    assert!(flux.friction_velocity_m_s > 0.0);
    // 应力方向与风向相反。
    assert!(flux.taux < 0.0 && flux.tauy > 0.0);
    assert!(flux.bulk_richardson <= 5.0);
}

#[test]
fn cold_water_is_stable_and_takes_heat_from_the_air() {
    let flux = flood_evaporation(input(285.0)).unwrap();
    assert!(flux.sensible_heat_w_m2 < 0.0, "{flux:?}");
    assert!(flux.stability > 0.0 && flux.stability <= 2.0, "{flux:?}");
    // 2 m 温度落在水面与参考高度之间。
    assert!(flux.reference_temperature_k > 285.0 && flux.reference_temperature_k < 295.3);
}
