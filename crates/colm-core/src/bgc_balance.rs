//! `MOD_BGC_CNBalanceCheck.F90`：步首记下 C/N 总量，步末检查收支。
//!
//! **生成文件，勿手改**：由 `oracle/scripts/bgc_port/regen.py` 从上游 Fortran 与其 GIMPLE 转写
//! （`a ± b·c` 按 GCC 的规则收缩成 FMA，乘积被 CSE 共享到别的基本块的行不收缩），再由逐过程回放
//! 与 Fortran 追踪逐位核对。`DEF_USE_SASU`/`DiagMatrix`、作物分支照抄，尚无回放覆盖。

// 逐层循环的 `j` 同时索引若干按列主序展平的数组，保留下标写法以便与上游逐行对照。
#![allow(clippy::needless_range_loop)]
// 嵌套 IF 与"先声明、分支里赋值"都按上游结构保留，便于逐行对照。
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::needless_late_init
)]
// `a >= lo .and. a <= hi`、`max(lo, min(hi, x))` 照抄：改成 `contains`/`clamp` 会改变 NaN 的行为。
#![allow(clippy::manual_range_contains, clippy::manual_clamp)]

use crate::bgc_driver::{BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;

/// `BeginCNBalance`：记下步首总量。
pub fn begin_cn_balance(s: &mut BgcState, _p: &BgcPhysics, _c: &BgcPftConstants, _sw: BgcSwitches) {
    s.patch.col_begcb[0] = s.patch.totcolc[0];
    s.patch.col_begnb[0] = s.patch.totcoln[0];
    s.patch.col_vegbegcb[0] = s.patch.totvegc[0] + s.patch.ctrunc_veg[0];
    s.patch.col_vegbegnb[0] = s.patch.totvegn[0] + s.patch.ntrunc_veg[0];
    s.patch.col_soilbegcb[0] =
        s.patch.totsomc[0] + s.patch.totlitc[0] + s.patch.totcwdc[0] + s.patch.ctrunc_soil[0];
    s.patch.col_soilbegnb[0] =
        s.patch.totsomn[0] + s.patch.totlitn[0] + s.patch.totcwdn[0] + s.patch.ntrunc_soil[0];
    s.patch.col_sminnbegnb[0] = s.patch.sminn[0];
}

/// `CBalanceCheck`：C 收支检查（失败时上游 abort，这里返回错误）。
pub fn c_balance_check(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    _sw: BgcSwitches,
) -> anyhow::Result<()> {
    let cerror: f64 = 1.0e-7;
    s.patch.col_endcb[0] = s.patch.totcolc[0];
    s.patch.col_vegendcb[0] = s.patch.totvegc[0] + s.patch.ctrunc_veg[0];
    s.patch.col_soilendcb[0] =
        s.patch.totsomc[0] + s.patch.totlitc[0] + s.patch.totcwdc[0] + s.patch.ctrunc_soil[0];
    let col_cinputs: f64 = s.patch_flux.gpp[0];
    let col_coutputs: f64 = s.patch_flux.er[0]
        + s.patch_flux.fire_closs[0]
        + s.patch_flux.hrv_xsmrpool_to_atm[0]
        + s.patch_flux.wood_harvestc[0]
        + s.patch_flux.grainc_to_cropprodc[0]
        - s.patch_flux.som_c_leached[0];
    let col_errcb: f64 = (col_cinputs - col_coutputs)
        .mul_add(p.deltim, -(s.patch.col_endcb[0] - s.patch.col_begcb[0]));
    if col_errcb.abs() > cerror {
        // write(*,*)'column cbalance error    = ', col_errcb, i, p_iam_glb
        // write(*,*)'Latdeg,Londeg='             , dlat, dlon
        // write(*,*)'begcb                    = ',col_begcb(i)
        // write(*,*)'endcb                    = ',col_endcb(i)
        // write(*,*)'delta store              = ',col_endcb(i)-col_begcb(i)
        // write(*,*)'delta veg                = ',col_vegendcb(i) - col_vegbegcb(i),totvegc(i),col_vegendcb(i),col_vegbegcb(i)
        // write(*,*)'delta soil               = ',col_soilendcb(i) - col_soilbegcb(i),totsomc(i),totlitc(i),totcwdc(i),col_soilendcb(i),col_soilbegcb(i)
        // write(*,*)'m=',m,pftclass(m)
        // write(*,*)'vegc,leafc              = ',leafc_p(m)+leafc_storage_p(m)+leafc_xfer_p(m)
        // write(*,*)'vegc,frootc             = ',frootc_p(m)+frootc_storage_p(m)+frootc_xfer_p(m)
        // write(*,*)'vegc,livestemc          = ',livestemc_p(m)+livestemc_storage_p(m)+livestemc_xfer_p(m)
        // write(*,*)'vegc,deadstemc          = ',deadstemc_p(m)+deadstemc_storage_p(m)+deadstemc_xfer_p(m)
        // write(*,*)'vegc,livecrootc         = ',livecrootc_p(m)+livecrootc_storage_p(m)+livecrootc_xfer_p(m)
        // write(*,*)'vegc,deadcrootc         = ',deadcrootc_p(m)+deadcrootc_storage_p(m)+deadcrootc_xfer_p(m)
        // write(*,*)'grainc                  = ',grainc_p(m)+grainc_storage_p(m)+grainc_xfer_p(m)+cropseedc_deficit_p(m)
        // write(*,*)'growth respiration c    = ',gresp_storage_p(m)+gresp_xfer_p(m)+xsmrpool_p(m)
        // write(*,*)'--------veg output to litter-------------'
        // write(*,*)'veg to soil and litter   = ', veg_to_litter, phen_to_litter, gap_leaf_to_litter, gap_froot_to_litter, gap_livestem_to_litter,  gap_deadstem_to_litter, gap_livecroot_to_litter, gap_deadcroot_to_litter , gap_gresp_to_litter
        // write(*,*)'--------liter and soil input from veg----'
        // write(*,*)'input to soil and litter = ',gap_veg_to_litter + phen_veg_to_litter
        // write(*,*)'phen, gap to litter      = ',phen_veg_to_litter, gap_veg_to_litter
        // write(*,*)'--- Inputs ---'
        // write(*,*)'gpp                      = ',gpp(i)*deltim
        // write(*,*)'--- Outputs ---'
        // write(*,*)'er                       = ',er(i)*deltim
        // write(*,*)'ar                       = ',ar(i)*deltim
        // write(*,*)'decomp_hr                = ',decomp_hr(i)*deltim
        // write(*,*)'fire_closs               = ',fire_closs(i)*deltim
        // write(*,*)'col_hrv_xsmrpool_to_atm  = ',hrv_xsmrpool_to_atm(i)*deltim
        // write(*,*)'wood_harvestc            = ',wood_harvestc(i)*deltim
        // write(*,*)'grainc_to_cropprodc      = ',grainc_to_cropprodc(i)*deltim, grainc_to_food_p(ps)*deltim
        // write(*,*)'-1*som_c_leached         = ',som_c_leached(i)*deltim
        // `#ifdef USEMPI` 分支：单点 Rust 引擎里该宏未定义。
        {
            anyhow::bail!("CBalanceCheck：上游在此 abort（收支/廓线检查失败）");
        }
    }
    Ok(())
}

/// `NBalanceCheck`：N 收支检查（失败时上游 abort，这里返回错误）。
pub fn n_balance_check(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) -> anyhow::Result<()> {
    let nerror: f64 = 1.0e-7;
    let mut col_ninputs: f64;
    let mut col_noutputs: f64;
    s.patch.col_endnb[0] = s.patch.totcoln[0];
    s.patch.col_vegendnb[0] = s.patch.totvegn[0] + s.patch.ntrunc_veg[0];
    s.patch.col_soilendnb[0] =
        s.patch.totsomn[0] + s.patch.totlitn[0] + s.patch.totcwdn[0] + s.patch.ntrunc_soil[0];
    s.patch.col_sminnendnb[0] = s.patch.sminn[0];
    col_ninputs = s.patch_flux.ndep_to_sminn[0]
        + s.patch_flux.nfix_to_sminn[0]
        + s.patch_flux.supplement_to_sminn[0];
    col_ninputs = col_ninputs + s.patch_flux.fert_to_sminn[0] + s.patch_flux.soyfixn_to_sminn[0];
    col_noutputs = s.patch_flux.denit[0]
        + s.patch_flux.fire_nloss[0]
        + s.patch_flux.wood_harvestn[0]
        + s.patch_flux.grainn_to_cropprodn[0];
    if sw.nitrif {
        col_noutputs = col_noutputs
            + s.patch_flux.f_n2o_nit[0]
            + s.patch_flux.smin_no3_leached[0]
            + s.patch_flux.smin_no3_runoff[0];
    } else {
        col_noutputs += s.patch_flux.sminn_leached[0];
    }
    col_noutputs -= s.patch_flux.som_n_leached[0];
    let col_errnb: f64 =
        (col_ninputs - col_noutputs) * p.deltim - (s.patch.col_endnb[0] - s.patch.col_begnb[0]); // 无 FMA（上游第 236 行，乘积被 CSE 共享）
    if col_errnb.abs() > nerror {
        // write(*,*)'column nbalance error    = ',col_errnb, i, p_iam_glb
        // write(*,*)'Latdeg,Londeg            = ',dlat, dlon
        // write(*,*)'begnb                    = ',col_begnb(i)
        // write(*,*)'endnb                    = ',col_endnb(i)
        // write(*,*)'delta store              = ',col_endnb(i)-col_begnb(i)
        // write(*,*)'delta veg                = ',col_vegendnb(i)-col_vegbegnb(i)
        // write(*,*)'delta soil               = ',col_soilendnb(i)-col_soilbegnb(i)
        // write(*,*)'delta sminn              = ',col_sminnendnb(i)-col_sminnbegnb(i)
        // write(*,*)'smin_to_plant            = ',sminn_to_plant(i)*deltim
        // write(*,*)'input mass               = ',col_ninputs*deltim
        // write(*,*)'output mass              = ',col_noutputs*deltim,f_n2o_nit(i)*deltim,smin_no3_leached(i)*deltim, smin_no3_runoff(i)*deltim, denit(i)*deltim,fire_nloss(i)*deltim, ( wood_harvestn(i) + grainn_to_cropprodn(i))*deltim
        // write(*,*)'net flux                 = ',(col_ninputs-col_noutputs)*deltim
        // write(*,*)'inputs,ffix,nfix,ndep    = ',ffix_to_sminn(i)*deltim,nfix_to_sminn(i)*deltim,ndep_to_sminn(i)*deltim, fert_to_sminn(i)*deltim,soyfixn_to_sminn(i)*deltim
        if sw.nitrif {
            // write(*,*)'outputs,leached,runoff,denit = ',smin_no3_leached(i)*deltim, smin_no3_runoff(i)*deltim,f_n2o_nit(i)*deltim
        } else {
            // write(*,*)'outputs,leached,denit,fire,harvest,som_n_leached', sminn_leached(i)*deltim,denit(i)*deltim,fire_nloss(i)*deltim, (wood_harvestn(i)+grainn_to_cropprodn(i))*deltim, - som_n_leached(i)
        }
        // `#ifdef USEMPI` 分支：单点 Rust 引擎里该宏未定义。
        {
            anyhow::bail!("NBalanceCheck：上游在此 abort（收支/廓线检查失败）");
        }
    }
    Ok(())
}
