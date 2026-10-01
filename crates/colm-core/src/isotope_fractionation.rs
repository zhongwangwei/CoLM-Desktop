//! 水同位素分馏物理（`MOD_Tracer_Frac`、`MOD_Tracer_Isotope_O18/HDO`、`MOD_Tracer_Isotope_Registry`）。
//!
//! 全是纯函数：平衡分馏系数、动力分馏（Craig-Gordon、Merlivat-Jouzel、叶片与土壤的阻力加权）、
//! Jouzel-Merlivat 过饱和凝华、Rayleigh 冻结、土壤液相与气相扩散、叶片水非稳态模型。
//! 上游通过 `itrc` 查全局示踪物表与 namelist；这里把它们显式成 [`FractionationConfig`] 与
//! [`IsotopeTracer`] 两个参数。
//!
//! 收缩形状逐个取自 latlon 内核的 GIMPLE（`gimpleL/MOD_Tracer_Frac.F90`），并由
//! `isotope_fractionation_tests.rs` 用按内核选项编出的上游模块逐位对照。

// 夹紧保留上游 `MIN(MAX(·))` 的次序。
#![allow(clippy::manual_clamp)]

use crate::LibmPow;

/// `trc_tiny`。
pub const TRC_TINY: f64 = 1.0e-30;
/// 本模块自带的 `tfrz`（273.15，与陆面常数 273.16 不同）。
const TFRZ: f64 = 273.15;
const WATER_MOLES_PER_MM: f64 = 1000.0 / 18.01528;
const LIQUID_WATER_MOLAR_DENSITY: f64 = 55.5e3;
const UNIVERSAL_GAS_CONSTANT: f64 = 8.31446261815324;
const CRAIG_GORDON_MAX_RATIO_AMPLIFICATION: f64 = 10.0;
/// `2._r8/3._r8`。
const TWO_THIRDS: f64 = 2.0 / 3.0;
/// `7._r8/3._r8`。
const SEVEN_THIRDS: f64 = 7.0 / 3.0;
const JM84_FULL_KINETIC_TEMP: f64 = 253.15;
const MJ79_SMOOTH_K: f64 = 0.006;
const MJ79_ROUGH_SLOPE: f64 = 0.000285;
const MJ79_ROUGH_OFFSET: f64 = 0.00082;
const MJ79_WIND_THRESHOLD: f64 = 7.0;
/// `Rsmow_18O`、`Rsmow_D`。
pub const RSMOW_18O: f64 = 2.0052e-3;
pub const RSMOW_D: f64 = 1.5576e-4;

/// 已登记分馏物理的同位素（`include/tracer_isotope_species.inc`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsotopeSpecies {
    O18,
    Hdo,
}

/// `DEF_TRACER_KINETIC_SCHEME`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KineticScheme {
    Merlivat1978,
    Cappa2003,
}

/// `DEF_TRACER_OPEN_WATER_KINETIC`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenWaterKinetic {
    Mj79,
    Exponent,
}

/// 分馏用到的 namelist 量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FractionationConfig {
    /// `DEF_TRACER_USE_FRACTIONATION`。
    pub use_fractionation: bool,
    pub kinetic_scheme: KineticScheme,
    pub open_water_kinetic: OpenWaterKinetic,
    /// `DEF_TRACER_ICE_SUPERSAT_SLOPE`。
    pub ice_supersat_slope: f64,
    /// `DEF_TRACER_CG_RELHUM_MAX`。
    pub cg_relhum_max: f64,
    /// `DEF_TRACER_NSS_LEAF_WATER_PER_LAI`、`_PATH_LENGTH`、`_RB`。
    pub nss_leaf_water_per_lai: f64,
    pub nss_leaf_path_length: f64,
    pub nss_leaf_rb: f64,
}

impl Default for FractionationConfig {
    /// `MOD_Namelist` 的缺省值（分馏默认关）。
    fn default() -> Self {
        Self {
            use_fractionation: false,
            kinetic_scheme: KineticScheme::Merlivat1978,
            open_water_kinetic: OpenWaterKinetic::Mj79,
            ice_supersat_slope: 0.003,
            cg_relhum_max: 0.99,
            nss_leaf_water_per_lai: 0.12,
            nss_leaf_path_length: 0.01,
            nss_leaf_rb: 100.0,
        }
    }
}

/// 一个示踪物在分馏里要看的两样：是不是已登记的同位素（`isotope_fractionation_registered`），
/// 以及参考比值（`tracers(itrc)%ref_ratio`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IsotopeTracer {
    /// `None`：不是同位素，或名字没匹配上已登记的物种。
    pub species: Option<IsotopeSpecies>,
    pub ref_ratio: f64,
}

impl IsotopeSpecies {
    /// `isotope_name_matches`：按名字的小写子串匹配（O18：`18o,o18`；HDO：`hdo,2h,deuter,=h2`）。
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if ["18o", "o18"].iter().any(|p| lower.contains(p)) {
            Some(Self::O18)
        } else if ["hdo", "2h", "deuter", "=h2"]
            .iter()
            .any(|p| lower.contains(p))
        {
            Some(Self::Hdo)
        } else {
            None
        }
    }

    /// `mj79_relative_factor`。
    pub fn mj79_relative_factor(self) -> f64 {
        match self {
            Self::O18 => 1.0,
            Self::Hdo => 0.88,
        }
    }

    /// `alpha_liq_vap_fn`：液/汽平衡分馏系数。
    pub fn alpha_liq_vap(self, temp_k: f64) -> f64 {
        let tk = temp_k.max(150.0);
        match self {
            Self::O18 => (1137.0 / (tk * tk) - 0.4156 / tk - 0.0020667).exp(),
            Self::Hdo => (24844.0 / (tk * tk) - 76.248 / tk + 0.052612).exp(),
        }
    }

    /// `alpha_ice_vap_fn`：冰/汽平衡分馏系数。
    pub fn alpha_ice_vap(self, temp_k: f64) -> f64 {
        let tk = temp_k.max(150.0);
        match self {
            Self::O18 => (11.839 / tk - 0.028224).exp(),
            Self::Hdo => (16289.0 / (tk * tk) - 0.0945).exp(),
        }
    }

    /// `diffusivity_ratio_air_fn`：空气中轻/重同位素分子扩散系数比。
    pub fn diffusivity_ratio(self, scheme: KineticScheme) -> f64 {
        match (self, scheme) {
            (Self::O18, KineticScheme::Merlivat1978) => 1.0285,
            (Self::O18, KineticScheme::Cappa2003) => 1.03189,
            (Self::Hdo, KineticScheme::Merlivat1978) => 1.0251,
            (Self::Hdo, KineticScheme::Cappa2003) => 1.01636,
        }
    }

    /// `leaf_liquid_diffusivity_fn`：液态水中的自扩散系数 [m²/s]。
    pub fn leaf_liquid_diffusivity(self, temp_k: f64) -> f64 {
        let tk = temp_k.max(150.0);
        match self {
            Self::O18 => 119.0e-9 * (-637.0 / (tk - 137.0).max(1.0)).exp(),
            Self::Hdo => 116.0e-9 * (-626.0 / (tk - 139.0).max(1.0)).exp(),
        }
    }
}

impl IsotopeTracer {
    /// `tracer_fractionation_active`。
    pub fn fractionation_active(&self, config: &FractionationConfig) -> bool {
        config.use_fractionation && self.species.is_some()
    }

    /// 上游对未登记物种返回的默认值（`isotope_*` 查不到时都是 1）。
    fn species_or_unit<F: Fn(IsotopeSpecies) -> f64>(&self, f: F) -> f64 {
        self.species.map_or(1.0, f)
    }

    /// `tracer_alpha_liq_vap`。
    pub fn alpha_liq_vap(&self, temp_k: f64) -> f64 {
        self.species_or_unit(|s| s.alpha_liq_vap(temp_k))
    }

    /// `tracer_alpha_ice_vap`。
    pub fn alpha_ice_vap(&self, temp_k: f64) -> f64 {
        self.species_or_unit(|s| s.alpha_ice_vap(temp_k))
    }

    /// `tracer_alpha_ice_liq = alpha_ice_vap / max(alpha_liq_vap, trc_tiny)`。
    pub fn alpha_ice_liq(&self, temp_k: f64) -> f64 {
        self.alpha_ice_vap(temp_k) / self.alpha_liq_vap(temp_k).max(TRC_TINY)
    }

    /// `tracer_diffusivity_ratio_air`。
    pub fn diffusivity_ratio(&self, config: &FractionationConfig) -> f64 {
        self.species_or_unit(|s| s.diffusivity_ratio(config.kinetic_scheme))
    }

    /// `tracer_leaf_liquid_diffusivity`（也是 `tracer_liquid_self_diffusivity`）。
    pub fn leaf_liquid_diffusivity(&self, temp_k: f64) -> f64 {
        self.species
            .map_or(0.0, |s| s.leaf_liquid_diffusivity(temp_k))
    }

    /// `tracer_ratio_to_delta`：`(R/R_ref − 1)·1000`。
    pub fn ratio_to_delta(&self, ratio: f64) -> f64 {
        if self.ref_ratio > TRC_TINY {
            (ratio / self.ref_ratio - 1.0) * 1000.0
        } else {
            0.0
        }
    }

    /// `tracer_delta_to_ratio`：`R_ref·(1 + δ/1000)`。
    pub fn delta_to_ratio(&self, delta: f64) -> f64 {
        if self.ref_ratio > TRC_TINY {
            self.ref_ratio * (1.0 + delta / 1000.0)
        } else {
            0.0
        }
    }

    /// `tracer_rayleigh_freezing_loss`：有限液池平衡冻结时转入冰相的示踪物量。
    pub fn rayleigh_freezing_loss(
        &self,
        config: &FractionationConfig,
        pool_trc: f64,
        pool_water: f64,
        freeze_water: f64,
        temp_k: f64,
    ) -> f64 {
        if pool_water <= TRC_TINY || freeze_water <= TRC_TINY || pool_trc <= TRC_TINY {
            return 0.0;
        }
        if freeze_water >= pool_water * (1.0 - 1.0e-12) {
            return pool_trc.max(0.0);
        }
        let source_ratio = pool_trc.max(0.0) / pool_water;
        if !self.fractionation_active(config) {
            return (freeze_water * source_ratio).min(pool_trc.max(0.0));
        }
        let alpha_il = self.alpha_ice_liq(temp_k);
        if alpha_il <= TRC_TINY || alpha_il.is_nan() {
            return (freeze_water * source_ratio).min(pool_trc.max(0.0));
        }
        let remaining_water = pool_water - freeze_water;
        let liquid_fraction = (remaining_water / pool_water).max(TRC_TINY);
        let remaining_ratio = liquid_fraction.lpow(alpha_il - 1.0) * source_ratio;
        // `pool_trc − remaining_water·remaining_ratio` 收缩成 FNMA。
        (-remaining_water)
            .mul_add(remaining_ratio, pool_trc)
            .max(0.0)
            .min(pool_trc.max(0.0))
    }

    /// `tracer_alpha_ice_vap_deposition`。
    pub fn alpha_ice_vap_deposition(&self, config: &FractionationConfig, temp_k: f64) -> f64 {
        ice_deposition_alpha(
            temp_k,
            config.ice_supersat_slope,
            self.diffusivity_ratio(config),
            self.alpha_ice_vap(temp_k),
            self.alpha_ice_vap(TFRZ),
            self.alpha_ice_vap(JM84_FULL_KINETIC_TEMP),
        )
    }

    /// `tracer_alpha_kinetic_craig_gordon`：液面 n = 2/3，冰面 n = 1。
    pub fn alpha_kinetic_craig_gordon(&self, config: &FractionationConfig, from_ice: bool) -> f64 {
        let ratio = self.diffusivity_ratio(config);
        if from_ice {
            ratio
        } else {
            ratio.lpow(TWO_THIRDS)
        }
    }

    /// `tracer_alpha_kinetic_open_water`。
    pub fn alpha_kinetic_open_water(&self, config: &FractionationConfig, wind: f64) -> f64 {
        match config.open_water_kinetic {
            OpenWaterKinetic::Exponent => self.alpha_kinetic_craig_gordon(config, false),
            OpenWaterKinetic::Mj79 => {
                mj79_kinetic_alpha(wind, self.species_or_unit(|s| s.mj79_relative_factor()))
            }
        }
    }

    /// `tracer_alpha_kinetic_leaf`：`(ra + rb·D^(2/3) + rc·D)/(ra+rb+rc)`。
    pub fn alpha_kinetic_leaf(
        &self,
        config: &FractionationConfig,
        ra: f64,
        rb: f64,
        rc: f64,
    ) -> f64 {
        let (ra1, rb1, rc1) = (ra.max(0.0), rb.max(0.0), rc.max(0.0));
        let denom = ra1 + rb1 + rc1;
        if denom <= TRC_TINY {
            return 1.0;
        }
        let ratio = self.diffusivity_ratio(config);
        rc1.mul_add(ratio, ratio.lpow(TWO_THIRDS).mul_add(rb1, ra1)) / denom
    }

    /// `tracer_alpha_kinetic_soil`。
    pub fn alpha_kinetic_soil(&self, config: &FractionationConfig, ra: f64, rs: f64) -> f64 {
        soil_kinetic_alpha_core(ra, rs, self.diffusivity_ratio(config))
    }

    /// `tracer_craig_gordon_evap_ratio`：蒸发/升华通量的同位素比。
    #[allow(clippy::too_many_arguments)]
    pub fn craig_gordon_evap_ratio(
        &self,
        config: &FractionationConfig,
        source_ratio: f64,
        vapor_ratio: f64,
        temp_k: f64,
        relhum: f64,
        alpha_k: f64,
        from_ice: bool,
    ) -> f64 {
        if !self.fractionation_active(config) {
            return source_ratio;
        }
        let alpha_eq = if from_ice {
            self.alpha_ice_vap(temp_k)
        } else {
            self.alpha_liq_vap(temp_k)
        };
        if alpha_eq <= TRC_TINY || alpha_eq.is_nan() {
            return source_ratio;
        }
        let ratio = craig_gordon_ratio_core(
            source_ratio,
            vapor_ratio,
            alpha_eq,
            alpha_k.max(TRC_TINY),
            relhum,
            config.cg_relhum_max,
        );
        if ratio.is_nan() {
            source_ratio
        } else {
            ratio
        }
    }

    /// `tracer_equilibrium_deposition_ratio`：凝结/凝华通量的同位素比。
    pub fn equilibrium_deposition_ratio(
        &self,
        config: &FractionationConfig,
        vapor_ratio: f64,
        temp_k: f64,
        from_ice: bool,
    ) -> f64 {
        if !self.fractionation_active(config) {
            return vapor_ratio;
        }
        if from_ice {
            self.alpha_ice_vap_deposition(config, temp_k) * vapor_ratio
        } else {
            self.alpha_liq_vap(temp_k) * vapor_ratio
        }
    }

    /// `tracer_transpiration_nss_ratio`：叶片水非稳态模型（Farquhar & Cernusak），返回蒸腾通量的
    /// 同位素比与更新后的叶片状态。
    #[allow(clippy::too_many_arguments)]
    pub fn transpiration_nss_ratio(
        &self,
        config: &FractionationConfig,
        source_ratio: f64,
        vapor_ratio: f64,
        temp_k: f64,
        relhum: f64,
        psrf: f64,
        transp_water: f64,
        deltim: f64,
        leaf_area: f64,
        aerodynamic_resistance: f64,
        stomatal_resistance: f64,
        prev: LeafWaterState,
    ) -> (f64, LeafWaterState) {
        let mut trans_ratio = source_ratio;
        let mut new = LeafWaterState {
            delta_e: self.ratio_to_delta(source_ratio),
            delta_b: 0.0,
            peclet: 1.0,
            leaf_moles: 0.0,
        };
        new.delta_b = new.delta_e;
        if !self.fractionation_active(config) {
            return (trans_ratio, new);
        }
        if source_ratio <= TRC_TINY || vapor_ratio <= TRC_TINY {
            return (trans_ratio, new);
        }
        if transp_water <= TRC_TINY || deltim <= TRC_TINY {
            return (trans_ratio, new);
        }
        let leaf_moles =
            leaf_area.max(0.0) * config.nss_leaf_water_per_lai.max(0.0) * WATER_MOLES_PER_MM;
        new.leaf_moles = leaf_moles;
        if leaf_moles <= TRC_TINY {
            return (trans_ratio, new);
        }
        let tk = temp_k.max(150.0);
        let h = relhum
            .max(0.0)
            .min(config.cg_relhum_max.min(0.999999).max(0.0));
        let one_minus_h = (1.0 - h).max(1.0e-6);
        let alpha_eq = self.alpha_liq_vap(tk);
        let eps_eq = (alpha_eq - 1.0) * 1000.0;
        let alpha_k = self.alpha_kinetic_leaf(
            config,
            aerodynamic_resistance,
            config.nss_leaf_rb / leaf_area.max(TRC_TINY),
            stomatal_resistance,
        );
        let eps_k = (alpha_k - 1.0) * 1000.0;
        let delta_x = self.ratio_to_delta(source_ratio);
        let delta_v = self.ratio_to_delta(vapor_ratio);
        // `delta_es = delta_x + eps_k + eps_eq + h*(delta_v - eps_k - delta_x)`：末项收缩成 FMA。
        let delta_es = h.mul_add((delta_v - eps_k) - delta_x, (delta_x + eps_k) + eps_eq);
        let transp_moles = transp_water * WATER_MOLES_PER_MM;
        let transp_moles_leaf_s = transp_moles / (deltim * leaf_area.max(TRC_TINY));
        let liquid_diff = self.leaf_liquid_diffusivity(tk);
        if liquid_diff > TRC_TINY && config.nss_leaf_path_length > 0.0 {
            let peclet_number = transp_moles_leaf_s * config.nss_leaf_path_length
                / (LIQUID_WATER_MOLAR_DENSITY * liquid_diff);
            new.peclet = if peclet_number > 1.0e-8 {
                (1.0 - (-peclet_number).exp()) / peclet_number
            } else {
                (-peclet_number).mul_add(0.5, 1.0)
            };
        }
        new.peclet = new.peclet.max(0.0).min(1.0);
        let (prev_w, prev_e, prev_p) = if prev.leaf_moles > TRC_TINY {
            let leaf_moles_ratio = prev.leaf_moles / leaf_moles.max(TRC_TINY);
            let prev_w = if leaf_moles_ratio > 0.25 && leaf_moles_ratio < 4.0 {
                prev.leaf_moles
            } else {
                leaf_moles
            };
            (prev_w, prev.delta_e, prev.peclet.max(0.0).min(1.0))
        } else {
            (leaf_moles, delta_x, new.peclet)
        };
        let mut gross_moles = transp_moles / one_minus_h;
        let mut conductance_gross_moles = 0.0;
        let total_resistance = aerodynamic_resistance.max(0.0)
            + stomatal_resistance.max(0.0)
            + config.nss_leaf_rb.max(0.0) / leaf_area.max(TRC_TINY);
        if total_resistance > TRC_TINY && psrf > TRC_TINY {
            let vapor_molar_density_sat =
                saturation_vapor_pressure(tk, false) / (UNIVERSAL_GAS_CONSTANT * tk);
            conductance_gross_moles = deltim * vapor_molar_density_sat / total_resistance;
        }
        gross_moles = gross_moles.max(conductance_gross_moles);
        let relax_b = alpha_k * alpha_eq / gross_moles.max(TRC_TINY);
        let leaf_peclet = leaf_moles * new.peclet;
        let denom = relax_b.mul_add(leaf_peclet, 1.0);
        new.delta_e = if denom > TRC_TINY {
            let memory = (prev_w * prev_p).mul_add(prev_e - delta_x, leaf_peclet * delta_x);
            relax_b.mul_add(memory, delta_es) / denom
        } else {
            delta_es
        };
        new.delta_b = new.peclet.mul_add(new.delta_e - delta_x, delta_x);
        let (prev_leaf_water, prev_bulk_ratio) = if prev.leaf_moles > TRC_TINY {
            (
                prev_w / WATER_MOLES_PER_MM,
                self.delta_to_ratio(prev.delta_b),
            )
        } else {
            (leaf_moles / WATER_MOLES_PER_MM, source_ratio)
        };
        let new_leaf_water = leaf_moles / WATER_MOLES_PER_MM;
        let mut new_bulk_ratio = self.delta_to_ratio(new.delta_b);
        let prev_storage = prev_leaf_water * prev_bulk_ratio;
        let storage_tracer_change = new_leaf_water.mul_add(new_bulk_ratio, -prev_storage);
        let mut storage_tracer_change_used = storage_tracer_change;
        let storage_scale = (transp_water * source_ratio).max(TRC_TINY);
        let storage_bound = 0.95 * storage_scale;
        if storage_tracer_change.abs() > 10.0 * storage_scale {
            storage_tracer_change_used = 0.0;
        } else {
            storage_tracer_change_used = storage_tracer_change_used
                .max(-storage_bound)
                .min(storage_bound);
        }
        if (storage_tracer_change_used - storage_tracer_change).abs() > TRC_TINY {
            let target_leaf_storage = prev_storage + storage_tracer_change_used;
            if new_leaf_water > TRC_TINY && target_leaf_storage > TRC_TINY {
                new_bulk_ratio = target_leaf_storage / new_leaf_water;
                new.delta_b = self.ratio_to_delta(new_bulk_ratio);
                new.delta_e = if new.peclet > 1.0e-6 {
                    delta_x + (new.delta_b - delta_x) / new.peclet
                } else {
                    new.delta_b
                };
            } else {
                storage_tracer_change_used = 0.0;
                new.delta_e = delta_x;
                new.delta_b = delta_x;
            }
        }
        trans_ratio =
            transp_water.mul_add(source_ratio, -storage_tracer_change_used) / transp_water;
        if trans_ratio.is_nan() || trans_ratio <= 0.0 {
            trans_ratio = source_ratio;
            new.delta_e = prev.delta_e;
            new.delta_b = prev.delta_b;
        }
        if new.delta_b.is_nan() {
            new.delta_b = prev.delta_b;
        }
        (trans_ratio, new)
    }
}

/// 叶片水非稳态模型的状态（`trc_leaf_delta_e/_delta_b/_peclet/_water_moles`）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LeafWaterState {
    pub delta_e: f64,
    pub delta_b: f64,
    pub peclet: f64,
    pub leaf_moles: f64,
}

/// `tracer_jm84_effective_alpha`：Jouzel & Merlivat (1984) 过饱和凝华的有效冰/汽分馏系数。
pub fn jm84_effective_alpha(alpha_eq: f64, diff_ratio: f64, tc: f64, slope: f64) -> f64 {
    if slope <= 0.0 || tc >= 0.0 {
        return alpha_eq;
    }
    let s = (-slope).mul_add(tc, 1.0);
    let denom = (alpha_eq * diff_ratio).mul_add(s - 1.0, 1.0);
    if denom <= 0.0 {
        return alpha_eq;
    }
    alpha_eq * s / denom
}

/// `tracer_ice_deposition_alpha`：冰点以上取平衡值，253.15 K 以下取完整动力值，其间线性过渡。
pub fn ice_deposition_alpha(
    temp_k: f64,
    slope: f64,
    diff_ratio: f64,
    alpha_eq_at_t: f64,
    alpha_eq_at_tfrz: f64,
    alpha_eq_at_tcold: f64,
) -> f64 {
    if slope <= 0.0 || temp_k >= TFRZ {
        return alpha_eq_at_t;
    }
    if temp_k <= JM84_FULL_KINETIC_TEMP {
        return jm84_effective_alpha(alpha_eq_at_t, diff_ratio, temp_k - TFRZ, slope);
    }
    let alpha_cold = jm84_effective_alpha(
        alpha_eq_at_tcold,
        diff_ratio,
        JM84_FULL_KINETIC_TEMP - TFRZ,
        slope,
    );
    let span = TFRZ - JM84_FULL_KINETIC_TEMP;
    alpha_eq_at_tfrz.mul_add(
        temp_k - JM84_FULL_KINETIC_TEMP,
        (TFRZ - temp_k) * alpha_cold,
    ) / span
}

/// `tracer_mj79_kinetic_alpha`：Merlivat & Jouzel (1979) 自由水面的动力分馏系数 `1/(1−k)`。
pub fn mj79_kinetic_alpha(wind: f64, relative_factor: f64) -> f64 {
    let u = wind.max(0.0);
    let mut k = if u >= MJ79_WIND_THRESHOLD {
        u.mul_add(MJ79_ROUGH_SLOPE, MJ79_ROUGH_OFFSET)
    } else {
        MJ79_SMOOTH_K
    };
    k *= relative_factor.max(0.0);
    k = k.max(0.0).min(0.5);
    1.0 / (1.0 - k)
}

/// `tracer_soil_kinetic_alpha_core`：`(ra·D^(2/3) + rs·D)/(ra+rs)`。
pub fn soil_kinetic_alpha_core(ra: f64, rs: f64, diff_ratio: f64) -> f64 {
    let (ra1, rs1) = (ra.max(0.0), rs.max(0.0));
    let denom = ra1 + rs1;
    if denom <= 0.0 || diff_ratio <= 0.0 {
        return 1.0;
    }
    diff_ratio.lpow(TWO_THIRDS).mul_add(ra1, diff_ratio * rs1) / denom
}

/// `tracer_soil_effective_diffusivity`：Millington-Quirk，`D·θ^(7/3)/φ²` [m²/s]。
pub fn soil_effective_diffusivity(water_mm: f64, dz: f64, porsl: f64, diff_liquid: f64) -> f64 {
    if water_mm <= 0.0 || dz <= 0.0 || porsl <= 0.0 || diff_liquid <= 0.0 {
        return 0.0;
    }
    let theta = (water_mm / (dz * 1000.0)).min(porsl);
    if theta <= 0.0 {
        return 0.0;
    }
    diff_liquid * theta.lpow(SEVEN_THIRDS) / (porsl * porsl)
}

/// `D_vap_air·(T/273.15)²·(p_ref/p)`：标准状况 2.12e-5 m²/s。
fn vapor_diffusivity_air(tk: f64, psrf: f64) -> f64 {
    let scaled = tk / 273.15;
    scaled * scaled * 2.12e-5 * (101325.0 / psrf)
}

/// `tracer_soil_vapor_equivalent_diffusivity`：土壤孔隙气相扩散折成的等效液相扩散系数。
#[allow(clippy::too_many_arguments)]
pub fn soil_vapor_equivalent_diffusivity(
    water_mm: f64,
    ice_mm: f64,
    dz: f64,
    porsl: f64,
    temp_k: f64,
    psrf: f64,
    diff_ratio: f64,
    alpha_liq_vap: f64,
) -> f64 {
    if dz <= 0.0 || porsl <= 0.0 || psrf <= 0.0 || diff_ratio <= 0.0 || alpha_liq_vap <= 0.0 {
        return 0.0;
    }
    let thickness = dz * 1000.0;
    let theta = water_mm.max(0.0) / thickness;
    let theta_ice = ice_mm.max(0.0) / thickness;
    let air_porosity = porsl - (theta + theta_ice).min(porsl);
    if air_porosity <= 0.0 {
        return 0.0;
    }
    let tau_gas = air_porosity.lpow(SEVEN_THIRDS) / (porsl * porsl);
    let tk = temp_k.max(150.0);
    let d_vap_air = vapor_diffusivity_air(tk, psrf);
    let rho_v_sat = saturation_vapor_pressure(tk, false) / (tk * 461.5);
    tau_gas * d_vap_air / diff_ratio * rho_v_sat / (alpha_liq_vap * 1000.0)
}

/// `tracer_snow_vapor_equivalent_diffusivity`：雪层孔隙气相扩散折成的等效液相扩散系数。
pub fn snow_vapor_equivalent_diffusivity(
    wliq_mm: f64,
    wice_mm: f64,
    dz: f64,
    temp_k: f64,
    psrf: f64,
    diff_ratio: f64,
    alpha_ice_vap: f64,
) -> f64 {
    if dz <= 0.0 || psrf <= 0.0 || diff_ratio <= 0.0 || alpha_ice_vap <= 0.0 {
        return 0.0;
    }
    let solid_fraction = wice_mm.max(0.0) / (dz * 917.0);
    let liquid_fraction = wliq_mm.max(0.0) / (dz * 1000.0);
    let phi = 1.0 - (solid_fraction + liquid_fraction).min(1.0);
    if phi <= 0.0 {
        return 0.0;
    }
    let tau_gas = phi.lpow(SEVEN_THIRDS);
    let tk = temp_k.max(150.0);
    let d_vap_air = vapor_diffusivity_air(tk, psrf);
    let rho_v_sat = saturation_vapor_pressure(tk, true) / (tk * 461.5);
    tau_gas * d_vap_air / diff_ratio * rho_v_sat / (alpha_ice_vap * 1000.0)
}

/// `tracer_soil_diffusive_transfer`：相邻两层之间一步的扩散交换量（向下为正），不超过把两层
/// 拉平所需的量。
#[allow(clippy::too_many_arguments)]
pub fn soil_diffusive_transfer(
    ratio_upper: f64,
    ratio_lower: f64,
    water_upper: f64,
    water_lower: f64,
    d_eff_upper: f64,
    d_eff_lower: f64,
    dz_upper: f64,
    dz_lower: f64,
    deltim: f64,
) -> f64 {
    if water_upper <= 0.0 || water_lower <= 0.0 || d_eff_upper <= 0.0 || d_eff_lower <= 0.0 {
        return 0.0;
    }
    if dz_upper <= 0.0 || dz_lower <= 0.0 || deltim <= 0.0 {
        return 0.0;
    }
    let gradient = ratio_upper - ratio_lower;
    if gradient == 0.0 {
        return 0.0;
    }
    let thickness = dz_upper + dz_lower;
    let d_harmonic = thickness / (dz_upper / d_eff_upper + dz_lower / d_eff_lower);
    let dz_mid = thickness * 0.5;
    let transfer = gradient * d_harmonic / dz_mid * 1000.0 * deltim;
    let equalise = gradient / (1.0 / water_upper + 1.0 / water_lower);
    if transfer.abs() > equalise.abs() {
        equalise
    } else {
        transfer
    }
}

/// `tracer_equilibration_exchange`：池子向 `alpha_eq·R_vapor` 平衡的交换量（取走不超过现存量）。
pub fn equilibration_exchange(
    pool_trc: f64,
    pool_water: f64,
    vapor_ratio: f64,
    alpha_eq: f64,
    equilibration_fraction: f64,
) -> f64 {
    if pool_water <= 0.0 || alpha_eq <= 0.0 || vapor_ratio < 0.0 {
        return 0.0;
    }
    let f = equilibration_fraction.max(0.0).min(1.0);
    if f <= 0.0 {
        return 0.0;
    }
    let ratio_old = pool_trc / pool_water;
    let exchange = pool_water * f * alpha_eq.mul_add(vapor_ratio, -ratio_old);
    if exchange < 0.0 {
        -(pool_trc.max(0.0).min(-exchange))
    } else {
        exchange
    }
}

/// `tracer_craig_gordon_ratio_core`：`(R/α_eq − h·R_v)/(α_k·(1−h))`，夹在 ±10·R_eq。
pub fn craig_gordon_ratio_core(
    source_ratio: f64,
    vapor_ratio: f64,
    alpha_eq: f64,
    alpha_k: f64,
    relhum: f64,
    relhum_max: f64,
) -> f64 {
    if alpha_eq <= 0.0 || alpha_k <= 0.0 {
        return source_ratio;
    }
    let equilibrium_ratio = source_ratio / alpha_eq;
    let h = relhum.max(0.0).min(relhum_max.min(0.999999).max(0.0));
    let one_minus_h = 1.0 - h;
    if one_minus_h <= 0.0 {
        return equilibrium_ratio;
    }
    let ratio = (-h).mul_add(vapor_ratio, equilibrium_ratio) / (alpha_k * one_minus_h);
    let bound = CRAIG_GORDON_MAX_RATIO_AMPLIFICATION * equilibrium_ratio.abs();
    ratio.max(-bound).min(bound)
}

/// `tracer_saturation_vapor_pressure`：Magnus 式 [Pa]（与陆面的 `qsadv` 不同）。
pub fn saturation_vapor_pressure(temp_k: f64, over_ice: bool) -> f64 {
    let tc = temp_k.max(150.0) - TFRZ;
    if over_ice {
        (tc * 22.46 / (tc + 272.62)).exp() * 611.2
    } else {
        (tc * 17.67 / (tc + 243.5)).exp() * 611.2
    }
}

/// `tracer_surface_relhum`：由比湿与气压求相对湿度，夹在 `[0, 0.999999]`。
pub fn surface_relhum(qair: f64, psrf: f64, temp_k: f64, over_ice: bool) -> f64 {
    let qsafe = qair.max(0.0);
    let psafe = psrf.max(1.0);
    let eair = qsafe * psafe / qsafe.mul_add(0.378, 0.622);
    let esat = saturation_vapor_pressure(temp_k, over_ice);
    if esat > TRC_TINY {
        (eair / esat).max(0.0).min(0.999999)
    } else {
        0.0
    }
}

#[cfg(test)]
#[path = "isotope_fractionation_tests.rs"]
mod isotope_fractionation_tests;
