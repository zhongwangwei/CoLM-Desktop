//! `MOD_Qsadv:qsadv` 的差分探针（与 `oracle/scripts/qsadv_diff.f90` 配对）。
use colm_core::saturation_specific_humidity;

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
    let mut rng = Lcg(20250506);
    let mut out = String::new();
    for _ in 0..20000 {
        let tmin = std::env::var("QS_TMIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(150.0);
        let tmax = std::env::var("QS_TMAX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(350.0);
        let temperature_k = tmin + rng.uni() * (tmax - tmin);
        let pressure_pa = 30000.0 + rng.uni() * 80000.0;
        let state = saturation_specific_humidity(temperature_k, pressure_pa).expect("valid");
        let values = [
            temperature_k,
            pressure_pa,
            state.vapor_pressure_pa,
            state.vapor_pressure_temperature_slope_pa_k,
            state.specific_humidity,
            state.specific_humidity_temperature_slope_k,
        ];
        for value in values {
            out.push_str(&format!("{:016X} ", value.to_bits()));
        }
        out.push('\n');
    }
    std::fs::write("/tmp/gf/qs_diff/qs_rust.txt", out).unwrap();
    println!("done");
}
