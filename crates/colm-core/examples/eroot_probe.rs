//! `MOD_Eroot:eroot` 的差分探针（配对物 `oracle/scripts/eroot_diff.f90`）。
//!
//! 为什么要这个闭环：`eroot` 的两个输入里只有 `rstfac` 被黄金窗口走到，
//! `rootr`（逐层根阻力份额）与 `etrc`（最大可能蒸腾率）只在
//! `DEF_USE_PLANTHYDRAULICS = .false.` 那一支里用 —— 三个黄金算例全开植物水力，
//! 所以这两个输出在本机从来没有端到端信号。
//!
//! 覆盖：Campbell / van Genuchten × `DEF_RSTFAC` 1/2 ×（1000 组均匀随机 +
//! 1000 组边界/零值）。**抽签次数与顺序必须与 Fortran 侧逐条对齐**：
//! 边界那一批只抽 `trsmx0` 一次，逐层字段全走 `MERGE`（不抽签）。
use colm_core::{root_uptake, RootUptakeInput, SoilHydraulicModel};

const NL: usize = 10;

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
    let mut s = Lcg(20260925);
    let mut out = String::new();
    for k in 0..4usize {
        let campbell = k % 2 == 0;
        let stress_scheme = 1 + (k / 2) as i32;
        for i in 1..=2000usize {
            let trsmx0 = s.uni() * 1.0e-6;
            let mut porsl = [0.0; NL];
            let mut bsw = [0.0; NL];
            let mut theta_r = [0.0; NL];
            let mut alpha = [0.0; NL];
            let mut n_vgm = [0.0; NL];
            let mut l_vgm = [0.0; NL];
            let mut sc_vgm = [0.0; NL];
            let mut fc_vgm = [0.0; NL];
            let mut psi0 = [0.0; NL];
            let mut rootfr = [0.0; NL];
            let mut dz = [0.0; NL];
            let mut t_soisno = [0.0; NL];
            let mut wliq = [0.0; NL];
            for j in 0..NL {
                if i <= 1000 {
                    porsl[j] = 0.35 + s.uni() * 0.25;
                    bsw[j] = 2.0 + s.uni() * 10.0;
                    theta_r[j] = 0.02 + s.uni() * 0.10;
                    alpha[j] = 5.0e-4 + s.uni() * 7.5e-3;
                    n_vgm[j] = 1.2 + s.uni() * 1.5;
                    l_vgm[j] = 0.5 + s.uni() * 1.5;
                    sc_vgm[j] = 0.1 + s.uni() * 0.7;
                    fc_vgm[j] = 0.1 + s.uni() * 0.8;
                    psi0[j] = -(10.0 + s.uni() * 990.0);
                    rootfr[j] = s.uni();
                    dz[j] = 0.05 + s.uni() * 0.5;
                    t_soisno[j] = 250.0 + s.uni() * 70.0;
                    wliq[j] = s.uni() * porsl[j] * dz[j] * 1000.0;
                } else {
                    // 边界批的下标条件照抄 Fortran 的 **1-based** `j`：
                    // `MERGE(..., MOD(j,2)==0)`、`MOD(j,3)==0` 里的 j 是 1..nl，
                    // 直接用 0-based 的 j 会让两侧拿到**不同的输入**（第一版就踩了）。
                    let even = (j + 1) % 2 == 0;
                    let third = (j + 1) % 3 == 0;
                    porsl[j] = if even { 0.30 } else { 0.60 };
                    bsw[j] = if even { 2.0 } else { 12.0 };
                    theta_r[j] = if even { 0.0 } else { 0.12 };
                    alpha[j] = if even { 5.0e-4 } else { 8.0e-3 };
                    n_vgm[j] = if even { 1.2 } else { 2.7 };
                    l_vgm[j] = if even { 0.5 } else { 2.0 };
                    sc_vgm[j] = if even { 0.1 } else { 0.8 };
                    fc_vgm[j] = if even { 0.1 } else { 0.9 };
                    psi0[j] = if even { -10.0 } else { -1000.0 };
                    rootfr[j] = if third { 0.0 } else { 1.0 };
                    dz[j] = if even { 0.05 } else { 0.55 };
                    t_soisno[j] = if even { 260.0 } else { 300.0 };
                    wliq[j] = if third {
                        0.0
                    } else {
                        porsl[j] * dz[j] * 1000.0
                    };
                }
            }
            let model: Vec<SoilHydraulicModel> = (0..NL)
                .map(|j| {
                    if campbell {
                        SoilHydraulicModel::Campbell { bsw: bsw[j] }
                    } else {
                        SoilHydraulicModel::VanGenuchten {
                            alpha_vgm: alpha[j],
                            n_vgm: n_vgm[j],
                            l_vgm: l_vgm[j],
                            sc_vgm: sc_vgm[j],
                            fc_vgm: fc_vgm[j],
                        }
                    }
                })
                .collect();
            let state = root_uptake(RootUptakeInput {
                maximum_transpiration_mm_s: trsmx0,
                porosity: &porsl,
                residual_water: &theta_r,
                saturated_soil_suction_mm: &psi0,
                hydraulic_model: &model,
                root_fraction: &rootfr,
                layer_thickness_m: &dz,
                temperature_k: &t_soisno,
                liquid_water_kg_m2: &wliq,
                stress_scheme,
            })
            .unwrap_or_else(|error| panic!("k={k} i={i}: {error}"));
            out.push_str(&format!("{k:2}"));
            for value in &state.layer_fraction {
                out.push_str(&format!(" {:016X}", value.to_bits()));
            }
            out.push_str(&format!(
                " {:016X} {:016X}\n",
                state.maximum_transpiration_mm_s.to_bits(),
                state.soil_water_stress.to_bits()
            ));
        }
    }
    print!("{out}");
}
