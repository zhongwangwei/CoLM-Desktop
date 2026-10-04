//! 河道续跑文件 `<case>_restart_gridriver_<date>_lc<year>.nc`（`READ/WRITE_GridRiverLakeTimeVars`）。
//!
//! schema 2（无水库、示踪物）：标记、`gridriver_ucatch_identity`、
//! `wdsrf_ucat`、`veloc_riv`、`acctime_rnof`、`acc_rnof_uc`、`volwater_ucat`；
//! 开堤防时再加 `levsto`（`gridriver_restart_feature_levee = 1`），开分汊时再加
//! `wdsrf_ucat_prev` 与路径的 `bif_path_signature`、`pth_veloc`、`pth_momen`
//! （`gridriver_restart_feature_bifurcation = 1`）。

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

use super::network::RiverNetwork;
use super::reservoir::Reservoir;
use super::{BifurcationState, RiverHistory, RiverState};

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
///
/// 开水库时（`reservoir` 非空）读 `volresv`，并按 `validate_gridriver_reservoir_identity`
/// 核对逐水库的 `(版本, dam_seq)`。
pub fn read_river_state(
    path: &Path,
    network: &RiverNetwork,
    reservoir: Option<&Reservoir>,
) -> Result<RiverState> {
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
    let bifurcation_flag = scalar_i32("gridriver_restart_feature_bifurcation")?;
    ensure!(
        bifurcation_flag == 0 || bifurcation_flag == 1,
        "{} has an invalid bifurcation feature flag",
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
        // 标记为 1 时上一子步水深与路径状态都必须在；签名由构造模型时核对。
        volresv: match reservoir {
            Some(reservoir) if !reservoir.is_empty() => {
                let identity = file
                    .variable("gridriver_reservoir_identity")
                    .with_context(|| {
                        format!("{} has no gridriver_reservoir_identity", path.display())
                    })?
                    .get_values::<f64, _>(..)?;
                ensure!(
                    identity == reservoir.identity(),
                    "{} was written for a different reservoir table",
                    path.display()
                );
                Some(read_vector(&file, "volresv", reservoir.len(), path)?)
            }
            _ => None,
        },
        bifurcation: if bifurcation_flag == 1 {
            let matrix = |name: &str| -> Result<Vec<f64>> {
                file.variable(name)
                    .with_context(|| {
                        format!(
                            "{} declares bifurcation enabled but has no {name}",
                            path.display()
                        )
                    })?
                    .get_values::<f64, _>(..)
                    .with_context(|| format!("cannot read {name} from {}", path.display()))
            };
            let wdsrf_prev = read_vector(&file, "wdsrf_ucat_prev", n, path)?;
            ensure!(
                wdsrf_prev.iter().all(|w| w.is_finite() && *w >= 0.0),
                "GridRiverLake restart has invalid wdsrf_ucat_prev"
            );
            let levels = file
                .dimension("bifurcation_level")
                .with_context(|| format!("{} has no bifurcation_level", path.display()))?
                .len();
            Some(BifurcationState {
                levels,
                wdsrf_prev,
                veloc: matrix("pth_veloc")?,
                momen: matrix("pth_momen")?,
                signature: matrix("bif_path_signature")?,
            })
        } else {
            None
        },
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
    reservoir_identity: Option<&[f64]>,
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
        (
            "gridriver_restart_feature_bifurcation",
            i32::from(state.bifurcation.is_some()),
        ),
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
    if let Some(bif) = &state.bifurcation {
        vector(
            &mut file,
            "wdsrf_ucat_prev",
            &["ucatch"],
            &bif.wdsrf_prev,
            true,
        )?;
    }
    vector(&mut file, "veloc_riv", &["ucatch"], &state.veloc, true)?;
    file.add_variable::<f64>("acctime_rnof", &[])?
        .put_values(&[state.acctime_rnof], ..)?;
    vector(&mut file, "acc_rnof_uc", &["ucatch"], &state.acc_rnof, true)?;
    // 水库：`reservoir` 维、`gridriver_reservoir_identity`、`volresv`（在 `volwater_ucat` 之前）。
    if let (Some(volresv), Some(identity)) = (&state.volresv, reservoir_identity) {
        if !volresv.is_empty() {
            file.add_dimension("reservoir", volresv.len())?;
            file.add_dimension("gridriver_reservoir_identity_field", 2)?;
            vector(
                &mut file,
                "gridriver_reservoir_identity",
                &["reservoir", "gridriver_reservoir_identity_field"],
                identity,
                false,
            )?;
            vector(&mut file, "volresv", &["reservoir"], volresv, true)?;
        }
    }
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
    // `write_bifurcation_restart`：二维量用 `ncio_write_serial`（压缩、不带缺测属性）。
    if let Some(bif) = &state.bifurcation {
        let levels = bif.levels;
        let paths = bif.veloc.len() / levels.max(1);
        file.add_dimension("bifurcation_signature_field", 4 + 3 * levels)?;
        file.add_dimension("bifurcation_level", levels)?;
        file.add_dimension("bifurcation_pathway", paths)?;
        for (name, dims, values) in [
            (
                "bif_path_signature",
                ["bifurcation_pathway", "bifurcation_signature_field"],
                &bif.signature,
            ),
            (
                "pth_veloc",
                ["bifurcation_pathway", "bifurcation_level"],
                &bif.veloc,
            ),
            (
                "pth_momen",
                ["bifurcation_pathway", "bifurcation_level"],
                &bif.momen,
            ),
        ] {
            vector(&mut file, name, &dims, values, false)?;
        }
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
    let bifout = history.bifout.iter().map(|b| ("hist_bifout", b));
    let reservoirs = history.acctime_resv.as_ref().map_or(0, Vec::len);
    let fields = HISTORY_FIELDS
        .iter()
        .copied()
        .zip(history_fields(history))
        .chain(levee)
        .chain(bifout);
    for (name, values) in fields {
        let mut variable = file.add_variable::<f64>(name, &["ucatch"])?;
        variable.put_attribute("missing_value", colm_core::MISSING)?;
        variable.put_values(values, ..)?;
    }
    // 水库累加（`reservoir` 维）。
    if reservoirs > 0 {
        file.add_dimension("reservoir", reservoirs)?;
        for (name, values) in [
            ("hist_acctime_resv", &history.acctime_resv),
            ("hist_volresv", &history.volresv),
            ("hist_qresv_in", &history.qresv_in),
            ("hist_qresv_out", &history.qresv_out),
        ] {
            let mut variable = file.add_variable::<f64>(name, &["reservoir"])?;
            variable.put_attribute("missing_value", colm_core::MISSING)?;
            variable.put_values(values.as_ref().expect("reservoir history"), ..)?;
        }
    }
    // 分汊路径的累加（`ncio_write_serial` 的二维量，不带缺测属性）。
    if let (Some(lev), Some(acctime)) = (&history.bifflw_lev, &history.bifflw_acctime) {
        let paths = acctime.len();
        file.add_dimension("bifurcation_level", lev.len() / paths.max(1))?;
        file.add_dimension("bifurcation_pathway", paths)?;
        file.add_dimension("bifurcation_history_scalar", 1)?;
        file.add_variable::<f64>(
            "hist_bifflw_lev",
            &["bifurcation_pathway", "bifurcation_level"],
        )?
        .put_values(lev, ..)?;
        file.add_variable::<f64>(
            "hist_bifflw_acctime",
            &["bifurcation_pathway", "bifurcation_history_scalar"],
        )?
        .put_values(acctime, ..)?;
    }
    file.close()?;
    Ok(())
}

/// 读回河道 history 旁车（`read_gridriverlake_hist_restart`，`strict`：十个量都必须在，
/// 开堤防时 `hist_levsto/levdph` 也必须在，开分汊时 `hist_bifout/bifflw_lev/bifflw_acctime`
/// 也必须在；`bifurcation = (路径数, 层数)`）。
pub fn read_river_history(
    path: &Path,
    n: usize,
    levee: bool,
    bifurcation: Option<(usize, usize)>,
    reservoirs: Option<usize>,
) -> Result<RiverHistory> {
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut fields = HISTORY_FIELDS
        .iter()
        .map(|name| read_vector(&file, name, n, path))
        .collect::<Result<Vec<_>>>()?
        .into_iter();
    let mut next = || fields.next().expect("ten river-history fields");
    // 没有水库时上游不写这几项（`totalnumresv > 0` 才写）。
    let read_resv = |name: &str| -> Result<Option<Vec<f64>>> {
        match reservoirs {
            Some(0) => Ok(Some(Vec::new())),
            Some(m) => Ok(Some(read_vector(&file, name, m, path)?)),
            None => Ok(None),
        }
    };
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
        bifout: bifurcation
            .map(|_| read_vector(&file, "hist_bifout", n, path))
            .transpose()?,
        bifflw_lev: bifurcation
            .map(|(paths, levels)| read_vector(&file, "hist_bifflw_lev", paths * levels, path))
            .transpose()?,
        bifflw_acctime: bifurcation
            .map(|(paths, _)| read_vector(&file, "hist_bifflw_acctime", paths, path))
            .transpose()?,
        acctime_resv: read_resv("hist_acctime_resv")?,
        volresv: read_resv("hist_volresv")?,
        qresv_in: read_resv("hist_qresv_in")?,
        qresv_out: read_resv("hist_qresv_out")?,
    })
}

/// `RIVER_TRACER_RESTART_SCHEMA_VERSION`。
const RIVER_TRACER_SCHEMA: i32 = 2;

/// `write_tracer_restart`：在河道续跑文件末尾追加示踪物事务（先 `complete = 0`，网络元数据、
/// 逐示踪物的质量/待释放/累计输入/堤内池（有溶解度上限的另带固相），共享的 `acc_rnof_ref`，
/// history 累加行，最后写描述符并 `complete = 1`）。没有输运示踪物时写空事务。
/// `write_tracer_restart` 在只有 provider 示踪物（没有通用输运）时：提交一个空事务，
/// 以后加上输运示踪物的续跑能干净地冷启动，而不是把它当成写到一半的事务。
pub fn write_empty_river_tracers(path: &Path) -> Result<()> {
    let mut file =
        netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
    let mut scalar = |name: &str, value: i32| -> Result<()> {
        match file.variable_mut(name) {
            Some(mut variable) => variable.put_values(&[value], ..)?,
            None => file
                .add_variable::<i32>(name, &[])?
                .put_values(&[value], ..)?,
        }
        Ok(())
    };
    scalar("trc_river_restart_complete", 0)?;
    scalar("trc_river_descriptor_count", 0)?;
    scalar("trc_river_restart_schema", RIVER_TRACER_SCHEMA)?;
    scalar("trc_river_restart_complete", 1)?;
    file.close()?;
    Ok(())
}

pub fn write_river_tracers(
    path: &Path,
    network: &RiverNetwork,
    tracers: &mut super::tracer::RiverTracers,
    compression_level: u8,
) -> Result<()> {
    let identity = tracers.set.descriptor_identity();
    let mut file =
        netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
    let scalar = |file: &mut netcdf::FileMut, name: &str, value: i32| -> Result<()> {
        match file.variable_mut(name) {
            Some(mut variable) => variable.put_values(&[value], ..)?,
            None => file
                .add_variable::<i32>(name, &[])?
                .put_values(&[value], ..)?,
        }
        Ok(())
    };
    if identity.is_empty() {
        scalar(&mut file, "trc_river_restart_complete", 0)?;
        scalar(&mut file, "trc_river_descriptor_count", 0)?;
        scalar(&mut file, "trc_river_restart_schema", RIVER_TRACER_SCHEMA)?;
        scalar(&mut file, "trc_river_restart_complete", 1)?;
        file.close()?;
        return Ok(());
    }
    tracers.validate_for_restart()?;
    scalar(&mut file, "trc_river_restart_complete", 0)?;
    let vector = |file: &mut netcdf::FileMut, name: &str, values: &[f64]| -> Result<()> {
        let mut variable = file.add_variable::<f64>(name, &["ucatch"])?;
        if compression_level > 0 {
            variable.set_compression(i32::from(compression_level), false)?;
        }
        variable.put_attribute("missing_value", colm_core::MISSING)?;
        variable.put_values(values, ..)?;
        Ok(())
    };
    let n = network.len();
    vector(&mut file, "trc_numucat_meta", &vec![n as f64; n])?;
    let gdid = (0..n)
        .map(|i| f64::from((network.y[i] - 1) * network.nlon as i32 + network.x[i]))
        .collect::<Vec<_>>();
    vector(&mut file, "trc_ucat_gdid_meta", &gdid)?;
    let next = network
        .next
        .iter()
        .map(|&next| f64::from(if next >= 0 { next + 1 } else { next }))
        .collect::<Vec<_>>();
    vector(&mut file, "trc_ucat_next_meta", &next)?;
    let transport = tracers.set.transport_indices().collect::<Vec<_>>();
    for &itrc in &transport {
        let tracer = &tracers.set.tracers[itrc];
        let name = tracer.name.trim();
        vector(&mut file, &format!("trc_mass_{name}"), &tracers.mass[itrc])?;
        vector(
            &mut file,
            &format!("trc_inpbuf_{name}"),
            &tracers.inp_buf[itrc],
        )?;
        vector(
            &mut file,
            &format!("trc_accinp_{name}"),
            &tracers.acc_inp[itrc],
        )?;
        vector(
            &mut file,
            &format!("trc_levsto_{name}"),
            &tracers.levsto[itrc],
        )?;
        if tracer.has_dissolved_limit() {
            let (solid, levsto_solid) = tracers.solid.as_ref().context("solid pools")?;
            vector(&mut file, &format!("trc_solid_{name}"), &solid[itrc])?;
            vector(
                &mut file,
                &format!("trc_levsto_solid_{name}"),
                &levsto_solid[itrc],
            )?;
        }
    }
    vector(&mut file, "acc_rnof_ref", &tracers.acc_rnof_ref)?;
    let h = &tracers.history;
    for &itrc in &transport {
        let name = tracers.set.tracers[itrc].name.trim().to_owned();
        vector(
            &mut file,
            &format!("trc_hist_stor_{name}"),
            &h.storage_mass[itrc],
        )?;
        vector(
            &mut file,
            &format!("trc_hist_levsto_{name}"),
            &h.levsto_mass[itrc],
        )?;
        vector(&mut file, &format!("trc_hist_out_{name}"), &h.out[itrc])?;
        vector(
            &mut file,
            &format!("trc_hist_bifout_{name}"),
            &h.bifout[itrc],
        )?;
    }
    vector(&mut file, "trc_hist_water_storage", &h.water_storage)?;
    vector(&mut file, "trc_hist_levsto_water", &h.levsto_water)?;
    vector(&mut file, "trc_hist_acctime", &h.acctime)?;
    file.add_dimension(
        "trc_river_descriptor_field",
        colm_core::tracer::DESCRIPTOR_IDENTITY_WIDTH,
    )?;
    file.add_dimension("trc_river_transport_tracer", identity.len())?;
    scalar(
        &mut file,
        "trc_river_descriptor_count",
        identity.len() as i32,
    )?;
    let flat = identity.iter().flatten().copied().collect::<Vec<i32>>();
    file.add_variable::<i32>(
        "trc_river_descriptor_identity",
        &["trc_river_transport_tracer", "trc_river_descriptor_field"],
    )?
    .put_values(&flat, ..)?;
    scalar(&mut file, "trc_river_restart_schema", RIVER_TRACER_SCHEMA)?;
    scalar(&mut file, "trc_river_restart_complete", 1)?;
    file.close()?;
    Ok(())
}

/// `read_tracer_restart`：续跑里有完整、描述符相符的河道示踪物事务就读回并返回真；没有提交标记
/// （旧格式或 mkinidata 写的初始续跑）、描述符不符时返回假（调用方按水量冷启动）。读回后
/// 当前配置下没有堤的单元流域把堤内池并回可见池。
pub fn read_river_tracers(
    path: &Path,
    network: &RiverNetwork,
    levee: Option<&super::levee::Levee>,
    tracers: &mut super::tracer::RiverTracers,
) -> Result<bool> {
    let malformed = "malformed/incomplete river tracer restart descriptor metadata";
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let Some(complete) = file.variable("trc_river_restart_complete") else {
        return Ok(false);
    };
    let complete: i32 = complete.get_value(..)?;
    ensure!(complete == 1, "{malformed}");
    let schema: i32 = file
        .variable("trc_river_restart_schema")
        .context(malformed)?
        .get_value(..)?;
    let count: i32 = file
        .variable("trc_river_descriptor_count")
        .context(malformed)?
        .get_value(..)?;
    ensure!((0..=1000).contains(&count), "{malformed}");
    let expected = tracers.set.descriptor_identity();
    if schema != 1 && schema != RIVER_TRACER_SCHEMA {
        return Ok(false);
    }
    if count == 0 {
        return Ok(expected.is_empty());
    }
    let identity = file
        .variable("trc_river_descriptor_identity")
        .context(malformed)?
        .get_values::<i32, _>(..)?;
    ensure!(
        identity.len() == count as usize * colm_core::tracer::DESCRIPTOR_IDENTITY_WIDTH,
        "{malformed}"
    );
    let flat = expected.iter().flatten().copied().collect::<Vec<i32>>();
    if count as usize != expected.len() || identity != flat {
        return Ok(false);
    }
    let n = network.len();
    let vector = |name: &str, required: bool| -> Result<Option<Vec<f64>>> {
        match file.variable(name) {
            Some(variable) => {
                let values = variable.get_values::<f64, _>(..)?;
                ensure!(
                    values.len() == n,
                    "incomplete or malformed committed river tracer restart ({name})"
                );
                Ok(Some(values))
            }
            None if required => {
                bail!("incomplete or malformed committed river tracer restart ({name})")
            }
            None => Ok(None),
        }
    };
    let gdid = vector("trc_ucat_gdid_meta", true)?.expect("required");
    let next = vector("trc_ucat_next_meta", true)?.expect("required");
    let matches = (0..n).all(|i| {
        let expect_gdid = (network.y[i] - 1) * network.nlon as i32 + network.x[i];
        let expect_next = if network.next[i] >= 0 {
            network.next[i] + 1
        } else {
            network.next[i]
        };
        gdid[i].round() as i32 == expect_gdid && next[i].round() as i32 == expect_next
    });
    ensure!(
        matches,
        "river/lake tracer restart belongs to a different catchment network"
    );
    let numucat = vector("trc_numucat_meta", true)?.expect("required");
    ensure!(
        numucat.iter().all(|&v| v.round() as usize == n),
        "river/lake tracer restart catchment-count metadata is inconsistent"
    );
    let transport = tracers.set.transport_indices().collect::<Vec<_>>();
    for &itrc in &transport {
        let tracer = tracers.set.tracers[itrc].clone();
        let name = tracer.name.trim().to_owned();
        tracers.mass[itrc] = vector(&format!("trc_mass_{name}"), true)?.expect("required");
        tracers.inp_buf[itrc] = vector(&format!("trc_inpbuf_{name}"), true)?.expect("required");
        tracers.levsto[itrc] = vector(&format!("trc_levsto_{name}"), true)?.expect("required");
        if schema >= 2 && tracer.has_dissolved_limit() {
            let (solid, levsto_solid) = tracers.solid.as_mut().context("solid pools")?;
            solid[itrc] = vector(&format!("trc_solid_{name}"), true)?.expect("required");
            levsto_solid[itrc] =
                vector(&format!("trc_levsto_solid_{name}"), true)?.expect("required");
        }
        tracers.acc_inp[itrc] = vector(&format!("trc_accinp_{name}"), true)?.expect("required");
    }
    tracers.acc_rnof_ref = vector("acc_rnof_ref", true)?.expect("required");
    // history 累加行：全在才读（不是有限数的清成 0），否则从续跑起新开一个窗口。
    tracers.history.reset();
    let mut names = Vec::new();
    for &itrc in &transport {
        let name = tracers.set.tracers[itrc].name.trim().to_owned();
        for prefix in [
            "trc_hist_stor_",
            "trc_hist_levsto_",
            "trc_hist_out_",
            "trc_hist_bifout_",
        ] {
            names.push(format!("{prefix}{name}"));
        }
    }
    names.push("trc_hist_water_storage".to_owned());
    names.push("trc_hist_levsto_water".to_owned());
    let complete_history = names.iter().all(|name| file.variable(name).is_some());
    if complete_history {
        let finite = |values: Vec<f64>| -> Vec<f64> {
            values
                .into_iter()
                .map(|v| if v.is_finite() { v } else { 0.0 })
                .collect()
        };
        for &itrc in &transport {
            let name = tracers.set.tracers[itrc].name.trim().to_owned();
            let h = &mut tracers.history;
            h.storage_mass[itrc] =
                finite(vector(&format!("trc_hist_stor_{name}"), true)?.expect("required"));
            h.levsto_mass[itrc] =
                finite(vector(&format!("trc_hist_levsto_{name}"), true)?.expect("required"));
            h.out[itrc] = finite(vector(&format!("trc_hist_out_{name}"), true)?.expect("required"));
            h.bifout[itrc] =
                finite(vector(&format!("trc_hist_bifout_{name}"), true)?.expect("required"));
        }
        let h = &mut tracers.history;
        h.water_storage = finite(vector("trc_hist_water_storage", true)?.expect("required"));
        h.levsto_water = finite(vector("trc_hist_levsto_water", true)?.expect("required"));
        if let Some(acctime) = vector("trc_hist_acctime", false)? {
            h.acctime = finite(acctime);
        }
    }
    tracers.validate_for_restart()?;
    // 当前配置下没有堤的单元流域：堤内池并回可见池（开了堤防才有 `has_levee`）。
    if let Some(levee) = levee {
        for i in 0..n {
            if levee.has[i] {
                continue;
            }
            for &itrc in &transport {
                tracers.mass[itrc][i] += tracers.levsto[itrc][i];
                tracers.levsto[itrc][i] = 0.0;
                if tracers.set.tracers[itrc].has_dissolved_limit() {
                    if let Some((solid, levsto_solid)) = tracers.solid.as_mut() {
                        solid[itrc][i] += levsto_solid[itrc][i];
                        levsto_solid[itrc][i] = 0.0;
                    }
                }
            }
        }
    }
    tracers.validate_for_restart()?;
    Ok(true)
}
