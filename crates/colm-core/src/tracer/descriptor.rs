//! 示踪物描述符与注册（`main/MATERIAL/MOD_MT_Defs.F90:tracer_defs_init` 及其辅助例程）。
//!
//! 上游把描述符放在全局表 `tracers(:)` 里，按 `itrc` 查询；这里是显式的 [`TracerSet`]。
//! 上游 7d2fca93 之后，条目由 family 开关的顺序决定（同位素 → 溶质 → 泥沙 → 甲烷），
//! 不再有 `DEF_TRACER_NUM/_NAMES/_TYPES` 那张 CSV；溶质的参数文件
//! （`&nl_colm_solute_parameter` 的 `DEF_SOLUTE%*`）由调用方读成
//! [`TracerParameterOverrides`] 后交给 [`TracerSet::build`]，本模块不碰文件。

use anyhow::{bail, ensure, Context, Result};

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

/// `&nl_colm_solute_parameter` 的 `DEF_SOLUTE%*`：只放文件里写了的字段。
///
/// 上游把旧的 `tracer_parameter_type` 换成了按 species 的 `solute_parameter_type`，
/// 其中没有 `ref_ratio`/`init_delta`（溶质固定 1/0，同位素由主 namelist 的
/// `DEF_ISO_REF_RATIO_*` 决定），也没有通用的 `unit_kind` 以外的东西。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TracerParameterOverrides {
    pub unit_kind: Option<String>,
    pub mol_weight: Option<f64>,
    pub init_conc: Option<f64>,
    pub precip_default_conc: Option<f64>,
    pub vapor_default_conc: Option<f64>,
    pub max_dissolved_conc: Option<f64>,
    pub decay_rate: Option<f64>,
    pub charge: Option<i32>,
}

/// 四个 family 各自的参数文件字段（上游 `tracer_param_file_for_index` 的输入）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TracerParamFiles {
    /// `DEF_ISOTOPE_PARAM_FILE`：所有同位素共用（物理与逐 species 的比值默认值）。
    pub isotope: String,
    /// `DEF_SOLUTE_PARAM_FILES`：`NAME:path` 列表，按溶质自己的顺序对应。
    pub solutes: String,
    /// `DEF_SEDIMENT_PARAM_FILE`。
    pub sediment: String,
    /// `DEF_METHANE_PARAM_FILE`。
    pub methane: String,
}

/// `tracer_defs_init` 的输入：family 开关、各 family 的名单与同位素物种参数。
///
/// 形状照抄 `MOD_MT_Namelist.F90` 的 `&nl_colm_material` / `&nl_colm_isotope_parameter`；
/// 调用方（运行期、前处理）负责把算例文档读成这个结构，本模块不碰文件。
#[derive(Debug, Clone)]
pub struct TracerNamelist {
    /// `DEF_USE_ISOTOPE`。
    pub use_isotope: bool,
    /// `DEF_ISOTOPE_SPECIES`（`H2_18O,HDO`；只有这两个有编译好的分馏物理）。
    pub isotope_species: String,
    /// `DEF_ISO_REF_RATIO_H2_18O` / `DEF_ISO_INIT_DELTA_H2_18O`。
    pub ref_ratio_h2_18o: f64,
    pub init_delta_h2_18o: f64,
    /// `DEF_ISO_REF_RATIO_HDO` / `DEF_ISO_INIT_DELTA_HDO`。
    pub ref_ratio_hdo: f64,
    pub init_delta_hdo: f64,
    /// `DEF_SOLUTE_NAMES`。
    pub solutes: String,
    /// `DEF_USE_SEDIMENT`。
    pub use_sediment: bool,
    /// `DEF_USE_METHANE`。
    pub use_methane: bool,
    /// `DEF_USE_BGC`（甲烷需要 BGC，上游是编译期 `BGC` 宏，这里按运行期开关判）。
    pub use_bgc: bool,
    /// `DEF_USE_VariablySaturatedFlow` 的生效值。
    pub variably_saturated_flow: bool,
    /// `DEF_MT_AQUIFER_MIXING_WATER_MM`。
    pub aquifer_mixing_water_mm: f64,
    /// 四个 family 的参数文件字段。
    pub param_files: TracerParamFiles,
}

impl Default for TracerNamelist {
    /// 与 `MOD_MT_Namelist.F90` 的声明默认值一致。
    fn default() -> Self {
        Self {
            use_isotope: false,
            isotope_species: "H2_18O,HDO".to_owned(),
            ref_ratio_h2_18o: 2.0052e-3,
            init_delta_h2_18o: -10.0,
            ref_ratio_hdo: 1.5576e-4,
            init_delta_hdo: -70.0,
            solutes: String::new(),
            use_sediment: false,
            use_methane: false,
            use_bgc: false,
            variably_saturated_flow: false,
            aquifer_mixing_water_mm: -1.0,
            param_files: TracerParamFiles::default(),
        }
    }
}

/// 注册好的示踪物表（上游的 `ntracers` 与 `tracers(:)`）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TracerSet {
    pub tracers: Vec<TracerDescriptor>,
}

impl TracerSet {
    /// `tracer_defs_init`（`MOD_MT_Defs.F90`）。条目顺序固定：同位素 → 溶质 → 泥沙 → 甲烷，
    /// 与上游一致（描述符指纹和历史变量名按这个顺序排）。
    ///
    /// `read_parameters(path)` 读一个溶质参数文件里的 `&nl_colm_solute_parameter`：文件不存在
    /// 报错，没有这一组返回 `None`。同位素/泥沙/甲烷的参数文件不由这里套用（上游分别在
    /// 分馏物理、`MOD_MT_Sediment`、`MOD_MT_Methane_Const` 里读）。
    pub fn build(
        namelist: &TracerNamelist,
        mut read_parameters: impl FnMut(&str) -> Result<Option<TracerParameterOverrides>>,
    ) -> Result<Self> {
        let isotopes = if namelist.use_isotope {
            split_names(&namelist.isotope_species)?
        } else {
            Vec::new()
        };
        if namelist.use_isotope && isotopes.is_empty() {
            bail!("tracer_defs_init: DEF_USE_ISOTOPE is on but DEF_ISOTOPE_SPECIES is empty.");
        }
        let solutes = split_names(&namelist.solutes)?;
        let nsed = usize::from(namelist.use_sediment);
        let ngas = usize::from(namelist.use_methane);
        let n = isotopes.len() + solutes.len() + nsed + ngas;
        if n == 0 {
            return Ok(Self::default());
        }
        let mut tracers = Vec::with_capacity(n);
        for token in &isotopes {
            // 只有编译了分馏物理的两个物种；mol_weight、ref_ratio、init_delta 来自主 namelist。
            let (name, mol_weight, ref_ratio, init_delta) =
                match token.trim().to_ascii_uppercase().as_str() {
                    "H2_18O" => (
                        "H2_18O",
                        20.0,
                        namelist.ref_ratio_h2_18o,
                        namelist.init_delta_h2_18o,
                    ),
                    "HDO" => ("HDO", 19.0, namelist.ref_ratio_hdo, namelist.init_delta_hdo),
                    other => bail!(
                        "tracer_defs_init: unknown isotope species \"{other}\" in \
                         DEF_ISOTOPE_SPECIES; available: H2_18O, HDO."
                    ),
                };
            let mut tracer = TracerDescriptor {
                name: name.to_owned(),
                category: "isotope".to_owned(),
                family: TracerFamily::Unresolved,
                state_owner: StateOwner::Unknown,
                reaction_mode: ReactionMode::None,
                unit_kind: String::new(),
                mol_weight,
                ref_ratio,
                init_delta,
                // 同位素从 init_delta 起跑也回落到它；浓度字段只进描述符指纹。
                init_conc: init_delta,
                precip_default_conc: init_delta,
                vapor_default_conc: init_delta,
                max_dissolved_conc: HUGE,
                reactive_decay_rate: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut tracer);
            tracers.push(tracer);
        }
        for token in &solutes {
            let mut tracer = TracerDescriptor {
                name: sanitize_ncname(token),
                category: "solute".to_owned(),
                family: TracerFamily::Unresolved,
                state_owner: StateOwner::Unknown,
                reaction_mode: ReactionMode::None,
                unit_kind: String::new(),
                mol_weight: 0.0,
                ref_ratio: 1.0,
                init_delta: 0.0,
                init_conc: 0.0,
                precip_default_conc: 0.0,
                vapor_default_conc: 0.0,
                max_dissolved_conc: HUGE,
                reactive_decay_rate: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut tracer);
            tracers.push(tracer);
        }
        if nsed > 0 {
            // 状态与参数由 `MOD_MT_Sediment` 拥有，注册时挂到这个条目上。
            let mut tracer = TracerDescriptor {
                name: "SEDIMENT".to_owned(),
                category: "particle".to_owned(),
                family: TracerFamily::Unresolved,
                state_owner: StateOwner::Unknown,
                reaction_mode: ReactionMode::None,
                unit_kind: String::new(),
                mol_weight: 1.0,
                ref_ratio: 1.0,
                init_delta: 0.0,
                init_conc: 0.0,
                precip_default_conc: 0.0,
                vapor_default_conc: 0.0,
                max_dissolved_conc: HUGE,
                reactive_decay_rate: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut tracer);
            tracers.push(tracer);
        }
        if ngas > 0 {
            // 状态、参数与反应由 `MOD_MT_Methane_Const` 拥有，注册时挂到这个条目上。
            let mut tracer = TracerDescriptor {
                name: "CH4".to_owned(),
                category: "gas".to_owned(),
                family: TracerFamily::Unresolved,
                state_owner: StateOwner::Unknown,
                reaction_mode: ReactionMode::None,
                unit_kind: String::new(),
                mol_weight: 16.04,
                ref_ratio: 1.0,
                init_delta: 0.0,
                init_conc: 0.0,
                precip_default_conc: 0.0,
                vapor_default_conc: 0.0,
                max_dissolved_conc: HUGE,
                reactive_decay_rate: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut tracer);
            tracers.push(tracer);
        }
        // 名字会成为 NetCDF 变量名：重名直接停机（上游 `Do i = 2, nmts` 的两两比较）。
        for i in 1..tracers.len() {
            for j in 0..i {
                ensure!(
                    !param_equal(&tracers[i].name, &tracers[j].name),
                    "tracer_defs_init: material name \"{}\" is used twice.",
                    tracers[i].name
                );
            }
        }
        // `mt_solute_read_parameters`：只有溶质的参数文件覆盖描述符。
        for i in 0..tracers.len() {
            if tracers[i].family != TracerFamily::Solute {
                continue;
            }
            let Some(path) = param_file_for_index(&namelist.param_files, &tracers, i)? else {
                continue;
            };
            if let Some(overrides) = read_parameters(&path)? {
                apply_overrides(&mut tracers[i], &overrides);
            }
        }
        for tracer in &mut tracers {
            // 上游 `tracer_defs_validate`：非有限与负的 decay 都停机（旧实现把负值夹成 0，
            // 那会让写错符号的实验静默变成不衰减）。
            ensure!(
                tracer.reactive_decay_rate.is_finite(),
                "tracer_defs_init: non-finite decay_rate for {}",
                tracer.name
            );
            ensure!(
                tracer.reactive_decay_rate >= 0.0,
                "tracer_defs_init: negative decay_rate for {}",
                tracer.name
            );
            derive_taxonomy(tracer);
            validate(tracer, namelist.use_bgc)?;
            if namelist.variably_saturated_flow && tracer.family == TracerFamily::Isotope {
                ensure!(
                    namelist.aquifer_mixing_water_mm.is_finite()
                        && namelist.aquifer_mixing_water_mm > 0.0,
                    "tracer_defs_init: VSF isotopes require explicit positive \
                     DEF_MT_AQUIFER_MIXING_WATER_MM"
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

/// `split_names`：按逗号切分，去首尾空白，空项跳过；单名超过 32 字符停机。
///
/// 上游 7d2fca93 的 `tracer_defs_init` 只接受名字列表，不再有乱填的补名/自动去重；
/// 名字后来会成为 NetCDF 变量名，超过 32 字符上游直接 `CoLM_stop`。
fn split_names(raw: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        ensure!(
            token.len() <= 32,
            "Material name longer than 32 characters: {token}"
        );
        out.push(token.to_owned());
    }
    Ok(out)
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

fn param_equal(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
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

/// `read_solute_parameter_file` 的赋值部分：文件里没写的字段保持描述符当前值。
///
/// 上游先把 `DEF_SOLUTE%*` 全填成描述符的当前值再读文件，所以"没写"等价于"不变"，
/// 不是回落到某个默认值。
fn apply_overrides(tracer: &mut TracerDescriptor, overrides: &TracerParameterOverrides) {
    if let Some(unit_kind) = &overrides.unit_kind {
        tracer.unit_kind = unit_kind.trim().to_ascii_lowercase();
    }
    if let Some(value) = overrides.mol_weight {
        tracer.mol_weight = value;
    }
    tracer.init_conc = overrides.init_conc.unwrap_or(tracer.init_conc);
    tracer.precip_default_conc = overrides
        .precip_default_conc
        .unwrap_or(tracer.precip_default_conc);
    tracer.vapor_default_conc = overrides
        .vapor_default_conc
        .unwrap_or(tracer.vapor_default_conc);
    if let Some(value) = overrides.max_dissolved_conc {
        tracer.max_dissolved_conc = value;
    }
    if let Some(value) = overrides.decay_rate {
        tracer.reactive_decay_rate = value;
    }
    if let Some(value) = overrides.charge {
        tracer.charge = value;
    }
}

/// `tracer_param_file_for_index`：按 family 选字段。
///
/// 同位素、泥沙、甲烷各读自己的单一路径字段；溶质把 `DEF_SOLUTE_PARAM_FILES` 的
/// `key:path` 按名字匹配、无冒号项按**溶质自己的顺序**（不是全局示踪物序号）对应。
/// `null` 与空串表示不读。
pub fn param_file_for_index(
    files: &TracerParamFiles,
    tracers: &[TracerDescriptor],
    index: usize,
) -> Result<Option<String>> {
    let tracer = tracers
        .get(index)
        .with_context(|| format!("tracer index {index} is not registered"))?;
    let single = |raw: &str| {
        let text = raw.trim();
        (!text.is_empty() && !text.eq_ignore_ascii_case("null")).then(|| text.to_owned())
    };
    match tracer.family {
        TracerFamily::Isotope => Ok(single(&files.isotope)),
        TracerFamily::Particle => Ok(single(&files.sediment)),
        TracerFamily::Gas => Ok(single(&files.methane)),
        TracerFamily::Solute => {
            let solute_index = tracers[..=index]
                .iter()
                .filter(|t| t.family == TracerFamily::Solute)
                .count()
                - 1;
            colm_namelist::tracer_files::param_file_for(&files.solutes, solute_index, |key| {
                param_equal(key, &tracer.name)
            })
        }
        TracerFamily::Unresolved => Ok(None),
    }
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
