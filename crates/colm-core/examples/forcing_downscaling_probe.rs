//! `MOD_ForcingDownscaling` 两个风场降尺度入口的差分探针。
//!
//! 配对物 `oracle/scripts/forcingdownscaling_wind_diff.f90`（上游侧直接链接内核
//! `.bld/*.o`，跑的是真正的产线对象），两侧共用同一串 LCG；输出写到 `$FD_OUT`
//! （默认 `/tmp/gf/fd_diff/`），由 `compare_forcingdownscaling_wind.sh` 逐位比对。
//! 抽签次数与顺序必须与上游驱动逐条对齐。
// 上游驱动里写的是 `6.28318_r8`，这里必须逐位复刻同一个字面量 —— 换成
// `std::f64::consts::TAU` 会让两侧的输入不再对齐，差分就白跑了。
#![allow(clippy::approx_constant)]

use colm_core::{downscale_wind, downscale_wind_simple};

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
    let mut rng = Lcg(20250509);
    let mut out = String::new();
    for index in 1..=20000usize {
        let mut us = -10.0 + rng.uni() * 20.0;
        if index % 37 == 0 {
            us = 0.0;
        }
        let vs = -10.0 + rng.uni() * 20.0;
        let mut curvature = -2.0 + rng.uni() * 4.0;
        if index % 17 == 0 {
            curvature = -1.0e36;
        }
        let mut slope_full = [0.0; 4];
        let mut aspect_full = [0.0; 4];
        let mut area_full = [0.0; 4];
        for k in 0..4 {
            slope_full[k] = rng.uni() * 1.5;
            aspect_full[k] = rng.uni() * 6.28318;
            area_full[k] = rng.uni();
        }
        let mut slope_simple = [0.0; 9];
        let mut area_simple = [0.0; 9];
        for k in 0..9 {
            slope_simple[k] = rng.uni() * 1.5;
            area_simple[k] = rng.uni();
        }
        if index % 11 == 0 {
            slope_simple[index % 9] = -1.0e36;
        }
        if index % 13 == 0 {
            area_simple[index % 9] = -1.0e36;
        }
        let (full_east, full_north) =
            downscale_wind(us, vs, &slope_full, &aspect_full, &area_full, curvature)
                .expect("validated inputs");
        let (simple_east, simple_north) =
            downscale_wind_simple(us, vs, &slope_simple, &area_simple, curvature)
                .expect("validated inputs");
        out.push_str(&format!(
            "{:016X} {:016X} {:016X} {:016X}\n",
            full_east.to_bits(),
            full_north.to_bits(),
            simple_east.to_bits(),
            simple_north.to_bits()
        ));
    }
    let directory = std::env::var("FD_OUT").unwrap_or_else(|_| "/tmp/gf/fd_diff".to_string());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(format!("{directory}/fdwind_rust.txt"), out).unwrap();
    println!("done");
}
