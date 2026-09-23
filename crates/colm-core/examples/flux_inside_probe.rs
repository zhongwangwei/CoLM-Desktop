//! `flux_inside_hm_soil` 的差分探针（配对物 `oracle/scripts/flux_inside_diff.f90`）。
//!
//! 抽签次数与顺序必须与 Fortran 侧逐条对齐：每例 `psi_s, hksat, bsw, alpha, n, L,
//! sc, fc, dz, psi_u, psi_l, hk_u, hk_l` 共 13 次（边界分支是 13 次 `pick`）。
//! 输出三列十六进制：`flux`、`grad_psi`（本地算，确认六条分支都覆盖）、`hk_u`。
use colm_core::VariableSaturatedHomogeneousFluxInput;
use colm_core::{flux_inside_variable_saturated_soil, SoilHydraulicModel};

struct Lcg(u64);
impl Lcg {
    fn uni(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / 9007199254740992.0
    }
    fn pick(&mut self, pool: &[f64]) -> f64 {
        let index = ((self.uni() * pool.len() as f64) as usize).min(pool.len() - 1);
        pool[index]
    }
}

const P_PSIS: &[f64] = &[-1.0e4, -1.0e3, -100.0, -5.0e3, -50.0];
const P_HK: &[f64] = &[1.0e-6, 1.0e-5, 1.0e-4, 1.0e-3, 5.0e-5];
const P_BSW: &[f64] = &[2.0, 4.0, 7.0, 11.0, 14.0];
const P_ALPHA: &[f64] = &[1.0e-3, 5.0e-3, 1.0e-2, 5.0e-2, 0.1];
const P_N: &[f64] = &[1.1, 1.5, 2.0, 2.5, 3.0];
const P_L: &[f64] = &[0.5, 0.7, 0.9, 1.0, 0.6];
const P_SC: &[f64] = &[0.2, 0.4, 0.6, 0.8, 1.0];
const P_FC: &[f64] = &[0.2, 0.4, 0.6, 0.8, 1.0];
const P_DZ: &[f64] = &[1.0e-3, 1.0e-2, 0.1, 0.5, 1.0];
const P_PSIU: &[f64] = &[-1.0e4, -1.0e3, -100.0, -1.0, 0.0];
const P_PSIL: &[f64] = &[-1.0e4, -1.0e3, -100.0, -1.0, 0.0];
const P_HKU: &[f64] = &[1.0e-8, 1.0e-6, 1.0e-5, 1.0e-4, 1.0e-3];

fn main() {
    let mut s = Lcg(20260810);
    let mut out = String::new();
    for k in 0..2u32 {
        let campbell = k == 1;
        for i in 1..=6000u32 {
            let (psi_s, hksat, bsw, alpha, n, l, sc, fc, dz, psi_u, psi_l, hk_u, hk_l);
            if i <= 3000 {
                psi_s = -(100.0 + s.uni() * 1.0e4);
                hksat = 1.0e-6 + s.uni() * 1.0e-3;
                bsw = 2.0 + s.uni() * 12.0;
                alpha = 1.0e-3 + s.uni() * 0.1;
                n = 1.05 + s.uni() * 2.0;
                l = 0.5 + s.uni() * 0.5;
                sc = 0.2 + s.uni() * 0.8;
                fc = 0.2 + s.uni() * 0.8;
                dz = 1.0e-3 + s.uni() * 1.0;
                psi_u = -(1.0 + s.uni() * 1.0e4);
                psi_l = -(1.0 + s.uni() * 1.0e4);
                hk_u = 1.0e-9 + s.uni() * 1.0e-3;
                hk_l = 1.0e-9 + s.uni() * 1.0e-3;
            } else {
                psi_s = s.pick(P_PSIS);
                hksat = s.pick(P_HK);
                bsw = s.pick(P_BSW);
                alpha = s.pick(P_ALPHA);
                n = s.pick(P_N);
                l = s.pick(P_L);
                sc = s.pick(P_SC);
                fc = s.pick(P_FC);
                dz = s.pick(P_DZ);
                psi_u = s.pick(P_PSIU);
                psi_l = s.pick(P_PSIL);
                hk_u = s.pick(P_HKU);
                hk_l = s.pick(P_HKU);
            }
            let model = if campbell {
                SoilHydraulicModel::Campbell { bsw }
            } else {
                SoilHydraulicModel::VanGenuchten {
                    alpha_vgm: alpha,
                    n_vgm: n,
                    l_vgm: l,
                    sc_vgm: sc,
                    fc_vgm: fc,
                }
            };
            let grad_psi = 1.0 - (psi_l - psi_u) / dz;
            let flag: i32 = if grad_psi < 0.0 {
                1
            } else if grad_psi == 0.0 {
                2
            } else if grad_psi < 1.0 {
                3
            } else if grad_psi == 1.0 {
                4
            } else {
                5
            };
            let flux = flux_inside_variable_saturated_soil(VariableSaturatedHomogeneousFluxInput {
                saturated_potential_mm: psi_s,
                saturated_hydraulic_conductivity_mm_s: hksat,
                hydraulic_model: model,
                distance_mm: dz,
                upper_pressure_head_mm: psi_u,
                lower_pressure_head_mm: psi_l,
                upper_hydraulic_conductivity_mm_s: hk_u,
                lower_hydraulic_conductivity_mm_s: hk_l,
            })
            .unwrap_or_else(|error| panic!("k={k} i={i}: {error}"));
            out.push_str(&format!(
                "{k:2} {flag} {:016X} {:016X} {:016X}\n",
                flux.to_bits(),
                grad_psi.to_bits(),
                hk_u.to_bits()
            ));
        }
    }
    print!("{out}");
}
