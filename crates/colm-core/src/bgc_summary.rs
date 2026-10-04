//! `MOD_BGC_CNSummary.F90`：C/N 池与通量的逐层积分与 patch 汇总。
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
// `x = x * y` 照上游一条赋值写（与 `*=` 舍入相同，保留以便对照）。
#![allow(clippy::assign_op_pattern)]

use crate::bgc_driver::{vectorized_dot, BgcPftConstants, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;
use crate::MISSING;

/// `CNDriverSummarizeStates`：状态量汇总。
pub fn cn_driver_summarize_states(
    s: &mut BgcState,
    p: &mut BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
    init: bool,
) {
    soilbiogeochem_carbonstate_summary(s, p, c, sw);
    soilbiogeochem_nitrogenstate_summary(s, p, c, sw);
    cnveg_carbonstate_summary(s, p, c, sw, init);
    cnveg_nitrogenstate_summary(s, p, c, sw);
}

/// `CNDriverSummarizeFluxes`：通量汇总。
pub fn cn_driver_summarize_fluxes(
    s: &mut BgcState,
    p: &BgcPhysics,
    c: &BgcPftConstants,
    sw: BgcSwitches,
) -> anyhow::Result<()> {
    soilbiogeochem_carbonflux_summary(s, p, c, sw);
    soilbiogeochem_nitrogenflux_summary(s, p, c, sw);
    cnveg_carbonflux_summary(s, p, c, sw)?;
    cnveg_nitrogenflux_summary(s, p, c, sw)?;
    Ok(())
}

/// 土壤 C 池汇总（湿地 CH4 的 `CNDriverSummarizeNonvegetatedSoilStates` 也用）。
pub fn soilbiogeochem_carbonstate_summary(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    _sw: BgcSwitches,
) {
    let d = s.dims;
    s.patch.totsomc[0] = 0.0;
    s.patch.totlitc[0] = 0.0;
    s.patch.totcwdc[0] = 0.0;
    s.patch.ctrunc_soil[0] = 0.0;
    for l in 0..d.ndecomp_pools {
        s.patch.decomp_cpools[l] = 0.0;
        for j in 0..d.nl_soil {
            s.patch.decomp_cpools[l] = s.patch.decomp_cpools_vr[j + d.nl_soil_full * l]
                .mul_add(p.dz_soi[j], s.patch.decomp_cpools[l]);
        }
    }
    for l in 0..d.ndecomp_pools {
        if s.invariants.is_litter[l] {
            s.patch.totlitc[0] += s.patch.decomp_cpools[l];
        }
        if s.invariants.is_soil[l] {
            s.patch.totsomc[0] += s.patch.decomp_cpools[l];
        }
        if s.invariants.is_cwd[l] {
            s.patch.totcwdc[0] += s.patch.decomp_cpools[l];
        }
    }
    for j in 0..d.nl_soil {
        s.patch.ctrunc_soil[0] = s.patch.ctrunc_vr[j].mul_add(p.dz_soi[j], s.patch.ctrunc_soil[0]);
    }
}

/// 土壤 N 池汇总（湿地 CH4 的 `CNDriverSummarizeNonvegetatedSoilStates` 也用）。
pub fn soilbiogeochem_nitrogenstate_summary(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    _sw: BgcSwitches,
) {
    let d = s.dims;
    s.patch.totsomn[0] = 0.0;
    s.patch.totlitn[0] = 0.0;
    s.patch.totcwdn[0] = 0.0;
    s.patch.sminn[0] = 0.0;
    s.patch.ntrunc_soil[0] = 0.0;
    for j in 0..d.nl_soil {
        s.patch.totsoiln_vr[j] = 0.0;
    }
    for l in 0..d.ndecomp_pools {
        s.patch.decomp_npools[l] = 0.0;
        for j in 0..d.nl_soil {
            s.patch.decomp_npools[l] = s.patch.decomp_npools_vr[j + d.nl_soil_full * l]
                .mul_add(p.dz_soi[j], s.patch.decomp_npools[l]);
            s.patch.totsoiln_vr[j] = (s.patch.decomp_npools_vr[j + d.nl_soil_full * l]
                / (p.BD_all[j] * 1000.0))
                .mul_add(100.0, s.patch.totsoiln_vr[j]);
        }
    }
    for j in 0..d.nl_soil {
        s.patch.sminn[0] = s.patch.sminn_vr[j].mul_add(p.dz_soi[j], s.patch.sminn[0]);
        s.patch.totsoiln_vr[j] =
            (s.patch.sminn_vr[j] / (p.BD_all[j] * 1000.0)).mul_add(100.0, s.patch.totsoiln_vr[j]);
    }
    for l in 0..d.ndecomp_pools {
        if s.invariants.is_litter[l] {
            s.patch.totlitn[0] += s.patch.decomp_npools[l];
        }
        if s.invariants.is_soil[l] {
            s.patch.totsomn[0] += s.patch.decomp_npools[l];
        }
        if s.invariants.is_cwd[l] {
            s.patch.totcwdn[0] += s.patch.decomp_npools[l];
        }
    }
    for j in 0..d.nl_soil {
        s.patch.ntrunc_soil[0] = s.patch.ntrunc_vr[j].mul_add(p.dz_soi[j], s.patch.ntrunc_soil[0]);
    }
}

/// 植被 C 池汇总。
fn cnveg_carbonstate_summary(
    s: &mut BgcState,
    p: &mut BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
    init: bool,
) {
    let npft = p.pftclass.len();
    s.patch.leafc[0] = (0..npft).fold(0.0, |acc, m| s.pft.leafc_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.leafc_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.leafc_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.leafc_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.leafc_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.frootc[0] = (0..npft).fold(0.0, |acc, m| s.pft.frootc_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.frootc_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.frootc_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.frootc_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.frootc_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livestemc[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livestemc_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livestemc_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livestemc_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livestemc_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livestemc_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadstemc[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadstemc_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadstemc_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadstemc_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadstemc_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadstemc_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livecrootc[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livecrootc_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livecrootc_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livecrootc_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livecrootc_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livecrootc_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadcrootc[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadcrootc_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadcrootc_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadcrootc_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadcrootc_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadcrootc_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.xsmrpool[0] =
        (0..npft).fold(0.0, |acc, m| s.pft.xsmrpool_p[m].mul_add(p.pftfrac[m], acc));
    if sw.crop {
        s.patch.grainc[0] =
            (0..npft).fold(0.0, |acc, m| s.pft.grainc_p[m].mul_add(p.pftfrac[m], acc));
        s.patch.grainc_storage[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.grainc_storage_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.grainc_xfer[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.grainc_xfer_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.cropseedc_deficit[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.cropseedc_deficit_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.cropprod1c[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.cropprod1c_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.cphase[0] =
            (0..npft).fold(0.0, |acc, m| s.pft.cphase_p[m].mul_add(p.pftfrac[m], acc));
        s.patch.hui[0] = s.pft.hui_p[0];
        s.patch.gddplant[0] =
            (0..npft).fold(0.0, |acc, m| s.pft.gddplant_p[m].mul_add(p.pftfrac[m], acc));
        if (0..npft).any(|m| s.pft.gddmaturity_p[m] != MISSING) {
            s.patch.gddmaturity[0] = (0..npft).fold(0.0, |acc, m| {
                if s.pft.gddmaturity_p[m] != MISSING {
                    s.pft.gddmaturity_p[m].mul_add(p.pftfrac[m], acc)
                } else {
                    acc
                }
            });
        } else {
            s.patch.gddmaturity[0] = MISSING;
        }
        s.patch.vf[0] = (0..npft).fold(0.0, |acc, m| s.pft.vf_p[m].mul_add(p.pftfrac[m], acc));
        s.patch.fertnitro_corn[0] = 0.0;
        s.patch.fertnitro_swheat[0] = 0.0;
        s.patch.fertnitro_wwheat[0] = 0.0;
        s.patch.fertnitro_soybean[0] = 0.0;
        s.patch.fertnitro_cotton[0] = 0.0;
        s.patch.fertnitro_rice1[0] = 0.0;
        s.patch.fertnitro_rice2[0] = 0.0;
        s.patch.fertnitro_sugarcane[0] = 0.0;
        s.patch.manunitro[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.manunitro_p[m].mul_add(p.pftfrac[m], acc)
        });
    }
    if sw.diag_matrix {
        s.patch.leafcCap[0] =
            (0..npft).fold(0.0, |acc, m| s.pft.leafcCap_p[m].mul_add(p.pftfrac[m], acc));
        s.patch.leafc_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.leafc_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.leafc_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.leafc_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.frootcCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.frootcCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.frootc_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.frootc_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.frootc_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.frootc_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livestemcCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livestemcCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livestemc_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livestemc_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livestemc_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livestemc_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadstemcCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadstemcCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadstemc_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadstemc_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadstemc_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadstemc_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livecrootcCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livecrootcCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livecrootc_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livecrootc_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livecrootc_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livecrootc_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadcrootcCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadcrootcCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadcrootc_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadcrootc_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadcrootc_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadcrootc_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
    }
    for m in 0..npft {
        if sw.crop {
            s.pft.totvegc_p[m] = s.pft.leafc_p[m]
                + s.pft.frootc_p[m]
                + s.pft.livestemc_p[m]
                + s.pft.deadstemc_p[m]
                + s.pft.livecrootc_p[m]
                + s.pft.deadcrootc_p[m]
                + s.pft.leafc_storage_p[m]
                + s.pft.frootc_storage_p[m]
                + s.pft.livestemc_storage_p[m]
                + s.pft.deadstemc_storage_p[m]
                + s.pft.livecrootc_storage_p[m]
                + s.pft.deadcrootc_storage_p[m]
                + s.pft.leafc_xfer_p[m]
                + s.pft.frootc_xfer_p[m]
                + s.pft.livestemc_xfer_p[m]
                + s.pft.deadstemc_xfer_p[m]
                + s.pft.livecrootc_xfer_p[m]
                + s.pft.deadcrootc_xfer_p[m]
                + s.pft.grainc_p[m]
                + s.pft.grainc_storage_p[m]
                + s.pft.grainc_xfer_p[m]
                + s.pft.cropseedc_deficit_p[m]
                + s.pft.gresp_storage_p[m]
                + s.pft.gresp_xfer_p[m]
                + s.pft.xsmrpool_p[m]
                + s.pft.cpool_p[m];
        } else {
            s.pft.totvegc_p[m] = s.pft.leafc_p[m]
                + s.pft.frootc_p[m]
                + s.pft.livestemc_p[m]
                + s.pft.deadstemc_p[m]
                + s.pft.livecrootc_p[m]
                + s.pft.deadcrootc_p[m]
                + s.pft.leafc_storage_p[m]
                + s.pft.frootc_storage_p[m]
                + s.pft.livestemc_storage_p[m]
                + s.pft.deadstemc_storage_p[m]
                + s.pft.livecrootc_storage_p[m]
                + s.pft.deadcrootc_storage_p[m]
                + s.pft.leafc_xfer_p[m]
                + s.pft.frootc_xfer_p[m]
                + s.pft.livestemc_xfer_p[m]
                + s.pft.deadstemc_xfer_p[m]
                + s.pft.livecrootc_xfer_p[m]
                + s.pft.deadcrootc_xfer_p[m]
                + s.pft.gresp_storage_p[m]
                + s.pft.gresp_xfer_p[m]
                + s.pft.xsmrpool_p[m]
                + s.pft.cpool_p[m];
        }
        if sw.crop {
            if p.pftclass[m] == 17
                || p.pftclass[m] == 18
                || p.pftclass[m] == 63
                || p.pftclass[m] == 64
            {
                s.patch.fertnitro_corn[0] = s.pft.fertnitro_p[m];
                p.irrig_method_corn[0] = p.irrig_method_p[m];
            } else if p.pftclass[m] == 19 || p.pftclass[m] == 20 {
                s.patch.fertnitro_swheat[0] = s.pft.fertnitro_p[m];
                p.irrig_method_swheat[0] = p.irrig_method_p[m];
            } else if p.pftclass[m] == 21 || p.pftclass[m] == 22 {
                s.patch.fertnitro_wwheat[0] = s.pft.fertnitro_p[m];
                p.irrig_method_wwheat[0] = p.irrig_method_p[m];
            } else if p.pftclass[m] == 23
                || p.pftclass[m] == 24
                || p.pftclass[m] == 77
                || p.pftclass[m] == 78
            {
                s.patch.fertnitro_soybean[0] = s.pft.fertnitro_p[m];
                p.irrig_method_soybean[0] = p.irrig_method_p[m];
            } else if p.pftclass[m] == 41 || p.pftclass[m] == 42 {
                s.patch.fertnitro_cotton[0] = s.pft.fertnitro_p[m];
                p.irrig_method_cotton[0] = p.irrig_method_p[m];
            } else if p.pftclass[m] == 61 || p.pftclass[m] == 62 {
                s.patch.fertnitro_rice1[0] = s.pft.fertnitro_p[m];
                s.patch.fertnitro_rice2[0] = s.pft.fertnitro_p[m];
                p.irrig_method_rice1[0] = p.irrig_method_p[m];
                p.irrig_method_rice2[0] = p.irrig_method_p[m];
            } else if p.pftclass[m] == 67 || p.pftclass[m] == 68 {
                s.patch.fertnitro_sugarcane[0] = s.pft.fertnitro_p[m];
                p.irrig_method_sugarcane[0] = p.irrig_method_p[m];
            }
        }
    }
    if !init {
        s.patch_flux.leafc_enftemp[0] = 0.0;
        s.patch_flux.leafc_enfboreal[0] = 0.0;
        s.patch_flux.leafc_dnfboreal[0] = 0.0;
        s.patch_flux.leafc_ebftrop[0] = 0.0;
        s.patch_flux.leafc_ebftemp[0] = 0.0;
        s.patch_flux.leafc_dbftrop[0] = 0.0;
        s.patch_flux.leafc_dbftemp[0] = 0.0;
        s.patch_flux.leafc_dbfboreal[0] = 0.0;
        s.patch_flux.leafc_ebstemp[0] = 0.0;
        s.patch_flux.leafc_dbstemp[0] = 0.0;
        s.patch_flux.leafc_dbsboreal[0] = 0.0;
        s.patch_flux.leafc_c3arcgrass[0] = 0.0;
        s.patch_flux.leafc_c3grass[0] = 0.0;
        s.patch_flux.leafc_c4grass[0] = 0.0;
        p.lai_enftemp[0] = 0.0;
        p.lai_enfboreal[0] = 0.0;
        p.lai_dnfboreal[0] = 0.0;
        p.lai_ebftrop[0] = 0.0;
        p.lai_ebftemp[0] = 0.0;
        p.lai_dbftrop[0] = 0.0;
        p.lai_dbftemp[0] = 0.0;
        p.lai_dbfboreal[0] = 0.0;
        p.lai_ebstemp[0] = 0.0;
        p.lai_dbstemp[0] = 0.0;
        p.lai_dbsboreal[0] = 0.0;
        p.lai_c3arcgrass[0] = 0.0;
        p.lai_c3grass[0] = 0.0;
        p.lai_c4grass[0] = 0.0;
        for m in 0..npft {
            if p.pftclass[m] == 1 {
                s.patch_flux.leafc_enftemp[0] = s.pft.leafc_p[m];
                p.lai_enftemp[0] = p.lai_p[m];
            } else if p.pftclass[m] == 2 {
                s.patch_flux.leafc_enfboreal[0] = s.pft.leafc_p[m];
                p.lai_enfboreal[0] = p.lai_p[m];
            } else if p.pftclass[m] == 3 {
                s.patch_flux.leafc_dnfboreal[0] = s.pft.leafc_p[m];
                p.lai_dnfboreal[0] = p.lai_p[m];
            } else if p.pftclass[m] == 4 {
                s.patch_flux.leafc_ebftrop[0] = s.pft.leafc_p[m];
                p.lai_ebftrop[0] = p.lai_p[m];
            } else if p.pftclass[m] == 5 {
                s.patch_flux.leafc_ebftemp[0] = s.pft.leafc_p[m];
                p.lai_ebftemp[0] = p.lai_p[m];
            } else if p.pftclass[m] == 6 {
                s.patch_flux.leafc_dbftrop[0] = s.pft.leafc_p[m];
                p.lai_dbftrop[0] = p.lai_p[m];
            } else if p.pftclass[m] == 7 {
                s.patch_flux.leafc_dbftemp[0] = s.pft.leafc_p[m];
                p.lai_dbftemp[0] = p.lai_p[m];
            } else if p.pftclass[m] == 8 {
                s.patch_flux.leafc_dbfboreal[0] = s.pft.leafc_p[m];
                p.lai_dbfboreal[0] = p.lai_p[m];
            } else if p.pftclass[m] == 9 {
                s.patch_flux.leafc_ebstemp[0] = s.pft.leafc_p[m];
                p.lai_ebstemp[0] = p.lai_p[m];
            } else if p.pftclass[m] == 10 {
                s.patch_flux.leafc_dbstemp[0] = s.pft.leafc_p[m];
                p.lai_dbstemp[0] = p.lai_p[m];
            } else if p.pftclass[m] == 11 {
                s.patch_flux.leafc_dbsboreal[0] = s.pft.leafc_p[m];
                p.lai_dbsboreal[0] = p.lai_p[m];
            } else if p.pftclass[m] == 12 {
                s.patch_flux.leafc_c3arcgrass[0] = s.pft.leafc_p[m];
                p.lai_c3arcgrass[0] = p.lai_p[m];
            } else if p.pftclass[m] == 13 {
                s.patch_flux.leafc_c3grass[0] = s.pft.leafc_p[m];
                p.lai_c3grass[0] = p.lai_p[m];
            } else if p.pftclass[m] == 14 {
                s.patch_flux.leafc_c4grass[0] = s.pft.leafc_p[m];
                p.lai_c4grass[0] = p.lai_p[m];
            }
        }
    }
    s.patch.totvegc[0] =
        (0..npft).fold(0.0, |acc, m| s.pft.totvegc_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.ctrunc_veg[0] =
        (0..npft).fold(0.0, |acc, m| s.pft.ctrunc_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.totcolc[0] = s.patch.totvegc[0]
        + s.patch.totcwdc[0]
        + s.patch.totlitc[0]
        + s.patch.totsomc[0]
        + s.patch.ctrunc_veg[0]
        + s.patch.ctrunc_soil[0];
}

/// 植被 N 池汇总。
fn cnveg_nitrogenstate_summary(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let npft = p.pftclass.len();
    s.patch.leafn[0] = (0..npft).fold(0.0, |acc, m| s.pft.leafn_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.leafn_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.leafn_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.leafn_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.leafn_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.frootn[0] = (0..npft).fold(0.0, |acc, m| s.pft.frootn_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.frootn_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.frootn_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.frootn_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.frootn_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livestemn[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livestemn_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livestemn_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livestemn_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livestemn_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livestemn_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadstemn[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadstemn_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadstemn_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadstemn_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadstemn_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadstemn_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livecrootn[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livecrootn_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livecrootn_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livecrootn_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.livecrootn_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.livecrootn_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadcrootn[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadcrootn_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadcrootn_storage[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadcrootn_storage_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.deadcrootn_xfer[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft.deadcrootn_xfer_p[m].mul_add(p.pftfrac[m], acc)
    });
    if sw.crop {
        s.patch.grainn[0] =
            (0..npft).fold(0.0, |acc, m| s.pft.grainn_p[m].mul_add(p.pftfrac[m], acc));
        s.patch.grainn_storage[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.grainn_storage_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.grainn_xfer[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.grainn_xfer_p[m].mul_add(p.pftfrac[m], acc)
        });
    }
    s.patch.retransn[0] =
        (0..npft).fold(0.0, |acc, m| s.pft.retransn_p[m].mul_add(p.pftfrac[m], acc));
    if sw.diag_matrix {
        s.patch.leafnCap[0] =
            (0..npft).fold(0.0, |acc, m| s.pft.leafnCap_p[m].mul_add(p.pftfrac[m], acc));
        s.patch.leafn_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.leafn_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.leafn_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.leafn_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.frootnCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.frootnCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.frootn_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.frootn_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.frootn_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.frootn_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livestemnCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livestemnCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livestemn_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livestemn_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livestemn_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livestemn_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadstemnCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadstemnCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadstemn_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadstemn_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadstemn_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadstemn_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livecrootnCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livecrootnCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livecrootn_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livecrootn_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.livecrootn_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.livecrootn_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadcrootnCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadcrootnCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadcrootn_storageCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadcrootn_storageCap_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch.deadcrootn_xferCap[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft.deadcrootn_xferCap_p[m].mul_add(p.pftfrac[m], acc)
        });
    }
    for m in 0..npft {
        if sw.crop {
            s.pft.totvegn_p[m] = s.pft.leafn_p[m]
                + s.pft.frootn_p[m]
                + s.pft.livestemn_p[m]
                + s.pft.deadstemn_p[m]
                + s.pft.livecrootn_p[m]
                + s.pft.deadcrootn_p[m]
                + s.pft.leafn_storage_p[m]
                + s.pft.frootn_storage_p[m]
                + s.pft.livestemn_storage_p[m]
                + s.pft.deadstemn_storage_p[m]
                + s.pft.livecrootn_storage_p[m]
                + s.pft.deadcrootn_storage_p[m]
                + s.pft.leafn_xfer_p[m]
                + s.pft.frootn_xfer_p[m]
                + s.pft.livestemn_xfer_p[m]
                + s.pft.deadstemn_xfer_p[m]
                + s.pft.livecrootn_xfer_p[m]
                + s.pft.deadcrootn_xfer_p[m]
                + s.pft.grainn_p[m]
                + s.pft.grainn_storage_p[m]
                + s.pft.grainn_xfer_p[m]
                + s.pft.cropseedn_deficit_p[m]
                + s.pft.npool_p[m]
                + s.pft.retransn_p[m];
        } else {
            s.pft.totvegn_p[m] = s.pft.leafn_p[m]
                + s.pft.frootn_p[m]
                + s.pft.livestemn_p[m]
                + s.pft.deadstemn_p[m]
                + s.pft.livecrootn_p[m]
                + s.pft.deadcrootn_p[m]
                + s.pft.leafn_storage_p[m]
                + s.pft.frootn_storage_p[m]
                + s.pft.livestemn_storage_p[m]
                + s.pft.deadstemn_storage_p[m]
                + s.pft.livecrootn_storage_p[m]
                + s.pft.deadcrootn_storage_p[m]
                + s.pft.leafn_xfer_p[m]
                + s.pft.frootn_xfer_p[m]
                + s.pft.livestemn_xfer_p[m]
                + s.pft.deadstemn_xfer_p[m]
                + s.pft.livecrootn_xfer_p[m]
                + s.pft.deadcrootn_xfer_p[m]
                + s.pft.npool_p[m]
                + s.pft.retransn_p[m];
        }
    }
    s.patch.totvegn[0] =
        (0..npft).fold(0.0, |acc, m| s.pft.totvegn_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.ntrunc_veg[0] =
        (0..npft).fold(0.0, |acc, m| s.pft.ntrunc_p[m].mul_add(p.pftfrac[m], acc));
    s.patch.totcoln[0] = s.patch.totvegn[0]
        + s.patch.totcwdn[0]
        + s.patch.totlitn[0]
        + s.patch.totsomn[0]
        + s.patch.sminn[0]
        + s.patch.ntrunc_veg[0]
        + s.patch.ntrunc_soil[0];
}

/// 土壤 C 通量汇总。
fn soilbiogeochem_carbonflux_summary(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    _sw: BgcSwitches,
) {
    let d = s.dims;
    for k in 0..d.ndecomp_transitions {
        for j in 0..d.nl_soil {
            s.patch_flux.decomp_hr[0] = s.patch_flux.decomp_hr_vr[j + d.nl_soil_full * k]
                .mul_add(p.dz_soi[j], s.patch_flux.decomp_hr[0]);
        }
    }
    for l in 0..d.ndecomp_pools {
        for j in 0..d.nl_soil {
            s.patch_flux.som_c_leached[0] = s.patch_flux.decomp_cpools_transport_tendency
                [j + d.nl_soil_full * l]
                .mul_add(p.dz_soi[j], s.patch_flux.som_c_leached[0]);
        }
    }
}

/// 土壤 N 通量汇总。
fn soilbiogeochem_nitrogenflux_summary(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) {
    let d = s.dims;
    for l in 0..d.ndecomp_pools {
        for j in 0..d.nl_soil {
            s.patch_flux.som_n_leached[0] = s.patch_flux.decomp_npools_transport_tendency
                [j + d.nl_soil_full * l]
                .mul_add(p.dz_soi[j], s.patch_flux.som_n_leached[0]);
        }
    }
    for j in 0..d.nl_soil {
        s.patch_flux.supplement_to_sminn[0] = s.patch_flux.supplement_to_sminn_vr[j]
            .mul_add(p.dz_soi[j], s.patch_flux.supplement_to_sminn[0]);
        s.patch_flux.smin_no3_leached[0] = s.patch_flux.smin_no3_leached_vr[j]
            .mul_add(p.dz_soi[j], s.patch_flux.smin_no3_leached[0]);
        s.patch_flux.smin_no3_runoff[0] = s.patch_flux.smin_no3_runoff_vr[j]
            .mul_add(p.dz_soi[j], s.patch_flux.smin_no3_runoff[0]);
        s.patch_flux.sminn_leached[0] =
            s.patch_flux.sminn_leached_vr[j].mul_add(p.dz_soi[j], s.patch_flux.sminn_leached[0]);
        s.patch_flux.f_n2o_nit[0] =
            s.patch_flux.f_n2o_nit_vr[j].mul_add(p.dz_soi[j], s.patch_flux.f_n2o_nit[0]);
        if sw.nitrif {
            s.patch_flux.denit[0] =
                s.patch_flux.f_denit_vr[j].mul_add(p.dz_soi[j], s.patch_flux.denit[0]);
        } else {
            s.patch_flux.denit[0] = s.patch_flux.sminn_to_denit_excess_vr[j]
                .mul_add(p.dz_soi[j], s.patch_flux.denit[0]);
        }
        if !sw.nitrif {
            for k in 0..d.ndecomp_transitions {
                s.patch_flux.denit[0] = s.patch_flux.sminn_to_denit_decomp_vr
                    [j + d.nl_soil_full * k]
                    .mul_add(p.dz_soi[j], s.patch_flux.denit[0]);
            }
        }
    }
}

/// 植被 C 通量汇总。
fn cnveg_carbonflux_summary(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) -> anyhow::Result<()> {
    let d = s.dims;
    let npft = p.pftclass.len();
    let mut ar_p: f64;
    s.patch_flux.gpp[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft_flux.psn_to_cpool_p[m].mul_add(p.pftfrac[m], acc)
    });
    s.patch.downreg[0] =
        (0..npft).fold(0.0, |acc, m| s.pft.downreg_p[m].mul_add(p.pftfrac[m], acc));
    s.patch_flux.ar[0] = vectorized_dot(npft, |m| {
        (
            s.pft_flux.leaf_mr_p[m]
                + s.pft_flux.froot_mr_p[m]
                + s.pft_flux.livestem_mr_p[m]
                + s.pft_flux.livecroot_mr_p[m]
                + s.pft_flux.cpool_leaf_gr_p[m]
                + s.pft_flux.cpool_froot_gr_p[m]
                + s.pft_flux.cpool_livestem_gr_p[m]
                + s.pft_flux.cpool_deadstem_gr_p[m]
                + s.pft_flux.cpool_livecroot_gr_p[m]
                + s.pft_flux.cpool_deadcroot_gr_p[m]
                + s.pft_flux.transfer_leaf_gr_p[m]
                + s.pft_flux.transfer_froot_gr_p[m]
                + s.pft_flux.transfer_livestem_gr_p[m]
                + s.pft_flux.transfer_deadstem_gr_p[m]
                + s.pft_flux.transfer_livecroot_gr_p[m]
                + s.pft_flux.transfer_deadcroot_gr_p[m]
                + s.pft_flux.cpool_leaf_storage_gr_p[m]
                + s.pft_flux.cpool_froot_storage_gr_p[m]
                + s.pft_flux.cpool_livestem_storage_gr_p[m]
                + s.pft_flux.cpool_deadstem_storage_gr_p[m]
                + s.pft_flux.cpool_livecroot_storage_gr_p[m]
                + s.pft_flux.cpool_deadcroot_storage_gr_p[m]
                + s.pft_flux.grain_mr_p[m]
                + s.pft_flux.xsmrpool_to_atm_p[m]
                + s.pft_flux.cpool_grain_gr_p[m]
                + s.pft_flux.transfer_grain_gr_p[m]
                + s.pft_flux.cpool_grain_storage_gr_p[m],
            p.pftfrac[m],
        )
    }); /*FMA? gimple=1 rust=0 L780*/
    s.patch_flux.gpp_enftemp[0] = 0.0;
    s.patch_flux.gpp_enfboreal[0] = 0.0;
    s.patch_flux.gpp_dnfboreal[0] = 0.0;
    s.patch_flux.gpp_ebftrop[0] = 0.0;
    s.patch_flux.gpp_ebftemp[0] = 0.0;
    s.patch_flux.gpp_dbftrop[0] = 0.0;
    s.patch_flux.gpp_dbftemp[0] = 0.0;
    s.patch_flux.gpp_dbfboreal[0] = 0.0;
    s.patch_flux.gpp_ebstemp[0] = 0.0;
    s.patch_flux.gpp_dbstemp[0] = 0.0;
    s.patch_flux.gpp_dbsboreal[0] = 0.0;
    s.patch_flux.gpp_c3arcgrass[0] = 0.0;
    s.patch_flux.gpp_c3grass[0] = 0.0;
    s.patch_flux.gpp_c4grass[0] = 0.0;
    s.patch_flux.npp_enftemp[0] = 0.0;
    s.patch_flux.npp_enfboreal[0] = 0.0;
    s.patch_flux.npp_dnfboreal[0] = 0.0;
    s.patch_flux.npp_ebftrop[0] = 0.0;
    s.patch_flux.npp_ebftemp[0] = 0.0;
    s.patch_flux.npp_dbftrop[0] = 0.0;
    s.patch_flux.npp_dbftemp[0] = 0.0;
    s.patch_flux.npp_dbfboreal[0] = 0.0;
    s.patch_flux.npp_ebstemp[0] = 0.0;
    s.patch_flux.npp_dbstemp[0] = 0.0;
    s.patch_flux.npp_dbsboreal[0] = 0.0;
    s.patch_flux.npp_c3arcgrass[0] = 0.0;
    s.patch_flux.npp_c3grass[0] = 0.0;
    s.patch_flux.npp_c4grass[0] = 0.0;
    s.patch_flux.npptoleafc_enftemp[0] = 0.0;
    s.patch_flux.npptoleafc_enfboreal[0] = 0.0;
    s.patch_flux.npptoleafc_dnfboreal[0] = 0.0;
    s.patch_flux.npptoleafc_ebftrop[0] = 0.0;
    s.patch_flux.npptoleafc_ebftemp[0] = 0.0;
    s.patch_flux.npptoleafc_dbftrop[0] = 0.0;
    s.patch_flux.npptoleafc_dbftemp[0] = 0.0;
    s.patch_flux.npptoleafc_dbfboreal[0] = 0.0;
    s.patch_flux.npptoleafc_ebstemp[0] = 0.0;
    s.patch_flux.npptoleafc_dbstemp[0] = 0.0;
    s.patch_flux.npptoleafc_dbsboreal[0] = 0.0;
    s.patch_flux.npptoleafc_c3arcgrass[0] = 0.0;
    s.patch_flux.npptoleafc_c3grass[0] = 0.0;
    s.patch_flux.npptoleafc_c4grass[0] = 0.0;
    for m in 0..npft {
        ar_p = s.pft_flux.leaf_mr_p[m]
            + s.pft_flux.froot_mr_p[m]
            + s.pft_flux.livestem_mr_p[m]
            + s.pft_flux.livecroot_mr_p[m]
            + s.pft_flux.cpool_leaf_gr_p[m]
            + s.pft_flux.cpool_froot_gr_p[m]
            + s.pft_flux.cpool_livestem_gr_p[m]
            + s.pft_flux.cpool_deadstem_gr_p[m]
            + s.pft_flux.cpool_livecroot_gr_p[m]
            + s.pft_flux.cpool_deadcroot_gr_p[m]
            + s.pft_flux.transfer_leaf_gr_p[m]
            + s.pft_flux.transfer_froot_gr_p[m]
            + s.pft_flux.transfer_livestem_gr_p[m]
            + s.pft_flux.transfer_deadstem_gr_p[m]
            + s.pft_flux.transfer_livecroot_gr_p[m]
            + s.pft_flux.transfer_deadcroot_gr_p[m]
            + s.pft_flux.cpool_leaf_storage_gr_p[m]
            + s.pft_flux.cpool_froot_storage_gr_p[m]
            + s.pft_flux.cpool_livestem_storage_gr_p[m]
            + s.pft_flux.cpool_deadstem_storage_gr_p[m]
            + s.pft_flux.cpool_livecroot_storage_gr_p[m]
            + s.pft_flux.cpool_deadcroot_storage_gr_p[m]
            + s.pft_flux.grain_mr_p[m]
            + s.pft_flux.xsmrpool_to_atm_p[m]
            + s.pft_flux.cpool_grain_gr_p[m]
            + s.pft_flux.transfer_grain_gr_p[m]
            + s.pft_flux.cpool_grain_storage_gr_p[m];
        if p.pftclass[m] == 1 {
            s.patch_flux.gpp_enftemp[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_enftemp[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_enftemp[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 2 {
            s.patch_flux.gpp_enfboreal[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_enfboreal[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_enfboreal[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 3 {
            s.patch_flux.gpp_dnfboreal[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_dnfboreal[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_dnfboreal[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 4 {
            s.patch_flux.gpp_ebftrop[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_ebftrop[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_ebftrop[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 5 {
            s.patch_flux.gpp_ebftemp[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_ebftemp[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_ebftemp[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 6 {
            s.patch_flux.gpp_dbftrop[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_dbftrop[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_dbftrop[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 7 {
            s.patch_flux.gpp_dbftemp[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_dbftemp[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_dbftemp[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 8 {
            s.patch_flux.gpp_dbfboreal[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_dbfboreal[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_dbfboreal[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 9 {
            s.patch_flux.gpp_ebstemp[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_ebstemp[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_ebstemp[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 10 {
            s.patch_flux.gpp_dbstemp[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_dbstemp[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_dbstemp[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 11 {
            s.patch_flux.gpp_dbsboreal[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_dbsboreal[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_dbsboreal[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 12 {
            s.patch_flux.gpp_c3arcgrass[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_c3arcgrass[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_c3arcgrass[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 13 {
            s.patch_flux.gpp_c3grass[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_c3grass[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_c3grass[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        } else if p.pftclass[m] == 14 {
            s.patch_flux.gpp_c4grass[0] = s.pft_flux.psn_to_cpool_p[m];
            s.patch_flux.npp_c4grass[0] = s.pft_flux.psn_to_cpool_p[m] - ar_p;
            s.patch_flux.npptoleafc_c4grass[0] =
                s.pft_flux.cpool_to_leafc_p[m] + s.pft_flux.cpool_to_leafc_storage_p[m];
        }
    }
    // `#ifdef FUN` 分支：单点 Rust 引擎里该宏未定义。
    {}
    s.patch_flux.er[0] = s.patch_flux.ar[0] + s.patch_flux.decomp_hr[0];
    if sw.crop {
        if p.patchclass == 12 {
            if 0 != (npft as i32 - 1) {
                // write(*,*) 'Error: crop patch CONTAINS multiple pfts:',p_iam_glb,'i=',i,'ps',ps,'does not equal to pe',pe
                anyhow::bail!("cnveg_carbonflux_summary：上游在此 abort（收支/廓线检查失败）");
            } else {
                s.patch_flux.cropprod1c_loss[0] = s.pft_flux.cropprod1c_loss_p[0];
                s.patch_flux.grainc_to_cropprodc[0] = s.pft_flux.grainc_to_food_p[0];
                s.patch_flux.grainc_to_seed[0] = s.pft_flux.grainc_to_seed_p[0];
                s.patch.plantdate[0] = s.pft.plantdate_p[0];
            }
        } else {
            s.patch_flux.cropprod1c_loss[0] = 0.0;
            s.patch_flux.grainc_to_cropprodc[0] = 0.0;
            s.patch_flux.grainc_to_seed[0] = 0.0;
        }
    }
    if sw.fire {
        for m in 0..npft {
            s.pft_flux.fire_closs_p[m] = s.pft_flux.m_leafc_to_fire_p[m]
                + s.pft_flux.m_leafc_storage_to_fire_p[m]
                + s.pft_flux.m_leafc_xfer_to_fire_p[m]
                + s.pft_flux.m_frootc_to_fire_p[m]
                + s.pft_flux.m_frootc_storage_to_fire_p[m]
                + s.pft_flux.m_frootc_xfer_to_fire_p[m]
                + s.pft_flux.m_livestemc_to_fire_p[m]
                + s.pft_flux.m_livestemc_storage_to_fire_p[m]
                + s.pft_flux.m_livestemc_xfer_to_fire_p[m]
                + s.pft_flux.m_deadstemc_to_fire_p[m]
                + s.pft_flux.m_deadstemc_storage_to_fire_p[m]
                + s.pft_flux.m_deadstemc_xfer_to_fire_p[m]
                + s.pft_flux.m_livecrootc_to_fire_p[m]
                + s.pft_flux.m_livecrootc_storage_to_fire_p[m]
                + s.pft_flux.m_livecrootc_xfer_to_fire_p[m]
                + s.pft_flux.m_deadcrootc_to_fire_p[m]
                + s.pft_flux.m_deadcrootc_storage_to_fire_p[m]
                + s.pft_flux.m_deadcrootc_xfer_to_fire_p[m]
                + s.pft_flux.m_gresp_storage_to_fire_p[m]
                + s.pft_flux.m_gresp_xfer_to_fire_p[m];
        }
        s.patch_flux.pft_fire_closs[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.fire_closs_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafc_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafc_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootc_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootc_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemc_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemc_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemc_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemc_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootc_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootc_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootc_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootc_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafc_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafc_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootc_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootc_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemc_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemc_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemc_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemc_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootc_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootc_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootc_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootc_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_gresp_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_gresp_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafc_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafc_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootc_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootc_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemc_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemc_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemc_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemc_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootc_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootc_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootc_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootc_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_gresp_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_gresp_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemc_to_deadstemc_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemc_to_deadstemc_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootc_to_deadcrootc_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootc_to_deadcrootc_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafc_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafc_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootc_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootc_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemc_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemc_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemc_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemc_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootc_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootc_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootc_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootc_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafc_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafc_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootc_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootc_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemc_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemc_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemc_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemc_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootc_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootc_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootc_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootc_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_gresp_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_gresp_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafc_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafc_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootc_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootc_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemc_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemc_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemc_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemc_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootc_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootc_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootc_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootc_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_gresp_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_gresp_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.fire_closs[0] = s.patch_flux.pft_fire_closs[0];
        for j in 0..d.nl_soil {
            for l in 0..d.ndecomp_pools {
                if s.invariants.is_litter[l] || s.invariants.is_cwd[l] {
                    s.patch_flux.fire_closs[0] = s.patch_flux.m_decomp_cpools_to_fire_vr
                        [j + d.nl_soil_full * l]
                        .mul_add(p.dz_soi[j], s.patch_flux.fire_closs[0]);
                }
            }
        }
        s.patch_flux.m_litr1_c_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_cpools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
        s.patch_flux.m_litr2_c_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_cpools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
        s.patch_flux.m_litr3_c_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_cpools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
        s.patch_flux.m_cwd_c_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_cpools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
        s.patch_flux.litfire[0] = 0.0;
        s.patch_flux.somfire[0] = 0.0;
        for l in 0..d.ndecomp_pools {
            if s.invariants.is_litter[l] {
                s.patch_flux.litfire[0] += (0..d.nl_soil).fold(0.0, |acc, j| {
                    s.patch_flux.m_decomp_cpools_to_fire_vr[j + d.nl_soil_full * l]
                        .mul_add(p.dz_soi[j], acc)
                });
            }
            if s.invariants.is_soil[l] {
                s.patch_flux.somfire[0] += (0..d.nl_soil).fold(0.0, |acc, j| {
                    s.patch_flux.m_decomp_cpools_to_fire_vr[j + d.nl_soil_full * l]
                        .mul_add(p.dz_soi[j], acc)
                });
            }
        }
        s.patch_flux.totfire[0] =
            s.patch_flux.fire_closs[0] + s.patch_flux.somfire[0] + s.patch_flux.somc_fire[0];
    }
    s.patch_flux.hrv_xsmrpool_to_atm[0] = (0..npft).fold(0.0, |acc, m| {
        s.pft_flux.hrv_xsmrpool_to_atm_p[m].mul_add(p.pftfrac[m], acc)
    });
    let nfixlags: f64 = s.constants.nfix_timeconst * 86400.0;
    if nfixlags > 0.0 && s.patch.lag_npp[0] != MISSING {
        s.patch.lag_npp[0] = s.patch.lag_npp[0].mul_add(
            (-(p.deltim / nfixlags)).exp(),
            (s.patch_flux.gpp[0] - s.patch_flux.ar[0]) * (1.0 - (-(p.deltim / nfixlags)).exp()),
        );
    } else {
        s.patch.lag_npp[0] = s.patch_flux.gpp[0] - s.patch_flux.ar[0];
    }
    Ok(())
}

/// 植被 N 通量汇总。
fn cnveg_nitrogenflux_summary(
    s: &mut BgcState,
    p: &BgcPhysics,
    _c: &BgcPftConstants,
    sw: BgcSwitches,
) -> anyhow::Result<()> {
    let d = s.dims;
    let npft = p.pftclass.len();
    if sw.crop {
        if p.patchclass == 12 {
            if 0 != (npft as i32 - 1) {
                // write(*,*) 'Error: crop patch contains multiple pfts:',p_iam_glb,'i=',i,'ps',ps,'does not equal to pe',pe
                anyhow::bail!("cnveg_nitrogenflux_summary：上游在此 abort（收支/廓线检查失败）");
            } else {
                s.patch_flux.grainn_to_cropprodn[0] = s.pft_flux.grainn_to_food_p[0];
            }
        } else {
            s.patch_flux.grainn_to_cropprodn[0] = 0.0;
        }
    }
    if sw.fire {
        for m in 0..npft {
            s.pft_flux.fire_nloss_p[m] = s.pft_flux.m_leafn_to_fire_p[m]
                + s.pft_flux.m_leafn_storage_to_fire_p[m]
                + s.pft_flux.m_leafn_xfer_to_fire_p[m]
                + s.pft_flux.m_frootn_to_fire_p[m]
                + s.pft_flux.m_frootn_storage_to_fire_p[m]
                + s.pft_flux.m_frootn_xfer_to_fire_p[m]
                + s.pft_flux.m_livestemn_to_fire_p[m]
                + s.pft_flux.m_livestemn_storage_to_fire_p[m]
                + s.pft_flux.m_livestemn_xfer_to_fire_p[m]
                + s.pft_flux.m_deadstemn_to_fire_p[m]
                + s.pft_flux.m_deadstemn_storage_to_fire_p[m]
                + s.pft_flux.m_deadstemn_xfer_to_fire_p[m]
                + s.pft_flux.m_livecrootn_to_fire_p[m]
                + s.pft_flux.m_livecrootn_storage_to_fire_p[m]
                + s.pft_flux.m_livecrootn_xfer_to_fire_p[m]
                + s.pft_flux.m_deadcrootn_to_fire_p[m]
                + s.pft_flux.m_deadcrootn_storage_to_fire_p[m]
                + s.pft_flux.m_deadcrootn_xfer_to_fire_p[m]
                + s.pft_flux.m_retransn_to_fire_p[m];
        }
        s.patch_flux.pft_fire_nloss[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.fire_nloss_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafn_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafn_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootn_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootn_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemn_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemn_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemn_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemn_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootn_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootn_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootn_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootn_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafn_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafn_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootn_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootn_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemn_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemn_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemn_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemn_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootn_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootn_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootn_storage_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootn_storage_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafn_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafn_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootn_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootn_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemn_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemn_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemn_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemn_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootn_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootn_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootn_xfer_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootn_xfer_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemn_to_deadstemn_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemn_to_deadstemn_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootn_to_deadcrootn_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootn_to_deadcrootn_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_retransn_to_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_retransn_to_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafn_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafn_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootn_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootn_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemn_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemn_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemn_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemn_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootn_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootn_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootn_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootn_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafn_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafn_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootn_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootn_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemn_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemn_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemn_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemn_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootn_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootn_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootn_storage_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootn_storage_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_leafn_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_leafn_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_frootn_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_frootn_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livestemn_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livestemn_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadstemn_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadstemn_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_livecrootn_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_livecrootn_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_deadcrootn_xfer_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_deadcrootn_xfer_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.m_retransn_to_litter_fire[0] = (0..npft).fold(0.0, |acc, m| {
            s.pft_flux.m_retransn_to_litter_fire_p[m].mul_add(p.pftfrac[m], acc)
        });
        s.patch_flux.fire_nloss[0] = s.patch_flux.pft_fire_nloss[0];
        for j in 0..d.nl_soil {
            for l in 0..d.ndecomp_pools {
                if s.invariants.is_litter[l] || s.invariants.is_cwd[l] {
                    s.patch_flux.fire_nloss[0] = s.patch_flux.m_decomp_npools_to_fire_vr
                        [j + d.nl_soil_full * l]
                        .mul_add(p.dz_soi[j], s.patch_flux.fire_nloss[0]);
                }
            }
        }
        s.patch_flux.m_litr1_n_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_npools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_met_lit - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
        s.patch_flux.m_litr2_n_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_npools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_cel_lit - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
        s.patch_flux.m_litr3_n_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_npools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_lig_lit - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
        s.patch_flux.m_cwd_n_to_fire[0] = (0..d.nl_soil).fold(0.0, |acc, j| {
            s.patch_flux.m_decomp_npools_to_fire_vr
                [j + d.nl_soil_full * ((s.constants.i_cwd - 1) as usize)]
                .mul_add(p.dz_soi[j], acc)
        });
    }
    Ok(())
}
