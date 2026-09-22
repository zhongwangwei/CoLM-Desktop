//! `MOD_TurbulenceLEddy`（LZD2022 大涡近地层方案）的差分探针。
//!
//! 配对物 `oracle/scripts/turbulence_leddy_diff.f90`，两侧共用同一串 LCG；
//! 输出写到 `$LEDDY_OUT`（默认 `/tmp/gf/leddy_diff/`），由
//! `compare_leddy.sh` 逐位比对 17 个输出。抽签次数与顺序必须与上游驱动对齐。
use colm_core::{
    canopy_monin_obukhov_with_scheme, monin_obukhov_with_scheme, CanopyMoninObukhovInput,
    MoninObukhovInput, SurfaceLayerScheme,
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
    let mut rng = Lcg(20250508);
    let mut out = String::new();
    for _ in 0..20000 {
        let displa = rng.uni() * 5.0;
        let z0m = 0.001 + rng.uni();
        let z0h = 0.001 + rng.uni();
        let z0q = 0.001 + rng.uni();
        let z0mt = 0.001 + rng.uni();
        let htop = displa + 1.0 + rng.uni() * 20.0;
        let mut displat = displa + rng.uni() * (htop - displa);
        if displat + z0mt <= displa {
            displat = displa + z0mt + 0.5;
        }
        let hu = displa + 1.0 + rng.uni() * 40.0;
        let ht = displa + 1.0 + rng.uni() * 40.0;
        let hq = displa + 1.0 + rng.uni() * 40.0;
        let um = 0.5 + rng.uni() * 10.0;
        let obu = if rng.uni() < 0.5 {
            -(5.0 + rng.uni() * 500.0)
        } else {
            5.0 + rng.uni() * 500.0
        };
        let hpbl = 10.0 + rng.uni() * 2990.0;
        let surface = MoninObukhovInput {
            wind_height_m: hu,
            temperature_height_m: ht,
            humidity_height_m: hq,
            displacement_height_m: displa,
            momentum_roughness_m: z0m,
            heat_roughness_m: z0h,
            moisture_roughness_m: z0q,
            obukhov_length_m: obu,
            stability_adjusted_wind_m_s: um,
            boundary_layer_height_m: Some(hpbl),
        };
        let plain = monin_obukhov_with_scheme(surface, SurfaceLayerScheme::LargeEddy)
            .expect("validated inputs");
        let canopy = canopy_monin_obukhov_with_scheme(
            CanopyMoninObukhovInput {
                surface,
                top_layer_displacement_m: displat,
                top_layer_roughness_m: z0mt,
                canopy_top_height_m: htop,
            },
            SurfaceLayerScheme::LargeEddy,
        )
        .expect("validated inputs");
        let values = [
            plain.friction_velocity_m_s,
            plain.heat_at_2m,
            plain.moisture_at_2m,
            plain.momentum_at_10m,
            plain.momentum,
            plain.heat,
            plain.moisture,
            canopy.surface.friction_velocity_m_s,
            canopy.surface.heat_at_2m,
            canopy.surface.moisture_at_2m,
            canopy.momentum_at_canopy_top,
            canopy.surface.momentum,
            canopy.surface.heat,
            canopy.surface.moisture,
            canopy.heat_at_top_layer,
            canopy.moisture_at_top_layer,
            canopy.canopy_top_heat_similarity,
        ];
        for value in values {
            out.push_str(&format!("{:016X} ", value.to_bits()));
        }
        out.push('\n');
    }
    let directory = std::env::var("LEDDY_OUT").unwrap_or_else(|_| "/tmp/gf/leddy_diff".to_string());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(format!("{directory}/leddy_rust.txt"), out).unwrap();
    println!("done");
}
