//! LULCC 换年时让陆面材料**跟随主机水**（上游 `MOD_MT_Lulcc::mt_lulcc_host_sync`）。
//!
//! 上游的主机 LULCC 先把每个 patch 按新年份冷启动，再只把 `REST_LulccTimeVariables` 里
//! 那些量抄回去；材料（`mt_*`）则是按转移份额重映射的。两者在下面这些地方会不一致：
//!
//! - 没有源 patch 的新 patch：主机保留冷启动水，材料也按冷启动比值从这份水起算；
//! - 湿地水（`wetwat`）不在抄回的清单里，每个 patch 都回到冷启动值，材料随之重算；
//! - MEC 把雪层并入占优的源 patch、丢掉其他源的前置雪（`scv`），PFT/PC 又按新的 PFT
//!   份额重建冠层水——材料按面积搬过去就会和新的主机水对不上。
//!
//! 这里复刻上游 `follow_host_water`：`src` 是把**旧主机水**用与材料相同的权重搬到新
//! patch 上的结果（上游 `src_water_2d/3d`）。某个仓室与主机水不一致时，材料按它原来的
//! 比值（材料 / 搬过来的旧主机水）跟到新水量上；旧水量小到没有意义时用冷启动比值。
//! 雪柱当作一个整体：任一层或前置雪变了，整柱按柱比值重标，未溶解的固相按新雪层水量
//! 重新分配，没有雪层时并到第一层土壤。质量变化（每单位面积乘 patch 面积）由调用方记进
//! LULCC 的质量账（上游 `change`），这样 `assert_lulcc_land_water_mass_conserved` 的
//! `before + change == after` 仍然成立。

use anyhow::{ensure, Result};

use colm_core::material::{
    soisno_slot, MaterialColdStart, MaterialPools, MaterialSet, PatchMaterialState, WaterInventory,
    MAX_SNOW_LAYERS, SOIL_LAYERS, SOISNO_LAYERS, TRC_WATER_MIN_FOR_RATIO,
};

/// 上游 `HW_*` 行与逐层液相/冰相（一个 patch 的主机水）。
#[derive(Debug, Clone)]
pub struct HostWater {
    /// `patchtype`（0 土壤、1 城市、2 湿地、3 冰川）。
    pub patch_type: i64,
    pub ldew: f64,
    pub ldew_rain: f64,
    pub ldew_snow: f64,
    pub wdsrf: f64,
    pub wetwat: f64,
    pub wa: f64,
    /// `HW_THIN_SNOW`：雪层 0 为空时 `scv` 里的那层薄雪。
    pub thin_snow: f64,
    pub tleaf: f64,
    /// 灌溉蓄水（`DEF_USE_IRRIGATION` 时才有）。
    pub waterstorage: Option<f64>,
    /// `wliq_soisno(maxsnl+1:nl_soil)`，与 [`soisno_slot`] 同序。
    pub wliq_soisno: [f64; SOISNO_LAYERS],
    /// `wice_soisno(maxsnl+1:nl_soil)`，与 [`soisno_slot`] 同序。
    pub wice_soisno: [f64; SOISNO_LAYERS],
}

impl HostWater {
    /// 全零的一侧（缺省 `patch_type = 0`）。
    pub fn zeroed() -> Self {
        Self {
            patch_type: 0,
            ldew: 0.0,
            ldew_rain: 0.0,
            ldew_snow: 0.0,
            wdsrf: 0.0,
            wetwat: 0.0,
            wa: 0.0,
            thin_snow: 0.0,
            tleaf: 0.0,
            waterstorage: None,
            wliq_soisno: [0.0; SOISNO_LAYERS],
            wice_soisno: [0.0; SOISNO_LAYERS],
        }
    }

    /// 按上游 `mt_lulcc_host_sync` 的规则补 `HW_THIN_SNOW`：只有雪层 0
    /// （`wliq_soisno(0) + wice_soisno(0)`）为空时，`scv` 才是前置雪。
    pub fn with_thin_snow(mut self, scv: f64) -> Self {
        let j0 = soisno_slot(0);
        if self.wliq_soisno[j0] + self.wice_soisno[j0] <= 0.0 {
            self.thin_snow = scv;
        } else {
            self.thin_snow = 0.0;
        }
        self
    }

    fn inventory(&self) -> WaterInventory<'_> {
        WaterInventory {
            patch_type: i32::try_from(self.patch_type).unwrap_or(0),
            ldew_rain: self.ldew_rain,
            ldew_snow: self.ldew_snow,
            wliq_soisno: &self.wliq_soisno,
            wice_soisno: &self.wice_soisno,
            wa: self.wa,
            wdsrf: self.wdsrf,
            wetwat: self.wetwat,
            scv: self.thin_snow,
            waterstorage: self.waterstorage,
        }
    }
}

/// 跟随主机水的开关。
#[derive(Debug, Clone, Copy)]
pub struct HostFollowOptions {
    /// `DEF_VEG_SNOW`：冠层材料分雨/雪两池时，两池各自跟随；否则按总 `ldew` 跟随
    /// （材料自己的降水过程会把总量拆回雨/雪）。
    pub vegetation_snow: bool,
    /// 没有源 patch 的新 patch 冷启动材料时用的全局开关（变饱和流与含水层混合水量）。
    pub cold_start: MaterialColdStart,
}

/// 一次跟随主机水的结果。
#[derive(Debug, Clone, PartialEq)]
pub struct HostFollowOutcome {
    /// 按主机水冷启动材料的新 patch 数（上游打印的 `ncold`）。
    pub cold_started: usize,
    /// 有任何仓室跟随主机水重标过的 patch 数（上游打印的 `nfollow`）。
    pub followed: usize,
    /// 逐材料的质量变化（每单位面积乘 patch 面积；`areas` 为空时全 0）。
    pub change: Vec<f64>,
}

/// 一个 patch 的材料总量（上游 `mt_patch_land_water_mass`，逐材料）。
fn patch_land_water_mass(set: &MaterialSet, state: &PatchMaterialState) -> Vec<f64> {
    let mut total = vec![0.0; set.len()];
    for (itrc, material) in set.materials.iter().enumerate() {
        if !material.uses_land_water_transport() {
            continue;
        }
        let pools = &state.pools[itrc];
        total[itrc] = matrix_sum(pools);
    }
    total
}

/// 上游 `mt_patch_land_water_mass` 的加法次序。
fn matrix_sum(pools: &MaterialPools) -> f64 {
    let mut total = 0.0;
    total += pools.ldew_rain;
    total += pools.ldew_snow;
    total += pools.wliq_soisno.iter().sum::<f64>();
    total += pools.wice_soisno.iter().sum::<f64>();
    total += pools.wa;
    total += pools.aquifer_ref_mass;
    total += pools.wdsrf;
    total += pools.wetwat;
    total += pools.surface_residue;
    total += pools.subsurface_residue;
    total += pools.solid_soisno.iter().sum::<f64>();
    total += pools.canopy_solid;
    total += pools.surface_solid;
    total += pools.subsurface_solid;
    total += pools.waterstorage_solid;
    total += pools.waterstorage;
    total += pools.scv;
    total
}

/// `water_differs`：阈值以上才算"主机没有按同一套权重搬水"。
fn water_differs(a: f64, b: f64) -> bool {
    (a - b).abs() > TRC_WATER_MIN_FOR_RATIO + 1.0e-12 * a.abs().max(b.abs())
}

/// `follow_pool`：材料按旧水量下的比值跟到新水量；旧水量太小时用冷启动比值。
fn follow_pool(pool: &mut f64, w_src: f64, w_new: f64, r_init: f64, followed: &mut bool) {
    if !water_differs(w_src, w_new) {
        return;
    }
    if w_src > TRC_WATER_MIN_FOR_RATIO {
        *pool = *pool / w_src * w_new.max(0.0);
    } else {
        *pool = r_init * w_new.max(0.0);
    }
    *followed = true;
}

/// 上游 `mt_canopy_parts`：有效冠层水的相态拆分。
fn canopy_parts(
    ldew: f64,
    ldew_rain: f64,
    ldew_snow: f64,
    tleaf: f64,
    vegetation_snow: bool,
) -> (f64, f64) {
    if vegetation_snow {
        return (ldew_rain, ldew_snow);
    }
    if ldew_rain + ldew_snow > 1.0e-10 {
        let rain = ldew * (ldew_rain / (ldew_rain + ldew_snow));
        (rain, ldew - rain)
    } else if ldew > 0.0 {
        if tleaf > colm_core::FREEZING_K {
            (ldew, 0.0)
        } else {
            (0.0, ldew)
        }
    } else {
        (0.0, 0.0)
    }
}

/// 按主机水重标材料（上游 `mt_lulcc_host_sync` 的逐 patch 部分）。
///
/// * `new`：新 patch 的主机水（新年冷启动 + SAT/MEC 之后的合并重启）；
/// * `src`：旧主机水用**与材料相同的权重**搬到新 patch 上的结果；
/// * `sourced`：该新 patch 有没有源 patch（没有则按主机水冷启动材料）；
/// * `areas`：新 patch 面积；非空时把质量变化乘面积累加进返回值，空时只做重标。
///
/// 注意：这里会**改写** `states`，调用方应在它之前留好"重映射后"的材料快照用于质量账
/// （本函数自己按 `areas` 计算出 `change`，直接交给上游 `before + change == after` 的判据）。
pub fn follow_host_water(
    set: &MaterialSet,
    states: &mut [PatchMaterialState],
    new: &[HostWater],
    src: &[HostWater],
    sourced: &[bool],
    areas: &[f64],
    options: HostFollowOptions,
) -> Result<HostFollowOutcome> {
    let nnew = new.len();
    ensure!(
        states.len() == nnew && src.len() == nnew && sourced.len() == nnew,
        "LULCC host-water state, sources and masks disagree on the patch count"
    );
    ensure!(
        areas.is_empty() || areas.len() == nnew,
        "LULCC host-water areas disagree on the patch count"
    );

    // 质量账的 before：重映射之后、跟随主机水之前。没有面积就不记账。
    let before = if areas.is_empty() {
        Vec::new()
    } else {
        states
            .iter()
            .map(|state| patch_land_water_mass(set, state))
            .collect::<Vec<_>>()
    };

    let mut cold_started = 0;
    for np in 0..nnew {
        if sourced[np] {
            continue;
        }
        cold_started += 1;
        states[np].init_from_water(set, new[np].inventory(), options.cold_start)?;
    }

    let mut followed = 0;
    for np in 0..nnew {
        if !sourced[np] {
            continue;
        }
        let mut did_follow = false;
        let (host_new, host_src) = (&new[np], &src[np]);
        // 雪层 j = -maxsnl..0 在 soilsnow 行的下标是 0..MAX_SNOW_LAYERS（见 soisno_slot）。
        let snow_new: [f64; MAX_SNOW_LAYERS] =
            std::array::from_fn(|layer| host_new.wliq_soisno[layer] + host_new.wice_soisno[layer]);
        for (itrc, material) in set.materials.iter().enumerate() {
            if !material.uses_land_water_transport() {
                continue;
            }
            let r_init = material.init_water_ratio();
            let pools = &mut states[np].pools[itrc];

            // 湿地水：主机不把它抄回来，每个 patch 都回到冷启动值。
            pools.wetwat = 0.0;
            if host_new.patch_type == 2 {
                pools.wetwat = host_new.wetwat.max(0.0) * r_init;
            }

            // 冠层。
            if options.vegetation_snow {
                follow_pool(
                    &mut pools.ldew_rain,
                    host_src.ldew_rain,
                    host_new.ldew_rain,
                    r_init,
                    &mut did_follow,
                );
                follow_pool(
                    &mut pools.ldew_snow,
                    host_src.ldew_snow,
                    host_new.ldew_snow,
                    r_init,
                    &mut did_follow,
                );
            } else if water_differs(host_src.ldew, host_new.ldew) {
                if host_src.ldew > TRC_WATER_MIN_FOR_RATIO {
                    let scale = host_new.ldew.max(0.0) / host_src.ldew;
                    pools.ldew_rain *= scale;
                    pools.ldew_snow *= scale;
                } else {
                    pools.ldew_rain = r_init * host_new.ldew.max(0.0);
                    pools.ldew_snow = 0.0;
                }
                did_follow = true;
            }

            // 雪柱：任一层或前置雪变了就整柱重标。
            let mut snow_differs = water_differs(host_src.thin_snow, host_new.thin_snow);
            for slot in 0..MAX_SNOW_LAYERS {
                snow_differs = snow_differs
                    || water_differs(host_src.wliq_soisno[slot], host_new.wliq_soisno[slot])
                    || water_differs(host_src.wice_soisno[slot], host_new.wice_soisno[slot]);
            }
            if snow_differs {
                let w_src = host_src.thin_snow
                    + host_src.wliq_soisno[..MAX_SNOW_LAYERS].iter().sum::<f64>()
                    + host_src.wice_soisno[..MAX_SNOW_LAYERS].iter().sum::<f64>();
                let ratio = if w_src > TRC_WATER_MIN_FOR_RATIO {
                    (pools.scv
                        + pools.wliq_soisno[..MAX_SNOW_LAYERS].iter().sum::<f64>()
                        + pools.wice_soisno[..MAX_SNOW_LAYERS].iter().sum::<f64>())
                        / w_src
                } else {
                    r_init
                };
                pools.scv = ratio * host_new.thin_snow.max(0.0);
                for layer in 0..MAX_SNOW_LAYERS {
                    pools.wliq_soisno[layer] = ratio * host_new.wliq_soisno[layer].max(0.0);
                    pools.wice_soisno[layer] = ratio * host_new.wice_soisno[layer].max(0.0);
                }
                // 雪层里未溶解的固相跟着新雪层的水量走；一层都不剩时并到第一层土壤。
                let solid: f64 = pools.solid_soisno[..MAX_SNOW_LAYERS].iter().sum();
                if solid != 0.0 {
                    let wsnow: f64 = snow_new.iter().sum();
                    if wsnow > 0.0 {
                        for (layer, &water) in snow_new.iter().enumerate() {
                            pools.solid_soisno[layer] = solid * water / wsnow;
                        }
                    } else {
                        pools.solid_soisno[..MAX_SNOW_LAYERS].fill(0.0);
                        pools.solid_soisno[soisno_slot(1)] += solid;
                    }
                }
                did_follow = true;
            }

            // 土壤每层液相、冰相各自跟随。
            for j in 1..=SOIL_LAYERS as i32 {
                let slot = soisno_slot(j);
                follow_pool(
                    &mut pools.wliq_soisno[slot],
                    host_src.wliq_soisno[slot],
                    host_new.wliq_soisno[slot],
                    r_init,
                    &mut did_follow,
                );
                follow_pool(
                    &mut pools.wice_soisno[slot],
                    host_src.wice_soisno[slot],
                    host_new.wice_soisno[slot],
                    r_init,
                    &mut did_follow,
                );
            }

            // 地表水。
            follow_pool(
                &mut pools.wdsrf,
                host_src.wdsrf,
                host_new.wdsrf,
                r_init,
                &mut did_follow,
            );

            // 有溶解度上限的材料：按最终池子的有效液相再平衡，湿地水与 ponded 水
            // 共享一个地表固相库，合并求一次容量再按水量比拆回。
            if material.has_dissolved_limit() {
                let (canopy_liquid, _) = canopy_parts(
                    host_new.ldew,
                    host_new.ldew_rain,
                    host_new.ldew_snow,
                    host_new.tleaf,
                    options.vegetation_snow,
                );
                material.equilibrate_dissolved(
                    canopy_liquid.max(0.0),
                    &mut pools.ldew_rain,
                    &mut pools.canopy_solid,
                );
                for slot in 0..SOISNO_LAYERS {
                    material.equilibrate_dissolved(
                        host_new.wliq_soisno[slot].max(0.0),
                        &mut pools.wliq_soisno[slot],
                        &mut pools.solid_soisno[slot],
                    );
                }
                let wetland_water = if host_new.patch_type == 2 {
                    host_new.wetwat.max(0.0)
                } else {
                    0.0
                };
                let surface_water = host_new.wdsrf.max(0.0) + wetland_water;
                let mut surface_mass = pools.wdsrf + pools.wetwat;
                material.equilibrate_dissolved(
                    surface_water,
                    &mut surface_mass,
                    &mut pools.surface_solid,
                );
                if surface_water > TRC_WATER_MIN_FOR_RATIO {
                    pools.wdsrf = surface_mass * host_new.wdsrf.max(0.0) / surface_water;
                    pools.wetwat = surface_mass * wetland_water / surface_water;
                } else {
                    pools.wdsrf = 0.0;
                    pools.wetwat = 0.0;
                }
            }
        }
        if did_follow {
            followed += 1;
        }
    }

    let mut change = vec![0.0; set.len()];
    if !areas.is_empty() {
        for (np, &area) in areas.iter().enumerate() {
            let after = patch_land_water_mass(set, &states[np]);
            for (itrc, slot) in change.iter_mut().enumerate() {
                *slot += (after[itrc] - before[np][itrc]) * area;
            }
        }
    }

    Ok(HostFollowOutcome {
        cold_started,
        followed,
        change,
    })
}

#[cfg(test)]
#[path = "lulcc_host_tests.rs"]
mod lulcc_host_tests;
