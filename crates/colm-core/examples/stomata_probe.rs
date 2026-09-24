//! `MOD_AssimStomataConductance:stomata` 的差分探针（配对物
//! `oracle/scripts/stomata_diff.f90`）。
//!
//! 第 329 轮普查：这条链在核心里有 25 条 FMA，本仓库一处 `mul_add` 都没有。
//! 这个闭环在合成输入上逐位判 `assim`/`respc`/`rst`。
//!
//! 抽签次数与顺序必须与 Fortran 侧逐条对齐（改一边就得同步改另一边）。
//! 三种气孔模型各 1000 例：k=0 Ball-Berry、k=1 Medlyn、k=2 WUE。
use colm_core::{stomata, LeafBiochemistry, LeafPhotosynthesisInput, StomataInput, StomataOptions};

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
    let mut s = Lcg(20260924);
    let mut out = String::new();
    for k in 0..3u32 {
        let options = StomataOptions {
            use_medlyn: k == 1,
            use_wue: k == 2,
            ..StomataOptions::default()
        };
        for _ in 1..=1000u32 {
            let vmax25 = 10.0 + s.uni() * 80.0;
            let effcon = 0.01 + s.uni() * 0.08;
            let mut c3c4 = if s.uni() < 0.5 { 1 } else { 0 };
            if k == 2 {
                c3c4 = 1;
            }
            let slti = 0.1 + s.uni() * 0.3;
            let hlti = 280.0 + s.uni() * 20.0;
            let shti = 0.2 + s.uni() * 0.3;
            let hhti = 300.0 + s.uni() * 30.0;
            let trda = 1.0 + s.uni() * 0.5;
            let trdm = 320.0 + s.uni() * 20.0;
            let trop = 298.16;
            let g1 = 1.0 + s.uni() * 10.0;
            let g0 = s.uni() * 0.1;
            let gradm = 5.0 + s.uni() * 15.0;
            let binter = s.uni() * 0.05;
            let tm = 250.0 + s.uni() * 60.0;
            let psrf = 60_000.0 + s.uni() * 50_000.0;
            let po2m = psrf * 0.209;
            let pco2m = 30.0 + s.uni() * 60.0;
            let pco2a = pco2m * (0.9 + s.uni() * 0.15);
            let ea = s.uni() * 2000.0;
            let ei = 100.0 + s.uni() * 4000.0;
            let tlef = 250.0 + s.uni() * 60.0;
            let par = s.uni() * 300.0;
            let _o3coefv = s.uni();
            let _o3coefg = s.uni();
            let lambda = 100.0 + s.uni() * 5000.0;
            let rb = 10.0 + s.uni() * 200.0;
            let _ra = 10.0 + s.uni() * 200.0;
            let rstfac = s.uni();
            let cint = [s.uni(), s.uni(), s.uni()];
            let _ = tm;

            let state = stomata(
                StomataInput {
                    photosynthesis: LeafPhotosynthesisInput {
                        biochemistry: LeafBiochemistry {
                            quantum_efficiency: effcon,
                            maximum_carboxylation_25c_mol_m2_s: vmax25,
                            c3c4,
                            low_temperature_slope: slti,
                            low_temperature_half_k: hlti,
                            high_temperature_slope: shti,
                            high_temperature_half_k: hhti,
                            respiration_temperature_slope: trda,
                            respiration_temperature_half_k: trdm,
                            optimum_temperature_k: trop,
                            medlyn_g1: g1,
                            medlyn_g0: g0,
                            ball_berry_slope: gradm,
                            ball_berry_intercept: binter,
                        },
                        canopy_integration: cint,
                        leaf_temperature_k: tlef,
                        oxygen_partial_pressure_pa: po2m,
                        absorbed_par_w_m2: par,
                        air_pressure_pa: psrf,
                        soil_water_stress: rstfac,
                        leaf_boundary_resistance_s_m: rb,
                    },
                    atmospheric_co2_pa: pco2m,
                    canopy_air_co2_pa: pco2a,
                    canopy_air_vapor_pressure_pa: ea,
                    leaf_saturation_vapor_pressure_pa: ei,
                    wue_lambda: lambda,
                },
                options,
            )
            .unwrap_or_else(|error| panic!("k={k}: {error}"));

            out.push_str(&format!(
                "{k:2} {:016X} {:016X} {:016X}\n",
                state.assimilation_mol_m2_s.to_bits(),
                state.respiration_mol_m2_s.to_bits(),
                state.stomatal_resistance_s_m.to_bits(),
            ));
        }
    }
    print!("{out}");
}
