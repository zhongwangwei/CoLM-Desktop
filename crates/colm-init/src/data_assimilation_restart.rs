//! Data-assimilation restart output for `mkinidata`.
//!
//! Upstream initializes every ensemble member from the same common cold-start
//! state.  Reusing the common restart keeps this path bit-for-bit aligned with
//! the already-validated initializer instead of duplicating its physics.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use netcdf::types::{FloatType, NcVariableType};

use crate::restart::validate_restart_compression;

const VARIABLES: &[&str] = &[
    "z_sno",
    "dz_sno",
    "t_soisno",
    "wliq_soisno",
    "wice_soisno",
    "smp",
    "hk",
    "t_grnd",
    "tleaf",
    "ldew",
    "ldew_rain",
    "ldew_snow",
    "fwet_snow",
    "sag",
    "scv",
    "snowdp",
    "fveg",
    "fsno",
    "sigf",
    "green",
    "tlai",
    "lai",
    "tsai",
    "sai",
    "alb",
    "ssun",
    "ssha",
    "ssoi",
    "ssno",
    "thermk",
    "extkb",
    "extkd",
    "zwt",
    "wdsrf",
    "wa",
    "wetwat",
    "t_lake",
    "lake_icefrc",
    "savedtke1",
];

const DIMENSIONS: &[&str] = &[
    "patch", "snow", "snowp1", "soilsnow", "soil", "lake", "band", "rtyp",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataAssimilationRestartFile {
    pub path: PathBuf,
}

/// Returns the block filename used by upstream's vector NetCDF writer.
pub fn data_assimilation_restart_path(common_restart: impl AsRef<Path>) -> Result<PathBuf> {
    let common_restart = common_restart.as_ref();
    let name = common_restart
        .file_name()
        .and_then(|name| name.to_str())
        .context("common restart path has no UTF-8 filename")?;
    let marker = "_restart_";
    let offset = name
        .rfind(marker)
        .context("common restart filename is missing _restart_")?;
    let mut output = name.to_owned();
    output.insert_str(offset + "_restart".len(), "_DA");
    Ok(common_restart.with_file_name(output))
}

/// Replicates a common cold-start restart into every DA ensemble member.
pub fn write_data_assimilation_restart(
    common_restart: impl AsRef<Path>,
    ensemble_members: usize,
    compression_level: u8,
) -> Result<DataAssimilationRestartFile> {
    ensure!(ensemble_members > 0, "DA ensemble count must be positive");
    validate_restart_compression(compression_level)?;
    let common_restart = common_restart.as_ref();
    let source = netcdf::open(common_restart)
        .with_context(|| format!("cannot open common restart {}", common_restart.display()))?;
    let patches = source
        .dimension_len("patch")
        .context("common restart is missing patch dimension")?;
    ensure!(
        patches > 0,
        "common restart patch dimension must be positive"
    );
    let dimensions = DIMENSIONS
        .iter()
        .map(|&name| {
            source
                .dimension_len(name)
                .with_context(|| format!("common restart is missing {name} dimension"))
                .map(|length| (name, length))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut fields = Vec::with_capacity(VARIABLES.len());
    for &name in VARIABLES {
        let variable = source
            .variable(name)
            .with_context(|| format!("common restart is missing {name}"))?;
        ensure!(
            variable.vartype() == NcVariableType::Float(FloatType::F64),
            "common restart variable {name} must be f64"
        );
        let dimensions = variable.dimensions();
        ensure!(
            dimensions
                .first()
                .is_some_and(|dimension| dimension.name() == "patch"),
            "common restart variable {name} must be patch-first"
        );
        ensure!(
            dimensions
                .first()
                .is_some_and(|dimension| dimension.len() == patches),
            "common restart variable {name} has the wrong patch length"
        );
        let tail_dimensions = dimensions[1..]
            .iter()
            .map(|dimension| dimension.name())
            .collect::<Vec<_>>();
        let tail = dimensions[1..]
            .iter()
            .map(|dimension| dimension.len())
            .product::<usize>();
        fields.push((name, tail_dimensions, tail));
    }

    let path = data_assimilation_restart_path(common_restart)?;
    let mut output = netcdf::create(&path)
        .with_context(|| format!("cannot create DA restart {}", path.display()))?;
    for (name, length) in dimensions {
        output.add_dimension(name, length)?;
    }
    output.add_dimension("ens", ensemble_members)?;

    for (name, tail_dimensions, tail) in fields {
        let values = source
            .variable(name)
            .expect("validated DA source variable disappeared")
            .get_values::<f64, _>(..)?;
        let mut dimension_names = Vec::with_capacity(tail_dimensions.len() + 2);
        dimension_names.push("patch");
        dimension_names.push("ens");
        dimension_names.extend(tail_dimensions.iter().map(String::as_str));
        let mut replicated = Vec::with_capacity(values.len() * ensemble_members);
        for patch in 0..patches {
            let values = &values[patch * tail..(patch + 1) * tail];
            for _ in 0..ensemble_members {
                replicated.extend_from_slice(values);
            }
        }
        let mut variable = output.add_variable::<f64>(name, &dimension_names)?;
        variable.set_compression(compression_level.into(), false)?;
        variable.put_values(&replicated, ..)?;
    }
    output
        .close()
        .with_context(|| format!("cannot close DA restart {}", path.display()))?;
    Ok(DataAssimilationRestartFile { path })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ensemble_member_is_an_exact_common_restart_copy() {
        let root = std::env::temp_dir().join(format!(
            "colm-da-restart-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let common = root.join("case_restart_2005-001-00000_lc2005_x001_y001.nc");
        let mut file = netcdf::create(&common).unwrap();
        for (name, length) in [
            ("patch", 2),
            ("snow", 5),
            ("snowp1", 6),
            ("soilsnow", 15),
            ("soil", 10),
            ("lake", 10),
            ("band", 2),
            ("rtyp", 2),
        ] {
            file.add_dimension(name, length).unwrap();
        }
        for &name in VARIABLES {
            let tail = match name {
                "z_sno" | "dz_sno" => Some("snow"),
                "t_soisno" | "wliq_soisno" | "wice_soisno" => Some("soilsnow"),
                "smp" | "hk" => Some("soil"),
                "t_lake" | "lake_icefrc" => Some("lake"),
                _ => None,
            };
            let dimensions = if matches!(name, "alb" | "ssun" | "ssha" | "ssoi" | "ssno") {
                vec!["patch", "rtyp", "band"]
            } else if let Some(tail) = tail {
                vec!["patch", tail]
            } else {
                vec!["patch"]
            };
            let entries = dimensions
                .iter()
                .map(|dimension| file.dimension_len(dimension).unwrap())
                .product::<usize>();
            let values = (0..entries)
                .map(|index| index as f64 + 0.25)
                .collect::<Vec<_>>();
            file.add_variable::<f64>(name, &dimensions)
                .unwrap()
                .put_values(&values, ..)
                .unwrap();
        }
        file.close().unwrap();

        let written = write_data_assimilation_restart(&common, 3, 1).unwrap();
        assert_eq!(
            written.path,
            root.join("case_restart_DA_2005-001-00000_lc2005_x001_y001.nc")
        );
        let file = netcdf::open(&written.path).unwrap();
        assert_eq!(file.dimension_len("ens"), Some(3));
        let variable = file.variable("alb").unwrap();
        assert_eq!(
            variable
                .dimensions()
                .iter()
                .map(|dimension| dimension.name())
                .collect::<Vec<_>>(),
            ["patch", "ens", "rtyp", "band"]
        );
        assert_eq!(
            variable.get_values::<f64, _>(..).unwrap(),
            vec![
                0.25, 1.25, 2.25, 3.25, 0.25, 1.25, 2.25, 3.25, 0.25, 1.25, 2.25, 3.25, 4.25, 5.25,
                6.25, 7.25, 4.25, 5.25, 6.25, 7.25, 4.25, 5.25, 6.25, 7.25,
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_an_empty_ensemble() {
        let error = write_data_assimilation_restart("missing.nc", 0, 0).unwrap_err();
        assert!(error.to_string().contains("positive"));
    }
}
