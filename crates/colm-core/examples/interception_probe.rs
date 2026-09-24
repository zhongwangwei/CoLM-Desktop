//! `LEAF_interception_CoLM2014` 的差分探针（配对物
//! `oracle/scripts/interception_diff.f90`）。
//!
//! 第 326 轮修好 `p0`/`pinf`/`:328-329` 之后，剩下的 12 条 FMA 只影响
//! `ldew_rain`/`ldew_snow` 这些 history 不导出的分量，所以只能在这个闭环里逐位判。
//!
//! 抽签次数与顺序必须与 Fortran 侧逐条对齐（改一边就得同步改另一边）：
//! 每例 20 步（见 `interception_diff.f90` 注释）。`vegetation_snow` 在 k=0/1 两档各跑一遍。
//!
//! 输出：`k` 后 8 列十六进制 —— `ldew`/`ldew_rain`/`ldew_snow`（更新后）、
//! `pg_rain`/`pg_snow`/`qintr`/`qintr_rain`/`qintr_snow`。
use colm_core::{intercept_canopy, CanopyInterceptionInput, CanopyWater};

struct Lcg(u64);
impl Lcg {
    fn uni(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / 9007199254740992.0
    }
    /// 与 Fortran 的 `uni2()` 对齐：先抽 `a`、再抽 `b`、然后相乘。
    fn uni2(&mut self) -> f64 {
        let a = self.uni();
        let b = self.uni();
        a * b
    }
}

fn main() {
    let mut s = Lcg(20260924);
    let mut out = String::new();
    for k in 0..2u32 {
        let vegetation_snow = k == 1;
        for _ in 1..=2000u32 {
            let time_step_seconds = 60.0 + s.uni() * 3540.0;
            let maximum_dew_mm = 0.05 + s.uni() * 0.45;
            let eastward_wind_m_s = (s.uni() - 0.5) * 20.0;
            let northward_wind_m_s = (s.uni() - 0.5) * 20.0;
            let leaf_angle_distribution = -1.0 + s.uni() * 2.0;
            let _sigf = s.uni();
            let (leaf_area_index, stem_area_index) = if s.uni() < 0.1 {
                (0.0, 0.0)
            } else {
                (s.uni() * 6.0, s.uni() * 2.0)
            };
            let _tair = 250.0 + s.uni() * 60.0;
            let leaf_temperature_k = 250.0 + s.uni() * 60.0;
            let convective_rain_kg_m2_s = s.uni2() * 2.0e-4;
            let convective_snow_kg_m2_s = s.uni2() * 2.0e-4;
            let large_scale_rain_kg_m2_s = s.uni2() * 2.0e-4;
            let large_scale_snow_kg_m2_s = s.uni2() * 2.0e-4;
            let sprinkler_irrigation_kg_m2_s = s.uni() * 1.0e-5;
            let _bifall = 50.0 + s.uni() * 300.0;
            let rain_mm = s.uni() * 0.3;
            let snow_mm = s.uni() * 0.3;
            let total_mm = if s.uni() < 0.5 {
                rain_mm + snow_mm
            } else {
                s.uni() * 0.6
            };
            let _z0m = 0.01 + s.uni() * 1.0;
            let _hu = 1.0 + s.uni() * 40.0;

            let mut water = CanopyWater {
                total_mm,
                rain_mm,
                snow_mm,
            };
            let flux = intercept_canopy(
                CanopyInterceptionInput {
                    time_step_seconds,
                    maximum_dew_mm,
                    eastward_wind_m_s,
                    northward_wind_m_s,
                    leaf_angle_distribution,
                    leaf_area_index,
                    stem_area_index,
                    leaf_temperature_k,
                    convective_rain_kg_m2_s,
                    convective_snow_kg_m2_s,
                    large_scale_rain_kg_m2_s,
                    large_scale_snow_kg_m2_s,
                    sprinkler_irrigation_kg_m2_s,
                    vegetation_snow,
                },
                &mut water,
            )
            .unwrap_or_else(|error| panic!("k={k}: {error}"));

            out.push_str(&format!(
                "{k:2} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X}\n",
                water.total_mm.to_bits(),
                water.rain_mm.to_bits(),
                water.snow_mm.to_bits(),
                flux.ground_rain_kg_m2_s.to_bits(),
                flux.ground_snow_kg_m2_s.to_bits(),
                flux.retained_kg_m2_s.to_bits(),
                flux.retained_rain_kg_m2_s.to_bits(),
                flux.retained_snow_kg_m2_s.to_bits(),
            ));
        }
    }
    print!("{out}");
}
