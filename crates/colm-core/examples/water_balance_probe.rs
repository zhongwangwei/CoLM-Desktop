//! `MOD_Hydro_SoilWater:water_balance` 的差分探针（配对物
//! `oracle/scripts/water_balance_diff.f90`）。
//!
//! 第 309/317 轮在这个例程的汇编里认出 5 条"和（差）与乘积"形状的 FMA；这个探针
//! 在合成输入上逐位判它们。抽签次数与顺序必须与 Fortran 侧逐条对齐（改一边就得
//! 同步改另一边）：每例依次抽 `nlev`、`nlev` 个界面增量、`nlev` 组七列状态、
//! `nlev` 个饱和标记、`nlev+1` 个界面通量，再抽 9 个标量与两个边界值。
//!
//! `interface_depth_mm` 由界面增量累加得到，`dz` 由相邻界面相减复原 —— 与内核侧
//! `water_balance` 的入参 `dz` 逐位相同（`validate_water_balance` 内部也是这样
//! 从界面深度推厚度的）。
use colm_core::{
    variable_saturated_water_balance, VariableSaturatedBoundary, VariableSaturatedBoundaryKind,
    VariableSaturatedWaterBalanceInput,
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

const SAT: [f64; 4] = [0.0, 0.3, 0.7, 1.0];
const CASES_PER_SCENARIO: usize = 2000;

fn main() {
    let mut s = Lcg(20260924);
    let mut out = String::new();
    for k in 0..6usize {
        let ubc_kind = if k == 1 || k == 3 || k == 5 {
            VariableSaturatedBoundaryKind::Rainfall
        } else {
            VariableSaturatedBoundaryKind::FixedHead
        };
        let lbc_kind = if k >= 2 {
            VariableSaturatedBoundaryKind::Drainage
        } else {
            VariableSaturatedBoundaryKind::FixedHead
        };
        let sat_prob = SAT[k % 4];
        for _ in 1..=CASES_PER_SCENARIO {
            let nl = 1 + (s.uni() * 6.0) as usize;
            let mut interfaces = vec![0.0f64; nl + 1];
            for j in 1..=nl {
                interfaces[j] = interfaces[j - 1] + (1.0e-3 + s.uni() * 1.0);
            }
            let dz = interfaces
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .collect::<Vec<_>>();

            let mut porosity = vec![0.0; nl];
            for value in porosity.iter_mut() {
                *value = 0.2 + s.uni() * 0.6;
            }
            let mut wetting_front = vec![0.0; nl];
            for (j, value) in wetting_front.iter_mut().enumerate() {
                *value = s.uni() * dz[j] * 0.4;
            }
            let mut water_table = vec![0.0; nl];
            for (j, value) in water_table.iter_mut().enumerate() {
                *value = s.uni() * dz[j] * 0.4;
            }
            let mut liquid = vec![0.0; nl];
            for value in liquid.iter_mut() {
                *value = 0.05 + s.uni() * 0.5;
            }
            let mut previous_wetting_front = vec![0.0; nl];
            for (j, value) in previous_wetting_front.iter_mut().enumerate() {
                *value = s.uni() * dz[j] * 0.4;
            }
            let mut previous_water_table = vec![0.0; nl];
            for (j, value) in previous_water_table.iter_mut().enumerate() {
                *value = s.uni() * dz[j] * 0.4;
            }
            let mut previous_liquid = vec![0.0; nl];
            for value in previous_liquid.iter_mut() {
                *value = 0.05 + s.uni() * 0.5;
            }
            let saturated = (0..nl).map(|_| s.uni() < sat_prob).collect::<Vec<_>>();
            let mut flux = vec![0.0; nl + 1];
            for value in flux.iter_mut() {
                *value = (s.uni() - 0.5) * 1.0e-3;
            }
            let time_step_seconds = 1.0 + s.uni() * 100.0;
            let ponding_depth_mm = s.uni() * 10.0;
            let previous_ponding_depth_mm = s.uni() * 10.0;
            let mut aquifer_water_mm = s.uni() * 100.0;
            let previous_aquifer_water_mm = s.uni() * 100.0;
            let tolerance_mm = 1.0e-8 + s.uni() * 1.0e-4;
            let upper_value = (s.uni() - 0.3) * 1.0e-3;
            let lower_value = (s.uni() - 0.3) * 1.0e-3;
            if k == 2 || k == 3 {
                aquifer_water_mm = 0.0;
                flux[nl] = flux[nl].abs();
            }

            let balance = variable_saturated_water_balance(VariableSaturatedWaterBalanceInput {
                time_step_seconds,
                interface_depth_mm: &interfaces,
                saturated: &saturated,
                porosity: &porosity,
                interface_flux_mm_s: &flux,
                upper_boundary: VariableSaturatedBoundary {
                    kind: ubc_kind,
                    value: upper_value,
                },
                lower_boundary: VariableSaturatedBoundary {
                    kind: lbc_kind,
                    value: lower_value,
                },
                wetting_front_mm: &wetting_front,
                liquid_water: &liquid,
                water_table_thickness_mm: &water_table,
                ponding_depth_mm,
                aquifer_water_mm,
                previous_wetting_front_mm: &previous_wetting_front,
                previous_liquid_water: &previous_liquid,
                previous_water_table_thickness_mm: &previous_water_table,
                previous_ponding_depth_mm,
                previous_aquifer_water_mm,
                tolerance_mm,
            })
            .unwrap_or_else(|error| panic!("k={k}: {error}"));

            out.push_str(&format!("{k:2} {nl}"));
            for value in &balance.residual_mm {
                out.push_str(&format!(" {:016X}", value.to_bits()));
            }
            out.push_str(&format!(" {}\n", u8::from(balance.solvable)));
        }
    }
    print!("{out}");
}
