//! 材料运行时强迫（`main/MATERIAL/GENERIC_WATER/MOD_MT_ForcingInput.F90` +
//! `MOD_MT_Forcing.F90` 的网格路径）。
//!
//! 每个走陆面水输运的材料可在自己的参数文件里用 `&nl_colm_mt_forcing` 声明 `precip`、
//! `vapor` 两类输入；同位素共用一份文件，靠 `forcing_species` 区分。逐变量维护上下界记录
//! （与主强迫同一套 `setstampLB/UB`），在强迫格上做时间插值、`grid2pset` 到 patch，再按
//! `input_mode` 解码成比值。解码失败（近干、缺测、越界）就保留上一次的有效值；起点是描述符的
//! 默认比值。`*_over_total` 模式另读一份主强迫的总降水/总比湿 —— 直接读文件，**不经**
//! `metpreprocess`。
//!
//! 「最近一次有效值」是预报量：写进陆面重启（`mt_forcing_*_last`），续跑原样读回。

use colm_numeric::Contract;
use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};
use colm_core::tracer::descriptor::{
    delta_to_ratio, param_file_for_index, TracerParamFiles, TRC_TINY,
};
use colm_core::tracer::{TracerPhysics, TracerSet};
use colm_core::CalendarTime;
use colm_namelist::{Segment, Value};

use super::forcing::{
    GriddedForcing, GriddedForcingConfig, GroupBy, Interpolation, Stamp, Variable,
};
use super::mapping::AreaWeightedMapping;

/// `TRACER_FORCING_MAX`。
pub const TRACER_FORCING_MAX: usize = 8;
/// `trc_forc_min_prcp` 已换成 namelist 参数 `DEF_MT_FORCING_MIN_PRECIP`（默认 5e-6）；
/// `trc_forc_min_q`、`trc_forc_max_abs` 仍是常量。
const MIN_Q: f64 = 1.0e-12;
const MAX_ABS: f64 = 1.0e10;
/// `DEF_MT_FORCING_MIN_PRECIP`、`DEF_MT_FORCING_MEMORY_PRECIP` 的声明默认值
/// （`MOD_MT_Namelist.F90:78/88`）。
pub const DEFAULT_MIN_PRECIP: f64 = 5.0e-6;
pub const DEFAULT_MEMORY_PRECIP: f64 = 2.0e-5;
/// `trc_delta_sanity_max`。
const DELTA_SANITY_MAX: f64 = 2.0e3;
/// `MT_FORC_CACHE_SCHEMA`、`MT_FORC_ID_WIDTH`。schema 2 多出 `mt_forcing_precip_reliable`；
/// schema 1 的重启仍可读，其中的"记住值"当时就是最近一个有效记录，按可靠处理。
pub const CACHE_SCHEMA: i32 = 2;
pub const ID_WIDTH: usize = 8 + 6 * 256;

/// `DECODE_INVALID` / `DECODE_NEAR_DRY` / `DECODE_VALID`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeStatus {
    Invalid,
    NearDry,
    Valid,
}

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

/// `tracer_forcing_input_load`：逐材料读参数文件里的 `&nl_colm_mt_forcing`。
///
/// 上游把强迫组从 `&nl_colm_tracer_forcing` 改成 `&nl_colm_mt_forcing`，并用
/// `forcing_species` 选中材料：同位素文件被多个 species 共用，所以每个组都必须写
/// `forcing_species`；溶质自己的文件里可以省略（省略即属于该文件对应的溶质）。
/// 没有参数文件、文件不存在、或没有匹配的组时，输入个数为 0。
pub fn load_specs(
    set: &TracerSet,
    param_files: &TracerParamFiles,
) -> Result<Vec<Vec<ForcingSpec>>> {
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
        let mut matched: Option<(Option<String>, i64, Vec<ForcingSpec>)> = None;
        for group in forcing_groups(&source) {
            let (species, num, specs) = parse_forcing_group(&group, &path)?;
            let applies = match &species {
                Some(name) => name.eq_ignore_ascii_case(&tracer.name),
                None => {
                    // 同位素共用一份文件：省略 forcing_species 就无法判断属于谁（上游停机）。
                    ensure!(
                        !tracer.is_isotope(),
                        "forcing_species must be set in each &nl_colm_mt_forcing of {path}"
                    );
                    true
                }
            };
            if !applies {
                continue;
            }
            ensure!(
                matched.is_none(),
                "More than one &nl_colm_mt_forcing group for {} in {path}",
                tracer.name
            );
            matched = Some((species, num, specs));
        }
        let Some((_, num, mut specs)) = matched else {
            out.push(Vec::new());
            continue;
        };
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
            if let Some(kdup) =
                (0..k).find(|&j| specs[j].role.trim_end() == specs[k].role.trim_end())
            {
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

/// 解析一组 `&nl_colm_mt_forcing`：返回 `forcing_species`（可省略）、`forcing_num` 与逐项设置。
fn parse_forcing_group(group: &str, path: &str) -> Result<(Option<String>, i64, Vec<ForcingSpec>)> {
    let document = colm_namelist::parse(group)
        .with_context(|| format!("invalid &nl_colm_mt_forcing in {path}"))?;
    let mut species: Option<String> = None;
    let mut num: i64 = 0;
    let mut specs = vec![ForcingSpec::default(); TRACER_FORCING_MAX];
    for item in &document.items {
        let colm_namelist::document::Item::Entry(entry) = item else {
            continue;
        };
        let (name, index) = match entry.path.segments.as_slice() {
            [Segment::Field(name)] => (name.to_ascii_lowercase(), None),
            [Segment::Field(name), Segment::Index(k)] => (name.to_ascii_lowercase(), Some(*k)),
            _ => bail!("invalid &nl_colm_mt_forcing entry {} in {path}", entry.path),
        };
        if name == "forcing_species" {
            let Value::Str(value) = &entry.value else {
                bail!("forcing_species in {path} must be a string");
            };
            let value = value.trim();
            species = (!value.is_empty()).then(|| value.to_owned());
            continue;
        }
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
                other => bail!("invalid &nl_colm_mt_forcing in {path}: unknown {other}"),
            }
        }
    }
    Ok((species, num, specs))
}

/// `tracer_forcing_group_present` 判定后，截出每一组 `&nl_colm_mt_forcing` 的原文。
///
/// 同位素文件里每个 species 一组，所以要全部返回（旧实现只取第一组）。
fn forcing_groups(source: &str) -> Vec<String> {
    let mut groups = Vec::new();
    let mut current: Option<String> = None;
    for line in source.lines() {
        let low = line.trim_start().to_ascii_lowercase();
        if current.is_none() {
            if !low.starts_with('!')
                && (low.starts_with("&nl_colm_mt_forcing")
                    || low.starts_with("$nl_colm_mt_forcing"))
            {
                current = Some(format!("{}\n", line.trim_start().replacen('$', "&", 1)));
            }
            continue;
        }
        let group = current.as_mut().expect("inside a group");
        group.push_str(line);
        group.push('\n');
        let code = line.split('!').next().unwrap_or("").trim();
        if code == "/" || code == "&end" || code == "$end" {
            groups.push(current.take().expect("inside a group"));
        }
    }
    if let Some(mut group) = current {
        group.push_str("/\n");
        groups.push(group);
    }
    groups
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
    /// `tracer_forcing_precip_configured`。
    pub precip_configured: Vec<bool>,
    /// `mt_forc_phase_fill`：既是同位素、又同时有降水与蒸汽强迫的物种。只有它们
    /// 在近干记录时按相态分别回填（雨=与蒸汽平衡的液相，雪=记住的可靠比值）。
    pub phase_fill: Vec<bool>,
    /// `DEF_MT_FORCING_MIN_PRECIP`：低于它按 near-dry 处理、不用重/总比值。
    pub min_precip: f64,
    /// `DEF_MT_FORCING_MEMORY_PRECIP`：高于它才被记住为雪的回落比值。
    pub memory_precip: f64,
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
    min_precip: f64,
    memory_precip: f64,
) -> Result<ForcingConfig> {
    let n = set.len();
    let mut config = ForcingConfig {
        runtime_forced: vec![false; n],
        vapor_configured: vec![false; n],
        precip_configured: vec![false; n],
        phase_fill: vec![false; n],
        min_precip,
        memory_precip,
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
        config.precip_configured[itrc] = has[0];
        // `mt_forcing_allocate_state`：只有"同位素 + 降水强迫 + 蒸汽强迫"才做相态回填。
        config.phase_fill[itrc] = tracer.is_isotope() && has[0] && has[1];
    }
    // `MOD_MT_Forcing.F90:1024`：做相态回填时回落阈值必须不低于近干阈值。
    ensure!(
        !config.phase_fill.iter().any(|&fill| fill) || memory_precip >= min_precip,
        "DEF_MT_FORCING_MEMORY_PRECIP must be >= DEF_MT_FORCING_MIN_PRECIP for phase fill."
    );
    Ok(config)
}

/// `mt_forc_phase_fill` 的独立判定：同位素且同时配置了 precip 与 vapor 强迫。
///
/// `configure` 在遍历变量时用同一条件；LULCC 迁移没有主强迫 totals、不能跑完整
/// `configure`，所以单独暴露这条判定，保证两处一致。
pub fn phase_fill_flags(set: &TracerSet, specs: &[Vec<ForcingSpec>]) -> Vec<bool> {
    set.tracers
        .iter()
        .enumerate()
        .map(|(itrc, tracer)| {
            if !tracer.is_isotope() || !tracer.uses_land_water_transport() {
                return false;
            }
            let has = |role: &str| {
                specs.get(itrc).is_some_and(|list| {
                    list.iter().any(|spec| {
                        spec.role.trim_end() == role
                            && token_present(&spec.fprefix)
                            && token_present(&spec.vname)
                    })
                })
            };
            has("precip") && has("vapor")
        })
        .collect()
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
    /// `mt_forc_precip_value`、`mt_forc_vapor_value`（记住值：最近一个可靠记录。
    /// 同位素 + 双相强迫时，冷启动或新 patch 先放"冰-汽沉积平衡"估计）。
    pub precip: Vec<f64>,
    pub vapor: Vec<f64>,
    /// `mt_forc_precip_step`：本步有效的降水比值（`has_precip` 为真处）。
    precip_step: Vec<f64>,
    /// `mt_forc_precip_reliable`：1 表示记住值来自 total > `MEMORY_PRECIP` 的记录。
    pub precip_reliable: Vec<f64>,
    /// `mt_forc_has_precip` / `mt_forc_has_vapor`。
    has_precip: Vec<bool>,
    has_vapor: Vec<bool>,
    /// `mt_forc_precip_status` / `mt_forc_vapor_status`。
    precip_status: Vec<DecodeStatus>,
    vapor_status: Vec<DecodeStatus>,
    /// 本步按相态算好的雨/雪比值（`tracer_forcing_rain_value` / `_snow_value`）。
    /// 非相态回填时两者都等于记住值，与旧实现逐位相同。
    rain: Vec<f64>,
    snow: Vec<f64>,
    /// 相态回填要的 `tracer_alpha_*_vap` 与 `tracer_alpha_ice_vap_deposition`。
    physics: TracerPhysics,
    descriptors: Vec<colm_core::tracer::descriptor::TracerDescriptor>,
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
        physics: &TracerPhysics,
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
        let precip_default: Vec<f64> = set
            .tracers
            .iter()
            .map(|t| t.precip_default_ratio())
            .collect();
        let vapor_default: Vec<f64> = set
            .tracers
            .iter()
            .map(|t| t.vapor_default_ratio())
            .collect();
        let state = ntracers * patches;
        let mut forcing = Self {
            brackets: vec![Bracket::default(); config.vars.len()],
            config,
            ntracers,
            ref_ratio: set.tracers.iter().map(|t| t.ref_ratio).collect(),
            isotope: set.tracers.iter().map(|t| t.is_isotope()).collect(),
            precip: (0..patches)
                .flat_map(|_| precip_default.iter().copied())
                .collect(),
            vapor: (0..patches)
                .flat_map(|_| vapor_default.iter().copied())
                .collect(),
            precip_step: vec![0.0; state],
            precip_reliable: vec![0.0; state],
            has_precip: vec![false; state],
            has_vapor: vec![false; state],
            precip_status: vec![DecodeStatus::Invalid; state],
            vapor_status: vec![DecodeStatus::Invalid; state],
            rain: (0..patches)
                .flat_map(|_| precip_default.iter().copied())
                .collect(),
            snow: (0..patches)
                .flat_map(|_| precip_default.iter().copied())
                .collect(),
            physics: *physics,
            descriptors: set.tracers.clone(),
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
            reliable: &self.precip_reliable,
        }
    }

    pub fn config(&self) -> &ForcingConfig {
        &self.config
    }

    /// 第 `patch` 个 patch 的 `(rain, snow, vapor)` 比值（逐示踪物）。
    ///
    /// 上游 `tracer_forcing_rain_value` / `_snow_value`：没有相态回填的物种两者都取记住值，
    /// 所以这个接口在旧配置下与原来的单一 `precip` 逐位相同。
    pub fn ratios(&self, patch: usize) -> (&[f64], &[f64], &[f64]) {
        let range = patch * self.ntracers..(patch + 1) * self.ntracers;
        (
            &self.rain[range.clone()],
            &self.snow[range.clone()],
            &self.vapor[range],
        )
    }

    /// `read_tracer_forcing`：读上下界、插值、`grid2pset`、解码并更新最近一次有效值。
    /// `air_temperature` 是本步逐 patch 的 `forc_t`，相态回填的平衡分馏系数用它。
    pub fn step(
        &mut self,
        now: CalendarTime,
        forcing: &GriddedForcing,
        mapping: &AreaWeightedMapping,
        air_temperature: &[f64],
    ) -> Result<()> {
        if !self.config.enabled() {
            return Ok(());
        }
        // `tracer_forcing_prepare_step`：每步先清本步的有效标志与解码状态；
        // 记住值（`precip`/`vapor`）跨步保留。
        self.has_precip.fill(false);
        self.has_vapor.fill(false);
        self.precip_status.fill(DecodeStatus::Invalid);
        self.vapor_status.fill(DecodeStatus::Invalid);
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
                            .map(|(&a, &b)| a.contract(alp1, b * alp2))
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
        self.update_values(&patch_values, air_temperature);
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
                None => bracket
                    .lower_stamp
                    .expect("set above")
                    .add_seconds(var.dtime),
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
    ///
    /// 与旧实现的差别全在这里：有效值分"本步值"（`precip_step`）与"记住值"（`precip`），
    /// 记住值只在可靠记录（total > `MEMORY_PRECIP`）时更新；最后按相态算出雨/雪两列。
    fn update_values(&mut self, patch_values: &[Vec<f64>], air_temperature: &[f64]) {
        let vars = self.config.vars.clone();
        for (iv, var) in vars.iter().enumerate() {
            let Some(itrc) = var.itrc else {
                continue;
            };
            for (ip, &raw) in patch_values[iv].iter().enumerate() {
                let total = var.total.map(|t| patch_values[t][ip]);
                let (value, status) = self.decode(var, itrc, raw, total);
                let slot = ip * self.ntracers + itrc;
                match var.stream {
                    Stream::Precip => self.precip_status[slot] = status,
                    Stream::Vapor => self.vapor_status[slot] = status,
                    Stream::TotalPrecip | Stream::TotalVapor => {}
                }
                let Some(value) = value else {
                    continue;
                };
                match var.stream {
                    Stream::Precip => {
                        self.precip_step[slot] = value;
                        self.has_precip[slot] = true;
                        if self.reliable_record(var, total) {
                            self.precip[slot] = value;
                            self.precip_reliable[slot] = 1.0;
                        }
                    }
                    Stream::Vapor => {
                        self.vapor[slot] = value;
                        self.has_vapor[slot] = true;
                    }
                    Stream::TotalPrecip | Stream::TotalVapor => {}
                }
            }
        }
        // 冷启动或 LULCC 后的新 patch 还没见过可靠记录：记住值取"从强迫蒸汽凝华出来的冰"
        // 的平衡比值（Jouzel-Merlivat），每步更新，而不是固定的默认比值。
        if self.ntracers == 0 {
            return;
        }
        let patches = self.precip.len() / self.ntracers;
        for itrc in 0..self.ntracers {
            if !self.config.phase_fill.get(itrc).copied().unwrap_or(false) {
                continue;
            }
            let tracer = &self.descriptors[itrc];
            for ip in 0..patches {
                let slot = ip * self.ntracers + itrc;
                if self.precip_reliable[slot] > 0.5 {
                    continue;
                }
                let temp = air_temperature.get(ip).copied().unwrap_or(0.0);
                let value = self.physics.alpha_ice_vap_deposition(tracer, temp) * self.vapor[slot];
                if value.abs() < MAX_ABS && value > TRC_TINY {
                    self.precip[slot] = value;
                }
            }
        }
        self.update_phase_ratios(air_temperature);
    }

    /// `tracer_forcing_rain_value` / `_snow_value` 的逐步预计算。
    ///
    /// 有相态回填时：本步记录有效则雨雪都取本步值；只有"近干"时雨取与蒸汽平衡的液相、
    /// 雪取记住值；解码无效时两者都保持记住值。没有相态回填的物种两者都等于记住值，
    /// 与旧实现逐位相同。
    fn update_phase_ratios(&mut self, air_temperature: &[f64]) {
        if self.ntracers == 0 {
            return;
        }
        let patches = self.precip.len() / self.ntracers;
        for ip in 0..patches {
            let temp = air_temperature.get(ip).copied().unwrap_or(0.0);
            for itrc in 0..self.ntracers {
                let slot = ip * self.ntracers + itrc;
                let remembered = self.precip[slot];
                let (mut rain, mut snow) = (remembered, remembered);
                if self.config.phase_fill.get(itrc).copied().unwrap_or(false) {
                    if self.has_precip[slot] {
                        rain = self.precip_step[slot];
                        snow = self.precip_step[slot];
                    } else if self.precip_status[slot] == DecodeStatus::NearDry {
                        let value = self.physics.alpha_liq_vap(&self.descriptors[itrc], temp)
                            * self.vapor[slot];
                        if value.abs() < MAX_ABS && value > TRC_TINY {
                            rain = value;
                        }
                    }
                }
                self.rain[slot] = rain;
                self.snow[slot] = snow;
            }
        }
    }

    /// `tracer_forcing_reliable_record`：相态回填的物种，重/总比值只有在
    /// total > `MEMORY_PRECIP` 时才值得记住（刚过近干阈值的记录 d-excess 仍然很吵）。
    fn reliable_record(&self, var: &ForcingVar, total: Option<f64>) -> bool {
        let Some(itrc) = var.itrc else {
            return true;
        };
        if !self.config.phase_fill.get(itrc).copied().unwrap_or(false) {
            return true;
        }
        if !matches!(var.mode, Mode::HeavyOverTotal | Mode::NormalizedOverTotal) {
            return true;
        }
        total.is_some_and(|total| total > self.config.memory_precip)
    }

    /// `tracer_forcing_decode_value`：返回有效值与解码状态（无效 / 近干 / 有效）。
    fn decode(
        &self,
        var: &ForcingVar,
        itrc: usize,
        raw: f64,
        total: Option<f64>,
    ) -> (Option<f64>, DecodeStatus) {
        let valid = |x: f64| x.abs() < MAX_ABS;
        let value = match var.mode {
            Mode::HeavyOverTotal | Mode::NormalizedOverTotal => {
                let Some(total) = total else {
                    return (None, DecodeStatus::Invalid);
                };
                if !valid(total) {
                    return (None, DecodeStatus::Invalid);
                }
                let min_total = if var.stream == Stream::Precip {
                    self.config.min_precip
                } else {
                    MIN_Q
                };
                if total <= min_total {
                    return (None, DecodeStatus::NearDry);
                }
                if !valid(raw) || raw < 0.0 {
                    return (None, DecodeStatus::Invalid);
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
                    return (None, DecodeStatus::Invalid);
                }
                if self.isotope[itrc] {
                    delta_to_ratio(raw, self.ref_ratio[itrc])
                } else {
                    raw
                }
            }
            Mode::Value => {
                if !valid(raw) {
                    return (None, DecodeStatus::Invalid);
                }
                raw
            }
        };
        if self.isotope[itrc] {
            if value <= TRC_TINY {
                return (None, DecodeStatus::Invalid);
            }
            if ratio_to_delta(value, self.ref_ratio[itrc]).abs() > DELTA_SANITY_MAX {
                return (None, DecodeStatus::Invalid);
            }
        } else if value < 0.0 {
            return (None, DecodeStatus::Invalid);
        }
        (Some(value), DecodeStatus::Valid)
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
    ///
    /// `reliable` 为 `None` 表示 schema 1 的重启：当时每个有效记录都会更新记住值，
    /// 所以整段按可靠处理（上游 `mt_forcing_read_restart` 的同一规则）。
    pub fn restore(
        &mut self,
        first: usize,
        precip: &[f64],
        vapor: &[f64],
        reliable: Option<&[f64]>,
    ) -> Result<()> {
        let start = first * self.ntracers;
        ensure!(
            precip.len() == vapor.len() && start + precip.len() <= self.precip.len(),
            "tracer forcing cache restart has an unexpected size"
        );
        if let Some(reliable) = reliable {
            ensure!(
                reliable.len() == precip.len() && reliable.iter().all(|&v| v == 0.0 || v == 1.0),
                "non-finite or invalid tracer forcing cache restart"
            );
        }
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
        for slot in start..start + precip.len() {
            self.precip_reliable[slot] = reliable.map_or(1.0, |values| values[slot - start]);
        }
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
fn file_name(
    main: &GriddedForcingConfig,
    var: &ForcingVar,
    year: i32,
    month: i32,
) -> Result<PathBuf> {
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
