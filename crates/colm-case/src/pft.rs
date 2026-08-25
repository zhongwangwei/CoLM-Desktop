//! PFT/PC expert-parameter defaults from CoLM's `MOD_Const_PFT.F90`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Result};

const SOURCE: &str = include_str!("../../../vendor/CoLM202X/main/MOD_Const_PFT.F90");
const OVERRIDES: &str = include_str!("../../../vendor/CoLM202X/include/pft_override_fields.inc");
const PFT_LEN: usize = 79;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Real,
    Integer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Always,
    BallBerry,
    Medlyn,
    Wue,
    PlantHydraulics,
    Bgc,
    Fire,
    Crop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PftName {
    pub zh: &'static str,
    pub en: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterMeta {
    pub name: &'static str,
    pub source: &'static str,
    pub label_zh: &'static str,
    pub label_en: &'static str,
    pub group_zh: &'static str,
    pub group_en: &'static str,
    pub unit: Option<&'static str>,
    pub condition: Condition,
    pub kind: Kind,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

macro_rules! p {
    ($name:literal, $source:literal, $zh:literal, $en:literal, $group_zh:literal, $group_en:literal, $unit:expr, $kind:expr, $condition:expr, $min:expr, $max:expr) => {
        ParameterMeta {
            name: $name,
            source: $source,
            label_zh: $zh,
            label_en: $en,
            group_zh: $group_zh,
            group_en: $group_en,
            unit: $unit,
            condition: $condition,
            kind: $kind,
            min: $min,
            max: $max,
        }
    };
}

pub const PARAMETERS: &[ParameterMeta] = &[
    p!(
        "DEF_PFT_HTOP0",
        "htop0_p",
        "冠层顶部高度",
        "Canopy top height",
        "冠层与辐射",
        "Canopy and radiation",
        Some("m"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_HBOT0",
        "hbot0_p",
        "冠层底部高度",
        "Canopy bottom height",
        "冠层与辐射",
        "Canopy and radiation",
        Some("m"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_SQRTDI",
        "sqrtdi_p",
        "叶片特征尺寸倒平方根",
        "Inverse square root leaf dimension",
        "冠层与辐射",
        "Canopy and radiation",
        Some("m-0.5"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_CHIL",
        "chil_p",
        "叶倾角分布参数",
        "Leaf angle distribution parameter",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(-1.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_RHOL_VIS",
        "rhol_vis_p",
        "绿叶可见光反射率",
        "Green leaf visible reflectance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_RHOL_NIR",
        "rhol_nir_p",
        "绿叶近红外反射率",
        "Green leaf near-IR reflectance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_RHOS_VIS",
        "rhos_vis_p",
        "枯叶可见光反射率",
        "Dead leaf visible reflectance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_RHOS_NIR",
        "rhos_nir_p",
        "枯叶近红外反射率",
        "Dead leaf near-IR reflectance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_TAUL_VIS",
        "taul_vis_p",
        "绿叶可见光透射率",
        "Green leaf visible transmittance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_TAUL_NIR",
        "taul_nir_p",
        "绿叶近红外透射率",
        "Green leaf near-IR transmittance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_TAUS_VIS",
        "taus_vis_p",
        "枯叶可见光透射率",
        "Dead leaf visible transmittance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_TAUS_NIR",
        "taus_nir_p",
        "枯叶近红外透射率",
        "Dead leaf near-IR transmittance",
        "冠层与辐射",
        "Canopy and radiation",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_VMAX25",
        "vmax25_p",
        "25°C最大羧化速率",
        "Maximum carboxylation rate at 25°C",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("umol m-2 s-1"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_EFFCON",
        "effcon_p",
        "量子效率",
        "Quantum efficiency",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_C3C4",
        "c3c4_p",
        "C3/C4 标志",
        "C3/C4 flag",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Integer,
        Condition::Always,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_G1",
        "g1_p",
        "Medlyn 斜率 g1",
        "Medlyn slope g1",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::Medlyn,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_G0",
        "g0_p",
        "Medlyn 截距 g0",
        "Medlyn intercept g0",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::Medlyn,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_GRADM",
        "gradm_p",
        "Ball–Berry 斜率",
        "Ball-Berry slope",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::BallBerry,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_BINTER",
        "binter_p",
        "Ball–Berry 截距",
        "Ball-Berry intercept",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::BallBerry,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_LAMBDA",
        "lambda_p",
        "WUE 边际水分成本",
        "WUE marginal water cost",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("mol mol-1"),
        Kind::Real,
        Condition::Wue,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_SHTI",
        "shti_p",
        "高温抑制斜率",
        "High-temperature inhibition slope",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_SLTI",
        "slti_p",
        "低温抑制斜率",
        "Low-temperature inhibition slope",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_TRDA",
        "trda_p",
        "气孔温度系数 A",
        "Stomatal temperature coefficient A",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_TRDM",
        "trdm_p",
        "气孔温度系数 M",
        "Stomatal temperature coefficient M",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("K"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_TROP",
        "trop_p",
        "最适温度",
        "Optimum temperature",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("K"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_HHTI",
        "hhti_p",
        "高温抑制半响应温度",
        "High-temperature half-inhibition temperature",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("K"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_HLTI",
        "hlti_p",
        "低温抑制半响应温度",
        "Low-temperature half-inhibition temperature",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("K"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_EXTKN",
        "extkn_p",
        "叶氮分配系数",
        "Leaf nitrogen allocation coefficient",
        "光合与气孔",
        "Photosynthesis and stomata",
        Some("-"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_D50",
        "d50_p",
        "50%根系深度",
        "Depth at 50% roots",
        "根系与水力",
        "Roots and hydraulics",
        Some("cm"),
        Kind::Real,
        Condition::Always,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_BETA",
        "beta_p",
        "根系分布形状参数",
        "Root profile coefficient",
        "根系与水力",
        "Roots and hydraulics",
        Some("-"),
        Kind::Real,
        Condition::Always,
        None,
        Some(0.0)
    ),
    p!(
        "DEF_PFT_KMAX_SUN",
        "kmax_sun_p",
        "阳叶最大导水率",
        "Sunlit leaf maximum conductance",
        "根系与水力",
        "Roots and hydraulics",
        None,
        Kind::Real,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_KMAX_SHA",
        "kmax_sha_p",
        "阴叶最大导水率",
        "Shaded leaf maximum conductance",
        "根系与水力",
        "Roots and hydraulics",
        None,
        Kind::Real,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_KMAX_XYL",
        "kmax_xyl_p",
        "木质部最大导水率",
        "Xylem maximum conductance",
        "根系与水力",
        "Roots and hydraulics",
        None,
        Kind::Real,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_KMAX_ROOT",
        "kmax_root_p",
        "根最大导水率",
        "Root maximum conductance",
        "根系与水力",
        "Roots and hydraulics",
        None,
        Kind::Real,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_PSI50_SUN",
        "psi50_sun_p",
        "阳叶50%失导水势",
        "Sunlit leaf P50 water potential",
        "根系与水力",
        "Roots and hydraulics",
        Some("mmH2O"),
        Kind::Real,
        Condition::PlantHydraulics,
        None,
        Some(0.0)
    ),
    p!(
        "DEF_PFT_PSI50_SHA",
        "psi50_sha_p",
        "阴叶50%失导水势",
        "Shaded leaf P50 water potential",
        "根系与水力",
        "Roots and hydraulics",
        Some("mmH2O"),
        Kind::Real,
        Condition::PlantHydraulics,
        None,
        Some(0.0)
    ),
    p!(
        "DEF_PFT_PSI50_XYL",
        "psi50_xyl_p",
        "木质部50%失导水势",
        "Xylem P50 water potential",
        "根系与水力",
        "Roots and hydraulics",
        Some("mmH2O"),
        Kind::Real,
        Condition::PlantHydraulics,
        None,
        Some(0.0)
    ),
    p!(
        "DEF_PFT_PSI50_ROOT",
        "psi50_root_p",
        "根50%失导水势",
        "Root P50 water potential",
        "根系与水力",
        "Roots and hydraulics",
        Some("mmH2O"),
        Kind::Real,
        Condition::PlantHydraulics,
        None,
        Some(0.0)
    ),
    p!(
        "DEF_PFT_CK",
        "ck_p",
        "脆弱性曲线形状参数",
        "Vulnerability curve shape parameter",
        "根系与水力",
        "Roots and hydraulics",
        Some("-"),
        Kind::Real,
        Condition::PlantHydraulics,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_FSR_PFT",
        "fsr_pft",
        "火灾存活率",
        "Fire survival fraction",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_FD_PFT",
        "fd_pft",
        "火灾持续时间",
        "Fire duration",
        "火扰动",
        "Fire disturbance",
        Some("h"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        None
    ),
    p!(
        "DEF_PFT_CC_LEAF",
        "cc_leaf",
        "叶片燃烧完全度",
        "Leaf combustion completeness",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_CC_LSTEM",
        "cc_lstem",
        "活茎燃烧完全度",
        "Live-stem combustion completeness",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_CC_DSTEM",
        "cc_dstem",
        "死茎燃烧完全度",
        "Dead-stem combustion completeness",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_CC_OTHER",
        "cc_other",
        "其他组织燃烧完全度",
        "Other-tissue combustion completeness",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_FM_LEAF",
        "fm_leaf",
        "叶片火致死亡比例",
        "Leaf fire-mortality fraction",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_FM_LSTEM",
        "fm_lstem",
        "活茎火致死亡比例",
        "Live-stem fire-mortality fraction",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_FM_LROOT",
        "fm_lroot",
        "活粗根火致死亡比例",
        "Live coarse-root fire-mortality fraction",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_FM_ROOT",
        "fm_root",
        "细根火致死亡比例",
        "Fine-root fire-mortality fraction",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_FM_DROOT",
        "fm_droot",
        "死粗根火致死亡比例",
        "Dead coarse-root fire-mortality fraction",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_FM_OTHER",
        "fm_other",
        "其他组织火致死亡比例",
        "Other-tissue fire-mortality fraction",
        "火扰动",
        "Fire disturbance",
        Some("-"),
        Kind::Real,
        Condition::Fire,
        Some(0.0),
        Some(1.0)
    ),
    p!(
        "DEF_PFT_GRPERC",
        "grperc",
        "生长呼吸比例",
        "Growth respiration fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_GRPNOW",
        "grpnow",
        "生长呼吸即时支付比例",
        "Fraction of growth respiration paid immediately",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_LF_FLAB",
        "lf_flab",
        "叶凋落物易分解组分比例",
        "Leaf-litter labile fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_LF_FCEL",
        "lf_fcel",
        "叶凋落物纤维素组分比例",
        "Leaf-litter cellulose fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_LF_FLIG",
        "lf_flig",
        "叶凋落物木质素组分比例",
        "Leaf-litter lignin fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_FR_FLAB",
        "fr_flab",
        "细根凋落物易分解组分比例",
        "Fine-root litter labile fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_FR_FCEL",
        "fr_fcel",
        "细根凋落物纤维素组分比例",
        "Fine-root litter cellulose fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_FR_FLIG",
        "fr_flig",
        "细根凋落物木质素组分比例",
        "Fine-root litter lignin fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_LEAFCN",
        "leafcn",
        "叶片碳氮比",
        "Leaf C:N ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_FROOTCN",
        "frootcn",
        "细根碳氮比",
        "Fine-root C:N ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_LIVEWDCN",
        "livewdcn",
        "活木质部碳氮比",
        "Live-wood C:N ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_DEADWDCN",
        "deadwdcn",
        "死木质部碳氮比",
        "Dead-wood C:N ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_GRAINCN",
        "graincn",
        "籽粒碳氮比",
        "Grain C:N ratio",
        "作物",
        "Crop",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_LFLITCN",
        "lflitcn",
        "叶凋落物碳氮比",
        "Leaf-litter C:N ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_LEAF_LONG",
        "leaf_long",
        "叶片寿命",
        "Leaf longevity",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("yr"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_FROOT_LEAF",
        "froot_leaf",
        "细根与叶片分配比",
        "Fine-root:leaf allocation ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_CROOT_STEM",
        "croot_stem",
        "粗根与茎分配比",
        "Coarse-root:stem allocation ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_STEM_LEAF",
        "stem_leaf",
        "茎与叶片分配比",
        "Stem:leaf allocation ratio",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_FLIVEWD",
        "flivewd",
        "活木质部比例",
        "Live-wood fraction",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_FCUR2",
        "fcur2",
        "当前光合产物直接生长比例",
        "Fraction of current photosynthate used directly for growth",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("-"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_SLATOP",
        "slatop",
        "冠层顶部比叶面积",
        "Specific leaf area at canopy top",
        "BGC与植被结构",
        "BGC and vegetation structure",
        Some("m2 gC-1"),
        Kind::Real,
        Condition::Bgc,
        None,
        None
    ),
    p!(
        "DEF_PFT_LAIMX",
        "laimx",
        "最大叶面积指数",
        "Maximum leaf area index",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_MANURE",
        "manure",
        "最大有机肥氮施用量",
        "Maximum manure nitrogen application",
        "作物",
        "Crop",
        Some("kg N m-2"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_LFEMERG",
        "lfemerg",
        "出苗热量指数阈值",
        "Leaf-emergence heat-unit threshold",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_MXMAT",
        "mxmat",
        "最大成熟天数",
        "Maximum days to maturity",
        "作物",
        "Crop",
        Some("d"),
        Kind::Integer,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_GRNFILL",
        "grnfill",
        "灌浆热量指数阈值",
        "Grain-fill heat-unit threshold",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_BASET",
        "baset",
        "积温基准温度",
        "Base temperature for growing degree days",
        "作物",
        "Crop",
        Some("°C"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_ASTEMF",
        "astemf",
        "成熟期最低茎分配比例",
        "Minimum stem allocation fraction at maturity",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_AROOTI",
        "arooti",
        "初始根分配比例",
        "Initial root allocation fraction",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_AROOTF",
        "arootf",
        "最终根分配比例",
        "Final root allocation fraction",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_FLEAFI",
        "fleafi",
        "初始叶片分配比例",
        "Initial leaf allocation fraction",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_BFACT",
        "bfact",
        "叶片分配曲线形状系数",
        "Leaf-allocation curve shape factor",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_ALLCONSS",
        "allconss",
        "茎分配衰减指数",
        "Stem-allocation decline exponent",
        "作物",
        "Crop",
        Some("-"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_FLEAFCN",
        "fleafcn",
        "灌浆期叶片碳氮比",
        "Leaf C:N ratio during grain fill",
        "作物",
        "Crop",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_FSTEMCN",
        "fstemcn",
        "灌浆期茎碳氮比",
        "Stem C:N ratio during grain fill",
        "作物",
        "Crop",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
    p!(
        "DEF_PFT_FFROOTCN",
        "ffrootcn",
        "灌浆期细根碳氮比",
        "Fine-root C:N ratio during grain fill",
        "作物",
        "Crop",
        Some("gC gN-1"),
        Kind::Real,
        Condition::Crop,
        None,
        None
    ),
];

pub fn all_parameters() -> &'static [ParameterMeta] {
    PARAMETERS
}

pub fn parameter(name: &str) -> Option<&'static ParameterMeta> {
    let base = match override_path(name) {
        Some((base, index)) if index < PFT_LEN => base,
        Some(_) => return None,
        None => name,
    };
    PARAMETERS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(base))
}

pub fn is_parameter(name: &str) -> bool {
    parameter(name).is_some()
}

pub fn is_override_path(name: &str) -> bool {
    override_path(name).is_some_and(|(base, index)| index < PFT_LEN && is_parameter(base))
}

pub fn default_literal(
    name: &str,
    pft_type: u8,
    campbell: bool,
    pc: bool,
) -> Result<Option<String>> {
    Ok(default_value(name, pft_type, campbell, pc)?.map(|value| {
        if parameter(name).is_some_and(|p| p.kind == Kind::Integer) {
            (value as i64).to_string()
        } else {
            format_value(value)
        }
    }))
}

pub fn default_value(name: &str, pft_type: u8, campbell: bool, pc: bool) -> Result<Option<f64>> {
    let (base, index) = override_path(name).unwrap_or((name, pft_type as usize));
    let Some(meta) = parameter(base) else {
        return Ok(None);
    };
    table().value(meta.source, index, campbell, pc).map(Some)
}

pub fn validate_override(name: &str, value: f64) -> Result<()> {
    let Some(meta) = parameter(name) else {
        bail!("{name} is not a PFT parameter");
    };
    if !value.is_finite() {
        bail!("{name} must be finite");
    }
    if meta.kind == Kind::Integer && value.fract() != 0.0 {
        bail!("{name} must be an integer");
    }
    let rule = validation_rule(meta.name)
        .ok_or_else(|| anyhow!("{} is missing from pft_override_fields.inc", meta.name))?;
    let valid = match rule {
        "range01" => (0.0..=1.0).contains(&value),
        "range_chil" => (-1.0..=1.0).contains(&value),
        "open01" => value > 0.0 && value < 1.0,
        "ge" => value >= 0.0,
        "gt" => value > 0.0,
        "gt_1_6" => value > 1.6,
        "negative" => value < 0.0,
        "dynamic_or_ge" => value == -1.0 || value >= 0.0,
        "finite" => true,
        "binary" => value == 0.0 || value == 1.0,
        _ => false,
    };
    if !valid {
        bail!("{name}={value} violates CoLM PFT rule {rule}");
    }
    Ok(())
}

fn validation_rule(name: &str) -> Option<&'static str> {
    OVERRIDES.lines().find_map(|line| {
        let body = line
            .strip_prefix("PFT_OVERRIDE_REAL(")
            .or_else(|| line.strip_prefix("PFT_OVERRIDE_INTEGER("))?;
        let mut parts = body.split(',').map(str::trim);
        if !parts.next()?.eq_ignore_ascii_case(name) {
            return None;
        }
        parts.next()?;
        Some(parts.next()?.trim_matches('\''))
    })
}

pub fn pft_name(pft_type: u8) -> Option<PftName> {
    PFT_NAMES.get(pft_type as usize).copied()
}

pub const PFT_NAMES: &[PftName; PFT_LEN] = &[
    PftName {
        zh: "非植被",
        en: "not vegetated",
    },
    PftName {
        zh: "温带常绿针叶树",
        en: "needleleaf evergreen temperate tree",
    },
    PftName {
        zh: "寒带常绿针叶树",
        en: "needleleaf evergreen boreal tree",
    },
    PftName {
        zh: "寒带落叶针叶树",
        en: "needleleaf deciduous boreal tree",
    },
    PftName {
        zh: "热带常绿阔叶树",
        en: "broadleaf evergreen tropical tree",
    },
    PftName {
        zh: "温带常绿阔叶树",
        en: "broadleaf evergreen temperate tree",
    },
    PftName {
        zh: "热带落叶阔叶树",
        en: "broadleaf deciduous tropical tree",
    },
    PftName {
        zh: "温带落叶阔叶树",
        en: "broadleaf deciduous temperate tree",
    },
    PftName {
        zh: "寒带落叶阔叶树",
        en: "broadleaf deciduous boreal tree",
    },
    PftName {
        zh: "常绿阔叶灌木",
        en: "broadleaf evergreen shrub",
    },
    PftName {
        zh: "温带落叶阔叶灌木",
        en: "broadleaf deciduous temperate shrub",
    },
    PftName {
        zh: "寒带落叶阔叶灌木",
        en: "broadleaf deciduous boreal shrub",
    },
    PftName {
        zh: "C3 北极草",
        en: "c3 arctic grass",
    },
    PftName {
        zh: "C3 非北极草",
        en: "c3 non-arctic grass",
    },
    PftName {
        zh: "C4 草",
        en: "c4 grass",
    },
    PftName {
        zh: "C3 作物",
        en: "c3 crop",
    },
    PftName {
        zh: "灌溉 C3",
        en: "c3 irrigated",
    },
    PftName {
        zh: "温带玉米",
        en: "temperate corn",
    },
    PftName {
        zh: "灌溉温带玉米",
        en: "irrigated temperate corn",
    },
    PftName {
        zh: "春小麦",
        en: "spring wheat",
    },
    PftName {
        zh: "灌溉春小麦",
        en: "irrigated spring wheat",
    },
    PftName {
        zh: "冬小麦",
        en: "winter wheat",
    },
    PftName {
        zh: "灌溉冬小麦",
        en: "irrigated winter wheat",
    },
    PftName {
        zh: "温带大豆",
        en: "temperate soybean",
    },
    PftName {
        zh: "灌溉温带大豆",
        en: "irrigated temperate soybean",
    },
    PftName {
        zh: "大麦",
        en: "barley",
    },
    PftName {
        zh: "灌溉大麦",
        en: "irrigated barley",
    },
    PftName {
        zh: "冬大麦",
        en: "winter barley",
    },
    PftName {
        zh: "灌溉冬大麦",
        en: "irrigated winter barley",
    },
    PftName {
        zh: "黑麦",
        en: "rye",
    },
    PftName {
        zh: "灌溉黑麦",
        en: "irrigated rye",
    },
    PftName {
        zh: "冬黑麦",
        en: "winter rye",
    },
    PftName {
        zh: "灌溉冬黑麦",
        en: "irrigated winter rye",
    },
    PftName {
        zh: "木薯",
        en: "cassava",
    },
    PftName {
        zh: "灌溉木薯",
        en: "irrigated cassava",
    },
    PftName {
        zh: "柑橘",
        en: "citrus",
    },
    PftName {
        zh: "灌溉柑橘",
        en: "irrigated citrus",
    },
    PftName {
        zh: "可可",
        en: "cocoa",
    },
    PftName {
        zh: "灌溉可可",
        en: "irrigated cocoa",
    },
    PftName {
        zh: "咖啡",
        en: "coffee",
    },
    PftName {
        zh: "灌溉咖啡",
        en: "irrigated coffee",
    },
    PftName {
        zh: "棉花",
        en: "cotton",
    },
    PftName {
        zh: "灌溉棉花",
        en: "irrigated cotton",
    },
    PftName {
        zh: "椰枣",
        en: "datepalm",
    },
    PftName {
        zh: "灌溉椰枣",
        en: "irrigated datepalm",
    },
    PftName {
        zh: "饲草",
        en: "foddergrass",
    },
    PftName {
        zh: "灌溉饲草",
        en: "irrigated foddergrass",
    },
    PftName {
        zh: "葡萄",
        en: "grapes",
    },
    PftName {
        zh: "灌溉葡萄",
        en: "irrigated grapes",
    },
    PftName {
        zh: "花生",
        en: "groundnuts",
    },
    PftName {
        zh: "灌溉花生",
        en: "irrigated groundnuts",
    },
    PftName {
        zh: "小米",
        en: "millet",
    },
    PftName {
        zh: "灌溉小米",
        en: "irrigated millet",
    },
    PftName {
        zh: "油棕",
        en: "oilpalm",
    },
    PftName {
        zh: "灌溉油棕",
        en: "irrigated oilpalm",
    },
    PftName {
        zh: "马铃薯",
        en: "potatoes",
    },
    PftName {
        zh: "灌溉马铃薯",
        en: "irrigated potatoes",
    },
    PftName {
        zh: "豆类",
        en: "pulses",
    },
    PftName {
        zh: "灌溉豆类",
        en: "irrigated pulses",
    },
    PftName {
        zh: "油菜",
        en: "rapeseed",
    },
    PftName {
        zh: "灌溉油菜",
        en: "irrigated rapeseed",
    },
    PftName {
        zh: "水稻",
        en: "rice",
    },
    PftName {
        zh: "灌溉水稻",
        en: "irrigated rice",
    },
    PftName {
        zh: "高粱",
        en: "sorghum",
    },
    PftName {
        zh: "灌溉高粱",
        en: "irrigated sorghum",
    },
    PftName {
        zh: "甜菜",
        en: "sugarbeet",
    },
    PftName {
        zh: "灌溉甜菜",
        en: "irrigated sugarbeet",
    },
    PftName {
        zh: "甘蔗",
        en: "sugarcane",
    },
    PftName {
        zh: "灌溉甘蔗",
        en: "irrigated sugarcane",
    },
    PftName {
        zh: "向日葵",
        en: "sunflower",
    },
    PftName {
        zh: "灌溉向日葵",
        en: "irrigated sunflower",
    },
    PftName {
        zh: "芒草",
        en: "miscanthus",
    },
    PftName {
        zh: "灌溉芒草",
        en: "irrigated miscanthus",
    },
    PftName {
        zh: "柳枝稷",
        en: "switchgrass",
    },
    PftName {
        zh: "灌溉柳枝稷",
        en: "irrigated switchgrass",
    },
    PftName {
        zh: "热带玉米",
        en: "tropical corn",
    },
    PftName {
        zh: "灌溉热带玉米",
        en: "irrigated tropical corn",
    },
    PftName {
        zh: "热带大豆",
        en: "tropical soybean",
    },
    PftName {
        zh: "灌溉热带大豆",
        en: "irrigated tropical soybean",
    },
];

#[derive(Debug)]
struct Table(BTreeMap<&'static str, Vec<f64>>);

impl Table {
    fn value(
        &self,
        source: &'static str,
        pft_type: usize,
        campbell: bool,
        pc: bool,
    ) -> Result<f64> {
        if pft_type >= PFT_LEN {
            bail!("PFT type {pft_type} is outside range 0..=78");
        }
        let key = match source {
            "vmax25_p" => {
                if campbell {
                    "vmax25_p_campbell"
                } else {
                    "vmax25_p_vgm"
                }
            }
            "lambda_p" => {
                if campbell {
                    "lambda_p_campbell"
                } else {
                    "lambda_p_vgm"
                }
            }
            "rhol_vis_p" => {
                if pc {
                    "rhol_vis_p_pc"
                } else {
                    "rhol_vis_p"
                }
            }
            "rhol_nir_p" => {
                if pc {
                    "rhol_nir_p_pc"
                } else {
                    "rhol_nir_p"
                }
            }
            "taul_vis_p" => {
                if pc {
                    "taul_vis_p_pc"
                } else {
                    "taul_vis_p"
                }
            }
            "taul_nir_p" => {
                if pc {
                    "taul_nir_p_pc"
                } else {
                    "taul_nir_p"
                }
            }
            other => other,
        };
        self.0
            .get(key)
            .and_then(|values| values.get(pft_type).copied())
            .ok_or_else(|| anyhow!("{key} is missing from MOD_Const_PFT.F90"))
    }
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| parse_source(SOURCE).expect("MOD_Const_PFT.F90 constants must parse"))
}

fn parse_source(src: &str) -> Result<Table> {
    let clean: Vec<String> = src.lines().map(clean_line).collect();
    let mut map = BTreeMap::new();
    for meta in PARAMETERS {
        match meta.source {
            "vmax25_p" | "lambda_p" => {}
            source => {
                map.insert(
                    source,
                    parse_assignment(&clean, assignment_key(source), PFT_LEN)?,
                );
            }
        }
    }
    for source in ["rhol_vis_p", "rhol_nir_p", "taul_vis_p", "taul_nir_p"] {
        let pc_key = match source {
            "rhol_vis_p" => "rhol_vis_p_pc",
            "rhol_nir_p" => "rhol_nir_p_pc",
            "taul_vis_p" => "taul_vis_p_pc",
            "taul_nir_p" => "taul_nir_p_pc",
            _ => unreachable!(),
        };
        map.insert(pc_key, parse_assignment(&clean, pc_key, PFT_LEN)?);
    }
    for source in ["vmax25_p", "lambda_p"] {
        map.insert(
            branch_key(source, true),
            parse_branch(&clean, source, true)?,
        );
        map.insert(
            branch_key(source, false),
            parse_branch(&clean, source, false)?,
        );
    }
    Ok(Table(map))
}

fn assignment_key(source: &'static str) -> &'static str {
    match source {
        "rhol_vis_p" => "rhol_vis_p_default",
        "rhol_nir_p" => "rhol_nir_p_default",
        "taul_vis_p" => "taul_vis_p_default",
        "taul_nir_p" => "taul_nir_p_default",
        other => other,
    }
}

fn branch_key(source: &'static str, campbell: bool) -> &'static str {
    match (source, campbell) {
        ("vmax25_p", true) => "vmax25_p_campbell",
        ("vmax25_p", false) => "vmax25_p_vgm",
        ("lambda_p", true) => "lambda_p_campbell",
        ("lambda_p", false) => "lambda_p_vgm",
        _ => source,
    }
}

fn parse_branch(lines: &[String], var: &str, campbell: bool) -> Result<Vec<f64>> {
    let if_pos = lines
        .iter()
        .position(|line| line.contains("IF (DEF_USE_Campbell_SOIL_MODEL)"))
        .ok_or_else(|| anyhow!("Campbell/VGM branch not found"))?;
    let else_rel = lines[if_pos..]
        .iter()
        .position(|line| line.trim() == "ELSE")
        .ok_or_else(|| anyhow!("Campbell/VGM ELSE not found"))?;
    let end_rel = lines[if_pos + else_rel..]
        .iter()
        .position(|line| line.trim() == "ENDIF")
        .ok_or_else(|| anyhow!("Campbell/VGM ENDIF not found"))?;
    let slice = if campbell {
        &lines[if_pos..if_pos + else_rel]
    } else {
        &lines[if_pos + else_rel..if_pos + else_rel + end_rel]
    };
    parse_assignment(slice, var, PFT_LEN)
}

fn parse_assignment(lines: &[String], var: &str, len: usize) -> Result<Vec<f64>> {
    let Some(start) = lines.iter().position(|line| contains_var(line, var)) else {
        bail!("{var} not found");
    };
    let mut rhs = String::new();
    for line in &lines[start..] {
        if rhs.is_empty() {
            if let Some((_, tail)) = line.split_once('=') {
                rhs.push_str(tail);
            }
        } else {
            rhs.push(' ');
            rhs.push_str(line);
        }
        if !rhs.is_empty() && (rhs.contains("/)") || (!rhs.contains("(/") && !line.contains('&'))) {
            break;
        }
    }
    if rhs.is_empty() {
        bail!("{var} has no assignment");
    }
    let rhs = rhs
        .replace("_r8", "")
        .replace("_r4", "")
        .replace(['D', 'd'], "e")
        .replace(".True.", "1")
        .replace(".False.", "0");
    let values = if let (Some(a), Some(b)) = (rhs.find("(/"), rhs.find("/)")) {
        rhs[a + 2..b].split(',').filter_map(parse_number).collect()
    } else {
        let value =
            parse_number(&rhs).ok_or_else(|| anyhow!("cannot parse scalar {var} = {rhs:?}"))?;
        vec![value; len]
    };
    if values.len() != len {
        bail!("{var} has {} values, expected {len}", values.len());
    }
    Ok(values)
}

fn clean_line(line: &str) -> String {
    line.split('!').next().unwrap_or("").replace('&', " ")
}

fn contains_var(line: &str, var: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.contains(&format!(":: {var}"))
        || trimmed.contains(&format!("::{var}"))
        || trimmed.starts_with(&format!("{var} "))
        || (trimmed.starts_with(var) && trimmed.contains('='))
}

fn parse_number(raw: &str) -> Option<f64> {
    let token = raw.trim().trim_end_matches('*').split_whitespace().next()?;
    if token.is_empty() {
        None
    } else {
        token.parse().ok()
    }
}

fn format_value(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    let s = format!("{value:.12e}");
    s.replace("e+000", "e0")
        .replace("e+00", "e")
        .replace("e+0", "e")
        .replace("e-00", "e-")
        .replace("e-0", "e-")
}

fn override_path(name: &str) -> Option<(&str, usize)> {
    let open = name.find('(')?;
    let close = name[open + 1..].find(')')? + open + 1;
    if close + 1 != name.len() {
        return None;
    }
    let slot: usize = name[open + 1..close].trim().parse().ok()?;
    Some((&name[..open], slot.checked_sub(1)?))
}
