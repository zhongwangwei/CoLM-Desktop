//! `MOD_Hydro_SoilFunction` 的差分探针（与 `oracle/scripts/soil_hydro_fn_diff.f90` 配对）：
//! `soil_psi_from_vliq` / `soil_hk_from_psi` / `soil_vliq_from_psi`，
//! 两档模型 × (2500 均匀随机 + 2500 边界取值)。
//!
//! 抽签次数与顺序必须与 Fortran 侧逐条对齐，改一边就得同步改另一边：
//! 每例先抽 `porsl, vl_r, psi_s, hksat, bsw, alpha, n, l, sc, fc`，再抽 `vliq`
//! （边界分支是 10 次 `pick` + 1 次 `pick`）。第三列 `flag` 标早退分支，不参与筛选。
use colm_core::{
    soil_hydraulic_conductivity, soil_psi_from_vliq, soil_vliq_from_psi, SoilHydraulicModel,
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
    fn pick(&mut self, pool: &[f64]) -> f64 {
        let index = ((self.uni() * pool.len() as f64) as usize).min(pool.len() - 1);
        pool[index]
    }
}

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
const P_VLIQ: &[f64] = &[0.0, 1.0e-12, 0.05, 0.25, 0.99];

fn main() {
    let mut s = Lcg(20260808);
    for k in 0..2u32 {
        let campbell = k == 1;
        for i in 1..=5000u32 {
            let (porsl, vl_r, psi_s, hksat, bsw, alpha, n, l, sc, fc, vliq);
            if i <= 2500 {
                porsl = 0.30 + s.uni() * 0.30;
                vl_r = 0.005 + s.uni() * 0.15;
                psi_s = -(100.0 + s.uni() * 1.0e4);
                hksat = 1.0e-6 + s.uni() * 1.0e-3;
                bsw = 2.0 + s.uni() * 12.0;
                alpha = 1.0e-3 + s.uni() * 0.1;
                n = 1.05 + s.uni() * 2.0;
                l = 0.5 + s.uni() * 0.5;
                sc = 0.2 + s.uni() * 0.8;
                fc = 0.2 + s.uni() * 0.8;
                vliq = s.uni() * porsl * 1.2;
            } else {
                porsl = s.pick(P_POR);
                vl_r = s.pick(P_VLR);
                psi_s = s.pick(P_PSIS);
                hksat = s.pick(P_HK);
                bsw = s.pick(P_BSW);
                alpha = s.pick(P_ALPHA);
                n = s.pick(P_N);
                l = s.pick(P_L);
                sc = s.pick(P_SC);
                fc = s.pick(P_FC);
                vliq = s.pick(P_VLIQ) * porsl;
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
            let psi = soil_psi_from_vliq(vliq, porsl, vl_r, psi_s, model);
            let hk = soil_hydraulic_conductivity(psi, psi_s, hksat, model);
            let vl = soil_vliq_from_psi(psi, porsl, vl_r, psi_s, model);
            let mut flag = 0;
            if vliq >= porsl {
                flag = 1;
            }
            if vliq <= vl_r.max(1.0e-8) {
                flag = 2;
            }
            println!(
                "{k:2} {flag} {:016X} {:016X} {:016X}",
                psi.to_bits(),
                hk.to_bits(),
                vl.to_bits()
            );
        }
    }
}
