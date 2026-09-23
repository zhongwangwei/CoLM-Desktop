//! `get_zwt_from_wa`（含水层亏缺 → 水位埋深）的差分探针，与
//! `oracle/scripts/get_zwt_diff.f90` 配对。
//!
//! **抽签次数与顺序必须与 Fortran 侧逐条对齐**，改一边就得同步改另一边：
//! 每例先抽 `vl_s, vl_r, psi_s, hksat, bsw, alpha, n, l, sc, fc`，再抽
//! `wa, zmin, tol_v, tol_z`（边界分支是 14 次 `pick`）。
//!
//! 为什么要单列：这一段是干窗第 19 步那颗持久种子的所在地，但它**不能**用黄金窗口
//! 判形状 —— 那三份窗口由 `f_vegwp` 混沌主导，改对末位会把混沌轨道换一条、指标可能
//! 反向（第 301 轮：探针 12/240 → 7/240，干窗 `over_tol` 却 28 → 36）。这里在合成输入上
//! 逐位比 `zwt`，把"形状对不对"与"混沌指标好不好"彻底分开。
//!
//! 输出五列十六进制位型：`wa, zmin, vl_s, psi_s, zwt`（前两列是十进制 `k`/`flag`）。
use colm_core::{water_table_from_aquifer, SoilHydraulicModel};

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

const P_VLS: &[f64] = &[0.30, 0.40, 0.50, 0.60, 0.45];
const P_VLR: &[f64] = &[0.02, 0.05, 0.10, 0.15, 1.0e-9];
const P_PSIS: &[f64] = &[-1.0e4, -1.0e3, -100.0, -5.0e3, -50.0];
const P_HK: &[f64] = &[1.0e-6, 1.0e-5, 1.0e-4, 1.0e-3, 5.0e-5];
const P_BSW: &[f64] = &[2.0, 4.0, 7.0, 11.0, 14.0];
const P_ALPHA: &[f64] = &[1.0e-3, 5.0e-3, 1.0e-2, 5.0e-2, 0.1];
const P_N: &[f64] = &[1.1, 1.5, 2.0, 2.5, 3.0];
const P_L: &[f64] = &[0.5, 0.7, 0.9, 1.0, 0.6];
const P_SC: &[f64] = &[0.2, 0.4, 0.6, 0.8, 1.0];
const P_FC: &[f64] = &[0.2, 0.4, 0.6, 0.8, 1.0];
/// 前 4 个是负的（含水层亏缺，目标分支），后 2 个覆盖 `wa >= 0` 的早退。
const P_WA: &[f64] = &[-1.0e-6, -1.0e-3, -0.5, -5.0, 0.0, 5.0];
const P_ZMIN: &[f64] = &[10.0, 100.0, 500.0, 1000.0, 2000.0];
const P_TOLV: &[f64] = &[1.0e-8, 1.0e-6, 1.0e-4, 1.0e-3, 1.0e-2];
const P_TOLZ: &[f64] = &[1.0e-6, 1.0e-4, 1.0e-2, 1.0, 10.0];

fn main() {
    let mut s = Lcg(20260809);
    let mut out = String::new();
    for k in 0..2u32 {
        let campbell = k == 1;
        for i in 1..=5000u32 {
            let (vl_s, vl_r, psi_s, bsw, alpha, n, l, sc, fc, wa, zmin, tol_v, tol_z);
            if i <= 2500 {
                vl_s = 0.30 + s.uni() * 0.30;
                vl_r = 0.005 + s.uni() * 0.15;
                psi_s = -(100.0 + s.uni() * 1.0e4);
                let _hksat = 1.0e-6 + s.uni() * 1.0e-3;
                bsw = 2.0 + s.uni() * 12.0;
                alpha = 1.0e-3 + s.uni() * 0.1;
                n = 1.05 + s.uni() * 2.0;
                l = 0.5 + s.uni() * 0.5;
                sc = 0.2 + s.uni() * 0.8;
                fc = 0.2 + s.uni() * 0.8;
                wa = -(s.uni() * 200.0);
                zmin = 10.0 + s.uni() * 2000.0;
                tol_v = 1.0e-6 + s.uni() * 1.0e-3;
                tol_z = 1.0e-4 + s.uni() * 1.0e-1;
            } else {
                vl_s = s.pick(P_VLS);
                vl_r = s.pick(P_VLR);
                psi_s = s.pick(P_PSIS);
                let _hksat = s.pick(P_HK);
                bsw = s.pick(P_BSW);
                alpha = s.pick(P_ALPHA);
                n = s.pick(P_N);
                l = s.pick(P_L);
                sc = s.pick(P_SC);
                fc = s.pick(P_FC);
                wa = s.pick(P_WA);
                zmin = s.pick(P_ZMIN);
                tol_v = s.pick(P_TOLV);
                tol_z = s.pick(P_TOLZ);
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
            let flag = i32::from(wa >= 0.0);
            let zwt = water_table_from_aquifer(vl_s, vl_r, psi_s, model, tol_v, tol_z, wa, zmin)
                .unwrap_or_else(|error| panic!("k={k} i={i}: {error}"));
            out.push_str(&format!(
                "{k:2} {flag} {:016X} {:016X} {:016X} {:016X} {:016X}\n",
                wa.to_bits(),
                zmin.to_bits(),
                vl_s.to_bits(),
                psi_s.to_bits(),
                zwt.to_bits()
            ));
        }
    }
    print!("{out}");
}
