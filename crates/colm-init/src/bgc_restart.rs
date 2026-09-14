//! BGC constant restart output from `mkinidata/MOD_Initialize.F90`.
//!
//! The BGC time state remains a separate restart family.  This module only
//! serializes the deterministic invariant values and their per-patch transition
//! matrices, so callers cannot mistake it for a complete BGC cold start.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::restart::validate_restart_compression;

const SOIL_LAYERS: usize = 10;
const TRANSITIONS: usize = 10;
const POOLS: usize = 7;
const MISSING_I32: i32 = -9_999;

/// The two paths emitted by CoLM's `WRITE_BGCTimeInvariants`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgcConstantRestartFiles {
    pub constants: PathBuf,
    pub block: PathBuf,
}

/// Writes the BGC invariant restart pair for a fresh cold start.
///
/// `use_nitrification` controls only the Fortran `nfix_timeconst` default; the
/// BGC time-state writer is deliberately not implied by this function.
pub fn write_cold_start_bgc_constant_restart(
    restart_dir: impl AsRef<Path>,
    case_name: &str,
    land_cover_year: i32,
    block_label: &str,
    patches: usize,
    use_nitrification: bool,
    compression_level: u8,
) -> Result<BgcConstantRestartFiles> {
    validate_restart_compression(compression_level)?;
    validate_filename_component(case_name, "case name")?;
    validate_filename_component(block_label, "block label")?;
    ensure!(
        (0..=9999).contains(&land_cover_year),
        "land-cover year {land_cover_year} is outside the four-digit restart filename range"
    );
    ensure!(patches > 0, "a BGC restart block needs at least one patch");

    let constants_dir = restart_dir.as_ref().join("const");
    std::fs::create_dir_all(&constants_dir)
        .with_context(|| format!("cannot create {}", constants_dir.display()))?;
    let stem = format!("{case_name}_restart_bgc_const_lc{land_cover_year:04}");
    let constants = constants_dir.join(format!("{stem}.nc"));
    let block = constants_dir.join(format!("{stem}_{block_label}.nc"));
    write_constants(&constants, use_nitrification)?;
    write_block(&block, patches, compression_level)?;
    Ok(BgcConstantRestartFiles { constants, block })
}

fn write_constants(path: &Path, use_nitrification: bool) -> Result<()> {
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create BGC constant restart {}", path.display()))?;
    file.add_dimension("ndecomp_transitions", TRANSITIONS)?;
    file.add_dimension("ndecomp_pools", POOLS)?;
    file.add_dimension("nlitter_fire", 2)?;

    put_i32_1d(
        &mut file,
        "donor_pool",
        "ndecomp_transitions",
        &[1, 2, 3, 5, 4, 4, 5, 6, 6, 7],
    )?;
    put_i32_1d(
        &mut file,
        "receiver_pool",
        "ndecomp_transitions",
        &[5, 5, 6, 6, 2, 3, 7, 5, 7, 5],
    )?;
    put_i8_1d(
        &mut file,
        "floating_cn_ratio",
        "ndecomp_pools",
        &[1, 1, 1, 1, 0, 0, 0],
    )?;
    put_f64_1d(
        &mut file,
        "initial_cn_ratio",
        "ndecomp_pools",
        &[90.0, 90.0, 90.0, 90.0, 8.0, 11.0, 11.0],
    )?;
    put_i8_1d(&mut file, "is_cwd", "ndecomp_pools", &[0, 0, 0, 1, 0, 0, 0])?;
    put_i8_1d(
        &mut file,
        "is_litter",
        "ndecomp_pools",
        &[1, 1, 1, 0, 0, 0, 0],
    )?;
    put_i8_1d(
        &mut file,
        "is_soil",
        "ndecomp_pools",
        &[0, 0, 0, 0, 1, 1, 1],
    )?;
    put_f64_1d(&mut file, "cmb_cmplt_fact", "nlitter_fire", &[0.5, 0.25])?;

    for (name, value) in [
        ("i_met_lit", 1),
        ("i_cel_lit", 2),
        ("i_lig_lit", 3),
        ("i_cwd", 4),
        ("i_soil1", 5),
        ("i_soil2", 6),
        ("i_soil3", 7),
        ("i_atm", 0),
    ] {
        file.add_variable::<i32>(name, &[])?
            .put_values(&[value], ..)?;
    }
    let nfix_timeconst = if use_nitrification { 10.0 } else { 0.0 };
    for (name, value) in [
        ("nitrif_n2o_loss_frac", 6.0e-4),
        ("dnp", 0.01),
        ("bdnr", 0.5),
        ("compet_plant_no3", 1.0),
        ("compet_plant_nh4", 1.0),
        ("compet_decomp_no3", 1.0),
        ("compet_decomp_nh4", 1.0),
        ("compet_denit", 1.0),
        ("compet_nit", 1.0),
        ("surface_tension_water", 0.073),
        ("rij_kro_a", 1.5e-10),
        ("rij_kro_alpha", 1.26),
        ("rij_kro_beta", 0.6),
        ("rij_kro_gamma", 0.6),
        ("rij_kro_delta", 0.85),
        ("nfix_timeconst", nfix_timeconst),
        ("organic_max", 130.0),
        ("d_con_g21", 0.1759),
        ("d_con_g22", 0.00117),
        ("d_con_w21", 1.172),
        ("d_con_w22", 0.03443),
        ("d_con_w23", 0.0005048),
        ("denit_resp_coef", 0.1),
        ("denit_resp_exp", 1.3),
        ("denit_nitrate_coef", 1.15),
        ("denit_nitrate_exp", 0.57),
        ("k_nitr_max", 1.1574074e-6),
        ("Q10", 1.5),
        ("froz_q10", 1.5),
        ("tau_l1", 1.0 / 18.5),
        ("tau_l2_l3", 1.0 / 4.9),
        ("tau_s1", 1.0 / 7.3),
        ("tau_s2", 1.0 / 0.2),
        ("tau_s3", 1.0 / 0.0045),
        ("tau_cwd", 1.0 / 0.3),
        ("lwtop", 0.7 / 31_536_000.0),
        ("som_adv_flux", 0.0),
        ("som_diffus", 3.170_979_198_376_459e-12),
        ("cryoturb_diffusion_k", 1.585_489_599_188_229e-11),
        ("max_altdepth_cryoturbation", 2.0),
        ("max_depth_cryoturb", 3.0),
        ("am", 0.02),
        ("br", 2.525e-6),
        ("br_root", 2.0e-6),
        ("fstor2tran", 0.5),
        ("ndays_on", 30.0),
        ("ndays_off", 15.0),
        ("crit_dayl", 39_300.0),
        ("crit_onset_fdd", 15.0),
        ("crit_onset_swi", 15.0),
        ("crit_offset_fdd", 15.0),
        ("crit_offset_swi", 15.0),
        ("soilpsi_on", -0.6),
        ("soilpsi_off", -0.8),
        ("occur_hi_gdp_tree", 0.39),
        ("lfuel", 75.0),
        ("ufuel", 650.0),
        ("cropfire_a1", 0.3),
        ("borealat", 40.0 / std::f64::consts::PI),
        ("troplat", 23.5 / std::f64::consts::PI),
        ("non_boreal_peatfire_c", 0.001),
        ("boreal_peatfire_c", 4.2e-5),
        ("rh_low", 30.0),
        ("rh_hgh", 80.0),
        ("bt_min", 0.3),
        ("bt_max", 0.7),
        ("pot_hmn_ign_counts_alpha", 0.0035),
        ("g0_fire", 0.05),
        ("sf", 0.1),
        ("sf_no3", 1.0),
    ] {
        file.add_variable::<f64>(name, &[])?
            .put_values(&[value], ..)?;
    }
    file.close()
        .with_context(|| format!("cannot close BGC constant restart {}", path.display()))?;
    Ok(())
}

fn write_block(path: &Path, patches: usize, compression_level: u8) -> Result<()> {
    validate_restart_compression(compression_level)?;
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create BGC constant block {}", path.display()))?;
    file.add_dimension("patch", patches)?;
    file.add_dimension("soil", SOIL_LAYERS)?;
    file.add_dimension("ndecomp_transitions", TRANSITIONS)?;
    let transfer = transfer_coefficients();
    let mut rf_decomp = Vec::with_capacity(patches * TRANSITIONS * SOIL_LAYERS);
    let mut pathfrac = Vec::with_capacity(rf_decomp.capacity());
    for _ in 0..patches {
        for transition in 0..TRANSITIONS {
            rf_decomp.extend(std::iter::repeat_n(transfer.rf[transition], SOIL_LAYERS));
            pathfrac.extend(std::iter::repeat_n(
                transfer.pathfrac[transition],
                SOIL_LAYERS,
            ));
        }
    }
    let mut rf =
        file.add_variable::<f64>("rf_decomp", &["patch", "ndecomp_transitions", "soil"])?;
    rf.set_compression(compression_level.into(), false)?;
    rf.put_values(&rf_decomp, (.., .., ..))?;
    let mut pathfrac_var =
        file.add_variable::<f64>("pathfrac_decomp", &["patch", "ndecomp_transitions", "soil"])?;
    pathfrac_var.set_compression(compression_level.into(), false)?;
    pathfrac_var.put_values(&pathfrac, (.., .., ..))?;
    let mut rice = file.add_variable::<i32>("rice2pdt", &["patch"])?;
    rice.set_compression(compression_level.into(), false)?;
    rice.put_values(&vec![MISSING_I32; patches], ..)?;
    file.close()
        .with_context(|| format!("cannot close BGC constant block {}", path.display()))?;
    Ok(())
}

struct TransferCoefficients {
    rf: [f64; TRANSITIONS],
    pathfrac: [f64; TRANSITIONS],
}

fn transfer_coefficients() -> TransferCoefficients {
    let turnover = 0.85 - 0.68 * 0.01 * (100.0 - 50.0);
    let soil1_to_soil3 = 0.004 / (1.0 - turnover);
    TransferCoefficients {
        rf: [
            0.55, 0.5, 0.5, turnover, 0.0, 0.0, turnover, 0.55, 0.55, 0.55,
        ],
        pathfrac: [
            1.0,
            1.0,
            1.0,
            1.0 - soil1_to_soil3,
            0.76,
            0.24,
            soil1_to_soil3,
            0.42 / 0.45,
            0.03 / 0.45,
            1.0,
        ],
    }
}

fn put_i32_1d(
    file: &mut netcdf::FileMut,
    name: &str,
    dimension: &str,
    values: &[i32],
) -> Result<()> {
    file.add_variable::<i32>(name, &[dimension])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_i8_1d(file: &mut netcdf::FileMut, name: &str, dimension: &str, values: &[i8]) -> Result<()> {
    file.add_variable::<i8>(name, &[dimension])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_f64_1d(
    file: &mut netcdf::FileMut,
    name: &str,
    dimension: &str,
    values: &[f64],
) -> Result<()> {
    file.add_variable::<f64>(name, &[dimension])?
        .put_values(values, ..)?;
    Ok(())
}

fn validate_filename_component(value: &str, name: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "{name} must be a nonempty filename component"
    );
    Ok(())
}

#[cfg(test)]
#[path = "bgc_restart_tests.rs"]
mod tests;
