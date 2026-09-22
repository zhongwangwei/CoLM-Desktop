//! `MOD_ForcingDownscaling:downscale_forcings` 的差分探针（简单地形那一支）。
//!
//! 配对物 `oracle/scripts/forcingdownscaling_full_diff.f90`（上游侧直接链接内核
//! `.bld` 对象，`DEF_DS_*` 是真的 namelist 变量），两侧共用同一串 LCG；
//! 输出写到 `$FD_OUT`（默认 `/tmp/gf/fd_diff/`），由
//! `compare_forcingdownscaling.sh` 逐位比对 11 个输出 × 4 组配置。
//!
//! **`area_fraction` 收的是「坡向」数组**：上游 `downscale_forcings` 的简单支把
//! `asp_type_patches` 传给了被调方的 `area_type_c`（`MOD_ForcingDownscaling.F90:289-293`
//! 对 `:878-911` 的形参名），所以这里必须照抄这个别名，否则两边输入不一致。
//!
//! 抽签次数与顺序必须与上游驱动逐条对齐。
// 上游驱动里写的是 `6.28318_r8`，这里必须逐位复刻同一个字面量。
#![allow(clippy::approx_constant)]

use colm_core::{
    downscale_forcings, DownscaledForcing, DownscalingSolarGeometry, DownscalingTerrain,
    ForcingDownscalingConfig, ForcingDownscalingInput, GridForcing, LongwaveDownscaling,
    PrecipitationDownscaling, SimpleTerrain,
};

struct Lcg(u64);
impl Lcg {
    fn uni(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / 9007199254740992.0
    }
}

fn main() {
    let mut rng = Lcg(20250510);
    let mut out = String::new();
    for index in 1..=5000usize {
        let surface_elevation_m = rng.uni() * 3000.0;
        let mut maximum_elevation_m = rng.uni() * 3000.0;
        if index % 7 == 0 {
            maximum_elevation_m = 0.0;
        }
        let air_temperature_k = 250.0 + rng.uni() * 70.0;
        let potential_temperature_k = 250.0 + rng.uni() * 80.0;
        let specific_humidity = 0.0001 + rng.uni() * 0.02;
        let bottom_pressure_pa = 60000.0 + rng.uni() * 41325.0;
        let density_kg_m3 = 0.7 + rng.uni() * 0.7;
        let convective_precipitation_kg_m2_s = rng.uni() * 0.001;
        let large_scale_precipitation_kg_m2_s = rng.uni() * 0.001;
        let downward_longwave_w_m2 = 100.0 + rng.uni() * 300.0;
        let reference_height_m = 10.0 + rng.uni() * 90.0;
        let downward_shortwave_w_m2 = rng.uni() * 1000.0;
        let eastward_wind_m_s = -10.0 + rng.uni() * 20.0;
        let northward_wind_m_s = -10.0 + rng.uni() * 20.0;
        let column_surface_elevation_m = rng.uni() * 3000.0;
        let calendar_day = 1.0 + rng.uni() * 364.0;
        let cosine_zenith = 0.05 + rng.uni() * 0.95;
        let cosine_azimuth = -1.0 + rng.uni() * 2.0;
        let _curvature = -2.0 + rng.uni() * 2.0;
        // 与上游驱动同序：slope/aspect **交错**抽（上游是一个循环里连抽两个）。
        let mut slope_tangent = [0.0; 9];
        let mut aspect = [0.0; 9];
        for k in 0..9 {
            slope_tangent[k] = rng.uni() * 1.5;
            aspect[k] = rng.uni() * 6.28318;
        }
        let temperature_lapse_rate_k_m = rng.uni() * 0.01;
        let glacier_longwave_lapse_rate_w_m2_m = rng.uni() * 0.05;
        let longwave_limit = rng.uni();
        let full_shortwave_limit = rng.uni();
        let simple_shortwave_limit = rng.uni();
        let glacier = index % 3 == 0;
        let grid = GridForcing {
            surface_elevation_m,
            maximum_elevation_m,
            air_temperature_k,
            potential_temperature_k,
            specific_humidity,
            bottom_pressure_pa,
            density_kg_m3,
            convective_precipitation_kg_m2_s,
            large_scale_precipitation_kg_m2_s,
            downward_longwave_w_m2,
            reference_height_m,
            downward_shortwave_w_m2,
            eastward_wind_m_s,
            northward_wind_m_s,
        };
        for (ip, precipitation) in [
            (1_u8, PrecipitationDownscaling::ElevationFraction),
            (2_u8, PrecipitationDownscaling::ListonElder),
        ] {
            for (il, longwave) in [
                (1_u8, LongwaveDownscaling::ClearSky),
                (2_u8, LongwaveDownscaling::LapseRate),
            ] {
                let config = ForcingDownscalingConfig {
                    temperature_lapse_rate_k_m,
                    glacier_longwave_lapse_rate_w_m2_m,
                    longwave_limit,
                    full_shortwave_limit,
                    simple_shortwave_limit,
                    longwave,
                    precipitation,
                };
                let DownscaledForcing {
                    air_temperature_k,
                    potential_temperature_k,
                    specific_humidity,
                    bottom_pressure_pa,
                    density_kg_m3,
                    convective_precipitation_kg_m2_s,
                    large_scale_precipitation_kg_m2_s,
                    downward_longwave_w_m2,
                    downward_shortwave_w_m2,
                    eastward_wind_m_s,
                    northward_wind_m_s,
                } = downscale_forcings(
                    ForcingDownscalingInput {
                        glacier,
                        grid,
                        column_surface_elevation_m,
                        solar: DownscalingSolarGeometry {
                            calendar_day,
                            cosine_zenith,
                            cosine_azimuth,
                        },
                        terrain: DownscalingTerrain::Simple(SimpleTerrain {
                            slope_tangent: &slope_tangent,
                            area_fraction: &aspect,
                        }),
                    },
                    config,
                )
                .expect("validated inputs");
                out.push_str(&format!(
                    "{ip}{il} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} \
                     {:016X} {:016X} {:016X} {:016X}\n",
                    air_temperature_k.to_bits(),
                    potential_temperature_k.to_bits(),
                    specific_humidity.to_bits(),
                    bottom_pressure_pa.to_bits(),
                    density_kg_m3.to_bits(),
                    convective_precipitation_kg_m2_s.to_bits(),
                    large_scale_precipitation_kg_m2_s.to_bits(),
                    downward_longwave_w_m2.to_bits(),
                    downward_shortwave_w_m2.to_bits(),
                    eastward_wind_m_s.to_bits(),
                    northward_wind_m_s.to_bits(),
                ));
            }
        }
    }
    let directory = std::env::var("FD_OUT").unwrap_or_else(|_| "/tmp/gf/fd_diff".to_string());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(format!("{directory}/fdfull_rust.txt"), out).unwrap();
    println!("done");
}
