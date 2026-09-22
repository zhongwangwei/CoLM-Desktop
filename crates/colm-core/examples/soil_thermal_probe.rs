//! `MOD_SoilThermalParameters:soil_hcap_cond` 的差分探针（与
//! `oracle/scripts/soil_hcap_cond_diff.f90` 配对；8 档方案 × (2500 均匀随机 +
//! 2500 边界取值)）。
//!
//! 抽签次数与顺序必须与 Fortran 侧逐条对齐，改一边就得同步改另一边。
//! 输出的 `flag` 标出上游 `sr >= 1e-10` 那道门是否成立：不成立时上游只把 `ke`
//! 置 0，`thk` 的赋值在门外的 `:422-517`，仍然有定义。`flag` 只用来确认干土
//! 路径被抽到过，不参与筛选。
use colm_core::{soil_thermal_properties, SoilThermalInput, ThermalConductivityScheme};

struct Lcg(u64);
impl Lcg {
    fn uni(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / 9007199254740992.0
    }

    /// 与 Fortran 侧 `P()` 同构：`i = MIN(INT(uni()*n)+1, n)`。
    fn pick(&mut self, pool: &[f64]) -> f64 {
        let index = ((self.uni() * pool.len() as f64) as usize).min(pool.len() - 1);
        pool[index]
    }
}

const P_VFP: &[f64] = &[0.05, 0.10, 0.25, 0.40, 0.60, 0.90];
const P_VFG: &[f64] = &[0.0, 0.001, 0.2, 0.5, 0.9];
const P_VFO: &[f64] = &[0.0, 0.05, 0.5];
const P_VFS: &[f64] = &[0.0, 0.005, 0.02, 0.25, 0.5];
const P_WF: &[f64] = &[0.0, 0.5, 1.0];
const P_KSOL: &[f64] = &[0.1, 1.0, 8.0];
const P_CSOL: &[f64] = &[1.0e5, 3.0e6];
const P_KD: &[f64] = &[0.0, 0.05, 0.5];
const P_KS: &[f64] = &[0.0, 0.5, 4.0];
const P_BA_A: &[f64] = &[0.0, 0.5];
const P_BA_B: &[f64] = &[1.0, 25.0];
const P_TG: &[f64] = &[200.0, 250.0, 273.15, 273.16, 300.0, 350.0, 273.0];
const P_FI: &[f64] = &[0.0, 1.0e-12, 0.25, 0.5, 1.0];
const P_FL: &[f64] = &[0.0, 1.0e-12, 0.01, 0.5, 1.0];

fn main() {
    let schemes = [
        ThermalConductivityScheme::Oleson,
        ThermalConductivityScheme::Johansen,
        ThermalConductivityScheme::CoteKonrad,
        ThermalConductivityScheme::BallandArp,
        ThermalConductivityScheme::Lu,
        ThermalConductivityScheme::TarnawskiLeong,
        ThermalConductivityScheme::DeVries,
        ThermalConductivityScheme::YanHe,
    ];
    let mut rng = Lcg(20250507);
    let mut out = String::new();
    for scheme in schemes {
        for record in 1..=5000 {
            let (
                gravel_volume_fraction_of_solids,
                organic_volume_fraction_of_solids,
                sand_volume_fraction_of_solids,
                pore_volume_fraction,
                gravel_mass_fraction,
                sand_mass_fraction,
                solid_conductivity_w_m_k,
                dry_heat_capacity_j_m3_k,
                dry_conductivity_w_m_k,
                saturated_unfrozen_conductivity_w_m_k,
                saturated_frozen_conductivity_w_m_k,
                balland_alpha,
                balland_beta,
                temperature_k,
                ice_volume_fraction,
                liquid_volume_fraction,
            ) = if record <= 2500 {
                let gravel_volume_fraction_of_solids = rng.uni() * 0.4;
                let organic_volume_fraction_of_solids = rng.uni() * 0.3;
                let sand_volume_fraction_of_solids = rng.uni() * 0.3;
                let pore_volume_fraction = 0.25 + rng.uni() * 0.4;
                let gravel_mass_fraction = rng.uni();
                let sand_mass_fraction = rng.uni();
                let solid_conductivity_w_m_k = 1.0 + rng.uni() * 7.0;
                let dry_heat_capacity_j_m3_k = 1.0e6 + rng.uni() * 2.0e6;
                let dry_conductivity_w_m_k = 0.05 + rng.uni() * 0.45;
                let saturated_unfrozen_conductivity_w_m_k = 0.3 + rng.uni() * 2.7;
                let saturated_frozen_conductivity_w_m_k = 0.5 + rng.uni() * 3.5;
                let balland_alpha = 0.1 + rng.uni() * 0.4;
                let balland_beta = 5.0 + rng.uni() * 20.0;
                let temperature_k = 230.0 + rng.uni() * 90.0;
                let ice_volume_fraction = rng.uni() * pore_volume_fraction * 0.5;
                let liquid_volume_fraction =
                    rng.uni() * (pore_volume_fraction - ice_volume_fraction);
                (
                    gravel_volume_fraction_of_solids,
                    organic_volume_fraction_of_solids,
                    sand_volume_fraction_of_solids,
                    pore_volume_fraction,
                    gravel_mass_fraction,
                    sand_mass_fraction,
                    solid_conductivity_w_m_k,
                    dry_heat_capacity_j_m3_k,
                    dry_conductivity_w_m_k,
                    saturated_unfrozen_conductivity_w_m_k,
                    saturated_frozen_conductivity_w_m_k,
                    balland_alpha,
                    balland_beta,
                    temperature_k,
                    ice_volume_fraction,
                    liquid_volume_fraction,
                )
            } else {
                let pore_volume_fraction = rng.pick(P_VFP);
                let gravel_volume_fraction_of_solids = rng.pick(P_VFG);
                let organic_volume_fraction_of_solids = rng.pick(P_VFO);
                let sand_volume_fraction_of_solids = rng.pick(P_VFS);
                let gravel_mass_fraction = rng.pick(P_WF);
                let sand_mass_fraction = rng.pick(P_WF);
                let solid_conductivity_w_m_k = rng.pick(P_KSOL);
                let dry_heat_capacity_j_m3_k = rng.pick(P_CSOL);
                let dry_conductivity_w_m_k = rng.pick(P_KD);
                let saturated_unfrozen_conductivity_w_m_k = rng.pick(P_KS);
                let saturated_frozen_conductivity_w_m_k = rng.pick(P_KS);
                let balland_alpha = rng.pick(P_BA_A);
                let balland_beta = rng.pick(P_BA_B);
                let temperature_k = rng.pick(P_TG);
                let ice_volume_fraction = rng.pick(P_FI) * pore_volume_fraction;
                let liquid_volume_fraction =
                    rng.pick(P_FL) * (pore_volume_fraction - ice_volume_fraction);
                (
                    gravel_volume_fraction_of_solids,
                    organic_volume_fraction_of_solids,
                    sand_volume_fraction_of_solids,
                    pore_volume_fraction,
                    gravel_mass_fraction,
                    sand_mass_fraction,
                    solid_conductivity_w_m_k,
                    dry_heat_capacity_j_m3_k,
                    dry_conductivity_w_m_k,
                    saturated_unfrozen_conductivity_w_m_k,
                    saturated_frozen_conductivity_w_m_k,
                    balland_alpha,
                    balland_beta,
                    temperature_k,
                    ice_volume_fraction,
                    liquid_volume_fraction,
                )
            };
            let properties = soil_thermal_properties(
                SoilThermalInput {
                    gravel_volume_fraction_of_solids,
                    organic_volume_fraction_of_solids,
                    sand_volume_fraction_of_solids,
                    pore_volume_fraction,
                    gravel_mass_fraction,
                    sand_mass_fraction,
                    solid_conductivity_w_m_k,
                    dry_heat_capacity_j_m3_k,
                    dry_conductivity_w_m_k,
                    saturated_unfrozen_conductivity_w_m_k,
                    saturated_frozen_conductivity_w_m_k,
                    balland_alpha,
                    balland_beta,
                    temperature_k,
                    liquid_volume_fraction,
                    ice_volume_fraction,
                },
                scheme,
            )
            .expect("validated inputs");
            let saturation = (liquid_volume_fraction + ice_volume_fraction) / pore_volume_fraction;
            let flag = u8::from(saturation >= 1.0e-10);
            out.push_str(&format!(
                "{}{} {:016X} {:016X}\n",
                scheme_number(scheme),
                flag,
                properties.heat_capacity_j_m3_k.to_bits(),
                properties.conductivity_w_m_k.to_bits()
            ));
        }
    }
    let directory = std::env::var("HC_OUT").unwrap_or_else(|_| "/tmp/gf/hc".to_string());
    std::fs::write(format!("{directory}/hc_rust.txt"), out).unwrap();
    println!("done");
}

fn scheme_number(scheme: ThermalConductivityScheme) -> u8 {
    match scheme {
        ThermalConductivityScheme::Oleson => 1,
        ThermalConductivityScheme::Johansen => 2,
        ThermalConductivityScheme::CoteKonrad => 3,
        ThermalConductivityScheme::BallandArp => 4,
        ThermalConductivityScheme::Lu => 5,
        ThermalConductivityScheme::TarnawskiLeong => 6,
        ThermalConductivityScheme::DeVries => 7,
        ThermalConductivityScheme::YanHe => 8,
    }
}
