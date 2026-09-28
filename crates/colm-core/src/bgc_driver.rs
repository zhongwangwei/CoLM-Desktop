//! `bgc_driver`（`MOD_BGC_driver.F90`）：一个土壤 patch 一步的 C/N 循环。
//!
//! 过程按上游 driver 的调用顺序拆成阶段（[`run_stage`]），阶段名就是 Fortran 插桩追踪里的
//! 记录名（`oracle/scripts/gen_bgc_trace.py`）。这样既能整步运行（[`bgc_driver`]），也能从
//! Fortran 追踪的任意一条记录出发只跑一个阶段，逐过程逐位验证。
//!
//! 输入分三块：[`BgcState`]（`MOD_BGC_Vars_*`，BGC 自己的状态）、[`BgcPhysics`]（BGC 读写的
//! 物理量，字段名沿用 Fortran 名以便与追踪对齐）、[`BgcPftConstants`]（`MOD_Const_PFT` 的按
//! 类别参数）。

use anyhow::{bail, ensure, Context, Result};

use crate::bgc_state::BgcState;
use crate::bgc_trace::TraceRecord;
use crate::bgc_zero_fluxes_generated::{cn_zero_fluxes, ZeroFluxSwitches};
use crate::calendar::is_leap_year;

/// driver 用到的 `DEF_USE_*` 开关与 `#ifdef CROP`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BgcSwitches {
    pub crop: bool,
    pub nitrif: bool,
    pub fire: bool,
    pub sasu: bool,
    pub diag_matrix: bool,
    pub cnsoyfixn: bool,
    pub fert: bool,
    pub irrigation: bool,
    pub laifeedback: bool,
    pub nostressnitrogen: bool,
}

macro_rules! physics_fields {
    (
        vectors: [$($vec:ident),* $(,)?],
        scalars: [$($scalar:ident),* $(,)?],
        optional_patch: [$($opatch:ident),* $(,)?],
        optional_pft: [$($opft:ident),* $(,)?] $(,)?
    ) => {
        /// BGC 读写的非 BGC 量（一个 patch）。PFT 量按 patch 内 PFT 顺序，土壤量取 `1:nl_soil`，
        /// `rootfr_p` 是 `rootfr_p(1:nl_soil, pftclass(m))` 按 PFT 排开（列主序）。
        #[derive(Debug, Clone, PartialEq, Default)]
        #[allow(non_snake_case)]
        pub struct BgcPhysics {
            /// 年、年内日、日内秒
            pub idate: [i32; 3],
            pub pftclass: Vec<i32>,
            pub patchclass: i32,
            $(pub $vec: Vec<f64>,)*
            $(pub $scalar: f64,)*
            $(pub $opatch: Vec<f64>,)*
            $(pub $opft: Vec<f64>,)*
        }

        impl BgcPhysics {
            /// 从 Fortran 追踪记录的物理输入段构造。
            pub fn from_trace(record: &TraceRecord) -> Result<Self> {
                let get = |name: &str| {
                    record
                        .input(name)
                        .map(<[f64]>::to_vec)
                        .with_context(|| format!("trace record lacks input {name}"))
                };
                let scalar = |name: &str| -> Result<f64> {
                    let values = get(name)?;
                    ensure!(values.len() == 1, "input {name} is not a scalar");
                    Ok(values[0])
                };
                // 只有 BGC 历史/作物用到的量：旧追踪里没有时按上游初值 spval 填。
                let optional = |name: &str, len: usize| {
                    record.input(name).map_or_else(|| vec![crate::MISSING; len], <[f64]>::to_vec)
                };
                let npft = get("pftfrac")?.len();
                let idate = get("idate")?;
                ensure!(idate.len() == 3, "idate must have three entries");
                Ok(Self {
                    idate: [idate[0] as i32, idate[1] as i32, idate[2] as i32],
                    pftclass: get("pftclass")?.iter().map(|value| *value as i32).collect(),
                    patchclass: scalar("patchclass")? as i32,
                    $($vec: get(stringify!($vec))?,)*
                    $($scalar: scalar(stringify!($scalar))?,)*
                    $($opatch: optional(stringify!($opatch), 1),)*
                    $($opft: optional(stringify!($opft), npft),)*
                })
            }

            /// 追踪记录的物理输入段（名字与 Fortran 插桩一致）。
            pub fn trace_inputs(&self) -> Vec<(&'static str, Vec<f64>)> {
                let mut out = vec![
                    ("idate", self.idate.iter().map(|value| f64::from(*value)).collect()),
                    ("pftclass", self.pftclass.iter().map(|value| f64::from(*value)).collect()),
                    ("patchclass", vec![f64::from(self.patchclass)]),
                ];
                $(out.push((stringify!($vec), self.$vec.clone()));)*
                $(out.push((stringify!($scalar), vec![self.$scalar]));)*
                $(out.push((stringify!($opatch), self.$opatch.clone()));)*
                $(out.push((stringify!($opft), self.$opft.clone()));)*
                out
            }
        }
    };
}

physics_fields!(
    vectors: [
        z_soi, dz_soi, zi_soi, pftfrac, rootfr_p, tsai_p, tlai_p, lai_p, laisun_p, laisha_p,
        sigf_p, tref_p, assim_p, respc_p, patchlatr, porsl, psi0, bsw, theta_r, alpha_vgm, n_vgm,
        L_vgm, sc_vgm, fc_vgm, BD_all, wfc, OM_density, lai, tlai, tref, t_soisno, wliq_soisno,
        wice_soisno, smp, h2osoi, rsur, rnof, forc_t, forc_q, forc_psrf, forc_prc, forc_prl,
        forc_us, forc_vs,
    ],
    scalars: [deltim, dlat, dlon, smpmax_hr, smpmin_hr],
    optional_patch: [
        lai_enftemp, lai_enfboreal, lai_dnfboreal, lai_ebftrop, lai_ebftemp, lai_dbftrop, lai_dbftemp, lai_dbfboreal, lai_ebstemp, lai_dbstemp, lai_dbsboreal, lai_c3arcgrass, lai_c3grass, lai_c4grass,
        irrig_method_corn, irrig_method_swheat, irrig_method_wwheat, irrig_method_soybean, irrig_method_cotton, irrig_method_rice1, irrig_method_rice2, irrig_method_sugarcane,
    ],
    optional_pft: [irrig_method_p],
);

impl BgcPhysics {
    /// 形参声明成 `zi_soi(0:…)` 时的 `zi_soi(k)`（`SoilBiogeochemNLeaching`、`LittVertTransp`）。
    ///
    /// driver 传的是全局 `zi_soi(1:nl_soil)`，按序列关联形参 `zi_soi(k)` = 全局 `zi_soi(k+1)`；
    /// `k ≥ nl_soil` 越界，参考内核里 `MOD_Vars_Global` 的布局是 `zi_soi` 之后紧接 `z_soi`，
    /// 所以读到的是 `z_soi(k − nl_soil + 1)`。
    pub fn zi_soi_from_zero(&self, k: usize) -> f64 {
        let nl = self.zi_soi.len();
        if k < nl {
            self.zi_soi[k]
        } else {
            self.z_soi[k - nl]
        }
    }
}

macro_rules! pft_constants {
    ($($name:ident),* $(,)?) => {
        /// `MOD_Const_PFT` 里 BGC 用到的按类别参数，下标是 PFT 类别（0 起）。
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct BgcPftConstants {
            $(pub $name: Vec<f64>,)*
        }

        impl BgcPftConstants {
            /// 参数的 Fortran 名。
            pub const NAMES: &'static [&'static str] = &[$(stringify!($name)),*];

            pub fn field_mut(&mut self, name: &str) -> Option<&mut Vec<f64>> {
                match name {
                    $(stringify!($name) => Some(&mut self.$name),)*
                    _ => None,
                }
            }
        }
    };
}

// 前 14 个是 `MOD_Const_PFT` 写死的类别标志与常数（逻辑量读成 1/0），其余可被 `DEF_PFT_*` 覆盖。
pft_constants!(
    woody, isevg, issed, isstd, isbare, iscrop, isnatveg, isshrub, isgrass, isbetr, isbdtr,
    dsladlai, declfact, allconsl, cc_dstem, cc_leaf, cc_lstem, cc_other, croot_stem, deadwdcn,
    fcur2, fd_pft, flivewd, fm_droot, fm_leaf, fm_lroot, fm_lstem, fm_other, fm_root, fr_fcel,
    fr_flab, fr_flig, froot_leaf, frootcn, fsr_pft, graincn, grperc, grpnow, laimx, leaf_long,
    leafcn, lf_fcel, lf_flab, lf_flig, lflitcn, livewdcn, slatop, stem_leaf,
);

/// `MOD_Vars_Global` 的 `npcropmin`：第一个作物 PFT 类别。
pub const NPCROPMIN: i32 = 17;

/// `MOD_TimeManager:isendofyear(idate, sec)`：`idate + int(sec)` 是否跨年（秒数进位条件是
/// 严格大于 86400）。
pub fn is_end_of_year(idate: [i32; 3], seconds: f64) -> bool {
    let (mut year, mut day, mut sec) = (idate[0], idate[1], idate[2] + seconds as i32);
    while sec > 86400 {
        sec -= 86400;
        day += 1;
        if day > if is_leap_year(year) { 366 } else { 365 } {
            year += 1;
            day = 1;
        }
    }
    year != idate[0]
}

/// gfortran -O2 把个别 `sum(x(ps:pe) * pftfrac(ps:pe))` 向量化成保序（fold-left）归约时的
/// 求值顺序（GIMPLE 里是 `BIT_FIELD_REF` 取 lane）：只有一个 PFT（`ps == pe`）时整段走标量，
/// 收缩成 FMA；否则每两个一组，乘积用向量乘法算好再按 lane 顺序加（**不融合**），元素数为奇数时
/// 最后一个走标量尾部，又收缩成 FMA。`term(m)` 返回乘积的两个因子。
pub(crate) fn vectorized_dot(n: usize, term: impl Fn(usize) -> (f64, f64)) -> f64 {
    let mut acc = 0.0;
    let pairs = if n == 1 { 0 } else { n / 2 };
    for k in 0..2 * pairs {
        let (a, b) = term(k);
        acc += a * b;
    }
    for k in 2 * pairs..n {
        let (a, b) = term(k);
        acc = a.mul_add(b, acc);
    }
    acc
}

/// 一个阶段需要的全部输入。
pub struct BgcStep<'a> {
    pub state: &'a mut BgcState,
    pub physics: &'a mut BgcPhysics,
    pub pft: &'a BgcPftConstants,
    pub switches: BgcSwitches,
}

/// 上游 driver 的阶段序列（不含依状态而定的平衡检查，见 [`bgc_driver`]）。
pub fn stage_sequence(switches: BgcSwitches) -> Vec<&'static str> {
    let mut out = vec![
        "BeginCNBalance",
        "CNZeroFluxes",
        "CNNFixation",
        "CNMResp",
        "decomp_rate_constants_bgc",
        "SoilBiogeochemPotential",
        "SoilBiogeochemVerticalProfile",
    ];
    if switches.nitrif {
        out.push("SoilBiogeochemNitrifDenitrif");
    }
    out.extend([
        "calc_plant_nutrient_demand_CLM45",
        "SoilBiogeochemCompetition",
        "calc_plant_nutrient_competition_",
    ]);
    if switches.crop && switches.cnsoyfixn {
        out.push("CNSoyfix");
    }
    out.extend(["SoilBiogeochemDecomp", "CNPhenology1", "CNPhenology2"]);
    if switches.crop {
        out.push("CNNFert");
    }
    out.push("CNGResp");
    if switches.crop && switches.irrigation {
        out.push("CalIrrigationNeeded");
    }
    out.extend([
        "CStateUpdate1",
        "NStateUpdate1",
        "SoilBiogeochemNStateUpdate1",
        "SoilBiogeochemLittVertTransp",
        "CNGapMortality",
        "CStateUpdate2",
        "NStateUpdate2",
    ]);
    if switches.fire {
        out.extend(["CNFireArea", "CNFireFluxes"]);
    }
    out.extend([
        "CStateUpdate3",
        "CNAnnualUpdate",
        "SoilBiogeochemNLeaching",
        "NstateUpdate3",
    ]);
    if switches.sasu || switches.diag_matrix {
        out.push("CNSASU");
    }
    out.extend(["CNDriverSummarizeStates", "CNDriverSummarizeFluxes"]);
    out
}

/// 跑一个阶段。阶段名见 [`stage_sequence`]，外加 `CBalanceCheck`/`NBalanceCheck`/
/// `CNVegStructUpdate`。
pub fn run_stage(stage: &str, step: &mut BgcStep<'_>) -> Result<()> {
    let switches = step.switches;
    macro_rules! gen {
        ($f:path) => {
            $f(step.state, step.physics, step.pft, switches)
        };
    }
    match stage {
        "CNZeroFluxes" => cn_zero_fluxes(
            step.state,
            ZeroFluxSwitches {
                crop: switches.crop,
                use_nitrif: switches.nitrif,
            },
        ),
        "CNNFixation" => crate::bgc_n_dynamics::cn_n_fixation(step.state, step.physics.idate),
        "CNMResp" => crate::bgc_resp::cn_m_resp(step.state, step.physics, step.pft),
        "decomp_rate_constants_bgc" => {
            crate::bgc_decomp::decomp_rate_constants_bgc(step.state, step.physics)
        }
        "SoilBiogeochemPotential" => crate::bgc_decomp::soil_biogeochem_potential(step.state),
        "SoilBiogeochemVerticalProfile" => {
            crate::bgc_vertical_profile::soil_biogeochem_vertical_profile(step.state, step.physics)?
        }
        "calc_plant_nutrient_demand_CLM45" => {
            crate::bgc_nutrient::plant_nutrient_demand(step.state, step.physics, step.pft)
        }
        "SoilBiogeochemCompetition" => {
            // driver 在需求与竞争之间内联了 patch 级需求的求和。
            crate::bgc_nutrient::patch_plant_ndemand(step.state, step.physics);
            gen!(crate::bgc_soil_competition::soil_biogeochem_competition)
        }
        "SoilBiogeochemNitrifDenitrif" => {
            crate::bgc_nitrif::soil_biogeochem_nitrif_denitrif(step.state, step.physics)
        }
        "calc_plant_nutrient_competition_" => {
            crate::bgc_nutrient::plant_nutrient_competition(step.state, step.physics, step.pft)
        }
        "SoilBiogeochemDecomp" => {
            crate::bgc_decomp::soil_biogeochem_decomp(step.state, step.physics, switches)
        }
        "CNPhenology1" => {
            crate::bgc_phenology::cn_phenology_phase1(step.state, step.physics, step.pft)
        }
        "CNPhenology2" => {
            crate::bgc_phenology::cn_phenology_phase2(step.state, step.physics, step.pft)
        }
        "CNGResp" => crate::bgc_resp::cn_g_resp(step.state, step.physics, step.pft),

        "SoilBiogeochemLittVertTransp" => {
            crate::bgc_litt_vert_transp::soil_biogeochem_litt_vert_transp(
                step.state,
                step.physics,
                switches,
            )?
        }

        // 以下阶段由 oracle/scripts/bgc_port/regen.py 生成，签名统一。
        "BeginCNBalance" => gen!(crate::bgc_balance::begin_cn_balance),
        "CStateUpdate1" => gen!(crate::bgc_c_state_update::c_state_update1),
        "NStateUpdate1" => gen!(crate::bgc_n_state_update::n_state_update1),
        "SoilBiogeochemNStateUpdate1" => {
            gen!(crate::bgc_soil_n_state_update::soil_biogeochem_n_state_update1)
        }
        "CNGapMortality" => gen!(crate::bgc_gap_mortality::cn_gap_mortality),
        "CStateUpdate2" => gen!(crate::bgc_c_state_update::c_state_update2),
        "NStateUpdate2" => gen!(crate::bgc_n_state_update::n_state_update2),
        "CStateUpdate3" => gen!(crate::bgc_c_state_update::c_state_update3),
        "CNAnnualUpdate" => gen!(crate::bgc_annual_update::cn_annual_update),
        "SoilBiogeochemNLeaching" => gen!(crate::bgc_n_leaching::soil_biogeochem_n_leaching),
        "NstateUpdate3" => gen!(crate::bgc_n_state_update::n_state_update3),
        // driver 以 init=.false. 调用
        "CNDriverSummarizeStates" => crate::bgc_summary::cn_driver_summarize_states(
            step.state,
            step.physics,
            step.pft,
            switches,
            false,
        ),
        "CNDriverSummarizeFluxes" => gen!(crate::bgc_summary::cn_driver_summarize_fluxes)?,
        "CBalanceCheck" => gen!(crate::bgc_balance::c_balance_check)?,
        "NBalanceCheck" => gen!(crate::bgc_balance::n_balance_check)?,
        "CNVegStructUpdate" => gen!(crate::bgc_veg_struct::cn_veg_struct_update),
        _ => bail!("BGC stage {stage} is not ported yet"),
    }
    Ok(())
}

/// 整步运行。`trace` 在每个阶段之后被调用（首尾是 `begin`/`end`），与 Fortran 插桩位置一致。
pub fn bgc_driver(
    step: &mut BgcStep<'_>,
    trace: &mut dyn FnMut(&str, &BgcState, &BgcPhysics),
) -> Result<()> {
    trace("begin", step.state, step.physics);
    for stage in stage_sequence(step.switches) {
        run_stage(stage, step)?;
        trace(stage, step.state, step.physics);
    }
    if step.state.patch.skip_balance_check[0] {
        step.state.patch.skip_balance_check[0] = false;
    } else {
        for stage in ["CBalanceCheck", "NBalanceCheck"] {
            run_stage(stage, step)?;
            trace(stage, step.state, step.physics);
        }
    }
    run_stage("CNVegStructUpdate", step)?;
    trace("CNVegStructUpdate", step.state, step.physics);
    trace("end", step.state, step.physics);
    Ok(())
}

#[cfg(test)]
#[path = "bgc_driver_tests.rs"]
mod bgc_driver_tests;
