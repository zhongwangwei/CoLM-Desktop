//! `MOD_Const_LC.F90` 的地类常量：取值与派生。
//!
//! 表在 `land_cover_generated.rs`（`xtask gen-landcover` 生成、drift 测试守住），
//! 这里放**不能靠抄表得到的东西**：分类体系的选择、`vmax25` 的 `1.e-6` 缩放、
//! `rho`/`tau` 的二维铺排，以及 `rootfr` 的两种根系分布。
//!
//! 上游把这些写在 `Init_LC_Const` 里；本模块逐条照做，不自作聪明地"简化"。
//! `ROOTFR_SCHEME` 的两支公式在 `MOD_Const_LC.F90` 里各自成段，取值必须一起看。

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::land_cover_generated::{LandCoverTables, IGBP, IGBP_CLASSES, USGS, USGS_CLASSES};
use crate::{LandCoverScheme, LeafOptics};

pub use crate::land_cover_generated;

/// 选中分类体系的常量表。
pub fn land_cover_tables(scheme: LandCoverScheme) -> &'static LandCoverTables {
    match scheme {
        LandCoverScheme::Igbp => &IGBP,
        LandCoverScheme::Usgs => &USGS,
    }
}

/// 一个分类体系的地类数。
pub fn land_cover_classes(scheme: LandCoverScheme) -> usize {
    match scheme {
        LandCoverScheme::Igbp => IGBP_CLASSES,
        LandCoverScheme::Usgs => USGS_CLASSES,
    }
}

/// `WATERBODY`：`MOD_Vars_Global.F90:25,37` —— USGS 16、IGBP 17。
///
/// **必须显式给**，不能只靠 `fveg0 > 0`：两张 `FVEG0_*` 表**每一类都是 1.0**，
/// 水体也不例外，所以"覆盖度为正"判不出水体。`MOD_LAIReadin.F90:128` 第一句就是
/// `IF (m == 0 .or. m == WATERBODY) green = 0`。
pub fn waterbody_class(scheme: LandCoverScheme) -> usize {
    match scheme {
        LandCoverScheme::Igbp => 17,
        LandCoverScheme::Usgs => 16,
    }
}

/// `DEF_RootReachScheme` / `ROOTFR_SCHEME`：上默认与备用的两套根系分布。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootFractionScheme {
    /// `ROOTFR_SCHEME == 1`：Schenk & Jackson (2002) 的 `d50`/`beta` 形式。
    SchenkJackson,
    /// 其余取值：`roota`/`rootb` 的双指数形式。
    Exponential,
}

/// `Init_LC_Const` 里被选中的那些数组，按**地类号**访问。
///
/// 上游那些表的维度是 `(N_land_classification)`，即位置 1..17，而查表写的是
/// `patchtypes(SITE_landtype)`（`MOD_Vars_TimeVariables.F90:1273`）——
/// **位置号就是地类号**，所以草地（`patchclass = 10`）读的是第 10 个元素。
/// 类名表 `patchclassname` 是 `(0:N_land_classification)`，多出的那一个位置 0
/// 是海洋，数值表里没有对应行，因此本类型只收 1..=N。
/// 一个地类的植物水力性状，即上游 `MOD_Vars_TimeVariables` 里
/// `kmax_sun`/`kmax_sha`/`kmax_xyl`/`kmax_root`、四个 `psi50_*` 与 `ck`
/// 这九个数组的第 `lc` 个元素。
///
/// 单位与上游一致、不做换算：`kmax` 是导度、`psi50` 是**毫米水柱**（注释里的
/// mmH2O），`ck` 是脆弱性曲线的形状参数（无量纲）。`MOD_PlantHydraulic.F90`
/// 直接拿它们参与运算，换算会在两处各写一份。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantHydraulicTraits {
    pub maximum_sunlit_leaf_conductance: f64,
    pub maximum_shaded_leaf_conductance: f64,
    pub maximum_xylem_conductance: f64,
    pub maximum_root_conductance: f64,
    pub sunlit_leaf_psi50_mm: f64,
    pub shaded_leaf_psi50_mm: f64,
    pub xylem_psi50_mm: f64,
    pub root_psi50_mm: f64,
    pub vulnerability_shape: f64,
}

/// `DEF_LC_*` 的九个可选覆盖（`MOD_Namelist.F90:573-581`）。
///
/// `None` = namelist 没写（schema 的声明默认值是 `-1.e36`，也就是上游的
/// `LC_OVERRIDE_UNSET`），用表里的值；`Some(_)` = 该列整体覆盖成同一个数。
/// 上游的判据是 `IF (DEF_LC_X /= LC_OVERRIDE_UNSET) X(lc) = DEF_LC_X`，
/// 所以是"整列一个数"，不是逐地类。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlantHydraulicOverrides {
    pub maximum_sunlit_leaf_conductance: Option<f64>,
    pub maximum_shaded_leaf_conductance: Option<f64>,
    pub maximum_xylem_conductance: Option<f64>,
    pub maximum_root_conductance: Option<f64>,
    pub sunlit_leaf_psi50_mm: Option<f64>,
    pub shaded_leaf_psi50_mm: Option<f64>,
    pub xylem_psi50_mm: Option<f64>,
    pub root_psi50_mm: Option<f64>,
    pub vulnerability_shape: Option<f64>,
}

/// `apply_lc_scalar_overrides`（`MOD_Const_LC.F90:879-930`）里 PHS 之外的地类表逐列覆盖。
///
/// 上游只在单点、`DEF_USE_LCT` 时对本站地类（`SITE_landtype`）生效，在 `Init_LC_Const` 里、根系分布
/// 之前覆盖——所以 `d50`/`beta` 的覆盖也改 `rootfr`。`vmax25` 以 µmol/m²/s 给出，与表值一样再乘 1e-6。
/// `respcp` 覆盖原来不起作用（`calc_photo_params` 把同名局部量按 `0.015*c3 + 0.025*c4` 重算，
/// upstream-bugs 第 32 条）；vendor 已修，现由 [`crate::LeafBiochemistry::respiration_fraction_override`] 带进光合。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LandClassOverrides {
    pub htop0: Option<f64>,
    pub hbot0: Option<f64>,
    pub fveg0: Option<f64>,
    pub sai0: Option<f64>,
    pub z0mr: Option<f64>,
    pub displar: Option<f64>,
    pub sqrtdi: Option<f64>,
    pub chil: Option<f64>,
    pub rhol_vis: Option<f64>,
    pub rhol_nir: Option<f64>,
    pub rhos_vis: Option<f64>,
    pub rhos_nir: Option<f64>,
    pub taul_vis: Option<f64>,
    pub taul_nir: Option<f64>,
    pub taus_vis: Option<f64>,
    pub taus_nir: Option<f64>,
    pub vmax25_umol: Option<f64>,
    pub effcon: Option<f64>,
    pub c3c4: Option<i32>,
    pub respcp: Option<f64>,
    pub shti: Option<f64>,
    pub slti: Option<f64>,
    pub trda: Option<f64>,
    pub trdm: Option<f64>,
    pub trop: Option<f64>,
    pub hhti: Option<f64>,
    pub hlti: Option<f64>,
    pub extkn: Option<f64>,
    pub d50: Option<f64>,
    pub beta: Option<f64>,
}

impl LandClassOverrides {
    /// namelist 名（`DEF_LC_*`，不含 PHS 九列与 `C3C4`）→ 字段。
    pub const REAL_NAMES: [&'static str; 29] = [
        "DEF_LC_HTOP0",
        "DEF_LC_HBOT0",
        "DEF_LC_FVEG0",
        "DEF_LC_SAI0",
        "DEF_LC_Z0MR",
        "DEF_LC_DISPLAR",
        "DEF_LC_SQRTDI",
        "DEF_LC_CHIL",
        "DEF_LC_RHOL_VIS",
        "DEF_LC_RHOL_NIR",
        "DEF_LC_RHOS_VIS",
        "DEF_LC_RHOS_NIR",
        "DEF_LC_TAUL_VIS",
        "DEF_LC_TAUL_NIR",
        "DEF_LC_TAUS_VIS",
        "DEF_LC_TAUS_NIR",
        "DEF_LC_VMAX25",
        "DEF_LC_EFFCON",
        "DEF_LC_RESPCP",
        "DEF_LC_SHTI",
        "DEF_LC_SLTI",
        "DEF_LC_TRDA",
        "DEF_LC_TRDM",
        "DEF_LC_TROP",
        "DEF_LC_HHTI",
        "DEF_LC_HLTI",
        "DEF_LC_EXTKN",
        "DEF_LC_D50",
        "DEF_LC_BETA",
    ];

    /// 按 [`Self::REAL_NAMES`] 的名字设一列；名字不认识时报错。
    pub fn set_real(&mut self, name: &str, value: f64) -> Result<()> {
        let slot = match name {
            "DEF_LC_HTOP0" => &mut self.htop0,
            "DEF_LC_HBOT0" => &mut self.hbot0,
            "DEF_LC_FVEG0" => &mut self.fveg0,
            "DEF_LC_SAI0" => &mut self.sai0,
            "DEF_LC_Z0MR" => &mut self.z0mr,
            "DEF_LC_DISPLAR" => &mut self.displar,
            "DEF_LC_SQRTDI" => &mut self.sqrtdi,
            "DEF_LC_CHIL" => &mut self.chil,
            "DEF_LC_RHOL_VIS" => &mut self.rhol_vis,
            "DEF_LC_RHOL_NIR" => &mut self.rhol_nir,
            "DEF_LC_RHOS_VIS" => &mut self.rhos_vis,
            "DEF_LC_RHOS_NIR" => &mut self.rhos_nir,
            "DEF_LC_TAUL_VIS" => &mut self.taul_vis,
            "DEF_LC_TAUL_NIR" => &mut self.taul_nir,
            "DEF_LC_TAUS_VIS" => &mut self.taus_vis,
            "DEF_LC_TAUS_NIR" => &mut self.taus_nir,
            "DEF_LC_VMAX25" => &mut self.vmax25_umol,
            "DEF_LC_EFFCON" => &mut self.effcon,
            "DEF_LC_RESPCP" => &mut self.respcp,
            "DEF_LC_SHTI" => &mut self.shti,
            "DEF_LC_SLTI" => &mut self.slti,
            "DEF_LC_TRDA" => &mut self.trda,
            "DEF_LC_TRDM" => &mut self.trdm,
            "DEF_LC_TROP" => &mut self.trop,
            "DEF_LC_HHTI" => &mut self.hhti,
            "DEF_LC_HLTI" => &mut self.hlti,
            "DEF_LC_EXTKN" => &mut self.extkn,
            "DEF_LC_D50" => &mut self.d50,
            "DEF_LC_BETA" => &mut self.beta,
            other => anyhow::bail!("{other} is not a land-class scalar override"),
        };
        *slot = Some(value);
        Ok(())
    }
}

pub struct ClassConstants {
    scheme: LandCoverScheme,
    class: usize,
    overrides: LandClassOverrides,
}

impl ClassConstants {
    /// `class_number` 是重启里的 `patchclass`，也就是上游的表位置号。
    pub fn new(scheme: LandCoverScheme, class_number: usize) -> Result<Self> {
        let classes = land_cover_classes(scheme);
        ensure!(
            (1..=classes).contains(&class_number),
            "land class {class_number} is outside 1..={classes} for {scheme:?}; \
             class 0 is ocean, which has no lookup row"
        );
        Ok(Self {
            scheme,
            class: class_number,
            overrides: LandClassOverrides::default(),
        })
    }

    /// 叠上本站的 `DEF_LC_*` 覆盖（只该用于单点 LCT 的本站地类）。
    pub fn with_overrides(mut self, overrides: LandClassOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    /// 表值，有覆盖时取覆盖。
    fn column(
        &self,
        column: fn(&'static LandCoverTables) -> &'static [f64],
        over: fn(&LandClassOverrides) -> Option<f64>,
    ) -> f64 {
        over(&self.overrides).unwrap_or_else(|| self.value(column))
    }

    fn tables(&self) -> &'static LandCoverTables {
        land_cover_tables(self.scheme)
    }

    /// 查表用的 0 基下标（Rust 数组里的位置）。
    ///
    /// **不是上游的 `patchclass`** —— 那个数就是 [`Self::class_number`] 本身，
    /// 这里是再减一之后的数组下标。
    pub fn table_index(&self) -> usize {
        self.class - 1
    }

    /// 重启里的 `patchclass`，即上游的 `SITE_landtype`。
    pub fn class_number(&self) -> usize {
        self.class
    }

    fn value(&self, column: fn(&'static LandCoverTables) -> &'static [f64]) -> f64 {
        column(self.tables())[self.class - 1]
    }

    /// `patchtypes`：0 土壤、1 城市、2 湿地、3 冰、4 湖。
    pub fn patch_type(&self) -> i32 {
        self.tables().patchtypes[self.class - 1]
    }

    /// `Init_LC_Const` 把植被最大羧化速率从 `umol/m2/s` 折成 `mol/m2/s`。
    pub fn maximum_carboxylation_25c_mol_m2_s(&self) -> f64 {
        self.column(|table| table.vmax25, |o| o.vmax25_umol) * 1.0e-6
    }

    pub fn canopy_top_m(&self) -> f64 {
        self.column(|table| table.htop0, |o| o.htop0)
    }

    /// `lambda`：WUE 气孔模型的边际耗水成本**基准值**。
    ///
    /// 基准来自地类表，不来自 namelist。上游 `DEF_WUE_LAMBDA` 默认 `-1` 表示
    /// "用表值"，只有 `> 0` 才是覆盖（`MOD_AssimStomataConductance.F90:201`）。
    /// 拿 namelist 的值当基准，默认算例就会把 `-1` 送进 WUE 求解。
    pub fn wue_lambda(&self) -> f64 {
        self.value(|table| table.lambda)
    }

    pub fn canopy_bottom_m(&self) -> f64 {
        self.column(|table| table.hbot0, |o| o.hbot0)
    }

    pub fn vegetation_fraction(&self) -> f64 {
        self.column(|table| table.fveg0, |o| o.fveg0)
    }

    pub fn stem_area_index(&self) -> f64 {
        self.column(|table| table.sai0, |o| o.sai0)
    }

    /// `z0mr`：粗糙度长与冠层高度之比。
    pub fn roughness_to_height_ratio(&self) -> f64 {
        self.column(|table| table.z0mr, |o| o.z0mr)
    }

    /// `displar`：零平面位移与冠层高度之比。
    pub fn displacement_to_height_ratio(&self) -> f64 {
        self.column(|table| table.displar, |o| o.displar)
    }

    /// `sqrtdi`：叶片尺度的 `m**-0.5`。
    pub fn inverse_sqrt_leaf_dimension_m_neg_half(&self) -> f64 {
        self.column(|table| table.sqrtdi, |o| o.sqrtdi)
    }

    /// `fveg0`：该地类的最大植被覆盖度。
    ///
    /// `MOD_LAIReadin.F90:132-143` 用它判 `green`：`fveg0(m) > 0` 才是绿叶。
    pub fn maximum_vegetation_fraction(&self) -> f64 {
        self.column(|table| table.fveg0, |o| o.fveg0)
    }

    /// `chil`：叶倾角分布参数，即内核里的 `leaf_angle_distribution`。
    pub fn leaf_angle_distribution(&self) -> f64 {
        self.column(|table| table.chil, |o| o.chil)
    }

    pub fn extinction_coefficient(&self) -> f64 {
        self.column(|table| table.extkn, |o| o.extkn)
    }

    /// 地类表里的九个植物水力性状，再按 `DEF_LC_*` 覆盖。
    ///
    /// 上游 `MOD_Const_LC.F90:763`/`:815` 把 `kmax_sun0_usgs`/`kmax_sun0_igbp`
    /// 整列抄进 `kmax_sun`，`:919` 再按 `DEF_LC_KMAX_SUN /= LC_OVERRIDE_UNSET`
    /// 逐个覆盖。**标准 LCT 路径用的是这张地类表**；per-PFT 的那一份
    /// （`MOD_Const_PFT.F90` 的 `kmax_sun_p`）只被 `LeafTemperaturePC` 用
    /// （`MOD_Thermal_CanopyPhase_Extended.F90:706` 对 `:922`）。
    pub fn plant_hydraulic_traits(
        &self,
        overrides: PlantHydraulicOverrides,
    ) -> PlantHydraulicTraits {
        let pick = |table: f64, over: Option<f64>| over.unwrap_or(table);
        PlantHydraulicTraits {
            maximum_sunlit_leaf_conductance: pick(
                self.value(|table| table.kmax_sun0),
                overrides.maximum_sunlit_leaf_conductance,
            ),
            maximum_shaded_leaf_conductance: pick(
                self.value(|table| table.kmax_sha0),
                overrides.maximum_shaded_leaf_conductance,
            ),
            maximum_xylem_conductance: pick(
                self.value(|table| table.kmax_xyl0),
                overrides.maximum_xylem_conductance,
            ),
            maximum_root_conductance: pick(
                self.value(|table| table.kmax_root0),
                overrides.maximum_root_conductance,
            ),
            sunlit_leaf_psi50_mm: pick(
                self.value(|table| table.psi50_sun0),
                overrides.sunlit_leaf_psi50_mm,
            ),
            shaded_leaf_psi50_mm: pick(
                self.value(|table| table.psi50_sha0),
                overrides.shaded_leaf_psi50_mm,
            ),
            xylem_psi50_mm: pick(
                self.value(|table| table.psi50_xyl0),
                overrides.xylem_psi50_mm,
            ),
            root_psi50_mm: pick(
                self.value(|table| table.psi50_root0),
                overrides.root_psi50_mm,
            ),
            vulnerability_shape: pick(self.value(|table| table.ck0), overrides.vulnerability_shape),
        }
    }

    /// `d50`：根系分布的特征深度。
    pub fn root_d50(&self) -> f64 {
        self.column(|table| table.d50, |o| o.d50)
    }

    /// `beta`：根系分布的形参。
    pub fn root_beta(&self) -> f64 {
        self.column(|table| table.beta, |o| o.beta)
    }

    /// `roota` / `rootb`：双指数根系分布的两个衰减率。
    pub fn root_exponential_rates(&self) -> (f64, f64) {
        (
            self.value(|table| table.roota),
            self.value(|table| table.rootb),
        )
    }

    /// `MOD_AssimStomataConductance:stomata` 的生化参数，逐项取自上表。
    ///
    /// 冠层积分因子 `cint(1:3)` **不在这里**：它是每步从 `lai`/`extkb`/`extkd` 算出的
    /// `cintsun`/`cintsha`（`MOD_LeafTemperature.F90:460-466`），由
    /// `LeafPhotosynthesisInput::canopy_integration` 逐群体传入。
    pub fn biochemistry(&self) -> crate::LeafBiochemistry {
        crate::LeafBiochemistry {
            quantum_efficiency: self.column(|table| table.effcon, |o| o.effcon),
            maximum_carboxylation_25c_mol_m2_s: self.maximum_carboxylation_25c_mol_m2_s(),
            c3c4: self.c3c4(),
            respiration_fraction_override: self.overrides.respcp,
            low_temperature_slope: self.column(|table| table.slti, |o| o.slti),
            low_temperature_half_k: self.column(|table| table.hlti, |o| o.hlti),
            high_temperature_slope: self.column(|table| table.shti, |o| o.shti),
            high_temperature_half_k: self.column(|table| table.hhti, |o| o.hhti),
            respiration_temperature_slope: self.column(|table| table.trda, |o| o.trda),
            respiration_temperature_half_k: self.column(|table| table.trdm, |o| o.trdm),
            optimum_temperature_k: self.column(|table| table.trop, |o| o.trop),
            medlyn_g1: self.value(|table| table.g1),
            medlyn_g0: self.value(|table| table.g0),
            ball_berry_slope: self.value(|table| table.gradm),
            ball_berry_intercept: self.value(|table| table.binter),
        }
    }

    /// `c3c4`：1 = C3，0 = C4。
    pub fn c3c4(&self) -> i32 {
        self.overrides
            .c3c4
            .unwrap_or(self.tables().c3c4[self.class - 1])
    }

    /// `Init_LC_Const` 的 `rho(1,1,:)=rhol_vis` 那一组赋值。
    pub fn leaf_optics(&self) -> LeafOptics {
        LeafOptics {
            chil: self.column(|table| table.chil, |o| o.chil),
            reflectance: [
                [
                    self.column(|table| table.rhol_vis, |o| o.rhol_vis),
                    self.column(|table| table.rhos_vis, |o| o.rhos_vis),
                ],
                [
                    self.column(|table| table.rhol_nir, |o| o.rhol_nir),
                    self.column(|table| table.rhos_nir, |o| o.rhos_nir),
                ],
            ],
            transmittance: [
                [
                    self.column(|table| table.taul_vis, |o| o.taul_vis),
                    self.column(|table| table.taus_vis, |o| o.taus_vis),
                ],
                [
                    self.column(|table| table.taul_nir, |o| o.taul_nir),
                    self.column(|table| table.taus_nir, |o| o.taus_nir),
                ],
            ],
        }
    }
}

/// `Init_LC_Const` 的 `rootfr`：逐层根系比例，求和恰为 1。
///
/// `interface_depth_m` 是共享土层网格的**层底界面深度**（`zi_soi`，`[0] = 0`），
/// 长度必须是 `layers + 1`。上游的 `zi_soi(nsl)` 就是这里的
/// `interface_depth_m[nsl]`（`nsl` 自 1 起），也就是第 `nsl` 层的层底深度。
///
/// 两支的分母都写成 `1 + f`，所以中间层是**差分**；正因为是差分，求和必然为 1
/// （实测两支都到 1e-15 以内），这一条也写成了测试。
pub fn root_fraction(
    scheme: LandCoverScheme,
    land_class: i32,
    root_scheme: RootFractionScheme,
    interface_depth_m: &[f64],
    overrides: LandClassOverrides,
) -> Result<Vec<f64>> {
    let classes = land_cover_classes(scheme);
    let index = usize::try_from(land_class)
        .ok()
        .filter(|index| (1..=classes).contains(index))
        .ok_or_else(|| {
            anyhow::anyhow!("land class index {land_class} is outside 1..={classes} for {scheme:?}")
        })?;
    let layers = interface_depth_m.len().checked_sub(1).ok_or_else(|| {
        anyhow::anyhow!("the interface depth array must hold one more entry than layers")
    })?;
    ensure!(layers >= 2, "root fractions need at least two soil layers");
    ensure!(
        interface_depth_m.windows(2).all(|pair| pair[0] < pair[1]),
        "soil interface depths must increase downwards"
    );
    // `zi_soi(nsl)`，`nsl` 自 1 起。
    let zi = |nsl: usize| interface_depth_m[nsl];
    // 上游的 `i` 是 1 基地类下标。
    let class = ClassConstants::new(scheme, index)?.with_overrides(overrides);
    let mut fractions = vec![0.0; layers];
    match root_scheme {
        RootFractionScheme::SchenkJackson => {
            let d50 = class.root_d50();
            let beta = class.root_beta();
            ensure!(
                d50 > 0.0,
                "land class {land_class} has a non-positive d50, so its root distribution is undefined"
            );
            fractions = schenk_jackson_root_fraction(d50, beta, interface_depth_m);
        }
        RootFractionScheme::Exponential => {
            let (roota, rootb) = class.root_exponential_rates();
            let decay = |nsl: usize| 0.5 * ((-roota * zi(nsl)).exp() + (-rootb * zi(nsl)).exp());
            fractions[0] = 1.0 - decay(1);
            fractions[layers - 1] = decay(layers);
            for nsl in 2..layers {
                fractions[nsl - 1] = decay(nsl - 1) - decay(nsl);
            }
        }
    }
    Ok(fractions)
}

/// `ROOTFR_SCHEME == 1` 的 Schenk & Jackson (2002) 根系分布。
///
/// 地类表（`MOD_Const_LC`）与 PFT 表（`MOD_Const_PFT.F90:1870-1883`）写的是同一个
/// 式子，只是 `d50`/`beta` 的来源不同 —— PFT 那一份的 `ROOTFR_SCHEME` 是模块私有的
/// 常量 1，不受 namelist 控制。`interface_depth_m` 是 `zi_soi(0:nl_soil)`。
pub fn schenk_jackson_root_fraction(d50: f64, beta: f64, interface_depth_m: &[f64]) -> Vec<f64> {
    let layers = interface_depth_m.len() - 1;
    let cumulative = |nsl: usize| 1.0 / (1.0 + (interface_depth_m[nsl] * 100.0 / d50).lpow(beta));
    let mut fractions = vec![0.0; layers];
    fractions[0] = cumulative(1);
    fractions[layers - 1] = 1.0 - cumulative(layers - 1);
    for nsl in 2..layers {
        fractions[nsl - 1] = cumulative(nsl) - cumulative(nsl - 1);
    }
    fractions
}

#[cfg(test)]
#[path = "land_cover_tests.rs"]
mod land_cover_tests;
