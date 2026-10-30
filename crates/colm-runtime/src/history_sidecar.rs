//! 历史累加器续跑旁车（上游 `main/MOD_HistAccRestart.F90`）。
//!
//! 每写一份续跑文件，同时写 `<case>_restart_hist_<cdate>_<block>.nc`：先写标记
//! `hacc_complete = 0`、`hacc_freq`、`hacc_nac`，区间跨过重启（`nac > 0`）时再按
//! [`MANIFEST`] 顺序转存全部**已分配**的累加器（`nac_ln`、`nac_dt` 与 `a_*` 原始累加和，
//! 从未累加过的是 `spval`）；河湖构建最后写 `hacc_river`（有河道累加量时为 1，另写
//! `<旁车基名>.river`），全部落盘后把 `hacc_complete` 改成 1。续跑方读回后，区间内的平均与
//! 不中断的运行逐位相同。
//!
//! 标记名与上游一致，所以 Fortran 与 Rust 可以互读对方写的旁车（累加器名 `a_us`…、维度名
//! `d1_<name>`/`d2_<name>`、`.river` 文件名本来就已经相同）。旧 Rust 旁车用
//! `history_schema`/`history_freq`/`history_nac`/`history_complete`/`history_river_required`，
//! 读回时逐标记回退；新格式优先。
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
    /// 内核编进了 `GridRiverLakeFlow`（空间构建）：旁车多一个 `hacc_river`（河道累加器
    /// 有值时为 1，另写河道旁车），读回时要求它与 `.river` 一起成套。
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

/// `freq_code(DEF_HIST_FREQ)`：旁车标记 `hacc_freq` 的取值。
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

/// 旁车标记（上游 `MOD_HistAccRestart.F90`）。本轮起 Rust 也写这套名字。
const HACC_COMPLETE: &str = "hacc_complete";
const HACC_FREQ: &str = "hacc_freq";
const HACC_NAC: &str = "hacc_nac";
/// 只在河湖构建里写：本窗口有河道累加量时为 1（`.river` 里另存求和）。
const HACC_RIVER: &str = "hacc_river";

/// 旧 Rust 旁车的标记。只用于读回兼容，不再写出。
const LEGACY_SCHEMA: &str = "history_schema";
const LEGACY_COMPLETE: &str = "history_complete";
const LEGACY_FREQ: &str = "history_freq";
const LEGACY_NAC: &str = "history_nac";
const LEGACY_RIVER: &str = "history_river_required";

/// 写旁车（上游 `write_sidecar` + `save_history_acc_restart` 的完成标记）。
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

/// [`write_sidecar`]；空间构建（`river_lake_flow`）另写 `hacc_river`：河道累加器
/// （`acctime_ucat`）有值时为 1，调用方写完 `<sidecar 基名>.river` 后再落完成标记。
pub fn write_sidecar_with_river(
    path: &Path,
    patches: usize,
    config: &SidecarConfig,
    windows: &[HistoryWindow],
    river_required: bool,
    urban: &[bool],
) -> Result<()> {
    write_sidecar_uncommitted(path, patches, config, windows, river_required, urban)?;
    commit_sidecar(path)
}

/// 上游 `write_sidecar`：先写 `hacc_complete = 0`、`hacc_freq`、`hacc_nac`，再写累加器，
/// 河湖构建最后写 `hacc_river`。完成标记见 [`commit_sidecar`]。
///
/// 拆成两步是崩溃安全的一半，也和上游一致：被 kill 的运行留下 `hacc_complete = 0`，读回方
/// 按上游规则警告后从重启处重新开窗，而不是拿半份累加器续跑。
pub fn write_sidecar_uncommitted(
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("patch", patches)?;
    if config.urban_run {
        file.add_dimension("urban", urban_units(config, urban))?;
    }
    let per_patch = |value: f64| vec![value; patches];
    // 上游顺序：完成标记先写 0（最后才改 1），再频率与样本数。
    for (field, value) in [
        (HACC_COMPLETE, 0.0),
        (HACC_FREQ, f64::from(frequency_code(config.frequency))),
        (HACC_NAC, window.steps as f64),
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
    if config.river_lake_flow {
        file.add_variable::<f64>(HACC_RIVER, &["patch"])?
            .put_values(&per_patch(if river_required { 1.0 } else { 0.0 }), ..)?;
    } else {
        ensure!(
            !river_required,
            "only river-lake builds carry river history"
        );
    }
    file.close()?;
    Ok(())
}

/// 上游 `save_history_acc_restart` 的最后一步：把 `hacc_complete` 从 0 改成 1。
///
/// 必须在全部内容（含 `<sidecar>.river` 的河道求和）落盘之后调；读回方只认 1。
pub fn commit_sidecar(path: &Path) -> Result<()> {
    let mut file =
        netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
    let patches = file
        .dimension_len("patch")
        .with_context(|| format!("{} has no patch dimension", path.display()))?;
    let mut marker = file
        .variable_mut(HACC_COMPLETE)
        .with_context(|| format!("{} has no {HACC_COMPLETE} marker", path.display()))?;
    marker
        .put_values(&vec![1.0; patches], ..)
        .with_context(|| format!("cannot complete {}", path.display()))?;
    drop(marker);
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

/// 旁车盘上格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidecarFormat {
    /// 上游 `MOD_HistAccRestart.F90` 的 `hacc_*`（本轮起 Rust 也写这套）。
    Upstream,
    /// 旧 Rust 旁车（`history_schema`/`history_complete`），只读。
    Legacy,
}

impl SidecarFormat {
    /// 新名 `hacc_complete` 优先；没有它时旧 Rust 的 `history_schema`/`history_complete`
    /// 才算旧格式。两者都没有的是不认识的标记集。
    fn detect(file: &netcdf::File) -> Option<Self> {
        if file.variable(HACC_COMPLETE).is_some() {
            Some(Self::Upstream)
        } else if file.variable(LEGACY_SCHEMA).is_some() || file.variable(LEGACY_COMPLETE).is_some()
        {
            Some(Self::Legacy)
        } else {
            None
        }
    }

    fn complete(self) -> &'static str {
        match self {
            Self::Upstream => HACC_COMPLETE,
            Self::Legacy => LEGACY_COMPLETE,
        }
    }

    fn freq(self) -> &'static str {
        match self {
            Self::Upstream => HACC_FREQ,
            Self::Legacy => LEGACY_FREQ,
        }
    }

    fn nac(self) -> &'static str {
        match self {
            Self::Upstream => HACC_NAC,
            Self::Legacy => LEGACY_NAC,
        }
    }

    fn river(self) -> &'static str {
        match self {
            Self::Upstream => HACC_RIVER,
            Self::Legacy => LEGACY_RIVER,
        }
    }
}

/// 读一个标记向量：必须是 `(patch)` 上的 f64、值全部有限。
///
/// 形状与有限性照上游 `require_var` + `read_marker`（非有限标记上游直接 `CoLM_stop`）；
/// 值本身的语义（提交、频率、样本数）由调用方判。
fn read_marker(file: &netcdf::File, name: &str, path: &Path) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no {name} marker", path.display()))?;
    ensure!(
        variable.dimensions().len() == 1 && variable.dimensions()[0].name() == "patch",
        "{name} in {} is not a (patch) marker vector",
        path.display()
    );
    let values = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "non-finite {name} in {}",
        path.display()
    );
    Ok(values)
}

/// 上游对这些情况都只是 `write(*,...)` 警告后从重启处重新开窗（不停机）：
/// 缺/未提交完成标记、频率变了、标记集对不上。
fn warn_ignored(path: &Path, reason: &str) {
    eprintln!(
        "Warning: ignoring the history-accumulator restart {} ({reason}); \
         the history window starts at the restart.",
        path.display()
    );
}

/// 读旁车（`read_history_acc_restart`），逐项照上游的校验。
///
/// - 旁车不存在：主重启要求旁车（Rust-only 的 `history_sidecar_required`）就报错，否则返回
///   `None`（上游只打一行警告）。
/// - 标记集不认识、缺完成标记、完成标记不是 1、频率变了：**警告后返回 `None`**，调用方从
///   重启处重新开窗（上游同样不停机）。主重启标了 required 时这些属于自家文件损坏，报错。
/// - 新格式 `hacc_freq`/`hacc_nac`（旧格式 `history_freq`/`history_nac`）缺失、形状不对、
///   非有限、`nac` 不是同一个非负整数：报错（上游这里 `require_var`/`read_marker` 会停机）。
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
    let Some(format) = SidecarFormat::detect(&file) else {
        // 不认识的标记集（例如别的实现、或上游更早/更晚的格式）：上游对缺完成标记的旁车是
        // 警告后重开窗；主重启标了 required 的旁车一定是 Rust 自己写的，这时标记对不上是损坏。
        ensure!(
            !required,
            "{} carries no recognised history-accumulator markers, but {} marks it required",
            sidecar.display(),
            primary.display()
        );
        warn_ignored(sidecar, "no recognised hacc_* or legacy marker set");
        return Ok(None);
    };
    if file.variable(format.complete()).is_none() {
        // 旧格式才可能只有 `history_schema` 而没有完成标记：那是被 kill 的运行留下的半成品。
        warn_ignored(sidecar, "no completion marker");
        return Ok(None);
    }
    let complete = read_marker(&file, format.complete(), sidecar)?;
    if complete.iter().any(|&value| value != 1.0) {
        warn_ignored(sidecar, "the completion marker is not 1");
        return Ok(None);
    }
    // 频率变了：上游警告后重开窗（要求 `hacc_freq` 存在且形状对，缺失算损坏）。
    let expected_freq = f64::from(frequency_code(config.frequency));
    let freq = read_marker(&file, format.freq(), sidecar)?;
    if freq.iter().any(|&value| value != expected_freq) {
        warn_ignored(sidecar, "DEF_HIST_FREQ changed across the restart");
        return Ok(None);
    }
    if format == SidecarFormat::Legacy {
        // 旧 Rust 用 `history_schema` 编码构建类型（1 非河湖、2 河湖）；构建变了整组累加器
        // 的含义就变了，按与"频率变了"同样的方式丢弃。
        let expected_schema = if config.river_lake_flow { 2.0 } else { 1.0 };
        let schema = read_marker(&file, LEGACY_SCHEMA, sidecar)?;
        if schema.iter().any(|&value| value != expected_schema) {
            warn_ignored(sidecar, "the build changed across the restart");
            return Ok(None);
        }
    }
    let counts = read_marker(&file, format.nac(), sidecar)?;
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
    // 河湖构建要求 `hacc_river` 成套存在；缺失说明这份旁车是别的构建写的（上游这时
    // `require_var` 会停机，这里按"标记集对不上"警告后重开窗，不中止分段跑）。
    if config.river_lake_flow && file.variable(format.river()).is_none() {
        warn_ignored(sidecar, "the sidecar has no river-accumulator marker");
        return Ok(None);
    }
    if window_is_complete(config.frequency, time_step_seconds, date, nac as usize)? {
        return Ok(Some(ReadWindow {
            windows: vec![HistoryWindow::default(); patches],
            restored: false,
        }));
    }
    if nac == 0.0 {
        return Ok(Some(ReadWindow {
            windows: vec![HistoryWindow::default(); patches],
            restored: false,
        }));
    }
    let mut windows = vec![
        HistoryWindow {
            steps: nac as usize,
            ..HistoryWindow::default()
        };
        patches
    ];
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

/// 该旁车是否要求读回 `<sidecar>.river` 的河道求和。
///
/// 上游判据是 `hacc_river` 的 `active > 0`（旧 Rust 是 `history_river_required = 1`）；
/// 未提交、标记集不认识、或不是河湖构建写的旁车返回 `false` —— 上游这些情况也根本
/// 不会去读 `.river`。
pub fn sidecar_river_required(sidecar: &Path) -> Result<bool> {
    if !sidecar.is_file() {
        return Ok(false);
    }
    let file =
        netcdf::open(sidecar).with_context(|| format!("cannot open {}", sidecar.display()))?;
    let Some(format) = SidecarFormat::detect(&file) else {
        return Ok(false);
    };
    if file.variable(format.complete()).is_none()
        || read_marker(&file, format.complete(), sidecar)?
            .iter()
            .any(|&value| value != 1.0)
    {
        return Ok(false);
    }
    match file.variable(format.river()) {
        Some(_) => Ok(read_marker(&file, format.river(), sidecar)?
            .iter()
            .any(|&value| value > 0.0)),
        None => Ok(false),
    }
}

#[cfg(test)]
#[path = "history_sidecar_tests.rs"]
mod history_sidecar_tests;
