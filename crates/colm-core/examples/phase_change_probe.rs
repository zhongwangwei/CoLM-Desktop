//! `MOD_PhaseChange:meltf` 的差分探针（与 `oracle/scripts/phasechange_diff.f90` 配对）。
//!
//! 两侧共用同一串 LCG；抽签**次数与顺序**必须逐条对齐。上游驱动的写法有两处容易抄错：
//! 1) `fact(lb:NL) = 100 + uni()*2000` 是**标量右值**，只抽 1 次却填满 6 个元素；
//! 2) `t(lb:NL) = t_bef(lb:NL) + (-0.5+uni())*2` 同理只抽 1 次。
//!
//! 所以每个 case 是 6（数组填充）+ 40（4 层 × 10 参数）+ 7（标量）= **53 次抽签**。
//!
//! 配置：`nsnow = 2`、`nl_soil = 4`（上游 `lb = 1-nsnow = -1`，数组共 6 层，雪在上）；
//! 水力模型为该 namelist 默认值 —— 实测为 **van Genuchten**（见 docs 里那一节）。
use colm_core::{phase_change, PhaseChangeInput, SoilHydraulicModel};

const TOTAL: usize = 6; // lb..nl_soil = -1..4
const SOIL: usize = 4;

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
    let mut rng = Lcg(20250512);
    let mut out = String::new();
    for patch_type in 0..=4i32 {
        for _ in 0..2000 {
            let fact_value = 100.0 + rng.uni() * 2000.0;
            let brr_value = -50.0 + rng.uni() * 100.0;
            let previous_value = 250.0 + rng.uni() * 40.0;
            let delta = (-0.5 + rng.uni()) * 2.0;
            let wliq_value = rng.uni() * 5.0;
            let wice_value = rng.uni() * 5.0;
            let mut porosity = [0.0; SOIL];
            let mut suction = [0.0; SOIL];
            let mut bsw = [0.0; SOIL];
            let mut residual = [0.0; SOIL];
            let mut thickness = [0.0; SOIL];
            let mut alpha = [0.0; SOIL];
            let mut n_vgm = [0.0; SOIL];
            let mut l_vgm = [0.0; SOIL];
            let mut sc_vgm = [0.0; SOIL];
            let mut fc_vgm = [0.0; SOIL];
            for k in 0..SOIL {
                porosity[k] = 0.3 + rng.uni() * 0.3;
                suction[k] = -(10.0 + rng.uni() * 300.0);
                bsw[k] = 2.0 + rng.uni() * 8.0;
                residual[k] = 0.02 + rng.uni() * 0.08;
                thickness[k] = 0.05 + rng.uni() * 0.4;
                alpha[k] = 0.005 + rng.uni() * 0.05;
                n_vgm[k] = 1.2 + rng.uni() * 1.5;
                l_vgm[k] = 0.3 + rng.uni() * 0.7;
                sc_vgm[k] = 0.02 + rng.uni() * 0.08;
                fc_vgm[k] = 0.1 + rng.uni() * 0.3;
            }
            let surface_heat_flux = -100.0 + rng.uni() * 200.0;
            let soil_heat_flux = -100.0 + rng.uni() * 200.0;
            let snow_heat_flux = -100.0 + rng.uni() * 200.0;
            let snow_cover_fraction = rng.uni();
            let derivative = -20.0 + rng.uni() * 20.0;
            let snow_water_equivalent = rng.uni() * 20.0;
            let snow_depth = rng.uni() * 0.3;

            let state = phase_change(PhaseChangeInput {
                patch_type,
                is_dry_lake: false,
                time_step_seconds: 1800.0,
                fact_seconds_per_j_m2_k: &[fact_value; TOTAL],
                residual_heat_flux_w_m2: &[brr_value; TOTAL],
                snow_layer_absorption_w_m2: None,
                surface_heat_flux_w_m2: surface_heat_flux,
                soil_heat_flux_w_m2: soil_heat_flux,
                snow_heat_flux_w_m2: snow_heat_flux,
                snow_cover_fraction,
                surface_heat_flux_temperature_derivative_w_m2_k: derivative,
                previous_temperature_k: &[previous_value; TOTAL],
                temperature_k: &[previous_value + delta; TOTAL],
                liquid_water_kg_m2: &[wliq_value; TOTAL],
                ice_water_kg_m2: &[wice_value; TOTAL],
                snow_water_equivalent_kg_m2: snow_water_equivalent,
                snow_depth_m: snow_depth,
                snow_layers: 2,
                split_soil_snow: false,
                // 实测 `.bld` namelist 默认：campbell=F（走 VGM）、supercool=T、split=F
                supercool_water: true,
                soil_layer_thickness_m: &thickness,
                soil_porosity: &porosity,
                soil_residual_water: &residual,
                soil_suction_mm: &suction,
                soil_hydraulic_model: &std::array::from_fn::<_, SOIL, _>(|k| {
                    SoilHydraulicModel::VanGenuchten {
                        alpha_vgm: alpha[k],
                        n_vgm: n_vgm[k],
                        l_vgm: l_vgm[k],
                        sc_vgm: sc_vgm[k],
                        fc_vgm: fc_vgm[k],
                    }
                }),
            })
            .expect("validated inputs");

            let thaw: f64 = state.thaw_mass_kg_m2.iter().sum();
            let freeze: f64 = state.freeze_mass_kg_m2.iter().sum();
            let flagged: i32 = state.phase_flag.iter().sum();
            out.push_str(&format!(
                "{patch_type}2 {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} {:016X} \
                 {flagged:8}\n",
                state.temperature_k[0].to_bits(),
                state.liquid_water_kg_m2[0].to_bits(),
                state.ice_water_kg_m2[0].to_bits(),
                state.snow_water_equivalent_kg_m2.to_bits(),
                state.snow_melt_rate_kg_m2_s.to_bits(),
                state.latent_heat_flux_w_m2.to_bits(),
                thaw.to_bits(),
                freeze.to_bits(),
            ));
        }
    }
    let directory = std::env::var("PC_OUT").unwrap_or_else(|_| "/tmp/gf/pc_diff".to_string());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(format!("{directory}/pc_rust.txt"), out).unwrap();
    println!("done");
}
