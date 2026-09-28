//! `MOD_BGC_CNSASU.F90`：半解析谱加速（SASU）与诊断矩阵（`DEF_USE_DiagMatrix`）。
//!
//! 年初第一步（`idate(2) == 1 .and. idate(3) == deltim`）把各池存成 `*0`；年末最后一步用一年里
//! 累加的转移/流出通量除以年初池得到速率矩阵 `A`、累加输入得到 `I`，容量 `X = −A⁻¹·I`。
//! SASU 用容量替换死茎/死粗根与土壤池，DiagMatrix 只写出 `*Cap`。之后清零全部累加量。
//!
//! 数值上只有两处要按 gfortran -O2 的 GIMPLE 对齐：
//! - [`inverse`] 的三处消元/回代都是 `.FNMA`（`x − a·b` 一次舍入）；
//! - `matmul` 被内联成"外层列 `j`、内层行 `i`、`res(i) = fma(A(i,j), b(j), res(i))`"，
//!   累加从 0 开始、按 `j` 递增（行方向被向量化，不改变每个元素的求值顺序）。
//!
//! 其余都是单次除法与比较。矩阵按 Fortran 列主序展平：`(row, col)` → `row + col·n`。

use anyhow::{bail, Result};

use crate::bgc_driver::{is_end_of_year, BgcPhysics, BgcSwitches};
use crate::bgc_state::BgcState;

const EPSI: f64 = 1.0e-8;
const NVEGC: usize = 21;
const NVEGN: usize = 22;

// 上游 `ileaf … iretrans`（1 起），这里减一。
const ILEAF: usize = 0;
const ILEAF_ST: usize = 1;
const ILEAF_XF: usize = 2;
const IFROOT: usize = 3;
const IFROOT_ST: usize = 4;
const IFROOT_XF: usize = 5;
const ILIVESTEM: usize = 6;
const ILIVESTEM_ST: usize = 7;
const ILIVESTEM_XF: usize = 8;
const IDEADSTEM: usize = 9;
const IDEADSTEM_ST: usize = 10;
const IDEADSTEM_XF: usize = 11;
const ILIVECROOT: usize = 12;
const ILIVECROOT_ST: usize = 13;
const ILIVECROOT_XF: usize = 14;
const IDEADCROOT: usize = 15;
const IDEADCROOT_ST: usize = 16;
const IDEADCROOT_XF: usize = 17;
const IGRAIN: usize = 18;
const IGRAIN_ST: usize = 19;
const IGRAIN_XF: usize = 20;
const IRETRANS: usize = 21;

/// 年初保存的池：`(年初池, 当前池)`，`年初池 = max(当前池, epsi)`（`:221-264`）。
#[rustfmt::skip]
const POOL_STARTS: &[(&str, &str)] = &[
    ("leafc0_p", "leafc_p"),
    ("leafc0_storage_p", "leafc_storage_p"),
    ("leafc0_xfer_p", "leafc_xfer_p"),
    ("frootc0_p", "frootc_p"),
    ("frootc0_storage_p", "frootc_storage_p"),
    ("frootc0_xfer_p", "frootc_xfer_p"),
    ("livestemc0_p", "livestemc_p"),
    ("livestemc0_storage_p", "livestemc_storage_p"),
    ("livestemc0_xfer_p", "livestemc_xfer_p"),
    ("deadstemc0_p", "deadstemc_p"),
    ("deadstemc0_storage_p", "deadstemc_storage_p"),
    ("deadstemc0_xfer_p", "deadstemc_xfer_p"),
    ("livecrootc0_p", "livecrootc_p"),
    ("livecrootc0_storage_p", "livecrootc_storage_p"),
    ("livecrootc0_xfer_p", "livecrootc_xfer_p"),
    ("deadcrootc0_p", "deadcrootc_p"),
    ("deadcrootc0_storage_p", "deadcrootc_storage_p"),
    ("deadcrootc0_xfer_p", "deadcrootc_xfer_p"),
    ("grainc0_p", "grainc_p"),
    ("grainc0_storage_p", "grainc_storage_p"),
    ("grainc0_xfer_p", "grainc_xfer_p"),
    ("leafn0_p", "leafn_p"),
    ("leafn0_storage_p", "leafn_storage_p"),
    ("leafn0_xfer_p", "leafn_xfer_p"),
    ("frootn0_p", "frootn_p"),
    ("frootn0_storage_p", "frootn_storage_p"),
    ("frootn0_xfer_p", "frootn_xfer_p"),
    ("livestemn0_p", "livestemn_p"),
    ("livestemn0_storage_p", "livestemn_storage_p"),
    ("livestemn0_xfer_p", "livestemn_xfer_p"),
    ("deadstemn0_p", "deadstemn_p"),
    ("deadstemn0_storage_p", "deadstemn_storage_p"),
    ("deadstemn0_xfer_p", "deadstemn_xfer_p"),
    ("livecrootn0_p", "livecrootn_p"),
    ("livecrootn0_storage_p", "livecrootn_storage_p"),
    ("livecrootn0_xfer_p", "livecrootn_xfer_p"),
    ("deadcrootn0_p", "deadcrootn_p"),
    ("deadcrootn0_storage_p", "deadcrootn_storage_p"),
    ("deadcrootn0_xfer_p", "deadcrootn_xfer_p"),
    ("grainn0_p", "grainn_p"),
    ("grainn0_storage_p", "grainn_storage_p"),
    ("grainn0_xfer_p", "grainn_xfer_p"),
    ("retransn0_p", "retransn_p"),
];

/// 植被 C 速率矩阵的非零元：`(行, 列, 累加通量, 年初池, 是否流出项)`。流出项在对角线上取负
/// （`:273-315`）。`- a/b` 与 `(-a)/b` 逐位相同。
#[rustfmt::skip]
const VEG_C: &[(usize, usize, &str, &str, bool)] = &[
    (ILEAF, ILEAF_XF, "AKX_leafc_xf_to_leafc_p_acc", "leafc0_xfer_p", false),
    (IFROOT, IFROOT_XF, "AKX_frootc_xf_to_frootc_p_acc", "frootc0_xfer_p", false),
    (ILIVESTEM, ILIVESTEM_XF, "AKX_livestemc_xf_to_livestemc_p_acc", "livestemc0_xfer_p", false),
    (IDEADSTEM, IDEADSTEM_XF, "AKX_deadstemc_xf_to_deadstemc_p_acc", "deadstemc0_xfer_p", false),
    (ILIVECROOT, ILIVECROOT_XF, "AKX_livecrootc_xf_to_livecrootc_p_acc", "livecrootc0_xfer_p", false),
    (IDEADCROOT, IDEADCROOT_XF, "AKX_deadcrootc_xf_to_deadcrootc_p_acc", "deadcrootc0_xfer_p", false),
    (IGRAIN, IGRAIN_XF, "AKX_grainc_xf_to_grainc_p_acc", "grainc0_xfer_p", false),
    (IDEADSTEM, ILIVESTEM, "AKX_livestemc_to_deadstemc_p_acc", "livestemc0_p", false),
    (IDEADCROOT, ILIVECROOT, "AKX_livecrootc_to_deadcrootc_p_acc", "livecrootc0_p", false),
    (ILEAF_XF, ILEAF_ST, "AKX_leafc_st_to_leafc_xf_p_acc", "leafc0_storage_p", false),
    (IFROOT_XF, IFROOT_ST, "AKX_frootc_st_to_frootc_xf_p_acc", "frootc0_storage_p", false),
    (ILIVESTEM_XF, ILIVESTEM_ST, "AKX_livestemc_st_to_livestemc_xf_p_acc", "livestemc0_storage_p", false),
    (IDEADSTEM_XF, IDEADSTEM_ST, "AKX_deadstemc_st_to_deadstemc_xf_p_acc", "deadstemc0_storage_p", false),
    (ILIVECROOT_XF, ILIVECROOT_ST, "AKX_livecrootc_st_to_livecrootc_xf_p_acc", "livecrootc0_storage_p", false),
    (IDEADCROOT_XF, IDEADCROOT_ST, "AKX_deadcrootc_st_to_deadcrootc_xf_p_acc", "deadcrootc0_storage_p", false),
    (IGRAIN_XF, IGRAIN_ST, "AKX_grainc_st_to_grainc_xf_p_acc", "grainc0_storage_p", false),
    (ILEAF, ILEAF, "AKX_leafc_exit_p_acc", "leafc0_p", true),
    (ILEAF_ST, ILEAF_ST, "AKX_leafc_st_exit_p_acc", "leafc0_storage_p", true),
    (ILEAF_XF, ILEAF_XF, "AKX_leafc_xf_exit_p_acc", "leafc0_xfer_p", true),
    (IFROOT, IFROOT, "AKX_frootc_exit_p_acc", "frootc0_p", true),
    (IFROOT_ST, IFROOT_ST, "AKX_frootc_st_exit_p_acc", "frootc0_storage_p", true),
    (IFROOT_XF, IFROOT_XF, "AKX_frootc_xf_exit_p_acc", "frootc0_xfer_p", true),
    (ILIVESTEM, ILIVESTEM, "AKX_livestemc_exit_p_acc", "livestemc0_p", true),
    (ILIVESTEM_ST, ILIVESTEM_ST, "AKX_livestemc_st_exit_p_acc", "livestemc0_storage_p", true),
    (ILIVESTEM_XF, ILIVESTEM_XF, "AKX_livestemc_xf_exit_p_acc", "livestemc0_xfer_p", true),
    (IDEADSTEM, IDEADSTEM, "AKX_deadstemc_exit_p_acc", "deadstemc0_p", true),
    (IDEADSTEM_ST, IDEADSTEM_ST, "AKX_deadstemc_st_exit_p_acc", "deadstemc0_storage_p", true),
    (IDEADSTEM_XF, IDEADSTEM_XF, "AKX_deadstemc_xf_exit_p_acc", "deadstemc0_xfer_p", true),
    (ILIVECROOT, ILIVECROOT, "AKX_livecrootc_exit_p_acc", "livecrootc0_p", true),
    (ILIVECROOT_ST, ILIVECROOT_ST, "AKX_livecrootc_st_exit_p_acc", "livecrootc0_storage_p", true),
    (ILIVECROOT_XF, ILIVECROOT_XF, "AKX_livecrootc_xf_exit_p_acc", "livecrootc0_xfer_p", true),
    (IDEADCROOT, IDEADCROOT, "AKX_deadcrootc_exit_p_acc", "deadcrootc0_p", true),
    (IDEADCROOT_ST, IDEADCROOT_ST, "AKX_deadcrootc_st_exit_p_acc", "deadcrootc0_storage_p", true),
    (IDEADCROOT_XF, IDEADCROOT_XF, "AKX_deadcrootc_xf_exit_p_acc", "deadcrootc0_xfer_p", true),
    (IGRAIN, IGRAIN, "AKX_grainc_exit_p_acc", "grainc0_p", true),
    (IGRAIN_ST, IGRAIN_ST, "AKX_grainc_st_exit_p_acc", "grainc0_storage_p", true),
    (IGRAIN_XF, IGRAIN_XF, "AKX_grainc_xf_exit_p_acc", "grainc0_xfer_p", true),
];

/// 植被 N 速率矩阵的非零元（`:332-390`），含再转移池 `iretrans`。
#[rustfmt::skip]
const VEG_N: &[(usize, usize, &str, &str, bool)] = &[
    (ILEAF, ILEAF_XF, "AKX_leafn_xf_to_leafn_p_acc", "leafn0_xfer_p", false),
    (IFROOT, IFROOT_XF, "AKX_frootn_xf_to_frootn_p_acc", "frootn0_xfer_p", false),
    (ILIVESTEM, ILIVESTEM_XF, "AKX_livestemn_xf_to_livestemn_p_acc", "livestemn0_xfer_p", false),
    (IDEADSTEM, IDEADSTEM_XF, "AKX_deadstemn_xf_to_deadstemn_p_acc", "deadstemn0_xfer_p", false),
    (ILIVECROOT, ILIVECROOT_XF, "AKX_livecrootn_xf_to_livecrootn_p_acc", "livecrootn0_xfer_p", false),
    (IDEADCROOT, IDEADCROOT_XF, "AKX_deadcrootn_xf_to_deadcrootn_p_acc", "deadcrootn0_xfer_p", false),
    (IGRAIN, IGRAIN_XF, "AKX_grainn_xf_to_grainn_p_acc", "grainn0_xfer_p", false),
    (IDEADSTEM, ILIVESTEM, "AKX_livestemn_to_deadstemn_p_acc", "livestemn0_p", false),
    (IDEADCROOT, ILIVECROOT, "AKX_livecrootn_to_deadcrootn_p_acc", "livecrootn0_p", false),
    (ILEAF_XF, ILEAF_ST, "AKX_leafn_st_to_leafn_xf_p_acc", "leafn0_storage_p", false),
    (IFROOT_XF, IFROOT_ST, "AKX_frootn_st_to_frootn_xf_p_acc", "frootn0_storage_p", false),
    (ILIVESTEM_XF, ILIVESTEM_ST, "AKX_livestemn_st_to_livestemn_xf_p_acc", "livestemn0_storage_p", false),
    (IDEADSTEM_XF, IDEADSTEM_ST, "AKX_deadstemn_st_to_deadstemn_xf_p_acc", "deadstemn0_storage_p", false),
    (ILIVECROOT_XF, ILIVECROOT_ST, "AKX_livecrootn_st_to_livecrootn_xf_p_acc", "livecrootn0_storage_p", false),
    (IDEADCROOT_XF, IDEADCROOT_ST, "AKX_deadcrootn_st_to_deadcrootn_xf_p_acc", "deadcrootn0_storage_p", false),
    (IGRAIN_XF, IGRAIN_ST, "AKX_grainn_st_to_grainn_xf_p_acc", "grainn0_storage_p", false),
    (IRETRANS, ILEAF, "AKX_leafn_to_retransn_p_acc", "leafn0_p", false),
    (IRETRANS, IFROOT, "AKX_frootn_to_retransn_p_acc", "frootn0_p", false),
    (IRETRANS, ILIVESTEM, "AKX_livestemn_to_retransn_p_acc", "livestemn0_p", false),
    (IRETRANS, ILIVECROOT, "AKX_livecrootn_to_retransn_p_acc", "livecrootn0_p", false),
    (ILEAF, IRETRANS, "AKX_retransn_to_leafn_p_acc", "retransn0_p", false),
    (IFROOT, IRETRANS, "AKX_retransn_to_frootn_p_acc", "retransn0_p", false),
    (ILIVESTEM, IRETRANS, "AKX_retransn_to_livestemn_p_acc", "retransn0_p", false),
    (IDEADSTEM, IRETRANS, "AKX_retransn_to_deadstemn_p_acc", "retransn0_p", false),
    (ILIVECROOT, IRETRANS, "AKX_retransn_to_livecrootn_p_acc", "retransn0_p", false),
    (IDEADCROOT, IRETRANS, "AKX_retransn_to_deadcrootn_p_acc", "retransn0_p", false),
    (IGRAIN, IRETRANS, "AKX_retransn_to_grainn_p_acc", "retransn0_p", false),
    (ILEAF_ST, IRETRANS, "AKX_retransn_to_leafn_st_p_acc", "retransn0_p", false),
    (IFROOT_ST, IRETRANS, "AKX_retransn_to_frootn_st_p_acc", "retransn0_p", false),
    (ILIVESTEM_ST, IRETRANS, "AKX_retransn_to_livestemn_st_p_acc", "retransn0_p", false),
    (IDEADSTEM_ST, IRETRANS, "AKX_retransn_to_deadstemn_st_p_acc", "retransn0_p", false),
    (ILIVECROOT_ST, IRETRANS, "AKX_retransn_to_livecrootn_st_p_acc", "retransn0_p", false),
    (IDEADCROOT_ST, IRETRANS, "AKX_retransn_to_deadcrootn_st_p_acc", "retransn0_p", false),
    (IGRAIN_ST, IRETRANS, "AKX_retransn_to_grainn_st_p_acc", "retransn0_p", false),
    (ILEAF, ILEAF, "AKX_leafn_exit_p_acc", "leafn0_p", true),
    (ILEAF_ST, ILEAF_ST, "AKX_leafn_st_exit_p_acc", "leafn0_storage_p", true),
    (ILEAF_XF, ILEAF_XF, "AKX_leafn_xf_exit_p_acc", "leafn0_xfer_p", true),
    (IFROOT, IFROOT, "AKX_frootn_exit_p_acc", "frootn0_p", true),
    (IFROOT_ST, IFROOT_ST, "AKX_frootn_st_exit_p_acc", "frootn0_storage_p", true),
    (IFROOT_XF, IFROOT_XF, "AKX_frootn_xf_exit_p_acc", "frootn0_xfer_p", true),
    (ILIVESTEM, ILIVESTEM, "AKX_livestemn_exit_p_acc", "livestemn0_p", true),
    (ILIVESTEM_ST, ILIVESTEM_ST, "AKX_livestemn_st_exit_p_acc", "livestemn0_storage_p", true),
    (ILIVESTEM_XF, ILIVESTEM_XF, "AKX_livestemn_xf_exit_p_acc", "livestemn0_xfer_p", true),
    (IDEADSTEM, IDEADSTEM, "AKX_deadstemn_exit_p_acc", "deadstemn0_p", true),
    (IDEADSTEM_ST, IDEADSTEM_ST, "AKX_deadstemn_st_exit_p_acc", "deadstemn0_storage_p", true),
    (IDEADSTEM_XF, IDEADSTEM_XF, "AKX_deadstemn_xf_exit_p_acc", "deadstemn0_xfer_p", true),
    (ILIVECROOT, ILIVECROOT, "AKX_livecrootn_exit_p_acc", "livecrootn0_p", true),
    (ILIVECROOT_ST, ILIVECROOT_ST, "AKX_livecrootn_st_exit_p_acc", "livecrootn0_storage_p", true),
    (ILIVECROOT_XF, ILIVECROOT_XF, "AKX_livecrootn_xf_exit_p_acc", "livecrootn0_xfer_p", true),
    (IDEADCROOT, IDEADCROOT, "AKX_deadcrootn_exit_p_acc", "deadcrootn0_p", true),
    (IDEADCROOT_ST, IDEADCROOT_ST, "AKX_deadcrootn_st_exit_p_acc", "deadcrootn0_storage_p", true),
    (IDEADCROOT_XF, IDEADCROOT_XF, "AKX_deadcrootn_xf_exit_p_acc", "deadcrootn0_xfer_p", true),
    (IGRAIN, IGRAIN, "AKX_grainn_exit_p_acc", "grainn0_p", true),
    (IGRAIN_ST, IGRAIN_ST, "AKX_grainn_st_exit_p_acc", "grainn0_storage_p", true),
    (IGRAIN_XF, IGRAIN_XF, "AKX_grainn_xf_exit_p_acc", "grainn0_xfer_p", true),
    (IRETRANS, IRETRANS, "AKX_retransn_exit_p_acc", "retransn0_p", true),
];

/// 植被输入（`I_veg_acc`/`I_veg_nacc`，`:317-330`、`:392-405`）：C 与 N 用同一组下标。
#[rustfmt::skip]
const VEG_INPUTS: &[(usize, &str, &str)] = &[
    (ILEAF, "I_leafc_p_acc", "I_leafn_p_acc"),
    (ILEAF_ST, "I_leafc_st_p_acc", "I_leafn_st_p_acc"),
    (IFROOT, "I_frootc_p_acc", "I_frootn_p_acc"),
    (IFROOT_ST, "I_frootc_st_p_acc", "I_frootn_st_p_acc"),
    (ILIVESTEM, "I_livestemc_p_acc", "I_livestemn_p_acc"),
    (ILIVESTEM_ST, "I_livestemc_st_p_acc", "I_livestemn_st_p_acc"),
    (IDEADSTEM, "I_deadstemc_p_acc", "I_deadstemn_p_acc"),
    (IDEADSTEM_ST, "I_deadstemc_st_p_acc", "I_deadstemn_st_p_acc"),
    (ILIVECROOT, "I_livecrootc_p_acc", "I_livecrootn_p_acc"),
    (ILIVECROOT_ST, "I_livecrootc_st_p_acc", "I_livecrootn_st_p_acc"),
    (IDEADCROOT, "I_deadcrootc_p_acc", "I_deadcrootn_p_acc"),
    (IDEADCROOT_ST, "I_deadcrootc_st_p_acc", "I_deadcrootn_st_p_acc"),
    (IGRAIN, "I_grainc_p_acc", "I_grainn_p_acc"),
    (IGRAIN_ST, "I_grainc_st_p_acc", "I_grainn_st_p_acc"),
];

/// DiagMatrix 写出的植被容量（`:444-481`）：`(下标, C 容量, N 容量)`。谷物池没有 `*Cap_p`。
#[rustfmt::skip]
const VEG_CAPACITIES: &[(usize, &str, &str)] = &[
    (ILEAF, "leafcCap_p", "leafnCap_p"),
    (ILEAF_ST, "leafc_storageCap_p", "leafn_storageCap_p"),
    (ILEAF_XF, "leafc_xferCap_p", "leafn_xferCap_p"),
    (IFROOT, "frootcCap_p", "frootnCap_p"),
    (IFROOT_ST, "frootc_storageCap_p", "frootn_storageCap_p"),
    (IFROOT_XF, "frootc_xferCap_p", "frootn_xferCap_p"),
    (ILIVESTEM, "livestemcCap_p", "livestemnCap_p"),
    (ILIVESTEM_ST, "livestemc_storageCap_p", "livestemn_storageCap_p"),
    (ILIVESTEM_XF, "livestemc_xferCap_p", "livestemn_xferCap_p"),
    (IDEADSTEM, "deadstemcCap_p", "deadstemnCap_p"),
    (IDEADSTEM_ST, "deadstemc_storageCap_p", "deadstemn_storageCap_p"),
    (IDEADSTEM_XF, "deadstemc_xferCap_p", "deadstemn_xferCap_p"),
    (ILIVECROOT, "livecrootcCap_p", "livecrootnCap_p"),
    (ILIVECROOT_ST, "livecrootc_storageCap_p", "livecrootn_storageCap_p"),
    (ILIVECROOT_XF, "livecrootc_xferCap_p", "livecrootn_xferCap_p"),
    (IDEADCROOT, "deadcrootcCap_p", "deadcrootnCap_p"),
    (IDEADCROOT_ST, "deadcrootc_storageCap_p", "deadcrootn_storageCap_p"),
    (IDEADCROOT_XF, "deadcrootc_xferCap_p", "deadcrootn_xferCap_p"),
];

/// SASU 用容量替换的植被池（`:482-491`）。
#[rustfmt::skip]
const VEG_SPINUP: &[(usize, &str, &str)] = &[
    (IDEADSTEM, "deadstemc_p", "deadstemn_p"),
    (IDEADSTEM_ST, "deadstemc_storage_p", "deadstemn_storage_p"),
    (IDEADCROOT, "deadcrootc_p", "deadcrootn_p"),
    (IDEADCROOT_ST, "deadcrootc_storage_p", "deadcrootn_storage_p"),
];

/// 土壤池之间的转移：`(接收池, 供给池, C 累加通量)`（`:519-538`）；N 把名字里的 `_c_` 换成 `_n_`。
/// 池名是 `BgcConstants` 里 `i_*` 下标的名字。
#[rustfmt::skip]
const SOIL_TRANSFERS: &[(&str, &str, &str)] = &[
    ("soil1", "met_lit", "AKX_met_to_soil1_c_vr_acc"),
    ("soil1", "cel_lit", "AKX_cel_to_soil1_c_vr_acc"),
    ("soil2", "lig_lit", "AKX_lig_to_soil2_c_vr_acc"),
    ("soil2", "soil1", "AKX_soil1_to_soil2_c_vr_acc"),
    ("cel_lit", "cwd", "AKX_cwd_to_cel_c_vr_acc"),
    ("lig_lit", "cwd", "AKX_cwd_to_lig_c_vr_acc"),
    ("soil3", "soil1", "AKX_soil1_to_soil3_c_vr_acc"),
    ("soil1", "soil2", "AKX_soil2_to_soil1_c_vr_acc"),
    ("soil3", "soil2", "AKX_soil2_to_soil3_c_vr_acc"),
    ("soil1", "soil3", "AKX_soil3_to_soil1_c_vr_acc"),
];

/// 各池的流出通量（`:505-517`）。`cwd` 没有垂直输运项 `diagVX`。
#[rustfmt::skip]
const SOIL_EXITS: &[(&str, &str, bool)] = &[
    ("met_lit", "AKX_met_exit_c_vr_acc", true),
    ("cel_lit", "AKX_cel_exit_c_vr_acc", true),
    ("lig_lit", "AKX_lig_exit_c_vr_acc", true),
    ("cwd", "AKX_cwd_exit_c_vr_acc", false),
    ("soil1", "AKX_soil1_exit_c_vr_acc", true),
    ("soil2", "AKX_soil2_exit_c_vr_acc", true),
    ("soil3", "AKX_soil3_exit_c_vr_acc", true),
];

/// 土壤输入（`:541-545`）。
#[rustfmt::skip]
const SOIL_INPUTS: &[(&str, &str)] = &[
    ("met_lit", "I_met_c_vr_acc"),
    ("cel_lit", "I_cel_c_vr_acc"),
    ("lig_lit", "I_lig_c_vr_acc"),
    ("cwd", "I_cwd_c_vr_acc"),
];

/// 有垂直输运三对角项的池（`:582-...`，`cwd` 除外）。
const SOIL_MIXED: &[&str] = &["met_lit", "cel_lit", "lig_lit", "soil1", "soil2", "soil3"];

/// 年末清零的 PFT 级累加量（`:760-878`）：植被 C/N 输入与全部 `AKX_*_p_acc`。
fn pft_accumulators() -> impl Iterator<Item = &'static str> {
    VEG_INPUTS
        .iter()
        .flat_map(|(_, c, n)| [*c, *n])
        .chain(VEG_C.iter().map(|entry| entry.2))
        .chain(VEG_N.iter().map(|entry| entry.2))
}

/// 年末清零的 patch 级累加量（`:880-960`）。
const PATCH_ACCUMULATORS: &[&str] = &[
    "AKX_met_exit_c_vr_acc",
    "AKX_cel_exit_c_vr_acc",
    "AKX_lig_exit_c_vr_acc",
    "AKX_cwd_exit_c_vr_acc",
    "AKX_soil1_exit_c_vr_acc",
    "AKX_soil2_exit_c_vr_acc",
    "AKX_soil3_exit_c_vr_acc",
    "AKX_met_to_soil1_c_vr_acc",
    "AKX_cel_to_soil1_c_vr_acc",
    "AKX_lig_to_soil2_c_vr_acc",
    "AKX_soil1_to_soil2_c_vr_acc",
    "AKX_cwd_to_cel_c_vr_acc",
    "AKX_cwd_to_lig_c_vr_acc",
    "AKX_soil1_to_soil3_c_vr_acc",
    "AKX_soil2_to_soil1_c_vr_acc",
    "AKX_soil2_to_soil3_c_vr_acc",
    "AKX_soil3_to_soil1_c_vr_acc",
    "AKX_met_exit_n_vr_acc",
    "AKX_cel_exit_n_vr_acc",
    "AKX_lig_exit_n_vr_acc",
    "AKX_cwd_exit_n_vr_acc",
    "AKX_soil1_exit_n_vr_acc",
    "AKX_soil2_exit_n_vr_acc",
    "AKX_soil3_exit_n_vr_acc",
    "AKX_met_to_soil1_n_vr_acc",
    "AKX_cel_to_soil1_n_vr_acc",
    "AKX_lig_to_soil2_n_vr_acc",
    "AKX_soil1_to_soil2_n_vr_acc",
    "AKX_cwd_to_cel_n_vr_acc",
    "AKX_cwd_to_lig_n_vr_acc",
    "AKX_soil1_to_soil3_n_vr_acc",
    "AKX_soil2_to_soil1_n_vr_acc",
    "AKX_soil2_to_soil3_n_vr_acc",
    "AKX_soil3_to_soil1_n_vr_acc",
    "I_met_c_vr_acc",
    "I_cel_c_vr_acc",
    "I_lig_c_vr_acc",
    "I_cwd_c_vr_acc",
    "I_met_n_vr_acc",
    "I_cel_n_vr_acc",
    "I_lig_n_vr_acc",
    "I_cwd_n_vr_acc",
];

/// 按 `(nl_soil, ndecomp_pools)` 存的三对角累加量，上游在清零循环里逐池置 0（每个池都清，含 `cwd`）。
const PATCH_POOL_ACCUMULATORS: &[&str] = &[
    "diagVX_c_vr_acc",
    "upperVX_c_vr_acc",
    "lowerVX_c_vr_acc",
    "diagVX_n_vr_acc",
    "upperVX_n_vr_acc",
    "lowerVX_n_vr_acc",
];

fn field<'a>(s: &'a BgcState, name: &str) -> Result<&'a Vec<f64>> {
    s.f64_field(name)
        .ok_or_else(|| anyhow::anyhow!("CNSASU: BGC state has no {name}"))
}

fn field_mut<'a>(s: &'a mut BgcState, name: &str) -> Result<&'a mut Vec<f64>> {
    s.f64_field_mut(name)
        .ok_or_else(|| anyhow::anyhow!("CNSASU: BGC state has no {name}"))
}

/// 池名 → 0 起的池下标（`BgcConstants::i_*` 是 1 起）。
fn pool(s: &BgcState, name: &str) -> Result<usize> {
    let c = &s.constants;
    let index = match name {
        "met_lit" => c.i_met_lit,
        "cel_lit" => c.i_cel_lit,
        "lig_lit" => c.i_lig_lit,
        "cwd" => c.i_cwd,
        "soil1" => c.i_soil1,
        "soil2" => c.i_soil2,
        "soil3" => c.i_soil3,
        _ => bail!("CNSASU: unknown decomposition pool {name}"),
    };
    usize::try_from(index - 1).map_err(|_| anyhow::anyhow!("CNSASU: pool index {name} = {index}"))
}

/// `CNSASU`。
pub fn cn_sasu(s: &mut BgcState, p: &BgcPhysics, sw: BgcSwitches) -> Result<()> {
    let nl = s.dims.nl_soil;
    let nl_full = s.dims.nl_soil_full;
    let npools = s.dims.ndecomp_pools;
    let npft = s.pft.leafc_p.len();

    // 年初：`idate(3)` 是整型、`deltim` 是实型，比较时整型提升为实型。
    if p.idate[1] == 1 && f64::from(p.idate[2]) == p.deltim {
        for &(start, current) in POOL_STARTS {
            let values: Vec<f64> = field(s, current)?.iter().map(|v| v.max(EPSI)).collect();
            field_mut(s, start)?[..npft].copy_from_slice(&values[..npft]);
        }
        for k in 0..npools {
            for j in 0..nl {
                s.patch.decomp0_cpools_vr[j + k * nl] =
                    s.patch.decomp_cpools_vr[j + k * nl_full].max(EPSI);
                s.patch.decomp0_npools_vr[j + k * nl] =
                    s.patch.decomp_npools_vr[j + k * nl_full].max(EPSI);
            }
        }
    }

    if !is_end_of_year(p.idate, p.deltim) {
        return Ok(());
    }

    for m in 0..npft {
        let (mut c_cap, mut n_cap) = (
            vegetation_capacity(s, m, VEG_C, NVEGC, true)?,
            vegetation_capacity(s, m, VEG_N, NVEGN, false)?,
        );
        for cap in c_cap.iter_mut().chain(n_cap.iter_mut()) {
            if *cap < 0.0 {
                *cap = EPSI;
            }
        }
        if sw.diag_matrix {
            for &(index, c_name, n_name) in VEG_CAPACITIES {
                field_mut(s, c_name)?[m] = c_cap[index];
                field_mut(s, n_name)?[m] = n_cap[index];
            }
        }
        if sw.sasu {
            for &(index, c_name, n_name) in VEG_SPINUP {
                field_mut(s, c_name)?[m] = c_cap[index];
                field_mut(s, n_name)?[m] = n_cap[index];
            }
        }
    }

    let (mut soil_c, mut soil_n) = soil_capacity(s)?;
    for value in soil_c.iter_mut().chain(soil_n.iter_mut()) {
        if *value < 0.0 {
            *value = 0.0;
        }
    }
    if sw.diag_matrix {
        for k in 0..npools {
            for j in 0..nl {
                s.patch.decomp_cpools_vr_Cap[j + k * nl_full] = soil_c[j + k * nl];
                s.patch.decomp_npools_vr_Cap[j + k * nl_full] = soil_n[j + k * nl];
            }
        }
    }
    if sw.sasu {
        let cwd = pool(s, "cwd")?;
        for k in 0..npools {
            for j in 0..nl {
                let at = j + k * nl;
                let (cap_c, cap_n) = (soil_c[at], soil_n[at]);
                let c_high = cap_c / s.patch.decomp0_cpools_vr[at] > 100.0 && cap_c > 1.0e5;
                let n_high = cap_n / s.patch.decomp0_npools_vr[at] > 100.0 && cap_n > 1.0e3;
                // 上游条件是 `(c_high .or. n_high) .or. k == i_cwd .and. (c_high .or. n_high)`
                // （`.and.` 先于 `.or.`），后一半被前一半蕴含，照抄形状。
                if (c_high || n_high) || (k == cwd && (c_high || n_high)) {
                    soil_c[at] = s.patch.decomp_cpools_vr[j + k * nl_full];
                    soil_n[at] = s.patch.decomp_npools_vr[j + k * nl_full];
                }
            }
        }
        if soil_c.iter().any(|v| *v > 1.0e8) || soil_n.iter().any(|v| *v > 1.0e8) {
            for k in 0..npools {
                for j in 0..nl {
                    soil_c[j + k * nl] = s.patch.decomp_cpools_vr[j + k * nl_full];
                    soil_n[j + k * nl] = s.patch.decomp_npools_vr[j + k * nl_full];
                }
            }
        }
        for k in 0..npools {
            for j in 0..nl {
                let full = j + k * nl_full;
                s.patch.decomp_cpools_vr[full] = soil_c[j + k * nl];
                s.patch.decomp_npools_vr[full] = if s.invariants.floating_cn_ratio[k] {
                    soil_n[j + k * nl]
                } else {
                    s.patch.decomp_cpools_vr[full] / s.patch.cn_decomp_pools[j + k * nl]
                };
            }
        }
        s.patch.skip_balance_check[0] = true;
    }

    // 年末清零全部累加量。
    for name in pft_accumulators() {
        field_mut(s, name)?[..npft].fill(0.0);
    }
    for name in PATCH_ACCUMULATORS {
        field_mut(s, name)?[..nl].fill(0.0);
    }
    for name in PATCH_POOL_ACCUMULATORS {
        field_mut(s, name)?[..nl * npools].fill(0.0);
    }
    Ok(())
}

/// 一个 PFT 的植被容量 `−A⁻¹·I`（`:271-438`）。
fn vegetation_capacity(
    s: &BgcState,
    m: usize,
    entries: &[(usize, usize, &str, &str, bool)],
    n: usize,
    carbon: bool,
) -> Result<Vec<f64>> {
    let mut a = vec![0.0; n * n];
    for &(row, col, flux, start, exit) in entries {
        let ratio = field(s, flux)?[m] / field(s, start)?[m];
        a[row + col * n] = if exit { -ratio } else { ratio };
    }
    let mut input = vec![0.0; n];
    for &(index, c_name, n_name) in VEG_INPUTS {
        input[index] = field(s, if carbon { c_name } else { n_name })?[m];
    }
    // `-0.0 .eq. 0` 为真，与 Rust 的 `==` 一致。
    for j in 0..n {
        if a[j + j * n] == 0.0 {
            a[j + j * n] = -1.0e36;
        }
    }
    Ok(negated_matmul(&inverse(&a, n)?, &input, n))
}

/// 土壤容量（`:494-665`）：`nl_soil·ndecomp_pools` 阶的块三对角矩阵，行/列下标 `pool·nl + j`。
fn soil_capacity(s: &BgcState) -> Result<(Vec<f64>, Vec<f64>)> {
    let nl = s.dims.nl_soil;
    let npools = s.dims.ndecomp_pools;
    let n = nl * npools;
    let v = &s.patch;
    let (mut ac, mut an) = (vec![0.0; n * n], vec![0.0; n * n]);
    let (mut ic, mut inn) = (vec![0.0; n], vec![0.0; n]);
    let at = |row: usize, col: usize| row + col * n;
    for j in 0..nl {
        for &(name, flux, mixed) in SOIL_EXITS {
            let k = pool(s, name)?;
            let d = j + k * nl;
            let flux_n = flux.replace("_c_", "_n_");
            let (exit_c, exit_n) = (field(s, flux)?[j], field(s, &flux_n)?[j]);
            // `-(AKX_exit + diagVX)/pool0`；`cwd` 只有 `-AKX_exit/pool0`。
            let (sum_c, sum_n) = if mixed {
                (exit_c + v.diagVX_c_vr_acc[d], exit_n + v.diagVX_n_vr_acc[d])
            } else {
                (exit_c, exit_n)
            };
            ac[at(d, d)] = -(sum_c / v.decomp0_cpools_vr[d]);
            an[at(d, d)] = -(sum_n / v.decomp0_npools_vr[d]);
        }
        for &(receiver, donor, flux) in SOIL_TRANSFERS {
            let (r, dnr) = (pool(s, receiver)?, pool(s, donor)?);
            let (row, col) = (j + r * nl, j + dnr * nl);
            ac[at(row, col)] = field(s, flux)?[j] / v.decomp0_cpools_vr[col];
            an[at(row, col)] = field(s, &flux.replace("_c_", "_n_"))?[j] / v.decomp0_npools_vr[col];
        }
        for &(name, flux) in SOIL_INPUTS {
            ic[j + pool(s, name)? * nl] = field(s, flux)?[j];
        }
    }
    for j in 0..nl.saturating_sub(1) {
        for &name in SOIL_MIXED {
            let k = pool(s, name)?;
            let (upper, lower) = (j + k * nl, j + 1 + k * nl);
            // 上三对角：`upperVX(j)/pool0(j+1)`；下三对角：`lowerVX(j+1)/pool0(j)`。
            ac[at(upper, lower)] = v.upperVX_c_vr_acc[upper] / v.decomp0_cpools_vr[lower];
            an[at(upper, lower)] = v.upperVX_n_vr_acc[upper] / v.decomp0_npools_vr[lower];
            ac[at(lower, upper)] = v.lowerVX_c_vr_acc[lower] / v.decomp0_cpools_vr[upper];
            an[at(lower, upper)] = v.lowerVX_n_vr_acc[lower] / v.decomp0_npools_vr[upper];
        }
        // 上游把 "N input" 写进了 `DO j = 1, nl_soil-1`（`:640-644`），最底层的 N 输入从未赋值、
        // 保持 0 —— C 的输入在 `DO j = 1, nl_soil` 里。底层 N 容量因此偏小几个量级，照样复现。
        for &(name, flux) in SOIL_INPUTS {
            inn[j + pool(s, name)? * nl] = field(s, &flux.replace("_c_", "_n_"))?[j];
        }
    }
    for matrix in [&mut ac, &mut an] {
        for k in 0..n {
            if matrix[at(k, k)].abs() <= EPSI {
                matrix[at(k, k)] = -1.0e36;
            }
        }
    }
    Ok((
        negated_matmul(&inverse(&ac, n)?, &ic, n),
        negated_matmul(&inverse(&an, n)?, &inn, n),
    ))
}

/// `-matmul(A, b)`：gfortran 内联成外层列、内层行的 FMA 累加（见文件头）。
fn negated_matmul(a: &[f64], b: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n];
    for j in 0..n {
        for i in 0..n {
            out[i] = a[i + j * n].mul_add(b[j], out[i]);
        }
    }
    out.iter_mut().for_each(|v| *v = -*v);
    out
}

/// `inverse`（`:964-1070`）：Doolittle LU 分解后逐列回代。对角元为 0 时上游 `abort`。
pub(crate) fn inverse(a: &[f64], n: usize) -> Result<Vec<f64>> {
    for k in 0..n {
        if a[k + k * n] == 0.0 {
            bail!("CNSASU inverse: zero pivot a({0},{0})", k + 1);
        }
    }
    let at = |row: usize, col: usize| row + col * n;
    let mut aa = a.to_vec();
    let mut l = vec![0.0; n * n];
    // Step 1：`aa(i,j) = aa(i,j) − coeff·aa(k,j)` 是 `.FNMA`。
    for k in 0..n.saturating_sub(1) {
        for i in k + 1..n {
            let coeff = aa[at(i, k)] / aa[at(k, k)];
            l[at(i, k)] = coeff;
            for j in k + 1..n {
                aa[at(i, j)] = (-coeff).mul_add(aa[at(k, j)], aa[at(i, j)]);
            }
        }
    }
    // Step 3：逐列解 `L·d = e_k`、`U·x = d`（`U` 即 `aa` 的上三角），两处累减都是 `.FNMA`。
    let mut c = vec![0.0; n * n];
    let (mut b, mut d, mut x) = (vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    for k in 0..n {
        b[k] = 1.0;
        d[0] = b[0];
        for i in 1..n {
            d[i] = b[i];
            for j in 0..i {
                d[i] = (-l[at(i, j)]).mul_add(d[j], d[i]);
            }
        }
        x[n - 1] = d[n - 1] / aa[at(n - 1, n - 1)];
        for i in (0..n - 1).rev() {
            x[i] = d[i];
            for j in (i + 1..n).rev() {
                x[i] = (-aa[at(i, j)]).mul_add(x[j], x[i]);
            }
            x[i] /= aa[at(i, i)];
        }
        c[k * n..(k + 1) * n].copy_from_slice(&x);
        b[k] = 0.0;
    }
    Ok(c)
}

#[cfg(test)]
#[path = "bgc_sasu_tests.rs"]
mod bgc_sasu_tests;
