//! 同位素分馏物理（`MOD_Tracer_Frac`、`MOD_Tracer_Isotope_Registry`、`MOD_Tracer_Isotope_O18`、
//! `MOD_Tracer_Isotope_HDO`）。
//!
//! 上游的注册表是运行期的函数指针表，按名字模式（再退到 `ref_ratio` 相近）把示踪物映射到
//! 已实现的物种；这里物种是封闭的枚举 [`IsotopeSpecies`]，匹配规则相同。没匹配上的同位素
//! 照样是同位素，只是"分馏不生效"（α = 1，扩散比 = 1）。
//!
//! 运算次序与融合逐条对照 gfortran -O2 的 GIMPLE（`MOD_Tracer_Frac.F90.273t.optimized`）：
//! 常数指数的 `**(7/3)`、`**(2/3)` 走 libm `pow`（[`LibmPow`]），`**2` 是自乘。

// 夹紧保留上游 `MIN(MAX(·))` 的次序（`clamp` 对 NaN 的行为不同）。
#![allow(clippy::manual_clamp)]

use crate::LibmPow;

use super::{TracerDescriptor, TracerPhysics, TRC_TINY};

const TFRZ: f64 = 273.15;
/// `water_moles_per_mm = 1000/18.01528`。
const WATER_MOLES_PER_MM: f64 = 1000.0 / 18.01528;
const LIQUID_WATER_MOLAR_DENSITY: f64 = 55.5e3;
const UNIVERSAL_GAS_CONSTANT: f64 = 8.314_462_618_153_24;
const CRAIG_GORDON_MAX_RATIO_AMPLIFICATION: f64 = 10.0;
const CG_EXPONENT_LIQUID: f64 = 2.0 / 3.0;
const JM84_FULL_KINETIC_TEMP: f64 = 253.15;
const MJ79_SMOOTH_K: f64 = 0.006;
const MJ79_ROUGH_SLOPE: f64 = 0.000285;
const MJ79_ROUGH_OFFSET: f64 = 0.00082;
const MJ79_WIND_THRESHOLD: f64 = 7.0;
const SEVEN_THIRDS: f64 = 7.0 / 3.0;
/// `MOD_Tracer_Defs`：`Rsmow_18O`、`Rsmow_D`。
pub const RSMOW_18O: f64 = 2.0052e-3;
pub const RSMOW_D: f64 = 1.5576e-4;

/// `DEF_TRACER_KINETIC_SCHEME`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KineticScheme {
    #[default]
    Merlivat1978,
    Cappa2003,
}

impl KineticScheme {
    /// 上游的规范化（`MOD_Namelist.F90:1703-1710`）：大小写的三种写法，其余停机。
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "CAPPA2003" | "Cappa2003" | "cappa2003" => Some(Self::Cappa2003),
            "MERLIVAT1978" | "Merlivat1978" | "merlivat1978" => Some(Self::Merlivat1978),
            _ => None,
        }
    }
}

/// `DEF_TRACER_OPEN_WATER_KINETIC`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenWaterKinetic {
    #[default]
    Mj79,
    Exponent,
}

impl OpenWaterKinetic {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "MJ79" | "mj79" | "MERLIVAT_JOUZEL1979" => Some(Self::Mj79),
            "EXPONENT" | "exponent" => Some(Self::Exponent),
            _ => None,
        }
    }
}

/// 已实现分馏物理的同位素（`tracer_isotope_species.inc` 的顺序：O18、HDO）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsotopeSpecies {
    O18,
    Hdo,
}

impl IsotopeSpecies {
    const ALL: [Self; 2] = [Self::O18, Self::Hdo];

    fn name_patterns(self) -> &'static str {
        match self {
            Self::O18 => "18o,o18",
            Self::Hdo => "hdo,2h,deuter,=h2",
        }
    }

    fn ref_ratio_hint(self) -> f64 {
        match self {
            Self::O18 => RSMOW_18O,
            Self::Hdo => RSMOW_D,
        }
    }

    /// `mj79_relative_factor`。
    pub fn mj79_relative_factor(self) -> f64 {
        match self {
            Self::O18 => 1.0,
            Self::Hdo => 0.88,
        }
    }

    /// `default_soil_init_varname`。
    pub fn default_soil_init_varname(self) -> &'static str {
        match self {
            Self::O18 => "soilwat_O18",
            Self::Hdo => "soilwat_H2",
        }
    }

    /// `o18_alpha_liq_vap`/`hdo_alpha_liq_vap`。
    pub fn alpha_liq_vap(self, temp_k: f64) -> f64 {
        let tk = temp_k.max(150.0);
        match self {
            Self::O18 => ((1137.0 / (tk * tk) - 0.4156 / tk) - 0.0020667).exp(),
            Self::Hdo => ((24844.0 / (tk * tk) - 76.248 / tk) + 0.052612).exp(),
        }
    }

    /// `o18_alpha_ice_vap`/`hdo_alpha_ice_vap`。
    pub fn alpha_ice_vap(self, temp_k: f64) -> f64 {
        let tk = temp_k.max(150.0);
        match self {
            Self::O18 => (11.839 / tk - 0.028224).exp(),
            Self::Hdo => (16289.0 / (tk * tk) - 0.0945).exp(),
        }
    }

    /// `o18_diffusivity_ratio`/`hdo_diffusivity_ratio`。
    pub fn diffusivity_ratio_air(self, scheme: KineticScheme) -> f64 {
        match (self, scheme) {
            (Self::O18, KineticScheme::Merlivat1978) => 1.0285,
            (Self::O18, KineticScheme::Cappa2003) => 1.03189,
            (Self::Hdo, KineticScheme::Merlivat1978) => 1.0251,
            (Self::Hdo, KineticScheme::Cappa2003) => 1.01636,
        }
    }

    /// `o18_leaf_liquid_diffusivity`/`hdo_leaf_liquid_diffusivity`：`exp(-c/x)*d`。
    pub fn leaf_liquid_diffusivity(self, temp_k: f64) -> f64 {
        let tk = temp_k.max(150.0);
        match self {
            Self::O18 => (-(637.0 / (tk - 137.0).max(1.0))).exp() * 119.0e-9,
            Self::Hdo => (-(626.0 / (tk - 139.0).max(1.0))).exp() * 116.0e-9,
        }
    }

    /// `find_isotope_physics`：只对同位素；先按名字模式（小写子串，`=` 前缀为全等），
    /// 再按 `|ref_ratio - hint|/hint < 0.1`。
    pub fn find(tracer: &TracerDescriptor) -> Option<Self> {
        if !tracer.is_isotope() {
            return None;
        }
        let lname = tracer.name.trim().to_ascii_lowercase();
        let by_name = Self::ALL.into_iter().find(|species| {
            species.name_patterns().split(',').any(|token| {
                let token = token.trim();
                match token.strip_prefix('=') {
                    Some(exact) => lname == exact,
                    None => !token.is_empty() && lname.contains(token),
                }
            })
        });
        if by_name.is_some() {
            return by_name;
        }
        if tracer.ref_ratio > 0.0 {
            return Self::ALL.into_iter().find(|species| {
                let hint = species.ref_ratio_hint();
                hint > TRC_TINY && (tracer.ref_ratio - hint).abs() / hint < 0.1
            });
        }
        None
    }
}

impl TracerPhysics {
    fn species(tracer: &TracerDescriptor) -> Option<IsotopeSpecies> {
        IsotopeSpecies::find(tracer)
    }

    /// `tracer_alpha_liq_vap`（未注册时 1）。
    pub fn alpha_liq_vap(&self, tracer: &TracerDescriptor, temp_k: f64) -> f64 {
        Self::species(tracer).map_or(1.0, |species| species.alpha_liq_vap(temp_k))
    }

    /// `tracer_alpha_ice_vap`。
    pub fn alpha_ice_vap(&self, tracer: &TracerDescriptor, temp_k: f64) -> f64 {
        Self::species(tracer).map_or(1.0, |species| species.alpha_ice_vap(temp_k))
    }

    /// `tracer_alpha_ice_liq = alpha_ice_vap / max(alpha_liq_vap, trc_tiny)`。
    pub fn alpha_ice_liq(&self, tracer: &TracerDescriptor, temp_k: f64) -> f64 {
        let liq = self.alpha_liq_vap(tracer, temp_k).max(TRC_TINY);
        self.alpha_ice_vap(tracer, temp_k) / liq
    }

    /// `tracer_diffusivity_ratio_air`。
    pub fn diffusivity_ratio_air(&self, tracer: &TracerDescriptor) -> f64 {
        Self::species(tracer).map_or(1.0, |species| {
            species.diffusivity_ratio_air(self.kinetic_scheme)
        })
    }

    /// `tracer_leaf_liquid_diffusivity`/`tracer_liquid_self_diffusivity`（未注册时 0）。
    pub fn leaf_liquid_diffusivity(&self, tracer: &TracerDescriptor, temp_k: f64) -> f64 {
        Self::species(tracer).map_or(0.0, |species| species.leaf_liquid_diffusivity(temp_k))
    }

    /// `tracer_rayleigh_freezing_loss`。
    pub fn rayleigh_freezing_loss(
        &self,
        tracer: &TracerDescriptor,
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
        let proportional = (freeze_water * source_ratio).min(pool_trc.max(0.0));
        if !self.fractionation_active(tracer) {
            return proportional;
        }
        let alpha_il = self.alpha_ice_liq(tracer, temp_k);
        if alpha_il <= TRC_TINY || alpha_il.is_nan() {
            return proportional;
        }
        let remaining_water = pool_water - freeze_water;
        let liquid_fraction = (remaining_water / pool_water).max(TRC_TINY);
        let remaining_ratio = liquid_fraction.lpow(alpha_il - 1.0) * source_ratio;
        // `:153` `.FNMA (remaining_water, remaining_ratio, pool_trc)`
        (-remaining_water)
            .mul_add(remaining_ratio, pool_trc)
            .max(0.0)
            .min(pool_trc.max(0.0))
    }

    /// `tracer_alpha_ice_vap_deposition`。
    pub fn alpha_ice_vap_deposition(&self, tracer: &TracerDescriptor, temp_k: f64) -> f64 {
        ice_deposition_alpha(
            temp_k,
            self.ice_supersat_slope,
            self.diffusivity_ratio_air(tracer),
            self.alpha_ice_vap(tracer, temp_k),
            self.alpha_ice_vap(tracer, TFRZ),
            self.alpha_ice_vap(tracer, JM84_FULL_KINETIC_TEMP),
        )
    }

    /// `tracer_alpha_kinetic_craig_gordon`：冰 `D^1`，液 `D^(2/3)`。
    pub fn alpha_kinetic_craig_gordon(&self, tracer: &TracerDescriptor, from_ice: bool) -> f64 {
        let ratio = self.diffusivity_ratio_air(tracer);
        if from_ice {
            ratio
        } else {
            ratio.lpow(CG_EXPONENT_LIQUID)
        }
    }

    /// `tracer_alpha_kinetic_open_water`。
    pub fn alpha_kinetic_open_water(&self, tracer: &TracerDescriptor, wind: f64) -> f64 {
        match self.open_water_kinetic {
            OpenWaterKinetic::Exponent => self.alpha_kinetic_craig_gordon(tracer, false),
            OpenWaterKinetic::Mj79 => mj79_kinetic_alpha(
                wind,
                Self::species(tracer).map_or(1.0, IsotopeSpecies::mj79_relative_factor),
            ),
        }
    }

    /// `tracer_alpha_kinetic_leaf`：`(ra + rb·D^(2/3) + rc·D)/(ra+rb+rc)`，
    /// GIMPLE `FMA(rc, D, FMA(D^(2/3), rb, ra))`。
    pub fn alpha_kinetic_leaf(&self, tracer: &TracerDescriptor, ra: f64, rb: f64, rc: f64) -> f64 {
        let (ra1, rb1, rc1) = (ra.max(0.0), rb.max(0.0), rc.max(0.0));
        let denom = (ra1 + rb1) + rc1;
        if denom <= TRC_TINY {
            return 1.0;
        }
        let diff_ratio = self.diffusivity_ratio_air(tracer);
        let turbulent = diff_ratio.lpow(CG_EXPONENT_LIQUID).mul_add(rb1, ra1);
        rc1.mul_add(diff_ratio, turbulent) / denom
    }

    /// `tracer_alpha_kinetic_soil`。
    pub fn alpha_kinetic_soil(&self, tracer: &TracerDescriptor, ra: f64, rs: f64) -> f64 {
        soil_kinetic_alpha_core(ra, rs, self.diffusivity_ratio_air(tracer))
    }

    /// `tracer_craig_gordon_evap_ratio`。
    #[allow(clippy::too_many_arguments)]
    pub fn craig_gordon_evap_ratio(
        &self,
        tracer: &TracerDescriptor,
        source_ratio: f64,
        vapor_ratio: f64,
        temp_k: f64,
        relhum: f64,
        alpha_k: f64,
        from_ice: bool,
    ) -> f64 {
        if !self.fractionation_active(tracer) {
            return source_ratio;
        }
        let alpha_eq = if from_ice {
            self.alpha_ice_vap(tracer, temp_k)
        } else {
            self.alpha_liq_vap(tracer, temp_k)
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
            self.cg_relhum_max,
        );
        if ratio.is_nan() {
            source_ratio
        } else {
            ratio
        }
    }

    /// `tracer_equilibrium_deposition_ratio`。
    pub fn equilibrium_deposition_ratio(
        &self,
        tracer: &TracerDescriptor,
        vapor_ratio: f64,
        temp_k: f64,
        from_ice: bool,
    ) -> f64 {
        if !self.fractionation_active(tracer) {
            return vapor_ratio;
        }
        if from_ice {
            vapor_ratio * self.alpha_ice_vap_deposition(tracer, temp_k)
        } else {
            vapor_ratio * self.alpha_liq_vap(tracer, temp_k)
        }
    }

    /// `tracer_ratio_to_delta`：`(R/R_ref - 1)*1000`（`R_ref <= tiny` 时 0）。
    pub fn ratio_to_delta(tracer: &TracerDescriptor, ratio: f64) -> f64 {
        if tracer.ref_ratio > TRC_TINY {
            (ratio / tracer.ref_ratio - 1.0) * 1000.0
        } else {
            0.0
        }
    }

    /// `tracer_delta_to_ratio`：`(δ/1000 + 1)*R_ref`。
    pub fn delta_to_ratio(tracer: &TracerDescriptor, delta: f64) -> f64 {
        if tracer.ref_ratio > TRC_TINY {
            (delta / 1000.0 + 1.0) * tracer.ref_ratio
        } else {
            0.0
        }
    }

    /// `tracer_transpiration_nss_ratio`（Farquhar–Cernusak 非稳态叶水）。
    pub fn transpiration_nss_ratio(
        &self,
        tracer: &TracerDescriptor,
        input: &NssInput,
    ) -> NssOutput {
        let source_ratio = input.source_ratio;
        let initial_delta = Self::ratio_to_delta(tracer, source_ratio);
        let mut out = NssOutput {
            trans_ratio: source_ratio,
            new_delta_e: initial_delta,
            new_delta_b: initial_delta,
            new_peclet: 1.0,
            new_leaf_moles: 0.0,
        };
        if !self.fractionation_active(tracer) {
            return out;
        }
        if source_ratio <= TRC_TINY || input.vapor_ratio <= TRC_TINY {
            return out;
        }
        let transp_water = input.transp_water;
        let deltim = input.deltim;
        if transp_water <= TRC_TINY || deltim <= TRC_TINY {
            return out;
        }
        let leaf_moles =
            (input.leaf_area.max(0.0) * self.nss_leaf_water_per_lai.max(0.0)) * WATER_MOLES_PER_MM;
        out.new_leaf_moles = leaf_moles;
        if leaf_moles <= TRC_TINY {
            return out;
        }
        let tk = input.temp_k.max(150.0);
        let h = input
            .relhum
            .max(0.0)
            .min(self.cg_relhum_max.min(0.999_999).max(0.0));
        let one_minus_h = (1.0 - h).max(1.0e-6);
        let alpha_eq = self.alpha_liq_vap(tracer, tk);
        let eps_eq = (alpha_eq - 1.0) * 1000.0;
        let leaf_area_safe = input.leaf_area.max(TRC_TINY);
        let alpha_k = self.alpha_kinetic_leaf(
            tracer,
            input.aerodynamic_resistance,
            self.nss_leaf_rb / leaf_area_safe,
            input.stomatal_resistance,
        );
        let eps_k = (alpha_k - 1.0) * 1000.0;
        let delta_x = Self::ratio_to_delta(tracer, source_ratio);
        let delta_v = Self::ratio_to_delta(tracer, input.vapor_ratio);
        // GIMPLE `FMA((δv - ε_k) - δx, h, (ε_k + δx) + ε_eq)`。
        let delta_es = ((delta_v - eps_k) - delta_x).mul_add(h, (eps_k + delta_x) + eps_eq);

        let transp_moles = transp_water * WATER_MOLES_PER_MM;
        let transp_moles_leaf_s = transp_moles / (deltim * leaf_area_safe);
        let liquid_diff = self.leaf_liquid_diffusivity(tracer, tk);
        let mut peclet = out.new_peclet;
        if liquid_diff > TRC_TINY && self.nss_leaf_path_length > 0.0 {
            let peclet_number = (self.nss_leaf_path_length * transp_moles_leaf_s)
                / (liquid_diff * LIQUID_WATER_MOLAR_DENSITY);
            peclet = if peclet_number > 1.0e-8 {
                (1.0 - (-peclet_number).exp()) / peclet_number
            } else {
                (-peclet_number).mul_add(0.5, 1.0)
            };
        }
        let peclet = peclet.max(0.0).min(1.0);
        out.new_peclet = peclet;

        let (prev_w, prev_e, prev_p) = if input.prev_leaf_moles > TRC_TINY {
            let leaf_moles_ratio = input.prev_leaf_moles / leaf_moles.max(TRC_TINY);
            let prev_w = if leaf_moles_ratio > 0.25 && leaf_moles_ratio < 4.0 {
                input.prev_leaf_moles
            } else {
                leaf_moles
            };
            (
                prev_w,
                input.prev_delta_e,
                input.prev_peclet.max(0.0).min(1.0),
            )
        } else {
            (leaf_moles, delta_x, peclet)
        };

        let gross_moles = transp_moles / one_minus_h;
        let mut conductance_gross_moles = 0.0;
        let total_resistance = (input.aerodynamic_resistance.max(0.0)
            + input.stomatal_resistance.max(0.0))
            + self.nss_leaf_rb.max(0.0) / leaf_area_safe;
        if total_resistance > TRC_TINY && input.psrf > TRC_TINY {
            let vapor_molar_density_sat =
                saturation_vapor_pressure(tk, false) / (tk * UNIVERSAL_GAS_CONSTANT);
            conductance_gross_moles = (deltim * vapor_molar_density_sat) / total_resistance;
        }
        let gross_moles = conductance_gross_moles.max(gross_moles.max(TRC_TINY));
        let relax_b = (alpha_k * alpha_eq) / gross_moles;
        let denom = (leaf_moles * relax_b).mul_add(peclet, 1.0);
        out.new_delta_e = if denom > TRC_TINY {
            let memory = (prev_p * prev_w) * (prev_e - delta_x);
            let numerator = (leaf_moles * peclet).mul_add(delta_x, memory);
            numerator.mul_add(relax_b, delta_es) / denom
        } else {
            delta_es
        };
        out.new_delta_b = (out.new_delta_e - delta_x).mul_add(peclet, delta_x);

        let (prev_leaf_water, prev_bulk_ratio) = if input.prev_leaf_moles > TRC_TINY {
            (
                prev_w / WATER_MOLES_PER_MM,
                Self::delta_to_ratio(tracer, input.prev_delta_b),
            )
        } else {
            (leaf_moles / WATER_MOLES_PER_MM, source_ratio)
        };
        let new_leaf_water = leaf_moles / WATER_MOLES_PER_MM;
        let new_bulk_ratio = Self::delta_to_ratio(tracer, out.new_delta_b);
        let previous_storage = prev_leaf_water * prev_bulk_ratio;
        let storage_tracer_change = new_bulk_ratio.mul_add(new_leaf_water, -previous_storage);
        let storage_scale = (source_ratio * transp_water).max(TRC_TINY);
        let storage_bound = storage_scale * 0.95;
        let storage_tracer_change_used = if storage_tracer_change.abs() > storage_scale * 10.0 {
            0.0
        } else {
            storage_tracer_change.max(-storage_bound).min(storage_bound)
        };
        let mut used = storage_tracer_change_used;
        if (used - storage_tracer_change).abs() > TRC_TINY {
            let target_leaf_storage = used + previous_storage;
            if new_leaf_water > TRC_TINY && target_leaf_storage > TRC_TINY {
                out.new_delta_b =
                    Self::ratio_to_delta(tracer, target_leaf_storage / new_leaf_water);
                out.new_delta_e = if peclet > 1.0e-6 {
                    (out.new_delta_b - delta_x) / peclet + delta_x
                } else {
                    out.new_delta_b
                };
            } else {
                used = 0.0;
                out.new_delta_e = delta_x;
                out.new_delta_b = delta_x;
            }
        }
        out.trans_ratio = (source_ratio * transp_water - used) / transp_water;
        if out.trans_ratio.is_nan() || out.trans_ratio <= 0.0 {
            out.trans_ratio = source_ratio;
            out.new_delta_e = input.prev_delta_e;
            out.new_delta_b = input.prev_delta_b;
        }
        if out.new_delta_b.is_nan() {
            out.new_delta_b = input.prev_delta_b;
        }
        out
    }
}

/// `tracer_transpiration_nss_ratio` 的输入（`prev_*` 是上一步的叶水状态）。
#[derive(Debug, Clone, Copy)]
pub struct NssInput {
    pub source_ratio: f64,
    pub vapor_ratio: f64,
    pub temp_k: f64,
    pub relhum: f64,
    pub psrf: f64,
    pub transp_water: f64,
    pub deltim: f64,
    pub leaf_area: f64,
    pub aerodynamic_resistance: f64,
    pub stomatal_resistance: f64,
    pub prev_delta_e: f64,
    pub prev_delta_b: f64,
    pub prev_peclet: f64,
    pub prev_leaf_moles: f64,
}

/// `tracer_transpiration_nss_ratio` 的输出。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NssOutput {
    pub trans_ratio: f64,
    pub new_delta_e: f64,
    pub new_delta_b: f64,
    pub new_peclet: f64,
    pub new_leaf_moles: f64,
}

/// `tracer_jm84_effective_alpha`。
pub fn jm84_effective_alpha(alpha_eq: f64, diff_ratio: f64, tc: f64, slope: f64) -> f64 {
    if slope <= 0.0 || tc >= 0.0 {
        return alpha_eq;
    }
    let s = (-slope).mul_add(tc, 1.0);
    let denom = (alpha_eq * diff_ratio).mul_add(s - 1.0, 1.0);
    if denom <= 0.0 {
        return alpha_eq;
    }
    (alpha_eq * s) / denom
}

/// `tracer_ice_deposition_alpha`。中段 GIMPLE：`FMA(α_tfrz, T-253.15, (tfrz-T)·α_cold) / span`，
/// `span` 与冷端 `tc` 都是编译期折叠的常数。
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

/// `tracer_mj79_kinetic_alpha`。
pub fn mj79_kinetic_alpha(wind: f64, relative_factor: f64) -> f64 {
    let u = wind.max(0.0);
    let k = if u >= MJ79_WIND_THRESHOLD {
        u.mul_add(MJ79_ROUGH_SLOPE, MJ79_ROUGH_OFFSET)
    } else {
        MJ79_SMOOTH_K
    };
    let k = (k * relative_factor.max(0.0)).max(0.0).min(0.5);
    1.0 / (1.0 - k)
}

/// `tracer_soil_kinetic_alpha_core`：GIMPLE `FMA(D^(2/3), ra, D·rs) / (ra+rs)`。
pub fn soil_kinetic_alpha_core(ra: f64, rs: f64, diff_ratio: f64) -> f64 {
    let (ra1, rs1) = (ra.max(0.0), rs.max(0.0));
    let denom = ra1 + rs1;
    if denom <= 0.0 || diff_ratio <= 0.0 {
        return 1.0;
    }
    diff_ratio
        .lpow(CG_EXPONENT_LIQUID)
        .mul_add(ra1, diff_ratio * rs1)
        / denom
}

/// `tracer_soil_effective_diffusivity`：`D·θ^(7/3)/φ²`。
pub fn soil_effective_diffusivity(water_mm: f64, dz: f64, porsl: f64, diff_liquid: f64) -> f64 {
    if water_mm <= 0.0 || dz <= 0.0 || porsl <= 0.0 || diff_liquid <= 0.0 {
        return 0.0;
    }
    let theta = (water_mm / (dz * 1000.0)).min(porsl);
    if theta <= 0.0 {
        return 0.0;
    }
    (diff_liquid * theta.lpow(SEVEN_THIRDS)) / (porsl * porsl)
}

/// 两个等效扩散率共用的气相部分：`((tk/273.15)²·2.12e-5)·(p_ref/psrf)` 与 `e_sat/(Rv·tk)`。
fn vapor_terms(temp_k: f64, psrf: f64, over_ice: bool) -> (f64, f64) {
    let tk = temp_k.max(150.0);
    let scaled = tk / TFRZ;
    let d_vap_air = ((scaled * scaled) * 2.12e-5) * (101_325.0 / psrf);
    let rho_v_sat = saturation_vapor_pressure(tk, over_ice) / (tk * 461.5);
    (d_vap_air, rho_v_sat)
}

/// `tracer_soil_vapor_equivalent_diffusivity`。
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
    if dz <= 0.0 || porsl <= 0.0 {
        return 0.0;
    }
    if psrf <= 0.0 || diff_ratio <= 0.0 || alpha_liq_vap <= 0.0 {
        return 0.0;
    }
    let layer = dz * 1000.0;
    let theta = water_mm.max(0.0) / layer;
    let theta_ice = ice_mm.max(0.0) / layer;
    let air_porosity = porsl - (theta + theta_ice).min(porsl);
    if air_porosity <= 0.0 {
        return 0.0;
    }
    let tau_gas = air_porosity.lpow(SEVEN_THIRDS) / (porsl * porsl);
    let (d_vap_air, rho_v_sat) = vapor_terms(temp_k, psrf, false);
    (((tau_gas * d_vap_air) / diff_ratio) * rho_v_sat) / (alpha_liq_vap * 1000.0)
}

/// `tracer_snow_vapor_equivalent_diffusivity`。
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
    let (d_vap_air, rho_v_sat) = vapor_terms(temp_k, psrf, true);
    (((tau_gas * d_vap_air) / diff_ratio) * rho_v_sat) / (alpha_ice_vap * 1000.0)
}

/// `tracer_soil_diffusive_transfer`：上层→下层的示踪物质量，限到两层比值相等为止。
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
    if water_upper <= 0.0 || water_lower <= 0.0 {
        return 0.0;
    }
    if d_eff_upper <= 0.0 || d_eff_lower <= 0.0 {
        return 0.0;
    }
    if dz_upper <= 0.0 || dz_lower <= 0.0 || deltim <= 0.0 {
        return 0.0;
    }
    let gradient = ratio_upper - ratio_lower;
    if gradient == 0.0 {
        return 0.0;
    }
    let dz_sum = dz_upper + dz_lower;
    let d_harmonic = dz_sum / (dz_upper / d_eff_upper + dz_lower / d_eff_lower);
    let dz_mid = dz_sum * 0.5;
    let transfer = deltim * (((gradient * d_harmonic) / dz_mid) * 1000.0);
    let equalise = gradient / (1.0 / water_upper + 1.0 / water_lower);
    if transfer.abs() > equalise.abs() {
        equalise
    } else {
        transfer
    }
}

/// `tracer_equilibration_exchange`：GIMPLE `(W·f)·FMS(α, R_v, R_old)`。
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
    let exchange = (pool_water * f) * alpha_eq.mul_add(vapor_ratio, -ratio_old);
    if exchange < 0.0 {
        -(-exchange).min(pool_trc.max(0.0))
    } else {
        exchange
    }
}

/// `tracer_craig_gordon_ratio_core`：`(R_s/α_eq - h·R_a)/(α_k(1-h))`，限幅 ±10·|R_s/α_eq|。
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
    let h = relhum.max(0.0).min(relhum_max.min(0.999_999).max(0.0));
    let one_minus_h = 1.0 - h;
    if one_minus_h <= 0.0 {
        return equilibrium_ratio;
    }
    let ratio = (-vapor_ratio).mul_add(h, equilibrium_ratio) / (alpha_k * one_minus_h);
    let bound = equilibrium_ratio.abs() * CRAIG_GORDON_MAX_RATIO_AMPLIFICATION;
    bound.min(ratio.max(-bound))
}

/// `tracer_surface_relhum`。
pub fn surface_relhum(qair: f64, psrf: f64, temp_k: f64, over_ice: bool) -> f64 {
    let esat = saturation_vapor_pressure(temp_k, over_ice);
    if esat <= TRC_TINY {
        return 0.0;
    }
    let qsafe = qair.max(0.0);
    let psafe = psrf.max(1.0);
    let eair = (qsafe * psafe) / qsafe.mul_add(0.378, 0.622);
    (eair / esat).max(0.0).min(0.999_999)
}

/// `tracer_saturation_vapor_pressure`（Magnus 式）。
pub fn saturation_vapor_pressure(temp_k: f64, over_ice: bool) -> f64 {
    let tc = temp_k.max(150.0) - TFRZ;
    if over_ice {
        ((tc * 22.46) / (tc + 272.62)).exp() * 611.2
    } else {
        ((tc * 17.67) / (tc + 243.5)).exp() * 611.2
    }
}

#[cfg(test)]
#[path = "frac_tests.rs"]
mod frac_tests;
