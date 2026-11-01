//! 材料描述符与注册（`main/MATERIAL/MOD_MT_Defs.F90:material_defs_init` 及其辅助例程）。
//!
//! 上游把描述符放在全局表 `materials(:)` 里，按 `itrc` 查询；这里是显式的 [`MaterialSet`]。
//! 上游 7d2fca93 之后，条目由 family 开关的顺序决定（同位素 → 溶质 → 泥沙 → 甲烷），
//! 不再有 `@@DEF_MATERIAL_@@NUM/_NAMES/_TYPES` 那张 CSV；溶质的参数文件
//! （`&nl_colm_solute_parameter` 的 `DEF_SOLUTE%*`）由调用方读成
//! [`MaterialParameterOverrides`] 后交给 [`MaterialSet::build`]，本模块不碰文件。

use anyhow::{bail, ensure, Context, Result};

/// `trc_tiny`：只作算术非零保护。
pub const TRC_TINY: f64 = 1.0e-30;
/// `trc_water_min_for_ratio`：由水量求比值时的物理下限 [kg m-2]。
pub const TRC_WATER_MIN_FOR_RATIO: f64 = 1.0e-12;
/// `MATERIAL_DESCRIPTOR_IDENTITY_WIDTH`。
pub const DESCRIPTOR_IDENTITY_WIDTH: usize = 384;
/// 上游 `huge(1.0_r8)`：参数文件里"未给"的哨兵，也是"不限溶解度"。
const HUGE: f64 = f64::MAX;

/// `FAMILY_*`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialFamily {
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

/// `material_info_type`。
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialDescriptor {
    pub name: String,
    pub category: String,
    pub family: MaterialFamily,
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
    /// `DEF_SOLUTE%n2o_ef5`：进入河道时换算成 N2O-N 的间接排放因子 [kg N2O-N/kg N]。
    /// 默认 0（不换算）；只有它进描述符指纹（上游 `material_build_descriptor_identity` 的
    /// 第 9 个实数），改了它旧重启就不再匹配。
    pub n2o_ef5: f64,
    pub charge: i32,
}

/// `&nl_colm_solute_parameter` 的 `DEF_SOLUTE%*`：只放文件里写了的字段。
///
/// 上游把旧的 `material_parameter_type` 换成了按 species 的 `solute_parameter_type`，
/// 其中没有 `ref_ratio`/`init_delta`（溶质固定 1/0，同位素由主 namelist 的
/// `DEF_ISO_REF_RATIO_*` 决定），也没有通用的 `unit_kind` 以外的东西。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialParameterOverrides {
    pub unit_kind: Option<String>,
    pub mol_weight: Option<f64>,
    pub init_conc: Option<f64>,
    pub precip_default_conc: Option<f64>,
    pub vapor_default_conc: Option<f64>,
    pub max_dissolved_conc: Option<f64>,
    pub decay_rate: Option<f64>,
    /// `DEF_SOLUTE%n2o_ef5`（`MOD_MT_Solute.F90:85-115`）。
    pub n2o_ef5: Option<f64>,
    pub charge: Option<i32>,
}

/// 四个 family 各自的参数文件字段（上游 `material_param_file_for_index` 的输入）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterialParamFiles {
    /// `DEF_ISOTOPE_PARAM_FILE`：所有同位素共用（物理与逐 species 的比值默认值）。
    pub isotope: String,
    /// `DEF_SOLUTE_PARAM_FILES`：`NAME:path` 列表，按溶质自己的顺序对应。
    pub solutes: String,
    /// `DEF_SEDIMENT_PARAM_FILE`。
    pub sediment: String,
    /// `DEF_METHANE_PARAM_FILE`。
    pub methane: String,
}

/// `material_defs_init` 的输入：family 开关、各 family 的名单与同位素物种参数。
///
/// 形状照抄 `MOD_MT_Namelist.F90` 的 `&nl_colm_material` / `&nl_colm_isotope_parameter`；
/// 调用方（运行期、前处理）负责把算例文档读成这个结构，本模块不碰文件。
#[derive(Debug, Clone)]
pub struct MaterialNamelist {
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
    pub param_files: MaterialParamFiles,
}

impl Default for MaterialNamelist {
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
            param_files: MaterialParamFiles::default(),
        }
    }
}

/// 注册好的材料表（上游的 `nmaterials` 与 `materials(:)`）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MaterialSet {
    pub materials: Vec<MaterialDescriptor>,
}

impl MaterialSet {
    /// `material_defs_init`（`MOD_MT_Defs.F90`）。条目顺序固定：同位素 → 溶质 → 泥沙 → 甲烷，
    /// 与上游一致（描述符指纹和历史变量名按这个顺序排）。
    ///
    /// `read_parameters(path)` 读一个溶质参数文件里的 `&nl_colm_solute_parameter`：文件不存在
    /// 报错，没有这一组返回 `None`。同位素/泥沙/甲烷的参数文件不由这里套用（上游分别在
    /// 分馏物理、`MOD_MT_Sediment`、`MOD_MT_Methane_Const` 里读）。
    pub fn build(
        namelist: &MaterialNamelist,
        mut read_parameters: impl FnMut(&str) -> Result<Option<MaterialParameterOverrides>>,
    ) -> Result<Self> {
        let isotopes = if namelist.use_isotope {
            split_names(&namelist.isotope_species)?
        } else {
            Vec::new()
        };
        if namelist.use_isotope && isotopes.is_empty() {
            bail!("material_defs_init: DEF_USE_ISOTOPE is on but DEF_ISOTOPE_SPECIES is empty.");
        }
        let solutes = split_names(&namelist.solutes)?;
        let nsed = usize::from(namelist.use_sediment);
        let ngas = usize::from(namelist.use_methane);
        let n = isotopes.len() + solutes.len() + nsed + ngas;
        if n == 0 {
            return Ok(Self::default());
        }
        let mut materials = Vec::with_capacity(n);
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
                        "material_defs_init: unknown isotope species \"{other}\" in \
                         DEF_ISOTOPE_SPECIES; available: H2_18O, HDO."
                    ),
                };
            let mut material = MaterialDescriptor {
                name: name.to_owned(),
                category: "isotope".to_owned(),
                family: MaterialFamily::Unresolved,
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
                n2o_ef5: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut material);
            materials.push(material);
        }
        for token in &solutes {
            let mut material = MaterialDescriptor {
                name: sanitize_ncname(token),
                category: "solute".to_owned(),
                family: MaterialFamily::Unresolved,
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
                n2o_ef5: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut material);
            materials.push(material);
        }
        if nsed > 0 {
            // 状态与参数由 `MOD_MT_Sediment` 拥有，注册时挂到这个条目上。
            let mut material = MaterialDescriptor {
                name: "SEDIMENT".to_owned(),
                category: "particle".to_owned(),
                family: MaterialFamily::Unresolved,
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
                n2o_ef5: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut material);
            materials.push(material);
        }
        if ngas > 0 {
            // 状态、参数与反应由 `MOD_MT_Methane_Const` 拥有，注册时挂到这个条目上。
            let mut material = MaterialDescriptor {
                name: "CH4".to_owned(),
                category: "gas".to_owned(),
                family: MaterialFamily::Unresolved,
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
                n2o_ef5: 0.0,
                charge: 0,
            };
            set_category_defaults(&mut material);
            materials.push(material);
        }
        // 名字会成为 NetCDF 变量名：重名直接停机（上游 `Do i = 2, nmts` 的两两比较）。
        for i in 1..materials.len() {
            for j in 0..i {
                ensure!(
                    !param_equal(&materials[i].name, &materials[j].name),
                    "material_defs_init: material name \"{}\" is used twice.",
                    materials[i].name
                );
            }
        }
        // `mt_solute_read_parameters`：只有溶质的参数文件覆盖描述符。
        for i in 0..materials.len() {
            if materials[i].family != MaterialFamily::Solute {
                continue;
            }
            let Some(path) = param_file_for_index(&namelist.param_files, &materials, i)? else {
                continue;
            };
            if let Some(overrides) = read_parameters(&path)? {
                apply_overrides(&mut materials[i], &overrides);
            }
        }
        for material in &mut materials {
            // 上游 `material_defs_validate`：非有限与负的 decay 都停机（旧实现把负值夹成 0，
            // 那会让写错符号的实验静默变成不衰减）。
            ensure!(
                material.reactive_decay_rate.is_finite(),
                "material_defs_init: non-finite decay_rate for {}",
                material.name
            );
            ensure!(
                material.reactive_decay_rate >= 0.0,
                "material_defs_init: negative decay_rate for {}",
                material.name
            );
            derive_taxonomy(material);
            validate(material, namelist.use_bgc)?;
            if namelist.variably_saturated_flow && material.family == MaterialFamily::Isotope {
                ensure!(
                    namelist.aquifer_mixing_water_mm.is_finite()
                        && namelist.aquifer_mixing_water_mm > 0.0,
                    "material_defs_init: VSF isotopes require explicit positive \
                     DEF_MT_AQUIFER_MIXING_WATER_MM"
                );
            }
        }
        // `register_material_provider`（`material_lifecycle_init` 里编进来的 provider）：注册时以 provider
        // 的声明覆盖 `reaction_mode`。CH4 声明 `REACTION_PROVIDER`
        // （`MOD_Material_Reactive_Methane.F90:102`），SEDIMENT 声明 `REACTION_NONE`。它进
        // 描述符指纹（history 旁车的 `mt_hist_descriptor`）。
        for material in &mut materials {
            if matches!(
                material.name.trim().to_ascii_uppercase().as_str(),
                "CH4" | "METHANE"
            ) && material.family == MaterialFamily::Gas
            {
                material.reaction_mode = ReactionMode::Provider;
            }
        }
        Ok(Self { materials })
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }

    /// 走通用陆面水输运的材料（`material_uses_land_water_transport`）的序号。
    pub fn transport_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.materials
            .iter()
            .enumerate()
            .filter(|(_, material)| material.uses_land_water_transport())
            .map(|(index, _)| index)
    }

    /// `material_build_descriptor_identity(identity, transport_only=.true.)`：每个输运材料一行
    /// 384 个 ASCII 码（空格补齐）。
    pub fn descriptor_identity(&self) -> Vec<[i32; DESCRIPTOR_IDENTITY_WIDTH]> {
        self.materials
            .iter()
            .filter(|material| material.uses_land_water_transport())
            .map(MaterialDescriptor::identity)
            .collect()
    }

    /// `material_build_descriptor_identity(identity)`（不限输运材料）：history 旁车的
    /// `mt_hist_descriptor` 每个注册材料一行。
    pub fn descriptor_identity_all(&self) -> Vec<[i32; DESCRIPTOR_IDENTITY_WIDTH]> {
        self.materials
            .iter()
            .map(MaterialDescriptor::identity)
            .collect()
    }
}

impl MaterialDescriptor {
    pub fn is_isotope(&self) -> bool {
        self.family == MaterialFamily::Isotope
    }

    pub fn is_solute(&self) -> bool {
        self.family == MaterialFamily::Solute
    }

    pub fn uses_land_water_transport(&self) -> bool {
        self.state_owner == StateOwner::GenericWater
    }

    /// `material_is_nonvolatile_solute`。
    pub fn is_nonvolatile_solute(&self) -> bool {
        self.is_solute() && self.uses_land_water_transport()
    }

    /// `material_has_dissolved_limit`。
    pub fn has_dissolved_limit(&self) -> bool {
        self.is_nonvolatile_solute() && self.max_dissolved_conc < HUGE
    }

    /// `material_can_use_fixed_signature`。
    pub fn can_use_fixed_signature(&self) -> bool {
        self.is_isotope()
    }

    /// `material_init_water_ratio`。
    pub fn init_water_ratio(&self) -> f64 {
        if self.is_isotope() {
            delta_to_ratio(self.init_delta, self.ref_ratio)
        } else {
            self.init_conc
        }
    }

    /// `material_precip_default_ratio`。
    pub fn precip_default_ratio(&self) -> f64 {
        if self.is_isotope() {
            delta_to_ratio(self.init_delta, self.ref_ratio)
        } else {
            self.precip_default_conc
        }
    }

    /// `material_vapor_default_ratio`。
    pub fn vapor_default_ratio(&self) -> f64 {
        if self.is_isotope() {
            delta_to_ratio(self.init_delta, self.ref_ratio)
        } else {
            self.vapor_default_conc
        }
    }

    /// `material_reactive_decay_fraction`：`1 - exp(-min(k*dt, 700))`。
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

    /// `material_equilibrate_dissolved`：溶解量不超过 `max_dissolved_conc * water`，多余的进固相；
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
            self.n2o_ef5,
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
/// 上游 7d2fca93 的 `material_defs_init` 只接受名字列表，不再有乱填的补名/自动去重；
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

/// `set_material_category_defaults`。
fn set_category_defaults(material: &mut MaterialDescriptor) {
    material.unit_kind = "material_per_water".to_owned();
    material.charge = 0;
    material.max_dissolved_conc = HUGE;
    material.family = MaterialFamily::Unresolved;
    material.state_owner = StateOwner::Unknown;
    material.reaction_mode = ReactionMode::None;
    match material.category.as_str() {
        "isotope" => {
            material.unit_kind = "ratio".to_owned();
            material.family = MaterialFamily::Isotope;
            material.state_owner = StateOwner::GenericWater;
        }
        "solute" => {
            material.family = MaterialFamily::Solute;
            material.state_owner = StateOwner::GenericWater;
        }
        "reactive" => material.state_owner = StateOwner::GenericWater,
        "particle" => {
            material.unit_kind = "volume_fraction".to_owned();
            material.family = MaterialFamily::Particle;
            material.state_owner = StateOwner::Provider;
        }
        "gas" => {
            material.unit_kind = "species_owned".to_owned();
            material.family = MaterialFamily::Gas;
            material.state_owner = StateOwner::Provider;
        }
        _ => {}
    }
}

/// `read_solute_parameter_file` 的赋值部分：文件里没写的字段保持描述符当前值。
///
/// 上游先把 `DEF_SOLUTE%*` 全填成描述符的当前值再读文件，所以"没写"等价于"不变"，
/// 不是回落到某个默认值。
fn apply_overrides(material: &mut MaterialDescriptor, overrides: &MaterialParameterOverrides) {
    if let Some(unit_kind) = &overrides.unit_kind {
        material.unit_kind = unit_kind.trim().to_ascii_lowercase();
    }
    if let Some(value) = overrides.mol_weight {
        material.mol_weight = value;
    }
    material.init_conc = overrides.init_conc.unwrap_or(material.init_conc);
    material.precip_default_conc = overrides
        .precip_default_conc
        .unwrap_or(material.precip_default_conc);
    material.vapor_default_conc = overrides
        .vapor_default_conc
        .unwrap_or(material.vapor_default_conc);
    if let Some(value) = overrides.max_dissolved_conc {
        material.max_dissolved_conc = value;
    }
    if let Some(value) = overrides.decay_rate {
        material.reactive_decay_rate = value;
    }
    if let Some(value) = overrides.n2o_ef5 {
        material.n2o_ef5 = value;
    }
    if let Some(value) = overrides.charge {
        material.charge = value;
    }
}

/// `material_param_file_for_index`：按 family 选字段。
///
/// 同位素、泥沙、甲烷各读自己的单一路径字段；溶质把 `DEF_SOLUTE_PARAM_FILES` 的
/// `key:path` 按名字匹配、无冒号项按**溶质自己的顺序**（不是全局材料序号）对应。
/// `null` 与空串表示不读。
pub fn param_file_for_index(
    files: &MaterialParamFiles,
    materials: &[MaterialDescriptor],
    index: usize,
) -> Result<Option<String>> {
    let material = materials
        .get(index)
        .with_context(|| format!("material index {index} is not registered"))?;
    let single = |raw: &str| {
        let text = raw.trim();
        (!text.is_empty() && !text.eq_ignore_ascii_case("null")).then(|| text.to_owned())
    };
    match material.family {
        MaterialFamily::Isotope => Ok(single(&files.isotope)),
        MaterialFamily::Particle => Ok(single(&files.sediment)),
        MaterialFamily::Gas => Ok(single(&files.methane)),
        MaterialFamily::Solute => {
            let solute_index = materials[..=index]
                .iter()
                .filter(|t| t.family == MaterialFamily::Solute)
                .count()
                - 1;
            colm_namelist::material_files::param_file_for(&files.solutes, solute_index, |key| {
                param_equal(key, &material.name)
            })
        }
        MaterialFamily::Unresolved => Ok(None),
    }
}

/// `derive_material_taxonomy`。
fn derive_taxonomy(material: &mut MaterialDescriptor) {
    material.family = match material.category.as_str() {
        "isotope" => MaterialFamily::Isotope,
        "solute" => MaterialFamily::Solute,
        "particle" => MaterialFamily::Particle,
        "gas" => MaterialFamily::Gas,
        _ => MaterialFamily::Unresolved,
    };
    material.state_owner = match material.family {
        MaterialFamily::Isotope | MaterialFamily::Solute => StateOwner::GenericWater,
        MaterialFamily::Particle | MaterialFamily::Gas => StateOwner::Provider,
        MaterialFamily::Unresolved if material.category == "reactive" => {
            if material.unit_kind.trim() == "species_owned" {
                StateOwner::Provider
            } else {
                StateOwner::GenericWater
            }
        }
        MaterialFamily::Unresolved => StateOwner::Unknown,
    };
    material.reaction_mode = if matches!(
        material.family,
        MaterialFamily::Solute | MaterialFamily::Unresolved
    ) && material.reactive_decay_rate > 0.0
    {
        ReactionMode::FirstOrder
    } else {
        ReactionMode::None
    };
}

/// `validate_material_descriptor`。
fn validate(material: &mut MaterialDescriptor, use_bgc: bool) -> Result<()> {
    let name = material.name.clone();
    let fail = |field: &str, reason: &str| -> Result<()> {
        bail!("material_defs_init: material \"{name}\" field {field}: {reason}")
    };
    material.unit_kind = material.unit_kind.trim().to_ascii_lowercase();
    if material.family == MaterialFamily::Unresolved && material.category != "reactive" {
        fail("family_id", "only legacy reactive may remain unresolved")?;
    }
    if !matches!(
        material.state_owner,
        StateOwner::GenericWater | StateOwner::Provider
    ) {
        fail(
            "state_owner",
            "must resolve before transport initialization",
        )?;
    }
    if matches!(
        material.family,
        MaterialFamily::Isotope | MaterialFamily::Particle
    ) && material.reaction_mode != ReactionMode::None
    {
        fail(
            "reaction_mode",
            "isotope and particle families do not use the generic reaction capability",
        )?;
    }
    if !matches!(
        material.unit_kind.as_str(),
        "ratio" | "material_per_water" | "mass_fraction" | "volume_fraction" | "species_owned"
    ) {
        fail("unit_kind", "unsupported unit convention")?;
    }
    for (field, value) in [
        ("mol_weight", material.mol_weight),
        ("ref_ratio", material.ref_ratio),
        ("init_delta", material.init_delta),
        ("init_conc", material.init_conc),
        ("precip_default_conc", material.precip_default_conc),
        ("vapor_default_conc", material.vapor_default_conc),
        ("max_dissolved_conc", material.max_dissolved_conc),
        ("reactive_decay_rate", material.reactive_decay_rate),
        ("n2o_ef5", material.n2o_ef5),
    ] {
        if !value.is_finite() {
            fail(field, "must be finite")?;
        }
    }
    if material.mol_weight < 0.0 {
        fail("mol_weight", "must be non-negative")?;
    }
    if material.ref_ratio <= 0.0 {
        fail("ref_ratio", "must be positive")?;
    }
    if material.is_isotope() {
        if material.init_delta < -1000.0 {
            fail("init_delta", "must be at least -1000 permil")?;
        }
    } else {
        for (field, value) in [
            ("init_conc", material.init_conc),
            ("precip_default_conc", material.precip_default_conc),
            ("vapor_default_conc", material.vapor_default_conc),
        ] {
            if value < 0.0 {
                fail(field, "must be non-negative")?;
            }
        }
    }
    if material.max_dissolved_conc <= 0.0 {
        fail("max_dissolved_conc", "must be positive")?;
    }
    if material.is_isotope() && material.unit_kind != "ratio" {
        fail("unit_kind", "isotope materials require ratio")?;
    }
    if material.charge != 0 && !material.is_solute() {
        fail(
            "charge",
            "non-zero ionic charge is valid only for solute materials",
        )?;
    }
    if (material.is_isotope() || material.is_solute())
        && material.state_owner != StateOwner::GenericWater
    {
        fail(
            "state_owner",
            "isotope and solute materials require generic land-water transport",
        )?;
    }
    if matches!(
        material.family,
        MaterialFamily::Particle | MaterialFamily::Gas
    ) && material.state_owner != StateOwner::Provider
    {
        fail(
            "state_owner",
            "particle and gas materials require provider-owned state",
        )?;
    }
    if material.max_dissolved_conc < HUGE && !material.is_solute() {
        fail("max_dissolved_conc", "is valid only for solute materials")?;
    }
    // `MOD_MT_Solute.F90:114-115`：只有溶质会从参数文件读它，范围 [0,1)。
    if material.is_solute() && !(0.0..1.0).contains(&material.n2o_ef5) {
        fail("n2o_ef5", "must be in [0,1)")?;
    }
    if material.reactive_decay_rate > 0.0
        && !(material.is_solute() || material.family == MaterialFamily::Unresolved)
    {
        fail(
            "reactive_decay_rate",
            "first-order decay is supported only by solute materials",
        )?;
    }
    if material.state_owner == StateOwner::Provider
        && material.family != MaterialFamily::Particle
        && material.unit_kind != "species_owned"
    {
        fail(
            "unit_kind",
            "species-owned state requires unit_kind=species_owned",
        )?;
    }
    if !use_bgc
        && (material.name.eq_ignore_ascii_case("CH4")
            || material.name.eq_ignore_ascii_case("METHANE"))
        && material.state_owner == StateOwner::Provider
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
