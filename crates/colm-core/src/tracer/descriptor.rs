//! 示踪物描述符与注册（`MOD_Tracer_Defs:tracer_defs_init` 及其辅助例程）。
//!
//! 上游把描述符放在全局表 `tracers(:)` 里，按 `itrc` 查询；这里是显式的 [`TracerSet`]。
//! namelist 的 CSV 键与逐示踪物参数文件（`&nl_colm_tracer_parameter`）由调用方读成字符串/
//! 数值后交给 [`TracerSet::build`]，本模块不碰文件。

use anyhow::{bail, ensure, Result};

/// `trc_tiny`：只作算术非零保护。
pub const TRC_TINY: f64 = 1.0e-30;
/// `trc_water_min_for_ratio`：由水量求比值时的物理下限 [kg m-2]。
pub const TRC_WATER_MIN_FOR_RATIO: f64 = 1.0e-12;
/// `TRACER_DESCRIPTOR_IDENTITY_WIDTH`。
pub const DESCRIPTOR_IDENTITY_WIDTH: usize = 384;
/// 上游 `huge(1.0_r8)`：参数文件里"未给"的哨兵，也是"不限溶解度"。
const HUGE: f64 = f64::MAX;

/// `FAMILY_*`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracerFamily {
    Unresolved = 0,
    Isotope = 1,
    Solute = 2,
    Particle = 3,
    Gas = 4,
}

/// `STATE_OWNER_*`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateOwner {
    Unknown = 0,
    GenericWater = 1,
    Provider = 2,
}

/// `REACTION_*`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionMode {
    None = 0,
    FirstOrder = 1,
    Provider = 2,
}

/// `tracer_info_type`。
#[derive(Debug, Clone, PartialEq)]
pub struct TracerDescriptor {
    pub name: String,
    pub category: String,
    pub family: TracerFamily,
    pub state_owner: StateOwner,
    pub reaction_mode: ReactionMode,
    pub unit_kind: String,
    pub mol_weight: f64,
    pub ref_ratio: f64,
    pub init_delta: f64,
    pub init_conc: f64,
    pub precip_default_conc: f64,
    pub vapor_default_conc: f64,
    pub max_dissolved_conc: f64,
    pub reactive_decay_rate: f64,
    pub charge: i32,
}

/// `&nl_colm_tracer_parameter` 的 `DEF_TRACER%*`：只放文件里写了的字段。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TracerParameterOverrides {
    pub unit_kind: Option<String>,
    pub mol_weight: Option<f64>,
    pub ref_ratio: Option<f64>,
    pub init_delta: Option<f64>,
    pub init_conc: Option<f64>,
    pub precip_default_conc: Option<f64>,
    pub vapor_default_conc: Option<f64>,
    pub max_dissolved_conc: Option<f64>,
    pub reactive_decay_rate: Option<f64>,
    pub charge: Option<i32>,
}

/// 构建描述符所需的 namelist 键（`DEF_TRACER_*`，均为原始字符串）。
#[derive(Debug, Clone, Default)]
pub struct TracerNamelist {
    pub num: i64,
    pub names: String,
    pub types: String,
    pub mrat: String,
    pub ref_ratio: String,
    pub init_delta: String,
    pub reactive_decay_rate: String,
    pub param_files: String,
    pub use_bgc: bool,
    pub variably_saturated_flow: bool,
    pub aquifer_mixing_water_mm: f64,
}

/// 注册好的示踪物表（上游的 `ntracers` 与 `tracers(:)`）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TracerSet {
    pub tracers: Vec<TracerDescriptor>,
}

impl TracerSet {
    /// `tracer_defs_init`（开关已打开时）。`read_parameters(path)` 读一个参数文件里的
    /// `&nl_colm_tracer_parameter`：文件不存在报错，没有这一组返回 `None`。
    pub fn build(
        namelist: &TracerNamelist,
        mut read_parameters: impl FnMut(&str) -> Result<Option<TracerParameterOverrides>>,
    ) -> Result<Self> {
        ensure!(
            namelist.num >= 0,
            "tracer_defs_init: DEF_TRACER_NUM must be >= 0, got {}",
            namelist.num
        );
        ensure!(
            namelist.num <= 1000,
            "tracer_defs_init: DEF_TRACER_NUM={} is unreasonably large",
            namelist.num
        );
        let n = usize::try_from(namelist.num)?;
        if n == 0 {
            return Ok(Self::default());
        }
        let names = unique_names(&parse_csv(&namelist.names), n);
        let types = parse_csv(&namelist.types);
        ensure!(
            types.len() == n,
            "tracer_defs_init: DEF_TRACER_TYPES has {} entries but DEF_TRACER_NUM={n}; every \
             tracer needs an explicit type",
            types.len()
        );
        let mol_weight = parse_csv_real(&namelist.mrat, n, 18.0);
        let ref_ratio = parse_csv_real(&namelist.ref_ratio, n, 1.0);
        let init_delta = parse_csv_real(&namelist.init_delta, n, 0.0);
        let decay = parse_csv_real(&namelist.reactive_decay_rate, n, 0.0);
        let mut tracers = Vec::with_capacity(n);
        for i in 0..n {
            let mut tracer = TracerDescriptor {
                name: names[i].clone(),
                category: canonical_category(&types[i]),
                family: TracerFamily::Unresolved,
                state_owner: StateOwner::Unknown,
                reaction_mode: ReactionMode::None,
                unit_kind: String::new(),
                mol_weight: mol_weight[i],
                ref_ratio: ref_ratio[i],
                init_delta: init_delta[i],
                init_conc: init_delta[i],
                precip_default_conc: init_delta[i],
                vapor_default_conc: init_delta[i],
                max_dissolved_conc: HUGE,
                reactive_decay_rate: decay[i],
                charge: 0,
            };
            set_category_defaults(&mut tracer);
            tracers.push(tracer);
        }
        // `apply_tracer_param_files`。
        for i in 0..n {
            let Some(path) = param_file_for_index(&namelist.param_files, &tracers, i)? else {
                continue;
            };
            if let Some(overrides) = read_parameters(&path)? {
                apply_overrides(&mut tracers[i], &overrides);
            }
        }
        for tracer in &mut tracers {
            ensure!(
                category_supported(&tracer.category),
                "tracer_defs_init: unknown tracer category \"{}\" for {}",
                tracer.category,
                tracer.name
            );
            ensure!(
                tracer.reactive_decay_rate.is_finite(),
                "tracer_defs_init: non-finite reactive_decay_rate for {}",
                tracer.name
            );
            if tracer.reactive_decay_rate < 0.0 {
                tracer.reactive_decay_rate = 0.0;
            }
            derive_taxonomy(tracer);
            validate(tracer, namelist.use_bgc)?;
            if namelist.variably_saturated_flow && tracer.family == TracerFamily::Isotope {
                ensure!(
                    namelist.aquifer_mixing_water_mm.is_finite()
                        && namelist.aquifer_mixing_water_mm > 0.0,
                    "tracer_defs_init: VSF isotopes require explicit positive \
                     DEF_TRACER_AQUIFER_MIXING_WATER_MM"
                );
            }
        }
        // `register_tracer_provider`（`tracer_lifecycle_init` 里编进来的 provider）：注册时以 provider
        // 的声明覆盖 `reaction_mode`。CH4 声明 `REACTION_PROVIDER`
        // （`MOD_Tracer_Reactive_Methane.F90:102`），SEDIMENT 声明 `REACTION_NONE`。它进
        // 描述符指纹（history 旁车的 `trc_hist_descriptor`）。
        for tracer in &mut tracers {
            if matches!(
                tracer.name.trim().to_ascii_uppercase().as_str(),
                "CH4" | "METHANE"
            ) && tracer.family == TracerFamily::Gas
            {
                tracer.reaction_mode = ReactionMode::Provider;
            }
        }
        Ok(Self { tracers })
    }

    pub fn len(&self) -> usize {
        self.tracers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracers.is_empty()
    }

    /// 走通用陆面水输运的示踪物（`tracer_uses_land_water_transport`）的序号。
    pub fn transport_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.tracers
            .iter()
            .enumerate()
            .filter(|(_, tracer)| tracer.uses_land_water_transport())
            .map(|(index, _)| index)
    }

    /// `tracer_build_descriptor_identity(identity, transport_only=.true.)`：每个输运示踪物一行
    /// 384 个 ASCII 码（空格补齐）。
    pub fn descriptor_identity(&self) -> Vec<[i32; DESCRIPTOR_IDENTITY_WIDTH]> {
        self.tracers
            .iter()
            .filter(|tracer| tracer.uses_land_water_transport())
            .map(TracerDescriptor::identity)
            .collect()
    }

    /// `tracer_build_descriptor_identity(identity)`（不限输运示踪物）：history 旁车的
    /// `trc_hist_descriptor` 每个注册示踪物一行。
    pub fn descriptor_identity_all(&self) -> Vec<[i32; DESCRIPTOR_IDENTITY_WIDTH]> {
        self.tracers
            .iter()
            .map(TracerDescriptor::identity)
            .collect()
    }
}

impl TracerDescriptor {
    pub fn is_isotope(&self) -> bool {
        self.family == TracerFamily::Isotope
    }

    pub fn is_solute(&self) -> bool {
        self.family == TracerFamily::Solute
    }

    pub fn uses_land_water_transport(&self) -> bool {
        self.state_owner == StateOwner::GenericWater
    }

    /// `tracer_is_nonvolatile_solute`。
    pub fn is_nonvolatile_solute(&self) -> bool {
        self.is_solute() && self.uses_land_water_transport()
    }

    /// `tracer_has_dissolved_limit`。
    pub fn has_dissolved_limit(&self) -> bool {
        self.is_nonvolatile_solute() && self.max_dissolved_conc < HUGE
    }

    /// `tracer_can_use_fixed_signature`。
    pub fn can_use_fixed_signature(&self) -> bool {
        self.is_isotope()
    }

    /// `tracer_init_water_ratio`。
    pub fn init_water_ratio(&self) -> f64 {
        if self.is_isotope() {
            delta_to_ratio(self.init_delta, self.ref_ratio)
        } else {
            self.init_conc
        }
    }

    /// `tracer_precip_default_ratio`。
    pub fn precip_default_ratio(&self) -> f64 {
        if self.is_isotope() {
            delta_to_ratio(self.init_delta, self.ref_ratio)
        } else {
            self.precip_default_conc
        }
    }

    /// `tracer_vapor_default_ratio`。
    pub fn vapor_default_ratio(&self) -> f64 {
        if self.is_isotope() {
            delta_to_ratio(self.init_delta, self.ref_ratio)
        } else {
            self.vapor_default_conc
        }
    }

    /// `tracer_reactive_decay_fraction`：`1 - exp(-min(k*dt, 700))`。
    pub fn reactive_decay_fraction(&self, deltim: f64) -> f64 {
        if self.reaction_mode != ReactionMode::FirstOrder
            || deltim <= 0.0
            || self.reactive_decay_rate <= 0.0
        {
            return 0.0;
        }
        let kdt = (self.reactive_decay_rate * deltim).min(700.0);
        1.0 - (-kdt).exp()
    }

    /// `tracer_equilibrate_dissolved`：溶解量不超过 `max_dissolved_conc * water`，多余的进固相；
    /// 固相在容量内可重新溶解。负值（含水层亏欠记账）不动。
    pub fn equilibrate_dissolved(&self, water_mass: f64, dissolved: &mut f64, solid: &mut f64) {
        if !self.has_dissolved_limit() || *dissolved < 0.0 || *solid < 0.0 {
            return;
        }
        let total = *dissolved + *solid;
        let capacity = if water_mass > TRC_WATER_MIN_FOR_RATIO {
            self.max_dissolved_conc * water_mass
        } else {
            0.0
        };
        *dissolved = total.min(capacity);
        *solid = total - *dissolved;
    }

    /// 一行描述符指纹：`'(A,"|",A,"|",A,"|",4(I0,"|"),8(ES24.16E3,"|"))'` 写进
    /// `character(len=384)`，逐字符取 `iachar`。
    fn identity(&self) -> [i32; DESCRIPTOR_IDENTITY_WIDTH] {
        let mut text = format!(
            "{}|{}|{}|{}|{}|{}|{}|",
            self.name,
            self.category,
            self.unit_kind,
            self.family as i32,
            self.state_owner as i32,
            self.reaction_mode as i32,
            self.charge
        );
        for value in [
            self.mol_weight,
            self.ref_ratio,
            self.init_delta,
            self.init_conc,
            self.precip_default_conc,
            self.vapor_default_conc,
            self.max_dissolved_conc,
            self.reactive_decay_rate,
        ] {
            text.push_str(&format_es24_16e3(value));
            text.push('|');
        }
        let mut identity = [i32::from(b' '); DESCRIPTOR_IDENTITY_WIDTH];
        for (slot, byte) in identity.iter_mut().zip(text.bytes()) {
            *slot = i32::from(byte);
        }
        identity
    }
}

/// `delta_to_R`：`ref_ratio * (1 + delta/1000)`。
pub fn delta_to_ratio(delta: f64, ref_ratio: f64) -> f64 {
    ref_ratio * (1.0 + delta / 1000.0)
}

/// gfortran 的 `ES24.16E3`：宽 24，一位整数、16 位小数、`E±ddd`，右对齐。
pub fn format_es24_16e3(value: f64) -> String {
    let formatted = format!("{value:.16e}");
    let (mantissa, exponent) = formatted
        .split_once('e')
        .expect("Rust scientific formatting always has an exponent");
    let exponent: i32 = exponent.parse().expect("integer exponent");
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{:>24}", format!("{mantissa}E{sign}{:03}", exponent.abs()))
}

/// `parse_csv`：按逗号切分，逐项去首尾空白；末尾无逗号的残段也算一项，全空串得零项。
pub fn parse_csv(raw: &str) -> Vec<String> {
    let text = raw.trim();
    if text.is_empty() {
        return Vec::new();
    }
    let mut tokens: Vec<String> = text
        .split(',')
        .map(|token| token.trim().to_owned())
        .collect();
    // 上游：最后一个逗号之后没有字符时不再补一项（`j <= slen` 不成立）。
    if text.ends_with(',') {
        tokens.pop();
    }
    tokens
}

/// `parse_csv_real`：不足或不是数的项用默认值。
fn parse_csv_real(raw: &str, n: usize, default: f64) -> Vec<f64> {
    let tokens = parse_csv(raw);
    (0..n)
        .map(|i| {
            tokens
                .get(i)
                .and_then(|token| parse_fortran_real(token))
                .unwrap_or(default)
        })
        .collect()
}

/// 列表式读入一个实数（接受 `d`/`D` 指数）。
fn parse_fortran_real(token: &str) -> Option<f64> {
    token.trim().replace(['d', 'D'], "e").parse().ok()
}

/// `sanitize_ncname`：只留字母数字、下划线与句点，最多 32 个；全被删掉时为 `unnamed`。
fn sanitize_ncname(raw: &str) -> String {
    let clean: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .take(32)
        .collect();
    if clean.is_empty() {
        "unnamed".to_owned()
    } else {
        clean
    }
}

/// 名字：清洗、缺的补 `tracer_N`，再按 `_<序号>` 去重（与已有名字大小写不敏感比较）。
fn unique_names(tokens: &[String], n: usize) -> Vec<String> {
    let mut names: Vec<String> = (0..n)
        .map(|i| match tokens.get(i) {
            Some(token) => sanitize_ncname(token),
            None => format!("tracer_{}", i + 1),
        })
        .collect();
    for i in 0..n {
        let base = names[i].clone();
        let mut suffix_index = i + 1;
        while names[..i].iter().any(|prior| param_equal(prior, &names[i])) {
            let suffix = format!("_{suffix_index}");
            suffix_index += 1;
            let max_base = 32usize.saturating_sub(suffix.len()).max(1);
            let stem: String = base.chars().take(max_base).collect();
            names[i] = format!("{stem}{suffix}");
        }
    }
    names
}

fn param_equal(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// `canonical_tracer_category`：小写，`conservative` 归为 `solute`。
fn canonical_category(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    if lower == "conservative" {
        "solute".to_owned()
    } else {
        lower
    }
}

fn category_supported(category: &str) -> bool {
    matches!(
        category,
        "isotope" | "solute" | "particle" | "gas" | "reactive"
    )
}

/// `set_tracer_category_defaults`。
fn set_category_defaults(tracer: &mut TracerDescriptor) {
    tracer.unit_kind = "tracer_per_water".to_owned();
    tracer.charge = 0;
    tracer.max_dissolved_conc = HUGE;
    tracer.family = TracerFamily::Unresolved;
    tracer.state_owner = StateOwner::Unknown;
    tracer.reaction_mode = ReactionMode::None;
    match tracer.category.as_str() {
        "isotope" => {
            tracer.unit_kind = "ratio".to_owned();
            tracer.family = TracerFamily::Isotope;
            tracer.state_owner = StateOwner::GenericWater;
        }
        "solute" => {
            tracer.family = TracerFamily::Solute;
            tracer.state_owner = StateOwner::GenericWater;
        }
        "reactive" => tracer.state_owner = StateOwner::GenericWater,
        "particle" => {
            tracer.unit_kind = "volume_fraction".to_owned();
            tracer.family = TracerFamily::Particle;
            tracer.state_owner = StateOwner::Provider;
        }
        "gas" => {
            tracer.unit_kind = "species_owned".to_owned();
            tracer.family = TracerFamily::Gas;
            tracer.state_owner = StateOwner::Provider;
        }
        _ => {}
    }
}

/// `read_tracer_parameter_file` 的赋值部分：三个浓度先回落到（可能被改过的）`init_delta`，
/// 文件里写了才覆盖。
fn apply_overrides(tracer: &mut TracerDescriptor, overrides: &TracerParameterOverrides) {
    if let Some(unit_kind) = &overrides.unit_kind {
        tracer.unit_kind = unit_kind.trim().to_ascii_lowercase();
    }
    if let Some(value) = overrides.mol_weight {
        tracer.mol_weight = value;
    }
    if let Some(value) = overrides.ref_ratio {
        tracer.ref_ratio = value;
    }
    if let Some(value) = overrides.init_delta {
        tracer.init_delta = value;
    }
    tracer.init_conc = overrides.init_conc.unwrap_or(tracer.init_delta);
    tracer.precip_default_conc = overrides.precip_default_conc.unwrap_or(tracer.init_delta);
    tracer.vapor_default_conc = overrides.vapor_default_conc.unwrap_or(tracer.init_delta);
    if let Some(value) = overrides.max_dissolved_conc {
        tracer.max_dissolved_conc = value;
    }
    if let Some(value) = overrides.reactive_decay_rate {
        tracer.reactive_decay_rate = value;
    }
    if let Some(value) = overrides.charge {
        tracer.charge = value;
    }
}

/// `tracer_param_file_for_index`：`,`/`;` 分隔；`key:path` 按名字匹配（大小写不敏感），
/// 无冒号的项按位置。第一个匹配生效；`null` 表示不读。
pub fn param_file_for_index(
    raw: &str,
    tracers: &[TracerDescriptor],
    index: usize,
) -> Result<Option<String>> {
    let list = raw.trim_end();
    if list.trim().is_empty() || list.trim().eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    let mut positional = 0usize;
    for entry in list.split([',', ';']) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // Windows 原生路径的盘符（`C:\`）属于按位置的路径，不是 `key:path` 映射（vendor 同步）。
        // `X:/...` 仍按映射解释：单字母的示踪物名可以这样写（见测试）。
        let drive_letter =
            matches!(entry.as_bytes(), [letter, b':', b'\\', ..] if letter.is_ascii_alphabetic());
        if let Some((key, value)) = entry.split_once(':').filter(|_| !drive_letter) {
            let (key, value) = (key.trim(), value.trim());
            if key.is_empty() || value.is_empty() {
                bail!("MOD_Tracer_Defs: empty tracer parameter file mapping entry: {entry}");
            }
            if param_equal(key, &tracers[index].name) {
                return Ok((!value.eq_ignore_ascii_case("null")).then(|| value.to_owned()));
            }
        } else {
            positional += 1;
            if positional == index + 1 {
                return Ok((!entry.eq_ignore_ascii_case("null")).then(|| entry.to_owned()));
            }
        }
    }
    Ok(None)
}

/// `derive_tracer_taxonomy`。
fn derive_taxonomy(tracer: &mut TracerDescriptor) {
    tracer.family = match tracer.category.as_str() {
        "isotope" => TracerFamily::Isotope,
        "solute" => TracerFamily::Solute,
        "particle" => TracerFamily::Particle,
        "gas" => TracerFamily::Gas,
        _ => TracerFamily::Unresolved,
    };
    tracer.state_owner = match tracer.family {
        TracerFamily::Isotope | TracerFamily::Solute => StateOwner::GenericWater,
        TracerFamily::Particle | TracerFamily::Gas => StateOwner::Provider,
        TracerFamily::Unresolved if tracer.category == "reactive" => {
            if tracer.unit_kind.trim() == "species_owned" {
                StateOwner::Provider
            } else {
                StateOwner::GenericWater
            }
        }
        TracerFamily::Unresolved => StateOwner::Unknown,
    };
    tracer.reaction_mode = if matches!(
        tracer.family,
        TracerFamily::Solute | TracerFamily::Unresolved
    ) && tracer.reactive_decay_rate > 0.0
    {
        ReactionMode::FirstOrder
    } else {
        ReactionMode::None
    };
}

/// `validate_tracer_descriptor`。
fn validate(tracer: &mut TracerDescriptor, use_bgc: bool) -> Result<()> {
    let name = tracer.name.clone();
    let fail = |field: &str, reason: &str| -> Result<()> {
        bail!("tracer_defs_init: tracer \"{name}\" field {field}: {reason}")
    };
    tracer.unit_kind = tracer.unit_kind.trim().to_ascii_lowercase();
    if tracer.family == TracerFamily::Unresolved && tracer.category != "reactive" {
        fail("family_id", "only legacy reactive may remain unresolved")?;
    }
    if !matches!(
        tracer.state_owner,
        StateOwner::GenericWater | StateOwner::Provider
    ) {
        fail(
            "state_owner",
            "must resolve before transport initialization",
        )?;
    }
    if matches!(
        tracer.family,
        TracerFamily::Isotope | TracerFamily::Particle
    ) && tracer.reaction_mode != ReactionMode::None
    {
        fail(
            "reaction_mode",
            "isotope and particle families do not use the generic reaction capability",
        )?;
    }
    if !matches!(
        tracer.unit_kind.as_str(),
        "ratio" | "tracer_per_water" | "mass_fraction" | "volume_fraction" | "species_owned"
    ) {
        fail("unit_kind", "unsupported unit convention")?;
    }
    for (field, value) in [
        ("mol_weight", tracer.mol_weight),
        ("ref_ratio", tracer.ref_ratio),
        ("init_delta", tracer.init_delta),
        ("init_conc", tracer.init_conc),
        ("precip_default_conc", tracer.precip_default_conc),
        ("vapor_default_conc", tracer.vapor_default_conc),
        ("max_dissolved_conc", tracer.max_dissolved_conc),
        ("reactive_decay_rate", tracer.reactive_decay_rate),
    ] {
        if !value.is_finite() {
            fail(field, "must be finite")?;
        }
    }
    if tracer.mol_weight < 0.0 {
        fail("mol_weight", "must be non-negative")?;
    }
    if tracer.ref_ratio <= 0.0 {
        fail("ref_ratio", "must be positive")?;
    }
    if tracer.is_isotope() {
        if tracer.init_delta < -1000.0 {
            fail("init_delta", "must be at least -1000 permil")?;
        }
    } else {
        for (field, value) in [
            ("init_conc", tracer.init_conc),
            ("precip_default_conc", tracer.precip_default_conc),
            ("vapor_default_conc", tracer.vapor_default_conc),
        ] {
            if value < 0.0 {
                fail(field, "must be non-negative")?;
            }
        }
    }
    if tracer.max_dissolved_conc <= 0.0 {
        fail("max_dissolved_conc", "must be positive")?;
    }
    if tracer.is_isotope() && tracer.unit_kind != "ratio" {
        fail("unit_kind", "isotope tracers require ratio")?;
    }
    if tracer.charge != 0 && !tracer.is_solute() {
        fail(
            "charge",
            "non-zero ionic charge is valid only for solute tracers",
        )?;
    }
    if (tracer.is_isotope() || tracer.is_solute()) && tracer.state_owner != StateOwner::GenericWater
    {
        fail(
            "state_owner",
            "isotope and solute tracers require generic land-water transport",
        )?;
    }
    if matches!(tracer.family, TracerFamily::Particle | TracerFamily::Gas)
        && tracer.state_owner != StateOwner::Provider
    {
        fail(
            "state_owner",
            "particle and gas tracers require provider-owned state",
        )?;
    }
    if tracer.max_dissolved_conc < HUGE && !tracer.is_solute() {
        fail("max_dissolved_conc", "is valid only for solute tracers")?;
    }
    if tracer.reactive_decay_rate > 0.0
        && !(tracer.is_solute() || tracer.family == TracerFamily::Unresolved)
    {
        fail(
            "reactive_decay_rate",
            "first-order decay is supported only by solute tracers",
        )?;
    }
    if tracer.state_owner == StateOwner::Provider
        && tracer.family != TracerFamily::Particle
        && tracer.unit_kind != "species_owned"
    {
        fail(
            "unit_kind",
            "species-owned state requires unit_kind=species_owned",
        )?;
    }
    if !use_bgc
        && (tracer.name.eq_ignore_ascii_case("CH4") || tracer.name.eq_ignore_ascii_case("METHANE"))
        && tracer.state_owner == StateOwner::Provider
    {
        fail(
            "unit_kind",
            "species-owned CH4 requires DEF_USE_BGC = .true.",
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "descriptor_tests.rs"]
mod tests;
