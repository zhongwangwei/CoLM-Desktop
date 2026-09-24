//! `MOD_Hydro_SoilWater:get_water_equilibrium_state` 的差分探针
//! （与 `oracle/scripts/water_equilibrium_diff.f90` 配对）。
//!
//! 这个例程只在冷启动（`MOD_IniTimeVariable.F90:463` 的 `use_wtd` 分支）被调用，
//! `colm.x` 运行时一次都不进 ⇒ 黄金窗口与干湿窗首分歧都对它不敏感，
//! 形状对不对只能靠这个闭环判。两侧共用同一串 LCG：
//! 每例先抽 1 次定 `nlev`、1 次定 `zwtmm`，再逐层抽 11 次（10 个层参数 + 1 个层厚），
//! 两档模型各 4000 例；抽签次数与顺序必须逐条对齐，改一边就得同步改另一边。
//!
//! 打印：`k flag nlev izwt` + `wa` + 逐层 `wliq / smp / hk`（十六进制位型）。
use colm_core::{equilibrium_water_state, SoilHydraulicModel};

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

const MAXLEV: usize = 10;
const P_NLEV: &[f64] = &[1.0, 2.0, 3.0, 5.0, 10.0];
const P_ZWT: &[f64] = &[0.0, 1.0, 50.0, 500.0, 2000.0, 20000.0];
const P_DZ: &[f64] = &[10.0, 50.0, 100.0, 200.0, 500.0, 2000.0];
const P_POR: &[f64] = &[0.30, 0.40, 0.50, 0.60, 0.45];
const P_VLR: &[f64] = &[0.02, 0.05, 0.10, 0.15, 1.0e-9];
const P_PSIS: &[f64] = &[-1.0e4, -1.0e3, -100.0, -5.0e3, -50.0];
const P_HK: &[f64] = &[1.0e-6, 1.0e-5, 1.0e-4, 1.0e-3, 5.0e-5];
const P_BSW: &[f64] = &[2.0, 4.0, 7.0, 11.0, 14.0];
const P_ALPHA: &[f64] = &[1.0e-3, 5.0e-3, 1.0e-2, 5.0e-2, 0.1];
const P_N: &[f64] = &[1.1, 1.5, 2.0, 2.5, 3.0];
const P_L: &[f64] = &[0.5, 0.7, 0.9, 1.0, 0.6];
const P_SC: &[f64] = &[0.2, 0.4, 0.6, 0.8, 1.0];
const P_FC: &[f64] = &[0.2, 0.4, 0.6, 0.8, 1.0];

fn main() {
    let mut s = Lcg(20260825);
    for k in 0..2u32 {
        let campbell = k == 1;
        for i in 1..=4000u32 {
            let mut interface = [0.0f64; MAXLEV + 1];
            let mut center = [0.0f64; MAXLEV];
            let mut porosity = [0.0f64; MAXLEV];
            let mut residual = [0.0f64; MAXLEV];
            let mut psi_s = [0.0f64; MAXLEV];
            let mut conductivity = [0.0f64; MAXLEV];
            let mut bsw = [0.0f64; MAXLEV];
            let mut alpha = [0.0f64; MAXLEV];
            let mut n = [0.0f64; MAXLEV];
            let mut l = [0.0f64; MAXLEV];
            let mut sc = [0.0f64; MAXLEV];
            let mut fc = [0.0f64; MAXLEV];
            let (nlev, water_table_mm);
            if i <= 2000 {
                nlev = s.pick(P_NLEV) as usize;
                water_table_mm = s.uni() * 20000.0;
                for layer in 0..nlev {
                    porosity[layer] = 0.30 + s.uni() * 0.30;
                    residual[layer] = 0.005 + s.uni() * 0.15;
                    psi_s[layer] = -(100.0 + s.uni() * 1.0e4);
                    conductivity[layer] = 1.0e-6 + s.uni() * 1.0e-3;
                    bsw[layer] = 2.0 + s.uni() * 12.0;
                    alpha[layer] = 1.0e-3 + s.uni() * 0.1;
                    n[layer] = 1.05 + s.uni() * 2.0;
                    l[layer] = 0.5 + s.uni() * 0.5;
                    sc[layer] = 0.2 + s.uni() * 0.8;
                    fc[layer] = 0.2 + s.uni() * 0.8;
                    interface[layer + 1] = interface[layer] + 100.0 + s.uni() * 900.0;
                }
            } else {
                nlev = s.pick(P_NLEV) as usize;
                water_table_mm = s.pick(P_ZWT);
                for layer in 0..nlev {
                    porosity[layer] = s.pick(P_POR);
                    residual[layer] = s.pick(P_VLR);
                    psi_s[layer] = s.pick(P_PSIS);
                    conductivity[layer] = s.pick(P_HK);
                    bsw[layer] = s.pick(P_BSW);
                    alpha[layer] = s.pick(P_ALPHA);
                    n[layer] = s.pick(P_N);
                    l[layer] = s.pick(P_L);
                    sc[layer] = s.pick(P_SC);
                    fc[layer] = s.pick(P_FC);
                    interface[layer + 1] = interface[layer] + s.pick(P_DZ);
                }
            }
            for layer in 0..nlev {
                center[layer] = (interface[layer] + interface[layer + 1]) * 0.5;
            }
            let model: Vec<SoilHydraulicModel> = (0..nlev)
                .map(|layer| {
                    if campbell {
                        SoilHydraulicModel::Campbell { bsw: bsw[layer] }
                    } else {
                        SoilHydraulicModel::VanGenuchten {
                            alpha_vgm: alpha[layer],
                            n_vgm: n[layer],
                            l_vgm: l[layer],
                            sc_vgm: sc[layer],
                            fc_vgm: fc[layer],
                        }
                    }
                })
                .collect();
            let state = equilibrium_water_state(
                water_table_mm,
                &center[..nlev],
                &interface[..nlev + 1],
                &porosity[..nlev],
                &residual[..nlev],
                &psi_s[..nlev],
                &conductivity[..nlev],
                &model,
            )
            .unwrap();
            // 诊断列：与 Fortran 驱动里 `findloc_ud(..., back=.true.)` 的等价写法。
            let water_layer = interface[..nlev + 1]
                .iter()
                .rposition(|&z| water_table_mm >= z)
                .unwrap_or(0)
                + 1;
            let flag = usize::from(water_layer == nlev + 1);
            print!("{k:3} {flag:3} {nlev:3} {water_layer:3}");
            print!(" {:016X}", state.aquifer_water_mm.to_bits());
            for layer in 0..nlev {
                print!(
                    " {:016X} {:016X} {:016X}",
                    state.liquid_water_kg_m2[layer].to_bits(),
                    state.matric_potential_mm[layer].to_bits(),
                    state.hydraulic_conductivity_mm_s[layer].to_bits()
                );
            }
            println!();
        }
    }
}
