//! `MOD_ForcingDownscaling:downscale_forcings` full 支（`downscale_shortwave`）
//! 的差分探针。
//!
//! 配对物 `oracle/scripts/forcingdownscaling_shortwave_diff.f90`（上游侧直接链接
//! 内核 `.bld` 对象），两侧共用同一串 LCG；输出写到 `$FD_OUT`（默认
//! `/tmp/gf/fd_diff/`），由 `compare_forcingdownscaling_shortwave.sh` 逐位比对。
//!
//! 阴影表有**两套**（与上游驱动、与链接的模块对象一致），由 `FD_VARIANT` 选：
//! `lut`（SinglePoint，16×101，1616 个值）或 `curve`（网格内核，16×3，48 个值）。
//! 两套都由 LCG 抽 —— 解析式会在两侧的 libm 之间差 1 ULP，不能用来对齐输入。
//! 抽签次数与顺序必须与上游驱动逐条对齐。
// 上游驱动里写的是 `6.28318_r8`，这里必须逐位复刻同一个字面量。
#![allow(clippy::approx_constant)]

use colm_core::{
    downscale_forcings, DownscalingSolarGeometry, DownscalingTerrain, ForcingDownscalingConfig,
    ForcingDownscalingInput, FullTerrain, GridForcing, LongwaveDownscaling,
    PrecipitationDownscaling, ShadowMask, AZIMUTH_BINS, SHADOW_CURVE_PARAMETERS, ZENITH_BINS,
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

// 抽签要按上游的**列主序**填表（curve 是 parameter 在外、lut 是 zenith 在外），
// 用迭代器改写会丢掉这个顺序，差分立刻失效 —— 所以这里显式按下标写。
#[allow(clippy::needless_range_loop)]
fn main() {
    let mut rng = Lcg(20250511);
    let mut out = String::new();
    for index in 1..=2000usize {
        let surface_elevation_m = rng.uni() * 3000.0;
        let maximum_elevation_m = rng.uni() * 3000.0;
        let air_temperature_k = 250.0 + rng.uni() * 70.0;
        let potential_temperature_k = 250.0 + rng.uni() * 80.0;
        let specific_humidity = 0.0001 + rng.uni() * 0.02;
        let bottom_pressure_pa = 60000.0 + rng.uni() * 41325.0;
        let density_kg_m3 = 0.7 + rng.uni() * 0.7;
        let convective_precipitation_kg_m2_s = rng.uni() * 0.001;
        let large_scale_precipitation_kg_m2_s = rng.uni() * 0.001;
        let downward_longwave_w_m2 = 100.0 + rng.uni() * 300.0;
        let reference_height_m = 10.0 + rng.uni() * 90.0;
        let mut downward_shortwave_w_m2 = rng.uni() * 1000.0;
        if index % 7 == 0 {
            downward_shortwave_w_m2 = 0.0;
        }
        if index % 13 == 0 {
            downward_shortwave_w_m2 = rng.uni() * 1.0e-5;
        }
        let eastward_wind_m_s = -10.0 + rng.uni() * 20.0;
        let northward_wind_m_s = -10.0 + rng.uni() * 20.0;
        let column_surface_elevation_m = rng.uni() * 3000.0;
        let calendar_day = 1.0 + rng.uni() * 364.0;
        let mut cosine_zenith = 0.05 + rng.uni() * 0.95;
        if index % 11 == 0 {
            cosine_zenith = 0.0;
        }
        let cosine_azimuth = -1.0 + rng.uni() * 2.0;
        let _curvature = -2.0 + rng.uni() * 2.0;
        let mut slope_radians = [0.0; 4];
        let mut aspect_radians = [0.0; 4];
        let mut area_fraction = [0.0; 4];
        for k in 0..4 {
            slope_radians[k] = rng.uni() * 1.5;
            aspect_radians[k] = rng.uni() * 6.28318;
            area_fraction[k] = rng.uni();
        }
        let temperature_lapse_rate_k_m = rng.uni() * 0.01;
        let glacier_longwave_lapse_rate_w_m2_m = rng.uni() * 0.05;
        let longwave_limit = rng.uni();
        let full_shortwave_limit = rng.uni();
        let simple_shortwave_limit = rng.uni();
        let sky_view_factor = -0.2 + rng.uni() * 1.4;
        let mut blue_sky_albedo = rng.uni();
        if index % 4 == 0 {
            blue_sky_albedo = f64::NAN;
        }
        let variant = std::env::var("FD_VARIANT").unwrap_or_else(|_| "lut".to_string());
        let mut shadow_table = [[0.0; ZENITH_BINS]; AZIMUTH_BINS];
        let mut shadow_curves = [[0.0; SHADOW_CURVE_PARAMETERS]; AZIMUTH_BINS];
        if variant == "curve" {
            for parameter in 0..SHADOW_CURVE_PARAMETERS {
                for azimuth in 0..AZIMUTH_BINS {
                    shadow_curves[azimuth][parameter] = rng.uni();
                }
            }
        } else {
            for zenith in 0..ZENITH_BINS {
                for azimuth in 0..AZIMUTH_BINS {
                    shadow_table[azimuth][zenith] = rng.uni();
                }
            }
        }
        let shadow = if variant == "curve" {
            ShadowMask::Curve(&shadow_curves)
        } else {
            ShadowMask::Lookup(&shadow_table)
        };
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
        let config = ForcingDownscalingConfig {
            temperature_lapse_rate_k_m,
            glacier_longwave_lapse_rate_w_m2_m,
            longwave_limit,
            full_shortwave_limit,
            simple_shortwave_limit,
            longwave: LongwaveDownscaling::LapseRate,
            precipitation: PrecipitationDownscaling::ElevationFraction,
        };
        let output = downscale_forcings(
            ForcingDownscalingInput {
                glacier,
                grid,
                column_surface_elevation_m,
                solar: DownscalingSolarGeometry {
                    calendar_day,
                    cosine_zenith,
                    cosine_azimuth,
                },
                terrain: DownscalingTerrain::Full(FullTerrain {
                    slope_radians: &slope_radians,
                    aspect_radians: &aspect_radians,
                    area_fraction: &area_fraction,
                    sky_view_factor,
                    blue_sky_albedo,
                    shadow,
                }),
            },
            config,
        )
        .expect("validated inputs");
        out.push_str(&format!(
            "{:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} \
             {:016X} {:016X}\n",
            output.air_temperature_k.to_bits(),
            output.potential_temperature_k.to_bits(),
            output.specific_humidity.to_bits(),
            output.bottom_pressure_pa.to_bits(),
            output.density_kg_m3.to_bits(),
            output.convective_precipitation_kg_m2_s.to_bits(),
            output.large_scale_precipitation_kg_m2_s.to_bits(),
            output.downward_longwave_w_m2.to_bits(),
            output.downward_shortwave_w_m2.to_bits(),
            output.eastward_wind_m_s.to_bits(),
            output.northward_wind_m_s.to_bits(),
        ));
    }
    let directory = std::env::var("FD_OUT").unwrap_or_else(|_| "/tmp/gf/fd_diff".to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let suffix = std::env::var("FD_VARIANT").unwrap_or_else(|_| "lut".to_string());
    std::fs::write(format!("{directory}/fdsw_rust_{suffix}.txt"), out).unwrap();
    println!("done");
}
