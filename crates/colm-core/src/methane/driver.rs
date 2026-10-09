//! `methane_driver` 的土壤与湿地 patch 路径（`MOD_Tracer_Reactive_Methane_Driver` +
//! `MOD_Tracer_Reactive_Methane_Impl:ch4_impl_soil_step`），以及 `core` history 累加。
//!
//! 土壤 patch 的水稻比例为 0：只跑土壤分量，两分量合并（`aggregate_methane_columns`）在
//! `ws = 1, wr = 0` 下对每个量都是恒等，所以聚合状态就是土壤分量。湿地 patch 不分分量，直接用
//! patch 级聚合状态调 `methane`，分量状态停在冷启动值。水稻、湖未移植。

use anyhow::{ensure, Result};
use colm_numeric::Contract;

use super::bgc_link::{self, BgcInputs};
use super::column::{self, ColumnInput, ColumnResult, ComponentState, LakeHost, LakeState};
use super::config::{MethaneParameters, COMP_RICE, COMP_SOIL};
use super::physics::{self, sn, MAXSNL, NL_SOIL, SOISNO, SPVAL};
use crate::bgc_state::BgcState;

/// 一个 patch 的甲烷状态（续跑量 + 最近一步的诊断）。
#[derive(Debug, Clone, PartialEq)]
pub struct MethanePatch {
    pub components: [ComponentState; 2],
    /// patch 级聚合状态（`conc_*_unsat/sat(i)`、`layer_sat_lag(i)`、年累加、`fsat_bef(i)` 等）。
    pub aggregate: ComponentState,
    pub rice_fraction_prev: f64,
    pub f_h2osfc: f64,
    pub lake_soilc: [f64; NL_SOIL],
    /// 合并后的列总量（下一步收支检查的步首值）与地表导度。
    pub totcol_methane: f64,
    pub grnd_methane_cond: f64,
    /// 最近一步的结果；冷启动尚未走过一步时为 `None`。
    pub last: Option<ColumnResult>,
    /// `methane_soil_zwt`（history 用）。
    pub soil_zwt: f64,
    /// 湖泊 patch 的沉积层与水柱状态（其余 patch 停在冷启动值）。
    pub lake: LakeState,
    /// 网格河湖每次汇流末推到 patch 的淹没比例（`publish_methane_levee_flood_patch`、
    /// `publish_methane_flood_patch`）：`f_inund_levee_patch`、`f_inund_flood_patch` 与
    /// `f_inund_flood_depth_patch`。`wetwat` 方案的物理不用它们，只随重启往返。
    pub flood: [f64; 3],
    /// `methane_patch_active_mask`：history 的活跃甲烷 patch（累加计数与网格分母都按它）。
    /// 每步在 [`soil_step`] 开头按 `patchtype`、`only_wetland` 与稻田重算；不进重启。
    /// 还没走过一步时为 `None`，按默认（土壤或湿地）处理。
    pub history_active: Option<bool>,
}

impl MethanePatch {
    /// 当前的活跃掩膜；没算过时按 `only_wetland = .false.`、不开稻田的默认。
    pub fn history_active_or_default(&self, patch_type: i32) -> bool {
        self.history_active
            .unwrap_or(patch_type == 0 || patch_type == 2)
    }

    /// `allocate_methane_state` 的冷启动值。
    pub fn cold(params: &MethaneParameters) -> Self {
        Self {
            components: [ComponentState::cold(), ComponentState::cold()],
            aggregate: ComponentState::cold(),
            rice_fraction_prev: 0.0,
            f_h2osfc: 0.0,
            lake_soilc: [0.0; NL_SOIL],
            totcol_methane: 0.0,
            grnd_methane_cond: params.methane.grnd_methane_cond_default,
            last: None,
            soil_zwt: SPVAL,
            lake: LakeState::cold(params.methane.grnd_methane_cond_default),
            flood: [0.0; 3],
            history_active: None,
        }
    }
}

/// patch 的静态量。
#[derive(Debug, Clone, PartialEq)]
pub struct MethaneSite {
    pub patchtype: i32,
    pub patchclass: i32,
    pub dlat: f64,
    pub slpratio: f64,
    /// `rootfr_lc(1:nl_soil, patchclass)`。
    pub rootfr: [f64; NL_SOIL],
    /// `z_soi`、`dz_soi`、`zi_soi`（1..nl_soil）。
    pub z_soi: [f64; NL_SOIL],
    pub dz_soi: [f64; NL_SOIL],
    pub zi_soi: [f64; NL_SOIL],
    pub bsw: [f64; NL_SOIL],
    pub porsl: [f64; NL_SOIL],
    pub organic_max: f64,
    /// `wetwatmax`（湿地水桶容量，`enable_wetwat_finundated_override` 用）。
    pub wetwatmax: f64,
    /// `wetland_frac_per_patch`（`init_methane_wetland_fraction_cache`）：单元里湿地占
    /// 土壤 + 湿地面积的份额；单点与没有算过时是分配值 1。
    pub wetland_fraction: f64,
    /// 方案 5 的 GIEMS 月序列（`read_methane_giems` 按 patch 中心取最近像元）。
    pub giems: Option<super::giems::GiemsPatch>,
}

/// 一步里来自宿主（能量、水、强迫）的量。
#[derive(Debug, Clone, Copy)]
pub struct HostInputs<'a> {
    /// 步末日历 `(year, julian_day, seconds)`。
    pub idate: [i32; 3],
    pub deltim: f64,
    pub t_soisno: &'a [f64; SOISNO],
    pub wliq_soisno: &'a [f64; SOISNO],
    pub wice_soisno: &'a [f64; SOISNO],
    pub t_grnd: f64,
    pub forc_t: f64,
    pub forc_pbot: f64,
    pub forc_po2m: f64,
    pub forc_pco2m: f64,
    pub ustar: f64,
    pub fq: f64,
    pub zwt: f64,
    pub snowdp: f64,
    pub etr: f64,
    pub wdsrf: f64,
    pub wetwat: f64,
    pub smp: &'a [f64; NL_SOIL],
    pub lai: f64,
    pub sai: f64,
    pub rootr: &'a [f64; NL_SOIL],
    pub frcsat: f64,
    /// 逐 PFT 的类别、面积份额、`lai_p`（`bgc_driver` 之后）与 `irrig_method_p`；湿地为空。
    pub pft: bgc_link::PftInputs<'a>,
    /// 湖泊 patch 的湖层与风；其余 patch 为 `None`。
    pub lake: Option<LakeHost<'a>>,
    pub dynamic_wetland: bool,
}

/// `methane_soisno_geometry`：土层用静态深度，雪层厚度按各层质量分配 `snowdp`。
/// 返回 `(snl, z, dz, zi)`；`zi` 的 `j` 存在 `j - MAXSNL`。
pub fn soisno_geometry(
    site: &MethaneSite,
    wliq: &[f64; SOISNO],
    wice: &[f64; SOISNO],
    snowdp: f64,
) -> (i32, [f64; SOISNO], [f64; SOISNO], [f64; SOISNO + 1]) {
    use physics::{DENH2O, DENICE};
    let mut z = [0.0; SOISNO];
    let mut dz = [0.0; SOISNO];
    let mut zi = [0.0; SOISNO + 1];
    let zi_at = |j: i32| (j - MAXSNL) as usize;
    for k in 0..NL_SOIL {
        let j = k as i32 + 1;
        z[sn(j)] = site.z_soi[k];
        dz[sn(j)] = site.dz_soi[k];
        zi[zi_at(j)] = site.zi_soi[k];
    }
    let mut snl = 0;
    for j in (MAXSNL + 1..=0).rev() {
        if wliq[sn(j)] + wice[sn(j)] > 0.0 {
            snl -= 1;
        } else {
            break;
        }
    }
    let lb = snl + 1;
    if snl < 0 {
        let mut total = 0.0;
        for j in (lb..=0).rev() {
            total += (wliq[sn(j)] + wice[sn(j)]).max(0.0);
        }
        for j in (lb..=0).rev() {
            let mass = (wliq[sn(j)] + wice[sn(j)]).max(0.0);
            let depth = if snowdp > 0.0 && total > 0.0 {
                snowdp * mass / total
            } else if mass > 0.0 {
                wliq[sn(j)] / DENH2O + wice[sn(j)] / DENICE
            } else {
                0.0
            };
            dz[sn(j)] = depth.max(1.0e-12);
            z[sn(j)] = zi[zi_at(j)] - 0.5 * dz[sn(j)];
            zi[zi_at(j - 1)] = zi[zi_at(j)] - dz[sn(j)];
        }
    }
    (snl, z, dz, zi)
}

/// 宿主强迫的合法化（`methane_driver` 开头）。
fn sanitize(x: f64, bad: impl Fn(f64) -> bool, fallback: f64) -> f64 {
    if x.is_nan() || x.abs() >= 0.5 * SPVAL.abs() || bad(x) {
        fallback
    } else {
        x
    }
}

/// `ch4_impl_soil_step` + `methane_driver`（`patchtype == 0`、非水稻）。BGC 的
/// `decomp_hr`/`er` 在步末经 [`bgc_link::finalize`] 改写。
pub fn soil_step(
    params: &MethaneParameters,
    scheme: i32,
    site: &MethaneSite,
    host: &HostInputs<'_>,
    bgc: &mut BgcState,
    patch: &mut MethanePatch,
) -> Result<()> {
    let m = &params.methane;
    ensure!(
        matches!(site.patchtype, 0 | 2 | 4),
        "methane on patchtype {} is not ported to the Rust runtime yet",
        site.patchtype
    );
    // `methane_patch_active_mask`：湿地；不是 `only_wetland` 时加土壤；开了稻田时再加有稻田的土壤
    // （上游只在 CROP 内核里认稻田；非 CROP 运行没有稻田 PFT，份额恒为 0）。
    patch.history_active = Some(
        site.patchtype == 2
            || (site.patchtype == 0
                && (!m.only_wetland
                    || (m.enable_rice_paddy
                        && bgc_link::paddy_rice_fraction(&host.pft)
                            > bgc_link::PADDY_RICE_FRAC_MIN))),
    );
    // 湖泊 patch（`ch4_impl_lake_step`）：只在 `allowlakeprod` 下跑，否则状态与诊断都不动。
    let lake = site.patchtype == 4;
    if lake && !m.allowlakeprod {
        return Ok(());
    }
    // 湿地与湖都不分分量，直接推进 patch 级聚合状态。
    let wetland = site.patchtype == 2;
    let direct = wetland || lake;
    // `ch4_impl_soil_step`：稻田模式下土壤 patch 的稻田 PFT 份额超过 `PADDY_RICE_FRAC_MIN` 才算稻田，
    // 那时即使 `only_wetland` 也要跑。
    let rice_pft_frac = if m.enable_rice_paddy && site.patchtype == 0 {
        bgc_link::paddy_rice_fraction(&host.pft)
    } else {
        0.0
    };
    let is_rice_paddy = rice_pft_frac > bgc_link::PADDY_RICE_FRAC_MIN;
    let run = lake
        || if m.only_wetland {
            site.patchtype == 2 || is_rice_paddy
        } else {
            site.patchtype == 2 || site.patchtype == 0
        };
    if !run {
        return Ok(());
    }
    patch.f_h2osfc = physics::f_h2osfc(&params.hydrology, site.slpratio, host.wdsrf);
    let (snl, z, dz, zi) = soisno_geometry(site, host.wliq_soisno, host.wice_soisno, host.snowdp);
    // `forc_*_eff`。
    let forc_t = sanitize(host.forc_t, |x| x < 150.0 || x > 350.0, 288.15);
    let forc_pbot = sanitize(host.forc_pbot, |x| x <= 0.0, 101325.0);
    let forc_po2m = sanitize(host.forc_po2m, |x| x <= 0.0, 0.2095 * forc_pbot);
    let forc_pco2m = sanitize(host.forc_pco2m, |x| x < 0.0, 415.0e-6 * forc_pbot);
    let mut inputs: BgcInputs =
        bgc_link::patch_inputs(m, bgc, &site.rootfr, host.pft.fraction, &site.dz_soi)?;
    // 湿地植被代理（`get_wetland_veg_proxy`）：没有 PFT，LAI/NPP/根廓线按气候带给，`crootfr` 与根吸水
    // 廓线都换成代理廓线，根呼吸取 `max(rr, 0.5·bgnpp)`；通气组织覆盖在这条路径上生效。
    let mut lai = host.lai;
    let mut rootfr = site.rootfr;
    let mut rootr = *host.rootr;
    let mut aere_override = None;
    if wetland {
        let proxy = bgc_link::wetland_veg_proxy(site.dlat, inputs.cellorg[0], host.lai);
        lai = proxy.lai;
        inputs.annsum_npp = proxy.annsum_npp;
        inputs.agnpp = proxy.agnpp;
        inputs.bgnpp = proxy.bgnpp;
        rootfr = proxy.rootfr;
        inputs.crootfr = proxy.rootfr;
        rootr = proxy.rootfr;
        inputs.rr = inputs.rr.max(0.5 * inputs.bgnpp);
        aere_override = Some(proxy.aere);
    }
    // 稻田（`methane_driver` 开头）：稻田参数在生长季生效，收获后排水期（`rice_drain_window_days`）内仍生效。
    let rice_pft_frac = rice_pft_frac.max(0.0).min(1.0);
    let rice_live = is_rice_paddy && bgc_link::is_paddy_rice_live(bgc, &host.pft);
    let mut rice_parameter_active = is_rice_paddy && rice_pft_frac > 0.0;
    if rice_parameter_active && !rice_live {
        let dsh = bgc_link::rice_days_since_harvest(bgc, &host.pft, host.idate[1], host.idate[0]);
        rice_parameter_active = dsh >= 0 && f64::from(dsh) < m.rice_drain_window_days;
    }
    // 土壤 patch 的两个分量：按稻田份额整 patch 划给水稻或土壤分量。
    let mut component = COMP_SOIL;
    let mut veg = None;
    if !direct {
        let mut rice_weight = if is_rice_paddy { rice_pft_frac } else { 0.0 };
        rice_weight = rice_weight.max(0.0).min(1.0);
        if rice_weight <= 1.0e-14 {
            rice_weight = 0.0;
        }
        if rice_weight >= 1.0 - 1.0e-14 {
            rice_weight = 1.0;
        }
        if is_rice_paddy {
            let v = bgc_link::component_veg_inputs(bgc, &host.pft, &site.rootfr, &site.dz_soi);
            ensure!(
                !(v.ready.iter().any(|&r| r)
                    && (v.fraction[COMP_RICE] - rice_weight).abs() > 1.0e-8),
                "rice methane component fraction disagrees with PFT bridge"
            );
            veg = Some(v);
        }
        // 稻田份额只在 CROP 内核里非零，而 CROP 的农田 patch 恰好一个 PFT（`MOD_LandPFT.F90:211-216`、
        // `:306-310`：`patch_pft_s = patch_pft_e`），份额只会是 0 或 1。两分量都跑再按份额合并的
        // `aggregate_methane_columns` 在上游的标准布局里走不到，没有移植。
        ensure!(
            rice_weight == 0.0 || rice_weight == 1.0,
            "mixed soil/rice methane patches (rice fraction {rice_weight}) cannot occur in upstream \
             layouts (a CROP cropland patch has exactly one PFT); the mixed-column aggregation is \
             not ported"
        );
        repartition(patch, patch.rice_fraction_prev, rice_weight);
        patch.rice_fraction_prev = rice_weight;
        if rice_weight == 1.0 {
            component = COMP_RICE;
        }
    }
    let rice_column = component == COMP_RICE;
    // `run_methane_component`：分量就绪时用分量的植被输入；稻田分量在生长季用稻田通气组织几何。
    if let Some(v) = veg.filter(|v| v.ready[component]) {
        lai = v.lai[component];
        inputs.crootfr = v.crootfr[component];
        inputs.rr = v.rr[component];
        inputs.agnpp = v.agnpp[component];
        inputs.bgnpp = v.bgnpp[component];
        inputs.annsum_npp = v.annsum_npp[component];
    }
    if !direct {
        aere_override = (rice_column && rice_live).then(|| bgc_link::rice_veg_proxy(lai, 1.0));
    }
    let rice = bgc_link::RiceWeight {
        is_rice_paddy: rice_column,
        fraction: if rice_column { 1.0 } else { 0.0 },
        parameter_active: rice_column && rice_parameter_active,
    };
    let biome_f = bgc_link::biome_f_methane(m, site.patchtype, site.dlat, inputs.cellorg[0], rice);
    let biome_redox =
        bgc_link::biome_redoxlag(m, site.patchtype, site.dlat, inputs.cellorg[0], rice);
    // 湿地：上一步的 `totcol_methane(i)` 就是步首列总量；土壤（`run_methane_component`）：由本分量
    // 的两相浓度与 `fsat_bef` 拼出（只给收支检查用）。
    let previous_totcol = patch.totcol_methane;
    let comp = if direct {
        &mut patch.aggregate
    } else {
        &mut patch.components[component]
    };
    let fsat = comp.fsat_bef;
    let col_sum = |x: &[f64; NL_SOIL]| {
        (0..NL_SOIL).fold(0.0f64, |acc, k| x[k].contract(dz[sn(k as i32 + 1)], acc))
    };
    let totcol_unsat = col_sum(&comp.conc_methane_unsat);
    let totcol_sat = col_sum(&comp.conc_methane_sat);
    let totcol_before = if direct {
        previous_totcol
    } else if (0.0..=1.0).contains(&fsat) {
        fsat.contract(totcol_sat, (1.0 - fsat) * totcol_unsat)
    } else {
        0.5 * (totcol_sat + totcol_unsat)
    };
    let mut result = column::methane(
        m,
        &ColumnInput {
            idate: host.idate,
            patchclass: site.patchclass,
            patchtype: site.patchtype,
            snl,
            dlat: site.dlat,
            deltim: host.deltim,
            z_soisno: &z,
            dz_soisno: &dz,
            zi_soisno: &zi,
            t_soisno: host.t_soisno,
            t_grnd: host.t_grnd,
            wliq_soisno: host.wliq_soisno,
            wice_soisno: host.wice_soisno,
            forc_t,
            forc_pbot,
            forc_po2m,
            forc_pco2m,
            zwt: host.zwt,
            rootfr: &rootfr,
            snowdp: host.snowdp,
            etr: host.etr,
            wdsrf: host.wdsrf,
            wetwat: host.wetwat,
            bsw: &site.bsw,
            smp: host.smp,
            porsl: &site.porsl,
            lai,
            sai: host.sai,
            rootr: &rootr,
            annsum_npp: inputs.annsum_npp,
            rr: inputs.rr,
            frcsat: host.frcsat,
            f_h2osfc: patch.f_h2osfc,
            agnpp: inputs.agnpp,
            bgnpp: inputs.bgnpp,
            somhr: inputs.somhr,
            crootfr: &inputs.crootfr,
            lithr: inputs.lithr,
            hr_vr: &inputs.hr_vr,
            o_scalar: &inputs.o_scalar,
            fphr: &inputs.fphr,
            pot_f_nit_vr: &inputs.pot_f_nit_vr,
            ph: inputs.ph,
            cellorg: &inputs.cellorg,
            t_h2osfc: host.t_grnd,
            organic_max: site.organic_max,
            ustar: host.ustar,
            fq: host.fq,
            atm_methane_mix: m.atm_methane,
            dynamic_wetland: host.dynamic_wetland,
            scheme,
            flood_fraction: patch.flood[1],
            flood_depth_m: patch.flood[2],
            wetland_fraction: site.wetland_fraction,
            giems: site.giems.as_ref(),
            biome_f_methane: Some(biome_f),
            biome_redoxlag: Some(biome_redox),
            aere_override,
            wetwatmax: site.wetwatmax,
            totcol_before,
            lake_soilc: &patch.lake_soilc,
            lake: host.lake,
        },
        comp,
        lake.then_some(&mut patch.lake),
    )?;
    bgc_link::finalize(bgc, site.patchtype, result.merged.net, &site.dz_soi)?;
    patch.totcol_methane = result.merged.totcol;
    patch.grnd_methane_cond = result.merged.grnd_cond;
    if lake {
        // 湖跳过非饱和相时 `grnd_methane_cond_unsat(i)` 不被改写，沿用上一步（或冷启动默认）的值。
        result.unsat.grnd_cond = patch
            .last
            .map_or(m.grnd_methane_cond_default, |last| last.unsat.grnd_cond);
        if let Some(soilc) = result.lake_soilc {
            patch.lake_soilc = soilc;
        }
    } else if !wetland {
        // `aggregate_methane_columns`：份额 0/1 时聚合逐位等于跑过的那个分量。
        patch.soil_zwt = host.zwt;
        patch.aggregate = patch.components[component];
    }
    // 水稻分量不跑，状态保持。
    patch.last = Some(result);
    Ok(())
}

/// `core` history 的 14 个累加量与计数（`accumulate_methane_fluxes` 的 `core_history_only` 支）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoreAccumulator {
    pub surf_flux_tot: f64,
    pub surf_flux_tot_phys: f64,
    pub balance_residual: f64,
    pub ch4_clip_credit: f64,
    pub o2_cap_loss: f64,
    pub o2_cap_gain: f64,
    pub prod_tot: f64,
    pub oxid_tot: f64,
    pub totcol: f64,
    pub surf_flux_wetland: f64,
    pub surf_flux_soil: f64,
    pub surf_flux_lake: f64,
    pub surf_flux_rice: f64,
    pub surf_flux_tot_lake: f64,
    /// `a_methane_acc_num`、`a_methane_acc_num_lake`。
    pub acc_num: f64,
    pub acc_num_lake: f64,
}

impl Default for CoreAccumulator {
    /// `allocate_methane_acc_fluxes`/`flush_methane_acc_fluxes`：全部清零。
    fn default() -> Self {
        Self {
            surf_flux_tot: 0.0,
            surf_flux_tot_phys: 0.0,
            balance_residual: 0.0,
            ch4_clip_credit: 0.0,
            o2_cap_loss: 0.0,
            o2_cap_gain: 0.0,
            prod_tot: 0.0,
            oxid_tot: 0.0,
            totcol: 0.0,
            surf_flux_wetland: 0.0,
            surf_flux_soil: 0.0,
            surf_flux_lake: 0.0,
            surf_flux_rice: 0.0,
            surf_flux_tot_lake: 0.0,
            acc_num: 0.0,
            acc_num_lake: 0.0,
        }
    }
}

/// `acc1d` 的一个元素：值有效才加，累加量无效时先清零。
fn acc(sum: &mut f64, value: f64) {
    let valid = |x: f64| !x.is_nan() && x.abs() < 0.5 * SPVAL.abs();
    if valid(value) {
        if !valid(*sum) {
            *sum = 0.0;
        }
        *sum += value;
    }
}

impl CoreAccumulator {
    /// 一步的累加。`patch_type` 决定 `methane_active_mask`（土壤/湿地 patch）与通量归到哪一类：
    /// 湿地 patch 的 `methane_surf_flux_wetland` 是总通量、土壤类为 0；土壤 patch 反之。没跑过
    /// 甲烷的 patch 用冷启动时的状态值（`allocate_methane_state` 的 0 与默认值）。
    pub fn accumulate(&mut self, patch: &MethanePatch, patch_type: i32) {
        let active = patch.history_active_or_default(patch_type);
        let r = patch.last.unwrap_or_default();
        let g = &r.merged;
        acc(&mut self.surf_flux_tot, g.surf_flux);
        acc(&mut self.surf_flux_tot_phys, r.surf_flux_phys);
        acc(&mut self.balance_residual, g.balance_residual);
        acc(&mut self.ch4_clip_credit, g.ch4_clip_credit);
        acc(&mut self.o2_cap_loss, g.o2_cap_loss);
        acc(&mut self.o2_cap_gain, g.o2_cap_gain);
        acc(&mut self.prod_tot, g.prod_tot);
        acc(&mut self.oxid_tot, g.oxid_tot);
        acc(&mut self.totcol, patch.totcol_methane);
        let stepped = if patch.last.is_some() {
            g.surf_flux
        } else {
            0.0
        };
        // 土壤 patch 按 `aggregate_methane_columns` 分到土壤/水稻两类：`ws·flux` 与 `wr·flux`。
        let wr = patch.rice_fraction_prev;
        let lake = patch_type == 4;
        let (wetland, soil, rice) = if patch_type == 2 || lake {
            (if lake { 0.0 } else { stepped }, 0.0, 0.0)
        } else {
            (0.0, (1.0 - wr) * stepped, wr * stepped)
        };
        // 湖（只在 `allowlakeprod` 下跑过）：`methane_surf_flux_lake = methane_surf_flux_tot_lake`。
        let lake_flux = if lake { r.surf_flux_tot_lake } else { 0.0 };
        acc(&mut self.surf_flux_wetland, wetland);
        acc(&mut self.surf_flux_soil, soil);
        acc(&mut self.surf_flux_lake, lake_flux);
        acc(&mut self.surf_flux_rice, rice);
        acc(&mut self.surf_flux_tot_lake, lake_flux);
        let valid = |x: f64| !x.is_nan() && x.abs() < 0.5 * SPVAL.abs();
        if active && valid(patch.totcol_methane) {
            self.acc_num += 1.0;
        }
        // 湖的计数掩码只认开了 `allowlakeprod` 的湖 patch（跑过甲烷的湖），按湖沉积层的列总量计数。
        if lake && patch.last.is_some() && valid(patch.lake.totcol) {
            self.acc_num_lake += 1.0;
        }
    }

    /// 写出的 18 个 `core` 变量：`(名字, long_name, units, 值)`。单点 patch 维的值；
    /// `active` 是 `filter`（活跃甲烷 patch），`land` 是 `filter_all_land`（`patchtype < 99`）。
    /// `lake` 是开了 `allowlakeprod` 的湖 patch：它的地面合计量与"含湖"全局量按 `acc_num_lake` 平均。
    pub fn core_values(
        &self,
        active: bool,
        land: bool,
        lake: bool,
    ) -> Vec<(&'static str, &'static str, &'static str, f64)> {
        let per = |sum: f64, num: f64| {
            if sum != SPVAL && num > 0.0 {
                sum / num
            } else {
                SPVAL
            }
        };
        let with_filter = |value: f64, ok: bool| if ok { value } else { SPVAL };
        // `hist_ch4_*`：活跃 patch 且有计数时为 `a/acc_num`，否则 0；再以 `acc_num = 1` 写出。
        let derived = |sum: f64| {
            let value = if active && self.acc_num > 0.0 {
                sum / self.acc_num
            } else {
                0.0
            };
            with_filter(value / 1.0, land)
        };
        // 湖（`MOD_Tracer_Reactive_Methane_Hist.F90`）：`patchtype == 4 .and. allowlakeprod` 且有湖计数时
        // 用 `acc_num_lake` 覆盖；湿地/土壤/水稻三类保持 0。
        let lake_or = |lake_sum: f64, derived_value: f64| {
            if lake && land && self.acc_num_lake > 0.0 {
                lake_sum / self.acc_num_lake
            } else {
                derived_value
            }
        };
        let mean = |sum: f64| with_filter(per(sum, self.acc_num), active);
        vec![
            (
                "f_methane_surf_flux_tot_active",
                "corrected CH4 total surface flux; active-CH4-area mean; lake excluded",
                "mol/m2/s",
                mean(self.surf_flux_tot),
            ),
            (
                "f_methane_surf_flux_tot_phys",
                "CH4 physical total surface flux before clip/residual; active-CH4-area mean; lake excluded",
                "mol/m2/s",
                mean(self.surf_flux_tot_phys),
            ),
            (
                "f_methane_balance_residual",
                "CH4 numerical closure flux; active-CH4-area mean; lake excluded",
                "mol/m2/s",
                mean(self.balance_residual),
            ),
            (
                "f_methane_ch4_clip_credit",
                "CH4 negative-concentration clip credit; active-CH4-area mean; lake excluded",
                "mol/m2/s",
                mean(self.ch4_clip_credit),
            ),
            (
                "f_o2_cap_loss",
                "O2 column sink from post-solve physical concentration cap",
                "mol/m2/s",
                mean(self.o2_cap_loss),
            ),
            (
                "f_o2_cap_gain",
                "O2 column source from post-solve nonnegative concentration floor",
                "mol/m2/s",
                mean(self.o2_cap_gain),
            ),
            ("f_methane_prod_tot", "CH4 column production", "mol/m2/s", mean(self.prod_tot)),
            ("f_methane_oxid_tot", "CH4 column oxidation", "mol/m2/s", mean(self.oxid_tot)),
            ("f_totcol_methane", "total CH4 in soil column", "mol/m2", mean(self.totcol)),
            (
                "f_methane_surf_flux_tot",
                "CH4 total surface flux including wetland/soil/rice/lake; land-area mean",
                "mol/m2/s",
                lake_or(self.surf_flux_tot_lake, derived(self.surf_flux_tot)),
            ),
            (
                "f_methane_surf_flux_wetland",
                "wetland contribution to CH4 surface flux; land-area mean; multiply by landarea for global total",
                "mol/m2/s",
                derived(self.surf_flux_wetland),
            ),
            (
                "f_methane_surf_flux_soil",
                "non-rice soil contribution to CH4 surface flux; land-area mean; multiply by landarea for global total",
                "mol/m2/s",
                derived(self.surf_flux_soil),
            ),
            (
                "f_methane_surf_flux_lake",
                "lake contribution to CH4 surface flux; land-area mean; multiply by landarea for global total",
                "mol/m2/s",
                lake_or(self.surf_flux_lake, derived(self.surf_flux_lake)),
            ),
            (
                "f_methane_surf_flux_rice",
                "rice-paddy contribution to CH4 surface flux; land-area mean; multiply by landarea for global total",
                "mol/m2/s",
                derived(self.surf_flux_rice),
            ),
            (
                "f_methane_surf_flux_global_total_with_lake",
                "global CH4 total surface flux including lake; land-area mean contribution",
                "mol/m2/s",
                lake_or(self.surf_flux_tot_lake, derived(self.surf_flux_tot)),
            ),
            (
                "f_methane_surf_flux_global_phys_with_lake",
                "physical CH4 surface flux including lake; land-area mean contribution",
                "mol/m2/s",
                lake_or(self.surf_flux_tot_phys, derived(self.surf_flux_tot_phys)),
            ),
            (
                "f_methane_balance_residual_global_with_lake",
                "CH4 column balance residual including lake; land-area mean contribution",
                "mol/m2/s",
                lake_or(self.balance_residual, derived(self.balance_residual)),
            ),
            (
                "f_methane_ch4_clip_credit_global_with_lake",
                "CH4 nonnegative-storage correction including lake; land-area mean contribution",
                "mol/m2/s",
                lake_or(self.ch4_clip_credit, derived(self.ch4_clip_credit)),
            ),
        ]
    }
}

/// `repartition_methane_column_state`：稻田份额从 `old` 变到 `new` 时，在两个分量之间按面积守恒
/// 重分配两相浓度（新增的那部分取另一分量的面积加权平均）与各标量记忆。
///
/// GIMPLE（`repartition_phase_state`/`repartition_scalar`）：内层 `FMA(sat, h, (1-h)·unsat)`，
/// 外层 `FMA(inner_old, old, delta·inner_other)/new`（反向是 `FMA(1-old, inner, delta·other)/(1-new)`）。
/// 动态湖的干湖子步（`ch4_impl_lake_step` 的 `wdsrf < 100 .or. zwt > 0` 支 →
/// `handle_methane_dry_lake_substep`，`MOD_Tracer_Reactive_Methane_State.F90:1071-1131`）。
///
/// - 先清过程诊断（`reset_methane_inactive_lake_diagnostics`）：各速率与通量归 0，三个地表导度回到
///   默认值，应激因子归 0；浓度与沉积层存量是预报量，不动。
/// - 湖水与湖冰里的 CH4/O2 存量一次性导出：CH4 作为本子步的扩散通量（地表、物理、湖泊三套通量都取它），
///   O2 进 `lake_air_o2_flux`（Rust 不单存它）。
/// - 列总量只剩沉积层：`totcol = totcol_sat = totcol_lake`，`totcol_unsat = 0`。
/// - 冷启动标记置 `spval`，复湿时水柱按冷启动重建。
pub fn dry_lake_substep(params: &MethaneParameters, patch: &mut MethanePatch, substep_dt: f64) {
    let default_cond = params.methane.grnd_methane_cond_default;
    let previous = patch.last;
    let mut result = ColumnResult::default();
    // 浓度是预报量：沿用上一步聚合出来的；冷启动尚未走步时是分配时的值。
    match previous {
        Some(r) => {
            result.conc_o2 = r.conc_o2;
            result.conc_methane = r.conc_methane;
            result.c_atm = r.c_atm;
            result.forc_pmethanem = r.forc_pmethanem;
        }
        None => {
            result.conc_o2 = [1.0; NL_SOIL];
            result.conc_methane = [1.0e-6; NL_SOIL];
        }
    }
    for phase in [&mut result.merged, &mut result.unsat, &mut result.sat] {
        phase.o2stress = [0.0; NL_SOIL];
        phase.ch4stress = [0.0; NL_SOIL];
        phase.grnd_cond = default_cond;
    }
    patch.grnd_methane_cond = default_cond;
    patch.lake.grnd_cond = default_cond;
    patch.f_h2osfc = 0.0;
    patch.soil_zwt = SPVAL;
    if substep_dt > 0.0 {
        let lake = &mut patch.lake;
        let ch4 = lake.water.ch4.max(0.0) + lake.frozen_ch4.max(0.0);
        lake.water.ch4 = 0.0;
        lake.frozen_ch4 = 0.0;
        lake.water.o2 = 0.0;
        lake.frozen_o2 = 0.0;
        let totcol_lake = lake.totcol;
        result.merged.totcol = totcol_lake;
        result.sat.totcol = totcol_lake;
        result.unsat.totcol = 0.0;
        patch.totcol_methane = totcol_lake;
        if ch4 > 0.0 {
            let flux = ch4 / substep_dt;
            result.merged.surf_diff = flux;
            result.merged.surf_diff_phys = flux;
            result.merged.surf_flux = flux;
            result.surf_flux_phys = flux;
            result.surf_flux_tot_lake = flux;
        }
        patch.aggregate.fsat_bef = SPVAL;
        patch.aggregate.finundated_lag = SPVAL;
        patch.aggregate.layer_sat_lag = [SPVAL; NL_SOIL];
        patch.lake.liquid_fraction_prev = SPVAL;
    }
    patch.last = Some(result);
}

fn repartition(patch: &mut MethanePatch, old: f64, new: f64) {
    let rold = old.max(0.0).min(1.0);
    let rnew = new.max(0.0).min(1.0);
    if (rnew - rold).abs() <= 1.0e-14 {
        return;
    }
    let [soil, rice] = &mut patch.components;
    let fraction = |x: f64| {
        if x.is_nan() || x.abs() >= 0.5 * SPVAL.abs() || !(0.0..=1.0).contains(&x) {
            0.5
        } else {
            x
        }
    };
    let hs = fraction(soil.fsat_bef);
    let hr = fraction(rice.fsat_bef);
    let phase = |su: &mut [f64; NL_SOIL],
                 ss: &mut [f64; NL_SOIL],
                 ru: &mut [f64; NL_SOIL],
                 rs: &mut [f64; NL_SOIL]| {
        for j in 0..NL_SOIL {
            let s_mix = |a: f64, b: f64| b.contract(hs, (1.0 - hs) * a);
            let r_mix = |a: f64, b: f64| b.contract(hr, (1.0 - hr) * a);
            if rnew > rold {
                let delta = rnew - rold;
                let mixed = r_mix(ru[j], rs[j]).contract(rold, delta * s_mix(su[j], ss[j])) / rnew;
                ru[j] = mixed;
                rs[j] = mixed;
            } else {
                let delta = rold - rnew;
                let mixed = (1.0 - rold).contract(s_mix(su[j], ss[j]), delta * r_mix(ru[j], rs[j]))
                    / (1.0 - rnew);
                su[j] = mixed;
                ss[j] = mixed;
            }
        }
    };
    phase(
        &mut soil.conc_o2_unsat,
        &mut soil.conc_o2_sat,
        &mut rice.conc_o2_unsat,
        &mut rice.conc_o2_sat,
    );
    phase(
        &mut soil.conc_methane_unsat,
        &mut soil.conc_methane_sat,
        &mut rice.conc_methane_unsat,
        &mut rice.conc_methane_sat,
    );
    let scalar = |s: &mut f64, r: &mut f64| {
        let valid = |x: f64| !x.is_nan() && x.abs() < 0.5 * SPVAL.abs();
        if rnew > rold {
            if !valid(*s) {
                return;
            }
            if !valid(*r) || rold <= 1.0e-14 {
                *r = *s;
            } else {
                let delta = rnew - rold;
                *r = rold.contract(*r, *s * delta) / rnew;
            }
        } else {
            if !valid(*r) {
                return;
            }
            if !valid(*s) || rold >= 1.0 - 1.0e-14 {
                *s = *r;
            } else {
                let delta = rold - rnew;
                *s = s.contract(1.0 - rold, delta * *r) / (1.0 - rnew);
            }
        }
    };
    for j in 0..NL_SOIL {
        scalar(&mut soil.layer_sat_lag[j], &mut rice.layer_sat_lag[j]);
    }
    let (sa, ra) = (&mut soil.annual, &mut rice.annual);
    scalar(&mut sa.annavg_agnpp, &mut ra.annavg_agnpp);
    scalar(&mut sa.annavg_bgnpp, &mut ra.annavg_bgnpp);
    scalar(&mut sa.annavg_somhr, &mut ra.annavg_somhr);
    scalar(&mut sa.annavg_finrw, &mut ra.annavg_finrw);
    scalar(&mut sa.tempavg_agnpp, &mut ra.tempavg_agnpp);
    scalar(&mut sa.tempavg_bgnpp, &mut ra.tempavg_bgnpp);
    scalar(&mut sa.annsum_counter, &mut ra.annsum_counter);
    scalar(&mut sa.tempavg_somhr, &mut ra.tempavg_somhr);
    scalar(&mut sa.tempavg_finrw, &mut ra.tempavg_finrw);
    scalar(&mut soil.fsat_bef, &mut rice.fsat_bef);
    scalar(&mut soil.finundated_lag, &mut rice.finundated_lag);
}

/// `accumulate_methane_lake_substep_diagnostics`（`nsub > 1` 时）：湖在水体子步里跑甲烷，诊断量按
/// 时间加权平均（每个子步 `FMA(var, dt, acc)`，最后一个子步 `acc/(dt·nsub)` 写回）。这里只平均
/// 下游读得到的量：`core` history 用的通量与总量，以及写进续跑文件的三个地表导度。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LakeSubstepMean {
    surf_flux: f64,
    surf_flux_phys: f64,
    balance_residual: f64,
    ch4_clip_credit: f64,
    o2_cap_loss: f64,
    o2_cap_gain: f64,
    prod_tot: f64,
    oxid_tot: f64,
    surf_flux_tot_lake: f64,
    grnd_cond: f64,
    grnd_cond_sat: f64,
    grnd_cond_lake: f64,
}

impl LakeSubstepMean {
    /// 一个子步之后累加（`add1d`）。
    pub fn add(&mut self, patch: &MethanePatch, dt: f64) {
        let Some(r) = patch.last else { return };
        let g = &r.merged;
        for (acc, value) in [
            (&mut self.surf_flux, g.surf_flux),
            (&mut self.surf_flux_phys, r.surf_flux_phys),
            (&mut self.balance_residual, g.balance_residual),
            (&mut self.ch4_clip_credit, g.ch4_clip_credit),
            (&mut self.o2_cap_loss, g.o2_cap_loss),
            (&mut self.o2_cap_gain, g.o2_cap_gain),
            (&mut self.prod_tot, g.prod_tot),
            (&mut self.oxid_tot, g.oxid_tot),
            (&mut self.surf_flux_tot_lake, r.surf_flux_tot_lake),
            (&mut self.grnd_cond, patch.grnd_methane_cond),
            (&mut self.grnd_cond_sat, r.sat.grnd_cond),
            (&mut self.grnd_cond_lake, patch.lake.grnd_cond),
        ] {
            *acc = value.contract(dt, *acc);
        }
    }

    /// 最后一个子步写回时间平均（`finish1d`）。
    pub fn finish(&self, patch: &mut MethanePatch, total_dt: f64) {
        if total_dt <= 0.0 {
            return;
        }
        let Some(r) = patch.last.as_mut() else { return };
        r.merged.surf_flux = self.surf_flux / total_dt;
        r.surf_flux_phys = self.surf_flux_phys / total_dt;
        r.merged.balance_residual = self.balance_residual / total_dt;
        r.merged.ch4_clip_credit = self.ch4_clip_credit / total_dt;
        r.merged.o2_cap_loss = self.o2_cap_loss / total_dt;
        r.merged.o2_cap_gain = self.o2_cap_gain / total_dt;
        r.merged.prod_tot = self.prod_tot / total_dt;
        r.merged.oxid_tot = self.oxid_tot / total_dt;
        r.surf_flux_tot_lake = self.surf_flux_tot_lake / total_dt;
        patch.grnd_methane_cond = self.grnd_cond / total_dt;
        r.merged.grnd_cond = patch.grnd_methane_cond;
        r.sat.grnd_cond = self.grnd_cond_sat / total_dt;
        patch.lake.grnd_cond = self.grnd_cond_lake / total_dt;
    }
}
