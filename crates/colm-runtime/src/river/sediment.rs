//! 河道泥沙（`MOD_Tracer_Particle_Sediment.F90`）：网格河湖汇流上的 provider 示踪物 `SEDIMENT`。
//!
//! 每个陆面步把降水按面积汇进单元流域（`sediment_forcing_put`，降水强度的幂次在步内累加，
//! 避免 Jensen 偏差）；汇流的每个子步累加流速平方、水深、蓄量端点、出口通量与淹没面积
//! （`sediment_diag_accumulate`）；一次汇流结束后按 `max_timestep_s` 切成若干形态步，依次做
//! 悬沙/推移质输运、坡面产沙、悬沙—床面交换、产沙入河、history 累加与床层重新分层
//! （`grid_sediment_calc`）。
//!
//! 开堤防或分汊时（`DEF_USE_LEVEE`/`DEF_USE_BIFURCATION`），普通面、漫堤（`transfer_levee_sediment`）
//! 与分汊路径（`prepare_bif_sediment`）都从同一份期初存量里按联合缩放取量：堤内另有悬沙与不动的
//! 床沙池，分汊的到达量作为信用在普通输运之后才加；续跑写 schema 5。
//! 收缩形状取自 latlon 内核的 GIMPLE（`-fdump-tree-optimized-lineno`），逐句注明。
//!
//! 数组布局：逐单元流域连续，`[i*nsed + s]`；沉积层 `[(i*totlyrnum + l)*nsed + s]`。

// 逐单元流域、逐粒径的循环按下标写多组数组（与上游的 DO 循环一一对应）；`max().min()` 保留上游
// `min(max(x,0),1)` 对 NaN 的语义（`clamp` 会把 NaN 原样传下去）。
#![allow(clippy::needless_range_loop, clippy::manual_clamp)]

use colm_numeric::Contract;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::LibmPow;
use colm_namelist::{Segment, Value};

use super::bifurcation::Bifurcation;
use super::network::RiverNetwork;

/// 悬沙体积浓度上限 [m³/m³]。
const MAX_SED_CONC: f64 = 0.01;
const SED_BEDLOAD_COEFF: f64 = 17.0;
/// 一个强迫步的降水强度不超过它（mm/day）就不产沙。
const SED_PRECIP_THRESHOLD_MM_DAY: f64 = 10.0;
const EXCH_SHEARVEL_MIN: f64 = 1.0e-4;
const EXCH_SHEARVEL_BLEND: f64 = 2.0 * EXCH_SHEARVEL_MIN;
const EXCH_ZD_MAX: f64 = 100.0;
const SED_BALANCE_ABS_TOL: f64 = 1.0e-10;
const SED_BALANCE_REL_TOL: f64 = 1.0e-10;
/// 不开堤防/分汊时续跑写的 schema。
const SCHEMA_PLAIN: f64 = 2.0;
/// 开堤防或分汊时的 schema（`SED_RESTART_SCHEMA_VERSION`）。
const SCHEMA_JOINT: f64 = 5.0;
/// `MOD_Const_Physical` 的 `grav`。
const GRAV: f64 = 9.80616;

/// 注册的泥沙 provider 示踪物（`register_tracer_provider('SEDIMENT', 'SED', 'sediment', …)`）。
pub fn is_sediment_tracer(tracer: &colm_core::tracer::TracerDescriptor) -> bool {
    tracer.category == "particle"
        && matches!(
            tracer.name.trim().to_ascii_uppercase().as_str(),
            "SEDIMENT" | "SED"
        )
}

/// 泥沙参数（`read_sediment_parameter_file` 套用参数文件之后的模块量）。
#[derive(Debug, Clone, PartialEq)]
pub struct SedimentParams {
    pub nsed: usize,
    pub totlyrnum: usize,
    pub nlfp: usize,
    pub lambda: f64,
    pub lyrdph: f64,
    pub psedd: f64,
    pub pwatd: f64,
    pub viskin: f64,
    pub vonkar: f64,
    pub pset: f64,
    pub pyld: f64,
    pub pyldc: f64,
    pub pyldpc: f64,
    pub dsylunit: f64,
    pub ignore_dph: f64,
    pub cfl_adv: f64,
    pub dt_max: f64,
    pub bed_depth: f64,
    /// `sDiam` [m]。
    pub diam: Vec<f64>,
    /// `setvel` [m/s]。
    pub setvel: Vec<f64>,
}

/// `&nl_colm_sediment_parameter` 里读到的原始记录（`sediment_parameter_type`，未给的保持 -1）。
#[derive(Debug, Clone, PartialEq)]
struct ParameterRecord {
    nsed: i64,
    grain_diameter: Vec<f64>,
    grain_density: f64,
    water_density: f64,
    porosity: f64,
    ndeposit_layers: i64,
    ignore_depth_m: f64,
    active_layer_depth: f64,
    viscosity: f64,
    von_karman: f64,
    settling_multiplier: f64,
    yield_coefficient: f64,
    slope_exponent: f64,
    precipitation_exponent: f64,
    unit_conversion: f64,
    cfl_adv: f64,
    max_timestep_s: f64,
    bed_depth: f64,
}

/// `MAX_SED_PARAM_CLASSES`。
const MAX_SED_PARAM_CLASSES: usize = 100;

impl Default for ParameterRecord {
    fn default() -> Self {
        Self {
            nsed: -1,
            grain_diameter: vec![-1.0; MAX_SED_PARAM_CLASSES],
            grain_density: -1.0,
            water_density: -1.0,
            porosity: -1.0,
            ndeposit_layers: -1,
            ignore_depth_m: -1.0,
            active_layer_depth: -1.0,
            viscosity: -1.0,
            von_karman: -1.0,
            settling_multiplier: -1.0,
            yield_coefficient: -1.0,
            slope_exponent: -1.0,
            precipitation_exponent: -1.0,
            unit_conversion: -1.0,
            cfl_adv: -1.0,
            max_timestep_s: -1.0,
            bed_depth: -1.0,
        }
    }
}

fn read_record(path: &str) -> Result<ParameterRecord> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("sediment parameter file does not exist: {path}"))?;
    let mut lines = source.lines();
    lines
        .by_ref()
        .find(|line| {
            line.trim_start()
                .to_ascii_lowercase()
                .starts_with("&nl_colm_sediment_parameter")
        })
        .with_context(|| format!("no &nl_colm_sediment_parameter in {path}"))?;
    let mut group = String::from("&nl_colm_sediment_parameter\n");
    for line in lines {
        let code = line.split('!').next().unwrap_or("").trim();
        if code == "/" || code.eq_ignore_ascii_case("&end") {
            break;
        }
        group.push_str(line);
        group.push('\n');
    }
    group.push_str("/\n");
    let document = colm_namelist::parse(&group)
        .with_context(|| format!("invalid &nl_colm_sediment_parameter in {path}"))?;
    let mut record = ParameterRecord::default();
    for item in &document.items {
        let colm_namelist::document::Item::Entry(entry) = item else {
            continue;
        };
        let (owner, field, index) = match entry.path.segments.as_slice() {
            [Segment::Field(owner), Segment::Member(field)] => (owner, field, None),
            [Segment::Field(owner), Segment::Member(field), Segment::Index(index)] => {
                (owner, field, Some(*index))
            }
            _ => bail!(
                "invalid &nl_colm_sediment_parameter entry {} in {path}",
                entry.path
            ),
        };
        ensure!(
            owner.eq_ignore_ascii_case("DEF_SEDIMENT"),
            "invalid &nl_colm_sediment_parameter: unknown {}",
            entry.path
        );
        let real = |value: &Value| -> Result<f64> {
            value
                .as_f64()
                .with_context(|| format!("{} in {path} is not a real", entry.path))
        };
        let integer = |value: &Value| -> Result<i64> {
            match value {
                Value::Int(i) => Ok(*i),
                _ => bail!("{} in {path} is not an integer", entry.path),
            }
        };
        let field = field.to_ascii_lowercase();
        if field == "grain_diameter" {
            // 数组成员：列表从下标（缺省 1）起逐个赋值。
            let start = index.unwrap_or(1);
            ensure!(
                start >= 1,
                "{} in {path}: index must start at 1",
                entry.path
            );
            let values = match &entry.value {
                Value::List(values) => values.iter().map(real).collect::<Result<Vec<_>>>()?,
                value => vec![real(value)?],
            };
            ensure!(
                start - 1 + values.len() <= MAX_SED_PARAM_CLASSES,
                "{} in {path} exceeds {MAX_SED_PARAM_CLASSES} classes",
                entry.path
            );
            record.grain_diameter[start - 1..start - 1 + values.len()].copy_from_slice(&values);
            continue;
        }
        ensure!(
            index.is_none(),
            "{} in {path}: {field} is not an array",
            entry.path
        );
        let value = &entry.value;
        match field.as_str() {
            "nsed" => record.nsed = integer(value)?,
            "ndeposit_layers" => record.ndeposit_layers = integer(value)?,
            "grain_density" => record.grain_density = real(value)?,
            "water_density" => record.water_density = real(value)?,
            "porosity" => record.porosity = real(value)?,
            "ignore_depth_m" => record.ignore_depth_m = real(value)?,
            "active_layer_depth" => record.active_layer_depth = real(value)?,
            "viscosity" => record.viscosity = real(value)?,
            "von_karman" => record.von_karman = real(value)?,
            "settling_multiplier" => record.settling_multiplier = real(value)?,
            "yield_coefficient" => record.yield_coefficient = real(value)?,
            "slope_exponent" => record.slope_exponent = real(value)?,
            "precipitation_exponent" => record.precipitation_exponent = real(value)?,
            "unit_conversion" => record.unit_conversion = real(value)?,
            "cfl_adv" => record.cfl_adv = real(value)?,
            "max_timestep_s" => record.max_timestep_s = real(value)?,
            "bed_depth" => record.bed_depth = real(value)?,
            other => bail!("invalid &nl_colm_sediment_parameter: unknown DEF_SEDIMENT%{other}"),
        }
    }
    Ok(record)
}

impl SedimentParams {
    /// `grid_sediment_init` 的参数部分：缺省值、参数文件覆盖、粒径、沉速与校验。
    /// `nsed`、`nlfp` 取自单元流域文件的 `sed_n`、`slope_layers` 维。
    pub fn read(path: &str, nsed: usize, nlfp: usize) -> Result<Self> {
        let record = read_record(path)?;
        let finite = [
            record.grain_density,
            record.water_density,
            record.porosity,
            record.ignore_depth_m,
            record.active_layer_depth,
            record.viscosity,
            record.von_karman,
            record.settling_multiplier,
            record.yield_coefficient,
            record.slope_exponent,
            record.precipitation_exponent,
            record.unit_conversion,
            record.cfl_adv,
            record.max_timestep_s,
            record.bed_depth,
        ]
        .iter()
        .chain(&record.grain_diameter)
        .all(|v| v.is_finite());
        ensure!(
            finite,
            "sediment parameter file has a non-finite value: {path}"
        );
        ensure!(
            !(record.nsed > 0 && record.nsed as usize != nsed),
            "DEF_SEDIMENT%nsed = {} does not match the network sed_n = {nsed}",
            record.nsed
        );
        let mut p = Self {
            nsed,
            totlyrnum: 5,
            nlfp,
            lambda: 0.4,
            lyrdph: 0.05,
            psedd: 2.65,
            pwatd: 1.0,
            viskin: 1.0e-6,
            vonkar: 0.4,
            pset: 1.0,
            pyld: 0.01,
            pyldc: 2.0,
            pyldpc: 2.0,
            dsylunit: 1.0e-6,
            ignore_dph: 0.05,
            cfl_adv: 0.5,
            dt_max: 3600.0,
            bed_depth: 10.0,
            diam: Vec::new(),
            setvel: Vec::new(),
        };
        if record.grain_density >= 0.0 {
            ensure!(
                record.grain_density >= 100.0,
                "DEF_SEDIMENT%grain_density must be in kg/m3: {}",
                record.grain_density
            );
            p.psedd = record.grain_density / 1000.0;
        }
        if record.water_density >= 0.0 {
            ensure!(
                record.water_density >= 100.0,
                "DEF_SEDIMENT%water_density must be in kg/m3: {}",
                record.water_density
            );
            p.pwatd = record.water_density / 1000.0;
        }
        if record.porosity >= 0.0 {
            p.lambda = record.porosity;
        }
        if record.ndeposit_layers > 0 {
            p.totlyrnum = record.ndeposit_layers as usize;
        }
        if record.ignore_depth_m >= 0.0 {
            p.ignore_dph = record.ignore_depth_m;
        }
        if record.active_layer_depth > 0.0 {
            p.lyrdph = record.active_layer_depth;
        }
        if record.viscosity > 0.0 {
            p.viskin = record.viscosity;
        }
        if record.von_karman > 0.0 {
            p.vonkar = record.von_karman;
        }
        if record.settling_multiplier > 0.0 {
            p.pset = record.settling_multiplier;
        }
        if record.yield_coefficient >= 0.0 {
            p.pyld = record.yield_coefficient;
        }
        if record.slope_exponent >= 0.0 {
            p.pyldc = record.slope_exponent;
        }
        if record.precipitation_exponent >= 0.0 {
            p.pyldpc = record.precipitation_exponent;
        }
        if record.unit_conversion >= 0.0 {
            p.dsylunit = record.unit_conversion;
        }
        if record.cfl_adv >= 0.0 {
            p.cfl_adv = record.cfl_adv;
        }
        if record.max_timestep_s > 0.0 {
            p.dt_max = record.max_timestep_s;
        }
        if record.bed_depth > 0.0 {
            p.bed_depth = record.bed_depth;
        }
        p.validate()?;
        // `parse_grain_diameters`：参数文件给了第一个粒径就全用参数文件的，否则用缺省串。
        p.diam = if record.grain_diameter[0] > 0.0 {
            ensure!(
                nsed <= MAX_SED_PARAM_CLASSES,
                "nsed exceeds {MAX_SED_PARAM_CLASSES} parameter classes"
            );
            let diam = record.grain_diameter[..nsed].to_vec();
            ensure!(
                diam.iter().all(|d| d.is_finite() && *d > 0.0),
                "DEF_SEDIMENT%grain_diameter must be positive for every class"
            );
            diam
        } else {
            let diam = [0.0002, 0.002, 0.02];
            ensure!(
                nsed == diam.len(),
                "Number of diameters does not match nsed: {} {nsed}",
                diam.len()
            );
            diam.to_vec()
        };
        // `calc_settling_velocities`：`pset*(sqrt(2/3*(ρs-ρw)/ρw*g*d + s²) - s)`，`s = 6ν/d`；
        // 根号里是 `FMA(d, (ρs-ρw)*(2/3)/ρw*g, s*s)`。
        p.setvel = p
            .diam
            .iter()
            .map(|&d| {
                let s = 6.0 * p.viskin / d;
                let factor = (p.psedd - p.pwatd) * (2.0 / 3.0) / p.pwatd * GRAV;
                (d.contract(factor, s * s).sqrt() - s) * p.pset
            })
            .collect();
        p.validate()?;
        Ok(p)
    }

    /// `validate_sediment_parameters`。
    fn validate(&self) -> Result<()> {
        ensure!(self.nsed > 0, "sediment nsed must be positive");
        ensure!(self.nlfp > 0, "sediment slope_layers must be positive");
        ensure!(
            [
                self.lambda,
                self.lyrdph,
                self.psedd,
                self.pwatd,
                self.viskin,
                self.vonkar,
                self.pset,
                self.pyld,
                self.pyldc,
                self.pyldpc,
                self.dsylunit,
                self.ignore_dph,
                self.cfl_adv,
                self.dt_max,
                self.bed_depth
            ]
            .iter()
            .all(|v| v.is_finite()),
            "sediment parameters must be finite"
        );
        ensure!(
            (0.0..1.0).contains(&self.lambda),
            "sediment porosity must be in [0,1)"
        );
        ensure!(
            self.lyrdph > 0.0,
            "sediment active layer depth must be positive"
        );
        ensure!(
            self.psedd > self.pwatd,
            "sediment density must exceed water density"
        );
        ensure!(
            self.pwatd > 0.0 && self.viskin > 0.0 && self.vonkar > 0.0 && self.pset > 0.0,
            "sediment water density, viscosity, von Karman and settling multiplier must be positive"
        );
        ensure!(
            self.totlyrnum > 0,
            "sediment deposit layers must be positive"
        );
        ensure!(
            self.cfl_adv > 0.0 && self.cfl_adv <= 1.0,
            "sediment cfl_adv must be in (0,1]"
        );
        ensure!(
            self.dt_max > 0.0,
            "sediment max_timestep_s must be positive"
        );
        ensure!(self.bed_depth > 0.0, "sediment bed_depth must be positive");
        ensure!(
            self.bed_depth >= self.lyrdph * self.totlyrnum as f64,
            "sediment bed_depth {} is below the minimum {}",
            self.bed_depth,
            self.lyrdph * self.totlyrnum as f64
        );
        ensure!(
            self.ignore_dph >= 0.0,
            "sediment ignore_depth_m must be non-negative"
        );
        ensure!(
            self.pyld >= 0.0 && self.pyldc >= 0.0 && self.pyldpc >= 0.0 && self.dsylunit >= 0.0,
            "sediment yield parameters must be non-negative"
        );
        if !self.diam.is_empty() {
            ensure!(
                self.diam.iter().all(|d| d.is_finite() && *d > 0.0),
                "sediment grain diameters must be positive"
            );
            let dmax = self.diam.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));
            ensure!(
                self.lyrdph >= dmax,
                "sediment active layer depth is thinner than the largest grain"
            );
        }
        if !self.setvel.is_empty() {
            ensure!(
                self.setvel.iter().all(|v| v.is_finite() && *v >= 0.0),
                "sediment settling velocities must be finite and non-negative"
            );
        }
        Ok(())
    }
}

/// 一次汇流里逐单元流域的水量累加（`sed_acc_*`）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaterAcc {
    pub time: f64,
    pub v2: f64,
    pub wdsrf: f64,
    pub rivsto: f64,
    pub rivsto_start: f64,
    pub rivsto_end: f64,
    pub rivout: f64,
    pub abs_rivout: f64,
    pub floodarea: f64,
    /// 堤内蓄量的首末端点（`sed_acc_protected_start/end`）与淹没面积累加。
    pub protected_start: f64,
    pub protected_end: f64,
    pub protected_area: f64,
    /// 首个子步之前已有一次堤防重新分区记下了首端点（`sed_acc_pre_repartition_start`）。
    pub pre_repartition_start: bool,
    /// 堤防重新分区的毛转移量（`sed_acc_to/from_protected`，m³）。
    pub to_protected: f64,
    pub from_protected: f64,
}

/// 一个子步末一个单元流域的水（`sediment_diag_accumulate` 的入参）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SubstepWater {
    /// 子步首、末的总蓄量（可见 + 堤内，水库取 `volresv`，都截到非负）。
    pub start: f64,
    pub end: f64,
    /// 堤内蓄量的首、末端点。
    pub protected_start: f64,
    pub protected_end: f64,
    /// 堤内淹没面积（只在开堤防时累加）。
    pub protected_area: Option<f64>,
}

impl WaterAcc {
    /// `sediment_diag_accumulate` 对一个单元流域的一个子步：`rivout` 是（洼地溢流之后的）`hflux_fc`。
    pub fn add(
        &mut self,
        dt: f64,
        veloc: f64,
        wdsrf: f64,
        water: SubstepWater,
        rivout: f64,
        floodarea: f64,
    ) -> Result<()> {
        let first = self.time == 0.0 && !self.pre_repartition_start;
        if first {
            self.rivsto_start = water.start;
        }
        self.rivsto_end = water.end;
        ensure!(
            water.protected_start.is_finite()
                && water.protected_end.is_finite()
                && water.protected_start.min(water.protected_end) >= 0.0,
            "sediment protected carrier invalid"
        );
        ensure!(
            water.protected_start <= water.start + SED_BALANCE_ABS_TOL
                && water.protected_end <= water.end + SED_BALANCE_ABS_TOL,
            "sediment protected carrier exceeds total water"
        );
        if first {
            self.protected_start = water.protected_start;
        }
        self.protected_end = water.protected_end;
        self.time += dt;
        self.v2 = dt.contract(veloc * veloc, self.v2);
        self.wdsrf = wdsrf.contract(dt, self.wdsrf);
        self.rivsto = dt.contract(water.end, self.rivsto);
        self.rivout = rivout.contract(dt, self.rivout);
        self.abs_rivout = rivout.abs().contract(dt, self.abs_rivout);
        self.floodarea = floodarea.contract(dt, self.floodarea);
        if let Some(area) = water.protected_area {
            ensure!(
                area.is_finite() && area >= 0.0,
                "sediment protected area invalid"
            );
            self.protected_area = dt.contract(area, self.protected_area);
        }
        Ok(())
    }

    /// `sediment_levee_repartition`：只夹住堤防重新分区的那一对水量（分汊通量之后）；
    /// 堤内增加记成可见→堤内，减少记成反向。首个子步之前的分区给出首端点。
    pub fn levee_repartition(&mut self, water: [f64; 4]) -> Result<()> {
        let [visible_before, protected_before, visible_after, protected_after] = water;
        ensure!(
            water.iter().all(|v| v.is_finite() && *v >= 0.0),
            "sediment levee repartition water storage invalid"
        );
        let transfer = protected_after - protected_before;
        let before = visible_before + protected_before;
        let after = visible_after + protected_after;
        ensure!(
            (after - before).abs() <= SED_BALANCE_ABS_TOL + SED_BALANCE_REL_TOL * before.max(after),
            "sediment levee repartition water volume not conserved"
        );
        if self.time == 0.0 && !self.pre_repartition_start {
            self.rivsto_start = before;
            self.protected_start = protected_before;
            self.pre_repartition_start = true;
        }
        self.to_protected += transfer.max(0.0);
        self.from_protected += (-transfer).max(0.0);
        Ok(())
    }
}

/// 分汊路径的毛水量（`sed_acc_bif_*`，`[p*levels + l]`）：正反两向分开，不在子步之间抵消。
#[derive(Debug, Clone, PartialEq)]
pub struct BifWaterAcc {
    pub levels: usize,
    pub forward: Vec<f64>,
    pub reverse: Vec<f64>,
    pub forward_time: Vec<f64>,
    pub reverse_time: Vec<f64>,
}

impl BifWaterAcc {
    pub fn new(paths: usize, levels: usize) -> Self {
        let zeros = vec![0.0; paths * levels];
        Self {
            levels,
            forward: zeros.clone(),
            reverse: zeros.clone(),
            forward_time: zeros.clone(),
            reverse_time: zeros,
        }
    }

    /// `sediment_bif_accumulate`：本子步每条活动路径逐层的（限流后的）`bif_hflux_lev`。
    pub fn add(&mut self, dt: f64, hflux_lev: &[f64], active: &[bool]) -> Result<()> {
        ensure!(
            dt.is_finite() && dt > 0.0,
            "sediment bifurcation water flux invalid"
        );
        for (p, &on) in active.iter().enumerate() {
            if !on {
                continue;
            }
            let row = p * self.levels..(p + 1) * self.levels;
            ensure!(
                hflux_lev[row.clone()].iter().all(|h| h.is_finite()),
                "sediment bifurcation water flux invalid"
            );
            for k in row {
                let h = hflux_lev[k];
                self.forward[k] = dt.contract(h.max(0.0), self.forward[k]);
                self.reverse[k] = dt.contract((-h).max(0.0), self.reverse[k]);
                if h > 0.0 {
                    self.forward_time[k] += dt;
                }
                if h < 0.0 {
                    self.reverse_time[k] += dt;
                }
            }
        }
        Ok(())
    }

    fn clear(&mut self) {
        for values in [
            &mut self.forward,
            &mut self.reverse,
            &mut self.forward_time,
            &mut self.reverse_time,
        ] {
            values.fill(0.0);
        }
    }
}

/// 泥沙状态、诊断与累加（`grid_sediment_init` 之后的模块量）。
#[derive(Debug, Clone, PartialEq)]
pub struct Sediment {
    pub p: SedimentParams,
    /// `sed_frc` `[i*nsed + s]`（已归一化）。
    pub frc: Vec<f64>,
    /// `sed_slope` `[i*nlfp + l]`（已截到非负）。
    pub slope: Vec<f64>,
    pub sedcon: Vec<f64>,
    pub sedsto: Vec<f64>,
    pub layer: Vec<f64>,
    pub seddep: Vec<f64>,
    pub sedout: Vec<f64>,
    pub bedout: Vec<f64>,
    pub sedinp: Vec<f64>,
    pub netflw: Vec<f64>,
    pub exch_es_raw: Vec<f64>,
    pub exch_d_raw: Vec<f64>,
    pub exch_es_eff: Vec<f64>,
    pub exch_d_eff: Vec<f64>,
    pub netflw_adv_step: Vec<f64>,
    pub exch_d_adv_step: Vec<f64>,
    pub shearvel: Vec<f64>,
    pub critshearvel: Vec<f64>,
    pub susvel: Vec<f64>,
    /// 堤内悬沙与（不动的）堤内床沙 `[i*nsed + s]`。
    pub sedsto_protected: Vec<f64>,
    pub sedbed_protected: Vec<f64>,
    pub acc: Vec<WaterAcc>,
    /// `DEF_USE_LEVEE`：每个单元流域有没有堤（`has_levee`），没开堤防时为 `None`。
    pub levee: Option<Vec<bool>>,
    /// `DEF_USE_BIFURCATION`：分汊路径的毛水量。
    pub bif_acc: Option<BifWaterAcc>,
    pub precip: Vec<f64>,
    pub precip_yield: Vec<f64>,
    pub precip_time: Vec<f64>,
    pub hist_acctime: f64,
    pub a_sedcon: Vec<f64>,
    pub a_sedout: Vec<f64>,
    pub a_bedout: Vec<f64>,
    pub a_sedinp: Vec<f64>,
    pub a_netflw: Vec<f64>,
    pub a_layer: Vec<f64>,
    pub a_shearvel: Vec<f64>,
}

/// `assert_sediment_mass_balance`。
fn assert_balance(context: &str, i: usize, before: f64, after: f64, expected: f64) -> Result<()> {
    let residual = after - before - expected;
    let scale = before.abs().max(after.abs()).max(expected.abs());
    let tolerance = SED_BALANCE_ABS_TOL + SED_BALANCE_REL_TOL * scale;
    ensure!(
        residual.is_finite() && residual.abs() <= tolerance,
        "sediment mass balance failure ({context}, unit catchment {}): residual {residual:e}",
        i + 1
    );
    Ok(())
}

/// 顺序求和（`sum(x)`：从 0 起逐项相加）。
fn sum(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |a, &b| a + b)
}

/// `calc_critical_shear_vel_sq`：临界剪切流速的平方 [(cm/s)²]。
fn critical_shear_vel_sq(diam: f64) -> f64 {
    let (a, b) = if diam >= 0.00303 {
        (80.9, 1.0)
    } else if diam >= 0.00118 {
        (134.6, 31.0 / 22.0)
    } else if diam >= 0.000565 {
        (55.0, 1.0)
    } else if diam >= 0.000065 {
        (8.41, 11.0 / 32.0)
    } else {
        (226.0, 1.0)
    };
    if b == 1.0 {
        a * (diam * 100.0)
    } else {
        a * (diam * 100.0).lpow(b)
    }
}

/// 开堤防/分汊时一个形态步的联合供体：初始浓度、初始床沙与各面共用的供体缩放。
#[derive(Debug, Clone)]
struct JointDonor {
    conc: Vec<f64>,
    bed: Vec<f64>,
    sed_scale: Vec<f64>,
    bed_scale: Vec<f64>,
}

/// 分汊路径的到达量（`bif_credit_*`），普通输运之后才加。
#[derive(Debug, Clone)]
struct BifCredits {
    sed_visible: Vec<f64>,
    sed_protected: Vec<f64>,
    bed_visible: Vec<f64>,
    bed_protected: Vec<f64>,
}

/// `prepare_bif_sediment` 的结果：各供体的联合缩放与路径信用。
struct BifPlan {
    sed_visible_scale: Vec<f64>,
    sed_protected_scale: Vec<f64>,
    bed_visible_scale: Vec<f64>,
    credits: BifCredits,
}

/// `joint_sediment_scale`：所有外流面共用同一份期初存量，总需求超过时按同一比例缩。
fn joint_scale(stock: f64, total_demand: f64) -> f64 {
    if total_demand > 0.0 {
        1.0f64.min(stock.max(0.0) / total_demand)
    } else {
        1.0
    }
}

impl Sediment {
    /// `grid_sediment_init`：读单元流域文件的 `sed_frc`、`sed_slope`，参数文件，建冷启动状态。
    pub fn init(network: &RiverNetwork, unitcatchment: &Path, param_file: &str) -> Result<Self> {
        let file = netcdf::open(unitcatchment).with_context(|| {
            format!(
                "cannot open unit-catchment file {}",
                unitcatchment.display()
            )
        })?;
        let nsed = file
            .dimension("sed_n")
            .context("the unit-catchment file has no sed_n dimension (sediment)")?
            .len();
        let nlfp = file
            .dimension("slope_layers")
            .context("the unit-catchment file has no slope_layers dimension (sediment)")?
            .len();
        let p = SedimentParams::read(param_file, nsed, nlfp)?;
        let n = network.len();
        let read = |name: &str, width: usize| -> Result<Vec<f64>> {
            let values = file
                .variable(name)
                .with_context(|| format!("the unit-catchment file has no {name}"))?
                .get_values::<f64, _>(..)
                .with_context(|| format!("cannot read {name}"))?;
            ensure!(
                values.len() == n * width,
                "{name} is not (nseqmax, {width})"
            );
            Ok(values)
        };
        let mut frc = read("sed_frc", nsed)?;
        let mut slope = read("sed_slope", nlfp)?;
        // `validate_sediment_parameters` 之后才截断/归一化，非法值不能被 max 掩盖。
        ensure!(
            frc.iter().all(|v| v.is_finite() && *v >= 0.0),
            "sed_frc must be finite and non-negative"
        );
        ensure!(
            slope
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0 && v.abs() <= 0.5 * colm_core::MISSING.abs()),
            "sed_slope must be finite, non-negative and not missing"
        );
        for v in &mut slope {
            *v = v.max(0.0);
        }
        // `normalize_sed_frc`。
        for row in frc.chunks_mut(nsed) {
            for v in row.iter_mut() {
                *v = v.max(0.0);
            }
            let total = sum(row);
            if total > 0.0 {
                for v in row.iter_mut() {
                    *v /= total;
                }
            } else {
                row.fill(1.0 / nsed as f64);
            }
        }
        let cells = n * nsed;
        let zeros = vec![0.0; cells];
        let mut sediment = Self {
            frc,
            slope,
            sedcon: zeros.clone(),
            sedsto: zeros.clone(),
            layer: zeros.clone(),
            seddep: vec![0.0; cells * p.totlyrnum],
            sedout: zeros.clone(),
            bedout: zeros.clone(),
            sedinp: zeros.clone(),
            netflw: zeros.clone(),
            exch_es_raw: zeros.clone(),
            exch_d_raw: zeros.clone(),
            exch_es_eff: zeros.clone(),
            exch_d_eff: zeros.clone(),
            netflw_adv_step: zeros.clone(),
            exch_d_adv_step: zeros.clone(),
            shearvel: vec![0.0; n],
            critshearvel: zeros.clone(),
            susvel: zeros.clone(),
            sedsto_protected: zeros.clone(),
            sedbed_protected: zeros.clone(),
            acc: vec![WaterAcc::default(); n],
            levee: None,
            bif_acc: None,
            precip: vec![0.0; n],
            precip_yield: vec![0.0; n],
            precip_time: vec![0.0; n],
            hist_acctime: 0.0,
            a_sedcon: zeros.clone(),
            a_sedout: zeros.clone(),
            a_bedout: zeros.clone(),
            a_sedinp: zeros.clone(),
            a_netflw: zeros.clone(),
            a_layer: zeros,
            a_shearvel: vec![0.0; n],
            p,
        };
        sediment.cold_start(network);
        Ok(sediment)
    }

    /// `initialize_sediment_state`：活动层厚 `lyrdph`，前 `totlyrnum-1` 层同活动层，最底层补足
    /// `sed_bed_depth`。
    fn cold_start(&mut self, network: &RiverNetwork) {
        let (ns, nl) = (self.p.nsed, self.p.totlyrnum);
        // `max(FNMA(lyrdph, totlyrnum, bed_depth), 0)`。
        let bottom = (-self.p.lyrdph)
            .contract(nl as f64, self.p.bed_depth)
            .max(0.0);
        for i in 0..network.len() {
            for s in 0..ns {
                let layer =
                    self.p.lyrdph * network.rivwth[i] * network.rivlen[i] * self.frc[i * ns + s];
                self.layer[i * ns + s] = layer;
                for l in 0..nl - 1 {
                    self.seddep[(i * nl + l) * ns + s] = layer;
                }
                self.seddep[(i * nl + nl - 1) * ns + s] =
                    bottom * network.rivwth[i] * network.rivlen[i] * self.frc[i * ns + s];
            }
        }
    }

    fn n(&self) -> usize {
        self.shearvel.len()
    }

    /// `sediment_forcing_put`：`precip` [mm/s]、`valid_fraction` 都是单元流域上的值。
    pub fn forcing_put(&mut self, precip: &[f64], dt: f64, valid_fraction: &[f64]) -> Result<()> {
        ensure!(dt.is_finite(), "sediment forcing: non-finite timestep");
        ensure!(dt > 0.0, "sediment forcing: non-positive timestep");
        for i in 0..self.n() {
            ensure!(
                valid_fraction[i].is_finite(),
                "sediment forcing: non-finite coverage"
            );
            ensure!(
                valid_fraction[i] >= 0.0,
                "sediment forcing: negative coverage"
            );
            let weight = dt * valid_fraction[i];
            if weight <= 0.0 {
                continue;
            }
            ensure!(
                precip[i].is_finite(),
                "sediment forcing: non-finite rain on valid area"
            );
            ensure!(
                precip[i] >= 0.0,
                "sediment forcing: negative rain on valid area"
            );
            self.precip[i] = precip[i].contract(weight, self.precip[i]);
            if precip[i] * 86400.0 > SED_PRECIP_THRESHOLD_MM_DAY {
                let rate = precip[i] * 3600.0;
                self.precip_yield[i] = rate
                    .lpow(self.p.pyldpc)
                    .contract(weight, self.precip_yield[i]);
            }
            self.precip_time[i] += weight;
        }
        Ok(())
    }

    /// `push_next2ucat`：每个单元流域取下游单元流域的值，没有下游的取 `fill`。
    fn push_next(network: &RiverNetwork, values: &[f64], width: usize, fill: f64) -> Vec<f64> {
        let mut out = vec![fill; values.len()];
        for (i, &next) in network.next.iter().enumerate() {
            if next >= 0 {
                let j = next as usize;
                out[i * width..(i + 1) * width]
                    .copy_from_slice(&values[j * width..(j + 1) * width]);
            }
        }
        out
    }

    /// `push_ups2ucat`（sum）：每个单元流域收上游单元流域的值之和（按上游序号递增；跳过 0，
    /// 首项直接赋值，与 `worker_push_data` 相同）。
    fn push_ups(network: &RiverNetwork, values: &[f64], width: usize) -> Vec<f64> {
        let mut out = vec![0.0; values.len()];
        for (j, ups) in network.upstream.iter().enumerate() {
            for s in 0..width {
                let mut total = 0.0;
                for &u in ups {
                    let value = values[u * width + s];
                    if value == 0.0 {
                        continue;
                    }
                    total = if total == 0.0 { value } else { total + value };
                }
                out[j * width + s] = total;
            }
        }
        out
    }

    /// `calc_critical_shear_egiazoroff`。
    fn critical_shear(&mut self, i: usize) {
        let ns = self.p.nsed;
        let layer = &self.layer[i * ns..(i + 1) * ns];
        let layer_sum = sum(layer);
        let out = &mut self.critshearvel[i * ns..(i + 1) * ns];
        if layer_sum <= 0.0 {
            out.fill(1.0e20);
            return;
        }
        let mut dmean = 0.0;
        for s in 0..ns {
            dmean += self.p.diam[s] * layer[s] / layer_sum;
        }
        let cs0 = critical_shear_vel_sq(dmean);
        for s in 0..ns {
            let ratio = self.p.diam[s] / dmean;
            out[s] = if ratio >= 0.4 {
                (cs0 * self.p.diam[s] / dmean).sqrt()
                    * (LOG10_19 / (19.0 * self.p.diam[s] / dmean).log10())
                    * 0.01
            } else {
                (0.85 * cs0).sqrt() * 0.01
            };
        }
    }

    /// `calc_suspend_velocity`。
    fn suspend_velocity(&mut self, i: usize) {
        let ns = self.p.nsed;
        let svel = self.shearvel[i];
        let alpha = self.p.vonkar / 6.0;
        let a = 0.08;
        let cb = 1.0 - self.p.lambda;
        for s in 0..ns {
            let k = i * ns + s;
            self.susvel[k] = 0.0;
            if self.critshearvel[k] > svel || svel <= 0.0 {
                continue;
            }
            // `cb*setvel/(1+s) * FNMA(s, a, 1) / FMA(s, 1-a, 1)`。
            let stmp = self.p.setvel[s] / alpha / svel;
            self.susvel[k] = (self.p.setvel[s] * cb / (1.0 + stmp) * (-stmp).contract(a, 1.0)
                / stmp.contract(1.0 - a, 1.0))
            .max(0.0);
        }
    }

    /// 一个流向的面通量（`calc_sediment_advection_one_direction`）。
    ///
    /// `conc` 是供体浓度（缺省用 `sedcon`）；`scales` 是联合供体缩放 `(悬沙, 推移质)`，
    /// 反向面取下游单元流域的缩放（没有下游时为 1）；`limit` 为假时不按供体存量限流。
    #[allow(clippy::too_many_arguments)]
    fn one_direction(
        &mut self,
        network: &RiverNetwork,
        dt: f64,
        rivout: &[f64],
        rivout_abs: &[f64],
        bed_donor: &[f64],
        avail_sto: &[f64],
        avail_bed: &[f64],
        conc: Option<&[f64]>,
        scales: Option<(&[f64], &[f64])>,
        limit: bool,
    ) {
        let ns = self.p.nsed;
        let conc = conc.map_or_else(|| self.sedcon.clone(), <[f64]>::to_vec);
        let sedcon_next = Self::push_next(network, &conc, ns, 0.0);
        let layer_next = Self::push_next(network, bed_donor, ns, 0.0);
        let crit_next = Self::push_next(network, &self.critshearvel, ns, 1.0e20);
        let shear_next = Self::push_next(network, &self.shearvel, 1, 0.0);
        let rivwth_next = Self::push_next(network, &network.rivwth, 1, 0.0);
        let rel = (self.p.psedd - self.p.pwatd) / self.p.pwatd;
        for i in 0..self.n() {
            let q = rivout[i];
            for s in 0..ns {
                let k = i * ns + s;
                self.sedout[k] = if q >= 0.0 {
                    conc[k] * q
                } else {
                    sedcon_next[k] * q
                };
                self.bedout[k] = 0.0;
            }
            let (donor, crit, shear, width, sign) = if q > 0.0 {
                (
                    &bed_donor[i * ns..(i + 1) * ns],
                    &self.critshearvel[i * ns..(i + 1) * ns],
                    self.shearvel[i],
                    network.rivwth[i],
                    1.0,
                )
            } else if q < 0.0 {
                (
                    &layer_next[i * ns..(i + 1) * ns],
                    &crit_next[i * ns..(i + 1) * ns],
                    shear_next[i],
                    rivwth_next[i],
                    -1.0,
                )
            } else {
                continue;
            };
            let layer_sum = sum(donor);
            if crit.iter().all(|&c| c >= shear) || layer_sum <= 0.0 {
                continue;
            }
            // 推移质取决于时段平均的剪切速度而不是流量，每个流向只算它所占的份额
            // `|rivout|/rivout_abs`（单向流为 1）。原来反向哪怕只占极小份额，也再算一份
            // 满强度的推移质，毛输运翻倍（upstream-bugs #80，vendor 同步）。
            let weight = if rivout_abs[i] > 0.0 {
                q.abs() / rivout_abs[i]
            } else {
                0.0
            };
            for s in 0..ns {
                if crit[s] >= shear || donor[s] <= 0.0 {
                    continue;
                }
                let plus = shear + crit[s];
                let minus = shear - crit[s];
                self.bedout[i * ns + s] =
                    sign * SED_BEDLOAD_COEFF * width * plus * (minus * minus) / rel / GRAV
                        * donor[s]
                        / layer_sum
                        * weight;
            }
        }
        if let Some((sed_scale, bed_scale)) = scales {
            for (flux, scale) in [(&mut self.sedout, sed_scale), (&mut self.bedout, bed_scale)] {
                let scale_next = Self::push_next(network, scale, ns, 1.0);
                for i in 0..rivout.len() {
                    let row = if rivout[i] >= 0.0 { scale } else { &scale_next };
                    for k in i * ns..(i + 1) * ns {
                        flux[k] *= row[k];
                    }
                }
            }
        }
        if !limit {
            return;
        }
        for k in 0..self.sedout.len() {
            if self.sedout[k] > 0.0 {
                self.sedout[k] = self.sedout[k].min(avail_sto[k] / dt);
            }
            if self.bedout[k] > 0.0 {
                self.bedout[k] = self.bedout[k].min(avail_bed[k] / dt);
            }
        }
        Self::limit_reverse(network, ns, &mut self.sedout, avail_sto, dt);
        Self::limit_reverse(network, ns, &mut self.bedout, avail_bed, dt);
    }

    /// `ordinary_sediment_donor_demand`：普通面的毛需求（与分汊同一份初始浓度与床沙组成），
    /// 反向面的需求记在真正的供体（下游单元流域）上。返回 `(悬沙, 推移质)` 体积。
    fn donor_demand(
        &mut self,
        network: &RiverNetwork,
        dt: f64,
        rivout_signed: &[f64],
        rivout_abs: &[f64],
        conc: &[f64],
        bed: &[f64],
    ) -> (Vec<f64>, Vec<f64>) {
        let ns = self.p.nsed;
        let n = self.n();
        let solid = 1.0 - self.p.lambda;
        let avail_sto = self.sedsto.clone();
        let avail_bed = bed.iter().map(|&b| solid * b).collect::<Vec<_>>();
        let forward = (0..n)
            .map(|i| (0.5 * (rivout_abs[i] + rivout_signed[i])).max(0.0))
            .collect::<Vec<_>>();
        self.one_direction(
            network,
            dt,
            &forward,
            rivout_abs,
            bed,
            &avail_sto,
            &avail_bed,
            Some(conc),
            None,
            false,
        );
        let mut sed_demand = self
            .sedout
            .iter()
            .map(|&v| v.max(0.0) * dt)
            .collect::<Vec<_>>();
        let mut bed_demand = self
            .bedout
            .iter()
            .map(|&v| v.max(0.0) * dt)
            .collect::<Vec<_>>();
        let reverse = (0..n)
            .map(|i| (0.5 * (rivout_signed[i] - rivout_abs[i])).min(0.0))
            .collect::<Vec<_>>();
        self.one_direction(
            network,
            dt,
            &reverse,
            rivout_abs,
            bed,
            &avail_sto,
            &avail_bed,
            Some(conc),
            None,
            false,
        );
        let sed_rev = self
            .sedout
            .iter()
            .map(|&v| (-v).max(0.0) * dt)
            .collect::<Vec<_>>();
        let bed_rev = self
            .bedout
            .iter()
            .map(|&v| (-v).max(0.0) * dt)
            .collect::<Vec<_>>();
        let sed_sum = Self::push_ups(network, &sed_rev, ns);
        let bed_sum = Self::push_ups(network, &bed_rev, ns);
        for k in 0..sed_demand.len() {
            sed_demand[k] += sed_sum[k];
            bed_demand[k] += bed_sum[k];
        }
        (sed_demand, bed_demand)
    }

    /// `limit_reverse_flux`：同一供体被多个上游反向抽取时，按供体存量统一缩放。
    fn limit_reverse(
        network: &RiverNetwork,
        ns: usize,
        flux: &mut [f64],
        storage: &[f64],
        dt: f64,
    ) {
        let n = network.len();
        for s in 0..ns {
            let demand = (0..n)
                .map(|i| (-flux[i * ns + s]).max(0.0) * dt)
                .collect::<Vec<_>>();
            let total = Self::push_ups(network, &demand, 1);
            let rate = (0..n)
                .map(|i| {
                    if total[i] > 1.0e-20 {
                        (storage[i * ns + s] / total[i]).min(1.0)
                    } else {
                        1.0
                    }
                })
                .collect::<Vec<_>>();
            let edge = Self::push_next(network, &rate, 1, 1.0);
            for i in 0..n {
                if flux[i * ns + s] < 0.0 {
                    flux[i * ns + s] *= edge[i];
                }
            }
        }
    }

    /// `calc_sediment_advection`（不开堤防/分汊）：两个流向都用同一份供体快照算面通量，
    /// 再一起散度更新。
    #[allow(clippy::too_many_arguments)]
    fn advection(
        &mut self,
        network: &RiverNetwork,
        dt: f64,
        rivout_signed: &[f64],
        rivout_abs: &[f64],
        rivsto_donor: &[f64],
        rivsto_end: &[f64],
        joint: Option<&JointDonor>,
    ) -> Result<()> {
        let ns = self.p.nsed;
        let n = self.n();
        let solid = 1.0 - self.p.lambda;
        self.netflw_adv_step.fill(0.0);
        self.exch_d_adv_step.fill(0.0);
        ensure!(
            self.sedsto.iter().all(|&v| v >= 0.0) && self.layer.iter().all(|&v| v >= 0.0),
            "sediment advection received invalid donor inventory"
        );
        let bed_donor = joint.map_or_else(|| self.layer.clone(), |j| j.bed.clone());
        for i in 0..n {
            let range = i * ns..(i + 1) * ns;
            let total = sum(&self.sedsto[range.clone()]);
            let cap = rivsto_donor[i].max(0.0) * MAX_SED_CONC;
            // 联合供体输运已从同一份初始存量里给各个面预留了量，这里不能先沉积。
            if joint.is_none() && total > cap {
                for k in range.clone() {
                    let d = ((total - cap) * self.sedsto[k] / total).min(self.sedsto[k]);
                    self.netflw_adv_step[k] -= d / dt;
                    self.exch_d_adv_step[k] += d / dt;
                    self.sedsto[k] -= d;
                    self.layer[k] += d / solid;
                }
            }
            for k in range {
                self.sedcon[k] = if rivsto_donor[i] > 0.0 {
                    self.sedsto[k] / rivsto_donor[i]
                } else {
                    0.0
                };
            }
        }
        let forward = (0..n)
            .map(|i| (0.5 * (rivout_abs[i] + rivout_signed[i])).max(0.0))
            .collect::<Vec<_>>();
        let reverse = (0..n)
            .map(|i| (0.5 * (rivout_signed[i] - rivout_abs[i])).min(0.0))
            .collect::<Vec<_>>();
        let mut avail_sto = self.sedsto.clone();
        let mut avail_bed = match joint {
            Some(_) => self.layer.iter().map(|&b| solid * b).collect::<Vec<_>>(),
            None => bed_donor.iter().map(|&b| solid * b).collect::<Vec<_>>(),
        };
        let conc = joint.map(|j| j.conc.as_slice());
        let scales = joint.map(|j| (j.sed_scale.as_slice(), j.bed_scale.as_slice()));
        // `FMA(sum(layer), 1-λ, sum(sedsto))`。
        let mass_before = (0..n)
            .map(|i| {
                sum(&self.layer[i * ns..(i + 1) * ns])
                    .contract(solid, sum(&self.sedsto[i * ns..(i + 1) * ns]))
            })
            .collect::<Vec<_>>();
        self.one_direction(
            network, dt, &forward, rivout_abs, &bed_donor, &avail_sto, &avail_bed, conc, scales,
            true,
        );
        let sedout_first = self.sedout.clone();
        let bedout_first = self.bedout.clone();
        for k in 0..avail_sto.len() {
            // `max(FNMA(dt, max(out_first, 0), avail), 0)`。
            avail_sto[k] = (-dt)
                .contract(sedout_first[k].max(0.0), avail_sto[k])
                .max(0.0);
            avail_bed[k] = (-dt)
                .contract(bedout_first[k].max(0.0), avail_bed[k])
                .max(0.0);
        }
        self.one_direction(
            network, dt, &reverse, rivout_abs, &bed_donor, &avail_sto, &avail_bed, conc, scales,
            true,
        );
        for k in 0..self.sedout.len() {
            self.sedout[k] += sedout_first[k];
            self.bedout[k] += bedout_first[k];
        }
        let sed_ups = Self::push_ups(network, &self.sedout, ns);
        let bed_ups = Self::push_ups(network, &self.bedout, ns);
        let eps8 = 8.0 * f64::EPSILON;
        for i in 0..n {
            let range = i * ns..(i + 1) * ns;
            for k in range.clone() {
                let sed_round = self.sedsto[k]
                    .max(dt * (self.sedout[k].abs() + sed_ups[k].abs()))
                    .contract(eps8, SED_BALANCE_ABS_TOL);
                let bed_round = self.layer[k]
                    .max(dt * (self.bedout[k].abs() + bed_ups[k].abs()) / solid)
                    .contract(eps8, SED_BALANCE_ABS_TOL);
                // `sedsto = FMA(dt, ups - out, sedsto)`；活动层不收缩。
                self.sedsto[k] = dt.contract(sed_ups[k] - self.sedout[k], self.sedsto[k]);
                self.layer[k] += dt * (bed_ups[k] - self.bedout[k]) / solid;
                ensure!(
                    self.sedsto[k] >= -sed_round && self.layer[k] >= -bed_round,
                    "sediment donor limiter produced negative inventory"
                );
            }
            for k in range.clone() {
                self.sedsto[k] = self.sedsto[k].max(0.0);
                self.layer[k] = self.layer[k].max(0.0);
            }
            if rivsto_end[i] > 0.0 {
                let total = sum(&self.sedsto[range.clone()]);
                let cap = rivsto_end[i] * MAX_SED_CONC;
                if total > cap {
                    for k in range.clone() {
                        let d = ((total - cap) * self.sedsto[k] / total).min(self.sedsto[k]);
                        self.netflw_adv_step[k] -= d / dt;
                        self.exch_d_adv_step[k] += d / dt;
                        self.sedsto[k] -= d;
                        self.layer[k] += d / solid;
                    }
                }
                for k in range.clone() {
                    self.sedcon[k] = self.sedsto[k] / rivsto_end[i];
                }
            } else {
                if sum(&self.sedsto[range.clone()]) > 0.0 {
                    for k in range.clone() {
                        self.netflw_adv_step[k] -= self.sedsto[k] / dt;
                        self.exch_d_adv_step[k] += self.sedsto[k] / dt;
                        self.layer[k] += self.sedsto[k] / solid;
                    }
                }
                for k in range.clone() {
                    self.sedcon[k] = 0.0;
                    self.sedsto[k] = 0.0;
                }
            }
            let after = sum(&self.sedsto[range.clone()]) + solid * sum(&self.layer[range.clone()]);
            let mut change = 0.0;
            for k in range {
                change += -self.sedout[k] + sed_ups[k] - self.bedout[k] + bed_ups[k];
            }
            assert_balance("advection", i, mass_before[i], after, dt * change)?;
        }
        Ok(())
    }

    /// `transfer_levee_sediment`：两遍保持供体次序。`to_protected` 为真时（输运之前）漫堤与
    /// 普通输运争同一份可见悬沙；为假时（输运之后）退水只用期初的堤内悬沙，然后堤内沉降、
    /// 两侧各自截浓度上限。堤内床沙在没有堤内剪切闭合之前不动。
    #[allow(clippy::too_many_arguments)]
    fn transfer_levee(
        &mut self,
        dt_morph: f64,
        dt_call: f64,
        visible_water: &[f64],
        protected_start: &[f64],
        protected_end: &[f64],
        protected_initial: &[f64],
        to_protected: bool,
        initial_visible: &[f64],
        initial_protected: &[f64],
        visible_scale: &[f64],
        protected_scale: &[f64],
    ) -> Result<()> {
        let ns = self.p.nsed;
        let solid = 1.0 - self.p.lambda;
        let mut before = vec![0.0; ns];
        let mut settled = vec![0.0; ns];
        for i in 0..self.n() {
            let range = i * ns..(i + 1) * ns;
            for (s, k) in range.clone().enumerate() {
                before[s] = solid
                    .contract(self.layer[k], self.sedsto[k] + self.sedsto_protected[k])
                    + self.sedbed_protected[k];
            }
            let acc = self.acc[i];
            if to_protected {
                let gross = acc.to_protected * dt_morph / dt_call;
                if gross > 0.0 && visible_water[i] > 0.0 {
                    for k in range.clone() {
                        let transfer = (initial_visible[k] * gross / visible_water[i]
                            * visible_scale[k])
                            .min(self.sedsto[k]);
                        self.sedsto[k] -= transfer;
                        self.sedsto_protected[k] += transfer;
                    }
                }
            } else {
                let gross = acc.from_protected * dt_morph / dt_call;
                if gross > 0.0 && protected_start[i] > 0.0 {
                    for k in range.clone() {
                        let transfer = (initial_protected[k] * gross / protected_start[i]
                            * protected_scale[k])
                            .min(protected_initial[k])
                            .min(self.sedsto_protected[k]);
                        self.sedsto_protected[k] -= transfer;
                        self.sedsto[k] += transfer;
                    }
                }
                if protected_end[i] > 0.0 {
                    let area = (acc.protected_area / acc.time.max(dt_morph)).max(0.0);
                    for (s, k) in range.clone().enumerate() {
                        let concentration = self.sedsto_protected[k] / protected_end[i];
                        settled[s] = self.sedsto_protected[k]
                            .min(concentration * (area * self.p.setvel[s]) * dt_morph);
                    }
                    for (s, k) in range.clone().enumerate() {
                        self.sedsto_protected[k] -= settled[s];
                        self.sedbed_protected[k] += settled[s];
                        self.netflw_adv_step[k] -= settled[s] / dt_morph;
                        self.exch_d_adv_step[k] += settled[s] / dt_morph;
                    }
                    let total = sum(&self.sedsto_protected[range.clone()]);
                    // `max(FNMA(protected_end, MAX_SED_CONC, Σ), 0)`。
                    let excess = (-protected_end[i]).contract(MAX_SED_CONC, total).max(0.0);
                    if excess > 0.0 {
                        for k in range.clone() {
                            let stock = self.sedsto_protected[k];
                            let transfer = if excess >= total {
                                stock
                            } else {
                                stock.min(stock * excess / total)
                            };
                            self.sedsto_protected[k] -= transfer;
                            self.sedbed_protected[k] += transfer;
                            self.netflw_adv_step[k] -= transfer / dt_morph;
                            self.exch_d_adv_step[k] += transfer / dt_morph;
                        }
                    }
                } else {
                    for k in range.clone() {
                        self.netflw_adv_step[k] -= self.sedsto_protected[k] / dt_morph;
                        self.exch_d_adv_step[k] += self.sedsto_protected[k] / dt_morph;
                        self.sedbed_protected[k] += self.sedsto_protected[k];
                        self.sedsto_protected[k] = 0.0;
                    }
                }
                self.settle_visible(i, visible_water[i], dt_morph);
            }
            for (s, k) in range.enumerate() {
                let after = solid
                    .contract(self.layer[k], self.sedsto[k] + self.sedsto_protected[k])
                    + self.sedbed_protected[k];
                assert_balance("levee exchange", i, before[s], after, 0.0)?;
            }
        }
        Ok(())
    }

    /// 可见悬沙截浓度上限、算浓度；没水时全部沉到活动层
    /// （`transfer_levee_sediment` 与 `apply_bif_sediment_credits` 共用的那一段）。
    fn settle_visible(&mut self, i: usize, water: f64, dt: f64) {
        let ns = self.p.nsed;
        let solid = 1.0 - self.p.lambda;
        let range = i * ns..(i + 1) * ns;
        if water > 0.0 {
            let total = sum(&self.sedsto[range.clone()]);
            let excess = (-water).contract(MAX_SED_CONC, total).max(0.0);
            if excess > 0.0 {
                for k in range.clone() {
                    let stock = self.sedsto[k];
                    let transfer = if excess >= total {
                        stock
                    } else {
                        stock.min(stock * excess / total)
                    };
                    self.sedsto[k] -= transfer;
                    self.layer[k] += transfer / solid;
                    self.netflw_adv_step[k] -= transfer / dt;
                    self.exch_d_adv_step[k] += transfer / dt;
                }
            }
            for k in range {
                self.sedcon[k] = self.sedsto[k] / water;
            }
        } else {
            for k in range {
                self.netflw_adv_step[k] -= self.sedsto[k] / dt;
                self.exch_d_adv_step[k] += self.sedsto[k] / dt;
                self.layer[k] += self.sedsto[k] / solid;
                self.sedsto[k] = 0.0;
                self.sedcon[k] = 0.0;
            }
        }
    }

    /// `prepare_bif_sediment`：先按逐层的分汊毛水量扣掉路径输运（与普通面争同一份期初存量），
    /// 再给出各供体的联合缩放；到达量留作信用，等普通输运之后再加（`apply_bif_credits`）。
    /// 返回 `(悬沙可见缩放, 悬沙堤内缩放, 推移质可见缩放, 信用)`。
    #[allow(clippy::too_many_arguments)]
    fn prepare_bif(
        &mut self,
        bif: &Bifurcation,
        dt_morph: f64,
        dt_call: f64,
        visible_water: &[f64],
        protected_water: &[f64],
        main_sed_demand: &[f64],
        main_bed_demand: &[f64],
        levee_to_demand: &[f64],
        levee_from_demand: &[f64],
    ) -> Result<BifPlan> {
        let ns = self.p.nsed;
        let n = self.n();
        let solid = 1.0 - self.p.lambda;
        let acc = self
            .bif_acc
            .clone()
            .context("sediment bifurcation accumulators")?;
        let levels = bif.levels;
        let paths = bif.paths();
        let cells = n * ns;
        let mut credits = BifCredits {
            sed_visible: vec![0.0; cells],
            sed_protected: vec![0.0; cells],
            bed_visible: vec![0.0; cells],
            bed_protected: vec![0.0; cells],
        };
        let mut sed_visible_scale = vec![1.0; cells];
        let mut sed_protected_scale = vec![1.0; cells];
        let mut bed_visible_scale = vec![1.0; cells];
        let flag = (0..n)
            .map(|i| self.levee.as_ref().is_some_and(|has| has[i]))
            .collect::<Vec<_>>();
        // `push_bif_dn2pth`：路径下游单元流域的值（下游不在网络里时取填充值）。
        let dn = |values: &[f64], fill: f64| -> Vec<f64> {
            bif.down
                .iter()
                .map(|d| d.map_or(fill, |j| values[j]))
                .collect()
        };
        // `push_bif_influx`（sum）：逐路径的值汇到下游单元流域，按路径号递增，跳过 0、首项直接赋值。
        let influx = |values: &[f64]| -> Vec<f64> {
            let mut out = vec![0.0; n];
            for (j, incoming) in bif.incoming.iter().enumerate() {
                for &p in incoming {
                    super::bifurcation::accumulate(&mut out[j], values[p]);
                }
            }
            out
        };
        let flag_dn = bif
            .down
            .iter()
            .map(|d| d.is_some_and(|j| flag[j]))
            .collect::<Vec<_>>();
        let density_ratio = (self.p.psedd - self.p.pwatd) / self.p.pwatd;
        let bed_snapshot = self.layer.clone();
        for kind in 0..2 {
            for s in 0..ns {
                let mut rate_visible = vec![0.0; n];
                let mut rate_protected = vec![0.0; n];
                let mut available_visible;
                let mut available_protected;
                if kind == 0 {
                    available_visible = (0..n).map(|i| self.sedsto[i * ns + s]).collect::<Vec<_>>();
                    available_protected = (0..n)
                        .map(|i| self.sedsto_protected[i * ns + s])
                        .collect::<Vec<_>>();
                    for i in 0..n {
                        if visible_water[i] > 0.0 {
                            rate_visible[i] =
                                available_visible[i] / visible_water[i].max(f64::MIN_POSITIVE);
                        }
                        if protected_water[i] > 0.0 {
                            rate_protected[i] =
                                available_protected[i] / protected_water[i].max(f64::MIN_POSITIVE);
                        }
                    }
                } else {
                    available_visible = (0..n)
                        .map(|i| solid * bed_snapshot[i * ns + s])
                        .collect::<Vec<_>>();
                    available_protected = vec![0.0; n];
                    for i in 0..n {
                        let layer_sum = sum(&bed_snapshot[i * ns..(i + 1) * ns]);
                        let crit = self.critshearvel[i * ns + s];
                        if layer_sum <= 0.0 || self.shearvel[i] <= crit {
                            continue;
                        }
                        let plus = self.shearvel[i] + crit;
                        let minus = self.shearvel[i] - crit;
                        rate_visible[i] =
                            SED_BEDLOAD_COEFF * plus * (minus * minus) / density_ratio / GRAV
                                * bed_snapshot[i * ns + s]
                                / layer_sum;
                    }
                }
                let budget_before = sum(&available_visible) + sum(&available_protected);
                let rate_visible_dn = dn(&rate_visible, 0.0);
                let rate_protected_dn = dn(&rate_protected, 0.0);
                let mut demand_forward = vec![0.0; paths * levels];
                let mut demand_reverse = vec![0.0; paths * levels];
                for p in 0..paths {
                    let i = bif.upst[p];
                    for l in 0..levels {
                        let k = p * levels + l;
                        let q_forward = acc.forward[k] * dt_morph / dt_call;
                        let q_reverse = acc.reverse[k] * dt_morph / dt_call;
                        let upstream_protected = l >= 1 && flag[i];
                        let downstream_protected = l >= 1 && flag_dn[p];
                        if q_forward > 0.0 {
                            if kind == 0 {
                                demand_forward[k] = q_forward
                                    * if upstream_protected {
                                        rate_protected[i]
                                    } else {
                                        rate_visible[i]
                                    };
                            } else if !upstream_protected {
                                demand_forward[k] =
                                    bif.wth[k] * rate_visible[i] * acc.forward_time[k] * dt_morph
                                        / dt_call;
                            }
                        }
                        if q_reverse > 0.0 {
                            if kind == 0 {
                                demand_reverse[k] = q_reverse
                                    * if downstream_protected {
                                        rate_protected_dn[p]
                                    } else {
                                        rate_visible_dn[p]
                                    };
                            } else if !downstream_protected {
                                demand_reverse[k] = bif.wth[k]
                                    * rate_visible_dn[p]
                                    * acc.reverse_time[k]
                                    * dt_morph
                                    / dt_call;
                            }
                        }
                    }
                }
                // 反向路径的需求收到它的下游供体上，再按供体的共享存量缩放所有外流路径。
                let mut recv_visible = vec![0.0; n];
                let mut recv_protected = vec![0.0; n];
                let mut path_visible = vec![0.0; paths];
                let mut path_protected = vec![0.0; paths];
                for p in 0..paths {
                    let i = bif.upst[p];
                    for l in 0..levels {
                        let k = p * levels + l;
                        if l >= 1 && flag[i] {
                            recv_protected[i] += demand_forward[k];
                        } else {
                            recv_visible[i] += demand_forward[k];
                        }
                        if l >= 1 && flag_dn[p] {
                            path_protected[p] += demand_reverse[k];
                        } else {
                            path_visible[p] += demand_reverse[k];
                        }
                    }
                }
                let in_visible = influx(&path_visible);
                let in_protected = influx(&path_protected);
                for i in 0..n {
                    recv_visible[i] += in_visible[i];
                    recv_protected[i] += in_protected[i];
                    let k = i * ns + s;
                    if kind == 0 {
                        recv_visible[i] = recv_visible[i] + main_sed_demand[k] + levee_to_demand[k];
                        recv_protected[i] += levee_from_demand[k];
                    } else {
                        recv_visible[i] += main_bed_demand[k];
                    }
                }
                let scale_visible = (0..n)
                    .map(|i| joint_scale(available_visible[i], recv_visible[i]))
                    .collect::<Vec<_>>();
                let scale_protected = (0..n)
                    .map(|i| joint_scale(available_protected[i], recv_protected[i]))
                    .collect::<Vec<_>>();
                for i in 0..n {
                    if kind == 0 {
                        sed_visible_scale[i * ns + s] = scale_visible[i];
                        sed_protected_scale[i * ns + s] = scale_protected[i];
                    } else {
                        bed_visible_scale[i * ns + s] = scale_visible[i];
                    }
                }
                let scale_visible_dn = dn(&scale_visible, 0.0);
                let scale_protected_dn = dn(&scale_protected, 0.0);
                path_visible.fill(0.0);
                path_protected.fill(0.0);
                for p in 0..paths {
                    let i = bif.upst[p];
                    for l in 0..levels {
                        let k = p * levels + l;
                        if l >= 1 && flag[i] {
                            demand_forward[k] *= scale_protected[i];
                            available_protected[i] -= demand_forward[k];
                        } else {
                            demand_forward[k] *= scale_visible[i];
                            available_visible[i] -= demand_forward[k];
                        }
                        if l >= 1 && flag_dn[p] {
                            demand_reverse[k] *= scale_protected_dn[p];
                            path_protected[p] += demand_reverse[k];
                        } else {
                            demand_reverse[k] *= scale_visible_dn[p];
                            path_visible[p] += demand_reverse[k];
                        }
                    }
                }
                let out_visible = influx(&path_visible);
                let out_protected = influx(&path_protected);
                ensure!(
                    (0..n).all(
                        |i| available_visible[i] - out_visible[i] >= -SED_BALANCE_ABS_TOL
                            && available_protected[i] - out_protected[i] >= -SED_BALANCE_ABS_TOL
                    ),
                    "sediment BIF donor limiter overdraw"
                );
                for i in 0..n {
                    available_visible[i] = (available_visible[i] - out_visible[i]).max(0.0);
                    available_protected[i] = (available_protected[i] - out_protected[i]).max(0.0);
                }
                // 信用取另一端：正向路径的到达量收在下游，反向路径的到达量收在本地上游。
                path_visible.fill(0.0);
                path_protected.fill(0.0);
                for p in 0..paths {
                    for l in 0..levels {
                        let k = p * levels + l;
                        if l >= 1 && flag_dn[p] {
                            path_protected[p] += demand_forward[k];
                        } else {
                            path_visible[p] += demand_forward[k];
                        }
                    }
                }
                let mut credit_visible = influx(&path_visible);
                let mut credit_protected = influx(&path_protected);
                for p in 0..paths {
                    let i = bif.upst[p];
                    for l in 0..levels {
                        let k = p * levels + l;
                        if l >= 1 && flag[i] {
                            credit_protected[i] += demand_reverse[k];
                        } else {
                            credit_visible[i] += demand_reverse[k];
                        }
                    }
                }
                let budget_after = sum(&available_visible)
                    + (sum(&available_protected) + (sum(&credit_visible) + sum(&credit_protected)));
                assert_balance("bif path transport", s, budget_before, budget_after, 0.0)?;
                for i in 0..n {
                    let k = i * ns + s;
                    if kind == 0 {
                        self.sedsto[k] = available_visible[i];
                        self.sedsto_protected[k] = available_protected[i];
                        credits.sed_visible[k] = credit_visible[i];
                        credits.sed_protected[k] = credit_protected[i];
                    } else {
                        self.layer[k] = available_visible[i] / solid;
                        credits.bed_visible[k] = credit_visible[i];
                        credits.bed_protected[k] = credit_protected[i];
                    }
                }
            }
        }
        Ok(BifPlan {
            sed_visible_scale,
            sed_protected_scale,
            bed_visible_scale,
            credits,
        })
    }

    /// `apply_bif_sediment_credits`：普通输运之后加上分汊路径的到达量，再截可见侧浓度上限。
    fn apply_bif_credits(&mut self, dt: f64, visible_water: &[f64], credits: &BifCredits) {
        let solid = 1.0 - self.p.lambda;
        for k in 0..self.sedsto.len() {
            self.sedsto[k] += credits.sed_visible[k];
            self.sedsto_protected[k] += credits.sed_protected[k];
            self.layer[k] += credits.bed_visible[k] / solid;
            self.sedbed_protected[k] += credits.bed_protected[k];
        }
        for i in 0..self.n() {
            self.settle_visible(i, visible_water[i], dt);
        }
    }

    /// `calc_sediment_yield`：坡面产沙（按淹没比例只算未淹没的坡面层）。
    fn yield_input(&mut self, fldfrc: &[f64], area: &[f64]) {
        let (ns, nlfp) = (self.p.nsed, self.p.nlfp);
        self.sedinp.fill(0.0);
        for i in 0..self.n() {
            if self.precip_time[i] <= 0.0 || self.precip_yield[i] <= 0.0 {
                continue;
            }
            let avg = self.precip_yield[i] / self.precip_time[i];
            for l in 1..=nlfp {
                if fldfrc[i] * nlfp as f64 > l as f64 {
                    continue;
                }
                let base = self.p.pyld * avg * self.slope[i * nlfp + l - 1].lpow(self.p.pyldc)
                    / 3600.0
                    * area[i]
                    * (l as f64 / nlfp as f64 - fldfrc[i]).min(1.0 / nlfp as f64)
                    * self.p.dsylunit;
                for s in 0..ns {
                    let k = i * ns + s;
                    self.sedinp[k] = base.contract(self.frc[k], self.sedinp[k]);
                }
            }
        }
    }

    /// `calc_sediment_exchange`：悬浮（Es）与沉降（D，含 Rouse 剖面修正）的交换。
    fn exchange(&mut self, dt: f64, rivsto: &[f64], bed_area: &[f64]) -> Result<()> {
        let ns = self.p.nsed;
        let solid = 1.0 - self.p.lambda;
        self.exch_es_raw.fill(0.0);
        self.exch_d_raw.fill(0.0);
        self.exch_es_eff.fill(0.0);
        self.exch_d_eff.fill(0.0);
        let mut es = vec![0.0; ns];
        let mut d = vec![0.0; ns];
        for i in 0..self.n() {
            let range = i * ns..(i + 1) * ns;
            if rivsto[i] <= 0.0 || rivsto[i] < bed_area[i] * self.p.ignore_dph {
                self.netflw[range].fill(0.0);
                continue;
            }
            let before = sum(&self.sedsto[range.clone()]) + solid * sum(&self.layer[range.clone()]);
            let layer_sum = sum(&self.layer[range.clone()]);
            let area = bed_area[i];
            if layer_sum <= 0.0 || self.susvel[range.clone()].iter().all(|&v| v <= 0.0) {
                es.fill(0.0);
            } else {
                for s in 0..ns {
                    let k = i * ns + s;
                    es[s] = (self.susvel[k] * solid * area * self.layer[k] / layer_sum).max(0.0);
                }
            }
            if self.p.setvel.iter().all(|&v| v <= 0.0) {
                d.fill(0.0);
            } else {
                let shear = self.shearvel[i];
                let shear_eff = shear.max(EXCH_SHEARVEL_MIN);
                for s in 0..ns {
                    let k = i * ns + s;
                    let rouse = if shear <= EXCH_SHEARVEL_MIN {
                        1.0
                    } else {
                        let zd =
                            (6.0 * self.p.setvel[s] / self.p.vonkar / shear_eff).min(EXCH_ZD_MAX);
                        let profile = if zd.abs() < 1.0e-8 {
                            1.0
                        } else {
                            zd / (1.0 - (-zd).exp())
                        };
                        let mut w = ((shear - EXCH_SHEARVEL_MIN)
                            / (EXCH_SHEARVEL_BLEND - EXCH_SHEARVEL_MIN))
                            .max(0.0)
                            .min(1.0);
                        w = w * w * (-w).contract(2.0, 3.0);
                        // `FMA(w, profile-1, 1)`。
                        w.contract(profile - 1.0, 1.0)
                    };
                    let raw = self.p.setvel[s] * area * self.sedcon[k] * rouse;
                    d[s] = raw.max(0.0).min(self.sedsto[k] / dt);
                }
            }
            for s in 0..ns {
                let k = i * ns + s;
                self.exch_es_raw[k] = es[s];
                self.exch_d_raw[k] = d[s];
                self.netflw[k] = es[s] - d[s];
            }
            for s in 0..ns {
                let k = i * ns + s;
                let net = self.netflw[k];
                if net.abs() < 1.0e-20 {
                    continue;
                } else if net > 0.0 {
                    let take = net * dt / solid;
                    if take < self.layer[k] {
                        self.layer[k] -= take;
                    } else {
                        self.netflw[k] = self.layer[k] * solid / dt;
                        self.layer[k] = 0.0;
                    }
                    self.sedsto[k] += self.netflw[k] * dt;
                } else {
                    if net.abs() * dt < self.sedsto[k] {
                        self.sedsto[k] = (self.sedsto[k] - net.abs() * dt).max(0.0);
                    } else {
                        self.netflw[k] = -self.sedsto[k] / dt;
                        self.sedsto[k] = 0.0;
                    }
                    self.layer[k] += self.netflw[k].abs() * dt / solid;
                }
            }
            for s in 0..ns {
                let k = i * ns + s;
                if es[s] >= d[s] {
                    self.exch_d_eff[k] = d[s];
                    self.exch_es_eff[k] = d[s] + self.netflw[k].max(0.0);
                } else {
                    self.exch_es_eff[k] = es[s];
                    self.exch_d_eff[k] = es[s] + (-self.netflw[k]).max(0.0);
                }
            }
            self.cap_concentration(i, rivsto[i], dt, true);
            if rivsto[i] > 0.0 {
                for k in range.clone() {
                    self.sedcon[k] = self.sedsto[k] / rivsto[i];
                }
            }
            let after = sum(&self.sedsto[range.clone()]) + solid * sum(&self.layer[range]);
            assert_balance("exchange", i, before, after, 0.0)?;
        }
        Ok(())
    }

    /// 交换与产沙入河之后的浓度上限：超出的部分沉到活动层，记成净沉积。
    fn cap_concentration(&mut self, i: usize, rivsto: f64, dt: f64, require_water: bool) {
        let ns = self.p.nsed;
        let solid = 1.0 - self.p.lambda;
        let range = i * ns..(i + 1) * ns;
        let total = sum(&self.sedsto[range.clone()]);
        if (!require_water || rivsto > 0.0) && total > rivsto * MAX_SED_CONC {
            for k in range {
                let d = ((total - rivsto * MAX_SED_CONC) * self.sedsto[k] / total.max(1.0e-20))
                    .min(self.sedsto[k]);
                self.netflw[k] -= d / dt;
                self.exch_d_eff[k] += d / dt;
                self.sedsto[k] -= d;
                self.layer[k] += d / solid;
            }
        }
    }

    /// `apply_sediment_input`：水够深就进悬沙（再截浓度上限），否则直接沉到活动层。
    fn apply_input(&mut self, dt: f64, rivsto: &[f64], bed_area: &[f64]) -> Result<()> {
        let ns = self.p.nsed;
        let solid = 1.0 - self.p.lambda;
        for i in 0..self.n() {
            let range = i * ns..(i + 1) * ns;
            let input = sum(&self.sedinp[range.clone()]);
            if input <= 0.0 {
                continue;
            }
            let before = sum(&self.sedsto[range.clone()]) + solid * sum(&self.layer[range.clone()]);
            if rivsto[i] > 0.0 && rivsto[i] >= bed_area[i] * self.p.ignore_dph {
                for k in range.clone() {
                    self.sedsto[k] = self.sedinp[k].contract(dt, self.sedsto[k]);
                }
                self.cap_concentration(i, rivsto[i], dt, false);
                for k in range.clone() {
                    self.sedcon[k] = self.sedsto[k] / rivsto[i];
                }
            } else {
                for k in range.clone() {
                    self.layer[k] += self.sedinp[k] * dt / solid;
                    self.netflw[k] -= self.sedinp[k];
                    self.exch_d_eff[k] += self.sedinp[k];
                }
            }
            let after = sum(&self.sedsto[range.clone()]) + solid * sum(&self.layer[range]);
            assert_balance("hillslope input", i, before, after, input * dt)?;
        }
        Ok(())
    }

    /// `accumulate_sediment_output`。
    fn accumulate_output(&mut self, dt: f64) {
        for k in 0..self.sedcon.len() {
            self.a_sedcon[k] = self.sedcon[k].contract(dt, self.a_sedcon[k]);
            self.a_sedout[k] = self.sedout[k].contract(dt, self.a_sedout[k]);
            self.a_bedout[k] = self.bedout[k].contract(dt, self.a_bedout[k]);
            self.a_sedinp[k] = self.sedinp[k].contract(dt, self.a_sedinp[k]);
            self.a_netflw[k] =
                (self.netflw[k] + self.netflw_adv_step[k]).contract(dt, self.a_netflw[k]);
            self.a_layer[k] = self.layer[k].contract(dt, self.a_layer[k]);
        }
        for i in 0..self.n() {
            self.a_shearvel[i] = self.shearvel[i].contract(dt, self.a_shearvel[i]);
        }
    }

    /// `calc_layer_redistribution`：活动层保持 `lyrdph*bed_area`，多余的压进沉积层、不足的从
    /// 沉积层自上而下补。
    fn layer_redistribution(&mut self, bed_area: &[f64]) -> Result<()> {
        let (ns, nl) = (self.p.nsed, self.p.totlyrnum);
        let solid = 1.0 - self.p.lambda;
        let mut layer_p = vec![0.0; ns];
        let mut dep_p = vec![0.0; ns * (nl + 1)];
        for i in 0..self.n() {
            let lyrvol = self.p.lyrdph * bed_area[i];
            let lr = i * ns..(i + 1) * ns;
            let dr = i * nl * ns..(i + 1) * nl * ns;
            let before = solid * (sum(&self.layer[lr.clone()]) + sum(&self.seddep[dr.clone()]));
            let mut slyr = 0usize;
            for k in lr.clone() {
                self.layer[k] = self.layer[k].max(0.0);
            }
            for k in dr.clone() {
                self.seddep[k] = self.seddep[k].max(0.0);
            }
            let check = |s: &Self| -> Result<()> {
                let after = solid * (sum(&s.layer[lr.clone()]) + sum(&s.seddep[dr.clone()]));
                assert_balance("layer redistribution", i, before, after, 0.0)
            };
            if sum(&self.layer[lr.clone()]) + sum(&self.seddep[dr.clone()]) <= lyrvol {
                for s in 0..ns {
                    let mut total = 0.0;
                    for l in 0..nl {
                        total += self.seddep[(i * nl + l) * ns + s];
                    }
                    self.layer[i * ns + s] += total;
                }
                self.seddep[dr.clone()].fill(0.0);
                check(self)?;
                continue;
            }
            layer_p.copy_from_slice(&self.layer[lr.clone()]);
            if sum(&layer_p) >= lyrvol {
                let scale = (lyrvol / sum(&layer_p).max(1.0e-20)).min(1.0);
                for s in 0..ns {
                    self.layer[i * ns + s] = layer_p[s] * scale;
                    layer_p[s] = (layer_p[s] - self.layer[i * ns + s]).max(0.0);
                }
                slyr = 0;
            } else if sum(&self.seddep[dr.clone()]) > 0.0 {
                layer_p.fill(0.0);
                for l in 1..=nl {
                    let diff = lyrvol - sum(&self.layer[lr.clone()]);
                    if diff <= 0.0 {
                        break;
                    }
                    let dl = (i * nl + l - 1) * ns..(i * nl + l) * ns;
                    let tmpsum = sum(&self.seddep[dl.clone()]);
                    if tmpsum <= diff {
                        for s in 0..ns {
                            self.layer[i * ns + s] += self.seddep[dl.start + s];
                        }
                        self.seddep[dl].fill(0.0);
                        slyr = l + 1;
                    } else {
                        for s in 0..ns {
                            let tmp = if tmpsum > 1.0e-20 {
                                diff * self.seddep[dl.start + s] / tmpsum
                            } else {
                                0.0
                            };
                            self.layer[i * ns + s] += tmp;
                            self.seddep[dl.start + s] = (self.seddep[dl.start + s] - tmp).max(0.0);
                        }
                        slyr = l;
                        break;
                    }
                }
            } else {
                self.seddep[dr.clone()].fill(0.0);
                check(self)?;
                continue;
            }
            if sum(&self.seddep[dr.clone()]) <= 0.0 && sum(&layer_p) <= 0.0 {
                check(self)?;
                continue;
            }
            // `seddepP(:,1)` 是活动层压出的部分，`seddepP(:,2:)` 是原来的沉积层。
            dep_p[..ns].copy_from_slice(&layer_p);
            dep_p[ns..].copy_from_slice(&self.seddep[dr.clone()]);
            self.seddep[dr.clone()].fill(0.0);
            for il in 1..nl {
                let dl = (i * nl + il - 1) * ns..(i * nl + il) * ns;
                if sum(&self.seddep[dl.clone()]) >= lyrvol {
                    continue;
                }
                for jl in slyr + 1..=nl + 1 {
                    let diff = lyrvol - sum(&self.seddep[dl.clone()]);
                    if diff <= 0.0 {
                        break;
                    }
                    let pl = (jl - 1) * ns..jl * ns;
                    let tmpsum = sum(&dep_p[pl.clone()]);
                    if tmpsum <= diff {
                        for s in 0..ns {
                            self.seddep[dl.start + s] += dep_p[pl.start + s];
                        }
                        dep_p[pl].fill(0.0);
                    } else {
                        for s in 0..ns {
                            let tmp = if tmpsum > 1.0e-20 {
                                diff * dep_p[pl.start + s] / tmpsum
                            } else {
                                0.0
                            };
                            self.seddep[dl.start + s] += tmp;
                            dep_p[pl.start + s] = (dep_p[pl.start + s] - tmp).max(0.0);
                        }
                        break;
                    }
                }
            }
            if sum(&dep_p) > 0.0 {
                for s in 0..ns {
                    let mut total = 0.0;
                    for jl in 0..=nl {
                        total += dep_p[jl * ns + s];
                    }
                    self.seddep[(i * nl + nl - 1) * ns + s] += total;
                }
            }
            check(self)?;
        }
        Ok(())
    }

    /// `grid_sediment_calc`：一次汇流（`deltime = acctime_rnof`）之后的形态步。
    pub fn calc(
        &mut self,
        network: &RiverNetwork,
        bif: Option<&Bifurcation>,
        deltime: f64,
    ) -> Result<()> {
        let n = self.n();
        let ns = self.p.nsed;
        let levee_on = self.levee.is_some();
        ensure!(
            bif.is_some() == self.bif_acc.is_some(),
            "sediment bifurcation accumulators do not match the river model"
        );
        let joint = levee_on || bif.is_some();
        let fldfrc = (0..n)
            .map(|i| {
                let acc = &self.acc[i];
                let f = if acc.time > 0.0 && network.area[i] > 0.0 {
                    acc.floodarea / acc.time / network.area[i]
                } else {
                    0.0
                };
                f.max(0.0).min(1.0)
            })
            .collect::<Vec<_>>();
        let mut rivsto = vec![0.0; n];
        let mut rivsto_donor = vec![0.0; n];
        let mut rivout = vec![0.0; n];
        let mut rivout_abs = vec![0.0; n];
        let mut bed_area = vec![0.0; n];
        let mut protected_start = vec![0.0; n];
        let mut protected_end = vec![0.0; n];
        let mut remaining = deltime;
        let mut elapsed = 0.0;
        let mut iter = 0;
        while remaining > 0.0 {
            iter += 1;
            let dt = remaining.min(self.p.dt_max);
            let start_fraction = elapsed / deltime;
            let end_fraction = (elapsed + dt) / deltime;
            for i in 0..n {
                let acc = self.acc[i];
                if acc.time > 0.0 {
                    let avg_v2 = acc.v2 / acc.time;
                    let avg_wdsrf = acc.wdsrf / acc.time;
                    let avg_rivsto = acc.rivsto / acc.time;
                    let avg_rivout = acc.rivout / acc.time;
                    let avg_abs = acc.abs_rivout / acc.time;
                    ensure!(
                        avg_rivsto.is_finite() && avg_rivout.is_finite() && avg_abs.is_finite(),
                        "sediment routing water storage/discharge must be finite"
                    );
                    self.shearvel[i] = if avg_wdsrf > 0.0 && avg_v2 > 0.0 {
                        let man = network.rivman[i];
                        (GRAV * (man * man) * avg_v2 * avg_wdsrf.lpow(-1.0 / 3.0)).sqrt()
                    } else {
                        0.0
                    };
                    self.critical_shear(i);
                    self.suspend_velocity(i);
                    // 堤内与总蓄量都在首末端点之间线性插值：
                    // `max(FMA(end-start, f, start), 0)`，可见 `max(FMA(...) - protected, 0)`。
                    let pspan = acc.protected_end - acc.protected_start;
                    protected_start[i] =
                        pspan.contract(start_fraction, acc.protected_start).max(0.0);
                    protected_end[i] = pspan.contract(end_fraction, acc.protected_start).max(0.0);
                    let span = acc.rivsto_end - acc.rivsto_start;
                    rivsto_donor[i] = (span.contract(start_fraction, acc.rivsto_start)
                        - protected_start[i])
                        .max(0.0);
                    rivsto[i] =
                        (span.contract(end_fraction, acc.rivsto_start) - protected_end[i]).max(0.0);
                    rivout[i] = avg_rivout;
                    rivout_abs[i] = avg_abs;
                    bed_area[i] = network.rivwth[i] * network.rivlen[i];
                    if avg_v2 <= 0.0 {
                        bed_area[i] = bed_area[i].max(acc.floodarea / acc.time);
                    }
                } else {
                    self.shearvel[i] = 0.0;
                    self.critshearvel[i * ns..(i + 1) * ns].fill(1.0e20);
                    self.susvel[i * ns..(i + 1) * ns].fill(0.0);
                    rivsto[i] = 0.0;
                    rivsto_donor[i] = 0.0;
                    protected_start[i] = 0.0;
                    protected_end[i] = 0.0;
                    rivout[i] = 0.0;
                    rivout_abs[i] = 0.0;
                    bed_area[i] = network.rivwth[i] * network.rivlen[i];
                }
            }
            if iter == 1 || joint {
                self.begin_period(&rivsto_donor)?;
            }
            ensure!(
                dt.is_finite() && dt > 0.0,
                "sediment morphology interval must be finite and positive"
            );
            // 开堤防/分汊：普通面、漫堤与分汊路径都从同一份期初存量里按联合缩放取量。
            let mut joint_donor = None;
            let mut credits = None;
            let (donor_visible, donor_protected) =
                (self.sedsto.clone(), self.sedsto_protected.clone());
            let mut sed_protected_scale = vec![1.0; n * ns];
            if joint {
                let donor_bed = self.layer.clone();
                let donor_conc = self.sedcon.clone();
                let (main_sed, main_bed) =
                    self.donor_demand(network, dt, &rivout, &rivout_abs, &donor_conc, &donor_bed);
                let mut levee_to = vec![0.0; n * ns];
                let mut levee_from = vec![0.0; n * ns];
                if levee_on {
                    for i in 0..n {
                        let acc = self.acc[i];
                        for k in i * ns..(i + 1) * ns {
                            if rivsto_donor[i] > 0.0 {
                                levee_to[k] = donor_visible[k] * acc.to_protected * dt
                                    / deltime
                                    / rivsto_donor[i];
                            }
                            if protected_start[i] > 0.0 {
                                levee_from[k] = donor_protected[k] * acc.from_protected * dt
                                    / deltime
                                    / protected_start[i];
                            }
                        }
                    }
                }
                let (sed_visible_scale, bed_visible_scale) = if let Some(bif) = bif {
                    let plan = self.prepare_bif(
                        bif,
                        dt,
                        deltime,
                        &rivsto_donor,
                        &protected_start,
                        &main_sed,
                        &main_bed,
                        &levee_to,
                        &levee_from,
                    )?;
                    sed_protected_scale = plan.sed_protected_scale;
                    credits = Some(plan.credits);
                    (plan.sed_visible_scale, plan.bed_visible_scale)
                } else {
                    let solid = 1.0 - self.p.lambda;
                    let sv = (0..n * ns)
                        .map(|k| joint_scale(donor_visible[k], main_sed[k] + levee_to[k]))
                        .collect::<Vec<_>>();
                    sed_protected_scale = (0..n * ns)
                        .map(|k| joint_scale(donor_protected[k], levee_from[k]))
                        .collect();
                    let bv = (0..n * ns)
                        .map(|k| joint_scale(solid * donor_bed[k], main_bed[k]))
                        .collect::<Vec<_>>();
                    (sv, bv)
                };
                joint_donor = Some(JointDonor {
                    conc: donor_conc,
                    bed: donor_bed,
                    sed_scale: sed_visible_scale,
                    bed_scale: bed_visible_scale,
                });
            }
            let protected_initial = self.sedsto_protected.clone();
            if let (true, Some(j)) = (levee_on, joint_donor.as_ref()) {
                let scale = j.sed_scale.clone();
                self.transfer_levee(
                    dt,
                    deltime,
                    &rivsto_donor,
                    &protected_start,
                    &protected_end,
                    &protected_initial,
                    true,
                    &donor_visible,
                    &donor_protected,
                    &scale,
                    &sed_protected_scale,
                )?;
            }
            self.advection(
                network,
                dt,
                &rivout,
                &rivout_abs,
                &rivsto_donor,
                &rivsto,
                joint_donor.as_ref(),
            )?;
            if let Some(credits) = credits.as_ref() {
                self.apply_bif_credits(dt, &rivsto, credits);
            }
            if let (true, Some(j)) = (levee_on, joint_donor.as_ref()) {
                self.transfer_levee(
                    dt,
                    deltime,
                    &rivsto,
                    &protected_start,
                    &protected_end,
                    &protected_initial,
                    false,
                    &donor_visible,
                    &donor_protected,
                    &j.sed_scale,
                    &sed_protected_scale,
                )?;
            }
            self.yield_input(&fldfrc, &network.area);
            self.exchange(dt, &rivsto, &bed_area)?;
            self.apply_input(dt, &rivsto, &bed_area)?;
            self.accumulate_output(dt);
            self.layer_redistribution(&bed_area)?;
            remaining -= dt;
            elapsed += dt;
        }
        self.commit_period(&rivsto)?;
        self.hist_acctime += deltime;
        self.acc.fill(WaterAcc::default());
        if let Some(acc) = self.bif_acc.as_mut() {
            acc.clear();
        }
        self.precip.fill(0.0);
        self.precip_yield.fill(0.0);
        self.precip_time.fill(0.0);
        Ok(())
    }

    /// `begin_suspended_period`：由悬沙体积与当前水量求浓度。
    fn begin_period(&mut self, rivsto: &[f64]) -> Result<()> {
        ensure!(
            self.sedsto.iter().all(|&v| v >= 0.0),
            "invalid suspended sediment solid volume at period boundary"
        );
        let ns = self.p.nsed;
        for i in 0..self.n() {
            for k in i * ns..(i + 1) * ns {
                self.sedcon[k] = if rivsto[i] > 0.0 {
                    self.sedsto[k] / rivsto[i]
                } else {
                    0.0
                };
            }
        }
        Ok(())
    }

    /// `commit_suspended_period`。
    fn commit_period(&mut self, rivsto: &[f64]) -> Result<()> {
        let ns = self.p.nsed;
        for i in 0..self.n() {
            let range = i * ns..(i + 1) * ns;
            if rivsto[i] > 0.0 {
                for k in range {
                    self.sedcon[k] = self.sedsto[k] / rivsto[i];
                }
            } else {
                ensure!(
                    self.sedsto[range.clone()].iter().all(|&v| v <= 0.0),
                    "dry cell retained suspended sediment after advection"
                );
                self.sedcon[range].fill(0.0);
            }
        }
        Ok(())
    }

    /// `write_sediment_history`：按 `sed_hist_acctime` 平均（零时长时全 0）的 unitcat 量。
    pub fn history_fields(&self) -> Vec<(String, String, String, Vec<f64>)> {
        let ns = self.p.nsed;
        let t = self.hist_acctime;
        let mean = |values: &[f64], s: usize| -> Vec<f64> {
            (0..self.n())
                .map(|i| if t > 0.0 { values[i * ns + s] / t } else { 0.0 })
                .collect()
        };
        let mut out = Vec::new();
        for (prefix, long, units, values) in [
            (
                "f_sedcon_",
                "suspended sediment concentration, size class ",
                "m^3/m^3",
                &self.a_sedcon,
            ),
            (
                "f_sedout_",
                "suspended sediment flux, size class ",
                "m^3/s",
                &self.a_sedout,
            ),
            (
                "f_bedout_",
                "bedload solid-volume flux, size class ",
                "m^3/s",
                &self.a_bedout,
            ),
            (
                "f_sedinp_",
                "sediment erosion input, size class ",
                "m^3/s",
                &self.a_sedinp,
            ),
            (
                "f_netflw_",
                "net bed-water exchange flux (incl. shallow deposit), size class ",
                "m^3/s",
                &self.a_netflw,
            ),
            (
                "f_layer_",
                "active layer storage, size class ",
                "m^3",
                &self.a_layer,
            ),
        ] {
            for s in 0..ns {
                out.push((
                    format!("{prefix}{}", s + 1),
                    format!("{long}{}", s + 1),
                    units.to_owned(),
                    mean(values, s),
                ));
            }
        }
        out.push((
            "f_shearvel".to_owned(),
            "shear velocity".to_owned(),
            "m/s".to_owned(),
            self.a_shearvel
                .iter()
                .map(|&v| if t > 0.0 { v / t } else { 0.0 })
                .collect(),
        ));
        out
    }

    /// `flush_sediment_history`。
    pub fn flush_history(&mut self) {
        for values in [
            &mut self.a_sedcon,
            &mut self.a_sedout,
            &mut self.a_bedout,
            &mut self.a_sedinp,
            &mut self.a_netflw,
            &mut self.a_layer,
            &mut self.a_shearvel,
        ] {
            values.fill(0.0);
        }
        self.hist_acctime = 0.0;
    }

    /// 一个粒径类的一行（`[i*nsed + s]` → 逐单元流域）。
    fn row(values: &[f64], width: usize, s: usize) -> Vec<f64> {
        values.iter().skip(s).step_by(width).copied().collect()
    }

    fn set_row(values: &mut [f64], width: usize, s: usize, row: &[f64]) {
        for (i, &v) in row.iter().enumerate() {
            values[i * width + s] = v;
        }
    }

    /// `write_sediment_restart`（不开堤防/分汊：schema 2）：追加到河道续跑文件。
    pub fn write_restart(
        &self,
        path: &Path,
        network: &RiverNetwork,
        compression_level: u8,
    ) -> Result<()> {
        self.validate_checkpoint("write")?;
        let n = self.n();
        let (ns, nl) = (self.p.nsed, self.p.totlyrnum);
        let mut file =
            netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
        let mut vector = |name: &str, values: &[f64]| -> Result<()> {
            let mut variable = match file.variable_mut(name) {
                Some(variable) => variable,
                None => {
                    let mut variable = file.add_variable::<f64>(name, &["ucatch"])?;
                    if compression_level > 0 {
                        variable.set_compression(i32::from(compression_level), false)?;
                    }
                    variable.put_attribute("missing_value", colm_core::MISSING)?;
                    variable
                }
            };
            variable.put_values(values, ..)?;
            Ok(())
        };
        let scalar = |value: f64| vec![value; n];
        let levee_on = self.levee.is_some();
        let bif_on = self.bif_acc.is_some();
        if let Some(acc) = self.bif_acc.as_ref() {
            ensure!(
                [
                    &acc.forward,
                    &acc.reverse,
                    &acc.forward_time,
                    &acc.reverse_time
                ]
                .iter()
                .all(|values| values.iter().all(|&v| v == 0.0)),
                "sediment BIF gross flux is nonzero at restart boundary"
            );
        }
        let schema = if levee_on || bif_on {
            SCHEMA_JOINT
        } else {
            SCHEMA_PLAIN
        };
        vector("sed_restart_schema_meta", &scalar(schema))?;
        vector("sed_restart_complete_meta", &scalar(0.0))?;
        if levee_on || bif_on {
            vector("sed_use_levee_meta", &scalar(f64::from(u8::from(levee_on))))?;
            vector("sed_use_bif_meta", &scalar(f64::from(u8::from(bif_on))))?;
        }
        let p = &self.p;
        for (name, value) in [
            ("sed_n_meta", ns as f64),
            ("sed_totlyrnum_meta", nl as f64),
            ("sed_nlfp_meta", p.nlfp as f64),
            ("sed_lambda_meta", p.lambda),
            ("sed_lyrdph_meta", p.lyrdph),
            ("sed_psedd_meta", p.psedd),
            ("sed_pwatd_meta", p.pwatd),
            ("sed_viskin_meta", p.viskin),
            ("sed_vonkar_meta", p.vonkar),
            ("sed_pset_meta", p.pset),
            ("sed_ignore_dph_meta", p.ignore_dph),
            ("sed_cfl_adv_meta", p.cfl_adv),
            ("sed_dt_max_meta", p.dt_max),
            ("sed_bed_depth_meta", p.bed_depth),
            ("sed_pyld_meta", p.pyld),
            ("sed_pyldc_meta", p.pyldc),
            ("sed_pyldpc_meta", p.pyldpc),
            ("sed_dsylunit_meta", p.dsylunit),
            ("sed_max_conc_meta", MAX_SED_CONC),
            ("sed_bedload_coeff_meta", SED_BEDLOAD_COEFF),
            ("sed_precip_threshold_meta", SED_PRECIP_THRESHOLD_MM_DAY),
            ("sed_exch_shear_min_meta", EXCH_SHEARVEL_MIN),
            ("sed_exch_shear_blend_meta", EXCH_SHEARVEL_BLEND),
            ("sed_exch_zd_max_meta", EXCH_ZD_MAX),
        ] {
            vector(name, &scalar(value))?;
        }
        for s in 0..ns {
            vector(&format!("sed_diam_meta_{}", s + 1), &scalar(p.diam[s]))?;
            vector(&format!("sed_setvel_meta_{}", s + 1), &scalar(p.setvel[s]))?;
            vector(
                &format!("sed_frc_meta_{}", s + 1),
                &Self::row(&self.frc, ns, s),
            )?;
        }
        for l in 0..p.nlfp {
            vector(
                &format!("sed_slope_meta_{}", l + 1),
                &Self::row(&self.slope, p.nlfp, l),
            )?;
        }
        vector("sed_rivwth_meta", &network.rivwth)?;
        vector("sed_rivlen_meta", &network.rivlen)?;
        for s in 0..ns {
            vector(
                &format!("sedcon_{}", s + 1),
                &Self::row(&self.sedcon, ns, s),
            )?;
        }
        for s in 0..ns {
            vector(
                &format!("sedsto_{}", s + 1),
                &Self::row(&self.sedsto, ns, s),
            )?;
        }
        if levee_on {
            for s in 0..ns {
                vector(
                    &format!("sedsto_protected_{}", s + 1),
                    &Self::row(&self.sedsto_protected, ns, s),
                )?;
                vector(
                    &format!("sedbed_protected_{}", s + 1),
                    &Self::row(&self.sedbed_protected, ns, s),
                )?;
            }
        }
        for s in 0..ns {
            vector(&format!("layer_{}", s + 1), &Self::row(&self.layer, ns, s))?;
        }
        for s in 0..ns {
            for l in 0..nl {
                let values = (0..n)
                    .map(|i| self.seddep[(i * nl + l) * ns + s])
                    .collect::<Vec<_>>();
                vector(&format!("seddep_{}_{}", s + 1, l + 1), &values)?;
            }
        }
        let acc = |f: fn(&WaterAcc) -> f64| self.acc.iter().map(f).collect::<Vec<_>>();
        vector("sed_acc_time", &acc(|a| a.time))?;
        vector("sed_acc_v2", &acc(|a| a.v2))?;
        vector("sed_acc_wdsrf", &acc(|a| a.wdsrf))?;
        vector("sed_acc_rivsto", &acc(|a| a.rivsto))?;
        vector("sed_acc_rivsto_start", &acc(|a| a.rivsto_start))?;
        vector("sed_acc_rivsto_end", &acc(|a| a.rivsto_end))?;
        if levee_on {
            vector("sed_acc_protected_start", &acc(|a| a.protected_start))?;
            vector(
                "sed_acc_pre_repartition_start",
                &acc(|a| f64::from(u8::from(a.pre_repartition_start))),
            )?;
            vector("sed_acc_protected_end", &acc(|a| a.protected_end))?;
            vector("sed_acc_to_protected", &acc(|a| a.to_protected))?;
            vector("sed_acc_from_protected", &acc(|a| a.from_protected))?;
        }
        vector("sed_acc_rivout", &acc(|a| a.rivout))?;
        vector("sed_acc_abs_rivout", &acc(|a| a.abs_rivout))?;
        vector("sed_acc_floodarea", &acc(|a| a.floodarea))?;
        if levee_on {
            vector("sed_acc_protected_area", &acc(|a| a.protected_area))?;
        }
        vector("sed_precip", &self.precip)?;
        vector("sed_precip_yield", &self.precip_yield)?;
        vector("sed_precip_time_vec", &self.precip_time)?;
        for s in 0..ns {
            vector(
                &format!("a_sedcon_{}", s + 1),
                &Self::row(&self.a_sedcon, ns, s),
            )?;
            vector(
                &format!("a_sedout_{}", s + 1),
                &Self::row(&self.a_sedout, ns, s),
            )?;
            vector(
                &format!("a_bedout_{}", s + 1),
                &Self::row(&self.a_bedout, ns, s),
            )?;
            vector(
                &format!("a_sedinp_{}", s + 1),
                &Self::row(&self.a_sedinp, ns, s),
            )?;
            vector(
                &format!("a_netflw_{}", s + 1),
                &Self::row(&self.a_netflw, ns, s),
            )?;
            vector(
                &format!("a_layer_{}", s + 1),
                &Self::row(&self.a_layer, ns, s),
            )?;
        }
        vector("a_shearvel", &self.a_shearvel)?;
        vector("sed_hist_acctime_vec", &scalar(self.hist_acctime))?;
        vector("sed_restart_complete_meta", &scalar(1.0))?;
        file.close()?;
        Ok(())
    }

    /// `validate_sediment_checkpoint_state`：状态非负有限才写/读回续跑。
    fn validate_checkpoint(&self, context: &str) -> Result<()> {
        let ok = |values: &[f64]| values.iter().all(|v| v.is_finite() && *v >= 0.0);
        let finite = |values: &[f64]| values.iter().all(|v| v.is_finite());
        let accs = self.acc.iter().all(|a| {
            ok(&[
                a.time,
                a.v2,
                a.wdsrf,
                a.rivsto,
                a.rivsto_start,
                a.rivsto_end,
                a.protected_start,
                a.protected_end,
                a.to_protected,
                a.from_protected,
                a.abs_rivout,
                a.floodarea,
                a.protected_area,
            ]) && a.rivout.is_finite()
                && !(a.time == 0.0 && a.rivsto_end != 0.0)
                && !(a.time == 0.0 && a.rivsto_start != 0.0 && !a.pre_repartition_start)
                && !(a.pre_repartition_start && a.time > 0.0)
                && a.protected_start <= a.rivsto_start + SED_BALANCE_ABS_TOL
        });
        let bif = self.bif_acc.as_ref().is_none_or(|b| {
            ok(&b.forward) && ok(&b.reverse) && ok(&b.forward_time) && ok(&b.reverse_time)
        });
        let precip = (0..self.n()).all(|i| {
            ok(&[self.precip[i], self.precip_yield[i], self.precip_time[i]])
                && !(self.precip_time[i] == 0.0
                    && (self.precip[i] != 0.0 || self.precip_yield[i] != 0.0))
        });
        ensure!(
            ok(&self.sedcon)
                && ok(&self.sedsto)
                && ok(&self.sedsto_protected)
                && ok(&self.sedbed_protected)
                && ok(&self.layer)
                && ok(&self.seddep)
                && accs
                && bif
                && precip
                && ok(&self.a_sedcon)
                && finite(&self.a_sedout)
                && finite(&self.a_bedout)
                && ok(&self.a_sedinp)
                && finite(&self.a_netflw)
                && ok(&self.a_layer)
                && ok(&self.a_shearvel)
                && self.hist_acctime.is_finite()
                && self.hist_acctime >= 0.0,
            "ERROR: sediment checkpoint {context} contains non-finite or physically negative state."
        );
        Ok(())
    }

    /// `read_sediment_restart`：文件里没有任何泥沙量时保持冷启动并返回假。
    pub fn read_restart(&mut self, path: &Path, network: &RiverNetwork) -> Result<bool> {
        let file = match netcdf::open(path) {
            Ok(file) => file,
            Err(_) => return Ok(false),
        };
        let has = |name: &str| file.variable(name).is_some();
        let has_schema = has("sed_restart_schema_meta");
        let has_complete = has("sed_restart_complete_meta");
        let has_canonical = has("sedsto_1");
        let any = has_schema
            || has_complete
            || has_canonical
            || [
                "sed_n_meta",
                "sedcon_1",
                "layer_1",
                "seddep_1_1",
                "sed_acc_time",
                "a_sedcon_1",
            ]
            .iter()
            .any(|name| has(name));
        let joint = self.levee.is_some() || self.bif_acc.is_some();
        if !any {
            ensure!(
                !(joint && has("trc_river_restart_complete")),
                "ERROR: levee/bifurcation sediment restart requires a complete sediment transaction."
            );
            return Ok(false);
        }
        ensure!(
            has_schema || !(has_complete || has_canonical),
            "ERROR: partial sediment transaction has canonical fields but no schema marker."
        );
        let n = self.n();
        let (ns, nl) = (self.p.nsed, self.p.totlyrnum);
        let read = |name: &str| -> Result<Vec<f64>> {
            let values = file
                .variable(name)
                .with_context(|| format!("missing required sediment restart variable {name}"))?
                .get_values::<f64, _>(..)?;
            ensure!(
                values.len() == n,
                "sediment restart {name} has the wrong length"
            );
            Ok(values)
        };
        let matches =
            |a: f64, b: f64| a == b || (a - b).abs() <= 1.0e-12 * a.abs().max(b.abs()).max(1.0);
        let mut bad = false;
        let schema = if has_schema {
            let values = read("sed_restart_schema_meta")?;
            let v = values.first().copied().unwrap_or(0.0);
            if v == 1.0 || v == 2.0 || v == 5.0 {
                v
            } else {
                0.0
            }
        } else {
            0.0
        };
        if has_schema && schema == 0.0 {
            bad = true;
        }
        if joint && schema != SCHEMA_JOINT {
            bad = true;
        }
        if !joint && schema > SCHEMA_PLAIN {
            bad = true;
        }
        let p = &self.p;
        if has_schema {
            let mut check = |name: &str, expected: f64| -> Result<()> {
                if !read(name)?.iter().all(|&v| matches(v, expected)) {
                    bad = true;
                }
                Ok(())
            };
            if schema == SCHEMA_JOINT {
                check(
                    "sed_use_levee_meta",
                    f64::from(u8::from(self.levee.is_some())),
                )?;
                check(
                    "sed_use_bif_meta",
                    f64::from(u8::from(self.bif_acc.is_some())),
                )?;
            }
            check("sed_restart_schema_meta", schema)?;
            check("sed_restart_complete_meta", 1.0)?;
            check("sed_nlfp_meta", p.nlfp as f64)?;
            for (name, value) in [
                ("sed_lambda_meta", p.lambda),
                ("sed_lyrdph_meta", p.lyrdph),
                ("sed_psedd_meta", p.psedd),
                ("sed_pwatd_meta", p.pwatd),
                ("sed_viskin_meta", p.viskin),
                ("sed_vonkar_meta", p.vonkar),
                ("sed_pset_meta", p.pset),
                ("sed_ignore_dph_meta", p.ignore_dph),
                ("sed_cfl_adv_meta", p.cfl_adv),
                ("sed_dt_max_meta", p.dt_max),
                ("sed_bed_depth_meta", p.bed_depth),
                ("sed_pyld_meta", p.pyld),
                ("sed_pyldc_meta", p.pyldc),
                ("sed_pyldpc_meta", p.pyldpc),
                ("sed_dsylunit_meta", p.dsylunit),
                ("sed_max_conc_meta", MAX_SED_CONC),
                ("sed_bedload_coeff_meta", SED_BEDLOAD_COEFF),
                ("sed_precip_threshold_meta", SED_PRECIP_THRESHOLD_MM_DAY),
                ("sed_exch_shear_min_meta", EXCH_SHEARVEL_MIN),
                ("sed_exch_shear_blend_meta", EXCH_SHEARVEL_BLEND),
                ("sed_exch_zd_max_meta", EXCH_ZD_MAX),
            ] {
                check(name, value)?;
            }
            for s in 0..ns {
                check(&format!("sed_diam_meta_{}", s + 1), p.diam[s])?;
                check(&format!("sed_setvel_meta_{}", s + 1), p.setvel[s])?;
            }
        }
        if !read("sed_n_meta")?
            .iter()
            .all(|&v| v.round() as usize == ns)
        {
            bad = true;
        }
        if !read("sed_totlyrnum_meta")?
            .iter()
            .all(|&v| v.round() as usize == nl)
        {
            bad = true;
        }
        if has_schema {
            let same = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(&x, &y)| matches(x, y));
            for s in 0..ns {
                if !same(
                    &read(&format!("sed_frc_meta_{}", s + 1))?,
                    &Self::row(&self.frc, ns, s),
                ) {
                    bad = true;
                }
            }
            for l in 0..p.nlfp {
                if !same(
                    &read(&format!("sed_slope_meta_{}", l + 1))?,
                    &Self::row(&self.slope, p.nlfp, l),
                ) {
                    bad = true;
                }
            }
            if !same(&read("sed_rivwth_meta")?, &network.rivwth)
                || !same(&read("sed_rivlen_meta")?, &network.rivlen)
            {
                bad = true;
            }
        }
        ensure!(
            !bad,
            "ERROR: sediment restart schema/configuration does not match the active sediment model."
        );
        for s in 0..ns {
            let row = read(&format!("sedcon_{}", s + 1))?;
            Self::set_row(&mut self.sedcon, ns, s, &row);
        }
        if has_schema {
            for s in 0..ns {
                let row = read(&format!("sedsto_{}", s + 1))?;
                Self::set_row(&mut self.sedsto, ns, s, &row);
            }
        } else {
            ensure!(
                self.sedcon.iter().all(|&v| v == 0.0),
                "ERROR: legacy sediment restart has nonzero concentration but no exact suspended mass."
            );
            self.sedsto.fill(0.0);
        }
        if self.levee.is_some() {
            for s in 0..ns {
                let row = read(&format!("sedsto_protected_{}", s + 1))?;
                Self::set_row(&mut self.sedsto_protected, ns, s, &row);
                let row = read(&format!("sedbed_protected_{}", s + 1))?;
                Self::set_row(&mut self.sedbed_protected, ns, s, &row);
            }
        }
        for s in 0..ns {
            let row = read(&format!("layer_{}", s + 1))?;
            Self::set_row(&mut self.layer, ns, s, &row);
        }
        for s in 0..ns {
            for l in 0..nl {
                let row = read(&format!("seddep_{}_{}", s + 1, l + 1))?;
                for i in 0..n {
                    self.seddep[(i * nl + l) * ns + s] = row[i];
                }
            }
        }
        let time = read("sed_acc_time")?;
        let v2 = read("sed_acc_v2")?;
        let wdsrf = read("sed_acc_wdsrf")?;
        let rivsto = read("sed_acc_rivsto")?;
        // schema 1 与更早的文件只有水量的时间平均，没有端点；只能在干净的汇流边界迁移。
        let (start, end) = if schema >= 2.0 {
            (read("sed_acc_rivsto_start")?, read("sed_acc_rivsto_end")?)
        } else {
            ensure!(
                time.iter().all(|&t| t <= 0.0),
                "ERROR: sediment legacy restart has an unfinished routing window without carrier endpoints."
            );
            (vec![0.0; n], vec![0.0; n])
        };
        let rivout = read("sed_acc_rivout")?;
        let abs_rivout = read("sed_acc_abs_rivout")?;
        let floodarea = read("sed_acc_floodarea")?;
        for i in 0..n {
            self.acc[i] = WaterAcc {
                time: time[i],
                v2: v2[i],
                wdsrf: wdsrf[i],
                rivsto: rivsto[i],
                rivsto_start: start[i],
                rivsto_end: end[i],
                rivout: rivout[i],
                abs_rivout: abs_rivout[i],
                floodarea: floodarea[i],
                ..Default::default()
            };
        }
        if self.levee.is_some() {
            let area = read("sed_acc_protected_area")?;
            let pstart = read("sed_acc_protected_start")?;
            let flag = read("sed_acc_pre_repartition_start")?;
            ensure!(
                flag.iter().all(|&v| v == 0.0 || v == 1.0),
                "sediment restart invalid pre-repartition flag"
            );
            let pend = read("sed_acc_protected_end")?;
            let to = read("sed_acc_to_protected")?;
            let from = read("sed_acc_from_protected")?;
            for i in 0..n {
                let acc = &mut self.acc[i];
                acc.protected_area = area[i];
                acc.protected_start = pstart[i];
                acc.pre_repartition_start = flag[i] == 1.0;
                acc.protected_end = pend[i];
                acc.to_protected = to[i];
                acc.from_protected = from[i];
            }
        }
        // 续跑只写在驱动边界，分汊的路径毛水量在每次汇流里用完，不带进续跑。
        if let Some(acc) = self.bif_acc.as_mut() {
            acc.clear();
        }
        self.precip = read("sed_precip")?;
        self.precip_yield = read("sed_precip_yield")?;
        self.precip_time = read("sed_precip_time_vec")?;
        for s in 0..ns {
            for (name, values) in [
                ("a_sedcon_", &mut self.a_sedcon),
                ("a_sedout_", &mut self.a_sedout),
                ("a_bedout_", &mut self.a_bedout),
                ("a_sedinp_", &mut self.a_sedinp),
                ("a_netflw_", &mut self.a_netflw),
                ("a_layer_", &mut self.a_layer),
            ] {
                let row = read(&format!("{name}{}", s + 1))?;
                Self::set_row(values, ns, s, &row);
            }
        }
        self.a_shearvel = read("a_shearvel")?;
        self.hist_acctime = read("sed_hist_acctime_vec")?
            .first()
            .copied()
            .unwrap_or(0.0);
        self.validate_checkpoint("read")?;
        Ok(true)
    }
}

/// `log10(19._r8)`（编译期常量折叠）。
const LOG10_19: f64 = 1.278_753_600_952_828_9;

#[cfg(test)]
#[path = "sediment_tests.rs"]
mod tests;
