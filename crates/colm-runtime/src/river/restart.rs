//! 河道续跑文件 `<case>_restart_gridriver_<date>_lc<year>.nc`（`READ/WRITE_GridRiverLakeTimeVars`）。
//!
//! schema 2（无分汊、水库、示踪物）：标记、`gridriver_ucatch_identity`、
//! `wdsrf_ucat`、`veloc_riv`、`acctime_rnof`、`acc_rnof_uc`、`volwater_ucat`，
//! 开堤防时再加 `levsto`（`gridriver_restart_feature_levee = 1`）。

use std::path::Path;

use anyhow::{ensure, Context, Result};

use super::network::RiverNetwork;
use super::{RiverHistory, RiverState};

const SCHEMA: i32 = 2;
const IDENTITY_VERSION: f64 = 1.0;

fn read_vector(file: &netcdf::File, name: &str, n: usize, path: &Path) -> Result<Vec<f64>> {
    let values = file
        .variable(name)
        .with_context(|| format!("{} has no {name}", path.display()))?
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    ensure!(
        values.len() == n,
        "{name} in {} has {} values for {n} unit catchments",
        path.display(),
        values.len()
    );
    Ok(values)
}

/// 读回河道状态（冷启动文件与续跑文件同形）。
pub fn read_river_state(path: &Path, network: &RiverNetwork) -> Result<RiverState> {
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let n = network.len();
    let scalar_i32 = |name: &str| -> Result<i32> {
        let values = file
            .variable(name)
            .with_context(|| format!("{} has no {name}", path.display()))?
            .get_values::<i32, _>(..)?;
        values
            .first()
            .copied()
            .with_context(|| format!("{name} in {} is empty", path.display()))
    };
    ensure!(
        scalar_i32("gridriver_restart_schema")? == SCHEMA,
        "{} is not a schema-2 GridRiverLake restart",
        path.display()
    );
    ensure!(
        scalar_i32("gridriver_restart_complete")? == 1,
        "{} is an uncommitted GridRiverLake restart",
        path.display()
    );
    ensure!(
        scalar_i32("gridriver_restart_feature_bifurcation")? == 0,
        "{} was written with bifurcation, which the Rust river model does not run yet",
        path.display()
    );
    let levee_flag = scalar_i32("gridriver_restart_feature_levee")?;
    ensure!(
        levee_flag == 0 || levee_flag == 1,
        "{} has an invalid levee feature flag",
        path.display()
    );
    // `validate_gridriver_ucatch_identity`：网络必须与写续跑时相同。
    let identity = file
        .variable("gridriver_ucatch_identity")
        .with_context(|| format!("{} has no gridriver_ucatch_identity", path.display()))?
        .get_values::<f64, _>(..)?;
    ensure!(
        identity.len() == 4 * n,
        "{} was written for a different unit-catchment network",
        path.display()
    );
    for i in 0..n {
        let next = network.next[i];
        let next_id = if next >= 0 { next + 1 } else { next };
        ensure!(
            identity[4 * i] == IDENTITY_VERSION
                && identity[4 * i + 1] == f64::from(network.x[i])
                && identity[4 * i + 2] == f64::from(network.y[i])
                && identity[4 * i + 3] == f64::from(next_id),
            "{} was written for a different unit-catchment network (catchment {})",
            path.display(),
            i + 1
        );
    }
    let acctime = file
        .variable("acctime_rnof")
        .with_context(|| format!("{} has no acctime_rnof", path.display()))?
        .get_values::<f64, _>(..)?;
    let state = RiverState {
        wdsrf: read_vector(&file, "wdsrf_ucat", n, path)?,
        veloc: read_vector(&file, "veloc_riv", n, path)?,
        acc_rnof: read_vector(&file, "acc_rnof_uc", n, path)?,
        volwater: read_vector(&file, "volwater_ucat", n, path)?,
        acctime_rnof: *acctime
            .first()
            .with_context(|| format!("acctime_rnof in {} is empty", path.display()))?,
        // `read_levee_restart`：标记为 1 时 `levsto` 必须在；标记为 0 时即使有也不读。
        levsto: if levee_flag == 1 {
            Some(read_vector(&file, "levsto", n, path)?)
        } else {
            None
        },
        levdph: None,
    };
    if let Some(levsto) = &state.levsto {
        ensure!(
            levsto.iter().all(|v| v.is_finite() && *v >= 0.0),
            "levsto in {} contains a negative or non-finite value",
            path.display()
        );
    }
    ensure!(
        state.wdsrf.iter().all(|w| w.is_finite() && *w >= 0.0)
            && state.veloc.iter().all(|v| v.is_finite() && v.abs() <= 50.0)
            && state.acc_rnof.iter().all(|a| a.is_finite())
            && state.volwater.iter().all(|v| v.is_finite() && *v >= 0.0)
            && state.acctime_rnof.is_finite()
            && state.acctime_rnof >= 0.0,
        "invalid GridRiverLake restart base state in {}",
        path.display()
    );
    Ok(state)
}

/// 写河道续跑文件；先写 `gridriver_restart_complete = 0`，全部写完再改成 1。
pub fn write_river_state(
    path: &Path,
    network: &RiverNetwork,
    state: &RiverState,
    compression_level: u8,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let n = network.len();
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("ucatch", n)?;
    for (name, value) in [
        ("gridriver_restart_schema", SCHEMA),
        ("gridriver_restart_complete", 0),
        ("gridriver_restart_feature_bifurcation", 0),
        (
            "gridriver_restart_feature_levee",
            i32::from(state.levsto.is_some()),
        ),
    ] {
        file.add_variable::<i32>(name, &[])?
            .put_values(&[value], ..)?;
    }
    file.add_dimension("gridriver_ucatch_identity_field", 4)?;
    let mut identity = Vec::with_capacity(4 * n);
    for i in 0..n {
        let next = network.next[i];
        identity.extend([
            IDENTITY_VERSION,
            f64::from(network.x[i]),
            f64::from(network.y[i]),
            f64::from(if next >= 0 { next + 1 } else { next }),
        ]);
    }
    let vector = |file: &mut netcdf::FileMut,
                  name: &str,
                  dims: &[&str],
                  values: &[f64],
                  missing: bool|
     -> Result<()> {
        let mut variable = file.add_variable::<f64>(name, dims)?;
        if compression_level > 0 {
            variable.set_compression(i32::from(compression_level), false)?;
        }
        if missing {
            variable.put_attribute("missing_value", colm_core::MISSING)?;
        }
        variable.put_values(values, ..)?;
        Ok(())
    };
    vector(
        &mut file,
        "gridriver_ucatch_identity",
        &["ucatch", "gridriver_ucatch_identity_field"],
        &identity,
        false,
    )?;
    vector(&mut file, "wdsrf_ucat", &["ucatch"], &state.wdsrf, true)?;
    vector(&mut file, "veloc_riv", &["ucatch"], &state.veloc, true)?;
    file.add_variable::<f64>("acctime_rnof", &[])?
        .put_values(&[state.acctime_rnof], ..)?;
    vector(&mut file, "acc_rnof_uc", &["ucatch"], &state.acc_rnof, true)?;
    vector(
        &mut file,
        "volwater_ucat",
        &["ucatch"],
        &state.volwater,
        true,
    )?;
    if let Some(levsto) = &state.levsto {
        vector(&mut file, "levsto", &["ucatch"], levsto, true)?;
    }
    file.variable_mut("gridriver_restart_complete")
        .context("the completion marker disappeared")?
        .put_values(&[1], ..)?;
    file.close()?;
    Ok(())
}

/// 河道 history 旁车的变量（`write_gridriverlake_hist_restart` 的默认路径）。
const HISTORY_FIELDS: [&str; 10] = [
    "hist_acctime_ucat",
    "hist_wdsrf_ucat",
    "hist_veloc_riv",
    "hist_discharge",
    "hist_floodarea",
    "hist_rivsto",
    "hist_fldsto",
    "hist_flddph",
    "hist_storge",
    "hist_sfcelv",
];

fn history_fields(history: &RiverHistory) -> [&Vec<f64>; 10] {
    [
        &history.acctime,
        &history.wdsrf,
        &history.veloc,
        &history.discharge,
        &history.floodarea,
        &history.rivsto,
        &history.fldsto,
        &history.flddph,
        &history.storge,
        &history.sfcelv,
    ]
}

/// 河道 history 旁车 `<陆面旁车基名>.river`（区间跨过重启且 `acctime_ucat` 有值时写）。
pub fn write_river_history(path: &Path, history: &RiverHistory) -> Result<()> {
    let n = history.acctime.len();
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("ucatch", n)?;
    let levee = history
        .levsto
        .iter()
        .zip(&history.levdph)
        .flat_map(|(s, d)| [("hist_levsto", s), ("hist_levdph", d)]);
    let fields = HISTORY_FIELDS
        .iter()
        .copied()
        .zip(history_fields(history))
        .chain(levee);
    for (name, values) in fields {
        let mut variable = file.add_variable::<f64>(name, &["ucatch"])?;
        variable.put_attribute("missing_value", colm_core::MISSING)?;
        variable.put_values(values, ..)?;
    }
    file.close()?;
    Ok(())
}

/// 读回河道 history 旁车（`read_gridriverlake_hist_restart`，`strict`：十个量都必须在，
/// 开堤防时 `hist_levsto/levdph` 也必须在）。
pub fn read_river_history(path: &Path, n: usize, levee: bool) -> Result<RiverHistory> {
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut fields = HISTORY_FIELDS
        .iter()
        .map(|name| read_vector(&file, name, n, path))
        .collect::<Result<Vec<_>>>()?
        .into_iter();
    let mut next = || fields.next().expect("ten river-history fields");
    Ok(RiverHistory {
        acctime: next(),
        wdsrf: next(),
        veloc: next(),
        discharge: next(),
        floodarea: next(),
        rivsto: next(),
        fldsto: next(),
        flddph: next(),
        storge: next(),
        sfcelv: next(),
        levsto: levee
            .then(|| read_vector(&file, "hist_levsto", n, path))
            .transpose()?,
        levdph: levee
            .then(|| read_vector(&file, "hist_levdph", n, path))
            .transpose()?,
    })
}
