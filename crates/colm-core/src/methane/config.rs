//! `MOD_Tracer_Reactive_Methane_Const`：常数、`&nl_colm_methane_parameter` 的读入与校验、
//! 淹没方案（`configure_methane_inundation_mode`）。字段表见 [`super::config_generated`]。

use anyhow::{bail, ensure, Result};

pub use super::config_generated::{MethaneConfig, MethaneHydrology};

/// `ngases`：CH4、O2、CO2。
pub const NGASES: usize = 3;
/// `METHANE_COMP_SOIL`、`METHANE_COMP_RICE`（0 起）。
pub const COMP_SOIL: usize = 0;
pub const COMP_RICE: usize = 1;
pub const N_COMP: usize = 2;
/// `catomw`：碳原子摩尔质量（g/mol）。
pub const CATOMW: f64 = 12.011;
pub const GC_PER_KG_OM: f64 = 580.0;
pub const METHANE_ATOMW: f64 = 16.04;
/// `s_con(ngases,4)`：Schmidt 数系数。
pub const S_CON: [[f64; 4]; NGASES] = [
    [1898.0, -110.1, 2.834, -0.02791],
    [1801.0, -120.1, 3.7818, -0.047608],
    [1911.0, -113.7, 2.967, -0.02943],
];
/// `d_con_w(ngases,3)`：水中扩散系数（×1e-9 m2/s）。
pub const D_CON_W: [[f64; 3]; NGASES] = [
    [0.9798, 0.02986, 0.0004381],
    [1.172, 0.03443, 0.0005048],
    [0.939, 0.02671, 0.0004095],
];
/// `d_con_g(ngases,2)`：气相扩散系数（×1e-4 m2/s）。
pub const D_CON_G: [[f64; 2]; NGASES] = [[0.1875, 0.0013], [0.1759, 0.00117], [0.1325, 0.0009]];
/// `c_h`：Henry 定律温度常数（K）。
pub const C_H: [f64; NGASES] = [1600.0, 1500.0, 2400.0];
/// `kh_theta`：298 K 的 Henry 常数（mol/L/atm）。
pub const KH_THETA: [f64; NGASES] = [1.4e-3, 1.3e-3, 3.4e-2];
pub const KH_TBASE: f64 = 298.15;
pub const RGASM: f64 = 8.31446261815324;
pub const RGAS_LATM: f64 = 0.08206;
pub const SECSPDAY: f64 = 86400.0;

/// 一个 namelist 值（由调用方从文本解析好；实数字段也接受整数写法）。
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Real(f64),
    Int(i64),
    Logical(bool),
    Text(String),
}

impl FieldValue {
    pub(super) fn real(&self, path: &str) -> Result<f64> {
        match self {
            FieldValue::Real(x) => Ok(*x),
            FieldValue::Int(i) => Ok(*i as f64),
            other => bail!("{path} must be a real, got {other:?}"),
        }
    }

    pub(super) fn logical(&self, path: &str) -> Result<bool> {
        match self {
            FieldValue::Logical(b) => Ok(*b),
            other => bail!("{path} must be a logical, got {other:?}"),
        }
    }

    pub(super) fn int(&self, path: &str) -> Result<i32> {
        match self {
            FieldValue::Int(i) => Ok(i32::try_from(*i)?),
            other => bail!("{path} must be an integer, got {other:?}"),
        }
    }

    pub(super) fn text(&self, path: &str) -> Result<String> {
        match self {
            FieldValue::Text(s) => Ok(s.clone()),
            other => bail!("{path} must be a string, got {other:?}"),
        }
    }
}

/// 读出来的整套甲烷配置。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MethaneParameters {
    pub methane: MethaneConfig,
    pub hydrology: MethaneHydrology,
}

/// 淹没方案解析的结果（`configure_methane_inundation_mode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InundationMode {
    /// `DEF_wetland_finundation_scheme`。
    pub scheme: i32,
}

impl MethaneParameters {
    /// `read_methane_namelist`：先全部取默认，再按出现顺序套用 `&nl_colm_methane_parameter`
    /// 的赋值（`owner` 为 `DEF_METHANE`/`DEF_METHANE_hydrology`，`field` 为成员名），最后校验。
    pub fn from_entries<'a>(
        entries: impl IntoIterator<Item = (&'a str, &'a str, FieldValue)>,
    ) -> Result<Self> {
        let mut out = Self::default();
        for (owner, field, value) in entries {
            let path = format!("{owner}%{field}");
            let field = field.to_ascii_lowercase();
            let known = match owner.to_ascii_lowercase().as_str() {
                "def_methane" => out.methane.set(&field, &value, &path)?,
                "def_methane_hydrology" => out.hydrology.set(&field, &value, &path)?,
                _ => false,
            };
            ensure!(known, "invalid &nl_colm_methane_parameter: unknown {path}");
        }
        out.validate()?;
        Ok(out)
    }

    /// `configure_methane_inundation_mode`：把用户的五选一解析成内部方案号与配套开关。
    /// `grid_river` 是内核是否编进 `GridRiverLakeFlow`（单点为假）。
    pub fn configure_inundation(
        &mut self,
        dynamic_wetland: bool,
        grid_river: bool,
    ) -> Result<InundationMode> {
        let m = &mut self.methane;
        let mode = m.inundation_mode.trim().to_ascii_lowercase();
        m.enable_wetwat_finundated_override = false;
        m.wetland_dry_unsat_branch = true;
        m.use_routing_for_soil = false;
        let scheme = match mode.as_str() {
            "wetwat" => {
                m.enable_wetwat_finundated_override = true;
                ensure!(
                    !dynamic_wetland,
                    "wetwat methane inundation mode requires DEF_USE_Dynamic_Wetland = .false."
                );
                1
            }
            "satellite" | "giems" => {
                ensure!(
                    !dynamic_wetland,
                    "satellite methane inundation mode requires DEF_USE_Dynamic_Wetland = .false."
                );
                5
            }
            "routing" => {
                ensure!(
                    grid_river,
                    "routing methane inundation mode requires a GridRiverLakeFlow-enabled kernel."
                );
                ensure!(
                    !dynamic_wetland,
                    "routing methane inundation mode requires DEF_USE_Dynamic_Wetland = .false."
                );
                7
            }
            "dynamic_wtd" | "dynamic-wtd" => {
                ensure!(
                    dynamic_wetland,
                    "dynamic_wtd requires DEF_USE_Dynamic_Wetland = .true."
                );
                6
            }
            "hybrid" | "dh_all_thr05" | "dyn_routing_hybrid" => {
                ensure!(
                    grid_river,
                    "hybrid methane inundation mode requires a GridRiverLakeFlow-enabled kernel."
                );
                m.use_routing_for_soil = true;
                ensure!(
                    dynamic_wetland,
                    "hybrid mode requires DEF_USE_Dynamic_Wetland = .true."
                );
                6
            }
            _ => bail!(
                "unsupported DEF_METHANE%inundation_mode = {}; expected wetwat, satellite, \
                 routing, dynamic_wtd, or hybrid.",
                m.inundation_mode.trim()
            ),
        };
        Ok(InundationMode { scheme })
    }

    /// `methane_history_accumulation_mode`：0 不写、1 只写核心量、2 其余。
    pub fn history_accumulation_mode(&self) -> i32 {
        if !self.methane.write_ch4_history {
            return 0;
        }
        match self
            .methane
            .ch4_history_vars
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "none" | "off" | "false" | ".false." => 0,
            "core" | "default" | "minimal" | "fast" => 1,
            _ => 2,
        }
    }

    /// `validate_methane_namelist`：任何一项不合法就停机（上游先逐项打印再一次停机）。
    pub fn validate(&self) -> Result<()> {
        let m = &self.methane;
        let h = &self.hydrology;
        let mut bad: Vec<String> = Vec::new();
        let mut check = |ok: bool, message: String| {
            if !ok {
                bad.push(message);
            }
        };
        let reals = [
            m.q10methane,
            m.f_methane,
            m.f_methane_tropical_peat,
            m.f_methane_tropical_floodplain,
            m.f_methane_floodplain,
            m.f_methane_temperate_marsh,
            m.f_methane_boreal_fen,
            m.f_methane_boreal_bog,
            m.f_methane_rice_paddy,
            m.f_methane_upland_soil,
            m.redoxlag_tropical_peat,
            m.redoxlag_tropical_floodplain,
            m.redoxlag_temperate_marsh,
            m.redoxlag_boreal_fen,
            m.redoxlag_boreal_bog,
            m.redoxlag_rice_paddy,
            m.redoxlag_upland_soil,
            m.z0_methane_prod,
            m.vmax_methane_oxid,
            m.vmax_oxid_unsat,
            m.k_m,
            m.k_m_unsat,
            m.k_m_o2,
            m.q10_methane_oxid,
            m.lake_oxid_scale,
            m.lake_k_m_o2,
            m.lake_vmax_methane_oxid,
            m.lake_oxic_sediment_depth,
            m.b_init_methanogen,
            m.b_init_methanotroph,
            m.b_min_methanogen,
            m.b_min_methanotroph,
            m.b_max_fraction_methanogen,
            m.b_max_fraction_methanotroph,
            m.mu_max_methanogen,
            m.mu_max_methanotroph,
            m.gamma_methanogen,
            m.gamma_methanotroph,
            m.gamma_microbial_dormant,
            m.gamma_microbial_freeze,
            m.k_substrate_methanogen_pool,
            m.k_inh_o2_methanogen,
            m.kappa_m_methanogen,
            m.kappa_m_methanotroph,
            m.max_microbe_prod_multiplier,
            m.q10_microbe_growth,
            m.t_ref_microbe,
            m.dormancy_rate_active,
            m.dormancy_rate_revive,
            m.dormancy_threshold_methanogen_fs,
            m.dormancy_threshold_methanogen_fo2,
            m.dormancy_threshold_methanotroph_fs,
            m.dormancy_threshold_methanotroph_fo2,
            m.vgc_max,
            m.nongrassporosratio,
            m.poros_tiller,
            m.unsat_aere_ratio,
            m.porosmin,
            m.aere_radius,
            m.rob,
            m.scale_factor_aere,
            m.scale_factor_gasdiff,
            m.scale_factor_liqdiff,
            m.lake_liqdiff_scale,
            m.lake_o2_liqdiff_scale,
            m.grnd_methane_cond_default,
            m.mino2lim,
            m.q10methane_base,
            m.q10lakebase,
            m.cnscalefactor,
            m.redoxlag,
            m.lake_decomp_fact,
            m.redoxlag_vertical,
            m.phmax,
            m.phmin,
            m.oxinhib,
            m.smp_crit,
            m.bubble_f,
            m.aereoxid,
            m.tiller_c,
            m.satpow,
            m.capthick,
            m.atm_methane,
            m.om_frac_sf,
            m.wtd_inflection,
            m.wtd_steepness,
            m.wtd_inflection_soil,
            m.wtd_steepness_soil,
            m.hybrid_soil_threshold,
            m.rice_drain_window_days,
            m.rice_substrate_boost,
            m.numerical_correction_fatal_threshold,
            m.host_water_tolerance,
            h.vdcf,
            h.slopebeta,
            h.slopemax,
            h.pc,
        ];
        check(
            reals.iter().all(|x| x.is_finite()),
            "methane namelist contains a NaN or infinite real parameter.".into(),
        );
        check(
            m.methane_offline,
            "methane_offline=.false. requires an online atmosphere/NEE coupling that is not implemented."
                .into(),
        );
        check(
            !m.methane_frzout,
            "methane_frzout is retired; ice is always excluded from mobile CH4 storage.".into(),
        );
        for (name, value) in [
            ("f_methane", m.f_methane),
            ("f_methane_tropical_peat", m.f_methane_tropical_peat),
            (
                "f_methane_tropical_floodplain",
                m.f_methane_tropical_floodplain,
            ),
            ("f_methane_floodplain", m.f_methane_floodplain),
            ("f_methane_temperate_marsh", m.f_methane_temperate_marsh),
            ("f_methane_boreal_fen", m.f_methane_boreal_fen),
            ("f_methane_boreal_bog", m.f_methane_boreal_bog),
            ("f_methane_rice_paddy", m.f_methane_rice_paddy),
            ("f_methane_upland_soil", m.f_methane_upland_soil),
        ] {
            check(
                !(value < 0.0 || value > 0.5),
                format!("{name} out of [0,0.5]: {value}"),
            );
        }
        check(!(m.q10methane <= 0.0), "q10methane must be > 0".into());
        check(
            !(m.q10_methane_oxid <= 0.0),
            "q10_methane_oxid must be > 0".into(),
        );
        check(
            !(m.vmax_methane_oxid < 0.0),
            "vmax_methane_oxid must be >= 0".into(),
        );
        check(
            !(m.vmax_oxid_unsat < 0.0),
            "vmax_oxid_unsat must be >= 0".into(),
        );
        check(!(m.k_m <= 0.0), "k_m must be > 0".into());
        check(!(m.k_m_unsat <= 0.0), "k_m_unsat must be > 0".into());
        check(!(m.k_m_o2 <= 0.0), "k_m_o2 must be > 0".into());
        check(
            !(m.lake_oxid_scale < 0.0),
            "lake_oxid_scale must be >= 0".into(),
        );
        check(
            !(m.lake_k_m_o2 != -1.0 && m.lake_k_m_o2 <= 0.0),
            "lake_k_m_o2 must be -1 or > 0".into(),
        );
        check(
            !(m.lake_vmax_methane_oxid != -1.0 && m.lake_vmax_methane_oxid < 0.0),
            "lake_vmax_methane_oxid must be -1 or >= 0".into(),
        );
        check(
            !(m.lake_oxic_sediment_depth != -1.0 && m.lake_oxic_sediment_depth <= 0.0),
            "lake_oxic_sediment_depth must be -1 or > 0".into(),
        );
        check(
            !(m.aereoxid < 0.0 || m.aereoxid > 1.0),
            "aereoxid out of [0,1]".into(),
        );
        check(
            !(m.bubble_f <= 0.0 || m.bubble_f > 1.0),
            "bubble_f out of (0,1]".into(),
        );
        check(!(m.phmin >= m.phmax), "pHmin must be < pHmax".into());
        check(
            !(m.mino2lim < 0.0 || m.mino2lim > 1.0),
            "mino2lim out of [0,1]".into(),
        );
        check(!(m.atm_methane < 0.0), "atm_methane must be >= 0".into());
        check(
            !(m.host_water_tolerance < 0.0 || m.host_water_tolerance >= 1.0),
            format!(
                "host_water_tolerance out of [0,1): {}",
                m.host_water_tolerance
            ),
        );
        check(
            !(m.wtd_steepness <= 0.0),
            "wtd_steepness must be > 0".into(),
        );
        check(
            m.wtd_inflection_soil.is_finite() && !(m.wtd_inflection_soil < 0.0),
            "wtd_inflection_soil must be >= 0".into(),
        );
        check(
            m.wtd_steepness_soil.is_finite() && !(m.wtd_steepness_soil <= 0.0),
            "wtd_steepness_soil must be > 0".into(),
        );
        check(
            m.hybrid_soil_threshold.is_finite()
                && !(m.hybrid_soil_threshold < 0.0 || m.hybrid_soil_threshold > 1.0),
            "hybrid_soil_threshold out of [0,1]".into(),
        );
        check(
            m.z0_methane_prod.is_finite() && !(m.z0_methane_prod < 0.0),
            "z0_methane_prod must be >= 0".into(),
        );
        check(!(m.vgc_max <= 0.0), "vgc_max must be > 0".into());
        check(
            !(m.poros_tiller < 0.0 || m.poros_tiller > 1.0),
            "poros_tiller out of [0,1]".into(),
        );
        check(
            !(m.nongrassporosratio < 0.0),
            "nongrassporosratio must be >= 0".into(),
        );
        check(
            !(m.unsat_aere_ratio < 0.0),
            "unsat_aere_ratio must be >= 0".into(),
        );
        check(
            !(m.porosmin < 0.0 || m.porosmin > 1.0),
            "porosmin out of [0,1]".into(),
        );
        check(!(m.aere_radius <= 0.0), "aere_radius must be > 0".into());
        check(!(m.rob <= 0.0), "rob must be > 0".into());
        check(
            !(m.scale_factor_aere < 0.0
                || m.scale_factor_gasdiff < 0.0
                || m.scale_factor_liqdiff < 0.0
                || m.lake_liqdiff_scale < 0.0
                || m.lake_o2_liqdiff_scale < 0.0),
            "methane scale factors must be >= 0".into(),
        );
        check(
            !(m.grnd_methane_cond_default <= 0.0),
            "grnd_methane_cond_default must be > 0".into(),
        );
        check(
            !(m.q10methane_base <= 0.0 || m.q10lakebase <= 0.0),
            "q10methane_base and q10lakebase must be > 0".into(),
        );
        check(
            !(m.cnscalefactor < 0.0),
            "cnscalefactor must be >= 0".into(),
        );
        check(
            !(m.redoxlag < 0.0 || m.redoxlag_vertical < 0.0),
            "redoxlag and redoxlag_vertical must be >= 0".into(),
        );
        let min_redox = [
            m.redoxlag_tropical_peat,
            m.redoxlag_tropical_floodplain,
            m.redoxlag_temperate_marsh,
            m.redoxlag_boreal_fen,
            m.redoxlag_boreal_bog,
            m.redoxlag_rice_paddy,
            m.redoxlag_upland_soil,
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min);
        check(
            !(min_redox < 0.0),
            "biome redoxlag values must be >= 0".into(),
        );
        check(!(m.oxinhib < 0.0), "oxinhib must be >= 0".into());
        check(
            !(m.b_max_fraction_methanogen > 1.0 || m.b_max_fraction_methanotroph > 1.0),
            "B_max fractions must be <= 1".into(),
        );
        check(
            !(m.lake_decomp_fact < 0.0),
            "lake_decomp_fact must be >= 0".into(),
        );
        check(!(m.smp_crit >= 0.0), "smp_crit must be < 0".into());
        check(!(m.satpow <= 0.0), "satpow must be > 0".into());
        check(!(m.capthick < 0.0), "capthick must be >= 0".into());
        check(
            !(m.rice_drain_window_days <= 0.0),
            "rice_drain_window_days must be > 0".into(),
        );
        check(
            !((m.rice_substrate_boost - 1.0).abs() > 10.0 * f64::EPSILON),
            "rice_substrate_boost must remain 1 until methane production debits BGC carbon".into(),
        );
        check(
            !m.ch4_history_vars.trim_end().is_empty(),
            "ch4_history_vars must be core/diagnostic/all/none or a comma-separated list".into(),
        );
        check(!(m.tiller_c <= 0.0), "tiller_C must be > 0".into());
        check(!(m.om_frac_sf < 0.0), "om_frac_sf must be >= 0".into());
        check(
            !(m.k_substrate_methanogen_pool <= 0.0 || m.k_inh_o2_methanogen <= 0.0),
            "K_substrate_methanogen_pool and K_inh_O2_methanogen must be > 0".into(),
        );
        check(
            ![
                m.b_init_methanogen,
                m.b_init_methanotroph,
                m.b_min_methanogen,
                m.b_min_methanotroph,
                m.mu_max_methanogen,
                m.mu_max_methanotroph,
                m.gamma_methanogen,
                m.gamma_methanotroph,
                m.gamma_microbial_dormant,
                m.gamma_microbial_freeze,
                m.kappa_m_methanogen,
                m.kappa_m_methanotroph,
            ]
            .iter()
            .any(|&x| x < 0.0),
            "microbial parameters must be >= 0".into(),
        );
        check(
            !(m.use_microbial_flux_override && !m.use_microbial_pools),
            "use_microbial_flux_override requires use_microbial_pools.".into(),
        );
        check(
            !(m.use_microbial_dormancy && !m.use_microbial_pools),
            "use_microbial_dormancy requires use_microbial_pools.".into(),
        );
        check(
            !m.use_microbial_pools,
            "microbial pools are disabled until biomass growth/loss has donor/sink carbon coupling."
                .into(),
        );
        check(
            !(m.max_microbe_prod_multiplier <= 0.0),
            "max_microbe_prod_multiplier must be > 0".into(),
        );
        check(
            !(m.q10_microbe_growth <= 0.0 || m.t_ref_microbe <= 0.0),
            "q10_microbe_growth and T_ref_microbe must be > 0".into(),
        );
        check(
            ![
                m.dormancy_rate_active,
                m.dormancy_rate_revive,
                m.dormancy_threshold_methanogen_fs,
                m.dormancy_threshold_methanogen_fo2,
                m.dormancy_threshold_methanotroph_fs,
                m.dormancy_threshold_methanotroph_fo2,
            ]
            .iter()
            .any(|&x| x < 0.0),
            "dormancy parameters must be >= 0".into(),
        );
        check(
            !(h.slopemax <= 0.0 || h.slopebeta >= 0.0),
            "methane hydrology requires slopemax > 0 and slopebeta < 0".into(),
        );
        check(
            !((h.vdcf - 2.0).abs() > 64.0 * f64::EPSILON
                || (h.pc - 0.4).abs() > 64.0 * f64::EPSILON),
            "retired methane hydrology knobs vdcf/pc must retain defaults 2.0/0.4".into(),
        );
        let file = m.atm_methane_file.trim().to_ascii_lowercase();
        check(
            !(m.use_transient_atm_methane && (file.is_empty() || file == "null")),
            "transient atmospheric CH4 requires atm_methane_file.".into(),
        );
        ensure!(
            bad.is_empty(),
            "methane namelist validation failed:\n  {}",
            bad.join("\n  ")
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
