//! 同位素分馏物理（`MOD_Material_Frac`、`MOD_Material_Isotope_Registry`、`MOD_Material_Isotope_O18`、
//! `MOD_Material_Isotope_HDO`）。
//!
//! 上游的注册表是运行期的函数指针表，按名字模式（再退到 `ref_ratio` 相近）把材料映射到
//! 已实现的物种；这里物种是封闭的枚举 [`IsotopeSpecies`]，匹配规则相同。没匹配上的同位素
//! 照样是同位素，只是"分馏不生效"（α = 1，扩散比 = 1）。
//!
//! 运算次序与融合逐条对照 gfortran -O2 的 GIMPLE（`MOD_Material_Frac.F90.273t.optimized`）：
//! 常数指数的 `**(7/3)`、`**(2/3)` 走 libm `pow`（[`LibmPow`]），`**2` 是自乘。

// 夹紧保留上游 `MIN(MAX(·))` 的次序（`clamp` 对 NaN 的行为不同）。
#![allow(clippy::manual_clamp)]

use crate::LibmPow;
use colm_numeric::Contract;

use super::{MaterialDescriptor, MaterialPhysics, TRC_TINY};

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
/// `MOD_Material_Defs`：`Rsmow_18O`、`Rsmow_D`。
pub const RSMOW_18O: f64 = 2.0052e-3;
pub const RSMOW_D: f64 = 1.5576e-4;

/// `DEF_ISO_KINETIC_SCHEME`。
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

/// `DEF_ISO_OPEN_WATER_KINETIC`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenWaterKinetic {
    #[default]
    Mj79,
    Exponent,
}

impl OpenWaterKinetic {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "MJ79" | "mj79" => Some(Self::Mj79),
            "EXPONENT" | "exponent" => Some(Self::Exponent),
            _ => None,
        }
    }
}

/// 已实现分馏物理的同位素（`material_isotope_species.inc` 的顺序：O18、HDO）。
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
    pub fn find(material: &MaterialDescriptor) -> Option<Self> {
        if !material.is_isotope() {
            return None;
        }
        let lname = material.name.trim().to_ascii_lowercase();
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
        if material.ref_ratio > 0.0 {
            return Self::ALL.into_iter().find(|species| {
                let hint = species.ref_ratio_hint();
                hint > TRC_TINY && (material.ref_ratio - hint).abs() / hint < 0.1
            });
        }
        None
    }
}

impl MaterialPhysics {
    fn species(material: &MaterialDescriptor) -> Option<IsotopeSpecies> {
        IsotopeSpecies::find(material)
    }

    /// `material_alpha_liq_vap`（未注册时 1）。
    pub fn alpha_liq_vap(&self, material: &MaterialDescriptor, temp_k: f64) -> f64 {
        Self::species(material).map_or(1.0, |species| species.alpha_liq_vap(temp_k))
    }

    /// `material_alpha_ice_vap`。
    pub fn alpha_ice_vap(&self, material: &MaterialDescriptor, temp_k: f64) -> f64 {
        Self::species(material).map_or(1.0, |species| species.alpha_ice_vap(temp_k))
    }

    /// `material_alpha_ice_liq = alpha_ice_vap / max(alpha_liq_vap, trc_tiny)`。
    pub fn alpha_ice_liq(&self, material: &MaterialDescriptor, temp_k: f64) -> f64 {
        let liq = self.alpha_liq_vap(material, temp_k).max(TRC_TINY);
        self.alpha_ice_vap(material, temp_k) / liq
    }

    /// `material_diffusivity_ratio_air`。
    pub fn diffusivity_ratio_air(&self, material: &MaterialDescriptor) -> f64 {
        Self::species(material).map_or(1.0, |species| {
            species.diffusivity_ratio_air(self.kinetic_scheme)
        })
    }

    /// `material_leaf_liquid_diffusivity`/`material_liquid_self_diffusivity`（未注册时 0）。
    pub fn leaf_liquid_diffusivity(&self, material: &MaterialDescriptor, temp_k: f64) -> f64 {
        Self::species(material).map_or(0.0, |species| species.leaf_liquid_diffusivity(temp_k))
    }

    /// `material_rayleigh_freezing_loss`。
    pub fn rayleigh_freezing_loss(
        &self,
        material: &MaterialDescriptor,
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
        if !self.fractionation_active(material) {
            return proportional;
        }
        let alpha_il = self.alpha_ice_liq(material, temp_k);
        if alpha_il <= TRC_TINY || alpha_il.is_nan() {
            return proportional;
        }
        let remaining_water = pool_water - freeze_water;
        let liquid_fraction = (remaining_water / pool_water).max(TRC_TINY);
        let remaining_ratio = liquid_fraction.lpow(alpha_il - 1.0) * source_ratio;
        // `:153` `.FNMA (remaining_water, remaining_ratio, pool_trc)`
        (-remaining_water)
            .contract(remaining_ratio, pool_trc)
            .max(0.0)
            .min(pool_trc.max(0.0))
    }

    /// `material_alpha_ice_vap_deposition`。
    pub fn alpha_ice_vap_deposition(&self, material: &MaterialDescriptor, temp_k: f64) -> f64 {
        ice_deposition_alpha(
            temp_k,
            self.ice_supersat_slope,
            self.diffusivity_ratio_air(material),
            self.alpha_ice_vap(material, temp_k),
            self.alpha_ice_vap(material, TFRZ),
            self.alpha_ice_vap(material, JM84_FULL_KINETIC_TEMP),
        )
    }

    /// `material_alpha_kinetic_craig_gordon`：冰 `D^1`，液 `D^(2/3)`。
    pub fn alpha_kinetic_craig_gordon(&self, material: &MaterialDescriptor, from_ice: bool) -> f64 {
        let ratio = self.diffusivity_ratio_air(material);
        if from_ice {
            ratio
        } else {
            ratio.lpow(CG_EXPONENT_LIQUID)
        }
    }

    /// `material_alpha_kinetic_open_water`。
    pub fn alpha_kinetic_open_water(&self, material: &MaterialDescriptor, wind: f64) -> f64 {
        match self.open_water_kinetic {
            OpenWaterKinetic::Exponent => self.alpha_kinetic_craig_gordon(material, false),
            OpenWaterKinetic::Mj79 => mj79_kinetic_alpha(
                wind,
                Self::species(material).map_or(1.0, IsotopeSpecies::mj79_relative_factor),
            ),
        }
    }

    /// `material_alpha_kinetic_leaf`：`(ra + rb·D^(2/3) + rc·D)/(ra+rb+rc)`，
    /// GIMPLE `FMA(rc, D, FMA(D^(2/3), rb, ra))`。
    pub fn alpha_kinetic_leaf(
        &self,
        material: &MaterialDescriptor,
        ra: f64,
        rb: f64,
        rc: f64,
    ) -> f64 {
        let (ra1, rb1, rc1) = (ra.max(0.0), rb.max(0.0), rc.max(0.0));
        let denom = (ra1 + rb1) + rc1;
        if denom <= TRC_TINY {
            return 1.0;
        }
        let diff_ratio = self.diffusivity_ratio_air(material);
        let turbulent = diff_ratio.lpow(CG_EXPONENT_LIQUID).contract(rb1, ra1);
        rc1.contract(diff_ratio, turbulent) / denom
    }

    /// `material_alpha_kinetic_soil`。
    pub fn alpha_kinetic_soil(&self, material: &MaterialDescriptor, ra: f64, rs: f64) -> f64 {
        soil_kinetic_alpha_core(ra, rs, self.diffusivity_ratio_air(material))
    }

    /// `material_craig_gordon_evap_ratio`。
    #[allow(clippy::too_many_arguments)]
    pub fn craig_gordon_evap_ratio(
        &self,
        material: &MaterialDescriptor,
        source_ratio: f64,
        vapor_ratio: f64,
        temp_k: f64,
        relhum: f64,
        alpha_k: f64,
        from_ice: bool,
    ) -> f64 {
        if !self.fractionation_active(material) {
            return source_ratio;
        }
        // 上游 `material_craig_gordon_evap_ratio`：冰→汽（升华）不分馏，蒸汽按冰自身
        // 的比值离开。冰内自扩散太慢、形不成分馏边界层，这是雪/冰升华的常用假设
        // （Friedman et al. 1991；Moser & Stichler 1975）。对冰套 Craig–Gordon、且
        // 雪面湿度接近冰面饱和（h 达到 DEF_ISO_CG_RELHUM_MAX=0.99）时，上游实测
        // 2000–2004 IsoGSM 算例的雪面蒸汽 δ18O 到 -122‰、δD 到 -830‰，远超观测。
        // 因此这里也必须先返回源端，不能走 α_ice。
        if from_ice {
            return source_ratio;
        }
        let alpha_eq = self.alpha_liq_vap(material, temp_k);
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

    /// `material_craig_gordon_evap_affine`：Craig–Gordon 蒸发比值写成
    /// `R_E = slope*R_s + offset` 的系数。上游用它在有限水池上做**精确积分**
    /// （`material_evaporative_mt_loss`，Gonfiantini 1986 的干涸池解），
    /// 而不是按 10% 池量显式子步——后者在近饱和（slope ~ 1/(1-h)）时会失稳，
    /// 上游实测雪融池 δ18O 被推到 -969‰。分馏未启用、冰→汽或 α 非法时
    /// 返回 `(1, 0)`，与上游一致。
    pub fn craig_gordon_evap_affine(
        &self,
        material: &MaterialDescriptor,
        vapor_ratio: f64,
        temp_k: f64,
        relhum: f64,
        alpha_k: f64,
        from_ice: bool,
    ) -> (f64, f64) {
        if !self.fractionation_active(material) || from_ice {
            return (1.0, 0.0);
        }
        let alpha_eq = self.alpha_liq_vap(material, temp_k);
        if alpha_eq <= TRC_TINY || alpha_eq.is_nan() {
            return (1.0, 0.0);
        }
        let alpha_k = alpha_k.max(TRC_TINY);
        let h = relhum
            .max(0.0)
            .min(self.cg_relhum_max.min(0.999_999).max(0.0));
        let one_minus_h = 1.0 - h;
        if one_minus_h <= 0.0 {
            return (1.0 / alpha_eq, 0.0);
        }
        let slope = 1.0 / (alpha_eq * alpha_k * one_minus_h);
        let offset = (-h * vapor_ratio) / (alpha_k * one_minus_h);
        if slope.is_nan() || offset.is_nan() {
            (1.0, 0.0)
        } else {
            (slope, offset)
        }
    }

    /// `material_equilibrium_deposition_ratio`。
    pub fn equilibrium_deposition_ratio(
        &self,
        material: &MaterialDescriptor,
        vapor_ratio: f64,
        temp_k: f64,
        from_ice: bool,
    ) -> f64 {
        if !self.fractionation_active(material) {
            return vapor_ratio;
        }
        if from_ice {
            vapor_ratio * self.alpha_ice_vap_deposition(material, temp_k)
        } else {
            vapor_ratio * self.alpha_liq_vap(material, temp_k)
        }
    }

    /// `material_ratio_to_delta`：`(R/R_ref - 1)*1000`（`R_ref <= tiny` 时 0）。
    pub fn ratio_to_delta(material: &MaterialDescriptor, ratio: f64) -> f64 {
        if material.ref_ratio > TRC_TINY {
            (ratio / material.ref_ratio - 1.0) * 1000.0
        } else {
            0.0
        }
    }

    /// `material_delta_to_ratio`：`(δ/1000 + 1)*R_ref`。
    pub fn delta_to_ratio(material: &MaterialDescriptor, delta: f64) -> f64 {
        if material.ref_ratio > TRC_TINY {
            (delta / 1000.0 + 1.0) * material.ref_ratio
        } else {
            0.0
        }
    }

    /// `material_transpiration_nss_ratio`（Farquhar–Cernusak 非稳态叶水）。
    pub fn transpiration_nss_ratio(
        &self,
        material: &MaterialDescriptor,
        input: &NssInput,
    ) -> NssOutput {
        let source_ratio = input.source_ratio;
        let initial_delta = Self::ratio_to_delta(material, source_ratio);
        let mut out = NssOutput {
            trans_ratio: source_ratio,
            new_delta_e: initial_delta,
            new_delta_b: initial_delta,
            new_peclet: 1.0,
            new_leaf_moles: 0.0,
        };
        if !self.fractionation_active(material) {
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
        let alpha_eq = self.alpha_liq_vap(material, tk);
        let eps_eq = (alpha_eq - 1.0) * 1000.0;
        let leaf_area_safe = input.leaf_area.max(TRC_TINY);
        let alpha_k = self.alpha_kinetic_leaf(
            material,
            input.aerodynamic_resistance,
            self.nss_leaf_rb / leaf_area_safe,
            input.stomatal_resistance,
        );
        let eps_k = (alpha_k - 1.0) * 1000.0;
        let delta_x = Self::ratio_to_delta(material, source_ratio);
        let delta_v = Self::ratio_to_delta(material, input.vapor_ratio);
        // GIMPLE `FMA((δv - ε_k) - δx, h, (ε_k + δx) + ε_eq)`。
        let delta_es = ((delta_v - eps_k) - delta_x).contract(h, (eps_k + delta_x) + eps_eq);

        let transp_moles = transp_water * WATER_MOLES_PER_MM;
        let transp_moles_leaf_s = transp_moles / (deltim * leaf_area_safe);
        let liquid_diff = self.leaf_liquid_diffusivity(material, tk);
        let mut peclet = out.new_peclet;
        if liquid_diff > TRC_TINY && self.nss_leaf_path_length > 0.0 {
            let peclet_number = (self.nss_leaf_path_length * transp_moles_leaf_s)
                / (liquid_diff * LIQUID_WATER_MOLAR_DENSITY);
            peclet = if peclet_number > 1.0e-8 {
                (1.0 - (-peclet_number).exp()) / peclet_number
            } else {
                (-peclet_number).contract(0.5, 1.0)
            };
        }
        let peclet = peclet.max(0.0).min(1.0);
        out.new_peclet = peclet;

        // 上游（`material_transpiration_nss_ratio`）把历史项定义为
        // `prev_w * (prev_delta_b - delta_x)`：存的是**上一时刻的叶水整体** δb，
        // 不是蒸发端元 δe，也不再乘 Péclet。W = LAI * nss_leaf_water_per_lai 是诊断
        // 尺寸而非宿主水量，LAI 变化时叶水按当前 W 取上一状态（否则 LAI 阶跃会被当成
        // 同位素储量变化，上游在 NEC 例子上实测到 +530~+1700‰ 的错误）。
        let (prev_w, prev_bulk_delta) = if input.prev_leaf_moles > TRC_TINY {
            (leaf_moles, input.prev_delta_b)
        } else {
            (leaf_moles, delta_x)
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
        let denom = (leaf_moles * relax_b).contract(peclet, 1.0);
        out.new_delta_e = if denom > TRC_TINY {
            // 上游是 `prev_w * (prev_bulk_delta - delta_x)`；`contract` 的第三参是
            // **加数**而不是乘数，写成 `prev_w.contract(prev_bulk_delta, -delta_x)` 会
            // 变成 `prev_w*prev_bulk_delta - delta_x`（少了 prev_w），白天叶水 δ 被推飞。
            let memory = prev_w * (prev_bulk_delta - delta_x);
            let numerator = (leaf_moles * peclet).contract(delta_x, memory);
            numerator.contract(relax_b, delta_es) / denom
        } else {
            delta_es
        };
        out.new_delta_b = (out.new_delta_e - delta_x).contract(peclet, delta_x);

        let (prev_leaf_water, prev_bulk_ratio) = if input.prev_leaf_moles > TRC_TINY {
            (
                prev_w / WATER_MOLES_PER_MM,
                Self::delta_to_ratio(material, input.prev_delta_b),
            )
        } else {
            (leaf_moles / WATER_MOLES_PER_MM, source_ratio)
        };
        let new_leaf_water = leaf_moles / WATER_MOLES_PER_MM;
        let new_bulk_ratio = Self::delta_to_ratio(material, out.new_delta_b);
        let previous_storage = prev_leaf_water * prev_bulk_ratio;
        let storage_material_change = new_bulk_ratio.contract(new_leaf_water, -previous_storage);
        let storage_scale = (source_ratio * transp_water).max(TRC_TINY);
        let storage_bound = storage_scale * 0.95;
        let storage_material_change_used = if storage_material_change.abs() > storage_scale * 10.0 {
            0.0
        } else {
            storage_material_change
                .max(-storage_bound)
                .min(storage_bound)
        };
        let mut used = storage_material_change_used;
        if (used - storage_material_change).abs() > TRC_TINY {
            let target_leaf_storage = used + previous_storage;
            if new_leaf_water > TRC_TINY && target_leaf_storage > TRC_TINY {
                out.new_delta_b =
                    Self::ratio_to_delta(material, target_leaf_storage / new_leaf_water);
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

/// `material_transpiration_nss_ratio` 的输入（`prev_*` 是上一步的叶水状态）。
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

/// `material_transpiration_nss_ratio` 的输出。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NssOutput {
    pub trans_ratio: f64,
    pub new_delta_e: f64,
    pub new_delta_b: f64,
    pub new_peclet: f64,
    pub new_leaf_moles: f64,
}

/// `material_jm84_effective_alpha`。
pub fn jm84_effective_alpha(alpha_eq: f64, diff_ratio: f64, tc: f64, slope: f64) -> f64 {
    if slope <= 0.0 || tc >= 0.0 {
        return alpha_eq;
    }
    let s = (-slope).contract(tc, 1.0);
    let denom = (alpha_eq * diff_ratio).contract(s - 1.0, 1.0);
    if denom <= 0.0 {
        return alpha_eq;
    }
    (alpha_eq * s) / denom
}

/// `material_ice_deposition_alpha`。中段 GIMPLE：`FMA(α_tfrz, T-253.15, (tfrz-T)·α_cold) / span`，
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
    alpha_eq_at_tfrz.contract(
        temp_k - JM84_FULL_KINETIC_TEMP,
        (TFRZ - temp_k) * alpha_cold,
    ) / span
}

/// `material_mj79_kinetic_alpha`。
pub fn mj79_kinetic_alpha(wind: f64, relative_factor: f64) -> f64 {
    let u = wind.max(0.0);
    let k = if u >= MJ79_WIND_THRESHOLD {
        u.contract(MJ79_ROUGH_SLOPE, MJ79_ROUGH_OFFSET)
    } else {
        MJ79_SMOOTH_K
    };
    let k = (k * relative_factor.max(0.0)).max(0.0).min(0.5);
    1.0 / (1.0 - k)
}

/// `material_soil_kinetic_alpha_core`：GIMPLE `FMA(D^(2/3), ra, D·rs) / (ra+rs)`。
pub fn soil_kinetic_alpha_core(ra: f64, rs: f64, diff_ratio: f64) -> f64 {
    let (ra1, rs1) = (ra.max(0.0), rs.max(0.0));
    let denom = ra1 + rs1;
    if denom <= 0.0 || diff_ratio <= 0.0 {
        return 1.0;
    }
    diff_ratio
        .lpow(CG_EXPONENT_LIQUID)
        .contract(ra1, diff_ratio * rs1)
        / denom
}

/// `material_soil_effective_diffusivity`：`D·θ^(7/3)/φ²`。
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

/// `material_soil_vapor_equivalent_diffusivity`。
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

/// `material_snow_vapor_equivalent_diffusivity`。
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

/// `material_soil_diffusive_transfer`：上层→下层的材料质量，限到两层比值相等为止。
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

/// `material_equilibration_exchange`：GIMPLE `(W·f)·FMS(α, R_v, R_old)`。
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
    let exchange = (pool_water * f) * alpha_eq.contract(vapor_ratio, -ratio_old);
    if exchange < 0.0 {
        -(-exchange).min(pool_trc.max(0.0))
    } else {
        exchange
    }
}

/// `material_craig_gordon_ratio_core`：`(R_s/α_eq - h·R_a)/(α_k(1-h))`，限幅 ±10·|R_s/α_eq|。
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
    let ratio = (-vapor_ratio).contract(h, equilibrium_ratio) / (alpha_k * one_minus_h);
    let bound = equilibrium_ratio.abs() * CRAIG_GORDON_MAX_RATIO_AMPLIFICATION;
    bound.min(ratio.max(-bound))
}

/// `material_surface_relhum`。
pub fn surface_relhum(qair: f64, psrf: f64, temp_k: f64, over_ice: bool) -> f64 {
    let esat = saturation_vapor_pressure(temp_k, over_ice);
    if esat <= TRC_TINY {
        return 0.0;
    }
    let qsafe = qair.max(0.0);
    let psafe = psrf.max(1.0);
    let eair = (qsafe * psafe) / qsafe.contract(0.378, 0.622);
    (eair / esat).max(0.0).min(0.999_999)
}

/// `material_saturation_vapor_pressure`（Magnus 式）。
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
