//! 历史累加器续跑旁车（上游 `include/land_history_restart.inc`）。
//!
//! 每写一份续跑文件，上游同时写 `<case>_restart_hist_<cdate>_<block>.nc`：
//! `history_schema = 1`、`history_freq`、`history_nac`，区间跨过重启（`nac > 0`）时再按
//! [`MANIFEST`] 顺序转存全部**已分配**的累加器（`nac_ln`、`nac_dt` 与 `a_*` 原始累加和，
//! 从未累加过的是 `spval`），最后 `history_complete = 1`。续跑方读回后，区间内的平均与
//! 不中断的运行逐位相同。
//!
//! 字段的分配条件与形状由 `oracle/scripts/gen_hist_manifest.py` 从上游生成（[`MANIFEST`]），
//! 这里只做读写与校验。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

use crate::history_manifest::MANIFEST;

/// 一个累加器在什么配置下被分配（`allocate_acc_fluxes` 的条件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requires {
    Always,
    /// `numurban > 0`：写在 `urban` 维上。
    Urban,
    /// `DEF_USE_PFT .or. DEF_USE_PC`。
    PftOrPc,
    /// `DEF_USE_BGC`。
    Bgc,
    /// `DEF_USE_BGC` 且内核带 `CROP` 宏。
    BgcCrop,
    /// 只在 Rust 引擎拒绝的宏（`CatchLateralFlow`、`HYPERSPECTRAL`、`DataAssimilation`…）下存在。
    Unsupported,
}

/// 清单的一项。
#[derive(Debug, Clone, Copy)]
pub struct ManifestEntry {
    /// 旁车里的变量名（`nac_ln`、`nac_dt` 或 `a_*`）。
    pub name: &'static str,
    /// 含向量维的秩：1 是每 patch 一个数，2/3 带 `d1_<name>`/`d2_<name>` 维。
    pub rank: u8,
    pub urban: bool,
    pub requires: Requires,
    /// 前导维长度（秩 1 时为 0）。
    pub n1: usize,
    pub n2: usize,
    /// 上游直接由这个累加器写出的历史量名（去掉 `f_`）；`None` 表示只累加、不直接写出。
    pub history: Option<&'static str>,
}

impl ManifestEntry {
    /// 每个 patch 的元素数。
    pub fn width(&self) -> usize {
        match self.rank {
            1 => 1,
            2 => self.n1,
            _ => self.n1 * self.n2,
        }
    }

    /// Rust 累加器里的键：直接写出的量用历史名，其余用去掉 `a_` 的累加器名。
    pub fn window_key(&self) -> &'static str {
        self.history
            .unwrap_or_else(|| self.name.strip_prefix("a_").unwrap_or(self.name))
    }

    fn allocated(&self, config: &SidecarConfig) -> bool {
        match self.requires {
            Requires::Always => true,
            Requires::Urban => config.urban_patches > 0,
            Requires::PftOrPc => config.pft_or_pc,
            Requires::Bgc => config.bgc,
            Requires::BgcCrop => config.bgc && config.crop,
            Requires::Unsupported => false,
        }
    }
}

/// 决定哪些累加器被分配的运行配置。
#[derive(Debug, Clone, Copy, Default)]
pub struct SidecarConfig {
    /// `history_acc_freq()`：`DEF_HIST_FREQ` 的代码（none 0、TIMESTEP 1 … YEARLY 5）。
    pub frequency_code: u8,
    /// `DEF_URBAN_RUN`：旁车多定义一个 `urban` 维。
    pub urban_run: bool,
    /// `numurban`：本算例的城市 patch 数。
    pub urban_patches: usize,
    pub pft_or_pc: bool,
    pub bgc: bool,
    pub crop: bool,
    /// 内核编进了 `GridRiverLakeFlow`（空间构建）：`history_schema = 2`，并多一个
    /// `history_river_required`（河道累加器有值时为 1，另写河道旁车）。
    pub river_lake_flow: bool,
}

/// 一个累加器的原始和。
#[derive(Debug, Clone, PartialEq)]
pub enum WindowValue {
    Scalar(f64),
    Column(Vec<f64>),
}

/// 一个历史区间的原始累加状态：上游的 `nac`、`nac_ln`、`nac_dt` 与 `a_*`。
///
/// `sums` 以 Rust 累加器的键（[`ManifestEntry::window_key`]）为键；表里没有的量就是
/// 从未累加过（旁车里写 `spval`）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryWindow {
    pub steps: usize,
    pub local_noon_steps: usize,
    pub daytime_steps: usize,
    pub sums: BTreeMap<String, WindowValue>,
}

/// 这是不是某个累加器在 Rust 里的键（见 [`ManifestEntry::window_key`]）。历史文件没声明的键
/// 只为旁车累加，写平均时跳过。
pub fn is_window_key(name: &str) -> bool {
    MANIFEST.iter().any(|entry| entry.window_key() == name)
}

/// 本配置下被分配的清单项（上游 `local(i) = 1` 的那些），按清单顺序。
pub fn allocated_fields(
    config: &SidecarConfig,
) -> impl Iterator<Item = &'static ManifestEntry> + '_ {
    MANIFEST.iter().filter(move |entry| entry.allocated(config))
}

/// 写旁车（`write_history_acc_restart` + `complete_history_acc_restart`）。
///
/// `patches` 是主重启的 `patch` 维长度；`windows` 每个 patch 一份（多作物单点），区间为空时可以只给
/// 一份（各 patch 的 `nac` 相同）。累加器按 patch 拼接：`patch` 是最外层维，每个 patch 的值连续。
pub fn write_sidecar(
    path: &Path,
    patches: usize,
    config: &SidecarConfig,
    windows: &[HistoryWindow],
) -> Result<()> {
    let window = windows
        .first()
        .context("the history sidecar needs at least one window")?;
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("patch", patches)?;
    if config.urban_run {
        file.add_dimension("urban", config.urban_patches)?;
    }
    let per_patch = |value: f64| vec![value; patches];
    let schema = if config.river_lake_flow { 2.0 } else { 1.0 };
    for (field, value) in [
        ("history_schema", schema),
        ("history_freq", f64::from(config.frequency_code)),
        ("history_nac", window.steps as f64),
    ] {
        file.add_variable::<f64>(field, &["patch"])?
            .put_values(&per_patch(value), ..)?;
    }
    if window.steps > 0 {
        ensure!(
            windows.len() == patches,
            "the history window spans the restart, but {} holds {patches} patches and colm-rs \
             accumulated {}",
            path.display(),
            windows.len()
        );
        ensure!(
            windows.iter().all(|other| other.steps == window.steps),
            "the patches disagree on the history sample count"
        );
        for entry in allocated_fields(config) {
            ensure!(
                !entry.urban || patches == 1,
                "urban accumulators are written for single-patch urban sites only"
            );
            let mut values = Vec::new();
            for window in windows {
                values.extend(field_values(entry, window)?);
            }
            let vector = if entry.urban { "urban" } else { "patch" };
            let d1 = format!("d1_{}", entry.name);
            let d2 = format!("d2_{}", entry.name);
            let dims: Vec<&str> = match entry.rank {
                1 => vec![vector],
                2 => {
                    file.add_dimension(&d1, entry.n1)?;
                    vec![vector, &d1]
                }
                _ => {
                    file.add_dimension(&d1, entry.n1)?;
                    file.add_dimension(&d2, entry.n2)?;
                    vec![vector, &d2, &d1]
                }
            };
            file.add_variable::<f64>(entry.name, &dims)?
                .put_values(&values, ..)
                .with_context(|| format!("cannot write {}", entry.name))?;
        }
    }
    // `river_active`：河道累加器（`acctime_ucat`）有值才为 1。河道还没移植，区间跨过重启时
    // 那份河道旁车写不出来，只好拒绝；区间已经关上时河道累加器也已清零，标记就是 0。
    if config.river_lake_flow {
        ensure!(
            window.steps == 0,
            "the history window spans this restart, but the river-lake history sidecar is not \
             ported yet; choose a restart frequency that closes the history window"
        );
        file.add_variable::<f64>("history_river_required", &["patch"])?
            .put_values(&per_patch(0.0), ..)?;
    }
    // 上游先写 0、全部转存完再改成 1（`complete_history_acc_restart`）；落盘结果只有 1。
    file.add_variable::<f64>("history_complete", &["patch"])?
        .put_values(&per_patch(1.0), ..)?;
    file.close()?;
    Ok(())
}

/// 一个清单项在单 patch 旁车里的取值。
fn field_values(entry: &ManifestEntry, window: &HistoryWindow) -> Result<Vec<f64>> {
    let width = entry.width();
    match entry.name {
        "nac_ln" => return Ok(vec![window.local_noon_steps as f64]),
        "nac_dt" => return Ok(vec![window.daytime_steps as f64]),
        _ => {}
    }
    // 城市量写在 `urban` 维上：非城市 patch 的城市维长度为 0，但那时它们也不会被分配。
    let values = match window.sums.get(entry.window_key()) {
        None => vec![colm_core::MISSING; width],
        Some(WindowValue::Scalar(sum)) => {
            ensure!(
                entry.rank == 1,
                "{} is a rank-{} accumulator, but Rust accumulated {} as a scalar",
                entry.name,
                entry.rank,
                entry.window_key()
            );
            vec![*sum]
        }
        Some(WindowValue::Column(sum)) => {
            ensure!(
                entry.rank > 1 && sum.len() == width,
                "{} holds {width} values per patch, but Rust accumulated {} values of {}",
                entry.name,
                sum.len(),
                entry.window_key()
            );
            sum.clone()
        }
    };
    Ok(values)
}

/// 读旁车（`read_history_acc_restart`），逐项照上游的校验。
///
/// - 旁车不存在：主重启要求旁车（`history_sidecar_required`）就报错，否则是旧式重启，
///   返回 `None`（上游只打一行警告）。
/// - 标记不全、未提交（`history_complete /= 1`）、历史频率变了、`nac` 不是非负整数：报错。
/// - `nac = 0`：返回空窗口；`nac > 0`：按本配置的清单读回全部已分配的累加器，缺一个就报错。
pub fn read_sidecar(
    primary: &Path,
    sidecar: &Path,
    config: &SidecarConfig,
) -> Result<Option<Vec<HistoryWindow>>> {
    let required = {
        let file =
            netcdf::open(primary).with_context(|| format!("cannot open {}", primary.display()))?;
        let marked = file.variable("history_sidecar_required").is_some();
        marked
    };
    if !sidecar.is_file() {
        ensure!(
            !required,
            "{} requires the history sidecar {}, which is missing",
            primary.display(),
            sidecar.display()
        );
        return Ok(None);
    }
    let file =
        netcdf::open(sidecar).with_context(|| format!("cannot open {}", sidecar.display()))?;
    let markers = [
        "history_schema",
        "history_freq",
        "history_nac",
        "history_complete",
    ];
    ensure!(
        markers.iter().all(|name| file.variable(name).is_some()),
        "incomplete land-history restart sidecar {}",
        sidecar.display()
    );
    let marker = |name: &str| -> Result<Vec<f64>> {
        let values: Vec<f64> = file
            .variable(name)
            .context("checked above")?
            .get_values(..)
            .with_context(|| format!("cannot read {name} from {}", sidecar.display()))?;
        ensure!(
            values.iter().all(|value| value.is_finite()),
            "non-finite {name} in {}",
            sidecar.display()
        );
        Ok(values)
    };
    ensure!(
        marker("history_schema")?
            .iter()
            .all(|&value| value == if config.river_lake_flow { 2.0 } else { 1.0 }),
        "unsupported land-history sidecar schema in {}",
        sidecar.display()
    );
    ensure!(
        marker("history_complete")?
            .iter()
            .all(|&value| value == 1.0),
        "incomplete land-history sidecar commit marker in {}",
        sidecar.display()
    );
    ensure!(
        marker("history_freq")?
            .iter()
            .all(|&value| value == f64::from(config.frequency_code)),
        "history frequency changed across the land restart {}",
        sidecar.display()
    );
    let counts = marker("history_nac")?;
    let Some(&nac) = counts.first() else {
        bail!("{} has no patches", sidecar.display());
    };
    ensure!(
        counts
            .iter()
            .all(|&value| value >= 0.0 && value == value.round() && value == nac),
        "invalid or inconsistent land-history sample counts in {}",
        sidecar.display()
    );
    let patches = counts.len();
    let mut windows = vec![
        HistoryWindow {
            steps: nac as usize,
            ..HistoryWindow::default()
        };
        patches
    ];
    if nac == 0.0 {
        return Ok(Some(windows));
    }
    for entry in allocated_fields(config) {
        let variable = file.variable(entry.name).with_context(|| {
            format!(
                "missing land-history accumulator {} in {}",
                entry.name,
                sidecar.display()
            )
        })?;
        let values: Vec<f64> = variable
            .get_values(..)
            .with_context(|| format!("cannot read {} from {}", entry.name, sidecar.display()))?;
        ensure!(
            values.len() == entry.width() * patches,
            "incompatible land-history sidecar shape for {} in {}",
            entry.name,
            sidecar.display()
        );
        let count = |value: f64| -> Result<usize> {
            ensure!(
                value >= 0.0 && value == value.round(),
                "invalid {} in {}",
                entry.name,
                sidecar.display()
            );
            Ok(value as usize)
        };
        for (window, values) in windows.iter_mut().zip(values.chunks(entry.width())) {
            match entry.name {
                "nac_ln" => window.local_noon_steps = count(values[0])?,
                "nac_dt" => window.daytime_steps = count(values[0])?,
                _ => {
                    // 从未累加过（整项 spval）的量不进窗口：与不中断的运行里"没有条目"同态。
                    if values.iter().all(|&value| value == colm_core::MISSING) {
                        continue;
                    }
                    let value = if entry.rank == 1 {
                        WindowValue::Scalar(values[0])
                    } else {
                        WindowValue::Column(values.to_vec())
                    };
                    window.sums.insert(entry.window_key().to_owned(), value);
                }
            }
        }
    }
    Ok(Some(windows))
}

#[cfg(test)]
#[path = "history_sidecar_tests.rs"]
mod history_sidecar_tests;
