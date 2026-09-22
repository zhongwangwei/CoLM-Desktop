//! `MOD_FrictionVelocity:moninobukm` 的差分探针（与上游驱动 `oracle/scripts/moninobukm_diff.f90` 配对）。
//!
//! 两侧用同一串 LCG；输出写到 `/tmp/gf/mo_diff/`，由 `compare_moninobukm.sh` 逐位比对。
use colm_core::{
    canopy_monin_obukhov_with_scheme, CanopyMoninObukhovInput, MoninObukhovInput,
    SurfaceLayerScheme,
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
    let mut rng = Lcg(20250505);
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
        let state = canopy_monin_obukhov_with_scheme(
            CanopyMoninObukhovInput {
                surface: MoninObukhovInput {
                    wind_height_m: hu,
                    temperature_height_m: ht,
                    humidity_height_m: hq,
                    displacement_height_m: displa,
                    momentum_roughness_m: z0m,
                    heat_roughness_m: z0h,
                    moisture_roughness_m: z0q,
                    obukhov_length_m: obu,
                    stability_adjusted_wind_m_s: um,
                    boundary_layer_height_m: None,
                },
                top_layer_displacement_m: displat,
                top_layer_roughness_m: z0mt,
                canopy_top_height_m: htop,
            },
            SurfaceLayerScheme::Standard,
        )
        .expect("validated inputs");
        let values = [
            state.surface.friction_velocity_m_s,
            state.surface.heat_at_2m,
            state.surface.moisture_at_2m,
            state.momentum_at_canopy_top,
            state.surface.momentum,
            state.surface.heat,
            state.surface.moisture,
            state.heat_at_top_layer,
            state.moisture_at_top_layer,
            state.canopy_top_heat_similarity,
        ];
        for value in values {
            out.push_str(&format!("{:016X} ", value.to_bits()));
        }
        out.push('\n');
    }
    std::fs::write("/tmp/gf/mo_diff/mo_rust.txt", out).unwrap();
    println!("done");
}
