//! 示踪物运行时强迫（`MOD_Tracer_ForcingInput` + `MOD_Tracer_Forcing` 的网格路径）。
//!
//! 每个走陆面水输运的示踪物可在自己的参数文件里用 `&nl_colm_tracer_forcing` 声明 `precip`、
//! `vapor` 两类输入。逐变量维护上下界记录（与主强迫同一套 `setstampLB/UB`），在强迫格上做时间
//! 插值、`grid2pset` 到 patch，再按 `input_mode` 解码成比值。解码失败（近干、缺测、越界）就保留
//! 上一次的有效值；起点是描述符的默认比值。`*_over_total` 模式另读一份主强迫的总降水/总比湿
//! —— 直接读文件，**不经** `metpreprocess`。
//!
//! 「最近一次有效值」是预报量：写进陆面重启（`trc_forcing_*_last`），续跑原样读回。

use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};
use colm_core::tracer::descriptor::{delta_to_ratio, param_file_for_index, TRC_TINY};
use colm_core::tracer::{TracerPhysics, TracerSet};
use colm_core::CalendarTime;
use colm_namelist::{Segment, Value};

use super::forcing::{GriddedForcing, GriddedForcingConfig, GroupBy, Interpolation, Stamp, Variable};
use super::mapping::AreaWeightedMapping;

/// `TRACER_FORCING_MAX`。
pub const TRACER_FORCING_MAX: usize = 8;
/// `trc_forc_min_prcp`、`trc_forc_min_q`、`trc_forc_max_abs`。
const MIN_PRECIP: f64 = 1.0e-7;
const MIN_Q: f64 = 1.0e-12;
const MAX_ABS: f64 = 1.0e10;
/// `trc_delta_sanity_max`。
const DELTA_SANITY_MAX: f64 = 2.0e3;
/// `TRC_FORC_CACHE_SCHEMA`、`TRC_FORC_ID_WIDTH`。
pub const CACHE_SCHEMA: i32 = 1;
pub const ID_WIDTH: usize = 8 + 6 * 256;

/// `tracer_forcing_spec_type`（字段已按上游规范化：`role`/`tintalgo`/`input_mode` 小写、全部左对齐）。
#[derive(Debug, Clone, PartialEq)]
pub struct ForcingSpec {
    pub role: String,
    pub fprefix: String,
    pub vname: String,
    pub tintalgo: String,
    pub dtime: i32,
    pub offset: i32,
    pub input_mode: String,
}

impl Default for ForcingSpec {
    fn default() -> Self {
        Self {
            role: "none".into(),
            fprefix: "null".into(),
            vname: "null".into(),
            tintalgo: "linear".into(),
            dtime: 21_600,
            offset: 0,
            input_mode: "normalized_over_total".into(),
        }
    }
}

/// `tracer_forcing_input_load`：逐示踪物读参数文件里的 `&nl_colm_tracer_forcing`。
/// 没有参数文件、文件不存在或没有这一组的示踪物，输入个数为 0。
pub fn load_specs(set: &TracerSet, param_files: &str) -> Result<Vec<Vec<ForcingSpec>>> {
    let mut out = Vec::with_capacity(set.len());
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        let Some(path) = param_file_for_index(param_files, &set.tracers, itrc)? else {
            out.push(Vec::new());
            continue;
        };
        let Ok(source) = std::fs::read_to_string(&path) else {
            out.push(Vec::new());
            continue;
        };
        let Some(group) = forcing_group(&source) else {
            out.push(Vec::new());
            continue;
        };
        let document = colm_namelist::parse(&group)
            .with_context(|| format!("invalid &nl_colm_tracer_forcing in {path}"))?;
        let mut num: i64 = 0;
        let mut specs = vec![ForcingSpec::default(); TRACER_FORCING_MAX];
        for item in &document.items {
            let colm_namelist::document::Item::Entry(entry) = item else {
                continue;
            };
            let (name, index) = match entry.path.segments.as_slice() {
                [Segment::Field(name)] => (name.to_ascii_lowercase(), None),
                [Segment::Field(name), Segment::Index(k)] => (name.to_ascii_lowercase(), Some(*k)),
                _ => bail!("invalid &nl_colm_tracer_forcing entry {} in {path}", entry.path),
            };
            if name == "forcing_num" {
                let Value::Int(value) = entry.value else {
                    bail!("forcing_num in {path} must be an integer");
                };
                num = value;
                continue;
            }
            // 整列赋值从 1 号元素起依次填；带下标的赋值只改那一个。
            let values: Vec<&Value> = match &entry.value {
                Value::List(items) => items.iter().collect(),
                other => vec![other],
            };
            let start = index.unwrap_or(1);
            ensure!(
                start >= 1 && start - 1 + values.len() <= TRACER_FORCING_MAX,
                "{name} index out of range in {path}"
            );
            for (k, value) in values.into_iter().enumerate() {
                let spec = &mut specs[start - 1 + k];
                let text = || -> Result<String> {
                    match value {
                        Value::Str(s) => Ok(s.clone()),
                        other => bail!("{name} in {path} must hold strings, got {other:?}"),
                    }
                };
                let int = || -> Result<i32> {
                    match value {
                        Value::Int(i) => Ok(i32::try_from(*i)?),
                        other => bail!("{name} in {path} must hold integers, got {other:?}"),
                    }
                };
                match name.as_str() {
                    "forcing_role" => spec.role = text()?,
                    "forcing_fprefix" => spec.fprefix = text()?,
                    "forcing_vname" => spec.vname = text()?,
                    "forcing_tintalgo" => spec.tintalgo = text()?,
                    "forcing_dtime" => spec.dtime = int()?,
                    "forcing_offset" => spec.offset = int()?,
                    "forcing_input_mode" => spec.input_mode = text()?,
                    other => bail!("invalid &nl_colm_tracer_forcing in {path}: unknown {other}"),
                }
            }
        }
        ensure!(
            (0..=TRACER_FORCING_MAX as i64).contains(&num),
            "tracer_forcing_input_load: forcing_num={num} must be between 0 and \
             {TRACER_FORCING_MAX} in {path}."
        );
        specs.truncate(num as usize);
        for (k, spec) in specs.iter_mut().enumerate() {
            let k = k + 1;
            ensure!(
                spec.dtime > 0,
                "tracer_forcing_input_load: forcing_dtime({k}) for tracer \"{}\" must be > 0, got {}.",
                tracer.name,
                spec.dtime
            );
            spec.tintalgo = spec.tintalgo.trim_start().to_ascii_lowercase();
            ensure!(
                matches!(spec.tintalgo.trim_end(), "linear" | "nearest"),
                "tracer_forcing_input_load: forcing_tintalgo({k}) for tracer \"{}\" has invalid value \"{}\".",
                tracer.name,
                spec.tintalgo.trim_end()
            );
            spec.input_mode = spec.input_mode.trim_start().to_ascii_lowercase();
            ensure!(
                parse_mode(&spec.input_mode).is_some(),
                "tracer_forcing_input_load: forcing_input_mode({k}) for tracer \"{}\" has invalid value \"{}\".",
                tracer.name,
                spec.input_mode.trim_end()
            );
            spec.role = spec.role.trim_start().to_ascii_lowercase();
            ensure!(
                matches!(spec.role.trim_end(), "precip" | "vapor")
                    && tracer.uses_land_water_transport(),
                "tracer_forcing_input_load: forcing_role({k}) for tracer \"{}\" has invalid value \"{}\".",
                tracer.name,
                spec.role.trim_end()
            );
            spec.fprefix = spec.fprefix.trim_start().to_owned();
            spec.vname = spec.vname.trim_start().to_owned();
        }
        for k in 1..specs.len() {
            if let Some(kdup) = (0..k).find(|&j| specs[j].role.trim_end() == specs[k].role.trim_end()) {
                bail!(
                    "tracer_forcing_input_load: forcing_role({}) and ({}) for tracer \"{}\" are both \"{}\".",
                    kdup + 1,
                    k + 1,
                    tracer.name,
                    specs[k].role.trim_end()
                );
            }
        }
        out.push(specs);
    }
    Ok(out)
}

/// `tracer_forcing_group_present` 判定后，截出 `&nl_colm_tracer_forcing` 那一组的原文。
fn forcing_group(source: &str) -> Option<String> {
    let mut lines = source.lines();
    let header = lines.by_ref().find(|line| {
        let low = line.trim_start().to_ascii_lowercase();
        !low.starts_with('!')
            && (low.starts_with("&nl_colm_tracer_forcing") || low.starts_with("$nl_colm_tracer_forcing"))
    })?;
    let mut group = format!("{}\n", header.trim_start().replacen('$', "&", 1));
    for line in lines {
        group.push_str(line);
        group.push('\n');
        let code = line.split('!').next().unwrap_or("").trim();
        if code == "/" || code == "&end" || code == "$end" {
            return Some(group);
        }
    }
    group.push_str("/\n");
    Some(group)
}

/// 强迫变量的去向（`STREAM_*`，1 起编号即重启指纹里的值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Precip = 1,
    Vapor = 2,
    TotalPrecip = 3,
    TotalVapor = 4,
}

/// `MODE_*`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Value = 1,
    Delta = 2,
    HeavyOverTotal = 3,
    NormalizedOverTotal = 4,
}

/// `tracer_forcing_parse_mode`。
fn parse_mode(token: &str) -> Option<Mode> {
    match token.trim().to_ascii_lowercase().as_str() {
        "delta" => Some(Mode::Delta),
        "heavy_over_total" | "ratio_to_total" | "over_total" => Some(Mode::HeavyOverTotal),
        "normalized_over_total" | "normalized_ratio" | "isogsm" | "standard_over_total" => {
            Some(Mode::NormalizedOverTotal)
        }
        "direct" | "value" | "raw" => Some(Mode::Value),
        _ => None,
    }
}

/// `tracer_forcing_token_present`。
fn token_present(token: &str) -> bool {
    let low = token.trim().to_ascii_lowercase();
    !low.is_empty() && low != "null" && low != "none"
}

/// 一个示踪物强迫变量（`trc_var_*(iv)`）。
#[derive(Debug, Clone)]
pub struct ForcingVar {
    pub stream: Stream,
    /// 0 起的示踪物号；总量变量为 `None`（上游记 0）。
    pub itrc: Option<usize>,
    pub mode: Mode,
    /// 0 起的总量变量号（上游 1 起，0 表示没有）。
    pub total: Option<usize>,
    pub fprefix: String,
    pub vname: String,
    pub tintalgo: String,
    pub timelog: String,
    pub dtime: i32,
    pub offset: i32,
}

/// `tracer_forcing_configure` 的结果。
#[derive(Debug, Clone, Default)]
pub struct ForcingConfig {
    pub vars: Vec<ForcingVar>,
    pub total_precip: Option<usize>,
    pub total_vapor: Option<usize>,
    /// `trc_runtime_forced`：本示踪物有 precip 或 vapor 强迫变量。
    pub runtime_forced: Vec<bool>,
    /// `tracer_forcing_vapor_configured`（`tracer_forcing_has_vapor` 恒为真的那部分）。
    pub vapor_configured: Vec<bool>,
}

impl ForcingConfig {
    pub fn enabled(&self) -> bool {
        !self.vars.is_empty()
    }
}

/// 主强迫里总降水（4 号）与总比湿（2 号）变量的配置，`ensure_total` 用。
#[derive(Debug, Clone)]
pub struct MainTotals {
    /// `(fprefix, vname, tintalgo, timelog, dtime, offset)`，按 `[总比湿, 总降水]`。
    pub vapor: (String, String, String, String, i32, i32),
    pub precip: (String, String, String, String, i32, i32),
}

impl MainTotals {
    pub fn from_config(config: &GriddedForcingConfig) -> Self {
        let pick = |i: usize| {
            let v = &config.variables[i];
            (
                v.prefix.clone(),
                v.name.clone(),
                config.tintalgo_text[i].trim().to_ascii_lowercase(),
                config.timelog_text[i].trim().to_ascii_lowercase(),
                v.dtime,
                v.offset,
            )
        };
        Self {
            vapor: pick(1),
            precip: pick(3),
        }
    }
}

/// `tracer_forcing_configure`。`totals` 为 `None` 时（单点）不建总量变量，只判定配置与
/// `trc_runtime_forced`；`*_over_total` 模式需要总量，调用方在 POINT 下会先被拒绝。
pub fn configure(
    set: &TracerSet,
    physics: &TracerPhysics,
    specs: &[Vec<ForcingSpec>],
    totals: Option<&MainTotals>,
) -> Result<ForcingConfig> {
    let n = set.len();
    let mut config = ForcingConfig {
        runtime_forced: vec![false; n],
        vapor_configured: vec![false; n],
        ..ForcingConfig::default()
    };
    for (itrc, tracer) in set.tracers.iter().enumerate() {
        if !tracer.uses_land_water_transport() {
            continue;
        }
        let mut has = [false; 2];
        for (slot, (role, stream)) in [("precip", Stream::Precip), ("vapor", Stream::Vapor)]
            .into_iter()
            .enumerate()
        {
            let Some(spec) = specs[itrc].iter().find(|s| s.role.trim_end() == role) else {
                continue;
            };
            let mode = parse_mode(&spec.input_mode).expect("validated at load");
            ensure!(
                spec.dtime > 0,
                "tracer_forcing_add_var: forcing dtime for tracer {} must be > 0, got {}.",
                itrc + 1,
                spec.dtime
            );
            if !token_present(&spec.fprefix) || !token_present(&spec.vname) {
                continue;
            }
            let mut total = None;
            let mut timelog = "instant".to_owned();
            if matches!(mode, Mode::HeavyOverTotal | Mode::NormalizedOverTotal) {
                let index = ensure_total(&mut config, stream, totals)?;
                let t = &config.vars[index];
                ensure!(
                    spec.dtime == t.dtime
                        && spec.offset == t.offset
                        && spec.tintalgo.trim().to_ascii_lowercase() == t.tintalgo.trim(),
                    "tracer_forcing_add_var: raw/total forcing temporal config mismatch for tracer {}.",
                    itrc + 1
                );
                timelog = t.timelog.clone();
                total = Some(index);
            }
            config.vars.push(ForcingVar {
                stream,
                itrc: Some(itrc),
                mode,
                total,
                fprefix: spec.fprefix.trim_end().to_owned(),
                vname: spec.vname.trim_end().to_owned(),
                tintalgo: spec.tintalgo.trim_end().to_owned(),
                timelog,
                dtime: spec.dtime,
                offset: spec.offset,
            });
            has[slot] = true;
        }
        let allow_unforced = std::env::var_os("COLM_RS_ALLOW_UNFORCED_FRACTIONATION").is_some();
        ensure!(
            !physics.fractionation_active(tracer) || has[1] || allow_unforced,
            "tracer_forcing_configure: fractionating tracer {} ({}) must define vapor forcing \
             with role='vapor' in &nl_colm_tracer_forcing.",
            itrc + 1,
            tracer.name
        );
        config.runtime_forced[itrc] = has[0] || has[1];
        config.vapor_configured[itrc] = has[1];
    }
    Ok(config)
}

/// `tracer_forcing_ensure_total`。
fn ensure_total(
    config: &mut ForcingConfig,
    stream: Stream,
    totals: Option<&MainTotals>,
) -> Result<usize> {
    let precip = stream == Stream::Precip;
    let existing = if precip {
        config.total_precip
    } else {
        config.total_vapor
    };
    if let Some(index) = existing {
        return Ok(index);
    }
    let totals = totals.context(
        "tracer runtime forcing does not support DEF_forcing%dataset=POINT \
         (*_over_total tracer forcing needs the gridded total fields)",
    )?;
    let (fprefix, vname, tintalgo, timelog, dtime, offset) = if precip {
        totals.precip.clone()
    } else {
        totals.vapor.clone()
    };
    ensure!(
        dtime > 0,
        "tracer_forcing_ensure_total: total {} dtime must be > 0, got {dtime}.",
        if precip { "precipitation" } else { "vapor" }
    );
    let index = config.vars.len();
    config.vars.push(ForcingVar {
        stream: if precip {
            Stream::TotalPrecip
        } else {
            Stream::TotalVapor
        },
        itrc: None,
        mode: Mode::Value,
        total: None,
        fprefix: fprefix.trim_end().to_owned(),
        vname: vname.trim_end().to_owned(),
        tintalgo,
        timelog,
        dtime,
        offset,
    });
    if precip {
        config.total_precip = Some(index);
    } else {
        config.total_vapor = Some(index);
    }
    Ok(index)
}

#[derive(Debug, Clone, Default)]
struct Bracket {
    lower_stamp: Option<Stamp>,
    upper_stamp: Option<Stamp>,
    lower: Vec<f64>,
    upper: Vec<f64>,
}

/// 网格示踪物强迫的运行态。比值按 `[patch * ntracers + itrc]` 存。
#[derive(Debug)]
pub struct GriddedTracerForcing {
    config: ForcingConfig,
    brackets: Vec<Bracket>,
    ntracers: usize,
    ref_ratio: Vec<f64>,
    isotope: Vec<bool>,
    /// `trc_forc_precip_value`、`trc_forc_vapor_value`（最近一次有效值）。
    pub precip: Vec<f64>,
    pub vapor: Vec<f64>,
    /// 重启指纹要的主强迫原文：`DEF_Forcing_Interp_Method`。
    interp_method: String,
    /// `tracer_forcing_identity`（构造时按主强迫配置算好）。
    identity: std::sync::Arc<Vec<i32>>,
}

impl GriddedTracerForcing {
    /// `tracer_forcing_reset`：预热回卷时清掉上下界时间戳。
    pub fn reset(&mut self) {
        for bracket in &mut self.brackets {
            bracket.lower_stamp = None;
            bracket.upper_stamp = None;
        }
    }

    /// `tracer_forcing_init` + `tracer_forcing_allocate_state`：比值从描述符默认值起步。
    pub fn new(
        config: ForcingConfig,
        set: &TracerSet,
        patches: usize,
        interp_method: String,
        main: &GriddedForcingConfig,
    ) -> Result<Self> {
        ensure!(
            interp_method.trim() == "arealweight",
            "tracer forcing unsupported interp method in the Rust runtime: {}",
            interp_method.trim()
        );
        let ntracers = set.len();
        let precip_default: Vec<f64> = set.tracers.iter().map(|t| t.precip_default_ratio()).collect();
        let vapor_default: Vec<f64> = set.tracers.iter().map(|t| t.vapor_default_ratio()).collect();
        let mut forcing = Self {
            brackets: vec![Bracket::default(); config.vars.len()],
            config,
            ntracers,
            ref_ratio: set.tracers.iter().map(|t| t.ref_ratio).collect(),
            isotope: set.tracers.iter().map(|t| t.is_isotope()).collect(),
            precip: (0..patches).flat_map(|_| precip_default.iter().copied()).collect(),
            vapor: (0..patches).flat_map(|_| vapor_default.iter().copied()).collect(),
            interp_method,
            identity: std::sync::Arc::default(),
        };
        forcing.identity = std::sync::Arc::new(forcing.compute_identity(main));
        Ok(forcing)
    }

    /// 写续跑用的整段缓存（所有 patch）。
    pub fn cache(&self) -> crate::tracer::ForcingCache<'_> {
        crate::tracer::ForcingCache {
            nvars: self.config.vars.len(),
            ntracers: self.ntracers,
            identity: std::sync::Arc::clone(&self.identity),
            precip: &self.precip,
            vapor: &self.vapor,
        }
    }

    pub fn config(&self) -> &ForcingConfig {
        &self.config
    }

    /// 第 `patch` 个 patch 的 `(precip, vapor)` 比值（逐示踪物）。
    pub fn ratios(&self, patch: usize) -> (&[f64], &[f64]) {
        let range = patch * self.ntracers..(patch + 1) * self.ntracers;
        (&self.precip[range.clone()], &self.vapor[range])
    }

    /// `read_tracer_forcing`：读上下界、插值、`grid2pset`、解码并更新最近一次有效值。
    pub fn step(
        &mut self,
        now: CalendarTime,
        forcing: &GriddedForcing,
        mapping: &AreaWeightedMapping,
    ) -> Result<()> {
        if !self.config.enabled() {
            return Ok(());
        }
        let now = Stamp::from_calendar(now);
        let mut patch_values: Vec<Vec<f64>> = Vec::with_capacity(self.config.vars.len());
        for iv in 0..self.config.vars.len() {
            self.read_brackets(iv, now, forcing)?;
            let var = &self.config.vars[iv];
            let bracket = &self.brackets[iv];
            let lower = bracket.lower_stamp.expect("read above");
            let upper = bracket.upper_stamp.expect("read above");
            ensure!(
                !(now.less_than(lower) || upper.less_than(now)),
                "tracer forcing data required is out of range"
            );
            let dt_lower = now.seconds_since(lower);
            let dt_upper = upper.seconds_since(now);
            let field: Vec<f64> = match var.tintalgo.trim() {
                "linear" => {
                    if dt_lower + dt_upper > 0 {
                        let total = f64::from(dt_lower + dt_upper);
                        let alp1 = f64::from(dt_upper) / total;
                        let alp2 = f64::from(dt_lower) / total;
                        // `block_data_linear_interp`：`.FMA (from1, alp1, from2*alp2)`。
                        bracket
                            .lower
                            .iter()
                            .zip(&bracket.upper)
                            .map(|(&a, &b)| a.mul_add(alp1, b * alp2))
                            .collect()
                    } else {
                        bracket.lower.clone()
                    }
                }
                "nearest" => {
                    if dt_lower <= dt_upper {
                        bracket.lower.clone()
                    } else {
                        bracket.upper.clone()
                    }
                }
                "uniform" => {
                    if var.timelog.trim() == "forward" {
                        bracket.lower.clone()
                    } else {
                        bracket.upper.clone()
                    }
                }
                other => bail!(
                    "tracer_forcing_interpolate_var: variable {} has invalid interpolation mode \"{other}\".",
                    iv + 1
                ),
            };
            let mapped = (0..mapping.parts.len())
                .map(|iset| {
                    mapping.grid_to_set(iset, |ilon, ilat| field[forcing.cell_index(ilon, ilat)])
                })
                .collect();
            patch_values.push(mapped);
        }
        self.update_values(&patch_values);
        Ok(())
    }

    /// `tracer_forcing_read_LBUB` 的一个变量。
    fn read_brackets(&mut self, iv: usize, now: Stamp, forcing: &GriddedForcing) -> Result<()> {
        let main = forcing.config();
        let var = self.config.vars[iv].clone();
        let variable = Variable {
            prefix: var.fprefix.clone(),
            name: var.vname.clone(),
            forward: var.timelog.trim() == "forward",
            interpolation: Interpolation::Linear,
            dtime: var.dtime,
            offset: var.offset,
        };
        let bracket = &self.brackets[iv];
        if let (Some(lower), Some(upper)) = (bracket.lower_stamp, bracket.upper_stamp) {
            if lower.less_equal(now) && now.less_than(upper) {
                return Ok(());
            }
        }
        if self.brackets[iv].lower_stamp.is_none() {
            let (year, month, record, lower) = main
                .lower_record_for(now, &variable)
                .context("got the wrong time record of tracer forcing")?;
            let path = file_name(main, &var, year, month)?;
            let values = forcing.read_cells(&path, var.vname.trim(), record)?;
            let bracket = &mut self.brackets[iv];
            bracket.lower_stamp = Some(lower);
            bracket.lower = values;
        }
        loop {
            let needs_upper = match self.brackets[iv].upper_stamp {
                None => true,
                Some(upper) => upper.less_equal(now),
            };
            if !needs_upper {
                return Ok(());
            }
            let bracket = &mut self.brackets[iv];
            let upper = match bracket.upper_stamp {
                None => bracket.lower_stamp.expect("set above").add_seconds(var.dtime),
                Some(upper) => {
                    bracket.lower = std::mem::take(&mut bracket.upper);
                    bracket.lower_stamp = Some(upper);
                    upper.add_seconds(var.dtime)
                }
            };
            bracket.upper_stamp = Some(upper);
            let (year, month, record) = main
                .upper_record_for(upper, &variable)
                .context("got the wrong time record of tracer forcing")?;
            let path = file_name(main, &var, year, month)?;
            self.brackets[iv].upper = forcing.read_cells(&path, var.vname.trim(), record)?;
        }
    }

    /// `tracer_forcing_update_values`：逐变量、逐 patch 解码；有效才覆盖。
    fn update_values(&mut self, patch_values: &[Vec<f64>]) {
        for (iv, var) in self.config.vars.iter().enumerate() {
            let Some(itrc) = var.itrc else {
                continue;
            };
            for (ip, &raw) in patch_values[iv].iter().enumerate() {
                let total = var.total.map(|t| patch_values[t][ip]);
                let Some(value) = self.decode(var, itrc, raw, total) else {
                    continue;
                };
                let slot = ip * self.ntracers + itrc;
                match var.stream {
                    Stream::Precip => self.precip[slot] = value,
                    Stream::Vapor => self.vapor[slot] = value,
                    Stream::TotalPrecip | Stream::TotalVapor => {}
                }
            }
        }
    }

    /// `tracer_forcing_decode_value`：返回有效值，无效（含近干）为 `None`。
    fn decode(&self, var: &ForcingVar, itrc: usize, raw: f64, total: Option<f64>) -> Option<f64> {
        let valid = |x: f64| x.abs() < MAX_ABS;
        let value = match var.mode {
            Mode::HeavyOverTotal | Mode::NormalizedOverTotal => {
                let total = total?;
                if !valid(total) {
                    return None;
                }
                let min_total = if var.stream == Stream::Precip {
                    MIN_PRECIP
                } else {
                    MIN_Q
                };
                if total <= min_total || !valid(raw) || raw < 0.0 {
                    return None;
                }
                let value = raw / total;
                if var.mode == Mode::NormalizedOverTotal {
                    value * self.ref_ratio[itrc]
                } else {
                    value
                }
            }
            Mode::Delta => {
                if !valid(raw) {
                    return None;
                }
                if self.isotope[itrc] {
                    delta_to_ratio(raw, self.ref_ratio[itrc])
                } else {
                    raw
                }
            }
            Mode::Value => {
                if !valid(raw) {
                    return None;
                }
                raw
            }
        };
        if self.isotope[itrc] {
            if value <= TRC_TINY {
                return None;
            }
            if ratio_to_delta(value, self.ref_ratio[itrc]).abs() > DELTA_SANITY_MAX {
                return None;
            }
        } else if value < 0.0 {
            return None;
        }
        Some(value)
    }

    /// `tracer_forcing_identity`：`(TRC_FORC_ID_WIDTH, n+1)` 的整数指纹，按 Fortran 列序展平。
    pub fn identity(&self) -> &[i32] {
        &self.identity
    }

    fn compute_identity(&self, main: &GriddedForcingConfig) -> Vec<i32> {
        let n = self.config.vars.len();
        let mut id = vec![0i32; ID_WIDTH * (n + 1)];
        let put = |id: &mut [i32], column: usize, slot: usize, value: &str| {
            let start = column * ID_WIDTH + 8 + (slot - 1) * 256;
            for (k, byte) in value.trim_end().bytes().take(256).enumerate() {
                id[start + k] = i32::from(byte);
            }
        };
        let index = |i: Option<usize>| i.map_or(0, |i| i as i32 + 1);
        let head = [
            n as i32,
            self.ntracers as i32,
            main.start_year,
            main.start_month,
            i32::from(main.leapyear),
            index(self.config.total_precip),
            index(self.config.total_vapor),
            0,
        ];
        id[..8].copy_from_slice(&head);
        put(&mut id, 0, 1, &main.dataset);
        put(&mut id, 0, 2, &main.groupby_text);
        put(&mut id, 0, 3, &self.interp_method);
        put(&mut id, 0, 4, &main.variables[1].prefix);
        put(&mut id, 0, 5, &main.variables[3].prefix);
        put(&mut id, 0, 6, &main.directory.to_string_lossy());
        for (iv, var) in self.config.vars.iter().enumerate() {
            let column = iv + 1;
            let head = [
                var.stream as i32,
                index(var.itrc),
                var.mode as i32,
                index(var.total),
                var.dtime,
                var.offset,
                0,
                0,
            ];
            id[column * ID_WIDTH..column * ID_WIDTH + 8].copy_from_slice(&head);
            put(&mut id, column, 1, &var.fprefix);
            put(&mut id, column, 2, &var.vname);
            put(&mut id, column, 3, &var.tintalgo);
            put(&mut id, column, 4, &var.timelog);
        }
        id
    }

    /// `tracer_forcing_read_restart` 读回的最近一次有效值（`patches` 个 patch 起自 `first`）。
    pub fn restore(&mut self, first: usize, precip: &[f64], vapor: &[f64]) -> Result<()> {
        let start = first * self.ntracers;
        ensure!(
            precip.len() == vapor.len() && start + precip.len() <= self.precip.len(),
            "tracer forcing cache restart has an unexpected size"
        );
        for (itrc, tracer_isotope) in self.isotope.iter().enumerate() {
            let column = |values: &[f64]| {
                values
                    .iter()
                    .skip(itrc)
                    .step_by(self.ntracers)
                    .copied()
                    .collect::<Vec<_>>()
            };
            for values in [column(precip), column(vapor)] {
                let bad = values.iter().any(|&v| {
                    !v.is_finite()
                        || v.abs() >= MAX_ABS
                        || (*tracer_isotope && v <= TRC_TINY)
                        || (!*tracer_isotope && v < 0.0)
                });
                ensure!(!bad, "non-finite or invalid tracer forcing cache restart");
            }
        }
        self.precip[start..start + precip.len()].copy_from_slice(precip);
        self.vapor[start..start + vapor.len()].copy_from_slice(vapor);
        Ok(())
    }
}

/// `tracer_forcing_ratio_to_delta`。
fn ratio_to_delta(ratio: f64, ref_ratio: f64) -> f64 {
    if ratio > TRC_TINY && ref_ratio > TRC_TINY {
        (ratio / ref_ratio - 1.0) * 1000.0
    } else {
        0.0
    }
}

/// `trim(dir_forcing)//tracer_forcing_filename(year, month, day, iv)`：前缀与主强迫同名变量
/// 相同就用主强迫的 `metfilename`，否则按 `groupby` 拼。
fn file_name(main: &GriddedForcingConfig, var: &ForcingVar, year: i32, month: i32) -> Result<PathBuf> {
    let main_var = match var.stream {
        Stream::Precip | Stream::TotalPrecip => 3,
        Stream::Vapor | Stream::TotalVapor => 1,
    };
    if var.fprefix.trim() == main.variables[main_var].prefix.trim() {
        return Ok(main.file_name(year, month, main_var));
    }
    let directory = main.directory.to_string_lossy();
    let prefix = var.fprefix.trim();
    Ok(PathBuf::from(match main.groupby {
        GroupBy::Year => format!("{directory}/{prefix}_{year:04}.nc"),
        GroupBy::Month => format!("{directory}/{prefix}_{year:04}_{month:02}.nc"),
    }))
}

#[cfg(test)]
#[path = "tracer_forcing_tests.rs"]
mod tracer_forcing_tests;
