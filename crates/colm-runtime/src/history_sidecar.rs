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
use colm_core::{is_leap_year, month_day, month_lengths, CalendarTime};
use colm_hist::schedule::HistoryFrequency;

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
#[derive(Debug, Clone, Copy)]
pub struct SidecarConfig {
    /// `DEF_HIST_FREQ`：决定旁车标记里的频率代码，也决定一个窗口有多少秒。
    pub frequency: HistoryFrequency,
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

impl Default for SidecarConfig {
    /// `DEF_HIST_FREQ` 的缺省是 `none`。
    fn default() -> Self {
        Self {
            frequency: HistoryFrequency::None,
            urban_run: false,
            urban_patches: 0,
            pft_or_pc: false,
            bgc: false,
            crop: false,
            river_lake_flow: false,
        }
    }
}

/// `freq_code(DEF_HIST_FREQ)`：旁车标记 `history_freq` 的取值。
pub fn frequency_code(frequency: HistoryFrequency) -> u8 {
    use HistoryFrequency as F;
    match frequency {
        F::None => 0,
        F::Timestep => 1,
        F::Hourly => 2,
        F::Daily => 3,
        F::Monthly => 4,
        F::Yearly => 5,
    }
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

/// `hacc_window_seconds`：以 `date` 结束的那个 history 窗口的秒数；频率定不下来时 `-1`。
///
/// MONTHLY/YEARLY 只有在重启落在窗口边界上（每月 1 日 / 1 月 1 日）才算得出长度：那时窗口
/// 正好是上一个月/上一年（闰月、闰年按上一个月/上一年算）。不在边界上返回 `-1`，调用方据此
/// 不判"完整"——一个落在月中的日重启当然可能已经攒满一个月的步数，但那不是完整窗口。
pub fn window_seconds(
    frequency: HistoryFrequency,
    time_step_seconds: f64,
    date: CalendarTime,
) -> Result<f64> {
    use HistoryFrequency as F;
    let (month, day) = month_day(date)?;
    Ok(match frequency {
        F::None => -1.0,
        F::Timestep => time_step_seconds,
        F::Hourly => 3_600.0,
        F::Daily => 86_400.0,
        F::Monthly if day == 1 => {
            let (year, previous) = if month == 1 {
                (date.year - 1, 12)
            } else {
                (date.year, month - 1)
            };
            f64::from(month_lengths(year)[usize::from(previous) - 1]) * 86_400.0
        }
        // 上游判的是 jdate(2) == 1（儒略日 1 = 1 月 1 日），不是"1 月的任何一天"。
        F::Yearly if date.julian_day == 1 => {
            f64::from(if is_leap_year(date.year - 1) {
                366
            } else {
                365
            }) * 86_400.0
        }
        F::Monthly | F::Yearly => -1.0,
    })
}

/// 重启点上的窗口是否已经完整：`save_history_acc_restart` 据此不写旁车，
/// `read_history_acc_restart` 据此丢弃读到的窗口。
///
/// 判据用的是上游的半步容差（`window_nac * timestep >= window - timestep/2`）：重启时刻通常
/// 落在步末，窗口长度不必整除步长（月长不是步长的整数倍），差半步以内都算同一个窗口。
pub fn window_is_complete(
    frequency: HistoryFrequency,
    time_step_seconds: f64,
    date: CalendarTime,
    steps: usize,
) -> Result<bool> {
    let seconds = window_seconds(frequency, time_step_seconds, date)?;
    Ok(seconds > 0.0
        && steps > 0
        && steps as f64 * time_step_seconds >= seconds - 0.5 * time_step_seconds)
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
    let urban = vec![config.urban_run; patches];
    write_sidecar_with_river(path, patches, config, windows, false, &urban)
}

/// 本文件里城市 patch 的个数（`numurban`，旁车 `urban` 维的长度）。`urban` 为空时退回配置里的总数。
fn urban_units(config: &SidecarConfig, urban: &[bool]) -> usize {
    if urban.is_empty() {
        config.urban_patches
    } else {
        urban.iter().filter(|&&is_urban| is_urban).count()
    }
}

/// [`write_sidecar`]；空间构建（`river_lake_flow`）另写 `history_river_required`：河道累加器
/// （`acctime_ucat`）有值时为 1，调用方同时写河道旁车（`<sidecar 基名>.river`）。
pub fn write_sidecar_with_river(
    path: &Path,
    patches: usize,
    config: &SidecarConfig,
    windows: &[HistoryWindow],
    river_required: bool,
    urban: &[bool],
) -> Result<()> {
    let window = windows
        .first()
        .context("the history sidecar needs at least one window")?;
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("patch", patches)?;
    if config.urban_run {
        file.add_dimension("urban", urban_units(config, urban))?;
    }
    let per_patch = |value: f64| vec![value; patches];
    let schema = if config.river_lake_flow { 2.0 } else { 1.0 };
    for (field, value) in [
        ("history_schema", schema),
        ("history_freq", f64::from(frequency_code(config.frequency))),
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
            // 城市量排在 `urban` 维上：只取城市 patch 的窗口，按 patch 顺序（`landurban` 即此序）。
            ensure!(
                !entry.urban || patches == 1 || urban.len() == patches,
                "urban accumulators need the urban flag of every patch"
            );
            let mut values = Vec::new();
            for (index, window) in windows.iter().enumerate() {
                if entry.urban && patches > 1 && !urban[index] {
                    continue;
                }
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
    // `river_active`：河道累加器（`acctime_ucat`）有值才为 1。
    if config.river_lake_flow {
        file.add_variable::<f64>("history_river_required", &["patch"])?
            .put_values(&per_patch(if river_required { 1.0 } else { 0.0 }), ..)?;
    } else {
        ensure!(
            !river_required,
            "only river-lake builds carry river history"
        );
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

/// 读回的历史窗口与"真的恢复了没有"的判决（上游 `hist_acc_window_restored`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReadWindow {
    /// 每个 patch 一份；完整窗口被丢弃时是全新空窗口。
    pub windows: Vec<HistoryWindow>,
    /// 只在真的续接了**不完整**的陆面窗口时为真；完整窗口读回后为假。
    /// 河道累加器旁车另读（上游在 `river_sums` 读回后也会把它置真），由调用方取或。
    pub restored: bool,
}

/// 读旁车（`read_history_acc_restart`），逐项照上游的校验。
///
/// - 旁车不存在：主重启要求旁车（`history_sidecar_required`）就报错，否则是旧式重启，
///   返回 `None`（上游只打一行警告）。
/// - 标记不全、未提交（`history_complete /= 1`）、历史频率变了、`nac` 不是非负整数：报错。
/// - `nac = 0`：返回空窗口；`nac > 0`：按本配置的清单读回全部已分配的累加器，缺一个就报错。
/// - 重启点上的窗口已经完整（[`window_is_complete`]）：**丢弃**，返回空窗口且 `restored = false`。
///   完整窗口在重启时已经被（或将要被）写进历史文件，续接它会让重启后的第一个窗口覆盖两个
///   窗口（分段跑报 mean(window1, window2) 而不是 window2）。只有中途重启的不完整窗口才续接。
///   判定在读取累加器之前，与上游一致：完整窗口里那些字段坏没坏都不影响判决。
pub fn read_sidecar(
    primary: &Path,
    sidecar: &Path,
    config: &SidecarConfig,
    urban: &[bool],
    date: CalendarTime,
    time_step_seconds: f64,
) -> Result<Option<ReadWindow>> {
    let required = {
        let file =
            netcdf::open(primary).with_context(|| format!("cannot open {}", primary.display()))?;
        // 标记可能从上一份续跑原样复制过来：只有值为 1 才要求旁车，被清零的旧标记不算。
        file.variable("history_sidecar_required")
            .is_some_and(|marker| {
                marker
                    .get_values::<f64, _>(..)
                    .is_ok_and(|values| values.contains(&1.0))
            })
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
            .all(|&value| value == f64::from(frequency_code(config.frequency))),
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
    if window_is_complete(config.frequency, time_step_seconds, date, nac as usize)? {
        return Ok(Some(ReadWindow {
            windows: vec![HistoryWindow::default(); patches],
            restored: false,
        }));
    }
    if nac == 0.0 {
        return Ok(Some(ReadWindow {
            windows,
            restored: false,
        }));
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
        // 城市量排在 `urban` 维上，只对应城市 patch（多 patch 时按 `urban` 标记分配）。
        let targets: Vec<usize> = if entry.urban && patches > 1 {
            ensure!(
                urban.len() == patches,
                "urban accumulators need the urban flag of every patch"
            );
            (0..patches).filter(|&patch| urban[patch]).collect()
        } else {
            (0..patches).collect()
        };
        ensure!(
            values.len() == entry.width() * targets.len(),
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
        for (&target, values) in targets.iter().zip(values.chunks(entry.width())) {
            let window = &mut windows[target];
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
    Ok(Some(ReadWindow {
        windows,
        restored: true,
    }))
}

#[cfg(test)]
#[path = "history_sidecar_tests.rs"]
mod history_sidecar_tests;
