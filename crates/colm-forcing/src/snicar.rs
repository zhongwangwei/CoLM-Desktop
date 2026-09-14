//! Native SNICAR optical and grain-aging tables, shared by cold start/runtime.

use std::path::Path;

use anyhow::{ensure, Context, Result};
use colm_core::{SnicarAgingTable, SnicarOptics, SnicarSpectralTable};

use crate::high_res::read_float_variable;

/// Reads the six ice fields and eight bulk aerosol species used by CoLM.
/// Ice arrays retain C `(band, radius)` = `(5,1471)` order.
pub fn read_snicar_optics(path: impl AsRef<Path>) -> Result<SnicarOptics> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open SNICAR optics {}", path.display()))?;
    let ice = |suffix| -> Result<SnicarSpectralTable> {
        SnicarSpectralTable::new(
            read_table(&file, &format!("ss_alb_ice_{suffix}"), &[5, 1471])?,
            read_table(&file, &format!("asm_prm_ice_{suffix}"), &[5, 1471])?,
            read_table(&file, &format!("ext_cff_mss_ice_{suffix}"), &[5, 1471])?,
        )
    };
    // Original SnowOptics_init reads these even with the compile-time default
    // atmosphere, whose solver weights are constants rather than these tables.
    for (name, shape) in [
        ("flx_wgt_dir", &[5, 90, 6][..]),
        ("flx_wgt_dif", &[5, 6][..]),
    ] {
        let values = read_table(&file, name, shape)?;
        ensure!(
            values.iter().all(|&x| (0.0..=1.0).contains(&x)),
            "SNICAR {name} weights must be in [0,1]"
        );
    }
    let mut ssa = [[0.0; 5]; 8];
    let mut asymmetry = [[0.0; 5]; 8];
    let mut extinction = [[0.0; 5]; 8];
    for (species, suffix) in [
        "bcphil", "bcphob", "ocphil", "ocphob", "dust01", "dust02", "dust03", "dust04",
    ]
    .iter()
    .enumerate()
    {
        ssa[species].copy_from_slice(&read_table(&file, &format!("ss_alb_{suffix}"), &[5])?);
        asymmetry[species].copy_from_slice(&read_table(&file, &format!("asm_prm_{suffix}"), &[5])?);
        extinction[species].copy_from_slice(&read_table(
            &file,
            &format!("ext_cff_mss_{suffix}"),
            &[5],
        )?);
    }
    SnicarOptics::new(ice("drc")?, ice("dfs")?, ssa, asymmetry, extinction)
}

/// Reads `SnowAge_init` fields in C `(T, dTdz, density)` = `(11,31,8)` order.
pub fn read_snicar_aging(path: impl AsRef<Path>) -> Result<SnicarAgingTable> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open SNICAR aging {}", path.display()))?;
    SnicarAgingTable::new(
        read_table(&file, "tau", &[11, 31, 8])?,
        read_table(&file, "kappa", &[11, 31, 8])?,
        read_table(&file, "drdsdt0", &[11, 31, 8])?,
    )
}

fn read_table(file: &netcdf::File, name: &str, expected: &[usize]) -> Result<Vec<f64>> {
    let (dimensions, values) = read_float_variable(file, name)?;
    ensure!(
        dimensions == expected,
        "SNICAR {name} has dimensions {dimensions:?}; expected {expected:?}"
    );
    let variable = file
        .variable(name)
        .context("SNICAR table variable disappeared")?;
    for attribute in ["_FillValue", "missing_value"] {
        if let Some(value) = variable.attribute_value(attribute) {
            let missing = f64::try_from(value?)
                .with_context(|| format!("SNICAR {name} {attribute} must be numeric"))?;
            ensure!(
                !values.contains(&missing),
                "SNICAR {name} contains missing table values"
            );
        }
    }
    Ok(values)
}

#[cfg(test)]
#[path = "snicar_tests.rs"]
mod tests;
