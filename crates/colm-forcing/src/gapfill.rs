//! 强迫场缺测诊断与修复。
//!
//! 实现计划与科学边界见 `docs/plan-forcing-gap-repair.md`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

pub const QC_OBSERVED: u8 = 0;
pub const QC_INTERPOLATED: u8 = 1;
pub const QC_ERA5_CORRECTED: u8 = 2;
pub const QC_UNRESOLVED: u8 = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableKind {
    Continuous,
    NonNegative,
    Precipitation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapRun {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapReport {
    pub missing: usize,
    pub short_missing: usize,
    pub long_missing: usize,
    pub longest_gap: usize,
    pub runs: Vec<GapRun>,
}

pub fn analyze_gaps(values: &[f64], short_gap_max: usize) -> GapReport {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < values.len() {
        if values[i].is_finite() {
            i += 1;
            continue;
        }
        let start = i;
        while i < values.len() && !values[i].is_finite() {
            i += 1;
        }
        runs.push(GapRun {
            start,
            len: i - start,
        });
    }
    let missing = runs.iter().map(|run| run.len).sum();
    let short_missing = runs
        .iter()
        .filter(|run| run.len <= short_gap_max)
        .map(|run| run.len)
        .sum();
    GapReport {
        missing,
        short_missing,
        long_missing: missing - short_missing,
        longest_gap: runs.iter().map(|run| run.len).max().unwrap_or(0),
        runs,
    }
}

/// 填补**有两侧观测约束**的短缺口。边界缺口没有外推依据，始终保留。
/// 降水不做线性插值：只有两侧都为零时，才有足够依据补零。
pub fn fill_short_gaps(values: &mut [f64], short_gap_max: usize, kind: VariableKind) -> Vec<u8> {
    let report = analyze_gaps(values, short_gap_max);
    let mut qc = values
        .iter()
        .map(|value| {
            if value.is_finite() {
                QC_OBSERVED
            } else {
                QC_UNRESOLVED
            }
        })
        .collect::<Vec<_>>();

    for run in report.runs {
        if run.len > short_gap_max || run.start == 0 || run.start + run.len >= values.len() {
            continue;
        }
        let left = values[run.start - 1];
        let right = values[run.start + run.len];
        if !left.is_finite() || !right.is_finite() {
            continue;
        }
        if kind == VariableKind::Precipitation && (left != 0.0 || right != 0.0) {
            continue;
        }
        for j in 0..run.len {
            let fraction = (j + 1) as f64 / (run.len + 1) as f64;
            let mut value = left + (right - left) * fraction;
            if matches!(
                kind,
                VariableKind::NonNegative | VariableKind::Precipitation
            ) {
                value = value.max(0.0);
            }
            values[run.start + j] = value;
            qc[run.start + j] = QC_INTERPOLATED;
        }
    }
    qc
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimezoneSource {
    ManualOverride,
    FileMetadata,
    LongitudeInferred,
}

impl TimezoneSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManualOverride => "manual_override",
            Self::FileMetadata => "file_metadata",
            Self::LongitudeInferred => "longitude_inferred_offset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimezoneDecision {
    /// Local time = UTC + offset_hours.
    pub offset_hours: f64,
    pub source: TimezoneSource,
}

/// 人工值是明确覆盖，因此优先于文件；没有人工值时文件元数据优先，最后才
/// 用经度得到太阳时区近似。后者不是行政时区，调用方必须显示“推断”。
pub fn decide_timezone(
    manual_offset: Option<f64>,
    metadata: Option<&str>,
    longitude: Option<f64>,
) -> Result<TimezoneDecision> {
    if let Some(offset_hours) = manual_offset {
        validate_offset(offset_hours)?;
        return Ok(TimezoneDecision {
            offset_hours,
            source: TimezoneSource::ManualOverride,
        });
    }
    if let Some(raw) = metadata.filter(|value| !value.trim().is_empty()) {
        let offset_hours = parse_utc_offset(raw)
            .with_context(|| format!("unsupported forcing timezone metadata {raw:?}"))?;
        return Ok(TimezoneDecision {
            offset_hours,
            source: TimezoneSource::FileMetadata,
        });
    }
    let lon = longitude.context(
        "forcing timezone is absent and longitude is unavailable; give a UTC offset explicitly",
    )?;
    if !lon.is_finite() || !(-180.0..=180.0).contains(&lon) {
        bail!("longitude {lon} is outside -180..=180 degrees");
    }
    let offset_hours = (lon / 15.0).round().clamp(-12.0, 14.0);
    Ok(TimezoneDecision {
        offset_hours,
        source: TimezoneSource::LongitudeInferred,
    })
}

fn validate_offset(offset: f64) -> Result<()> {
    if !offset.is_finite() || !(-12.0..=14.0).contains(&offset) {
        bail!("UTC offset {offset} is outside -12..=14 hours");
    }
    // Civil offsets use whole minutes. Rejecting arbitrary fractions catches input slips
    // while still accepting half/quarter-hour zones.
    if (offset * 60.0 - (offset * 60.0).round()).abs() > 1e-9 {
        bail!("UTC offset {offset} must resolve to whole minutes");
    }
    Ok(())
}

fn parse_utc_offset(raw: &str) -> Result<f64> {
    let mut text = raw.trim().to_ascii_uppercase().replace(' ', "");
    for prefix in ["TIMEZONE=", "TIME_ZONE="] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.to_string();
        }
    }
    if matches!(text.as_str(), "UTC" | "GMT" | "Z") {
        return Ok(0.0);
    }
    let signed = text
        .strip_prefix("UTC")
        .or_else(|| text.strip_prefix("GMT"))
        .unwrap_or(&text);
    let (sign, number) = match signed.as_bytes().first() {
        Some(b'+') => (1.0, &signed[1..]),
        Some(b'-') => (-1.0, &signed[1..]),
        _ => bail!("timezone must be UTC, GMT, Z, or a signed UTC offset"),
    };
    let (hours, minutes) = match number.split_once(':') {
        Some((h, m)) => (h.parse::<f64>()?, m.parse::<f64>()?),
        None if number.len() == 4 => (number[..2].parse::<f64>()?, number[2..].parse::<f64>()?),
        None => (number.parse::<f64>()?, 0.0),
    };
    if !(0.0..60.0).contains(&minutes) {
        bail!("timezone minute component {minutes} is invalid");
    }
    let offset = sign * (hours + minutes / 60.0);
    validate_offset(offset)?;
    Ok(offset)
}

/// 找到球面经纬网的最近一行、一列。经度距离按 360° 环绕，避免 -0.1°
/// 错配到 0..360 网格的远端。
pub fn nearest_grid_point(
    latitudes: &[f64],
    longitudes: &[f64],
    latitude: f64,
    longitude: f64,
) -> Result<(usize, usize)> {
    if latitudes.is_empty() || longitudes.is_empty() {
        bail!("ERA5-Land latitude/longitude coordinate is empty");
    }
    if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
        bail!("latitude {latitude} is outside -90..=90 degrees");
    }
    let lat = latitudes
        .iter()
        .enumerate()
        .filter(|(_, value)| value.is_finite())
        .min_by(|(_, a), (_, b)| (**a - latitude).abs().total_cmp(&(**b - latitude).abs()))
        .map(|(index, _)| index)
        .context("ERA5-Land latitude coordinate has no finite value")?;
    let circular = |value: f64| {
        let raw = (value - longitude).abs().rem_euclid(360.0);
        raw.min(360.0 - raw)
    };
    let lon = longitudes
        .iter()
        .enumerate()
        .filter(|(_, value)| value.is_finite())
        .min_by(|(_, a), (_, b)| circular(**a).total_cmp(&circular(**b)))
        .map(|(index, _)| index)
        .context("ERA5-Land longitude coordinate has no finite value")?;
    Ok((lat, lon))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionKind {
    Additive,
    Multiplicative,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BiasCorrection {
    pub kind: CorrectionKind,
    pub parameter: f64,
    pub overlap: usize,
}

impl BiasCorrection {
    pub fn apply(self, donor: f64) -> f64 {
        match self.kind {
            CorrectionKind::Additive => donor + self.parameter,
            CorrectionKind::Multiplicative => donor * self.parameter,
        }
    }
}

/// 用同时存在的观测与 donor 估计一个透明、可复现的全期订正。文件层会按月
/// 调用它；某月样本不足时再回退到全期订正，不用缺测段本身参与拟合。
pub fn correction(
    observed: &[Option<f64>],
    donor: &[f64],
    kind: CorrectionKind,
    min_overlap: usize,
) -> Result<BiasCorrection> {
    if observed.len() != donor.len() {
        bail!(
            "observation has {} steps but donor has {}",
            observed.len(),
            donor.len()
        );
    }
    let pairs = observed
        .iter()
        .zip(donor)
        .filter_map(|(obs, era)| {
            obs.filter(|v| v.is_finite())
                .zip(era.is_finite().then_some(*era))
        })
        .collect::<Vec<_>>();
    if pairs.len() < min_overlap {
        bail!(
            "bias correction needs at least {min_overlap} overlapping samples, found {}",
            pairs.len()
        );
    }
    let parameter = match kind {
        CorrectionKind::Additive => {
            pairs.iter().map(|(obs, era)| obs - era).sum::<f64>() / pairs.len() as f64
        }
        CorrectionKind::Multiplicative => {
            let obs_mean = pairs.iter().map(|(obs, _)| obs).sum::<f64>() / pairs.len() as f64;
            let era_mean = pairs.iter().map(|(_, era)| era).sum::<f64>() / pairs.len() as f64;
            if era_mean.abs() < f64::EPSILON {
                bail!("cannot estimate multiplicative bias because ERA5-Land overlap mean is zero");
            }
            obs_mean / era_mean
        }
    };
    if !parameter.is_finite() {
        bail!("bias correction parameter is not finite");
    }
    Ok(BiasCorrection {
        kind,
        parameter,
        overlap: pairs.len(),
    })
}

#[derive(Debug, Clone)]
pub struct RepairSlot {
    pub index: usize,
    pub source_name: String,
    pub source_units: String,
    pub also_add: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RepairPlan {
    pub slots: Vec<RepairSlot>,
    pub short_gap_max: usize,
    pub manual_utc_offset: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub era5: Option<PathBuf>,
    pub min_overlap: usize,
}

#[derive(Debug, Clone)]
pub struct VariableRepairSummary {
    pub slot: usize,
    pub variable: String,
    pub missing: usize,
    pub short_missing: usize,
    pub long_missing: usize,
    pub longest_gap: usize,
    pub interpolated: usize,
    pub era5_corrected: usize,
    pub unresolved: usize,
}

#[derive(Debug, Clone)]
pub struct RepairSummary {
    pub timezone: TimezoneDecision,
    pub latitude: f64,
    pub longitude: f64,
    pub start_utc: i64,
    pub end_utc: i64,
    pub variables: Vec<VariableRepairSummary>,
}

impl RepairSummary {
    pub fn missing(&self) -> usize {
        self.variables.iter().map(|variable| variable.missing).sum()
    }

    pub fn unresolved(&self) -> usize {
        self.variables
            .iter()
            .map(|variable| variable.unresolved)
            .sum()
    }

    pub fn needs_era5(&self) -> bool {
        self.variables
            .iter()
            .any(|variable| variable.long_missing > 0)
    }
}

/// 读取被选槽位并报告缺口，不改文件，也不要求 ERA5-Land 已经下载。
pub fn diagnose_file(src: &Path, plan: &RepairPlan) -> Result<RepairSummary> {
    let file = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let (source_local_times, _) = file_times(&file, "time", 0.0)?;
    source_step_seconds(&source_local_times)?;
    let steps = source_local_times.len();
    let (latitude, longitude) = site_coordinates(&file, plan.latitude, plan.longitude)?;
    let metadata = timezone_metadata(&file);
    let timezone = decide_timezone(plan.manual_utc_offset, metadata.as_deref(), Some(longitude))?;
    let offset_seconds = (timezone.offset_hours * 3600.0).round() as i64;
    let mut variables = Vec::with_capacity(plan.slots.len());
    for slot in &plan.slots {
        let mut values = combined_canonical(&file, src, slot, steps)?;
        let gap = analyze_gaps(&values, plan.short_gap_max);
        let qc = fill_short_gaps(&mut values, plan.short_gap_max, variable_kind(slot.index)?);
        let locally_fillable = qc.iter().filter(|value| **value == QC_INTERPOLATED).count();
        variables.push(VariableRepairSummary {
            slot: slot.index,
            variable: slot.source_name.clone(),
            missing: gap.missing,
            short_missing: locally_fillable,
            long_missing: gap.missing - locally_fillable,
            longest_gap: gap.longest_gap,
            interpolated: 0,
            era5_corrected: 0,
            unresolved: gap.missing,
        });
    }
    Ok(RepairSummary {
        timezone,
        latitude,
        longitude,
        start_utc: source_local_times[0] - offset_seconds,
        end_utc: source_local_times[steps - 1] - offset_seconds,
        variables,
    })
}

/// 生成新的已修复中间文件。原始文件只读；任何长缺口无法经偏差订正的
/// ERA5-Land donor 解决时，函数在写产物前失败。
pub fn repair_file(src: &Path, dst: &Path, plan: &RepairPlan) -> Result<RepairSummary> {
    if src == dst {
        bail!("gap repair destination must differ from its source");
    }
    let file = netcdf::open(src).with_context(|| format!("cannot open {}", src.display()))?;
    let (source_local_times, time_units) = file_times(&file, "time", 0.0)?;
    let steps = source_local_times.len();
    let (latitude, longitude) = site_coordinates(&file, plan.latitude, plan.longitude)?;
    let metadata = timezone_metadata(&file);
    let timezone = decide_timezone(plan.manual_utc_offset, metadata.as_deref(), Some(longitude))?;
    let offset_seconds = (timezone.offset_hours * 3600.0).round() as i64;
    let source_utc_times = source_local_times
        .iter()
        .map(|time| time - offset_seconds)
        .collect::<Vec<_>>();
    let months = source_local_times
        .iter()
        .map(|time| month_from_unix(*time))
        .collect::<Vec<_>>();

    let donor = plan.era5.as_deref().map(Era5Catalog::open).transpose()?;
    let mut replacements = BTreeMap::<String, Vec<f64>>::new();
    let mut quality = BTreeMap::<String, Vec<u8>>::new();
    let mut summaries = Vec::with_capacity(plan.slots.len());

    for slot in &plan.slots {
        let mut values = combined_canonical(&file, src, slot, steps)?;
        let original = values.clone();
        let gap = analyze_gaps(&values, plan.short_gap_max);
        let kind = variable_kind(slot.index)?;
        let mut qc = fill_short_gaps(&mut values, plan.short_gap_max, kind);

        if qc.contains(&QC_UNRESOLVED) {
            let catalog = donor.as_ref().with_context(|| {
                format!(
                    "slot {} ({}) has long or edge gaps; choose/download an ERA5-Land cache",
                    slot.index, slot.source_name
                )
            })?;
            let donor_values = catalog.series(
                slot.index,
                slot.index == 6 && !plan.slots.iter().any(|candidate| candidate.index == 5),
                &source_utc_times,
                latitude,
                longitude,
                source_step_seconds(&source_utc_times)?,
            )?;
            fill_from_donor(
                &mut values,
                &original,
                &donor_values,
                &months,
                &mut qc,
                slot.index,
                plan.min_overlap,
            )?;
        }

        let unresolved = qc.iter().filter(|value| **value == QC_UNRESOLVED).count();
        if unresolved > 0 {
            bail!(
                "slot {} ({}) still has {unresolved} unresolved value(s) after repair",
                slot.index,
                slot.source_name
            );
        }

        // 合并槽位的修复值写入主变量，额外相态变量在被修复的时刻置零；
        // 这样后续 forcing-convert 的总和精确等于修复后的槽位值，同时所有原始
        // 完整时刻仍逐位保留。
        let source_values = crate::units::from_canonical(
            crate::convert::canonical_units(slot.index),
            &slot.source_units,
            &values,
        )?;
        let mut primary = variable_values(&file, src, &slot.source_name, steps)?;
        for (index, code) in qc.iter().enumerate() {
            if *code != QC_OBSERVED {
                primary[index] = source_values[index];
            }
        }
        replacements.insert(slot.source_name.clone(), primary);
        for extra in &slot.also_add {
            let mut extra_values = variable_values(&file, src, extra, steps)?;
            for (index, code) in qc.iter().enumerate() {
                if *code != QC_OBSERVED {
                    extra_values[index] = 0.0;
                }
            }
            replacements.insert(extra.clone(), extra_values);
        }
        quality.insert(slot.source_name.clone(), qc.clone());
        summaries.push(VariableRepairSummary {
            slot: slot.index,
            variable: slot.source_name.clone(),
            missing: gap.missing,
            short_missing: qc.iter().filter(|value| **value == QC_INTERPOLATED).count(),
            long_missing: qc
                .iter()
                .filter(|value| **value == QC_ERA5_CORRECTED)
                .count(),
            longest_gap: gap.longest_gap,
            interpolated: qc.iter().filter(|value| **value == QC_INTERPOLATED).count(),
            era5_corrected: qc
                .iter()
                .filter(|value| **value == QC_ERA5_CORRECTED)
                .count(),
            unresolved,
        });
    }
    drop(file);

    write_repaired(
        src,
        dst,
        &replacements,
        &quality,
        plan,
        timezone,
        latitude,
        longitude,
        donor.as_ref().map(|catalog| catalog.label.as_str()),
        &time_units,
    )?;
    Ok(RepairSummary {
        timezone,
        latitude,
        longitude,
        start_utc: source_utc_times[0],
        end_utc: source_utc_times[steps - 1],
        variables: summaries,
    })
}

fn variable_kind(slot: usize) -> Result<VariableKind> {
    match slot {
        4 => Ok(VariableKind::Precipitation),
        2 | 7 | 8 => Ok(VariableKind::NonNegative),
        1 | 3 | 5 | 6 => Ok(VariableKind::Continuous),
        _ => bail!("unknown CoLM forcing slot {slot}"),
    }
}

fn correction_kind(slot: usize) -> Result<CorrectionKind> {
    match slot {
        4 | 7 | 8 => Ok(CorrectionKind::Multiplicative),
        1 | 2 | 3 | 5 | 6 => Ok(CorrectionKind::Additive),
        _ => bail!("unknown CoLM forcing slot {slot}"),
    }
}

fn fill_from_donor(
    values: &mut [f64],
    original: &[f64],
    donor: &[f64],
    months: &[u32],
    qc: &mut [u8],
    slot: usize,
    min_overlap: usize,
) -> Result<()> {
    if values.len() != donor.len() || values.len() != months.len() || values.len() != qc.len() {
        bail!("source, ERA5-Land, month and QC axes have different lengths");
    }
    let observed = original
        .iter()
        .map(|value| value.is_finite().then_some(*value))
        .collect::<Vec<_>>();
    let kind = correction_kind(slot)?;
    let global = correction(&observed, donor, kind, min_overlap)?;
    let mut by_month = BTreeMap::<u32, BiasCorrection>::new();
    for month in 1..=12 {
        let mut month_obs = Vec::new();
        let mut month_donor = Vec::new();
        for index in 0..months.len() {
            if months[index] == month {
                month_obs.push(observed[index]);
                month_donor.push(donor[index]);
            }
        }
        if let Ok(value) = correction(&month_obs, &month_donor, kind, min_overlap) {
            by_month.insert(month, value);
        }
    }
    for index in 0..values.len() {
        if qc[index] != QC_UNRESOLVED {
            continue;
        }
        let era = donor[index];
        if !era.is_finite() {
            continue;
        }
        let bias = by_month.get(&months[index]).copied().unwrap_or(global);
        let mut value = bias.apply(era);
        if matches!(
            variable_kind(slot)?,
            VariableKind::NonNegative | VariableKind::Precipitation
        ) {
            value = value.max(0.0);
        }
        if value.is_finite() {
            values[index] = value;
            qc[index] = QC_ERA5_CORRECTED;
        }
    }
    Ok(())
}

fn combined_canonical(
    file: &netcdf::File,
    path: &Path,
    slot: &RepairSlot,
    steps: usize,
) -> Result<Vec<f64>> {
    let mut values = variable_values(file, path, &slot.source_name, steps)?;
    let primary_fill = fill_value(file, &slot.source_name);
    normalize_missing(&mut values, primary_fill);
    for extra in &slot.also_add {
        let mut add = variable_values(file, path, extra, steps)?;
        normalize_missing(&mut add, fill_value(file, extra));
        for (value, extra_value) in values.iter_mut().zip(add) {
            if value.is_finite() && extra_value.is_finite() {
                *value += extra_value;
            } else {
                *value = f64::NAN;
            }
        }
    }
    crate::units::convert_units(
        &slot.source_units,
        crate::convert::canonical_units(slot.index),
        &values,
    )
}

fn variable_values(file: &netcdf::File, path: &Path, name: &str, steps: usize) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    let values: Vec<f64> = variable
        .get_values(netcdf::Extents::All)
        .with_context(|| format!("cannot read {name} from {}", path.display()))?;
    if values.len() != steps {
        bail!(
            "{} has {} values but the time axis has {steps}; gap repair accepts a point series, not a spatial field",
            name,
            values.len()
        );
    }
    Ok(values)
}

fn fill_value(file: &netcdf::File, name: &str) -> Option<f64> {
    file.variable(name).and_then(|variable| {
        variable
            .fill_value::<f64>()
            .ok()
            .flatten()
            .or_else(|| variable.fill_value::<f32>().ok().flatten().map(f64::from))
    })
}

fn normalize_missing(values: &mut [f64], fill: Option<f64>) {
    for value in values {
        if !value.is_finite()
            || fill.is_some_and(|missing| {
                let tolerance = missing.abs().max(1.0) * 1e-12;
                (*value - missing).abs() <= tolerance
            })
        {
            *value = f64::NAN;
        }
    }
}

fn source_step_seconds(times: &[i64]) -> Result<i64> {
    let Some((&first, rest)) = times.split_first() else {
        bail!("forcing time axis is empty");
    };
    let Some(second) = rest.first() else {
        bail!("at least two time steps are required for gap repair");
    };
    let step = *second - first;
    if step <= 0 || !times.windows(2).all(|pair| pair[1] - pair[0] == step) {
        bail!(
            "forcing time axis is not strictly uniform; gap repair will not silently resample it"
        );
    }
    Ok(step)
}

fn site_coordinates(
    file: &netcdf::File,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> Result<(f64, f64)> {
    let latitude =
        latitude.or_else(|| scalar_variable(file, &["latitude", "lat", "LATITUDE", "LAT"]));
    let longitude =
        longitude.or_else(|| scalar_variable(file, &["longitude", "lon", "LONGITUDE", "LON"]));
    let latitude = latitude.context("site latitude is absent; give it explicitly")?;
    let longitude = longitude.context("site longitude is absent; give it explicitly")?;
    if !(-90.0..=90.0).contains(&latitude) {
        bail!("site latitude {latitude} is outside -90..=90");
    }
    if !(-180.0..=180.0).contains(&longitude) {
        bail!("site longitude {longitude} is outside -180..=180");
    }
    Ok((latitude, longitude))
}

fn scalar_variable(file: &netcdf::File, names: &[&str]) -> Option<f64> {
    names.iter().find_map(|name| {
        file.variable(name)
            .and_then(|variable| variable.get_values::<f64, _>(netcdf::Extents::All).ok())
            .and_then(|values| values.first().copied())
            .filter(|value| value.is_finite())
    })
}

fn string_attribute(file: &netcdf::File, name: &str) -> Option<String> {
    file.attribute(name)
        .and_then(|attribute| match attribute.value() {
            Ok(netcdf::AttributeValue::Str(value)) => Some(value),
            Ok(netcdf::AttributeValue::Strs(values)) => values.into_iter().next(),
            _ => None,
        })
}

fn timezone_metadata(file: &netcdf::File) -> Option<String> {
    for name in ["time_shown_in", "timezone", "time_zone"] {
        if let Some(value) = string_attribute(file, name) {
            let normalized = value.trim().to_ascii_lowercase();
            if !matches!(
                normalized.as_str(),
                "local" | "local time" | "local standard time" | "lst"
            ) {
                return Some(value);
            }
        }
    }
    for name in [
        "colm_gapfill_timezone_offset_hours",
        "local_utc_offset_hours",
        "utc_offset_hours",
        "utc_offset",
    ] {
        if let Some(value) = numeric_attribute(file, name) {
            return Some(format!("UTC{value:+}"));
        }
        if let Some(value) = string_attribute(file, name) {
            return Some(value);
        }
    }
    None
}

fn numeric_attribute(file: &netcdf::File, name: &str) -> Option<f64> {
    file.attribute(name)
        .and_then(|attribute| attribute.value().ok())
        .and_then(|value| match value {
            netcdf::AttributeValue::Double(value) => Some(value),
            netcdf::AttributeValue::Float(value) => Some(f64::from(value)),
            netcdf::AttributeValue::Int(value) => Some(f64::from(value)),
            netcdf::AttributeValue::Short(value) => Some(f64::from(value)),
            _ => None,
        })
}

fn month_from_unix(seconds: i64) -> u32 {
    let days = seconds.div_euclid(86400);
    crate::civil::civil_from_days(days).1
}

fn file_times(file: &netcdf::File, name: &str, offset_hours: f64) -> Result<(Vec<i64>, String)> {
    let variable = file
        .variable(name)
        .with_context(|| format!("file has no {name} time coordinate"))?;
    let units = variable
        .attribute("units")
        .context("time coordinate has no units")?
        .value()?;
    let units = match units {
        netcdf::AttributeValue::Str(value) => value,
        netcdf::AttributeValue::Strs(values) => values
            .into_iter()
            .next()
            .context("time units string array is empty")?,
        other => bail!("time units is not a string: {other:?}"),
    };
    let (factor, origin) = parse_cf_time_units(&units)?;
    let offset = (offset_hours * 3600.0).round() as i64;
    let values: Vec<f64> = variable.get_values(netcdf::Extents::All)?;
    let times = values
        .into_iter()
        .map(|value| origin + (value * factor).round() as i64 - offset)
        .collect();
    Ok((times, units))
}

fn parse_cf_time_units(units: &str) -> Result<(f64, i64)> {
    let (unit, origin) = units
        .split_once(" since ")
        .with_context(|| format!("time units {units:?} has no ' since '"))?;
    let factor = match unit.trim().to_ascii_lowercase().as_str() {
        "seconds" | "second" | "sec" | "s" => 1.0,
        "minutes" | "minute" | "min" => 60.0,
        "hours" | "hour" | "h" => 3600.0,
        "days" | "day" | "d" => 86400.0,
        other => bail!("unsupported time unit {other:?}"),
    };
    let cleaned = origin.trim().trim_end_matches('Z').trim();
    if cleaned.len() < 10 {
        bail!("time origin {cleaned:?} is too short");
    }
    let date = &cleaned[..10];
    let time = cleaned.get(11..19).unwrap_or("00:00:00");
    let mut date_parts = date.split('-');
    let year: i32 = date_parts
        .next()
        .context("time origin has no year")?
        .parse()?;
    let month: u32 = date_parts
        .next()
        .context("time origin has no month")?
        .parse()?;
    let day: u32 = date_parts
        .next()
        .context("time origin has no day")?
        .parse()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next().unwrap_or("0").parse()?;
    let minute: i64 = time_parts.next().unwrap_or("0").parse()?;
    let second: i64 = time_parts
        .next()
        .unwrap_or("0")
        .split('.')
        .next()
        .unwrap_or("0")
        .parse()?;
    let origin = crate::civil::days_from_civil(year, month, day) * 86400
        + hour * 3600
        + minute * 60
        + second;
    Ok((factor, origin))
}

struct Era5Catalog {
    files: Vec<PathBuf>,
    label: String,
}

impl Era5Catalog {
    fn open(path: &Path) -> Result<Self> {
        let mut files = if path.is_dir() {
            std::fs::read_dir(path)
                .with_context(|| format!("cannot read ERA5-Land directory {}", path.display()))?
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .filter(|entry| {
                    entry
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("nc"))
                })
                .collect::<Vec<_>>()
        } else {
            vec![path.to_path_buf()]
        };
        files.sort();
        if files.is_empty() {
            bail!("{} contains no ERA5-Land .nc file", path.display());
        }
        Ok(Self {
            files,
            label: path.display().to_string(),
        })
    }

    fn series(
        &self,
        slot: usize,
        scalar_wind: bool,
        target_times: &[i64],
        latitude: f64,
        longitude: f64,
        target_step: i64,
    ) -> Result<Vec<f64>> {
        match slot {
            2 => {
                if let Ok(q) = self.variable_series(
                    &["q", "Qair", "specific_humidity"],
                    target_times,
                    latitude,
                    longitude,
                    target_step,
                    DonorTransform::Identity,
                    VariableKind::NonNegative,
                ) {
                    return Ok(q);
                }
                let dewpoint = self.variable_series(
                    &["d2m", "2m_dewpoint_temperature"],
                    target_times,
                    latitude,
                    longitude,
                    target_step,
                    DonorTransform::Identity,
                    VariableKind::Continuous,
                )?;
                let pressure =
                    self.series(3, false, target_times, latitude, longitude, target_step)?;
                Ok(dewpoint
                    .into_iter()
                    .zip(pressure)
                    .map(|(dewpoint, pressure)| specific_humidity(dewpoint, pressure))
                    .collect())
            }
            1 => self.variable_series(
                &["t2m", "2m_temperature", "Tair"],
                target_times,
                latitude,
                longitude,
                target_step,
                DonorTransform::Identity,
                VariableKind::Continuous,
            ),
            3 => self.variable_series(
                &["sp", "surface_pressure", "PSurf", "Psurf"],
                target_times,
                latitude,
                longitude,
                target_step,
                DonorTransform::Identity,
                VariableKind::Continuous,
            ),
            4 => self.variable_series(
                &["tp", "total_precipitation", "Precip"],
                target_times,
                latitude,
                longitude,
                target_step,
                DonorTransform::AccumulatedWater,
                VariableKind::Precipitation,
            ),
            5 => self.variable_series(
                &["u10", "10m_u_component_of_wind", "Wind_E"],
                target_times,
                latitude,
                longitude,
                target_step,
                DonorTransform::Identity,
                VariableKind::Continuous,
            ),
            6 if scalar_wind => {
                let east = self.series(5, false, target_times, latitude, longitude, target_step)?;
                let north =
                    self.series(6, false, target_times, latitude, longitude, target_step)?;
                Ok(east
                    .into_iter()
                    .zip(north)
                    .map(|(east, north)| east.hypot(north))
                    .collect())
            }
            6 => self.variable_series(
                &["v10", "10m_v_component_of_wind", "Wind_N"],
                target_times,
                latitude,
                longitude,
                target_step,
                DonorTransform::Identity,
                VariableKind::Continuous,
            ),
            7 => self.variable_series(
                &["ssrd", "surface_solar_radiation_downwards", "SWdown"],
                target_times,
                latitude,
                longitude,
                target_step,
                DonorTransform::AccumulatedEnergy,
                VariableKind::NonNegative,
            ),
            8 => self.variable_series(
                &["strd", "surface_thermal_radiation_downwards", "LWdown"],
                target_times,
                latitude,
                longitude,
                target_step,
                DonorTransform::AccumulatedEnergy,
                VariableKind::NonNegative,
            ),
            _ => bail!("unknown CoLM forcing slot {slot}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn variable_series(
        &self,
        candidates: &[&str],
        target_times: &[i64],
        latitude: f64,
        longitude: f64,
        target_step: i64,
        transform: DonorTransform,
        kind: VariableKind,
    ) -> Result<Vec<f64>> {
        let mut merged = BTreeMap::<i64, f64>::new();
        let mut found = false;
        for path in &self.files {
            let file = netcdf::open(path)
                .with_context(|| format!("cannot open ERA5-Land file {}", path.display()))?;
            let Some(name) = candidates
                .iter()
                .find(|candidate| file.variable(candidate).is_some())
            else {
                continue;
            };
            found = true;
            let time_name = if file.variable("valid_time").is_some() {
                "valid_time"
            } else {
                "time"
            };
            let (times, _) = file_times(&file, time_name, 0.0)?;
            let variable = file
                .variable(name)
                .context("ERA5-Land variable disappeared")?;
            let mut values = extract_grid_series(&file, &variable, time_name, latitude, longitude)?;
            normalize_missing(&mut values, fill_value(&file, name));
            let units = variable
                .attribute_value("units")
                .and_then(|value| value.ok())
                .and_then(|value| match value {
                    netcdf::AttributeValue::Str(value) => Some(value),
                    netcdf::AttributeValue::Strs(values) => values.into_iter().next(),
                    _ => None,
                })
                .unwrap_or_default();
            apply_donor_transform(&times, &mut values, &units, transform)?;
            for (time, value) in times.into_iter().zip(values) {
                // Monthly downloads overlap by a padded day. Prefer a finite value if
                // the first file's boundary sample could not be de-accumulated.
                match merged.entry(time) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(value);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry)
                        if !entry.get().is_finite() && value.is_finite() =>
                    {
                        entry.insert(value);
                    }
                    _ => {}
                }
            }
        }
        if !found {
            bail!(
                "ERA5-Land cache {} has none of: {}",
                self.label,
                candidates.join(", ")
            );
        }
        let (times, values): (Vec<_>, Vec<_>) = merged.into_iter().unzip();
        sample_donor(&times, &values, target_times, target_step, kind)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DonorTransform {
    Identity,
    AccumulatedWater,
    AccumulatedEnergy,
}

fn extract_grid_series(
    file: &netcdf::File,
    variable: &netcdf::Variable,
    time_name: &str,
    latitude: f64,
    longitude: f64,
) -> Result<Vec<f64>> {
    let dims = variable.dimensions();
    let time_dim = dims
        .iter()
        .position(|dimension| dimension.name() == time_name || dimension.name() == "time")
        .with_context(|| {
            format!(
                "ERA5-Land variable {} has no time dimension",
                variable.name()
            )
        })?;
    let lat_name = ["latitude", "lat"]
        .iter()
        .find(|name| file.variable(name).is_some())
        .copied();
    let lon_name = ["longitude", "lon"]
        .iter()
        .find(|name| file.variable(name).is_some())
        .copied();
    let (lat_index, lon_index) = match (lat_name, lon_name) {
        (Some(lat_name), Some(lon_name)) => {
            let lats: Vec<f64> = file
                .variable(lat_name)
                .unwrap()
                .get_values(netcdf::Extents::All)?;
            let lons: Vec<f64> = file
                .variable(lon_name)
                .unwrap()
                .get_values(netcdf::Extents::All)?;
            let (lat, lon) = nearest_grid_point(&lats, &lons, latitude, longitude)?;
            let lat_distance = (lats[lat] - latitude).abs();
            let raw_lon_distance = (lons[lon] - longitude).abs().rem_euclid(360.0);
            let lon_distance = raw_lon_distance.min(360.0 - raw_lon_distance);
            const MAX_ERA5_GRID_DISTANCE_DEGREES: f64 = 0.15;
            if lat_distance > MAX_ERA5_GRID_DISTANCE_DEGREES
                || lon_distance > MAX_ERA5_GRID_DISTANCE_DEGREES
            {
                bail!(
                    "nearest ERA5-Land grid point ({}, {}) is too far from site ({latitude}, {longitude}); choose/download the corresponding 0.1-degree grid cache",
                    lats[lat],
                    lons[lon]
                );
            }
            (Some((lat_name, lat)), Some((lon_name, lon)))
        }
        _ => bail!(
            "ERA5-Land donor {} must include latitude and longitude coordinates so the site grid point can be verified",
            variable.name()
        ),
    };
    let raw: Vec<f64> = variable.get_values(netcdf::Extents::All)?;
    let shape = dims
        .iter()
        .map(|dimension| dimension.len())
        .collect::<Vec<_>>();
    let time_len = shape[time_dim];
    let mut output = Vec::with_capacity(time_len);
    for time in 0..time_len {
        let mut flat = 0;
        for (axis, dimension) in dims.iter().enumerate() {
            let index = if axis == time_dim {
                time
            } else if lat_index
                .as_ref()
                .is_some_and(|(name, _)| dimension.name() == *name)
            {
                lat_index.unwrap().1
            } else if lon_index
                .as_ref()
                .is_some_and(|(name, _)| dimension.name() == *name)
            {
                lon_index.unwrap().1
            } else if dimension.len() == 1 {
                0
            } else {
                bail!(
                    "ERA5-Land variable {} has unsupported non-singleton dimension {}={}",
                    variable.name(),
                    dimension.name(),
                    dimension.len()
                );
            };
            flat = flat * shape[axis] + index;
        }
        output.push(raw[flat]);
    }
    Ok(output)
}

fn apply_donor_transform(
    times: &[i64],
    values: &mut [f64],
    units: &str,
    transform: DonorTransform,
) -> Result<()> {
    if transform == DonorTransform::Identity {
        return Ok(());
    }
    if times.len() != values.len() {
        bail!("ERA5-Land time and value axes differ");
    }
    let step = source_step_seconds(times)? as f64;
    // Downloaded ERA5-Land accumulated fields are cumulative within the forecast day.
    // Convert them to each interval before unit conversion. At 01 UTC a new daily
    // forecast starts; at other hours subtract the preceding accumulation.
    let original = values.to_vec();
    for index in 0..values.len() {
        if !original[index].is_finite() {
            values[index] = f64::NAN;
            continue;
        }
        let hour = times[index].rem_euclid(86400) / 3600;
        let amount = if hour == 1 {
            original[index]
        } else if index > 0 && original[index - 1].is_finite() {
            original[index] - original[index - 1]
        } else {
            f64::NAN
        };
        values[index] = if amount.is_finite() {
            match transform {
                DonorTransform::AccumulatedWater => {
                    let normalized = units.to_ascii_lowercase().replace(' ', "");
                    if normalized.contains("/s") || normalized.contains("s-1") {
                        original[index].max(0.0)
                    } else if units.eq_ignore_ascii_case("m") {
                        amount.max(0.0) * 1000.0 / step
                    } else if units.contains("kg") || units.contains("mm") {
                        amount.max(0.0) / step
                    } else {
                        bail!("unsupported ERA5-Land precipitation unit {units:?}");
                    }
                }
                DonorTransform::AccumulatedEnergy => {
                    if units.contains('J') || units.contains('j') {
                        amount.max(0.0) / step
                    } else if units.contains('W') || units.contains('w') {
                        // Some pre-normalized cache files already contain a rate.
                        original[index].max(0.0)
                    } else {
                        bail!("unsupported ERA5-Land radiation unit {units:?}");
                    }
                }
                DonorTransform::Identity => unreachable!(),
            }
        } else {
            f64::NAN
        };
    }
    Ok(())
}

fn sample_donor(
    times: &[i64],
    values: &[f64],
    targets: &[i64],
    target_step: i64,
    kind: VariableKind,
) -> Result<Vec<f64>> {
    if times.len() != values.len() || times.len() < 2 {
        bail!("ERA5-Land series needs matching time/value axes with at least two steps");
    }
    if !times.windows(2).all(|pair| pair[1] > pair[0]) {
        bail!("ERA5-Land time axis is not strictly increasing");
    }
    let tolerance = (target_step.abs().min(times[1] - times[0]) / 2).max(1);
    let mut output = Vec::with_capacity(targets.len());
    for target in targets {
        match times.binary_search(target) {
            Ok(index) => output.push(values[index]),
            Err(right) if right > 0 && right < times.len() => {
                let left = right - 1;
                let left_distance = target - times[left];
                let right_distance = times[right] - target;
                if left_distance > tolerance && right_distance > tolerance {
                    output.push(f64::NAN);
                } else if matches!(kind, VariableKind::Precipitation) {
                    output.push(if left_distance <= right_distance {
                        values[left]
                    } else {
                        values[right]
                    });
                } else if values[left].is_finite() && values[right].is_finite() {
                    let fraction = left_distance as f64 / (times[right] - times[left]) as f64;
                    output.push(values[left] + (values[right] - values[left]) * fraction);
                } else {
                    output.push(f64::NAN);
                }
            }
            _ => output.push(f64::NAN),
        }
    }
    Ok(output)
}

fn specific_humidity(dewpoint_kelvin: f64, pressure_pa: f64) -> f64 {
    if !dewpoint_kelvin.is_finite() || !pressure_pa.is_finite() || pressure_pa <= 0.0 {
        return f64::NAN;
    }
    let dewpoint_c = dewpoint_kelvin - 273.15;
    let vapor_pressure = 611.2 * ((17.67 * dewpoint_c) / (dewpoint_c + 243.5)).exp();
    (0.622 * vapor_pressure / (pressure_pa - 0.378 * vapor_pressure)).max(0.0)
}

#[allow(clippy::too_many_arguments)]
fn write_repaired(
    src: &Path,
    dst: &Path,
    replacements: &BTreeMap<String, Vec<f64>>,
    quality: &BTreeMap<String, Vec<u8>>,
    plan: &RepairPlan,
    timezone: TimezoneDecision,
    latitude: f64,
    longitude: f64,
    era5_source: Option<&str>,
    time_units: &str,
) -> Result<()> {
    crate::convert::ensure_parent(dst)?;
    let file_name = dst
        .file_name()
        .and_then(|name| name.to_str())
        .context("gap repair destination has no filename")?;
    let temp = dst.with_file_name(format!(".{file_name}.gapfill-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    std::fs::copy(src, &temp).with_context(|| {
        format!(
            "cannot copy original forcing {} to temporary repair file {}",
            src.display(),
            temp.display()
        )
    })?;
    let result = update_repaired_file(
        &temp,
        replacements,
        quality,
        plan,
        timezone,
        latitude,
        longitude,
        era5_source,
        time_units,
    );
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    install_repaired_file(&temp, dst)
}

#[allow(clippy::too_many_arguments)]
fn update_repaired_file(
    path: &Path,
    replacements: &BTreeMap<String, Vec<f64>>,
    quality: &BTreeMap<String, Vec<u8>>,
    plan: &RepairPlan,
    timezone: TimezoneDecision,
    latitude: f64,
    longitude: f64,
    era5_source: Option<&str>,
    time_units: &str,
) -> Result<()> {
    let mut output =
        netcdf::append(path).with_context(|| format!("cannot update {}", path.display()))?;
    for (name, values) in replacements {
        let mut variable = output
            .variable_mut(name)
            .with_context(|| format!("source variable {name} disappeared"))?;
        variable.put_attribute(
            "gapfill_note",
            "missing values repaired by CoLM Desktop; see paired *_gapfill_qc",
        )?;
        variable.put_values(values, netcdf::Extents::All)?;
    }
    for (name, values) in quality {
        let dimensions = output
            .variable(name)
            .with_context(|| format!("source variable {name} disappeared"))?
            .dimensions()
            .iter()
            .map(|dimension| dimension.name())
            .collect::<Vec<_>>();
        let refs = dimensions.iter().map(String::as_str).collect::<Vec<_>>();
        let quality_name = format!("{name}_gapfill_qc");
        let mut variable = match output.variable_mut(&quality_name) {
            Some(variable) => variable,
            None => output.add_variable::<u8>(&quality_name, &refs)?,
        };
        variable.put_attribute("long_name", format!("gap-fill provenance for {name}"))?;
        variable.put_attribute(
            "flag_meanings",
            "observed short_gap_interpolation era5_land_bias_corrected unresolved",
        )?;
        variable.put_attribute("flag_values", "0 1 2 9")?;
        variable.put_values(values, netcdf::Extents::All)?;
    }
    output.add_attribute("colm_gapfill_version", env!("CARGO_PKG_VERSION"))?;
    output.add_attribute("colm_gapfill_timezone_offset_hours", timezone.offset_hours)?;
    output.add_attribute("colm_gapfill_timezone_source", timezone.source.as_str())?;
    output.add_attribute(
        "time_shown_in",
        if timezone.offset_hours.abs() < 1e-12 {
            "UTC"
        } else {
            "local"
        },
    )?;
    output.add_attribute("colm_gapfill_latitude", latitude)?;
    output.add_attribute("colm_gapfill_longitude", longitude)?;
    output.add_attribute(
        "colm_gapfill_short_gap_max_steps",
        plan.short_gap_max as i64,
    )?;
    output.add_attribute(
        "colm_gapfill_correction",
        "monthly additive for state variables; monthly multiplicative for precipitation/radiation; global fallback",
    )?;
    output.add_attribute("colm_gapfill_time_units", time_units)?;
    output.add_attribute("colm_gapfill_era5_source", era5_source.unwrap_or(""))?;
    Ok(())
}

fn install_repaired_file(temp: &Path, dst: &Path) -> Result<()> {
    if !dst.exists() {
        return std::fs::rename(temp, dst)
            .with_context(|| format!("cannot install repaired file {}", dst.display()));
    }
    let file_name = dst
        .file_name()
        .and_then(|name| name.to_str())
        .context("gap repair destination has no filename")?;
    let backup = dst.with_file_name(format!(
        ".{file_name}.gapfill-{}.backup",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&backup);
    std::fs::rename(dst, &backup)
        .with_context(|| format!("cannot preserve previous repaired file {}", dst.display()))?;
    if let Err(error) = std::fs::rename(temp, dst) {
        let restore = std::fs::rename(&backup, dst);
        return match restore {
            Ok(()) => Err(error).with_context(|| {
                format!("cannot install repaired file {}; previous file restored", dst.display())
            }),
            Err(restore_error) => bail!(
                "cannot install repaired file {} ({error}); previous file is preserved at {} but could not be restored ({restore_error})",
                dst.display(),
                backup.display()
            ),
        };
    }
    std::fs::remove_file(&backup)
        .with_context(|| format!("cannot remove repair backup {}", backup.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "gapfill_tests.rs"]
mod gapfill_tests;
