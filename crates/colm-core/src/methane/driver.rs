//! `methane_driver` 的土壤 patch 路径（`MOD_Tracer_Reactive_Methane_Driver` +
//! `MOD_Tracer_Reactive_Methane_Impl:ch4_impl_soil_step`），以及 `core` history 累加。
//!
//! 土壤 patch 的水稻比例为 0：只跑土壤分量，两分量合并（`aggregate_methane_columns`）在
//! `ws = 1, wr = 0` 下对每个量都是恒等，所以直接取土壤分量的结果。水稻、湿地、湖未移植。

use anyhow::{ensure, Result};

use super::bgc_link::{self, BgcInputs};
use super::column::{self, ColumnInput, ColumnResult, ComponentState};
use super::config::{MethaneParameters, COMP_SOIL};
use super::physics::{self, sn, MAXSNL, NL_SOIL, SOISNO, SPVAL};
use crate::bgc_state::BgcState;

/// 一个 patch 的甲烷状态（续跑量 + 最近一步的诊断）。
#[derive(Debug, Clone, PartialEq)]
pub struct MethanePatch {
    pub components: [ComponentState; 2],
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
}

impl MethanePatch {
    /// `allocate_methane_state` 的冷启动值。
    pub fn cold(params: &MethaneParameters) -> Self {
        Self {
            components: [ComponentState::cold(), ComponentState::cold()],
            rice_fraction_prev: 0.0,
            f_h2osfc: 0.0,
            lake_soilc: [0.0; NL_SOIL],
            totcol_methane: 0.0,
            grnd_methane_cond: params.methane.grnd_methane_cond_default,
            last: None,
            soil_zwt: SPVAL,
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
    pub pftfrac: &'a [f64],
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
        site.patchtype == 0,
        "only soil-patch methane is ported to the Rust runtime yet"
    );
    ensure!(
        !m.enable_rice_paddy,
        "rice-paddy methane is not ported to the Rust runtime yet"
    );
    let run = if m.only_wetland {
        site.patchtype == 2
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
    let inputs: BgcInputs =
        bgc_link::patch_inputs(m, bgc, &site.rootfr, host.pftfrac, &site.dz_soi)?;
    let biome_f = bgc_link::biome_f_methane(m, site.patchtype, site.dlat, inputs.cellorg[0]);
    let biome_redox = bgc_link::biome_redoxlag(m, site.patchtype, site.dlat, inputs.cellorg[0]);
    // 水稻比例 0：`repartition_methane_column_state(i, rice_fraction_prev, 0)` 在上一步也是 0 时什么都不做。
    ensure!(
        patch.rice_fraction_prev.abs() <= 1.0e-14,
        "a methane patch with a rice fraction is not ported to the Rust runtime yet"
    );
    let comp = &mut patch.components[COMP_SOIL];
    // `run_methane_component`：列总量由本分量的两相浓度与 `fsat_bef` 拼出（只给收支检查用）。
    let fsat = comp.fsat_bef;
    let col_sum = |x: &[f64; NL_SOIL]| {
        (0..NL_SOIL).fold(0.0f64, |acc, k| x[k].mul_add(dz[sn(k as i32 + 1)], acc))
    };
    let totcol_unsat = col_sum(&comp.conc_methane_unsat);
    let totcol_sat = col_sum(&comp.conc_methane_sat);
    let totcol_before = if (0.0..=1.0).contains(&fsat) {
        fsat.mul_add(totcol_sat, (1.0 - fsat) * totcol_unsat)
    } else {
        0.5 * (totcol_sat + totcol_unsat)
    };
    let result = column::methane(
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
            rootfr: &site.rootfr,
            snowdp: host.snowdp,
            etr: host.etr,
            wdsrf: host.wdsrf,
            wetwat: host.wetwat,
            bsw: &site.bsw,
            smp: host.smp,
            porsl: &site.porsl,
            lai: host.lai,
            sai: host.sai,
            rootr: host.rootr,
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
            biome_f_methane: Some(biome_f),
            biome_redoxlag: Some(biome_redox),
            aere_override: None,
            totcol_before,
            lake_soilc: &patch.lake_soilc,
        },
        comp,
    )?;
    bgc_link::finalize(bgc, site.patchtype, result.merged.net, &site.dz_soi)?;
    patch.totcol_methane = result.merged.totcol;
    patch.grnd_methane_cond = result.merged.grnd_cond;
    patch.soil_zwt = host.zwt;
    patch.rice_fraction_prev = 0.0;
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
    /// 一步的累加。`active` 是 `methane_active_mask`（土壤/湿地 patch）。没跑过甲烷的 patch
    /// 用冷启动时的状态值（`allocate_methane_state` 的 0 与默认值）。
    pub fn accumulate(&mut self, patch: &MethanePatch, active: bool) {
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
        acc(&mut self.surf_flux_wetland, 0.0);
        acc(
            &mut self.surf_flux_soil,
            if patch.last.is_some() {
                g.surf_flux
            } else {
                0.0
            },
        );
        acc(&mut self.surf_flux_lake, 0.0);
        acc(&mut self.surf_flux_rice, 0.0);
        acc(&mut self.surf_flux_tot_lake, 0.0);
        let valid = |x: f64| !x.is_nan() && x.abs() < 0.5 * SPVAL.abs();
        if active && valid(patch.totcol_methane) {
            self.acc_num += 1.0;
        }
        // 湖的计数掩码只认开了 `allowlakeprod` 的湖 patch；土壤 patch 从不计数。
    }

    /// 写出的 18 个 `core` 变量：`(名字, long_name, units, 值)`。单点 patch 维的值；
    /// `active` 是 `filter`（活跃甲烷 patch），`land` 是 `filter_all_land`（`patchtype < 99`）。
    pub fn core_values(
        &self,
        active: bool,
        land: bool,
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
                derived(self.surf_flux_tot),
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
                derived(self.surf_flux_lake),
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
                derived(self.surf_flux_tot),
            ),
            (
                "f_methane_surf_flux_global_phys_with_lake",
                "physical CH4 surface flux including lake; land-area mean contribution",
                "mol/m2/s",
                derived(self.surf_flux_tot_phys),
            ),
            (
                "f_methane_balance_residual_global_with_lake",
                "CH4 column balance residual including lake; land-area mean contribution",
                "mol/m2/s",
                derived(self.balance_residual),
            ),
            (
                "f_methane_ch4_clip_credit_global_with_lake",
                "CH4 nonnegative-storage correction including lake; land-area mean contribution",
                "mol/m2/s",
                derived(self.ch4_clip_credit),
            ),
        ]
    }
}
